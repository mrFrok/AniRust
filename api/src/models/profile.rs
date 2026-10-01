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
