// SPDX-License-Identifier: GPL-3.0-or-later

// A release build on Windows is a window, not a console program with a
// window: no black box behind it. Debug builds keep the console for logs.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

//! AniRust.
//!
//! One window, two screens: the release screen chooses what to watch, the
//! player plays it. The player is a screen rather than a separate window, so
//! leaving an episode lands back on the release it came from instead of on
//! nothing.
//!
//! Nothing here blocks the event loop. Every lookup and every extractor run
//! goes through [`tasks`] and comes back on the UI thread.

mod adapt;
mod bookmarks;
mod channel_admin;
mod collections;
mod comments;
mod desktop;
mod downloads;
mod editor;
mod feed;
mod home;
mod notifications;
mod people;
mod preferences;
mod progress;
mod register;
mod relay;
mod release;
mod reports;
mod runtimes;
mod session;
mod settings;
mod standing;
mod tasks;
mod video;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use slint::ComponentHandle;

use anirust_api::{Client, EpisodeSort};
use anirust_extract::{Registry, ResolvedStream};
use anirust_player::{
    Backend, Enhancement, FrameGeneration, MediaSource, Mlrt, Networks, PictureAdjust,
    PlaybackState, Player, PlayerConfig, RifeInstall, RifeModel, TargetRate, Track, TrackKind,
    UpscaleMode, UpscalePreset, UpscaleQuality,
};

use crate::home::HomeState;
use crate::release::ReleaseState;
use crate::session::Session;
use crate::video::VideoBridge;

slint::include_modules!();

/// Where the feed and the profile sit on the rail. Named because two places
/// have to agree about each: the rail's own order, and what arriving there has
/// to fetch — neither is a grid, so nothing in `home` fetches for them.
const FEED_DESTINATION: i32 = 3;
const SETTINGS_DESTINATION: i32 = 6;
const PROFILE_DESTINATION: i32 = 4;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive("anirust=info".parse()?)
                .from_env_lossy(),
        )
        .with_writer(log_writer())
        .with_ansi(cfg!(not(all(windows, not(debug_assertions)))))
        .init();

    let opening = release_id_from_args()?;
    tasks::init()?;
    relay::start();

    desktop::identify();
    let window = MainWindow::new().context("creating the window")?;
    window.set_lang(if is_russian_locale() {
        "ru".into()
    } else {
        "en".into()
    });
    window.set_app_version(env!("CARGO_PKG_VERSION").into());
    window.on_open_link(|url| release::open_in_browser(&url));
    let prefs = Rc::new(Cell::new(preferences::Preferences::load()));
    wire_preferences(&window, &prefs);

    let config = player_config();
    tracing::info!(hwdec = %config.hwdec, "player config");
    let player = Player::new(&config).context("creating the player")?;
    let bridge = Rc::new(VideoBridge::new(player).context("creating the video bridge")?);

    bridge
        .attach(&window, |window, frame| {
            window.set_video_frame(frame);
            window.set_has_video(true);
        })
        .context("attaching video to the window")?;

    let http = reqwest_client();
    let account = Rc::new(RefCell::new(Session::default()));
    let app = Rc::new(App {
        client: Rc::new(api_client().context("creating the API client")?),
        registry: Rc::new(Registry::new(http.clone())),
        http,
        bridge,
        release: Rc::new(RefCell::new(ReleaseState::new(Rc::new(RefCell::new(
            progress::Store::load(),
        ))))),
        home: Rc::new(RefCell::new(HomeState::new(Rc::clone(&account)))),
        collections: Rc::new(RefCell::new(collections::CollectionsState::default())),
        pending: Rc::new(RefCell::new(register::Pending::default())),
        email_change: Rc::new(RefCell::new(settings::EmailChange::default())),
        reports: Rc::new(RefCell::new(reports::ReportState::default())),
        standing: Rc::new(RefCell::new(standing::Standing::default())),
        editor: Rc::new(RefCell::new(editor::EditorState::default())),
        admin: Rc::new(RefCell::new(channel_admin::AdminState::default())),
        came_from: RefCell::new(None),
        feed: Rc::new(RefCell::new(feed::FeedState::new(Rc::clone(&account)))),
        comments: Rc::new(RefCell::new(comments::CommentsState::default())),
        notifications: Rc::new(RefCell::new(notifications::NotificationsState::default())),
        people: Rc::new(RefCell::new(people::PeopleState::default())),
        queue: Rc::new(RefCell::new(downloads::Queue::default())),
        account,
        playing: Rc::new(RefCell::new(None)),
        settings: Rc::new(Settings::from_preferences(prefs.get().player.in_force())),
        prefs: Rc::clone(&prefs),
    });

    wire_home(&window, &app);
    wire_collections(&window, &app);
    wire_settings(&window, &app);
    wire_reports(&window, &app);
    wire_editor(&window, &app);
    wire_channel_admin(&window, &app);
    wire_feed(&window, &app);
    wire_comments(&window, &app);
    wire_notifications(&window, &app);
    wire_people(&window, &app);
    wire_account(&window, &app);
    wire_release(&window, &app);
    let advance = wire_player(&window, &app);
    drive_status(&window, &app, advance);

    // Before anything is fetched: a restored session changes what the server
    // answers with, down to which episodes are marked watched.
    session::restore(&window, &app.account, &app.client, app.http.clone());

    // A release id on the command line opens straight into it; otherwise the
    // client starts where a client should, on something to choose from.
    match opening {
        Some(release_id) => {
            window.set_screen("release".into());
            release::load(
                &window,
                &app.release,
                Rc::clone(&app.client),
                app.http.clone(),
                release_id,
            );
        }
        None => home::open(&window, &app.home, Rc::clone(&app.client), app.http.clone()),
    }

    // Held for as long as the window runs: dropping the timer stops it.
    let _bell = notifications::start_polling(&window, &app.client);

    window.run().context("running the event loop")?;
    Ok(())
}

/// Everything the screens share.
///
/// Held behind one `Rc` because every callback wants some subset of it, and
/// threading eight handles through each one was turning every signature into a
/// list of its dependencies rather than a description of what it does.
struct App {
    client: Rc<Client>,
    http: reqwest::Client,
    registry: Rc<Registry>,
    bridge: Rc<VideoBridge>,
    release: Rc<RefCell<ReleaseState>>,
    home: Rc<RefCell<HomeState>>,
    feed: Rc<RefCell<feed::FeedState>>,
    collections: Rc<RefCell<collections::CollectionsState>>,
    pending: Rc<RefCell<register::Pending>>,
    email_change: Rc<RefCell<settings::EmailChange>>,
    reports: Rc<RefCell<reports::ReportState>>,
    standing: Rc<RefCell<standing::Standing>>,
    editor: Rc<RefCell<editor::EditorState>>,
    admin: Rc<RefCell<channel_admin::AdminState>>,
    came_from: RefCell<Option<CameFrom>>,
    comments: Rc<RefCell<comments::CommentsState>>,
    notifications: Rc<RefCell<notifications::NotificationsState>>,
    people: Rc<RefCell<people::PeopleState>>,
    queue: Rc<RefCell<downloads::Queue>>,
    account: Rc<RefCell<Session>>,
    /// The stream currently loaded, so the quality menu and the skip button
    /// act on what is playing rather than on what was resolved first.
    playing: Playing,
    settings: Rc<Settings>,
    /// What is kept between runs, written whenever it changes.
    prefs: Rc<Cell<preferences::Preferences>>,
}

/// Bring-up knobs, so a picture problem can be bisected without a rebuild.
///
/// Every one of these eliminated a suspect while the GL path was being brought
/// up; they stay because the next driver will raise the same questions.
fn player_config() -> PlayerConfig {
    PlayerConfig {
        hwdec: std::env::var("ANIRUST_HWDEC")
            .map(std::borrow::Cow::Owned)
            .unwrap_or(std::borrow::Cow::Borrowed(anirust_player::DEFAULT_HWDEC)),
        fbo_format: std::env::var("ANIRUST_FBO")
            .map(std::borrow::Cow::Owned)
            .ok()
            .or(Some(std::borrow::Cow::Borrowed(
                anirust_player::DEFAULT_FBO_FORMAT,
            ))),
        direct_rendering: std::env::var("ANIRUST_DR").as_deref() != Ok("no"),
        dither_depth: std::env::var("ANIRUST_DITHER")
            .ok()
            .and_then(|v| v.parse().ok()),
        dumb_mode: std::env::var("ANIRUST_DUMB").is_ok(),
        verbose_log: std::env::var("ANIRUST_MPV_LOG").is_ok(),
        ..PlayerConfig::default()
    }
}

/// The stream currently loaded, kept so the quality menu and the skip button
/// can act on what is playing rather than on what was resolved first.
type Playing = Rc<RefCell<Option<ResolvedStream>>>;

// ---------------------------------------------------------------------------
// Account
// ---------------------------------------------------------------------------

fn wire_account(window: &MainWindow, app: &Rc<App>) {
    let weak = window.as_weak();
    let app = Rc::clone(app);
    window.on_submit_sign_in({
        let app = Rc::clone(&app);
        let weak = weak.clone();
        move || {
            let Some(window) = weak.upgrade() else { return };
            if window.get_sign_in_mode() != "sign-in" {
                register::submit(&register_context(&window, &app));
                return;
            }
            session::sign_in(
                &window,
                &app.account,
                Rc::clone(&app.client),
                app.http.clone(),
                window.get_login().trim().to_string(),
                window.get_password().to_string(),
            );
        }
    });

    window.on_resend_sign_in_code({
        let app = Rc::clone(&app);
        let weak = weak.clone();
        move || {
            let Some(window) = weak.upgrade() else { return };
            register::resend(&register_context(&window, &app));
        }
    });

    window.on_sign_out(move || {
        let Some(window) = weak.upgrade() else { return };
        session::sign_out(&window, &app.account, &app.client);
    });
}

/// Puts what was chosen last time on screen, and keeps the file level with it.
///
/// The window is told twice over: once as the appearance itself, which is what
/// the colours are drawn from, and once as its position in the row the profile
/// screen offers, which is what that row lights up. Rust owns the order of
/// that row, so the two cannot drift apart.
fn wire_preferences(window: &MainWindow, held: &Rc<Cell<preferences::Preferences>>) {
    let preferences = held.get();
    show_appearance(window, preferences.appearance);
    show_accent(window, preferences.accent);
    window.set_translucent(preferences.translucent);
    window.set_remember_player(preferences.player.remember);

    window.on_select_accent({
        let held = Rc::clone(held);
        let weak = window.as_weak();
        move |index| {
            let Some(window) = weak.upgrade() else { return };
            let mut preferences = held.get();
            preferences.accent = preferences::Accent::at(index);
            held.set(preferences);
            show_accent(&window, preferences.accent);
            preferences.save();
        }
    });
    window.on_set_translucent({
        let held = Rc::clone(held);
        let weak = window.as_weak();
        move |on| {
            let Some(window) = weak.upgrade() else { return };
            let mut preferences = held.get();
            preferences.translucent = on;
            held.set(preferences);
            window.set_translucent(on);
            preferences.save();
        }
    });
    let held = Rc::clone(held);

    window.on_set_remember_player({
        let held = Rc::clone(&held);
        let weak = window.as_weak();
        move |on| {
            let Some(window) = weak.upgrade() else { return };
            let mut preferences = held.get();
            preferences.player.remember = on;
            held.set(preferences);
            window.set_remember_player(on);
            preferences.save();
        }
    });

    let weak = window.as_weak();
    window.on_select_appearance(move |index| {
        let Some(window) = weak.upgrade() else { return };
        let chosen = preferences::Appearance::at(index);

        let mut preferences = held.get();
        if preferences.appearance == chosen {
            return;
        }
        preferences.appearance = chosen;
        held.set(preferences);

        show_appearance(&window, chosen);
        preferences.save();
    });
}

fn show_accent(window: &MainWindow, chosen: preferences::Accent) {
    use preferences::Accent as Kept;
    window.set_accent(match chosen {
        Kept::Violet => Accent::Violet,
        Kept::Blue => Accent::Blue,
        Kept::Indigo => Accent::Indigo,
        Kept::Teal => Accent::Teal,
        Kept::Green => Accent::Green,
        Kept::Amber => Accent::Amber,
        Kept::Rose => Accent::Rose,
    });
    window.set_accent_index(chosen.index());
}

fn show_appearance(window: &MainWindow, chosen: preferences::Appearance) {
    window.set_appearance(match chosen {
        preferences::Appearance::System => Appearance::System,
        preferences::Appearance::Light => Appearance::Light,
        preferences::Appearance::Dark => Appearance::Dark,
        preferences::Appearance::Amoled => Appearance::Amoled,
    });
    window.set_appearance_choice(chosen.index());
}

// ---------------------------------------------------------------------------
// Browsing
// ---------------------------------------------------------------------------

/// What the comment thread needs to know about whoever is reading it.
fn viewer(app: &App) -> comments::Viewer {
    comments::Viewer {
        profile_id: app.account.borrow().id,
        http: app.http.clone(),
    }
}

/// A window callback that receives a row index and hands it on, if it is one.
macro_rules! on_row {
    ($window:expr, $app:expr, $setter:ident, |$w:ident, $a:ident, $i:ident| $body:expr) => {{
        let app = Rc::clone($app);
        let weak = $window.as_weak();
        $window.$setter(move |index| {
            let Some($w) = weak.upgrade() else { return };
            let Ok($i) = usize::try_from(index) else {
                return;
            };
            let $a = &app;
            $body;
        });
    }};
}

/// Opens someone's profile on the profile page, from anywhere.
fn open_person(window: &MainWindow, app: &App, id: i64) {
    // Where the profile was opened from, to go back to. Only the first
    // step away is kept: a friend of a friend's "back" goes to the screen
    // the first profile was opened from.
    if window.get_profile_is_mine() || !window.get_profile_can_go_back() {
        *app.came_from.borrow_mut() = Some(CameFrom {
            screen: window.get_screen(),
            destination: window.get_destination(),
            comments: window.get_comments_open(),
            notifications: window.get_notifications_open(),
        });
    }
    window.set_profile_can_go_back(true);
    window.set_comments_open(false);
    window.set_notifications_open(false);
    window.set_screen("home".into());
    if window.get_destination() != PROFILE_DESTINATION {
        window.set_destination(PROFILE_DESTINATION);
    }
    let me = app.account.borrow().id;
    people::open(window, &app.people, &app.client, app.http.clone(), me, id);
}

/// Where someone's profile was opened from.
struct CameFrom {
    screen: slint::SharedString,
    destination: i32,
    comments: bool,
    notifications: bool,
}

/// Back from someone's profile to where it was opened.
fn go_back_from_profile(window: &MainWindow, app: &App) {
    let Some(from) = app.came_from.borrow_mut().take() else {
        open_my_profile(window, app);
        return;
    };
    window.set_profile_can_go_back(false);
    people::back_to_mine(window, &app.people);
    // The rail is set directly: going through it would reload the tab and
    // lose what was on screen.
    window.set_destination(from.destination);
    window.set_screen(from.screen);
    window.set_comments_open(from.comments);
    window.set_notifications_open(from.notifications);
}

/// Back to the account's own profile, freshly read.
fn open_my_profile(window: &MainWindow, app: &App) {
    window.set_profile_can_go_back(false);
    app.came_from.borrow_mut().take();
    people::back_to_mine(window, &app.people);
    session::refresh_profile(
        window,
        &app.account,
        &app.people,
        &app.client,
        app.http.clone(),
    );
    people::load_requests(window, &app.people, &app.client, app.http.clone());
}

fn wire_people(window: &MainWindow, app: &Rc<App>) {
    window.on_profile_go_back({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move || {
            let Some(window) = weak.upgrade() else { return };
            go_back_from_profile(&window, &app);
        }
    });
    window.on_back_to_my_profile({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move || {
            let Some(window) = weak.upgrade() else { return };
            open_my_profile(&window, &app);
        }
    });
    window.on_friend_action({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move || {
            let Some(window) = weak.upgrade() else { return };
            people::friend_action(&window, &app.people, &app.client, app.http.clone());
        }
    });
    window.on_toggle_block({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move || {
            let Some(window) = weak.upgrade() else { return };
            people::toggle_block(&window, &app.people, &app.client, app.http.clone());
        }
    });
    on_row!(window, app, on_open_friend, |window, app, index| {
        if let Some(id) = people::friend_at(&app.people, index) {
            open_person(&window, app, id);
        }
    });
    on_row!(window, app, on_open_request, |window, app, index| {
        if let Some(id) = people::request_at(&app.people, index) {
            open_person(&window, app, id);
        }
    });
    window.on_answer_request({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move |index, accept| {
            let Some(window) = weak.upgrade() else { return };
            let Ok(index) = usize::try_from(index) else {
                return;
            };
            people::answer_request(
                &window,
                &app.people,
                &app.client,
                app.http.clone(),
                index,
                accept,
            );
        }
    });
    on_row!(window, app, on_open_comment_author, |window, app, index| {
        if let Some(id) = comments::author_at(&app.comments, index) {
            open_person(&window, app, id);
        }
    });
}

fn wire_notifications(window: &MainWindow, app: &Rc<App>) {
    window.on_open_notifications({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move || {
            let Some(window) = weak.upgrade() else { return };
            notifications::open(&window, &app.notifications, &app.client);
        }
    });
    window.on_close_notifications({
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade() {
                window.set_notifications_open(false);
            }
        }
    });
    window.on_select_notification_kind({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move |index| {
            let Some(window) = weak.upgrade() else { return };
            notifications::select_kind(&window, &app.notifications, &app.client, index);
        }
    });
    window.on_load_more_notifications({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move || {
            let Some(window) = weak.upgrade() else { return };
            notifications::load_more(&window, &app.notifications, &app.client);
        }
    });
    window.on_mark_notifications_seen({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move || {
            let Some(window) = weak.upgrade() else { return };
            notifications::mark_all_seen(&window, &app.notifications, &app.client);
        }
    });
    window.on_clear_notifications({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move || {
            let Some(window) = weak.upgrade() else { return };
            notifications::clear_all(&window, &app.notifications, &app.client);
        }
    });
    on_row!(window, app, on_delete_notification, |window, app, index| {
        notifications::delete(&window, &app.notifications, &app.client, index)
    });
    on_row!(window, app, on_open_notification, |window, app, index| {
        if let Some(id) = notifications::person_at(&app.notifications, index) {
            open_person(&window, app, id);
        } else if let Some(release_id) = notifications::release_at(&app.notifications, index) {
            window.set_notifications_open(false);
            window.set_screen("release".into());
            release::load(
                &window,
                &app.release,
                Rc::clone(&app.client),
                app.http.clone(),
                release_id,
            );
        }
    });
    window.on_flip_notify({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move |index| {
            let Some(window) = weak.upgrade() else { return };
            notifications::flip(&window, &app.client, index);
        }
    });
}

fn wire_comments(window: &MainWindow, app: &Rc<App>) {
    window.on_open_release_comments({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move || {
            let Some(window) = weak.upgrade() else { return };
            let found = app
                .release
                .borrow()
                .release
                .as_ref()
                .map(|r| (r.id, r.title().to_owned()));
            let Some((id, title)) = found else { return };
            comments::open(
                &window,
                &app.comments,
                &app.client,
                viewer(&app),
                anirust_api::CommentTarget::Release,
                id,
                &title,
            );
        }
    });

    on_row!(window, app, on_open_post_comments, |window, app, index| {
        if let Some((id, title)) = feed::article_at(&app.feed, index) {
            comments::open(
                &window,
                &app.comments,
                &app.client,
                viewer(app),
                anirust_api::CommentTarget::Article,
                id,
                &title,
            );
        }
    });

    window.on_close_comments({
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade() {
                comments::close(&window);
            }
        }
    });

    window.on_select_comments_sort({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move |index| {
            let Some(window) = weak.upgrade() else { return };
            comments::select_sort(&window, &app.comments, &app.client, viewer(&app), index);
        }
    });

    window.on_load_more_comments({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move || {
            let Some(window) = weak.upgrade() else { return };
            comments::load_more(&window, &app.comments, &app.client, viewer(&app));
        }
    });

    window.on_send_comment({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move || {
            let Some(window) = weak.upgrade() else { return };
            comments::send(&window, &app.comments, &app.client, viewer(&app));
        }
    });

    window.on_cancel_comment_reply({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move || {
            let Some(window) = weak.upgrade() else { return };
            comments::cancel(&window, &app.comments);
        }
    });

    window.on_vote_comment({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move |index, vote| {
            let Some(window) = weak.upgrade() else { return };
            let Ok(index) = usize::try_from(index) else {
                return;
            };
            comments::vote(
                &window,
                &app.comments,
                &app.client,
                viewer(&app),
                index,
                vote,
            );
        }
    });

    on_row!(window, app, on_reply_to_comment, |window, app, index| {
        comments::reply(&window, &app.comments, index)
    });
    on_row!(window, app, on_edit_comment, |window, app, index| {
        comments::edit(&window, &app.comments, index)
    });
    on_row!(window, app, on_delete_comment, |window, app, index| {
        comments::delete(&window, &app.comments, &app.client, viewer(app), index)
    });
    on_row!(
        window,
        app,
        on_toggle_comment_replies,
        |window, app, index| {
            comments::toggle_replies(&window, &app.comments, &app.client, viewer(app), index)
        }
    );
    on_row!(window, app, on_reveal_comment, |window, app, index| {
        comments::reveal(&window, &app.comments, viewer(app), index)
    });
}

fn wire_feed(window: &MainWindow, app: &Rc<App>) {
    let weak = window.as_weak();

    window.on_select_feed_tab({
        let app = Rc::clone(app);
        let weak = weak.clone();
        move |index| {
            let Some(window) = weak.upgrade() else { return };
            feed::select_tab(
                &window,
                &app.feed,
                &app.client,
                app.http.clone(),
                app.account.borrow().authenticated,
                index,
            );
        }
    });

    window.on_vote_post({
        let app = Rc::clone(app);
        let weak = weak.clone();
        move |index, vote| {
            let Some(window) = weak.upgrade() else { return };
            if !app.client.is_authenticated() {
                window.set_sign_in_error("".into());
                window.set_show_sign_in(true);
                return;
            }
            feed::vote_post(&window, &app.feed, &app.client, index, vote);
        }
    });

    window.on_toggle_subscription({
        let app = Rc::clone(app);
        move |index| {
            let Some(window) = weak.upgrade() else { return };
            feed::toggle_subscription(&window, &app.feed, &app.client, index);
        }
    });

    on_row!(window, app, on_open_post_channel, |window, app, index| {
        feed::open_post_channel(&window, &app.feed, &app.client, app.http.clone(), index)
    });
    on_row!(window, app, on_open_channel, |window, app, index| {
        feed::open_listed_channel(&window, &app.feed, &app.client, app.http.clone(), index)
    });

    window.on_close_channel({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move || {
            let Some(window) = weak.upgrade() else { return };
            let signed_in = app.account.borrow().authenticated;
            feed::close_channel(&window, &app.feed, &app.client, app.http.clone(), signed_in);
        }
    });

    // Both act on a row, or on the open channel with -1.
    window.on_toggle_channel_subscription({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move |index| {
            let Some(window) = weak.upgrade() else { return };
            if !app.client.is_authenticated() {
                window.set_sign_in_error("".into());
                window.set_show_sign_in(true);
                return;
            }
            feed::toggle_channel_subscription(
                &window,
                &app.feed,
                &app.client,
                app.http.clone(),
                index,
            );
        }
    });
    window.on_toggle_channel_mute({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move |index| {
            let Some(window) = weak.upgrade() else { return };
            if !app.client.is_authenticated() {
                window.set_sign_in_error("".into());
                window.set_show_sign_in(true);
                return;
            }
            feed::toggle_channel_mute(&window, &app.feed, &app.client, app.http.clone(), index);
        }
    });
}

fn editor_context<'a>(window: &'a MainWindow, app: &'a App) -> editor::Context<'a> {
    editor::Context {
        window,
        state: &app.editor,
        client: &app.client,
        me: app.account.borrow().id,
    }
}

fn wire_editor(window: &MainWindow, app: &Rc<App>) {
    // Writing from the feed goes to the account's blog or a channel it runs;
    // from a channel's page, to that channel or as a suggestion to it.
    window.on_write_post({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move || {
            let Some(window) = weak.upgrade() else { return };
            let channel = feed::open_channel_whole(&app.feed);
            editor::open_new(&editor_context(&window, &app), channel);
        }
    });
    on_row!(window, app, on_edit_post, |window, app, index| {
        if let Some(article) = feed::post_at(&app.feed, index) {
            editor::open_edit(&editor_context(&window, app), &article);
        }
    });
    on_row!(window, app, on_pin_post, |window, app, index| {
        feed::toggle_pin(&window, &app.feed, &app.client, app.http.clone(), index)
    });
    on_row!(window, app, on_delete_post, |window, app, index| {
        feed::delete_post(&window, &app.feed, &app.client, index)
    });
    on_row!(
        window,
        app,
        on_editor_select_channel,
        |window, app, index| { editor::select_channel(&editor_context(&window, app), index) }
    );
    on_row!(window, app, on_editor_remove_block, |window, app, index| {
        editor::remove_block(&editor_context(&window, app), index)
    });
    {
        let app = Rc::clone(app);
        window.on_editor_set_text(move |index, text| {
            if let Ok(index) = usize::try_from(index) {
                editor::set_text(&app.editor, index, &text);
            }
        });
    }
    {
        let app = Rc::clone(app);
        window.on_editor_set_caption(move |index, text| {
            if let Ok(index) = usize::try_from(index) {
                editor::set_caption(&app.editor, index, &text);
            }
        });
    }
    {
        let app = Rc::clone(app);
        let weak = window.as_weak();
        window.on_editor_add_block(move |kind| {
            let Some(window) = weak.upgrade() else { return };
            editor::add_block(&editor_context(&window, &app), &kind);
        });
    }
    {
        let app = Rc::clone(app);
        let weak = window.as_weak();
        window.on_editor_move_block(move |index, delta| {
            let Some(window) = weak.upgrade() else { return };
            let Ok(index) = usize::try_from(index) else {
                return;
            };
            editor::move_block(&editor_context(&window, &app), index, delta);
        });
    }
    {
        let app = Rc::clone(app);
        let weak = window.as_weak();
        window.on_editor_publish(move || {
            let Some(window) = weak.upgrade() else { return };
            let again = Rc::clone(&app);
            editor::publish(&editor_context(&window, &app), move |window| {
                feed::reload(window, &again.feed, &again.client, again.http.clone(), true);
            });
        });
    }
}

fn admin_context<'a>(window: &'a MainWindow, app: &'a App) -> channel_admin::Context<'a> {
    channel_admin::Context {
        window,
        state: &app.admin,
        client: &app.client,
        http: app.http.clone(),
    }
}

/// A window callback that acts on the channel being run.
macro_rules! on_admin {
    ($window:expr, $app:expr, $setter:ident, |$cx:ident $(, $arg:ident)*| $body:expr) => {{
        let app = Rc::clone($app);
        let weak = $window.as_weak();
        $window.$setter(move |$($arg),*| {
            let Some(window) = weak.upgrade() else { return };
            let $cx = admin_context(&window, &app);
            $body;
        });
    }};
}

fn wire_channel_admin(window: &MainWindow, app: &Rc<App>) {
    {
        let app = Rc::clone(app);
        let weak = window.as_weak();
        window.on_manage_channel(move || {
            let Some(window) = weak.upgrade() else { return };
            if let Some(channel) = feed::open_channel_whole(&app.feed) {
                channel_admin::open(&admin_context(&window, &app), channel);
            }
        });
    }
    on_admin!(window, app, on_create_channel, |cx| channel_admin::create(
        &cx
    ));
    {
        let app = Rc::clone(app);
        let weak = window.as_weak();
        window.on_create_blog(move || {
            let Some(window) = weak.upgrade() else { return };
            let again = Rc::clone(&app);
            channel_admin::create_blog(&window, &app.client, move |window, blog| {
                feed::open_found_channel(
                    window,
                    &again.feed,
                    &again.client,
                    again.http.clone(),
                    blog,
                );
            });
        });
    }
    {
        let app = Rc::clone(app);
        let weak = window.as_weak();
        window.on_admin_save(move || {
            let Some(window) = weak.upgrade() else { return };
            let again = Rc::clone(&app);
            channel_admin::save(&admin_context(&window, &app), move |window, channel| {
                // The channel as it now is, on its own page.
                feed::open_found_channel(
                    window,
                    &again.feed,
                    &again.client,
                    again.http.clone(),
                    channel,
                );
            });
        });
    }
    on_admin!(window, app, on_admin_change_avatar, |cx| {
        channel_admin::change_picture(&cx, false)
    });
    on_admin!(window, app, on_admin_change_cover, |cx| {
        channel_admin::change_picture(&cx, true)
    });
    on_admin!(window, app, on_admin_delete_cover, |cx| {
        channel_admin::delete_cover(&cx)
    });
    on_admin!(window, app, on_admin_select_tab, |cx, tab| {
        channel_admin::select_tab(&cx, tab)
    });
    on_admin!(window, app, on_admin_search, |cx, query| {
        channel_admin::search(&cx, query.into())
    });
    on_admin!(window, app, on_admin_publish, |cx, index, signed| {
        if let Ok(index) = usize::try_from(index) {
            channel_admin::publish(&cx, index, signed);
        }
    });
    on_admin!(window, app, on_admin_reject, |cx, index| {
        if let Ok(index) = usize::try_from(index) {
            channel_admin::reject(&cx, index);
        }
    });
    on_admin!(window, app, on_admin_remove_admin, |cx, index| {
        if let Ok(index) = usize::try_from(index) {
            channel_admin::remove_admin(&cx, index);
        }
    });
    on_admin!(window, app, on_admin_make_admin, |cx, index| {
        if let Ok(index) = usize::try_from(index) {
            channel_admin::make_admin(&cx, index);
        }
    });
    on_admin!(window, app, on_admin_unblock, |cx, index| {
        if let Ok(index) = usize::try_from(index) {
            channel_admin::unblock(&cx, index);
        }
    });
    on_admin!(window, app, on_admin_block, |cx, index| {
        if let Ok(index) = usize::try_from(index) {
            channel_admin::block(&cx, index);
        }
    });
}

fn wire_reports(window: &MainWindow, app: &Rc<App>) {
    use anirust_api::{CommentTarget, ReportTarget};

    window.on_report({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move |kind, index| {
            let Some(window) = weak.upgrade() else { return };
            let row = usize::try_from(index).ok();
            let found: Option<(ReportTarget, i64, String)> = match kind.as_str() {
                "release" => app
                    .release
                    .borrow()
                    .release
                    .as_ref()
                    .map(|r| (ReportTarget::Release, r.id, r.title().to_owned())),
                "profile" => people::viewing(&app.people)
                    .map(|(id, login)| (ReportTarget::Profile, id, login)),
                "channel" => feed::open_channel_of(&app.feed)
                    .map(|(id, title)| (ReportTarget::Channel, id, title)),
                "collection" => collections::open_one(&app.collections)
                    .map(|(id, title, _)| (ReportTarget::Collection, id, title)),
                "post" => row
                    .and_then(|row| feed::article_at(&app.feed, row))
                    .map(|(id, title)| (ReportTarget::Article, id, title)),
                "comment" => row
                    .and_then(|row| comments::reportable_at(&app.comments, row))
                    .map(|(thread, id, text)| {
                        let target = match thread {
                            CommentTarget::Release => ReportTarget::ReleaseComment,
                            CommentTarget::Article => ReportTarget::ArticleComment,
                            CommentTarget::Collection => ReportTarget::CollectionComment,
                        };
                        (target, id, text)
                    }),
                _ => None,
            };
            if let Some((target, id, subject)) = found {
                reports::open(&window, &app.reports, &app.client, target, id, &subject);
            }
        }
    });

    window.on_send_report({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move |reason, message| {
            let Some(window) = weak.upgrade() else { return };
            let Ok(reason) = usize::try_from(reason) else {
                return;
            };
            reports::send(&window, &app.reports, &app.client, reason, message.into());
        }
    });
}

fn settings_context<'a>(window: &'a MainWindow, app: &'a App) -> settings::Context<'a> {
    settings::Context {
        window,
        email: &app.email_change,
        session: &app.account,
        people: &app.people,
        client: &app.client,
        http: app.http.clone(),
    }
}

/// A window callback that acts on the account's settings.
macro_rules! on_settings {
    ($window:expr, $app:expr, $setter:ident, |$cx:ident $(, $arg:ident)*| $body:expr) => {{
        let app = Rc::clone($app);
        let weak = $window.as_weak();
        $window.$setter(move |$($arg),*| {
            let Some(window) = weak.upgrade() else { return };
            let $cx = settings_context(&window, &app);
            $body;
        });
    }};
}

fn wire_settings(window: &MainWindow, app: &Rc<App>) {
    {
        let app = Rc::clone(app);
        let weak = window.as_weak();
        window.on_open_settings(move || {
            let Some(window) = weak.upgrade() else { return };
            settings::open(&settings_context(&window, &app));
            standing::load(&window, &app.standing, &app.client);
        });
    }
    {
        let app = Rc::clone(app);
        let weak = window.as_weak();
        window.on_appeal(move |index, message| {
            let Some(window) = weak.upgrade() else { return };
            let Ok(index) = usize::try_from(index) else {
                return;
            };
            standing::appeal(&window, &app.standing, &app.client, index, message.into());
        });
    }
    on_settings!(window, app, on_open_deletion, |cx| standing::open_deletion(
        cx.window, cx.client
    ));
    on_settings!(window, app, on_request_deletion, |cx, password| {
        standing::request(cx.window, cx.client, password.into())
    });
    on_settings!(window, app, on_cancel_deletion, |cx| standing::cancel(
        cx.window, cx.client
    ));
    on_settings!(window, app, on_change_avatar, |cx| settings::change_avatar(
        &cx
    ));
    on_settings!(window, app, on_delete_avatar, |cx| settings::delete_avatar(
        &cx
    ));
    on_settings!(window, app, on_save_status, |cx, status| {
        settings::save_status(&cx, status.into())
    });
    on_settings!(window, app, on_set_privacy, |cx, what, value| {
        settings::set_privacy(&cx, what, value)
    });
    on_settings!(window, app, on_toggle_incognito, |cx| {
        settings::toggle_incognito(&cx)
    });
    on_settings!(
        window,
        app,
        on_save_socials,
        |cx, vk, tg, inst, tt, discord| {
            settings::save_socials(
                &cx,
                [vk, tg, inst, tt, discord].map(|page| page.to_string()),
            )
        }
    );
    on_settings!(window, app, on_change_login, |cx, login| {
        settings::change_login(&cx, login.into())
    });
    on_settings!(window, app, on_change_password, |cx, current, new| {
        settings::change_password(&cx, current.into(), new.into())
    });
    on_settings!(
        window,
        app,
        on_change_email,
        |cx, current, password, new| {
            settings::change_email(&cx, current.into(), password.into(), new.into())
        }
    );
    on_settings!(window, app, on_verify_email, |cx, code| {
        settings::verify_email(&cx, code.into())
    });
    on_settings!(window, app, on_resend_email, |cx| settings::resend_email(
        &cx
    ));
    on_settings!(window, app, on_import_bookmarks, |cx| {
        bookmarks::import(cx.window, cx.client)
    });
    on_settings!(window, app, on_export_bookmarks, |cx| {
        bookmarks::export(cx.window, cx.client)
    });
}

fn register_context<'a>(window: &'a MainWindow, app: &'a App) -> register::Context<'a> {
    register::Context {
        window,
        pending: &app.pending,
        session: &app.account,
        client: &app.client,
        http: app.http.clone(),
    }
}

fn collections_context<'a>(window: &'a MainWindow, app: &'a App) -> collections::Context<'a> {
    collections::Context {
        window,
        state: &app.collections,
        home: &app.home,
        client: &app.client,
        http: app.http.clone(),
        me: if app.client.is_authenticated() {
            app.account.borrow().id
        } else {
            0
        },
    }
}

/// A window callback with no arguments that acts on collections.
macro_rules! on_collections {
    ($window:expr, $app:expr, $setter:ident, |$cx:ident| $body:expr) => {{
        let app = Rc::clone($app);
        let weak = $window.as_weak();
        $window.$setter(move || {
            let Some(window) = weak.upgrade() else { return };
            let $cx = collections_context(&window, &app);
            $body;
        });
    }};
}

fn wire_collections(window: &MainWindow, app: &Rc<App>) {
    on_collections!(window, app, on_close_collection, |cx| collections::close(
        &cx
    ));
    on_collections!(window, app, on_toggle_collection_favourite, |cx| {
        collections::toggle_favourite(&cx)
    });
    on_collections!(window, app, on_edit_collection, |cx| collections::edit(&cx));
    on_collections!(window, app, on_save_collection, |cx| collections::save(&cx));
    on_collections!(window, app, on_delete_collection, |cx| collections::delete(
        &cx
    ));
    on_collections!(window, app, on_change_collection_cover, |cx| {
        collections::change_cover(&cx)
    });

    window.on_open_collection_comments({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move || {
            let Some(window) = weak.upgrade() else { return };
            let Some((id, title, _)) = collections::open_one(&app.collections) else {
                return;
            };
            comments::open(
                &window,
                &app.comments,
                &app.client,
                viewer(&app),
                anirust_api::CommentTarget::Collection,
                id,
                &title,
            );
        }
    });

    window.on_open_collection_creator({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move || {
            let Some(window) = weak.upgrade() else { return };
            if let Some((_, _, creator)) = collections::open_one(&app.collections) {
                open_person(&window, &app, creator);
            }
        }
    });

    // From a release's page the new collection starts with that release.
    window.on_new_collection({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move |with_release| {
            let Some(window) = weak.upgrade() else { return };
            let first = if with_release >= 0 {
                app.release.borrow().release.as_ref().map(|r| r.id)
            } else {
                None
            };
            collections::create(&collections_context(&window, &app), first);
        }
    });

    window.on_add_to_collection({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move || {
            let Some(window) = weak.upgrade() else { return };
            let Some(id) = app.release.borrow().release.as_ref().map(|r| r.id) else {
                return;
            };
            collections::pick_for(&collections_context(&window, &app), id);
        }
    });

    window.on_pick_collection({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move |index| {
            let Some(window) = weak.upgrade() else { return };
            let Ok(index) = usize::try_from(index) else {
                return;
            };
            collections::pick(&collections_context(&window, &app), index);
        }
    });
}

fn wire_home(window: &MainWindow, app: &Rc<App>) {
    let weak = window.as_weak();

    window.on_load_more_results({
        let app = Rc::clone(app);
        let weak = weak.clone();
        move || {
            let Some(window) = weak.upgrade() else { return };
            home::load_more(&window, &app.home, &app.client, app.http.clone());
        }
    });

    on_row!(window, app, on_open_found_person, |window, app, index| {
        let found = app.home.borrow().found_person(index);
        if let Some(id) = found {
            open_person(&window, app, id);
        }
    });

    // A channel opens on the feed, which is where channels live.
    on_row!(window, app, on_open_found_channel, |window, app, index| {
        let found = app.home.borrow().found_channel(index);
        if let Some(channel) = found {
            window.set_screen("home".into());
            home::select_destination(
                &window,
                &app.home,
                Rc::clone(&app.client),
                app.http.clone(),
                FEED_DESTINATION as usize,
            );
            feed::open_found_channel(&window, &app.feed, &app.client, app.http.clone(), channel);
        }
    });

    window.on_search({
        let app = Rc::clone(app);
        let weak = weak.clone();
        move |query| {
            let Some(window) = weak.upgrade() else { return };
            home::search(
                &window,
                &app.home,
                Rc::clone(&app.client),
                app.http.clone(),
                query.to_string(),
            );
        }
    });

    window.on_clear_finished_downloads({
        let app = Rc::clone(app);
        let weak = weak.clone();
        move || {
            let Some(window) = weak.upgrade() else { return };
            downloads::clear_finished(&window, &app.queue);
        }
    });

    window.on_select_destination({
        let app = Rc::clone(app);
        let weak = weak.clone();
        move |index| {
            let Some(window) = weak.upgrade() else { return };
            home::select_destination(
                &window,
                &app.home,
                Rc::clone(&app.client),
                app.http.clone(),
                index.max(0) as usize,
            );
            // The profile is not a grid, so nothing above fetches anything for
            // it. What it shows goes stale as soon as an episode is watched.
            if index == PROFILE_DESTINATION {
                // The rail always means the account's own profile; someone
                // else's is reached from their name, not from here.
                people::back_to_mine(&window, &app.people);
                window.set_profile_can_go_back(false);
                app.came_from.borrow_mut().take();
                people::load_requests(&window, &app.people, &app.client, app.http.clone());
                session::refresh_profile(
                    &window,
                    &app.account,
                    &app.people,
                    &app.client,
                    app.http.clone(),
                );
                notifications::load_switches(&window, &app.client);
                session::load_recent(
                    &window,
                    &app.account,
                    Rc::clone(&app.client),
                    app.http.clone(),
                );
            }
            if index == SETTINGS_DESTINATION {
                notifications::load_switches(&window, &app.client);
            }
            if index == FEED_DESTINATION {
                feed::open(
                    &window,
                    &app.feed,
                    &app.client,
                    app.http.clone(),
                    app.account.borrow().authenticated,
                );
            }
        }
    });

    window.on_select_tab({
        let app = Rc::clone(app);
        let weak = weak.clone();
        move |index| {
            let Some(window) = weak.upgrade() else { return };
            home::select_tab(
                &window,
                &app.home,
                Rc::clone(&app.client),
                app.http.clone(),
                index.max(0) as usize,
            );
        }
    });

    window.on_open_recent({
        let app = Rc::clone(app);
        let weak = weak.clone();
        move |index| {
            let Some(window) = weak.upgrade() else { return };
            let Some(release_id) = app
                .account
                .borrow()
                .recent_at(index.max(0) as usize)
                .map(|release| release.id)
            else {
                return;
            };

            window.set_screen("release".into());
            release::load(
                &window,
                &app.release,
                Rc::clone(&app.client),
                app.http.clone(),
                release_id,
            );
        }
    });

    window.on_remove_recent({
        let app = Rc::clone(app);
        let weak = weak.clone();
        move |index| {
            let Some(window) = weak.upgrade() else { return };
            let Ok(index) = usize::try_from(index) else {
                return;
            };
            session::remove_recent(
                &window,
                &app.account,
                Rc::clone(&app.client),
                app.http.clone(),
                index,
            );
        }
    });

    window.on_open_list({
        let app = Rc::clone(app);
        let weak = weak.clone();
        move |index| {
            let Some(window) = weak.upgrade() else { return };
            home::open_list(
                &window,
                &app.home,
                Rc::clone(&app.client),
                app.http.clone(),
                index.max(0) as usize,
            );
        }
    });

    window.on_open_random({
        let app = Rc::clone(app);
        let weak = weak.clone();
        move || {
            let Some(window) = weak.upgrade() else { return };
            let api = (*app.client).clone();
            let app = Rc::clone(&app);
            let weak = window.as_weak();
            tasks::spawn(
                async move { api.random_release(false).await },
                move |found| {
                    let Some(window) = weak.upgrade() else { return };
                    match found {
                        Ok(release) if release.id > 0 => {
                            window.set_screen("release".into());
                            release::load(
                                &window,
                                &app.release,
                                Rc::clone(&app.client),
                                app.http.clone(),
                                release.id,
                            );
                        }
                        Ok(_) => tracing::warn!("the service offered no random release"),
                        Err(error) => tracing::warn!(%error, "no random release"),
                    }
                },
            );
        }
    });

    window.on_select_genre({
        let app = Rc::clone(app);
        let weak = weak.clone();
        move |index| {
            let Some(window) = weak.upgrade() else { return };
            home::select_genre(
                &window,
                &app.home,
                Rc::clone(&app.client),
                app.http.clone(),
                index.max(0) as usize,
            );
        }
    });

    let app = Rc::clone(app);
    window.on_open_release(move |index| {
        let Some(window) = weak.upgrade() else { return };
        // A grid of collections opens the collection instead.
        let collection = app
            .home
            .borrow()
            .collection_at(index.max(0) as usize)
            .cloned();
        if let Some(collection) = collection {
            collections::open(&collections_context(&window, &app), collection);
            return;
        }
        let Some(release_id) = app
            .home
            .borrow()
            .release_at(index.max(0) as usize)
            .map(|release| release.id)
        else {
            return;
        };

        window.set_screen("release".into());
        release::load(
            &window,
            &app.release,
            Rc::clone(&app.client),
            app.http.clone(),
            release_id,
        );
    });
}

// ---------------------------------------------------------------------------
// Release screen
// ---------------------------------------------------------------------------

fn wire_release(window: &MainWindow, app: &Rc<App>) {
    let weak = window.as_weak();

    window.on_select_dubber({
        let app = Rc::clone(app);
        let weak = weak.clone();
        move |index| {
            let Some(window) = weak.upgrade() else { return };
            release::select_dubber(
                &window,
                &app.release,
                Rc::clone(&app.client),
                index.max(0) as usize,
            );
        }
    });

    window.on_select_source({
        let app = Rc::clone(app);
        let weak = weak.clone();
        move |index| {
            let Some(window) = weak.upgrade() else { return };
            release::select_source(
                &window,
                &app.release,
                Rc::clone(&app.client),
                index.max(0) as usize,
            );
        }
    });

    window.on_play_episode({
        let app = Rc::clone(app);
        let weak = weak.clone();
        move |position| {
            let Some(window) = weak.upgrade() else { return };
            play(&window, &app, position);
        }
    });

    window.on_download_episode({
        let app = Rc::clone(app);
        let weak = weak.clone();
        move |position| {
            let Some(window) = weak.upgrade() else { return };
            let job = {
                let state = app.release.borrow();
                let Some(episode) = state.episode_at(position) else {
                    return;
                };
                downloads::Job {
                    release: state
                        .release
                        .as_ref()
                        .map(|r| r.title().to_owned())
                        .unwrap_or_default(),
                    position,
                    dubber: state
                        .selected_dubber()
                        .map(|d| d.name.clone())
                        .unwrap_or_default(),
                    url: episode.url.clone(),
                }
            };
            downloads::enqueue(&window, &app.queue, &app.registry, &app.http, job);
        }
    });

    let app = Rc::clone(app);
    window.on_set_release_list({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move |index| {
            let Some(window) = weak.upgrade() else { return };
            release::set_list(&window, &app.release, &app.client, index);
        }
    });

    window.on_toggle_release_favourite({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move || {
            let Some(window) = weak.upgrade() else { return };
            release::toggle_favourite(&window, &app.release, &app.client);
        }
    });

    window.on_set_release_vote({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move |stars| {
            let Some(window) = weak.upgrade() else { return };
            release::set_vote(&window, &app.release, &app.client, stars);
        }
    });

    window.on_open_franchise({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move || {
            let Some(window) = weak.upgrade() else { return };
            release::open_franchise(&window, &app.release, &app.client, app.http.clone());
        }
    });

    window.on_open_franchise_item({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move |index| {
            let Some(window) = weak.upgrade() else { return };
            let Some(release_id) = usize::try_from(index)
                .ok()
                .and_then(|at| release::franchise_at(&app.release, at))
            else {
                return;
            };
            window.set_franchise_open(false);
            release::load(
                &window,
                &app.release,
                Rc::clone(&app.client),
                app.http.clone(),
                release_id,
            );
        }
    });

    window.on_open_related({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move |index| {
            let Some(window) = weak.upgrade() else { return };
            let Some(release_id) = usize::try_from(index)
                .ok()
                .and_then(|at| app.release.borrow().related.get(at).map(|r| r.id))
            else {
                return;
            };
            release::load(
                &window,
                &app.release,
                Rc::clone(&app.client),
                app.http.clone(),
                release_id,
            );
        }
    });

    window.on_open_video({
        let app = Rc::clone(&app);
        move |index| {
            if let Ok(index) = usize::try_from(index) {
                release::open_video(&app.release, index);
            }
        }
    });

    window.on_open_platform({
        let app = Rc::clone(&app);
        move |index| {
            let url = usize::try_from(index)
                .ok()
                .and_then(|at| app.release.borrow().platforms.get(at).cloned());
            if let Some(url) = url {
                release::open_in_browser(&url);
            }
        }
    });

    window.on_toggle_dubber_pin({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move || {
            let Some(window) = weak.upgrade() else { return };
            // Pinning is the account's, so without one it is a reason to sign
            // in rather than a click that does nothing.
            if !app.client.is_authenticated() {
                window.set_sign_in_error("".into());
                window.set_show_sign_in(true);
                return;
            }
            release::toggle_pin(&window, &app.release, &app.client);
        }
    });

    window.on_toggle_watched({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move |position| {
            let Some(window) = weak.upgrade() else { return };
            release::toggle_watched(&window, &app.release, &app.client, position);
        }
    });

    window.on_toggle_all_watched({
        let app = Rc::clone(&app);
        let weak = window.as_weak();
        move || {
            let Some(window) = weak.upgrade() else { return };
            release::toggle_all_watched(&window, &app.release, &app.client);
        }
    });

    window.on_go_back(move || {
        let Some(window) = weak.upgrade() else { return };
        // Leaving the release leaves its episode: a player decoding behind a
        // screen nobody watches holds its frames, its buffer and the network
        // for nothing.
        if window.get_playing() {
            window.invoke_close_player();
        }
        window.set_screen("home".into());
        // Opened straight into a release from the command line, the browsing
        // screen behind it was never filled. Going back to an empty grid would
        // be a dead end.
        if app.home.borrow().releases.is_empty() {
            home::open(&window, &app.home, Rc::clone(&app.client), app.http.clone());
        }
    });
}

/// Resolves an episode and hands it to the player.
///
/// The selected source is tried first and the rest of the release after it.
/// One host failing is routine rather than exceptional — Kodik answers `500`
/// for stretches at a time and carries most of the catalogue — so giving up on
/// the first failure would present a temporary outage as a broken client.
fn play(window: &MainWindow, app: &Rc<App>, position: i32) {
    let (attempt, dubber, resume, index) = {
        let state = app.release.borrow();
        let Some(episode) = state.episode_at(position) else {
            tracing::warn!(position, "no such episode in this source");
            return;
        };
        let Some(attempt) = Attempt::for_episode(&state, position) else {
            tracing::warn!(position, "no voice-over selected");
            return;
        };
        (
            attempt,
            state.selected_dubber().map(|d| d.name.clone()),
            state.resume_of(episode),
            state.index_of(position).unwrap_or_default(),
        )
    };

    window.set_current_episode(position);
    window.set_current_index(index as i32);
    window.set_episode_label(match &dubber {
        Some(name) => format!("{position} - {name}").into(),
        None => position.to_string().into(),
    });
    show_neighbours(window, &app.release, position);

    // The picture area is handed over before the stream resolves: resolution
    // goes through a CDN that can take tens of seconds just to accept a
    // connection, and the viewer should watch that happen in the player rather
    // than on a screen that appears to have ignored the click.
    window.set_has_video(false);
    window.set_state("loading".into());
    window.set_qualities(slint::ModelRc::new(slint::VecModel::from(Vec::<
        slint::SharedString,
    >::new())));
    window.set_has_skip(false);
    window.set_episode_failed(false);
    window.set_playing(true);

    // "Continue watching" on every other client the account is signed in to is
    // built from this.
    record_in_history(&app.release, &app.client, position);

    let weak = window.as_weak();
    let api = (*app.client).clone();
    let resolver = (*app.registry).clone();
    let app = Rc::clone(app);

    tasks::spawn(
        async move { attempt.resolve(&api, &resolver).await },
        move |resolved| {
            let Some(window) = weak.upgrade() else { return };

            // The viewer may have stepped on while this was in flight; what
            // came back is then for an episode nobody is waiting for.
            if window.get_current_episode() != position {
                tracing::debug!(position, "discarding a stream for a superseded episode");
                return;
            }

            match resolved {
                Ok(stream) => start(
                    &window,
                    &app.bridge,
                    &app.playing,
                    &app.settings,
                    stream,
                    resume,
                ),
                Err(error) => {
                    tracing::error!(%error, position, "could not resolve the episode");
                    // Nothing to show, so the picture area goes back to the
                    // release — saying why, rather than looking like the click
                    // was ignored.
                    window.set_state("idle".into());
                    window.set_playing(false);
                    window.set_current_episode(0);
                    window.set_episode_failed(true);
                }
            }
        },
    );
}

/// Everywhere one episode might be found, in the order worth trying.
///
/// Fallback stays inside the chosen voice-over: dropping to another one would
/// silently change the language being spoken, which is not a decision a failed
/// request gets to make.
struct Attempt {
    release_id: i64,
    dubber_id: i64,
    position: i32,
    /// The URL the selected source already gave us, so the common case costs
    /// no extra request.
    known: Option<String>,
    /// The voice-over's other sources, tried only if that URL fails.
    fallbacks: Vec<i64>,
}

impl Attempt {
    fn for_episode(state: &ReleaseState, position: i32) -> Option<Self> {
        let dubber = state.selected_dubber()?;
        let selected = state.selected_source().map(|s| s.id);

        Some(Self {
            release_id: state.release_id,
            dubber_id: dubber.id,
            position,
            known: state.episode_at(position).map(|e| e.url.clone()),
            fallbacks: state
                .sources
                .iter()
                .map(|s| s.id)
                .filter(|id| Some(*id) != selected)
                .collect(),
        })
    }

    /// Resolves the first source that yields a playable stream.
    async fn resolve(self, client: &Client, registry: &Registry) -> Result<ResolvedStream> {
        let mut failures = Vec::new();

        if let Some(url) = self.known
            && let Some(stream) = try_url(registry, url, &mut failures).await
        {
            return Ok(stream);
        }

        for source_id in self.fallbacks {
            let episodes = match client
                .episodes(
                    self.release_id,
                    self.dubber_id,
                    source_id,
                    EpisodeSort::Ascending,
                )
                .await
            {
                Ok(episodes) => episodes,
                Err(error) => {
                    failures.push(format!("source {source_id}: {error}"));
                    continue;
                }
            };

            let Some(episode) = episodes.into_iter().find(|e| e.position == self.position) else {
                continue;
            };

            if let Some(stream) = try_url(registry, episode.url, &mut failures).await {
                return Ok(stream);
            }
        }

        bail!(
            "no source could play episode {}:\n  {}",
            self.position,
            failures.join("\n  ")
        )
    }
}

/// Resolves one URL, recording why it failed rather than propagating it.
async fn try_url(
    registry: &Registry,
    url: String,
    failures: &mut Vec<String>,
) -> Option<ResolvedStream> {
    match resolve_url(registry, url.clone()).await {
        Ok(stream) if stream.best().is_some() => Some(stream),
        Ok(_) => {
            failures.push(format!("{url}: nothing playable"));
            None
        }
        Err(error) => {
            tracing::warn!(%url, %error, "source failed, trying the next");
            failures.push(format!("{url}: {error}"));
            None
        }
    }
}

/// Points the player at a resolved stream.
fn start(
    window: &MainWindow,
    bridge: &Rc<VideoBridge>,
    playing: &Playing,
    settings: &Rc<Settings>,
    stream: ResolvedStream,
    resume: Option<Duration>,
) {
    let Some(best) = stream.best() else {
        tracing::error!("resolved a stream with no renditions");
        window.set_state("idle".into());
        window.set_playing(false);
        window.set_episode_failed(true);
        return;
    };
    tracing::info!(url = %best.url, height = best.height, "resolved");

    window.set_qualities(slint::ModelRc::new(slint::VecModel::from(
        stream
            .variants
            .iter()
            .map(|v| quality_name(v.height).into())
            .collect::<Vec<slint::SharedString>>(),
    )));
    window.set_quality(0);
    window.set_has_skip(stream.opening.is_some());

    let source_url = best.url.clone();
    let mut source = {
        let (url, headers) = relay::route(&best.url, stream.headers.clone());
        MediaSource::new(&url).headers(
            headers
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str())),
        )
    };
    if let Some(at) = resume {
        source = source.start_at(at);
    }

    if let Err(error) = bridge.play(source) {
        tracing::error!(error = format!("{error:#}"), url = %source_url, "could not start playback");
        window.set_state("idle".into());
        window.set_playing(false);
        window.set_episode_failed(true);
        return;
    }

    *playing.borrow_mut() = Some(stream);
    // A different file: whatever the menus list now belongs to the last one.
    settings.tracks_stale.set(true);
}

/// Turns one episode URL into a stream, through an extractor when a host
/// claims it.
///
/// Routing is by host rather than by the API's `iframe` flag, which lies for
/// several of them.
async fn resolve_url(registry: &Registry, url: String) -> Result<ResolvedStream> {
    if registry.supports(&url) {
        return Ok(registry.resolve(&url).await?);
    }

    Ok(ResolvedStream {
        variants: vec![anirust_extract::StreamVariant {
            height: anirust_extract::UNKNOWN_HEIGHT,
            kind: anirust_extract::StreamKind::classify(None, &url),
            url,
        }],
        ..Default::default()
    })
}

/// Adds an episode to the account's history.
///
/// Silent when signed out, which is most of the time: an anonymous client has
/// nowhere to put this, and the local store already covers the same ground for
/// this machine.
fn record_in_history(state: &Rc<RefCell<ReleaseState>>, client: &Rc<Client>, position: i32) {
    if !client.is_authenticated() {
        return;
    }

    let (release_id, source_id) = {
        let state = state.borrow();
        (
            state.release_id,
            state.selected_source().map(|source| source.id),
        )
    };
    let Some(source_id) = source_id else { return };

    let api = (**client).clone();
    tasks::spawn(
        async move { api.history_add(release_id, source_id, position).await },
        move |result| {
            if let Err(error) = result {
                tracing::warn!(%error, position, "could not add to history");
            }
        },
    );
}

/// Tells the transport whether there is an episode either side of this one.
fn show_neighbours(window: &MainWindow, state: &Rc<RefCell<ReleaseState>>, position: i32) {
    let state = state.borrow();
    window.set_has_previous(state.previous_before(position).is_some());
    window.set_has_next(state.next_after(position).is_some());
}

// ---------------------------------------------------------------------------
// Player screen
// ---------------------------------------------------------------------------

/// What the overlay's controls are set to.
///
/// Kept beside the player because "which preset is selected" is a choice the
/// interface owns; mpv has no notion of a preset once a shader list is applied.
struct Settings {
    speed: Cell<f64>,
    upscale_mode: Cell<usize>,
    upscale_quality: Cell<usize>,
    interpolation: Cell<bool>,
    force_4k: Cell<bool>,
    /// The episode loops instead of going on to the next. Not kept: it is a
    /// choice about this episode.
    repeat: Cell<bool>,
    /// Shifts in seconds. Not kept either: they belong to a source, and the
    /// next run may well play another.
    audio_delay: Cell<f64>,
    subtitle_delay: Cell<f64>,
    normalize: Cell<bool>,
    subtitle_scale: Cell<usize>,
    ass_override: Cell<bool>,
    picture: Cell<usize>,
    /// Frame generation as the menu has it: 0 off, then the target rates.
    frame_rate: Cell<usize>,
    rife_model: Cell<usize>,
    /// Where the Vulkan RIFE is, looked for once.
    rife: Option<RifeInstall>,
    /// Which engine runs the networks: 0 Vulkan, then vs-mlrt on each of
    /// [`Backend::ALL`] — TensorRT, OpenVINO, MIGraphX.
    engine: Cell<usize>,
    /// Real-ESRGAN doubling the picture first. vs-mlrt only.
    neural_upscale: Cell<bool>,
    /// Which of the vs-mlrt plugins the program carries, by backend.
    shipped: [bool; 3],
    /// vs-mlrt on each backend whose vendor runtime is there too.
    mlrt: RefCell<[Option<Mlrt>; 3]>,
    /// What fetching a vendor runtime is doing, for the line on screen.
    runtime_fetch: RefCell<String>,
    /// When frame generation was last switched on, until it has been seen
    /// working or failing.
    generation_since: Cell<Option<std::time::Instant>>,
    /// Whether the line about TensorRT building an engine has been shown
    /// since the filters were last applied.
    engine_build_said: Cell<bool>,
    /// The monitor's refresh rate as last told to mpv.
    display_fps: Cell<Option<f64>>,
    /// Whether to lower the load when frames drop, and the watch doing it.
    adaptive: Cell<bool>,
    watch: RefCell<adapt::Watch>,
    quality: Cell<usize>,
    decoder: Cell<usize>,
    /// mpv track ids behind the subtitle and audio menus.
    ///
    /// The menus are lists of labels, but mpv selects by id, and ids are
    /// neither contiguous nor equal to a position in the list. `None` in the
    /// subtitle list is the "off" entry.
    subtitles: RefCell<Vec<Option<i64>>>,
    audio: RefCell<Vec<Option<i64>>>,
    /// The subtitle track to restore when captions are switched back on.
    last_subtitle: Cell<Option<i64>>,
    /// Set when a different file is loaded.
    ///
    /// The menus are otherwise rebuilt only when the stream count changes, and
    /// two consecutive episodes routinely have the same count with different
    /// languages in it.
    tracks_stale: Cell<bool>,
}

impl Settings {
    /// The player as the preferences open it.
    fn from_preferences(kept: preferences::PlayerPreferences) -> Self {
        Self {
            speed: Cell::new(kept.speed),
            upscale_mode: Cell::new(kept.upscale_mode.min(UpscaleMode::ALL.len() - 1)),
            upscale_quality: Cell::new(kept.upscale_quality.min(UpscaleQuality::ALL.len() - 1)),
            interpolation: Cell::new(kept.interpolation),
            force_4k: Cell::new(kept.force_4k),
            repeat: Cell::new(false),
            audio_delay: Cell::new(0.0),
            subtitle_delay: Cell::new(0.0),
            normalize: Cell::new(kept.normalize),
            subtitle_scale: Cell::new(kept.subtitle_scale.min(SUBTITLE_SCALES.len() - 1)),
            ass_override: Cell::new(kept.ass_override),
            picture: Cell::new(kept.picture.min(PictureAdjust::PRESETS.len() - 1)),
            frame_rate: Cell::new(kept.frame_rate.min(adapt::RATE_HALF_DISPLAY)),
            rife_model: Cell::new(kept.rife_model.min(RifeModel::ALL.len() - 1)),
            rife: RifeInstall::find(),
            engine: Cell::new(kept.engine.min(Backend::ALL.len())),
            neural_upscale: Cell::new(kept.neural_upscale),
            shipped: Backend::ALL.map(Mlrt::ships),
            mlrt: RefCell::new(Backend::ALL.map(Mlrt::find)),
            runtime_fetch: RefCell::new(String::new()),
            generation_since: Cell::new(None),
            engine_build_said: Cell::new(false),
            display_fps: Cell::new(None),
            adaptive: Cell::new(kept.adaptive),
            watch: RefCell::new(adapt::Watch::default()),
            quality: Cell::new(0),
            decoder: Cell::new(kept.decoder.min(DECODERS.len() - 1)),
            subtitles: RefCell::new(Vec::new()),
            audio: RefCell::new(Vec::new()),
            last_subtitle: Cell::new(None),
            tracks_stale: Cell::new(true),
        }
    }
}

impl Settings {
    /// The load the menus put on the GPU.
    fn load(&self) -> adapt::Load {
        adapt::Load {
            upscale_mode: self.upscale_mode.get(),
            upscale_quality: self.upscale_quality.get(),
            frame_rate: self.frame_rate.get(),
            rife_model: self.rife_model.get(),
            neural_upscale: self.on_tensorrt() && self.neural_upscale.get(),
            display_hz: self.display_fps.get().map_or(0, |fps| fps.round() as u32),
        }
    }

    fn set_load(&self, load: adapt::Load) {
        self.upscale_mode.set(load.upscale_mode);
        self.upscale_quality.set(load.upscale_quality);
        self.frame_rate.set(load.frame_rate);
        self.rife_model.set(load.rife_model);
        if !load.neural_upscale {
            self.neural_upscale.set(false);
        }
    }

    /// The frame generation the menus are set to, if any and if possible.
    /// The rate the frame-generation menu asks for, if any.
    fn target_rate(&self) -> Option<TargetRate> {
        match self.frame_rate.get() {
            adapt::RATE_DOUBLE => Some(TargetRate::Double),
            adapt::RATE_SIXTY => Some(TargetRate::Sixty),
            adapt::RATE_DISPLAY => Some(TargetRate::Display),
            adapt::RATE_HALF_DISPLAY => Some(TargetRate::HalfDisplay),
            _ => None,
        }
    }

    /// The vs-mlrt backend chosen, if one is.
    fn backend(&self) -> Option<Backend> {
        Backend::ALL.get(self.engine.get().checked_sub(1)?).copied()
    }

    /// Whether a vs-mlrt backend is chosen and complete.
    fn on_mlrt(&self) -> bool {
        self.backend()
            .is_some_and(|backend| self.mlrt.borrow()[backend_index(backend)].is_some())
    }

    /// Whether TensorRT in particular is chosen and complete: it alone keeps
    /// 1080p at its size, and builds engines the first time.
    fn on_tensorrt(&self) -> bool {
        self.on_mlrt() && self.backend() == Some(Backend::TensorRt)
    }

    /// Looks for the vendor runtimes again, after one was fetched.
    fn refresh_mlrt(&self) {
        *self.mlrt.borrow_mut() = Backend::ALL.map(Mlrt::find);
    }

    /// What the menus ask the neural filters for.
    ///
    /// TensorRT keeps 1080p sources at their size up to about 72 frames a
    /// second — it makes 66 at 1080p — and brings them down to 720p above
    /// that; Vulkan always brings them down.
    fn enhancement(&self) -> Enhancement {
        let tensorrt = self.on_tensorrt();
        let frames = self.target_rate().map(|rate| {
            let target = match rate {
                TargetRate::Double => 60.0,
                TargetRate::Sixty => 60.0,
                TargetRate::Display => self.display_fps.get().unwrap_or(60.0),
                TargetRate::HalfDisplay => self.display_fps.get().unwrap_or(120.0) / 2.0,
            };
            FrameGeneration {
                rate,
                model: RifeModel::ALL[self.rife_model.get()],
                max_height: if tensorrt && target <= 72.0 {
                    1080
                } else {
                    720
                },
            }
        });
        Enhancement {
            frames,
            upscale: self.on_mlrt() && self.neural_upscale.get(),
        }
    }

    /// Applies the menus' frame generation to the player.
    /// Applies the menus' neural filters to the player.
    ///
    /// vs-mlrt when a backend is chosen and complete; Vulkan otherwise,
    /// which also covers a backend chosen before its runtime is there.
    fn apply_generation(&self, player: &Player) -> anirust_player::Result<()> {
        let enhancement = self.enhancement();
        let mlrt = self.mlrt.borrow();
        let chosen = self
            .backend()
            .and_then(|backend| mlrt[backend_index(backend)].as_ref());
        let networks = match (chosen, self.rife.as_ref()) {
            (Some(mlrt), _) => Some(Networks::Mlrt(mlrt)),
            (None, Some(rife)) => Some(Networks::Vulkan(rife)),
            (None, None) => None,
        };
        let wanted = networks.filter(|_| !enhancement.is_empty());
        if wanted.is_some() {
            anirust_player::frames::prepare_vapoursynth();
        }
        self.generation_since
            .set((wanted.is_some() && enhancement.frames.is_some()).then(std::time::Instant::now));
        self.engine_build_said.set(false);
        player.set_enhancement(wanted.map(|networks| (networks, enhancement)))
    }

    /// The upscaling the menus are set to.
    fn upscale(&self) -> UpscalePreset {
        UpscalePreset::new(
            UpscaleMode::ALL[self.upscale_mode.get()],
            UpscaleQuality::ALL[self.upscale_quality.get()],
        )
    }
}

/// mpv `hwdec` values behind the decoder choice, in the order the sheet lists
/// them: automatic, hardware, software.
const DECODERS: [&str; 3] = [anirust_player::DEFAULT_HWDEC, "nvdec,vaapi", "no"];

/// How fast the picture plays while the button is held down on it.
const HOLD_SPEED: f64 = 2.0;

/// Subtitle sizes the menu offers, as mpv's `sub-scale`.
const SUBTITLE_SCALES: [f64; 5] = [0.75, 1.0, 1.25, 1.5, 2.0];

/// How far one press moves a delay, in seconds.
const DELAY_STEP: f64 = 0.1;

/// Where screenshots go: a folder of the user's pictures.
fn screenshots_dir() -> Option<std::path::PathBuf> {
    dirs::picture_dir()
        .or_else(dirs::home_dir)
        .map(|dir| dir.join("AniRust"))
}

/// Where subtitles look for fonts the system lacks. Fan subtitles name fonts
/// nobody has installed; dropped here, they are found.
fn subtitle_fonts_dir() -> Option<std::path::PathBuf> {
    dirs::data_dir().map(|dir| dir.join("anirust").join("fonts"))
}

/// A line for fetching NVIDIA's runtime: how much has come, then unpacking.
/// A backend's name, for lines on screen.
fn backend_name(backend: Backend) -> &'static str {
    match backend {
        Backend::TensorRt => "TensorRT",
        Backend::OpenVino => "OpenVINO",
        Backend::MigraphX => "MIGraphX",
    }
}

/// The engine that suits the GPU in this machine, by its vendor: TensorRT
/// for NVIDIA, MIGraphX for AMD, OpenVINO for Intel, Vulkan when it cannot
/// be told. Read from the kernel's view of the display devices on Linux.
fn recommended_engine() -> usize {
    use std::sync::OnceLock;
    static VENDOR: OnceLock<usize> = OnceLock::new();
    *VENDOR.get_or_init(|| {
        let Ok(cards) = std::fs::read_dir("/sys/class/drm") else {
            return 0;
        };
        let vendors: Vec<String> = cards
            .flatten()
            .filter_map(|card| std::fs::read_to_string(card.path().join("device/vendor")).ok())
            .map(|vendor| vendor.trim().to_owned())
            .collect();
        // A discrete card over an integrated one: NVIDIA, then AMD, then Intel.
        for (vendor, backend) in [
            ("0x10de", Backend::TensorRt),
            ("0x1002", Backend::MigraphX),
            ("0x8086", Backend::OpenVino),
        ] {
            if vendors.iter().any(|found| found == vendor) {
                return backend_index(backend) + 1;
            }
        }
        0
    })
}

/// A backend's place in [`Backend::ALL`], and in the menus' engine list
/// after Vulkan.
fn backend_index(backend: Backend) -> usize {
    Backend::ALL
        .iter()
        .position(|candidate| *candidate == backend)
        .unwrap_or(0)
}

fn describe_fetch(progress: runtimes::Progress, ru: bool) -> String {
    const MB: u64 = 1024 * 1024;
    match (progress, ru) {
        (
            runtimes::Progress::Fetching {
                done,
                total: Some(total),
            },
            true,
        ) => {
            format!("Скачано {} из {} МБ", done / MB, total / MB)
        }
        (
            runtimes::Progress::Fetching {
                done,
                total: Some(total),
            },
            false,
        ) => {
            format!("{} of {} MB", done / MB, total / MB)
        }
        (runtimes::Progress::Fetching { done, total: None }, true) => {
            format!("Скачано {} МБ", done / MB)
        }
        (runtimes::Progress::Fetching { done, total: None }, false) => format!("{} MB", done / MB),
        (runtimes::Progress::Unpacking, true) => "Распаковываю…".to_owned(),
        (runtimes::Progress::Unpacking, false) => "Unpacking…".to_owned(),
    }
}

/// A delay as the menu shows it: "0", "+0.3", "−1.2", with a decimal comma
/// in Russian.
fn format_delay(seconds: f64, ru: bool) -> String {
    if seconds.abs() < DELAY_STEP / 2.0 {
        return "0".to_owned();
    }
    let text = format!("{seconds:+.1}").replace('-', "−");
    if ru { text.replace('.', ",") } else { text }
}

/// A delay moved one step either way, or back to none, rounded to the step
/// so that ten presses come back to exactly where they started.
fn step_delay(current: f64, direction: i32) -> f64 {
    if direction == 0 {
        return 0.0;
    }
    ((current + f64::from(direction.signum()) * DELAY_STEP) / DELAY_STEP).round() * DELAY_STEP
}

/// The rates the speed menu offers, which are also the notches `<` and `>`
/// step between.
const SPEEDS: [f64; 7] = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];

/// Puts a line on screen for a moment.
///
/// The counter is not decoration: Slint hides this on a `changed` handler, so
/// pressing the same key twice has to look like a change or the second press
/// would leave the first one's timer to expire on it.
fn show_hint(window: &MainWindow, text: String) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NONCE: AtomicU32 = AtomicU32::new(0);

    let nonce = NONCE.fetch_add(1, Ordering::Relaxed);
    // A zero-width space carries the counter without printing it.
    let padding = "\u{200b}".repeat((nonce % 4) as usize + 1);
    window.set_hint(format!("{text}{padding}").into());
}

fn wire_player(window: &MainWindow, app: &Rc<App>) -> Rc<dyn Fn()> {
    let player = app.bridge.player();
    let settings = &app.settings;
    let playing = &app.playing;
    let bridge = &app.bridge;

    // The player opens as it was left, when that is kept.
    let kept = app.prefs.get().player.in_force();
    let applied = player
        .set_speed(kept.speed)
        .and_then(|()| player.set_volume(kept.volume))
        .and_then(|()| player.set_upscale(settings.upscale()))
        .and_then(|()| player.set_interpolation(kept.interpolation))
        .and_then(|()| player.set_hwdec(DECODERS[settings.decoder.get()]))
        .and_then(|()| player.set_loudness_normalization(settings.normalize.get()))
        .and_then(|()| player.set_subtitle_scale(SUBTITLE_SCALES[settings.subtitle_scale.get()]))
        .and_then(|()| player.set_ass_override(settings.ass_override.get()))
        .and_then(|()| player.set_picture(PictureAdjust::PRESETS[settings.picture.get()]))
        .and_then(|()| settings.apply_generation(player));
    if let Err(error) = applied {
        tracing::warn!(%error, "the kept player settings were not applied");
    }

    bridge.set_force_4k(kept.force_4k);
    window.set_force_4k(kept.force_4k);

    if let Some(fonts) = subtitle_fonts_dir() {
        let ready = std::fs::create_dir_all(&fonts)
            .map_err(anyhow::Error::from)
            .and_then(|()| Ok(player.set_subtitle_fonts_dir(Some(&fonts))?));
        if let Err(error) = ready {
            tracing::warn!(%error, dir = %fonts.display(), "the subtitle fonts folder is not in use");
        }
    }

    // Writes the player's settings out, when they are kept.
    let keep: Rc<dyn Fn()> = {
        let prefs = Rc::clone(&app.prefs);
        let settings = Rc::clone(&app.settings);
        Rc::new(move || {
            let mut all = prefs.get();
            if !all.player.remember {
                return;
            }
            all.player.speed = settings.speed.get();
            all.player.upscale_mode = settings.upscale_mode.get();
            all.player.upscale_quality = settings.upscale_quality.get();
            all.player.interpolation = settings.interpolation.get();
            all.player.decoder = settings.decoder.get();
            all.player.volume = player.volume();
            all.player.force_4k = settings.force_4k.get();
            all.player.normalize = settings.normalize.get();
            all.player.subtitle_scale = settings.subtitle_scale.get();
            all.player.ass_override = settings.ass_override.get();
            all.player.picture = settings.picture.get();
            all.player.frame_rate = settings.frame_rate.get();
            all.player.rife_model = settings.rife_model.get();
            all.player.adaptive = settings.adaptive.get();
            all.player.engine = settings.engine.get();
            all.player.neural_upscale = settings.neural_upscale.get();
            prefs.set(all);
            all.save();
        })
    };

    // Re-sends the pointer where it is, so the cursor the hidden controls
    // asked for is applied without waiting for the mouse to move.
    {
        let keep_now = Rc::clone(&keep);
        let chosen = Rc::clone(settings);
        let bridge = Rc::clone(bridge);
        let weak = window.as_weak();
        window.on_toggle_force_4k(move || {
            let Some(window) = weak.upgrade() else { return };
            let on = !chosen.force_4k.get();
            chosen.force_4k.set(on);
            bridge.set_force_4k(on);
            window.set_force_4k(on);
            keep_now();
        });
    }

    let weak = window.as_weak();
    window.on_pointer_hidden(move |x, y| {
        let Some(window) = weak.upgrade() else { return };
        window
            .window()
            .dispatch_event(slint::platform::WindowEvent::PointerMoved {
                position: slint::LogicalPosition::new(x, y),
            });
    });

    window.on_toggle_pause(move || {
        if let Err(error) = player.toggle_pause() {
            tracing::warn!(%error, "pause failed");
        }
    });

    window.on_seek_relative(move |delta| {
        if let Err(error) = player.seek_by(f64::from(delta)) {
            tracing::warn!(%error, "seek failed");
        }
    });

    window.on_seek_fraction(move |fraction| {
        let Some(duration) = player.duration() else {
            return;
        };
        let target = duration.mul_f64(f64::from(fraction).clamp(0.0, 1.0));
        if let Err(error) = player.seek_to(target) {
            tracing::warn!(%error, "seek failed");
        }
    });

    let current = Rc::clone(playing);
    window.on_skip_opening(move || {
        let ends_at = current
            .borrow()
            .as_ref()
            .and_then(|stream| stream.opening)
            .map(|range| Duration::from_secs(u64::from(range.end)));
        if let Err(error) = player.skip_opening(ends_at) {
            tracing::warn!(%error, "skip failed");
        }
    });

    let advance = wire_stepping(window, app);

    let chosen = Rc::clone(settings);
    let keep_now = Rc::clone(&keep);
    window.on_set_speed(move |speed| {
        let speed = f64::from(speed);
        chosen.speed.set(speed);
        if let Err(error) = player.set_speed(speed) {
            tracing::warn!(%error, "speed change failed");
        }
        keep_now();
    });

    let chosen = Rc::clone(settings);
    let keep_now = Rc::clone(&keep);
    window.on_set_upscale(move |index| {
        let index = (index.max(0) as usize).min(UpscaleMode::ALL.len() - 1);
        chosen.upscale_mode.set(index);
        keep_now();
        let preset = chosen.upscale();
        if let Err(error) = player.set_upscale(preset) {
            tracing::warn!(%error, preset = %preset.name(), "upscale change failed");
        }
    });

    let chosen = Rc::clone(settings);
    let keep_now = Rc::clone(&keep);
    window.on_set_upscale_quality(move |index| {
        let index = (index.max(0) as usize).min(UpscaleQuality::ALL.len() - 1);
        chosen.upscale_quality.set(index);
        keep_now();
        let preset = chosen.upscale();
        if let Err(error) = player.set_upscale(preset) {
            tracing::warn!(%error, preset = %preset.name(), "upscale change failed");
        }
    });

    let chosen = Rc::clone(settings);
    let keep_now = Rc::clone(&keep);
    window.on_toggle_interpolation(move || {
        let next = !chosen.interpolation.get();
        chosen.interpolation.set(next);
        keep_now();
        if let Err(error) = player.set_interpolation(next) {
            tracing::warn!(%error, "interpolation change failed");
        }
    });

    // Switching rendition reopens a different URL, so playback resumes where
    // it left off rather than starting over.
    let chosen = Rc::clone(settings);
    let current = Rc::clone(playing);
    let switch = Rc::clone(bridge);
    window.on_set_quality(move |index| {
        let index = index.max(0) as usize;
        let stream = current.borrow();
        let Some(stream) = stream.as_ref() else {
            return;
        };
        let Some(variant) = stream.variants.get(index) else {
            return;
        };
        chosen.quality.set(index);

        let resume = player.position().unwrap_or_default();
        let mut source = {
            let (url, headers) = relay::route(&variant.url, stream.headers.clone());
            MediaSource::new(&url).headers(
                headers
                    .iter()
                    .map(|(name, value)| (name.as_str(), value.as_str())),
            )
        };
        if resume > Duration::ZERO {
            source = source.start_at(resume);
        }

        if let Err(error) = switch.play(source) {
            tracing::warn!(%error, height = variant.height, "quality change failed");
            return;
        }
        chosen.tracks_stale.set(true);
    });

    let chosen = Rc::clone(settings);
    window.on_set_subtitle_track(move |index| {
        let Some(&id) = chosen.subtitles.borrow().get(index.max(0) as usize) else {
            return;
        };
        if let Err(error) = player.set_subtitle_track(id) {
            tracing::warn!(%error, ?id, "subtitle change failed");
        }
    });

    let chosen = Rc::clone(settings);
    window.on_set_audio_track(move |index| {
        let Some(&Some(id)) = chosen.audio.borrow().get(index.max(0) as usize) else {
            return;
        };
        if let Err(error) = player.set_audio_track(Some(id)) {
            tracing::warn!(%error, id, "audio change failed");
        }
    });

    let chosen = Rc::clone(settings);
    let keep_now = Rc::clone(&keep);
    window.on_set_decoder(move |index| {
        let index = (index.max(0) as usize).min(DECODERS.len() - 1);
        chosen.decoder.set(index);
        keep_now();
        if let Err(error) = player.set_hwdec(DECODERS[index]) {
            tracing::warn!(%error, value = DECODERS[index], "decoder change failed");
        }
    });

    // ---- the keys that change something with no control of its own -------

    let weak = window.as_weak();
    window.on_toggle_muted(move || {
        let Some(window) = weak.upgrade() else { return };
        match player.toggle_muted() {
            Ok(muted) => show_hint(
                &window,
                if muted {
                    "🔇".to_owned()
                } else {
                    format!("🔊 {}%", player.volume())
                },
            ),
            Err(error) => tracing::warn!(%error, "mute failed"),
        }
    });

    let weak = window.as_weak();
    let keep_now = Rc::clone(&keep);
    window.on_adjust_volume(move |step| {
        let Some(window) = weak.upgrade() else { return };
        let volume = (player.volume() + i64::from(step)).clamp(0, 150);
        if let Err(error) = player.set_volume(volume) {
            tracing::warn!(%error, volume, "volume change failed");
            return;
        }
        keep_now();
        // Reaching for the volume means wanting to hear it.
        let _ = player.set_muted(false);
        show_hint(&window, format!("🔊 {volume}%"));
    });

    let chosen = Rc::clone(settings);
    let weak = window.as_weak();
    let keep_now = Rc::clone(&keep);
    window.on_adjust_speed(move |step| {
        let Some(window) = weak.upgrade() else { return };
        let next = SPEEDS
            .iter()
            .position(|rate| (*rate - chosen.speed.get()).abs() < f64::EPSILON)
            .map_or(SPEEDS.len() / 2, |at| {
                at.saturating_add_signed(step as isize)
                    .min(SPEEDS.len() - 1)
            });
        let speed = SPEEDS[next];

        chosen.speed.set(speed);
        keep_now();
        if let Err(error) = player.set_speed(speed) {
            tracing::warn!(%error, speed, "speed change failed");
            return;
        }
        show_hint(&window, format_speed(speed));
    });

    window.on_step_frame(move |forward| {
        if let Err(error) = player.step_frame(forward) {
            tracing::warn!(%error, forward, "frame step failed");
        }
    });

    // Fast while held, then back to whatever the menu says. The menu's speed
    // is not touched, so the hold is never kept or shown as the choice.
    let chosen = Rc::clone(settings);
    let weak = window.as_weak();
    window.on_hold_speed(move |holding| {
        let Some(window) = weak.upgrade() else { return };
        let speed = if holding {
            HOLD_SPEED.max(chosen.speed.get())
        } else {
            chosen.speed.get()
        };
        if let Err(error) = player.set_speed(speed) {
            tracing::warn!(%error, speed, "hold speed change failed");
            return;
        }
        if holding {
            show_hint(&window, format!("{} ▶▶", format_speed(speed)));
        }
    });

    let weak = window.as_weak();
    window.on_screenshot(move |with_subtitles| {
        let Some(window) = weak.upgrade() else { return };
        let ru = window.get_lang() == "ru";
        let Some(dir) = screenshots_dir() else {
            return;
        };
        let at = player.position().unwrap_or_default().as_secs();
        let stem = anirust_download::safe_stem(&format!(
            "{} - {:02} - {:02}m{:02}s{}",
            window.get_release_title(),
            window.get_current_episode(),
            at / 60,
            at % 60,
            if with_subtitles { "" } else { " (clean)" },
        ));
        let path = dir.join(format!("{stem}.png"));
        let saved = std::fs::create_dir_all(&dir)
            .map_err(anyhow::Error::from)
            .and_then(|()| Ok(player.screenshot(&path, with_subtitles)?));
        match saved {
            Ok(()) => {
                tracing::info!(path = %path.display(), with_subtitles, "screenshot saved");
                show_hint(
                    &window,
                    format!(
                        "📷 {}",
                        if ru {
                            "Снимок сохранён"
                        } else {
                            "Screenshot saved"
                        }
                    ),
                );
            }
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "screenshot failed");
                show_hint(
                    &window,
                    if ru {
                        "Снимок не сохранился"
                    } else {
                        "The screenshot failed"
                    }
                    .to_owned(),
                );
            }
        }
    });

    window.on_open_screenshots_folder(|| {
        let Some(dir) = screenshots_dir() else { return };
        match std::fs::create_dir_all(&dir) {
            Ok(()) => desktop::open_folder(&dir),
            Err(error) => tracing::warn!(%error, dir = %dir.display(), "no screenshots folder"),
        }
    });

    window.on_open_fonts_folder(|| {
        let Some(dir) = subtitle_fonts_dir() else {
            return;
        };
        match std::fs::create_dir_all(&dir) {
            Ok(()) => desktop::open_folder(&dir),
            Err(error) => tracing::warn!(%error, dir = %dir.display(), "no subtitle fonts folder"),
        }
    });

    let chosen = Rc::clone(settings);
    let weak = window.as_weak();
    window.on_step_audio_delay(move |direction| {
        let Some(window) = weak.upgrade() else { return };
        let delay = step_delay(chosen.audio_delay.get(), direction);
        if let Err(error) = player.set_audio_delay(delay) {
            tracing::warn!(%error, delay, "audio delay change failed");
            return;
        }
        chosen.audio_delay.set(delay);
        let ru = window.get_lang() == "ru";
        let what = if ru { "Звук" } else { "Audio" };
        show_hint(&window, format!("{what} {}", format_delay(delay, ru)));
    });

    let chosen = Rc::clone(settings);
    let weak = window.as_weak();
    window.on_step_subtitle_delay(move |direction| {
        let Some(window) = weak.upgrade() else { return };
        let delay = step_delay(chosen.subtitle_delay.get(), direction);
        if let Err(error) = player.set_subtitle_delay(delay) {
            tracing::warn!(%error, delay, "subtitle delay change failed");
            return;
        }
        chosen.subtitle_delay.set(delay);
        let ru = window.get_lang() == "ru";
        let what = if ru { "Субтитры" } else { "Subtitles" };
        show_hint(&window, format!("{what} {}", format_delay(delay, ru)));
    });

    let chosen = Rc::clone(settings);
    let keep_now = Rc::clone(&keep);
    window.on_toggle_normalize(move || {
        let on = !chosen.normalize.get();
        if let Err(error) = player.set_loudness_normalization(on) {
            tracing::warn!(%error, on, "loudness normalisation change failed");
            return;
        }
        chosen.normalize.set(on);
        keep_now();
    });

    let chosen = Rc::clone(settings);
    let keep_now = Rc::clone(&keep);
    window.on_set_subtitle_scale(move |index| {
        let index = (index.max(0) as usize).min(SUBTITLE_SCALES.len() - 1);
        if let Err(error) = player.set_subtitle_scale(SUBTITLE_SCALES[index]) {
            tracing::warn!(%error, index, "subtitle size change failed");
            return;
        }
        chosen.subtitle_scale.set(index);
        keep_now();
    });

    let chosen = Rc::clone(settings);
    let keep_now = Rc::clone(&keep);
    window.on_set_frame_rate(move |index| {
        let index = (index.max(0) as usize).min(TargetRate::ALL.len());
        chosen.frame_rate.set(index);
        keep_now();
        if let Err(error) = chosen.apply_generation(player) {
            tracing::warn!(%error, index, "frame generation change failed");
        }
    });

    let chosen = Rc::clone(settings);
    let keep_now = Rc::clone(&keep);
    window.on_toggle_adaptive(move || {
        chosen.adaptive.set(!chosen.adaptive.get());
        chosen.watch.borrow_mut().reset();
        keep_now();
    });

    let chosen = Rc::clone(settings);
    let keep_now = Rc::clone(&keep);
    window.on_set_neural_engine(move |index| {
        chosen
            .engine
            .set((index.max(0) as usize).min(Backend::ALL.len()));
        keep_now();
        if let Err(error) = chosen.apply_generation(player) {
            tracing::warn!(%error, index, "engine change failed");
        }
    });

    let chosen = Rc::clone(settings);
    let keep_now = Rc::clone(&keep);
    window.on_toggle_neural_upscale(move || {
        chosen.neural_upscale.set(!chosen.neural_upscale.get());
        keep_now();
        if let Err(error) = chosen.apply_generation(player) {
            tracing::warn!(%error, "neural upscaling change failed");
        }
    });

    // A vendor's runtime, fetched from the vendor when asked. The line on
    // screen follows the download; once it is in, the engine starts using it.
    let chosen = Rc::clone(settings);
    let http = app.http.clone();
    let weak = window.as_weak();
    window.on_fetch_runtime(move || {
        let Some(backend) = chosen.backend() else {
            return;
        };
        if !chosen.runtime_fetch.borrow().is_empty() {
            return;
        }
        let Some(window) = weak.upgrade() else { return };
        let ru = window.get_lang() == "ru";
        *chosen.runtime_fetch.borrow_mut() = if ru {
            "Скачиваю…"
        } else {
            "Fetching…"
        }
        .to_owned();

        let (send, mut receive) = tokio::sync::mpsc::unbounded_channel::<runtimes::Progress>();
        let reporter = Rc::clone(&chosen);
        let _ = slint::spawn_local(async move {
            while let Some(progress) = receive.recv().await {
                *reporter.runtime_fetch.borrow_mut() = describe_fetch(progress, ru);
            }
        });

        let chosen = Rc::clone(&chosen);
        let weak = weak.clone();
        let name = backend_name(backend);
        tasks::spawn(
            runtimes::fetch(http.clone(), backend, move |progress| {
                let _ = send.send(progress);
            }),
            move |result| {
                chosen.runtime_fetch.borrow_mut().clear();
                let Some(window) = weak.upgrade() else { return };
                match result {
                    Ok(_) => {
                        chosen.refresh_mlrt();
                        if let Err(error) = chosen.apply_generation(player) {
                            tracing::warn!(%error, ?backend, "the engine did not start");
                        }
                        show_hint(
                            &window,
                            if ru {
                                format!("{name} готов")
                            } else {
                                format!("{name} is ready")
                            },
                        );
                    }
                    Err(error) => {
                        tracing::warn!(%error, ?backend, "the runtime was not fetched");
                        show_hint(
                            &window,
                            if ru {
                                format!("{name} не скачался: {error}")
                            } else {
                                format!("{name} was not fetched: {error}")
                            },
                        );
                    }
                }
            },
        );
    });

    let chosen = Rc::clone(settings);
    let keep_now = Rc::clone(&keep);
    window.on_set_rife_model(move |index| {
        let index = (index.max(0) as usize).min(RifeModel::ALL.len() - 1);
        chosen.rife_model.set(index);
        keep_now();
        if chosen.frame_rate.get() > 0
            && let Err(error) = chosen.apply_generation(player)
        {
            tracing::warn!(%error, index, "RIFE model change failed");
        }
    });

    let chosen = Rc::clone(settings);
    let keep_now = Rc::clone(&keep);
    window.on_set_picture(move |index| {
        let index = (index.max(0) as usize).min(PictureAdjust::PRESETS.len() - 1);
        if let Err(error) = player.set_picture(PictureAdjust::PRESETS[index]) {
            tracing::warn!(%error, index, "colour preset change failed");
            return;
        }
        chosen.picture.set(index);
        keep_now();
    });

    let chosen = Rc::clone(settings);
    let keep_now = Rc::clone(&keep);
    window.on_toggle_ass_override(move || {
        let on = !chosen.ass_override.get();
        if let Err(error) = player.set_ass_override(on) {
            tracing::warn!(%error, on, "ASS override change failed");
            return;
        }
        chosen.ass_override.set(on);
        keep_now();
    });

    // Files someone picks: another dub, or subtitles for the episode. Each
    // becomes a track of this episode and is shown at once.
    let weak = window.as_weak();
    window.on_load_audio_file(move || {
        let weak = weak.clone();
        let picked = slint::spawn_local(async move {
            let Some(file) = rfd::AsyncFileDialog::new()
                .add_filter(
                    "Audio",
                    &[
                        "mka", "aac", "ac3", "eac3", "flac", "m4a", "mp3", "ogg", "opus", "wav",
                    ],
                )
                .pick_file()
                .await
            else {
                return;
            };
            let Some(window) = weak.upgrade() else { return };
            if let Err(error) = player.load_audio_file(file.path()) {
                tracing::warn!(%error, "the audio file was not loaded");
                show_hint(&window, "✕".to_owned());
            }
        });
        if let Err(error) = picked {
            tracing::warn!(%error, "the file picker did not open");
        }
    });

    let weak = window.as_weak();
    window.on_load_subtitle_file(move || {
        let weak = weak.clone();
        let picked = slint::spawn_local(async move {
            let Some(file) = rfd::AsyncFileDialog::new()
                .add_filter("Subtitles", &["ass", "ssa", "srt", "vtt", "sub", "sup"])
                .pick_file()
                .await
            else {
                return;
            };
            let Some(window) = weak.upgrade() else { return };
            if let Err(error) = player.load_subtitle_file(file.path()) {
                tracing::warn!(%error, "the subtitle file was not loaded");
                show_hint(&window, "✕".to_owned());
            }
        });
        if let Err(error) = picked {
            tracing::warn!(%error, "the file picker did not open");
        }
    });

    let chosen = Rc::clone(settings);
    let weak = window.as_weak();
    window.on_toggle_repeat(move || {
        let Some(window) = weak.upgrade() else { return };
        let on = !chosen.repeat.get();
        if let Err(error) = player.set_loop(on) {
            tracing::warn!(%error, on, "repeat change failed");
            return;
        }
        chosen.repeat.set(on);
        window.set_repeat(on);
        let ru = window.get_lang() == "ru";
        show_hint(
            &window,
            match (on, ru) {
                (true, true) => "Повтор серии включён",
                (true, false) => "Repeating the episode",
                (false, true) => "Повтор серии выключен",
                (false, false) => "Not repeating",
            }
            .to_owned(),
        );
    });

    // Off and back on, keeping whichever track was last chosen rather than
    // always landing on the first one.
    let chosen = Rc::clone(settings);
    let weak = window.as_weak();
    window.on_toggle_captions(move || {
        let Some(window) = weak.upgrade() else { return };
        let tracks = chosen.subtitles.borrow();
        if tracks.len() < 2 {
            show_hint(&window, "CC —".to_owned());
            return;
        }

        let showing = player.current_track(TrackKind::Subtitle).is_some();
        let wanted = if showing {
            None
        } else {
            chosen
                .last_subtitle
                .get()
                .or_else(|| tracks.iter().flatten().copied().next())
        };
        if showing {
            chosen
                .last_subtitle
                .set(player.current_track(TrackKind::Subtitle));
        }

        if let Err(error) = player.set_subtitle_track(wanted) {
            tracing::warn!(%error, "caption toggle failed");
            return;
        }
        show_hint(
            &window,
            if wanted.is_some() { "CC" } else { "CC off" }.to_owned(),
        );
    });

    let weak = window.as_weak();
    window.on_toggle_fullscreen(move || {
        let Some(window) = weak.upgrade() else { return };
        let next = !window.get_fullscreen();
        window.set_fullscreen(next);
        window.window().set_fullscreen(next);
        // Filling the screen with a window that still boxes the picture into
        // one corner of itself is not what the button promises.
        window.set_theatre(next);
    });

    // Closing the player stops decoding: a stream running behind a screen
    // nobody is looking at costs bandwidth for nothing.
    let current = Rc::clone(playing);
    let weak = window.as_weak();
    window.on_close_player(move || {
        if let Err(error) = player.stop() {
            tracing::warn!(%error, "stopping playback failed");
        }
        current.borrow_mut().take();

        let Some(window) = weak.upgrade() else { return };
        window.set_playing(false);
        window.set_current_episode(0);
        window.set_state("idle".into());
        window.set_has_video(false);
    });

    advance
}

/// The previous and next episode buttons.
///
/// Both do the same thing with a different neighbour, so they are built from
/// one closure rather than written twice.
fn wire_stepping(window: &MainWindow, app: &Rc<App>) -> Rc<dyn Fn()> {
    let stepper = |forward: bool| {
        let weak = window.as_weak();
        let app = Rc::clone(app);

        move || {
            let Some(window) = weak.upgrade() else { return };
            let current = window.get_current_episode();
            let neighbour = {
                let state = app.release.borrow();
                if forward {
                    state.next_after(current)
                } else {
                    state.previous_before(current)
                }
            };

            match neighbour {
                Some(position) => play(&window, &app, position),
                None => tracing::debug!(current, forward, "no episode that way"),
            }
        }
    };

    window.on_previous_episode(stepper(false));

    let advance: Rc<dyn Fn()> = Rc::new(stepper(true));
    window.on_next_episode({
        let advance = Rc::clone(&advance);
        move || advance()
    });
    advance
}

/// Mirrors the player's state into the window, four times a second.
///
/// Fast enough that a clock and a progress bar look alive, slow enough that it
/// costs nothing next to rendering. The video itself is not driven from here —
/// that runs at display rate in the video bridge.
/// Keeps mpv told the refresh rate of the monitor the window is on.
///
/// Checked on every status tick — cheap — because the window can be dragged
/// to another monitor. When the rate changes while frames are generated at
/// the screen's rate, the script is written again so it aims at the new one.
fn follow_display(window: &MainWindow, player: &Player, settings: &Settings) {
    let Some(fps) = desktop::refresh_rate(window.window()) else {
        return;
    };
    let changed = settings
        .display_fps
        .get()
        .is_none_or(|known| (known - fps).abs() > 0.5);
    if !changed {
        return;
    }
    settings.display_fps.set(Some(fps));
    window.set_display_fps(fps.round() as i32);
    if let Err(error) = player.set_display_fps(Some(fps)) {
        tracing::warn!(%error, fps, "the display rate was not passed on");
        return;
    }
    tracing::info!(fps, "display rate");
    let at_display_rate = settings.frame_rate.get() >= adapt::RATE_DISPLAY;
    if at_display_rate && let Err(error) = settings.apply_generation(player) {
        tracing::warn!(%error, "frame generation did not follow the display");
    }
}

/// Lowers the load a step when the machine is dropping frames, if allowed.
///
/// Only plain playback is judged — not a pause, not buffering — and only when
/// something heavy is on. The step is applied at once, kept in the
/// preferences when they are kept, and said on screen.
fn keep_up(
    window: &MainWindow,
    player: &Player,
    settings: &Settings,
    prefs: &Cell<preferences::Preferences>,
    playback: PlaybackState,
) {
    let load = settings.load();
    let mut watch = settings.watch.borrow_mut();
    if !settings.adaptive.get() || playback != PlaybackState::Playing || !load.is_enhanced() {
        watch.reset();
        return;
    }
    let now = std::time::Instant::now();
    if !watch.falling_behind(now, player.dropped_frames()) {
        return;
    }
    let Some(step) = load.next_step() else {
        return;
    };
    let lowered = load.after(step);
    tracing::warn!(
        ?step,
        ?load,
        ?lowered,
        "frames are dropping; lowering the load"
    );
    settings.set_load(lowered);
    let applied = player
        .set_upscale(settings.upscale())
        .and_then(|()| settings.apply_generation(player));
    if let Err(error) = applied {
        tracing::warn!(%error, "the lower load was not applied");
    }
    watch.settle(now);

    let mut all = prefs.get();
    if all.player.remember {
        all.player.upscale_mode = lowered.upscale_mode;
        all.player.upscale_quality = lowered.upscale_quality;
        all.player.frame_rate = lowered.frame_rate;
        all.player.rife_model = lowered.rife_model;
        all.player.neural_upscale = settings.neural_upscale.get();
        prefs.set(all);
        all.save();
    }
    show_hint(window, step.notice(window.get_lang() == "ru").to_owned());
}

fn drive_status(window: &MainWindow, app: &Rc<App>, advance: Rc<dyn Fn()>) {
    let player = app.bridge.player();
    let prefs = Rc::clone(&app.prefs);
    let weak = window.as_weak();
    let settings = Rc::clone(&app.settings);
    let bridge = Rc::clone(&app.bridge);
    let state = Rc::clone(&app.release);
    let client = Rc::clone(&app.client);
    // Edge-triggered: mpv stays in `Ended` until something else is loaded, and
    // an episode should only be followed by the next one once.
    let was_ended = Cell::new(false);
    // Positions are written every few seconds rather than four times a second,
    // which would rewrite the file for nothing.
    let ticks = Cell::new(0u32);
    // Track menus are rebuilt only when the file's stream count changes:
    // building one reads seven properties per track, and the answer is the same
    // for the whole episode.
    let known_tracks = Cell::new(usize::MAX);

    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(250),
        move || {
            let Some(window) = weak.upgrade() else { return };

            // Before anything else: mpv should know the screen's rate by the
            // time an episode starts, not a tick after.
            follow_display(&window, player, &settings);

            // With nothing loaded there is nothing to mirror, and asking mpv
            // for properties it has no file for is pure waste.
            if !window.get_playing() {
                return;
            }

            let position = player.position().unwrap_or_default();
            let duration = player.duration();

            window.set_position_text(format_time(position).into());
            window.set_duration_text(
                duration
                    .map_or_else(|| "--:--".to_owned(), format_time)
                    .into(),
            );

            // While the viewer is dragging, the bar shows where their pointer
            // is. Writing the player's clock over it here would make the thumb
            // fight the hand.
            if !window.get_scrubbing() {
                window.set_progress(fraction_of(position, duration));
            }
            window.set_buffered(fraction_of(
                player.buffered_until().unwrap_or(position),
                duration,
            ));

            // The decoder mpv actually engaged, which is not always the one
            // asked for — worth showing rather than hiding.
            window.set_decoder_label(
                player
                    .active_hwdec()
                    .map_or_else(|| "SW".to_owned(), |name| name.to_uppercase())
                    .into(),
            );

            // Say what is being produced, not just what arrived: with
            // upscaling on, the picture leaving the renderer is larger than
            // the source, and reporting the source would understate it.
            let generated = generated_fps(player, &settings);
            if generated.is_none() {
                explain_engine_build(&window, &settings);
            }
            window.set_quality_label(
                quality_label(
                    player.video_size(),
                    player.filtered_size(),
                    bridge.rendered_size(),
                    generated,
                    window.get_lang() == "ru",
                )
                .into(),
            );

            let playback = player.state();
            // Idle while an episode is wanted is an episode still being
            // found and opened, which the viewer should see as loading
            // rather than as nothing to play.
            let shown = if playback == PlaybackState::Idle && window.get_playing() {
                "loading"
            } else {
                state_name(playback)
            };
            window.set_state(shown.into());
            window.set_paused(playback == PlaybackState::Paused);
            keep_up(&window, player, &settings, &prefs, playback);
            // The render loop reads this instead of querying mpv on every
            // frame; a quarter-second of staleness costs nothing here.
            bridge.set_advancing(playback.is_active());

            remember(&state, &client, &window, position, duration, &ticks);

            // Following on to the next episode is what a viewer who watched
            // one to the end was going to ask for anyway.
            if playback == PlaybackState::Ended {
                if !was_ended.replace(true) {
                    tracing::info!("episode ended; going on to the next");
                    advance();
                }
            } else {
                was_ended.set(false);
            }

            show_tracks(&window, player, &settings, &known_tracks);

            window.set_speed_label(format_speed(settings.speed.get()).into());
            window.set_upscale(settings.upscale_mode.get() as i32);
            let ru = window.get_lang() == "ru";
            window.set_audio_delay_label(format_delay(settings.audio_delay.get(), ru).into());
            window.set_subtitle_delay_label(format_delay(settings.subtitle_delay.get(), ru).into());
            window.set_normalize(settings.normalize.get());
            window.set_subtitle_scale(settings.subtitle_scale.get() as i32);
            window.set_ass_override(settings.ass_override.get());
            window.set_picture(settings.picture.get() as i32);
            window.set_frame_rate(settings.frame_rate.get() as i32);
            window.set_rife_model(settings.rife_model.get() as i32);
            window.set_frames_available(settings.rife.is_some() || settings.on_tensorrt());
            window.set_neural_engine(settings.engine.get() as i32);
            let choices: Vec<i32> = std::iter::once(0)
                .chain(
                    (0..Backend::ALL.len())
                        .filter(|&at| settings.shipped[at])
                        .map(|at| at as i32 + 1),
                )
                .collect();
            if slint::Model::row_count(&window.get_engine_choices()) != choices.len() {
                window.set_engine_choices(slint::ModelRc::new(slint::VecModel::from(choices)));
            }
            window.set_engine_ready(settings.on_mlrt());
            window.set_engine_fetchable(
                settings
                    .backend()
                    .is_some_and(|backend| backend.download_url().is_some()),
            );
            window.set_runtime_fetch(settings.runtime_fetch.borrow().as_str().into());
            window.set_recommended_engine(recommended_engine() as i32);
            window.set_neural_upscale(settings.neural_upscale.get());
            window.set_adaptive(settings.adaptive.get());
            window.set_upscale_quality(settings.upscale_quality.get() as i32);
            window.set_interpolation(settings.interpolation.get());
            window.set_quality(settings.quality.get() as i32);
            window.set_decoder(settings.decoder.get() as i32);
        },
    );
    // The window needs this for its whole life; dropping the timer would stop
    // the clock.
    std::mem::forget(timer);
}

/// Keeps the subtitle and audio menus in step with the file.
///
/// The lists themselves change only when a new file loads; which entry is
/// selected can change at any time, including from mpv's own defaults, so that
/// is read every tick.
fn show_tracks(
    window: &MainWindow,
    player: &'static Player,
    settings: &Rc<Settings>,
    known: &Cell<usize>,
) {
    let count = player.track_count();
    if settings.tracks_stale.replace(false) | (count != known.replace(count)) {
        let subtitles = menu_for(player, TrackKind::Subtitle, true);
        let audio = menu_for(player, TrackKind::Audio, false);

        window.set_subtitle_tracks(labels(&subtitles, window));
        window.set_audio_tracks(labels(&audio, window));

        *settings.subtitles.borrow_mut() = subtitles.iter().map(|(id, _)| *id).collect();
        *settings.audio.borrow_mut() = audio.iter().map(|(id, _)| *id).collect();
    }

    window.set_subtitle_track(index_of(
        &settings.subtitles.borrow(),
        player.current_track(TrackKind::Subtitle),
    ));
    window.set_audio_track(index_of(
        &settings.audio.borrow(),
        player.current_track(TrackKind::Audio),
    ));
}

/// The entries of one menu: an mpv id, and what to call it.
///
/// Subtitles get an "off" entry first, because turning them off is the most
/// common thing anyone does to them.
fn menu_for(
    player: &'static Player,
    kind: TrackKind,
    offer_off: bool,
) -> Vec<(Option<i64>, String)> {
    let mut entries: Vec<(Option<i64>, String)> = Vec::new();
    if offer_off {
        entries.push((None, String::new()));
    }
    entries.extend(
        player
            .tracks_of(kind)
            .into_iter()
            .map(|track: Track| (Some(track.id), track.label())),
    );
    entries
}

/// Menu labels, with the empty one standing for "off" in whichever language
/// the window is in.
fn labels(
    entries: &[(Option<i64>, String)],
    window: &MainWindow,
) -> slint::ModelRc<slint::SharedString> {
    let off: slint::SharedString = if window.get_lang() == "ru" {
        "Выкл".into()
    } else {
        "Off".into()
    };

    slint::ModelRc::new(slint::VecModel::from(
        entries
            .iter()
            .map(|(id, label)| match id {
                Some(_) => slint::SharedString::from(label.as_str()),
                None => off.clone(),
            })
            .collect::<Vec<_>>(),
    ))
}

/// Where a selected id sits in a menu, or 0 when it is not in it.
fn index_of(entries: &[Option<i64>], selected: Option<i64>) -> i32 {
    entries
        .iter()
        .position(|entry| *entry == selected)
        .unwrap_or(0) as i32
}

/// Writes the current position to the store, every few seconds.
///
/// Called from the status poll because that is already asking mpv where it is;
/// doing it again on its own timer would be the same question twice.
fn remember(
    state: &Rc<RefCell<ReleaseState>>,
    client: &Rc<Client>,
    window: &MainWindow,
    position: Duration,
    duration: Option<Duration>,
    ticks: &Cell<u32>,
) {
    const EVERY: u32 = 8;

    let episode = window.get_current_episode();
    if episode == 0 || position == Duration::ZERO {
        return;
    }

    let tick = ticks.get().wrapping_add(1);
    ticks.set(tick);
    if !tick.is_multiple_of(EVERY) {
        return;
    }

    let (release_id, source_id, finished) = {
        let state = state.borrow();
        let mut store = state.progress.borrow_mut();
        let finished = store.record(state.release_id, episode, position, duration);
        store.flush();
        (
            state.release_id,
            state.selected_source().map(|source| source.id),
            finished,
        )
    };

    if !finished {
        return;
    }
    tracing::info!(episode, "episode finished");

    // The account is told once, at the moment it becomes true. The API has no
    // endpoint for a position, so this is the whole of what can be synced.
    let (Some(source_id), true) = (source_id, client.is_authenticated()) else {
        return;
    };

    let api = (**client).clone();
    tasks::spawn(
        async move { api.mark_watched(release_id, source_id, episode).await },
        move |result| match result {
            Ok(()) => tracing::info!(episode, "marked watched on the account"),
            Err(error) => tracing::warn!(%error, episode, "could not mark the episode watched"),
        },
    );
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

fn fraction_of(position: Duration, duration: Option<Duration>) -> f32 {
    match duration {
        Some(total) if total.as_secs_f32() > 0.0 => {
            (position.as_secs_f32() / total.as_secs_f32()).clamp(0.0, 1.0)
        }
        _ => 0.0,
    }
}

/// Describes the picture: the source resolution, and what it is being
/// rendered at when that is larger.
///
/// A frame rate only when frames are being generated and the rate leaving
/// the filters shows it. Interpolation resamples timing without making
/// frames, so advertising a higher fps for it would be a claim the player
/// cannot back.
fn quality_label(
    source: Option<(u32, u32)>,
    filtered: Option<(u32, u32)>,
    rendered: Option<(u32, u32)>,
    generated_fps: Option<f64>,
    ru: bool,
) -> String {
    let Some((_, source_h)) = source else {
        return "—".to_owned();
    };

    // A network that enlarged the picture is named, so the step reads as
    // the network's and not the shaders'.
    let mut label = format!("{source_h}p");
    let mut top = source_h;
    if let Some((_, filtered_h)) = filtered
        && filtered_h > source_h
    {
        let ai = if ru { "ИИ" } else { "AI" };
        label = format!("{label} → {filtered_h}p {ai}");
        top = filtered_h;
    }
    if let Some((_, rendered_h)) = rendered
        && rendered_h > top
    {
        label = format!("{label} → {rendered_h}p");
    }
    match generated_fps {
        Some(fps) => format!("{label} · {fps:.0} fps"),
        None => label,
    }
}

/// The rate frame generation is producing, when it is on and working.
///
/// mpv drops a failed filter and plays on, so "on" is not the same as
/// "working": the rate leaving the filters says which. A few seconds after
/// it was switched on, a rate no higher than the source's is a failure, and
/// the note the script left is logged once.
fn generated_fps(player: &Player, settings: &Settings) -> Option<f64> {
    settings.frame_rate.get().checked_sub(1)?;
    let source = player.source_fps()?;
    let out = player.output_fps()?;
    let working = out > source * 1.3;
    if let Some(since) = settings.generation_since.get() {
        // mpv's estimate trails a change: right after one it still speaks
        // of the filter before.
        if since.elapsed() < std::time::Duration::from_secs(3) {
            return working.then_some(out);
        }
        // TensorRT compiles a network for the card the first time it meets a
        // picture size, which holds the picture for ten seconds or so.
        let grace = if settings.on_tensorrt() { 30 } else { 6 };
        if working {
            tracing::info!(source, out, "frame generation is working");
            settings.generation_since.set(None);
        } else if since.elapsed() > std::time::Duration::from_secs(grace) && player.is_playing() {
            let note = anirust_player::frames::last_error().unwrap_or_else(|| {
                "no note from the script: is VapourSynth installed, and does this libmpv have \
                 its filter?"
                    .to_owned()
            });
            tracing::warn!(source, out, %note, "frame generation did not start");
            settings.generation_since.set(None);
        }
    }
    working.then_some(out)
}

/// Says, once, that TensorRT is compiling a network for this card, when the
/// picture has stood still for a few seconds after the filters changed —
/// otherwise a frozen frame goes unexplained.
fn explain_engine_build(window: &MainWindow, settings: &Settings) {
    let Some(since) = settings.generation_since.get() else {
        return;
    };
    if !settings.on_tensorrt()
        || settings.engine_build_said.get()
        || since.elapsed() < std::time::Duration::from_secs(3)
    {
        return;
    }
    settings.engine_build_said.set(true);
    let ru = window.get_lang() == "ru";
    show_hint(
        window,
        if ru {
            "Собираю движок TensorRT под эту видеокарту: один раз, около 15 секунд"
        } else {
            "Building a TensorRT engine for this card: once, about 15 seconds"
        }
        .to_owned(),
    );
}

/// What the quality menu calls a rendition.
fn quality_name(height: u32) -> String {
    if height == anirust_extract::UNKNOWN_HEIGHT {
        "auto".to_owned()
    } else {
        format!("{height}p")
    }
}

/// Matches the names the Slint side compares against.
fn state_name(state: PlaybackState) -> &'static str {
    match state {
        PlaybackState::Idle => "idle",
        PlaybackState::Loading => "loading",
        PlaybackState::Buffering => "buffering",
        PlaybackState::Playing => "playing",
        PlaybackState::Paused => "paused",
        PlaybackState::Ended => "ended",
    }
}

/// `1x`, `1.5x` — no trailing zero on whole rates.
fn format_speed(speed: f64) -> String {
    if (speed.fract()).abs() < f64::EPSILON {
        format!("{speed:.0}x")
    } else {
        format!("{speed}x")
    }
}

/// Russian when the locale asks for it, matching the probe's behaviour.
fn is_russian_locale() -> bool {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|var| std::env::var(var).ok().filter(|v| !v.is_empty()))
        .is_some_and(|locale| locale.to_ascii_lowercase().starts_with("ru"))
}

pub fn format_time(value: Duration) -> String {
    let total = value.as_secs();
    let (hours, minutes, seconds) = (total / 3600, (total % 3600) / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// Where the log goes. A release build on Windows has no console, so its log
/// is a file in the data folder, `anirust.log`, started afresh each run —
/// something a tester can send. Everywhere else it is standard error.
fn log_writer() -> tracing_subscriber::fmt::writer::BoxMakeWriter {
    use tracing_subscriber::fmt::writer::BoxMakeWriter;

    if cfg!(all(windows, not(debug_assertions))) {
        let file = dirs::data_local_dir()
            .map(|dir| dir.join("anirust"))
            .and_then(|dir| {
                std::fs::create_dir_all(&dir).ok()?;
                std::fs::File::create(dir.join("anirust.log")).ok()
            });
        if let Some(file) = file {
            return BoxMakeWriter::new(std::sync::Mutex::new(file));
        }
    }
    BoxMakeWriter::new(std::io::stderr)
}

/// The API client. `ANIRUST_API_URL` points it elsewhere — another of the
/// service's hosts — for checking whether a behaviour is the host's.
fn api_client() -> anirust_api::Result<Client> {
    match std::env::var("ANIRUST_API_URL") {
        Ok(url) if !url.trim().is_empty() => Client::builder().base_urls([url.trim()]).build(),
        _ => Client::new(),
    }
}

/// The client pictures come through. Bounded in time: a picture from a host
/// the network blocks — YouTube's thumbnails, from some countries — would
/// otherwise hang its request for good.
fn reqwest_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// A release to open straight away, if one was named.
fn release_id_from_args() -> Result<Option<i64>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => Ok(None),
        [id] => id
            .parse()
            .map(Some)
            .context("the release id must be a number"),
        _ => bail!("usage: anirust [release-id]"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_delay_reads_with_its_sign_and_the_locale_decimal() {
        assert_eq!(format_delay(0.0, true), "0");
        assert_eq!(format_delay(0.3, false), "+0.3");
        assert_eq!(format_delay(-1.2, true), "−1,2");
        assert_eq!(format_delay(0.04, false), "0");
    }

    #[test]
    fn ten_steps_up_and_ten_down_come_back_to_none() {
        let mut delay = 0.0;
        for _ in 0..10 {
            delay = step_delay(delay, 1);
        }
        assert!((delay - 1.0).abs() < 1e-9, "{delay}");
        for _ in 0..10 {
            delay = step_delay(delay, -1);
        }
        assert_eq!(format_delay(delay, false), "0");
        assert_eq!(step_delay(2.7, 0), 0.0);
    }

    #[test]
    fn upscaling_is_reported_as_a_transformation() {
        assert_eq!(
            quality_label(Some((1280, 720)), None, Some((2560, 1440)), None, true),
            "720p → 1440p"
        );
    }

    #[test]
    fn rendering_at_the_source_size_reports_one_number() {
        assert_eq!(
            quality_label(Some((1280, 720)), None, Some((1280, 720)), None, true),
            "720p"
        );
    }

    #[test]
    fn a_smaller_render_never_reads_as_an_upgrade() {
        assert_eq!(
            quality_label(Some((1920, 1080)), None, Some((1280, 720)), None, true),
            "1080p"
        );
    }

    #[test]
    fn generated_frames_add_their_rate() {
        assert_eq!(
            quality_label(
                Some((1280, 720)),
                None,
                Some((2560, 1440)),
                Some(59.94),
                true
            ),
            "720p → 1440p · 60 fps"
        );
    }

    #[test]
    fn a_network_upscale_is_named_on_its_step() {
        assert_eq!(
            quality_label(
                Some((1280, 720)),
                Some((2560, 1440)),
                Some((3840, 2160)),
                None,
                true
            ),
            "720p → 1440p ИИ → 2160p"
        );
        // Frame generation brought the picture down: not an upscale.
        assert_eq!(
            quality_label(
                Some((1920, 1080)),
                Some((1280, 720)),
                Some((3840, 2160)),
                Some(60.0),
                true
            ),
            "1080p → 2160p · 60 fps"
        );
    }

    #[test]
    fn nothing_loaded_shows_a_placeholder() {
        assert_eq!(quality_label(None, None, None, None, true), "—");
    }

    #[test]
    fn an_unlabelled_rendition_is_called_auto() {
        assert_eq!(quality_name(anirust_extract::UNKNOWN_HEIGHT), "auto");
        assert_eq!(quality_name(1080), "1080p");
    }

    #[test]
    fn whole_speeds_lose_the_decimal() {
        assert_eq!(format_speed(1.0), "1x");
        assert_eq!(format_speed(2.0), "2x");
        assert_eq!(format_speed(1.5), "1.5x");
    }

    #[test]
    fn times_gain_an_hour_field_only_when_needed() {
        assert_eq!(format_time(Duration::from_secs(62)), "1:02");
        assert_eq!(format_time(Duration::from_secs(3_723)), "1:02:03");
    }

    #[test]
    fn progress_is_bounded_and_safe_without_a_duration() {
        assert_eq!(
            fraction_of(Duration::from_secs(30), Some(Duration::from_secs(60))),
            0.5
        );
        assert_eq!(
            fraction_of(Duration::from_secs(90), Some(Duration::from_secs(60))),
            1.0
        );
        assert_eq!(fraction_of(Duration::from_secs(30), None), 0.0);
    }
}
