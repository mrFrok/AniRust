// SPDX-License-Identifier: GPL-3.0-or-later

//! Putting the downloaded stream into a container.
//!
//! Joined HLS segments are a valid elementary stream but a poor file: there is
//! no index, so seeking means scanning. ffmpeg copies the streams into an MP4
//! with one — `-c copy`, so nothing is re-encoded, nothing is lost, and the
//! whole thing takes about as long as reading the file.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum FfmpegError {
    #[error(
        "ffmpeg was not found. Install it with your package manager \
         (Debian and Ubuntu: `ffmpeg`; Arch: `ffmpeg`), or point ANIRUST_FFMPEG at it."
    )]
    NotFound,

    #[error("ffmpeg could not be started: {0}")]
    Spawn(#[source] std::io::Error),

    #[error("ffmpeg failed: {0}")]
    Failed(String),

    #[error("cancelled")]
    Cancelled,
}

/// Where ffmpeg is, once.
#[derive(Debug, Clone)]
pub struct Ffmpeg {
    program: PathBuf,
}

impl Ffmpeg {
    /// Finds ffmpeg on the system, or on `ANIRUST_FFMPEG`.
    ///
    /// No portable build is fetched: downloading and running a binary from the
    /// internet on the user's behalf is a bigger decision than saving an
    /// episode, and every platform this targets has a package for it.
    pub fn find() -> Result<Self, FfmpegError> {
        if let Some(from_env) = std::env::var_os("ANIRUST_FFMPEG") {
            return Ok(Self {
                program: PathBuf::from(from_env),
            });
        }

        // `-version` rather than `which`: it answers the question that
        // actually matters, which is whether it runs.
        let found = std::process::Command::new("ffmpeg")
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success());

        if found {
            Ok(Self {
                program: PathBuf::from("ffmpeg"),
            })
        } else {
            Err(FfmpegError::NotFound)
        }
    }

    /// Copies the streams of `source` into an MP4 at `destination`.
    pub async fn remux(&self, source: &Path, destination: &Path) -> Result<(), FfmpegError> {
        let output = Command::new(&self.program)
            .arg("-y")
            .arg("-loglevel")
            .arg("error")
            // The joined segments have no container header of their own, so
            // ffmpeg is told to work it out rather than trust the extension.
            .arg("-i")
            .arg(source)
            .arg("-c")
            .arg("copy")
            // Lets a player start before the whole file is read, which matters
            // for something that will be watched off a disk.
            .arg("-movflags")
            .arg("+faststart")
            .arg(destination)
            .output()
            .await
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    FfmpegError::NotFound
                } else {
                    FfmpegError::Spawn(error)
                }
            })?;

        if output.status.success() {
            return Ok(());
        }

        let message = String::from_utf8_lossy(&output.stderr);
        Err(FfmpegError::Failed(
            message.lines().last().unwrap_or("no output").to_owned(),
        ))
    }

    /// Puts the picture from `picture` together with every sound and
    /// subtitle track of `original`, copied as they are, into `destination`.
    ///
    /// For an episode whose picture was made again — upscaled, given more
    /// frames — and whose sound should not be touched at all.
    pub async fn merge(
        &self,
        picture: &Path,
        original: &Path,
        destination: &Path,
        fps: Option<f64>,
        stop: &AtomicBool,
    ) -> Result<(), FfmpegError> {
        let mut command = Command::new(&self.program);
        command
            .arg("-y")
            .arg("-nostdin")
            .arg("-loglevel")
            .arg("error")
            .arg("-i")
            .arg(picture)
            .arg("-i")
            .arg(original)
            .args([
                "-map",
                "0:v:0",
                "-map",
                "1:a?",
                "-map",
                "1:s?",
                "-map",
                "1:t?",
                "-map_metadata",
                "1",
                "-map_chapters",
                "1",
            ])
            .args(["-c", "copy"]);
        // Set the nominal rate when Matroska's millisecond packet durations
        // rounded it. Stream copy leaves the timestamps intact.
        if let Some(fps) = fps {
            command.arg("-r:v:0").arg(fps.to_string());
        }
        let mut child = command
            .arg(destination)
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    FfmpegError::NotFound
                } else {
                    FfmpegError::Spawn(error)
                }
            })?;

        let mut stderr = child.stderr.take().expect("ffmpeg stderr is piped");
        let diagnostics = async {
            let mut output = Vec::new();
            stderr.read_to_end(&mut output).await?;
            Ok::<_, std::io::Error>(output)
        };
        let mut poll = tokio::time::interval(Duration::from_millis(100));
        let cancelled = async {
            while !stop.load(Ordering::Relaxed) {
                poll.tick().await;
            }
        };
        let (status, stderr) = tokio::select! {
            result = async { tokio::try_join!(child.wait(), diagnostics) } => {
                result.map_err(FfmpegError::Spawn)?
            }
            () = cancelled => {
                let _ = child.kill().await;
                return Err(FfmpegError::Cancelled);
            }
        };
        if status.success() {
            return Ok(());
        }

        let message = String::from_utf8_lossy(&stderr);
        Err(FfmpegError::Failed(
            message.lines().last().unwrap_or("no output").to_owned(),
        ))
    }
}
