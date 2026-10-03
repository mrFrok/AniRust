// SPDX-License-Identifier: GPL-3.0-or-later

//! Plays a file headless with RIFE frame generation and reports the rates.
//!
//! ```text
//! ANIRUST_RIFE_DIR=/path/to/rife \
//!     cargo run -p anirust-player --example frames -- video.mkv [seconds] [fast|quality]
//! ```
//!
//! Needs a libmpv built with the VapourSynth filter (`LD_LIBRARY_PATH` at one
//! on Linux) and VapourSynth. Prints the source's rate and the rate leaving
//! the filters; equal rates mean the filter dropped out, and the script's
//! note says why.

use std::time::Duration;

use anirust_player::{
    FrameGeneration, MediaSource, Player, PlayerConfig, RifeInstall, RifeModel, frames,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let file = args
        .next()
        .ok_or("usage: frames <file> [seconds] [fast|quality]")?;
    let seconds: u64 = args.next().map_or(Ok(8), |s| s.parse())?;
    let model = match args.next().as_deref() {
        Some("quality") => RifeModel::Quality,
        _ => RifeModel::Fast,
    };

    let install = RifeInstall::find().ok_or("RIFE is not installed; set ANIRUST_RIFE_DIR")?;
    let player = Player::new(&PlayerConfig::headless())?;
    player.open(&MediaSource::new(file))?;
    player.set_frame_generation(Some((
        &install,
        FrameGeneration {
            model,
            ..FrameGeneration::default()
        },
    )))?;

    std::thread::sleep(Duration::from_secs(seconds));
    println!(
        "{:.2} fps in, {:.2} fps out",
        player.source_fps().unwrap_or_default(),
        player.output_fps().unwrap_or_default()
    );
    if let Some(error) = frames::last_error() {
        println!("frame generation failed:\n{error}");
    }
    player.stop()?;
    Ok(())
}
