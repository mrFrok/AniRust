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
}
