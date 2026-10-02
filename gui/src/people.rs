// SPDX-License-Identifier: GPL-3.0-or-later

//! Other people: opening someone's profile, being friends, blocking.
//!
//! Someone else's profile is drawn by the same page as the account's own,
//! told it is not the account's: the account's settings, history and
//! sign-out stay out of it, and a friend button and a block appear. Going
//! back puts the account's own profile up again.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, Model, VecModel};

use anirust_api::{Client, FriendStatus, Profile};

use crate::{MainWindow, PersonItem, session, tasks};

/// Whose profile is up, and the people listed on it.
#[derive(Default)]
pub struct PeopleState {
    /// The profile on screen when it is not the account's own.
    viewing: Option<Profile>,
    /// The friends shown on the profile page, in order.
    friends: Vec<Profile>,
    /// Requests waiting on the account, in order.
    requests: Vec<Profile>,
}

/// Opens someone's profile by id. The account's own id opens the account's
/// own profile, which is the one with its settings on it.
pub fn open(
    window: &MainWindow,
    state: &Rc<RefCell<PeopleState>>,
    client: &Client,
    http: reqwest::Client,
    me: i64,
    id: i64,
) {
    if id <= 0 {
        return;
    }
    if id == me {
        back_to_mine(window, state);
        return;
    }
    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = client.clone();
    tasks::spawn(async move { api.profile(id).await }, move |result| {
        let Some(window) = weak.upgrade() else { return };
        match result {
            Ok(profile) => {
                window.set_profile_is_mine(false);
                show(&window, &state, profile, http);
            }
            Err(error) => tracing::warn!(%error, id, "the profile could not be opened"),
        }
    });
}

/// Back to the account's own profile. The caller refreshes it.
pub fn back_to_mine(window: &MainWindow, state: &Rc<RefCell<PeopleState>>) {
    state.borrow_mut().viewing = None;
    window.set_profile_is_mine(true);
}

fn show(
    window: &MainWindow,
    state: &Rc<RefCell<PeopleState>>,
    profile: Profile,
    http: reqwest::Client,
) {
    session::show_account(window, &profile, http.clone());
    let friends = profile.friends_preview.clone();
    {
        let mut state = state.borrow_mut();
        state.viewing = Some(profile);
    }
    show_friends(window, state, friends, &http);
}

/// Puts the friends of whoever's profile is up on the page.
pub fn show_friends(
    window: &MainWindow,
    state: &Rc<RefCell<PeopleState>>,
    friends: Vec<Profile>,
    http: &reqwest::Client,
) {
    let model = people_model(&friends, http);
    window.set_profile_friends(slint::ModelRc::from(model));
    state.borrow_mut().friends = friends;
}

/// Loads the requests waiting on the account, for its own profile.
pub fn load_requests(
    window: &MainWindow,
    state: &Rc<RefCell<PeopleState>>,
    client: &Client,
    http: reqwest::Client,
) {
    if !client.is_authenticated() {
        window.set_friend_requests(slint::ModelRc::new(VecModel::<PersonItem>::default()));
        return;
    }
    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = client.clone();
    tasks::spawn(
        async move { api.friend_requests_in(0).await },
        move |result| {
            let Some(window) = weak.upgrade() else { return };
            match result {
                Ok(page) => {
                    let model = people_model(&page.content, &http);
                    window.set_friend_requests(slint::ModelRc::from(model));
                    state.borrow_mut().requests = page.content;
                }
                Err(error) => tracing::debug!(%error, "the friend requests were not loaded"),
            }
        },
    );
}

/// The person at a row of the friends list, or of the requests.
#[must_use]
pub fn friend_at(state: &Rc<RefCell<PeopleState>>, index: usize) -> Option<i64> {
    state.borrow().friends.get(index).map(|p| p.id)
}

#[must_use]
pub fn request_at(state: &Rc<RefCell<PeopleState>>, index: usize) -> Option<i64> {
    state.borrow().requests.get(index).map(|p| p.id)
}

/// The one friend button on someone's profile does whatever the two accounts'
/// standing calls for: ask, accept, withdraw the request, or unfriend.
pub fn friend_action(
    window: &MainWindow,
    state: &Rc<RefCell<PeopleState>>,
    client: &Client,
    http: reqwest::Client,
) {
    let Some((id, status)) = state
        .borrow()
        .viewing
        .as_ref()
        .map(|p| (p.id, p.friend_status))
    else {
        return;
    };
    let asking = matches!(status, None | Some(FriendStatus::RequestReceived));
    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = client.clone();
    let again = client.clone();
    tasks::spawn(
        async move {
            if asking {
                api.friend_request_send(id).await.map(|_| ())
            } else {
                api.friend_request_remove(id).await.map(|_| ())
            }
        },
        move |result| {
            if let Err(error) = result {
                tracing::warn!(%error, id, "the friend request was refused");
            }
            // Either way, the profile is read again: the standing is the
            // server's, and only it knows which way this went.
            if let Some(window) = weak.upgrade() {
                reload(&window, &state, &again, http);
            }
        },
    );
}

/// Answers a request on the account's own profile: accept or decline.
pub fn answer_request(
    window: &MainWindow,
    state: &Rc<RefCell<PeopleState>>,
    client: &Client,
    http: reqwest::Client,
    index: usize,
    accept: bool,
) {
    let Some(id) = request_at(state, index) else {
        return;
    };

    // Off the list at once; the list is read again afterwards either way.
    {
        let mut guard = state.borrow_mut();
        guard.requests.remove(index);
    }
    let model = window.get_friend_requests();
    if let Some(rows) = model.as_any().downcast_ref::<VecModel<PersonItem>>()
        && index < rows.row_count()
    {
        rows.remove(index);
    }

    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = client.clone();
    let again = client.clone();
    tasks::spawn(
        async move {
            if accept {
                api.friend_request_send(id).await.map(|_| ())
            } else {
                api.friend_request_remove(id).await.map(|_| ())
            }
        },
        move |result| {
            if let Err(error) = result {
                tracing::warn!(%error, id, "the friend request was not answered");
            }
            if let Some(window) = weak.upgrade() {
                load_requests(&window, &state, &again, http);
            }
        },
    );
}

/// Blocks or unblocks whoever's profile is up.
pub fn toggle_block(
    window: &MainWindow,
    state: &Rc<RefCell<PeopleState>>,
    client: &Client,
    http: reqwest::Client,
) {
    let Some((id, blocked)) = state
        .borrow()
        .viewing
        .as_ref()
        .map(|p| (p.id, p.is_blocked))
    else {
        return;
    };
    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = client.clone();
    let again = client.clone();
    tasks::spawn(
        async move {
            if blocked {
                api.unblock(id).await
            } else {
                api.block(id).await
            }
        },
        move |result| {
            if let Err(error) = result {
                tracing::warn!(%error, id, "the block was not changed");
            }
            if let Some(window) = weak.upgrade() {
                reload(&window, &state, &again, http);
            }
        },
    );
}

/// Reads the profile on screen again.
fn reload(
    window: &MainWindow,
    state: &Rc<RefCell<PeopleState>>,
    client: &Client,
    http: reqwest::Client,
) {
    let Some(id) = state.borrow().viewing.as_ref().map(|p| p.id) else {
        return;
    };
    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = client.clone();
    tasks::spawn(async move { api.profile(id).await }, move |result| {
        let Some(window) = weak.upgrade() else { return };
        // Still that person's page? The viewer may have gone back meanwhile.
        if state.borrow().viewing.as_ref().map(|p| p.id) != Some(id) {
            return;
        }
        if let Ok(profile) = result {
            show(&window, &state, profile, http);
        }
    });
}

/// Rows for a list of people, with their pictures fetched as they come.
pub(crate) fn people_model(people: &[Profile], http: &reqwest::Client) -> Rc<VecModel<PersonItem>> {
    faces(
        people
            .iter()
            .map(|p| (p.login.as_str(), p.avatar.as_str(), p.is_online)),
        http,
    )
}

/// Rows of a name and a picture — people, or channels drawn the same way —
/// with the pictures fetched as they come.
pub(crate) fn faces<'a>(
    rows: impl Iterator<Item = (&'a str, &'a str, bool)>,
    http: &reqwest::Client,
) -> Rc<VecModel<PersonItem>> {
    let rows: Vec<_> = rows.collect();
    let model = Rc::new(VecModel::from(
        rows.iter()
            .map(|(name, _, online)| PersonItem {
                login: (*name).into(),
                avatar: slint::Image::default(),
                avatar_loaded: false,
                online: *online,
            })
            .collect::<Vec<_>>(),
    ));
    for (index, (_, avatar, _)) in rows.iter().enumerate() {
        if !avatar.starts_with("http") {
            continue;
        }
        let model = Rc::clone(&model);
        tasks::spawn(
            tasks::fetch_image(http.clone(), (*avatar).to_owned()),
            move |result| {
                let Ok(buffer) = result else { return };
                if let Some(mut row) = model.row_data(index) {
                    row.avatar = slint::Image::from_rgba8(buffer);
                    row.avatar_loaded = true;
                    model.set_row_data(index, row);
                }
            },
        );
    }
    model
}
