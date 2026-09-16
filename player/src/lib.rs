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

pub mod render;
pub mod shaders;

pub use render::Renderer;
pub use shaders::UpscalePreset;

/// How long to wait on a CDN node before giving up.
///
/// Generous on purpose. The nodes these streams redirect to are erratic: one
/// answered a request in 14ms and then took 19.3s to accept the next
/// connection. ffmpeg's default is shorter, so a working stream fails outright
/// and reads like a broken extractor.
pub const DEFAULT_NETWORK_TIMEOUT_SECS: u32 = 60;

/// Hardware decoders to try, in order.
///
/// Deliberately not `auto-safe`. On NVIDIA that reaches VAAPI through the
/// `nvidia-vaapi-driver` shim, whose interop with the render API produced torn
/// bands across the picture on an RTX 4070 Ti SUPER; `nvdec` is NVIDIA's own
/// path and renders cleanly. mpv skips entries the machine does not have, so
/// listing `nvdec` first costs nothing on AMD or Intel, where `vaapi` is the
/// right answer and comes next.
pub const DEFAULT_HWDEC: &str = "nvdec,vaapi,vulkan";

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

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("mpv: {0}")]
    Mpv(#[from] libmpv2::Error),

    #[error("the shader directory is not configured, so {preset} cannot be applied")]
    ShadersUnavailable { preset: &'static str },
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
    /// Directory holding the Anime4K shaders. Required by every preset other
    /// than [`UpscalePreset::Off`].
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
            upscale: UpscalePreset::Off,
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
            init.set_property("osc", false)?;
            init.set_property("input-default-bindings", false)?;
            // Every URL reaching mpv has already been resolved by the
            // extractors, so mpv's youtube-dl hook has nothing to add. Leaving
            // it on spawns a subprocess and waits on its timeouts before
            // playback can even start.
            init.set_property("ytdl", false)?;
            // Likewise the config-file driven profile hooks: this is a library
            // embedded in an application, not a user's mpv install.
            init.set_property("config", false)?;
            init.set_property("load-scripts", false)?;
            Ok(())
        })?;

        let player = Self {
            mpv,
            shader_dir: config.shader_dir.clone(),
        };

        player.set_network_timeout(config.network_timeout_secs)?;
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

    pub fn set_muted(&self, muted: bool) -> Result<()> {
        self.mpv.set_property("mute", muted)?;
        Ok(())
    }

    // ---- tracks -----------------------------------------------------------

    /// Selects a subtitle track by id, or disables subtitles with `None`.
    pub fn set_subtitle_track(&self, id: Option<i64>) -> Result<()> {
        match id {
            Some(id) => self.mpv.set_property("sid", id)?,
            None => self.mpv.set_property("sid", "no")?,
        }
        Ok(())
    }

    pub fn set_audio_track(&self, id: Option<i64>) -> Result<()> {
        match id {
            Some(id) => self.mpv.set_property("aid", id)?,
            None => self.mpv.set_property("aid", "no")?,
        }
        Ok(())
    }

    /// Adds an external subtitle file or URL.
    pub fn add_subtitle(&self, url: &str) -> Result<()> {
        self.mpv.command("sub-add", &[url, "auto"])?;
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

    /// Applies an Anime4K preset, or clears shaders with
    /// [`UpscalePreset::Off`].
    pub fn set_upscale(&self, preset: UpscalePreset) -> Result<()> {
        let Some(chain) = preset.shader_chain() else {
            self.mpv.set_property("glsl-shaders", "")?;
            return Ok(());
        };

        let dir = self
            .shader_dir
            .as_deref()
            .ok_or(Error::ShadersUnavailable {
                preset: preset.name(),
            })?;

        // mpv separates list entries with ':' on Unix. A shader path
        // containing one would be ambiguous, which is why the directory is
        // configuration rather than something guessed from the environment.
        let paths: Vec<String> = chain
            .iter()
            .map(|file| dir.join(file).to_string_lossy().into_owned())
            .collect();

        self.mpv
            .set_property("glsl-shaders", paths.join(":").as_str())?;
        Ok(())
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
    fn defaults_are_conservative() {
        let config = PlayerConfig::default();
        assert_eq!(config.hwdec, DEFAULT_HWDEC);
        // Interpolation costs GPU time and is a matter of taste, so it is
        // opt-in rather than on by default.
        assert!(!config.interpolation);
        assert_eq!(config.upscale, UpscalePreset::Off);
        assert_eq!(config.network_timeout_secs, DEFAULT_NETWORK_TIMEOUT_SECS);
    }
}
