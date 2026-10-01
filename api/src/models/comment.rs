// SPDX-License-Identifier: GPL-3.0-or-later
//
// Comments: on releases, on posts, on collections. All three are one shape on
// the wire — the app's own classes share a base — and differ only in what
// they hang off and who wrote them.

use serde::{Deserialize, Deserializer, Serialize};

use crate::serde_ext::nullable;

use super::Release;

/// Who wrote a comment. The same for every kind of comment: posts carry a
/// channel-flavoured author with a few more fields, which arrive and are
/// ignored here.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ProfileCompact {
    #[serde(deserialize_with = "nullable")]
    pub id: i64,
    #[serde(deserialize_with = "nullable")]
    pub login: String,
    #[serde(deserialize_with = "nullable")]
    pub avatar: String,
    #[serde(deserialize_with = "nullable")]
    pub is_verified: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_sponsor: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_banned: bool,
    #[serde(deserialize_with = "nullable")]
    pub badge_name: String,
    #[serde(deserialize_with = "nullable")]
    pub badge_url: String,
}

/// An object the server may send in full or as a reference to one it already
/// sent in the same response.
///
/// The API serialises with Jackson's object identity: the first time an object
/// appears in a response it is written out with an `@id`, and every later
/// appearance is just that number. A page of comments on one release carries
/// the release once and then `1`, `1`, `1`. Observed on `release/comment/all`.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum Embedded<T> {
    Full(Box<T>),
    /// A reference to an object given earlier in the same response.
    Ref(i64),
}

impl<T> Embedded<T> {
    /// The object, when this is where it was given in full.
    #[must_use]
    pub fn full(&self) -> Option<&T> {
        match self {
            Self::Full(value) => Some(value),
            Self::Ref(_) => None,
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Embedded<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Untagged by hand: a number is a reference, anything else is the
        // object. Derived untagged would try the object first and report its
        // failure rather than the number's success.
        let value = serde_json::Value::deserialize(deserializer)?;
        if let Some(reference) = value.as_i64() {
            return Ok(Self::Ref(reference));
        }
        T::deserialize(value)
            .map(|object| Self::Full(Box::new(object)))
            .map_err(serde::de::Error::custom)
    }
}

/// A comment.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Comment {
    #[serde(deserialize_with = "nullable")]
    pub id: i64,
    #[serde(deserialize_with = "nullable")]
    pub message: String,
    /// Seconds since the epoch.
    #[serde(deserialize_with = "nullable")]
    pub timestamp: i64,
    #[serde(deserialize_with = "nullable")]
    pub profile: ProfileCompact,
    /// This account's vote on it: 0 none, 1 down, 2 up — [`CommentVote`].
    #[serde(deserialize_with = "nullable")]
    pub vote: i32,
    /// The score: up votes less down votes.
    #[serde(deserialize_with = "nullable")]
    pub vote_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub likes_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub can_like: bool,
    #[serde(deserialize_with = "nullable")]
    pub reply_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub parent_comment_id: Option<i64>,
    #[serde(deserialize_with = "nullable")]
    pub is_reply: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_spoiler: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_edited: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_deleted: bool,
    /// The episode a release comment was written at, when it was.
    #[serde(deserialize_with = "nullable")]
    pub posted_at_episode: Option<i32>,
    /// The release a release comment is on — in full the first time in a
    /// response, a reference after that. Absent on other kinds.
    #[serde(deserialize_with = "nullable")]
    pub release: Option<Embedded<Release>>,
}

/// A vote on a comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentVote {
    /// Withdraws the account's vote.
    None,
    Down,
    Up,
}

impl CommentVote {
    #[must_use]
    pub fn raw(self) -> i32 {
        match self {
            Self::None => 0,
            Self::Down => 1,
            Self::Up => 2,
        }
    }
}

/// The order of a page of comments.
///
/// The app's sort menu has three entries; their numbers were observed, not
/// read: on one release, `0` came back newest first, `2` oldest first, and
/// `3` by score. `1` answered the same as `0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CommentSort {
    #[default]
    Newest,
    Oldest,
    Popular,
}

impl CommentSort {
    #[must_use]
    pub fn raw(self) -> i32 {
        match self {
            Self::Newest => 0,
            Self::Oldest => 2,
            Self::Popular => 3,
        }
    }
}

/// What a comment hangs off. The three kinds share every endpoint but the
/// first word of its path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentTarget {
    Release,
    Article,
    Collection,
}

impl CommentTarget {
    /// The first segment of every path for this kind.
    #[must_use]
    pub fn segment(self) -> &'static str {
        match self {
            Self::Release => "release",
            Self::Article => "article",
            Self::Collection => "collection",
        }
    }
}

/// What a moderator decides about a comment. Only an account with the
/// privilege is listened to; anyone else's is refused by the server.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CommentModeration {
    pub message: String,
    pub reason: String,
    pub is_spoiler: bool,
    pub is_deleted: bool,
    pub is_banned: bool,
    /// When a ban ends, in seconds since the epoch.
    pub ban_expires: Option<i64>,
    pub ban_reason: Option<String>,
}

/// An account as one channel sees it: its member's standing there.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ChannelProfile {
    #[serde(flatten)]
    pub profile: ProfileCompact,
    #[serde(deserialize_with = "nullable")]
    pub channel_id: i64,
    /// Their permission level in the channel.
    #[serde(deserialize_with = "nullable")]
    pub permission: i32,
    #[serde(deserialize_with = "nullable")]
    pub is_blocked: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_perm_blocked: bool,
    #[serde(deserialize_with = "nullable")]
    pub block_reason: String,
    #[serde(deserialize_with = "nullable")]
    pub block_expire_date: Option<i64>,
}
