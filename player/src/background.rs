// SPDX-License-Identifier: GPL-3.0-or-later

//! Keeping mpv's own pauses out of the caller's thread.
//!
//! Every property read and every change goes through mpv's core thread and
//! waits for it. Most of the time that is a fraction of a millisecond, but the
//! core can stand still for seconds: the VapourSynth filter loads its script
//! there — Python, vs-mlrt, a TensorRT engine read from disk or built — when
//! the neural filters are switched on, and again after every seek. A UI thread
//! that asks mpv anything in that window freezes with it, and one that asks
//! for the video's size on every frame freezes outright.
//!
//! So a UI asks this module instead: [`Player::status`] answers from a
//! snapshot a thread of its own keeps fresh, and [`Player::later`] queues a
//! change for a second thread to make, in the order it was asked for. Before
//! [`Player::start_background`] both act directly, which is what the CLI and
//! the tests want.

use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use crate::{PlaybackState, Player, Result, TrackKind};

/// A change waiting for mpv.
pub(crate) type Job = Box<dyn FnOnce(&Player) + Send>;

/// The player as last read, all at once.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    /// Whether a file is loaded.
    pub loaded: bool,
    /// Failed file loads observed by the background reader.
    pub load_failures: u64,
    pub state: PlaybackState,
    pub position: Option<Duration>,
    pub duration: Option<Duration>,
    pub buffered_until: Option<Duration>,
    /// The decoded picture's size.
    pub video_size: Option<(u32, u32)>,
    /// The picture's size after the filters.
    pub filtered_size: Option<(u32, u32)>,
    /// The hardware decoder engaged, if any.
    pub hwdec: Option<String>,
    pub source_fps: Option<f64>,
    pub output_fps: Option<f64>,
    pub dropped_frames: u64,
    pub track_count: usize,
    pub subtitle_track: Option<i64>,
    pub audio_track: Option<i64>,
}

/// The two threads' ends, once started.
pub(crate) struct Background {
    pub(crate) jobs: Sender<Job>,
}

impl Player {
    /// Moves status reads and queued changes onto threads of their own, the
    /// snapshot refreshed every `every`. Later calls do nothing.
    ///
    /// Takes `&'static self` because the threads live as long as the process:
    /// a UI's player does too.
    pub fn start_background(&'static self, every: Duration) {
        let (send, receive) = mpsc::channel::<Job>();
        if self.background.set(Background { jobs: send }).is_err() {
            return;
        }
        *self.lock_status() = self.read_status();

        let spawned = std::thread::Builder::new()
            .name("anirust-mpv".into())
            .spawn(move || {
                for job in receive {
                    job(self);
                }
            })
            .and_then(|_| {
                std::thread::Builder::new()
                    .name("anirust-mpv-status".into())
                    .spawn(move || {
                        let mut load_failures = 0_u64;
                        loop {
                            while let Some(event) = self.mpv.wait_event(0.0) {
                                // libmpv2 returns a failed end-file event as Err.
                                if event.is_err() {
                                    load_failures = load_failures.wrapping_add(1);
                                }
                            }
                            let mut status = self.read_status();
                            status.load_failures = load_failures;
                            *self.lock_status() = status;
                            std::thread::sleep(every);
                        }
                    })
            });
        if let Err(error) = spawned {
            tracing::error!(%error, "the player's threads did not start");
        }
    }

    /// The player's state: from the snapshot once the background runs, read
    /// directly before that.
    #[must_use]
    pub fn status(&self) -> Status {
        if self.background.get().is_some() {
            self.lock_status().clone()
        } else {
            self.read_status()
        }
    }

    /// Makes a change without waiting for mpv: queued behind the changes
    /// before it once the background runs, made at once before that. A
    /// failure is logged under `what`; there is no one left to hand it to.
    pub fn later<F>(&self, what: &'static str, change: F)
    where
        F: FnOnce(&Player) -> Result<()> + Send + 'static,
    {
        let job: Job = Box::new(move |player| {
            if let Err(error) = change(player) {
                tracing::warn!(%error, what, "player change failed");
            }
        });
        match self.background.get() {
            Some(background) => {
                if let Err(mpsc::SendError(job)) = background.jobs.send(job) {
                    // The thread is gone; doing it here beats not at all.
                    job(self);
                }
            }
            None => job(self),
        }
    }

    fn lock_status(&self) -> std::sync::MutexGuard<'_, Status> {
        self.status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Everything a status line wants, straight from mpv. With nothing
    /// loaded only the one property is asked.
    fn read_status(&self) -> Status {
        if !self.is_playing() {
            return Status::default();
        }
        let position = self.position();
        Status {
            loaded: true,
            load_failures: 0,
            state: self.state(),
            position,
            duration: self.duration(),
            buffered_until: self.buffered_until(),
            video_size: self.video_size(),
            filtered_size: self.filtered_size(),
            hwdec: self.active_hwdec(),
            source_fps: self.source_fps(),
            output_fps: self.output_fps(),
            dropped_frames: self.dropped_frames(),
            track_count: self.track_count(),
            subtitle_track: self.current_track(TrackKind::Subtitle),
            audio_track: self.current_track(TrackKind::Audio),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PlayerConfig;

    #[test]
    fn nothing_loaded_reads_as_an_empty_status() {
        let player = Player::new(&PlayerConfig::headless()).expect("libmpv starts");
        assert_eq!(player.status(), Status::default());
    }

    #[test]
    fn a_change_before_the_background_is_made_at_once() {
        let player = Player::new(&PlayerConfig::headless()).expect("libmpv starts");
        player.later("speed", |player| player.set_speed(1.5));
        assert!((player.speed().unwrap() - 1.5).abs() < 1e-9);
    }

    #[test]
    fn queued_changes_are_made_in_order() {
        let player: &'static Player = Box::leak(Box::new(
            Player::new(&PlayerConfig::headless()).expect("libmpv starts"),
        ));
        player.start_background(Duration::from_millis(20));
        player.later("speed", |player| player.set_speed(2.0));
        player.later("speed", |player| player.set_speed(0.5));
        let (done, wait) = mpsc::channel();
        player.later("probe", move |_| {
            let _ = done.send(());
            Ok(())
        });
        wait.recv_timeout(Duration::from_secs(5))
            .expect("the queue runs");
        assert!((player.speed().unwrap() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_missing_local_file_reports_a_failed_load() {
        let player: &'static Player = Box::leak(Box::new(
            Player::new(&PlayerConfig::headless()).expect("libmpv starts"),
        ));
        player.start_background(Duration::from_millis(20));
        let missing = std::env::current_exe().unwrap().join("missing.mp4");
        player.later("open missing file", move |player| {
            player.open(&crate::MediaSource::new(missing.to_string_lossy()))
        });

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while player.status().load_failures == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(player.status().load_failures, 1);
    }
}
