// SPDX-License-Identifier: GPL-3.0-or-later

//! Whose GPUs the machine has.
//!
//! Each neural engine runs on one maker's cards — TensorRT on NVIDIA's,
//! MIGraphX on AMD's, OpenVINO on Intel's — and finds nothing on the
//! others'. So the menu offers only the engines this machine can run, and
//! marks the one that suits it best. Asked once: the GPUs do not change
//! while the program runs.

use std::sync::OnceLock;

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
