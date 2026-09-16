// SPDX-License-Identifier: GPL-3.0-or-later

//! Anime4K upscaling presets.
//!
//! Anime4K is a set of GLSL shaders published upstream at
//! <https://github.com/bloc97/Anime4K> under the MIT licence, and mpv loads
//! them natively through `glsl-shaders`. There is nothing to reimplement: a
//! preset is just an ordered list of shader files.
//!
//! The official Anixart player ships exactly these files. We take them from
//! upstream instead, which is both cleaner and identical in result.
//!
//! # Chains
//!
//! Anime4K's own documentation calls the useful combinations "modes". The
//! presets here follow Mode A — restore, then upscale — at three sizes, which
//! is the chain meant for typical anime sources rather than for heavily
//! degraded ones.
//!
//! Shaders are not vendored in this repository; [`crate::PlayerConfig`] points
//! at a directory holding them. See `player/shaders/README.md`.

/// How much work to spend on upscaling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UpscalePreset {
    /// No shaders. mpv's own scaler still applies.
    #[default]
    Off,
    /// Cheapest chain. Sensible on integrated graphics.
    Fast,
    /// The usual choice on a discrete GPU.
    Balanced,
    /// Heaviest chain; wants a capable GPU at 1080p and above.
    Quality,
}

impl UpscalePreset {
    /// Every preset, in increasing cost, for building a settings menu.
    pub const ALL: [Self; 4] = [Self::Off, Self::Fast, Self::Balanced, Self::Quality];

    /// Shader files to load, in order, or `None` when the preset loads none.
    ///
    /// Order matters: restoration runs before upscaling, and
    /// `Clamp_Highlights` must come first so the later passes see the
    /// untouched highlights.
    #[must_use]
    pub fn shader_chain(self) -> Option<&'static [&'static str]> {
        Some(match self {
            Self::Off => return None,
            Self::Fast => &[
                "Anime4K_Clamp_Highlights.glsl",
                "Anime4K_Restore_CNN_S.glsl",
                "Anime4K_Upscale_CNN_x2_S.glsl",
            ],
            Self::Balanced => &[
                "Anime4K_Clamp_Highlights.glsl",
                "Anime4K_Restore_CNN_M.glsl",
                "Anime4K_Upscale_CNN_x2_M.glsl",
                "Anime4K_AutoDownscalePre_x2.glsl",
            ],
            Self::Quality => &[
                "Anime4K_Clamp_Highlights.glsl",
                "Anime4K_Restore_CNN_L.glsl",
                "Anime4K_Upscale_CNN_x2_L.glsl",
                "Anime4K_AutoDownscalePre_x2.glsl",
                "Anime4K_Restore_CNN_S.glsl",
            ],
        })
    }

    /// Name for logs and settings UI.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Fast => "Fast",
            Self::Balanced => "Balanced",
            Self::Quality => "Quality",
        }
    }

    /// Every shader file any preset can ask for, for validating an install.
    #[must_use]
    pub fn required_files() -> Vec<&'static str> {
        let mut files: Vec<&'static str> = Self::ALL
            .iter()
            .filter_map(|p| p.shader_chain())
            .flatten()
            .copied()
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
    pub fn missing_from(dir: &std::path::Path) -> Vec<&'static str> {
        Self::required_files()
            .into_iter()
            .filter(|file| !dir.join(file).is_file())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_loads_nothing() {
        assert!(UpscalePreset::Off.shader_chain().is_none());
    }

    #[test]
    fn every_other_preset_loads_shaders() {
        for preset in UpscalePreset::ALL
            .iter()
            .filter(|p| **p != UpscalePreset::Off)
        {
            let chain = preset.shader_chain().expect("a chain");
            assert!(!chain.is_empty(), "{} has an empty chain", preset.name());
        }
    }

    #[test]
    fn highlights_are_clamped_before_anything_else() {
        for preset in UpscalePreset::ALL {
            if let Some(chain) = preset.shader_chain() {
                assert_eq!(
                    chain[0],
                    "Anime4K_Clamp_Highlights.glsl",
                    "{} clamps too late",
                    preset.name()
                );
            }
        }
    }

    #[test]
    fn restoration_precedes_upscaling() {
        for preset in UpscalePreset::ALL {
            let Some(chain) = preset.shader_chain() else {
                continue;
            };
            let restore = chain.iter().position(|f| f.contains("Restore"));
            let upscale = chain.iter().position(|f| f.contains("Upscale_CNN"));
            if let (Some(restore), Some(upscale)) = (restore, upscale) {
                assert!(restore < upscale, "{} upscales first", preset.name());
            }
        }
    }

    #[test]
    fn cost_grows_with_the_preset() {
        let size = |p: UpscalePreset| p.shader_chain().map_or(0, <[&str]>::len);
        assert!(size(UpscalePreset::Fast) <= size(UpscalePreset::Balanced));
        assert!(size(UpscalePreset::Balanced) <= size(UpscalePreset::Quality));
    }

    #[test]
    fn required_files_are_deduplicated() {
        let files = UpscalePreset::required_files();
        let mut unique = files.clone();
        unique.dedup();
        assert_eq!(files, unique);
        assert!(files.contains(&"Anime4K_Clamp_Highlights.glsl"));
    }

    #[test]
    fn an_empty_directory_is_reported_as_fully_missing() {
        let dir = std::path::Path::new("/nonexistent-shader-dir");
        assert_eq!(
            UpscalePreset::missing_from(dir),
            UpscalePreset::required_files()
        );
    }
}
