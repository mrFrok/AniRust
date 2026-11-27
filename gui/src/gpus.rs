// SPDX-License-Identifier: GPL-3.0-or-later

//! Whose GPUs the machine has.
//!
//! Each neural engine runs on one maker's cards — TensorRT on NVIDIA's,
//! MIGraphX on AMD's, OpenVINO on Intel's — and finds nothing on the
//! others'. So the menu offers only the engines this machine can run, and
//! marks the one that suits it best. Asked once: the GPUs do not change
//! while the program runs.

use std::path::Path;
use std::sync::OnceLock;

const NVIDIA: u32 = 0x10de;
const AMD: u32 = 0x1002;
const INTEL: u32 = 0x8086;

/// The PCI vendor ids of the machine's GPUs, or `None` where the system will
/// not say.
pub fn vendors() -> Option<&'static [u32]> {
    static VENDORS: OnceLock<Option<Vec<u32>>> = OnceLock::new();
    VENDORS.get_or_init(read).as_deref()
}

/// How much neural work a card can carry, as far as its name tells.
///
/// The figures the player's defaults were measured on — 1080p RIFE and
/// 1080p doubling — are a 4070 Ti SUPER's. A 2060 does not carry them, and
/// guessing from the vendor alone would offer them to it. So the name is
/// read where the system gives one, and a card it cannot place is treated
/// as the smallest kind: the adaptive watch can still lower the load from
/// there, and nothing is promised that the card cannot do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Tier {
    /// 2060, 3050, 4060 and the like: 720p neural work at most.
    #[default]
    Small,
    /// 3070, 4070 and the like: 1080p frame generation, 720p doubling.
    Medium,
    /// 3080, 4070 Ti and up: 1080p for both.
    Large,
}

/// The machine's best card, as far as its name can be read.
pub fn tier() -> Tier {
    static TIER: OnceLock<Tier> = OnceLock::new();
    *TIER.get_or_init(|| {
        names()
            .iter()
            .map(|name| tier_of(name))
            .max()
            .unwrap_or_default()
    })
}

/// What a card's marketing name says about it.
///
/// The number after RTX or GTX carries the class: 60 and below are the
/// small cards, 70 the middle, 80 and 90 the large. A Ti or Super 70 is
/// close enough to an 80 to count as large, and a laptop's card is a step
/// below its desktop name.
fn tier_of(name: &str) -> Tier {
    let name = name.to_ascii_uppercase();
    let laptop = name.contains("LAPTOP") || name.contains("MAX-Q");
    let Some(number) = model_number(&name) else {
        return Tier::Small;
    };
    let mut tier = match number % 100 {
        80..=99 => Tier::Large,
        70 => Tier::Medium,
        _ => Tier::Small,
    };
    if tier == Tier::Medium && (name.contains(" TI") || name.contains("SUPER")) {
        tier = Tier::Large;
    }
    if laptop && tier > Tier::Small {
        tier = if tier == Tier::Large {
            Tier::Medium
        } else {
            Tier::Small
        };
    }
    tier
}

/// The four digits after RTX or GTX, as in `NVIDIA GeForce RTX 4070 Ti SUPER`.
fn model_number(name: &str) -> Option<u32> {
    ["RTX ", "GTX "].iter().find_map(|mark| {
        let rest = name.split_once(mark)?.1;
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        (digits.len() == 4).then(|| digits.parse().ok()).flatten()
    })
}

/// The cards' names, where the system tells them.
#[cfg(target_os = "linux")]
fn names() -> Vec<String> {
    // NVIDIA's driver writes the marketing name here; the kernel's sysfs
    // has vendor and device ids only.
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/proc/driver/nvidia/gpus") {
        for entry in entries.flatten() {
            let Ok(text) = std::fs::read_to_string(entry.path().join("information")) else {
                continue;
            };
            for line in text.lines() {
                if let Some(model) = line.strip_prefix("Model:") {
                    names.push(model.trim().to_owned());
                }
            }
        }
    }
    names
}

/// The adapters DXGI lists, by name.
#[cfg(windows)]
fn names() -> Vec<String> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};

    // SAFETY: as in `read` above: COM objects the `windows` crate counts
    // references for, each call's result checked.
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else {
            return Vec::new();
        };
        (0..)
            .map_while(|index| factory.EnumAdapters1(index).ok())
            .filter_map(|adapter| adapter.GetDesc1().ok())
            .map(|description| {
                String::from_utf16_lossy(&description.Description)
                    .trim_end_matches('\0')
                    .to_owned()
            })
            .collect()
    }
}

/// macOS has none of the engines, and elsewhere there is no list to read.
#[cfg(not(any(target_os = "linux", windows)))]
fn names() -> Vec<String> {
    Vec::new()
}

/// The kernel's view of the display devices.
#[cfg(target_os = "linux")]
fn read() -> Option<Vec<u32>> {
    let vendors: Vec<u32> = std::fs::read_dir("/sys/class/drm")
        .ok()?
        .flatten()
        .filter_map(|card| std::fs::read_to_string(card.path().join("device/vendor")).ok())
        .filter_map(|vendor| u32::from_str_radix(vendor.trim().trim_start_matches("0x"), 16).ok())
        .collect();
    // Nothing found is no answer: a sandbox that hides the devices, say.
    (!vendors.is_empty()).then_some(vendors)
}

/// The adapters DXGI lists. The software renderer is among them, under
/// Microsoft's id, which no engine runs on.
#[cfg(windows)]
fn read() -> Option<Vec<u32>> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};

    // SAFETY: DXGI's factory and adapters are COM objects the `windows`
    // crate counts references for; each call's result is checked, and the
    // listing ends at the first index DXGI has no adapter for.
    let vendors: Vec<u32> = unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
        (0..)
            .map_while(|index| factory.EnumAdapters1(index).ok())
            .filter_map(|adapter| adapter.GetDesc1().ok())
            .map(|description| description.VendorId)
            .collect()
    };
    (!vendors.is_empty()).then_some(vendors)
}

/// macOS has none of the engines, and elsewhere there is no list to read.
#[cfg(not(any(target_os = "linux", windows)))]
fn read() -> Option<Vec<u32>> {
    None
}

/// A piece of the system an Intel or AMD GPU needs and this machine lacks,
/// which otherwise fails without a word: mpv falls back to decoding on the
/// CPU, OpenVINO finds no GPU. Linux outside Flatpak only — Windows' and
/// macOS's drivers carry both, and the Flatpak's runtime brings its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Missing {
    /// VA-API's driver for Intel: hardware decoding.
    IntelMediaDriver,
    /// VA-API's driver for AMD, Mesa's: hardware decoding.
    MesaVaDriver,
    /// Intel's OpenCL runtime, which OpenVINO runs the networks through.
    IntelComputeRuntime,
}

impl Missing {
    /// What to install, for a line on screen.
    pub fn advice(self, ru: bool) -> &'static str {
        match (self, ru) {
            (Self::IntelMediaDriver, true) => {
                "Видео декодируется процессором: нет драйвера VA-API. Установите \
                 intel-media-driver (в Debian и Ubuntu — intel-media-va-driver)"
            }
            (Self::IntelMediaDriver, false) => {
                "Decoding on the CPU: no VA-API driver. Install intel-media-driver \
                 (intel-media-va-driver on Debian and Ubuntu)"
            }
            (Self::MesaVaDriver, true) => {
                "Видео декодируется процессором: нет драйвера VA-API. Установите \
                 mesa-va-drivers (в Arch он входит в mesa)"
            }
            (Self::MesaVaDriver, false) => {
                "Decoding on the CPU: no VA-API driver. Install mesa-va-drivers \
                 (part of mesa on Arch)"
            }
            (Self::IntelComputeRuntime, true) => {
                "OpenVINO не видит видеокарту: нет OpenCL от Intel. Установите \
                 intel-compute-runtime (в Debian и Ubuntu — intel-opencl-icd)"
            }
            (Self::IntelComputeRuntime, false) => {
                "OpenVINO cannot see the GPU: no Intel OpenCL. Install \
                 intel-compute-runtime (intel-opencl-icd on Debian and Ubuntu)"
            }
        }
    }
}

/// The VA-API driver this machine's GPU lacks for hardware decoding, if any.
pub fn missing_vaapi_driver() -> Option<Missing> {
    static MISSING: OnceLock<Option<Missing>> = OnceLock::new();
    *MISSING.get_or_init(|| {
        if !host_drivers() {
            return None;
        }
        // Where libva looks: its variable, or the distributions' folders.
        let dirs: Vec<std::path::PathBuf> = match std::env::var_os("LIBVA_DRIVERS_PATH") {
            Some(paths) => std::env::split_paths(&paths).collect(),
            None => [
                "/usr/lib/dri",
                "/usr/lib64/dri",
                "/usr/lib/x86_64-linux-gnu/dri",
                "/usr/lib/aarch64-linux-gnu/dri",
                "/usr/local/lib/dri",
            ]
            .map(std::path::PathBuf::from)
            .to_vec(),
        };
        vaapi_driver_missing(vendors()?, |file| {
            dirs.iter().any(|dir| dir.join(file).exists())
        })
    })
}

/// Intel's OpenCL runtime, when OpenVINO will look for an Intel GPU and
/// find none without it.
pub fn missing_compute_runtime() -> Option<Missing> {
    static MISSING: OnceLock<Option<Missing>> = OnceLock::new();
    *MISSING.get_or_init(|| {
        if !host_drivers() || !vendors()?.contains(&INTEL) {
            return None;
        }
        // The loader finds a runtime through the files in this folder;
        // Intel's is named after it.
        let registered = std::fs::read_dir(Path::new("/etc/OpenCL/vendors"))
            .into_iter()
            .flatten()
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().starts_with("intel"));
        (!registered).then_some(Missing::IntelComputeRuntime)
    })
}

/// Whether the GPU's drivers are the host's own, installed beside the
/// program: on Linux, outside Flatpak.
fn host_drivers() -> bool {
    cfg!(target_os = "linux") && std::env::var_os("FLATPAK_ID").is_none()
}

fn vaapi_driver_missing(vendors: &[u32], installed: impl Fn(&str) -> bool) -> Option<Missing> {
    // An NVIDIA card decodes through NVDEC, Intel graphics beside it or not.
    if vendors.contains(&NVIDIA) {
        return None;
    }
    // iHD for Broadwell and newer, i965 before it.
    if vendors.contains(&INTEL) && !installed("iHD_drv_video.so") && !installed("i965_drv_video.so")
    {
        return Some(Missing::IntelMediaDriver);
    }
    if vendors.contains(&AMD) && !installed("radeonsi_drv_video.so") {
        return Some(Missing::MesaVaDriver);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_va_api_driver_is_named_for_the_gpu_that_needs_it() {
        let none = |_: &str| false;
        assert_eq!(
            vaapi_driver_missing(&[INTEL], none),
            Some(Missing::IntelMediaDriver)
        );
        assert_eq!(
            vaapi_driver_missing(&[AMD], none),
            Some(Missing::MesaVaDriver)
        );
        assert_eq!(vaapi_driver_missing(&[INTEL, NVIDIA], none), None);
        assert_eq!(
            vaapi_driver_missing(&[INTEL], |file| file == "iHD_drv_video.so"),
            None
        );
        assert_eq!(
            vaapi_driver_missing(&[AMD], |file| file == "radeonsi_drv_video.so"),
            None
        );
    }

    #[test]
    fn a_2060_is_small_where_a_3080_is_large() {
        assert_eq!(tier_of("NVIDIA GeForce RTX 2060"), Tier::Small);
        assert_eq!(tier_of("NVIDIA GeForce RTX 3050"), Tier::Small);
        assert_eq!(tier_of("NVIDIA GeForce RTX 4060"), Tier::Small);
        assert_eq!(tier_of("NVIDIA GeForce RTX 3060 Ti"), Tier::Small);
        assert_eq!(tier_of("NVIDIA GeForce RTX 3070"), Tier::Medium);
        assert_eq!(tier_of("NVIDIA GeForce RTX 4070"), Tier::Medium);
        assert_eq!(tier_of("NVIDIA GeForce RTX 3080"), Tier::Large);
        assert_eq!(tier_of("NVIDIA GeForce RTX 4070 Ti SUPER"), Tier::Large);
        assert_eq!(tier_of("NVIDIA GeForce RTX 4090"), Tier::Large);
        assert_eq!(tier_of("NVIDIA GeForce GTX 1660 SUPER"), Tier::Small);
        assert_eq!(tier_of("NVIDIA GeForce RTX 4070 Laptop GPU"), Tier::Small);
        // AMD and Intel have no measured ceiling yet; the smallest is safe.
        assert_eq!(tier_of("AMD Radeon RX 7900 XTX"), Tier::Small);
        assert_eq!(tier_of("AMD Radeon RX 6700 XT"), Tier::Small);
    }
}
