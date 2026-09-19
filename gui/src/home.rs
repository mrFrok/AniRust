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

use anirust_api::{Client, Filter, FilterSort, Release, SearchBy};

use crate::{MainWindow, ReleaseCard, tasks};

/// How many results one page of browsing shows.
///
/// The server decides for search; this caps what is asked of it for the
/// default list, which is otherwise long enough to fetch a hundred posters
/// nobody scrolls to.
const DEFAULT_LIMIT: usize = 30;

/// Genres offered as chips, most common in the catalogue first.
///
/// The server matches these by name — its own spelling, lowercase and Russian
/// — and no endpoint lists them. The tail of rarely used ones is left out: a
/// row of fifty chips is not a filter, it is a wall.
pub const GENRES: [&str; 18] = [
    "экшен",
    "фэнтези",
    "приключения",
    "драма",
    "комедия",
    "романтика",
    "школа",
    "исэкай",
    "сёнен",
    "сэйнэн",
    "триллер",
    "психологическое",
    "сверхъестественное",
    "фантастика",
    "детектив",
    "спорт",
    "ужасы",
    "повседневность",
];

/// Category and status ids, read off the wire rather than from documentation.
const FILM: i64 = 2;
const AIRING: i64 = 2;
const ANNOUNCED: i64 = 3;

/// One of the ways the catalogue can be sliced, in the order the chips offer
/// them.
///
/// The labels live on the Slint side with the rest of the translations, so this
/// is only the request each one stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// What other people have open right now, which is its own endpoint.
    Watching,
    Updates,
    Popular,
    Airing,
    Announced,
    Films,
    Top,
}

impl Section {
    const ALL: [Self; 7] = [
        Self::Watching,
        Self::Updates,
        Self::Popular,
        Self::Airing,
        Self::Announced,
        Self::Films,
        Self::Top,
    ];

    fn at(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or(Self::Watching)
    }

    fn index_of(self) -> usize {
        Self::ALL.iter().position(|it| *it == self).unwrap_or(0)
    }

    /// The catalogue request behind a section, or `None` for the one that is
    /// not a catalogue request at all.
    fn filter(self) -> Option<Filter> {
        Some(match self {
            Self::Watching => return None,
            Self::Updates => Filter::sorted_by(FilterSort::LastUpdate),
            Self::Popular => Filter::sorted_by(FilterSort::Popularity),
            Self::Airing => Filter::sorted_by(FilterSort::Popularity).status(AIRING),
            Self::Announced => Filter::sorted_by(FilterSort::Year).status(ANNOUNCED),
            Self::Films => Filter::sorted_by(FilterSort::Popularity).category(FILM),
            Self::Top => Filter::sorted_by(FilterSort::Rating),
        })
    }
}

/// What the browsing screen is showing.
#[derive(Default)]
pub struct HomeState {
    /// The releases behind the cards, in the same order.
    pub releases: Vec<Release>,
    cards: Option<Rc<VecModel<ReleaseCard>>>,
    /// Which chip is selected, and which genre narrows it.
    section: usize,
    /// 0 is every genre; otherwise an index into [`GENRES`] plus one.
    genre: usize,
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

/// Loads the selected section, which is what the screen opens on.
pub fn open(
    window: &MainWindow,
    state: &Rc<RefCell<HomeState>>,
    client: Rc<Client>,
    http: reqwest::Client,
) {
    // Which heading to print is the Slint side's business — that is where both
    // languages live.
    window.set_searching(false);
    window.set_results_loading(true);
    window.set_genres(slint::ModelRc::new(VecModel::from(
        GENRES
            .iter()
            .map(|name| slint::SharedString::from(*name))
            .collect::<Vec<_>>(),
    )));

    let (section, genre, generation) = {
        let mut state = state.borrow_mut();
        let generation = state.next_generation();
        (Section::at(state.section), state.genre, generation)
    };
    window.set_section(Section::index_of(section) as i32);
    window.set_genre(genre as i32);

    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = (*client).clone();

    // One section is not a catalogue query: "what people have open right now"
    // is not something the filter can ask for, so it keeps its own endpoint.
    let Some(mut filter) = section.filter() else {
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
        return;
    };

    if let Some(name) = genre.checked_sub(1).and_then(|at| GENRES.get(at)) {
        filter = filter.genre(*name);
    }

    tasks::spawn(async move { api.filter(&filter, 0).await }, move |page| {
        let Some(window) = weak.upgrade() else { return };
        match page {
            Ok(page) => show(&window, &state, page.content, generation, http),
            Err(error) => {
                tracing::error!(%error, "could not load the catalogue");
                window.set_results_loading(false);
            }
        }
    });
}

/// Switches section, which also clears the search so the chips describe what
/// is actually on screen.
pub fn select_section(
    window: &MainWindow,
    state: &Rc<RefCell<HomeState>>,
    client: Rc<Client>,
    http: reqwest::Client,
    index: usize,
) {
    state.borrow_mut().section = index.min(Section::ALL.len() - 1);
    window.set_query("".into());
    open(window, state, client, http);
}

/// Narrows the section by genre, or widens it back to all of them.
pub fn select_genre(
    window: &MainWindow,
    state: &Rc<RefCell<HomeState>>,
    client: Rc<Client>,
    http: reqwest::Client,
    index: usize,
) {
    {
        let mut state = state.borrow_mut();
        state.genre = index.min(GENRES.len());
        // A genre is a catalogue filter, and "what people are watching" is not
        // a catalogue query — so choosing one moves off that section.
        if state.genre > 0 && Section::at(state.section) == Section::Watching {
            state.section = 1;
        }
    }
    window.set_query("".into());
    open(window, state, client, http);
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
