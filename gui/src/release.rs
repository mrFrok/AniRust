// SPDX-License-Identifier: GPL-3.0-or-later

//! The release screen: loading a title and choosing what to play from it.
//!
//! Anixart makes this three dependent lookups — voice-overs, then that
//! voice-over's sources, then that source's episodes — so changing a
//! voice-over invalidates everything below it. The state here exists to keep
//! that cascade honest: whatever is on screen always came from the selection
//! currently shown, never from a slower request that finished late.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, Model, VecModel};

use anirust_api::{
    Client, CommentSort, CommentTarget, Dubber, Episode, EpisodeSort, ProfileList, Release, Source,
};

use crate::progress::Store;

use crate::{EpisodeItem, FranchiseItem, LinkItem, MainWindow, PickerOption, VideoItem, tasks};

/// What the release screen is showing.
///
/// Held in one place so the cascade — voice-over, source, episodes — can be
/// reasoned about as a whole rather than as three properties that happen to
/// agree most of the time.
#[derive(Default)]
pub struct ReleaseState {
    pub release_id: i64,
    pub release: Option<Release>,
    pub dubbers: Vec<Dubber>,
    pub dubber: usize,
    pub sources: Vec<Source>,
    pub source: usize,
    pub episodes: Vec<Episode>,
    /// The franchise's other releases, in the order their chips are shown.
    pub related: Vec<Release>,
    /// Where each of the platform chips leads.
    pub platforms: Vec<String>,
    /// Where each of the video thumbnails leads.
    pub videos: Vec<String>,
    /// The whole franchise, oldest first, as its sheet lists it.
    pub franchise: Vec<Release>,
    /// What this machine remembers, which fills in what an unauthenticated
    /// account cannot.
    pub progress: Rc<RefCell<Store>>,
    /// Bumped on every selection change.
    ///
    /// A request started for an older selection may still be in flight when a
    /// newer one begins; comparing this on arrival is what stops a slow answer
    /// overwriting a fast one the viewer has since moved past.
    generation: u64,
}

impl ReleaseState {
    /// A state that keeps its watch positions in `progress`.
    #[must_use]
    pub fn new(progress: Rc<RefCell<Store>>) -> Self {
        Self {
            progress,
            ..Self::default()
        }
    }

    /// Marks a new selection and returns the token to check results against.
    fn next_generation(&mut self) -> u64 {
        self.generation += 1;
        self.generation
    }

    fn is_current(&self, generation: u64) -> bool {
        self.generation == generation
    }

    pub fn selected_dubber(&self) -> Option<&Dubber> {
        self.dubbers.get(self.dubber)
    }

    pub fn selected_source(&self) -> Option<&Source> {
        self.sources.get(self.source)
    }

    pub fn episode_at(&self, position: i32) -> Option<&Episode> {
        self.episodes.iter().find(|e| e.position == position)
    }

    /// Index of an episode in the list, for scrolling to it.
    #[must_use]
    pub fn index_of(&self, position: i32) -> Option<usize> {
        self.episodes.iter().position(|e| e.position == position)
    }

    /// Whether an episode has been seen, according to either the account or
    /// this machine.
    #[must_use]
    pub fn watched(&self, episode: &Episode) -> bool {
        episode.is_watched
            || self
                .progress
                .borrow()
                .get(self.release_id, episode.position)
                .is_some_and(|entry| entry.finished)
    }

    /// Where to resume an episode from.
    ///
    /// This machine's record wins over the account's: it is the one that was
    /// updated a second ago, while the account's only moves when an episode is
    /// finished.
    #[must_use]
    pub fn resume_of(&self, episode: &Episode) -> Option<Duration> {
        if self.watched(episode) {
            return None;
        }

        self.progress
            .borrow()
            .get(self.release_id, episode.position)
            .and_then(|entry| entry.resume_at())
            .or_else(|| episode.resume_at())
    }

    /// Episode numbers in order, for stepping to the next or previous one.
    pub fn positions(&self) -> Vec<i32> {
        let mut positions: Vec<i32> = self.episodes.iter().map(|e| e.position).collect();
        positions.sort_unstable();
        positions
    }

    /// The episode after `position`, if the source has one.
    pub fn next_after(&self, position: i32) -> Option<i32> {
        self.positions().into_iter().find(|p| *p > position)
    }

    /// The episode before `position`, if the source has one.
    pub fn previous_before(&self, position: i32) -> Option<i32> {
        self.positions().into_iter().rev().find(|p| *p < position)
    }

    /// Where to resume: the furthest episode the account records as started or
    /// watched, or the first unwatched one after it.
    ///
    /// Returns 0 when there is nothing to resume, which is what the interface
    /// uses to hide the button.
    pub fn resume_position(&self) -> i32 {
        let started = self
            .episodes
            .iter()
            .filter(|e| self.watched(e) || self.resume_of(e).is_some())
            .map(|e| e.position)
            .max();

        match started {
            // Part-way through one: offer that one.
            Some(position) if self.partly_watched(position) => position,
            // Finished it: offer the next.
            Some(position) => self.next_after(position).unwrap_or(position),
            None => 0,
        }
    }

    fn partly_watched(&self, position: i32) -> bool {
        self.episode_at(position)
            .is_some_and(|e| self.resume_of(e).is_some())
    }
}

/// Loads a release and everything needed to choose an episode from it.
pub fn load(
    window: &MainWindow,
    state: &Rc<RefCell<ReleaseState>>,
    client: Rc<Client>,
    http: reqwest::Client,
    release_id: i64,
) {
    // Another release's episode stops: its picture would otherwise go on
    // playing over this one's page.
    if window.get_playing() && state.borrow().release_id != release_id {
        window.invoke_close_player();
    }
    window.set_release_loading(true);
    {
        let mut state = state.borrow_mut();
        state.release_id = release_id;
        state.related.clear();
        state.platforms.clear();
    }
    // The last release's links would otherwise sit under this one's title
    // until its own arrive.
    window.set_release_related(slint::ModelRc::new(VecModel::<LinkItem>::default()));
    window.set_release_platforms(slint::ModelRc::new(VecModel::<LinkItem>::default()));
    load_platforms(window, state, &client, release_id);
    load_comment_count(window, state, &client, release_id);
    load_videos(window, state, &client, http.clone(), release_id);

    let weak = window.as_weak();
    let state = Rc::clone(state);
    let fetch_client = Rc::clone(&client);

    // `Client` is cheap to clone and shares its connection pool, so the task
    // gets its own rather than borrowing across threads.
    let api = (*fetch_client).clone();
    tasks::spawn(
        async move {
            // Extended, so the franchise comes inlined in the same answer.
            let release = api.release(release_id, true).await;
            let dubbers = api.dubbers(release_id).await;
            (release, dubbers)
        },
        move |(release, dubbers)| {
            let Some(window) = weak.upgrade() else { return };

            match (release, dubbers) {
                (Ok(release), Ok(dubbers)) => {
                    show_release(&window, &release);
                    show_related(&window, &state, &release);
                    // Decorative, so these load on their own: the screen is
                    // usable before either image arrives.
                    load_poster(&window, http.clone(), release.poster_url());
                    load_backdrop(
                        &window,
                        http,
                        release
                            .screenshot_urls()
                            .first()
                            .cloned()
                            .unwrap_or_default(),
                    );
                    {
                        let mut state = state.borrow_mut();
                        state.release = Some(release);
                        state.dubbers = dubbers;
                        state.dubber = 0;
                    }
                    show_dubbers(&window, &state.borrow());
                    // The voice-over the account pinned, if it pinned one; the
                    // most watched otherwise, which the server lists first.
                    let first = state
                        .borrow()
                        .dubbers
                        .iter()
                        .position(|dubber| dubber.pinned)
                        .unwrap_or(0);
                    select_dubber(&window, &state, client, first);
                }
                (Err(error), _) | (_, Err(error)) => {
                    tracing::error!(%error, release_id, "could not load the release");
                    window.set_release_loading(false);
                }
            }
        },
    );
}

/// Switches voice-over, which invalidates the sources and episodes below it.
pub fn select_dubber(
    window: &MainWindow,
    state: &Rc<RefCell<ReleaseState>>,
    client: Rc<Client>,
    index: usize,
) {
    let (release_id, dubber_id, generation) = {
        let mut state = state.borrow_mut();
        if index >= state.dubbers.len() {
            return;
        }
        state.dubber = index;
        // Everything below this choice is now stale; clear it rather than
        // leaving the previous voice-over's episodes on screen.
        state.sources.clear();
        state.episodes.clear();
        let generation = state.next_generation();
        (state.release_id, state.dubbers[index].id, generation)
    };

    window.set_dubber(index as i32);
    window.set_sources(slint::ModelRc::new(VecModel::from(
        Vec::<PickerOption>::new(),
    )));
    window.set_episodes(slint::ModelRc::new(VecModel::from(
        Vec::<EpisodeItem>::new(),
    )));

    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = (*client).clone();

    tasks::spawn(
        async move { api.sources(release_id, dubber_id).await },
        move |sources| {
            let Some(window) = weak.upgrade() else { return };

            // A slower request for a voice-over the viewer has already moved
            // past must not overwrite what is on screen now.
            if !state.borrow().is_current(generation) {
                return;
            }

            match sources {
                Ok(sources) => {
                    state.borrow_mut().sources = sources;
                    state.borrow_mut().source = 0;
                    show_sources(&window, &state.borrow());
                    select_source(&window, &state, client, 0);
                }
                Err(error) => {
                    tracing::error!(%error, "could not load sources");
                    window.set_release_loading(false);
                }
            }
        },
    );
}

/// Switches source, which invalidates the episode list.
pub fn select_source(
    window: &MainWindow,
    state: &Rc<RefCell<ReleaseState>>,
    client: Rc<Client>,
    index: usize,
) {
    let (release_id, dubber_id, source_id, generation) = {
        let mut state = state.borrow_mut();
        let (Some(dubber), Some(source)) = (
            state.selected_dubber().map(|d| d.id),
            state.sources.get(index).map(|s| s.id),
        ) else {
            return;
        };
        state.source = index;
        state.episodes.clear();
        let generation = state.next_generation();
        (state.release_id, dubber, source, generation)
    };

    window.set_source(index as i32);

    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = (*client).clone();

    tasks::spawn(
        async move {
            api.episodes(release_id, dubber_id, source_id, EpisodeSort::Ascending)
                .await
        },
        move |episodes| {
            let Some(window) = weak.upgrade() else { return };
            if !state.borrow().is_current(generation) {
                return;
            }

            match episodes {
                Ok(episodes) => {
                    state.borrow_mut().episodes = episodes;
                    show_episodes(&window, &state.borrow());
                }
                Err(error) => tracing::error!(%error, "could not load episodes"),
            }
            window.set_release_loading(false);
        },
    );
}

/// Fetches the poster and hands it to the screen when it arrives.
pub fn load_poster(window: &MainWindow, http: reqwest::Client, url: String) {
    tasks::fetch_into(window, http, url, |window, image| {
        window.set_release_poster(image);
        window.set_poster_loaded(true);
    });
}

/// Fetches the still shown behind the play button.
pub fn load_backdrop(window: &MainWindow, http: reqwest::Client, url: String) {
    tasks::fetch_into(window, http, url, |window, image| {
        window.set_release_backdrop(image);
        window.set_backdrop_loaded(true);
    });
}

// ---------------------------------------------------------------------------
// Pushing state onto the screen
// ---------------------------------------------------------------------------

fn show_release(window: &MainWindow, release: &Release) {
    window.set_release_title(release.title().into());
    window.set_release_original_title(release.title_original.as_str().into());
    window.set_release_year(release.year.as_str().into());
    window.set_release_genres(release.genres.as_str().into());
    window.set_release_studio(release.studio.as_str().into());
    window.set_release_status(release.status_name().into());
    window.set_release_description(release.description.as_str().into());
    window.set_release_score(
        release
            .shown_score()
            .map(|score| format!("{score:.1}"))
            .unwrap_or_default()
            .into(),
    );
    window.set_release_rateable(!release.is_unreleased());
    window.set_release_watchable(!release.is_unreleased());
    window.set_release_note(anirust_api::plain_text(&release.note).into());
    window.set_release_episodes_label(
        format!("{}/{}", release.episodes_released, release.episodes_total).into(),
    );
    show_account_view(window, release);
}

/// The franchise's other releases, as chips under the description.
fn show_related(window: &MainWindow, state: &Rc<RefCell<ReleaseState>>, release: &Release) {
    let others: Vec<Release> = release
        .related_releases
        .iter()
        .filter(|other| other.id != release.id)
        .cloned()
        .collect();
    let items: Vec<LinkItem> = others
        .iter()
        .map(|other| LinkItem {
            label: other.title().into(),
            detail: other.year.as_str().into(),
        })
        .collect();
    window.set_release_related(slint::ModelRc::new(VecModel::from(items)));
    state.borrow_mut().related = others;
}

/// Opens the whole franchise, oldest first: every page of it, sorted by
/// when each release first aired, then by year and season for those the
/// service gives no date.
pub fn open_franchise(
    window: &MainWindow,
    state: &Rc<RefCell<ReleaseState>>,
    client: &Client,
    http: reqwest::Client,
) {
    let (current, related) = {
        let state = state.borrow();
        let Some(release) = state.release.as_ref() else {
            return;
        };
        (release.id, release.related.as_ref().map(|r| r.id))
    };
    let Some(related) = related.filter(|id| *id > 0) else {
        return;
    };
    window.set_franchise_items(slint::ModelRc::new(VecModel::<FranchiseItem>::default()));
    window.set_franchise_loading(true);
    window.set_franchise_open(true);

    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = client.clone();
    tasks::spawn(
        async move {
            let mut all = Vec::new();
            for page in 0..20 {
                let found = api.related(related, page).await?;
                let more = found.has_next();
                all.extend(found.content);
                if !more {
                    break;
                }
            }
            Ok::<_, anirust_api::Error>(all)
        },
        move |result| {
            let Some(window) = weak.upgrade() else { return };
            window.set_franchise_loading(false);
            let mut releases = match result {
                Ok(releases) => releases,
                Err(error) => {
                    tracing::warn!(%error, related, "the franchise was not loaded");
                    return;
                }
            };
            releases.sort_by_key(chronology);
            let items: Vec<FranchiseItem> = releases
                .iter()
                .map(|r| FranchiseItem {
                    title: r.title().into(),
                    detail: franchise_detail(r).into(),
                    poster: slint::Image::default(),
                    poster_loaded: false,
                    current: r.id == current,
                })
                .collect();
            let model = Rc::new(VecModel::from(items));
            window.set_franchise_items(slint::ModelRc::from(Rc::clone(&model)));
            for (index, release) in releases.iter().enumerate() {
                let url = release.poster_url();
                if url.is_empty() {
                    continue;
                }
                let model = Rc::clone(&model);
                tasks::spawn(tasks::fetch_image(http.clone(), url), move |result| {
                    let Ok(buffer) = result else { return };
                    if let Some(mut item) = model.row_data(index) {
                        item.poster = slint::Image::from_rgba8(buffer);
                        item.poster_loaded = true;
                        model.set_row_data(index, item);
                    }
                });
            }
            state.borrow_mut().franchise = releases;
        },
    );
}

/// The release at a row of the franchise sheet.
#[must_use]
pub fn franchise_at(state: &Rc<RefCell<ReleaseState>>, index: usize) -> Option<i64> {
    state.borrow().franchise.get(index).map(|r| r.id)
}

/// Where a release falls in its franchise: when it first aired, or failing
/// that its year and season. Those with neither go last.
fn chronology(release: &Release) -> (i64, i64, i32) {
    let year = release
        .year
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|y| *y > 1900);
    match (release.aired_on_date, year) {
        (aired, _) if aired > 0 => (aired, 0, 0),
        // The middle of the year, so a dated release of the same year sorts
        // around it rather than always after.
        (_, Some(year)) => ((year - 1970) * 31_556_952 + 15_778_476, 1, release.season),
        _ => (i64::MAX, 2, 0),
    }
}

/// Its kind and its year, as far as either is known.
fn franchise_detail(release: &Release) -> String {
    match (release.category.name.as_str(), release.year.as_str()) {
        ("", "") => String::new(),
        (kind, "") => kind.to_owned(),
        ("", year) => year.to_owned(),
        (kind, year) => format!("{kind} · {year}"),
    }
}

/// The services carrying the release. Anonymous, and on its own: the page is
/// complete without it, and it should not hold the release up.
fn load_platforms(
    window: &MainWindow,
    state: &Rc<RefCell<ReleaseState>>,
    client: &Client,
    release_id: i64,
) {
    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = client.clone();
    tasks::spawn(
        async move { api.streaming_platforms(release_id).await },
        move |platforms| {
            let Some(window) = weak.upgrade() else { return };
            if state.borrow().release_id != release_id {
                return;
            }
            let platforms = match platforms {
                Ok(platforms) => platforms,
                Err(error) => {
                    tracing::debug!(%error, release_id, "no streaming platforms");
                    return;
                }
            };
            let platforms: Vec<_> = platforms
                .into_iter()
                .filter(|platform| is_web_link(&platform.url))
                .collect();
            let items: Vec<LinkItem> = platforms
                .iter()
                .map(|platform| LinkItem {
                    label: platform.name.as_str().into(),
                    detail: "".into(),
                })
                .collect();
            window.set_release_platforms(slint::ModelRc::new(VecModel::from(items)));
            state.borrow_mut().platforms = platforms.into_iter().map(|p| p.url).collect();
        },
    );
}

/// Counts the release's comments. The release itself says how many it
/// carries as a preview — five — not how many there are; the thread knows.
fn load_comment_count(
    window: &MainWindow,
    state: &Rc<RefCell<ReleaseState>>,
    client: &Client,
    release_id: i64,
) {
    window.set_release_comment_count(0);
    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = client.clone();
    tasks::spawn(
        async move {
            api.comments(CommentTarget::Release, release_id, 0, CommentSort::Newest)
                .await
        },
        move |result| {
            let Some(window) = weak.upgrade() else { return };
            if state.borrow().release_id != release_id {
                return;
            }
            match result {
                Ok(page) => window
                    .set_release_comment_count(i32::try_from(page.total_count).unwrap_or(i32::MAX)),
                Err(error) => tracing::debug!(%error, release_id, "the comments were not counted"),
            }
        },
    );
}

/// Loads the release's videos: every kind of them, newest first within each.
fn load_videos(
    window: &MainWindow,
    state: &Rc<RefCell<ReleaseState>>,
    client: &Client,
    http: reqwest::Client,
    release_id: i64,
) {
    window.set_release_videos(slint::ModelRc::new(VecModel::<VideoItem>::default()));
    state.borrow_mut().videos.clear();
    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = client.clone();
    tasks::spawn(
        async move { api.release_videos(release_id).await },
        move |videos| {
            let Some(window) = weak.upgrade() else { return };
            if state.borrow().release_id != release_id {
                return;
            }
            let videos = match videos {
                Ok(videos) => videos,
                Err(error) => {
                    tracing::debug!(%error, release_id, "no videos");
                    return;
                }
            };
            // Only those with a page to open.
            let videos: Vec<_> = videos
                .blocks
                .into_iter()
                .flat_map(|block| block.videos)
                .filter(|video| is_web_link(&video.url))
                .collect();
            let model = Rc::new(VecModel::from(
                videos
                    .iter()
                    .map(|video| VideoItem {
                        title: video.title.as_str().into(),
                        detail: video_detail(video).into(),
                        image: slint::Image::default(),
                        image_loaded: false,
                    })
                    .collect::<Vec<_>>(),
            ));
            window.set_release_videos(slint::ModelRc::from(Rc::clone(&model)));
            for (index, video) in videos.iter().enumerate() {
                if !video.image.starts_with("http") {
                    continue;
                }
                let model = Rc::clone(&model);
                tasks::spawn(
                    tasks::fetch_image(http.clone(), video.image.clone()),
                    move |result| {
                        let Ok(buffer) = result else { return };
                        if let Some(mut item) = model.row_data(index) {
                            item.image = slint::Image::from_rgba8(buffer);
                            item.image_loaded = true;
                            model.set_row_data(index, item);
                        }
                    },
                );
            }
            state.borrow_mut().videos = videos.into_iter().map(|v| v.url).collect();
        },
    );
}

/// What kind of video, and where it is hosted.
fn video_detail(video: &anirust_api::ReleaseVideo) -> String {
    match (video.category.name.as_str(), video.hosting.name.as_str()) {
        ("", host) => host.to_owned(),
        (kind, "") => kind.to_owned(),
        (kind, host) => format!("{kind} · {host}"),
    }
}

/// Opens one of the release's videos where it is hosted.
pub fn open_video(state: &Rc<RefCell<ReleaseState>>, index: usize) {
    if let Some(url) = state.borrow().videos.get(index) {
        open_in_browser(url);
    }
}

/// Whether a link is one a browser should be handed: http or https and
/// nothing else, since anything else could be a command.
pub fn is_web_link(url: &str) -> bool {
    let url = url.trim();
    url.starts_with("https://") || url.starts_with("http://")
}

/// Opens a web page in the desktop's browser.
///
/// The URL goes to the platform's opener as a single argument, never through
/// a shell, and only if it is a web link — a server-supplied string is not
/// something to run.
pub fn open_in_browser(url: &str) {
    if !is_web_link(url) {
        tracing::warn!(url, "not opening a link that is not a web page");
        return;
    }
    #[cfg(target_os = "linux")]
    let opened = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let opened = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let opened = std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn();
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    let opened: std::io::Result<std::process::Child> =
        Err(std::io::Error::other("no browser opener on this platform"));

    if let Err(error) = opened {
        tracing::warn!(%error, url, "the browser could not be opened");
    }
}

/// What the account thinks of the release: its list, favourite, rating.
fn show_account_view(window: &MainWindow, release: &Release) {
    window.set_release_list(list_index(release.list()));
    window.set_release_favourite(release.is_favorite);
    window.set_release_vote(release.your_vote.clamp(0, 5));
}

/// A list as the header's dropdown counts it: 0 for none, then the five lists
/// in the order `ProfileList::ALL` gives them — which is also the order of
/// their numbers on the wire, so the two cannot drift apart.
fn list_index(list: Option<ProfileList>) -> i32 {
    list.map_or(0, ProfileList::raw)
}

/// The list at a position of the dropdown, or none for position 0.
fn list_at(index: i32) -> Option<ProfileList> {
    usize::try_from(index)
        .ok()
        .and_then(|at| at.checked_sub(1))
        .and_then(|at| ProfileList::ALL.get(at).copied())
}

// ---------------------------------------------------------------------------
// What the account does to a release
// ---------------------------------------------------------------------------
//
// All three change the screen first and the server second, and put the screen
// back if the server refuses. The control is under the pointer; a second of
// nothing reads as a click that did not land.

/// Moves the release into a list, or out of every list with position 0.
pub fn set_list(
    window: &MainWindow,
    state: &Rc<RefCell<ReleaseState>>,
    client: &Client,
    index: i32,
) {
    let Some((release_id, before)) = current(state, |r| r.list()) else {
        return;
    };
    let after = list_at(index);
    if after == before {
        return;
    }

    remember(state, |r| {
        r.profile_list_status = after.map_or(0, ProfileList::raw)
    });
    window.set_release_list(list_index(after));

    let api = client.clone();
    let undo = undo_with(window, state, move |r| {
        r.profile_list_status = before.map_or(0, ProfileList::raw);
    });
    tasks::spawn(
        async move {
            match (before, after) {
                // Out of every list: only the one it is in knows it.
                (Some(old), None) => api.profile_list_delete(old, release_id).await,
                (_, Some(new)) => api.profile_list_add(new, release_id).await,
                (None, None) => Ok(()),
            }
        },
        move |result| {
            if let Err(error) = result {
                tracing::warn!(%error, release_id, "the list was not changed");
                undo();
            }
        },
    );
}

/// Adds the release to the favourites, or takes it out.
pub fn toggle_favourite(window: &MainWindow, state: &Rc<RefCell<ReleaseState>>, client: &Client) {
    let Some((release_id, before)) = current(state, |r| r.is_favorite) else {
        return;
    };
    let after = !before;

    remember(state, |r| r.is_favorite = after);
    window.set_release_favourite(after);

    let api = client.clone();
    let undo = undo_with(window, state, move |r| r.is_favorite = before);
    tasks::spawn(
        async move {
            if after {
                api.favorite_add(release_id).await
            } else {
                api.favorite_delete(release_id).await
            }
        },
        move |result| {
            if let Err(error) = result {
                tracing::warn!(%error, release_id, "the favourite was not changed");
                undo();
            }
        },
    );
}

/// Rates the release. The rating it already has, clicked again, is withdrawn.
pub fn set_vote(
    window: &MainWindow,
    state: &Rc<RefCell<ReleaseState>>,
    client: &Client,
    stars: i32,
) {
    let Some((release_id, before)) = current(state, |r| r.your_vote) else {
        return;
    };
    let after = next_vote(before, stars);

    remember(state, |r| r.your_vote = after);
    window.set_release_vote(after);

    let api = client.clone();
    let undo = undo_with(window, state, move |r| r.your_vote = before);
    tasks::spawn(
        async move {
            match u8::try_from(after) {
                Ok(stars @ 1..=5) => api.release_vote(release_id, stars).await,
                _ => api.release_vote_delete(release_id).await,
            }
        },
        move |result| {
            if let Err(error) = result {
                tracing::warn!(%error, release_id, "the rating was not changed");
                undo();
            }
        },
    );
}

/// Pins the voice-over on screen as the one this release opens with, or
/// unpins it.
///
/// One per release: pinning one unpins whichever was pinned before, which is
/// how the server keeps it, so the screen does the same rather than showing
/// two pins until the next fetch.
pub fn toggle_pin(window: &MainWindow, state: &Rc<RefCell<ReleaseState>>, client: &Client) {
    let (release_id, dubber_id, before, pins) = {
        let state = state.borrow();
        let Some(dubber) = state.selected_dubber() else {
            return;
        };
        let pins: Vec<bool> = state.dubbers.iter().map(|d| d.pinned).collect();
        (state.release_id, dubber.id, dubber.pinned, pins)
    };
    let pinning = !before;

    set_pins(state, |id| pinning && id == dubber_id);
    show_dubbers(window, &state.borrow());

    let api = client.clone();
    let weak = window.as_weak();
    let state = Rc::clone(state);
    tasks::spawn(
        async move {
            if pinning {
                api.dubber_pin(release_id, dubber_id).await
            } else {
                api.dubber_unpin(release_id, dubber_id).await
            }
        },
        move |result| {
            let Err(error) = result else { return };
            tracing::warn!(%error, release_id, dubber_id, "the voice-over was not pinned");
            let Some(window) = weak.upgrade() else { return };
            if state.borrow().release_id != release_id {
                return;
            }
            for (dubber, pinned) in state.borrow_mut().dubbers.iter_mut().zip(pins) {
                dubber.pinned = pinned;
            }
            show_dubbers(&window, &state.borrow());
        },
    );
}

fn set_pins(state: &Rc<RefCell<ReleaseState>>, pinned: impl Fn(i64) -> bool) {
    for dubber in &mut state.borrow_mut().dubbers {
        dubber.pinned = pinned(dubber.id);
    }
}

/// Ticks one episode as watched, or takes the tick off.
///
/// Recorded on this machine always, and on the account when there is one: a
/// viewer without an account still keeps their place, so the tick has to
/// mean something for them too.
pub fn toggle_watched(
    window: &MainWindow,
    state: &Rc<RefCell<ReleaseState>>,
    client: &Client,
    position: i32,
) {
    let Some(change) = mark(state, &[position], None) else {
        return;
    };
    show_episodes(window, &state.borrow());
    send_marks(window, state, client, change, Some(position));
}

/// Ticks every episode, or takes every tick off when they are all ticked.
pub fn toggle_all_watched(window: &MainWindow, state: &Rc<RefCell<ReleaseState>>, client: &Client) {
    let (positions, all) = {
        let state = state.borrow();
        let positions = state.positions();
        let all = !state.episodes.is_empty() && state.episodes.iter().all(|e| state.watched(e));
        (positions, all)
    };
    let Some(change) = mark(state, &positions, Some(!all)) else {
        return;
    };
    show_episodes(window, &state.borrow());
    send_marks(window, state, client, change, None);
}

/// What a tick changed, kept so it can be put back.
struct MarkChange {
    release_id: i64,
    source_id: Option<i64>,
    /// The new state — what was asked for.
    watched: bool,
    /// Each episode as it was: the account's flag and this machine's entry.
    before: Vec<(i32, bool, Option<crate::progress::Entry>)>,
}

/// Sets the episodes at `positions` watched or not, on screen and on this
/// machine. With `to` unset, the one episode flips.
fn mark(
    state: &Rc<RefCell<ReleaseState>>,
    positions: &[i32],
    to: Option<bool>,
) -> Option<MarkChange> {
    let mut state = state.borrow_mut();
    let release_id = state.release_id;
    let source_id = state.selected_source().map(|source| source.id);
    let watched = match to {
        Some(watched) => watched,
        None => {
            let episode = state.episode_at(*positions.first()?)?;
            !state.watched(episode)
        }
    };

    let progress = Rc::clone(&state.progress);
    let mut store = progress.borrow_mut();
    let mut before = Vec::with_capacity(positions.len());
    for episode in state
        .episodes
        .iter_mut()
        .filter(|e| positions.contains(&e.position))
    {
        before.push((
            episode.position,
            episode.is_watched,
            store.get(release_id, episode.position),
        ));
        episode.is_watched = watched;
        store.set_finished(release_id, episode.position, watched);
    }
    store.flush();

    Some(MarkChange {
        release_id,
        source_id,
        watched,
        before,
    })
}

/// Tells the account, and puts everything back if it refuses.
fn send_marks(
    window: &MainWindow,
    state: &Rc<RefCell<ReleaseState>>,
    client: &Client,
    change: MarkChange,
    position: Option<i32>,
) {
    // Without an account the tick lives on this machine only, which is
    // already done.
    let Some(source_id) = change.source_id.filter(|_| client.is_authenticated()) else {
        return;
    };

    let api = client.clone();
    let (release_id, watched) = (change.release_id, change.watched);
    let weak = window.as_weak();
    let state = Rc::clone(state);
    tasks::spawn(
        async move {
            match (position, watched) {
                (Some(at), true) => api.mark_watched(release_id, source_id, at).await,
                (Some(at), false) => api.mark_unwatched(release_id, source_id, at).await,
                (None, true) => api.mark_all_watched(release_id, source_id).await,
                (None, false) => api.mark_all_unwatched(release_id, source_id).await,
            }
        },
        move |result| {
            let Err(error) = result else { return };
            tracing::warn!(%error, release_id, "the watched state was not changed");
            let Some(window) = weak.upgrade() else { return };
            let mut guard = state.borrow_mut();
            if guard.release_id != change.release_id {
                return;
            }
            let progress = Rc::clone(&guard.progress);
            let mut store = progress.borrow_mut();
            for (at, flag, entry) in change.before {
                if let Some(episode) = guard.episodes.iter_mut().find(|e| e.position == at) {
                    episode.is_watched = flag;
                }
                store.restore(change.release_id, at, entry);
            }
            store.flush();
            drop(store);
            drop(guard);
            show_episodes(&window, &state.borrow());
        },
    );
}

/// The rating after a click on `stars`: that many, or none if it was already
/// that many.
fn next_vote(before: i32, stars: i32) -> i32 {
    let stars = stars.clamp(1, 5);
    if before == stars { 0 } else { stars }
}

/// The release on screen and one thing about it, if one is loaded.
fn current<T>(
    state: &Rc<RefCell<ReleaseState>>,
    read: impl FnOnce(&Release) -> T,
) -> Option<(i64, T)> {
    let state = state.borrow();
    let release = state.release.as_ref()?;
    Some((release.id, read(release)))
}

/// Writes a change into the loaded release.
fn remember(state: &Rc<RefCell<ReleaseState>>, write: impl FnOnce(&mut Release)) {
    if let Some(release) = state.borrow_mut().release.as_mut() {
        write(release);
    }
}

/// What puts a change back: writes `restore` into the release — if it is still
/// the one on screen — and shows the result.
fn undo_with(
    window: &MainWindow,
    state: &Rc<RefCell<ReleaseState>>,
    restore: impl FnOnce(&mut Release) + 'static,
) -> impl FnOnce() + 'static {
    let weak = window.as_weak();
    let state = Rc::clone(state);
    let release_id = state.borrow().release_id;
    move || {
        let Some(window) = weak.upgrade() else { return };
        let mut guard = state.borrow_mut();
        // The viewer may have opened another release while the request was
        // out; undoing then would write one release's state into another's.
        if guard.release_id != release_id {
            return;
        }
        if let Some(release) = guard.release.as_mut() {
            restore(release);
            show_account_view(&window, release);
        }
    }
}

fn show_dubbers(window: &MainWindow, state: &ReleaseState) {
    let items: Vec<PickerOption> = state
        .dubbers
        .iter()
        .map(|d| PickerOption {
            label: d.name.as_str().into(),
            episodes: d.episodes_count as i32,
            is_sub: d.is_sub,
            pinned: d.pinned,
        })
        .collect();

    window.set_dubbers(slint::ModelRc::new(VecModel::from(items)));
}

fn show_sources(window: &MainWindow, state: &ReleaseState) {
    let items: Vec<PickerOption> = state
        .sources
        .iter()
        .map(|s| PickerOption {
            label: s.name.as_str().into(),
            episodes: s.episodes_count as i32,
            is_sub: false,
            pinned: false,
        })
        .collect();

    window.set_sources(slint::ModelRc::new(VecModel::from(items)));
}

fn show_episodes(window: &MainWindow, state: &ReleaseState) {
    let items: Vec<EpisodeItem> = state
        .episodes
        .iter()
        .map(|e| EpisodeItem {
            position: e.position,
            name: episode_name(e).into(),
            watched: state.watched(e),
            resume_at: state
                .resume_of(e)
                .map(crate::format_time)
                .unwrap_or_default()
                .into(),
            filler: e.is_filler,
        })
        .collect();

    let all_watched = !items.is_empty() && items.iter().all(|item| item.watched);
    window.set_episodes(slint::ModelRc::new(VecModel::from(items)));
    window.set_all_episodes_watched(all_watched);

    let resume = state.resume_position();
    window.set_resume_episode(resume);
    // Open the list where the viewer left off rather than at episode one.
    // Nothing is playing yet, so this scrolls without marking anything.
    if let Some(index) = state.index_of(resume) {
        window.set_current_index(index as i32);
    }
}

/// The host's title for an episode, or nothing when it only restates the
/// number the row already shows.
///
/// Most sources send exactly "12 серия", which beside a column of numbers read
/// as "12  12 серия" — the same fact twice, on every row.
fn episode_name(episode: &Episode) -> &str {
    let name = episode.name.trim();
    if name.is_empty() || says_only_the_number(name, episode.position) {
        return "";
    }
    name
}

/// Whether a title is the episode number dressed in a word.
fn says_only_the_number(name: &str, position: i32) -> bool {
    /// Every word the sources use for "episode", in both languages and in the
    /// abbreviations they favour. Anything else is a title worth showing.
    const FILLER_WORDS: [&str; 7] = ["серия", "серии", "эпизод", "episode", "ep", "e", "s"];

    let stripped = name
        .to_lowercase()
        .replace(&position.to_string(), " ")
        .replace(['.', '-', '–', '—', '#', ':', '№'], " ");

    stripped
        .split_whitespace()
        .all(|word| FILLER_WORDS.contains(&word))
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_franchise_runs_from_its_first_airing() {
        let release = |aired: i64, year: &str| Release {
            aired_on_date: aired,
            year: year.to_owned(),
            ..Release::default()
        };
        let mut all = [
            release(0, ""),
            release(1_600_000_000, "2020"),
            release(0, "2012"),
            release(1_300_000_000, "2011"),
        ];
        all.sort_by_key(chronology);
        let years: Vec<&str> = all.iter().map(|r| r.year.as_str()).collect();
        assert_eq!(years, ["2011", "2012", "2020", ""]);
    }
    use super::*;

    #[test]
    fn a_dropdown_position_names_the_same_list_back() {
        assert_eq!(list_at(0), None);
        for list in ProfileList::ALL {
            assert_eq!(list_at(list_index(Some(list))), Some(list));
        }
        assert_eq!(list_at(6), None);
        assert_eq!(list_at(-1), None);
    }

    #[test]
    fn only_web_pages_are_handed_to_the_browser() {
        assert!(is_web_link("https://www.crunchyroll.com/series/x"));
        assert!(is_web_link("http://example.org"));
        assert!(!is_web_link("file:///etc/passwd"));
        assert!(!is_web_link("javascript:alert(1)"));
        assert!(!is_web_link("--help"));
        assert!(!is_web_link(""));
    }

    #[test]
    fn clicking_the_rating_already_given_takes_it_back() {
        assert_eq!(next_vote(0, 4), 4);
        assert_eq!(next_vote(4, 4), 0);
        assert_eq!(next_vote(4, 2), 2);
        assert_eq!(next_vote(0, 9), 5);
    }

    fn episode(position: i32, watched: bool, playback_position: i64) -> Episode {
        Episode {
            position,
            is_watched: watched,
            playback_position,
            ..Episode::default()
        }
    }

    fn named(position: i32, name: &str) -> Episode {
        Episode {
            position,
            name: name.to_owned(),
            ..Episode::default()
        }
    }

    fn state_with(episodes: Vec<Episode>) -> ReleaseState {
        ReleaseState {
            episodes,
            ..ReleaseState::default()
        }
    }

    #[test]
    fn nothing_started_means_nothing_to_resume() {
        let state = state_with(vec![episode(1, false, 0), episode(2, false, 0)]);
        assert_eq!(state.resume_position(), 0);
    }

    #[test]
    fn a_part_watched_episode_is_offered_again() {
        // Left at 8 minutes into episode 3: that is where to go back to.
        let state = state_with(vec![
            episode(1, true, 0),
            episode(2, true, 0),
            episode(3, false, 480_000),
        ]);
        assert_eq!(state.resume_position(), 3);
    }

    #[test]
    fn a_finished_episode_offers_the_next_one() {
        let state = state_with(vec![
            episode(1, true, 0),
            episode(2, true, 0),
            episode(3, false, 0),
        ]);
        assert_eq!(state.resume_position(), 3);
    }

    #[test]
    fn finishing_the_last_episode_offers_it_rather_than_nothing() {
        let state = state_with(vec![episode(1, true, 0), episode(2, true, 0)]);
        assert_eq!(state.resume_position(), 2);
    }

    #[test]
    fn stepping_through_episodes_stops_at_the_ends() {
        let state = state_with(vec![
            episode(1, false, 0),
            episode(2, false, 0),
            episode(3, false, 0),
        ]);
        assert_eq!(state.next_after(1), Some(2));
        assert_eq!(state.next_after(3), None);
        assert_eq!(state.previous_before(2), Some(1));
        assert_eq!(state.previous_before(1), None);
    }

    #[test]
    fn stepping_works_on_gapped_numbering() {
        // Sources skip numbers more often than you would hope.
        let state = state_with(vec![
            episode(1, false, 0),
            episode(5, false, 0),
            episode(9, false, 0),
        ]);
        assert_eq!(state.next_after(1), Some(5));
        assert_eq!(state.previous_before(9), Some(5));
    }

    #[test]
    fn a_title_that_only_restates_the_number_is_dropped() {
        for name in [
            "12 серия",
            "Серия 12",
            "Episode 12",
            "  ep. 12 ",
            "12",
            "#12",
            "s12",
        ] {
            assert_eq!(episode_name(&named(12, name)), "", "{name:?}");
        }
    }

    #[test]
    fn a_real_title_is_kept() {
        for name in [
            "Рождение",
            "12 серия: Рождение",
            "Birth of a Demon",
            "Глава 12",
        ] {
            assert_eq!(episode_name(&named(12, name)), name.trim(), "{name:?}");
        }
    }

    #[test]
    fn a_stale_answer_is_recognised_as_stale() {
        let mut state = ReleaseState::default();
        let first = state.next_generation();
        let second = state.next_generation();

        assert!(
            !state.is_current(first),
            "the older request must be ignored"
        );
        assert!(state.is_current(second));
    }
}
