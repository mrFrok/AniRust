// SPDX-License-Identifier: GPL-3.0-or-later

//! Reporting something to the service's moderators.
//!
//! Each kind of thing has its own list of reasons, which the service keeps;
//! the sheet asks for them when it opens, and sends the chosen one with the
//! thing's id and whatever the viewer wrote.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, VecModel};

use anirust_api::{Client, ReportReason, ReportTarget};

use crate::{MainWindow, tasks};

/// What the open sheet reports, and the reasons it offers.
#[derive(Default)]
pub struct ReportState {
    about: Option<(ReportTarget, i64)>,
    reasons: Vec<ReportReason>,
}

/// Opens the sheet on one thing.
pub fn open(
    window: &MainWindow,
    state: &Rc<RefCell<ReportState>>,
    client: &Client,
    target: ReportTarget,
    id: i64,
    subject: &str,
) {
    if id <= 0 {
        return;
    }
    if !client.is_authenticated() {
        window.set_sign_in_error("".into());
        window.set_show_sign_in(true);
        return;
    }
    *state.borrow_mut() = ReportState {
        about: Some((target, id)),
        reasons: Vec::new(),
    };
    window.set_report_subject(subject.into());
    window.set_report_reasons(slint::ModelRc::new(
        VecModel::<slint::SharedString>::default(),
    ));
    window.set_report_outcome("".into());
    window.set_report_busy(false);
    window.set_report_loading(true);
    window.set_report_open(true);

    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = client.clone();
    tasks::spawn(
        async move { api.report_reasons(target).await },
        move |result| {
            let Some(window) = weak.upgrade() else { return };
            if state.borrow().about != Some((target, id)) {
                return;
            }
            window.set_report_loading(false);
            match result {
                Ok(reasons) => {
                    let names: Vec<slint::SharedString> =
                        reasons.iter().map(|r| r.name.as_str().into()).collect();
                    window.set_report_reasons(slint::ModelRc::new(VecModel::from(names)));
                    state.borrow_mut().reasons = reasons;
                }
                Err(error) => {
                    tracing::warn!(%error, "the report reasons were not loaded");
                    window.set_report_outcome("failed".into());
                }
            }
        },
    );
}

/// Sends the report with the reason at a row of the sheet.
pub fn send(
    window: &MainWindow,
    state: &Rc<RefCell<ReportState>>,
    client: &Client,
    reason: usize,
    message: String,
) {
    let (Some((target, id)), Some(reason)) = ({
        let state = state.borrow();
        (state.about, state.reasons.get(reason).map(|r| r.id))
    }) else {
        return;
    };
    window.set_report_busy(true);
    window.set_report_outcome("".into());
    let weak = window.as_weak();
    let api = client.clone();
    let message = message.trim().to_owned();
    tasks::spawn(
        async move { api.report(target, id, reason, &message).await },
        move |result| {
            let Some(window) = weak.upgrade() else { return };
            window.set_report_busy(false);
            match result {
                Ok(()) => window.set_report_outcome("sent".into()),
                Err(error) => {
                    tracing::warn!(%error, id, "the report was not sent");
                    window.set_report_outcome("failed".into());
                }
            }
        },
    );
}
