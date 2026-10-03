// SPDX-License-Identifier: GPL-3.0-or-later

//! Saving an episode to disk.
//!
//! Most of this catalogue is served as HLS: a manifest listing a few hundred
//! small segments. Fetching them one after another is bound by round trips
//! rather than by bandwidth — these CDN nodes can take seconds just to accept a
//! connection — so segments are fetched several at a time and written in order.
//!
//! What lands on disk is the segments joined end to end, which for MPEG-TS and
//! fragmented MP4 is already a valid stream. ffmpeg is then asked to remux it
//! into a container players seek properly in, copying the streams rather than
//! re-encoding: no quality is lost and the whole thing takes a second or two.

mod ffmpeg;
mod hls;

use std::io;
use std::path::{Path, PathBuf};

use anirust_extract::ResolvedStream;
use tokio::io::AsyncWriteExt;

pub use ffmpeg::{Ffmpeg, FfmpegError};

/// How many segments are in flight at once.
///
/// Enough to keep a link busy across hosts that answer slowly, few enough that
/// a CDN does not start refusing. Anixart's nodes tolerate this comfortably;
/// the measured limit was well above it.
const PARALLEL_SEGMENTS: usize = 8;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("nothing playable in this stream")]
    NothingToDownload,

    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("could not write {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("the manifest could not be read: {0}")]
    Manifest(String),

    #[error(transparent)]
    Ffmpeg(#[from] FfmpegError),

    #[error("cancelled")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, Error>;

/// How far along a download is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// Reading the manifest, before the size of the job is known.
    Preparing,
    /// `done` of `total` segments, or of `total` bytes for a plain file.
    Fetching {
        done: u64,
        total: u64,
    },
    /// Handing the result to ffmpeg, which is quick but not instant.
    Remuxing,
    Finished,
}

impl Progress {
    /// A fraction for a progress bar, or `None` when the total is not yet
    /// known.
    #[must_use]
    pub fn fraction(self) -> Option<f32> {
        match self {
            Self::Preparing => None,
            Self::Fetching { done, total } if total > 0 => {
                Some((done as f32 / total as f32).clamp(0.0, 1.0))
            }
            Self::Fetching { .. } => None,
            Self::Remuxing | Self::Finished => Some(1.0),
        }
    }
}

/// A download in progress.
pub struct Download {
    http: reqwest::Client,
    ffmpeg: Ffmpeg,
}

impl Download {
    #[must_use]
    pub fn new(http: reqwest::Client, ffmpeg: Ffmpeg) -> Self {
        Self { http, ffmpeg }
    }

    /// Saves the best rendition of a stream to `destination`.
    ///
    /// `report` is called as the job advances; it runs on the calling task, so
    /// it should not block.
    pub async fn save(
        &self,
        stream: &ResolvedStream,
        destination: &Path,
        mut report: impl FnMut(Progress),
    ) -> Result<()> {
        let variant = stream.best().ok_or(Error::NothingToDownload)?;
        report(Progress::Preparing);

        // Next to the destination rather than in a temporary directory: a
        // half-written episode should be obvious, and on the same filesystem
        // the final rename costs nothing.
        let scratch = destination.with_extension("part");

        match variant.kind {
            anirust_extract::StreamKind::Hls => {
                hls::fetch(
                    &self.http,
                    &variant.url,
                    &stream.headers,
                    &scratch,
                    PARALLEL_SEGMENTS,
                    &mut report,
                )
                .await?;
            }
            anirust_extract::StreamKind::Progressive => {
                self.fetch_file(&variant.url, stream, &scratch, &mut report)
                    .await?;
            }
        }

        report(Progress::Remuxing);
        self.ffmpeg.remux(&scratch, destination).await?;

        // The joined segments have served their purpose; what is left is the
        // container ffmpeg wrote.
        if let Err(error) = tokio::fs::remove_file(&scratch).await {
            tracing::debug!(%error, path = %scratch.display(), "could not remove the scratch file");
        }

        report(Progress::Finished);
        Ok(())
    }

    /// Streams a progressive file straight to disk.
    async fn fetch_file(
        &self,
        url: &str,
        stream: &ResolvedStream,
        destination: &Path,
        report: &mut impl FnMut(Progress),
    ) -> Result<()> {
        let mut request = self.http.get(url);
        for (name, value) in &stream.headers {
            request = request.header(name, value);
        }

        let response = request.send().await?.error_for_status()?;
        let total = response.content_length().unwrap_or(0);
        let mut done = 0;

        let mut file = tokio::fs::File::create(destination)
            .await
            .map_err(|source| Error::Write {
                path: destination.to_owned(),
                source,
            })?;

        let mut response = response;
        while let Some(chunk) = response.chunk().await? {
            file.write_all(&chunk)
                .await
                .map_err(|source| Error::Write {
                    path: destination.to_owned(),
                    source,
                })?;
            done += chunk.len() as u64;
            report(Progress::Fetching { done, total });
        }

        file.flush().await.map_err(|source| Error::Write {
            path: destination.to_owned(),
            source,
        })?;
        Ok(())
    }
}

/// A filename for an episode, safe on every platform this runs on.
#[must_use]
pub fn file_name(release: &str, episode: i32, dubber: &str) -> String {
    let stem = if dubber.is_empty() {
        format!("{release} - {episode:02}")
    } else {
        format!("{release} - {episode:02} [{dubber}]")
    };
    format!("{}.mp4", safe_stem(&stem))
}

/// `text` made safe as a file name, before its extension, on every platform
/// this runs on.
///
/// Titles routinely contain characters Windows refuses and `/`, which would
/// quietly write somewhere else entirely.
#[must_use]
pub fn safe_stem(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();

    // Trailing dots and spaces are legal to create and impossible to open on
    // Windows.
    cleaned.trim_end_matches(['.', ' ']).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_episode_is_named_after_its_release() {
        assert_eq!(
            file_name("Демоны старшей школы", 3, "AniLibria"),
            "Демоны старшей школы - 03 [AniLibria].mp4"
        );
    }

    #[test]
    fn a_nameless_voice_over_is_left_out() {
        assert_eq!(file_name("Bebop", 12, ""), "Bebop - 12.mp4");
    }

    #[test]
    fn characters_that_would_pick_a_different_directory_are_replaced() {
        let name = file_name("A/B: C?", 1, "");
        assert!(!name.contains('/'), "{name}");
        assert_eq!(name, "A-B- C- - 01.mp4");
    }

    #[test]
    fn a_name_never_ends_in_a_dot_or_a_space() {
        // Windows will create these and then refuse to open them.
        assert_eq!(file_name("Title.", 1, ""), "Title. - 01.mp4");
        assert!(!file_name("x", 1, "dub. ").ends_with(". .mp4"));
    }

    #[test]
    fn progress_reports_a_fraction_only_once_it_knows_one() {
        assert_eq!(Progress::Preparing.fraction(), None);
        assert_eq!(Progress::Fetching { done: 0, total: 0 }.fraction(), None);
        assert_eq!(
            Progress::Fetching { done: 5, total: 10 }.fraction(),
            Some(0.5)
        );
        assert_eq!(Progress::Finished.fraction(), Some(1.0));
    }
}
