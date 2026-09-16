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

use slint::{ComponentHandle, VecModel};

use anirust_api::{Client, Dubber, Episode, EpisodeSort, Release, Source};

use crate::{EpisodeItem, MainWindow, PickerOption, tasks};

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
    /// Bumped on every selection change.
    ///
    /// A request started for an older selection may still be in flight when a
    /// newer one begins; comparing this on arrival is what stops a slow answer
    /// overwriting a fast one the viewer has since moved past.
    generation: u64,
}

impl ReleaseState {
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
            .filter(|e| e.is_watched || e.playback_position > 0)
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
            .is_some_and(|e| e.playback_position > 0 && !e.is_watched)
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
    window.set_release_loading(true);
    state.borrow_mut().release_id = release_id;

    let weak = window.as_weak();
    let state = Rc::clone(state);
    let fetch_client = Rc::clone(&client);

    // `Client` is cheap to clone and shares its connection pool, so the task
    // gets its own rather than borrowing across threads.
    let api = (*fetch_client).clone();
    tasks::spawn(
        async move {
            let release = api.release(release_id, false).await;
            let dubbers = api.dubbers(release_id).await;
            (release, dubbers)
        },
        move |(release, dubbers)| {
            let Some(window) = weak.upgrade() else { return };

            match (release, dubbers) {
                (Ok(release), Ok(dubbers)) => {
                    show_release(&window, &release);
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
                    select_dubber(&window, &state, client, 0);
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
    fetch_into(window, http, url, |window, image| {
        window.set_release_poster(image);
        window.set_poster_loaded(true);
    });
}

/// Fetches the still shown behind the play button.
pub fn load_backdrop(window: &MainWindow, http: reqwest::Client, url: String) {
    fetch_into(window, http, url, |window, image| {
        window.set_release_backdrop(image);
        window.set_backdrop_loaded(true);
    });
}

/// Loads one image and puts it on the screen, or leaves the screen alone.
///
/// Both images are decoration: a release without them is still entirely
/// usable, so a failure here is a debug line rather than anything the viewer
/// is told about.
fn fetch_into(
    window: &MainWindow,
    http: reqwest::Client,
    url: String,
    apply: impl FnOnce(&MainWindow, slint::Image) + 'static,
) {
    if url.is_empty() {
        return;
    }

    let weak = window.as_weak();
    tasks::spawn(tasks::fetch_image(http, url), move |result| {
        let Some(window) = weak.upgrade() else { return };
        match result {
            Ok(buffer) => apply(&window, slint::Image::from_rgba8(buffer)),
            Err(error) => tracing::debug!(%error, "image not loaded"),
        }
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
    window.set_release_score(format!("{:.1}", release.score()).into());
    window.set_release_episodes_label(
        format!("{}/{}", release.episodes_released, release.episodes_total).into(),
    );
}

fn show_dubbers(window: &MainWindow, state: &ReleaseState) {
    let items: Vec<PickerOption> = state
        .dubbers
        .iter()
        .map(|d| PickerOption {
            label: d.name.as_str().into(),
            episodes: d.episodes_count as i32,
            is_sub: d.is_sub,
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
            watched: e.is_watched,
            resume_at: e
                .resume_at()
                // Only worth showing where it is genuinely mid-episode: a few
                // seconds in is not "resume from", it is "start".
                .filter(|d| d.as_secs() > 30 && !e.is_watched)
                .map(crate::format_time)
                .unwrap_or_default()
                .into(),
            filler: e.is_filler,
        })
        .collect();

    window.set_episodes(slint::ModelRc::new(VecModel::from(items)));
    window.set_resume_episode(state.resume_position());
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
    use super::*;

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
