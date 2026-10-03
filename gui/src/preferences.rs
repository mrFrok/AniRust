// SPDX-License-Identifier: GPL-3.0-or-later

//! What the viewer has chosen about the application itself.
//!
//! Preferences rather than settings: the player's overlay already has a
//! `Settings`, and that one is what its controls are set to right now, which
//! is a different thing from what this machine should remember.
//!
//! A preference is not data the application produced, so this goes to the
//! config directory rather than beside the watch positions in the data one.
//! It is read once at startup and written when something changes, which is
//! rarely — there is no reason for it to be anything cleverer than a file.
//!
//! Preferences that cannot be read are preferences at their defaults. An
//! application that refuses to start because it disagrees with a file about
//! what colour it should be has its priorities wrong.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// How the application should look.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Appearance {
    /// Whatever the desktop is set to.
    System,
    Light,
    /// The default: a room being used to watch something is not a lit one.
    #[default]
    Dark,
    /// Dark, with the surfaces at true black — the pixels an OLED panel
    /// switches off rather than lights dimly.
    Amoled,
}

impl Appearance {
    /// The order the interface offers them in.
    ///
    /// The interface hands back a position in this list rather than a name:
    /// the control it is chosen with is a row of segments, and a segment knows
    /// its index and nothing else.
    pub const ALL: [Self; 4] = [Self::System, Self::Light, Self::Dark, Self::Amoled];

    /// Where this one sits in that row.
    #[must_use]
    pub fn index(self) -> i32 {
        Self::ALL
            .iter()
            .position(|&value| value == self)
            .and_then(|at| i32::try_from(at).ok())
            .unwrap_or(0)
    }

    /// The one at that position, or the default for a position there is none.
    #[must_use]
    pub fn at(index: i32) -> Self {
        usize::try_from(index)
            .ok()
            .and_then(|at| Self::ALL.get(at).copied())
            .unwrap_or_default()
    }
}

/// Everything this machine remembers about how the application should behave.
/// The accent colour, in the order its swatches are shown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Accent {
    #[default]
    Violet,
    Blue,
    Indigo,
    Teal,
    Green,
    Amber,
    Rose,
}

impl Accent {
    pub const ALL: [Self; 7] = [
        Self::Violet,
        Self::Blue,
        Self::Indigo,
        Self::Teal,
        Self::Green,
        Self::Amber,
        Self::Rose,
    ];

    #[must_use]
    pub fn index(self) -> i32 {
        Self::ALL
            .iter()
            .position(|&value| value == self)
            .and_then(|at| i32::try_from(at).ok())
            .unwrap_or(0)
    }

    #[must_use]
    pub fn at(index: i32) -> Self {
        usize::try_from(index)
            .ok()
            .and_then(|at| Self::ALL.get(at).copied())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub appearance: Appearance,
    pub accent: Accent,
    /// The window's surfaces see-through.
    pub translucent: bool,
    pub player: PlayerPreferences,
}

/// The player as it was last set, to open the next run the same way.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayerPreferences {
    /// Whether to keep the rest at all. Off, every run starts from the
    /// player's own defaults.
    pub remember: bool,
    pub speed: f64,
    pub volume: i64,
    /// Positions in the player's menus: the upscale mode and its quality.
    pub upscale_mode: usize,
    pub upscale_quality: usize,
    pub interpolation: bool,
    pub decoder: usize,
    /// Upscale to 4K whatever the window's size.
    pub force_4k: bool,
    /// Loudness evened out.
    pub normalize: bool,
    /// Position in the subtitle size menu.
    pub subtitle_scale: usize,
    /// The player's subtitle style over the styles ASS files bring.
    pub ass_override: bool,
    /// Position in the colour preset menu.
    pub picture: usize,
    /// Frame generation: 0 off, then ×2, 60, the screen's rate.
    pub frame_rate: usize,
    /// Which RIFE network: 0 fast, 1 quality.
    pub rife_model: usize,
    /// Lower the load by itself when frames start dropping.
    pub adaptive: bool,
    /// The engine for the networks: 0 Vulkan, 1 TensorRT.
    pub engine: usize,
    /// Real-ESRGAN doubling the picture first.
    pub neural_upscale: bool,
}

impl Default for PlayerPreferences {
    fn default() -> Self {
        Self {
            remember: true,
            speed: 1.0,
            volume: 100,
            upscale_mode: 0,
            upscale_quality: 1,
            interpolation: false,
            decoder: 0,
            force_4k: false,
            normalize: false,
            subtitle_scale: 1,
            ass_override: false,
            picture: 0,
            frame_rate: 0,
            rife_model: 0,
            adaptive: true,
            engine: 0,
            neural_upscale: false,
        }
    }
}

impl PlayerPreferences {
    /// What a run starts with: these, or the defaults when not remembered.
    #[must_use]
    pub fn in_force(self) -> Self {
        if self.remember {
            Self {
                speed: self.speed.clamp(0.25, 4.0),
                volume: self.volume.clamp(0, 150),
                ..self
            }
        } else {
            Self {
                remember: false,
                ..Self::default()
            }
        }
    }
}

impl Preferences {
    /// Loads them, or starts from the defaults.
    #[must_use]
    pub fn load() -> Self {
        let Some(path) = Self::default_path() else {
            tracing::debug!("no config directory; preferences will not be kept");
            return Self::default();
        };

        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|error| {
                tracing::warn!(%error, path = %path.display(), "ignoring unreadable preferences");
                Self::default()
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "could not read the preferences");
                Self::default()
            }
        }
    }

    /// Writes them out.
    pub fn save(self) {
        let Some(path) = Self::default_path() else {
            return;
        };

        if let Err(error) = write(&path, self) {
            tracing::warn!(%error, path = %path.display(), "could not save the preferences");
        }
    }

    fn default_path() -> Option<PathBuf> {
        Some(dirs::config_dir()?.join("anirust").join("settings.json"))
    }
}

fn write(path: &PathBuf, preferences: Preferences) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Through a temporary file, as the watch positions are: a half-written
    // file reads as corrupt on the next start, and starting with the defaults
    // is a worse answer than starting with what was chosen.
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(&preferences)?)?;
    std::fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_dark() {
        assert_eq!(Preferences::default().appearance, Appearance::Dark);
    }

    #[test]
    fn a_position_names_the_same_appearance_back() {
        for appearance in Appearance::ALL {
            assert_eq!(Appearance::at(appearance.index()), appearance);
        }
    }

    #[test]
    fn a_position_that_is_not_one_of_them_is_the_default() {
        assert_eq!(Appearance::at(-1), Appearance::default());
        assert_eq!(Appearance::at(99), Appearance::default());
    }

    #[test]
    fn preferences_survive_a_round_trip_through_the_file() {
        let written = serde_json::to_string(&Preferences {
            appearance: Appearance::Amoled,
            ..Preferences::default()
        })
        .expect("preferences serialise");
        assert!(written.contains("amoled"), "{written}");

        let read: Preferences = serde_json::from_str(&written).expect("preferences parse");
        assert_eq!(read.appearance, Appearance::Amoled);
    }

    #[test]
    fn a_file_missing_the_field_is_read_at_its_default() {
        let read: Preferences = serde_json::from_str("{}").expect("an empty object parses");
        assert_eq!(read.appearance, Appearance::default());
    }

    #[test]
    fn a_file_without_player_settings_remembers_by_default() {
        let read: Preferences = serde_json::from_str(r#"{"appearance":"light"}"#).unwrap();
        assert!(read.player.remember);
        assert!((read.player.speed - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn forgetting_starts_from_the_defaults() {
        let kept = PlayerPreferences {
            remember: false,
            speed: 2.0,
            volume: 40,
            ..PlayerPreferences::default()
        };
        let in_force = kept.in_force();
        assert!((in_force.speed - 1.0).abs() < f64::EPSILON);
        assert_eq!(in_force.volume, 100);
        assert!(!in_force.remember);
    }

    #[test]
    fn an_accent_names_the_same_place_back() {
        for accent in Accent::ALL {
            assert_eq!(Accent::at(accent.index()), accent);
        }
        assert_eq!(Accent::at(99), Accent::Violet);
    }
}
