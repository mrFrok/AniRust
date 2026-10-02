// SPDX-License-Identifier: GPL-3.0-or-later

//! A local relay between the player and the video CDNs.
//!
//! mpv fetches an HLS stream one six-second segment at a time and opens a
//! new connection for every one of them. From some networks the CDN nodes
//! drop a share of new connections outright — measured from a Russian one
//! without a VPN, about one connection in three never got past the
//! handshake, while the next attempt to the same address took 13ms. A new
//! connection per segment then means a stall every few segments: episodes
//! that take a minute to start, or never do. The official app does not
//! suffer it because its HTTP client keeps connections and reuses them.
//!
//! So the player is pointed at this relay on 127.0.0.1 instead, and the relay
//! fetches from the CDN with a client that keeps its connections, and races
//! a second attempt against one that has not answered within a moment.
//!
//! An upstream address travels in the path, so relative addresses in a
//! playlist resolve to the relay as they would to the CDN:
//! `http://127.0.0.1:{port}/https/{host}/{path}?{query}`. Redirects are
//! handed back to the player rewritten the same way, and absolute addresses
//! inside playlists are rewritten when the playlist passes through.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use futures::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// How long an attempt may go unanswered before a second one is started
/// beside it. A healthy node answers in tens of milliseconds.
const HEDGE_AFTER: Duration = Duration::from_millis(1500);

/// Rounds of attempts before a request is given up on.
const ROUNDS: usize = 4;

/// The largest request head the relay reads. mpv's are a few hundred bytes.
const MAX_HEAD: usize = 16 * 1024;

struct Relay {
    port: u16,
    upstream: reqwest::Client,
    /// Headers the current stream's host wants: a `Referer`, a `User-Agent`.
    headers: RwLock<BTreeMap<String, String>>,
}

static RELAY: OnceLock<Arc<Relay>> = OnceLock::new();

/// Starts the relay. Called once at startup; a relay that cannot start
/// leaves the player fetching directly, as it did before.
pub fn start() {
    let upstream = match reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .tcp_keepalive(Duration::from_secs(30))
        .pool_idle_timeout(Duration::from_secs(90))
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            tracing::warn!(%error, "the stream relay has no client; playing directly");
            return;
        }
    };
    let listener = match std::net::TcpListener::bind(("127.0.0.1", 0)) {
        Ok(listener) => listener,
        Err(error) => {
            tracing::warn!(%error, "the stream relay could not listen; playing directly");
            return;
        }
    };
    let Ok(port) = listener.local_addr().map(|a| a.port()) else {
        return;
    };
    if listener.set_nonblocking(true).is_err() {
        return;
    }
    let relay = Arc::new(Relay {
        port,
        upstream,
        headers: RwLock::new(BTreeMap::new()),
    });
    if RELAY.set(Arc::clone(&relay)).is_err() {
        return;
    }
    crate::tasks::background(async move {
        let listener = match TcpListener::from_std(listener) {
            Ok(listener) => listener,
            Err(error) => {
                tracing::warn!(%error, "the stream relay could not listen");
                return;
            }
        };
        tracing::info!(port, "stream relay listening");
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                continue;
            };
            let relay = Arc::clone(&relay);
            tokio::spawn(async move {
                if let Err(error) = serve(&relay, socket).await {
                    tracing::debug!(error = format!("{error:#}"), "relay request failed");
                }
            });
        }
    });
}

/// The address the player should open for a stream, and the headers it
/// should send itself — none when the relay carries them.
pub fn route(url: &str, headers: BTreeMap<String, String>) -> (String, BTreeMap<String, String>) {
    let Some(relay) = RELAY.get() else {
        return (url.to_owned(), headers);
    };
    let Some(local) = to_local(relay.port, url) else {
        return (url.to_owned(), headers);
    };
    if let Ok(mut current) = relay.headers.write() {
        *current = headers;
    }
    (local, BTreeMap::new())
}

/// `https://host/path?q` as the relay's own address for it.
fn to_local(port: u16, url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    if scheme != "http" && scheme != "https" || rest.is_empty() {
        return None;
    }
    Some(format!("http://127.0.0.1:{port}/{scheme}/{rest}"))
}

/// The relay's address back to the upstream one.
fn to_upstream(target: &str) -> Option<String> {
    let rest = target.strip_prefix('/')?;
    let (scheme, rest) = rest.split_once('/')?;
    if (scheme != "http" && scheme != "https") || rest.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{rest}"))
}

async fn serve(relay: &Relay, mut socket: TcpStream) -> Result<()> {
    let head = read_head(&mut socket).await?;
    let mut lines = head.split("\r\n");
    let request = lines.next().unwrap_or_default();
    let mut parts = request.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    let range = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("range"))
        .map(|(_, value)| value.trim().to_owned());

    let Some(url) = to_upstream(target) else {
        return respond_status(&mut socket, 400, "Bad Request").await;
    };
    let head_only = method.eq_ignore_ascii_case("HEAD");
    if !head_only && !method.eq_ignore_ascii_case("GET") {
        return respond_status(&mut socket, 405, "Method Not Allowed").await;
    }

    let headers = relay.headers.read().map(|h| h.clone()).unwrap_or_default();
    let response = fetch(&relay.upstream, &url, &headers, range.as_deref(), head_only).await;
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            tracing::debug!(error = format!("{error:#}"), %url, "upstream unreachable");
            return respond_status(&mut socket, 502, "Bad Gateway").await;
        }
    };

    let status = response.status();
    // A redirect goes back to the player in the relay's terms, so the
    // address it resolves relative paths against stays the relay's.
    if status.is_redirection()
        && let Some(location) = response.headers().get(reqwest::header::LOCATION)
    {
        let location = location.to_str().unwrap_or_default();
        let resolved = reqwest::Url::parse(&url)
            .and_then(|base| base.join(location))
            .map(|u| u.to_string())
            .unwrap_or_else(|_| location.to_owned());
        let local = to_local(relay.port, &resolved).unwrap_or(resolved);
        let head = format!(
            "HTTP/1.1 302 Found\r\nLocation: {local}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        socket.write_all(head.as_bytes()).await?;
        return Ok(());
    }

    let content_type = header(&response, reqwest::header::CONTENT_TYPE);
    let playlist = content_type
        .as_deref()
        .is_some_and(|t| t.contains("mpegurl"))
        || url
            .split('?')
            .next()
            .is_some_and(|path| path.ends_with(".m3u8"));

    let mut head = format!(
        "HTTP/1.1 {} {}\r\nConnection: close\r\n",
        status.as_u16(),
        status.canonical_reason().unwrap_or("")
    );
    for name in [
        reqwest::header::CONTENT_TYPE,
        reqwest::header::CONTENT_RANGE,
        reqwest::header::ACCEPT_RANGES,
    ] {
        if let Some(value) = header(&response, name.clone()) {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
    }

    if playlist && !head_only {
        let text = response.text().await.context("reading the playlist")?;
        let text = rewrite_playlist(relay.port, &text);
        head.push_str(&format!("Content-Length: {}\r\n\r\n", text.len()));
        socket.write_all(head.as_bytes()).await?;
        socket.write_all(text.as_bytes()).await?;
        return Ok(());
    }

    if let Some(length) = header(&response, reqwest::header::CONTENT_LENGTH) {
        head.push_str(&format!("Content-Length: {length}\r\n"));
    }
    head.push_str("\r\n");
    socket.write_all(head.as_bytes()).await?;
    if head_only {
        return Ok(());
    }
    let mut body = response.bytes_stream();
    while let Some(chunk) = body.next().await {
        let chunk = chunk.context("reading from upstream")?;
        // The player closing the connection — it seeks, or moves on — ends
        // this copy; that is not an error worth a line in the log.
        if socket.write_all(&chunk).await.is_err() {
            return Ok(());
        }
    }
    Ok(())
}

fn header(response: &reqwest::Response, name: reqwest::header::HeaderName) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

async fn read_head(socket: &mut TcpStream) -> Result<String> {
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0_u8; 1024];
    loop {
        let read = socket.read(&mut chunk).await?;
        if read == 0 {
            return Err(anyhow!("the player closed the connection"));
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            buffer.truncate(end);
            return Ok(String::from_utf8_lossy(&buffer).into_owned());
        }
        if buffer.len() > MAX_HEAD {
            return Err(anyhow!("request head too large"));
        }
    }
}

async fn respond_status(socket: &mut TcpStream, code: u16, reason: &str) -> Result<()> {
    let head =
        format!("HTTP/1.1 {code} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    socket.write_all(head.as_bytes()).await?;
    Ok(())
}

/// Sends one request upstream. An attempt that has not answered after a
/// moment gets a second beside it, and the first to answer wins; a round
/// that fails is tried again.
async fn fetch(
    client: &reqwest::Client,
    url: &str,
    headers: &BTreeMap<String, String>,
    range: Option<&str>,
    head_only: bool,
) -> Result<reqwest::Response> {
    let attempt = || {
        let mut request = if head_only {
            client.head(url)
        } else {
            client.get(url)
        };
        for (name, value) in headers {
            request = request.header(name.as_str(), value.as_str());
        }
        if let Some(range) = range {
            request = request.header(reqwest::header::RANGE, range);
        }
        request.send()
    };

    let mut last = None;
    for _ in 0..ROUNDS {
        let first = attempt();
        tokio::pin!(first);
        let outcome = tokio::select! {
            result = &mut first => result,
            () = tokio::time::sleep(HEDGE_AFTER) => {
                let second = attempt();
                tokio::pin!(second);
                tokio::select! {
                    result = &mut first => match result {
                        Ok(response) => Ok(response),
                        Err(_) => second.await,
                    },
                    result = &mut second => match result {
                        Ok(response) => Ok(response),
                        Err(_) => first.await,
                    },
                }
            }
        };
        match outcome {
            Ok(response) => return Ok(response),
            Err(error) => last = Some(error),
        }
    }
    Err(last.map_or_else(|| anyhow!("no attempt was made"), anyhow::Error::from))
}

/// Points absolute addresses in a playlist at the relay. Relative ones need
/// nothing: they resolve against the relay's own address.
fn rewrite_playlist(port: u16, text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 256);
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            out.push_str(&to_local(port, trimmed).unwrap_or_else(|| line.to_owned()));
        } else if trimmed.starts_with('#') && line.contains("URI=\"http") {
            out.push_str(&rewrite_uri_attribute(port, line));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

/// `#EXT-X-KEY:...,URI="https://..."` with the address pointed at the relay.
fn rewrite_uri_attribute(port: u16, line: &str) -> String {
    let Some(start) = line.find("URI=\"").map(|at| at + 5) else {
        return line.to_owned();
    };
    let Some(length) = line[start..].find('"') else {
        return line.to_owned();
    };
    let uri = &line[start..start + length];
    match to_local(port, uri) {
        Some(local) => format!("{}{local}{}", &line[..start], &line[start + length..]),
        None => line.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_goes_through_the_relay_and_back() {
        let url = "https://cloud.solodcdn.com/a/b/720.mp4:hls:manifest.m3u8?x=1";
        let local = to_local(4000, url).unwrap();
        assert_eq!(
            local,
            "http://127.0.0.1:4000/https/cloud.solodcdn.com/a/b/720.mp4:hls:manifest.m3u8?x=1"
        );
        let target = local.strip_prefix("http://127.0.0.1:4000").unwrap();
        assert_eq!(to_upstream(target).as_deref(), Some(url));
    }

    #[test]
    fn only_the_web_goes_through_it() {
        assert!(to_local(1, "file:///tmp/x.mkv").is_none());
        assert!(to_upstream("/ftp/host/x").is_none());
    }

    #[test]
    fn absolute_addresses_in_a_playlist_are_rewritten_and_relative_ones_kept() {
        let playlist = "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"https://k.example/key\"\n#EXTINF:6,\n./seg-1.ts\n#EXTINF:6,\nhttps://cdn.example/seg-2.ts\n";
        let out = rewrite_playlist(9, playlist);
        assert!(
            out.contains("URI=\"http://127.0.0.1:9/https/k.example/key\""),
            "{out}"
        );
        assert!(out.contains("\n./seg-1.ts\n"), "{out}");
        assert!(
            out.contains("\nhttp://127.0.0.1:9/https/cdn.example/seg-2.ts\n"),
            "{out}"
        );
    }
}
