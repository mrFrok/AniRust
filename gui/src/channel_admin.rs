// SPDX-License-Identifier: GPL-3.0-or-later

//! Running a channel: making one, its settings and pictures, the posts its
//! readers suggest, its administrators, and who is blocked from it.
//!
//! Permission levels are the service's: 0 a member, 1 an administrator,
//! 2 the creator. Only the creator names or removes administrators.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, VecModel};

use anirust_api::{
    Article, Channel, ChannelBlockRequest, ChannelProfile, ChannelSettings, Client, ProfileCompact,
};

use crate::{MainWindow, SuggestionItem, people, tasks};

const ADMINISTRATOR: i32 = 1;

/// The channel being run, and the lists on its tabs.
#[derive(Default)]
pub struct AdminState {
    channel: Option<Channel>,
    suggested: Vec<Article>,
    admins: Vec<ProfileCompact>,
    blocked: Vec<ProfileCompact>,
    found: Vec<ProfileCompact>,
}

/// Everything the sheet's buttons need.
pub struct Context<'a> {
    pub window: &'a MainWindow,
    pub state: &'a Rc<RefCell<AdminState>>,
    pub client: &'a Rc<Client>,
    pub http: reqwest::Client,
}

fn reset(window: &MainWindow) {
    window.set_admin_message("".into());
    window.set_admin_busy(false);
    window.set_admin_loading(false);
    window.set_admin_tab(0);
    let none = || slint::ModelRc::new(VecModel::<crate::PersonItem>::default());
    window.set_admin_admins(none());
    window.set_admin_blocked(none());
    window.set_admin_found(none());
    window.set_admin_suggested(slint::ModelRc::new(VecModel::<SuggestionItem>::default()));
}

/// The sheet on a new channel: settings only.
pub fn create(cx: &Context<'_>) {
    *cx.state.borrow_mut() = AdminState::default();
    reset(cx.window);
    cx.window.set_admin_creating(true);
    cx.window.set_admin_creator(true);
    cx.window.set_admin_title("".into());
    cx.window.set_admin_description("".into());
    cx.window.set_admin_commenting(true);
    cx.window.set_admin_suggestions(false);
    cx.window.set_admin_open(true);
}

/// The sheet on a channel the account runs.
pub fn open(cx: &Context<'_>, channel: Channel) {
    reset(cx.window);
    cx.window.set_admin_creating(false);
    cx.window.set_admin_creator(channel.is_creator);
    cx.window.set_admin_title(channel.title.as_str().into());
    cx.window
        .set_admin_description(channel.description.as_str().into());
    cx.window
        .set_admin_commenting(channel.is_commenting_enabled);
    cx.window
        .set_admin_suggestions(channel.is_article_suggestion_enabled);
    *cx.state.borrow_mut() = AdminState {
        channel: Some(channel),
        ..AdminState::default()
    };
    cx.window.set_admin_open(true);
}

/// Saves the settings: a new channel, or changes to this one. `done` is
/// handed the channel as stored.
pub fn save(cx: &Context<'_>, done: impl FnOnce(&MainWindow, Channel) + 'static) {
    let window = cx.window;
    let settings = ChannelSettings {
        title: window.get_admin_title().trim().to_owned(),
        description: window.get_admin_description().trim().to_owned(),
        is_commenting_enabled: window.get_admin_commenting(),
        is_article_suggestion_enabled: window.get_admin_suggestions(),
        ..ChannelSettings::default()
    };
    if settings.title.is_empty() {
        return;
    }
    let id = cx.state.borrow().channel.as_ref().map(|c| c.id);
    window.set_admin_busy(true);
    window.set_admin_message("".into());
    let weak = window.as_weak();
    let state = Rc::clone(cx.state);
    let api = (**cx.client).clone();
    tasks::spawn(
        async move {
            match id {
                Some(id) => api.channel_edit(id, &settings).await,
                None => api.channel_create(&settings).await,
            }
        },
        move |result| {
            let Some(window) = weak.upgrade() else { return };
            window.set_admin_busy(false);
            match result {
                Ok(channel) => {
                    window.set_admin_message("saved".into());
                    if id.is_none() {
                        window.set_admin_open(false);
                    } else {
                        state.borrow_mut().channel = Some(channel.clone());
                    }
                    done(&window, channel);
                }
                Err(error) => {
                    tracing::warn!(%error, "the channel was not saved");
                    window.set_admin_message("failed".into());
                }
            }
        },
    );
}

/// Starts the account's personal blog; `done` is handed it.
pub fn create_blog(
    window: &MainWindow,
    client: &Client,
    done: impl FnOnce(&MainWindow, Channel) + 'static,
) {
    let weak = window.as_weak();
    let api = client.clone();
    tasks::spawn(async move { api.blog_create().await }, move |result| {
        let Some(window) = weak.upgrade() else { return };
        match result {
            Ok(blog) => done(&window, blog),
            Err(error) => {
                // The service refuses accounts it does not trust yet; the
                // feed says so where the button was.
                tracing::info!(%error, "the blog was not started");
                window.set_feed_notice(if window.get_lang() == "ru" {
                    "Блог пока не завести: сервису мало репутации аккаунта.".into()
                } else {
                    "No blog yet: the service wants more reputation first.".into()
                });
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Pictures
// ---------------------------------------------------------------------------

/// Asks for a picture and makes it the channel's avatar, or its cover.
pub fn change_picture(cx: &Context<'_>, cover: bool) {
    let Some(id) = cx.state.borrow().channel.as_ref().map(|c| c.id) else {
        return;
    };
    let weak = cx.window.as_weak();
    let api = (**cx.client).clone();
    // Opened from the UI thread, which is the one macOS insists on.
    let picked = slint::spawn_local(async move {
        let Some(file) = rfd::AsyncFileDialog::new()
            .add_filter("Image", &["png", "jpg", "jpeg", "webp"])
            .pick_file()
            .await
        else {
            return;
        };
        let name = file.file_name();
        let bytes = file.read().await;
        let Some(window) = weak.upgrade() else { return };
        let Some(mime) = crate::settings::mime_of(&name).filter(|_| !bytes.is_empty()) else {
            window.set_admin_message("avatar-unreadable".into());
            return;
        };
        window.set_admin_busy(true);
        let weak = window.as_weak();
        tasks::spawn(
            async move {
                if cover {
                    api.channel_cover_upload(id, &name, mime, bytes).await
                } else {
                    api.channel_avatar_upload(id, &name, mime, bytes).await
                }
            },
            move |result| {
                let Some(window) = weak.upgrade() else { return };
                window.set_admin_busy(false);
                window.set_admin_message(
                    match result {
                        Ok(_) => "avatar-saved",
                        Err(error) => {
                            tracing::warn!(%error, id, "the channel's picture was not changed");
                            "failed"
                        }
                    }
                    .into(),
                );
            },
        );
    });
    if let Err(error) = picked {
        tracing::warn!(%error, "the file chooser could not be opened");
    }
}

pub fn delete_cover(cx: &Context<'_>) {
    let Some(id) = cx.state.borrow().channel.as_ref().map(|c| c.id) else {
        return;
    };
    simple(
        cx,
        async move |api: Client| api.channel_cover_delete(id).await,
        |_, _| {},
    );
}

/// Runs one change on the channel, says how it went, and hands a success on.
fn simple<T, F>(
    cx: &Context<'_>,
    work: impl FnOnce(Client) -> F + 'static,
    then: impl FnOnce(&MainWindow, &Rc<RefCell<AdminState>>) + 'static,
) where
    T: Send + 'static,
    F: Future<Output = anirust_api::Result<T>> + Send + 'static,
{
    cx.window.set_admin_busy(true);
    cx.window.set_admin_message("".into());
    let weak = cx.window.as_weak();
    let state = Rc::clone(cx.state);
    tasks::spawn(work((**cx.client).clone()), move |result| {
        let Some(window) = weak.upgrade() else { return };
        window.set_admin_busy(false);
        match result {
            Ok(_) => {
                window.set_admin_message("saved".into());
                then(&window, &state);
            }
            Err(error) => {
                tracing::warn!(%error, "a channel change failed");
                window.set_admin_message("failed".into());
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Tabs
// ---------------------------------------------------------------------------

pub fn select_tab(cx: &Context<'_>, tab: i32) {
    cx.window.set_admin_tab(tab);
    cx.window.set_admin_message("".into());
    cx.window
        .set_admin_found(slint::ModelRc::new(VecModel::<crate::PersonItem>::default()));
    cx.state.borrow_mut().found.clear();
    match tab {
        1 => load_suggested(cx),
        2 => load_people(cx, false),
        3 => load_people(cx, true),
        _ => {}
    }
}

fn load_suggested(cx: &Context<'_>) {
    let Some(id) = cx.state.borrow().channel.as_ref().map(|c| c.id) else {
        return;
    };
    cx.window.set_admin_loading(true);
    let weak = cx.window.as_weak();
    let state = Rc::clone(cx.state);
    let api = (**cx.client).clone();
    tasks::spawn(async move { api.suggestions(id, 0).await }, move |result| {
        let Some(window) = weak.upgrade() else { return };
        window.set_admin_loading(false);
        let posts = result.map(|p| p.content).unwrap_or_else(|error| {
            tracing::warn!(%error, id, "the suggested posts were not loaded");
            Vec::new()
        });
        let items: Vec<SuggestionItem> = posts
            .iter()
            .map(|post| SuggestionItem {
                author: post.author.login.as_str().into(),
                text: post.plain_text().into(),
            })
            .collect();
        window.set_admin_suggested(slint::ModelRc::new(VecModel::from(items)));
        state.borrow_mut().suggested = posts;
    });
}

/// The administrators, or those blocked.
fn load_people(cx: &Context<'_>, blocked: bool) {
    let Some(id) = cx.state.borrow().channel.as_ref().map(|c| c.id) else {
        return;
    };
    cx.window.set_admin_loading(true);
    let weak = cx.window.as_weak();
    let state = Rc::clone(cx.state);
    let api = (**cx.client).clone();
    let http = cx.http.clone();
    tasks::spawn(
        async move {
            if blocked {
                api.channel_blocked(id, 0).await
            } else {
                api.channel_members(id, ADMINISTRATOR, 0).await
            }
        },
        move |result| {
            let Some(window) = weak.upgrade() else { return };
            window.set_admin_loading(false);
            let people: Vec<ProfileCompact> = result
                .map(|page| {
                    page.content
                        .into_iter()
                        .map(|m: ChannelProfile| m.profile)
                        .collect()
                })
                .unwrap_or_else(|error| {
                    tracing::warn!(%error, id, "the channel's people were not loaded");
                    Vec::new()
                });
            let model = faces(&people, &http);
            let mut state = state.borrow_mut();
            if blocked {
                window.set_admin_blocked(model);
                state.blocked = people;
            } else {
                window.set_admin_admins(model);
                state.admins = people;
            }
        },
    );
}

fn faces(people: &[ProfileCompact], http: &reqwest::Client) -> slint::ModelRc<crate::PersonItem> {
    slint::ModelRc::from(people::faces(
        people
            .iter()
            .map(|p| (p.login.as_str(), p.avatar.as_str(), false)),
        http,
    ))
}

/// Publishes the suggested post at a row, with its author's name or without.
pub fn publish(cx: &Context<'_>, index: usize, signed: bool) {
    let Some(id) = cx.state.borrow().suggested.get(index).map(|p| p.id) else {
        return;
    };
    simple(
        cx,
        async move |api: Client| api.suggestion_publish(id, signed).await,
        |window, state| {
            reload_tab(window, state, 1);
        },
    );
}

pub fn reject(cx: &Context<'_>, index: usize) {
    let Some(id) = cx.state.borrow().suggested.get(index).map(|p| p.id) else {
        return;
    };
    simple(
        cx,
        async move |api: Client| api.suggestion_delete(id).await,
        |window, state| {
            reload_tab(window, state, 1);
        },
    );
}

pub fn remove_admin(cx: &Context<'_>, index: usize) {
    let (Some(channel), Some(person)) = ({
        let state = cx.state.borrow();
        (
            state.channel.as_ref().map(|c| c.id),
            state.admins.get(index).map(|p| p.id),
        )
    }) else {
        return;
    };
    simple(
        cx,
        async move |api: Client| api.channel_permission(channel, person, None).await,
        |window, state| {
            reload_tab(window, state, 2);
        },
    );
}

pub fn make_admin(cx: &Context<'_>, index: usize) {
    let (Some(channel), Some(person)) = ({
        let state = cx.state.borrow();
        (
            state.channel.as_ref().map(|c| c.id),
            state.found.get(index).map(|p| p.id),
        )
    }) else {
        return;
    };
    simple(
        cx,
        async move |api: Client| {
            api.channel_permission(channel, person, Some(ADMINISTRATOR))
                .await
        },
        |window, state| reload_tab(window, state, 2),
    );
}

/// Blocks or unblocks; blocks are for good and give no reason, which is as
/// little as a block can say.
fn set_blocked(cx: &Context<'_>, person: Option<i64>, blocked: bool) {
    let (Some(channel), Some(person)) = (cx.state.borrow().channel.as_ref().map(|c| c.id), person)
    else {
        return;
    };
    let request = ChannelBlockRequest {
        target_profile_id: person,
        is_blocked: blocked,
        is_perm_blocked: blocked,
        ..ChannelBlockRequest::default()
    };
    simple(
        cx,
        async move |api: Client| api.channel_block_manage(channel, &request).await,
        |window, state| reload_tab(window, state, 3),
    );
}

pub fn unblock(cx: &Context<'_>, index: usize) {
    let person = cx.state.borrow().blocked.get(index).map(|p| p.id);
    set_blocked(cx, person, false);
}

pub fn block(cx: &Context<'_>, index: usize) {
    let person = cx.state.borrow().found.get(index).map(|p| p.id);
    set_blocked(cx, person, true);
}

/// Asks the window to show a tab again, through its own callback, so the
/// reload goes the way a click does.
fn reload_tab(window: &MainWindow, _state: &Rc<RefCell<AdminState>>, tab: i32) {
    window.invoke_admin_select_tab(tab);
}

/// Finds the channel's subscribers by name, to promote or to block.
pub fn search(cx: &Context<'_>, query: String) {
    let query = query.trim().to_owned();
    let Some(id) = cx.state.borrow().channel.as_ref().map(|c| c.id) else {
        return;
    };
    if query.is_empty() {
        return;
    }
    let weak = cx.window.as_weak();
    let state = Rc::clone(cx.state);
    let api = (**cx.client).clone();
    let http = cx.http.clone();
    tasks::spawn(
        async move { api.search_subscribers(id, &query, 0).await },
        move |result| {
            let Some(window) = weak.upgrade() else { return };
            let people: Vec<ProfileCompact> = result
                .map(|page| page.content.into_iter().map(|m| m.profile).collect())
                .unwrap_or_default();
            window.set_admin_found(faces(&people, &http));
            state.borrow_mut().found = people;
        },
    );
}
