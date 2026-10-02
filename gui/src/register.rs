// SPDX-License-Identifier: GPL-3.0-or-later

//! Getting into an account without a password that works: making one, and
//! setting a forgotten one anew.
//!
//! Both go the same way. The first form asks the service to email a code and
//! is answered with a `hash` tying the next step to it; the second sends the
//! code back with that hash, and the service answers with a session, exactly
//! as signing in does. The sign-in sheet holds the forms; this holds what
//! the service handed over between them.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, VecModel};

use anirust_api::{AuthStep, Client};

use crate::session::{self, Session};
use crate::{MainWindow, tasks};

/// What the service handed over at the first step, for the second.
#[derive(Default)]
pub struct Pending {
    hash: String,
    /// What the first step was asked with, so a resend asks the same.
    login: String,
    email: String,
    password: String,
}

/// Which step is being taken. Each answers with the service's own codes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    SignUp,
    SignUpVerify,
    SignUpResend,
    Restore,
    RestoreVerify,
    RestoreResend,
}

/// Everything the sheet's buttons need.
pub struct Context<'a> {
    pub window: &'a MainWindow,
    pub pending: &'a Rc<RefCell<Pending>>,
    pub session: &'a Rc<RefCell<Session>>,
    pub client: &'a Rc<Client>,
    pub http: reqwest::Client,
}

/// Submits whichever form of the sheet is up. Signing in proper is
/// `session::sign_in`'s; this takes the other four.
pub fn submit(cx: &Context<'_>) {
    let window = cx.window;
    let login = window.get_login().trim().to_owned();
    let email = window.get_sign_up_email().trim().to_owned();
    let password = window.get_password().to_string();
    let code = window.get_sign_in_code().trim().to_owned();

    match window.get_sign_in_mode().as_str() {
        "sign-up" => {
            *cx.pending.borrow_mut() = Pending {
                hash: String::new(),
                login,
                email,
                password,
            };
            run(cx, Step::SignUp, String::new());
        }
        "restore" => {
            *cx.pending.borrow_mut() = Pending {
                login,
                ..Pending::default()
            };
            run(cx, Step::Restore, String::new());
        }
        "sign-up-code" => run(cx, Step::SignUpVerify, code),
        "restore-code" => {
            // The new password is typed beside the code.
            cx.pending.borrow_mut().password = password;
            run(cx, Step::RestoreVerify, code);
        }
        _ => {}
    }
}

/// Asks for the emailed code again.
pub fn resend(cx: &Context<'_>) {
    match cx.window.get_sign_in_mode().as_str() {
        "sign-up-code" => run(cx, Step::SignUpResend, String::new()),
        "restore-code" => {
            cx.pending.borrow_mut().password = cx.window.get_password().to_string();
            run(cx, Step::RestoreResend, String::new());
        }
        _ => {}
    }
}

fn run(cx: &Context<'_>, step: Step, code: String) {
    let (login, email, password, hash) = {
        let p = cx.pending.borrow();
        (
            p.login.clone(),
            p.email.clone(),
            p.password.clone(),
            p.hash.clone(),
        )
    };
    cx.window.set_sign_in_busy(true);
    cx.window.set_sign_in_error("".into());

    let weak = cx.window.as_weak();
    let pending = Rc::clone(cx.pending);
    let session = Rc::clone(cx.session);
    let client = Rc::clone(cx.client);
    let api = (**cx.client).clone();
    let http = cx.http.clone();
    tasks::spawn(
        async move {
            match step {
                Step::SignUp => api.sign_up(&login, &email, &password).await,
                Step::SignUpVerify => {
                    api.sign_up_verify(&login, &email, &password, &hash, &code)
                        .await
                }
                Step::SignUpResend => api.sign_up_resend(&login, &email, &password, &hash).await,
                Step::Restore => api.restore(&login).await,
                Step::RestoreVerify => api.restore_verify(&login, &password, &hash, &code).await,
                Step::RestoreResend => api.restore_resend(&login, &password, &hash).await,
            }
        },
        move |result| {
            let Some(window) = weak.upgrade() else { return };
            window.set_sign_in_busy(false);
            let answer = match result {
                Ok(answer) => answer,
                Err(error) => {
                    tracing::warn!(%error, "the account step failed");
                    window.set_sign_in_error("failed".into());
                    return;
                }
            };
            window.set_login_suggestions(slint::ModelRc::new(VecModel::from(
                answer
                    .suggested_logins
                    .iter()
                    .take(3)
                    .map(|s| slint::SharedString::from(s.as_str()))
                    .collect::<Vec<_>>(),
            )));

            // A code already sent is still a code to type in.
            if let Some(key) = refusal(step, answer.code)
                && key != "code-already-sent"
            {
                window.set_sign_in_error(key.into());
                return;
            }

            match step {
                Step::SignUp | Step::Restore | Step::SignUpResend | Step::RestoreResend => {
                    if !answer.hash.is_empty() {
                        pending.borrow_mut().hash = answer.hash;
                    }
                    window.set_sign_in_code("".into());
                    window.set_sign_in_mode(
                        if matches!(step, Step::SignUp | Step::SignUpResend) {
                            "sign-up-code"
                        } else {
                            "restore-code"
                        }
                        .into(),
                    );
                }
                Step::SignUpVerify | Step::RestoreVerify => {
                    finish(&window, &pending, &session, &client, http, answer);
                }
            }
        },
    );
}

/// Takes up the session the last step handed over.
fn finish(
    window: &MainWindow,
    pending: &Rc<RefCell<Pending>>,
    session: &Rc<RefCell<Session>>,
    client: &Client,
    http: reqwest::Client,
    answer: AuthStep,
) {
    let (Some(profile), Some(token)) = (answer.profile, answer.profile_token) else {
        tracing::warn!("the account step succeeded but handed over no session");
        window.set_sign_in_error("start-over".into());
        return;
    };
    let login = std::mem::take(&mut *pending.borrow_mut()).login;
    window.set_sign_in_code("".into());
    window.set_sign_in_mode("sign-in".into());
    session::signed_in(window, session, client, http, profile, &token, login);
}

/// The sheet's name for a code the service refused a step with, or `None`
/// when it did not. The numbers are each step's own.
fn refusal(step: Step, code: i32) -> Option<&'static str> {
    if code == 0 {
        return None;
    }
    Some(match (step, code) {
        (Step::SignUp | Step::SignUpVerify | Step::SignUpResend, 2) => "bad-login",
        (Step::SignUp | Step::SignUpVerify | Step::SignUpResend, 3) => "bad-email",
        (Step::SignUp | Step::SignUpVerify | Step::SignUpResend, 4) => "bad-password",
        (Step::SignUp | Step::SignUpVerify, 5) => "login-taken",
        (Step::SignUpResend, 5) => "start-over",
        (Step::SignUp | Step::SignUpVerify, 6) => "email-taken",
        (Step::SignUpResend, 6) => "cannot-send",
        (Step::SignUp, 7) => "code-already-sent",
        (Step::SignUp, 8) => "cannot-send",
        (Step::SignUp, 9) => "email-service",
        (Step::SignUp, 10) => "too-many",
        (Step::SignUpVerify, 7) => "wrong-code",
        (Step::SignUpVerify, 8) => "code-expired",
        (Step::SignUpVerify, 10) => "email-service",
        (Step::SignUpVerify, 11) => "too-many",
        (Step::Restore | Step::RestoreVerify | Step::RestoreResend, 2) => "not-found",
        (Step::Restore, 3) => "code-already-sent",
        (Step::Restore | Step::RestoreResend, 4) => "cannot-send",
        (Step::RestoreVerify, 3) => "bad-password",
        (Step::RestoreVerify, 4) => "wrong-code",
        (Step::RestoreVerify, 5) => "code-expired",
        _ => "start-over",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_is_no_refusal() {
        assert_eq!(refusal(Step::SignUp, 0), None);
        assert_eq!(refusal(Step::RestoreVerify, 0), None);
    }

    #[test]
    fn the_same_number_means_different_things_per_step() {
        assert_eq!(refusal(Step::SignUp, 7), Some("code-already-sent"));
        assert_eq!(refusal(Step::SignUpVerify, 7), Some("wrong-code"));
        assert_eq!(refusal(Step::Restore, 4), Some("cannot-send"));
        assert_eq!(refusal(Step::RestoreVerify, 4), Some("wrong-code"));
    }

    #[test]
    fn a_bad_hash_starts_over() {
        assert_eq!(refusal(Step::SignUpVerify, 9), Some("start-over"));
        assert_eq!(refusal(Step::RestoreVerify, 6), Some("start-over"));
    }
}
