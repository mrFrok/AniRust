// SPDX-License-Identifier: GPL-3.0-or-later

//! The browsing screen: finding a release to open.
//!
//! Two sources of results, one list: what other people are watching right now,
//! which is what the screen opens on, and whatever was searched for. Posters
//! arrive afterwards and are dropped into the rows they belong to, so the grid
//! is usable the moment the titles land.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, Model, VecModel};

use anirust_api::{Client, Release, SearchBy};

use crate::{MainWindow, ReleaseCard, tasks};

/// How many results one page of browsing shows.
///
/// The server decides for search; this caps what is asked of it for the
/// default list, which is otherwise long enough to fetch a hundred posters
/// nobody scrolls to.
const DEFAULT_LIMIT: usize = 30;

/// What the browsing screen is showing.
#[derive(Default)]
pub struct HomeState {
    /// The releases behind the cards, in the same order.
    pub releases: Vec<Release>,
    cards: Option<Rc<VecModel<ReleaseCard>>>,
    /// Bumped on every new list, so a poster for a list the viewer has already
    /// moved past is dropped rather than drawn over the new one.
    generation: u64,
}

impl HomeState {
    fn next_generation(&mut self) -> u64 {
        self.generation += 1;
        self.generation
    }

    /// The release a card stands for.
    #[must_use]
    pub fn release_at(&self, index: usize) -> Option<&Release> {
        self.releases.get(index)
    }
}

/// Loads what the screen opens on.
pub fn open(
    window: &MainWindow,
    state: &Rc<RefCell<HomeState>>,
    client: Rc<Client>,
    http: reqwest::Client,
) {
    // Which heading to print is the Slint side's business — that is where
    // both languages live.
    window.set_searching(false);
    window.set_results_loading(true);

    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = (*client).clone();
    let generation = state.borrow_mut().next_generation();

    tasks::spawn(async move { api.discover_watching(0).await }, move |page| {
        let Some(window) = weak.upgrade() else { return };
        match page {
            Ok(page) => show(&window, &state, page.content, generation, http),
            Err(error) => {
                tracing::error!(%error, "could not load the discover list");
                window.set_results_loading(false);
            }
        }
    });
}

/// Searches, or goes back to the default list when the query is emptied.
pub fn search(
    window: &MainWindow,
    state: &Rc<RefCell<HomeState>>,
    client: Rc<Client>,
    http: reqwest::Client,
    query: String,
) {
    let query = query.trim().to_owned();
    if query.is_empty() {
        open(window, state, client, http);
        return;
    }

    window.set_searching(true);
    window.set_results_loading(true);

    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = (*client).clone();
    let generation = state.borrow_mut().next_generation();

    tasks::spawn(
        async move { api.search_releases(&query, SearchBy::Title, 0).await },
        move |found| {
            let Some(window) = weak.upgrade() else { return };
            match found {
                Ok(releases) => show(&window, &state, releases, generation, http),
                Err(error) => {
                    tracing::error!(%error, "search failed");
                    window.set_results_loading(false);
                }
            }
        },
    );
}

/// Puts a list of releases on the screen and starts fetching their posters.
fn show(
    window: &MainWindow,
    state: &Rc<RefCell<HomeState>>,
    releases: Vec<Release>,
    generation: u64,
    http: reqwest::Client,
) {
    if state.borrow().generation != generation {
        tracing::debug!("discarding results the viewer has moved past");
        return;
    }

    let releases: Vec<Release> = releases.into_iter().take(DEFAULT_LIMIT).collect();
    tracing::info!(count = releases.len(), "results");
    let cards: Vec<ReleaseCard> = releases.iter().map(card_for).collect();
    let model = Rc::new(VecModel::from(cards));

    window.set_results(slint::ModelRc::from(Rc::clone(&model)));
    window.set_results_loading(false);

    let posters: Vec<String> = releases.iter().map(Release::poster_url).collect();
    {
        let mut state = state.borrow_mut();
        state.releases = releases;
        state.cards = Some(Rc::clone(&model));
    }

    for (index, url) in posters.into_iter().enumerate() {
        load_poster(state, &model, index, generation, http.clone(), url);
    }
}

/// Fetches one poster and drops it into its row.
fn load_poster(
    state: &Rc<RefCell<HomeState>>,
    model: &Rc<VecModel<ReleaseCard>>,
    index: usize,
    generation: u64,
    http: reqwest::Client,
    url: String,
) {
    if url.is_empty() {
        return;
    }

    let state = Rc::clone(state);
    let model = Rc::clone(model);

    tasks::spawn(tasks::fetch_image(http, url), move |result| {
        // Two ways this can be stale: a newer list replaced this one, or the
        // row it belongs to no longer exists.
        if state.borrow().generation != generation {
            return;
        }
        let Ok(buffer) = result else {
            tracing::debug!(index, "poster not loaded");
            return;
        };
        let Some(mut card) = model.row_data(index) else {
            return;
        };

        card.poster = slint::Image::from_rgba8(buffer);
        card.poster_loaded = true;
        model.set_row_data(index, card);
    });
}

fn card_for(release: &Release) -> ReleaseCard {
    ReleaseCard {
        title: release.title().into(),
        subtitle: subtitle(release).into(),
        score: format!("{:.1}", release.score()).into(),
        poster: slint::Image::default(),
        poster_loaded: false,
    }
}

/// Year and episode count, joined — and gracefully short when the server sends
/// neither, which happens on sparse search results.
fn subtitle(release: &Release) -> String {
    let episodes = match (release.episodes_released, release.episodes_total) {
        (0, 0) => String::new(),
        (released, 0) => released.to_string(),
        (released, total) => format!("{released}/{total}"),
    };

    match (release.year.as_str(), episodes.as_str()) {
        ("", "") => String::new(),
        (year, "") => year.to_owned(),
        ("", episodes) => episodes.to_owned(),
        (year, episodes) => format!("{year} · {episodes}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(year: &str, released: i32, total: i32) -> Release {
        Release {
            year: year.to_owned(),
            episodes_released: released,
            episodes_total: total,
            ..Release::default()
        }
    }

    #[test]
    fn a_complete_release_reads_as_year_and_count() {
        assert_eq!(subtitle(&release("2012", 12, 12)), "2012 · 12/12");
    }

    #[test]
    fn an_airing_release_shows_what_is_out() {
        assert_eq!(subtitle(&release("2026", 7, 0)), "2026 · 7");
    }

    #[test]
    fn a_sparse_result_says_only_what_it_knows() {
        assert_eq!(subtitle(&release("2012", 0, 0)), "2012");
        assert_eq!(subtitle(&release("", 12, 12)), "12/12");
        assert_eq!(subtitle(&release("", 0, 0)), "");
    }
}
