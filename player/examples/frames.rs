// SPDX-License-Identifier: GPL-3.0-or-later

//! Plays a file headless through the neural filters and reports the rates.
//!
//! ```text
//! cargo run -p anirust-player --example frames -- video.mkv [options]
//!
//!   --seconds N        how long to play (8)
//!   --tensorrt         vs-mlrt on TensorRT (NVIDIA RTX) instead of Vulkan
//!   --openvino         vs-mlrt on OpenVINO (Intel)
//!   --migraphx         vs-mlrt on MIGraphX (AMD)
//!   --quality          the cleaner RIFE network
//!   --display FPS      aim at a screen of that rate instead of 60
//!   --no-frames        no frame generation
//!   --upscale          Real-ESRGAN first (vs-mlrt only)
//! ```
//!
//! Needs a libmpv built with the VapourSynth filter (`LD_LIBRARY_PATH` at one
//! on Linux) and VapourSynth; for Vulkan the RIFE plugin (`ANIRUST_RIFE_DIR`),
//! for vs-mlrt our mlrt folder (`ANIRUST_MLRT_DIR`) and the vendor's runtime
//! (`ANIRUST_TRT_RTX_DIR`, `ANIRUST_OPENVINO_DIR`, `ROCM_PATH`). Equal rates in
//! and out mean the filter dropped out, and the script's note says why.

use std::time::Duration;

use anirust_player::{
    Backend, Enhancement, FrameGeneration, MediaSource, Mlrt, Networks, Player, PlayerConfig,
    RifeInstall, RifeModel, TargetRate, frames,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let file = args.next().ok_or("usage: frames <file> [options]")?;
    let mut seconds = 8;
    let mut backend: Option<Backend> = None;
    let mut model = RifeModel::Fast;
    let mut display: Option<f64> = None;
    let mut generate = true;
    let mut upscale = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seconds" => seconds = args.next().ok_or("--seconds N")?.parse()?,
            "--tensorrt" => backend = Some(Backend::TensorRt),
            "--openvino" => backend = Some(Backend::OpenVino),
            "--migraphx" => backend = Some(Backend::MigraphX),
            "--quality" => model = RifeModel::Quality,
            "--display" => display = Some(args.next().ok_or("--display FPS")?.parse()?),
            "--no-frames" => generate = false,
            "--upscale" => upscale = true,
            other => return Err(format!("unknown option {other}").into()),
        }
    }

    let enhancement = Enhancement {
        frames: generate.then_some(FrameGeneration {
            model,
            rate: if display.is_some() {
                TargetRate::Display
            } else {
                TargetRate::Sixty
            },
            max_height: 720,
        }),
        upscale,
    };

    let vulkan;
    let mlrt;
    let networks = if let Some(backend) = backend {
        mlrt = Mlrt::find(backend)
            .ok_or("vs-mlrt is not there for that engine; see the variables above")?;
        Networks::Mlrt(&mlrt)
    } else {
        vulkan = RifeInstall::find().ok_or("RIFE is not installed; set ANIRUST_RIFE_DIR")?;
        Networks::Vulkan(&vulkan)
    };

    let player = Player::new(&PlayerConfig::headless())?;
    player.open(&MediaSource::new(file))?;
    player.set_display_fps(display)?;
    player.set_enhancement(Some((networks, enhancement)))?;

    std::thread::sleep(Duration::from_secs(seconds));
    println!(
        "{:.2} fps in, {:.2} fps out, {:?} → {:?}, {} frames dropped",
        player.source_fps().unwrap_or_default(),
        player.output_fps().unwrap_or_default(),
        player.video_size(),
        player.filtered_size(),
        player.dropped_frames()
    );
    if let Some(error) = frames::last_error() {
        println!("the filter failed:\n{error}");
    }
    player.stop()?;
    Ok(())
}
