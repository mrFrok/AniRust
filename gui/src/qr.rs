// SPDX-License-Identifier: GPL-3.0-or-later

//! Signing in by QR code, from a phone that already is.
//!
//! The service hands this device a key, a secret and a PIN. The key goes into a
//! QR code the official app scans; the person types the PIN on the phone and
//! confirms; this side asks every few seconds how it stands, and once it is
//! confirmed the answer carries the session. It is the one way in for accounts
//! made through Google, VK, Telegram or Yandex: their tokens are only ever
//! handed to the official app.
//!
//! The sheet can be closed, or another code asked for, while an answer is on
//! its way. Each attempt has a number, and an answer for an attempt that is no
//! longer the current one is dropped.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::{ComponentHandle, Rgba8Pixel, SharedPixelBuffer};

use anirust_api::{Client, QrSignIn, QrStatus};

use crate::session::{self, Session};
use crate::{MainWindow, tasks};

/// How many failed asks in a row before saying so: a dropped connection
/// should not end a sign-in that is otherwise going fine.
const FAILURES_TOLERATED: u32 = 3;

/// Asked no faster than this, whatever the service suggests.
const MIN_POLL: Duration = Duration::from_secs(2);

/// The attempt under way, if any.
#[derive(Default)]
pub struct State {
    attempt: u64,
    session: Option<Attempt>,
}

struct Attempt {
    key: String,
    secret: String,
    poll: Duration,
    expires: Instant,
    failures: u32,
}

/// Everything the sheet's QR form needs.
pub struct Context<'a> {
    pub window: &'a MainWindow,
    pub state: &'a Rc<RefCell<State>>,
    pub session: &'a Rc<RefCell<Session>>,
    pub client: &'a Rc<Client>,
    pub http: reqwest::Client,
}

/// Asks the service for a new code and shows it.
pub fn start(cx: &Context<'_>) {
    let attempt = {
        let mut state = cx.state.borrow_mut();
        state.attempt += 1;
        state.session = None;
        state.attempt
    };
    show(cx.window, "loading");
    cx.window.set_qr_image(slint::Image::default());
    cx.window.set_qr_pin("".into());
    cx.window.set_qr_device("".into());

    let api = (**cx.client).clone();
    let parts = Parts::of(cx);
    tasks::spawn(
        async move { api.qr_sign_in_create(&device_name()).await },
        move |result| {
            let Some(window) = parts.window.upgrade() else {
                return;
            };
            if parts.state.borrow().attempt != attempt {
                return;
            }
            let created = match result {
                Ok(created) if created.code == 0 && !created.key.is_empty() => created,
                Ok(refused) => {
                    tracing::warn!(code = refused.code, "no QR sign-in was started");
                    show(&window, state_name(refused.status()));
                    return;
                }
                Err(error) => {
                    tracing::warn!(%error, "no QR sign-in was started");
                    show(&window, "error");
                    return;
                }
            };
            match image(&created.link()) {
                Some(picture) => window.set_qr_image(picture),
                None => {
                    show(&window, "error");
                    return;
                }
            }
            window.set_qr_pin(created.pin.as_str().into());
            show(&window, "waiting");

            let poll =
                Duration::from_secs(created.poll_interval.max(0).unsigned_abs()).max(MIN_POLL);
            parts.state.borrow_mut().session = Some(Attempt {
                key: created.key,
                secret: created.secret,
                poll,
                expires: Instant::now()
                    + Duration::from_secs(created.expires_in.max(60).unsigned_abs()),
                failures: 0,
            });
            parts.ask_later(attempt, poll);
        },
    );
}

/// Drops the attempt under way: the sheet was closed, or left for the
/// password form.
pub fn stop(state: &Rc<RefCell<State>>) {
    let mut state = state.borrow_mut();
    state.attempt += 1;
    state.session = None;
}

/// What an answer needs once it is back on the interface's thread.
#[derive(Clone)]
struct Parts {
    window: slint::Weak<MainWindow>,
    state: Rc<RefCell<State>>,
    session: Rc<RefCell<Session>>,
    client: Rc<Client>,
    http: reqwest::Client,
}

impl Parts {
    fn of(cx: &Context<'_>) -> Self {
        Self {
            window: cx.window.as_weak(),
            state: Rc::clone(cx.state),
            session: Rc::clone(cx.session),
            client: Rc::clone(cx.client),
            http: cx.http.clone(),
        }
    }

    fn ask_later(self, attempt: u64, after: Duration) {
        slint::Timer::single_shot(after, move || self.ask(attempt));
    }

    /// Asks how the attempt stands, if it is still the one under way.
    fn ask(self, attempt: u64) {
        let (key, secret) = {
            let state = self.state.borrow();
            let Some(current) = state.session.as_ref().filter(|_| state.attempt == attempt) else {
                return;
            };
            if Instant::now() >= current.expires {
                drop(state);
                self.end(attempt, "expired");
                return;
            }
            (current.key.clone(), current.secret.clone())
        };

        let api = (*self.client).clone();
        tasks::spawn(
            async move { api.qr_sign_in_status(&key, &secret).await },
            move |result| self.answered(attempt, result),
        );
    }

    fn answered(self, attempt: u64, result: anirust_api::Result<QrSignIn>) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let poll = {
            let mut state = self.state.borrow_mut();
            if state.attempt != attempt {
                return;
            }
            let Some(current) = state.session.as_mut() else {
                return;
            };
            match &result {
                Ok(_) => current.failures = 0,
                Err(_) => current.failures += 1,
            }
            if current.failures > FAILURES_TOLERATED {
                drop(state);
                self.end(attempt, "error");
                return;
            }
            current.poll
        };

        let answer = match result {
            Ok(answer) => answer,
            Err(error) => {
                tracing::debug!(%error, "a QR sign-in ask failed; asking again");
                self.ask_later(attempt, poll);
                return;
            }
        };

        match answer.status() {
            QrStatus::Waiting => self.ask_later(attempt, poll),
            QrStatus::Scanned => {
                window.set_qr_device(answer.device.as_str().into());
                show(&window, "scanned");
                self.ask_later(attempt, poll);
            }
            QrStatus::Confirmed => {
                let (Some(profile), Some(token)) = (answer.profile, answer.profile_token) else {
                    tracing::warn!("the QR sign-in was confirmed but handed over no session");
                    self.end(attempt, "error");
                    return;
                };
                stop(&self.state);
                let login = profile.login.clone();
                window.set_sign_in_mode("sign-in".into());
                session::signed_in(
                    &window,
                    &self.session,
                    &self.client,
                    self.http,
                    profile,
                    &token,
                    login,
                );
            }
            QrStatus::TooManyRequests => {
                // Not the end of it: slower, then on.
                self.ask_later(attempt, poll * 2);
            }
            other => self.end(attempt, state_name(other)),
        }
    }

    fn end(&self, attempt: u64, how: &str) {
        let mut state = self.state.borrow_mut();
        if state.attempt != attempt {
            return;
        }
        state.session = None;
        drop(state);
        if let Some(window) = self.window.upgrade() {
            show(&window, how);
        }
    }
}

/// The sheet's name for where an attempt stands; see `SignInSheet.qr-state`.
fn state_name(status: QrStatus) -> &'static str {
    match status {
        QrStatus::Waiting => "waiting",
        QrStatus::Scanned => "scanned",
        QrStatus::Confirmed => "loading",
        QrStatus::Declined => "declined",
        QrStatus::Expired => "expired",
        QrStatus::TooManyAttempts => "too-many-attempts",
        QrStatus::TooManyRequests => "too-many-requests",
        QrStatus::Other(_) => "error",
    }
}

fn show(window: &MainWindow, state: &str) {
    window.set_qr_state(state.into());
}

/// What the phone is shown as this device: the program and the system, and
/// nothing about the person.
fn device_name() -> String {
    let system = match std::env::consts::OS {
        "windows" => "Windows",
        "macos" => "macOS",
        "linux" => "Linux",
        other => other,
    };
    format!("AniRust · {system}")
}

/// Light modules around the code, as the standard asks: a scanner finds the
/// code by its edge, and a dark background right up to it hides that.
const QUIET_ZONE: usize = 4;

/// Pixels per module: sharp at any size the sheet shows it, with no smoothing
/// to blur the edges a scanner looks for.
const MODULE_PIXELS: usize = 8;

/// The code for `text`, black on white whatever the theme: scanners expect
/// dark modules on a light ground, and an inverted code is one many refuse.
fn image(text: &str) -> Option<slint::Image> {
    let code = match qrcode::QrCode::new(text.as_bytes()) {
        Ok(code) => code,
        Err(error) => {
            tracing::warn!(%error, "the QR code could not be made");
            return None;
        }
    };
    let modules = code.width();
    let colors = code.to_colors();
    let side = (modules + 2 * QUIET_ZONE) * MODULE_PIXELS;
    let side_u32 = u32::try_from(side).ok()?;
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(side_u32, side_u32);
    let white = Rgba8Pixel::new(255, 255, 255, 255);
    let black = Rgba8Pixel::new(0, 0, 0, 255);
    let pixels = buffer.make_mut_slice();
    for (at, pixel) in pixels.iter_mut().enumerate() {
        let (x, y) = (at % side / MODULE_PIXELS, at / side / MODULE_PIXELS);
        let dark = x >= QUIET_ZONE
            && y >= QUIET_ZONE
            && x < QUIET_ZONE + modules
            && y < QUIET_ZONE + modules
            && colors[(y - QUIET_ZONE) * modules + (x - QUIET_ZONE)] == qrcode::Color::Dark;
        *pixel = if dark { black } else { white };
    }
    Some(slint::Image::from_rgba8(buffer))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_device_is_named_by_program_and_system_only() {
        let name = device_name();
        assert!(name.starts_with("AniRust · "));
        assert!(!name.contains('@'));
    }
}
