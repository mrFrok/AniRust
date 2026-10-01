// SPDX-License-Identifier: GPL-3.0-or-later
//
// The feed, and the channels it is made of.

use crate::client::{Ack, Client};
use crate::error::Result;
use crate::models::{Article, Page};

use super::PageablePayload;

// ---------------------------------------------------------------------------
// The feed
// ---------------------------------------------------------------------------

impl Client {
    /// Posts from the channels the account is subscribed to, newest first.
    /// Pages are 0-based. Requires a token.
    ///
    /// `GET feed/all/{page}`. The app also sends `date`, an anchor that keeps
    /// pages stable while new posts arrive; it starts at 0, which is what is
    /// sent here, and `channel_id`, which narrows the feed to one channel and
    /// is left out.
    pub async fn feed(&self, page: i32) -> Result<Page<Article>> {
        self.require_token()?;
        let payload: PageablePayload<Article> = self
            .send(
                self.get(format!("feed/all/{page}"))
                    .query("date", 0)
                    .with_token(),
            )
            .await?;
        Ok(payload.into())
    }

    /// The newest posts from every channel, subscribed or not. Pages are
    /// 0-based. Requires a token: the server answers 401 without one, although
    /// nothing in it is personal.
    ///
    /// `GET feed/latest/all/{page}`
    pub async fn feed_latest(&self, page: i32) -> Result<Page<Article>> {
        self.require_token()?;
        let payload: PageablePayload<Article> = self
            .send(self.get(format!("feed/latest/all/{page}")).with_token())
            .await?;
        Ok(payload.into())
    }

    /// Subscribes the account to a channel, which puts its posts in
    /// [`Self::feed`].
    ///
    /// `POST channel/subscribe/{channel_id}`
    pub async fn channel_subscribe(&self, channel_id: i64) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.post(format!("channel/subscribe/{channel_id}"))
                    .with_token(),
            )
            .await?;
        Ok(())
    }

    /// `POST channel/unsubscribe/{channel_id}`
    pub async fn channel_unsubscribe(&self, channel_id: i64) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.post(format!("channel/unsubscribe/{channel_id}"))
                    .with_token(),
            )
            .await?;
        Ok(())
    }
}
