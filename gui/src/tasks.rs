// SPDX-License-Identifier: GPL-3.0-or-later

//! Running network work without blocking the interface.
//!
//! Slint owns the main thread and its event loop; every API call and extractor
//! request is async and wants a Tokio reactor. Rather than bend one around the
//! other, a runtime lives on its own thread and results are posted back to the
//! event loop.
//!
//! The rule that makes this safe is narrow: **nothing touches the window off
//! the UI thread**. A task returns a plain value, and [`spawn`] hands it to a
//! closure that Slint runs on the event loop, where the window is legal to
//! touch again.
//!
//! The continuation runs through `slint::spawn_local`, which is what lets it
//! hold the `Rc`s the interface is built from — an ordinary cross-thread post
//! would demand `Send` of everything the screen owns.

use std::future::Future;
use std::sync::OnceLock;

use anyhow::{Context, Result};

/// The runtime every background task runs on.
///
/// One per process, created on first use. Multi-threaded because resolution
/// fans out across several hosts and a single-threaded runtime would serialise
/// waits that have no reason to be serial.
static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// Starts the runtime, failing loudly if the process cannot have one.
pub fn init() -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("anirust-net")
        .build()
        .context("starting the async runtime")?;

    // A second call would mean two runtimes, which is a bug rather than a
    // condition to recover from.
    RUNTIME
        .set(runtime)
        .map_err(|_| anyhow::anyhow!("the async runtime was already started"))
}

fn runtime() -> &'static tokio::runtime::Runtime {
    RUNTIME
        .get()
        .expect("tasks::init must run before any task is spawned")
}

/// Runs `work` in the background and hands its result to `then` on the UI
/// thread.
///
/// `then` is where the window may be touched; `work` must never do so. If the
/// event loop has already gone, the result is dropped — there is nothing left
/// to show it to.
///
/// Must be called from the UI thread, which every caller is: a task is always
/// started by something the viewer did.
pub fn spawn<T, F>(work: F, then: impl FnOnce(T) + 'static)
where
    T: Send + 'static,
    F: Future<Output = T> + Send + 'static,
{
    let running = runtime().spawn(work);

    let posted = slint::spawn_local(async move {
        match running.await {
            Ok(value) => then(value),
            // A panic in a task is a bug in the task, not a reason to take the
            // window down with it. The screen keeps whatever it was showing.
            Err(error) => tracing::error!(%error, "a background task did not finish"),
        }
    });

    if posted.is_err() {
        tracing::debug!("no event loop; dropping a task result");
    }
}

/// Fetches an image and decodes it for Slint.
///
/// Decoding happens on the worker thread: a poster is a megabyte of JPEG and
/// doing that on the UI thread would drop frames in the player running behind
/// the screen.
pub async fn fetch_image(
    http: reqwest::Client,
    url: String,
) -> Result<slint::SharedPixelBuffer<slint::Rgba8Pixel>> {
    let bytes = http
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    let decoded = image::load_from_memory(&bytes)
        .with_context(|| format!("decoding the image at {url}"))?
        .into_rgba8();

    Ok(slint::SharedPixelBuffer::clone_from_slice(
        decoded.as_raw(),
        decoded.width(),
        decoded.height(),
    ))
}
