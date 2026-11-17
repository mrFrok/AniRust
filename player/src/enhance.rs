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
    fn options(self) -> &'static str {
        match self {
            Self::Nvenc => "preset=p7,tune=hq,rc=vbr,cq=19,b=0,multipass=fullres,spatial-aq=1",
            Self::X265 => "crf=18,preset=slow",
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
    pub networks: Networks<'a>,
    pub enhancement: Enhancement,
    pub encoder: Encoder,
    /// A folder of the job's own for the script, so jobs and playback never
    /// share one.
    pub scratch: &'a Path,
}

/// Writes `source`'s picture through `enhancement` to `video_out`, a
/// Matroska file holding the picture alone.
///
/// Blocks until the episode is through, `cancel` is set, or something fails;
/// run it off the UI thread. `scratch` is a folder of the job's own for the
/// script, so jobs and playback never share one.
pub fn picture(
    job: &Job<'_>,
    cancel: &Arc<AtomicBool>,
    mut report: impl FnMut(Progress),
) -> Result<()> {
    let Job {
        source,
        video_out,
        networks,
        enhancement,
        encoder,
        scratch,
    } = *job;
    frames::point_at_bundled_vapoursynth();
    let io = |source| Error::Io {
        path: scratch.to_owned(),
        source,
    };
    std::fs::create_dir_all(scratch).map_err(io)?;
    std::fs::create_dir_all(frames::engines_dir()).map_err(io)?;
    if let Networks::Mlrt(mlrt) = networks {
        frames::copy_models(&mlrt.dir).map_err(io)?;
    }
    let script = scratch.join("enhance.vpy");
    let error_file = scratch.join("enhance-error.txt");
    let _ = std::fs::remove_file(&error_file);
    std::fs::write(
        &script,
        frames::script_with(
            networks,
            enhancement,
            None,
            &error_file,
            frames::Upscaler::RealEsrgan,
            1080,
        ),
    )
    .map_err(io)?;

    let filter = format!(
        "vapoursynth=file={}:buffered-frames=8:concurrent-frames=4",
        frames::mpv_quoted(&script.to_string_lossy())
    );
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
        init.set_property("vf", filter.as_str())?;
        init.set_property("o", output.as_str())?;
        init.set_property("of", "matroska")?;
        init.set_property("ovc", encoder.name())?;
        init.set_property("ovcopts", encoder.options())?;
        init.set_property("idle", "yes")?;
        for option in ["osc", "ytdl", "load-scripts"] {
            let _ = init.set_property(option, false);
        }
        Ok(())
    })?;

    report(Progress::Starting);
    let path = source.to_string_lossy();
    mpv.command("loadfile", &[path.as_ref()])?;

    let mut started = false;
    loop {
        std::thread::sleep(Duration::from_millis(250));
        if cancel.load(Ordering::Relaxed) {
            let _ = mpv.command("stop", &[]);
            drop(mpv);
            let _ = std::fs::remove_file(video_out);
            return Err(Error::Cancelled);
        }
        if let Some(note) = std::fs::read_to_string(&error_file)
            .ok()
            .filter(|text| !text.trim().is_empty())
        {
            let _ = mpv.command("stop", &[]);
            return Err(Error::Filter(note));
        }
        let idle = mpv.get_property::<bool>("idle-active").unwrap_or(false);
        let position = mpv.get_property::<f64>("time-pos").ok();
        let duration = mpv.get_property::<f64>("duration").ok();
        if let (Some(at), Some(of)) = (position, duration)
            && of > 0.0
        {
            started = true;
            report(Progress::Working((at / of).clamp(0.0, 1.0) as f32));
        }
        // Idle again once the file is through: mpv has closed the output by
        // the time it says so.
        if idle && started {
            break;
        }
    }
    // Dropping mpv finishes and closes the file it wrote.
    drop(mpv);
    Ok(())
}

/// A folder for one job's scripts, under the cache.
#[must_use]
pub fn scratch_for(name: &str) -> PathBuf {
    frames::work_dir().join("enhance").join(name)
}
