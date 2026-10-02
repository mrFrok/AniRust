// SPDX-License-Identifier: GPL-3.0-or-later

//! Collections: lists of releases someone put together and named.
//!
//! A collection opens over the browsing grid, which then holds its releases,
//! so opening one of them is the same click as anywhere else. The account's
//! own are written in a sheet — title, description, whether anyone else may
//! see it — and a release is put into one from its own page.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, VecModel};

use anirust_api::{Client, Collection};

use crate::home::{self, HomeState};
use crate::{CollectionItem, MainWindow, tasks};

/// The collection on screen, and what the sheets are working on.
#[derive(Default)]
pub struct CollectionsState {
    open: Option<Collection>,
    /// The collection the editor is changing; `None` writes a new one.
    editing: Option<i64>,
    /// The release a new collection starts with, when made from its page.
    first_release: Option<i64>,
    /// The release the picker is placing, and where it can go.
    placing: Option<i64>,
    mine: Vec<Collection>,
}

/// Everything a collection action needs, gathered once by the caller.
pub struct Context<'a> {
    pub window: &'a MainWindow,
    pub state: &'a Rc<RefCell<CollectionsState>>,
    pub home: &'a Rc<RefCell<HomeState>>,
    pub client: &'a Rc<Client>,
    pub http: reqwest::Client,
    /// The account's profile id, 0 without one.
    pub me: i64,
}

/// Opens a collection over the grid.
pub fn open(cx: &Context<'_>, collection: Collection) {
    let id = collection.id;
    if id <= 0 {
        return;
    }
    show(cx.window, &collection, cx.me, &cx.http);
    cx.state.borrow_mut().open = Some(collection);
    cx.window.set_collection_open(true);
    cx.window.set_notice("".into());
    cx.window.set_results(slint::ModelRc::new(
        VecModel::<crate::ReleaseCard>::default(),
    ));
    cx.window.set_results_loading(true);
    let generation = cx.home.borrow_mut().next_generation();

    let weak = cx.window.as_weak();
    let state = Rc::clone(cx.state);
    let home = Rc::clone(cx.home);
    let api = (**cx.client).clone();
    let (http, me) = (cx.http.clone(), cx.me);
    tasks::spawn(
        async move {
            (
                api.collection(id).await,
                api.collection_releases(id, 0).await,
            )
        },
        move |(view, releases)| {
            let Some(window) = weak.upgrade() else { return };
            if !window.get_collection_open()
                || state.borrow().open.as_ref().map(|c| c.id) != Some(id)
            {
                return;
            }
            if let Ok(view) = view {
                show(&window, &view.collection, me, &http);
                state.borrow_mut().open = Some(view.collection);
            }
            match releases {
                Ok(page) => home::show(&window, &home, page.content, generation, http),
                Err(error) => {
                    tracing::warn!(%error, id, "the collection's releases were not loaded");
                    window.set_results_loading(false);
                }
            }
        },
    );
}

/// Back from a collection to the tab it was opened from.
pub fn close(cx: &Context<'_>) {
    cx.state.borrow_mut().open = None;
    home::open(cx.window, cx.home, Rc::clone(cx.client), cx.http.clone());
}

/// The open collection: its id, title, and who made it.
#[must_use]
pub fn open_one(state: &Rc<RefCell<CollectionsState>>) -> Option<(i64, String, i64)> {
    state.borrow().open.as_ref().map(|c| {
        let creator = c.creator.as_ref().map_or(0, |p| p.id);
        (c.id, c.title.clone(), creator)
    })
}

/// Favourites the open collection, or stops; put back if the server refuses.
pub fn toggle_favourite(cx: &Context<'_>) {
    let Some((id, now)) = cx
        .state
        .borrow()
        .open
        .as_ref()
        .map(|c| (c.id, !c.is_favorite))
    else {
        return;
    };
    set_favourite(cx.window, cx.state, now, cx.me);

    let weak = cx.window.as_weak();
    let state = Rc::clone(cx.state);
    let api = (**cx.client).clone();
    let me = cx.me;
    tasks::spawn(
        async move {
            if now {
                api.collection_favorite_add(id).await
            } else {
                api.collection_favorite_delete(id).await
            }
        },
        move |result| {
            let Err(error) = result else { return };
            tracing::warn!(%error, id, "the collection's favourite was not changed");
            if let Some(window) = weak.upgrade() {
                set_favourite(&window, &state, !now, me);
            }
        },
    );
}

fn set_favourite(
    window: &MainWindow,
    state: &Rc<RefCell<CollectionsState>>,
    favourite: bool,
    me: i64,
) {
    let mut guard = state.borrow_mut();
    let Some(collection) = guard.open.as_mut() else {
        return;
    };
    if collection.is_favorite != favourite {
        collection.favorites_count += if favourite { 1 } else { -1 };
    }
    collection.is_favorite = favourite;
    let collection = collection.clone();
    drop(guard);
    // Redrawn without fetching the cover again.
    let mut item = window.get_open_collection_item();
    let fresh = item_for(&collection, me);
    item.favourite = fresh.favourite;
    item.favourites = fresh.favourites;
    window.set_open_collection_item(item);
}

// ---------------------------------------------------------------------------
// The editor
// ---------------------------------------------------------------------------

/// Opens the editor on the open collection.
pub fn edit(cx: &Context<'_>) {
    let Some(collection) = cx.state.borrow().open.clone() else {
        return;
    };
    {
        let mut state = cx.state.borrow_mut();
        state.editing = Some(collection.id);
        state.first_release = None;
    }
    cx.window
        .set_collection_draft_title(collection.title.as_str().into());
    cx.window
        .set_collection_draft_description(collection.description.as_str().into());
    cx.window
        .set_collection_draft_private(collection.is_private);
    open_editor(cx.window, true);
}

/// Opens the editor on a new collection, with a release in it to begin with
/// when one is given.
pub fn create(cx: &Context<'_>, first_release: Option<i64>) {
    {
        let mut state = cx.state.borrow_mut();
        state.editing = None;
        state.first_release = first_release;
    }
    cx.window.set_collection_draft_title("".into());
    cx.window.set_collection_draft_description("".into());
    cx.window.set_collection_draft_private(false);
    open_editor(cx.window, false);
}

fn open_editor(window: &MainWindow, editing: bool) {
    window.set_collection_editing(editing);
    window.set_collection_busy(false);
    window.set_collection_failed(false);
    window.set_collection_editor_open(true);
}

/// Writes what the editor holds: a new collection, or changes to one.
pub fn save(cx: &Context<'_>) {
    let title = cx.window.get_collection_draft_title().trim().to_owned();
    if title.is_empty() {
        return;
    }
    let description = cx
        .window
        .get_collection_draft_description()
        .trim()
        .to_owned();
    let private = cx.window.get_collection_draft_private();
    let (editing, first) = {
        let state = cx.state.borrow();
        (state.editing, state.first_release)
    };
    cx.window.set_collection_busy(true);
    cx.window.set_collection_failed(false);

    let weak = cx.window.as_weak();
    let state = Rc::clone(cx.state);
    let home = Rc::clone(cx.home);
    let client = Rc::clone(cx.client);
    let api = (**cx.client).clone();
    let (http, me) = (cx.http.clone(), cx.me);
    tasks::spawn(
        async move {
            match editing {
                // The edit rewrites the whole list of releases, so the list
                // is read first and sent back as it was.
                Some(id) => {
                    let releases: Vec<i64> = api
                        .my_collection_releases(id)
                        .await?
                        .iter()
                        .map(|r| r.id)
                        .collect();
                    api.collection_edit(id, &title, &description, private, &releases)
                        .await
                }
                None => {
                    let releases: Vec<i64> = first.into_iter().collect();
                    api.collection_create(&title, &description, private, &releases)
                        .await
                }
            }
        },
        move |result| {
            let Some(window) = weak.upgrade() else { return };
            window.set_collection_busy(false);
            match result {
                Ok(saved) => {
                    window.set_collection_editor_open(false);
                    let open = state.borrow().open.as_ref().map(|c| c.id);
                    if open.is_some() && open == editing {
                        // The answer may leave out the releases and creator;
                        // keep what is already known of them.
                        let mut kept = state.borrow().open.clone().unwrap_or_default();
                        kept.title = saved.title;
                        kept.description = saved.description;
                        kept.is_private = saved.is_private;
                        let mut item = window.get_open_collection_item();
                        let fresh = item_for(&kept, me);
                        item.title = fresh.title;
                        item.description = fresh.description;
                        item.private = fresh.private;
                        window.set_open_collection_item(item);
                        state.borrow_mut().open = Some(kept);
                    } else if !window.get_collection_open() && window.get_screen() == "home" {
                        // A new one shows up on the tab of the account's own.
                        home::open(&window, &home, client, http);
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "the collection was not saved");
                    window.set_collection_failed(true);
                }
            }
        },
    );
}

/// Deletes the collection the editor is on. The sheet has asked twice.
pub fn delete(cx: &Context<'_>) {
    let Some(id) = cx.state.borrow().editing else {
        return;
    };
    cx.window.set_collection_busy(true);

    let weak = cx.window.as_weak();
    let state = Rc::clone(cx.state);
    let home = Rc::clone(cx.home);
    let client = Rc::clone(cx.client);
    let api = (**cx.client).clone();
    let http = cx.http.clone();
    tasks::spawn(
        async move { api.collection_delete(id).await },
        move |result| {
            let Some(window) = weak.upgrade() else { return };
            window.set_collection_busy(false);
            match result {
                Ok(()) => {
                    window.set_collection_editor_open(false);
                    state.borrow_mut().open = None;
                    home::open(&window, &home, client, http);
                }
                Err(error) => {
                    tracing::warn!(%error, id, "the collection was not deleted");
                    window.set_collection_failed(true);
                }
            }
        },
    );
}

// ---------------------------------------------------------------------------
// The picker
// ---------------------------------------------------------------------------

/// Offers the account's collections to put a release into.
pub fn pick_for(cx: &Context<'_>, release_id: i64) {
    if cx.me <= 0 {
        return;
    }
    cx.state.borrow_mut().placing = Some(release_id);
    cx.window.set_collection_failed(false);
    cx.window.set_my_collections_loading(true);
    cx.window.set_collection_picker_open(true);

    let weak = cx.window.as_weak();
    let state = Rc::clone(cx.state);
    let api = (**cx.client).clone();
    let me = cx.me;
    tasks::spawn(
        async move { api.profile_collections(me, 0).await },
        move |result| {
            let Some(window) = weak.upgrade() else { return };
            window.set_my_collections_loading(false);
            let mine = result.map(|page| page.content).unwrap_or_else(|error| {
                tracing::warn!(%error, "the account's collections were not loaded");
                Vec::new()
            });
            let titles: Vec<slint::SharedString> =
                mine.iter().map(|c| c.title.as_str().into()).collect();
            window.set_my_collection_titles(slint::ModelRc::new(VecModel::from(titles)));
            state.borrow_mut().mine = mine;
        },
    );
}

/// Puts the release into the collection at a row of the picker.
pub fn pick(cx: &Context<'_>, index: usize) {
    let (id, release) = {
        let state = cx.state.borrow();
        let Some(id) = state.mine.get(index).map(|c| c.id) else {
            return;
        };
        let Some(release) = state.placing else { return };
        (id, release)
    };
    cx.window.set_collection_failed(false);

    let weak = cx.window.as_weak();
    let api = (**cx.client).clone();
    tasks::spawn(
        async move { api.collection_add_release(id, release).await },
        move |result| {
            let Some(window) = weak.upgrade() else { return };
            match result {
                Ok(()) => window.set_collection_picker_open(false),
                Err(error) => {
                    tracing::warn!(%error, id, release, "the release was not added");
                    window.set_collection_failed(true);
                }
            }
        },
    );
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

fn item_for(collection: &Collection, me: i64) -> CollectionItem {
    let creator = collection.creator.as_ref();
    CollectionItem {
        title: collection.title.as_str().into(),
        description: collection.description.as_str().into(),
        creator: creator.map(|p| p.login.as_str()).unwrap_or_default().into(),
        image: slint::Image::default(),
        image_loaded: false,
        favourite: collection.is_favorite,
        favourites: i32::try_from(collection.favorites_count).unwrap_or(i32::MAX),
        comments: i32::try_from(collection.comment_count).unwrap_or(i32::MAX),
        private: collection.is_private,
        mine: me > 0 && creator.is_some_and(|p| p.id == me),
    }
}

/// Puts a collection in the header, and its cover once it arrives.
fn show(window: &MainWindow, collection: &Collection, me: i64, http: &reqwest::Client) {
    window.set_open_collection_item(item_for(collection, me));
    if !collection.image.starts_with("http") {
        return;
    }
    let weak = window.as_weak();
    let title = slint::SharedString::from(collection.title.as_str());
    tasks::spawn(
        tasks::fetch_image(http.clone(), collection.image.clone()),
        move |result| {
            let (Some(window), Ok(buffer)) = (weak.upgrade(), result) else {
                return;
            };
            // Another collection may have been opened meanwhile.
            let mut item = window.get_open_collection_item();
            if !window.get_collection_open() || item.title != title {
                return;
            }
            item.image = slint::Image::from_rgba8(buffer);
            item.image_loaded = true;
            window.set_open_collection_item(item);
        },
    );
}
