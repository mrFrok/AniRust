// SPDX-License-Identifier: GPL-3.0-or-later
//
// Collections: reading them, favouriting them, and keeping the account's own.

use serde::Deserialize;
use serde_json::json;

use crate::client::{Ack, Client, Upload};
use crate::error::Result;
use crate::models::{Collection, Page, Release};

use super::PageablePayload;

/// A collection, and how much of it the account has in each of its lists.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct CollectionView {
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub collection: Collection,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub watching_count: i64,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub plan_count: i64,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub completed_count: i64,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub hold_on_count: i64,
    #[serde(deserialize_with = "crate::serde_ext::nullable")]
    pub dropped_count: i64,
}

/// The order of [`Client::collections`]. Observed, not read: on the live
/// list, 0 came back oldest first, 1 by how many favourited it, 2 the
/// popular among recent ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CollectionSort {
    Oldest,
    #[default]
    MostFavourited,
    Trending,
}

impl CollectionSort {
    #[must_use]
    pub fn raw(self) -> i32 {
        match self {
            Self::Oldest => 0,
            Self::MostFavourited => 1,
            Self::Trending => 2,
        }
    }
}

#[derive(Deserialize)]
struct CollectionPayload {
    #[serde(default)]
    collection: Collection,
}

#[derive(Deserialize)]
struct UrlPayload {
    #[serde(default)]
    url: String,
}

impl Client {
    /// One collection, and how much of it is in each of the account's lists.
    ///
    /// `GET collection/{id}`
    pub async fn collection(&self, id: i64) -> Result<CollectionView> {
        self.send(self.get(format!("collection/{id}")).with_token())
            .await
    }

    /// All collections. 0-based.
    ///
    /// `GET collection/all/{page}?previous_page=&where=&sort=`. `where` made
    /// no difference to anything observed without an account, and is sent as
    /// 0; `previous_page` keeps paging stable, as the recommendations' does.
    pub async fn collections(&self, page: i32, sort: CollectionSort) -> Result<Page<Collection>> {
        let payload: PageablePayload<Collection> = self
            .send(
                self.get(format!("collection/all/{page}"))
                    .query("previous_page", (page - 1).max(0))
                    .query("where", 0)
                    .query("sort", sort.raw())
                    .with_token(),
            )
            .await?;
        Ok(payload.into())
    }

    /// An account's collections. 0-based.
    ///
    /// `GET collection/all/profile/{profile_id}/{page}`
    pub async fn profile_collections(
        &self,
        profile_id: i64,
        page: i32,
    ) -> Result<Page<Collection>> {
        self.collection_page(format!("collection/all/profile/{profile_id}/{page}"), None)
            .await
    }

    /// The collections a release is in. 0-based.
    ///
    /// `GET collection/all/release/{release_id}/{page}?sort=`
    pub async fn release_collections(
        &self,
        release_id: i64,
        page: i32,
        sort: CollectionSort,
    ) -> Result<Page<Collection>> {
        self.collection_page(
            format!("collection/all/release/{release_id}/{page}"),
            Some(sort.raw()),
        )
        .await
    }

    /// The releases in a collection. 0-based.
    ///
    /// `GET collection/{id}/releases/{page}`
    pub async fn collection_releases(&self, id: i64, page: i32) -> Result<Page<Release>> {
        let payload: PageablePayload<Release> = self
            .send(
                self.get(format!("collection/{id}/releases/{page}"))
                    .with_token(),
            )
            .await?;
        Ok(payload.into())
    }

    /// The account's favourite collections. 0-based.
    ///
    /// `GET collectionFavorite/all/{page}`
    pub async fn favorite_collections(&self, page: i32) -> Result<Page<Collection>> {
        self.require_token()?;
        self.collection_page(format!("collectionFavorite/all/{page}"), None)
            .await
    }

    /// `GET collectionFavorite/add/{id}`
    pub async fn collection_favorite_add(&self, id: i64) -> Result<()> {
        self.ack_get(format!("collectionFavorite/add/{id}")).await
    }

    /// `GET collectionFavorite/delete/{id}`
    pub async fn collection_favorite_delete(&self, id: i64) -> Result<()> {
        self.ack_get(format!("collectionFavorite/delete/{id}"))
            .await
    }

    // ---- the account's own -------------------------------------------------------

    /// Creates a collection. Answers with it as stored.
    ///
    /// `POST collectionMy/create`
    pub async fn collection_create(
        &self,
        title: &str,
        description: &str,
        private: bool,
        releases: &[i64],
    ) -> Result<Collection> {
        self.require_token()?;
        let payload: CollectionPayload = self
            .send(
                self.post("collectionMy/create")
                    .with_token()
                    .json(edit_body(title, description, private, releases)),
            )
            .await?;
        Ok(payload.collection)
    }

    /// Rewrites a collection: its title, description, privacy, and the whole
    /// list of its releases.
    ///
    /// `POST collectionMy/edit/{id}`
    pub async fn collection_edit(
        &self,
        id: i64,
        title: &str,
        description: &str,
        private: bool,
        releases: &[i64],
    ) -> Result<Collection> {
        self.require_token()?;
        let payload: CollectionPayload = self
            .send(
                self.post(format!("collectionMy/edit/{id}"))
                    .with_token()
                    .json(edit_body(title, description, private, releases)),
            )
            .await?;
        Ok(payload.collection)
    }

    /// `GET collectionMy/delete/{id}`
    pub async fn collection_delete(&self, id: i64) -> Result<()> {
        self.ack_get(format!("collectionMy/delete/{id}")).await
    }

    /// Adds one release to a collection.
    ///
    /// `GET collectionMy/release/add/{id}?release_id=`
    pub async fn collection_add_release(&self, id: i64, release_id: i64) -> Result<()> {
        self.require_token()?;
        let _: Ack = self
            .send(
                self.get(format!("collectionMy/release/add/{id}"))
                    .query("release_id", release_id)
                    .with_token(),
            )
            .await?;
        Ok(())
    }

    /// Every release in one of the account's collections, unpaged — what the
    /// edit screen reads to show the whole list.
    ///
    /// `GET collectionMy/{id}/releases`
    pub async fn my_collection_releases(&self, id: i64) -> Result<Vec<Release>> {
        self.require_token()?;
        let payload: PageablePayload<Release> = self
            .send(self.get(format!("collectionMy/{id}/releases")).with_token())
            .await?;
        Ok(payload.content)
    }

    /// Replaces a collection's cover. Answers with its new address.
    ///
    /// `POST collectionMy/editImage/{id}`, multipart, the file as the part
    /// `image` with the empty `name` part beside it, as the avatar goes.
    pub async fn collection_image(
        &self,
        id: i64,
        file_name: &str,
        mime: &'static str,
        bytes: Vec<u8>,
    ) -> Result<String> {
        self.require_token()?;
        let payload: UrlPayload = self
            .send(
                self.post(format!("collectionMy/editImage/{id}"))
                    .with_token()
                    .upload(Upload {
                        part: "image",
                        file_name: file_name.to_owned(),
                        mime,
                        bytes,
                        fields: vec![("name", String::new())],
                    }),
            )
            .await?;
        Ok(payload.url)
    }

    // ---- shared -----------------------------------------------------------------------

    async fn collection_page(&self, path: String, sort: Option<i32>) -> Result<Page<Collection>> {
        let payload: PageablePayload<Collection> = self
            .send(self.get(path).query_opt("sort", sort).with_token())
            .await?;
        Ok(payload.into())
    }

    async fn ack_get(&self, path: String) -> Result<()> {
        self.require_token()?;
        let _: Ack = self.send(self.get(path).with_token()).await?;
        Ok(())
    }
}

fn edit_body(title: &str, description: &str, private: bool, releases: &[i64]) -> serde_json::Value {
    json!({
        "title": title,
        "description": description,
        "is_private": private,
        "releases": releases,
    })
}
