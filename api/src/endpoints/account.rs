// SPDX-License-Identifier: GPL-3.0-or-later
//
// Signing in, and everything that belongs to the account.

use serde::Deserialize;

use crate::client::{Ack, Client};
use crate::error::{ApiCode, Error, Result};
use crate::models::{Page, Profile, ProfileList, ProfileToken, Release};

use super::PageablePayload;

#[derive(Deserialize)]
struct ProfilePayload {
    #[serde(default)]
    profile: Profile,
}

/// Answer to `auth/signIn`.
///
/// `profileToken` is camelCase on the wire while almost everything else in this
/// API is snake_case: the entity models carry explicit property names, and this
/// response class does not — so its field goes out under its own spelling. Both
/// are accepted, because that difference is theirs to change.
#[derive(Deserialize)]
struct SignInPayload {
    #[serde(default)]
    profile: Option<Profile>,
    #[serde(default, alias = "profileToken")]
    profile_token: Option<ProfileToken>,
}

/// `code` values specific to `auth/signIn`.
const SIGN_IN_INVALID_LOGIN: i32 = 2;
const SIGN_IN_INVALID_PASSWORD: i32 = 3;

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
            // rather than fabricating an empty session. Saying which half is
            // missing is what turns the next report of this into a fix.
            Ok(payload) => {
                tracing::warn!(
                    profile = payload.profile.is_some(),
                    token = payload.profile_token.is_some(),
                    "signed in but the answer was incomplete"
                );
                Err(SignInError::Api(Error::Api {
                    code: ApiCode::Failed,
                }))
            }
            Err(Error::Api { code }) => Err(match code.raw() {
                SIGN_IN_INVALID_LOGIN => SignInError::UnknownLogin,
                SIGN_IN_INVALID_PASSWORD => SignInError::WrongPassword,
                _ => SignInError::Api(Error::Api { code }),
            }),
            Err(err) => Err(SignInError::Api(err)),
        }
    }

    /// One user's profile.
    ///
    /// `GET profile/{id}`. This is also how the client learns its own name:
    /// `profile/info` sounds like the endpoint for that and is not — it answers
    /// with counters and privilege flags and no profile at all, so asking it
    /// yields a successful response with every field empty.
    pub async fn profile(&self, id: i64) -> Result<Profile> {
        let payload: ProfilePayload = self
            .send(self.get(format!("profile/{id}")).with_token())
            .await?;
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
