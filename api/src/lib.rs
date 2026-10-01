// SPDX-License-Identifier: GPL-3.0-or-later

//! Typed client for the Anixart mobile API.
//!
//! Anixart publishes no API and does not sanction third-party clients, so
//! everything here is transcribed from the shipped Android app's Retrofit
//! interfaces. Two consequences shape the design:
//!
//! * **Every response model is `#[serde(default)]`.** A field the server adds,
//!   renames or drops degrades into a default rather than failing the whole
//!   request.
//! * **The base URL is a list, not a constant.** The app discovers mirrors
//!   through a chain of fetchers because its domain gets blocked; see
//!   [`ClientBuilder::base_urls`].
//!
//! Authentication is a `token` **query parameter**, not a header.
//!
//! # Playback chain
//!
//! Getting from a release to something playable takes three calls, then
//! possibly an extractor:
//!
//! ```text
//! dubbers(release_id)                       -> Vec<Dubber>   // voice-over tracks
//! sources(release_id, dubber_id)            -> Vec<Source>   // Kodik, Sibnet, …
//! episodes(release_id, dubber_id, source_id) -> Vec<Episode>
//! ```
//!
//! Each [`Episode`] carries a `url` and an `iframe` flag. When `iframe` is
//! false the URL goes straight to a player; when it is true the URL is an
//! embed page that must be resolved first — that is what `anirust-extract` is
//! for.
//!
//! ```no_run
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! use anirust_api::{Client, EpisodeSort, SearchBy};
//!
//! let client = Client::new()?;
//! let hits = client.search_releases("Fullmetal Alchemist", SearchBy::Title, 0).await?;
//! let release = &hits[0];
//!
//! let dubbers = client.dubbers(release.id).await?;
//! let sources = client.sources(release.id, dubbers[0].id).await?;
//! let episodes = client
//!     .episodes(release.id, dubbers[0].id, sources[0].id, EpisodeSort::Ascending)
//!     .await?;
//!
//! for ep in &episodes {
//!     println!("{} — {} ({})", ep.position, ep.url, if ep.iframe { "embed" } else { "direct" });
//! }
//! # Ok(())
//! # }
//! ```

pub mod client;
pub mod endpoints;
pub mod error;
pub mod models;
pub(crate) mod serde_ext;

pub use client::{Ack, Client, ClientBuilder, DEFAULT_BASE_URL};
pub use endpoints::{DubberChannel, EpisodeSort, SignInError};
pub use error::{ApiCode, Error, Result};
pub use models::{
    Article, ArticleBlock, ArticlePayload, Channel, Comment, CommentModeration, CommentSort,
    CommentTarget, CommentVote, Dubber, Embedded, Episode, EpisodeUpdate, Filter, FilterSort, Page,
    Profile, ProfileCompact, ProfileList, ProfileSlim, ProfileToken, Related, Release,
    ReleaseCategory, ReleaseStatus, SearchBy, Source, StreamingPlatform,
};
