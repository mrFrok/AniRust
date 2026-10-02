// SPDX-License-Identifier: GPL-3.0-or-later

//! The account's standing with the service — its bans and the sanctions
//! against it, with their appeals — and deleting the account.
//!
//! Deletion is never immediate: the service holds a request for a while and
//! it can be cancelled until then. The sheet that asks for it wants the login
//! typed out and the password, and then a second press.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, VecModel};

use anirust_api::{AppealStatus, Client, Deletion, Enforcement, Health};

use crate::{EnforcementItem, MainWindow, session, tasks};

/// The sanctions on screen, in their order, for appealing one by its row.
#[derive(Default)]
pub struct Standing {
    enforcements: Vec<Enforcement>,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_secs()).unwrap_or(0))
}

/// Reads the account's standing into the settings sheet.
pub fn load(window: &MainWindow, state: &Rc<RefCell<Standing>>, client: &Client) {
    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = client.clone();
    tasks::spawn(
        async move {
            let health = api.health().await;
            let mut all = api.enforcements_account(0).await.unwrap_or_default();
            all.extend(api.enforcements_content(0).await.unwrap_or_default());
            (health, all)
        },
        move |(health, enforcements)| {
            let Some(window) = weak.upgrade() else { return };
            let russian = window.get_lang() == "ru";
            window.set_standing(match health {
                Ok(health) => standing_line(&health, russian).into(),
                Err(error) => {
                    tracing::debug!(%error, "the account's standing was not loaded");
                    "".into()
                }
            });
            let now = now();
            let items: Vec<EnforcementItem> = enforcements
                .iter()
                .map(|e| EnforcementItem {
                    reason: e.reason.as_str().into(),
                    date: session::date(e.creation_timestamp)
                        .unwrap_or_default()
                        .into(),
                    status: status_key(e).into(),
                    answer: e.appeal_process_message.as_str().into(),
                    can_appeal: e.can_appeal(now),
                })
                .collect();
            window.set_enforcements(slint::ModelRc::new(VecModel::from(items)));
            state.borrow_mut().enforcements = enforcements;
        },
    );
}

/// The standing in one line. Worded here rather than in the interface,
/// which would need the dates as well as the numbers.
fn standing_line(health: &Health, russian: bool) -> String {
    let until = [
        health.last_ban_expires,
        health.blog_mute_expires,
        health.blog_suspension_expires,
    ]
    .into_iter()
    .filter(|until| *until > now())
    .max()
    .and_then(session::date);
    match (health.ban_count, until, russian) {
        (0, None, true) => "Нарушений нет.".to_owned(),
        (0, None, false) => "No sanctions.".to_owned(),
        (bans, None, true) => format!("Блокировок: {bans}."),
        (bans, None, false) => format!("Bans: {bans}."),
        (bans, Some(until), true) => format!("Блокировок: {bans}. Ограничение до {until}."),
        (bans, Some(until), false) => format!("Bans: {bans}. Restricted until {until}."),
    }
}

fn status_key(e: &Enforcement) -> &'static str {
    if e.is_revoked {
        return "revoked";
    }
    match e.appeal() {
        AppealStatus::Submitted => "submitted",
        AppealStatus::Accepted => "accepted",
        AppealStatus::Rejected => "rejected",
        AppealStatus::Unknown | AppealStatus::NotSubmitted => "",
    }
}

/// Appeals the sanction at a row with the viewer's words.
pub fn appeal(
    window: &MainWindow,
    state: &Rc<RefCell<Standing>>,
    client: &Client,
    index: usize,
    message: String,
) {
    let Some(id) = state.borrow().enforcements.get(index).map(|e| e.id) else {
        return;
    };
    let message = message.trim().to_owned();
    if message.is_empty() {
        return;
    }
    window.set_settings_busy(true);
    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = client.clone();
    let again = client.clone();
    tasks::spawn(
        async move { api.enforcement_appeal(id, &message).await },
        move |result| {
            let Some(window) = weak.upgrade() else { return };
            window.set_settings_busy(false);
            match result {
                Ok(()) => {
                    window.set_settings_message("appeal-sent".into());
                    load(&window, &state, &again);
                }
                Err(error) => {
                    tracing::warn!(%error, id, "the appeal was not sent");
                    window.set_settings_message("failed".into());
                }
            }
        },
    );
}

// ---------------------------------------------------------------------------
// Deleting the account
// ---------------------------------------------------------------------------

/// Opens the deletion sheet, reading whether a request is already in.
pub fn open_deletion(window: &MainWindow, client: &Client) {
    window.set_deletion_error("".into());
    window.set_deletion_busy(false);
    window.set_deletion_loading(true);
    window.set_deletion_open(true);
    let weak = window.as_weak();
    let api = client.clone();
    tasks::spawn(async move { api.deletion_status().await }, move |result| {
        let Some(window) = weak.upgrade() else { return };
        window.set_deletion_loading(false);
        match result {
            Ok(status) => show_deletion(&window, &status),
            Err(error) => {
                tracing::warn!(%error, "the deletion status was not read");
                window.set_deletion_error("failed".into());
            }
        }
    });
}

fn show_deletion(window: &MainWindow, status: &Deletion) {
    let pending = status.delete_at > 0;
    window.set_deletion_pending(pending);
    window.set_deletion_at(session::date(status.delete_at).unwrap_or_default().into());
}

/// Asks for the account to be deleted. The sheet has had the login typed
/// out, the password, and a second press.
pub fn request(window: &MainWindow, client: &Client, password: String) {
    window.set_deletion_busy(true);
    window.set_deletion_error("".into());
    let weak = window.as_weak();
    let api = client.clone();
    tasks::spawn(
        async move { api.deletion_request(&password).await },
        move |result| {
            let Some(window) = weak.upgrade() else { return };
            window.set_deletion_busy(false);
            match result {
                // 2 is a request already in, which is where this was going.
                Ok(status) if status.code == 0 || status.code == 2 => {
                    show_deletion(&window, &status);
                    window.set_deletion_pending(true);
                }
                Ok(status) => window.set_deletion_error(
                    match status.code {
                        5 => "in-progress",
                        6 => "wrong-password",
                        _ => "failed",
                    }
                    .into(),
                ),
                Err(error) => {
                    tracing::warn!(%error, "the deletion was not requested");
                    window.set_deletion_error("failed".into());
                }
            }
        },
    );
}

pub fn cancel(window: &MainWindow, client: &Client) {
    window.set_deletion_busy(true);
    window.set_deletion_error("".into());
    let weak = window.as_weak();
    let api = client.clone();
    tasks::spawn(async move { api.deletion_cancel().await }, move |result| {
        let Some(window) = weak.upgrade() else { return };
        window.set_deletion_busy(false);
        match result {
            Ok(status) if status.code == 0 => {
                window.set_deletion_pending(false);
                window.set_deletion_at("".into());
            }
            Ok(status) => {
                tracing::warn!(code = status.code, "the deletion was not cancelled");
                window.set_deletion_error(
                    if status.code == 5 {
                        "in-progress"
                    } else {
                        "failed"
                    }
                    .into(),
                );
            }
            Err(error) => {
                tracing::warn!(%error, "the deletion was not cancelled");
                window.set_deletion_error("failed".into());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_record_says_so() {
        assert_eq!(standing_line(&Health::default(), true), "Нарушений нет.");
        assert_eq!(standing_line(&Health::default(), false), "No sanctions.");
    }

    #[test]
    fn past_bans_are_counted_without_a_date() {
        let health = Health {
            ban_count: 2,
            last_ban_expires: 1,
            ..Health::default()
        };
        assert_eq!(standing_line(&health, false), "Bans: 2.");
    }
}
