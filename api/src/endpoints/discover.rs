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

/// Everything the front page shows, in one answer.
///
/// Since 11.0 the app asks for this once instead of for each part on its own:
/// the curated cards, recommendations, what is being watched and discussed, a
/// page of collections and the week's comments.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DiscoverHub {
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub interesting: Vec<Interesting>,
    /// Empty for an anonymous client.
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub recommendations: Vec<Release>,
    /// What to pass back as `recommendations_previous_page` for the next set:
    /// the service rotates through them. -1 when there are none.
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub recommendations_page: i32,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub watching: Vec<Release>,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub discussing: Vec<Release>,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub collections: Vec<Collection>,
    /// What to pass back as `collections_previous_page`.
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub collections_page: i32,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub collections_page_count: i32,
    /// Each comment's release in full, references already followed.
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub comments: Vec<Comment>,
}

impl Client {
    /// The front page at once. Pass -1 for both the first time, then the
    /// pages the previous answer gave, to move on to the next set.
    ///
    /// `GET discover/hub?recommendations_previous_page=&collections_previous_page=`,
    /// anonymous or not.
    pub async fn discover_hub(
        &self,
        recommendations_previous_page: i32,
        collections_previous_page: i32,
    ) -> Result<DiscoverHub> {
        let mut body: Value = self
            .send(
                self.get("discover/hub")
                    .query(
                        "recommendations_previous_page",
                        recommendations_previous_page,
                    )
                    .query("collections_previous_page", collections_previous_page)
                    .with_token(),
            )
            .await?;
        follow_release_references(&mut body);
        serde_json::from_value(body.clone()).map_err(|source| crate::Error::Decode {
            source,
            body: body.to_string(),
        })
    }

    /// The front page's curated cards.
    ///
    /// From [`Self::discover_hub`]: the app stopped asking
    /// `discover/interesting` in 11.0.
    pub async fn discover_interesting(&self) -> Result<Vec<Interesting>> {
        Ok(self.discover_hub(-1, -1).await?.interesting)
    }

    /// Comments from across the service that are getting attention.
    ///
    /// From [`Self::discover_hub`]: the app stopped asking `discover/comments`
    /// in 11.0.
    pub async fn discover_comments(&self) -> Result<Vec<Comment>> {
        Ok(self.discover_hub(-1, -1).await?.comments)
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

/// Puts each comment's release back in full where the hub gave only its
/// number.
///
/// The service writes with Jackson's object identity: a release is written out
/// once with an `@id`, and every later appearance — a second comment on it, or
/// a comment on one already in the watching list — is just that number. The
/// numbering is per type, so only objects found where releases go are taken:
/// a collection's `@id` 3 is not a release's.
fn follow_release_references(body: &mut Value) {
    use std::collections::HashMap;

    fn remember(release: &Value, known: &mut HashMap<i64, Value>) {
        if let Some(id) = release.get("@id").and_then(Value::as_i64) {
            known.entry(id).or_insert_with(|| release.clone());
        }
    }

    let mut known: HashMap<i64, Value> = HashMap::new();
    for list in ["recommendations", "watching", "discussing"] {
        for release in body[list].as_array().into_iter().flatten() {
            remember(release, &mut known);
        }
    }
    for collection in body["collections"].as_array().into_iter().flatten() {
        for list in ["releases", "releases_preview"] {
            for release in collection[list].as_array().into_iter().flatten() {
                remember(release, &mut known);
            }
        }
    }
    // In order: a comment may point back at a release an earlier comment
    // gave in full.
    if let Some(comments) = body["comments"].as_array_mut() {
        for comment in comments {
            let Some(release) = comment.get_mut("release") else {
                continue;
            };
            match release.as_i64() {
                Some(id) => {
                    if let Some(full) = known.get(&id) {
                        *release = full.clone();
                    }
                }
                None => remember(release, &mut known),
            }
        }
    }
}

/// The body of a plain search: the words, matched against names. The app's
/// `SearchRequest` also carries `searchBy`, which only the release search
/// reads; it is sent as the default, by title.
fn plain(query: &str) -> Value {
    json!({ "query": query, "searchBy": SearchBy::Title.raw() })
}
