// SPDX-License-Identifier: GPL-3.0-or-later
//
// Accounts, their lists, and the credentials a sign-in returns.

use serde::{Deserialize, Serialize};

use crate::serde_ext::nullable;

/// The user's per-release list. Values are the `status` path segment of
/// `profile/list/*` and the value of [`Release::profile_list_status`].
///
/// 1..=5 are corroborated twice: the unofficial spec names them in this order,
/// and [`Release`]'s own counter fields appear in exactly the same sequence
/// (`watching_count`, `plan_count`, `completed_count`, `hold_on_count`,
/// `dropped_count`). 0 is less certain — on a release it reads as "in no
/// list", but the filter API reuses 0 for "favorites", so the two are not
/// interchangeable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ProfileList {
    NotInList = 0,
    Watching = 1,
    Planned = 2,
    Watched = 3,
    HoldOn = 4,
    Dropped = 5,
}

impl ProfileList {
    /// Every list a release can actually be placed in, in the API's order.
    pub const ALL: [Self; 5] = [
        Self::Watching,
        Self::Planned,
        Self::Watched,
        Self::HoldOn,
        Self::Dropped,
    ];

    #[must_use]
    pub fn from_raw(value: i32) -> Option<Self> {
        Some(match value {
            0 => Self::NotInList,
            1 => Self::Watching,
            2 => Self::Planned,
            3 => Self::Watched,
            4 => Self::HoldOn,
            5 => Self::Dropped,
            _ => return None,
        })
    }

    #[must_use]
    pub fn raw(self) -> i32 {
        self as i32
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Profile {
    #[serde(deserialize_with = "nullable")]
    pub id: i64,
    #[serde(deserialize_with = "nullable")]
    pub login: String,
    /// Observed as a full URL, unlike [`Release::poster`], which is a storage
    /// id. Only ever seen populated for accounts that have set one, so a
    /// client that finds something else here should say so rather than
    /// quietly show nothing — see the GUI's own note where it is fetched.
    #[serde(deserialize_with = "nullable")]
    pub avatar: String,
    #[serde(deserialize_with = "nullable")]
    pub status: String,
    #[serde(deserialize_with = "nullable")]
    pub ban_expires: i64,
    #[serde(deserialize_with = "nullable")]
    pub is_banned: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_verified: bool,
    #[serde(deserialize_with = "nullable")]
    pub watching_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub plan_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub completed_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub hold_on_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub dropped_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub rate_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub comment_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub collection_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub friend_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub register_date: i64,
    #[serde(deserialize_with = "nullable")]
    pub last_activity_time: i64,

    // ---- what the profile screen draws ---------------------------------
    /// Episodes watched, all time.
    #[serde(deserialize_with = "nullable")]
    pub watched_episode_count: i64,
    /// Time spent watching, in minutes — 2242 episodes came with 50 577 of
    /// them, about 22½ each, which is an episode's length in minutes.
    #[serde(deserialize_with = "nullable")]
    pub watched_time: i64,
    /// Episodes watched per day, most recent last.
    #[serde(deserialize_with = "nullable")]
    pub watch_dynamics: Vec<WatchDay>,
    /// What the account watches, as shares of it.
    #[serde(deserialize_with = "nullable")]
    pub preferred_genres: Vec<Share>,
    #[serde(deserialize_with = "nullable")]
    pub preferred_audiences: Vec<Share>,
    #[serde(deserialize_with = "nullable")]
    pub preferred_themes: Vec<Share>,
    #[serde(deserialize_with = "nullable")]
    pub friends_preview: Vec<Profile>,
    #[serde(deserialize_with = "nullable")]
    pub favorite_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub video_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub rating_score: i64,
    #[serde(deserialize_with = "nullable")]
    pub privilege_level: i64,
    /// Where this account stands with the one asking: see [`FriendStatus`].
    /// Absent between strangers, and when nobody is signed in.
    #[serde(deserialize_with = "friend_status")]
    pub friend_status: Option<FriendStatus>,
    #[serde(deserialize_with = "nullable")]
    pub is_online: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_sponsor: bool,
    /// Whether the account's lists and statistics are hidden from others.
    #[serde(deserialize_with = "nullable")]
    pub is_counts_hidden: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_stats_hidden: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_social_hidden: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_friend_requests_disallowed: bool,
    /// The one asking has blocked this account, or been blocked by it.
    #[serde(deserialize_with = "nullable")]
    pub is_blocked: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_me_blocked: bool,
    #[serde(deserialize_with = "nullable")]
    pub vk_page: String,
    #[serde(deserialize_with = "nullable")]
    pub tg_page: String,
    #[serde(deserialize_with = "nullable")]
    pub inst_page: String,
    #[serde(deserialize_with = "nullable")]
    pub tt_page: String,
    #[serde(deserialize_with = "nullable")]
    pub discord_page: String,
}

/// One day of [`Profile::watch_dynamics`].
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct WatchDay {
    /// The day of the month.
    #[serde(deserialize_with = "nullable")]
    pub day: i32,
    #[serde(deserialize_with = "nullable")]
    pub count: i64,
    /// Seconds since the epoch, somewhere in that day.
    #[serde(deserialize_with = "nullable")]
    pub timestamp: i64,
}

/// A genre, audience or theme, and its share of what an account watches.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Share {
    #[serde(deserialize_with = "nullable")]
    pub name: String,
    /// Out of a hundred.
    #[serde(deserialize_with = "nullable")]
    pub percentage: i32,
}

/// Where two accounts stand. The app's enum: 0 the asker has sent a request,
/// 1 the asker has received one, 2 friends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum FriendStatus {
    RequestSent,
    RequestReceived,
    Friends,
}

/// Reads the friend status as a number or as the enum's name. The enum
/// carries a number but declares no `@JsonValue`, so Jackson's default — the
/// name — is what is expected; it has only been seen absent, so both are
/// taken, and anything else is no status rather than an error.
fn friend_status<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<FriendStatus>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(d)?;
    Ok(match value {
        Some(serde_json::Value::Number(n)) => match n.as_i64() {
            Some(0) => Some(FriendStatus::RequestSent),
            Some(1) => Some(FriendStatus::RequestReceived),
            Some(2) => Some(FriendStatus::Friends),
            _ => None,
        },
        Some(serde_json::Value::String(s)) => match s.as_str() {
            "PENDING_FIRST_SECOND" => Some(FriendStatus::RequestSent),
            "PENDING_SECOND_FIRST" => Some(FriendStatus::RequestReceived),
            "FRIEND" => Some(FriendStatus::Friends),
            _ => None,
        },
        _ => None,
    })
}

/// Credentials returned by `auth/signIn`. The `token` is what every
/// authenticated endpoint passes as its `token` query parameter.
///
/// `id` is numeric — the unofficial spec types it as a string, the app as a
/// `long`.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ProfileToken {
    #[serde(deserialize_with = "nullable")]
    pub id: i64,
    #[serde(deserialize_with = "nullable")]
    pub token: String,
}
