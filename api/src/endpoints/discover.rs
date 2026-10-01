// SPDX-License-Identifier: GPL-3.0-or-later
//
// Finding things: the front page's curated cards and comments, the week's
// schedule, the service's own configuration, and search in every place the
// official client has a search field.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::client::Client;
use crate::error::Result;
use crate::models::{
    Article, Channel, ChannelProfile, Collection, Comment, FeedSearch, Interesting, Page, Profile,
    ProfileList, Release, Schedule, SearchBy,
};

use super::PageablePayload;

#[derive(Deserialize)]
struct FeedSearchPayload {
    #[serde(default)]
    articles: Option<PageablePayload<Article>>,
    #[serde(default)]
    channels: Option<PageablePayload<Channel>>,
    #[serde(default)]
    blogs: Option<PageablePayload<Channel>>,
    #[serde(default)]
    tags: Option<PageablePayload<String>>,
}

/// Narrowing for [`Client::search_channels`]. Every field unset is "any".
#[derive(Debug, Clone, Default)]
pub struct ChannelSearch {
    /// Personal blogs only, or channels only.
    pub blogs: Option<bool>,
    /// Only those the account follows, or only those it does not.
    pub subscribed: Option<bool>,
    /// Only those where the account has this permission level.
    pub permission: Option<i32>,
}

impl Client {
    /// The front page's curated cards.
    ///
    /// `POST discover/interesting`, anonymous.
    pub async fn discover_interesting(&self) -> Result<Vec<Interesting>> {
        let payload: PageablePayload<Interesting> =
            self.send(self.post("discover/interesting")).await?;
        Ok(payload.content)
    }

    /// Comments from across the service that are getting attention.
    ///
    /// `POST discover/comments`, anonymous.
    pub async fn discover_comments(&self) -> Result<Vec<Comment>> {
        let payload: PageablePayload<Comment> = self.send(self.post("discover/comments")).await?;
        Ok(payload.content)
    }

    /// Which releases get an episode on which day of the week.
    ///
    /// `GET schedule`, anonymous.
    pub async fn schedule(&self) -> Result<Schedule> {
        self.send(self.get("schedule")).await
    }

    /// The service's settings for the official player. Its shape is the
    /// service's business and is returned as it came.
    ///
    /// `GET config/anixplayer`, anonymous.
    pub async fn config_player(&self) -> Result<Value> {
        self.send(self.get("config/anixplayer")).await
    }

    /// Feature switches the service sends the official client, for the given
    /// version of it. Returned as they came.
    ///
    /// `GET config/toggles?version_code=&is_beta=&is_api_alt=`
    pub async fn config_toggles(
        &self,
        version_code: i32,
        beta: bool,
        api_alt: bool,
    ) -> Result<Value> {
        self.send(
            self.get("config/toggles")
                .query("version_code", version_code)
                .query("is_beta", beta)
                .query("is_api_alt", api_alt)
                .with_token(),
        )
        .await
    }

    /// Addresses the service hands the official client. Returned as they came.
    ///
    /// `GET config/urls?version_code=&is_beta=`
    pub async fn config_urls(&self, version_code: i32, beta: bool) -> Result<Value> {
        self.send(
            self.get("config/urls")
                .query("version_code", version_code)
                .query("is_beta", beta)
                .with_token(),
        )
        .await
    }

    // ---- search ------------------------------------------------------------

    /// People, by login. 0-based.
    ///
    /// `POST search/profiles/{page}`
    pub async fn search_profiles(&self, query: &str, page: i32) -> Result<Page<Profile>> {
        self.search_page(format!("search/profiles/{page}"), plain(query), None)
            .await
    }

    /// Posts, anywhere or in one channel. 0-based.
    ///
    /// `POST search/articles/{page}` with `channel_id` 0 for anywhere.
    pub async fn search_articles(
        &self,
        query: &str,
        channel_id: Option<i64>,
        page: i32,
    ) -> Result<Page<Article>> {
        self.search_page(
            format!("search/articles/{page}"),
            json!({ "query": query, "channel_id": channel_id.unwrap_or(0) }),
            None,
        )
        .await
    }

    /// Channels and blogs. 0-based.
    ///
    /// `POST search/channels/{page}`
    pub async fn search_channels(
        &self,
        query: &str,
        narrow: &ChannelSearch,
        page: i32,
    ) -> Result<Page<Channel>> {
        self.search_page(
            format!("search/channels/{page}"),
            json!({
                "query": query,
                "is_blog": narrow.blogs,
                "is_subscribed": narrow.subscribed,
                "permission": narrow.permission,
            }),
            None,
        )
        .await
    }

    /// A channel's subscribers, by login. 0-based.
    ///
    /// `POST search/channel/{channel_id}/subscribers/{page}`
    pub async fn search_subscribers(
        &self,
        channel_id: i64,
        query: &str,
        page: i32,
    ) -> Result<Page<ChannelProfile>> {
        self.search_page(
            format!("search/channel/{channel_id}/subscribers/{page}"),
            plain(query),
            None,
        )
        .await
    }

    /// Collections. 0-based.
    ///
    /// `POST search/collections/{page}`
    pub async fn search_collections(&self, query: &str, page: i32) -> Result<Page<Collection>> {
        self.search_page(format!("search/collections/{page}"), plain(query), None)
            .await
    }

    /// The account's favourite collections. 0-based.
    ///
    /// `POST search/favoriteCollections/{page}`
    pub async fn search_favorite_collections(
        &self,
        query: &str,
        page: i32,
    ) -> Result<Page<Collection>> {
        self.search_page(
            format!("search/favoriteCollections/{page}"),
            plain(query),
            None,
        )
        .await
    }

    /// One account's collections, marking which hold `release_id` — what the
    /// "add to collection" picker searches. 0-based.
    ///
    /// `POST search/profileCollections/{profile_id}/{page}?release_id=`
    pub async fn search_profile_collections(
        &self,
        profile_id: i64,
        release_id: i64,
        query: &str,
        page: i32,
    ) -> Result<Page<Collection>> {
        self.search_page(
            format!("search/profileCollections/{profile_id}/{page}"),
            plain(query),
            Some(release_id),
        )
        .await
    }

    /// The account's favourites. 0-based.
    ///
    /// `POST search/favorites/{page}`
    pub async fn search_favorites(&self, query: &str, page: i32) -> Result<Page<Release>> {
        self.search_page(format!("search/favorites/{page}"), plain(query), None)
            .await
    }

    /// The account's history. 0-based.
    ///
    /// `POST search/history/{page}`
    pub async fn search_history(&self, query: &str, page: i32) -> Result<Page<Release>> {
        self.search_page(format!("search/history/{page}"), plain(query), None)
            .await
    }

    /// One of the account's lists. 0-based.
    ///
    /// `POST search/profile/list/{status}/{page}`
    pub async fn search_list(
        &self,
        list: ProfileList,
        query: &str,
        page: i32,
    ) -> Result<Page<Release>> {
        let status = list.raw();
        self.search_page(
            format!("search/profile/list/{status}/{page}"),
            plain(query),
            None,
        )
        .await
    }

    /// The feed: posts, channels, blogs and tags at once. 0-based.
    ///
    /// `POST search/feed/{page}`
    pub async fn search_feed(&self, query: &str, page: i32) -> Result<FeedSearch> {
        self.require_token()?;
        let payload: FeedSearchPayload = self
            .send(
                self.post(format!("search/feed/{page}"))
                    .with_token()
                    .json(plain(query)),
            )
            .await?;
        Ok(FeedSearch {
            articles: payload.articles.map(Into::into).unwrap_or_default(),
            channels: payload.channels.map(Into::into).unwrap_or_default(),
            blogs: payload.blogs.map(Into::into).unwrap_or_default(),
            tags: payload.tags.map(Into::into).unwrap_or_default(),
        })
    }

    /// Every paged search, which differ only in path and body.
    async fn search_page<T: for<'de> Deserialize<'de>>(
        &self,
        path: String,
        body: Value,
        release_id: Option<i64>,
    ) -> Result<Page<T>> {
        self.require_token()?;
        let payload: PageablePayload<T> = self
            .send(
                self.post(path)
                    .query_opt("release_id", release_id)
                    .with_token()
                    .json(body),
            )
            .await?;
        Ok(payload.into())
    }
}

/// The body of a plain search: the words, matched against names. The app's
/// `SearchRequest` also carries `searchBy`, which only the release search
/// reads; it is sent as the default, by title.
fn plain(query: &str) -> Value {
    json!({ "query": query, "searchBy": SearchBy::Title.raw() })
}
