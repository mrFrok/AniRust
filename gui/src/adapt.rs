// SPDX-License-Identifier: GPL-3.0-or-later

//! Keeping playback smooth on the machine it runs on.
//!
//! The heaviest upscaling and frame generation are offered to everyone, and
//! not every GPU can carry them. Rather than guessing from the card's name,
//! the player watches what it actually manages: mpv counts the frames it had
//! to drop because they were late. More than one a second, over ten seconds of
//! plain playback, and the load comes down one step — the RIFE network first,
//! then Anime4K's quality, then the generated frame rate, then each of them off
//! — with a line saying what changed. A step is kept, so the next episode
//! starts where this one settled.

use std::time::{Duration, Instant};

/// How long a stretch of playback is judged at a time.
const WINDOW: Duration = Duration::from_secs(10);

/// Dropped frames in [`WINDOW`] that count as not keeping up.
const TOO_MANY: u64 = 12;

/// How long to look away after a change: reloading shaders or the filter
/// drops a few frames of its own.
const SETTLE: Duration = Duration::from_secs(4);

/// What the menus are set to, as far as load goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Load {
    pub upscale_mode: usize,
    pub upscale_quality: usize,
    /// 0 off, then ×2, 60, the screen's rate.
    pub frame_rate: usize,
    /// 0 fast, 1 quality.
    pub rife_model: usize,
}

/// One step down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// The quality RIFE network for the fast one.
    FastRife,
    /// Anime4K one quality lower.
    LowerUpscale,
    /// Fewer generated frames: the screen's rate to 60, 60 to ×2.
    LowerFrameRate,
    FramesOff,
    UpscaleOff,
}

impl Load {
    /// The next step down from here, or `None` with nothing left to lower.
    #[must_use]
    pub fn next_step(self) -> Option<Step> {
        if self.frame_rate > 0 && self.rife_model > 0 {
            Some(Step::FastRife)
        } else if self.upscale_mode > 0 && self.upscale_quality > 0 {
            Some(Step::LowerUpscale)
        } else if self.frame_rate > 1 {
            Some(Step::LowerFrameRate)
        } else if self.frame_rate == 1 {
            Some(Step::FramesOff)
        } else if self.upscale_mode > 0 {
            Some(Step::UpscaleOff)
        } else {
            None
        }
    }

    /// The load after `step`.
    #[must_use]
    pub fn after(self, step: Step) -> Self {
        match step {
            Step::FastRife => Self {
                rife_model: 0,
                ..self
            },
            Step::LowerUpscale => Self {
                upscale_quality: self.upscale_quality - 1,
                ..self
            },
            Step::LowerFrameRate => Self {
                frame_rate: self.frame_rate - 1,
                ..self
            },
            Step::FramesOff => Self {
                frame_rate: 0,
                ..self
            },
            Step::UpscaleOff => Self {
                upscale_mode: 0,
                ..self
            },
        }
    }

    /// Whether there is any load to watch.
    #[must_use]
    pub fn is_enhanced(self) -> bool {
        self.upscale_mode > 0 || self.frame_rate > 0
    }
}

impl Step {
    /// The line shown when the step is taken.
    #[must_use]
    pub fn notice(self, ru: bool) -> &'static str {
        match (self, ru) {
            (Self::FastRife, true) => {
                "Видеокарта не успевает: генерация кадров переключена на быструю"
            }
            (Self::FastRife, false) => {
                "The GPU is falling behind: frame generation switched to fast"
            }
            (Self::LowerUpscale, true) => "Видеокарта не успевает: качество апскейла снижено",
            (Self::LowerUpscale, false) => "The GPU is falling behind: upscaling quality lowered",
            (Self::LowerFrameRate, true) => "Видеокарта не успевает: генерируется меньше кадров",
            (Self::LowerFrameRate, false) => "The GPU is falling behind: fewer frames generated",
            (Self::FramesOff, true) => "Видеокарта не успевает: генерация кадров выключена",
            (Self::FramesOff, false) => "The GPU is falling behind: frame generation off",
            (Self::UpscaleOff, true) => "Видеокарта не успевает: апскейл выключен",
            (Self::UpscaleOff, false) => "The GPU is falling behind: upscaling off",
        }
    }
}

/// The running count of dropped frames, judged a window at a time.
#[derive(Debug, Default)]
pub struct Watch {
    since: Option<(Instant, u64)>,
    quiet_until: Option<Instant>,
}

impl Watch {
    /// Forgets the window: playback paused, sought, or is not enhanced.
    pub fn reset(&mut self) {
        self.since = None;
    }

    /// Feeds the current drop count; true when the window just closed with
    /// too many drops in it.
    pub fn falling_behind(&mut self, now: Instant, dropped: u64) -> bool {
        if self.quiet_until.is_some_and(|until| now < until) {
            return false;
        }
        self.quiet_until = None;
        let Some((start, at_start)) = self.since else {
            self.since = Some((now, dropped));
            return false;
        };
        if dropped < at_start {
            // A new file starts its count again.
            self.since = Some((now, dropped));
            return false;
        }
        if now.duration_since(start) < WINDOW {
            return false;
        }
        self.since = Some((now, dropped));
        dropped - at_start >= TOO_MANY
    }

    /// Looks away for a moment after a change.
    pub fn settle(&mut self, now: Instant) {
        self.since = None;
        self.quiet_until = Some(now + SETTLE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heaviest() -> Load {
        Load {
            upscale_mode: 4,
            upscale_quality: 3,
            frame_rate: 3,
            rife_model: 1,
        }
    }

    #[test]
    fn the_load_comes_down_one_thing_at_a_time_until_nothing_is_left() {
        let mut load = heaviest();
        let mut steps = Vec::new();
        while let Some(step) = load.next_step() {
            steps.push(step);
            load = load.after(step);
        }
        assert_eq!(
            steps,
            [
                Step::FastRife,
                Step::LowerUpscale,
                Step::LowerUpscale,
                Step::LowerUpscale,
                Step::LowerFrameRate,
                Step::LowerFrameRate,
                Step::FramesOff,
                Step::UpscaleOff,
            ]
        );
        assert!(!load.is_enhanced());
    }

    #[test]
    fn a_few_drops_are_tolerated_and_a_steady_stream_is_not() {
        let start = Instant::now();
        let mut watch = Watch::default();
        assert!(!watch.falling_behind(start, 0));
        assert!(!watch.falling_behind(start + Duration::from_secs(5), 3));
        assert!(!watch.falling_behind(start + WINDOW, 5));
        assert!(watch.falling_behind(start + WINDOW * 2, 5 + TOO_MANY));
    }

    #[test]
    fn nothing_is_judged_while_settling_or_across_a_new_file() {
        let start = Instant::now();
        let mut watch = Watch::default();
        watch.settle(start);
        assert!(!watch.falling_behind(start + Duration::from_secs(1), 100));
        assert!(!watch.falling_behind(start + SETTLE, 100));
        // The count went back to zero: a new file.
        assert!(!watch.falling_behind(start + SETTLE + WINDOW, 0));
        assert!(!watch.falling_behind(start + SETTLE + WINDOW * 2, 2));
    }
}
