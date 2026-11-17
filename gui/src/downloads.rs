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

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use slint::{ComponentHandle, VecModel};

use anirust_download::{Download, Ffmpeg, Progress};
use anirust_extract::{Registry, ResolvedStream};

use crate::enhance::{self, Choice, Outcome};
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
    Failed,
}

impl Status {
    fn name(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
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
        Self {
            engine,
            can_upscale,
            can_generate,
            ..Self::default()
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
        let already = queue
            .entries
            .iter()
            .any(|entry| entry.job.url == job.url && entry.status != Status::Failed);
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

/// Pushes the queue onto the screen.
pub fn show(window: &MainWindow, queue: &Rc<RefCell<Queue>>) {
    let queue = queue.borrow();
    let items: Vec<DownloadItem> = queue
        .entries
        .iter()
        .map(|entry| DownloadItem {
            title: format!("{} — {}", entry.job.release, entry.job.position).into(),
            dubber: entry.job.dubber.as_str().into(),
            status: entry.status.name().into(),
            progress: entry.fraction,
            detail: entry.detail.as_str().into(),
            enhancing: entry.enhance.is_some(),
            // A saved episode, not one made better already, on a machine
            // that can do something to it.
            can_enhance: entry.status == Status::Done
                && entry.enhance.is_none()
                && entry.path.is_some()
                && (queue.can_upscale || queue.can_generate),
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
            Status::Done | Status::Failed => {}
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
