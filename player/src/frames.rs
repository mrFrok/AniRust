// SPDX-License-Identifier: GPL-3.0-or-later

//! Neural video filters through mpv's VapourSynth filter: RIFE frame
//! generation and Real-ESRGAN upscaling.
//!
//! mpv hands each decoded frame to a VapourSynth script, which is generated
//! here, and plays what comes back. Two engines run the networks:
//!
//! - **Vulkan**: VapourSynth-RIFE-ncnn-Vulkan,
//!   <https://github.com/styler00dollar/VapourSynth-RIFE-ncnn-Vulkan>, MIT —
//!   every vendor's card. Measured on an RTX 4070 Ti SUPER, RIFE 4.6 makes 74
//!   frames a second at 720p and 46 at 1080p.
//! - **TensorRT**: vs-mlrt, <https://github.com/AmusementClub/vs-mlrt>,
//!   GPL-3.0, on NVIDIA's TensorRT-RTX — the tensor cores of RTX cards. On
//!   the same card RIFE 4.26 makes 150 frames a second at 720p and 66 at
//!   1080p, and Real-ESRGAN takes 720p to 1440p at 43. The plugin is vs-mlrt's
//!   vstrt ported to VapourSynth's API 4 (`packaging/mlrt/`); TensorRT-RTX is
//!   NVIDIA's, fetched by the user from NVIDIA ([`TensorRt::download_url`]).
//!
//! Nothing here is linked. When a part is missing the filter fails and mpv
//! plays the episode as it is — the script writes down why, and
//! [`last_error`] reads it back.
//!
//! Networks: RIFE, <https://github.com/hzwer/Practical-RIFE>, MIT; Real-ESRGAN
//! AnimeVideo v3, <https://github.com/xinntao/Real-ESRGAN>, BSD-3-Clause.

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
    /// Half the screen's rate: 90 on a 180 Hz screen. Not offered as such —
    /// where the player steps down to when the full rate is too much, since
    /// every frame then lasts exactly two refreshes.
    HalfDisplay,
}

impl TargetRate {
    /// The rates a menu offers.
    pub const ALL: [Self; 3] = [Self::Double, Self::Sixty, Self::Display];
}

/// Which RIFE network draws the frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RifeModel {
    /// The lighter network: RIFE 4.6 on Vulkan, 4.25 lite on TensorRT.
    #[default]
    Fast,
    /// RIFE 4.26: cleaner motion.
    Quality,
}

impl RifeModel {
    pub const ALL: [Self; 2] = [Self::Fast, Self::Quality];

    /// The model's folder under the Vulkan install's `models`.
    #[must_use]
    pub fn folder(self) -> &'static str {
        match self {
            Self::Fast => "rife-v4.6_ensembleFalse",
            Self::Quality => "rife-v4.26_ensembleFalse",
        }
    }

    /// The model's name in vsmlrt's `RIFEModel`.
    fn mlrt_name(self) -> &'static str {
        match self {
            Self::Fast => "v4_25_lite",
            Self::Quality => "v4_26",
        }
    }

    /// The multiple a picture's sides must be for the network.
    fn mlrt_alignment(self) -> u32 {
        match self {
            Self::Fast => 128,
            Self::Quality => 64,
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

/// What the script does to the picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Enhancement {
    pub frames: Option<FrameGeneration>,
    /// Real-ESRGAN AnimeVideo v3 doubles the picture before anything else.
    /// TensorRT only; ignored on Vulkan.
    pub upscale: bool,
}

impl Enhancement {
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.frames.is_none() && !self.upscale
    }
}

/// The engine that runs the networks, with what it needs.
#[derive(Debug, Clone, Copy)]
pub enum Networks<'a> {
    Vulkan(&'a RifeInstall),
    TensorRt(&'a TensorRt),
}

/// Where the Vulkan RIFE plugin and its models are.
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

/// Where a build puts a folder named `name`: `env` first, then beside the
/// executable (the Windows zip, the macOS bundle), `../name` from it (the
/// Linux archive, whose programs sit in bin/), `../lib/anirust/name` (system
/// packages), then the user's data folder.
fn places(env: &str, name: &str) -> Vec<PathBuf> {
    let mut places: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::var_os(env) {
        places.push(dir.into());
    }
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        places.push(exe_dir.join(name));
        places.push(exe_dir.join("..").join(name));
        places.push(exe_dir.join("../lib/anirust").join(name));
    }
    if let Some(data) = dirs::data_dir() {
        places.push(data.join("anirust").join(name));
    }
    places
}

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

    /// Looks where a build puts it; `ANIRUST_RIFE_DIR` first.
    #[must_use]
    pub fn find() -> Option<Self> {
        places("ANIRUST_RIFE_DIR", "rife")
            .iter()
            .find_map(|dir| Self::in_dir(dir))
    }
}

/// The TensorRT engine: our part — the vstrt plugin, vsmlrt.py and the
/// networks — and NVIDIA's, the TensorRT-RTX runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TensorRt {
    /// vstrt_rtx, vsmlrt.py and `models/`, shipped with the program.
    pub mlrt: PathBuf,
    /// TensorRT-RTX as NVIDIA packs it: `lib/` and `bin/`.
    pub runtime: PathBuf,
}

#[cfg(target_os = "windows")]
const VSTRT: &str = "vstrt_rtx.dll";
#[cfg(not(target_os = "windows"))]
const VSTRT: &str = "libvstrt_rtx.so";

#[cfg(target_os = "windows")]
const ENGINE_BUILDER: &str = "tensorrt_rtx.exe";
#[cfg(not(target_os = "windows"))]
const ENGINE_BUILDER: &str = "tensorrt_rtx";

/// The networks the TensorRT engine runs, under `models/`.
const MLRT_MODELS: [&str; 3] = [
    "rife/rife_v4.26.onnx",
    "rife/rife_v4.25_lite.onnx",
    "RealESRGANv2/RealESRGANv2-animevideo-xsx2.onnx",
];

impl TensorRt {
    /// Our part, where a build puts it (`ANIRUST_MLRT_DIR` first), if it is
    /// complete. Only Linux and Windows have TensorRT-RTX.
    #[must_use]
    pub fn find_mlrt() -> Option<PathBuf> {
        if !cfg!(any(target_os = "linux", target_os = "windows")) {
            return None;
        }
        places("ANIRUST_MLRT_DIR", "mlrt").into_iter().find(|dir| {
            dir.join(VSTRT).is_file()
                && dir.join("vsmlrt.py").is_file()
                && MLRT_MODELS
                    .iter()
                    .all(|model| dir.join("models").join(model).is_file())
        })
    }

    /// Where the runtime goes when the player fetches it.
    #[must_use]
    pub fn runtime_dir() -> Option<PathBuf> {
        dirs::data_dir().map(|data| data.join("anirust").join("tensorrt-rtx"))
    }

    /// Whether `dir` holds an unpacked TensorRT-RTX.
    #[must_use]
    pub fn is_runtime(dir: &Path) -> bool {
        dir.join("bin").join(ENGINE_BUILDER).is_file()
    }

    /// Both parts, when both are there.
    #[must_use]
    pub fn find() -> Option<Self> {
        let mlrt = Self::find_mlrt()?;
        let runtime = std::env::var_os("ANIRUST_TRT_RTX_DIR")
            .map(PathBuf::from)
            .or_else(Self::runtime_dir)
            .filter(|dir| Self::is_runtime(dir))?;
        Some(Self { mlrt, runtime })
    }

    /// NVIDIA's archive of the TensorRT-RTX build the plugin was made
    /// against, for this platform: a .tar.gz on Linux, a .zip on Windows.
    #[must_use]
    pub fn download_url() -> Option<&'static str> {
        if cfg!(target_os = "linux") {
            Some(
                "https://developer.nvidia.com/downloads/trt/rtx_sdk/secure/1.3/TensorRT-RTX-1.3.0.35-Linux-x86_64-cuda-13.1-Release-external.tar.gz",
            )
        } else if cfg!(target_os = "windows") {
            Some(
                "https://developer.nvidia.com/downloads/trt/rtx_sdk/secure/1.6/TensorRT-RTX-1.6.1.120-Windows-amd64-cuda-13.4-Release-external.zip",
            )
        } else {
            None
        }
    }
}

/// Where compiled TensorRT engines are kept: building one takes ten seconds
/// or so the first time a resolution is seen, and the cache makes it once.
#[must_use]
pub fn engines_dir() -> PathBuf {
    work_dir().join("engines")
}

/// The VapourSynth script that makes `enhancement` happen on `networks`.
///
/// mpv defines `video_in`, `container_fps` and `display_fps` before it runs —
/// but its `display_fps` comes straight from the video output, which under
/// the render API knows no rate and says 0. So the screen's rate is written
/// into the script when the caller knows it, and mpv's own, then 60, are only
/// fallbacks.
///
/// Any exception is written to `error_file` and raised again, so mpv drops
/// the filter and plays on, and the reason is not lost.
#[must_use]
pub fn script(
    networks: Networks<'_>,
    enhancement: Enhancement,
    display_fps: Option<f64>,
    error_file: &Path,
) -> String {
    let mut body = String::new();

    // ---- the engine ----
    match networks {
        Networks::Vulkan(install) => {
            body.push_str(&format!(
                "    if not hasattr(core, \"rife\"):\n        core.std.LoadPlugin({})\n",
                python_string(&install.plugin)
            ));
        }
        Networks::TensorRt(trt) => {
            body.push_str(&format!(
                r#"    runtime = {runtime}
    mlrt = {mlrt}
    libs = [os.path.join(runtime, "lib"), os.path.join(runtime, "bin")]
    # TensorRT-RTX is where the user fetched it, not beside the plugin: its
    # libraries are loaded first, so the plugin finds them by name.
    if sys.platform == "win32":
        for d in libs:
            if os.path.isdir(d):
                os.add_dll_directory(d)
    else:
        for name in ("libtensorrt_rtx.so.1", "libtensorrt_onnxparser_rtx.so.1"):
            ctypes.CDLL(os.path.join(libs[0], name), mode=ctypes.RTLD_GLOBAL)
    if not hasattr(core, "trt_rtx"):
        core.std.LoadPlugin(os.path.join(mlrt, {vstrt}))
    if mlrt not in sys.path:
        sys.path.insert(0, mlrt)
    import vsmlrt
    vsmlrt.models_path = os.path.join(mlrt, "models")
    vsmlrt.tensorrt_rtx_path = os.path.join(runtime, "bin", {builder})
    # vsmlrt starts the engine builder with nothing but what is given here.
    path_var = "PATH" if sys.platform == "win32" else "LD_LIBRARY_PATH"
    backend = vsmlrt.Backend.TRT_RTX(
        fp16=False,  # the networks are fp16 already; see prepare-models.py
        num_streams=4,
        engine_folder={engines},
        custom_env={{path_var: os.pathsep.join(libs)}},
    )
"#,
                runtime = python_string(&trt.runtime),
                mlrt = python_string(&trt.mlrt),
                vstrt = python_string(Path::new(VSTRT)),
                builder = python_string(Path::new(ENGINE_BUILDER)),
                engines = python_string(&engines_dir()),
            ));
        }
    }

    // ---- the picture, in the networks' format ----
    let upscale = enhancement.upscale && matches!(networks, Networks::TensorRt(_));
    let max_height = match enhancement.frames {
        // Upscaled first, the picture is meant to stay large.
        Some(generation) if !upscale => generation.max_height,
        _ => 0,
    };
    body.push_str(&format!(
        r#"    clip = video_in
    source = Fraction(container_fps).limit_denominator(1001) if container_fps > 0 else Fraction(24000, 1001)
    clip = core.std.AssumeFPS(clip, fpsnum=source.numerator, fpsden=source.denominator)
    rate = source
    original = clip.format.id
    matrix = "709" if clip.height >= 600 else "170m"
    max_height = {max_height}
    if max_height and clip.height > max_height:
        width = round(clip.width * max_height / clip.height / 2) * 2
        clip = core.resize.Bilinear(clip, width=width, height=max_height, format=vs.RGBS, matrix_in_s=matrix)
    else:
        clip = core.resize.Bilinear(clip, format=vs.RGBS, matrix_in_s=matrix)
"#
    ));

    if upscale {
        body.push_str(
            "    clip = vsmlrt.RealESRGAN(clip, model=vsmlrt.RealESRGANv2Model.animevideo_xsx2, backend=backend)\n",
        );
    }

    if let Some(generation) = enhancement.frames {
        let target = match (generation.rate, display_fps) {
            (TargetRate::Double, _) => "source * 2".to_owned(),
            (TargetRate::Sixty, _) => "Fraction(60)".to_owned(),
            (TargetRate::Display, Some(fps)) => {
                format!("Fraction({fps:.3}).limit_denominator(1001)")
            }
            (TargetRate::Display, None) => {
                "Fraction(display_fps).limit_denominator(1001) if display_fps > 0 else Fraction(60)"
                    .to_owned()
            }
            (TargetRate::HalfDisplay, Some(fps)) => {
                format!("Fraction({:.3}).limit_denominator(1001)", fps / 2.0)
            }
            (TargetRate::HalfDisplay, None) => {
                "Fraction(display_fps / 2).limit_denominator(1001) if display_fps > 0 else Fraction(60)"
                    .to_owned()
            }
        };
        body.push_str(&format!(
            "    target = {target}\n    if target > source * Fraction(11, 10):\n"
        ));
        match networks {
            Networks::Vulkan(install) => body.push_str(&format!(
                r#"        clip = core.rife.RIFE(clip, model_path={model}, fps_num=target.numerator, fps_den=target.denominator, sc=True)
        rate = target
"#,
                model = python_string(&install.models.join(generation.model.folder())),
            )),
            Networks::TensorRt(_) => body.push_str(&format!(
                r#"        multi = target / source
        # A whole multiple runs faster in vsmlrt; one within a percent is
        # taken, and mpv's display sync absorbs the difference.
        if abs(multi - round(multi)) < multi / 100:
            multi = Fraction(round(multi))
        align = {align}
        pad_w, pad_h = -clip.width % align, -clip.height % align
        padded = core.std.AddBorders(clip, right=pad_w, bottom=pad_h) if pad_w or pad_h else clip
        padded = mark_scene_changes(padded)
        out = vsmlrt.RIFE(padded, multi=multi, model=vsmlrt.RIFEModel.{model}, backend=backend, video_player=True)
        clip = core.std.Crop(out, right=pad_w, bottom=pad_h) if pad_w or pad_h else out
        rate = source * multi
"#,
                align = generation.model.mlrt_alignment(),
                model = generation.model.mlrt_name(),
            )),
        }
    }

    body.push_str(
        r#"    clip = core.resize.Bilinear(clip, format=original, matrix_s=matrix)
    clip = core.std.AssumeFPS(clip, fpsnum=rate.numerator, fpsden=rate.denominator)
    clip.set_output()
"#,
    );

    format!(
        r#"# Written by AniRust for mpv's VapourSynth filter. Regenerated each time.
from fractions import Fraction
import ctypes
import os
import sys
import traceback
import vapoursynth as vs

core = vs.core


def mark_scene_changes(clip, threshold=0.12):
    """Marks the frame before a cut, where vsmlrt then holds the frame
    instead of blending two scenes. Judged on a small grey copy: cheap, and a
    cut changes everything."""
    small = core.resize.Bilinear(clip, width=max(clip.width // 8, 16), height=max(clip.height // 8, 16), format=vs.GRAYS, matrix_s="709")
    stats = core.std.PlaneStats(small, small[1:] + small[-1])

    def mark(n, f):
        out = f[0].copy()
        out.props["_SceneChangeNext"] = int(f[1].props["PlaneStatsDiff"] > threshold)
        return out

    return core.std.ModifyFrame(clip, [clip, stats], mark)


try:
{body}except Exception:
    with open({error_file}, "w", encoding="utf-8") as f:
        f.write(traceback.format_exc())
    raise
"#,
        error_file = python_string(error_file),
    )
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

    fn vulkan() -> RifeInstall {
        RifeInstall {
            plugin: PathBuf::from("/opt/rife/librife.so"),
            models: PathBuf::from("/opt/rife/models"),
        }
    }

    fn tensorrt() -> TensorRt {
        TensorRt {
            mlrt: PathBuf::from("/opt/mlrt"),
            runtime: PathBuf::from("/home/u/.local/share/anirust/tensorrt-rtx"),
        }
    }

    fn generation(rate: TargetRate, model: RifeModel) -> Enhancement {
        Enhancement {
            frames: Some(FrameGeneration {
                rate,
                model,
                max_height: 720,
            }),
            upscale: false,
        }
    }

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
    fn the_vulkan_script_names_the_model_rate_and_height() {
        let install = vulkan();
        let enhancement = generation(TargetRate::Display, RifeModel::Quality);
        let script = script(
            Networks::Vulkan(&install),
            enhancement,
            None,
            Path::new("/tmp/err.txt"),
        );
        assert!(script.contains("\"/opt/rife/models/rife-v4.26_ensembleFalse\""));
        assert!(script.contains("display_fps"));
        assert!(script.contains("max_height = 720"));
        assert!(script.contains("core.std.LoadPlugin(\"/opt/rife/librife.so\")"));
        let told = super::script(
            Networks::Vulkan(&install),
            enhancement,
            Some(179.999),
            Path::new("/tmp/err.txt"),
        );
        assert!(told.contains("target = Fraction(179.999).limit_denominator(1001)"));
    }

    #[test]
    fn the_tensorrt_script_loads_the_runtime_then_vsmlrt() {
        let trt = tensorrt();
        let script = script(
            Networks::TensorRt(&trt),
            generation(TargetRate::Sixty, RifeModel::Fast),
            None,
            Path::new("/tmp/err.txt"),
        );
        let preload = script
            .find("ctypes.CDLL")
            .expect("the runtime is preloaded");
        let plugin = script.find("core.std.LoadPlugin").expect("the plugin");
        let import = script.find("import vsmlrt").expect("vsmlrt");
        assert!(preload < plugin && plugin < import);
        assert!(script.contains("RIFEModel.v4_25_lite"));
        assert!(script.contains("align = 128"));
        assert!(script.contains("video_player=True"));
        assert!(script.contains("mark_scene_changes(padded)"));
        assert!(!script.contains("RealESRGAN("));
    }

    #[test]
    fn upscaling_runs_first_and_keeps_the_picture_large() {
        let trt = tensorrt();
        let mut enhancement = generation(TargetRate::Sixty, RifeModel::Quality);
        enhancement.upscale = true;
        let script = script(
            Networks::TensorRt(&trt),
            enhancement,
            None,
            Path::new("/tmp/err.txt"),
        );
        let upscale = script.find("vsmlrt.RealESRGAN(").expect("upscaling");
        let rife = script.find("vsmlrt.RIFE(").expect("frame generation");
        assert!(upscale < rife);
        assert!(script.contains("max_height = 0"));
    }

    #[test]
    fn vulkan_has_no_upscaling_to_offer() {
        let install = vulkan();
        let script = script(
            Networks::Vulkan(&install),
            Enhancement {
                frames: None,
                upscale: true,
            },
            None,
            Path::new("/tmp/err.txt"),
        );
        assert!(!script.contains("RealESRGAN"));
    }

    #[test]
    fn an_incomplete_install_is_not_found() {
        assert!(RifeInstall::in_dir(Path::new("/nonexistent-rife")).is_none());
        assert!(!TensorRt::is_runtime(Path::new("/nonexistent-trt")));
    }
}
