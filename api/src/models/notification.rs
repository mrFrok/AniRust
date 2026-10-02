// SPDX-License-Identifier: GPL-3.0-or-later
//
// Notifications, and what the account wants to be notified about.
//
// The app models each kind as its own class under a `type` discriminator —
// a new episode, a friend request, a reply, a related release, a post. Here
// they are one struct whose fields are present for the kinds that carry them:
// a list of notifications is shown in one column whatever is in it, and a
// kind this client has not met should still show its time and stay
// deletable rather than fail the whole page.

use serde::{Deserialize, Serialize};

use crate::serde_ext::nullable;

use super::{ArticlePayload, ProfileSlim};

/// A release as a notification names it.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ReleaseCompact {
    #[serde(deserialize_with = "nullable")]
    pub id: i64,
    #[serde(deserialize_with = "nullable")]
    pub title_ru: String,
    #[serde(deserialize_with = "nullable")]
    pub image: String,
}

/// A named thing, as the compact entities carry their voice-over and host.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Named {
    #[serde(deserialize_with = "nullable")]
    pub name: String,
}

/// The host an episode is on, and the voice-over it carries.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct SourceCompact {
    #[serde(deserialize_with = "nullable")]
    pub name: String,
    #[serde(rename = "type", deserialize_with = "nullable")]
    pub dubber: Named,
}

/// A new episode, as its notification names it.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct EpisodeCompact {
    /// Its number or title, as the host gives it.
    #[serde(deserialize_with = "nullable")]
    pub name: String,
    #[serde(deserialize_with = "nullable")]
    pub release: ReleaseCompact,
    #[serde(deserialize_with = "nullable")]
    pub source: SourceCompact,
}

/// A comment as a notification quotes it, and what it is on.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct CommentCompact {
    #[serde(deserialize_with = "nullable")]
    pub id: i64,
    #[serde(deserialize_with = "nullable")]
    pub message: String,
    #[serde(deserialize_with = "nullable")]
    pub is_spoiler: bool,
    #[serde(deserialize_with = "nullable")]
    pub profile: ProfileSlim,
    /// The release a release comment is on.
    #[serde(deserialize_with = "nullable")]
    pub release: Option<ReleaseCompact>,
}

/// A channel as a post's notification names it.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ChannelCompact {
    #[serde(deserialize_with = "nullable")]
    pub id: i64,
    #[serde(deserialize_with = "nullable")]
    pub title: String,
    #[serde(deserialize_with = "nullable")]
    pub avatar: String,
    #[serde(deserialize_with = "nullable")]
    pub is_blog: bool,
}

/// A post as its notification names it.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ArticleCompact {
    #[serde(deserialize_with = "nullable")]
    pub id: i64,
    #[serde(deserialize_with = "nullable")]
    pub channel: ChannelCompact,
    #[serde(deserialize_with = "nullable")]
    pub payload: ArticlePayload,
    #[serde(deserialize_with = "nullable")]
    pub creation_date: i64,
}

/// One notification, of any kind.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Notification {
    #[serde(deserialize_with = "nullable")]
    pub id: i64,
    /// The kind, as the server names it: `episode`, `friend`, `article`, and
    /// the comment and related-release kinds. Empty on the per-kind lists,
    /// where the list itself says which kind it is.
    #[serde(rename = "type", deserialize_with = "nullable")]
    pub kind: String,
    /// Seconds since the epoch.
    #[serde(deserialize_with = "nullable")]
    pub timestamp: i64,
    /// Not yet seen.
    #[serde(deserialize_with = "nullable")]
    pub is_new: bool,
    // ---- a new episode
    #[serde(deserialize_with = "nullable")]
    pub episode: Option<EpisodeCompact>,
    // ---- a friend request, or its answer
    #[serde(deserialize_with = "nullable")]
    pub by_profile: Option<ProfileSlim>,
    /// What happened between the two accounts, as the server numbers it.
    #[serde(deserialize_with = "nullable")]
    pub value: i32,
    // ---- a related release
    #[serde(deserialize_with = "nullable")]
    pub release: Option<ReleaseCompact>,
    // ---- a post
    #[serde(deserialize_with = "nullable")]
    pub article: Option<ArticleCompact>,
    // ---- a comment, and the one it answers
    #[serde(deserialize_with = "nullable")]
    pub comment: Option<CommentCompact>,
    #[serde(deserialize_with = "nullable")]
    pub parent_comment: Option<CommentCompact>,
}

/// One list of notifications. Each has a list of its own, and each but "all"
/// a delete of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationKind {
    /// Everything, mixed.
    All,
    Episodes,
    Friends,
    /// Replies to the account's comments on releases.
    ReleaseComments,
    /// Releases related to ones the account follows.
    RelatedReleases,
    /// New posts in followed channels.
    Articles,
    /// Comments on posts.
    ArticleComments,
    CollectionComments,
}

impl NotificationKind {
    /// The path the list is at, before the page.
    #[must_use]
    pub fn list_path(self) -> &'static str {
        match self {
            Self::All => "notification/all",
            Self::Episodes => "notification/episodes",
            Self::Friends => "notification/friends",
            Self::ReleaseComments => "notification/releaseComments",
            Self::RelatedReleases => "notification/related/release",
            Self::Articles => "notification/articles",
            Self::ArticleComments => "notification/article/comments",
            Self::CollectionComments => "notification/collectionComments",
        }
    }
}

/// Which notification a delete removes. Two more kinds than the lists have:
/// comments on the account's *own* posts and collections are deleted by
/// their own paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationDelete {
    Episode,
    Friend,
    ReleaseComment,
    RelatedRelease,
    ArticleComment,
    CollectionComment,
    MyArticleComment,
    MyCollectionComment,
}

impl NotificationDelete {
    #[must_use]
    pub fn path(self) -> &'static str {
        match self {
            Self::Episode => "notification/episode/delete",
            Self::Friend => "notification/friend/delete",
            Self::ReleaseComment => "notification/releaseComment/delete",
            Self::RelatedRelease => "notification/related/release/delete",
            Self::ArticleComment => "notification/article/comment/delete",
            Self::CollectionComment => "notification/collectionComment/delete",
            Self::MyArticleComment => "notification/my/article/comment/delete",
            Self::MyCollectionComment => "notification/my/collection/comment/delete",
        }
    }
}

/// What the account is notified about.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct NotificationPreferences {
    #[serde(deserialize_with = "nullable")]
    pub is_episode_notifications_enabled: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_first_episode_notification_enabled: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_comment_notifications_enabled: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_related_release_notifications_enabled: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_article_notifications_enabled: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_my_article_comment_notifications_enabled: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_my_collection_comment_notifications_enabled: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_release_type_notifications_enabled: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_report_process_notifications_enabled: bool,
}

/// One of the switches in [`NotificationPreferences`]. Each is flipped by a
/// request of its own that carries no value: the server turns it over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationSwitch {
    Episodes,
    FirstEpisode,
    Comments,
    RelatedReleases,
    Articles,
    MyArticleComments,
    MyCollectionComments,
    /// Only for the releases chosen by hand, rather than everything followed.
    SelectedReleases,
    ReportOutcomes,
}

impl NotificationSwitch {
    #[must_use]
    pub fn path(self) -> &'static str {
        match self {
            Self::Episodes => "profile/preference/notification/episode/edit",
            Self::FirstEpisode => "profile/preference/notification/episode/first/edit",
            Self::Comments => "profile/preference/notification/comment/edit",
            Self::RelatedReleases => "profile/preference/notification/related/release/edit",
            Self::Articles => "profile/preference/notification/article/edit",
            Self::MyArticleComments => "profile/preference/notification/my/article/comment/edit",
            Self::MyCollectionComments => {
                "profile/preference/notification/my/collection/comment/edit"
            }
            Self::SelectedReleases => "profile/preference/notification/selected/releases/edit",
            Self::ReportOutcomes => "profile/preference/notification/report/process/edit",
        }
    }
}
