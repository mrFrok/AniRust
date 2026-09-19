// SPDX-License-Identifier: GPL-3.0-or-later

//! Putting the downloaded stream into a container.
//!
//! Joined HLS segments are a valid elementary stream but a poor file: there is
//! no index, so seeking means scanning. ffmpeg copies the streams into an MP4
//! with one — `-c copy`, so nothing is re-encoded, nothing is lost, and the
//! whole thing takes about as long as reading the file.

use std::path::{Path, PathBuf};

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
}
