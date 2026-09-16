// SPDX-License-Identifier: GPL-3.0-or-later

//! Remembering where an episode was left off.
//!
//! The server knows this for a signed-in account, and a signed-in account is
//! where it belongs — but the API has no endpoint for writing a position back,
//! only for marking an episode watched. So the exact second lives here, on
//! disk, and is merged with whatever the account already says.
//!
//! Keyed by release and episode number rather than by source: a viewer who
//! switches voice-over part-way through is still in the same episode, and
//! starting them over would be the wrong answer to a question they did not ask.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// A position counts as finished this close to the end.
///
/// Endings run credits, and nobody sits through them to earn a tick. Anything
/// past this point is an episode watched.
const FINISHED_FRACTION: f64 = 0.92;

/// Below this, there is nothing worth resuming — that is a false start, not a
/// place in an episode.
const MINIMUM_RESUME: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Entry {
    /// Where playback was, in milliseconds.
    #[serde(default)]
    pub position_ms: i64,
    /// Watched to the end.
    #[serde(default)]
    pub finished: bool,
}

impl Entry {
    /// Where to resume, or nothing when the episode is finished or barely
    /// started.
    #[must_use]
    pub fn resume_at(&self) -> Option<Duration> {
        if self.finished {
            return None;
        }
        let at = Duration::from_millis(u64::try_from(self.position_ms).ok()?);
        (at >= MINIMUM_RESUME).then_some(at)
    }
}

/// Everything this machine remembers about what has been watched.
#[derive(Default)]
pub struct Store {
    path: Option<PathBuf>,
    entries: HashMap<String, Entry>,
    /// Whether anything has changed since the last write.
    dirty: bool,
}

impl Store {
    /// Loads the store, or starts an empty one.
    ///
    /// A store that cannot be read is not an error worth stopping for: the
    /// client works perfectly well without knowing where you were, and losing
    /// that is better than refusing to start.
    #[must_use]
    pub fn load() -> Self {
        let Some(path) = Self::default_path() else {
            tracing::debug!("no data directory; watch positions will not be kept");
            return Self::default();
        };

        let entries = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|error| {
                tracing::warn!(%error, path = %path.display(), "ignoring an unreadable store");
                HashMap::new()
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "could not read the store");
                HashMap::new()
            }
        };

        Self {
            path: Some(path),
            entries,
            dirty: false,
        }
    }

    fn default_path() -> Option<PathBuf> {
        Some(dirs::data_dir()?.join("anirust").join("progress.json"))
    }

    fn key(release_id: i64, position: i32) -> String {
        format!("{release_id}/{position}")
    }

    #[must_use]
    pub fn get(&self, release_id: i64, position: i32) -> Option<Entry> {
        self.entries.get(&Self::key(release_id, position)).copied()
    }

    /// Records where an episode is now.
    ///
    /// Returns whether this crossed into "finished", which is the moment worth
    /// telling the server about.
    pub fn record(
        &mut self,
        release_id: i64,
        position: i32,
        at: Duration,
        duration: Option<Duration>,
    ) -> bool {
        let finished = duration.is_some_and(|total| {
            total > Duration::ZERO && at.as_secs_f64() >= total.as_secs_f64() * FINISHED_FRACTION
        });

        let entry = self
            .entries
            .entry(Self::key(release_id, position))
            .or_default();
        let newly_finished = finished && !entry.finished;

        entry.position_ms = i64::try_from(at.as_millis()).unwrap_or(i64::MAX);
        entry.finished |= finished;
        self.dirty = true;

        newly_finished
    }

    /// Writes the store out, if anything changed.
    pub fn flush(&mut self) {
        if !self.dirty {
            return;
        }
        let Some(path) = &self.path else { return };

        if let Err(error) = write_json(path, &self.entries) {
            tracing::warn!(%error, path = %path.display(), "could not save watch positions");
            return;
        }
        self.dirty = false;
    }
}

fn write_json(path: &PathBuf, entries: &HashMap<String, Entry>) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Through a temporary file: a half-written store is worse than none, and
    // the process can be killed at any point while a video is playing.
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(entries)?)?;
    std::fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_few_seconds_in_is_not_a_place_to_resume_from() {
        let entry = Entry {
            position_ms: 5_000,
            finished: false,
        };
        assert_eq!(entry.resume_at(), None);
    }

    #[test]
    fn a_real_position_is_offered_back() {
        let entry = Entry {
            position_ms: 500_000,
            finished: false,
        };
        assert_eq!(entry.resume_at(), Some(Duration::from_secs(500)));
    }

    #[test]
    fn a_finished_episode_has_nothing_to_resume() {
        let entry = Entry {
            position_ms: 1_400_000,
            finished: true,
        };
        assert_eq!(entry.resume_at(), None);
    }

    #[test]
    fn the_credits_count_as_finished() {
        let mut store = Store::default();
        let duration = Some(Duration::from_secs(1_440));

        assert!(!store.record(1, 1, Duration::from_secs(700), duration));
        assert!(
            store.record(1, 1, Duration::from_secs(1_400), duration),
            "past the threshold is finished"
        );
        assert!(
            !store.record(1, 1, Duration::from_secs(1_430), duration),
            "and only crosses over once"
        );
        assert!(store.get(1, 1).is_some_and(|e| e.finished));
    }

    #[test]
    fn an_episode_of_unknown_length_never_finishes_by_itself() {
        let mut store = Store::default();
        assert!(!store.record(1, 1, Duration::from_secs(9_000), None));
        assert!(store.get(1, 1).is_some_and(|e| !e.finished));
    }

    #[test]
    fn episodes_of_different_releases_do_not_collide() {
        let mut store = Store::default();
        store.record(1, 5, Duration::from_secs(100), None);
        store.record(2, 5, Duration::from_secs(200), None);

        assert_eq!(store.get(1, 5).map(|e| e.position_ms), Some(100_000));
        assert_eq!(store.get(2, 5).map(|e| e.position_ms), Some(200_000));
    }
}
