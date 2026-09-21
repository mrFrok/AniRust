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

use anirust_api::{Client, Filter, FilterSort, ProfileList, Release, SearchBy};

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
const FINISHED: i64 = 1;
const AIRING: i64 = 2;
const ANNOUNCED: i64 = 3;

/// Where the rail is pointing.
///
/// The labels live on the Slint side with the rest of the translations; this is
/// only what each one fetches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Destination {
    /// What is new, airing, or finished — the page the client opens on.
    #[default]
    Home,
    /// The catalogue proper, narrowed by genre.
    Browse,
    /// The account's own lists, history and favourites.
    Saved,
    Downloads,
    Profile,
}

impl Destination {
    const ALL: [Self; 5] = [
        Self::Home,
        Self::Browse,
        Self::Saved,
        Self::Downloads,
        Self::Profile,
    ];

    fn at(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or(Self::Home)
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|it| *it == self).unwrap_or(0)
    }

    /// How many tabs the destination has, so a stale selection is clamped
    /// rather than pointing past the end.
    fn tabs(self) -> usize {
        match self {
            Self::Home => 3,
            Self::Browse => 4,
            Self::Saved => 7,
            Self::Downloads | Self::Profile => 0,
        }
    }

    /// Whether the genre chips apply here. Only the catalogue is a filter.
    fn has_genres(self) -> bool {
        self == Self::Browse
    }
}

/// What one tab fetches.
enum Query {
    /// The catalogue, filtered.
    Catalogue(Box<Filter>),
    /// One of the account's lists.
    List(ProfileList),
    History,
    Favourites,
    /// Nothing to fetch — the destination is not a grid of releases.
    None,
}

/// The request behind a destination's tab.
fn query_for(destination: Destination, tab: usize) -> Query {
    match (destination, tab) {
        (Destination::Home, 0) => {
            Query::Catalogue(Box::new(Filter::sorted_by(FilterSort::LastUpdate)))
        }
        (Destination::Home, 1) => Query::Catalogue(Box::new(
            Filter::sorted_by(FilterSort::Popularity).status(AIRING),
        )),
        (Destination::Home, _) => Query::Catalogue(Box::new(
            Filter::sorted_by(FilterSort::Popularity).status(FINISHED),
        )),

        (Destination::Browse, 0) => {
            Query::Catalogue(Box::new(Filter::sorted_by(FilterSort::Popularity)))
        }
        (Destination::Browse, 1) => {
            Query::Catalogue(Box::new(Filter::sorted_by(FilterSort::Rating)))
        }
        (Destination::Browse, 2) => Query::Catalogue(Box::new(
            Filter::sorted_by(FilterSort::Popularity).category(FILM),
        )),
        (Destination::Browse, _) => Query::Catalogue(Box::new(
            Filter::sorted_by(FilterSort::Year).status(ANNOUNCED),
        )),

        (Destination::Saved, 5) => Query::History,
        (Destination::Saved, 6) => Query::Favourites,
        (Destination::Saved, tab) => Query::List(
            ProfileList::ALL
                .get(tab)
                .copied()
                .unwrap_or(ProfileList::Watching),
        ),

        (Destination::Downloads | Destination::Profile, _) => Query::None,
    }
}

/// What the browsing screen is showing.
#[derive(Default)]
pub struct HomeState {
    /// The releases behind the cards, in the same order.
    pub releases: Vec<Release>,
    cards: Option<Rc<VecModel<ReleaseCard>>>,
    /// Where the rail points, and which of its tabs is open.
    destination: Destination,
    tab: usize,
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
    window.set_notice("".into());

    let (destination, tab, genre, generation) = {
        let mut state = state.borrow_mut();
        let generation = state.next_generation();
        (state.destination, state.tab, state.genre, generation)
    };

    window.set_destination(destination.index() as i32);
    window.set_tab(tab as i32);
    window.set_genre(genre as i32);
    window.set_genres(slint::ModelRc::new(VecModel::from(
        if destination.has_genres() {
            GENRES
                .iter()
                .map(|name| slint::SharedString::from(*name))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        },
    )));

    let mut query = query_for(destination, tab);
    if let Query::Catalogue(filter) = &mut query
        && let Some(name) = genre.checked_sub(1).and_then(|at| GENRES.get(at))
    {
        **filter = std::mem::take(filter.as_mut()).genre(*name);
    }

    // The account's own lists are the only thing here that needs a session.
    // Saying so beats an empty grid that looks like an empty account.
    let needs_account = matches!(query, Query::List(_) | Query::History | Query::Favourites);
    if needs_account && !client.is_authenticated() {
        show(window, state, Vec::new(), generation, http);
        window.set_notice(sign_in_notice(window));
        return;
    }

    window.set_results_loading(true);

    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = (*client).clone();

    tasks::spawn(
        async move {
            match query {
                Query::Catalogue(filter) => api.filter(&filter, 0).await.map(|page| page.content),
                Query::List(list) => api.profile_list(list, 0, None).await.map(|p| p.content),
                Query::History => api.history(0).await.map(|page| page.content),
                Query::Favourites => api.favorites(0, None).await.map(|page| page.content),
                Query::None => Ok(Vec::new()),
            }
        },
        move |found| {
            let Some(window) = weak.upgrade() else { return };
            match found {
                Ok(releases) => show(&window, &state, releases, generation, http),
                Err(error) => {
                    tracing::error!(%error, "could not load this tab");
                    window.set_results_loading(false);
                }
            }
        },
    );
}

/// The line shown where the account's lists would be.
///
/// Read back from the interface rather than spelled out, because both
/// languages live there.
fn sign_in_notice(window: &MainWindow) -> slint::SharedString {
    if window.get_lang() == "ru" {
        "Войдите, чтобы увидеть свои списки.".into()
    } else {
        "Sign in to see your lists.".into()
    }
}

/// Moves the rail, which resets the tab: a tab index means something different
/// under each destination.
pub fn select_destination(
    window: &MainWindow,
    state: &Rc<RefCell<HomeState>>,
    client: Rc<Client>,
    http: reqwest::Client,
    index: usize,
) {
    {
        let mut state = state.borrow_mut();
        state.destination = Destination::at(index);
        state.tab = 0;
        // Genres belong to the catalogue; carrying one onto a list would
        // filter something that cannot be filtered.
        if !state.destination.has_genres() {
            state.genre = 0;
        }
    }
    window.set_query("".into());
    open(window, state, client, http);
}

/// Opens one of the account's own lists by name, from wherever the viewer is.
///
/// The profile screen counts what is in each of them, and a count is only
/// worth stating if it can be followed. This is the destination and the tab in
/// one move, so the list behind it is fetched once rather than twice.
pub fn open_list(
    window: &MainWindow,
    state: &Rc<RefCell<HomeState>>,
    client: Rc<Client>,
    http: reqwest::Client,
    tab: usize,
) {
    {
        let mut state = state.borrow_mut();
        state.destination = Destination::Saved;
        state.tab = tab.min(Destination::Saved.tabs().saturating_sub(1));
        state.genre = 0;
    }
    window.set_query("".into());
    open(window, state, client, http);
}

/// Switches tab within the current destination.
pub fn select_tab(
    window: &MainWindow,
    state: &Rc<RefCell<HomeState>>,
    client: Rc<Client>,
    http: reqwest::Client,
    index: usize,
) {
    {
        let mut state = state.borrow_mut();
        let tabs = state.destination.tabs();
        state.tab = index.min(tabs.saturating_sub(1));
    }
    window.set_query("".into());
    open(window, state, client, http);
}

/// Narrows the catalogue by genre, or widens it back to all of them.
pub fn select_genre(
    window: &MainWindow,
    state: &Rc<RefCell<HomeState>>,
    client: Rc<Client>,
    http: reqwest::Client,
    index: usize,
) {
    state.borrow_mut().genre = index.min(GENRES.len());
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
