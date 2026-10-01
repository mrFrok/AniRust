// SPDX-License-Identifier: GPL-3.0-or-later
//
// The playback chain: voice-overs, then their sources, then episodes.

use serde::Deserialize;

use crate::client::{Ack, Client};
use crate::error::Result;
use crate::models::{Dubber, Episode, Source};

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
}
