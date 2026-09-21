// SPDX-License-Identifier: GPL-3.0-or-later
//
// Field names follow the wire format, which is Jackson with a snake_case
// naming strategy.
//
// Two defensive conventions apply to every model here, because the API is
// undocumented and changes without notice:
//
//   * `#[serde(default)]` at container level, so a field the server drops
//     degrades into a default instead of failing the response;
//   * `deserialize_with = "nullable"` on every field, because the server also
//     returns explicit `null` for fields its own types declare non-null —
//     including numeric ones. See `serde_ext` for the observed cases.
//
// Where the unofficial OpenAPI spec disagrees with the shipped app, the app
// wins and the spec's spelling is kept as a `serde(alias)`.

use serde::{Deserialize, Serialize};

use crate::serde_ext::nullable;

/// Where poster images are served from, for the responses that send only an
/// id. Observed, not documented.
const POSTER_BASE_URL: &str = "https://s.anixmirai.com/posters/";

/// A voice-over track for a release. The app calls this a `Type` and returns
/// it under a `types` key; "dubber" is used here because `Type` is unusable as
/// a Rust name.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Dubber {
    #[serde(deserialize_with = "nullable")]
    pub id: i64,
    #[serde(deserialize_with = "nullable")]
    pub name: String,
    #[serde(deserialize_with = "nullable")]
    pub icon: String,
    /// Free-text credit line for the people behind the dub.
    #[serde(deserialize_with = "nullable")]
    pub workers: String,
    #[serde(alias = "episode_count", deserialize_with = "nullable")]
    pub episodes_count: i64,
    /// Subtitles rather than a dub.
    #[serde(deserialize_with = "nullable")]
    pub is_sub: bool,
    #[serde(deserialize_with = "nullable")]
    pub pinned: bool,
    #[serde(deserialize_with = "nullable")]
    pub quality: i32,
    #[serde(deserialize_with = "nullable")]
    pub view_count: i64,
}

/// A player/host serving a given dubber's episodes — Kodik, Sibnet, and so on.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Source {
    #[serde(deserialize_with = "nullable")]
    pub id: i64,
    #[serde(deserialize_with = "nullable")]
    pub name: String,
    #[serde(alias = "episode_count", deserialize_with = "nullable")]
    pub episodes_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub quality: i32,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Episode {
    #[serde(deserialize_with = "nullable")]
    pub id: i64,
    /// 1-based index within the source, and the value the `watch` and
    /// `history` endpoints expect as `position`.
    #[serde(deserialize_with = "nullable")]
    pub position: i32,
    #[serde(deserialize_with = "nullable")]
    pub name: String,
    /// Either a direct media URL or an embed page, depending on [`Self::iframe`].
    #[serde(deserialize_with = "nullable")]
    pub url: String,
    /// `true` means [`Self::url`] is an embed page that needs an extractor
    /// before it can be handed to a player.
    #[serde(deserialize_with = "nullable")]
    pub iframe: bool,
    #[serde(deserialize_with = "nullable")]
    pub quality: i32,
    /// Server-side resume point, in milliseconds.
    #[serde(deserialize_with = "nullable")]
    pub playback_position: i64,
    #[serde(deserialize_with = "nullable")]
    pub added_date: i64,
    /// Filler episode. The spec calls this `is_filter`, which is a misreading.
    #[serde(alias = "is_filter", deserialize_with = "nullable")]
    pub is_filler: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_watched: bool,
    #[serde(deserialize_with = "nullable")]
    pub source_id: i64,
    #[serde(deserialize_with = "nullable")]
    pub release_id: i64,
}

impl Episode {
    /// Whether the URL can go straight to a player without an extractor.
    #[must_use]
    pub fn is_direct(&self) -> bool {
        !self.iframe && !self.url.is_empty()
    }

    /// Resume point, or `None` when the episode has not been started.
    #[must_use]
    pub fn resume_at(&self) -> Option<std::time::Duration> {
        u64::try_from(self.playback_position)
            .ok()
            .filter(|&ms| ms > 0)
            .map(std::time::Duration::from_millis)
    }
}

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

/// The user's per-release list. Values are the `status` path segment of
/// `profile/list/*` and the value of [`Release::profile_list_status`].
///
/// 1..=5 are corroborated twice: the unofficial spec names them in this order,
/// and [`Release`]'s own counter fields appear in exactly the same sequence
/// (`watching_count`, `plan_count`, `completed_count`, `hold_on_count`,
/// `dropped_count`). 0 is less certain — on a release it reads as "in no
/// list", but the filter API reuses 0 for "favorites", so the two are not
/// interchangeable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ProfileList {
    NotInList = 0,
    Watching = 1,
    Planned = 2,
    Watched = 3,
    HoldOn = 4,
    Dropped = 5,
}

impl ProfileList {
    /// Every list a release can actually be placed in, in the API's order.
    pub const ALL: [Self; 5] = [
        Self::Watching,
        Self::Planned,
        Self::Watched,
        Self::HoldOn,
        Self::Dropped,
    ];

    #[must_use]
    pub fn from_raw(value: i32) -> Option<Self> {
        Some(match value {
            0 => Self::NotInList,
            1 => Self::Watching,
            2 => Self::Planned,
            3 => Self::Watched,
            4 => Self::HoldOn,
            5 => Self::Dropped,
            _ => return None,
        })
    }

    #[must_use]
    pub fn raw(self) -> i32 {
        self as i32
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Profile {
    #[serde(deserialize_with = "nullable")]
    pub id: i64,
    #[serde(deserialize_with = "nullable")]
    pub login: String,
    /// Observed as a full URL, unlike [`Release::poster`], which is a storage
    /// id. Only ever seen populated for accounts that have set one, so a
    /// client that finds something else here should say so rather than
    /// quietly show nothing — see the GUI's own note where it is fetched.
    #[serde(deserialize_with = "nullable")]
    pub avatar: String,
    #[serde(deserialize_with = "nullable")]
    pub status: String,
    #[serde(deserialize_with = "nullable")]
    pub ban_expires: i64,
    #[serde(deserialize_with = "nullable")]
    pub is_banned: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_verified: bool,
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
    #[serde(deserialize_with = "nullable")]
    pub rate_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub comment_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub collection_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub friend_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub register_date: i64,
    #[serde(deserialize_with = "nullable")]
    pub last_activity_time: i64,
}

/// Credentials returned by `auth/signIn`. The `token` is what every
/// authenticated endpoint passes as its `token` query parameter.
///
/// `id` is numeric — the unofficial spec types it as a string, the app as a
/// `long`.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ProfileToken {
    #[serde(deserialize_with = "nullable")]
    pub id: i64,
    #[serde(deserialize_with = "nullable")]
    pub token: String,
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

/// One page of a paged collection. Pages are 0-based.
#[derive(Debug, Clone, Default)]
pub struct Page<T> {
    pub content: Vec<T>,
    pub current_page: i32,
    pub total_page_count: i32,
    pub total_count: i64,
}

impl<T> Page<T> {
    /// Whether another page exists after this one.
    #[must_use]
    pub fn has_next(&self) -> bool {
        self.current_page + 1 < self.total_page_count
    }

    /// Page index to request next, or `None` at the end of the collection.
    #[must_use]
    pub fn next_page(&self) -> Option<i32> {
        self.has_next().then(|| self.current_page + 1)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.content.len()
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
