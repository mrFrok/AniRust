// SPDX-License-Identifier: GPL-3.0-or-later

//! Getting mpv's output onto a Slint surface without copying frames.
//!
//! The two halves meet at an OpenGL texture. We allocate a texture and a
//! framebuffer, mpv renders into that framebuffer, and Slint borrows the same
//! texture as an [`slint::Image`]. No pixels travel through the CPU, which is
//! what keeps 1080p affordable.
//!
//! Slint's own ffmpeg example takes the other route — decode, convert to RGB,
//! copy into a `SharedPixelBuffer`. That is simpler and portable, but it pays
//! a colour conversion and a full-frame copy per frame, and it gives up
//! hardware decoding. Since mpv already renders on the GPU, borrowing its
//! output is both faster and less code.
//!
//! # Ordering
//!
//! Everything here runs on the UI thread inside Slint's rendering notifier,
//! where the GL context is current:
//!
//! * `RenderingSetup` — load GL, build the surface, create mpv's renderer;
//! * `BeforeRendering` — if mpv has a new frame, draw it into our framebuffer
//!   and hand the texture to the UI;
//! * `RenderingTeardown` — drop everything while the context is still alive.
//!
//! mpv's "new frame" callback fires on an mpv thread and may only wake the UI
//! thread; it must never touch GL or call back into mpv.

use std::cell::{Cell, RefCell};
use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, anyhow};
use glow::HasContext;
use slint::{ComponentHandle, GraphicsAPI, Image, RenderingState};

use anirust_player::{MediaSource, Player, Renderer, render::Target};

/// Frames at which `ANIRUST_DUMP` captures the texture.
///
/// The first is late enough that decoding has settled; the second tells a
/// defect fixed to the texture from one that drifts with the picture.
///
/// Debug builds only. This writes several megabytes to a path taken from the
/// environment, which has no business being reachable in a release binary.
#[cfg(debug_assertions)]
const DUMP_AT_FRAMES: [u64; 2] = [60, 300];

/// How often the repaint loop is nudged back into motion.
///
/// The loop itself is driven from `AfterRendering`, which keeps it in step
/// with the display: mpv derives the refresh rate from when frames are
/// reported as swapped, and a fixed-interval timer drifting out of phase makes
/// that estimate wrong — enough that interpolation stops producing frames
/// entirely.
///
/// This timer only starts the loop and restarts it if it ever stalls, so it
/// can be slow.
const REPAINT_KICK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

/// A texture plus the framebuffer that draws into it.
///
/// Sized to the *render* resolution, which is the window's size rather than
/// the source's — see [`target_size`]. Recreated whenever that changes.
struct Surface {
    gl: Rc<glow::Context>,
    texture: glow::Texture,
    framebuffer: glow::Framebuffer,
    width: u32,
    height: u32,
}

impl Surface {
    fn new(gl: Rc<glow::Context>, width: u32, height: u32) -> Result<Self> {
        // SAFETY: called from the rendering notifier, so the GL context this
        // `glow::Context` was loaded from is current on this thread.
        unsafe {
            let texture = gl
                .create_texture()
                .map_err(|e| anyhow!("creating the video texture: {e}"))?;
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            // The internal format must be *sized* (RGBA8, not RGBA). Slint
            // gives us an OpenGL ES context, and ES only guarantees sized
            // formats are colour-renderable; an unsized RGBA still passes the
            // completeness check but leaves the driver free to do as it likes
            // when rendering into it, which showed up as torn bands.
            // The upload format stays GL_RGBA/UNSIGNED_BYTE, which is what
            // Slint expects to sample.
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                width as i32,
                height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            // Linear filtering, and clamping so the edges never sample across
            // the texture when the image is scaled.
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );

            let framebuffer = gl
                .create_framebuffer()
                .map_err(|e| anyhow!("creating the video framebuffer: {e}"))?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );

            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            if status != glow::FRAMEBUFFER_COMPLETE {
                return Err(anyhow!(
                    "video framebuffer is incomplete (status {status:#x})"
                ));
            }

            // Leave the driver where we found it: Slint draws into the default
            // framebuffer right after this.
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.bind_texture(glow::TEXTURE_2D, None);

            Ok(Self {
                gl,
                texture,
                framebuffer,
                width,
                height,
            })
        }
    }

    /// Framebuffer id in the form mpv's render API expects.
    fn fbo_id(&self) -> i32 {
        // glow's newtype wraps the raw name GL itself uses.
        self.framebuffer.0.get() as i32
    }

    /// Borrows the texture as a Slint image.
    ///
    /// Exactly one vertical flip may happen between mpv and the screen. mpv is
    /// asked not to flip (`flip_y(false)`), and what lands in the texture is
    /// already row-zero-at-top, so Slint is told `TopLeft` and does not flip
    /// either. Declaring `BottomLeft` here was the second flip, and the
    /// picture came out upside down.
    fn as_image(&self) -> Option<Image> {
        let id = NonZeroU32::new(self.texture.0.get())?;

        // SAFETY: the texture was created by the GL context that is current
        // during the rendering notifier, which is the context Slint renders
        // with, and it outlives the image because `Surface` is dropped only in
        // `RenderingTeardown`.
        let builder = unsafe {
            slint::BorrowedOpenGLTextureBuilder::new_gl_2d_rgba_texture(
                id,
                (self.width, self.height).into(),
            )
        };
        Some(
            builder
                .origin(slint::BorrowedOpenGLTextureOrigin::TopLeft)
                .build(),
        )
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: teardown runs with the context still current.
        unsafe {
            self.gl.delete_framebuffer(self.framebuffer);
            self.gl.delete_texture(self.texture);
        }
    }
}

/// How many textures to rotate through.
///
/// Overwriting a texture the renderer may still be reading is a real hazard:
/// drawing is batched and the GPU trails the CPU, so a texture handed over can
/// still be in flight a frame or two later. Three gives the pipeline room, at
/// the cost of one extra 1280x720 texture - a few megabytes.
///
/// This is insurance, not a fix for anything observed: the banding that
/// prompted the investigation came from mpv's own float framebuffers, and no
/// amount of buffering or synchronisation on this side changed it.
const SURFACE_COUNT: usize = 3;

// `Surfaces::new` writes out this many constructor calls by hand; a mismatch
// is caught here rather than at the far end of a confusing type error.
const _: () = assert!(SURFACE_COUNT == 3, "Surfaces::new builds three surfaces");

/// Textures rendered into in rotation, so nothing is overwritten while the
/// renderer may still be reading it.
struct Surfaces {
    /// An array rather than a `Vec`: the count is fixed, and a type that
    /// cannot be empty removes the unchecked index below.
    surfaces: [Surface; SURFACE_COUNT],
    /// Index of the surface the next frame renders into.
    next: usize,
}

impl Surfaces {
    fn new(gl: &Rc<glow::Context>, width: u32, height: u32) -> Result<Self> {
        // `array::try_map` is still unstable, so the array is built
        // explicitly. Written out rather than collected from an iterator
        // because that would hand back a `Vec` and lose the very invariant
        // this type exists to state.
        let surfaces = [
            Surface::new(Rc::clone(gl), width, height)?,
            Surface::new(Rc::clone(gl), width, height)?,
            Surface::new(Rc::clone(gl), width, height)?,
        ];
        Ok(Self { surfaces, next: 0 })
    }

    fn matches(&self, width: u32, height: u32) -> bool {
        // Every surface in the set is created at the same size.
        let surface = &self.surfaces[0];
        surface.width == width && surface.height == height
    }

    /// The surface the next frame should be rendered into.
    fn target(&self) -> &Surface {
        &self.surfaces[self.next]
    }

    /// Marks the current target as presented and moves to the next one.
    fn advance(&mut self) {
        self.next = (self.next + 1) % self.surfaces.len();
    }
}

/// Everything that only exists while there is a live GL context.
struct Live {
    gl: Rc<glow::Context>,
    renderer: Renderer<'static>,
    surfaces: Option<Surfaces>,
    /// Counts frames that actually reached the UI, so the texture path can be
    /// confirmed from a log rather than by squinting at a window.
    frames_drawn: u64,
    /// Counts how often the UI asked for a frame, which distinguishes "nothing
    /// is repainting" from "repainting but mpv has nothing".
    draw_calls: u64,
}

/// Bridges a [`Player`] to a Slint window.
///
/// The player is borrowed for `'static` because mpv's render context must not
/// outlive it, and the renderer lives as long as the window does. Leaking one
/// allocation at startup buys that without threading a lifetime through the
/// whole UI — but only because there is exactly one player per process.
///
/// That constraint is enforced rather than assumed: a second bridge would leak
/// a second mpv instance, with its own threads and GPU context, and nothing in
/// the types would have stopped it once more screens exist.
pub struct VideoBridge {
    player: &'static Player,
    live: Rc<RefCell<Option<Live>>>,
    /// A source asked for before the render context existed.
    pending: Rc<RefCell<Option<MediaSource>>>,
    /// Resolution frames are actually rendered at.
    ///
    /// Not the same as the source's: with upscaling the chain runs at the
    /// window's size, and the interface should be able to say so rather than
    /// quietly reporting the input resolution.
    rendered: Rc<Cell<(u32, u32)>>,
    /// Whether the picture is advancing, sampled off the render loop.
    ///
    /// Asking mpv on every frame costs a property query — and a lock inside
    /// mpv — at display rate. Since this only decides whether to schedule the
    /// next repaint, a reading a few hundred milliseconds old is fine.
    advancing: Rc<Cell<bool>>,
    /// Restarts the repaint loop when it stalls.
    ///
    /// Owned rather than leaked: a Slint timer stops when dropped, so tying it
    /// to the bridge makes its lifetime the thing it actually depends on.
    repaint_kick: RefCell<Option<slint::Timer>>,
}

/// Guards the single-player invariant. See [`VideoBridge`].
static BRIDGE_EXISTS: AtomicBool = AtomicBool::new(false);

impl VideoBridge {
    /// Takes ownership of the player and pins it for the process lifetime.
    ///
    /// Fails if a bridge already exists, because the pinning is a deliberate
    /// leak and a second one would multiply it.
    pub fn new(player: Player) -> Result<Self> {
        if BRIDGE_EXISTS.swap(true, Ordering::SeqCst) {
            return Err(anyhow!(
                "a video bridge already exists; there is one player per process"
            ));
        }

        Ok(Self {
            player: Box::leak(Box::new(player)),
            live: Rc::new(RefCell::new(None)),
            pending: Rc::new(RefCell::new(None)),
            rendered: Rc::new(Cell::new((0, 0))),
            advancing: Rc::new(Cell::new(false)),
            repaint_kick: RefCell::new(None),
        })
    }

    /// Starts playing a source.
    ///
    /// Loading is deferred until a render context exists. With `vo=libmpv`
    /// mpv cannot bring up its video output before then, and a file loaded too
    /// early is abandoned rather than retried — which looks exactly like a
    /// stream that never buffers.
    pub fn play(&self, source: MediaSource) -> Result<()> {
        if self.live.borrow().is_some() {
            self.player.open(&source).context("opening the stream")?;
        } else {
            tracing::debug!("deferring playback until the render context exists");
            *self.pending.borrow_mut() = Some(source);
        }
        Ok(())
    }

    pub fn player(&self) -> &'static Player {
        self.player
    }

    /// Tells the render loop whether the picture is advancing.
    ///
    /// Called from the status poll rather than from the render loop itself,
    /// so the hot path never asks mpv. Passing `false` lets the loop wind
    /// down; the kick timer restarts it when playback resumes.
    pub fn set_advancing(&self, advancing: bool) {
        self.advancing.set(advancing);
    }

    /// Resolution the last frame was rendered at, or `None` before the first.
    #[must_use]
    pub fn rendered_size(&self) -> Option<(u32, u32)> {
        let size = self.rendered.get();
        (size.0 > 0 && size.1 > 0).then_some(size)
    }

    /// Hooks the bridge into a window's render loop.
    ///
    /// `on_frame` receives the borrowed texture whenever a new frame was
    /// drawn; the caller assigns it to whatever property the UI binds to.
    ///
    /// Frames are rendered at the window's size rather than the source's. That
    /// matters for upscaling: rendering at the source resolution would run
    /// Anime4K's chain and then squeeze the result back down to where it
    /// started, so the work would never reach the screen.
    pub fn attach<C>(&self, component: &C, on_frame: impl Fn(&C, Image) + 'static) -> Result<()>
    where
        C: ComponentHandle + 'static,
    {
        let player = self.player;
        let live = Rc::clone(&self.live);
        let pending = Rc::clone(&self.pending);
        let rendered = Rc::clone(&self.rendered);
        let advancing = Rc::clone(&self.advancing);
        let weak = component.as_weak();
        // Handed to mpv so it can wake the UI when a frame is ready. Cloned
        // because the notifier closure needs its own.
        let wake = component.as_weak();

        component
            .window()
            .set_rendering_notifier(move |state, graphics_api| {
                match state {
                    RenderingState::RenderingSetup => {
                        match setup(player, graphics_api) {
                            Ok(mut new_live) => {
                                tracing::info!("render context created");
                                // Fires on an mpv thread: the only safe action
                                // is to ask the UI thread to repaint. Touching
                                // GL or mpv from here is forbidden.
                                let wake = wake.clone();
                                new_live.renderer.set_update_callback(move || {
                                    let wake = wake.clone();
                                    let _ = slint::invoke_from_event_loop(move || {
                                        if let Some(component) = wake.upgrade() {
                                            component.window().request_redraw();
                                        }
                                    });
                                });
                                *live.borrow_mut() = Some(new_live);

                                // Now that mpv has somewhere to render, it can
                                // be told what to play.
                                if let Some(source) = pending.borrow_mut().take() {
                                    match player.open(&source) {
                                        Ok(()) => {
                                            tracing::info!(url = %source.url, "playback started")
                                        }
                                        Err(error) => {
                                            tracing::error!(%error, "could not open the stream");
                                        }
                                    }
                                }
                            }
                            Err(error) => {
                                // A failure here means no video, not a crash:
                                // the rest of the UI stays usable and the log
                                // says why.
                                tracing::error!(%error, "could not set up video rendering");
                            }
                        }
                    }

                    RenderingState::BeforeRendering => {
                        let Some(component) = weak.upgrade() else {
                            return;
                        };
                        let mut guard = live.borrow_mut();
                        let Some(live) = guard.as_mut() else {
                            tracing::debug!("before-rendering with no render context yet");
                            return;
                        };

                        let surface_size = (
                            component.window().size().width,
                            component.window().size().height,
                        );
                        match draw(player, live, surface_size, &rendered) {
                            Ok(Some(image)) => {
                                live.frames_drawn += 1;
                                // Evidence that the texture path is alive, at
                                // a cadence that does not flood a log.
                                if live.frames_drawn % 120 == 1 {
                                    tracing::info!(
                                        frames = live.frames_drawn,
                                        hwdec = ?player.active_hwdec(),
                                        "video frames are reaching the UI"
                                    );
                                }
                                on_frame(&component, image);
                            }
                            Ok(None) => {}
                            Err(error) => {
                                tracing::warn!(error = ?error, "video frame was not drawn")
                            }
                        }
                    }

                    RenderingState::AfterRendering => {
                        if let Some(live) = live.borrow().as_ref() {
                            // Frame pacing and interpolation depend on mpv
                            // being told when a frame actually reached the
                            // screen.
                            live.renderer.report_swap();
                        }

                        // Ask for the next frame from inside the render loop,
                        // so repaints follow the display rather than a timer
                        // of our own. This is what gives mpv a usable refresh
                        // estimate.
                        //
                        // Only while the picture is actually advancing. A
                        // paused file is still "loaded", so keying this off
                        // that would repaint at display rate forever with
                        // nothing changing — a flat battery for a video
                        // nobody is watching.
                        if advancing.get()
                            && let Some(component) = weak.upgrade()
                        {
                            component.window().request_redraw();
                        }
                    }

                    // Dropping here, rather than letting it happen whenever,
                    // guarantees the GL objects are deleted while their
                    // context is still current.
                    RenderingState::RenderingTeardown => *live.borrow_mut() = None,

                    _ => {}
                }
            })
            .context(
                "Slint rejected the rendering notifier; the renderer is probably not OpenGL",
            )?;

        self.drive_repaints(component);
        Ok(())
    }

    /// Starts the repaint loop, and restarts it if it ever stalls.
    ///
    /// The loop itself runs from `AfterRendering`; this only kicks it off.
    /// Something has to, because the window will not repaint until asked, and
    /// until it does mpv has no reason to produce a frame.
    ///
    /// Kept alive for the window's lifetime: dropping the timer would leave a
    /// stall unrecoverable.
    fn drive_repaints<C: ComponentHandle + 'static>(&self, component: &C) {
        let player = self.player;
        let weak = component.as_weak();

        let timer = slint::Timer::default();
        timer.start(
            slint::TimerMode::Repeated,
            REPAINT_KICK_INTERVAL,
            move || {
                let Some(component) = weak.upgrade() else {
                    return;
                };
                // Restart the loop whenever the picture should be moving and
                // is not. Asking mpv here is fine: a quarter-second cadence is
                // nowhere near the hot path.
                if player.state().is_active() {
                    component.window().request_redraw();
                }
            },
        );
        *self.repaint_kick.borrow_mut() = Some(timer);
    }
}

fn setup(player: &'static Player, graphics_api: &GraphicsAPI<'_>) -> Result<Live> {
    let GraphicsAPI::NativeOpenGL { get_proc_address } = graphics_api else {
        return Err(anyhow!(
            "video needs the OpenGL renderer; Slint is using a different one"
        ));
    };

    // SAFETY: Slint calls this with its GL context current, and the loader it
    // hands us resolves symbols from that context.
    let gl =
        Rc::new(unsafe { glow::Context::from_loader_function_cstr(|name| get_proc_address(name)) });

    // mpv resolves GL entry points through the same loader. The pointer is
    // stored in the render context, so it must not borrow anything local.
    let renderer = player
        .renderer(GlLoader::open()?, |loader, name| loader.resolve(name))
        .context("creating mpv's render context")?;

    Ok(Live {
        gl,
        renderer,
        surfaces: None,
        frames_drawn: 0,
        draw_calls: 0,
    })
}

/// Largest render target we will allocate, per side.
///
/// A guard rather than a preference: a window dragged onto a 5K display should
/// not silently ask the GPU for a texture that big every frame.
const MAX_TARGET_SIDE: u32 = 3840;

/// Chooses the resolution to render at.
///
/// Never below the source, because that would throw detail away, and never
/// above what the window can show, because those pixels are discarded on the
/// way to the screen. The aspect ratio is the source's: letterboxing is the
/// image element's job, not the renderer's.
fn target_size(source: (u32, u32), surface: (u32, u32)) -> (u32, u32) {
    let (source_w, source_h) = source;
    let (surface_w, surface_h) = surface;

    if source_w == 0 || source_h == 0 || surface_w == 0 || surface_h == 0 {
        return source;
    }

    // Scale the source up until it just covers the surface in one dimension.
    let scale = (f64::from(surface_w) / f64::from(source_w))
        .min(f64::from(surface_h) / f64::from(source_h))
        .max(1.0);

    let width = ((f64::from(source_w) * scale).round() as u32).min(MAX_TARGET_SIDE);
    let height = ((f64::from(source_h) * scale).round() as u32).min(MAX_TARGET_SIDE);

    // Even dimensions keep chroma subsampling and downscaling filters happy.
    (width & !1, height & !1)
}

/// Draws the current frame, returning the image to show when one was produced.
fn draw(
    player: &Player,
    live: &mut Live,
    surface_size: (u32, u32),
    rendered: &Cell<(u32, u32)>,
) -> Result<Option<Image>> {
    live.draw_calls += 1;

    // Size is checked first on purpose: `needs_redraw` consumes mpv's frame
    // flag, so bailing out after asking would throw the frame away and mpv
    // would never offer it again. That shows up as stutter, not as an error.
    let Some(source) = player.video_size() else {
        return Ok(None);
    };
    let (width, height) = target_size(source, surface_size);
    rendered.set((width, height));
    let size = Some((width, height));

    // Everything that can fail happens before the frame flag is touched.
    // `needs_redraw` *consumes* it, and mpv will not offer the same frame
    // twice — so an early return after asking loses that frame for good,
    // which presents as a stall rather than as the error it really is.
    if live
        .surfaces
        .as_ref()
        .is_none_or(|s| !s.matches(width, height))
    {
        tracing::debug!(width, height, "(re)creating the video surfaces");
        live.surfaces = Some(Surfaces::new(&live.gl, width, height)?);
    }

    let redraw = live.renderer.needs_redraw();

    // One line a second tells a stalled pipeline from a working one. Debug
    // level: in a release build this is noise, and the interface already
    // shows whether the picture is moving.
    if live.draw_calls % 60 == 1 {
        tracing::debug!(draw_calls = live.draw_calls, redraw, ?size, "draw tick");
    }

    if !redraw {
        return Ok(None);
    }

    let surfaces = live.surfaces.as_ref().expect("created above");
    let surface = surfaces.target();

    // mpv documents that it restores OpenGL state to defaults *except* for the
    // viewport and the scissor box. Slint draws the rest of the UI straight
    // after this with whatever state it left behind, which shows up as torn or
    // misplaced drawing. Saving and restoring around the call is the fix, and
    // it is what Slint's own OpenGL example does too.
    let saved = GlState::save(&live.gl);
    let result = live
        .renderer
        .render(Target::new(surface.fbo_id(), width, height).flip_y(false))
        .context("mpv failed to render a frame");
    // Diagnostic: capture what mpv actually put in the texture, so a defect
    // can be attributed to mpv's rendering or to how the UI samples it,
    // instead of being guessed at from the window. This is how the banding
    // was traced to mpv's own framebuffers.
    #[cfg(debug_assertions)]
    if DUMP_AT_FRAMES.contains(&live.frames_drawn)
        && let Ok(path) = std::env::var("ANIRUST_DUMP")
    {
        dump_framebuffer(&live.gl, surface, &format!("{path}.{}", live.frames_drawn));
    }

    saved.restore(&live.gl);
    result?;

    let image = surface.as_image();
    // The surface just drawn is now the one on screen; the next frame goes to
    // the next in rotation.
    live.surfaces.as_mut().expect("created above").advance();
    Ok(image)
}

/// The OpenGL state mpv is documented not to restore, plus the framebuffer
/// binding it takes over.
struct GlState {
    viewport: [i32; 4],
    scissor_box: [i32; 4],
    scissor_enabled: bool,
    draw_framebuffer: Option<glow::NativeFramebuffer>,
}

impl GlState {
    fn save(gl: &glow::Context) -> Self {
        let mut viewport = [0i32; 4];
        let mut scissor_box = [0i32; 4];

        // SAFETY: runs inside the rendering notifier, where the context these
        // queries address is current.
        unsafe {
            gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
            gl.get_parameter_i32_slice(glow::SCISSOR_BOX, &mut scissor_box);
            let scissor_enabled = gl.is_enabled(glow::SCISSOR_TEST);
            let raw = gl.get_parameter_i32(glow::DRAW_FRAMEBUFFER_BINDING);
            let draw_framebuffer = NonZeroU32::new(raw as u32).map(glow::NativeFramebuffer);

            Self {
                viewport,
                scissor_box,
                scissor_enabled,
                draw_framebuffer,
            }
        }
    }

    fn restore(&self, gl: &glow::Context) {
        // SAFETY: same context, same thread, immediately after `save`.
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, self.draw_framebuffer);
            gl.viewport(
                self.viewport[0],
                self.viewport[1],
                self.viewport[2],
                self.viewport[3],
            );
            gl.scissor(
                self.scissor_box[0],
                self.scissor_box[1],
                self.scissor_box[2],
                self.scissor_box[3],
            );
            if self.scissor_enabled {
                gl.enable(glow::SCISSOR_TEST);
            } else {
                gl.disable(glow::SCISSOR_TEST);
            }
        }
    }
}

/// Writes the framebuffer's pixels to a file as raw RGBA.
///
/// Debug builds only; see [`DUMP_AT_FRAMES`].
///
/// Raw rather than an image format on purpose: this is a diagnostic, and
/// pulling in an encoder to debug a texture would be a poor trade. `ffmpeg`
/// turns it into something viewable.
#[cfg(debug_assertions)]
fn dump_framebuffer(gl: &glow::Context, surface: &Surface, path: &str) {
    let (width, height) = (surface.width, surface.height);
    let mut pixels = vec![0u8; (width as usize) * (height as usize) * 4];

    // SAFETY: the context is current, and the buffer is sized for the format
    // and dimensions being requested.
    unsafe {
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(surface.framebuffer));
        gl.read_pixels(
            0,
            0,
            width as i32,
            height as i32,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut pixels)),
        );
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
    }

    match std::fs::write(path, &pixels) {
        Ok(()) => tracing::info!(path, width, height, "framebuffer captured"),
        Err(error) => tracing::warn!(%error, path, "could not write the capture"),
    }
}

/// Resolves GL entry points for mpv.
///
/// mpv wants a plain `fn` pointer plus a context value that outlives the
/// render context, while Slint's loader is a borrowed closure valid only
/// inside the notifier callback. So rather than smuggling Slint's loader out,
/// this opens the platform's own GL loader once and calls that.
///
/// It has to be the platform loader — `dlsym` against the process image finds
/// only exported symbols, and GL *extension* entry points are not exported.
/// Those are exactly what mpv resolves, so a naive lookup returns null on some
/// drivers. mpv's own header recommends this approach: "you can simply call
/// the GL context APIs from this callback (e.g. glXGetProcAddressARB or
/// wglGetProcAddress)".
///
/// On Windows the resolver is WGL's, with the GL 1.1 functions it does not
/// answer for taken from opengl32.dll directly; on macOS there is no resolver,
/// and every function is looked up in the OpenGL framework by name.
///
/// The libraries are opened by soname, so the dynamic linker's search path —
/// `LD_LIBRARY_PATH` included — decides which file is loaded. That is not a
/// trust boundary worth defending: anyone who can set that variable for this
/// process can already do anything the process can.
pub struct GlLoader {
    library: libloading::Library,
    /// The platform's resolver, where it has one. macOS has none: every GL
    /// function is an ordinary export of its OpenGL framework.
    get_proc_address: Option<GetProcAddress>,
}

type GetProcAddress = unsafe extern "C" fn(*const std::ffi::c_char) -> *mut std::ffi::c_void;

impl GlLoader {
    /// Libraries to try, in order, with the resolver each exposes.
    ///
    /// EGL comes first because Slint's winit backend prefers it on Linux, and
    /// it is the only option under Wayland. GLX is the X11 fallback.
    #[cfg(all(unix, not(target_os = "macos")))]
    const CANDIDATES: &'static [(&'static str, Option<&'static [u8]>)] = &[
        ("libEGL.so.1", Some(b"eglGetProcAddress\0")),
        ("libGLX.so.0", Some(b"glXGetProcAddressARB\0")),
        ("libGL.so.1", Some(b"glXGetProcAddressARB\0")),
    ];

    /// WGL is what a desktop GL context on Windows is made with; EGL only
    /// where ANGLE has been put beside the program.
    #[cfg(windows)]
    const CANDIDATES: &'static [(&'static str, Option<&'static [u8]>)] = &[
        ("opengl32.dll", Some(b"wglGetProcAddress\0")),
        ("libEGL.dll", Some(b"eglGetProcAddress\0")),
    ];

    #[cfg(target_os = "macos")]
    const CANDIDATES: &'static [(&'static str, Option<&'static [u8]>)] =
        &[("/System/Library/Frameworks/OpenGL.framework/OpenGL", None)];

    fn open() -> Result<Self> {
        let mut attempts = Vec::new();

        for &(library, symbol) in Self::CANDIDATES {
            // SAFETY: loading a system GL library by its canonical name.
            // Opening a library runs its initialisers, which is expected here
            // — the process has already loaded GL by the time Slint renders.
            match unsafe { libloading::Library::new(library) } {
                Ok(lib) => {
                    let Some(symbol) = symbol else {
                        return Ok(Self {
                            library: lib,
                            get_proc_address: None,
                        });
                    };
                    // SAFETY: the symbol's signature is fixed by the EGL, GLX
                    // and WGL specifications, and all three spell it alike.
                    let found = unsafe { lib.get::<GetProcAddress>(symbol) };
                    match found {
                        Ok(symbol) => {
                            // SAFETY: the pointer stays valid as long as the
                            // library, which is kept alive in the same struct.
                            let get_proc_address = unsafe { *symbol.into_raw() };
                            return Ok(Self {
                                library: lib,
                                get_proc_address: Some(get_proc_address),
                            });
                        }
                        Err(error) => attempts.push(format!("{library}: {error}")),
                    }
                }
                Err(error) => attempts.push(format!("{library}: {error}")),
            }
        }

        Err(anyhow!(
            "no OpenGL loader available; tried {}",
            attempts.join(", ")
        ))
    }

    fn resolve(&self, name: &str) -> *mut std::ffi::c_void {
        let Ok(symbol) = std::ffi::CString::new(name) else {
            return std::ptr::null_mut();
        };
        if let Some(get_proc_address) = self.get_proc_address {
            // SAFETY: `symbol` is a valid NUL-terminated string, and the
            // function pointer came from a library this struct keeps loaded.
            let found = unsafe { get_proc_address(symbol.as_ptr()) };
            // WGL answers only for what came after GL 1.1, and says "no"
            // with 0, 1, 2, 3 or -1; the rest are plain exports of
            // opengl32.dll, looked up below like everything on macOS.
            if !matches!(found as isize, -1..=3) {
                return found;
            }
        }
        // SAFETY: looked up by name in a GL library this struct keeps loaded;
        // the address is handed to mpv, which knows each function's type.
        unsafe {
            self.library
                .get::<*mut std::ffi::c_void>(symbol.as_bytes_with_nul())
                .map_or(std::ptr::null_mut(), |found| *found)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_small_window_never_renders_below_the_source() {
        // Downscaling here would throw away detail the source actually has.
        assert_eq!(target_size((1280, 720), (640, 360)), (1280, 720));
    }

    #[test]
    fn a_large_window_renders_up_to_it() {
        // This is what makes upscaling worth running: without it the chain
        // would be squeezed back to the source resolution before display.
        assert_eq!(target_size((1280, 720), (2560, 1440)), (2560, 1440));
    }

    #[test]
    fn the_source_aspect_is_kept() {
        // A window wider than the video must not stretch it; the spare width
        // becomes letterboxing when the image is drawn.
        let (w, h) = target_size((1280, 720), (3000, 1080));
        assert_eq!(w * 720 / 1280, h);
    }

    #[test]
    fn enormous_windows_are_capped() {
        let (w, h) = target_size((1280, 720), (8000, 5000));
        assert!(w <= MAX_TARGET_SIDE && h <= MAX_TARGET_SIDE, "{w}x{h}");
    }

    #[test]
    fn dimensions_stay_even() {
        let (w, h) = target_size((1280, 720), (1919, 1079));
        assert_eq!(w % 2, 0);
        assert_eq!(h % 2, 0);
    }

    #[test]
    fn a_missing_size_falls_back_to_the_source() {
        assert_eq!(target_size((1280, 720), (0, 0)), (1280, 720));
    }

    #[test]
    fn targets_carry_the_flip_toolkits_expect() {
        let target = Target::new(3, 1920, 1080);
        assert!(target.flip_y);
        assert_eq!((target.fbo, target.width, target.height), (3, 1920, 1080));
    }

    #[test]
    fn a_zero_sized_target_is_empty() {
        assert!(Target::new(0, 0, 1080).is_empty());
        assert!(!Target::new(0, 1920, 1080).is_empty());
    }
}
