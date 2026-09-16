// SPDX-License-Identifier: GPL-3.0-or-later

//! Sibnet.
//!
//! Anixart reports these URLs with `iframe = false`, which is wrong: the URL
//! is `shell.php?videoid=…`, a player page that answers `text/html`. Handing
//! it straight to a player fails. See [`crate::Registry::resolve`], which is
//! why extraction is driven by the host rather than by that flag.
//!
//! # Protocol
//!
//! Observed against the live host in September 2026, and refreshingly plain:
//! the embed page embeds the media path directly, as a site-relative
//! `/v/{hash}/{video_id}.mp4`. A `GET` with a matching `Referer` is enough;
//! the path then redirects to whichever delivery node serves it.
//!
//! The page is served as windows-1251, which the HTTP client decodes from the
//! `Content-Type` charset.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use async_trait::async_trait;
use regex::Regex;

use crate::{
    DEFAULT_USER_AGENT, ExtractError, Extractor, ResolvedStream, Result, StreamKind, StreamVariant,
};

const HOSTS: &[&str] = &["video.sibnet.ru"];

/// Site-relative media path embedded in the player page.
static MEDIA_PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(/v/[0-9a-f]{8,}/\d+\.mp4)"#).expect("static pattern"));

pub struct SibnetExtractor {
    http: reqwest::Client,
}

impl SibnetExtractor {
    #[must_use]
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }
}

#[async_trait]
impl Extractor for SibnetExtractor {
    fn hosts(&self) -> &'static [&'static str] {
        HOSTS
    }

    fn name(&self) -> &'static str {
        "Sibnet"
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

        let path = find_media_path(&page).ok_or_else(|| ExtractError::UnexpectedFormat {
            host: "Sibnet",
            what: "a /v/<hash>/<id>.mp4 media path".to_owned(),
        })?;

        Ok(ResolvedStream {
            variants: vec![StreamVariant {
                // The page advertises no resolution, and the API's `quality`
                // is not visible here. See `StreamVariant::height`.
                height: crate::UNKNOWN_HEIGHT,
                url: parsed.join(&path)?.to_string(),
                kind: StreamKind::Progressive,
            }],
            headers: BTreeMap::from([
                ("Referer".to_owned(), origin),
                ("User-Agent".to_owned(), DEFAULT_USER_AGENT.to_owned()),
            ]),
            subtitles: Vec::new(),
            opening: None,
        })
    }
}

fn find_media_path(page: &str) -> Option<String> {
    MEDIA_PATH.captures(page).map(|c| c[1].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = include_str!("../tests/fixtures/sibnet_embed.html");

    #[test]
    fn finds_the_media_path_in_a_captured_page() {
        assert_eq!(
            find_media_path(PAGE).as_deref(),
            Some("/v/cc8c87c174b502a6072db48be69721e6/3527756.mp4")
        );
    }

    #[test]
    fn resolves_against_the_embed_url() {
        let base = url::Url::parse("https://video.sibnet.ru/shell.php?videoid=3527756").unwrap();
        let joined = base.join(&find_media_path(PAGE).unwrap()).unwrap();
        assert_eq!(
            joined.as_str(),
            "https://video.sibnet.ru/v/cc8c87c174b502a6072db48be69721e6/3527756.mp4"
        );
    }

    #[test]
    fn a_page_without_a_media_path_yields_nothing() {
        assert_eq!(find_media_path("<html>no player here</html>"), None);
        // Asset paths must not be mistaken for media.
        assert_eq!(
            find_media_path(r#"<script src="/legacy-assets/x.js">"#),
            None
        );
    }
}
