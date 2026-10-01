// SPDX-License-Identifier: GPL-3.0-or-later

//! The feed: posts from channels.
//!
//! Its own module rather than another destination in `home`, because nothing
//! about it is a grid of releases — the posts carry text, a picture and a
//! channel that can be followed, and following one changes every post from
//! it at once.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, Model, VecModel};

use anirust_api::{Article, Client, CommentVote, Page};

use crate::{FeedPost, MainWindow, tasks};

/// Which of the two feeds is up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    /// The channels the account follows.
    #[default]
    Mine,
    /// The newest posts from every channel.
    Latest,
}

impl Tab {
    #[must_use]
    pub fn at(index: i32) -> Self {
        if index == 1 { Self::Latest } else { Self::Mine }
    }

    fn index(self) -> i32 {
        match self {
            Self::Mine => 0,
            Self::Latest => 1,
        }
    }
}

/// What the feed screen is showing, and what it was built from.
#[derive(Default)]
pub struct FeedState {
    tab: Tab,
    /// Bumped on every fetch, so a slow answer to an old question cannot
    /// overwrite the answer to a newer one.
    generation: u64,
    articles: Vec<Article>,
    posts: Option<Rc<VecModel<FeedPost>>>,
}

impl FeedState {
    fn next_generation(&mut self) -> u64 {
        self.generation += 1;
        self.generation
    }
}

/// Loads the tab that is up. Called on arriving at the feed and on switching
/// tabs; without an account there is nothing to load, and the screen says so
/// on its own.
pub fn open(
    window: &MainWindow,
    state: &Rc<RefCell<FeedState>>,
    client: &Client,
    http: reqwest::Client,
    signed_in: bool,
) {
    let (tab, generation) = {
        let mut state = state.borrow_mut();
        (state.tab, state.next_generation())
    };
    window.set_feed_tab(tab.index());
    window.set_feed_notice("".into());

    if !signed_in {
        window.set_posts(slint::ModelRc::new(VecModel::<FeedPost>::default()));
        return;
    }

    window.set_feed_loading(true);
    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = client.clone();

    tasks::spawn(
        async move {
            match tab {
                Tab::Mine => api.feed(0).await,
                Tab::Latest => api.feed_latest(0).await,
            }
        },
        move |page: anirust_api::Result<Page<Article>>| {
            let Some(window) = weak.upgrade() else { return };
            if state.borrow().generation != generation {
                return;
            }
            window.set_feed_loading(false);
            match page {
                Ok(page) => show(&window, &state, page.content, generation, http),
                Err(error) => {
                    tracing::warn!(%error, ?tab, "the feed could not be loaded");
                    window.set_feed_notice(error.to_string().into());
                }
            }
        },
    );
}

/// The post a row of the feed stands for: its id and a title for the
/// comment sheet — the channel's name, which is what heads the post.
#[must_use]
pub fn article_at(state: &Rc<RefCell<FeedState>>, index: usize) -> Option<(i64, String)> {
    let state = state.borrow();
    let article = state.articles.get(index)?;
    let name = if article.channel.is_blog && !article.author.login.is_empty() {
        article.author.login.clone()
    } else {
        article.channel.title.clone()
    };
    Some((article.id, name))
}

/// Switches between the account's own feed and the latest from everywhere.
pub fn select_tab(
    window: &MainWindow,
    state: &Rc<RefCell<FeedState>>,
    client: &Client,
    http: reqwest::Client,
    signed_in: bool,
    index: i32,
) {
    state.borrow_mut().tab = Tab::at(index);
    open(window, state, client, http, signed_in);
}

/// Follows or stops following the channel a post came from.
///
/// Every post from that channel changes at once, before the server has
/// answered: the button is under the pointer, and a second of nothing is
/// a second of wondering whether the click landed. If the server refuses,
/// the posts are put back.
pub fn toggle_subscription(
    window: &MainWindow,
    state: &Rc<RefCell<FeedState>>,
    client: &Client,
    index: i32,
) {
    let Some((channel_id, now_subscribed)) = ({
        let state = state.borrow();
        usize::try_from(index)
            .ok()
            .and_then(|at| state.articles.get(at))
            .map(|article| (article.channel.id, !article.channel.is_subscribed))
    }) else {
        return;
    };

    mark_channel(state, channel_id, now_subscribed);

    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = client.clone();
    tasks::spawn(
        async move {
            if now_subscribed {
                api.channel_subscribe(channel_id).await
            } else {
                api.channel_unsubscribe(channel_id).await
            }
        },
        move |result| {
            if weak.upgrade().is_none() {
                return;
            }
            if let Err(error) = result {
                tracing::warn!(%error, channel_id, "the subscription was not changed");
                mark_channel(&state, channel_id, !now_subscribed);
            }
        },
    );
}

/// The heart on a post: an up vote on the service's one vote scale.
const UP: i32 = 2;

/// Gives a post a heart, or takes it back. The count moves with it at once,
/// and both go back if the server refuses.
pub fn toggle_like(
    window: &MainWindow,
    state: &Rc<RefCell<FeedState>>,
    client: &Client,
    index: i32,
) {
    let Ok(at) = usize::try_from(index) else {
        return;
    };
    let Some((article_id, before_vote, before_count)) = state
        .borrow()
        .articles
        .get(at)
        .map(|a| (a.id, a.vote, a.vote_count))
    else {
        return;
    };
    let liking = before_vote != UP;
    let after_vote = if liking { UP } else { 0 };
    let after_count = recount(before_count, before_vote, after_vote);

    set_like(state, at, after_vote, after_count);

    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = client.clone();
    tasks::spawn(
        async move {
            let vote = if liking {
                CommentVote::Up
            } else {
                CommentVote::None
            };
            api.article_vote(article_id, vote).await
        },
        move |result| {
            if weak.upgrade().is_none() {
                return;
            }
            if let Err(error) = result {
                tracing::warn!(%error, article_id, "the heart was not counted");
                set_like(&state, at, before_vote, before_count);
            }
        },
    );
}

/// A post's score after the account's vote changes from `before` to `after`:
/// an up vote is worth one, a down vote minus one. A heart on a post this
/// account had voted down moves it by two.
fn recount(score: i64, before: i32, after: i32) -> i64 {
    let worth = |vote: i32| match vote {
        UP => 1,
        1 => -1,
        _ => 0,
    };
    score - worth(before) + worth(after)
}

fn set_like(state: &Rc<RefCell<FeedState>>, at: usize, vote: i32, count_now: i64) {
    let mut state = state.borrow_mut();
    let Some(article) = state.articles.get_mut(at) else {
        return;
    };
    article.vote = vote;
    article.vote_count = count_now;
    if let Some(model) = state.posts.clone()
        && let Some(mut post) = model.row_data(at)
    {
        post.liked = vote == UP;
        post.votes = count(count_now);
        model.set_row_data(at, post);
    }
}

/// Sets whether a channel is followed, on every post of it and on screen.
fn mark_channel(state: &Rc<RefCell<FeedState>>, channel_id: i64, subscribed: bool) {
    let mut state = state.borrow_mut();
    let Some(model) = state.posts.clone() else {
        return;
    };
    for (at, article) in state.articles.iter_mut().enumerate() {
        if article.channel.id != channel_id {
            continue;
        }
        article.channel.is_subscribed = subscribed;
        if let Some(mut post) = model.row_data(at) {
            post.subscribed = subscribed;
            model.set_row_data(at, post);
        }
    }
}

/// Puts a page of posts on the screen and starts fetching their pictures.
fn show(
    window: &MainWindow,
    state: &Rc<RefCell<FeedState>>,
    articles: Vec<Article>,
    generation: u64,
    http: reqwest::Client,
) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        });

    // Deleted posts arrive as tombstones; there is nothing to read in one.
    let articles: Vec<Article> = articles.into_iter().filter(|a| !a.is_deleted).collect();
    let posts: Vec<FeedPost> = articles.iter().map(|a| post_for(a, now)).collect();
    let model = Rc::new(VecModel::from(posts));
    window.set_posts(slint::ModelRc::from(Rc::clone(&model)));

    let images: Vec<(String, Option<String>)> = articles
        .iter()
        .map(|article| (avatar_of(article), article.first_image()))
        .collect();
    {
        let mut state = state.borrow_mut();
        state.articles = articles;
        state.posts = Some(Rc::clone(&model));
    }

    for (index, (avatar, picture)) in images.into_iter().enumerate() {
        load_image(
            state,
            &model,
            index,
            generation,
            http.clone(),
            avatar,
            |post, image| {
                post.channel_avatar = image;
                post.channel_avatar_loaded = true;
            },
        );
        if let Some(picture) = picture {
            load_image(
                state,
                &model,
                index,
                generation,
                http.clone(),
                picture,
                |post, image| {
                    post.picture = image;
                    post.picture_loaded = true;
                },
            );
        }
    }
}

/// One post as the screen shows it.
fn post_for(article: &Article, now: i64) -> FeedPost {
    let channel = &article.channel;
    // A personal blog is a channel named after its owner; the owner's name is
    // the one a reader knows, so that is the one shown.
    let name = if channel.is_blog && !article.author.login.is_empty() {
        article.author.login.clone()
    } else {
        channel.title.clone()
    };

    FeedPost {
        channel: name.into(),
        channel_avatar: slint::Image::default(),
        channel_avatar_loaded: false,
        subscribed: channel.is_subscribed,
        can_subscribe: !channel.is_blog && channel.id > 0,
        minutes_ago: crate::session::minutes_since(article.creation_date, now),
        text: article.plain_text().into(),
        picture: slint::Image::default(),
        picture_loaded: false,
        has_picture: article.first_image().is_some(),
        comments: count(article.comment_count),
        votes: count(article.vote_count),
        liked: article.vote == UP,
        pinned: article.is_pinned,
    }
}

/// The face beside a post: the channel's own picture, or the blog owner's.
fn avatar_of(article: &Article) -> String {
    if article.channel.avatar.is_empty() {
        article.author.avatar.clone()
    } else {
        article.channel.avatar.clone()
    }
}

/// A count as the interface carries it. No integer wider than 32 bits crosses
/// into Slint, and nothing here gets near one.
fn count(value: i64) -> i32 {
    i32::try_from(value.max(0)).unwrap_or(i32::MAX)
}

/// Fetches one picture and drops it into its post, unless the feed has moved
/// on since it was asked for.
fn load_image(
    state: &Rc<RefCell<FeedState>>,
    model: &Rc<VecModel<FeedPost>>,
    index: usize,
    generation: u64,
    http: reqwest::Client,
    url: String,
    apply: fn(&mut FeedPost, slint::Image),
) {
    if url.is_empty() || !url.starts_with("http") {
        return;
    }
    let state = Rc::clone(state);
    let model = Rc::clone(model);

    tasks::spawn(tasks::fetch_image(http, url), move |result| {
        if state.borrow().generation != generation {
            return;
        }
        let Ok(buffer) = result else { return };
        let Some(mut post) = model.row_data(index) else {
            return;
        };
        apply(&mut post, slint::Image::from_rgba8(buffer));
        model.set_row_data(index, post);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use anirust_api::{Channel, ProfileSlim};

    #[test]
    fn a_heart_moves_the_score_by_what_the_old_vote_was_worth() {
        assert_eq!(recount(10, 0, UP), 11);
        assert_eq!(recount(10, UP, 0), 9);
        assert_eq!(recount(10, 1, UP), 12, "a down vote turned into a heart");
    }

    #[test]
    fn a_tab_index_names_the_same_tab_back() {
        for tab in [Tab::Mine, Tab::Latest] {
            assert_eq!(Tab::at(tab.index()), tab);
        }
        assert_eq!(Tab::at(7), Tab::Mine);
    }

    #[test]
    fn a_blog_is_named_after_its_owner_and_cannot_be_subscribed_to() {
        let article = Article {
            channel: Channel {
                id: 5,
                title: "Блог".into(),
                is_blog: true,
                ..Channel::default()
            },
            author: ProfileSlim {
                login: "mrFrok".into(),
                ..ProfileSlim::default()
            },
            ..Article::default()
        };
        let post = post_for(&article, 0);
        assert_eq!(post.channel.as_str(), "mrFrok");
        assert!(!post.can_subscribe);
    }

    #[test]
    fn a_channel_is_named_after_itself() {
        let article = Article {
            channel: Channel {
                id: 9,
                title: "GDA | News".into(),
                is_subscribed: true,
                ..Channel::default()
            },
            ..Article::default()
        };
        let post = post_for(&article, 0);
        assert_eq!(post.channel.as_str(), "GDA | News");
        assert!(post.can_subscribe);
        assert!(post.subscribed);
    }
}
