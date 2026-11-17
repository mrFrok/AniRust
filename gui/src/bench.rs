// SPDX-License-Identifier: GPL-3.0-or-later

//! The first-run test: which picture settings this machine runs without
//! dropping frames, measured with the player's own filters and offered as
//! one "Apply".
//!
//! Smoothness is measured — RIFE at 60, then at ×2. The neural upscale is
//! measured on top of the smoothness found, where vs-mlrt and the vendor's
//! runtime are there. Anime4K's shaders cannot be run without a window, so
//! their strength follows the GPU: a discrete card gets the strongest, an
//! integrated one the lighter.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anirust_player::{
    Backend, Enhancement, FrameGeneration, Mlrt, Networks, RifeInstall, TargetRate, bench,
};

use slint::ComponentHandle;

use crate::{MainWindow, gpus, preferences};

/// The menus' positions for what the test found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Found {
    /// 0 Vulkan, then vs-mlrt on each of [`Backend::ALL`].
    pub engine: usize,
    /// Frame generation as the menu has it: 0 off, 1 ×2, 2 sixty.
    pub rate: usize,
    pub neural: bool,
    /// The picture panel's three steps: 0 off, 1 sharper, 2 max.
    pub sharpness: i32,
}

/// PCI vendor ids of the makers of discrete cards: NVIDIA, AMD.
const DISCRETE: [u32; 2] = [0x10de, 0x1002];

/// How strong the shaders can be on these GPUs: the strongest on a discrete
/// card, the lighter set on anything else.
fn shader_sharpness(vendors: Option<&[u32]>) -> i32 {
    let discrete = vendors.is_some_and(|vendors| vendors.iter().any(|v| DISCRETE.contains(v)));
    if discrete { 2 } else { 1 }
}

/// Wires the card and the settings button, and offers the test on the first
/// start.
pub fn wire(window: &MainWindow, held: &Rc<Cell<preferences::Preferences>>, engine: usize) {
    let found: Arc<Mutex<Found>> = Arc::default();

    if !held.get().machine_tested {
        window.set_bench_state("offer".into());
    }

    let mark_tested = {
        let held = Rc::clone(held);
        move || {
            let mut preferences = held.get();
            if !preferences.machine_tested {
                preferences.machine_tested = true;
                held.set(preferences);
                preferences.save();
            }
        }
    };

    window.on_bench_dismiss({
        let weak = window.as_weak();
        let mark_tested = mark_tested.clone();
        move || {
            let Some(window) = weak.upgrade() else { return };
            window.set_bench_state("".into());
            mark_tested();
        }
    });

    window.on_bench_start({
        let weak = window.as_weak();
        let mark_tested = mark_tested.clone();
        let found = Arc::clone(&found);
        move || {
            let Some(window) = weak.upgrade() else { return };
            if window.get_bench_state() == "running" {
                return;
            }
            mark_tested();
            window.set_bench_state("running".into());
            window.set_bench_step(0);
            let weak = window.as_weak();
            let found = Arc::clone(&found);
            std::thread::spawn(move || {
                let step = |at: i32| {
                    let weak = weak.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = weak.upgrade() {
                            window.set_bench_step(at);
                        }
                    });
                };
                let result = run_apart(engine, &step);
                tracing::info!(?result, "machine test done");
                if let Ok(mut kept) = found.lock() {
                    *kept = result;
                }
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = weak.upgrade() {
                        window.set_bench_sharpness(result.sharpness);
                        window.set_bench_neural(result.neural);
                        window.set_bench_rate(result.rate as i32);
                        window.set_bench_state("done".into());
                    }
                });
            });
        }
    });

    window.on_bench_apply({
        let weak = window.as_weak();
        move || {
            let Some(window) = weak.upgrade() else { return };
            window.set_bench_state("".into());
            let found = found.lock().map(|kept| *kept).unwrap_or_default();
            apply(&window, found);
        }
    });
}

/// Sets the player's menus to what was found, through the same callbacks the
/// picture panel uses, so it is kept as if chosen there.
fn apply(window: &MainWindow, found: Found) {
    if found.engine != 0 {
        window.invoke_set_neural_engine(found.engine as i32);
    }
    window.invoke_set_frame_rate(found.rate as i32);
    // Set, not flipped: the window's copy of it is only kept up to date while
    // an episode plays, and a flip read off a stale copy turned the network
    // on when the test had said no.
    window.invoke_set_neural_upscale(found.neural);
    match found.sharpness {
        0 => window.invoke_set_upscale(0),
        1 => {
            window.invoke_set_upscale(1);
            window.invoke_set_upscale_quality(1);
        }
        _ if found.neural => {
            window.invoke_set_upscale(1);
            window.invoke_set_upscale_quality(1);
        }
        _ => {
            window.invoke_set_upscale(4);
            window.invoke_set_upscale_quality(2);
        }
    }
}

/// The flag the program is started with to run the test in a process of its
/// own.
pub const FLAG: &str = "--machine-test";

/// Runs the test in a child process and reads what it found.
///
/// Apart because what the networks take is not given back while the process
/// lives: TensorRT's context and VapourSynth's frame pools stay allocated after
/// the player that used them is gone — gigabytes, for a test that runs once.
/// The child also writes its scripts to a folder of its own, so it never
/// touches the one an episode plays through.
fn run_apart(engine: usize, step: &dyn Fn(i32)) -> Found {
    use std::io::BufRead;

    let Ok(exe) = std::env::current_exe() else {
        return Found::default();
    };
    let child = std::process::Command::new(exe)
        .arg(FLAG)
        .arg(engine.to_string())
        .env(
            "ANIRUST_SCRIPT_DIR",
            anirust_player::frames::work_dir().join("machine-test"),
        )
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) => {
            tracing::warn!(%error, "could not start the machine test");
            return Found::default();
        }
    };
    let mut found = Found::default();
    if let Some(out) = child.stdout.take() {
        for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
            let words: Vec<&str> = line.split_whitespace().collect();
            match words.as_slice() {
                ["step", at] => step(at.parse().unwrap_or(0)),
                ["found", engine, rate, neural, sharpness] => {
                    found = Found {
                        engine: engine.parse().unwrap_or(0),
                        rate: rate.parse().unwrap_or(0),
                        neural: *neural == "1",
                        sharpness: sharpness.parse().unwrap_or(1),
                    };
                }
                _ => {}
            }
        }
    }
    let _ = child.wait();
    found
}

/// The child's side: runs the test and writes the steps and the result to
/// standard output, one line each, for [`run_apart`] to read.
pub fn child_main(engine: &str) {
    let found = run(engine.parse().unwrap_or(0), &|at| println!("step {at}"));
    println!(
        "found {} {} {} {}",
        found.engine,
        found.rate,
        u8::from(found.neural),
        found.sharpness
    );
}

/// The test itself. Blocks for a minute or so, longer the first time a
/// TensorRT engine has to be built; run it off the UI thread.
fn run(engine: usize, step: &dyn Fn(i32)) -> Found {
    let vendors = gpus::vendors();
    let backend = engine
        .checked_sub(1)
        .and_then(|at| Backend::ALL.get(at).copied());
    let mlrt = backend.and_then(Mlrt::find);
    let rife = RifeInstall::find();

    let networks = match (&mlrt, &rife) {
        (Some(mlrt), _) => Some(Networks::Mlrt(mlrt)),
        (None, Some(rife)) => Some(Networks::Vulkan(rife)),
        (None, None) => None,
    };
    let used_engine = if mlrt.is_some() { engine } else { 0 };

    let warm_up = Duration::from_secs(600);
    let seconds = Duration::from_secs(5);
    let keeps_up = |networks: Networks<'_>, enhancement: Enhancement| match bench::measure(
        networks,
        enhancement,
        warm_up,
        seconds,
    ) {
        Ok(Some(measured)) => {
            tracing::info!(?enhancement, speed = measured.speed, "measured");
            measured.keeps_up()
        }
        Ok(None) => false,
        Err(error) => {
            tracing::warn!(%error, "the machine test could not run the player");
            false
        }
    };
    let frames = |rate: TargetRate| FrameGeneration {
        rate,
        ..FrameGeneration::default()
    };

    step(0);
    let rate = match networks {
        Some(networks) => {
            if keeps_up(
                networks,
                Enhancement {
                    frames: Some(frames(TargetRate::Sixty)),
                    upscale: false,
                },
            ) {
                2
            } else if keeps_up(
                networks,
                Enhancement {
                    frames: Some(frames(TargetRate::Double)),
                    upscale: false,
                },
            ) {
                1
            } else {
                0
            }
        }
        None => 0,
    };

    // The network on top of the smoothness found, then with less smoothness
    // under it: doubled to 1440p first, RIFE then draws four times the
    // pixels, and a card that runs each alone with room to spare may not run
    // both at sixty. The network is the bigger step for the picture, so it
    // stays and the smoothness gives way.
    step(1);
    let mut rate = rate;
    let mut neural = false;
    if let Some(mlrt) = mlrt.as_ref() {
        for tried in (0..=rate).rev() {
            let generation = match tried {
                2 => Some(frames(TargetRate::Sixty)),
                1 => Some(frames(TargetRate::Double)),
                _ => None,
            };
            let enhancement = Enhancement {
                frames: generation,
                upscale: true,
            };
            if keeps_up(Networks::Mlrt(mlrt), enhancement) {
                neural = true;
                rate = tried;
                break;
            }
        }
    }

    Found {
        engine: used_engine,
        rate,
        neural,
        sharpness: if neural { 2 } else { shader_sharpness(vendors) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_discrete_card_gets_the_strongest_shaders() {
        assert_eq!(shader_sharpness(Some(&[0x8086, 0x10de])), 2);
        assert_eq!(shader_sharpness(Some(&[0x1002])), 2);
    }

    #[test]
    fn integrated_or_unknown_graphics_get_the_lighter_ones() {
        assert_eq!(shader_sharpness(Some(&[0x8086])), 1);
        assert_eq!(shader_sharpness(None), 1);
    }
}
