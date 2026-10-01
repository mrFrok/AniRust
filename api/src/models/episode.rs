// SPDX-License-Identifier: GPL-3.0-or-later
//
// Voice-overs, their sources, and episodes.

use serde::{Deserialize, Serialize};

use crate::serde_ext::nullable;

/// A voice-over track for a release. The app calls this a `Type` and returns
/// it under a `types` key; "dubber" is used here because `Type` is unusable as
/// a Rust name.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Dubber {
    #[serde(deserialize_with = "nullable")]
    pub id: i64,
    #[serde(deserialize_with = "nullable")]
    pub name: String,
    #[serde(deserialize_with = "nullable")]
    pub icon: String,
    /// Free-text credit line for the people behind the dub.
    #[serde(deserialize_with = "nullable")]
    pub workers: String,
    #[serde(alias = "episode_count", deserialize_with = "nullable")]
    pub episodes_count: i64,
    /// Subtitles rather than a dub.
    #[serde(deserialize_with = "nullable")]
    pub is_sub: bool,
    #[serde(deserialize_with = "nullable")]
    pub pinned: bool,
    #[serde(deserialize_with = "nullable")]
    pub quality: i32,
    #[serde(deserialize_with = "nullable")]
    pub view_count: i64,
}

/// A player/host serving a given dubber's episodes — Kodik, Sibnet, and so on.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Source {
    #[serde(deserialize_with = "nullable")]
    pub id: i64,
    #[serde(deserialize_with = "nullable")]
    pub name: String,
    #[serde(alias = "episode_count", deserialize_with = "nullable")]
    pub episodes_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub quality: i32,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Episode {
    #[serde(deserialize_with = "nullable")]
    pub id: i64,
    /// 1-based index within the source, and the value the `watch` and
    /// `history` endpoints expect as `position`.
    #[serde(deserialize_with = "nullable")]
    pub position: i32,
    #[serde(deserialize_with = "nullable")]
    pub name: String,
    /// Either a direct media URL or an embed page, depending on [`Self::iframe`].
    #[serde(deserialize_with = "nullable")]
    pub url: String,
    /// `true` means [`Self::url`] is an embed page that needs an extractor
    /// before it can be handed to a player.
    #[serde(deserialize_with = "nullable")]
    pub iframe: bool,
    #[serde(deserialize_with = "nullable")]
    pub quality: i32,
    /// Server-side resume point, in milliseconds.
    #[serde(deserialize_with = "nullable")]
    pub playback_position: i64,
    #[serde(deserialize_with = "nullable")]
    pub added_date: i64,
    /// Filler episode. The spec calls this `is_filter`, which is a misreading.
    #[serde(alias = "is_filter", deserialize_with = "nullable")]
    pub is_filler: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_watched: bool,
    #[serde(deserialize_with = "nullable")]
    pub source_id: i64,
    #[serde(deserialize_with = "nullable")]
    pub release_id: i64,
}

impl Episode {
    /// Whether the URL can go straight to a player without an extractor.
    #[must_use]
    pub fn is_direct(&self) -> bool {
        !self.iframe && !self.url.is_empty()
    }

    /// Resume point, or `None` when the episode has not been started.
    #[must_use]
    pub fn resume_at(&self) -> Option<std::time::Duration> {
        u64::try_from(self.playback_position)
            .ok()
            .filter(|&ms| ms > 0)
            .map(std::time::Duration::from_millis)
    }
}

/// When a release last gained an episode, and from which voice-over and host.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct EpisodeUpdate {
    #[serde(deserialize_with = "nullable")]
    pub last_episode_update_name: String,
    /// Seconds since the epoch.
    #[serde(deserialize_with = "nullable")]
    pub last_episode_update_date: i64,
    #[serde(deserialize_with = "nullable")]
    pub last_episode_type_update_id: i64,
    #[serde(deserialize_with = "nullable")]
    pub last_episode_type_update_name: String,
    #[serde(deserialize_with = "nullable")]
    pub last_episode_source_update_id: i64,
    #[serde(deserialize_with = "nullable")]
    pub last_episode_source_update_name: String,
}
