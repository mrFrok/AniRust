// SPDX-License-Identifier: GPL-3.0-or-later
//
// Releases: one at a time, searched for, and discovered.

use serde::Deserialize;

use crate::client::{Client, SEARCH_API_VERSION};
use crate::error::Result;
use crate::models::{Filter, Page, Release, SearchBy};

use super::PageablePayload;

#[derive(Deserialize)]
struct ReleasePayload {
    #[serde(default)]
    release: Release,
}

#[derive(Deserialize)]
struct ReleaseSearchPayload {
    #[serde(default)]
    releases: Vec<Release>,
}

// ---------------------------------------------------------------------------
// Releases, search, discovery
// ---------------------------------------------------------------------------

impl Client {
    /// A single release.
    ///
    /// `extended` asks the server to inline related and recommended releases;
    /// the app sets it when opening a release page.
    pub async fn release(&self, id: i64, extended: bool) -> Result<Release> {
        let payload: ReleasePayload = self
            .send(
                self.get(format!("release/{id}"))
                    .query("extended_mode", extended)
                    .with_token(),
            )
            .await?;
        Ok(payload.release)
    }

    pub async fn random_release(&self, extended: bool) -> Result<Release> {
        let payload: ReleasePayload = self
            .send(
                self.get("release/random")
                    .query("extended_mode", extended)
                    .with_token(),
            )
            .await?;
        Ok(payload.release)
    }

    /// Searches releases. Pages are 0-based.
    ///
    /// `POST search/releases/{page}` with `API-Version: v2`. The header is
    /// required — without it the endpoint answers in the older v1 shape.
    pub async fn search_releases(
        &self,
        query: &str,
        search_by: SearchBy,
        page: i32,
    ) -> Result<Vec<Release>> {
        let payload: ReleaseSearchPayload = self
            .send(
                self.post(format!("search/releases/{page}"))
                    .header("API-Version", SEARCH_API_VERSION)
                    .json(serde_json::json!({
                        "query": query,
                        "searchBy": search_by.raw(),
                    }))
                    .with_token(),
            )
            .await?;
        Ok(payload.releases)
    }

    /// Releases other people are watching right now.
    ///
    /// `POST discover/watching/{page}`
    pub async fn discover_watching(&self, page: i32) -> Result<Page<Release>> {
        let payload: PageablePayload<Release> = self
            .send(self.post(format!("discover/watching/{page}")).with_token())
            .await?;
        Ok(payload.into())
    }

    /// Personalised recommendations. Requires a token.
    ///
    /// `previous_page` is sent by the app to keep results stable while paging.
    pub async fn discover_recommendations(
        &self,
        page: i32,
        previous_page: i32,
    ) -> Result<Page<Release>> {
        self.require_token()?;
        let payload: PageablePayload<Release> = self
            .send(
                self.post(format!("discover/recommendations/{page}"))
                    .query("previous_page", previous_page)
                    .with_token(),
            )
            .await?;
        Ok(payload.into())
    }

    /// Most-discussed releases.
    pub async fn discover_discussing(&self) -> Result<Page<Release>> {
        let payload: PageablePayload<Release> = self
            .send(self.post("discover/discussing").with_token())
            .await?;
        Ok(payload.into())
    }

    /// The catalogue, filtered. Pages are 0-based.
    ///
    /// `POST filter/{page}` with the filter as a JSON body. No token needed,
    /// though one is sent when there is one: the answer then carries the
    /// account's own watched flags.
    pub async fn filter(&self, filter: &Filter, page: i32) -> Result<Page<Release>> {
        let payload: PageablePayload<Release> = self
            .send(
                self.post(format!("filter/{page}"))
                    // Serialising a filter cannot fail: every field is a
                    // scalar or a list of strings.
                    .json(serde_json::to_value(filter).unwrap_or_default())
                    .with_token(),
            )
            .await?;
        Ok(payload.into())
    }

    /// Releases related to the given one. Pages are 0-based.
    pub async fn related(&self, release_id: i64, page: i32) -> Result<Page<Release>> {
        let payload: PageablePayload<Release> = self
            .send(
                self.get(format!("related/{release_id}/{page}"))
                    .with_token(),
            )
            .await?;
        Ok(payload.into())
    }
}
