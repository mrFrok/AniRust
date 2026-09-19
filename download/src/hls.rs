// SPDX-License-Identifier: GPL-3.0-or-later

//! Fetching an HLS stream into one file.
//!
//! The manifest is a plain text list of segment URIs. A master manifest lists
//! other manifests instead, so the first job is to follow it down to a media
//! playlist — the extractors usually hand over one already, but not always.
//!
//! Segments are fetched several at a time and written in the order the manifest
//! lists them. Joining them end to end is not a trick: both MPEG-TS and
//! fragmented MP4 are designed to be concatenated, which is what lets a player
//! start mid-stream in the first place.

use std::collections::BTreeMap;
use std::path::Path;

use futures::StreamExt;
use tokio::io::AsyncWriteExt;

use crate::{Error, Progress, Result};

/// How deep to follow master manifests before deciding something is wrong.
const MAX_REDIRECTS: usize = 4;

pub(crate) async fn fetch(
    http: &reqwest::Client,
    url: &str,
    headers: &BTreeMap<String, String>,
    destination: &Path,
    parallel: usize,
    report: &mut impl FnMut(Progress),
) -> Result<()> {
    let (manifest_url, segments) = resolve_segments(http, url, headers).await?;
    if segments.is_empty() {
        return Err(Error::Manifest(format!("no segments in {manifest_url}")));
    }

    let total = segments.len() as u64;
    let mut file = tokio::fs::File::create(destination)
        .await
        .map_err(|source| Error::Write {
            path: destination.to_owned(),
            source,
        })?;

    // Fetched concurrently, written in order: `buffered` keeps the stream in
    // the order the futures were created, which is the order the manifest
    // lists them.
    let mut fetches = futures::stream::iter(segments.into_iter().map(|segment| {
        let http = http.clone();
        let headers = headers.clone();
        async move { fetch_segment(&http, &segment, &headers).await }
    }))
    .buffered(parallel.max(1));

    let mut done = 0;
    while let Some(bytes) = fetches.next().await {
        file.write_all(&bytes?)
            .await
            .map_err(|source| Error::Write {
                path: destination.to_owned(),
                source,
            })?;
        done += 1;
        report(Progress::Fetching { done, total });
    }

    file.flush().await.map_err(|source| Error::Write {
        path: destination.to_owned(),
        source,
    })?;
    Ok(())
}

async fn fetch_segment(
    http: &reqwest::Client,
    url: &str,
    headers: &BTreeMap<String, String>,
) -> Result<Vec<u8>> {
    let mut request = http.get(url);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    Ok(request
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?
        .to_vec())
}

/// Follows master manifests down to a media playlist and returns its segments,
/// already resolved to absolute URLs.
async fn resolve_segments(
    http: &reqwest::Client,
    url: &str,
    headers: &BTreeMap<String, String>,
) -> Result<(String, Vec<String>)> {
    let mut current = url.to_owned();

    for _ in 0..MAX_REDIRECTS {
        let body = fetch_text(http, &current, headers).await?;
        let base = url::Url::parse(&current)
            .map_err(|error| Error::Manifest(format!("{current}: {error}")))?;

        let entries = uris(&body, &base);
        if entries.is_empty() {
            return Err(Error::Manifest(format!("nothing listed in {current}")));
        }

        // A master manifest lists renditions rather than segments. The highest
        // one listed is the one to take: the extractor already picked a
        // quality, and this is that quality's own manifest.
        if body.contains("#EXT-X-STREAM-INF") {
            current = entries
                .into_iter()
                .next_back()
                .ok_or_else(|| Error::Manifest(format!("no rendition in {current}")))?;
            continue;
        }

        // A media playlist may open with an initialisation segment, which has
        // to come first or the rest is undecodable.
        let mut segments = init_segment(&body, &base).into_iter().collect::<Vec<_>>();
        segments.extend(entries);
        return Ok((current, segments));
    }

    Err(Error::Manifest(format!(
        "{url} kept pointing at another manifest"
    )))
}

async fn fetch_text(
    http: &reqwest::Client,
    url: &str,
    headers: &BTreeMap<String, String>,
) -> Result<String> {
    let mut request = http.get(url);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    Ok(request.send().await?.error_for_status()?.text().await?)
}

/// Every URI line in a manifest, resolved against it.
///
/// Lines beginning with `#` are tags; everything else is a URI, which may be
/// relative.
fn uris(body: &str, base: &url::Url) -> Vec<String> {
    body.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| base.join(line).ok())
        .map(String::from)
        .collect()
}

/// The `#EXT-X-MAP` initialisation segment, for fragmented MP4 playlists.
fn init_segment(body: &str, base: &url::Url) -> Option<String> {
    let line = body
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("#EXT-X-MAP:"))?;
    let uri = attribute(line, "URI")?;
    base.join(&uri).ok().map(String::from)
}

/// The value of a quoted attribute in a manifest tag.
fn attribute(line: &str, name: &str) -> Option<String> {
    let at = line.find(&format!("{name}=\""))? + name.len() + 2;
    let rest = &line[at..];
    let end = rest.find('"')?;
    Some(rest[..end].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> url::Url {
        url::Url::parse("https://cdn.example/a/b/index.m3u8").unwrap()
    }

    #[test]
    fn relative_segments_resolve_against_the_manifest() {
        let body = "#EXTM3U\n#EXTINF:6,\nseg1.ts\n#EXTINF:6,\nseg2.ts\n#EXT-X-ENDLIST\n";
        assert_eq!(
            uris(body, &base()),
            vec![
                "https://cdn.example/a/b/seg1.ts",
                "https://cdn.example/a/b/seg2.ts"
            ]
        );
    }

    #[test]
    fn absolute_segments_are_left_alone() {
        let body = "#EXTM3U\nhttps://other.example/x.ts\n";
        assert_eq!(uris(body, &base()), vec!["https://other.example/x.ts"]);
    }

    #[test]
    fn tags_are_not_segments() {
        let body = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:6\n";
        assert!(uris(body, &base()).is_empty());
    }

    #[test]
    fn an_initialisation_segment_is_found_and_resolved() {
        let body = "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:6,\n0.m4s\n";
        assert_eq!(
            init_segment(body, &base()).as_deref(),
            Some("https://cdn.example/a/b/init.mp4")
        );
    }

    #[test]
    fn a_playlist_without_one_reports_none() {
        assert_eq!(init_segment("#EXTM3U\nseg.ts\n", &base()), None);
    }

    #[test]
    fn an_attribute_is_read_out_of_a_tag() {
        let line = "#EXT-X-MAP:URI=\"init.mp4\",BYTERANGE=\"718@0\"";
        assert_eq!(attribute(line, "URI").as_deref(), Some("init.mp4"));
        assert_eq!(attribute(line, "BYTERANGE").as_deref(), Some("718@0"));
        assert_eq!(attribute(line, "MISSING"), None);
    }
}
