// SPDX-License-Identifier: GPL-3.0-or-later

//! Video playback on libmpv.
//!
//! The crate is split so that playback control and GPU rendering do not drag
//! each other along:
//!
//! * [`Player`] owns the mpv instance and everything that does not need a
//!   window — loading a URL, seeking, speed, tracks, properties. It runs
//!   headless, so it can be exercised without a GUI.
//! * [`Renderer`] borrows a `Player` and draws frames into a framebuffer the
//!   caller owns. It needs a live OpenGL context.
//!
//! Nothing here depends on the UI toolkit. The renderer hands back a plain
//! framebuffer id, and wrapping that in whatever the toolkit calls an image is
//! the caller's business — which keeps this crate testable and reusable.
//!
//! # Why mpv
//!
//! Because it already solves the part that matters: HLS, hardware decoding,
//! subtitle rendering, audio sync, seeking and speed control. The official
//! Anixart player ships libmpv and ffmpeg for exactly this reason; the
//! difference is that we link the upstream libraries rather than a vendored
//! copy.

use std::collections::BTreeMap;
use std::time::Duration;

use libmpv2::Mpv;

pub mod frames;
pub mod render;
pub mod shaders;
pub mod tracks;

pub use frames::{
    Enhancement, FrameGeneration, Networks, RifeInstall, RifeModel, TargetRate, TensorRt,
};
pub use render::{NativeDisplay, Renderer};
pub use shaders::{UpscaleMode, UpscalePreset, UpscaleQuality};
pub use tracks::{Track, TrackKind};

/// How long to wait on a connection or a read before trying again.
///
/// Short on purpose, and paired with retries below. Some CDN nodes drop a
/// share of new connections outright — measured from a Russian network
/// without a VPN, one node in three left the connection hanging while the
/// next attempt to the same address took 13ms. Waiting a minute on the dead
/// attempt is what made an episode sit on "loading"; giving up after ten
/// seconds and connecting again is what gets it playing. The official
/// player does the same: ten-second timeouts, five retries.
pub const DEFAULT_NETWORK_TIMEOUT_SECS: u32 = 10;

/// Retry options for ffmpeg's HTTP: reconnect on a dropped or failed
/// connection, backing off up to a few seconds between attempts.
const RECONNECT: &str =
    "reconnect=1,reconnect_streamed=1,reconnect_on_network_error=1,reconnect_delay_max=4";

/// Hardware decoders to try, in order.
///
/// Deliberately not `auto-safe`. On NVIDIA that reaches VAAPI through the
/// `nvidia-vaapi-driver` shim, whose interop with the render API produced torn
/// bands across the picture on an RTX 4070 Ti SUPER; `nvdec` is NVIDIA's own
/// path and renders cleanly. mpv skips entries the machine does not have, so
/// listing `nvdec` first costs nothing on AMD or Intel, where `vaapi` is the
/// right answer and comes next. The `-copy` variants close each list: they
/// decode on the GPU and copy frames back, which works when the zero-copy
/// interop does not come up, and is still far lighter than software.
#[cfg(all(unix, not(target_os = "macos")))]
pub const DEFAULT_HWDEC: &str = "nvdec,vaapi,vulkan,nvdec-copy,vaapi-copy";

/// Direct3D 11 is the decoder every Windows GPU has; NVIDIA's own after it.
#[cfg(windows)]
pub const DEFAULT_HWDEC: &str = "d3d11va,nvdec,d3d11va-copy,dxva2-copy";

#[cfg(target_os = "macos")]
pub const DEFAULT_HWDEC: &str = "videotoolbox,videotoolbox-copy";

/// Colour adjustments, each from −100 to 100, 0 leaving the source alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PictureAdjust {
    pub brightness: i8,
    pub contrast: i8,
    pub saturation: i8,
    pub gamma: i8,
}

impl PictureAdjust {
    /// The source as it is.
    pub const NATURAL: Self = Self::new(0, 0, 0, 0);
    /// Lifted shadows, for dark scenes and dim screens.
    pub const BRIGHTER: Self = Self::new(6, 4, 0, 8);
    /// More colour, for washed-out sources.
    pub const VIVID: Self = Self::new(0, 6, 22, 0);
    /// Less contrast and colour, for long evenings.
    pub const SOFT: Self = Self::new(0, -6, -12, 0);
    /// Slightly darker and deeper, for a dark room.
    pub const DARK_ROOM: Self = Self::new(-4, 2, 0, -6);

    /// Every preset, in the order a menu lists them.
    pub const PRESETS: [Self; 5] = [
        Self::NATURAL,
        Self::BRIGHTER,
        Self::VIVID,
        Self::SOFT,
        Self::DARK_ROOM,
    ];

    #[must_use]
    pub const fn new(brightness: i8, contrast: i8, saturation: i8, gamma: i8) -> Self {
        Self {
            brightness,
            contrast,
            saturation,
            gamma,
        }
    }
}

/// Pixel format for mpv's intermediate framebuffers.
///
/// mpv would choose `rgba16f`, and left to itself it renders a band of
/// corrupted scanlines across every frame when it is drawing into a caller's
/// framebuffer through the render API. It was reproduced on an RTX 4070 Ti
/// SUPER against an OpenGL **ES** 3.2 context, and confirmed by capturing the
/// texture straight after mpv wrote it: the band is in mpv's output, not in
/// the stream (ffmpeg decodes the same stream cleanly) and not in how the
/// picture is later sampled.
///
/// The cause is the float path: GLES only allows rendering into
/// floating-point framebuffers when `EXT_color_buffer_float` is present, and
/// that condition is evidently not met in a context mpv did not create itself.
///
/// `rgba16` is the fix rather than `rgba8` because both render correctly and
/// 16 bits per channel keeps the headroom mpv's multi-pass processing wants,
/// so no banding is traded away for the repair.
pub const DEFAULT_FBO_FORMAT: &str = "rgba16";

/// How far "skip opening" jumps when the source publishes no boundaries.
///
/// Television anime openings are a fixed 90 seconds almost without exception,
/// so a blind jump is genuinely useful rather than a guess. It is set a little
/// short deliberately: landing a few seconds early is a shrug, landing after
/// the first line of dialogue is not.
///
/// Only AniLibria publishes real boundaries today; everything else relies on
/// this.
pub const DEFAULT_OPENING_SECS: u64 = 85;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("mpv: {0}")]
    Mpv(#[from] libmpv2::Error),
    #[error("{path}: {source}")]
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

/// Video output backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VideoOutput {
    /// Frames are drawn through [`Renderer`] into a caller-owned framebuffer.
    /// This is what the GUI uses.
    #[default]
    Embedded,
    /// Decode without displaying. Used by tests and by the probe, where the
    /// point is to confirm a stream opens and decodes.
    Headless,
}

impl VideoOutput {
    fn mpv_value(self) -> &'static str {
        match self {
            // "libmpv" is the vo that hands frames to the render API.
            Self::Embedded => "libmpv",
            Self::Headless => "null",
        }
    }
}

/// Settings applied when the player is created.
#[derive(Debug, Clone)]
pub struct PlayerConfig {
    pub video_output: VideoOutput,
    /// mpv's `hwdec` value, or `"no"` to decode in software.
    ///
    /// See [`DEFAULT_HWDEC`] for why this is a priority list rather than
    /// `auto-safe`.
    pub hwdec: std::borrow::Cow<'static, str>,
    /// Smooths judder when the content's frame rate does not divide into the
    /// display's. This is temporal resampling, not motion interpolation: it
    /// does not invent frames.
    pub interpolation: bool,
    pub upscale: UpscalePreset,
    pub network_timeout_secs: u32,
    /// Seconds of stream to buffer ahead.
    pub cache_secs: u32,
    /// Where to keep the Anime4K shaders.
    ///
    /// `None` uses [`shaders::default_dir`]. The shaders are embedded in the
    /// binary and written there on startup, so this exists for packagers who
    /// want them somewhere specific — not as a setup step.
    pub shader_dir: Option<std::path::PathBuf>,
    /// Let the decoder write frames straight into GPU-mapped buffers.
    ///
    /// mpv calls this direct rendering. It saves a copy, but it relies on the
    /// GL context behaving as mpv expects, and inside a foreign context —
    /// one owned by a UI toolkit — the synchronisation can break, leaving
    /// bands of stale pixels in otherwise correct frames.
    pub direct_rendering: bool,
    /// Bit depth mpv should dither its output to, or `None` to let it decide.
    ///
    /// mpv renders internally at 16-bit float and dithers down to the target.
    /// Through the render API it cannot see the real target depth, so its
    /// automatic choice is a guess.
    pub dither_depth: Option<u8>,
    /// Pixel format for mpv's own intermediate framebuffers.
    ///
    /// See [`DEFAULT_FBO_FORMAT`] for why this is set rather than left alone.
    pub fbo_format: Option<std::borrow::Cow<'static, str>>,
    /// Bypass almost all of mpv's processing chain.
    ///
    /// A diagnostic, not a feature: it disables scaling, dithering and colour
    /// management, so a defect that survives it is not coming from those.
    pub dumb_mode: bool,
    /// Let mpv write its own diagnostics to stderr.
    ///
    /// Off by default: mpv's log belongs in the host application's log, not
    /// scribbled over its stdout. Worth turning on when playback misbehaves,
    /// because mpv explains stalls far better than any property poll can.
    pub verbose_log: bool,
}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self {
            video_output: VideoOutput::default(),
            hwdec: std::borrow::Cow::Borrowed(DEFAULT_HWDEC),
            interpolation: false,
            upscale: UpscalePreset::OFF,
            network_timeout_secs: DEFAULT_NETWORK_TIMEOUT_SECS,
            cache_secs: 30,
            shader_dir: None,
            fbo_format: Some(std::borrow::Cow::Borrowed(DEFAULT_FBO_FORMAT)),
            dumb_mode: false,
            direct_rendering: true,
            dither_depth: None,
            verbose_log: false,
        }
    }
}

impl PlayerConfig {
    /// Configuration for decoding without a window.
    #[must_use]
    pub fn headless() -> Self {
        Self {
            video_output: VideoOutput::Headless,
            ..Self::default()
        }
    }
}

/// What the player is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    /// Nothing loaded.
    Idle,
    /// Loading, with no picture yet.
    Loading,
    /// Stalled waiting for data. Distinct from [`Self::Paused`]: the viewer
    /// did not ask for this, and the UI should say so.
    Buffering,
    Playing,
    Paused,
    /// Reached the end of the file.
    Ended,
}

impl PlaybackState {
    /// Whether the picture is advancing.
    #[must_use]
    pub fn is_active(self) -> bool {
        matches!(self, Self::Playing)
    }

    /// Whether the viewer is waiting on something rather than on their own
    /// decision — worth a spinner, where [`Self::Paused`] is not.
    #[must_use]
    pub fn is_waiting(self) -> bool {
        matches!(self, Self::Loading | Self::Buffering)
    }
}

/// What to play, and how it must be requested.
#[derive(Debug, Clone, Default)]
pub struct MediaSource {
    pub url: String,
    /// Headers the host requires. Without the right `Referer` the manifest
    /// opens but its segments come back 403.
    pub headers: BTreeMap<String, String>,
    /// Where to resume from.
    pub start_at: Option<Duration>,
}

impl MediaSource {
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn headers<I, K, V>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        self.headers
            .extend(headers.into_iter().map(|(k, v)| (k.into(), v.into())));
        self
    }

    #[must_use]
    pub fn start_at(mut self, at: Duration) -> Self {
        self.start_at = Some(at);
        self
    }
}

pub struct Player {
    mpv: Mpv,
    shader_dir: Option<std::path::PathBuf>,
    /// The decoders asked for, kept so that frame generation can swap in
    /// their copy-back forms and put them back afterwards.
    hwdec: std::sync::Mutex<String>,
    generating: std::sync::atomic::AtomicBool,
    /// The display's refresh rate as last told, for scripts that aim at it.
    display_fps: std::sync::Mutex<Option<f64>>,
}

impl Player {
    pub fn new(config: &PlayerConfig) -> Result<Self> {
        // Options that must be set before initialisation go through the
        // initializer; the rest are properties and can change later.
        let mpv = Mpv::with_initializer(|init| {
            init.set_property("vo", config.video_output.mpv_value())?;
            init.set_property("hwdec", config.hwdec.as_ref())?;
            // Terminal output belongs to the host application's log, not to
            // mpv's own stdout scribbling.
            init.set_property("terminal", config.verbose_log)?;
            if config.verbose_log {
                init.set_property("msg-level", "all=v")?;
            }
            init.set_property("input-default-bindings", false)?;
            // Likewise the config-file driven profile hooks: this is a library
            // embedded in an application, not a user's mpv install.
            init.set_property("config", false)?;
            // The rest belong to mpv's Lua scripting, which a libmpv built
            // without it — the one with the VapourSynth filter for frame
            // generation, say — does not have at all. Off is what is wanted
            // either way, so an option that does not exist is as good.
            //
            // Every URL reaching mpv has already been resolved by the
            // extractors, so mpv's youtube-dl hook has nothing to add. Leaving
            // it on spawns a subprocess and waits on its timeouts before
            // playback can even start.
            for option in ["osc", "ytdl", "load-scripts"] {
                match init.set_property(option, false) {
                    Ok(()) => {}
                    Err(libmpv2::Error::Raw(code))
                        if code == libmpv2::mpv_error::OptionNotFound
                            || code == libmpv2::mpv_error::PropertyNotFound => {}
                    Err(error) => return Err(error),
                }
            }
            Ok(())
        })?;

        // Materialise the embedded shaders before any preset is applied, so
        // upscaling cannot fail for want of files the binary already carries.
        let shader_dir = config
            .shader_dir
            .clone()
            .unwrap_or_else(shaders::default_dir);
        if let Err(error) = shaders::install_to(&shader_dir) {
            tracing::warn!(%error, dir = %shader_dir.display(), "could not install the shaders; upscaling will be unavailable");
        }

        let player = Self {
            mpv,
            shader_dir: Some(shader_dir),
            hwdec: std::sync::Mutex::new(config.hwdec.to_string()),
            generating: std::sync::atomic::AtomicBool::new(false),
            display_fps: std::sync::Mutex::new(None),
        };

        player.set_network_timeout(config.network_timeout_secs)?;
        // A connection that stalls is tried again rather than ending the
        // episode: the playlist through mpv's own stream, and each HLS
        // segment through the demuxer, which otherwise gives a segment one
        // attempt. Measured on a node that drops connections: without these,
        // four starts in five failed; with them, five in five played.
        player.mpv.set_property("stream-lavf-o", RECONNECT)?;
        player.mpv.set_property(
            "demuxer-lavf-o",
            format!("seg_max_retry=5,{RECONNECT}").as_str(),
        )?;
        player.mpv.set_property("cache", "yes")?;
        player
            .mpv
            .set_property("cache-secs", i64::from(config.cache_secs))?;
        // Keep the player alive at end of file so the caller decides what
        // happens next rather than mpv shutting itself down.
        player.mpv.set_property("keep-open", "yes")?;
        player.mpv.set_property("idle", "yes")?;

        if let Some(format) = &config.fbo_format {
            player.mpv.set_property("fbo-format", format.as_ref())?;
        }
        if config.dumb_mode {
            player.mpv.set_property("gpu-dumb-mode", true)?;
        }
        if !config.direct_rendering {
            player.mpv.set_property("vd-lavc-dr", false)?;
        }
        // Only when asked. mpv's own default is not to dither, and its "auto"
        // guesses the target's bit depth - which it cannot actually see when
        // rendering through the render API into a caller's framebuffer.
        if let Some(depth) = config.dither_depth {
            player.mpv.set_property("dither-depth", i64::from(depth))?;
        }

        player.set_interpolation(config.interpolation)?;
        player.set_upscale(config.upscale)?;

        Ok(player)
    }

    /// Escape hatch for properties this API does not wrap.
    #[must_use]
    pub fn mpv(&self) -> &Mpv {
        &self.mpv
    }

    // ---- loading ----------------------------------------------------------

    /// Loads a stream, replacing whatever is playing.
    pub fn open(&self, source: &MediaSource) -> Result<()> {
        self.apply_headers(&source.headers)?;

        match source.start_at {
            // `start` is applied by loadfile, so setting it first avoids a
            // visible jump from 0 to the resume point.
            Some(at) => self.mpv.set_property("start", at.as_secs_f64())?,
            None => self.mpv.set_property("start", "none")?,
        }

        self.mpv.command("loadfile", &[&source.url, "replace"])?;
        Ok(())
    }

    pub fn stop(&self) -> Result<()> {
        self.mpv.command("stop", &[])?;
        Ok(())
    }

    /// Splits headers the way mpv wants them.
    ///
    /// `user-agent` is its own option: mpv sends a User-Agent of its own that
    /// a header field does not override. Everything else goes into
    /// `http-header-fields`, which is a comma-separated list — so a header
    /// whose *value* contains a comma cannot be expressed there. In practice
    /// only User-Agent does, and that one has its own home.
    fn apply_headers(&self, headers: &BTreeMap<String, String>) -> Result<()> {
        let mut fields = Vec::new();

        for (name, value) in headers {
            if name.eq_ignore_ascii_case("user-agent") {
                self.mpv.set_property("user-agent", value.as_str())?;
            } else if value.contains(',') {
                tracing::warn!(
                    header = %name,
                    "dropping a header whose value contains a comma: mpv's \
                     http-header-fields cannot express it"
                );
            } else {
                fields.push(format!("{name}: {value}"));
            }
        }

        self.mpv
            .set_property("http-header-fields", fields.join(",").as_str())?;
        Ok(())
    }

    // ---- transport --------------------------------------------------------

    pub fn set_paused(&self, paused: bool) -> Result<()> {
        self.mpv.set_property("pause", paused)?;
        Ok(())
    }

    pub fn is_paused(&self) -> Result<bool> {
        Ok(self.mpv.get_property::<bool>("pause")?)
    }

    pub fn toggle_pause(&self) -> Result<()> {
        self.set_paused(!self.is_paused()?)
    }

    /// Jumps to an absolute position.
    pub fn seek_to(&self, position: Duration) -> Result<()> {
        self.mpv
            .command("seek", &[&position.as_secs_f64().to_string(), "absolute"])?;
        Ok(())
    }

    /// Jumps forward, or backward for a negative value.
    pub fn seek_by(&self, delta_secs: f64) -> Result<()> {
        self.mpv
            .command("seek", &[&delta_secs.to_string(), "relative"])?;
        Ok(())
    }

    /// Skips the opening.
    ///
    /// `ends_at` is where the opening finishes, when the source says so — then
    /// the jump is exact. Without it the seek is forward by
    /// [`DEFAULT_OPENING_SECS`].
    ///
    /// Never seeks backwards: a viewer who is already past the opening and
    /// presses the button by reflex should not be thrown back into it.
    pub fn skip_opening(&self, ends_at: Option<Duration>) -> Result<()> {
        let position = self.position().unwrap_or_default();

        match ends_at {
            Some(end) if end > position => self.seek_to(end),
            // Already past a known opening: nothing to skip.
            Some(_) => Ok(()),
            None => self.seek_by(DEFAULT_OPENING_SECS as f64),
        }
    }

    /// Playback position, or `None` before anything has loaded.
    pub fn position(&self) -> Option<Duration> {
        self.duration_property("time-pos")
    }

    pub fn duration(&self) -> Option<Duration> {
        self.duration_property("duration")
    }

    fn duration_property(&self, name: &str) -> Option<Duration> {
        self.mpv
            .get_property::<f64>(name)
            .ok()
            .filter(|secs| secs.is_finite() && *secs >= 0.0)
            .map(Duration::from_secs_f64)
    }

    /// Playback rate. 1.0 is normal; mpv keeps the audio pitch corrected.
    pub fn set_speed(&self, speed: f64) -> Result<()> {
        self.mpv.set_property("speed", speed.clamp(0.25, 4.0))?;
        Ok(())
    }

    pub fn speed(&self) -> Result<f64> {
        Ok(self.mpv.get_property::<f64>("speed")?)
    }

    /// Volume as a percentage; above 100 amplifies.
    pub fn set_volume(&self, percent: i64) -> Result<()> {
        self.mpv.set_property("volume", percent.clamp(0, 150))?;
        Ok(())
    }

    /// Loudness as a percentage, where 100 is unattenuated.
    #[must_use]
    pub fn volume(&self) -> i64 {
        self.mpv.get_property::<i64>("volume").unwrap_or(100)
    }

    #[must_use]
    pub fn is_muted(&self) -> bool {
        self.mpv.get_property::<bool>("mute").unwrap_or(false)
    }

    /// Silences or unsilences, returning the state it settled on.
    pub fn toggle_muted(&self) -> Result<bool> {
        let next = !self.is_muted();
        self.set_muted(next)?;
        Ok(next)
    }

    /// Moves one frame forward or back.
    ///
    /// mpv pauses when stepping, which is what makes the step visible at all —
    /// so this is a pause as well as a step, exactly as it is everywhere else
    /// that offers it.
    pub fn step_frame(&self, forward: bool) -> Result<()> {
        let command = if forward {
            "frame-step"
        } else {
            "frame-back-step"
        };
        self.mpv.command(command, &[])?;
        Ok(())
    }

    /// Saves the frame on screen as a picture at `path`, its format taken
    /// from the extension: with the subtitles drawn on it as they are on
    /// screen, or the bare video.
    pub fn screenshot(&self, path: &std::path::Path, with_subtitles: bool) -> Result<()> {
        let path = path.to_string_lossy();
        let what = if with_subtitles { "subtitles" } else { "video" };
        self.mpv.command("screenshot-to-file", &[&path, what])?;
        Ok(())
    }

    /// Plays the file over and over, or once.
    ///
    /// A looping file never reaches its end, so whatever goes on to the next
    /// episode at the end simply never fires.
    pub fn set_loop(&self, on: bool) -> Result<()> {
        self.mpv
            .set_property("loop-file", if on { "inf" } else { "no" })?;
        Ok(())
    }

    pub fn set_muted(&self, muted: bool) -> Result<()> {
        self.mpv.set_property("mute", muted)?;
        Ok(())
    }

    // ---- tracks -----------------------------------------------------------

    /// Every stream in the current file.
    ///
    /// Empty until a file is loaded. Anime releases routinely carry several
    /// subtitle tracks, so a menu needs the whole list rather than just the
    /// selected id — see [`Track::label`] for turning one into menu text.
    #[must_use]
    pub fn tracks(&self) -> Vec<Track> {
        tracks::list(&self.mpv)
    }

    /// Tracks of one kind, in the order the file lists them.
    #[must_use]
    pub fn tracks_of(&self, kind: TrackKind) -> Vec<Track> {
        self.tracks()
            .into_iter()
            .filter(|track| track.kind == kind)
            .collect()
    }

    /// How many streams the file has, at the cost of one property read.
    ///
    /// A menu only has to be rebuilt when this changes, and rebuilding it means
    /// reading seven properties per track — worth avoiding on a status poll.
    #[must_use]
    pub fn track_count(&self) -> usize {
        self.mpv
            .get_property::<i64>("track-list/count")
            .map(|count| usize::try_from(count).unwrap_or(0))
            .unwrap_or(0)
    }

    /// Id of the selected track of a kind, read straight from the selector
    /// rather than by walking the list.
    ///
    /// `None` means that kind is switched off, which for subtitles is the
    /// usual state.
    #[must_use]
    pub fn current_track(&self, kind: TrackKind) -> Option<i64> {
        self.mpv.get_property::<i64>(kind.selector()).ok()
    }

    /// The selected track of a kind, if any.
    #[must_use]
    pub fn selected_track(&self, kind: TrackKind) -> Option<Track> {
        self.tracks_of(kind)
            .into_iter()
            .find(|track| track.selected)
    }

    /// Selects a track, or turns that kind off with `None`.
    pub fn select_track(&self, kind: TrackKind, id: Option<i64>) -> Result<()> {
        match id {
            Some(id) => self.mpv.set_property(kind.selector(), id)?,
            None => self.mpv.set_property(kind.selector(), "no")?,
        }
        Ok(())
    }

    /// Selects a subtitle track by id, or disables subtitles with `None`.
    pub fn set_subtitle_track(&self, id: Option<i64>) -> Result<()> {
        self.select_track(TrackKind::Subtitle, id)
    }

    pub fn set_audio_track(&self, id: Option<i64>) -> Result<()> {
        self.select_track(TrackKind::Audio, id)
    }

    /// Adds an external subtitle file or URL.
    pub fn add_subtitle(&self, url: &str) -> Result<()> {
        self.mpv.command("sub-add", &[url, "auto"])?;
        Ok(())
    }

    /// Adds a subtitle file someone picked, and shows it.
    pub fn load_subtitle_file(&self, path: &std::path::Path) -> Result<()> {
        self.mpv
            .command("sub-add", &[&path.to_string_lossy(), "select"])?;
        Ok(())
    }

    /// Adds an audio file someone picked — another dub for the same
    /// episode — and plays it instead of the stream's own.
    pub fn load_audio_file(&self, path: &std::path::Path) -> Result<()> {
        self.mpv
            .command("audio-add", &[&path.to_string_lossy(), "select"])?;
        Ok(())
    }

    // ---- sound and subtitles ---------------------------------------------

    /// Shifts the sound against the picture, in seconds; positive is later.
    pub fn set_audio_delay(&self, seconds: f64) -> Result<()> {
        self.mpv.set_property("audio-delay", seconds)?;
        Ok(())
    }

    /// Shifts the subtitles against the picture, in seconds; positive is
    /// later.
    pub fn set_subtitle_delay(&self, seconds: f64) -> Result<()> {
        self.mpv.set_property("sub-delay", seconds)?;
        Ok(())
    }

    /// Evens out loudness, so whispered lines and a loud opening sit at
    /// about the same level. ffmpeg's `dynaudnorm`, tuned to react within a
    /// few seconds rather than over the whole episode.
    pub fn set_loudness_normalization(&self, on: bool) -> Result<()> {
        let filter = if on {
            "lavfi=[dynaudnorm=f=250:g=15:p=0.9]"
        } else {
            ""
        };
        self.mpv.set_property("af", filter)?;
        Ok(())
    }

    /// Scales subtitle text; 1.0 is the file's own size.
    pub fn set_subtitle_scale(&self, scale: f64) -> Result<()> {
        self.mpv.set_property("sub-scale", scale)?;
        Ok(())
    }

    /// Whether the player's own subtitle style replaces the styles an ASS
    /// file brings. Off, ASS subtitles keep their look and only scale.
    pub fn set_ass_override(&self, on: bool) -> Result<()> {
        self.mpv
            .set_property("sub-ass-override", if on { "force" } else { "scale" })?;
        Ok(())
    }

    /// Where subtitles look for fonts they name, on top of the system's:
    /// fan subtitles often name fonts nobody has installed. `None` goes back
    /// to mpv's own folder.
    pub fn set_subtitle_fonts_dir(&self, dir: Option<&std::path::Path>) -> Result<()> {
        let dir = dir.map_or_else(String::new, |dir| dir.to_string_lossy().into_owned());
        self.mpv.set_property("sub-fonts-dir", dir.as_str())?;
        // Fonts are looked up when a track is loaded, so the one showing is
        // loaded again to pick the new folder up. Without one, nothing to do.
        let _ = self.mpv.command("sub-reload", &[]);
        Ok(())
    }

    // ---- video processing -------------------------------------------------

    /// Turns temporal resampling on or off.
    ///
    /// This smooths judder when the content's frame rate does not divide
    /// evenly into the display's refresh rate. It is *not* motion
    /// interpolation and does not synthesise frames — that would need
    /// something like RIFE through VapourSynth, which is a separate concern.
    pub fn set_interpolation(&self, enabled: bool) -> Result<()> {
        self.mpv.set_property("interpolation", enabled)?;
        if enabled {
            // Interpolation only does anything when the clock is resampled to
            // the display, and oversample is the cheap, judder-oriented filter.
            self.mpv.set_property("video-sync", "display-resample")?;
            self.mpv.set_property("tscale", "oversample")?;
        } else {
            self.mpv.set_property("video-sync", "audio")?;
        }
        Ok(())
    }

    /// Applies an upscaling preset, or clears shaders with
    /// [`UpscalePreset::OFF`].
    pub fn set_upscale(&self, preset: UpscalePreset) -> Result<()> {
        let Some(chain) = preset.shader_chain() else {
            self.mpv.set_property("glsl-shaders", "")?;
            return Ok(());
        };

        // Always present: the shaders are embedded and written out when the
        // player is created, so a preset cannot fail for want of files.
        let Some(dir) = self.shader_dir.as_deref() else {
            tracing::warn!(
                preset = %preset.name(),
                "no shader directory; upscaling stays off"
            );
            self.mpv.set_property("glsl-shaders", "")?;
            return Ok(());
        };

        // mpv separates path-list entries with ':' on Unix and ';' on
        // Windows, where ':' follows every drive letter. A shader path
        // containing the separator would be ambiguous, which is why the
        // directory is configuration rather than something guessed from the
        // environment.
        let separator = if cfg!(windows) { ";" } else { ":" };
        let paths: Vec<String> = chain
            .iter()
            .map(|file| dir.join(file).to_string_lossy().into_owned())
            .collect();

        self.mpv
            .set_property("glsl-shaders", paths.join(separator).as_str())?;
        tracing::info!(preset = %preset.name(), "upscaling");
        Ok(())
    }

    /// Sets the picture's brightness, contrast, saturation and gamma, each
    /// from −100 to 100 with 0 as the source has it.
    pub fn set_picture(&self, adjust: PictureAdjust) -> Result<()> {
        self.mpv
            .set_property("brightness", i64::from(adjust.brightness))?;
        self.mpv
            .set_property("contrast", i64::from(adjust.contrast))?;
        self.mpv
            .set_property("saturation", i64::from(adjust.saturation))?;
        self.mpv.set_property("gamma", i64::from(adjust.gamma))?;
        Ok(())
    }

    /// Switches hardware decoding while playing.
    ///
    /// mpv accepts this at runtime and reconfigures on the next frame, so a
    /// viewer troubleshooting a bad picture does not have to restart the
    /// episode. Takes any mpv `hwdec` value, including `"no"`.
    pub fn set_hwdec(&self, value: &str) -> Result<()> {
        if let Ok(mut kept) = self.hwdec.lock() {
            value.clone_into(&mut kept);
        }
        self.apply_hwdec()
    }

    /// The decoders asked for, in copy-back form while frames are generated.
    fn apply_hwdec(&self) -> Result<()> {
        let asked = self
            .hwdec
            .lock()
            .map(|kept| kept.clone())
            .unwrap_or_else(|_| DEFAULT_HWDEC.to_owned());
        let value = if self.generating.load(std::sync::atomic::Ordering::Relaxed) {
            frames::copy_back(&asked)
        } else {
            asked
        };
        self.mpv.set_property("hwdec", value.as_str())?;
        Ok(())
    }

    /// Runs neural filters on the picture — RIFE frame generation, neural
    /// upscaling — on the engine given, or stops them with `None`.
    ///
    /// The script is written fresh each time, and any note a previous one
    /// left is cleared, so [`frames::last_error`] only ever speaks of this
    /// attempt. A failure does not stop playback: mpv drops the filter and
    /// plays the episode as it is, which [`Self::output_fps`] shows.
    pub fn set_enhancement(
        &self,
        enhancement: Option<(frames::Networks<'_>, frames::Enhancement)>,
    ) -> Result<()> {
        use std::sync::atomic::Ordering;

        // Upscaling alone on Vulkan has nothing to run: no filter at all.
        let enhancement = enhancement.filter(|(networks, enhancement)| {
            enhancement.frames.is_some()
                || (enhancement.upscale && matches!(networks, frames::Networks::TensorRt(_)))
        });
        let Some((networks, enhancement)) = enhancement else {
            self.mpv.set_property("vf", "")?;
            self.generating.store(false, Ordering::Relaxed);
            return self.apply_hwdec();
        };

        let dir = frames::work_dir();
        let script = dir.join("rife.vpy");
        let error_file = dir.join("rife-error.txt");
        let _ = std::fs::remove_file(&error_file);
        let display_fps = self.display_fps.lock().ok().and_then(|kept| *kept);
        std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::create_dir_all(frames::engines_dir()))
            .and_then(|()| {
                std::fs::write(
                    &script,
                    frames::script(networks, enhancement, display_fps, &error_file),
                )
            })
            .map_err(|source| Error::Io {
                path: script.clone(),
                source,
            })?;

        self.generating.store(true, Ordering::Relaxed);
        self.apply_hwdec()?;
        let filter = format!(
            "vapoursynth=file={}:buffered-frames=8:concurrent-frames=4",
            frames::mpv_quoted(&script.to_string_lossy())
        );
        self.mpv.set_property("vf", filter.as_str())?;
        let engine = match networks {
            frames::Networks::Vulkan(_) => "vulkan",
            frames::Networks::TensorRt(_) => "tensorrt",
        };
        tracing::info!(engine, ?enhancement, script = %script.display(), "neural filters on");
        Ok(())
    }

    /// Tells mpv the display's refresh rate, or lets it guess again with
    /// `None`.
    ///
    /// Rendering through the render API, mpv has no window to ask, and
    /// assumes no rate at all: display-synced interpolation then has nothing
    /// to resample to, and a VapourSynth script's `display_fps` reads 0.
    ///
    /// Frame generation at the screen's rate reads it from here too: mpv's
    /// filters ask the video output directly and get no answer, override or
    /// not, so the next script written carries the rate itself.
    pub fn set_display_fps(&self, fps: Option<f64>) -> Result<()> {
        if let Ok(mut kept) = self.display_fps.lock() {
            *kept = fps;
        }
        self.mpv
            .set_property("display-fps-override", fps.unwrap_or(0.0))?;
        Ok(())
    }

    /// Frames a second leaving the filters: the source's rate, or the
    /// generated one when frame generation is working.
    #[must_use]
    pub fn output_fps(&self) -> Option<f64> {
        self.mpv
            .get_property::<f64>("estimated-vf-fps")
            .ok()
            .filter(|fps| *fps > 0.0)
    }

    /// Frames dropped so far in this file for being late: by the decoder and
    /// by the output together. A count that keeps rising during plain
    /// playback means the machine is not keeping up.
    #[must_use]
    pub fn dropped_frames(&self) -> u64 {
        ["frame-drop-count", "vo-drop-frame-count"]
            .iter()
            .filter_map(|name| self.mpv.get_property::<i64>(name).ok())
            .map(|count| u64::try_from(count).unwrap_or(0))
            .sum()
    }

    /// The source's own frame rate.
    #[must_use]
    pub fn source_fps(&self) -> Option<f64> {
        self.mpv
            .get_property::<f64>("container-fps")
            .ok()
            .filter(|fps| *fps > 0.0)
    }

    pub fn set_network_timeout(&self, secs: u32) -> Result<()> {
        self.mpv.set_property("network-timeout", i64::from(secs))?;
        Ok(())
    }

    // ---- state ------------------------------------------------------------

    /// Dimensions of the decoded video, once known.
    pub fn video_size(&self) -> Option<(u32, u32)> {
        let width = self.mpv.get_property::<i64>("width").ok()?;
        let height = self.mpv.get_property::<i64>("height").ok()?;
        (width > 0 && height > 0).then_some((width as u32, height as u32))
    }

    /// The picture's size after the filters: larger than [`Self::video_size`]
    /// when a network upscaled it, smaller when frame generation brought it
    /// down to run.
    #[must_use]
    pub fn filtered_size(&self) -> Option<(u32, u32)> {
        let width = self.mpv.get_property::<i64>("video-out-params/w").ok()?;
        let height = self.mpv.get_property::<i64>("video-out-params/h").ok()?;
        (width > 0 && height > 0).then_some((width as u32, height as u32))
    }

    /// The hardware decoder mpv actually engaged, if any.
    ///
    /// `hwdec` is a request; this is the answer. Worth logging, because a
    /// request that silently fell back to software looks identical from the
    /// outside until the picture misbehaves.
    pub fn active_hwdec(&self) -> Option<String> {
        self.mpv
            .get_property::<String>("hwdec-current")
            .ok()
            .filter(|value| !value.is_empty() && value != "no")
    }

    /// What the player is doing, as a UI needs to describe it.
    ///
    /// Assembled from properties rather than delivered as events, because a
    /// UI paints on its own schedule and wants the current truth when it does,
    /// not a backlog of transitions it has to fold together itself.
    #[must_use]
    pub fn state(&self) -> PlaybackState {
        if !self.is_playing() {
            return PlaybackState::Idle;
        }
        if self.flag("eof-reached") {
            return PlaybackState::Ended;
        }
        // `paused-for-cache` is mpv stalling on data, as distinct from the
        // viewer having pressed pause. Conflating them would show "paused"
        // while the network is the problem.
        if self.flag("paused-for-cache") {
            return PlaybackState::Buffering;
        }
        if self.is_paused().unwrap_or(false) {
            return PlaybackState::Paused;
        }
        if self.video_size().is_none() {
            return PlaybackState::Loading;
        }
        PlaybackState::Playing
    }

    /// How much is buffered ahead of the current position.
    ///
    /// What a progress bar shades in ahead of the playhead, and the honest
    /// answer to "why did it stop".
    #[must_use]
    pub fn buffered_ahead(&self) -> Option<Duration> {
        self.mpv
            .get_property::<f64>("demuxer-cache-duration")
            .ok()
            .filter(|secs| secs.is_finite() && *secs >= 0.0)
            .map(Duration::from_secs_f64)
    }

    /// Position the buffer currently reaches.
    #[must_use]
    pub fn buffered_until(&self) -> Option<Duration> {
        Some(self.position()? + self.buffered_ahead()?)
    }

    fn flag(&self, name: &str) -> bool {
        self.mpv.get_property::<bool>(name).unwrap_or(false)
    }

    /// Whether a file is loaded and decoding.
    pub fn is_playing(&self) -> bool {
        self.mpv
            .get_property::<bool>("idle-active")
            .map(|idle| !idle)
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_output_maps_to_mpv_names() {
        assert_eq!(VideoOutput::Embedded.mpv_value(), "libmpv");
        assert_eq!(VideoOutput::Headless.mpv_value(), "null");
    }

    #[test]
    fn media_source_builds_up() {
        let source = MediaSource::new("https://host/a.m3u8")
            .header("Referer", "https://host/")
            .start_at(Duration::from_secs(90));

        assert_eq!(source.url, "https://host/a.m3u8");
        assert_eq!(source.headers.get("Referer").unwrap(), "https://host/");
        assert_eq!(source.start_at, Some(Duration::from_secs(90)));
    }

    #[test]
    fn headless_config_does_not_ask_for_a_window() {
        assert_eq!(PlayerConfig::headless().video_output, VideoOutput::Headless);
    }

    #[test]
    fn waiting_is_distinct_from_paused() {
        // A spinner belongs on one of these and not the other.
        assert!(PlaybackState::Buffering.is_waiting());
        assert!(PlaybackState::Loading.is_waiting());
        assert!(!PlaybackState::Paused.is_waiting());
        assert!(!PlaybackState::Playing.is_waiting());
    }

    #[test]
    fn only_playing_counts_as_active() {
        for state in [
            PlaybackState::Idle,
            PlaybackState::Loading,
            PlaybackState::Buffering,
            PlaybackState::Paused,
            PlaybackState::Ended,
        ] {
            assert!(!state.is_active(), "{state:?} should not be active");
        }
        assert!(PlaybackState::Playing.is_active());
    }

    #[test]
    fn defaults_are_conservative() {
        let config = PlayerConfig::default();
        assert_eq!(config.hwdec, DEFAULT_HWDEC);
        // Interpolation costs GPU time and is a matter of taste, so it is
        // opt-in rather than on by default.
        assert!(!config.interpolation);
        assert_eq!(config.upscale, UpscalePreset::OFF);
        assert_eq!(config.network_timeout_secs, DEFAULT_NETWORK_TIMEOUT_SECS);
    }
}
