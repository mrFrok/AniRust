// SPDX-License-Identifier: GPL-3.0-or-later

//! Frame generation: RIFE through mpv's VapourSynth filter.
//!
//! RIFE, <https://github.com/hzwer/Practical-RIFE>, MIT, is a network that
//! draws the frames between two others. The VapourSynth plugin that runs it on
//! the GPU through ncnn and Vulkan,
//! <https://github.com/styler00dollar/VapourSynth-RIFE-ncnn-Vulkan>, MIT,
//! works on every vendor's card. mpv hands each decoded frame to a VapourSynth
//! script, which is generated here, and plays what comes back.
//!
//! Three things have to be present, and none of them is linked: a libmpv
//! built with the VapourSynth filter, VapourSynth itself, and the plugin with
//! its models (an [`RifeInstall`]). When any is missing the filter fails and
//! mpv plays the episode as it is — the script writes down why, and
//! [`last_error`] reads it back.
//!
//! The network runs at the height given, 720 lines unless asked otherwise:
//! measured on an RTX 4070 Ti SUPER, RIFE 4.6 makes 74 frames a second at
//! 720p and 46 at 1080p, so 1080p sources are brought down for it and the
//! upscaling shaders, which run after it, bring them back up.

use std::path::{Path, PathBuf};

/// How many frames a second to make.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TargetRate {
    /// Double: 24 becomes 48. The lightest.
    Double,
    /// Sixty, the rate most screens run at.
    #[default]
    Sixty,
    /// The screen's own rate, as mpv reports it; sixty when it does not.
    Display,
}

impl TargetRate {
    pub const ALL: [Self; 3] = [Self::Double, Self::Sixty, Self::Display];
}

/// Which RIFE network draws the frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RifeModel {
    /// RIFE 4.6: the long-standing choice for watching in real time.
    #[default]
    Fast,
    /// RIFE 4.26: cleaner motion, about a third slower.
    Quality,
}

impl RifeModel {
    pub const ALL: [Self; 2] = [Self::Fast, Self::Quality];

    /// The model's folder under the install's `models`.
    #[must_use]
    pub fn folder(self) -> &'static str {
        match self {
            Self::Fast => "rife-v4.6_ensembleFalse",
            Self::Quality => "rife-v4.26_ensembleFalse",
        }
    }
}

/// What to make, and from how tall a picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameGeneration {
    pub rate: TargetRate,
    pub model: RifeModel,
    /// Taller sources are brought down to this before RIFE sees them; 0 runs
    /// it at the source's own size.
    pub max_height: u32,
}

impl Default for FrameGeneration {
    fn default() -> Self {
        Self {
            rate: TargetRate::default(),
            model: RifeModel::default(),
            max_height: 720,
        }
    }
}

/// Where the RIFE plugin and its models are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RifeInstall {
    pub plugin: PathBuf,
    pub models: PathBuf,
}

/// The plugin's file name on this platform.
#[cfg(target_os = "windows")]
const PLUGIN: &str = "librife.dll";
#[cfg(target_os = "macos")]
const PLUGIN: &str = "librife.dylib";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const PLUGIN: &str = "librife.so";

impl RifeInstall {
    /// The install in `dir`, if the plugin and every model are there.
    #[must_use]
    pub fn in_dir(dir: &Path) -> Option<Self> {
        let install = Self {
            plugin: dir.join(PLUGIN),
            models: dir.join("models"),
        };
        let complete = install.plugin.is_file()
            && RifeModel::ALL.iter().all(|model| {
                install
                    .models
                    .join(model.folder())
                    .join("flownet.bin")
                    .is_file()
            });
        complete.then_some(install)
    }

    /// Looks where a build puts it: `ANIRUST_RIFE_DIR`, then `rife` beside
    /// the executable (the Windows and Linux archives), then
    /// `../lib/anirust/rife` from it (system packages), then the user's data
    /// folder.
    #[must_use]
    pub fn find() -> Option<Self> {
        let mut places: Vec<PathBuf> = Vec::new();
        if let Some(dir) = std::env::var_os("ANIRUST_RIFE_DIR") {
            places.push(dir.into());
        }
        if let Some(exe_dir) = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
        {
            places.push(exe_dir.join("rife"));
            places.push(exe_dir.join("../lib/anirust/rife"));
        }
        if let Some(data) = dirs::data_dir() {
            places.push(data.join("anirust").join("rife"));
        }
        places.iter().find_map(|dir| Self::in_dir(dir))
    }

    /// The VapourSynth script that makes `generation` happen.
    ///
    /// mpv defines `video_in`, `container_fps` and `display_fps` before it
    /// runs. Any exception is written to `error_file` and raised again, so mpv
    /// drops the filter and plays on, and the reason is not lost.
    #[must_use]
    pub fn script(&self, generation: FrameGeneration, error_file: &Path) -> String {
        let target = match generation.rate {
            TargetRate::Double => "source * 2",
            TargetRate::Sixty => "Fraction(60)",
            TargetRate::Display => {
                "Fraction(display_fps).limit_denominator(1001) if display_fps > 0 else Fraction(60)"
            }
        };
        format!(
            r#"# Written by AniRust for mpv's VapourSynth filter. Regenerated each time.
from fractions import Fraction
import traceback
import vapoursynth as vs

core = vs.core
try:
    if not hasattr(core, "rife"):
        core.std.LoadPlugin({plugin})
    clip = video_in
    source = Fraction(container_fps).limit_denominator(1001) if container_fps > 0 else Fraction(24000, 1001)
    target = {target}
    if target <= source * Fraction(11, 10):
        # Nothing to make: the source is already as fast as asked.
        clip.set_output()
    else:
        clip = core.std.AssumeFPS(clip, fpsnum=source.numerator, fpsden=source.denominator)
        original = clip.format.id
        matrix = "709" if clip.height >= 600 else "170m"
        max_height = {max_height}
        if max_height and clip.height > max_height:
            width = round(clip.width * max_height / clip.height / 2) * 2
            clip = core.resize.Bilinear(clip, width=width, height=max_height, format=vs.RGBS, matrix_in_s=matrix)
        else:
            clip = core.resize.Bilinear(clip, format=vs.RGBS, matrix_in_s=matrix)
        clip = core.rife.RIFE(clip, model_path={model}, fps_num=target.numerator, fps_den=target.denominator, sc=True)
        clip = core.resize.Bilinear(clip, format=original, matrix_s=matrix)
        clip = core.std.AssumeFPS(clip, fpsnum=target.numerator, fpsden=target.denominator)
        clip.set_output()
except Exception:
    with open({error_file}, "w", encoding="utf-8") as f:
        f.write(traceback.format_exc())
    raise
"#,
            plugin = python_string(&self.plugin),
            model = python_string(&self.models.join(generation.model.folder())),
            error_file = python_string(error_file),
            max_height = generation.max_height,
        )
    }
}

/// A path as a Python string literal: raw strings cannot end in a backslash
/// and paths may hold quotes, so it is escaped instead.
fn python_string(path: &Path) -> String {
    let text = path.to_string_lossy();
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Where the script and its error note go.
#[must_use]
pub fn work_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("anirust")
}

/// The note a failed script left, if it left one.
#[must_use]
pub fn last_error() -> Option<String> {
    std::fs::read_to_string(work_dir().join("rife-error.txt"))
        .ok()
        .filter(|text| !text.trim().is_empty())
}

/// Makes sure VapourSynth can be started from inside another program.
///
/// Since R73 VapourSynth needs to be told, once, which Python it belongs to:
/// `vapoursynth config` writes that down in
/// `~/.config/vapoursynth/vapoursynth.toml`, and without it every script
/// fails. When the file is missing this runs that command, which is what
/// VapourSynth's own instructions ask for. Windows keeps the same in the
/// registry, which VapourSynth's installer fills in, so there is nothing to
/// do there.
pub fn prepare_vapoursynth() {
    if cfg!(windows) {
        return;
    }
    let Some(config) = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".config")))
        .map(|dir| dir.join("vapoursynth").join("vapoursynth.toml"))
    else {
        return;
    };
    if config.is_file() {
        return;
    }
    match std::process::Command::new("python3")
        .args(["-m", "vapoursynth", "config"])
        .output()
    {
        Ok(output) if output.status.success() => {
            tracing::info!(path = %config.display(), "VapourSynth configured");
        }
        Ok(output) => tracing::warn!(
            stderr = %String::from_utf8_lossy(&output.stderr),
            "VapourSynth could not be configured"
        ),
        Err(error) => tracing::warn!(%error, "python3 did not run; is VapourSynth installed?"),
    }
}

/// The hardware decoders in `list`, each in its copy-back form.
///
/// VapourSynth works on frames in memory, so while it runs the decoder has to
/// hand frames back rather than keep them on the GPU. "no" stays "no".
#[must_use]
pub fn copy_back(list: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for entry in list.split(',').map(str::trim).filter(|e| !e.is_empty()) {
        let copied = if entry == "no" || entry == "auto-copy" || entry.ends_with("-copy") {
            entry.to_owned()
        } else if entry == "auto" || entry == "yes" {
            "auto-copy".to_owned()
        } else {
            format!("{entry}-copy")
        };
        if !out.contains(&copied) {
            out.push(copied);
        }
    }
    out.join(",")
}

/// An mpv option value holding any text: `%length%text`, so that colons,
/// commas and equals signs in a path are not read as separators.
#[must_use]
pub fn mpv_quoted(text: &str) -> String {
    format!("%{}%{text}", text.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoders_are_turned_into_their_copy_back_forms_once() {
        assert_eq!(
            copy_back("nvdec,vaapi,vulkan,nvdec-copy,vaapi-copy"),
            "nvdec-copy,vaapi-copy,vulkan-copy"
        );
        assert_eq!(copy_back("no"), "no");
        assert_eq!(copy_back("auto"), "auto-copy");
        assert_eq!(
            copy_back("d3d11va,nvdec,d3d11va-copy,dxva2-copy"),
            "d3d11va-copy,nvdec-copy,dxva2-copy"
        );
    }

    #[test]
    fn a_quoted_value_counts_bytes_not_characters() {
        assert_eq!(mpv_quoted("C:\\a,b"), "%6%C:\\a,b");
        assert_eq!(mpv_quoted("кэш"), "%6%кэш");
    }

    #[test]
    fn paths_become_python_strings_whatever_they_hold() {
        assert_eq!(
            python_string(Path::new("C:\\Users\\a \"b\"\\")),
            "\"C:\\\\Users\\\\a \\\"b\\\"\\\\\""
        );
    }

    #[test]
    fn the_script_names_the_model_rate_and_height() {
        let install = RifeInstall {
            plugin: PathBuf::from("/opt/rife/librife.so"),
            models: PathBuf::from("/opt/rife/models"),
        };
        let script = install.script(
            FrameGeneration {
                rate: TargetRate::Display,
                model: RifeModel::Quality,
                max_height: 720,
            },
            Path::new("/tmp/err.txt"),
        );
        assert!(script.contains("\"/opt/rife/models/rife-v4.26_ensembleFalse\""));
        assert!(script.contains("display_fps"));
        assert!(script.contains("max_height = 720"));
        assert!(script.contains("core.std.LoadPlugin(\"/opt/rife/librife.so\")"));
    }

    #[test]
    fn an_incomplete_install_is_not_found() {
        assert!(RifeInstall::in_dir(Path::new("/nonexistent-rife")).is_none());
    }
}
