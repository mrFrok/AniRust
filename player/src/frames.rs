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
//! - **vs-mlrt**, <https://github.com/AmusementClub/vs-mlrt>,
//!   GPL-3.0, on NVIDIA's TensorRT-RTX — the tensor cores of RTX cards. On
//!   the same card RIFE 4.26 makes 150 frames a second at 720p and 66 at
//!   1080p, and Real-ESRGAN takes 720p to 1440p at 43. The plugin is vs-mlrt's
//!   vstrt ported to VapourSynth's API 4 (`packaging/mlrt/`); TensorRT-RTX is
//!   NVIDIA's, fetched by the user from NVIDIA ([`Backend::download_url`]).
//!   The same vs-mlrt runs on Intel's OpenVINO and AMD's MIGraphX, each
//!   through its own ported plugin ([`Backend`]).
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
    /// RIFE through ncnn and Vulkan, on any GPU.
    Vulkan(&'a RifeInstall),
    /// vs-mlrt on the vendor's own runtime.
    Mlrt(&'a Mlrt),
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

/// Which vendor runtime vs-mlrt runs the networks on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// NVIDIA RTX tensor cores, through TensorRT-RTX.
    TensorRt,
    /// Intel GPUs' XMX engines (and any CPU), through OpenVINO.
    OpenVino,
    /// AMD GPUs, through ROCm's MIGraphX.
    MigraphX,
}

impl Backend {
    pub const ALL: [Self; 3] = [Self::TensorRt, Self::OpenVino, Self::MigraphX];

    /// Our ported plugin for this runtime, as built for this platform.
    #[must_use]
    pub fn plugin(self) -> &'static str {
        match (self, cfg!(windows)) {
            (Self::TensorRt, false) => "libvstrt_rtx.so",
            (Self::TensorRt, true) => "vstrt_rtx.dll",
            (Self::OpenVino, false) => "libvsov.so",
            (Self::OpenVino, true) => "vsov.dll",
            (Self::MigraphX, false) => "libvsmigx.so",
            (Self::MigraphX, true) => "vsmigx.dll",
        }
    }

    /// The variable that points at a runtime somewhere else.
    fn runtime_env(self) -> &'static str {
        match self {
            Self::TensorRt => "ANIRUST_TRT_RTX_DIR",
            Self::OpenVino => "ANIRUST_OPENVINO_DIR",
            Self::MigraphX => "ROCM_PATH",
        }
    }

    /// Where the runtime is: fetched into the data folder for TensorRT-RTX
    /// and OpenVINO, the system's ROCm for MIGraphX.
    #[must_use]
    pub fn runtime_dir(self) -> Option<PathBuf> {
        if let Some(dir) = std::env::var_os(self.runtime_env()) {
            return Some(dir.into());
        }
        match self {
            Self::TensorRt => {
                dirs::data_dir().map(|data| data.join("anirust").join("tensorrt-rtx"))
            }
            Self::OpenVino => dirs::data_dir().map(|data| data.join("anirust").join("openvino")),
            Self::MigraphX => Some(PathBuf::from("/opt/rocm")),
        }
    }

    /// Whether `dir` holds this runtime, unpacked as its vendor packs it.
    #[must_use]
    pub fn is_runtime(self, dir: &Path) -> bool {
        match self {
            Self::TensorRt => dir
                .join("bin")
                .join(if cfg!(windows) {
                    "tensorrt_rtx.exe"
                } else {
                    "tensorrt_rtx"
                })
                .is_file(),
            Self::OpenVino => openvino_libs(dir).is_dir(),
            Self::MigraphX => dir
                .join("bin")
                .join(if cfg!(windows) {
                    "migraphx-driver.exe"
                } else {
                    "migraphx-driver"
                })
                .is_file(),
        }
    }

    /// The vendor's archive of the runtime the plugin was built against, for
    /// the player to fetch when asked; `None` where the runtime comes from the
    /// system (ROCm) or has no build for this platform.
    #[must_use]
    pub fn download_url(self) -> Option<&'static str> {
        match (self, cfg!(target_os = "linux"), cfg!(target_os = "windows")) {
            (Self::TensorRt, true, _) => Some(
                "https://developer.nvidia.com/downloads/trt/rtx_sdk/secure/1.3/TensorRT-RTX-1.3.0.35-Linux-x86_64-cuda-13.1-Release-external.tar.gz",
            ),
            (Self::TensorRt, _, true) => Some(
                "https://developer.nvidia.com/downloads/trt/rtx_sdk/secure/1.6/TensorRT-RTX-1.6.1.120-Windows-amd64-cuda-13.4-Release-external.zip",
            ),
            (Self::OpenVino, true, _) => Some(
                "https://storage.openvinotoolkit.org/repositories/openvino/packages/2024.6/linux/l_openvino_toolkit_ubuntu24_2024.6.0.17404.4c0f47d2335_x86_64.tgz",
            ),
            (Self::OpenVino, _, true) => Some(
                "https://storage.openvinotoolkit.org/repositories/openvino/packages/2024.6/windows/w_openvino_toolkit_windows_2024.6.0.17404.4c0f47d2335_x86_64.zip",
            ),
            _ => None,
        }
    }

    /// The parts of the vendor's archive the runtime needs, by top folder,
    /// so the samples and wheels in it are left behind.
    #[must_use]
    pub fn kept_parts(self) -> &'static [&'static str] {
        match self {
            Self::TensorRt => &["lib", "bin"],
            Self::OpenVino => &["runtime"],
            Self::MigraphX => &[],
        }
    }
}

/// OpenVINO's libraries inside its archive.
fn openvino_libs(dir: &Path) -> PathBuf {
    if cfg!(windows) {
        dir.join("runtime")
            .join("bin")
            .join("intel64")
            .join("Release")
    } else {
        dir.join("runtime").join("lib").join("intel64")
    }
}

/// The networks vs-mlrt runs, under `models/`.
const MLRT_MODELS: [&str; 3] = [
    "rife/rife_v4.26.onnx",
    "rife/rife_v4.25_lite.onnx",
    "RealESRGANv2/RealESRGANv2-animevideo-xsx2.onnx",
];

/// vs-mlrt on one runtime: our part — the plugin, vsmlrt.py and the networks
/// — and the vendor's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mlrt {
    pub backend: Backend,
    /// vsmlrt.py, the plugins and `models/`, shipped with the program.
    pub dir: PathBuf,
    /// The vendor's runtime.
    pub runtime: PathBuf,
}

impl Mlrt {
    /// Our part, where a build puts it (`ANIRUST_MLRT_DIR` first), if
    /// vsmlrt.py and every network are there. Only Linux and Windows.
    #[must_use]
    pub fn shipped() -> Option<PathBuf> {
        if !cfg!(any(target_os = "linux", target_os = "windows")) {
            return None;
        }
        places("ANIRUST_MLRT_DIR", "mlrt").into_iter().find(|dir| {
            dir.join("vsmlrt.py").is_file()
                && MLRT_MODELS
                    .iter()
                    .all(|model| dir.join("models").join(model).is_file())
        })
    }

    /// Whether our part carries the plugin for `backend`.
    #[must_use]
    pub fn ships(backend: Backend) -> bool {
        Self::shipped().is_some_and(|dir| dir.join(backend.plugin()).is_file())
    }

    /// `backend` with both parts there, if they are.
    #[must_use]
    pub fn find(backend: Backend) -> Option<Self> {
        let dir = Self::shipped().filter(|dir| dir.join(backend.plugin()).is_file())?;
        let runtime = backend
            .runtime_dir()
            .filter(|runtime| backend.is_runtime(runtime))?;
        Some(Self {
            backend,
            dir,
            runtime,
        })
    }

    /// The script lines that load the runtime and the plugin, import
    /// vsmlrt and set `backend` up.
    fn setup(&self) -> String {
        let common = format!(
            "    mlrt = {mlrt}
    runtime = {runtime}
",
            mlrt = python_string(&self.dir),
            runtime = python_string(&self.runtime),
        );
        let load = match self.backend {
            // TensorRT-RTX is where the user fetched it, not beside the
            // plugin: its libraries are loaded first, so the plugin finds
            // them by name.
            Backend::TensorRt => r#"    libs = [os.path.join(runtime, "lib"), os.path.join(runtime, "bin")]
    if sys.platform == "win32":
        for d in libs:
            if os.path.isdir(d):
                os.add_dll_directory(d)
    else:
        for name in ("libtensorrt_rtx.so.1", "libtensorrt_onnxparser_rtx.so.1"):
            ctypes.CDLL(os.path.join(libs[0], name), mode=ctypes.RTLD_GLOBAL)
"#
            .to_owned(),
            // OpenVINO likewise, with TBB under it, which it is built on.
            Backend::OpenVino => r#"    if sys.platform == "win32":
        for d in (os.path.join(runtime, "runtime", "bin", "intel64", "Release"), os.path.join(runtime, "runtime", "3rdparty", "tbb", "bin")):
            if os.path.isdir(d):
                os.add_dll_directory(d)
    else:
        tbb = os.path.join(runtime, "runtime", "3rdparty", "tbb", "lib")
        for name in sorted(os.listdir(tbb)):
            if name.startswith(("libtbb.so.", "libtbbmalloc.so.")) and name.count(".") == 2:
                ctypes.CDLL(os.path.join(tbb, name), mode=ctypes.RTLD_GLOBAL)
        ctypes.CDLL(os.path.join(runtime, "runtime", "lib", "intel64", "libopenvino.so"), mode=ctypes.RTLD_GLOBAL)
"#
            .to_owned(),
            // ROCm is the system's, on the library path already.
            Backend::MigraphX => String::new(),
        };
        let (namespace, backend) = match self.backend {
            Backend::TensorRt => (
                "trt_rtx",
                format!(
                    r#"    vsmlrt.tensorrt_rtx_path = os.path.join(runtime, "bin", "tensorrt_rtx.exe" if sys.platform == "win32" else "tensorrt_rtx")
    # vsmlrt starts the engine builder with nothing but what is given here.
    path_var = "PATH" if sys.platform == "win32" else "LD_LIBRARY_PATH"
    backend = vsmlrt.Backend.TRT_RTX(
        fp16=False,  # the networks are fp16 already; see prepare-models.py
        num_streams=4,
        engine_folder={engines},
        custom_env={{path_var: os.pathsep.join(libs)}},
    )
"#,
                    engines = python_string(&engines_dir()),
                ),
            ),
            Backend::OpenVino => (
                "ov",
                "    backend = vsmlrt.Backend.OV_GPU(fp16=False, num_streams=2)
".to_owned(),
            ),
            Backend::MigraphX => (
                "migx",
                r#"    vsmlrt.migraphx_driver_path = os.path.join(runtime, "bin", "migraphx-driver")
    backend = vsmlrt.Backend.MIGX(fp16=False, custom_env=dict(os.environ))
"#
                .to_owned(),
            ),
        };
        format!(
            r#"{common}{load}    if not hasattr(core, "{namespace}"):
        core.std.LoadPlugin(os.path.join(mlrt, {plugin}))
    if mlrt not in sys.path:
        sys.path.insert(0, mlrt)
    import vsmlrt
    # The networks are read from a copy in the cache, where what the runtime
    # compiles from them can be kept beside them; the program's own folder
    # may not be writable.
    vsmlrt.models_path = {models}
{backend}"#,
            plugin = python_string(Path::new(self.backend.plugin())),
            models = python_string(&models_cache()),
        )
    }
}

/// The cached copy of the networks: see [`copy_models`].
#[must_use]
pub fn models_cache() -> PathBuf {
    work_dir().join("models")
}

/// Copies our networks into the cache, where vs-mlrt may write what it
/// compiles from them beside them — MIGraphX does so with no say in where.
/// Files already there at the same size are left alone.
pub fn copy_models(shipped: &Path) -> std::io::Result<()> {
    for model in MLRT_MODELS {
        let from = shipped.join("models").join(model);
        let to = models_cache().join(model);
        let same = std::fs::metadata(&to)
            .and_then(|to| std::fs::metadata(&from).map(|from| from.len() == to.len()))
            .unwrap_or(false);
        if !same {
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
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
        Networks::Mlrt(mlrt) => body.push_str(&mlrt.setup()),
    }

    // ---- the picture, in the networks' format ----
    let upscale = enhancement.upscale && matches!(networks, Networks::Mlrt(_));
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
            Networks::Mlrt(_) => body.push_str(&format!(
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

    fn tensorrt() -> Mlrt {
        Mlrt {
            backend: Backend::TensorRt,
            dir: PathBuf::from("/opt/mlrt"),
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
            Networks::Mlrt(&trt),
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
            Networks::Mlrt(&trt),
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
    fn each_backend_loads_its_own_plugin_and_runtime() {
        for (backend, namespace, marker) in [
            (Backend::TensorRt, "trt_rtx", "Backend.TRT_RTX("),
            (Backend::OpenVino, "ov", "Backend.OV_GPU("),
            (Backend::MigraphX, "migx", "Backend.MIGX("),
        ] {
            let mlrt = Mlrt {
                backend,
                dir: PathBuf::from("/opt/mlrt"),
                runtime: PathBuf::from("/opt/runtime"),
            };
            let script = script(
                Networks::Mlrt(&mlrt),
                generation(TargetRate::Sixty, RifeModel::Quality),
                None,
                Path::new("/tmp/err.txt"),
            );
            assert!(script.contains(&format!("hasattr(core, \"{namespace}\")")));
            assert!(script.contains(backend.plugin()));
            assert!(script.contains(marker));
            assert!(script.contains("vsmlrt.models_path"));
        }
    }

    #[test]
    fn an_incomplete_install_is_not_found() {
        assert!(RifeInstall::in_dir(Path::new("/nonexistent-rife")).is_none());
        for backend in Backend::ALL {
            assert!(!backend.is_runtime(Path::new("/nonexistent-runtime")));
        }
    }
}
