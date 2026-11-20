// SPDX-License-Identifier: GPL-3.0-or-later

//! Making a saved episode better, at leisure: a child process of the program
//! runs the networks over the file and writes a new one beside it.
//!
//! A process of its own for the same reason as the machine test: what
//! TensorRT and VapourSynth take is not given back while the process lives,
//! and an hour of encoding should not hold gigabytes of the window's memory.
//! Stopping a job is ending the child.

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anirust_player::enhance::{self, Encoder, Progress};
use anirust_player::{
    Backend, Enhancement, FrameGeneration, Mlrt, Networks, RifeInstall, RifeModel, TargetRate,
};

use crate::gpus;
use anirust_player::enhance::Profile;
use anirust_player::frames::{Upscaler, light_upscale};

/// The flag the program is started with to run one job.
pub const FLAG: &str = "--enhance";

/// What to do to the picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    /// Real-ESRGAN doubling the picture. vs-mlrt only.
    pub upscale: bool,
    pub profile: Profile,
    /// Frame generation: 0 none, 1 ×2, 2 sixty, 3 a hundred and twenty, 4
    /// the screen's rate.
    pub rate: usize,
    /// The screen's rate in millihertz, for 4; 0 when not known.
    pub display_millihz: u32,
}

/// What this machine can do at all: the neural upscale, frame generation.
#[must_use]
pub fn available(engine: usize) -> (bool, bool) {
    let mlrt = backend(engine).and_then(Mlrt::for_offline).is_some();
    (mlrt, mlrt || RifeInstall::find().is_some())
}

fn backend(engine: usize) -> Option<Backend> {
    engine
        .checked_sub(1)
        .and_then(|at| Backend::ALL.get(at).copied())
}

/// Where the improved episode goes: beside the original, marked.
#[must_use]
pub fn destination_for(source: &Path) -> PathBuf {
    let stem = source
        .file_stem()
        .map_or_else(|| "episode".into(), |stem| stem.to_string_lossy());
    source.with_file_name(format!("{stem} [AniRust].mkv"))
}

/// How a job ended.
pub enum Outcome {
    Done(PathBuf),
    Unchanged,
    Stopped,
    Failed(String),
}

/// Runs one job in a child process, reporting the fraction done (`None`
/// while the networks load), until it ends or `stop` is set. Blocks; run it
/// off the UI thread.
pub fn run_apart(
    source: &Path,
    choice: Choice,
    engine: usize,
    stop: &Arc<AtomicBool>,
    report: &dyn Fn(Option<f32>),
) -> Outcome {
    run_to(
        source,
        &destination_for(source),
        choice,
        engine,
        stop,
        report,
    )
}

fn run_to(
    source: &Path,
    destination: &Path,
    choice: Choice,
    engine: usize,
    stop: &AtomicBool,
    report: &dyn Fn(Option<f32>),
) -> Outcome {
    let Ok(exe) = std::env::current_exe() else {
        return Outcome::Failed("no path to the program".into());
    };
    let models: &[&str] = if choice.upscale {
        &["esrgan", "light", "off"]
    } else {
        &["off"]
    };
    for &model in models {
        if stop.load(Ordering::Relaxed) {
            return Outcome::Stopped;
        }
        if model == "off" && choice.rate == 0 {
            return Outcome::Unchanged;
        }
        let name = format!("job-{}", std::process::id());
        let staged = destination.with_extension(format!("{}.partial.mkv", std::process::id()));
        let child = std::process::Command::new(&exe)
            .arg(FLAG)
            .arg(source)
            .arg(&staged)
            .arg(u8::from(choice.upscale).to_string())
            .arg(choice.rate.to_string())
            .arg(engine.to_string())
            .arg(choice.display_millihz.to_string())
            .arg((choice.profile as usize).to_string())
            .arg(model)
            .env("ANIRUST_SCRIPT_DIR", enhance::scratch_for(&name))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(error) => return Outcome::Failed(error.to_string()),
        };

        // The child's lines are read on a thread of their own, so a stop is seen
        // at once rather than at the next line.
        let (send, receive) = std::sync::mpsc::channel::<String>();
        if let Some(out) = child.stdout.take() {
            std::thread::spawn(move || {
                for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                    if send.send(line).is_err() {
                        break;
                    }
                }
            });
        }

        let mut error = None;
        let mut unchanged = false;
        loop {
            if stop.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&staged);
                let _ = std::fs::remove_file(picture_path(&staged));
                return Outcome::Stopped;
            }
            match receive.recv_timeout(std::time::Duration::from_millis(200)) {
                Ok(line) => {
                    if line == "starting" {
                        report(None);
                    } else if line == "unchanged" {
                        unchanged = true;
                    } else if let Some(done) = line.strip_prefix("p ") {
                        report(done.parse().ok());
                    } else if let Some(message) = line.strip_prefix("error ") {
                        error = Some(message.to_owned());
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        let status = child.wait();
        let _ = std::fs::remove_file(picture_path(&staged));
        if status.as_ref().is_ok_and(|s| s.success()) && error.is_none() {
            if unchanged {
                let _ = std::fs::remove_file(&staged);
                return Outcome::Unchanged;
            }
            match std::fs::rename(&staged, destination) {
                Ok(()) => return Outcome::Done(destination.to_owned()),
                Err(e) => error = Some(e.to_string()),
            }
        }
        let _ = std::fs::remove_file(&staged);
        if model != "off"
            && status
                .as_ref()
                .is_ok_and(|s| !s.success() && s.code() != Some(1))
        {
            tracing::warn!(
                model,
                ?error,
                "retrying offline processing with the next model"
            );
            continue;
        }
        return match status {
            Ok(status) => Outcome::Failed(error.unwrap_or_else(|| status.to_string())),
            Err(failure) => Outcome::Failed(failure.to_string()),
        };
    }
    Outcome::Unchanged
}

/// The picture alone, before the sound is put back beside it.
fn picture_path(destination: &Path) -> PathBuf {
    destination.with_extension("picture.mkv")
}

/// The child's side. Prints `starting`, then `p <fraction>` as it goes, and
/// `error <message>` when it fails.
pub fn child_main(args: &[String]) {
    let [source, destination, upscale, rate, engine, rest @ ..] = args else {
        println!("error usage: {FLAG} <source> <destination> <upscale> <rate> <engine> [millihz]");
        std::process::exit(2);
    };
    let choice = Choice {
        upscale: upscale == "1",
        profile: rest
            .get(1)
            .and_then(|s| s.parse::<usize>().ok())
            .and_then(|i| Profile::ALL.get(i).copied())
            .unwrap_or(Profile::High),
        rate: rate.parse().unwrap_or(0),
        display_millihz: rest.first().and_then(|hz| hz.parse().ok()).unwrap_or(0),
    };
    let engine = engine.parse().unwrap_or(0);
    let Some(model) = rest.get(2) else {
        match run_to(
            source.as_ref(),
            destination.as_ref(),
            choice,
            engine,
            &AtomicBool::new(false),
            &|progress| {
                if let Some(done) = progress {
                    println!("p {done:.3}");
                } else {
                    println!("starting");
                }
            },
        ) {
            Outcome::Done(_) => (),
            Outcome::Unchanged => println!("unchanged"),
            Outcome::Stopped => std::process::exit(1),
            Outcome::Failed(error) => {
                println!("error {error}");
                std::process::exit(1);
            }
        }
        return;
    };
    if let Err(message) = child_job(
        Path::new(source),
        Path::new(destination),
        choice,
        engine,
        model,
    ) {
        println!("error {}", message.replace('\n', " "));
        std::process::exit(1);
    }
}

fn child_job(
    source: &Path,
    destination: &Path,
    choice: Choice,
    engine: usize,
    model: &str,
) -> Result<(), String> {
    let mlrt = backend(engine).and_then(Mlrt::for_offline);
    let rife = RifeInstall::find();
    let networks = match (&mlrt, &rife) {
        (Some(mlrt), _) => Networks::Mlrt(mlrt),
        (None, Some(rife)) => Networks::Vulkan(rife),
        (None, None) if choice.rate == 0 => {
            println!("unchanged");
            return Ok(());
        }
        (None, None) => return Err("the networks are not installed".into()),
    };
    // Nothing to hurry for: the cleaner network, on the full picture.
    let enhancement = Enhancement {
        frames: match choice.rate {
            0 => None,
            rate => Some(FrameGeneration {
                rate: match rate {
                    1 => TargetRate::Double,
                    3 => TargetRate::HundredTwenty,
                    4 if choice.display_millihz > 0 => TargetRate::Display,
                    _ => TargetRate::Sixty,
                },
                model: if choice.profile == Profile::Economy {
                    RifeModel::Fast
                } else {
                    RifeModel::Quality
                },
                max_height: 0,
            }),
        },
        upscale: model != "off" && mlrt.is_some(),
    };
    let nvidia = gpus::vendors().is_some_and(|vendors| vendors.contains(&0x10de));
    let picture = picture_path(destination);
    let scratch = anirust_player::frames::script_dir();
    let never = Arc::new(AtomicBool::new(false));
    let light;
    let run = |encoder, upscaler| {
        let mut last = -1;
        enhance::picture(
            &enhance::Job {
                source,
                video_out: &picture,
                networks,
                enhancement,
                encoder,
                upscaler,
                profile: choice.profile,
                scratch: &scratch,
                display_fps: (choice.display_millihz > 0)
                    .then(|| f64::from(choice.display_millihz) / 1000.0),
            },
            &never,
            |progress| match progress {
                Progress::Starting => println!("starting"),
                Progress::Working(done) => {
                    // A line a thousandth, not one per poll.
                    let step = (done * 1000.0) as i32;
                    if step != last {
                        last = step;
                        println!("p {done:.3}");
                    }
                }
            },
        )
    };
    // NVENC first on an NVIDIA card; x265 everywhere else, and where the
    // card's encoder would not open.
    let encode = |upscaler| {
        if nvidia {
            match run(Encoder::Nvenc, upscaler) {
                Err(anirust_player::Error::Mpv(error)) => {
                    tracing::warn!(%error, "retrying with CPU encoder")
                }
                result => return result,
            }
        }
        run(Encoder::X265, upscaler)
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    if !enhancement.upscale && choice.rate == 0 {
        println!("unchanged");
        return Ok(());
    }
    let upscaler = if model == "light" && enhancement.upscale {
        light = match light_upscale::installed() {
            Some(path) => path,
            None => runtime
                .block_on(crate::runtimes::fetch_light_upscale(crate::reqwest_client()))
                .unwrap_or_else(|error| model_failed(&error)),
        };
        Upscaler::Light(&light)
    } else {
        Upscaler::RealEsrgan
    };
    if let Err(error) = encode(upscaler) {
        if enhancement.upscale && matches!(error, anirust_player::Error::Filter(_)) {
            model_failed(&error.to_string());
        }
        return Err(error.to_string());
    }

    let ffmpeg = anirust_download::Ffmpeg::find().map_err(|error| error.to_string())?;
    let merged = runtime.block_on(ffmpeg.merge(&picture, source, destination));
    let _ = std::fs::remove_file(&picture);
    merged.map_err(|error| error.to_string())
}

fn model_failed(error: &str) -> ! {
    println!("error {}", error.replace('\n', " "));
    std::process::exit(42);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_improved_episode_sits_beside_the_original() {
        assert_eq!(
            destination_for(Path::new("/v/AniRust/Show - 03 [AniLibria].mp4")),
            PathBuf::from("/v/AniRust/Show - 03 [AniLibria] [AniRust].mkv")
        );
    }
}
