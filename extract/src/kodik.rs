// SPDX-License-Identifier: GPL-3.0-or-later

//! Kodik, and the domains it serves the same player from.
//!
//! By volume this is the extractor that matters: 96% of embedded episodes in a
//! sample of 832 went through `kodikplayer.com`.
//!
//! # Protocol
//!
//! Observed end to end against the live host in September 2026:
//!
//! 1. `GET` the embed page. Its URL carries the identifiers —
//!    `/{type}/{id}/{hash}/{height}p` — and the page body carries a set of
//!    signed tokens (`d_sign`, `pd_sign`, `ref_sign`), each a hash and an
//!    expiry joined by `:`.
//! 2. `POST /ftor` on the same host, form-encoded, echoing those tokens back
//!    together with `type`, `id` and `hash`.
//! 3. The reply is JSON: `links` maps a height to renditions, each with a
//!    `src` obfuscated by [`crate::rotate`] and a MIME type.
//! 4. Decoding `src` yields an HLS manifest URL, which plays once `Referer`
//!    and `User-Agent` are carried over.
//!
//! The page is parsed for exactly the fields step 2 needs; nothing else about
//! its markup is relied upon.

use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::Deserialize;

use crate::{
    DEFAULT_USER_AGENT, ExtractError, Extractor, ResolvedStream, Result, StreamKind, StreamVariant,
    rotate,
};

/// Domains serving the Kodik player. Taken from the official client's own host
/// list, which is an interface fact rather than an implementation.
const HOSTS: &[&str] = &["kodik.cc", "kodik.info", "aniqit.com", "kodikplayer.com"];

/// Endpoint the player posts to for stream links.
const FTOR_PATH: &str = "/ftor";

pub struct KodikExtractor {
    http: reqwest::Client,
}

impl KodikExtractor {
    #[must_use]
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }
}

/// The signed tokens and identifiers `/ftor` requires.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EmbedParams {
    domain: String,
    d_sign: String,
    pd: String,
    pd_sign: String,
    referer: String,
    ref_sign: String,
    media_type: String,
    id: String,
    hash: String,
}

impl EmbedParams {
    fn into_form(self) -> Vec<(&'static str, String)> {
        vec![
            ("d", self.domain),
            ("d_sign", self.d_sign),
            ("pd", self.pd),
            ("pd_sign", self.pd_sign),
            ("ref", self.referer),
            ("ref_sign", self.ref_sign),
            ("type", self.media_type),
            ("id", self.id),
            ("hash", self.hash),
            // The player reports these from its own settings; the host accepts
            // the optimistic values and they do not affect the links returned.
            ("bad_user", "true".to_owned()),
            ("cdn_is_working", "true".to_owned()),
        ]
    }
}

#[derive(Debug, Deserialize)]
struct FtorResponse {
    #[serde(default)]
    links: BTreeMap<String, Vec<FtorLink>>,
}

#[derive(Debug, Deserialize)]
struct FtorLink {
    #[serde(default)]
    src: String,
    #[serde(default, rename = "type")]
    mime: Option<String>,
}

#[async_trait]
impl Extractor for KodikExtractor {
    fn hosts(&self) -> &'static [&'static str] {
        HOSTS
    }

    fn name(&self) -> &'static str {
        "Kodik"
    }

    async fn resolve(&self, embed_url: &str) -> Result<ResolvedStream> {
        let embed_url = with_scheme(embed_url);
        let parsed = url::Url::parse(&embed_url)?;
        let origin = format!(
            "{}://{}/",
            parsed.scheme(),
            parsed.host_str().ok_or_else(|| ExtractError::NoHost {
                url: embed_url.clone()
            })?
        );

        let page = self
            .http
            .get(&embed_url)
            .header(reqwest::header::USER_AGENT, DEFAULT_USER_AGENT)
            .header(reqwest::header::REFERER, &origin)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;

        let params = parse_embed_params(&page, &parsed)?;
        tracing::debug!(id = %params.id, media_type = %params.media_type, "kodik params parsed");

        let ftor_url = parsed.join(FTOR_PATH)?;
        let response: FtorResponse = self
            .http
            .post(ftor_url)
            .header(reqwest::header::USER_AGENT, DEFAULT_USER_AGENT)
            .header(reqwest::header::REFERER, &embed_url)
            .header("X-Requested-With", "XMLHttpRequest")
            .form(&params.into_form())
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let variants = decode_links(&response);
        if variants.is_empty() {
            return Err(ExtractError::NoStreams { host: "Kodik" });
        }

        Ok(ResolvedStream {
            variants,
            // The CDN checks both; without them the manifest resolves but its
            // segments come back 403.
            headers: BTreeMap::from([
                ("Referer".to_owned(), origin),
                ("User-Agent".to_owned(), DEFAULT_USER_AGENT.to_owned()),
            ]),
            subtitles: Vec::new(),
        })
    }
}

/// Reads a `var <name> = "...";` assignment out of the page.
fn read_var(page: &str, name: &str) -> Option<String> {
    // Built per call rather than kept in a lazy static: this runs a handful of
    // times per episode, far from any hot path, and a local regex keeps the
    // parsing readable.
    let pattern = format!(r#"var\s+{name}\s*=\s*"([^"]*)""#);
    regex::Regex::new(&pattern)
        .ok()?
        .captures(page)?
        .get(1)
        .map(|m| m.as_str().to_owned())
}

fn missing(what: impl Into<String>) -> ExtractError {
    ExtractError::UnexpectedFormat {
        host: "Kodik",
        what: what.into(),
    }
}

/// Pulls the `/ftor` parameters from the embed page and its URL.
///
/// `id`, `hash` and the media type come from the path (`/{type}/{id}/{hash}/
/// {height}p`); the signed tokens only exist in the page body.
fn parse_embed_params(page: &str, embed_url: &url::Url) -> Result<EmbedParams> {
    let segments: Vec<&str> = embed_url
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).collect())
        .unwrap_or_default();

    let [path_type, id, hash, ..] = segments.as_slice() else {
        return Err(missing(format!(
            "path shaped /<type>/<id>/<hash>/… (got `{}`)",
            embed_url.path()
        )));
    };

    // The page repeats the type; prefer it, since the path is only a mirror.
    let media_type = read_var(page, "type").unwrap_or_else(|| (*path_type).to_owned());
    let domain = read_var(page, "domain").ok_or_else(|| missing("var domain"))?;
    let d_sign = read_var(page, "d_sign").ok_or_else(|| missing("var d_sign"))?;
    let pd = read_var(page, "pd").unwrap_or_else(|| domain.clone());
    let pd_sign = read_var(page, "pd_sign").ok_or_else(|| missing("var pd_sign"))?;
    let referer = read_var(page, "ref").unwrap_or_else(|| format!("https://{domain}/"));
    let ref_sign = read_var(page, "ref_sign").ok_or_else(|| missing("var ref_sign"))?;

    Ok(EmbedParams {
        domain,
        d_sign,
        pd,
        pd_sign,
        referer,
        ref_sign,
        media_type,
        id: (*id).to_owned(),
        hash: (*hash).to_owned(),
    })
}

/// Turns the `links` map into renditions, skipping any entry that will not
/// decode rather than failing the whole episode over one bad quality.
fn decode_links(response: &FtorResponse) -> Vec<StreamVariant> {
    let mut variants = Vec::new();

    for (quality, links) in &response.links {
        let Ok(height) = quality.trim_end_matches('p').parse::<u32>() else {
            tracing::debug!(%quality, "skipping unparsable quality key");
            continue;
        };

        for link in links {
            match rotate::decode_url(&link.src) {
                Some(url) => {
                    let kind = StreamKind::classify(link.mime.as_deref(), &url);
                    variants.push(StreamVariant { height, url, kind });
                }
                None => tracing::warn!(
                    height,
                    "could not decode a Kodik stream url — the host's obfuscation changed"
                ),
            }
        }
    }

    variants
}

/// Episode URLs are sometimes protocol-relative.
fn with_scheme(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("//") {
        format!("https://{rest}")
    } else {
        url.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMBED_PAGE: &str = include_str!("../tests/fixtures/kodik_embed.html");
    const FTOR_BODY: &str = include_str!("../tests/fixtures/kodik_ftor.json");
    const EMBED_URL: &str =
        "https://kodikplayer.com/seria/405469/c67b3b953348bfea339212aea33816bd/720p";

    fn params() -> EmbedParams {
        parse_embed_params(EMBED_PAGE, &url::Url::parse(EMBED_URL).unwrap()).unwrap()
    }

    #[test]
    fn parses_identifiers_from_the_path() {
        let p = params();
        assert_eq!(p.id, "405469");
        assert_eq!(p.hash, "c67b3b953348bfea339212aea33816bd");
        assert_eq!(p.media_type, "seria");
    }

    #[test]
    fn parses_signed_tokens_from_the_page() {
        let p = params();
        assert_eq!(p.domain, "kodikplayer.com");
        assert_eq!(p.pd, "kodikplayer.com");
        assert_eq!(p.referer, "https://kodikplayer.com/");
        // Each token is a hash and an expiry joined by ':'.
        for token in [&p.d_sign, &p.pd_sign, &p.ref_sign] {
            let (hash, expiry) = token.split_once(':').expect("token should carry an expiry");
            assert_eq!(hash.len(), 64, "expected a sha256 hex digest");
            assert!(expiry.chars().all(|c| c.is_ascii_digit()));
        }
    }

    #[test]
    fn form_carries_every_field_ftor_requires() {
        let form = params().into_form();
        let names: Vec<_> = form.iter().map(|(k, _)| *k).collect();
        for required in [
            "d", "d_sign", "pd", "pd_sign", "ref", "ref_sign", "type", "id", "hash",
        ] {
            assert!(names.contains(&required), "form is missing `{required}`");
        }
        assert!(form.iter().all(|(_, v)| !v.is_empty()));
    }

    #[test]
    fn a_page_without_the_tokens_names_what_is_missing() {
        let err = parse_embed_params(
            "<html>nothing here</html>",
            &url::Url::parse(EMBED_URL).unwrap(),
        )
        .unwrap_err();
        let message = err.to_string();
        // The error must name the field, so a host-side rename is diagnosable
        // from a log line alone.
        assert!(message.contains("var domain"), "unhelpful error: {message}");
        assert!(
            message.contains("Kodik"),
            "error should name the host: {message}"
        );
    }

    #[test]
    fn a_path_without_identifiers_is_rejected() {
        let err = parse_embed_params(
            EMBED_PAGE,
            &url::Url::parse("https://kodikplayer.com/x").unwrap(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("path shaped"));
    }

    #[test]
    fn decodes_every_rendition_from_a_captured_response() {
        let response: FtorResponse = serde_json::from_str(FTOR_BODY).unwrap();
        let variants = decode_links(&response);

        let mut heights: Vec<_> = variants.iter().map(|v| v.height).collect();
        heights.sort_unstable();
        assert_eq!(heights, vec![240, 360, 480, 720]);

        for v in &variants {
            assert!(v.url.starts_with("https://"), "not a url: {}", v.url);
            assert!(v.url.contains(".m3u8"), "expected HLS: {}", v.url);
            assert_eq!(v.kind, StreamKind::Hls);
        }
    }

    #[test]
    fn one_undecodable_rendition_does_not_lose_the_others() {
        let body = r#"{"links":{"480":[{"src":"!!!","type":"application/x-mpegURL"}],
                                 "720":[{"src":"aHR0cHM6Ly9jZG4uZXhhbXBsZS5jb20vYS5tM3U4","type":"application/x-mpegURL"}]}}"#;
        let response: FtorResponse = serde_json::from_str(body).unwrap();
        let variants = decode_links(&response);
        assert_eq!(variants.len(), 1);
        assert_eq!(variants[0].height, 720);
    }

    #[test]
    fn claims_the_hosts_the_official_client_uses() {
        let extractor = KodikExtractor::new(reqwest::Client::new());
        for host in ["kodik.cc", "kodik.info", "aniqit.com", "kodikplayer.com"] {
            assert!(extractor.hosts().contains(&host));
        }
    }
}
