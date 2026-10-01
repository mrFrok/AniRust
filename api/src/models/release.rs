// SPDX-License-Identifier: GPL-3.0-or-later
//
// Releases, and the filter and search they are found by.

use serde::{Deserialize, Serialize};

use crate::serde_ext::nullable;

use super::{Episode, ProfileList};

/// Where poster images are served from, for the responses that send only an
/// id. Observed, not documented.
const POSTER_BASE_URL: &str = "https://s.anixmirai.com/posters/";

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ReleaseStatus {
    #[serde(deserialize_with = "nullable")]
    pub id: i64,
    #[serde(deserialize_with = "nullable")]
    pub name: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ReleaseCategory {
    #[serde(deserialize_with = "nullable")]
    pub id: i64,
    #[serde(deserialize_with = "nullable")]
    pub name: String,
}

/// A title. This is the widest object in the API; only the fields the client
/// renders are modelled.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Release {
    #[serde(deserialize_with = "nullable")]
    pub id: i64,
    #[serde(deserialize_with = "nullable")]
    pub title_ru: String,
    #[serde(deserialize_with = "nullable")]
    pub title_original: String,
    #[serde(deserialize_with = "nullable")]
    pub title_alt: String,
    #[serde(deserialize_with = "nullable")]
    pub description: String,
    #[serde(deserialize_with = "nullable")]
    pub note: String,

    /// Storage id of the poster, not a URL. Use [`Self::poster_url`].
    #[serde(deserialize_with = "nullable")]
    pub poster: String,
    /// Full poster URL, sent by `release/{id}` but not by every listing.
    #[serde(deserialize_with = "nullable")]
    pub image: String,
    /// Both of these are sent, as separate fields rather than aliases of one
    /// another; `screenshot_images` is the one populated in practice. Use
    /// [`Self::screenshot_urls`] instead of reading either directly.
    #[serde(deserialize_with = "nullable")]
    pub screenshots: Vec<String>,
    #[serde(deserialize_with = "nullable")]
    pub screenshot_images: Vec<String>,

    #[serde(deserialize_with = "nullable")]
    pub year: String,
    #[serde(deserialize_with = "nullable")]
    pub genres: String,
    #[serde(deserialize_with = "nullable")]
    pub country: String,
    #[serde(deserialize_with = "nullable")]
    pub studio: String,
    #[serde(deserialize_with = "nullable")]
    pub director: String,
    #[serde(deserialize_with = "nullable")]
    pub author: String,
    #[serde(deserialize_with = "nullable")]
    pub translators: String,

    /// Populated by `release/{id}`. Search results carry only
    /// [`Self::status_id`], so prefer [`Self::status_name`].
    #[serde(deserialize_with = "nullable")]
    pub status: ReleaseStatus,
    #[serde(deserialize_with = "nullable")]
    pub status_id: i32,
    /// Populated by `release/{id}` only.
    #[serde(deserialize_with = "nullable")]
    pub category: ReleaseCategory,
    #[serde(deserialize_with = "nullable")]
    pub season: i32,
    #[serde(deserialize_with = "nullable")]
    pub broadcast: i32,
    #[serde(deserialize_with = "nullable")]
    pub creation_date: i64,
    /// Runtime of a single episode, in minutes.
    #[serde(deserialize_with = "nullable")]
    pub duration: i32,
    #[serde(deserialize_with = "nullable")]
    pub episodes_released: i32,
    #[serde(deserialize_with = "nullable")]
    pub episodes_total: i32,
    #[serde(deserialize_with = "nullable")]
    pub release_date: String,
    #[serde(deserialize_with = "nullable")]
    pub aired_on_date: i64,
    #[serde(deserialize_with = "nullable")]
    pub last_update_date: i64,

    /// Mean score on a 0..=5 scale, sent as a float (`4.835243196` observed).
    /// The five-star basis is visible in the `vote_N_count` tallies below.
    #[serde(deserialize_with = "nullable")]
    pub grade: f32,
    /// A second integer the API also labels "rating", distinct from
    /// [`Self::grade`]. Purpose unconfirmed, so it is carried but not
    /// rendered.
    #[serde(deserialize_with = "nullable")]
    pub rating: i32,
    #[serde(deserialize_with = "nullable")]
    pub vote_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub your_vote: i32,
    #[serde(deserialize_with = "nullable")]
    pub vote_1_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub vote_2_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub vote_3_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub vote_4_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub vote_5_count: i64,

    #[serde(deserialize_with = "nullable")]
    pub favorites_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub watching_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub plan_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub completed_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub hold_on_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub dropped_count: i64,
    /// The API sends both spellings; they agree in practice. Read
    /// [`Self::comments`] rather than picking one.
    #[serde(deserialize_with = "nullable")]
    pub comment_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub comments_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub collection_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub related_count: i64,

    #[serde(deserialize_with = "nullable")]
    pub age_rating: i32,
    #[serde(deserialize_with = "nullable")]
    pub is_adult: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_deleted: bool,
    /// Playback suppressed server-side; the client must not offer episodes.
    #[serde(deserialize_with = "nullable")]
    pub is_play_disabled: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_view_blocked: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_favorite: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_viewed: bool,

    /// Which of the user's lists this release sits in. See [`ProfileList`].
    #[serde(deserialize_with = "nullable")]
    pub profile_list_status: i32,
    #[serde(deserialize_with = "nullable")]
    pub last_view_timestamp: i64,
    #[serde(deserialize_with = "nullable")]
    pub last_view_episode: Option<Episode>,
}

impl Release {
    /// Display title, preferring Russian and falling back to the original.
    #[must_use]
    pub fn title(&self) -> &str {
        if self.title_ru.is_empty() {
            &self.title_original
        } else {
            &self.title_ru
        }
    }

    /// Mean score on a 0..=5 scale, as the API reports it.
    #[must_use]
    pub fn score(&self) -> f32 {
        self.grade
    }

    /// Poster URL.
    ///
    /// `image` carries it outright where the server sends it; elsewhere only
    /// the storage id in `poster` arrives, and the URL has to be built. The
    /// host is the one observed serving them, so a move on their side shows up
    /// as a missing poster rather than as a wrong release.
    #[must_use]
    pub fn poster_url(&self) -> String {
        if !self.image.is_empty() {
            return self.image.clone();
        }
        if self.poster.is_empty() {
            return String::new();
        }
        format!("{POSTER_BASE_URL}{}.jpg", self.poster)
    }

    /// Screenshot URLs, from whichever of the two fields the server filled.
    #[must_use]
    pub fn screenshot_urls(&self) -> &[String] {
        if self.screenshot_images.is_empty() {
            &self.screenshots
        } else {
            &self.screenshot_images
        }
    }

    /// Airing status name. Empty for search results, which send only
    /// [`Self::status_id`].
    #[must_use]
    pub fn status_name(&self) -> &str {
        &self.status.name
    }

    /// Comment count, tolerating the API's two spellings.
    #[must_use]
    pub fn comments(&self) -> i64 {
        self.comment_count.max(self.comments_count)
    }

    /// Whether the client should offer playback at all.
    #[must_use]
    pub fn is_playable(&self) -> bool {
        !self.is_play_disabled && !self.is_view_blocked && !self.is_deleted
    }

    /// Which of the user's lists this release is in, if any.
    #[must_use]
    pub fn list(&self) -> Option<ProfileList> {
        ProfileList::from_raw(self.profile_list_status).filter(|l| *l != ProfileList::NotInList)
    }
}

/// What a catalogue request asks for.
///
/// Every field the server accepts is optional but `sort`, and an omitted field
/// is left out of the body rather than sent as null — the endpoint treats a
/// present null as a value in some cases, so silence is the safer way to say
/// "no preference".
#[derive(Debug, Clone, Default, Serialize)]
pub struct Filter {
    /// 1 series, 2 film, 3 OVA. Observed, not documented.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_id: Option<i64>,
    /// 1 finished, 2 airing, 3 announced.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_id: Option<i64>,
    /// Genre names as the catalogue spells them, lowercase and in Russian.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_year: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_year: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub studio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    pub sort: i32,
}

impl Filter {
    #[must_use]
    pub fn sorted_by(sort: FilterSort) -> Self {
        Self {
            sort: sort.raw(),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn category(mut self, id: i64) -> Self {
        self.category_id = Some(id);
        self
    }

    #[must_use]
    pub fn status(mut self, id: i64) -> Self {
        self.status_id = Some(id);
        self
    }

    #[must_use]
    pub fn genre(mut self, name: impl Into<String>) -> Self {
        self.genres.push(name.into());
        self
    }
}

/// Orderings the catalogue accepts.
///
/// Read off the wire rather than from documentation: each was identified by
/// what the first page comes back sorted on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FilterSort {
    /// Most recently updated — new episodes first.
    #[default]
    LastUpdate,
    /// Highest rated.
    Rating,
    /// Newest by year, which puts announcements at the top.
    Year,
    /// Most watched.
    Popularity,
}

impl FilterSort {
    #[must_use]
    pub fn raw(self) -> i32 {
        match self {
            Self::LastUpdate => 0,
            Self::Rating => 1,
            Self::Year => 2,
            Self::Popularity => 3,
        }
    }
}

/// How `search/releases` interprets the query. Mirrors `SearchRequest.searchBy`.
///
/// Note the ordering: `Studio` precedes `Director`, which is not the order the
/// fields appear in [`Release`]. Endpoints taking the plain search request
/// shape (collections, profiles, history) accept only [`Self::Title`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i32)]
pub enum SearchBy {
    #[default]
    Title = 0,
    Studio = 1,
    Director = 2,
    Author = 3,
    Genre = 4,
}

impl SearchBy {
    #[must_use]
    pub fn raw(self) -> i32 {
        self as i32
    }
}
