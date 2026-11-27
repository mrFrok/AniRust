// SPDX-License-Identifier: GPL-3.0-or-later

//! Measures the neural filters on this machine, as the first-run test does.
//!
//! ```text
//! cargo run --release -p anirust-player --example bench -- [--tensorrt] [--upscale] [--double]
//! ```

use std::time::Duration;

use anirust_player::{
    Backend, Enhancement, FrameGeneration, Mlrt, Networks, RifeInstall, TargetRate, bench, frames,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|arg| arg == flag);
    let enhancement = Enhancement {
        frames: (!has("--no-frames")).then(|| FrameGeneration {
            rate: if has("--double") {
                TargetRate::Double
            } else {
                TargetRate::Sixty
            },
            ..FrameGeneration::default()
        }),
        upscale: has("--upscale"),
        ..Enhancement::default()
    };
    let rife;
    let mlrt;
    let networks = if has("--tensorrt") {
        mlrt = Mlrt::find(Backend::TensorRt).ok_or("no vs-mlrt for TensorRT")?;
        Networks::Mlrt(&mlrt)
    } else {
        rife = RifeInstall::find().ok_or("no RIFE")?;
        Networks::Vulkan(&rife)
    };
    let started = std::time::Instant::now();
    let result = bench::measure(
        networks,
        enhancement,
        Duration::from_secs(300),
        Duration::from_secs(6),
    )?;
    println!(
        "{result:?} keeps up: {:?} in {:.1}s",
        result.map(bench::Measurement::keeps_up),
        started.elapsed().as_secs_f64()
    );
    if let Some(error) = frames::last_error() {
        println!("{error}");
    }
    Ok(())
}
