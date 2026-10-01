// SPDX-License-Identifier: GPL-3.0-or-later

//! The comment thread in the sheet along the window's right edge.
//!
//! One thread at a time, on whatever opened it — a release, a post. The
//! thread is a flat list: each top-level comment, and under it its replies
//! once they are asked for, marked as replies. The sheet shows it as a
//! thread by indenting; this module keeps track of which row is which.
//!
//! Everything the account does here — vote, write, edit, delete — changes the
//! screen first and is put back if the server refuses, as elsewhere.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, Model, VecModel};

use anirust_api::{Client, Comment, CommentSort, CommentTarget, CommentVote, Error};

use crate::{CommentItem, MainWindow, tasks};

/// One row of the thread, and what it was made from.
#[derive(Clone)]
struct Row {
    comment: Comment,
    /// The top-level comment a reply answers; `None` for top-level rows.
    parent: Option<i64>,
    /// Top-level rows only: whether its replies are shown under it.
    expanded: bool,
    /// Whether a spoiler has been opened.
    revealed: bool,
}

/// The thread on screen.
#[derive(Default)]
pub struct CommentsState {
    target: Option<(CommentTarget, i64)>,
    sort: CommentSort,
    /// The last page of top-level comments fetched, and whether there is
    /// another after it.
    page: i32,
    has_more: bool,
    rows: Vec<Row>,
    /// Bumped whenever the thread is replaced, so a slow page for the last one
    /// cannot land in this one.
    generation: u64,
    /// What the comment being written does: answers a row, or replaces the
    /// text of one.
    reply_to: Option<usize>,
    editing: Option<usize>,
    model: Option<Rc<VecModel<CommentItem>>>,
}

impl CommentsState {
    fn row_id(&self, index: usize) -> Option<i64> {
        self.rows.get(index).map(|row| row.comment.id)
    }
}

/// What the screen needs to know about the account to draw the thread.
pub struct Viewer {
    pub profile_id: i64,
    pub http: reqwest::Client,
}

/// Opens the thread on a release or a post.
pub fn open(
    window: &MainWindow,
    state: &Rc<RefCell<CommentsState>>,
    client: &Client,
    viewer: Viewer,
    target: CommentTarget,
    id: i64,
    title: &str,
) {
    {
        let mut state = state.borrow_mut();
        state.target = Some((target, id));
        state.rows.clear();
        state.page = 0;
        state.has_more = false;
        state.reply_to = None;
        state.editing = None;
        state.generation += 1;
    }
    window.set_comments_title(title.into());
    window.set_comments_total(0);
    window.set_comments_error("".into());
    window.set_comment_draft("".into());
    window.set_comment_draft_spoiler(false);
    show_composer(window, &state.borrow());
    window.set_comments_open(true);
    redraw(window, state, &viewer);
    fetch_page(window, state, client, viewer, 0);
}

/// Closes the sheet. What was typed is kept until another thread is opened,
/// so closing it by accident loses nothing.
pub fn close(window: &MainWindow) {
    window.set_comments_open(false);
}

/// Orders the thread differently, starting again from the first page.
pub fn select_sort(
    window: &MainWindow,
    state: &Rc<RefCell<CommentsState>>,
    client: &Client,
    viewer: Viewer,
    index: i32,
) {
    let sort = sort_at(index);
    {
        let mut state = state.borrow_mut();
        if state.sort == sort {
            return;
        }
        state.sort = sort;
        state.rows.clear();
        state.page = 0;
        state.generation += 1;
    }
    window.set_comments_sort(index);
    redraw(window, state, &viewer);
    fetch_page(window, state, client, viewer, 0);
}

/// Fetches the page after the last one.
pub fn load_more(
    window: &MainWindow,
    state: &Rc<RefCell<CommentsState>>,
    client: &Client,
    viewer: Viewer,
) {
    let next = {
        let state = state.borrow();
        if !state.has_more {
            return;
        }
        state.page + 1
    };
    fetch_page(window, state, client, viewer, next);
}

fn fetch_page(
    window: &MainWindow,
    state: &Rc<RefCell<CommentsState>>,
    client: &Client,
    viewer: Viewer,
    page: i32,
) {
    let Some((target, id)) = state.borrow().target else {
        return;
    };
    let (sort, generation) = {
        let state = state.borrow();
        (state.sort, state.generation)
    };
    window.set_comments_loading(true);

    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = client.clone();
    tasks::spawn(
        async move { api.comments(target, id, page, sort).await },
        move |result| {
            let Some(window) = weak.upgrade() else { return };
            if state.borrow().generation != generation {
                return;
            }
            window.set_comments_loading(false);
            match result {
                Ok(page_of) => {
                    {
                        let mut state = state.borrow_mut();
                        state.page = page;
                        state.has_more = page_of.has_next();
                        state
                            .rows
                            .extend(page_of.content.into_iter().map(|comment| Row {
                                comment,
                                parent: None,
                                expanded: false,
                                revealed: false,
                            }));
                    }
                    window
                        .set_comments_total(i32::try_from(page_of.total_count).unwrap_or(i32::MAX));
                    redraw(&window, &state, &viewer);
                }
                Err(error) => {
                    tracing::warn!(%error, "the comments could not be loaded");
                    window.set_comments_error(error.to_string().into());
                }
            }
        },
    );
}

/// Shows or hides the replies under a top-level comment.
pub fn toggle_replies(
    window: &MainWindow,
    state: &Rc<RefCell<CommentsState>>,
    client: &Client,
    viewer: Viewer,
    index: usize,
) {
    let (target, comment_id, expanded) = {
        let state = state.borrow();
        let Some((target, _)) = state.target else {
            return;
        };
        let Some(row) = state.rows.get(index).filter(|row| row.parent.is_none()) else {
            return;
        };
        (target, row.comment.id, row.expanded)
    };

    if expanded {
        let mut guard = state.borrow_mut();
        guard.rows.retain(|row| row.parent != Some(comment_id));
        if let Some(row) = guard
            .rows
            .iter_mut()
            .find(|row| row.comment.id == comment_id)
        {
            row.expanded = false;
        }
        guard.reply_to = None;
        guard.editing = None;
        drop(guard);
        show_composer(window, &state.borrow());
        redraw(window, state, &viewer);
        return;
    }

    let generation = state.borrow().generation;
    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = client.clone();
    // Oldest first, as a conversation is read; the thread's own order is for
    // choosing which conversations to read.
    tasks::spawn(
        async move {
            api.comment_replies(target, comment_id, 0, CommentSort::Oldest)
                .await
        },
        move |result| {
            let Some(window) = weak.upgrade() else { return };
            let replies = match result {
                Ok(page) => page.content,
                Err(error) => {
                    tracing::warn!(%error, comment_id, "the replies could not be loaded");
                    return;
                }
            };
            {
                let mut state = state.borrow_mut();
                if state.generation != generation {
                    return;
                }
                let Some(at) = state
                    .rows
                    .iter()
                    .position(|row| row.comment.id == comment_id)
                else {
                    return;
                };
                state.rows[at].expanded = true;
                let rows: Vec<Row> = replies
                    .into_iter()
                    .map(|comment| Row {
                        comment,
                        parent: Some(comment_id),
                        expanded: false,
                        revealed: false,
                    })
                    .collect();
                state.rows.splice(at + 1..at + 1, rows);
                // Indices past this point moved; a reply or edit in progress
                // would now point at the wrong row.
                state.reply_to = None;
                state.editing = None;
            }
            show_composer(&window, &state.borrow());
            redraw(&window, &state, &viewer);
        },
    );
}

/// Opens a spoiler. Kept on the row, so it stays open.
pub fn reveal(
    window: &MainWindow,
    state: &Rc<RefCell<CommentsState>>,
    viewer: Viewer,
    index: usize,
) {
    if let Some(row) = state.borrow_mut().rows.get_mut(index) {
        row.revealed = true;
    }
    redraw(window, state, &viewer);
}

/// Votes a comment up or down, or withdraws the vote.
pub fn vote(
    window: &MainWindow,
    state: &Rc<RefCell<CommentsState>>,
    client: &Client,
    viewer: Viewer,
    index: usize,
    vote: i32,
) {
    let Some((target, _)) = state.borrow().target else {
        return;
    };
    let Some((comment_id, before_vote, before_score)) = state
        .borrow()
        .rows
        .get(index)
        .map(|row| (row.comment.id, row.comment.vote, row.comment.vote_count))
    else {
        return;
    };
    let after = vote.clamp(0, 2);
    if after == before_vote {
        return;
    }

    set_vote(
        state,
        comment_id,
        after,
        rescore(before_score, before_vote, after),
    );
    redraw(window, state, &viewer);

    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = client.clone();
    let cast = match after {
        2 => CommentVote::Up,
        1 => CommentVote::Down,
        _ => CommentVote::None,
    };
    tasks::spawn(
        async move { api.comment_vote(target, comment_id, cast).await },
        move |result| {
            let Err(error) = result else { return };
            tracing::warn!(%error, comment_id, "the vote was not counted");
            let Some(window) = weak.upgrade() else { return };
            set_vote(&state, comment_id, before_vote, before_score);
            redraw(&window, &state, &viewer);
        },
    );
}

fn set_vote(state: &Rc<RefCell<CommentsState>>, comment_id: i64, vote: i32, score: i64) {
    if let Some(row) = state
        .borrow_mut()
        .rows
        .iter_mut()
        .find(|row| row.comment.id == comment_id)
    {
        row.comment.vote = vote;
        row.comment.vote_count = score;
    }
}

/// The score after a vote changes from `before` to `after`: an up vote is
/// worth +1, a down vote −1, and changing one into the other moves it by two.
fn rescore(score: i64, before: i32, after: i32) -> i64 {
    let worth = |vote: i32| match vote {
        2 => 1,
        1 => -1,
        _ => 0,
    };
    score - worth(before) + worth(after)
}

/// Starts a reply to a row.
pub fn reply(window: &MainWindow, state: &Rc<RefCell<CommentsState>>, index: usize) {
    {
        let mut state = state.borrow_mut();
        if index >= state.rows.len() {
            return;
        }
        state.reply_to = Some(index);
        state.editing = None;
    }
    show_composer(window, &state.borrow());
}

/// Starts editing the account's own comment: its text goes into the composer.
pub fn edit(window: &MainWindow, state: &Rc<RefCell<CommentsState>>, index: usize) {
    let (text, spoiler) = {
        let mut state = state.borrow_mut();
        let Some(row) = state.rows.get(index) else {
            return;
        };
        let found = (row.comment.message.clone(), row.comment.is_spoiler);
        state.editing = Some(index);
        state.reply_to = None;
        found
    };
    window.set_comment_draft(text.into());
    window.set_comment_draft_spoiler(spoiler);
    show_composer(window, &state.borrow());
}

/// Drops a reply or an edit in progress, keeping what was typed for a new
/// comment — except an edit's text, which was the old comment's, not theirs.
pub fn cancel(window: &MainWindow, state: &Rc<RefCell<CommentsState>>) {
    let was_editing = {
        let mut state = state.borrow_mut();
        state.reply_to = None;
        state.editing.take().is_some()
    };
    if was_editing {
        window.set_comment_draft("".into());
        window.set_comment_draft_spoiler(false);
    }
    show_composer(window, &state.borrow());
}

/// Sends what is in the composer: a new comment, a reply, or an edit.
pub fn send(
    window: &MainWindow,
    state: &Rc<RefCell<CommentsState>>,
    client: &Client,
    viewer: Viewer,
) {
    let message = window.get_comment_draft().trim().to_owned();
    if message.is_empty() {
        return;
    }
    let spoiler = window.get_comment_draft_spoiler();
    let Some((target, id)) = state.borrow().target else {
        return;
    };
    let (reply_to, editing) = {
        let state = state.borrow();
        let reply_to = state.reply_to.and_then(|at| {
            let row = state.rows.get(at)?;
            // A reply to a reply is filed under the top-level comment, which
            // is the only level the server threads; it still names whom it
            // answers.
            Some((row.parent.unwrap_or(row.comment.id), row.comment.profile.id))
        });
        let editing = state.editing.and_then(|at| state.row_id(at));
        (reply_to, editing)
    };

    window.set_comments_sending(true);
    window.set_comments_error("".into());
    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = client.clone();

    tasks::spawn(
        async move {
            match editing {
                Some(comment_id) => api
                    .comment_edit(target, comment_id, &message, spoiler)
                    .await
                    .map(|()| Sent::Edited {
                        comment_id,
                        message,
                        spoiler,
                    }),
                None => api
                    .comment_add(target, id, &message, spoiler, reply_to)
                    .await
                    .map(|comment| Sent::Added {
                        comment: Box::new(comment),
                        parent: reply_to.map(|(p, _)| p),
                    }),
            }
        },
        move |result| {
            let Some(window) = weak.upgrade() else { return };
            window.set_comments_sending(false);
            match result {
                Ok(sent) => {
                    apply_sent(&state, sent);
                    {
                        let mut state = state.borrow_mut();
                        state.reply_to = None;
                        state.editing = None;
                    }
                    window.set_comment_draft("".into());
                    window.set_comment_draft_spoiler(false);
                    window.set_comments_total(
                        window.get_comments_total() + i32::from(editing.is_none()),
                    );
                    show_composer(&window, &state.borrow());
                    redraw(&window, &state, &viewer);
                }
                // The text stays in the composer: a refusal is a reason to
                // change it, not to type it again.
                Err(error) => {
                    tracing::info!(%error, "the comment was refused");
                    window.set_comments_error(refusal(&error, window.get_lang().as_str()).into());
                }
            }
        },
    );
}

enum Sent {
    Added {
        comment: Box<Comment>,
        parent: Option<i64>,
    },
    Edited {
        comment_id: i64,
        message: String,
        spoiler: bool,
    },
}

fn apply_sent(state: &Rc<RefCell<CommentsState>>, sent: Sent) {
    let mut state = state.borrow_mut();
    match sent {
        // A new comment goes to the top, where its writer is looking; a reply
        // goes at the end of its parent's replies, opened if they were not.
        Sent::Added {
            comment,
            parent: None,
        } => state.rows.insert(
            0,
            Row {
                comment: *comment,
                parent: None,
                expanded: false,
                revealed: true,
            },
        ),
        Sent::Added {
            comment,
            parent: Some(parent),
        } => {
            let Some(at) = state.rows.iter().position(|row| row.comment.id == parent) else {
                return;
            };
            state.rows[at].comment.reply_count += 1;
            state.rows[at].expanded = true;
            let end = state.rows[at + 1..]
                .iter()
                .position(|row| row.parent != Some(parent))
                .map_or(state.rows.len(), |offset| at + 1 + offset);
            state.rows.insert(
                end,
                Row {
                    comment: *comment,
                    parent: Some(parent),
                    expanded: false,
                    revealed: true,
                },
            );
        }
        Sent::Edited {
            comment_id,
            message,
            spoiler,
        } => {
            if let Some(row) = state
                .rows
                .iter_mut()
                .find(|row| row.comment.id == comment_id)
            {
                row.comment.message = message;
                row.comment.is_spoiler = spoiler;
                row.comment.is_edited = true;
                row.revealed = true;
            }
        }
    }
}

/// Deletes the account's own comment. It stays in the thread as "deleted",
/// so the replies under it keep their place.
pub fn delete(
    window: &MainWindow,
    state: &Rc<RefCell<CommentsState>>,
    client: &Client,
    viewer: Viewer,
    index: usize,
) {
    let Some((target, _)) = state.borrow().target else {
        return;
    };
    let Some(comment_id) = state.borrow().row_id(index) else {
        return;
    };

    mark_deleted(state, comment_id, true);
    redraw(window, state, &viewer);

    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = client.clone();
    tasks::spawn(
        async move { api.comment_delete(target, comment_id).await },
        move |result| {
            let Err(error) = result else { return };
            tracing::warn!(%error, comment_id, "the comment was not deleted");
            let Some(window) = weak.upgrade() else { return };
            mark_deleted(&state, comment_id, false);
            redraw(&window, &state, &viewer);
        },
    );
}

fn mark_deleted(state: &Rc<RefCell<CommentsState>>, comment_id: i64, deleted: bool) {
    if let Some(row) = state
        .borrow_mut()
        .rows
        .iter_mut()
        .find(|row| row.comment.id == comment_id)
    {
        row.comment.is_deleted = deleted;
    }
}

/// Says why a comment was refused, in the viewer's language.
///
/// The codes are `comment/add`'s: 5 too short, 6 too long, 7 a limit reached,
/// 8 blocked by the person answered. `comment/edit` numbers the first two
/// differently (3 and 4), so those are matched by name rather than number.
fn refusal(error: &Error, lang: &str) -> String {
    let ru = lang == "ru";
    let code = match error {
        Error::Api { code } => Some(code.raw()),
        _ => None,
    };
    match code {
        Some(5 | 3) => {
            if ru {
                "Слишком короткий комментарий."
            } else {
                "That comment is too short."
            }
        }
        Some(6 | 4) => {
            if ru {
                "Слишком длинный комментарий."
            } else {
                "That comment is too long."
            }
        }
        Some(7) => {
            if ru {
                "Слишком много комментариев подряд — попробуйте позже."
            } else {
                "Too many comments in a row — try again later."
            }
        }
        Some(8) => {
            if ru {
                "Этот пользователь вас заблокировал."
            } else {
                "This person has blocked you."
            }
        }
        _ => {
            if ru {
                "Комментарий не отправлен. Попробуйте ещё раз."
            } else {
                "The comment was not sent. Try again."
            }
        }
    }
    .to_owned()
}

/// What the composer says it is doing: answering someone, or editing.
fn show_composer(window: &MainWindow, state: &CommentsState) {
    let reply_to = state
        .reply_to
        .and_then(|at| state.rows.get(at))
        .map(|row| row.comment.profile.login.clone())
        .unwrap_or_default();
    window.set_comments_reply_to(reply_to.into());
    window.set_comments_editing(state.editing.is_some());
}

/// Puts the rows on screen and fetches the faces that are not there yet.
fn redraw(window: &MainWindow, state: &Rc<RefCell<CommentsState>>, viewer: &Viewer) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        });

    let (items, avatars, generation, has_more, kept) = {
        let state = state.borrow();
        // Faces already fetched are carried over by comment, so a redraw does
        // not refetch every picture in the thread.
        let kept: Vec<(i64, slint::Image)> = state
            .model
            .as_ref()
            .map(|model| {
                model
                    .iter()
                    .zip(&state.rows)
                    .filter(|(item, _)| item.avatar_loaded)
                    .map(|(item, row)| (row.comment.profile.id, item.avatar))
                    .collect()
            })
            .unwrap_or_default();
        let items: Vec<CommentItem> = state
            .rows
            .iter()
            .map(|row| item_for(row, viewer.profile_id, now))
            .collect();
        let avatars: Vec<String> = state
            .rows
            .iter()
            .map(|row| row.comment.profile.avatar.clone())
            .collect();
        (items, avatars, state.generation, state.has_more, kept)
    };

    let model = Rc::new(VecModel::from(items));
    window.set_comments(slint::ModelRc::from(Rc::clone(&model)));
    window.set_comments_has_more(has_more);
    state.borrow_mut().model = Some(Rc::clone(&model));

    for (index, url) in avatars.into_iter().enumerate() {
        let author = state
            .borrow()
            .rows
            .get(index)
            .map(|row| row.comment.profile.id);
        if let Some(image) = author
            .and_then(|id| kept.iter().find(|(k, _)| *k == id))
            .map(|(_, image)| image.clone())
        {
            if let Some(mut item) = model.row_data(index) {
                item.avatar = image;
                item.avatar_loaded = true;
                model.set_row_data(index, item);
            }
            continue;
        }
        if !url.starts_with("http") {
            continue;
        }
        let model = Rc::clone(&model);
        let state = Rc::clone(state);
        tasks::spawn(
            tasks::fetch_image(viewer.http.clone(), url),
            move |result| {
                if state.borrow().generation != generation {
                    return;
                }
                let Ok(buffer) = result else { return };
                if let Some(mut item) = model.row_data(index) {
                    item.avatar = slint::Image::from_rgba8(buffer);
                    item.avatar_loaded = true;
                    model.set_row_data(index, item);
                }
            },
        );
    }
}

fn item_for(row: &Row, me: i64, now: i64) -> CommentItem {
    let comment = &row.comment;
    CommentItem {
        author: comment.profile.login.as_str().into(),
        avatar: slint::Image::default(),
        avatar_loaded: false,
        minutes_ago: crate::session::minutes_since(comment.timestamp, now),
        text: comment.message.as_str().into(),
        spoiler: comment.is_spoiler,
        revealed: row.revealed,
        edited: comment.is_edited,
        deleted: comment.is_deleted,
        score: i32::try_from(comment.vote_count).unwrap_or(0),
        my_vote: comment.vote,
        replies: i32::try_from(comment.reply_count).unwrap_or(i32::MAX),
        expanded: row.expanded,
        is_reply: row.parent.is_some(),
        mine: me > 0 && comment.profile.id == me,
        episode: comment.posted_at_episode.unwrap_or(0),
    }
}

/// The sort menu's order: newest, oldest, popular.
fn sort_at(index: i32) -> CommentSort {
    match index {
        1 => CommentSort::Oldest,
        2 => CommentSort::Popular,
        _ => CommentSort::Newest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changing_a_vote_moves_the_score_by_what_it_was_worth() {
        assert_eq!(rescore(10, 0, 2), 11, "a new up vote");
        assert_eq!(rescore(10, 2, 0), 9, "withdrawn");
        assert_eq!(rescore(10, 2, 1), 8, "up turned down");
        assert_eq!(rescore(10, 1, 2), 12, "down turned up");
    }

    #[test]
    fn the_sort_menu_reads_in_its_own_order() {
        assert_eq!(sort_at(0), CommentSort::Newest);
        assert_eq!(sort_at(1), CommentSort::Oldest);
        assert_eq!(sort_at(2), CommentSort::Popular);
        assert_eq!(sort_at(9), CommentSort::Newest);
    }

    #[test]
    fn a_reply_goes_after_its_siblings_and_opens_them() {
        let state = Rc::new(RefCell::new(CommentsState::default()));
        let row = |id: i64, parent: Option<i64>| Row {
            comment: Comment {
                id,
                ..Comment::default()
            },
            parent,
            expanded: parent.is_none() && id == 1,
            revealed: false,
        };
        state.borrow_mut().rows = vec![
            row(1, None),
            row(10, Some(1)),
            row(11, Some(1)),
            row(2, None),
        ];

        apply_sent(
            &state,
            Sent::Added {
                comment: Box::new(Comment {
                    id: 12,
                    ..Comment::default()
                }),
                parent: Some(1),
            },
        );
        let ids: Vec<i64> = state.borrow().rows.iter().map(|r| r.comment.id).collect();
        assert_eq!(ids, [1, 10, 11, 12, 2]);
        assert_eq!(state.borrow().rows[0].comment.reply_count, 1);
    }

    #[test]
    fn a_new_comment_goes_to_the_top() {
        let state = Rc::new(RefCell::new(CommentsState::default()));
        state.borrow_mut().rows = vec![Row {
            comment: Comment {
                id: 1,
                ..Comment::default()
            },
            parent: None,
            expanded: false,
            revealed: false,
        }];
        apply_sent(
            &state,
            Sent::Added {
                comment: Box::new(Comment {
                    id: 2,
                    ..Comment::default()
                }),
                parent: None,
            },
        );
        assert_eq!(state.borrow().rows[0].comment.id, 2);
    }
}
