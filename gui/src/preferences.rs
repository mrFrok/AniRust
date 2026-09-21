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
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub appearance: Appearance,
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
}
