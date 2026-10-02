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

use anirust_api::{
    Client, Collection, CollectionSort, Filter, FilterSort, ProfileList, Release, SearchBy,
};

use crate::session::Session;
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
    /// Posts from channels. Not a grid of releases, so it has a page of its
    /// own and fetches through `feed`, not through here.
    Feed,
    Profile,
    Downloads,
}

impl Destination {
    /// The order the rail puts them in, and the order Rust and the interface
    /// both count by. The first five are the official client's own, in its
    /// order. Downloads is last and off the end of the rail: that client has
    /// no such destination, and a queue is reached from the toolbar instead.
    const ALL: [Self; 6] = [
        Self::Home,
        Self::Browse,
        Self::Saved,
        Self::Feed,
        Self::Profile,
        Self::Downloads,
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
            Self::Home => HOME_TABS,
            Self::Browse => 5,
            Self::Saved => 8,
            Self::Feed | Self::Profile | Self::Downloads => 0,
        }
    }

    /// Whether the genre chips apply here. Only the catalogue is a filter.
    fn has_genres(self) -> bool {
        self == Self::Browse
    }
}

/// The home destination's tabs, in their order on screen.
///
/// The first three are the catalogue cut three ways; the rest are what the
/// official client's front page offers: its curated cards, recommendations
/// for this account, what is being watched and discussed, and the week's
/// schedule.
const HOME_TABS: usize = 8;
const TAB_INTERESTING: usize = 3;
const TAB_RECOMMENDED: usize = 4;
const TAB_WATCHING: usize = 5;
const TAB_DISCUSSING: usize = 6;
pub const TAB_SCHEDULE: usize = 7;

/// The schedule's day chips: 0 the whole week, then Monday to Sunday.
pub const WEEKDAYS_RU: [&str; 7] = ["пн", "вт", "ср", "чт", "пт", "сб", "вс"];
pub const WEEKDAYS_EN: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// What one tab fetches.
enum Query {
    /// The catalogue, filtered.
    Catalogue(Box<Filter>),
    /// One of the account's lists.
    List(ProfileList),
    History,
    Favourites,
    /// The front page's curated cards, each standing for a release.
    Interesting,
    /// Recommendations for this account.
    Recommended,
    Watching,
    Discussing,
    /// The week's schedule, or one day of it: 0 the whole week, 1 Monday.
    Schedule(usize),
    /// Everybody's collections, the popular among recent ones first.
    Collections,
    /// The account's own collections, then those it favourited.
    MyCollections,
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
        (Destination::Home, 2) => Query::Catalogue(Box::new(
            Filter::sorted_by(FilterSort::Popularity).status(FINISHED),
        )),
        (Destination::Home, TAB_INTERESTING) => Query::Interesting,
        (Destination::Home, TAB_RECOMMENDED) => Query::Recommended,
        (Destination::Home, TAB_WATCHING) => Query::Watching,
        (Destination::Home, TAB_DISCUSSING) => Query::Discussing,
        (Destination::Home, _) => Query::Schedule(0),

        (Destination::Browse, 0) => {
            Query::Catalogue(Box::new(Filter::sorted_by(FilterSort::Popularity)))
        }
        (Destination::Browse, 1) => {
            Query::Catalogue(Box::new(Filter::sorted_by(FilterSort::Rating)))
        }
        (Destination::Browse, 2) => Query::Catalogue(Box::new(
            Filter::sorted_by(FilterSort::Popularity).category(FILM),
        )),
        (Destination::Browse, 3) => Query::Catalogue(Box::new(
            Filter::sorted_by(FilterSort::Year).status(ANNOUNCED),
        )),
        (Destination::Browse, _) => Query::Collections,

        (Destination::Saved, 5) => Query::History,
        (Destination::Saved, 6) => Query::Favourites,
        (Destination::Saved, 7) => Query::MyCollections,
        (Destination::Saved, tab) => Query::List(
            ProfileList::ALL
                .get(tab)
                .copied()
                .unwrap_or(ProfileList::Watching),
        ),

        (Destination::Feed | Destination::Profile | Destination::Downloads, _) => Query::None,
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
    /// The collections behind the cards, when the grid is of collections
    /// rather than releases.
    collections: Vec<Collection>,
    /// Who is signed in, for the tab of the account's own collections.
    account: Rc<RefCell<Session>>,
}

impl HomeState {
    #[must_use]
    pub fn new(account: Rc<RefCell<Session>>) -> Self {
        Self {
            account,
            ..Self::default()
        }
    }

    pub(crate) fn next_generation(&mut self) -> u64 {
        self.generation += 1;
        self.generation
    }

    /// The collection a card stands for, when the grid is of collections.
    #[must_use]
    pub fn collection_at(&self, index: usize) -> Option<&Collection> {
        self.collections.get(index)
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
    window.set_collection_open(false);

    let (destination, tab, genre, generation) = {
        let mut state = state.borrow_mut();
        let generation = state.next_generation();
        (state.destination, state.tab, state.genre, generation)
    };

    let schedule = destination == Destination::Home && tab == TAB_SCHEDULE;
    window.set_destination(destination.index() as i32);
    window.set_tab(tab as i32);
    window.set_genre(genre as i32);
    // The chip row is the catalogue's genres, or on the schedule the days of
    // the week — the same control choosing one of several, with "all" first.
    let chips: Vec<slint::SharedString> = if destination.has_genres() {
        GENRES.iter().map(|name| (*name).into()).collect()
    } else if schedule {
        let days = if window.get_lang() == "ru" {
            WEEKDAYS_RU
        } else {
            WEEKDAYS_EN
        };
        days.iter().map(|name| (*name).into()).collect()
    } else {
        Vec::new()
    };
    window.set_genres(slint::ModelRc::new(VecModel::from(chips)));
    window.set_chips_are_days(schedule);

    let mut query = query_for(destination, tab);
    if let Query::Schedule(day) = &mut query {
        *day = genre;
    }
    if let Query::Catalogue(filter) = &mut query
        && let Some(name) = genre.checked_sub(1).and_then(|at| GENRES.get(at))
    {
        **filter = std::mem::take(filter.as_mut()).genre(*name);
    }

    // The account's own lists are the only thing here that needs a session.
    // Saying so beats an empty grid that looks like an empty account.
    let needs_account = matches!(
        query,
        Query::List(_)
            | Query::History
            | Query::Favourites
            | Query::Recommended
            | Query::MyCollections
    );
    if needs_account && !client.is_authenticated() {
        show(window, state, Vec::new(), generation, http);
        window.set_notice(sign_in_notice(window));
        return;
    }

    window.set_results_loading(true);

    if matches!(query, Query::Collections | Query::MyCollections) {
        let me = state.borrow().account.borrow().id;
        load_collections(window, state, &client, http, generation, me);
        return;
    }

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
                Query::Interesting => api.discover_interesting().await.map(releases_of_cards),
                Query::Recommended => api
                    .discover_recommendations(0, 0)
                    .await
                    .map(|page| page.content),
                Query::Watching => api.discover_watching(0).await.map(|page| page.content),
                Query::Discussing => api.discover_discussing().await.map(|page| page.content),
                Query::Schedule(day) => api.schedule().await.map(|week| {
                    let days = week.days();
                    match day.checked_sub(1).and_then(|at| days.get(at)) {
                        Some(one) => one.to_vec(),
                        None => days.iter().flat_map(|d| d.iter().cloned()).collect(),
                    }
                }),
                Query::None | Query::Collections | Query::MyCollections => Ok(Vec::new()),
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

/// Today as a schedule chip: 1 for Monday through 7 for Sunday.
///
/// By UTC, which is a few hours off the viewer's own midnight for most of
/// them; the schedule is the service's, kept by its own calendar, and a day
/// that turns over a little late in the evening is the smaller error than
/// guessing a time zone.
fn today() -> usize {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() / 86_400);
    weekday(days)
}

/// The weekday of a day count since 1970-01-01, which was a Thursday:
/// 1 Monday through 7 Sunday.
fn weekday(days_since_epoch: u64) -> usize {
    // Thursday is day 4 of a Monday-first week.
    usize::try_from((days_since_epoch + 3) % 7).unwrap_or(0) + 1
}

/// The front page's curated cards as releases: each card leads to a release,
/// and the card's own picture and line stand in for the release's until it is
/// opened. Cards of a kind that does not lead to a release are left out.
fn releases_of_cards(cards: Vec<anirust_api::Interesting>) -> Vec<Release> {
    cards
        .into_iter()
        .filter(|card| !card.is_hidden)
        .filter_map(|card| {
            Some(Release {
                id: card.release_id()?,
                title_ru: card.title,
                description: card.description,
                image: card.image,
                ..Release::default()
            })
        })
        .collect()
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
        // The chips mean genres on one tab and days on another, so a choice
        // made under one is not carried to the other. The schedule opens on
        // today, which is the day anyone opening it is asking about.
        if state.destination == Destination::Home {
            state.genre = if state.tab == TAB_SCHEDULE {
                today()
            } else {
                0
            };
        }
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
    // Genres on the catalogue, days of the week on the schedule.
    state.borrow_mut().genre = index.min(GENRES.len().max(WEEKDAYS_RU.len()));
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
pub(crate) fn show(
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
        state.collections.clear();
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

/// Collections as a grid: everybody's, or the account's own followed by
/// those it favourited.
fn load_collections(
    window: &MainWindow,
    state: &Rc<RefCell<HomeState>>,
    client: &Client,
    http: reqwest::Client,
    generation: u64,
    me: i64,
) {
    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = client.clone();
    tasks::spawn(
        async move {
            if me <= 0 {
                return api
                    .collections(0, CollectionSort::Trending)
                    .await
                    .map(|page| page.content);
            }
            let mut mine = api.profile_collections(me, 0).await?.content;
            let favourites = api.favorite_collections(0).await.map(|p| p.content);
            for collection in favourites.unwrap_or_default() {
                if mine.iter().all(|c| c.id != collection.id) {
                    mine.push(collection);
                }
            }
            Ok(mine)
        },
        move |found| {
            let Some(window) = weak.upgrade() else { return };
            if state.borrow().generation != generation {
                return;
            }
            match found {
                Ok(collections) => show_collections(&window, &state, collections, generation, http),
                Err(error) => {
                    tracing::error!(%error, "could not load the collections");
                    window.set_results_loading(false);
                }
            }
        },
    );
}

fn show_collections(
    window: &MainWindow,
    state: &Rc<RefCell<HomeState>>,
    collections: Vec<Collection>,
    generation: u64,
    http: reqwest::Client,
) {
    let cards: Vec<ReleaseCard> = collections
        .iter()
        .map(|c| ReleaseCard {
            title: c.title.as_str().into(),
            subtitle: collection_subtitle(c).into(),
            score: slint::SharedString::default(),
            poster: slint::Image::default(),
            poster_loaded: false,
        })
        .collect();
    let model = Rc::new(VecModel::from(cards));
    window.set_results(slint::ModelRc::from(Rc::clone(&model)));
    window.set_results_loading(false);

    let images: Vec<String> = collections.iter().map(|c| c.image.clone()).collect();
    {
        let mut state = state.borrow_mut();
        state.releases.clear();
        state.collections = collections;
        state.cards = Some(Rc::clone(&model));
    }
    for (index, url) in images.into_iter().enumerate() {
        load_poster(state, &model, index, generation, http.clone(), url);
    }
}

/// Who made it and how many keep it, in marks rather than words, so the line
/// reads the same in either language.
fn collection_subtitle(collection: &Collection) -> String {
    match &collection.creator {
        Some(creator) if !creator.login.is_empty() => {
            format!("{} · ♥ {}", creator.login, collection.favorites_count)
        }
        _ => format!("♥ {}", collection.favorites_count),
    }
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

    #[test]
    fn the_epoch_was_a_thursday() {
        assert_eq!(weekday(0), 4);
        assert_eq!(weekday(4), 1, "1970-01-05 was a Monday");
        assert_eq!(weekday(20_363), 4, "2025-10-02 was a Thursday");
    }

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
