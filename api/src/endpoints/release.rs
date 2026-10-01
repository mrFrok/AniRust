// SPDX-License-Identifier: GPL-3.0-or-later
//
// Releases: one at a time, searched for, and discovered.

use serde::Deserialize;

use crate::client::{Ack, Client, SEARCH_API_VERSION};
use crate::error::Result;
use crate::models::{Filter, Page, ProfileList, Release, SearchBy, StreamingPlatform};

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

    /// The releases of one franchise. Pages are 0-based.
    ///
    /// `GET related/{related_id}/{page}` with `API-Version: v2`. Takes the
    /// franchise's id — [`Release::related`] — not a release's: the two are
    /// different numbers, and passing a release's id fetches whichever
    /// franchise happens to share it.
    pub async fn related(&self, related_id: i64, page: i32) -> Result<Page<Release>> {
        let payload: PageablePayload<Release> = self
            .send(
                self.get(format!("related/{related_id}/{page}"))
                    .header("API-Version", SEARCH_API_VERSION)
                    .with_token(),
            )
            .await?;
        Ok(payload.into())
    }

    /// Rates a release, 1 to 5.
    ///
    /// `GET release/vote/add/{release_id}/{vote}`. Voting again replaces the
    /// previous vote; the server keeps one per account.
    pub async fn release_vote(&self, release_id: i64, vote: u8) -> Result<()> {
        self.require_token()?;
        let vote = vote.clamp(1, 5);
        let _: Ack = self
            .send(
                self.get(format!("release/vote/add/{release_id}/{vote}"))
                    .with_token(),
            )
            .await?;
        Ok(())
    }

    /// Withdraws the account's vote on a release.
    ///
    /// `GET release/vote/delete/{release_id}`
    pub async fn release_vote_delete(&self, release_id: i64) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.get(format!("release/vote/delete/{release_id}"))
                    .with_token(),
            )
            .await?;
        Ok(())
    }

    /// A random release from the account's favourites.
    ///
    /// `GET release/random/favorite`
    pub async fn random_favorite(&self, extended: bool) -> Result<Release> {
        self.require_token()?;
        self.release_at("release/random/favorite".into(), extended)
            .await
    }

    /// A random release from one of an account's lists — this account's or
    /// anyone's whose lists are public.
    ///
    /// `GET release/random/profile/list/{profile_id}/{status}`
    pub async fn random_from_list(
        &self,
        profile_id: i64,
        list: ProfileList,
        extended: bool,
    ) -> Result<Release> {
        let status = list.raw();
        self.release_at(
            format!("release/random/profile/list/{profile_id}/{status}"),
            extended,
        )
        .await
    }

    /// A random release from a collection.
    ///
    /// `GET release/collection/{collection_id}/random`
    pub async fn random_from_collection(
        &self,
        collection_id: i64,
        extended: bool,
    ) -> Result<Release> {
        self.release_at(
            format!("release/collection/{collection_id}/random"),
            extended,
        )
        .await
    }

    /// One release, from any of the paths that answer with exactly one.
    async fn release_at(&self, path: String, extended: bool) -> Result<Release> {
        let payload: ReleasePayload = self
            .send(self.get(path).query("extended_mode", extended).with_token())
            .await?;
        Ok(payload.release)
    }

    /// Where else a release can be watched: the streaming services carrying it.
    ///
    /// `GET release/streaming/platform/{release_id}`, anonymous.
    pub async fn streaming_platforms(&self, release_id: i64) -> Result<Vec<StreamingPlatform>> {
        let payload: PageablePayload<StreamingPlatform> = self
            .send(self.get(format!("release/streaming/platform/{release_id}")))
            .await?;
        Ok(payload.content)
    }
}
