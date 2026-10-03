// SPDX-License-Identifier: GPL-3.0-or-later

//! Anime4K upscaling presets.
//!
//! Anime4K, <https://github.com/bloc97/Anime4K>, MIT, is a set of GLSL
//! restoration and upscaling networks at five sizes, S to UL, that mpv loads
//! natively through `glsl-shaders`. There is nothing to reimplement: a preset
//! is an ordered list of files.
//!
//! # Chains
//!
//! A preset is a [mode](UpscaleMode) — which recipe — and a
//! [quality](UpscaleQuality) — how big a network runs it. The Anime4K modes
//! are the recipes from Anime4K's own instructions for mpv, at sizes chosen
//! for desktop GPUs: the official Android player stops at L, and this goes on
//! to VL and UL.
//!
//! Two of the UL networks are left out: `Restore_CNN_UL` and its soft twin
//! need more varying variables than OpenGL allows a shader (31), and mpv
//! renders through OpenGL here, so they fail to link on every GPU. Ultra
//! restores with VL instead and keeps UL for the upscale, which links fine.
//!
//! ArtCNN, a cleaner doubler, was tried and left out for the same kind of
//! reason: its compute shaders build only under Vulkan, which libmpv's render
//! API does not offer.
//!
//! Every Anime4K chain upscales twice. The first doubling runs the big
//! network; the auto-downscale passes then shrink the picture if it overshot
//! the screen, and a second, smaller doubling runs only if it still falls
//! short. So 1080p reaches 4K in one network pass, and 720p reaches it in two
//! rather than one pass and a stretch. Each pass decides for itself from the
//! sizes involved, so a chain costs nothing it does not need.
//!
//! The shaders are vendored in `player/shaders/` and embedded in the binary,
//! then written to a cache directory the first time they are needed. That
//! costs a few megabytes and buys a feature that works with no setup and no
//! guessing where an install put its data files.

use std::path::Path;

/// A file name paired with the file, read at compile time.
macro_rules! embed {
    ($name:literal) => {
        ($name, include_str!(concat!("../shaders/", $name)))
    };
}

/// The vendored shaders, embedded so the feature needs no installation step.
///
/// Licence: `player/shaders/LICENSE-Anime4K`.
const EMBEDDED: &[(&str, &str)] = &[
    embed!("Anime4K_Clamp_Highlights.glsl"),
    embed!("Anime4K_AutoDownscalePre_x2.glsl"),
    embed!("Anime4K_AutoDownscalePre_x4.glsl"),
    embed!("Anime4K_Restore_CNN_S.glsl"),
    embed!("Anime4K_Restore_CNN_M.glsl"),
    embed!("Anime4K_Restore_CNN_L.glsl"),
    embed!("Anime4K_Restore_CNN_VL.glsl"),
    embed!("Anime4K_Restore_CNN_Soft_S.glsl"),
    embed!("Anime4K_Restore_CNN_Soft_M.glsl"),
    embed!("Anime4K_Restore_CNN_Soft_L.glsl"),
    embed!("Anime4K_Restore_CNN_Soft_VL.glsl"),
    embed!("Anime4K_Upscale_CNN_x2_S.glsl"),
    embed!("Anime4K_Upscale_CNN_x2_M.glsl"),
    embed!("Anime4K_Upscale_CNN_x2_L.glsl"),
    embed!("Anime4K_Upscale_CNN_x2_VL.glsl"),
    embed!("Anime4K_Upscale_CNN_x2_UL.glsl"),
    embed!("Anime4K_Upscale_Denoise_CNN_x2_S.glsl"),
    embed!("Anime4K_Upscale_Denoise_CNN_x2_M.glsl"),
    embed!("Anime4K_Upscale_Denoise_CNN_x2_L.glsl"),
    embed!("Anime4K_Upscale_Denoise_CNN_x2_VL.glsl"),
    embed!("Anime4K_Upscale_Denoise_CNN_x2_UL.glsl"),
];

/// Writes the embedded shaders to `dir`, creating it if needed.
///
/// Existing files are rewritten only when their contents differ, so upgrading
/// the application refreshes them while a normal start touches nothing.
pub fn install_to(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;

    for (name, contents) in EMBEDDED {
        let path = dir.join(name);
        let current = std::fs::read_to_string(&path).ok();
        if current.as_deref() != Some(*contents) {
            std::fs::write(&path, contents)?;
            tracing::debug!(shader = name, "shader written");
        }
    }
    Ok(())
}

/// Where the shaders live: the user's cache directory, falling back to a
/// temporary one when the platform has no cache directory.
#[must_use]
pub fn default_dir() -> std::path::PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("anirust")
        .join("shaders")
}

/// Which recipe sharpens the picture.
///
/// Anime4K's modes are named for the kind of source they suit, and picking
/// the wrong one is a matter of taste, not of breakage: A on a soft source
/// looks fine, C on a sharp one looks a little smooth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UpscaleMode {
    /// No shaders. mpv's own scaler still applies.
    #[default]
    Off,
    /// Restore, then upscale. For sources that are blurry from having been
    /// upscaled once already — most anime released at 1080p.
    A,
    /// A softer restore. For sources with ringing and aliasing from being
    /// downscaled — typically 720p.
    B,
    /// Upscale and denoise, no restore. For sources that are already sharp
    /// at their own size, or low resolution ones — 480p.
    C,
    /// A with a second restore after the first upscale. Sharper still, for
    /// very blurry sources.
    AA,
    /// B with a second soft restore after the downscale.
    BB,
    /// C with a restore after the downscale.
    CA,
}

impl UpscaleMode {
    /// Every mode, in the order a menu lists them.
    pub const ALL: [Self; 7] = [
        Self::Off,
        Self::A,
        Self::B,
        Self::C,
        Self::AA,
        Self::BB,
        Self::CA,
    ];

    /// Short name for menus and logs: Anime4K's own.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
            Self::AA => "A+A",
            Self::BB => "B+B",
            Self::CA => "C+A",
        }
    }
}

/// How big a network runs the mode, and so how much it asks of the GPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UpscaleQuality {
    /// Anime4K M then S. Integrated graphics and older cards.
    Fast,
    /// Anime4K L then M. A mid-range discrete card.
    #[default]
    High,
    /// Anime4K VL then M — Anime4K's own choice for high-end GPUs.
    Max,
    /// Anime4K VL restore, UL upscale, then L. The heaviest that links; a
    /// recent high-end card.
    Ultra,
}

impl UpscaleQuality {
    /// Every quality, in increasing cost.
    pub const ALL: [Self; 4] = [Self::Fast, Self::High, Self::Max, Self::Ultra];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Fast => "Fast",
            Self::High => "High",
            Self::Max => "Max",
            Self::Ultra => "Ultra",
        }
    }

    /// Network sizes: the restore and the upscale of the first doubling,
    /// and the passes after it.
    fn sizes(self) -> Sizes {
        let (restore, upscale, then) = match self {
            Self::Fast => (Size::M, Size::M, Size::S),
            Self::High => (Size::L, Size::L, Size::M),
            Self::Max => (Size::VL, Size::VL, Size::M),
            // Restore UL does not link under OpenGL; see the module notes.
            Self::Ultra => (Size::VL, Size::UL, Size::L),
        };
        Sizes {
            restore,
            upscale,
            then,
        }
    }
}

/// How much work to spend on upscaling: a recipe at a size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UpscalePreset {
    pub mode: UpscaleMode,
    pub quality: UpscaleQuality,
}

/// Anime4K's network sizes. Restoration stops at VL: see the module notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Size {
    S,
    M,
    L,
    VL,
    UL,
}

struct Sizes {
    restore: Size,
    upscale: Size,
    then: Size,
}

const CLAMP: &str = "Anime4K_Clamp_Highlights.glsl";
const DOWNSCALE_X2: &str = "Anime4K_AutoDownscalePre_x2.glsl";
const DOWNSCALE_X4: &str = "Anime4K_AutoDownscalePre_x4.glsl";

fn restore(size: Size) -> &'static str {
    match size {
        Size::S => "Anime4K_Restore_CNN_S.glsl",
        Size::M => "Anime4K_Restore_CNN_M.glsl",
        Size::L => "Anime4K_Restore_CNN_L.glsl",
        Size::VL | Size::UL => "Anime4K_Restore_CNN_VL.glsl",
    }
}

fn restore_soft(size: Size) -> &'static str {
    match size {
        Size::S => "Anime4K_Restore_CNN_Soft_S.glsl",
        Size::M => "Anime4K_Restore_CNN_Soft_M.glsl",
        Size::L => "Anime4K_Restore_CNN_Soft_L.glsl",
        Size::VL | Size::UL => "Anime4K_Restore_CNN_Soft_VL.glsl",
    }
}

fn upscale(size: Size) -> &'static str {
    match size {
        Size::S => "Anime4K_Upscale_CNN_x2_S.glsl",
        Size::M => "Anime4K_Upscale_CNN_x2_M.glsl",
        Size::L => "Anime4K_Upscale_CNN_x2_L.glsl",
        Size::VL => "Anime4K_Upscale_CNN_x2_VL.glsl",
        Size::UL => "Anime4K_Upscale_CNN_x2_UL.glsl",
    }
}

fn upscale_denoise(size: Size) -> &'static str {
    match size {
        Size::S => "Anime4K_Upscale_Denoise_CNN_x2_S.glsl",
        Size::M => "Anime4K_Upscale_Denoise_CNN_x2_M.glsl",
        Size::L => "Anime4K_Upscale_Denoise_CNN_x2_L.glsl",
        Size::VL => "Anime4K_Upscale_Denoise_CNN_x2_VL.glsl",
        Size::UL => "Anime4K_Upscale_Denoise_CNN_x2_UL.glsl",
    }
}

impl UpscalePreset {
    /// No upscaling.
    pub const OFF: Self = Self {
        mode: UpscaleMode::Off,
        quality: UpscaleQuality::High,
    };

    #[must_use]
    pub fn new(mode: UpscaleMode, quality: UpscaleQuality) -> Self {
        Self { mode, quality }
    }

    /// Every preset that loads something, for validating an install.
    pub fn all() -> impl Iterator<Item = Self> {
        UpscaleMode::ALL.into_iter().flat_map(|mode| {
            UpscaleQuality::ALL
                .into_iter()
                .map(move |quality| Self::new(mode, quality))
        })
    }

    /// Shader files to load, in order, or `None` when the preset loads none.
    ///
    /// Order matters: `Clamp_Highlights` comes first so the later passes see
    /// the untouched highlights, restoration runs before the upscale it
    /// feeds, and the downscale passes sit between the two doublings.
    #[must_use]
    pub fn shader_chain(self) -> Option<Vec<&'static str>> {
        let Sizes {
            restore: first_restore,
            upscale: first_upscale,
            then,
        } = self.quality.sizes();
        Some(match self.mode {
            UpscaleMode::Off => return None,
            UpscaleMode::A => vec![
                CLAMP,
                restore(first_restore),
                upscale(first_upscale),
                DOWNSCALE_X2,
                DOWNSCALE_X4,
                upscale(then),
            ],
            UpscaleMode::B => vec![
                CLAMP,
                restore_soft(first_restore),
                upscale(first_upscale),
                DOWNSCALE_X2,
                DOWNSCALE_X4,
                upscale(then),
            ],
            UpscaleMode::C => vec![
                CLAMP,
                upscale_denoise(first_upscale),
                DOWNSCALE_X2,
                DOWNSCALE_X4,
                upscale(then),
            ],
            UpscaleMode::AA => vec![
                CLAMP,
                restore(first_restore),
                upscale(first_upscale),
                restore(then),
                DOWNSCALE_X2,
                DOWNSCALE_X4,
                upscale(then),
            ],
            UpscaleMode::BB => vec![
                CLAMP,
                restore_soft(first_restore),
                upscale(first_upscale),
                DOWNSCALE_X2,
                DOWNSCALE_X4,
                restore_soft(then),
                upscale(then),
            ],
            UpscaleMode::CA => vec![
                CLAMP,
                upscale_denoise(first_upscale),
                DOWNSCALE_X2,
                DOWNSCALE_X4,
                restore(then),
                upscale(then),
            ],
        })
    }

    /// Name for logs and captions: "A+A · Max", "Off".
    #[must_use]
    pub fn name(self) -> String {
        match self.mode {
            UpscaleMode::Off => UpscaleMode::Off.name().to_owned(),
            mode => format!("{} · {}", mode.name(), self.quality.name()),
        }
    }

    /// Every shader file any preset can ask for, for validating an install.
    #[must_use]
    pub fn required_files() -> Vec<&'static str> {
        let mut files: Vec<&'static str> = Self::all()
            .filter_map(Self::shader_chain)
            .flatten()
            .collect();
        files.sort_unstable();
        files.dedup();
        files
    }

    /// Which required shaders are missing from `dir`.
    ///
    /// Returned in the order [`required_files`](Self::required_files) lists
    /// them, so a message about an incomplete install reads predictably.
    #[must_use]
    pub fn missing_from(dir: &Path) -> Vec<&'static str> {
        Self::required_files()
            .into_iter()
            .filter(|file| !dir.join(file).is_file())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chains() -> impl Iterator<Item = (UpscalePreset, Vec<&'static str>)> {
        UpscalePreset::all().filter_map(|p| p.shader_chain().map(|c| (p, c)))
    }

    #[test]
    fn off_loads_nothing() {
        for quality in UpscaleQuality::ALL {
            assert!(
                UpscalePreset::new(UpscaleMode::Off, quality)
                    .shader_chain()
                    .is_none()
            );
        }
        assert!(UpscalePreset::OFF.shader_chain().is_none());
        assert_eq!(UpscalePreset::default(), UpscalePreset::OFF);
    }

    #[test]
    fn every_other_preset_loads_shaders() {
        let count = chains().filter(|(_, chain)| !chain.is_empty()).count();
        assert_eq!(
            count,
            (UpscaleMode::ALL.len() - 1) * UpscaleQuality::ALL.len()
        );
    }

    #[test]
    fn every_file_a_chain_names_is_embedded() {
        for file in UpscalePreset::required_files() {
            assert!(
                EMBEDDED.iter().any(|(name, _)| *name == file),
                "{file} is not embedded"
            );
        }
    }

    #[test]
    fn highlights_are_clamped_before_anything_else() {
        for (preset, chain) in chains() {
            assert_eq!(chain[0], CLAMP, "{} clamps too late", preset.name());
        }
    }

    /// A restore cleans the picture for the upscale after it; one with no
    /// upscale left to feed would sharpen the final picture instead.
    #[test]
    fn every_restore_feeds_a_later_upscale() {
        for (preset, chain) in chains() {
            for (at, _) in chain
                .iter()
                .enumerate()
                .filter(|(_, f)| f.contains("Restore"))
            {
                assert!(
                    chain[at..].iter().any(|f| f.contains("Upscale")),
                    "{} restores after its last upscale",
                    preset.name()
                );
            }
        }
    }

    /// The downscale passes sit between two doublings: that is what lets 720p
    /// reach 4K through two network passes instead of one and a stretch.
    #[test]
    fn anime4k_doubles_twice_around_the_downscale() {
        for (preset, chain) in chains() {
            let downscale = chain
                .iter()
                .position(|f| *f == DOWNSCALE_X2)
                .expect("a downscale pass");
            assert_eq!(chain[downscale + 1], DOWNSCALE_X4);
            let before = chain[..downscale]
                .iter()
                .filter(|f| f.contains("Upscale"))
                .count();
            let after = chain[downscale..]
                .iter()
                .filter(|f| f.contains("Upscale"))
                .count();
            assert_eq!((before, after), (1, 1), "{}", preset.name());
        }
    }

    #[test]
    fn the_heavier_quality_runs_the_bigger_network() {
        let first = |quality| {
            UpscalePreset::new(UpscaleMode::A, quality)
                .shader_chain()
                .expect("a chain")[1]
        };
        assert_eq!(first(UpscaleQuality::Fast), restore(Size::M));
        assert_eq!(first(UpscaleQuality::High), restore(Size::L));
        assert_eq!(first(UpscaleQuality::Max), restore(Size::VL));
        assert_eq!(first(UpscaleQuality::Ultra), restore(Size::VL));
        let upscale_of = |quality| {
            UpscalePreset::new(UpscaleMode::A, quality)
                .shader_chain()
                .expect("a chain")[2]
        };
        assert_eq!(upscale_of(UpscaleQuality::Ultra), upscale(Size::UL));
    }

    /// The UL restores do not link under OpenGL, so no chain may name them.
    #[test]
    fn no_chain_restores_with_ul() {
        for (preset, chain) in chains() {
            assert!(
                !chain
                    .iter()
                    .any(|f| f.starts_with("Anime4K_Restore") && f.contains("_UL")),
                "{} restores with UL",
                preset.name()
            );
        }
    }

    /// Anime4K's published recipe for high-end GPUs, Mode A+A, verbatim.
    #[test]
    fn max_a_plus_a_is_the_upstream_high_end_recipe() {
        let chain = UpscalePreset::new(UpscaleMode::AA, UpscaleQuality::Max)
            .shader_chain()
            .expect("a chain");
        assert_eq!(
            chain,
            [
                "Anime4K_Clamp_Highlights.glsl",
                "Anime4K_Restore_CNN_VL.glsl",
                "Anime4K_Upscale_CNN_x2_VL.glsl",
                "Anime4K_Restore_CNN_M.glsl",
                "Anime4K_AutoDownscalePre_x2.glsl",
                "Anime4K_AutoDownscalePre_x4.glsl",
                "Anime4K_Upscale_CNN_x2_M.glsl",
            ]
        );
    }

    #[test]
    fn soft_modes_restore_softly() {
        for quality in UpscaleQuality::ALL {
            for mode in [UpscaleMode::B, UpscaleMode::BB] {
                let chain = UpscalePreset::new(mode, quality)
                    .shader_chain()
                    .expect("a chain");
                assert!(
                    chain
                        .iter()
                        .filter(|f| f.contains("Restore"))
                        .all(|f| f.contains("Soft"))
                );
            }
        }
    }

    #[test]
    fn required_files_are_deduplicated() {
        let files = UpscalePreset::required_files();
        let mut unique = files.clone();
        unique.dedup();
        assert_eq!(files, unique);
        assert!(files.contains(&CLAMP));
    }

    #[test]
    fn an_empty_directory_is_reported_as_fully_missing() {
        let dir = Path::new("/nonexistent-shader-dir");
        assert_eq!(
            UpscalePreset::missing_from(dir),
            UpscalePreset::required_files()
        );
    }

    #[test]
    fn names_read_as_mode_and_quality() {
        assert_eq!(UpscalePreset::OFF.name(), "Off");
        assert_eq!(
            UpscalePreset::new(UpscaleMode::CA, UpscaleQuality::Ultra).name(),
            "C+A · Ultra"
        );
    }
}
