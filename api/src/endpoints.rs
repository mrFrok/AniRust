// SPDX-License-Identifier: GPL-3.0-or-later
//
// Endpoint methods. Paths, verbs and parameters are transcribed from the
// shipped app's Retrofit interface declarations — which are interface facts,
// not implementation — rather than from the unofficial OpenAPI spec, which
// disagrees in places. Notably `episode/watch` and `episode/unwatch` are POST,
// and `discover/*` are POST despite reading like plain GETs.
//
// Response wrappers here carry only the payload: the `code` field every body
// includes is handled once, generically, in `client::Envelope`.

use serde::Deserialize;

use crate::client::{Ack, Client, SEARCH_API_VERSION};
use crate::error::{ApiCode, Error, Result};
use crate::models::{
    Dubber, Episode, Page, Profile, ProfileList, ProfileToken, Release, SearchBy, Source,
};

// ---------------------------------------------------------------------------
// Response payloads. Private: callers receive the contents, not the wrapper.
// ---------------------------------------------------------------------------

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

#[derive(Deserialize)]
struct ProfilePayload {
    #[serde(default)]
    profile: Profile,
}

/// Mirrors `PageableResponse<T>`. Pages are 0-based.
#[derive(Deserialize)]
struct PageablePayload<T> {
    #[serde(default = "Vec::new")]
    content: Vec<T>,
    #[serde(default)]
    current_page: i32,
    #[serde(default)]
    total_page_count: i32,
    #[serde(default)]
    total_count: i64,
}

impl<T> From<PageablePayload<T>> for Page<T> {
    fn from(p: PageablePayload<T>) -> Self {
        Self {
            content: p.content,
            current_page: p.current_page,
            total_page_count: p.total_page_count,
            total_count: p.total_count,
        }
    }
}

#[derive(Deserialize)]
struct SignInPayload {
    #[serde(default)]
    profile: Option<Profile>,
    #[serde(default)]
    profile_token: Option<ProfileToken>,
}

/// `code` values specific to `auth/signIn`.
const SIGN_IN_INVALID_LOGIN: i32 = 2;
const SIGN_IN_INVALID_PASSWORD: i32 = 3;

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

// ---------------------------------------------------------------------------
// Account
// ---------------------------------------------------------------------------

impl Client {
    /// Signs in with a login and password.
    ///
    /// `POST auth/signIn`, form-urlencoded. Bad credentials come back as body
    /// codes 2 and 3, which are translated into distinct [`SignInError`]
    /// variants so a caller never has to inspect raw codes.
    pub async fn sign_in(
        &self,
        login: &str,
        password: &str,
    ) -> std::result::Result<(Profile, ProfileToken), SignInError> {
        let result: Result<SignInPayload> = self
            .send(
                self.post("auth/signIn")
                    .form([("login", login), ("password", password)]),
            )
            .await;

        match result {
            Ok(SignInPayload {
                profile: Some(profile),
                profile_token: Some(token),
            }) => Ok((profile, token)),
            // Success code but an incomplete payload — treat as a failure
            // rather than fabricating an empty session.
            Ok(_) => Err(SignInError::Api(Error::Api {
                code: ApiCode::Failed,
            })),
            Err(Error::Api { code }) => Err(match code.raw() {
                SIGN_IN_INVALID_LOGIN => SignInError::UnknownLogin,
                SIGN_IN_INVALID_PASSWORD => SignInError::WrongPassword,
                _ => SignInError::Api(Error::Api { code }),
            }),
            Err(err) => Err(SignInError::Api(err)),
        }
    }

    /// The signed-in user's own profile.
    pub async fn my_profile(&self) -> Result<Profile> {
        self.require_token()?;
        let payload: ProfilePayload = self.send(self.get("profile/info").with_token()).await?;
        Ok(payload.profile)
    }

    /// One of the user's lists. Pages are 0-based.
    ///
    /// `GET profile/list/all/{status}/{page}`
    pub async fn profile_list(
        &self,
        list: ProfileList,
        page: i32,
        sort: Option<i32>,
    ) -> Result<Page<Release>> {
        self.require_token()?;
        let status = list.raw();
        let payload: PageablePayload<Release> = self
            .send(
                self.get(format!("profile/list/all/{status}/{page}"))
                    .query_opt("sort", sort)
                    .with_token(),
            )
            .await?;
        Ok(payload.into())
    }

    /// Moves a release into one of the user's lists.
    pub async fn profile_list_add(&self, list: ProfileList, release_id: i64) -> Result<()> {
        self.require_token()?;
        let status = list.raw();
        let _: Ack = self
            .send(
                self.get(format!("profile/list/add/{status}/{release_id}"))
                    .with_token(),
            )
            .await?;
        Ok(())
    }

    /// Removes a release from one of the user's lists.
    pub async fn profile_list_delete(&self, list: ProfileList, release_id: i64) -> Result<()> {
        self.require_token()?;
        let status = list.raw();
        let _: Ack = self
            .send(
                self.get(format!("profile/list/delete/{status}/{release_id}"))
                    .with_token(),
            )
            .await?;
        Ok(())
    }

    /// Watch history, newest first. Pages are 0-based.
    pub async fn history(&self, page: i32) -> Result<Page<Release>> {
        self.require_token()?;
        let payload: PageablePayload<Release> = self
            .send(self.get(format!("history/{page}")).with_token())
            .await?;
        Ok(payload.into())
    }

    /// Favourites. Pages are 0-based.
    pub async fn favorites(&self, page: i32, sort: Option<i32>) -> Result<Page<Release>> {
        self.require_token()?;
        let payload: PageablePayload<Release> = self
            .send(
                self.get(format!("favorite/all/{page}"))
                    .query_opt("sort", sort)
                    .with_token(),
            )
            .await?;
        Ok(payload.into())
    }

    pub async fn favorite_add(&self, release_id: i64) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(self.get(format!("favorite/add/{release_id}")).with_token())
            .await?;
        Ok(())
    }

    pub async fn favorite_delete(&self, release_id: i64) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.get(format!("favorite/delete/{release_id}"))
                    .with_token(),
            )
            .await?;
        Ok(())
    }
}

/// Why a sign-in failed.
#[derive(Debug, thiserror::Error)]
pub enum SignInError {
    #[error("no account with that login")]
    UnknownLogin,
    #[error("wrong password")]
    WrongPassword,
    #[error(transparent)]
    Api(#[from] Error),
}
