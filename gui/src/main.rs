// SPDX-License-Identifier: GPL-3.0-or-later

//! AniRust.
//!
//! At this stage the application exists to prove one thing: that mpv's output
//! reaches a Slint surface as a borrowed GL texture, with no per-frame copy
//! through the CPU. The design system and the real screens are built on top
//! once that is confirmed, because a beautiful UI around a player that cannot
//! draw is worth nothing.

mod video;

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use slint::ComponentHandle;

use anirust_api::{Client, EpisodeSort};
use anirust_extract::{Registry, ResolvedStream};
use anirust_player::{MediaSource, PlaybackState, Player, PlayerConfig, UpscalePreset};

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
    let playback = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting the async runtime")?
        .block_on(target.resolve())?;

    let Playback {
        stream,
        title,
        episode_label,
    } = playback;

    let best = stream
        .best()
        .context("nothing resolved to a playable stream")?;
    tracing::info!(url = %best.url, height = best.height, "resolved");

    let window = MainWindow::new().context("creating the window")?;
    window.set_lang(if is_russian_locale() {
        "ru".into()
    } else {
        "en".into()
    });
    window.set_release_title(title.into());
    window.set_episode_label(episode_label.into());
    window.set_state("loading".into());
    window.set_has_skip(stream.opening.is_some());
    window.set_quality_label(
        if best.height == anirust_extract::UNKNOWN_HEIGHT {
            "—".to_owned()
        } else {
            format!("{}p", best.height)
        }
        .into(),
    );

    // Bring-up knobs. Hardware decoding hands frames over through
    // DMA-BUF/EGLImage, which is where torn-band artefacts usually come from,
    // so it has to be togglable without a rebuild while this is being sorted.
    let config = PlayerConfig {
        hwdec: std::env::var("ANIRUST_HWDEC")
            .map(std::borrow::Cow::Owned)
            .unwrap_or(std::borrow::Cow::Borrowed(anirust_player::DEFAULT_HWDEC)),
        fbo_format: std::env::var("ANIRUST_FBO")
            .map(std::borrow::Cow::Owned)
            .ok()
            .or(Some(std::borrow::Cow::Borrowed(
                anirust_player::DEFAULT_FBO_FORMAT,
            ))),
        direct_rendering: std::env::var("ANIRUST_DR").as_deref() != Ok("no"),
        dither_depth: std::env::var("ANIRUST_DITHER")
            .ok()
            .and_then(|v| v.parse().ok()),
        dumb_mode: std::env::var("ANIRUST_DUMB").is_ok(),
        verbose_log: std::env::var("ANIRUST_MPV_LOG").is_ok(),
        ..PlayerConfig::default()
    };
    tracing::info!(hwdec = %config.hwdec, "player config");

    let player = Player::new(&config).context("creating the player")?;
    let bridge = VideoBridge::new(player).context("creating the video bridge")?;

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

    window.set_qualities(slint::ModelRc::new(slint::VecModel::from(
        stream
            .variants
            .iter()
            .map(|v| {
                if v.height == anirust_extract::UNKNOWN_HEIGHT {
                    slint::SharedString::from("auto")
                } else {
                    slint::SharedString::from(format!("{}p", v.height))
                }
            })
            .collect::<Vec<_>>(),
    )));

    let player = bridge.player();
    let settings = Rc::new(Settings::default());
    let bridge = Rc::new(bridge);
    wire_controls(&window, player, &settings, &bridge, stream);
    drive_status(&window, player, &settings, &bridge);

    window.run().context("running the event loop")?;
    Ok(())
}

/// Settings the overlay cycles through.
///
/// Kept beside the player rather than read back from mpv: "which preset is
/// selected" is a choice the interface owns, and mpv has no notion of a preset
/// once the shader list is applied.
struct Settings {
    speed: Cell<f64>,
    upscale: Cell<usize>,
    interpolation: Cell<bool>,
    quality: Cell<usize>,
    decoder: Cell<usize>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            speed: Cell::new(1.0),
            upscale: Cell::new(0),
            interpolation: Cell::new(false),
            quality: Cell::new(0),
            decoder: Cell::new(0),
        }
    }
}

const PRESETS: [UpscalePreset; 4] = [
    UpscalePreset::Off,
    UpscalePreset::Fast,
    UpscalePreset::Balanced,
    UpscalePreset::Quality,
];

/// mpv `hwdec` values behind the decoder choice, in the order the sheet lists
/// them: automatic, hardware, software.
const DECODERS: [&str; 3] = [anirust_player::DEFAULT_HWDEC, "nvdec,vaapi", "no"];

fn wire_controls(
    window: &MainWindow,
    player: &'static Player,
    settings: &Rc<Settings>,
    bridge: &Rc<VideoBridge>,
    stream: ResolvedStream,
) {
    let opening = stream.opening;
    let stream = Rc::new(stream);
    window.on_toggle_pause(move || {
        if let Err(error) = player.toggle_pause() {
            tracing::warn!(%error, "pause failed");
        }
    });

    window.on_seek_relative(move |delta| {
        if let Err(error) = player.seek_by(f64::from(delta)) {
            tracing::warn!(%error, "seek failed");
        }
    });

    window.on_seek_fraction(move |fraction| {
        let Some(duration) = player.duration() else {
            return;
        };
        let target = duration.mul_f64(f64::from(fraction).clamp(0.0, 1.0));
        if let Err(error) = player.seek_to(target) {
            tracing::warn!(%error, "seek failed");
        }
    });

    window.on_skip_opening(move || {
        let ends_at = opening.map(|range| Duration::from_secs(u64::from(range.end)));
        if let Err(error) = player.skip_opening(ends_at) {
            tracing::warn!(%error, "skip failed");
        }
    });

    let chosen = Rc::clone(settings);
    window.on_set_speed(move |speed| {
        let speed = f64::from(speed);
        chosen.speed.set(speed);
        if let Err(error) = player.set_speed(speed) {
            tracing::warn!(%error, "speed change failed");
        }
    });

    let chosen = Rc::clone(settings);
    window.on_set_upscale(move |index| {
        let index = (index.max(0) as usize).min(PRESETS.len() - 1);
        chosen.upscale.set(index);
        if let Err(error) = player.set_upscale(PRESETS[index]) {
            tracing::warn!(%error, preset = PRESETS[index].name(), "upscale change failed");
        }
    });

    // Switching rendition means opening a different URL, so playback resumes
    // where it left off rather than starting over.
    let chosen = Rc::clone(settings);
    let switch_stream = Rc::clone(&stream);
    let switch_bridge = Rc::clone(bridge);
    window.on_set_quality(move |index| {
        let index = index.max(0) as usize;
        let Some(variant) = switch_stream.variants.get(index) else {
            return;
        };
        chosen.quality.set(index);

        let resume = player.position().unwrap_or_default();
        let mut source = MediaSource::new(&variant.url).headers(
            switch_stream
                .headers
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str())),
        );
        if resume > Duration::ZERO {
            source = source.start_at(resume);
        }

        if let Err(error) = switch_bridge.play(source) {
            tracing::warn!(%error, height = variant.height, "quality change failed");
        }
    });

    let chosen = Rc::clone(settings);
    window.on_set_decoder(move |index| {
        let index = (index.max(0) as usize).min(DECODERS.len() - 1);
        chosen.decoder.set(index);
        if let Err(error) = player.set_hwdec(DECODERS[index]) {
            tracing::warn!(%error, value = DECODERS[index], "decoder change failed");
        }
    });

    let weak = window.as_weak();
    window.on_toggle_fullscreen(move || {
        let Some(window) = weak.upgrade() else { return };
        let next = !window.get_fullscreen();
        window.set_fullscreen(next);
        window.window().set_fullscreen(next);
    });

    let cycle = Rc::clone(settings);
    window.on_toggle_interpolation(move || {
        let next = !cycle.interpolation.get();
        cycle.interpolation.set(next);
        if let Err(error) = player.set_interpolation(next) {
            tracing::warn!(%error, "interpolation change failed");
        }
    });
}

/// Mirrors the player's state into the window, four times a second.
///
/// Fast enough that a clock and a progress bar look alive, slow enough that it
/// costs nothing next to rendering. The video itself is not driven from here —
/// that runs at display rate in the video bridge.
fn drive_status(
    window: &MainWindow,
    player: &'static Player,
    settings: &Rc<Settings>,
    bridge: &Rc<VideoBridge>,
) {
    let weak = window.as_weak();
    let settings = Rc::clone(settings);
    let bridge = Rc::clone(bridge);

    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(250),
        move || {
            let Some(window) = weak.upgrade() else { return };

            let position = player.position().unwrap_or_default();
            let duration = player.duration();

            window.set_position_text(format_time(position).into());
            window.set_duration_text(
                duration
                    .map_or_else(|| "--:--".to_owned(), format_time)
                    .into(),
            );

            // While the viewer is dragging, the bar shows where their pointer
            // is. Writing the player's clock over it here would make the thumb
            // fight the hand.
            if !window.get_scrubbing() {
                window.set_progress(fraction_of(position, duration));
            }
            window.set_buffered(fraction_of(
                player.buffered_until().unwrap_or(position),
                duration,
            ));

            // The decoder mpv actually engaged, which is not always the one
            // asked for — worth showing rather than hiding.
            window.set_decoder_label(
                player
                    .active_hwdec()
                    .map_or_else(|| "SW".to_owned(), |name| name.to_uppercase())
                    .into(),
            );

            // Say what is being produced, not just what arrived: with
            // upscaling on, the picture leaving the renderer is larger than
            // the source, and reporting the source would understate it.
            window.set_quality_label(
                quality_label(player.video_size(), bridge.rendered_size()).into(),
            );

            let state = player.state();
            window.set_state(state_name(state).into());
            window.set_paused(state == PlaybackState::Paused);
            // The render loop reads this instead of querying mpv on every
            // frame; a quarter-second of staleness costs nothing here.
            bridge.set_advancing(state.is_active());

            window.set_speed_label(format_speed(settings.speed.get()).into());
            window.set_upscale(settings.upscale.get() as i32);
            window.set_interpolation(settings.interpolation.get());
            window.set_quality(settings.quality.get() as i32);
            window.set_decoder(settings.decoder.get() as i32);
        },
    );
    // The window needs this for its whole life; dropping the timer would stop
    // the clock.
    std::mem::forget(timer);
}

fn fraction_of(position: Duration, duration: Option<Duration>) -> f32 {
    match duration {
        Some(total) if total.as_secs_f32() > 0.0 => {
            (position.as_secs_f32() / total.as_secs_f32()).clamp(0.0, 1.0)
        }
        _ => 0.0,
    }
}

/// Describes the picture: the source resolution, and what it is being
/// rendered at when that is larger.
///
/// Resolution only — never a frame rate. Interpolation resamples timing, it
/// does not synthesise frames, so advertising a higher fps would be a claim
/// the player cannot back.
fn quality_label(source: Option<(u32, u32)>, rendered: Option<(u32, u32)>) -> String {
    let Some((_, source_h)) = source else {
        return "—".to_owned();
    };

    match rendered {
        Some((_, rendered_h)) if rendered_h > source_h => {
            format!("{source_h}p → {rendered_h}p")
        }
        _ => format!("{source_h}p"),
    }
}

/// Matches the names the Slint side compares against.
fn state_name(state: PlaybackState) -> &'static str {
    match state {
        PlaybackState::Idle => "idle",
        PlaybackState::Loading => "loading",
        PlaybackState::Buffering => "buffering",
        PlaybackState::Playing => "playing",
        PlaybackState::Paused => "paused",
        PlaybackState::Ended => "ended",
    }
}

/// `1x`, `1.5x` — no trailing zero on whole rates.
fn format_speed(speed: f64) -> String {
    if (speed.fract()).abs() < f64::EPSILON {
        format!("{speed:.0}x")
    } else {
        format!("{speed}x")
    }
}

/// Russian when the locale asks for it, matching the probe's behaviour.
fn is_russian_locale() -> bool {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|var| std::env::var(var).ok().filter(|v| !v.is_empty()))
        .is_some_and(|locale| locale.to_ascii_lowercase().starts_with("ru"))
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

    async fn resolve(&self) -> Result<Playback> {
        let registry = Registry::new(reqwest_client());

        let mut episode_label = String::new();
        let title;

        let url = match self {
            Self::Url(url) => {
                // Opened by link, so there is no release to name. The source's
                // own name at least says something; a hostname is developer
                // output and does not belong on screen.
                title = host_of(url)
                    .and_then(|host| registry.for_host(&host).map(|e| e.name().to_owned()))
                    .unwrap_or_else(|| "AniRust".to_owned());
                url.clone()
            }
            Self::Episode {
                release_id,
                position,
            } => {
                let client = Client::new().context("creating the API client")?;

                // The screen names what is playing, so the release has to be
                // fetched even though the stream does not need it.
                let release = client.release(*release_id, false).await?;
                title = release.title().to_owned();

                let dubbers = client.dubbers(*release_id).await?;
                let dubber = dubbers.first().context("the release has no voice-overs")?;
                let sources = client.sources(*release_id, dubber.id).await?;
                let source = sources.first().context("the voice-over has no sources")?;
                let episodes = client
                    .episodes(*release_id, dubber.id, source.id, EpisodeSort::Ascending)
                    .await?;
                // The word "episode" is added on the Slint side, which owns
                // the translations.
                episode_label = format!("{position} - {}", dubber.name);

                episodes
                    .into_iter()
                    .find(|e| e.position == *position)
                    .with_context(|| format!("no episode {position}"))?
                    .url
            }
        };

        // Routing by host rather than by the API's `iframe` flag, which lies
        // for several hosts.
        let stream = if registry.supports(&url) {
            registry.resolve(&url).await?
        } else {
            ResolvedStream {
                variants: vec![anirust_extract::StreamVariant {
                    height: anirust_extract::UNKNOWN_HEIGHT,
                    kind: anirust_extract::StreamKind::classify(None, &url),
                    url,
                }],
                ..Default::default()
            }
        };

        Ok(Playback {
            stream,
            title,
            episode_label,
        })
    }
}

/// A resolved stream together with what the screen should call it.
struct Playback {
    stream: ResolvedStream,
    title: String,
    episode_label: String,
}

/// Host of a URL, used to find which extractor claims it.
fn host_of(url: &str) -> Option<String> {
    anirust_extract::host_of(url)
}

fn reqwest_client() -> reqwest::Client {
    reqwest::Client::builder()
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upscaling_is_reported_as_a_transformation() {
        assert_eq!(
            quality_label(Some((1280, 720)), Some((2560, 1440))),
            "720p → 1440p"
        );
    }

    #[test]
    fn rendering_at_the_source_size_reports_one_number() {
        assert_eq!(quality_label(Some((1280, 720)), Some((1280, 720))), "720p");
    }

    #[test]
    fn a_smaller_render_never_reads_as_an_upgrade() {
        assert_eq!(
            quality_label(Some((1920, 1080)), Some((1280, 720))),
            "1080p"
        );
    }

    #[test]
    fn nothing_loaded_shows_a_placeholder() {
        assert_eq!(quality_label(None, None), "—");
    }

    #[test]
    fn whole_speeds_lose_the_decimal() {
        assert_eq!(format_speed(1.0), "1x");
        assert_eq!(format_speed(2.0), "2x");
        assert_eq!(format_speed(1.5), "1.5x");
    }

    #[test]
    fn times_gain_an_hour_field_only_when_needed() {
        assert_eq!(format_time(Duration::from_secs(62)), "1:02");
        assert_eq!(format_time(Duration::from_secs(3_723)), "1:02:03");
    }

    #[test]
    fn progress_is_bounded_and_safe_without_a_duration() {
        assert_eq!(
            fraction_of(Duration::from_secs(30), Some(Duration::from_secs(60))),
            0.5
        );
        assert_eq!(
            fraction_of(Duration::from_secs(90), Some(Duration::from_secs(60))),
            1.0
        );
        assert_eq!(fraction_of(Duration::from_secs(30), None), 0.0);
    }
}
