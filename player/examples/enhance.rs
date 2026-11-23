// SPDX-License-Identifier: GPL-3.0-or-later

//! Runs a file through the networks at leisure, as the downloads page does.
//!
//! ```text
//! cargo run --release -p anirust-player --example enhance -- in.mkv out.mkv [--upscale] [--frames 60|x2] [--x265]
//! ```

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anirust_player::enhance::{self, Encoder, Progress};
use anirust_player::{
    Backend, Enhancement, FrameGeneration, Mlrt, Networks, RifeModel, TargetRate,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, output, rest @ ..] = args.as_slice() else {
        return Err("usage: enhance <in> <out> [--upscale] [--frames 60|x2] [--x265]".into());
    };
    let has = |flag: &str| rest.iter().any(|arg| arg == flag);
    let rate = rest
        .iter()
        .position(|arg| arg == "--frames")
        .and_then(|at| rest.get(at + 1))
        .map(|rate| {
            if rate == "x2" {
                TargetRate::Double
            } else {
                TargetRate::Sixty
            }
        });
    let mlrt = Mlrt::find(Backend::TensorRt).ok_or("no vs-mlrt for TensorRT")?;
    let enhancement = Enhancement {
        frames: rate.map(|rate| FrameGeneration {
            rate,
            model: RifeModel::Quality,
            max_height: 0,
        }),
        upscale: has("--upscale"),
    };
    let started = std::time::Instant::now();
    let mut last = -1;
    let scratch = enhance::scratch_for("example");
    enhance::picture(
        &enhance::Job {
            source: input.as_ref(),
            video_out: output.as_ref(),
            networks: Some(Networks::Mlrt(&mlrt)),
            enhancement,
            target_height: None,
            encoder: if has("--x265") {
                Encoder::X265
            } else {
                Encoder::Nvenc
            },
            profile: enhance::Profile::High,
            stream_limit: None,
            upscaler: anirust_player::frames::Upscaler::RealEsrgan,
            scratch: &scratch,
            display_fps: None,
        },
        &Arc::new(AtomicBool::new(false)),
        |progress| {
            let percent = match progress {
                Progress::Starting => 0,
                Progress::Working(done) => (done * 100.0) as i32,
            };
            if percent != last {
                last = percent;
                eprintln!("{percent}% at {:.0}s", started.elapsed().as_secs_f64());
            }
        },
    )?;
    eprintln!("done in {:.0}s", started.elapsed().as_secs_f64());
    Ok(())
}
