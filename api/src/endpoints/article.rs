// SPDX-License-Identifier: GPL-3.0-or-later
//
// Posts: reading one, voting, reposts, muting, pinning, writing them, and the
// suggestions a channel takes from its readers.

use serde::Deserialize;
use serde_json::json;

use crate::client::{Ack, Client};
use crate::error::Result;
use crate::models::{Article, ArticlePayload, CommentVote, Page, ProfileCompact};

use super::PageablePayload;

#[derive(Deserialize)]
struct ArticlePayloadResponse {
    #[serde(default)]
    article: Article,
}

/// Where a post was seen, for [`Client::article_event`]. The app's own enum,
/// sent by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArticleEntryPoint {
    Feed,
    Latest,
    Channel,
    Article,
    Search,
}

impl ArticleEntryPoint {
    fn name(self) -> &'static str {
        match self {
            Self::Feed => "FEED",
            Self::Latest => "LATEST",
            Self::Channel => "CHANNEL",
            Self::Article => "ARTICLE",
            Self::Search => "SEARCH",
        }
    }
}

/// What happened to a post, for [`Client::article_event`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArticleEventKind {
    /// It was on screen.
    View,
    /// It was opened.
    Open,
}

impl ArticleEventKind {
    fn name(self) -> &'static str {
        match self {
            Self::View => "VIEW",
            Self::Open => "OPEN",
        }
    }
}

impl Client {
    /// One post.
    ///
    /// `GET article/{article_id}`
    pub async fn article(&self, article_id: i64) -> Result<Article> {
        let payload: ArticlePayloadResponse = self
            .send(self.get(format!("article/{article_id}")).with_token())
            .await?;
        Ok(payload.article)
    }

    /// Votes on a post, or withdraws the vote. The feed's heart is an up vote.
    ///
    /// `GET article/vote/{article_id}/{vote}` — on the scale every vote in the
    /// service shares, the app's `Vote` class: 0 none, 1 down, 2 up.
    pub async fn article_vote(&self, article_id: i64, vote: CommentVote) -> Result<()> {
        self.require_token()?;
        let vote = vote.raw();
        let _: Ack = self
            .send(
                self.get(format!("article/vote/{article_id}/{vote}"))
                    .with_token(),
            )
            .await?;
        Ok(())
    }

    /// Who voted on a post. 0-based.
    ///
    /// `POST article/votes/{article_id}/{page}?sort=`
    pub async fn article_votes(
        &self,
        article_id: i64,
        page: i32,
        sort: i32,
    ) -> Result<Page<ProfileCompact>> {
        let payload: PageablePayload<ProfileCompact> = self
            .send(
                self.post(format!("article/votes/{article_id}/{page}"))
                    .query("sort", sort)
                    .with_token(),
            )
            .await?;
        Ok(payload.into())
    }

    /// The posts that repost this one. 0-based.
    ///
    /// `GET article/reposts/{article_id}/{page}?sort=`
    pub async fn article_reposts(
        &self,
        article_id: i64,
        page: i32,
        sort: i32,
    ) -> Result<Page<Article>> {
        let payload: PageablePayload<Article> = self
            .send(
                self.get(format!("article/reposts/{article_id}/{page}"))
                    .query("sort", sort)
                    .with_token(),
            )
            .await?;
        Ok(payload.into())
    }

    /// Stops notifications about a post's comments.
    ///
    /// `GET article/mute/{article_id}`
    pub async fn article_mute(&self, article_id: i64) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(self.get(format!("article/mute/{article_id}")).with_token())
            .await?;
        Ok(())
    }

    /// `GET article/unmute/{article_id}`
    pub async fn article_unmute(&self, article_id: i64) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.get(format!("article/unmute/{article_id}"))
                    .with_token(),
            )
            .await?;
        Ok(())
    }

    /// Pins or unpins a post at the top of its channel. The channel's own.
    ///
    /// `GET article/edit/pinned/{article_id}?is_pinned=`
    pub async fn article_pin(&self, article_id: i64, pinned: bool) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.get(format!("article/edit/pinned/{article_id}"))
                    .query("is_pinned", pinned)
                    .with_token(),
            )
            .await?;
        Ok(())
    }

    /// Reports that posts were seen or opened.
    ///
    /// `POST article/event` with the post ids, how they were seen and from
    /// where. This is the official client's view counter. It is here for
    /// completeness; whether this client should report what its viewer reads
    /// is a decision for its interface, and it does not.
    ///
    /// The two enums are sent by name, which is Jackson's default for an enum
    /// without a declared value; that has not been observed on the wire.
    pub async fn article_event(
        &self,
        articles: &[i64],
        kind: ArticleEventKind,
        from: ArticleEntryPoint,
    ) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(self.post("article/event").with_token().json(json!({
                "articles": articles,
                "type": kind.name(),
                "entry_point": from.name(),
            })))
            .await?;
        Ok(())
    }

    /// Publishes a post in a channel, or reposts one when `repost` names it.
    /// Answers with the post as stored.
    ///
    /// `POST article/create/{channel_id}`. The body's `payload` is the post's
    /// blocks serialised to a JSON *string* — the app writes it with
    /// `writeValueAsString` — not a nested object.
    pub async fn article_create(
        &self,
        channel_id: i64,
        payload: &ArticlePayload,
        signed: bool,
        repost: Option<i64>,
    ) -> Result<Article> {
        self.require_token()?;
        let response: ArticlePayloadResponse = self
            .send(
                self.post(format!("article/create/{channel_id}"))
                    .with_token()
                    .json(json!({
                        "payload": stringify(payload),
                        "repost_article_id": repost,
                        "is_signed": signed,
                    })),
            )
            .await?;
        Ok(response.article)
    }

    /// Rewrites a post.
    ///
    /// `POST article/edit/{article_id}`, the same body as creating one.
    pub async fn article_edit(
        &self,
        article_id: i64,
        payload: &ArticlePayload,
        signed: bool,
    ) -> Result<Article> {
        self.require_token()?;
        let response: ArticlePayloadResponse = self
            .send(
                self.post(format!("article/edit/{article_id}"))
                    .with_token()
                    .json(json!({
                        "payload": stringify(payload),
                        "repost_article_id": null,
                        "is_signed": signed,
                    })),
            )
            .await?;
        Ok(response.article)
    }

    /// Deletes a post.
    ///
    /// `POST article/delete/{article_id}`
    pub async fn article_delete(&self, article_id: i64) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.post(format!("article/delete/{article_id}"))
                    .with_token(),
            )
            .await?;
        Ok(())
    }

    // ---- suggestions ------------------------------------------------------

    /// One suggested post.
    ///
    /// `GET article/suggestion/{article_id}`
    pub async fn suggestion(&self, article_id: i64) -> Result<Article> {
        let payload: ArticlePayloadResponse = self
            .send(
                self.get(format!("article/suggestion/{article_id}"))
                    .with_token(),
            )
            .await?;
        Ok(payload.article)
    }

    /// The posts readers have suggested to a channel. 0-based.
    ///
    /// `POST article/suggestion/all/{page}` with the channel in the body.
    pub async fn suggestions(&self, channel_id: i64, page: i32) -> Result<Page<Article>> {
        self.require_token()?;
        let payload: PageablePayload<Article> = self
            .send(
                self.post(format!("article/suggestion/all/{page}"))
                    .with_token()
                    .json(json!({ "channel_id": channel_id })),
            )
            .await?;
        Ok(payload.into())
    }

    /// Suggests a post to a channel.
    ///
    /// `POST article/suggestion/create/{channel_id}`
    pub async fn suggestion_create(
        &self,
        channel_id: i64,
        payload: &ArticlePayload,
    ) -> Result<Article> {
        self.require_token()?;
        let response: ArticlePayloadResponse = self
            .send(
                self.post(format!("article/suggestion/create/{channel_id}"))
                    .with_token()
                    .json(json!({ "payload": stringify(payload) })),
            )
            .await?;
        Ok(response.article)
    }

    /// `POST article/suggestion/edit/{article_id}`
    pub async fn suggestion_edit(
        &self,
        article_id: i64,
        payload: &ArticlePayload,
    ) -> Result<Article> {
        self.require_token()?;
        let response: ArticlePayloadResponse = self
            .send(
                self.post(format!("article/suggestion/edit/{article_id}"))
                    .with_token()
                    .json(json!({ "payload": stringify(payload) })),
            )
            .await?;
        Ok(response.article)
    }

    /// `POST article/suggestion/delete/{article_id}`
    pub async fn suggestion_delete(&self, article_id: i64) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.post(format!("article/suggestion/delete/{article_id}"))
                    .with_token(),
            )
            .await?;
        Ok(())
    }

    /// Publishes a suggested post in the channel it was suggested to.
    ///
    /// `POST article/suggestion/publish/{article_id}?is_signed=`
    pub async fn suggestion_publish(&self, article_id: i64, signed: bool) -> Result<Article> {
        self.require_token()?;
        let response: ArticlePayloadResponse = self
            .send(
                self.post(format!("article/suggestion/publish/{article_id}"))
                    .query("is_signed", signed)
                    .with_token(),
            )
            .await?;
        Ok(response.article)
    }
}

/// A post's body as the server wants it: the payload as a JSON string.
fn stringify(payload: &ArticlePayload) -> String {
    serde_json::to_string(payload).unwrap_or_else(|_| "{}".to_owned())
}
