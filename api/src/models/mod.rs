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

mod episode;
mod feed;
mod profile;
mod release;

pub use episode::{Dubber, Episode, Source};
pub use feed::{Article, ArticleBlock, ArticlePayload, Channel, ProfileSlim};
pub use profile::{Profile, ProfileList, ProfileToken};
pub use release::{Filter, FilterSort, Release, ReleaseCategory, ReleaseStatus, SearchBy};

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
