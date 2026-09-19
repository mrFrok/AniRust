// SPDX-License-Identifier: GPL-3.0-or-later

//! Signing in, and keeping the session between runs.
//!
//! The token goes to the platform's secret store — Secret Service, Keychain,
//! Credential Manager — rather than to a file beside the config. It is a bearer
//! credential: anything holding it is the account, for as long as the account
//! lets it be.
//!
//! A machine with no secret store running (a headless Linux box, a session
//! without a keyring daemon) is not an error worth refusing to start over. The
//! session then lasts until the client is closed, and the viewer is told as
//! much by being asked to sign in again next time.

use std::cell::RefCell;
use std::rc::Rc;

use slint::ComponentHandle;

use anirust_api::{Client, SignInError};

use crate::{MainWindow, tasks};

/// What the secret store files this under.
const SERVICE: &str = "dev.anirust.client";
/// One account per installation for now, so the entry has a fixed name rather
/// than the login — which is not known until after it is needed.
const ENTRY: &str = "anixart-token";

/// Who is signed in.
#[derive(Default)]
pub struct Session {
    pub login: String,
    pub id: i64,
    pub authenticated: bool,
}

/// Restores a saved session, if the secret store has one.
///
/// Only sets the token: whether it is still valid is answered by the first
/// request that uses it, and asking up front would delay the screen for a
/// question nothing is waiting on.
pub fn restore(window: &MainWindow, session: &Rc<RefCell<Session>>, client: &Client) {
    let Some((id, token)) = stored_session() else {
        return;
    };

    client.set_token(Some(token));
    {
        let mut session = session.borrow_mut();
        session.authenticated = true;
        session.id = id;
    }
    window.set_signed_in(true);

    // The name is worth a request: the header says who is signed in, and a
    // token alone does not say.
    let weak = window.as_weak();
    let session = Rc::clone(session);
    let api = client.clone();
    let mine = client.clone();

    tasks::spawn(async move { mine.profile(id).await }, move |profile| {
        let Some(window) = weak.upgrade() else { return };
        match profile {
            Ok(profile) => {
                session.borrow_mut().login = profile.login.clone();
                window.set_account_name(profile.login.as_str().into());
            }
            // A token the server no longer accepts is a session that has
            // ended, whatever the store still holds.
            Err(error) => {
                tracing::info!(%error, "the saved session is no longer valid");
                forget();
                sign_out(&window, &session, &api);
            }
        }
    });
}

/// Signs in and, if that works, remembers the token.
pub fn sign_in(
    window: &MainWindow,
    session: &Rc<RefCell<Session>>,
    client: Rc<Client>,
    login: String,
    password: String,
) {
    if login.is_empty() || password.is_empty() {
        return;
    }

    window.set_sign_in_busy(true);
    window.set_sign_in_error("".into());

    let weak = window.as_weak();
    let session = Rc::clone(session);
    let api = (*client).clone();

    tasks::spawn(
        async move {
            let result = api.sign_in(&login, &password).await;
            (result, login)
        },
        move |(result, login)| {
            let Some(window) = weak.upgrade() else { return };
            window.set_sign_in_busy(false);

            match result {
                Ok((profile, token)) => {
                    client.set_token(Some(token.token.clone()));
                    remember(token.id, &token.token);

                    let name = if profile.login.is_empty() {
                        login
                    } else {
                        profile.login
                    };
                    window.set_account_name(name.as_str().into());
                    window.set_signed_in(true);
                    window.set_show_sign_in(false);
                    window.set_password("".into());

                    let mut session = session.borrow_mut();
                    session.login = name;
                    session.id = token.id;
                    session.authenticated = true;
                }
                Err(error) => {
                    tracing::info!(%error, "sign-in refused");
                    window.set_sign_in_error(reason(&error).into());
                }
            }
        },
    );
}

/// Ends the session, here and in the store.
pub fn sign_out(window: &MainWindow, session: &Rc<RefCell<Session>>, client: &Client) {
    client.set_token(None);
    forget();

    *session.borrow_mut() = Session::default();
    window.set_signed_in(false);
    window.set_account_name("".into());
}

/// Which message the sign-in form shows.
///
/// Named rather than spelled out: the strings live in the translations, and
/// this is the one place that knows which of them applies.
fn reason(error: &SignInError) -> &'static str {
    match error {
        SignInError::UnknownLogin => "unknown-login",
        SignInError::WrongPassword => "wrong-password",
        SignInError::Api(_) => "failed",
    }
}

// ---------------------------------------------------------------------------
// The secret store
// ---------------------------------------------------------------------------

fn entry() -> Option<keyring::Entry> {
    match keyring::Entry::new(SERVICE, ENTRY) {
        Ok(entry) => Some(entry),
        Err(error) => {
            tracing::info!(%error, "no secret store; the session will not outlive this run");
            None
        }
    }
}

/// The saved account id and token.
///
/// Stored as one secret with the id in front of it: the account's own name
/// comes from `profile/{id}`, so a token on its own would restore a session
/// that could not say whose it is.
fn stored_session() -> Option<(i64, String)> {
    let secret = match entry()?.get_password() {
        Ok(secret) if !secret.is_empty() => secret,
        Ok(_) | Err(keyring::Error::NoEntry) => return None,
        Err(error) => {
            tracing::info!(%error, "could not read the saved session");
            return None;
        }
    };

    let (id, token) = secret.split_once(':')?;
    Some((id.parse().ok()?, token.to_owned()))
}

fn remember(id: i64, token: &str) {
    let Some(entry) = entry() else { return };
    if let Err(error) = entry.set_password(&format!("{id}:{token}")) {
        tracing::warn!(%error, "could not save the session; it ends with this run");
    }
}

fn forget() {
    let Some(entry) = entry() else { return };
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(error) => tracing::warn!(%error, "could not clear the saved session"),
    }
}
