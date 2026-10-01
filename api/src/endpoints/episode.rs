// SPDX-License-Identifier: GPL-3.0-or-later
//
// The playback chain: voice-overs, then their sources, then episodes.

use serde::Deserialize;

use crate::client::{Ack, Client};
use crate::error::Result;
use crate::models::{Channel, Dubber, Episode, EpisodeUpdate, Page, Source};

use super::PageablePayload;

#[derive(Deserialize)]
struct TypesPayload {
    #[serde(default)]
    types: Vec<Dubber>,
}

#[derive(Deserialize)]
struct SourcesPayload {
    #[serde(default)]
    sources: Vec<Source>,
}

#[derive(Deserialize)]
struct EpisodesPayload {
    #[serde(default)]
    episodes: Vec<Episode>,
}

/// Ordering for [`Client::episodes`], sent as `?sort=`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i32)]
pub enum EpisodeSort {
    /// Episode 1 first.
    #[default]
    Ascending = 0,
    /// Newest episode first.
    Descending = 1,
}

impl EpisodeSort {
    #[must_use]
    pub fn raw(self) -> i32 {
        self as i32
    }
}

// ---------------------------------------------------------------------------
// Playback chain: dubbers -> sources -> episodes
// ---------------------------------------------------------------------------

impl Client {
    /// Voice-over tracks available for a release.
    ///
    /// `GET episode/{release_id}`
    pub async fn dubbers(&self, release_id: i64) -> Result<Vec<Dubber>> {
        let payload: TypesPayload = self
            .send(self.get(format!("episode/{release_id}")).with_token())
            .await?;
        Ok(payload.types)
    }

    /// Hosts serving a given dubber — Kodik, Sibnet, and so on.
    ///
    /// `GET episode/{release_id}/{dubber_id}`. Takes no token.
    pub async fn sources(&self, release_id: i64, dubber_id: i64) -> Result<Vec<Source>> {
        let payload: SourcesPayload = self
            .send(self.get(format!("episode/{release_id}/{dubber_id}")))
            .await?;
        Ok(payload.sources)
    }

    /// Episodes for one dubber/source pair.
    ///
    /// `GET episode/{release_id}/{dubber_id}/{source_id}?sort=`
    ///
    /// Each [`Episode`] carries a `url` and an `iframe` flag saying whether
    /// that URL is a direct stream or an embed page needing an extractor.
    pub async fn episodes(
        &self,
        release_id: i64,
        dubber_id: i64,
        source_id: i64,
        sort: EpisodeSort,
    ) -> Result<Vec<Episode>> {
        let payload: EpisodesPayload = self
            .send(
                self.get(format!("episode/{release_id}/{dubber_id}/{source_id}"))
                    .query("sort", sort.raw())
                    .with_token(),
            )
            .await?;
        Ok(payload.episodes)
    }

    /// Marks an episode watched.
    ///
    /// `POST episode/watch/{release_id}/{source_id}/{position}` — POST, not
    /// GET as the unofficial spec claims.
    pub async fn mark_watched(&self, release_id: i64, source_id: i64, position: i32) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.post(format!("episode/watch/{release_id}/{source_id}/{position}"))
                    .with_token(),
            )
            .await?;
        Ok(())
    }

    /// Clears the watched flag for an episode.
    pub async fn mark_unwatched(
        &self,
        release_id: i64,
        source_id: i64,
        position: i32,
    ) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.post(format!(
                    "episode/unwatch/{release_id}/{source_id}/{position}"
                ))
                .with_token(),
            )
            .await?;
        Ok(())
    }

    /// Records a watch in the user's history, which is what drives "continue
    /// watching".
    ///
    /// `GET history/add/{release_id}/{source_id}/{position}`
    pub async fn history_add(&self, release_id: i64, source_id: i64, position: i32) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.get(format!("history/add/{release_id}/{source_id}/{position}"))
                    .with_token(),
            )
            .await?;
        Ok(())
    }

    /// Removes a release from the watch history.
    pub async fn history_delete(&self, release_id: i64) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.get(format!("history/delete/{release_id}"))
                    .with_token(),
            )
            .await?;
        Ok(())
    }

    /// Marks every episode of a source watched.
    ///
    /// `POST episode/watch/{release_id}/{source_id}` — the same call as for one
    /// episode, without the position.
    pub async fn mark_all_watched(&self, release_id: i64, source_id: i64) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.post(format!("episode/watch/{release_id}/{source_id}"))
                    .with_token(),
            )
            .await?;
        Ok(())
    }

    /// Clears the watched flag on every episode of a source.
    ///
    /// `POST episode/unwatch/{release_id}/{source_id}`
    pub async fn mark_all_unwatched(&self, release_id: i64, source_id: i64) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.post(format!("episode/unwatch/{release_id}/{source_id}"))
                    .with_token(),
            )
            .await?;
        Ok(())
    }

    /// One episode by its place in a source, without walking the chain to it.
    ///
    /// `GET episode/target/{release_id}/{source_id}/{position}`, anonymous.
    /// What a notification about a new episode links to.
    pub async fn episode_target(
        &self,
        release_id: i64,
        source_id: i64,
        position: i32,
    ) -> Result<Episode> {
        let payload: EpisodeTargetPayload = self
            .send(self.get(format!(
                "episode/target/{release_id}/{source_id}/{position}"
            )))
            .await?;
        Ok(payload.episode)
    }

    /// When a release gained episodes, newest first. Pages are 0-based.
    ///
    /// `GET episode/updates/{release_id}/{page}`, anonymous.
    pub async fn episode_updates(&self, release_id: i64, page: i32) -> Result<Page<EpisodeUpdate>> {
        let payload: PageablePayload<EpisodeUpdate> = self
            .send(self.get(format!("episode/updates/{release_id}/{page}")))
            .await?;
        Ok(payload.into())
    }

    /// Every voice-over the service knows, across all releases.
    ///
    /// `GET type/all`
    pub async fn all_dubbers(&self) -> Result<Vec<Dubber>> {
        self.require_token()?;
        let payload: TypesPayload = self.send(self.get("type/all").with_token()).await?;
        Ok(payload.types)
    }

    /// The channel a voice-over team publishes in, if it has one.
    ///
    /// `GET type/{dubber_id}/channel`
    pub async fn dubber_channel(&self, dubber_id: i64) -> Result<DubberChannel> {
        self.require_token()?;
        self.send(self.get(format!("type/{dubber_id}/channel")).with_token())
            .await
    }

    /// Makes a voice-over the one a release opens with, for this account.
    ///
    /// `GET type/pin/{release_id}/{dubber_id}`
    pub async fn dubber_pin(&self, release_id: i64, dubber_id: i64) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.get(format!("type/pin/{release_id}/{dubber_id}"))
                    .with_token(),
            )
            .await?;
        Ok(())
    }

    /// `GET type/unpin/{release_id}/{dubber_id}`
    pub async fn dubber_unpin(&self, release_id: i64, dubber_id: i64) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.get(format!("type/unpin/{release_id}/{dubber_id}"))
                    .with_token(),
            )
            .await?;
        Ok(())
    }

    /// Hides a voice-over team's channel widget from the episode list.
    ///
    /// `GET type/widget/hide/{dubber_id}?permanent=` — `permanent` hides it
    /// for good rather than until the next post.
    pub async fn dubber_widget_hide(&self, dubber_id: i64, permanent: bool) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.get(format!("type/widget/hide/{dubber_id}"))
                    .query("permanent", permanent)
                    .with_token(),
            )
            .await?;
        Ok(())
    }

    /// `GET type/widget/unhide/{dubber_id}`
    pub async fn dubber_widget_unhide(&self, dubber_id: i64) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.get(format!("type/widget/unhide/{dubber_id}"))
                    .with_token(),
            )
            .await?;
        Ok(())
    }
}

#[derive(Deserialize)]
struct EpisodeTargetPayload {
    #[serde(default)]
    episode: Episode,
}

/// A voice-over team's channel, and whether its widget should show.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DubberChannel {
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub channel: Option<Channel>,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub is_widget_eligible: bool,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub is_hidden_by_user: bool,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub are_widgets_hidden_globally: bool,
}
