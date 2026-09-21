// SPDX-License-Identifier: GPL-3.0-or-later

//! AniRust.
//!
//! One window, two screens: the release screen chooses what to watch, the
//! player plays it. The player is a screen rather than a separate window, so
//! leaving an episode lands back on the release it came from instead of on
//! nothing.
//!
//! Nothing here blocks the event loop. Every lookup and every extractor run
//! goes through [`tasks`] and comes back on the UI thread.

mod downloads;
mod home;
mod preferences;
mod progress;
mod release;
mod session;
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
    MediaSource, PlaybackState, Player, PlayerConfig, Track, TrackKind, UpscalePreset,
};

use crate::home::HomeState;
use crate::release::ReleaseState;
use crate::session::Session;
use crate::video::VideoBridge;

slint::include_modules!();

/// Where the profile sits on the rail. Named because two places have to agree
/// about it: the rail's own order, and what arriving there has to fetch.
const PROFILE_DESTINATION: i32 = 4;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive("anirust=info".parse()?)
                .from_env_lossy(),
        )
        .with_writer(std::io::stderr)
        .init();

    let opening = release_id_from_args()?;
    tasks::init()?;

    let window = MainWindow::new().context("creating the window")?;
    window.set_lang(if is_russian_locale() {
        "ru".into()
    } else {
        "en".into()
    });
    wire_preferences(&window, preferences::Preferences::load());

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
    let app = Rc::new(App {
        client: Rc::new(Client::new().context("creating the API client")?),
        registry: Rc::new(Registry::new(http.clone())),
        http,
        bridge,
        release: Rc::new(RefCell::new(ReleaseState::new(Rc::new(RefCell::new(
            progress::Store::load(),
        ))))),
        home: Rc::new(RefCell::new(HomeState::default())),
        queue: Rc::new(RefCell::new(downloads::Queue::default())),
        account: Rc::new(RefCell::new(Session::default())),
        playing: Rc::new(RefCell::new(None)),
        settings: Rc::new(Settings::default()),
    });

    wire_home(&window, &app);
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
    queue: Rc<RefCell<downloads::Queue>>,
    account: Rc<RefCell<Session>>,
    /// The stream currently loaded, so the quality menu and the skip button
    /// act on what is playing rather than on what was resolved first.
    playing: Playing,
    settings: Rc<Settings>,
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
fn wire_preferences(window: &MainWindow, preferences: preferences::Preferences) {
    let held = Rc::new(Cell::new(preferences));
    show_appearance(window, preferences.appearance);

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

fn wire_home(window: &MainWindow, app: &Rc<App>) {
    let weak = window.as_weak();
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
                session::load_recent(
                    &window,
                    &app.account,
                    Rc::clone(&app.client),
                    app.http.clone(),
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
    window.on_go_back(move || {
        let Some(window) = weak.upgrade() else { return };
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

    let mut source = MediaSource::new(&best.url).headers(
        stream
            .headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str())),
    );
    if let Some(at) = resume {
        source = source.start_at(at);
    }

    if let Err(error) = bridge.play(source) {
        tracing::error!(%error, "could not start playback");
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
    upscale: Cell<usize>,
    interpolation: Cell<bool>,
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

impl Default for Settings {
    fn default() -> Self {
        Self {
            speed: Cell::new(1.0),
            upscale: Cell::new(0),
            interpolation: Cell::new(false),
            quality: Cell::new(0),
            decoder: Cell::new(0),
            subtitles: RefCell::new(Vec::new()),
            audio: RefCell::new(Vec::new()),
            last_subtitle: Cell::new(None),
            tracks_stale: Cell::new(true),
        }
    }
}

const PRESETS: [UpscalePreset; 4] = [
    UpscalePreset::Off,
    UpscalePreset::Fast,
    UpscalePreset::Balanced,
    UpscalePreset::Quality,
];

/// mpv `hwdec` values behind the decoder choice, in the order the sheet lists
/// them: automatic, hardware, software.
const DECODERS: [&str; 3] = [anirust_player::DEFAULT_HWDEC, "nvdec,vaapi", "no"];

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
    window.on_set_speed(move |speed| {
        let speed = f64::from(speed);
        chosen.speed.set(speed);
        if let Err(error) = player.set_speed(speed) {
            tracing::warn!(%error, "speed change failed");
        }
    });

    let chosen = Rc::clone(settings);
    window.on_set_upscale(move |index| {
        let index = (index.max(0) as usize).min(PRESETS.len() - 1);
        chosen.upscale.set(index);
        if let Err(error) = player.set_upscale(PRESETS[index]) {
            tracing::warn!(%error, preset = PRESETS[index].name(), "upscale change failed");
        }
    });

    let chosen = Rc::clone(settings);
    window.on_toggle_interpolation(move || {
        let next = !chosen.interpolation.get();
        chosen.interpolation.set(next);
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
        let mut source = MediaSource::new(&variant.url).headers(
            stream
                .headers
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str())),
        );
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
    window.on_set_decoder(move |index| {
        let index = (index.max(0) as usize).min(DECODERS.len() - 1);
        chosen.decoder.set(index);
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
    window.on_adjust_volume(move |step| {
        let Some(window) = weak.upgrade() else { return };
        let volume = (player.volume() + i64::from(step)).clamp(0, 150);
        if let Err(error) = player.set_volume(volume) {
            tracing::warn!(%error, volume, "volume change failed");
            return;
        }
        // Reaching for the volume means wanting to hear it.
        let _ = player.set_muted(false);
        show_hint(&window, format!("🔊 {volume}%"));
    });

    let chosen = Rc::clone(settings);
    let weak = window.as_weak();
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
fn drive_status(window: &MainWindow, app: &Rc<App>, advance: Rc<dyn Fn()>) {
    let player = app.bridge.player();
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
            window.set_quality_label(
                quality_label(player.video_size(), bridge.rendered_size()).into(),
            );

            let playback = player.state();
            window.set_state(state_name(playback).into());
            window.set_paused(playback == PlaybackState::Paused);
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
            window.set_upscale(settings.upscale.get() as i32);
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
/// Resolution only — never a frame rate. Interpolation resamples timing, it
/// does not synthesise frames, so advertising a higher fps would be a claim
/// the player cannot back.
fn quality_label(source: Option<(u32, u32)>, rendered: Option<(u32, u32)>) -> String {
    let Some((_, source_h)) = source else {
        return "—".to_owned();
    };

    match rendered {
        Some((_, rendered_h)) if rendered_h > source_h => {
            format!("{source_h}p → {rendered_h}p")
        }
        _ => format!("{source_h}p"),
    }
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

fn reqwest_client() -> reqwest::Client {
    reqwest::Client::builder()
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
    fn upscaling_is_reported_as_a_transformation() {
        assert_eq!(
            quality_label(Some((1280, 720)), Some((2560, 1440))),
            "720p → 1440p"
        );
    }

    #[test]
    fn rendering_at_the_source_size_reports_one_number() {
        assert_eq!(quality_label(Some((1280, 720)), Some((1280, 720))), "720p");
    }

    #[test]
    fn a_smaller_render_never_reads_as_an_upgrade() {
        assert_eq!(
            quality_label(Some((1920, 1080)), Some((1280, 720))),
            "1080p"
        );
    }

    #[test]
    fn nothing_loaded_shows_a_placeholder() {
        assert_eq!(quality_label(None, None), "—");
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
