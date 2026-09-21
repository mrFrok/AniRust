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

use anirust_api::{Client, Profile, SignInError};

use crate::{Account, MainWindow, tasks};

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
pub fn restore(
    window: &MainWindow,
    session: &Rc<RefCell<Session>>,
    client: &Client,
    http: reqwest::Client,
) {
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
                show_profile(&window, &profile, http);
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
    http: reqwest::Client,
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

                    // The sign-in answers with the profile, so the screen is
                    // complete before it is first looked at.
                    let mut profile = profile;
                    if profile.login.is_empty() {
                        profile.login = login;
                    }
                    let name = profile.login.clone();

                    show_profile(&window, &profile, http);
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
    window.set_account(Account::default());
    window.set_avatar_loaded(false);
}

/// Puts a profile on the screen, picture and all.
fn show_profile(window: &MainWindow, profile: &Profile, http: reqwest::Client) {
    window.set_account_name(profile.login.as_str().into());
    window.set_account(Account {
        login: profile.login.as_str().into(),
        status: profile.status.as_str().into(),
        registered: date(profile.register_date).unwrap_or_default().into(),
        verified: profile.is_verified,
        watching: count(profile.watching_count),
        planned: count(profile.plan_count),
        watched: count(profile.completed_count),
        hold: count(profile.hold_on_count),
        dropped: count(profile.dropped_count),
        votes: count(profile.rate_count),
        comments: count(profile.comment_count),
        collections: count(profile.collection_count),
        friends: count(profile.friend_count),
    });

    window.set_avatar_loaded(false);
    tasks::fetch_into(window, http, profile.avatar.clone(), |window, image| {
        window.set_avatar(image);
        window.set_avatar_loaded(true);
    });
}

/// A count as the interface carries it.
///
/// Slint has no 64-bit integer. Nobody has watched two billion of anything, so
/// the saturating cast is a formality — but a silent wrap would not be.
fn count(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// The calendar date of a moment, as `2019-05-12`.
///
/// UTC rather than local time. This is a registration date shown to the day,
/// and a client that names a different day depending on which side of midnight
/// the machine sits would be claiming a precision it does not have.
///
/// Nothing is added to the dependency list for it: the arithmetic below is the
/// standard civil-from-days conversion, and it is four lines longer than the
/// import would have been.
fn date(seconds: i64) -> Option<String> {
    if seconds <= 0 {
        return None;
    }

    let days = seconds.div_euclid(86_400) + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;

    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);

    Some(format!("{year:04}-{month:02}-{day:02}"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_epoch_and_a_known_date() {
        assert_eq!(date(1), Some("1970-01-01".to_owned()));
        assert_eq!(date(1_557_662_400), Some("2019-05-12".to_owned()));
    }

    #[test]
    fn a_leap_day_is_a_day_of_its_own() {
        assert_eq!(date(1_582_934_400), Some("2020-02-29".to_owned()));
        assert_eq!(date(1_583_020_800), Some("2020-03-01".to_owned()));
    }

    #[test]
    fn a_century_that_is_not_a_leap_year() {
        // 1900 was not one, 2000 was: the rule the naive version gets wrong.
        assert_eq!(date(951_782_400), Some("2000-02-29".to_owned()));
    }

    #[test]
    fn an_account_with_no_date_has_none_to_show() {
        assert_eq!(date(0), None);
        assert_eq!(date(-1), None);
    }

    #[test]
    fn a_count_too_large_for_the_interface_stops_at_the_top() {
        assert_eq!(count(12), 12);
        assert_eq!(count(i64::from(i32::MAX) + 1), i32::MAX);
    }
}
