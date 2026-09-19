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

use slint::{ComponentHandle, VecModel};

use anirust_download::{Download, Ffmpeg, Progress};
use anirust_extract::{Registry, ResolvedStream};

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
}

/// The queue, and whether something is being fetched from it right now.
#[derive(Default)]
pub struct Queue {
    entries: Vec<Entry>,
    waiting: VecDeque<usize>,
    running: bool,
    /// Where finished episodes are written.
    directory: Option<PathBuf>,
}

impl Queue {
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
        })
        .collect();

    window.set_downloads(slint::ModelRc::new(VecModel::from(items)));
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
            .filter(|(_, entry)| entry.status == Status::Queued)
            .map(|(at, _)| at)
            .collect();
    }
    show(window, queue);
}
