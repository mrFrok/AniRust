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

use anirust_player::enhance::{self, Encoder, Profile, Progress};
use anirust_player::frames::{Upscaler, light_upscale};
use anirust_player::{
    Backend, Enhancement, FrameGeneration, Mlrt, Networks, RifeInstall, TargetRate,
};

use crate::gpus;

/// The flag the program is started with to run one job.
pub const FLAG: &str = "--enhance";

const MODEL_FAILED: i32 = 42;
const RETRY_CONCURRENCY: i32 = 43;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Original,
    Double,
    P1440,
    P2160,
}

impl Resolution {
    pub const ALL: [Self; 4] = [Self::Original, Self::Double, Self::P1440, Self::P2160];

    fn height(self) -> Option<u32> {
        match self {
            Self::Original | Self::Double => None,
            Self::P1440 => Some(1440),
            Self::P2160 => Some(2160),
        }
    }

    fn argument(self) -> &'static str {
        match self {
            Self::Original => "0",
            Self::Double => "1",
            Self::P1440 => "1440",
            Self::P2160 => "2160",
        }
    }

    pub fn marker(self) -> &'static str {
        match self {
            Self::Original | Self::Double => " [AniRust]",
            Self::P1440 => " [AniRust 1440p]",
            Self::P2160 => " [AniRust 2160p]",
        }
    }
}

/// What to do to the picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    pub resolution: Resolution,
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
pub fn destination_for(source: &Path, resolution: Resolution) -> PathBuf {
    let mut name = source
        .file_stem()
        .unwrap_or("episode".as_ref())
        .to_os_string();
    name.push(resolution.marker());
    name.push(".mkv");
    source.with_file_name(name)
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
        &destination_for(source, choice.resolution),
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
    let mlrt = backend(engine).and_then(Mlrt::for_offline);
    let models: &[&str] = match (
        choice.resolution,
        choice.profile,
        mlrt.as_ref().map(|mlrt| mlrt.backend),
    ) {
        (Resolution::Original, _, _) | (_, _, None) => &["off"],
        (_, Profile::Standard, Some(Backend::TensorRt)) => &["light", "balanced", "esrgan", "off"],
        (_, Profile::High | Profile::Maximum, Some(Backend::TensorRt)) => {
            &["balanced", "esrgan", "light", "off"]
        }
        (_, _, Some(_)) => &["esrgan", "off"],
    };
    let cpus = std::thread::available_parallelism().map_or(1, |count| count.get());
    let streams = mlrt.map_or(1, |mlrt| enhance::offline_streams(mlrt.backend));
    let mut attempts = models
        .iter()
        .flat_map(|&model| {
            std::iter::once((model, None)).chain([2, 1].into_iter().filter_map(move |limit| {
                ((model != "off" || choice.rate > 0) && (streams > limit || cpus > limit * 2))
                    .then_some((model, Some(limit)))
            }))
        })
        .peekable();
    while let Some((model, limit)) = attempts.next() {
        if stop.load(Ordering::Relaxed) {
            return Outcome::Stopped;
        }
        if model == "off" && choice.rate == 0 && choice.resolution.height().is_none() {
            return Outcome::Unchanged;
        }
        let name = format!("job-{}", std::process::id());
        let staged = destination.with_extension(format!("{}.partial.mkv", std::process::id()));
        let picture = picture_path(&staged);
        let child = std::process::Command::new(&exe)
            .arg(FLAG)
            .arg(source)
            .arg(&staged)
            .arg(choice.resolution.argument())
            .arg(choice.rate.to_string())
            .arg(engine.to_string())
            .arg(choice.display_millihz.to_string())
            .arg((choice.profile as usize).to_string())
            .arg(model)
            .args(limit.map(|streams| streams.to_string()))
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
        let mut fps = None;
        loop {
            if stop.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&staged);
                let _ = std::fs::remove_file(&picture);
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
                    } else if let Some(rate) = line.strip_prefix("fps ") {
                        fps = rate.parse().ok();
                    } else if let Some(message) = line.strip_prefix("error ") {
                        error = Some(message.to_owned());
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        let status = child.wait();
        if stop.load(Ordering::Relaxed) {
            let _ = std::fs::remove_file(&picture);
            return Outcome::Stopped;
        }
        if status.as_ref().is_ok_and(|s| s.success()) && error.is_none() {
            if unchanged {
                return Outcome::Unchanged;
            }
            let merged = merge(&picture, source, &staged, fps, stop);
            let _ = std::fs::remove_file(&picture);
            if stop.load(Ordering::Relaxed) {
                let _ = std::fs::remove_file(&staged);
                return Outcome::Stopped;
            }
            match merged.and_then(|()| {
                std::fs::rename(&staged, destination).map_err(|error| error.to_string())
            }) {
                Ok(()) => return Outcome::Done(destination.to_owned()),
                Err(failure) => error = Some(failure),
            }
        }
        let _ = std::fs::remove_file(&picture);
        let _ = std::fs::remove_file(&staged);
        let retry_concurrency = status
            .as_ref()
            .is_ok_and(|status| matches!(status.code(), None | Some(RETRY_CONCURRENCY)));
        if retry_concurrency
            && let Some(&(next_model, Some(limit))) = attempts.peek()
            && next_model == model
        {
            tracing::warn!(model, ?error, limit, "retrying at lower concurrency");
            continue;
        }
        // A missing model cannot be fixed by allocating fewer streams.
        while attempts
            .next_if(|(next_model, _)| *next_model == model)
            .is_some()
        {}
        if model != "off"
            && status.as_ref().is_ok_and(|status| {
                matches!(status.code(), None | Some(MODEL_FAILED | RETRY_CONCURRENCY))
            })
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

fn merge(
    picture: &Path,
    source: &Path,
    destination: &Path,
    fps: Option<f64>,
    stop: &AtomicBool,
) -> Result<(), String> {
    let ffmpeg = anirust_download::Ffmpeg::find().map_err(|error| error.to_string())?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    runtime
        .block_on(ffmpeg.merge(picture, source, destination, fps, stop))
        .map_err(|error| error.to_string())
}

/// The child's side. Prints `starting`, then `p <fraction>` as it goes, and
/// `error <message>` when it fails.
pub fn child_main(args: &[String]) {
    let [source, destination, resolution, rate, engine, rest @ ..] = args else {
        println!(
            "error usage: {FLAG} <source> <destination> <0|1|1440|2160> <rate> <engine> [millihz]"
        );
        std::process::exit(2);
    };
    let Some(resolution) = Resolution::ALL
        .into_iter()
        .find(|mode| mode.argument() == resolution)
    else {
        println!("error invalid enhancement resolution");
        std::process::exit(2);
    };
    let choice = Choice {
        resolution,
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
        rest.get(3).and_then(|streams| streams.parse().ok()),
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
    stream_limit: Option<usize>,
) -> Result<(), String> {
    let (target_height, try_upscaler) = if choice.resolution == Resolution::Original {
        (None, false)
    } else {
        let (height, _) = enhance::source_video(source).map_err(|error| error.to_string())?;
        let target_height = choice.resolution.height().filter(|&target| height < target);
        let enlarge = choice.resolution == Resolution::Double || target_height.is_some();
        (
            target_height,
            enlarge && height <= enhance::NEURAL_MAX_HEIGHT && model != "off",
        )
    };
    if !try_upscaler && target_height.is_none() && choice.rate == 0 {
        println!("unchanged");
        return Ok(());
    }
    let mlrt = backend(engine).and_then(Mlrt::for_offline);
    if try_upscaler && mlrt.is_none() {
        model_failed("no neural upscaling backend is installed");
    }
    let rife = (choice.rate > 0 && mlrt.is_none())
        .then(RifeInstall::find)
        .flatten();
    let networks = if try_upscaler || choice.rate > 0 {
        Some(match (&mlrt, &rife) {
            (Some(mlrt), _) => Networks::Mlrt(mlrt),
            (None, Some(rife)) => Networks::Vulkan(rife),
            (None, None) => return Err("the frame generation networks are not installed".into()),
        })
    } else {
        None
    };
    // Offline frame generation works on the full input picture.
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
                model: choice.profile.rife_model(),
                max_height: 0,
            }),
        },
        upscale: try_upscaler,
        ..Enhancement::default()
    };
    let nvidia = gpus::vendors().is_some_and(|vendors| vendors.contains(&0x10de));
    let picture = picture_path(destination);
    let scratch = anirust_player::frames::script_dir();
    let never = Arc::new(AtomicBool::new(false));
    let light;
    let balanced;
    let run = |encoder, upscaler| {
        let mut last = -1;
        enhance::picture(
            &enhance::Job {
                source,
                video_out: &picture,
                networks,
                enhancement,
                target_height,
                encoder,
                upscaler,
                profile: choice.profile,
                stream_limit,
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
    let upscaler = match model {
        "light" if enhancement.upscale => {
            light = ensure_network(light_upscale::installed)?;
            Upscaler::Light(&light)
        }
        "balanced" if enhancement.upscale => {
            balanced = ensure_network(light_upscale::balanced_installed)?;
            Upscaler::Balanced(&balanced)
        }
        _ => Upscaler::RealEsrgan,
    };
    match encode(upscaler) {
        Ok(Some(fps)) => println!("fps {fps}"),
        Ok(None) => {}
        Err(anirust_player::Error::Filter(message)) if !enhancement.is_empty() => {
            job_failed(&message, RETRY_CONCURRENCY);
        }
        Err(error) => return Err(error.to_string()),
    }
    Ok(())
}

fn model_failed(error: &str) -> ! {
    job_failed(error, MODEL_FAILED);
}

/// A fetched network, downloaded now if it is not there yet.
fn ensure_network(
    installed: fn() -> Option<std::path::PathBuf>,
) -> Result<std::path::PathBuf, String> {
    if let Some(path) = installed() {
        return Ok(path);
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    runtime
        .block_on(crate::runtimes::fetch_light_upscale(crate::reqwest_client()))
        .unwrap_or_else(|error| model_failed(&error));
    installed().ok_or_else(|| "the network was not where it should be".to_owned())
}

fn job_failed(error: &str, code: i32) -> ! {
    println!("error {}", error.replace('\n', " "));
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_improved_episode_sits_beside_the_original() {
        assert_eq!(
            destination_for(
                Path::new("/v/AniRust/Show - 03 [AniLibria].mp4"),
                Resolution::Double
            ),
            PathBuf::from("/v/AniRust/Show - 03 [AniLibria] [AniRust].mkv")
        );
    }
}
