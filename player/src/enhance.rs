// SPDX-License-Identifier: GPL-3.0-or-later

//! Making a saved episode better at leisure: the same networks the player
//! runs live, run over a file as slowly as they need to, into a new file.
//!
//! What playback cannot afford, this can: the cleaner RIFE network, frame
//! generation on the full-size picture, the neural upscale and frame
//! generation together. mpv decodes and runs the filters in its encoding mode
//! — the picture goes through exactly the script the player uses — and writes
//! the picture alone; the caller puts the episode's own sound and subtitles
//! back beside it, untouched.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use libmpv2::Mpv;

use crate::frames::{self, Enhancement, Networks};
use crate::{Error, Result};

/// The offline neural upscalers can double sources through 1080p.
pub const NEURAL_MAX_HEIGHT: u32 = 1080;

/// Offline processing presets, from minimum memory use to highest quality.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Economy,
    Balanced,
    High,
    Maximum,
}

impl Profile {
    pub const ALL: [Self; 4] = [Self::Economy, Self::Balanced, Self::High, Self::Maximum];

    pub fn streams(self) -> u8 {
        match self {
            Self::Economy => 1,
            Self::Balanced => 2,
            Self::High => 6,
            Self::Maximum => 8,
        }
    }
}

/// How the picture is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoder {
    /// HEVC on an NVIDIA card's encoder: fast, and at this quality setting
    /// hard to tell from the CPU's.
    Nvenc,
    /// HEVC on the CPU, everywhere: slower, the smallest file for its look.
    X265,
}

impl Encoder {
    fn name(self) -> &'static str {
        match self {
            Self::Nvenc => "hevc_nvenc",
            Self::X265 => "libx265",
        }
    }

    /// Quality-led settings: a constant quality near transparent, 10 bits so
    /// the gradients anime is full of do not band.
    fn options(self, profile: Profile) -> String {
        let (cq, crf, nvenc, cpu) = match profile {
            Profile::Economy => (24, 23, "p3", "fast"),
            Profile::Balanced => (21, 20, "p5", "medium"),
            Profile::High => (19, 18, "p7", "slow"),
            Profile::Maximum => (17, 16, "p7", "slow"),
        };
        match self {
            Self::Nvenc => {
                let multipass = match profile {
                    Profile::Economy | Profile::Balanced => "disabled",
                    Profile::High | Profile::Maximum => "fullres",
                };
                format!(
                    "preset={nvenc},tune=hq,rc=vbr,cq={cq},b=0,multipass={multipass},spatial-aq=1"
                )
            }
            Self::X265 => format!("crf={crf},preset={cpu}"),
        }
    }
}

/// Where a job has got to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Progress {
    /// Loading the networks — building a TensorRT engine, the first time for
    /// a size, takes minutes.
    Starting,
    /// This fraction of the episode is done.
    Working(f32),
}

/// One file to go over.
#[derive(Debug, Clone, Copy)]
pub struct Job<'a> {
    pub source: &'a Path,
    /// A Matroska file holding the picture alone.
    pub video_out: &'a Path,
    pub networks: Option<Networks<'a>>,
    pub enhancement: Enhancement,
    /// Output height for an explicit resize, after the neural filters.
    pub target_height: Option<u32>,
    pub encoder: Encoder,
    pub profile: Profile,
    pub upscaler: frames::Upscaler<'a>,
    /// A folder of the job's own for the script, so jobs and playback never
    /// share one.
    pub scratch: &'a Path,
    /// The screen's rate, for frames made to match it.
    pub display_fps: Option<f64>,
}

/// Reads the decoded height and the nominal source rate without loading models.
pub fn source_video(source: &Path) -> Result<(u32, Option<f64>)> {
    let mpv = Mpv::with_initializer(|init| {
        init.set_property("config", false)?;
        init.set_property("terminal", false)?;
        init.set_property("vo", "null")?;
        init.set_property("audio", "no")?;
        init.set_property("sid", "no")?;
        init.set_property("pause", true)?;
        init.set_property("idle", "yes")?;
        for option in ["osc", "ytdl", "load-scripts"] {
            let _ = init.set_property(option, false);
        }
        Ok(())
    })?;
    mpv.command("loadfile", &[&source.to_string_lossy()])?;
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        if let Ok(height) = mpv.get_property::<i64>("video-dec-params/h")
            && let Ok(height) = u32::try_from(height)
            && height > 0
        {
            let fps = mpv
                .get_property::<f64>("container-fps")
                .ok()
                .filter(|fps| fps.is_finite() && *fps > 0.0);
            return Ok((height, fps));
        }
        if let Some(event) = mpv.wait_event(0.1)
            && matches!(event?, libmpv2::events::Event::EndFile(_))
        {
            break;
        }
    }
    Err(Error::Filter(
        "could not read the source video dimensions".into(),
    ))
}

/// Writes `source`'s picture through `enhancement` to `video_out`, a
/// Matroska file holding the picture alone. Returns its nominal frame rate
/// for the remux when the rate is known.
///
/// Blocks until the episode is through, `cancel` is set, or something fails;
/// run it off the UI thread. `scratch` is a folder of the job's own for the
/// script, so jobs and playback never share one.
pub fn picture(
    job: &Job<'_>,
    cancel: &Arc<AtomicBool>,
    mut report: impl FnMut(Progress),
) -> Result<Option<f64>> {
    let Job {
        video_out,
        networks,
        enhancement,
        target_height,
        profile,
        upscaler,
        scratch,
        display_fps,
        ..
    } = *job;
    // libavfilter can do a plain resize even if VapourSynth is absent.
    if let Some(height) = target_height.filter(|_| enhancement.is_empty()) {
        let filter = format!("lavfi=[zscale=w=-2:h={height}:filter=spline36]");
        match encode_picture(job, &filter, None, cancel, &mut report) {
            Ok(()) if source_video(video_out).is_ok_and(|(actual, _)| actual == height) => {
                return Ok(source_video(job.source).ok().and_then(|(_, fps)| fps));
            }
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            _ => {
                let _ = std::fs::remove_file(video_out);
            }
        }
    }

    frames::point_at_bundled_vapoursynth();
    let io = |source| Error::Io {
        path: scratch.to_owned(),
        source,
    };
    std::fs::create_dir_all(scratch).map_err(io)?;
    if let Some(Networks::Mlrt(mlrt)) = networks {
        std::fs::create_dir_all(frames::engines_dir()).map_err(io)?;
        frames::copy_offline_models(&mlrt.dir, enhancement, upscaler).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Error::Filter(format!("missing model: {error}"))
            } else {
                io(error)
            }
        })?;
    }
    let script = scratch.join("enhance.vpy");
    let error_file = scratch.join("enhance-error.txt");
    let rate_file = scratch.join("enhance-rate.txt");
    let _ = std::fs::remove_file(&error_file);
    let _ = std::fs::remove_file(&rate_file);
    std::fs::write(
        &script,
        frames::script_with_options(
            networks,
            enhancement,
            display_fps,
            &error_file,
            upscaler,
            NEURAL_MAX_HEIGHT,
            Some(frames::OfflineScript {
                streams: profile.streams(),
                target_height,
                rate_file: &rate_file,
            }),
        ),
    )
    .map_err(io)?;

    let streams = profile.streams();
    let filter = format!(
        "vapoursynth=file={}:buffered-frames={}:concurrent-frames={streams}",
        frames::mpv_quoted(&script.to_string_lossy()),
        (streams * 2).max(8)
    );
    encode_picture(job, &filter, Some(&error_file), cancel, &mut report)?;
    if let Some(height) = target_height
        && source_video(video_out)?.0 != height
    {
        return Err(Error::Filter(format!(
            "encoded picture did not reach {height}p"
        )));
    }
    let rate = std::fs::read_to_string(&rate_file)
        .ok()
        .and_then(|rate| rate.parse().ok());
    Ok(rate)
}

fn encode_picture(
    job: &Job<'_>,
    filter: &str,
    error_file: Option<&Path>,
    cancel: &AtomicBool,
    report: &mut impl FnMut(Progress),
) -> Result<()> {
    let Job {
        source,
        video_out,
        encoder,
        profile,
        ..
    } = *job;
    let output = video_out.to_string_lossy().into_owned();
    let mpv = Mpv::with_initializer(|init| {
        init.set_property("config", false)?;
        init.set_property("terminal", false)?;
        init.set_property("input-default-bindings", false)?;
        // Decoded on the CPU: the filters want the frames in memory anyway,
        // and a file on disk is never short of time.
        init.set_property("hwdec", "no")?;
        init.set_property("audio", "no")?;
        init.set_property("sid", "no")?;
        init.set_property("vf", filter)?;
        init.set_property("o", output.as_str())?;
        init.set_property("of", "matroska")?;
        init.set_property("ovc", encoder.name())?;
        init.set_property("ovcopts", encoder.options(profile).as_str())?;
        init.set_property("idle", "yes")?;
        for option in ["osc", "ytdl", "load-scripts"] {
            let _ = init.set_property(option, false);
        }
        Ok(())
    })?;

    report(Progress::Starting);
    let path = source.to_string_lossy();
    mpv.command("loadfile", &[path.as_ref()])?;

    'encode: loop {
        std::thread::sleep(Duration::from_millis(250));
        if cancel.load(Ordering::Relaxed) {
            let _ = mpv.command("stop", &[]);
            drop(mpv);
            let _ = std::fs::remove_file(video_out);
            return Err(Error::Cancelled);
        }
        if let Some(note) = error_file
            .and_then(|path| std::fs::read_to_string(path).ok())
            .filter(|text| !text.trim().is_empty())
        {
            let _ = mpv.command("stop", &[]);
            return Err(Error::Filter(note));
        }

        let position = mpv.get_property::<f64>("time-pos").ok();
        let duration = mpv.get_property::<f64>("duration").ok();
        if let (Some(at), Some(of)) = (position, duration)
            && of > 0.0
        {
            report(Progress::Working((at / of).clamp(0.0, 1.0) as f32));
        }
        while let Some(event) = mpv.wait_event(0.0) {
            if let libmpv2::events::Event::EndFile(reason) = event? {
                if reason == libmpv2::mpv_end_file_reason::Eof {
                    break 'encode;
                }
                return Err(Error::Filter(format!(
                    "encoding ended unexpectedly ({reason})"
                )));
            }
        }
    }
    // Dropping mpv finishes and closes the file it wrote.
    drop(mpv);
    if let Some(note) = error_file
        .and_then(|path| std::fs::read_to_string(path).ok())
        .filter(|text| !text.trim().is_empty())
    {
        return Err(Error::Filter(note));
    }
    Ok(())
}

/// A folder for one job's scripts, under the cache.
#[must_use]
pub fn scratch_for(name: &str) -> PathBuf {
    frames::work_dir().join("enhance").join(name)
}
