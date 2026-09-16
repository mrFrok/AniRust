// SPDX-License-Identifier: GPL-3.0-or-later

//! AniRust.
//!
//! At this stage the application exists to prove one thing: that mpv's output
//! reaches a Slint surface as a borrowed GL texture, with no per-frame copy
//! through the CPU. The design system and the real screens are built on top
//! once that is confirmed, because a beautiful UI around a player that cannot
//! draw is worth nothing.

mod video;

use std::time::Duration;

use anyhow::{Context, Result, bail};
use slint::ComponentHandle;

use anirust_api::{Client, EpisodeSort};
use anirust_extract::{Registry, ResolvedStream};
use anirust_player::{MediaSource, Player, PlayerConfig};

use crate::video::VideoBridge;

slint::include_modules!();

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive("anirust=info".parse()?)
                .from_env_lossy(),
        )
        .with_writer(std::io::stderr)
        .init();

    let target = Target::from_args()?;

    // The stream is resolved before the event loop starts: the UI has nothing
    // to show until there is something to play, and keeping the async work out
    // of the loop keeps this file about rendering.
    let stream = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting the async runtime")?
        .block_on(target.resolve())?;

    let best = stream
        .best()
        .context("nothing resolved to a playable stream")?;
    tracing::info!(url = %best.url, height = best.height, "resolved");

    let window = MainWindow::new().context("creating the window")?;
    window.set_status("connecting...".into());

    // Bring-up switches. Hardware decoding hands frames over through
    // DMA-BUF/EGLImage, which is where torn-band artefacts usually come from,
    // so it has to be togglable without a rebuild while this is being sorted.
    let config = PlayerConfig {
        hwdec: std::env::var("ANIRUST_HWDEC")
            .map(std::borrow::Cow::Owned)
            .unwrap_or(std::borrow::Cow::Borrowed(anirust_player::DEFAULT_HWDEC)),
        // Bring-up knobs, so the picture can be bisected without a rebuild.
        fbo_format: std::env::var("ANIRUST_FBO")
            .map(std::borrow::Cow::Owned)
            .ok()
            .or(Some(std::borrow::Cow::Borrowed(
                anirust_player::DEFAULT_FBO_FORMAT,
            ))),
        // Until there is a settings screen, these come from the environment.
        upscale: match std::env::var("ANIRUST_UPSCALE").as_deref() {
            Ok("fast") => anirust_player::UpscalePreset::Fast,
            Ok("balanced") => anirust_player::UpscalePreset::Balanced,
            Ok("quality") => anirust_player::UpscalePreset::Quality,
            _ => anirust_player::UpscalePreset::Off,
        },
        interpolation: std::env::var("ANIRUST_INTERP").is_ok(),
        dumb_mode: std::env::var("ANIRUST_DUMB").is_ok(),
        direct_rendering: std::env::var("ANIRUST_DR").as_deref() != Ok("no"),
        dither_depth: std::env::var("ANIRUST_DITHER")
            .ok()
            .and_then(|v| v.parse().ok()),
        verbose_log: std::env::var("ANIRUST_MPV_LOG").is_ok(),
        ..PlayerConfig::default()
    };
    tracing::info!(hwdec = %config.hwdec, "player config");
    let player = Player::new(&config).context("creating the player")?;
    let bridge = VideoBridge::new(player);

    bridge
        .attach(&window, |window, frame| {
            window.set_video_frame(frame);
            window.set_has_video(true);
        })
        .context("attaching video to the window")?;

    bridge
        .play(
            MediaSource::new(&best.url).headers(
                stream
                    .headers
                    .iter()
                    .map(|(name, value)| (name.as_str(), value.as_str())),
            ),
        )
        .context("starting playback")?;

    let player = bridge.player();

    wire_controls(&window, player);
    update_status(&window, player);

    window.run().context("running the event loop")?;
    Ok(())
}

fn wire_controls(window: &MainWindow, player: &'static Player) {
    window.on_toggle_pause(move || {
        if let Err(error) = player.toggle_pause() {
            tracing::warn!(%error, "pause failed");
        }
    });

    window.on_seek(move |delta| {
        if let Err(error) = player.seek_by(f64::from(delta)) {
            tracing::warn!(%error, "seek failed");
        }
    });
}

/// Refreshes the status line once a second.
fn update_status(window: &MainWindow, player: &'static Player) {
    let weak = window.as_weak();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_secs(1),
        move || {
            let Some(window) = weak.upgrade() else { return };
            let position = player.position().unwrap_or_default();
            let status = match player.duration() {
                Some(duration) => format!("{} / {}", format_time(position), format_time(duration)),
                None => "buffering...".to_owned(),
            };
            window.set_status(status.into());
        },
    );
    // The timer stops when dropped, and the UI needs it for the whole run.
    std::mem::forget(timer);
}

fn format_time(value: Duration) -> String {
    let total = value.as_secs();
    let (hours, minutes, seconds) = (total / 3600, (total % 3600) / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// What the application was asked to play.
enum Target {
    /// A URL, embed or direct.
    Url(String),
    /// A release and an episode number, resolved through the API.
    Episode { release_id: i64, position: i32 },
}

impl Target {
    fn from_args() -> Result<Self> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        match args.as_slice() {
            [url] if url.contains("://") => Ok(Self::Url(url.clone())),
            [release_id] => Ok(Self::Episode {
                release_id: release_id.parse().context("release id must be a number")?,
                position: 1,
            }),
            [release_id, position] => Ok(Self::Episode {
                release_id: release_id.parse().context("release id must be a number")?,
                position: position.parse().context("episode must be a number")?,
            }),
            _ => bail!("usage:\n  anirust <release-id> [episode]\n  anirust <url>"),
        }
    }

    async fn resolve(&self) -> Result<ResolvedStream> {
        let registry = Registry::new(reqwest_client());

        let url = match self {
            Self::Url(url) => url.clone(),
            Self::Episode {
                release_id,
                position,
            } => {
                let client = Client::new().context("creating the API client")?;
                let dubbers = client.dubbers(*release_id).await?;
                let dubber = dubbers.first().context("the release has no voice-overs")?;
                let sources = client.sources(*release_id, dubber.id).await?;
                let source = sources.first().context("the voice-over has no sources")?;
                let episodes = client
                    .episodes(*release_id, dubber.id, source.id, EpisodeSort::Ascending)
                    .await?;
                episodes
                    .into_iter()
                    .find(|e| e.position == *position)
                    .with_context(|| format!("no episode {position}"))?
                    .url
            }
        };

        // Routing by host rather than by the API's `iframe` flag, which lies
        // for several hosts.
        if registry.supports(&url) {
            Ok(registry.resolve(&url).await?)
        } else {
            Ok(ResolvedStream {
                variants: vec![anirust_extract::StreamVariant {
                    height: anirust_extract::UNKNOWN_HEIGHT,
                    kind: anirust_extract::StreamKind::classify(None, &url),
                    url,
                }],
                ..Default::default()
            })
        }
    }
}

fn reqwest_client() -> reqwest::Client {
    reqwest::Client::builder()
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}
