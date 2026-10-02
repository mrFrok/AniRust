// SPDX-License-Identifier: GPL-3.0-or-later
//
// Channels: reading one and its posts, finding them, following, muting —
// and running one: creating it, its pictures, who may write in it, who is
// blocked from it.
//
// Subscribing and unsubscribing live with the feed, where they were first
// needed.

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::client::{Ack, Client, Upload};
use crate::error::Result;
use crate::models::{Article, Channel, ChannelProfile, Page};

use super::PageablePayload;

#[derive(Deserialize)]
struct ChannelPayload {
    #[serde(default)]
    channel: Channel,
}

#[derive(Deserialize)]
struct UrlPayload {
    #[serde(default)]
    url: String,
}

#[derive(Deserialize)]
struct CountPayload {
    #[serde(default)]
    subscription_count: i64,
}

#[derive(Deserialize)]
struct EditorPayload {
    #[serde(default)]
    media_upload_token: String,
}

#[derive(Deserialize)]
struct EditorChannelsPayload {
    #[serde(default)]
    channels: Vec<Channel>,
}

#[derive(Deserialize)]
struct BlockPayload {
    #[serde(default)]
    channel_block: Option<ChannelBlock>,
}

/// One account's block from one channel.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ChannelBlock {
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub reason: String,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub added_date: i64,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub expire_date: i64,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub is_perm_blocked: bool,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub is_reason_showing_enabled: bool,
}

/// A channel's settings, as created or edited.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ChannelSettings {
    pub title: String,
    pub description: String,
    pub is_commenting_enabled: bool,
    pub is_article_suggestion_enabled: bool,
    /// The channel's widget on the episode list of releases dubbed by it.
    pub is_episode_channel_widget_enabled: Option<bool>,
    pub episode_channel_widget_article_count: Option<i32>,
    pub episode_channel_widget_popularity_period: Option<i32>,
    pub episode_channel_widget_sort: Option<i32>,
}

/// Narrowing for [`Client::channels`]. Every field unset is "any".
#[derive(Debug, Clone, Default, Serialize)]
pub struct ChannelFilter {
    pub is_blog: Option<bool>,
    pub is_subscribed: Option<bool>,
    pub permission: Option<i32>,
    /// 0 none, 1 by subscriber count.
    pub sort: Option<i32>,
}

/// A block, as a channel's owner sets it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ChannelBlockRequest {
    pub target_profile_id: i64,
    pub is_blocked: bool,
    pub is_perm_blocked: bool,
    pub reason: String,
    pub is_reason_showing_enabled: bool,
    /// Seconds since the epoch; none for a block without an end.
    pub expire_date: Option<i64>,
}

impl Client {
    /// One channel.
    ///
    /// `GET channel/{id}`
    pub async fn channel(&self, id: i64) -> Result<Channel> {
        let payload: ChannelPayload = self
            .send(self.get(format!("channel/{id}")).with_token())
            .await?;
        Ok(payload.channel)
    }

    /// An account's personal blog.
    ///
    /// `GET channel/blog/{profile_id}`
    pub async fn blog(&self, profile_id: i64) -> Result<Channel> {
        let payload: ChannelPayload = self
            .send(self.get(format!("channel/blog/{profile_id}")).with_token())
            .await?;
        Ok(payload.channel)
    }

    /// A channel's posts, newest first. 0-based.
    ///
    /// `POST channel/{id}/article/all/{page}` — a POST that only reads.
    pub async fn channel_articles(&self, id: i64, page: i32) -> Result<Page<Article>> {
        let payload: PageablePayload<Article> = self
            .send(
                self.post(format!("channel/{id}/article/all/{page}"))
                    .with_token(),
            )
            .await?;
        Ok(payload.into())
    }

    /// Channels, narrowed. 0-based.
    ///
    /// `POST channel/all/{page}`
    pub async fn channels(&self, filter: &ChannelFilter, page: i32) -> Result<Page<Channel>> {
        let payload: PageablePayload<Channel> = self
            .send(
                self.post(format!("channel/all/{page}"))
                    .with_token()
                    .json(serde_json::to_value(filter).unwrap_or_default()),
            )
            .await?;
        Ok(payload.into())
    }

    /// Channels the service suggests. 0-based.
    ///
    /// `GET channel/recommendations/{page}?is_blog=&exclude_subscribed=`
    pub async fn channel_recommendations(
        &self,
        page: i32,
        blogs: Option<bool>,
        exclude_subscribed: Option<bool>,
    ) -> Result<Page<Channel>> {
        let payload: PageablePayload<Channel> = self
            .send(
                self.get(format!("channel/recommendations/{page}"))
                    .query_opt("is_blog", blogs)
                    .query_opt("exclude_subscribed", exclude_subscribed)
                    .with_token(),
            )
            .await?;
        Ok(payload.into())
    }

    /// The channels the account follows. 0-based.
    ///
    /// `GET channel/subscription/all/{page}?sort=`
    pub async fn subscriptions(&self, page: i32, sort: i32) -> Result<Page<Channel>> {
        self.require_token()?;
        let payload: PageablePayload<Channel> = self
            .send(
                self.get(format!("channel/subscription/all/{page}"))
                    .query("sort", sort)
                    .with_token(),
            )
            .await?;
        Ok(payload.into())
    }

    /// How many channels the account follows.
    ///
    /// `GET channel/subscription/count`
    pub async fn subscription_count(&self) -> Result<i64> {
        self.require_token()?;
        let payload: CountPayload = self
            .send(self.get("channel/subscription/count").with_token())
            .await?;
        Ok(payload.subscription_count)
    }

    /// Stops a channel's posts reaching the feed, without unfollowing it.
    ///
    /// `POST channel/mute/{id}`
    pub async fn channel_mute(&self, id: i64) -> Result<()> {
        self.ack_post(format!("channel/mute/{id}")).await
    }

    /// `POST channel/unmute/{id}`
    pub async fn channel_unmute(&self, id: i64) -> Result<()> {
        self.ack_post(format!("channel/unmute/{id}")).await
    }

    /// The channels the account has muted. 0-based.
    ///
    /// `GET channel/mute/all/{page}`
    pub async fn muted_channels(&self, page: i32) -> Result<Page<Channel>> {
        self.require_token()?;
        let payload: PageablePayload<Channel> = self
            .send(self.get(format!("channel/mute/all/{page}")).with_token())
            .await?;
        Ok(payload.into())
    }

    // ---- running a channel ------------------------------------------------------

    /// Creates a channel. Answers with it as stored.
    ///
    /// `POST channel/create`
    pub async fn channel_create(&self, settings: &ChannelSettings) -> Result<Channel> {
        self.require_token()?;
        let payload: ChannelPayload = self
            .send(
                self.post("channel/create")
                    .with_token()
                    .json(serde_json::to_value(settings).unwrap_or_default()),
            )
            .await?;
        Ok(payload.channel)
    }

    /// Creates the account's personal blog. The service refuses accounts
    /// below some reputation, with code 2.
    ///
    /// `POST channel/blog/create`
    pub async fn blog_create(&self) -> Result<Channel> {
        self.require_token()?;
        let payload: ChannelPayload = self
            .send(self.post("channel/blog/create").with_token())
            .await?;
        Ok(payload.channel)
    }

    /// Changes a channel's settings.
    ///
    /// `POST channel/edit/{id}`
    pub async fn channel_edit(&self, id: i64, settings: &ChannelSettings) -> Result<Channel> {
        self.require_token()?;
        let payload: ChannelPayload = self
            .send(
                self.post(format!("channel/edit/{id}"))
                    .with_token()
                    .json(serde_json::to_value(settings).unwrap_or_default()),
            )
            .await?;
        Ok(payload.channel)
    }

    /// Replaces a channel's picture. Answers with its new address.
    ///
    /// `POST channel/avatar/upload/{id}`, multipart, the file as the part
    /// `image` — the same helper as the account's own picture.
    pub async fn channel_avatar_upload(
        &self,
        id: i64,
        file_name: &str,
        mime: &'static str,
        bytes: Vec<u8>,
    ) -> Result<String> {
        self.upload_picture(
            format!("channel/avatar/upload/{id}"),
            file_name,
            mime,
            bytes,
        )
        .await
    }

    /// Replaces a channel's cover.
    ///
    /// `POST channel/cover/upload/{id}`, multipart, as above.
    pub async fn channel_cover_upload(
        &self,
        id: i64,
        file_name: &str,
        mime: &'static str,
        bytes: Vec<u8>,
    ) -> Result<String> {
        self.upload_picture(format!("channel/cover/upload/{id}"), file_name, mime, bytes)
            .await
    }

    /// `POST channel/cover/delete/{id}`
    pub async fn channel_cover_delete(&self, id: i64) -> Result<()> {
        self.ack_post(format!("channel/cover/delete/{id}")).await
    }

    /// Whether the account may write in a channel now, and the token its
    /// pictures go up with if it may.
    ///
    /// `GET channel/{id}/editor/available?is_suggestion=&is_edit_mode=`
    pub async fn editor_available(
        &self,
        id: i64,
        suggestion: bool,
        edit_mode: bool,
    ) -> Result<String> {
        self.require_token()?;
        let payload: EditorPayload = self
            .send(
                self.get(format!("channel/{id}/editor/available"))
                    .query("is_suggestion", suggestion)
                    .query("is_edit_mode", edit_mode)
                    .with_token(),
            )
            .await?;
        Ok(payload.media_upload_token)
    }

    /// The channels the account may write in.
    ///
    /// `GET channel/editor/available/all?only_subscribed=`
    pub async fn editor_channels(&self, only_subscribed: Option<bool>) -> Result<Vec<Channel>> {
        self.require_token()?;
        let payload: EditorChannelsPayload = self
            .send(
                self.get("channel/editor/available/all")
                    .query_opt("only_subscribed", only_subscribed)
                    .with_token(),
            )
            .await?;
        Ok(payload.channels)
    }

    /// Who holds a permission level in a channel. 0-based.
    ///
    /// `POST channel/{id}/permission/all/{page}`
    pub async fn channel_members(
        &self,
        id: i64,
        permission: i32,
        page: i32,
    ) -> Result<Page<ChannelProfile>> {
        self.require_token()?;
        let payload: PageablePayload<ChannelProfile> = self
            .send(
                self.post(format!("channel/{id}/permission/all/{page}"))
                    .with_token()
                    .json(json!({ "permission": permission })),
            )
            .await?;
        Ok(payload.into())
    }

    /// Gives an account a permission level in a channel, or takes it away with
    /// none.
    ///
    /// `POST channel/{id}/permission/manage`
    pub async fn channel_permission(
        &self,
        id: i64,
        profile_id: i64,
        permission: Option<i32>,
    ) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.post(format!("channel/{id}/permission/manage"))
                    .with_token()
                    .json(json!({ "target_profile_id": profile_id, "permission": permission })),
            )
            .await?;
        Ok(())
    }

    /// The accounts blocked from a channel. 0-based.
    ///
    /// `GET channel/{id}/block/all/{page}`
    pub async fn channel_blocked(&self, id: i64, page: i32) -> Result<Page<ChannelProfile>> {
        self.require_token()?;
        let payload: PageablePayload<ChannelProfile> = self
            .send(
                self.get(format!("channel/{id}/block/all/{page}"))
                    .with_token(),
            )
            .await?;
        Ok(payload.into())
    }

    /// One account's block from a channel, if it has one.
    ///
    /// `GET channel/{id}/block/{profile_id}`
    pub async fn channel_block(&self, id: i64, profile_id: i64) -> Result<Option<ChannelBlock>> {
        self.require_token()?;
        let payload: BlockPayload = self
            .send(
                self.get(format!("channel/{id}/block/{profile_id}"))
                    .with_token(),
            )
            .await?;
        Ok(payload.channel_block)
    }

    /// Blocks an account from a channel, or lifts it.
    ///
    /// `POST channel/{id}/block/manage`
    pub async fn channel_block_manage(
        &self,
        id: i64,
        block: &ChannelBlockRequest,
    ) -> Result<Option<ChannelBlock>> {
        self.require_token()?;
        let payload: BlockPayload = self
            .send(
                self.post(format!("channel/{id}/block/manage"))
                    .with_token()
                    .json(serde_json::to_value(block).unwrap_or_default()),
            )
            .await?;
        Ok(payload.channel_block)
    }

    // ---- shared ---------------------------------------------------------------------

    async fn ack_post(&self, path: String) -> Result<()> {
        self.require_token()?;
        let _: Ack = self.send(self.post(path).with_token()).await?;
        Ok(())
    }

    async fn upload_picture(
        &self,
        path: String,
        file_name: &str,
        mime: &'static str,
        bytes: Vec<u8>,
    ) -> Result<String> {
        self.require_token()?;
        let payload: UrlPayload = self
            .send(self.post(path).with_token().upload(Upload {
                part: "image",
                file_name: file_name.to_owned(),
                mime,
                bytes,
                fields: Vec::new(),
            }))
            .await?;
        Ok(payload.url)
    }
}
