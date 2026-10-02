// SPDX-License-Identifier: GPL-3.0-or-later

//! The account's own settings: its picture and status, who sees what, its
//! links elsewhere, and its login, password and email.
//!
//! Every change goes to the server at once and the sheet says how it went in
//! one line at its foot. The privacy choices and incognito are drawn as the
//! server has them and flipped before it answers, then put back if it
//! refuses; the rest wait for the answer, because a refusal there has
//! something to say — a taken login, a wrong password.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, VecModel};

use anirust_api::{Client, Preferences, Privacy, SettingStep, Socials};

use crate::people::PeopleState;
use crate::session::{self, Session};
use crate::{AccountSettings, MainWindow, tasks};

/// An email change between its two steps.
#[derive(Default)]
pub struct EmailChange {
    current: String,
    password: String,
    new: String,
    hash: String,
}

/// Everything the sheet's buttons need.
pub struct Context<'a> {
    pub window: &'a MainWindow,
    pub email: &'a Rc<RefCell<EmailChange>>,
    pub session: &'a Rc<RefCell<Session>>,
    pub people: &'a Rc<RefCell<PeopleState>>,
    pub client: &'a Rc<Client>,
    pub http: reqwest::Client,
}

/// Opens the sheet and reads the settings into it.
pub fn open(cx: &Context<'_>) {
    if !cx.client.is_authenticated() {
        return;
    }
    let window = cx.window;
    window.set_settings_message("".into());
    window.set_settings_busy(false);
    window.set_email_code_pending(false);
    window.set_settings_login_suggestions(slint::ModelRc::new(
        VecModel::<slint::SharedString>::default(),
    ));
    window.set_settings_open(true);
    reload(window, cx.client);
}

fn reload(window: &MainWindow, client: &Client) {
    window.set_settings_loading(true);
    let weak = window.as_weak();
    let api = client.clone();
    tasks::spawn(
        async move { (api.preferences().await, api.login_change_info().await) },
        move |(preferences, login)| {
            let Some(window) = weak.upgrade() else { return };
            window.set_settings_loading(false);
            match preferences {
                Ok(preferences) => {
                    let mut shown = settings_for(&preferences);
                    if let Ok(login) = login {
                        shown.login_change_available = login.is_change_available;
                        shown.login_next_change = session::date(login.next_change_available_at)
                            .unwrap_or_default()
                            .into();
                    }
                    window.set_account_settings(shown);
                }
                Err(error) => {
                    tracing::warn!(%error, "the settings were not loaded");
                    window.set_settings_message("failed".into());
                }
            }
        },
    );
}

fn settings_for(p: &Preferences) -> AccountSettings {
    AccountSettings {
        status: p.status.as_str().into(),
        email_hint: p.email_hint.as_str().into(),
        vk: p.vk_page.as_str().into(),
        telegram: p.tg_page.as_str().into(),
        instagram: p.inst_page.as_str().into(),
        tiktok: p.tt_page.as_str().into(),
        discord: p.discord_page.as_str().into(),
        privacy_counts: p.privacy_counts,
        privacy_stats: p.privacy_stats,
        privacy_social: p.privacy_social,
        privacy_friend_requests: p.privacy_friend_requests,
        incognito: p.is_incognito,
        // Until the server says otherwise; a refusal there says why.
        login_change_available: !p.is_change_login_banned,
        login_next_change: slint::SharedString::default(),
        avatar_banned: p.is_change_avatar_banned,
    }
}

/// Runs one change, says how it went, and hands a success on.
fn change<T: Send + 'static>(
    window: &MainWindow,
    work: impl Future<Output = anirust_api::Result<T>> + Send + 'static,
    done: impl FnOnce(&MainWindow, T) + 'static,
) {
    window.set_settings_busy(true);
    window.set_settings_message("".into());
    let weak = window.as_weak();
    tasks::spawn(work, move |result| {
        let Some(window) = weak.upgrade() else { return };
        window.set_settings_busy(false);
        match result {
            Ok(value) => done(&window, value),
            Err(error) => {
                tracing::warn!(%error, "a settings change failed");
                window.set_settings_message("failed".into());
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Picture and status
// ---------------------------------------------------------------------------

/// Asks for a picture with the platform's own chooser, and uploads it.
pub fn change_avatar(cx: &Context<'_>) {
    let weak = cx.window.as_weak();
    let session = Rc::clone(cx.session);
    let people = Rc::clone(cx.people);
    let client = Rc::clone(cx.client);
    let http = cx.http.clone();
    let title = if cx.window.get_lang() == "ru" {
        "Аватар"
    } else {
        "Picture"
    };
    // The chooser is opened from the UI thread, which is the one macOS
    // insists on; the rest waits on it without holding anything up.
    let picked = slint::spawn_local(async move {
        let Some(file) = rfd::AsyncFileDialog::new()
            .set_title(title)
            .add_filter("Image", &["png", "jpg", "jpeg", "webp", "gif"])
            .pick_file()
            .await
        else {
            return;
        };
        let name = file.file_name();
        let bytes = file.read().await;
        let Some(window) = weak.upgrade() else { return };
        let Some(mime) = mime_of(&name) else {
            window.set_settings_message("avatar-unreadable".into());
            return;
        };
        if bytes.is_empty() {
            window.set_settings_message("avatar-unreadable".into());
            return;
        }
        let api = (*client).clone();
        change(
            &window,
            async move { api.avatar_edit(&name, mime, bytes).await },
            move |window, ()| {
                window.set_settings_message("avatar-saved".into());
                session::refresh_profile(window, &session, &people, &client, http);
            },
        );
    });
    if let Err(error) = picked {
        tracing::warn!(%error, "the file chooser could not be opened");
    }
}

/// The picture's type, by its name; `None` for what the server will not take.
pub(crate) fn mime_of(name: &str) -> Option<&'static str> {
    let extension = name.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => return None,
    })
}

pub fn delete_avatar(cx: &Context<'_>) {
    let api = (**cx.client).clone();
    let (session, people, client) = (
        Rc::clone(cx.session),
        Rc::clone(cx.people),
        Rc::clone(cx.client),
    );
    let http = cx.http.clone();
    change(
        cx.window,
        async move { api.avatar_delete().await },
        move |window, ()| {
            window.set_settings_message("saved".into());
            window.set_avatar_loaded(false);
            session::refresh_profile(window, &session, &people, &client, http);
        },
    );
}

pub fn save_status(cx: &Context<'_>, status: String) {
    let api = (**cx.client).clone();
    let wanted = status.trim().to_owned();
    let sent = wanted.clone();
    change(
        cx.window,
        async move { api.status_edit(&sent).await },
        move |window, ()| {
            window.set_settings_message("saved".into());
            let mut settings = window.get_account_settings();
            settings.status = wanted.as_str().into();
            window.set_account_settings(settings);
            let mut account = window.get_account();
            account.status = wanted.into();
            window.set_account(account);
        },
    );
}

// ---------------------------------------------------------------------------
// Who sees what
// ---------------------------------------------------------------------------

/// Changes one privacy setting: 0 counts, 1 statistics, 2 links, 3 friend
/// requests.
pub fn set_privacy(cx: &Context<'_>, what: i32, value: i32) {
    let (what, limit) = match what {
        0 => (Privacy::Counts, 2),
        1 => (Privacy::Stats, 2),
        2 => (Privacy::Social, 2),
        3 => (Privacy::FriendRequests, 1),
        _ => return,
    };
    if !(0..=limit).contains(&value) {
        return;
    }
    let before = privacy_of(&cx.window.get_account_settings(), what);
    put_privacy(cx.window, what, value);

    let weak = cx.window.as_weak();
    let api = (**cx.client).clone();
    tasks::spawn(
        async move { api.privacy_edit(what, value).await },
        move |result| {
            let Some(window) = weak.upgrade() else { return };
            if let Err(error) = result {
                tracing::warn!(%error, "the privacy setting was not changed");
                put_privacy(&window, what, before);
                window.set_settings_message("failed".into());
            }
        },
    );
}

fn privacy_of(settings: &AccountSettings, what: Privacy) -> i32 {
    match what {
        Privacy::Counts => settings.privacy_counts,
        Privacy::Stats => settings.privacy_stats,
        Privacy::Social => settings.privacy_social,
        Privacy::FriendRequests => settings.privacy_friend_requests,
    }
}

fn put_privacy(window: &MainWindow, what: Privacy, value: i32) {
    let mut settings = window.get_account_settings();
    match what {
        Privacy::Counts => settings.privacy_counts = value,
        Privacy::Stats => settings.privacy_stats = value,
        Privacy::Social => settings.privacy_social = value,
        Privacy::FriendRequests => settings.privacy_friend_requests = value,
    }
    window.set_account_settings(settings);
}

pub fn toggle_incognito(cx: &Context<'_>) {
    let put = |window: &MainWindow, on: bool| {
        let mut settings = window.get_account_settings();
        settings.incognito = on;
        window.set_account_settings(settings);
    };
    let now = !cx.window.get_account_settings().incognito;
    put(cx.window, now);

    let weak = cx.window.as_weak();
    let api = (**cx.client).clone();
    tasks::spawn(async move { api.incognito_switch().await }, move |result| {
        let Some(window) = weak.upgrade() else { return };
        if let Err(error) = result {
            tracing::warn!(%error, "incognito was not switched");
            put(&window, !now);
            window.set_settings_message("failed".into());
        }
    });
}

pub fn save_socials(cx: &Context<'_>, pages: [String; 5]) {
    let [vk, tg, inst, tt, discord] = pages.map(|page| page.trim().to_owned());
    let socials = Socials {
        vk_page: vk,
        tg_page: tg,
        inst_page: inst,
        tt_page: tt,
        discord_page: discord,
    };
    let api = (**cx.client).clone();
    let client = Rc::clone(cx.client);
    change(
        cx.window,
        async move { api.socials_edit(&socials).await },
        move |window, ()| {
            window.set_settings_message("saved".into());
            reload(window, &client);
        },
    );
}

// ---------------------------------------------------------------------------
// Login, password, email
// ---------------------------------------------------------------------------

pub fn change_login(cx: &Context<'_>, login: String) {
    let login = login.trim().to_owned();
    if login.is_empty() {
        return;
    }
    let api = (**cx.client).clone();
    let (session, people, client) = (
        Rc::clone(cx.session),
        Rc::clone(cx.people),
        Rc::clone(cx.client),
    );
    let http = cx.http.clone();
    let wanted = login.clone();
    change(
        cx.window,
        async move { api.login_change(&wanted).await },
        move |window, step: SettingStep| {
            window.set_settings_login_suggestions(slint::ModelRc::new(VecModel::from(
                step.suggested_logins
                    .iter()
                    .take(3)
                    .map(|s| slint::SharedString::from(s.as_str()))
                    .collect::<Vec<_>>(),
            )));
            let message = match step.code {
                0 => "login-saved",
                2 => "login-bad",
                3 => "login-taken",
                4 => "login-too-soon",
                _ => "failed",
            };
            window.set_settings_message(message.into());
            if step.code == 0 {
                session.borrow_mut().login = login;
                session::refresh_profile(window, &session, &people, &client, http);
                reload(window, &client);
            }
        },
    );
}

/// Changes the password. The service ends the session with it and hands
/// over a new one, which replaces the stored token.
pub fn change_password(cx: &Context<'_>, current: String, new: String) {
    let api = (**cx.client).clone();
    let (session, client) = (Rc::clone(cx.session), Rc::clone(cx.client));
    change(
        cx.window,
        async move { api.password_change(&current, &new).await },
        move |window, step: SettingStep| {
            let message = match step.code {
                0 => "password-saved",
                2 => "password-bad",
                3 => "password-wrong",
                _ => "failed",
            };
            window.set_settings_message(message.into());
            if step.code == 0 && !step.token.is_empty() {
                client.set_token(Some(step.token.clone()));
                session::remember(session.borrow().id, &step.token);
            }
        },
    );
}

/// Starts changing the email: the service sends a code to the new one.
pub fn change_email(cx: &Context<'_>, current: String, password: String, new: String) {
    *cx.email.borrow_mut() = EmailChange {
        current: current.trim().to_owned(),
        password,
        new: new.trim().to_owned(),
        hash: String::new(),
    };
    send_email_code(cx, false);
}

pub fn resend_email(cx: &Context<'_>) {
    send_email_code(cx, true);
}

fn send_email_code(cx: &Context<'_>, again: bool) {
    let (current, password, new, hash) = {
        let e = cx.email.borrow();
        (
            e.current.clone(),
            e.password.clone(),
            e.new.clone(),
            e.hash.clone(),
        )
    };
    let api = (**cx.client).clone();
    let email = Rc::clone(cx.email);
    change(
        cx.window,
        async move {
            if again {
                api.email_change_resend(&current, &password, &new, &hash)
                    .await
            } else {
                api.email_change(&current, &password, &new).await
            }
        },
        move |window, step: SettingStep| {
            // 6 is a code already sent, which is still a code to type in.
            let message = match (again, step.code) {
                (_, 0) | (false, 6) => "email-code-sent",
                (false, 2) => "email-wrong-password",
                (false, 3) => "email-wrong-current",
                (false, 4) => "email-bad",
                (false, 5) => "email-taken",
                (false, 7) | (true, 6) => "cannot-send",
                _ => "failed",
            };
            window.set_settings_message(message.into());
            if message == "email-code-sent" {
                if !step.hash.is_empty() {
                    email.borrow_mut().hash = step.hash;
                }
                window.set_email_code_pending(true);
            }
        },
    );
}

/// Finishes changing the email with the code sent to it.
pub fn verify_email(cx: &Context<'_>, code: String) {
    let (new, hash) = {
        let e = cx.email.borrow();
        (e.new.clone(), e.hash.clone())
    };
    let code = code.trim().to_owned();
    let api = (**cx.client).clone();
    let (email, client) = (Rc::clone(cx.email), Rc::clone(cx.client));
    change(
        cx.window,
        async move { api.email_change_verify(&new, &code, &hash).await },
        move |window, step: SettingStep| {
            if step.code != 0 {
                window.set_settings_message("wrong-code".into());
                return;
            }
            *email.borrow_mut() = EmailChange::default();
            window.set_email_code_pending(false);
            window.set_settings_message("email-saved".into());
            reload(window, &client);
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_are_known_by_their_names() {
        assert_eq!(mime_of("me.PNG"), Some("image/png"));
        assert_eq!(mime_of("me.jpeg"), Some("image/jpeg"));
        assert_eq!(mime_of("me.webp"), Some("image/webp"));
        assert_eq!(mime_of("me.bmp"), None);
        assert_eq!(mime_of("me"), None);
    }
}
