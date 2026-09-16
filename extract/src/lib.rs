// SPDX-License-Identifier: GPL-3.0-or-later

//! Resolves an embedded player page to a directly playable media stream.
//!
//! Anixart hands out a URL per episode plus an `iframe` flag. When that flag is
//! set the URL is a player page rather than media, and something has to turn it
//! into a stream a player can open. That is this crate.
//!
//! A measurement over 832 episodes across 8 releases: 97% are embeds, and of
//! those 96% are Kodik. Extractor coverage is therefore not a nice-to-have —
//! without Kodik the client plays essentially nothing.
//!
//! # Clean room
//!
//! Every extractor here is written against **observed protocol behaviour** —
//! the request the host expects and the response it returns — never by
//! translating the official app's code. Host names and endpoint paths are
//! interface facts; the implementation is ours. See `README.md`.
//!
//! # Headers matter
//!
//! Hosts reject requests lacking a plausible `Referer` or `User-Agent`, and the
//! same applies to the resolved stream: the headers in
//! [`ResolvedStream::headers`] must be handed to the player, or segment fetches
//! will fail even though the manifest URL is correct.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;

pub mod anilibria;
pub mod http;
pub mod kodik;
pub mod rotate;
pub mod sibnet;

pub use anilibria::AniLibriaExtractor;
pub use kodik::KodikExtractor;
pub use sibnet::SibnetExtractor;

/// Height for a rendition whose resolution the host does not advertise.
///
/// Sorting puts these last, so a labelled rendition always wins when both are
/// on offer, and a lone unlabelled one is still returned by
/// [`ResolvedStream::best`].
pub const UNKNOWN_HEIGHT: u32 = 0;

/// A browser-like identity. Several hosts serve an error page to anything that
/// looks automated, so this is the default for every extractor and is carried
/// through to the resolved stream.
pub const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";

/// How long a player should wait for one of these CDN nodes.
///
/// Deliberately generous. A manifest URL redirects to a randomly chosen edge
/// node, and those nodes vary wildly: the same host answered a request in 14ms
/// and then took 19.3s to accept the next connection. ffmpeg's default timeout
/// is shorter than that, so an episode that is merely slow fails outright with
/// `avformat_open_input() failed`, which reads like a broken extractor.
pub const NETWORK_TIMEOUT_SECS: u32 = 60;

/// One playable rendition of an episode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamVariant {
    /// Vertical resolution in pixels, as the host labels it (`720`, `1080`).
    pub height: u32,
    pub url: String,
    pub kind: StreamKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    /// HLS manifest. The common case.
    Hls,
    /// A progressive file, playable as-is.
    Progressive,
}

impl StreamKind {
    /// Classifies a stream from the MIME type the host reports, falling back
    /// to the URL when it reports nothing useful.
    #[must_use]
    pub fn classify(mime: Option<&str>, url: &str) -> Self {
        let looks_hls = mime.is_some_and(|m| {
            let m = m.to_ascii_lowercase();
            m.contains("mpegurl") || m.contains("m3u")
        }) || url.contains(".m3u8");

        if looks_hls {
            Self::Hls
        } else {
            Self::Progressive
        }
    }
}

/// Where the opening runs, in seconds from the start of the episode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkipRange {
    pub start: u32,
    pub end: u32,
}

impl SkipRange {
    #[must_use]
    pub fn duration_secs(self) -> u32 {
        self.end.saturating_sub(self.start)
    }
}

/// Everything a player needs to open an episode.
#[derive(Debug, Clone, Default)]
pub struct ResolvedStream {
    /// Renditions, highest quality first.
    pub variants: Vec<StreamVariant>,
    /// Headers the player must send. Typically `Referer` and `User-Agent`;
    /// omitting them usually yields a 403 on the segments.
    pub headers: BTreeMap<String, String>,
    pub subtitles: Vec<SubtitleTrack>,
    /// Opening boundaries, when the host publishes them. Only some do, so a
    /// "skip opening" control has to degrade gracefully rather than assume it.
    pub opening: Option<SkipRange>,
}

impl ResolvedStream {
    /// Highest-quality rendition, or `None` when nothing was resolved.
    #[must_use]
    pub fn best(&self) -> Option<&StreamVariant> {
        self.variants.first()
    }

    /// Rendition closest to `height` without exceeding it, falling back to the
    /// lowest available when everything is larger.
    #[must_use]
    pub fn at_most(&self, height: u32) -> Option<&StreamVariant> {
        self.variants
            .iter()
            .find(|v| v.height <= height)
            .or_else(|| self.variants.last())
    }

    /// Arguments that make mpv able to play this stream.
    ///
    /// Three details matter, each of which silently breaks playback:
    ///
    /// * `--http-header-fields` is a *list* option, so repeating it replaces
    ///   the previous value instead of adding to it. Each header therefore
    ///   uses `--http-header-fields-append`.
    /// * mpv sends its own `User-Agent`, which a header field does not
    ///   override, so that one becomes `--user-agent`.
    /// * the edge nodes these manifests redirect to can take a long time to
    ///   accept a connection — 19s was measured against a node that had
    ///   answered in 14ms moments earlier. ffmpeg's default is shorter, so it
    ///   gives up and the episode looks broken when it is merely slow. See
    ///   [`NETWORK_TIMEOUT_SECS`].
    #[must_use]
    pub fn mpv_args(&self) -> Vec<String> {
        let mut args = vec![format!("--network-timeout={NETWORK_TIMEOUT_SECS}")];
        args.extend(self.headers.iter().map(|(name, value)| {
            if name.eq_ignore_ascii_case("user-agent") {
                format!("--user-agent={value}")
            } else {
                format!("--http-header-fields-append={name}: {value}")
            }
        }));
        args
    }

    /// Sorts renditions highest-first and drops duplicates, so callers can
    /// rely on [`Self::best`] regardless of the order a host returned.
    ///
    /// Two kinds of duplicate go. Repeated heights are the obvious one. The
    /// subtler one is several heights sharing a single URL: hosts pad their
    /// menus by aliasing a lower rendition into a higher slot — Kodik offers
    /// "720p" for an episode whose file is plainly named `480.mp4`. Keeping
    /// both would put a choice in front of the viewer that changes nothing
    /// and labels it with a resolution the file does not have, so the lowest
    /// — and therefore truthful — label is the one kept.
    fn normalize(&mut self) {
        // Ascending first, so the survivor of a shared URL is its lowest,
        // honest label rather than the padded one.
        self.variants.sort_by_key(|v| v.height);
        let mut seen = std::collections::HashSet::new();
        self.variants.retain(|v| seen.insert(v.url.clone()));

        self.variants.sort_by_key(|v| std::cmp::Reverse(v.height));
        self.variants.dedup_by(|a, b| a.height == b.height);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubtitleTrack {
    pub language: String,
    pub url: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("no extractor is registered for host `{host}`")]
    UnsupportedHost { host: String },

    #[error("`{url}` has no host")]
    NoHost { url: String },

    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("invalid url: {0}")]
    Url(#[from] url::ParseError),

    /// The page loaded but did not contain what the protocol requires. Names
    /// the missing piece, because this is how a host-side change surfaces.
    #[error("{host} page is missing `{what}` — the host's format likely changed")]
    UnexpectedFormat { host: &'static str, what: String },

    /// The host answered, but with no playable rendition.
    #[error("{host} returned no playable stream")]
    NoStreams { host: &'static str },
}

pub type Result<T> = std::result::Result<T, ExtractError>;

/// Turns one host's embed page into a playable stream.
#[async_trait]
pub trait Extractor: Send + Sync {
    /// Hosts this extractor claims, lowercase and without a leading `www.`.
    fn hosts(&self) -> &'static [&'static str];

    /// Name used in errors and logs.
    fn name(&self) -> &'static str;

    async fn resolve(&self, embed_url: &str) -> Result<ResolvedStream>;
}

/// Dispatches an embed URL to whichever extractor claims its host.
#[derive(Clone)]
pub struct Registry {
    extractors: Vec<Arc<dyn Extractor>>,
}

impl Registry {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            extractors: Vec::new(),
        }
    }

    /// Every implemented extractor.
    ///
    /// Not yet covered: SovetRomantica, whose host is unreachable from the
    /// network this was developed on, so its protocol could not be observed —
    /// and guessing it would mean reading someone else's implementation.
    /// Allvideo, StudioMir, Myvi, VKVideo, OK, RuTube and MailRu are listed by
    /// the official client but did not appear in the sampled catalogue.
    pub fn new(http: reqwest::Client) -> Self {
        Self::empty()
            .with(KodikExtractor::new(http.clone()))
            .with(AniLibriaExtractor::new(http.clone()))
            .with(SibnetExtractor::new(http))
    }

    #[must_use]
    pub fn with(mut self, extractor: impl Extractor + 'static) -> Self {
        self.extractors.push(Arc::new(extractor));
        self
    }

    /// The extractor claiming `host`, if any.
    #[must_use]
    pub fn for_host(&self, host: &str) -> Option<&dyn Extractor> {
        let host = normalize_host(host);
        self.extractors
            .iter()
            .find(|e| e.hosts().iter().any(|h| *h == host))
            .map(AsRef::as_ref)
    }

    /// Whether any extractor can handle this URL.
    #[must_use]
    pub fn supports(&self, embed_url: &str) -> bool {
        host_of(embed_url).is_some_and(|h| self.for_host(&h).is_some())
    }

    /// Resolves an embed URL to a playable stream.
    pub async fn resolve(&self, embed_url: &str) -> Result<ResolvedStream> {
        let host = host_of(embed_url).ok_or_else(|| ExtractError::NoHost {
            url: embed_url.to_owned(),
        })?;

        let extractor = self
            .for_host(&host)
            .ok_or_else(|| ExtractError::UnsupportedHost { host: host.clone() })?;

        tracing::debug!(host = %host, extractor = extractor.name(), "resolving embed");
        let mut stream = extractor.resolve(embed_url).await?;
        stream.normalize();

        if stream.variants.is_empty() {
            return Err(ExtractError::NoStreams {
                host: extractor.name(),
            });
        }
        Ok(stream)
    }

    /// Hosts covered, for reporting what the client can and cannot play.
    #[must_use]
    pub fn supported_hosts(&self) -> Vec<&'static str> {
        let mut hosts: Vec<_> = self
            .extractors
            .iter()
            .flat_map(|e| e.hosts().iter().copied())
            .collect();
        hosts.sort_unstable();
        hosts
    }
}

/// Host of a URL, lowercased and stripped of `www.`.
#[must_use]
pub fn host_of(url: &str) -> Option<String> {
    // Episode URLs are occasionally protocol-relative.
    let parsed = if url.starts_with("//") {
        url::Url::parse(&format!("https:{url}"))
    } else {
        url::Url::parse(url)
    };
    parsed.ok()?.host_str().map(normalize_host)
}

fn normalize_host(host: &str) -> String {
    let host = host.to_ascii_lowercase();
    host.strip_prefix("www.").unwrap_or(&host).to_owned()
}

/// Gives a protocol-relative URL an explicit scheme. Episode URLs occasionally
/// arrive without one.
#[must_use]
pub(crate) fn with_scheme(url: &str) -> String {
    match url.strip_prefix("//") {
        Some(rest) => format!("https://{rest}"),
        None => url.to_owned(),
    }
}

/// `scheme://host/` for use as a `Referer`.
pub(crate) fn origin_of(url: &url::Url) -> Option<String> {
    Some(format!("{}://{}/", url.scheme(), url.host_str()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_is_normalized() {
        assert_eq!(
            host_of("https://WWW.Kodik.CC/seria/1/a/720p").as_deref(),
            Some("kodik.cc")
        );
        assert_eq!(
            host_of("//kodikplayer.com/x").as_deref(),
            Some("kodikplayer.com")
        );
        assert_eq!(host_of("not a url"), None);
    }

    #[test]
    fn stream_kind_prefers_mime_then_url() {
        assert_eq!(
            StreamKind::classify(Some("application/x-mpegURL"), "https://h/v.mp4"),
            StreamKind::Hls
        );
        assert_eq!(
            StreamKind::classify(None, "https://h/v/manifest.m3u8"),
            StreamKind::Hls
        );
        assert_eq!(
            StreamKind::classify(None, "https://h/v.mp4"),
            StreamKind::Progressive
        );
    }

    fn stream_of(heights: &[u32]) -> ResolvedStream {
        let mut s = ResolvedStream {
            variants: heights
                .iter()
                .map(|h| StreamVariant {
                    height: *h,
                    url: format!("https://h/{h}.m3u8"),
                    kind: StreamKind::Hls,
                })
                .collect(),
            ..Default::default()
        };
        s.normalize();
        s
    }

    #[test]
    fn normalize_orders_high_to_low_and_dedups() {
        let s = stream_of(&[360, 1080, 720, 720]);
        let heights: Vec<_> = s.variants.iter().map(|v| v.height).collect();
        assert_eq!(heights, vec![1080, 720, 360]);
    }

    #[test]
    fn heights_sharing_one_url_collapse_to_the_honest_label() {
        // Kodik pads its menu by serving the 480p file in the 720p slot. A
        // viewer picking "720p" would get 480p and no way to tell.
        let mut stream = ResolvedStream {
            variants: vec![
                StreamVariant {
                    height: 720,
                    url: "https://h/480.m3u8".to_owned(),
                    kind: StreamKind::Hls,
                },
                StreamVariant {
                    height: 480,
                    url: "https://h/480.m3u8".to_owned(),
                    kind: StreamKind::Hls,
                },
                StreamVariant {
                    height: 360,
                    url: "https://h/360.m3u8".to_owned(),
                    kind: StreamKind::Hls,
                },
            ],
            ..Default::default()
        };
        stream.normalize();

        let heights: Vec<_> = stream.variants.iter().map(|v| v.height).collect();
        assert_eq!(heights, vec![480, 360], "the aliased 720p must not survive");
    }

    #[test]
    fn genuinely_distinct_renditions_all_survive() {
        let mut stream = stream_of(&[1080, 720, 480]);
        stream.normalize();
        assert_eq!(stream.variants.len(), 3);
    }

    #[test]
    fn at_most_picks_the_largest_that_fits() {
        let s = stream_of(&[360, 720, 1080]);
        assert_eq!(s.at_most(1080).unwrap().height, 1080);
        assert_eq!(s.at_most(900).unwrap().height, 720);
        assert_eq!(s.at_most(480).unwrap().height, 360);
        // Everything is larger than requested: fall back rather than fail.
        assert_eq!(s.at_most(240).unwrap().height, 360);
    }

    #[test]
    fn unsupported_host_is_reported_by_name() {
        let registry = Registry::empty();
        assert!(!registry.supports("https://example.com/x"));
    }

    #[test]
    fn mpv_args_append_headers_and_split_out_the_user_agent() {
        let stream = ResolvedStream {
            headers: BTreeMap::from([
                ("Referer".to_owned(), "https://host/".to_owned()),
                ("User-Agent".to_owned(), "Agent/1.0".to_owned()),
            ]),
            ..Default::default()
        };
        let args = stream.mpv_args();

        assert!(args.contains(&"--user-agent=Agent/1.0".to_owned()));
        assert!(args.contains(&"--http-header-fields-append=Referer: https://host/".to_owned()));
        // Repeating the plain list option would discard earlier headers.
        assert!(
            !args.iter().any(|a| a.starts_with("--http-header-fields=")),
            "must not use the replacing form: {args:?}"
        );
        // Without this the slower edge nodes read as a broken extractor.
        assert!(
            args.iter().any(|a| a.starts_with("--network-timeout=")),
            "missing the network timeout: {args:?}"
        );
    }
}
