// SPDX-License-Identifier: GPL-3.0-or-later

//! The download queue.
//!
//! One episode at a time. The fetching is already parallel across segments, and
//! two episodes at once would halve each one's share of a link rather than
//! finish anything sooner — while making the progress on screen harder to
//! follow.
//!
//! A job is cancelled by dropping the task; the partly written file goes with
//! it, since a half-downloaded episode is not something to leave lying around
//! pretending to be an episode.

use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use slint::{ComponentHandle, VecModel};

use anirust_download::{Download, Ffmpeg, Progress};
use anirust_extract::{Registry, ResolvedStream};

use crate::enhance::{self, Choice, Outcome, Resolution};
use crate::{DownloadItem, MainWindow, tasks};

/// What a queued episode needs to be fetched.
#[derive(Clone)]
pub struct Job {
    pub release: String,
    pub position: i32,
    pub dubber: String,
    /// The URL to resolve. Held rather than the resolved stream: a link is only
    /// good for a few hours, and a queue can outlive that.
    pub url: String,
}

/// How a job ended, or that it has not.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Queued,
    Running,
    Done,
    Unchanged,
    Failed,
    Removed,
}

impl Status {
    fn name(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Done => "done",
            Self::Unchanged => "unchanged",
            Self::Failed => "failed",
            Self::Removed => "removed",
        }
    }
}

struct Entry {
    job: Job,
    status: Status,
    fraction: f32,
    detail: String,
    /// The file it wrote, once done.
    path: Option<PathBuf>,
    /// Set for a job that makes a saved episode better rather than fetching
    /// one: the file it starts from and what it does.
    enhance: Option<(PathBuf, Choice)>,
    /// Asks a running improvement to stop.
    stop: Option<Arc<AtomicBool>>,
}

#[derive(Serialize, Deserialize)]
struct SavedEpisode {
    release: String,
    position: i32,
    dubber: String,
    path: PathBuf,
}

impl SavedEpisode {
    fn from_legacy_file(path: PathBuf) -> Option<Self> {
        let stem = path.file_stem()?.to_str()?;
        let stem = match path.extension()?.to_str()? {
            "mp4" => stem,
            "mkv" => Resolution::ALL
                .iter()
                .find_map(|mode| stem.strip_suffix(mode.marker()))?,
            _ => return None,
        };
        let (stem, dubber) = stem
            .strip_suffix(']')
            .and_then(|s| s.rsplit_once(" ["))
            .unwrap_or((stem, ""));
        let (release, position) = stem.rsplit_once(" - ")?;
        Some(Self {
            release: release.into(),
            position: position.parse().ok()?,
            dubber: dubber.into(),
            path,
        })
    }

    fn into_entry(self) -> Entry {
        Entry {
            job: Job {
                release: self.release,
                position: self.position,
                dubber: self.dubber,
                url: String::new(),
            },
            status: Status::Done,
            fraction: 1.0,
            detail: self.path.display().to_string(),
            path: Some(self.path),
            enhance: None,
            stop: None,
        }
    }
}

fn restored(directory: &Path) -> Vec<Entry> {
    let saved: Vec<SavedEpisode> = std::fs::read(directory.join(".downloads.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_else(|| {
            let mut files: Vec<_> = std::fs::read_dir(directory)
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .filter_map(|entry| SavedEpisode::from_legacy_file(entry.path()))
                .collect();
            files.sort_by(|a, b| a.path.cmp(&b.path));
            files
        });
    saved
        .into_iter()
        .filter(|episode| episode.path.is_file())
        .map(SavedEpisode::into_entry)
        .collect()
}

/// The queue, and whether something is being fetched from it right now.
#[derive(Default)]
pub struct Queue {
    entries: Vec<Entry>,
    waiting: VecDeque<usize>,
    running: bool,
    /// Improvements waiting, and whether one is running: apart from the
    /// downloads, since one wants the network and the other the GPU.
    enhance_waiting: VecDeque<usize>,
    enhancing: bool,
    /// The engine the networks run on, and what this machine can do with it:
    /// the neural upscale, frame generation.
    engine: usize,
    pub can_upscale: bool,
    pub can_generate: bool,
    /// Where finished episodes are written.
    directory: Option<PathBuf>,
}

impl Queue {
    /// An empty queue whose improvements run on `engine`.
    #[must_use]
    pub fn new(engine: usize, can_upscale: bool, can_generate: bool) -> Self {
        let mut queue = Self {
            engine,
            can_upscale,
            can_generate,
            ..Self::default()
        };
        queue.entries = restored(&queue.directory());
        queue
    }

    fn save(&mut self) {
        let directory = self.directory();
        if !directory.is_dir() {
            return;
        }
        let saved: Vec<_> = self
            .entries
            .iter()
            .filter_map(|entry| {
                Some(SavedEpisode {
                    path: entry.path.clone()?,
                    release: entry.job.release.clone(),
                    position: entry.job.position,
                    dubber: entry.job.dubber.clone(),
                })
            })
            .collect();
        let write = || -> std::io::Result<()> {
            let temp = directory.join(".downloads.json.tmp");
            std::fs::write(&temp, serde_json::to_vec(&saved)?)?;
            std::fs::rename(temp, directory.join(".downloads.json"))
        };
        if let Err(error) = write() {
            tracing::warn!(%error, "saving download history failed");
        }
    }

    /// Where downloads land: the user's own videos directory, under a folder of
    /// ours, falling back to the home directory when the platform has no such
    /// notion.
    fn directory(&mut self) -> PathBuf {
        self.directory
            .get_or_insert_with(|| {
                dirs::video_dir()
                    .or_else(dirs::home_dir)
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("AniRust")
            })
            .clone()
    }

    fn enhancement_paths(&self) -> impl Iterator<Item = PathBuf> + '_ {
        self.entries
            .iter()
            .filter(|entry| matches!(entry.status, Status::Queued | Status::Running))
            .filter_map(|entry| entry.enhance.as_ref())
            .flat_map(|(source, choice)| {
                [
                    source.clone(),
                    enhance::destination_for(source, choice.resolution),
                ]
            })
    }

    fn remove_saved(&mut self, index: usize, expected_path: &str) -> bool {
        let Some(entry) = self.entries.get(index) else {
            return false;
        };
        let Some(path) = entry.path.clone() else {
            return false;
        };
        if entry.status != Status::Done
            || path.to_string_lossy() != expected_path
            || path.parent() != Some(self.directory().as_path())
            || self.enhancement_paths().any(|used| used == path)
        {
            return false;
        }
        if let Err(error) = std::fs::remove_file(&path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(%error, path = %path.display(), "deleting the saved episode failed");
            return false;
        }
        // Running jobs keep their indices while a completed file is removed.
        for entry in &mut self.entries {
            if entry.path.as_ref() == Some(&path) {
                entry.path = None;
                entry.status = Status::Removed;
            }
        }
        self.save();
        true
    }
}

/// Adds an episode to the queue and starts it if nothing else is running.
pub fn enqueue(
    window: &MainWindow,
    queue: &Rc<RefCell<Queue>>,
    registry: &Rc<Registry>,
    http: &reqwest::Client,
    job: Job,
) {
    {
        let mut queue = queue.borrow_mut();
        // Asking twice for the same episode is a mis-click, not a request for
        // two copies.
        let destination = queue.directory().join(anirust_download::file_name(
            &job.release,
            job.position,
            &job.dubber,
        ));
        let already = queue.entries.iter().any(|entry| {
            (entry.enhance.is_none()
                && entry.job.url == job.url
                && !matches!(entry.status, Status::Failed | Status::Removed))
                || (entry.path.as_ref() == Some(&destination) && destination.is_file())
        });
        if already {
            return;
        }

        queue.entries.push(Entry {
            job,
            status: Status::Queued,
            fraction: 0.0,
            detail: String::new(),
            path: None,
            enhance: None,
            stop: None,
        });
        let at = queue.entries.len() - 1;
        queue.waiting.push_back(at);
    }

    show(window, queue);
    pump(window, queue, registry, http);
}

/// Starts the next job, if there is one and nothing else is running.
fn pump(
    window: &MainWindow,
    queue: &Rc<RefCell<Queue>>,
    registry: &Rc<Registry>,
    http: &reqwest::Client,
) {
    let (at, job, directory) = {
        let mut queue = queue.borrow_mut();
        if queue.running {
            return;
        }
        let Some(at) = queue.waiting.pop_front() else {
            return;
        };
        let directory = queue.directory();
        queue.running = true;
        queue.entries[at].status = Status::Running;
        (at, queue.entries[at].job.clone(), directory)
    };

    show(window, queue);

    let destination = directory.join(anirust_download::file_name(
        &job.release,
        job.position,
        &job.dubber,
    ));
    let resolver = (**registry).clone();
    let http = http.clone();
    let weak = window.as_weak();
    let queue_handle = Rc::clone(queue);
    let registry = Rc::clone(registry);
    let http_for_next = http.clone();

    // Progress crosses back from the worker thread through the event loop:
    // the model it updates belongs to the interface.
    let reporter = window.as_weak();
    let (send, mut receive) = tokio::sync::mpsc::unbounded_channel::<Progress>();
    let progress_queue = Rc::clone(queue);

    let _ = slint::spawn_local(async move {
        while let Some(progress) = receive.recv().await {
            let Some(window) = reporter.upgrade() else {
                break;
            };
            if let Some(fraction) = progress.fraction() {
                progress_queue.borrow_mut().entries[at].fraction = fraction;
            }
            if progress == Progress::Remuxing {
                progress_queue.borrow_mut().entries[at].detail = "ffmpeg".to_owned();
            }
            show(&window, &progress_queue);
        }
    });

    tasks::spawn(
        async move {
            let stream: ResolvedStream = if resolver.supports(&job.url) {
                resolver
                    .resolve(&job.url)
                    .await
                    .map_err(|e| e.to_string())?
            } else {
                ResolvedStream {
                    variants: vec![anirust_extract::StreamVariant {
                        height: anirust_extract::UNKNOWN_HEIGHT,
                        kind: anirust_extract::StreamKind::classify(None, &job.url),
                        url: job.url.clone(),
                    }],
                    ..Default::default()
                }
            };

            let ffmpeg = Ffmpeg::find().map_err(|e| e.to_string())?;
            if let Some(parent) = destination.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| e.to_string())?;
            }

            Download::new(http, ffmpeg)
                .save(&stream, &destination, |progress| {
                    let _ = send.send(progress);
                })
                .await
                .map_err(|e| e.to_string())?;

            Ok::<PathBuf, String>(destination)
        },
        move |result| {
            {
                let mut queue = queue_handle.borrow_mut();
                queue.running = false;
                let entry = &mut queue.entries[at];
                match &result {
                    Ok(path) => {
                        entry.status = Status::Done;
                        entry.fraction = 1.0;
                        entry.detail = path.display().to_string();
                        entry.path = Some(path.clone());
                        tracing::info!(path = %path.display(), "download finished");
                        queue.save();
                    }
                    Err(error) => {
                        entry.status = Status::Failed;
                        entry.detail = error.clone();
                        tracing::warn!(%error, "download failed");
                    }
                }
            }

            let Some(window) = weak.upgrade() else { return };
            show(&window, &queue_handle);
            pump(&window, &queue_handle, &registry, &http_for_next);
        },
    );
}

/// The file and labels for a completed download or enhancement.
pub fn saved_episode(queue: &RefCell<Queue>, index: usize) -> Option<(PathBuf, String, String)> {
    let queue = queue.borrow();
    let entry = queue.entries.get(index)?;
    if entry.status != Status::Done {
        return None;
    }
    Some((
        entry.path.clone()?,
        entry.job.release.clone(),
        if entry.job.dubber.is_empty() {
            entry.job.position.to_string()
        } else {
            format!("{} · {}", entry.job.position, entry.job.dubber)
        },
    ))
}

pub fn remove_saved(
    window: &MainWindow,
    queue: &Rc<RefCell<Queue>>,
    index: usize,
    expected_path: &str,
) -> bool {
    let removed = queue.borrow_mut().remove_saved(index, expected_path);
    if removed {
        show(window, queue);
    }
    removed
}

/// Pushes the queue onto the screen.
pub fn show(window: &MainWindow, queue: &Rc<RefCell<Queue>>) {
    let queue = queue.borrow();
    let enhancement_paths: HashSet<_> = queue.enhancement_paths().collect();
    let items: Vec<DownloadItem> = queue
        .entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.status != Status::Removed)
        .map(|(index, entry)| DownloadItem {
            index: index as i32,
            title: format!("{} — {}", entry.job.release, entry.job.position).into(),
            dubber: entry.job.dubber.as_str().into(),
            status: entry.status.name().into(),
            progress: entry.fraction,
            detail: entry.detail.as_str().into(),
            enhancing: entry.enhance.is_some()
                || entry
                    .path
                    .as_ref()
                    .is_some_and(|p| p.extension().is_some_and(|ext| ext == "mkv")),
            // A saved episode, not one made better already, on a machine
            // that can do something to it.
            can_enhance: entry.status == Status::Done
                && entry.enhance.is_none()
                && entry
                    .path
                    .as_ref()
                    .is_some_and(|p| !p.extension().is_some_and(|ext| ext == "mkv")),
            can_delete: entry.status == Status::Done
                && entry
                    .path
                    .as_ref()
                    .is_some_and(|path| !enhancement_paths.contains(path)),
        })
        .collect();

    window.set_downloads(slint::ModelRc::new(VecModel::from(items)));

    // What the badge on the toolbar counts: a queue nobody can see from the
    // screen they are on still has to be able to say it is working.
    let working = queue
        .entries
        .iter()
        .filter(|entry| matches!(entry.status, Status::Queued | Status::Running))
        .count();
    window.set_downloads_working(i32::try_from(working).unwrap_or(i32::MAX));
}

/// Forgets finished and failed jobs, leaving what is still to come.
pub fn clear_finished(window: &MainWindow, queue: &Rc<RefCell<Queue>>) {
    {
        let mut queue = queue.borrow_mut();
        if queue.running || queue.enhancing {
            return;
        }
        let keep: Vec<bool> = queue
            .entries
            .iter()
            .map(|entry| matches!(entry.status, Status::Queued | Status::Running))
            .collect();

        let mut at = 0;
        queue.entries.retain(|_| {
            let keeping = keep[at];
            at += 1;
            keeping
        });

        queue.save();

        // The waiting list holds indices into `entries`, which have just moved.
        queue.waiting = queue
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.status == Status::Queued && entry.enhance.is_none())
            .map(|(at, _)| at)
            .collect();
        queue.enhance_waiting = queue
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.status == Status::Queued && entry.enhance.is_some())
            .map(|(at, _)| at)
            .collect();
    }
    show(window, queue);
}

/// Queues making the saved episode at `index` better, and starts it if no
/// other improvement is running.
pub fn enhance(window: &MainWindow, queue: &Rc<RefCell<Queue>>, index: usize, choice: Choice) {
    {
        let mut queue = queue.borrow_mut();
        let Some(entry) = queue.entries.get(index) else {
            return;
        };
        let Some(source) = entry.path.clone() else {
            return;
        };
        let destination = enhance::destination_for(&source, choice.resolution);
        if queue.enhancement_paths().any(|path| path == destination) {
            return;
        }
        let job = entry.job.clone();
        queue.entries.push(Entry {
            job,
            status: Status::Queued,
            fraction: 0.0,
            detail: String::new(),
            path: None,
            enhance: Some((source, choice)),
            stop: None,
        });
        let at = queue.entries.len() - 1;
        queue.enhance_waiting.push_back(at);
    }
    show(window, queue);
    pump_enhance(window, queue);
}

/// Stops the improvement at `index`, running or waiting.
pub fn stop_enhance(window: &MainWindow, queue: &Rc<RefCell<Queue>>, index: usize) {
    {
        let mut queue = queue.borrow_mut();
        queue.enhance_waiting.retain(|at| *at != index);
        let Some(entry) = queue.entries.get_mut(index) else {
            return;
        };
        match entry.status {
            Status::Running => {
                if let Some(stop) = &entry.stop {
                    stop.store(true, Ordering::Relaxed);
                }
            }
            Status::Queued => {
                entry.status = Status::Failed;
                entry.detail = String::new();
            }
            Status::Done | Status::Unchanged | Status::Failed | Status::Removed => {}
        }
    }
    show(window, queue);
}

fn pump_enhance(window: &MainWindow, queue: &Rc<RefCell<Queue>>) {
    let (at, source, choice, stop, engine) = {
        let mut queue = queue.borrow_mut();
        if queue.enhancing {
            return;
        }
        let Some(at) = queue.enhance_waiting.pop_front() else {
            return;
        };
        let engine = queue.engine;
        let entry = &mut queue.entries[at];
        let Some((source, choice)) = entry.enhance.clone() else {
            return;
        };
        let stop = Arc::new(AtomicBool::new(false));
        entry.status = Status::Running;
        entry.stop = Some(Arc::clone(&stop));
        queue.enhancing = true;
        (at, source, choice, stop, engine)
    };
    show(window, queue);

    // The worker reports through a channel the UI thread drains: the queue
    // is the window's, and the worker must not touch it.
    enum Message {
        Progress(Option<f32>),
        End(Outcome),
    }
    let (send, mut receive) = tokio::sync::mpsc::unbounded_channel::<Message>();
    let weak = window.as_weak();
    let queue_handle = Rc::clone(queue);
    let _ = slint::spawn_local(async move {
        while let Some(message) = receive.recv().await {
            let Some(window) = weak.upgrade() else {
                break;
            };
            let ended = matches!(message, Message::End(_));
            {
                let mut queue = queue_handle.borrow_mut();
                if ended {
                    queue.enhancing = false;
                }
                // A successful re-run replaces the saved variant at the same path.
                if let Message::End(Outcome::Done(path)) = &message {
                    for previous in &mut queue.entries {
                        if previous.path.as_ref() == Some(path) {
                            previous.status = Status::Removed;
                            previous.path = None;
                        }
                    }
                }
                let entry = &mut queue.entries[at];
                match message {
                    Message::Progress(done) => {
                        entry.fraction = done.unwrap_or(0.0);
                        entry.detail = if done.is_none() {
                            "starting".to_owned()
                        } else {
                            String::new()
                        };
                    }
                    Message::End(Outcome::Done(path)) => {
                        tracing::info!(path = %path.display(), "episode improved");
                        entry.stop = None;
                        entry.status = Status::Done;
                        entry.fraction = 1.0;
                        entry.detail = path.display().to_string();
                        entry.path = Some(path);
                        queue.save();
                    }
                    Message::End(Outcome::Unchanged) => {
                        entry.stop = None;
                        entry.status = Status::Unchanged;
                        entry.detail.clear();
                    }
                    Message::End(Outcome::Stopped) => {
                        entry.stop = None;
                        entry.status = Status::Failed;
                        entry.detail = String::new();
                    }
                    Message::End(Outcome::Failed(error)) => {
                        tracing::warn!(%error, "improving the episode failed");
                        entry.stop = None;
                        entry.status = Status::Failed;
                        entry.detail = error;
                    }
                }
            }
            show(&window, &queue_handle);
            if ended {
                pump_enhance(&window, &queue_handle);
                break;
            }
        }
    });

    std::thread::spawn(move || {
        let outcome = enhance::run_apart(&source, choice, engine, &stop, &|done| {
            let _ = send.send(Message::Progress(done));
        });
        let _ = send.send(Message::End(outcome));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_downloads_survive_restart_until_cleared() {
        let dir =
            std::env::temp_dir().join(format!("anirust-download-history-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let original = dir.join("Show - 03 [Dub].mp4");
        let improved = enhance::destination_for(&original, Resolution::Double);
        std::fs::write(&original, []).unwrap();
        std::fs::write(&improved, []).unwrap();

        let mut queue = Queue {
            entries: restored(&dir),
            directory: Some(dir.clone()),
            ..Queue::default()
        };
        assert_eq!(queue.entries.len(), 2);
        assert!(
            queue
                .entries
                .iter()
                .all(|entry| entry.job.position == 3 && entry.job.dubber == "Dub")
        );
        queue.save();
        std::fs::remove_file(improved).unwrap();
        assert_eq!(restored(&dir).len(), 1);
        queue.entries.clear();
        queue.save();
        assert!(restored(&dir).is_empty());
        assert!(original.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn deleting_one_copy_keeps_other_files_and_running_jobs() {
        let dir = std::env::temp_dir().join(format!("anirust-delete-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let original = dir.join("Show - 03 [Dub].mp4");
        let improved = enhance::destination_for(&original, Resolution::P1440);
        let alternative = enhance::destination_for(&original, Resolution::P2160);
        std::fs::write(&original, []).unwrap();
        std::fs::write(&improved, []).unwrap();
        std::fs::write(&alternative, []).unwrap();
        let mut queue = Queue {
            entries: restored(&dir),
            directory: Some(dir.clone()),
            running: true,
            ..Queue::default()
        };
        let index = queue
            .entries
            .iter()
            .position(|entry| entry.path.as_ref() == Some(&improved))
            .unwrap();
        let mut running = SavedEpisode::from_legacy_file(original.clone())
            .unwrap()
            .into_entry();
        running.status = Status::Running;
        running.path = None;
        running.enhance = Some((
            original.clone(),
            Choice {
                resolution: Resolution::P2160,
                profile: anirust_player::enhance::Profile::Economy,
                rate: 0,
                display_millihz: 0,
            },
        ));
        let running_index = queue.entries.len();
        queue.entries.push(running);

        let index_of = |path: &Path| {
            queue
                .entries
                .iter()
                .position(|entry| entry.path.as_deref() == Some(path))
                .unwrap()
        };
        let original_index = index_of(&original);
        let alternative_index = index_of(&alternative);
        assert!(!queue.remove_saved(original_index, &original.to_string_lossy()));
        assert!(!queue.remove_saved(alternative_index, &alternative.to_string_lossy()));
        assert!(!queue.remove_saved(index, &original.to_string_lossy()));
        assert!(queue.remove_saved(index, &improved.to_string_lossy()));
        assert!(!improved.exists());
        assert!(original.is_file());
        assert!(queue.entries[running_index].status == Status::Running);
        let after_restart = restored(&dir);
        assert_eq!(after_restart.len(), 2);
        assert!(
            after_restart
                .iter()
                .any(|entry| entry.path.as_deref() == Some(original.as_path()))
        );
        assert!(
            after_restart
                .iter()
                .any(|entry| entry.path.as_deref() == Some(alternative.as_path()))
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
