// SPDX-License-Identifier: GPL-3.0-or-later
//
// Endpoint methods. Paths, verbs and parameters are transcribed from the
// shipped app's Retrofit interface declarations — which are interface facts,
// not implementation — rather than from the unofficial OpenAPI spec, which
// disagrees in places. Notably `episode/watch` and `episode/unwatch` are POST,
// and `discover/*` are POST despite reading like plain GETs.
//
// Response wrappers here carry only the payload: the `code` field every body
// includes is handled once, generically, in `client::Envelope`.
//
// One module per area of the API, each with its own `impl Client`, so the
// methods for one part of the service sit together and a file stays short
// enough to read. Paging is shared, and lives here.

use serde::Deserialize;

use crate::models::Page;

mod account;
mod article;
mod comment;
mod discover;
mod episode;
mod feed;
mod preference;
mod release;

pub use account::SignInError;
pub use article::{ArticleEntryPoint, ArticleEventKind};
pub use discover::ChannelSearch;
pub use episode::{DubberChannel, EpisodeSort};

/// Mirrors `PageableResponse<T>`. Pages are 0-based.
#[derive(Deserialize)]
pub(crate) struct PageablePayload<T> {
    #[serde(default = "Vec::new")]
    content: Vec<T>,
    #[serde(default)]
    current_page: i32,
    #[serde(default)]
    total_page_count: i32,
    #[serde(default)]
    total_count: i64,
}

impl<T> From<PageablePayload<T>> for Page<T> {
    fn from(p: PageablePayload<T>) -> Self {
        Self {
            content: p.content,
            current_page: p.current_page,
            total_page_count: p.total_page_count,
            total_count: p.total_count,
        }
    }
}
