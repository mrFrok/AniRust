// SPDX-License-Identifier: GPL-3.0-or-later

//! How fast this machine runs the neural filters, measured rather than
//! guessed from the name of its GPU.
//!
//! A headless player plays a generated test picture with nothing to wait for
//! — mpv's `untimed` — so it goes exactly as fast as decoding and the filters
//! allow. How many seconds of picture it gets through in one second of
//! running is the margin: above one, the machine keeps up with an episode;
//! below, it would drop frames.
//!
//! The picture is 720p at 23.976 frames a second, the size and rate the
//! filters meet most: a 1080p source is brought down to 720 before RIFE sees
//! it, and the upscale passes taller ones by.
//!
//! The filter writes its script to the same place the player does, so a
//! measurement must not run while an episode plays with the filters on.

use std::time::{Duration, Instant};

use crate::frames::{self, Enhancement, Networks};
use crate::{MediaSource, Player, PlayerConfig, Result};

/// The test picture: lavfi's moving pattern, the size and rate most episodes
/// reach the filters at.
const SOURCE: &str = "av://lavfi:testsrc2=size=1280x720:rate=24000/1001";

/// What one run showed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measurement {
    /// Seconds of picture made per second of running. Real time is 1.
    pub speed: f64,
}

impl Measurement {
    /// Whether an episode would play through these filters without dropping
    /// frames, with a margin for the decoder, the shaders after them and
    /// whatever else the machine is doing.
    #[must_use]
    pub fn keeps_up(self) -> bool {
        self.speed >= 1.3
    }
}

/// Runs the filters for `seconds` once the first filtered frame is out, and
/// says how fast they went; `None` when they never started within `warm_up`
/// — an engine still building, a runtime that will not load — or dropped
/// out, in which case [`frames::last_error`] says why.
///
/// Blocks for up to `warm_up + seconds`; call it off the UI thread.
pub fn measure(
    networks: Networks<'_>,
    enhancement: Enhancement,
    warm_up: Duration,
    seconds: Duration,
) -> Result<Option<Measurement>> {
    let player = Player::new(&PlayerConfig {
        cache_secs: 1,
        ..PlayerConfig::headless()
    })?;
    player.mpv().set_property("untimed", true)?;
    player.mpv().set_property("audio", "no")?;
    player.open(&MediaSource::new(SOURCE))?;

    // The filter goes on once the picture is there, as the player does: put
    // on before the file opens, mpv starts without it.
    let opened = Instant::now() + Duration::from_secs(10);
    while player.video_size().is_none() {
        if Instant::now() >= opened {
            let _ = player.stop();
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    player.set_enhancement(Some((networks, enhancement)))?;
    let from = player.position().unwrap_or_default();

    // Started once a second of picture is through: by then the engine has
    // loaded and the first frames' extra cost is behind.
    let deadline = Instant::now() + warm_up;
    let start = loop {
        if let Some(at) = player
            .position()
            .filter(|at| *at >= from + Duration::from_secs(1))
        {
            break Some(at);
        }
        if Instant::now() >= deadline || frames::last_error().is_some() {
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let Some(start) = start else {
        let _ = player.stop();
        return Ok(None);
    };

    let began = Instant::now();
    std::thread::sleep(seconds);
    let made = player.position().unwrap_or(start).saturating_sub(start);
    let elapsed = began.elapsed();

    // A filter that fails is taken off and the picture goes through as it
    // is: fast, and not what was asked for. (mpv's own count of frames out
    // of the filters is no help here: unpaced, it reads the source's rate.)
    let still_on = player
        .mpv()
        .get_property::<String>("vf")
        .is_ok_and(|vf| vf.contains("vapoursynth"));
    let upscaled = matches!(
        (player.video_size(), player.filtered_size()),
        (Some((_, tall)), Some((_, out))) if out > tall
    );
    let _ = player.stop();
    if !still_on || (enhancement.upscale && !upscaled) || frames::last_error().is_some() {
        return Ok(None);
    }

    Ok(Some(Measurement {
        speed: made.as_secs_f64() / elapsed.as_secs_f64().max(0.001),
    }))
}
