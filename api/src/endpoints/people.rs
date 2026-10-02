// SPDX-License-Identifier: GPL-3.0-or-later
//
// Other people: their profiles and links, friends and requests, what they
// rated, the account's badges and block list, and staff by role.

use serde::Deserialize;
use serde_json::json;

use crate::client::{Ack, Client};
use crate::error::Result;
use crate::models::{Page, Profile, ProfileCompact, Release};

use super::PageablePayload;

/// A badge the account may wear beside its name.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Badge {
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub id: i64,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub name: String,
    #[serde(rename = "image_url", deserialize_with = "crate::serde_ext::nullable")]
    pub image: String,
    /// 0 a still picture, 1 animated.
    #[serde(rename = "type", deserialize_with = "crate::serde_ext::nullable")]
    pub kind: i32,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub timestamp: i64,
}

/// What the server says about the account itself that the profile does not:
/// its standing, its channel, its sponsorship.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ProfileInfo {
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub channel_id: Option<i64>,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub privilege_level: i64,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub rating_score: i64,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub is_sponsor: bool,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub sponsorship_expires: i64,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub is_perm_banned: bool,
}

/// An account's links elsewhere.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Socials {
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub vk_page: String,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub tg_page: String,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub inst_page: String,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub tt_page: String,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub discord_page: String,
}

/// A change of login, kept on the record.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LoginChange {
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub id: i64,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub login: String,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub timestamp: i64,
}

/// How a friend request was answered. The server's codes for `request/send`
/// and `request/remove`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FriendOutcome {
    /// Sent, and now waiting on the other side.
    Sent,
    /// They had asked first, so this made you friends.
    Confirmed,
    /// A request was withdrawn, or declined.
    RequestRemoved,
    /// You were friends, and are not now.
    FriendshipRemoved,
}

#[derive(Deserialize)]
struct BadgePage {
    #[serde(default)]
    content: Vec<Badge>,
    #[serde(default)]
    current_page: i32,
    #[serde(default)]
    total_page_count: i32,
    #[serde(default)]
    total_count: i64,
}

impl Client {
    /// The account's standing, channel and sponsorship.
    ///
    /// `GET profile/info`
    pub async fn profile_info(&self) -> Result<ProfileInfo> {
        self.require_token()?;
        self.send(self.get("profile/info").with_token()).await
    }

    /// An account's links elsewhere, as far as it shows them.
    ///
    /// `GET profile/social/{id}`
    pub async fn profile_socials(&self, id: i64) -> Result<Socials> {
        self.send(self.get(format!("profile/social/{id}")).with_token())
            .await
    }

    /// The logins an account has had. 0-based.
    ///
    /// `GET profile/login/history/all/{id}/{page}`
    pub async fn login_history(&self, id: i64, page: i32) -> Result<Page<LoginChange>> {
        self.paged(format!("profile/login/history/all/{id}/{page}"), None)
            .await
    }

    /// A moderator's decision about an account — refused for anyone without
    /// the privilege.
    ///
    /// `POST profile/process/{id}`
    pub async fn profile_moderate(
        &self,
        id: i64,
        banned: bool,
        ban_expires: Option<i64>,
        ban_reason: Option<&str>,
    ) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.post(format!("profile/process/{id}"))
                    .with_token()
                    .json(json!({
                        "is_banned": banned,
                        "ban_expires": ban_expires,
                        "ban_reason": ban_reason,
                    })),
            )
            .await?;
        Ok(())
    }

    // ---- friends ------------------------------------------------------------

    /// An account's friends. 0-based.
    ///
    /// `GET profile/friend/all/{id}/{page}`
    pub async fn friends(&self, id: i64, page: i32) -> Result<Page<Profile>> {
        self.paged(format!("profile/friend/all/{id}/{page}"), None)
            .await
    }

    /// People the service suggests befriending.
    ///
    /// `GET profile/friend/recommendations`
    pub async fn friend_recommendations(&self) -> Result<Vec<Profile>> {
        Ok(self
            .paged::<Profile>("profile/friend/recommendations".into(), None)
            .await?
            .content)
    }

    /// Requests waiting on the account. 0-based.
    ///
    /// `GET profile/friend/requests/in/{page}`
    pub async fn friend_requests_in(&self, page: i32) -> Result<Page<Profile>> {
        self.paged(format!("profile/friend/requests/in/{page}"), None)
            .await
    }

    /// The newest few requests waiting on the account.
    ///
    /// `GET profile/friend/requests/in/last?count=`
    pub async fn friend_requests_in_last(&self, count: i32) -> Result<Vec<Profile>> {
        Ok(self
            .paged::<Profile>("profile/friend/requests/in/last".into(), Some(count))
            .await?
            .content)
    }

    /// Requests the account has sent. 0-based.
    ///
    /// `GET profile/friend/requests/out/{page}`
    pub async fn friend_requests_out(&self, page: i32) -> Result<Page<Profile>> {
        self.paged(format!("profile/friend/requests/out/{page}"), None)
            .await
    }

    /// `GET profile/friend/requests/out/last?count=`
    pub async fn friend_requests_out_last(&self, count: i32) -> Result<Vec<Profile>> {
        Ok(self
            .paged::<Profile>("profile/friend/requests/out/last".into(), Some(count))
            .await?
            .content)
    }

    /// Asks to be friends, or accepts a request that is waiting.
    ///
    /// `GET profile/friend/request/send/{id}`. Success comes back as a code
    /// of its own — 2 accepted, 3 sent — rather than 0; refusals (blocked,
    /// a limit reached, requests not accepted) are errors with theirs.
    pub async fn friend_request_send(&self, id: i64) -> Result<FriendOutcome> {
        self.require_token()?;
        let code = self
            .send_code(
                self.get(format!("profile/friend/request/send/{id}"))
                    .with_token(),
            )
            .await?;
        match code {
            2 => Ok(FriendOutcome::Confirmed),
            0 | 3 => Ok(FriendOutcome::Sent),
            other => Err(crate::error::Error::Api {
                code: crate::error::ApiCode::from_raw(other),
            }),
        }
    }

    /// Withdraws a request, declines one, or ends a friendship — whichever
    /// stands between the two accounts.
    ///
    /// `GET profile/friend/request/remove/{id}`. 2 a request removed, 3 a
    /// friendship.
    pub async fn friend_request_remove(&self, id: i64) -> Result<FriendOutcome> {
        self.require_token()?;
        let code = self
            .send_code(
                self.get(format!("profile/friend/request/remove/{id}"))
                    .with_token(),
            )
            .await?;
        match code {
            3 => Ok(FriendOutcome::FriendshipRemoved),
            0 | 2 => Ok(FriendOutcome::RequestRemoved),
            other => Err(crate::error::Error::Api {
                code: crate::error::ApiCode::from_raw(other),
            }),
        }
    }

    /// Hides a waiting request without answering it.
    ///
    /// `GET profile/friend/request/hide/{id}`
    pub async fn friend_request_hide(&self, id: i64) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.get(format!("profile/friend/request/hide/{id}"))
                    .with_token(),
            )
            .await?;
        Ok(())
    }

    // ---- ratings --------------------------------------------------------------

    /// The releases an account has rated. 0-based.
    ///
    /// `GET profile/vote/release/voted/{profile_id}/{page}?sort=`
    pub async fn rated_releases(
        &self,
        profile_id: i64,
        page: i32,
        sort: Option<i32>,
    ) -> Result<Page<Release>> {
        let payload: PageablePayload<Release> = self
            .send(
                self.get(format!("profile/vote/release/voted/{profile_id}/{page}"))
                    .query_opt("sort", sort)
                    .with_token(),
            )
            .await?;
        Ok(payload.into())
    }

    /// Releases the account has watched and not rated. 0-based.
    ///
    /// `GET profile/vote/release/unvoted/{page}`
    pub async fn unrated_releases(&self, page: i32) -> Result<Page<Release>> {
        self.paged(format!("profile/vote/release/unvoted/{page}"), None)
            .await
    }

    /// The newest few of those.
    ///
    /// `GET profile/vote/release/unvoted/last`
    pub async fn unrated_releases_last(&self) -> Result<Vec<Release>> {
        Ok(self
            .paged::<Release>("profile/vote/release/unvoted/last".into(), None)
            .await?
            .content)
    }

    // ---- badges -----------------------------------------------------------------

    /// The badges the account may wear. 0-based.
    ///
    /// `GET profile/preference/badge/all/{page}`
    pub async fn badges(&self, page: i32) -> Result<Page<Badge>> {
        self.require_token()?;
        let payload: BadgePage = self
            .send(
                self.get(format!("profile/preference/badge/all/{page}"))
                    .with_token(),
            )
            .await?;
        Ok(Page {
            content: payload.content,
            current_page: payload.current_page,
            total_page_count: payload.total_page_count,
            total_count: payload.total_count,
        })
    }

    /// Wears a badge.
    ///
    /// `GET profile/preference/badge/edit/{id}`
    pub async fn badge_wear(&self, id: i64) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.get(format!("profile/preference/badge/edit/{id}"))
                    .with_token(),
            )
            .await?;
        Ok(())
    }

    /// Takes the badge off.
    ///
    /// `GET profile/preference/badge/remove`
    pub async fn badge_remove(&self) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(self.get("profile/preference/badge/remove").with_token())
            .await?;
        Ok(())
    }

    // ---- the block list ---------------------------------------------------------

    /// The accounts this one has blocked. 0-based.
    ///
    /// `GET profile/blocklist/all/{page}`
    pub async fn blocked(&self, page: i32) -> Result<Page<Profile>> {
        self.paged(format!("profile/blocklist/all/{page}"), None)
            .await
    }

    /// Blocks an account. Blocking one already blocked answers code 2, which
    /// is the state asked for and so not an error here.
    ///
    /// `GET profile/blocklist/add/{id}`
    pub async fn block(&self, id: i64) -> Result<()> {
        self.require_token()?;
        let code = self
            .send_code(self.get(format!("profile/blocklist/add/{id}")).with_token())
            .await?;
        match code {
            0 | 2 => Ok(()),
            other => Err(crate::error::Error::Api {
                code: crate::error::ApiCode::from_raw(other),
            }),
        }
    }

    /// `GET profile/blocklist/remove/{id}`
    pub async fn unblock(&self, id: i64) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.get(format!("profile/blocklist/remove/{id}"))
                    .with_token(),
            )
            .await?;
        Ok(())
    }

    // ---- staff ------------------------------------------------------------------

    /// The accounts holding a role. 0-based.
    ///
    /// `GET role/all/{page}/{role_id}`
    pub async fn role_holders(&self, role_id: i64, page: i32) -> Result<Page<ProfileCompact>> {
        self.paged(format!("role/all/{page}/{role_id}"), None).await
    }

    // ---- shared ---------------------------------------------------------------------

    async fn paged<T: for<'de> Deserialize<'de>>(
        &self,
        path: String,
        count: Option<i32>,
    ) -> Result<Page<T>> {
        let payload: PageablePayload<T> = self
            .send(self.get(path).query_opt("count", count).with_token())
            .await?;
        Ok(payload.into())
    }
}
