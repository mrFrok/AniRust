// SPDX-License-Identifier: GPL-3.0-or-later
//
// The account's standing and the service's own machinery: deleting the
// account, its sanctions and appeals, reports, videos attached to releases,
// moving bookmarks in and out, and the newest post id.

use serde::Deserialize;
use serde_json::json;

use crate::client::{Ack, Client};
use crate::error::Result;
use crate::models::{Page, Release, StreamingPlatform};

use super::PageablePayload;

/// Where a request to delete the account stands.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Deletion {
    /// 0 none pending; the service's codes otherwise — 2 already requested,
    /// 5 in progress, 6 a wrong password.
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub code: i32,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub requested_at: i64,
    /// When the account goes, unless the request is cancelled before.
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub delete_at: i64,
}

/// The account's standing: how often it has been banned and what is in force.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Health {
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub ban_count: i64,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub last_ban_timestamp: i64,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub last_ban_expires: i64,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub blog_mute_expires: i64,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub blog_suspension_expires: i64,
}

/// One sanction against the account or something it posted, and its appeal.
/// The enum fields are kept as the strings they arrive as.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Enforcement {
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub id: i64,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub reason: String,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub creation_timestamp: i64,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub is_revoked: bool,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub revocation_timestamp: Option<i64>,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub appeal_status: serde_json::Value,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub appeal_expires_timestamp: Option<i64>,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub appeal_submit_timestamp: Option<i64>,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub appeal_process_message: String,
}

/// A reason a report can give.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ReportReason {
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub id: i64,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub name: String,
}

/// What a report is about. Each has its own reasons and its own path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportTarget {
    Release,
    Episode,
    Profile,
    Channel,
    Article,
    Collection,
    ReleaseComment,
    ArticleComment,
    CollectionComment,
}

impl ReportTarget {
    fn path(self) -> &'static str {
        match self {
            Self::Release => "report/release",
            Self::Episode => "report/episode",
            Self::Profile => "report/profile",
            Self::Channel => "report/channel",
            Self::Article => "report/article",
            Self::Collection => "report/collection",
            Self::ReleaseComment => "report/comment/release",
            Self::ArticleComment => "report/comment/article",
            Self::CollectionComment => "report/comment/collection",
        }
    }
}

/// A video attached to a release: a trailer, an opening, a clip.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ReleaseVideo {
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub id: i64,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub title: String,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub image: String,
    /// The page on the hosting site.
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub url: String,
    /// An address to play it from, when the host gives one.
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub player_url: String,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub category: VideoCategory,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub hosting: VideoCategory,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub favorites_count: i64,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub is_favorite: bool,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub timestamp: i64,
}

/// A kind of video, or the site hosting one: both an id and a name.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct VideoCategory {
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub id: i64,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub name: String,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub icon: String,
}

/// A release's videos, grouped by kind, with its newest and where else it is
/// on.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ReleaseVideos {
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub blocks: Vec<VideoBlock>,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub last_videos: Vec<ReleaseVideo>,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub streaming_platforms: Vec<StreamingPlatform>,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub can_appeal: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct VideoBlock {
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub category: VideoCategory,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub videos: Vec<ReleaseVideo>,
}

/// The account's lists as release ids, for moving them in from elsewhere.
#[derive(Debug, Clone, Default)]
pub struct Bookmarks {
    pub watching: Vec<i64>,
    pub plans: Vec<i64>,
    pub completed: Vec<i64>,
    pub hold_on: Vec<i64>,
    pub dropped: Vec<i64>,
}

#[derive(Deserialize)]
struct ReasonsPayload {
    #[serde(default)]
    content: Option<Vec<ReportReason>>,
}

#[derive(Deserialize)]
struct EnforcementPayload {
    #[serde(default)]
    enforcement: Enforcement,
}

#[derive(Deserialize)]
struct CategoriesPayload {
    #[serde(default)]
    categories: Vec<VideoCategory>,
}

#[derive(Deserialize)]
struct LatestPayload {
    #[serde(default)]
    article_id: Option<i64>,
}

#[derive(Deserialize)]
struct ExportPayload {
    #[serde(default)]
    releases: Vec<Release>,
}

impl Client {
    // ---- deleting the account -----------------------------------------------

    /// Where a request to delete the account stands.
    ///
    /// `GET profile/deletion/status`
    pub async fn deletion_status(&self) -> Result<Deletion> {
        self.require_token()?;
        self.send_any_code(self.get("profile/deletion/status").with_token())
            .await
    }

    /// Asks for the account to be deleted. The service holds it for a while
    /// before it goes, and it can be cancelled until then.
    ///
    /// `POST profile/deletion/request`, with the password.
    pub async fn deletion_request(&self, password: &str) -> Result<Deletion> {
        self.require_token()?;
        self.send_any_code(
            self.post("profile/deletion/request")
                .with_token()
                .form([("password", password)]),
        )
        .await
    }

    /// `POST profile/deletion/cancel`
    pub async fn deletion_cancel(&self) -> Result<Deletion> {
        self.require_token()?;
        self.send_any_code(self.post("profile/deletion/cancel").with_token())
            .await
    }

    // ---- standing ------------------------------------------------------------------

    /// `GET profile/health/status`
    pub async fn health(&self) -> Result<Health> {
        self.require_token()?;
        self.send(self.get("profile/health/status").with_token())
            .await
    }

    /// Sanctions against the account itself. 0-based.
    ///
    /// `GET profile/health/enforcement/account/all/{page}` — a bare list.
    pub async fn enforcements_account(&self, page: i32) -> Result<Vec<Enforcement>> {
        self.enforcement_list(format!("profile/health/enforcement/account/all/{page}"))
            .await
    }

    /// Sanctions against what the account posted. 0-based.
    ///
    /// `GET profile/health/enforcement/content/all/{page}`
    pub async fn enforcements_content(&self, page: i32) -> Result<Vec<Enforcement>> {
        self.enforcement_list(format!("profile/health/enforcement/content/all/{page}"))
            .await
    }

    /// `GET profile/health/enforcement/{id}`
    pub async fn enforcement(&self, id: i64) -> Result<Enforcement> {
        self.require_token()?;
        let payload: EnforcementPayload = self
            .send(
                self.get(format!("profile/health/enforcement/{id}"))
                    .with_token(),
            )
            .await?;
        Ok(payload.enforcement)
    }

    /// Appeals a sanction.
    ///
    /// `POST profile/health/enforcement/{id}/appeal`
    pub async fn enforcement_appeal(&self, id: i64, message: &str) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.post(format!("profile/health/enforcement/{id}/appeal"))
                    .with_token()
                    .json(json!({ "message": message })),
            )
            .await?;
        Ok(())
    }

    // ---- reports ---------------------------------------------------------------------

    /// The reasons a report about this kind of thing can give.
    ///
    /// `GET report/{target}/reasons` — answered as a bare list, read here
    /// whether or not it comes wrapped.
    pub async fn report_reasons(&self, target: ReportTarget) -> Result<Vec<ReportReason>> {
        self.require_token()?;
        let body: serde_json::Value = self
            .send_any_code(self.get(format!("{}/reasons", target.path())).with_token())
            .await?;
        if body.is_array() {
            return Ok(serde_json::from_value(body).unwrap_or_default());
        }
        let wrapped: ReasonsPayload =
            serde_json::from_value(body).unwrap_or(ReasonsPayload { content: None });
        Ok(wrapped.content.unwrap_or_default())
    }

    /// Reports something to the service's moderators.
    ///
    /// `POST report/{target}` with the thing's id, the reason, and a message.
    pub async fn report(
        &self,
        target: ReportTarget,
        entity_id: i64,
        reason_id: i64,
        message: &str,
    ) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(self.post(target.path()).with_token().json(json!({
                "entity_id": entity_id,
                "reason": reason_id,
                "message": message,
            })))
            .await?;
        Ok(())
    }

    // ---- videos ------------------------------------------------------------------------

    /// The kinds of video a release can have.
    ///
    /// `GET video/release/categories`, anonymous.
    pub async fn video_categories(&self) -> Result<Vec<VideoCategory>> {
        let payload: CategoriesPayload = self.send(self.get("video/release/categories")).await?;
        Ok(payload.categories)
    }

    /// A release's videos, grouped by kind.
    ///
    /// `GET video/release/{release_id}`, anonymous.
    pub async fn release_videos(&self, release_id: i64) -> Result<ReleaseVideos> {
        self.send(self.get(format!("video/release/{release_id}")))
            .await
    }

    /// All of a release's videos, a page at a time. 0-based.
    ///
    /// `GET video/release/{release_id}/{page}`, anonymous.
    pub async fn release_videos_page(
        &self,
        release_id: i64,
        page: i32,
    ) -> Result<Page<ReleaseVideo>> {
        self.video_page(format!("video/release/{release_id}/{page}"), false)
            .await
    }

    /// A release's videos of one kind. 0-based.
    ///
    /// `GET video/release/{release_id}/category/{category_id}/{page}`
    pub async fn release_videos_of(
        &self,
        release_id: i64,
        category_id: i64,
        page: i32,
    ) -> Result<Page<ReleaseVideo>> {
        self.video_page(
            format!("video/release/{release_id}/category/{category_id}/{page}"),
            false,
        )
        .await
    }

    /// The videos an account added. 0-based.
    ///
    /// `GET video/profile/{profile_id}/{page}`
    pub async fn profile_videos(&self, profile_id: i64, page: i32) -> Result<Page<ReleaseVideo>> {
        self.video_page(format!("video/profile/{profile_id}/{page}"), true)
            .await
    }

    /// An account's favourite videos. 0-based.
    ///
    /// `GET releaseVideoFavorite/all/{profile_id}/{page}`
    pub async fn favorite_videos(&self, profile_id: i64, page: i32) -> Result<Page<ReleaseVideo>> {
        self.video_page(
            format!("releaseVideoFavorite/all/{profile_id}/{page}"),
            true,
        )
        .await
    }

    /// `GET releaseVideoFavorite/add/{video_id}`
    pub async fn video_favorite_add(&self, video_id: i64) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.get(format!("releaseVideoFavorite/add/{video_id}"))
                    .with_token(),
            )
            .await?;
        Ok(())
    }

    /// `GET releaseVideoFavorite/delete/{video_id}`
    pub async fn video_favorite_delete(&self, video_id: i64) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.get(format!("releaseVideoFavorite/delete/{video_id}"))
                    .with_token(),
            )
            .await?;
        Ok(())
    }

    /// Suggests a video for a release, which moderators look at.
    ///
    /// `POST video/appeal/add`
    pub async fn video_suggest(
        &self,
        release_id: i64,
        category_id: i64,
        title: &str,
        url: &str,
    ) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(self.post("video/appeal/add").with_token().json(json!({
                "release_id": release_id,
                "category_id": category_id,
                "title": title,
                "url": url,
            })))
            .await?;
        Ok(())
    }

    /// The videos the account has suggested. 0-based.
    ///
    /// `GET video/appeal/profile/{page}`
    pub async fn video_suggestions(&self, page: i32) -> Result<Page<ReleaseVideo>> {
        self.video_page(format!("video/appeal/profile/{page}"), true)
            .await
    }

    /// The newest few of those.
    ///
    /// `GET video/appeal/profile/last`
    pub async fn video_suggestions_last(&self) -> Result<Vec<ReleaseVideo>> {
        Ok(self
            .video_page("video/appeal/profile/last".into(), true)
            .await?
            .content)
    }

    /// Withdraws a suggestion.
    ///
    /// `POST video/appeal/delete/{id}`
    pub async fn video_suggestion_delete(&self, id: i64) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(self.post(format!("video/appeal/delete/{id}")).with_token())
            .await?;
        Ok(())
    }

    // ---- bookmarks in and out ---------------------------------------------------------

    /// Moves lists in from elsewhere, naming the service they came from.
    ///
    /// `POST import/bookmarks`
    pub async fn import_bookmarks(&self, from: &str, lists: &Bookmarks) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(self.post("import/bookmarks").with_token().json(json!({
                "watching": lists.watching,
                "plans": lists.plans,
                "completed": lists.completed,
                "hold_on": lists.hold_on,
                "dropped": lists.dropped,
                "selected_importer_name": from,
            })))
            .await?;
        Ok(())
    }

    /// Whether an import may be made now. Code 2 a limit reached.
    ///
    /// `POST import/status`
    pub async fn import_status(&self) -> Result<i32> {
        self.require_token()?;
        self.send_code(self.post("import/status").with_token())
            .await
    }

    /// The releases in the account's lists, for taking elsewhere. `lists` are
    /// list numbers, as [`crate::ProfileList::raw`] gives them.
    ///
    /// `POST export/bookmarks?sort=`
    pub async fn export_bookmarks(&self, lists: &[i32], sort: i32) -> Result<Vec<Release>> {
        self.require_token()?;
        let payload: ExportPayload = self
            .send(
                self.post("export/bookmarks")
                    .query("sort", sort)
                    .with_token()
                    .json(json!({ "bookmarks_export_profile_lists": lists })),
            )
            .await?;
        Ok(payload.releases)
    }

    // ---- the feed's newest ----------------------------------------------------------------

    /// The id of the newest post in the account's feed, for saying whether
    /// there is anything new without fetching a page.
    ///
    /// `GET feed/latest`
    pub async fn feed_latest_id(&self) -> Result<Option<i64>> {
        self.require_token()?;
        let payload: LatestPayload = self.send(self.get("feed/latest").with_token()).await?;
        Ok(payload.article_id)
    }

    // ---- shared ---------------------------------------------------------------------------

    async fn enforcement_list(&self, path: String) -> Result<Vec<Enforcement>> {
        self.require_token()?;
        let body: serde_json::Value = self.send_any_code(self.get(path).with_token()).await?;
        Ok(match body {
            serde_json::Value::Array(_) => serde_json::from_value(body).unwrap_or_default(),
            other => other
                .get("content")
                .cloned()
                .and_then(|c| serde_json::from_value(c).ok())
                .unwrap_or_default(),
        })
    }

    async fn video_page(&self, path: String, token: bool) -> Result<Page<ReleaseVideo>> {
        let mut spec = self.get(path);
        if token {
            spec = spec.with_token();
        }
        let payload: PageablePayload<ReleaseVideo> = self.send(spec).await?;
        Ok(payload.into())
    }
}
