// SPDX-License-Identifier: GPL-3.0-or-later
//
// Finding things: the curated cards on the front page, the week's schedule,
// collections, and what a search of the feed answers with.

use serde::{Deserialize, Deserializer, Serialize};

use crate::serde_ext::nullable;

use super::{Article, Channel, Embedded, Page, Profile, Release};

/// One card of the front page's "interesting" row: a picture and a line that
/// lead somewhere. Where depends on `kind`; every card observed was kind 1,
/// with `action` a release id written as a string — see [`Self::release_id`].
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Interesting {
    #[serde(deserialize_with = "nullable")]
    pub id: i64,
    #[serde(deserialize_with = "nullable")]
    pub title: String,
    #[serde(deserialize_with = "nullable")]
    pub description: String,
    #[serde(deserialize_with = "nullable")]
    pub image: String,
    #[serde(deserialize_with = "nullable")]
    pub action: String,
    #[serde(rename = "type", deserialize_with = "nullable")]
    pub kind: i32,
    #[serde(deserialize_with = "nullable")]
    pub is_hidden: bool,
}

impl Interesting {
    /// The release a card leads to, when it leads to one. Kind 1, observed on
    /// every card of the live front page, with the id as a string; other kinds
    /// have not been seen and are left alone rather than guessed at.
    #[must_use]
    pub fn release_id(&self) -> Option<i64> {
        (self.kind == 1)
            .then(|| self.action.trim().parse().ok())
            .flatten()
    }
}

/// The week's airing schedule: which releases get an episode on which day.
///
/// Every release in it arrived in full when observed, but each carries an
/// `@id`, so a release airing on two days would come back the second time as
/// a reference. Those are dropped: the day lists the releases that came in
/// full, and a release appears under the first day it airs on.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Schedule {
    #[serde(deserialize_with = "releases_in_full")]
    pub monday: Vec<Release>,
    #[serde(deserialize_with = "releases_in_full")]
    pub tuesday: Vec<Release>,
    #[serde(deserialize_with = "releases_in_full")]
    pub wednesday: Vec<Release>,
    #[serde(deserialize_with = "releases_in_full")]
    pub thursday: Vec<Release>,
    #[serde(deserialize_with = "releases_in_full")]
    pub friday: Vec<Release>,
    #[serde(deserialize_with = "releases_in_full")]
    pub saturday: Vec<Release>,
    #[serde(deserialize_with = "releases_in_full")]
    pub sunday: Vec<Release>,
}

impl Schedule {
    /// The seven days, Monday first, as the week is counted where this
    /// client's audience counts it.
    #[must_use]
    pub fn days(&self) -> [&[Release]; 7] {
        [
            &self.monday,
            &self.tuesday,
            &self.wednesday,
            &self.thursday,
            &self.friday,
            &self.saturday,
            &self.sunday,
        ]
    }
}

/// A collection: a list of releases someone put together and named.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Collection {
    #[serde(deserialize_with = "nullable")]
    pub id: i64,
    #[serde(deserialize_with = "nullable")]
    pub title: String,
    #[serde(deserialize_with = "nullable")]
    pub description: String,
    #[serde(deserialize_with = "nullable")]
    pub image: String,
    #[serde(deserialize_with = "nullable")]
    pub creator: Option<Profile>,
    #[serde(deserialize_with = "nullable")]
    pub releases: Vec<Release>,
    #[serde(deserialize_with = "nullable")]
    pub favorites_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub comment_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub is_favorite: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_private: bool,
    #[serde(deserialize_with = "nullable")]
    pub creation_date: i64,
    #[serde(deserialize_with = "nullable")]
    pub last_update_date: i64,
}

/// What a search of the feed finds: posts, channels, blogs and tags, each
/// paged on its own.
#[derive(Debug, Clone, Default)]
pub struct FeedSearch {
    pub articles: Page<Article>,
    pub channels: Page<Channel>,
    pub blogs: Page<Channel>,
    pub tags: Page<String>,
}

/// A list of releases where some may be references to ones given earlier;
/// keeps those given in full. A `null` list is an empty one.
fn releases_in_full<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Release>, D::Error> {
    let items: Option<Vec<Embedded<Release>>> = Option::deserialize(deserializer)?;
    Ok(items
        .unwrap_or_default()
        .into_iter()
        .filter_map(|item| match item {
            Embedded::Full(release) => Some(*release),
            Embedded::Ref(_) => None,
        })
        .collect())
}
