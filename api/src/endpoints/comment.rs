// SPDX-License-Identifier: GPL-3.0-or-later
//
// Comments, on all three things that carry them.
//
// The official client has three interfaces of eleven-odd endpoints each — one
// for releases, one for posts, one for collections — that differ in nothing
// but the first word of the path. Here they are one set of methods taking a
// `CommentTarget`, which is thirty-one requests in eleven functions and one
// place to get each of them right.
//
// Two quirks are kept rather than smoothed over, because the server holds
// them: `replies` is a POST although it only reads, and `votes` (who voted on
// a comment) is a POST on posts and a GET on the other two.

use serde::Deserialize;
use serde_json::json;

use crate::client::{Ack, Client};
use crate::error::Result;
use crate::models::{
    Comment, CommentModeration, CommentSort, CommentTarget, CommentVote, Page, ProfileCompact,
};

use super::PageablePayload;

#[derive(Deserialize)]
struct AddedPayload {
    #[serde(default)]
    comment: Comment,
}

impl Client {
    /// A page of comments on a release, post or collection. 0-based.
    ///
    /// `GET {target}/comment/all/{id}/{page}?sort=`. Readable without an
    /// account; the token is sent when there is one, which is what fills in
    /// this account's own vote on each comment.
    pub async fn comments(
        &self,
        target: CommentTarget,
        id: i64,
        page: i32,
        sort: CommentSort,
    ) -> Result<Page<Comment>> {
        let segment = target.segment();
        let payload: PageablePayload<Comment> = self
            .send(
                self.get(format!("{segment}/comment/all/{id}/{page}"))
                    .query("sort", sort.raw())
                    .with_token(),
            )
            .await?;
        Ok(payload.into())
    }

    /// The most popular comments on a post, as shown under it in the feed.
    ///
    /// `GET article/comment/all/{article_id}/popular` — posts only.
    pub async fn article_comments_popular(&self, article_id: i64) -> Result<Vec<Comment>> {
        let payload: PageablePayload<Comment> = self
            .send(
                self.get(format!("article/comment/all/{article_id}/popular"))
                    .with_token(),
            )
            .await?;
        Ok(payload.content)
    }

    /// One comment, by its own id.
    ///
    /// `GET {target}/comment/{comment_id}` — the app names the parameter after
    /// the target, but what it answers with is the comment of that id.
    pub async fn comment(&self, target: CommentTarget, comment_id: i64) -> Result<Comment> {
        let segment = target.segment();
        self.send(
            self.get(format!("{segment}/comment/{comment_id}"))
                .with_token(),
        )
        .await
    }

    /// The replies under a comment. 0-based.
    ///
    /// `POST {target}/comment/replies/{comment_id}/{page}?sort=` — a POST,
    /// although nothing is written.
    pub async fn comment_replies(
        &self,
        target: CommentTarget,
        comment_id: i64,
        page: i32,
        sort: CommentSort,
    ) -> Result<Page<Comment>> {
        let segment = target.segment();
        let payload: PageablePayload<Comment> = self
            .send(
                self.post(format!("{segment}/comment/replies/{comment_id}/{page}"))
                    .query("sort", sort.raw())
                    .with_token(),
            )
            .await?;
        Ok(payload.into())
    }

    /// Everything one account has written on one kind of thing. 0-based.
    ///
    /// `GET {target}/comment/all/profile/{profile_id}/{page}?sort=`
    pub async fn profile_comments(
        &self,
        target: CommentTarget,
        profile_id: i64,
        page: i32,
        sort: CommentSort,
    ) -> Result<Page<Comment>> {
        let segment = target.segment();
        let payload: PageablePayload<Comment> = self
            .send(
                self.get(format!("{segment}/comment/all/profile/{profile_id}/{page}"))
                    .query("sort", sort.raw())
                    .with_token(),
            )
            .await?;
        Ok(payload.into())
    }

    /// Writes a comment, or a reply when `reply_to` names the comment and its
    /// author. Answers with the comment as stored.
    ///
    /// `POST {target}/comment/add/{id}` with a JSON body. The server refuses
    /// comments that are too short, too long, or over a rate limit; those
    /// come back as [`crate::Error::Api`] with the code, for the caller to
    /// word.
    pub async fn comment_add(
        &self,
        target: CommentTarget,
        id: i64,
        message: &str,
        spoiler: bool,
        reply_to: Option<(i64, i64)>,
    ) -> Result<Comment> {
        self.require_token()?;
        let segment = target.segment();
        let (parent, profile) = reply_to.map_or((None, None), |(c, p)| (Some(c), Some(p)));
        let payload: AddedPayload = self
            .send(
                self.post(format!("{segment}/comment/add/{id}"))
                    .with_token()
                    .json(json!({
                        "message": message,
                        "spoiler": spoiler,
                        "parent_comment_id": parent,
                        "reply_to_profile_id": profile,
                    })),
            )
            .await?;
        Ok(payload.comment)
    }

    /// Rewrites the account's own comment.
    ///
    /// `POST {target}/comment/edit/{comment_id}`
    pub async fn comment_edit(
        &self,
        target: CommentTarget,
        comment_id: i64,
        message: &str,
        spoiler: bool,
    ) -> Result<()> {
        self.require_token()?;
        let segment = target.segment();
        let _: Ack = self
            .send(
                self.post(format!("{segment}/comment/edit/{comment_id}"))
                    .with_token()
                    .json(json!({ "message": message, "spoiler": spoiler })),
            )
            .await?;
        Ok(())
    }

    /// Deletes the account's own comment.
    ///
    /// `GET {target}/comment/delete/{comment_id}`
    pub async fn comment_delete(&self, target: CommentTarget, comment_id: i64) -> Result<()> {
        self.require_token()?;
        let segment = target.segment();
        let _: Ack = self
            .send(
                self.get(format!("{segment}/comment/delete/{comment_id}"))
                    .with_token(),
            )
            .await?;
        Ok(())
    }

    /// Votes a comment up or down, or withdraws the vote.
    ///
    /// `GET {target}/comment/vote/{comment_id}/{vote}`
    pub async fn comment_vote(
        &self,
        target: CommentTarget,
        comment_id: i64,
        vote: CommentVote,
    ) -> Result<()> {
        self.require_token()?;
        let segment = target.segment();
        let vote = vote.raw();
        let _: Ack = self
            .send(
                self.get(format!("{segment}/comment/vote/{comment_id}/{vote}"))
                    .with_token(),
            )
            .await?;
        Ok(())
    }

    /// Who voted on a comment. 0-based.
    ///
    /// `{target}/comment/votes/{comment_id}/{page}` — a POST on posts, a GET
    /// on releases and collections. That is how the server declares them.
    pub async fn comment_votes(
        &self,
        target: CommentTarget,
        comment_id: i64,
        page: i32,
    ) -> Result<Page<ProfileCompact>> {
        let segment = target.segment();
        let path = format!("{segment}/comment/votes/{comment_id}/{page}");
        let request = match target {
            CommentTarget::Article => self.post(path),
            CommentTarget::Release | CommentTarget::Collection => self.get(path),
        };
        let payload: PageablePayload<ProfileCompact> = self.send(request.with_token()).await?;
        Ok(payload.into())
    }

    /// A moderator's decision about a comment.
    ///
    /// `POST {target}/comment/process/{comment_id}` — refused for anyone
    /// without the privilege.
    pub async fn comment_moderate(
        &self,
        target: CommentTarget,
        comment_id: i64,
        decision: &CommentModeration,
    ) -> Result<()> {
        self.require_token()?;
        let segment = target.segment();
        let _: Ack = self
            .send(
                self.post(format!("{segment}/comment/process/{comment_id}"))
                    .with_token()
                    .json(serde_json::to_value(decision).unwrap_or_default()),
            )
            .await?;
        Ok(())
    }
}
