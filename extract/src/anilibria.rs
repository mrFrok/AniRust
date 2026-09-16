// SPDX-License-Identifier: GPL-3.0-or-later

//! AniLibria, across the domains it is mirrored on.
//!
//! Like Sibnet, Anixart reports these with `iframe = false` even though the
//! URL (`iframe.php?id=…&ep=…`) is a player page answering `text/html`.
//!
//! # Protocol
//!
//! Observed against the live host in September 2026. The page carries a single
//! inline player construction whose `file:` key is a JSON array of *every*
//! episode of the release, not just the requested one:
//!
//! ```text
//! file:[{"id":"s1","skip":"100-188","file":"[480p]https://…m3u8,[720p]https://…m3u8", …}, …]
//! ```
//!
//! So the `ep` query parameter of the embed URL selects which entry applies,
//! and each entry's `file` string packs the renditions as `[<height>p]<url>`
//! pairs separated by commas.
//!
//! The manifest URLs are already signed for the caller's address, so no second
//! request is needed.
//!
//! `skip` carries the opening's start and end in seconds. It is parsed and
//! kept because a "skip opening" control is exactly the sort of thing the
//! official client charges attention for, and the data is free here.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use async_trait::async_trait;
use regex::Regex;
use serde::Deserialize;

use crate::{
    DEFAULT_USER_AGENT, ExtractError, Extractor, ResolvedStream, Result, SkipRange, StreamKind,
    StreamVariant,
};

const HOSTS: &[&str] = &[
    "anilibria.tv",
    "anilibria.top",
    "new.anilib.one",
    "anixart.libria.fun",
];

/// The `file:[…]` array of the inline player construction.
static FILE_ARRAY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"file:\s*(\[.*\])\s*\}\s*\)").expect("static pattern"));

/// One `[<height>p]<url>` pair inside an entry's `file` string.
static RENDITION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[(\d+)p?\]([^,\[]+)").expect("static pattern"));

/// One episode as the player lists it.
#[derive(Debug, Deserialize)]
struct PlaylistEntry {
    #[serde(default)]
    id: String,
    /// `[480p]url,[720p]url`
    #[serde(default)]
    file: String,
    #[serde(default)]
    skip: Option<String>,
}

pub struct AniLibriaExtractor {
    http: reqwest::Client,
}

impl AniLibriaExtractor {
    #[must_use]
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }
}

#[async_trait]
impl Extractor for AniLibriaExtractor {
    fn hosts(&self) -> &'static [&'static str] {
        HOSTS
    }

    fn name(&self) -> &'static str {
        "AniLibria"
    }

    async fn resolve(&self, embed_url: &str) -> Result<ResolvedStream> {
        let embed_url = crate::with_scheme(embed_url);
        let parsed = url::Url::parse(&embed_url)?;
        let origin = crate::origin_of(&parsed).ok_or_else(|| ExtractError::NoHost {
            url: embed_url.clone(),
        })?;

        // Through the retrying helper: embed hosts answer 500 under load and
        // recover a moment later. See `crate::http`.
        let page = crate::http::send_with_retry(
            self.http
                .get(&embed_url)
                .header(reqwest::header::USER_AGENT, DEFAULT_USER_AGENT)
                .header(reqwest::header::REFERER, &origin),
        )
        .await?
        .text()
        .await?;

        let entries = parse_playlist(&page)?;
        let episode = episode_of(&parsed);
        let entry =
            select_entry(&entries, episode).ok_or_else(|| ExtractError::UnexpectedFormat {
                host: "AniLibria",
                what: format!("episode {episode} in a playlist of {}", entries.len()),
            })?;

        let variants = parse_renditions(&entry.file);
        if variants.is_empty() {
            return Err(ExtractError::NoStreams { host: "AniLibria" });
        }

        Ok(ResolvedStream {
            variants,
            headers: BTreeMap::from([
                ("Referer".to_owned(), origin),
                ("User-Agent".to_owned(), DEFAULT_USER_AGENT.to_owned()),
            ]),
            subtitles: Vec::new(),
            opening: parse_skip(entry.skip.as_deref()),
        })
    }
}

/// Episode number from the embed URL's `ep`, defaulting to the first.
fn episode_of(embed_url: &url::Url) -> u32 {
    embed_url
        .query_pairs()
        .find(|(k, _)| k == "ep")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(1)
}

fn parse_playlist(page: &str) -> Result<Vec<PlaylistEntry>> {
    let array = FILE_ARRAY
        .captures(page)
        .map(|c| c[1].to_owned())
        .ok_or_else(|| ExtractError::UnexpectedFormat {
            host: "AniLibria",
            what: "the player's file:[…] playlist".to_owned(),
        })?;

    serde_json::from_str(&array).map_err(|e| ExtractError::UnexpectedFormat {
        host: "AniLibria",
        what: format!("a JSON playlist ({e})"),
    })
}

/// Finds the entry for `episode`, matching the player's `sN` ids and falling
/// back to position so a renamed id does not break playback.
fn select_entry(entries: &[PlaylistEntry], episode: u32) -> Option<&PlaylistEntry> {
    let by_id = format!("s{episode}");
    entries
        .iter()
        .find(|e| e.id == by_id)
        .or_else(|| entries.get(episode.checked_sub(1)? as usize))
}

/// Splits a `[480p]url,[720p]url` string into renditions.
fn parse_renditions(file: &str) -> Vec<StreamVariant> {
    RENDITION
        .captures_iter(file)
        .filter_map(|c| {
            let height = c[1].parse().ok()?;
            let url = c[2].trim().to_owned();
            (!url.is_empty()).then_some(StreamVariant {
                height,
                kind: StreamKind::classify(None, &url),
                url,
            })
        })
        .collect()
}

/// Parses a `"<start>-<end>"` opening range.
#[must_use]
pub fn parse_skip(skip: Option<&str>) -> Option<SkipRange> {
    let (start, end) = skip?.split_once('-')?;
    Some(SkipRange {
        start: start.trim().parse().ok()?,
        end: end.trim().parse().ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = include_str!("../tests/fixtures/anilibria_embed.html");

    #[test]
    fn parses_every_episode_from_a_captured_page() {
        let entries = parse_playlist(PAGE).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].id, "s1");
        assert_eq!(entries[2].id, "s3");
    }

    #[test]
    fn selects_the_episode_named_by_the_url() {
        let entries = parse_playlist(PAGE).unwrap();
        let url =
            url::Url::parse("https://anixart.libria.fun/public/iframe.php?id=570&ep=2").unwrap();
        assert_eq!(episode_of(&url), 2);
        assert_eq!(select_entry(&entries, 2).unwrap().id, "s2");
    }

    #[test]
    fn defaults_to_the_first_episode() {
        let url = url::Url::parse("https://anixart.libria.fun/public/iframe.php?id=570").unwrap();
        assert_eq!(episode_of(&url), 1);
    }

    #[test]
    fn an_episode_beyond_the_playlist_is_not_selected() {
        let entries = parse_playlist(PAGE).unwrap();
        assert!(select_entry(&entries, 99).is_none());
    }

    #[test]
    fn splits_renditions_and_keeps_query_strings_intact() {
        let entries = parse_playlist(PAGE).unwrap();
        let variants = parse_renditions(&entries[0].file);

        let mut heights: Vec<_> = variants.iter().map(|v| v.height).collect();
        heights.sort_unstable();
        assert_eq!(heights, vec![480, 720]);

        for v in &variants {
            assert!(v.url.starts_with("https://"), "not a url: {}", v.url);
            assert!(v.url.contains(".m3u8"));
            assert_eq!(v.kind, StreamKind::Hls);
            // The signature lives in the query; truncating it breaks playback.
            assert!(v.url.contains("clientIp="), "query was lost: {}", v.url);
        }
    }

    #[test]
    fn reads_the_opening_range() {
        let entries = parse_playlist(PAGE).unwrap();
        assert_eq!(
            parse_skip(entries[0].skip.as_deref()),
            Some(SkipRange {
                start: 100,
                end: 188
            })
        );
        assert_eq!(parse_skip(None), None);
        assert_eq!(parse_skip(Some("nonsense")), None);
    }

    #[test]
    fn a_page_without_a_playlist_names_what_is_missing() {
        let err = parse_playlist("<html>nothing</html>").unwrap_err();
        assert!(err.to_string().contains("playlist"), "{err}");
        assert!(err.to_string().contains("AniLibria"), "{err}");
    }
}
