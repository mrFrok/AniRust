// SPDX-License-Identifier: GPL-3.0-or-later

//! The bell: how many notifications are unseen, the sheet that lists them,
//! and the switches that decide what brings one.
//!
//! The count is asked for once a minute while an account is signed in, and
//! not at all otherwise. A minute because that is about how long an episode
//! takes to reach a notification anyway, and asking more often would be
//! asking a service that is not this client's for nothing new.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, VecModel};

use anirust_api::{Client, Notification, NotificationDelete, NotificationKind, NotificationSwitch};

use crate::{MainWindow, NotificationItem, tasks};

/// How often the unseen count is asked for.
const POLL: Duration = Duration::from_secs(60);

/// The sheet's menu, in its order on screen.
const KINDS: [NotificationKind; 8] = [
    NotificationKind::All,
    NotificationKind::Episodes,
    NotificationKind::ReleaseComments,
    NotificationKind::Friends,
    NotificationKind::RelatedReleases,
    NotificationKind::Articles,
    NotificationKind::ArticleComments,
    NotificationKind::CollectionComments,
];

/// The switches on the profile screen, in the order of their labels there.
const SWITCHES: [NotificationSwitch; 9] = [
    NotificationSwitch::Episodes,
    NotificationSwitch::FirstEpisode,
    NotificationSwitch::Comments,
    NotificationSwitch::RelatedReleases,
    NotificationSwitch::Articles,
    NotificationSwitch::MyArticleComments,
    NotificationSwitch::MyCollectionComments,
    NotificationSwitch::SelectedReleases,
    NotificationSwitch::ReportOutcomes,
];

/// What the sheet is showing.
#[derive(Default)]
pub struct NotificationsState {
    kind: usize,
    page: i32,
    has_more: bool,
    items: Vec<(Notification, Kind)>,
    generation: u64,
}

/// What a notification is, as far as this client can tell: from its own
/// `type` on the mixed list, or from the list it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Episode,
    Friend,
    Article,
    Reply,
    Related,
    ArticleComment,
    CollectionComment,
    Other,
}

impl Kind {
    fn of(notification: &Notification, list: NotificationKind) -> Self {
        match notification.kind.as_str() {
            "episode" => return Self::Episode,
            "friend" => return Self::Friend,
            "article" => return Self::Article,
            _ => {}
        }
        match list {
            NotificationKind::Episodes => Self::Episode,
            NotificationKind::Friends => Self::Friend,
            NotificationKind::Articles => Self::Article,
            NotificationKind::ReleaseComments => Self::Reply,
            NotificationKind::RelatedReleases => Self::Related,
            NotificationKind::ArticleComments => Self::ArticleComment,
            NotificationKind::CollectionComments => Self::CollectionComment,
            NotificationKind::All => Self::Other,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Episode => "episode",
            Self::Friend => "friend",
            Self::Article => "article",
            Self::Reply => "reply",
            Self::Related => "related",
            Self::ArticleComment => "article-comment",
            Self::CollectionComment => "collection-comment",
            Self::Other => "",
        }
    }

    /// The delete for this kind. Posts have none in the service.
    fn delete(self) -> Option<NotificationDelete> {
        Some(match self {
            Self::Episode => NotificationDelete::Episode,
            Self::Friend => NotificationDelete::Friend,
            Self::Reply => NotificationDelete::ReleaseComment,
            Self::Related => NotificationDelete::RelatedRelease,
            Self::ArticleComment => NotificationDelete::ArticleComment,
            Self::CollectionComment => NotificationDelete::CollectionComment,
            Self::Article | Self::Other => return None,
        })
    }
}

/// The release a notification is about, which is what clicking it opens.
#[must_use]
pub fn release_of(notification: &Notification) -> Option<i64> {
    notification
        .episode
        .as_ref()
        .map(|e| e.release.id)
        .or_else(|| notification.release.as_ref().map(|r| r.id))
        .or_else(|| {
            notification
                .comment
                .as_ref()?
                .release
                .as_ref()
                .map(|r| r.id)
        })
        .filter(|id| *id > 0)
}

/// Starts asking for the unseen count, every [`POLL`], for as long as the
/// window lives. The timer is returned to be kept: dropping it stops it.
#[must_use]
pub fn start_polling(window: &MainWindow, client: &Client) -> slint::Timer {
    let timer = slint::Timer::default();
    let weak = window.as_weak();
    let api = client.clone();
    refresh_count(window, client);
    timer.start(slint::TimerMode::Repeated, POLL, move || {
        if let Some(window) = weak.upgrade() {
            refresh_count(&window, &api);
        }
    });
    timer
}

/// Asks for the unseen count now. Without an account the badge is simply
/// cleared — there is nothing to ask.
pub fn refresh_count(window: &MainWindow, client: &Client) {
    if !client.is_authenticated() {
        window.set_unseen(0);
        return;
    }
    let weak = window.as_weak();
    let api = client.clone();
    tasks::spawn(
        async move { api.notification_count().await },
        move |count| {
            let Some(window) = weak.upgrade() else { return };
            match count {
                Ok(count) => window.set_unseen(i32::try_from(count).unwrap_or(i32::MAX)),
                Err(error) => tracing::debug!(%error, "the notification count was not refreshed"),
            }
        },
    );
}

/// Opens the sheet on the list it was last on.
pub fn open(window: &MainWindow, state: &Rc<RefCell<NotificationsState>>, client: &Client) {
    window.set_notifications_open(true);
    reload(window, state, client);
}

/// Switches the sheet to another list.
pub fn select_kind(
    window: &MainWindow,
    state: &Rc<RefCell<NotificationsState>>,
    client: &Client,
    index: i32,
) {
    let kind = usize::try_from(index).unwrap_or(0).min(KINDS.len() - 1);
    state.borrow_mut().kind = kind;
    reload(window, state, client);
}

fn reload(window: &MainWindow, state: &Rc<RefCell<NotificationsState>>, client: &Client) {
    {
        let mut state = state.borrow_mut();
        state.items.clear();
        state.page = 0;
        state.generation += 1;
        window.set_notification_kind(i32::try_from(state.kind).unwrap_or(0));
    }
    show(window, &state.borrow());
    fetch(window, state, client, 0);
}

/// The page after the last one.
pub fn load_more(window: &MainWindow, state: &Rc<RefCell<NotificationsState>>, client: &Client) {
    let next = {
        let state = state.borrow();
        if !state.has_more {
            return;
        }
        state.page + 1
    };
    fetch(window, state, client, next);
}

fn fetch(window: &MainWindow, state: &Rc<RefCell<NotificationsState>>, client: &Client, page: i32) {
    let (list, generation) = {
        let state = state.borrow();
        (KINDS[state.kind], state.generation)
    };
    window.set_notifications_loading(true);
    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = client.clone();
    tasks::spawn(
        async move { api.notifications(list, page).await },
        move |result| {
            let Some(window) = weak.upgrade() else { return };
            if state.borrow().generation != generation {
                return;
            }
            window.set_notifications_loading(false);
            match result {
                Ok(found) => {
                    {
                        let mut state = state.borrow_mut();
                        state.page = page;
                        state.has_more = found.has_next();
                        state.items.extend(found.content.into_iter().map(|n| {
                            let kind = Kind::of(&n, list);
                            (n, kind)
                        }));
                    }
                    show(&window, &state.borrow());
                }
                Err(error) => tracing::warn!(%error, "the notifications could not be loaded"),
            }
        },
    );
}

/// Marks everything seen: the dots go and the badge empties at once.
pub fn mark_all_seen(
    window: &MainWindow,
    state: &Rc<RefCell<NotificationsState>>,
    client: &Client,
) {
    for (notification, _) in &mut state.borrow_mut().items {
        notification.is_new = false;
    }
    show(window, &state.borrow());
    window.set_unseen(0);

    let weak = window.as_weak();
    let api = client.clone();
    let mine = client.clone();
    tasks::spawn(
        async move { api.notifications_read().await },
        move |result| {
            if let Err(error) = result {
                tracing::warn!(%error, "the notifications were not marked seen");
            }
            // Either way the badge shows what the server now says.
            if let Some(window) = weak.upgrade() {
                refresh_count(&window, &mine);
            }
        },
    );
}

/// Deletes every notification. Asked for twice by the sheet, so this does
/// not ask again.
pub fn clear_all(window: &MainWindow, state: &Rc<RefCell<NotificationsState>>, client: &Client) {
    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = client.clone();
    let mine = client.clone();
    tasks::spawn(
        async move { api.notifications_delete_all().await },
        move |result| {
            let Some(window) = weak.upgrade() else { return };
            match result {
                Ok(()) => {
                    {
                        let mut state = state.borrow_mut();
                        state.items.clear();
                        state.has_more = false;
                    }
                    show(&window, &state.borrow());
                    refresh_count(&window, &mine);
                }
                Err(error) => tracing::warn!(%error, "the notifications were not cleared"),
            }
        },
    );
}

/// Deletes one notification. It goes at once and comes back if refused.
pub fn delete(
    window: &MainWindow,
    state: &Rc<RefCell<NotificationsState>>,
    client: &Client,
    index: usize,
) {
    let Some((removed, kind)) = ({
        let mut state = state.borrow_mut();
        (index < state.items.len()).then(|| state.items.remove(index))
    }) else {
        return;
    };
    let Some(delete) = kind.delete() else {
        state.borrow_mut().items.insert(index, (removed, kind));
        return;
    };
    show(window, &state.borrow());

    let id = removed.id;
    let weak = window.as_weak();
    let state = Rc::clone(state);
    let api = client.clone();
    tasks::spawn(
        async move { api.notification_delete(delete, id).await },
        move |result| {
            let Err(error) = result else { return };
            tracing::warn!(%error, id, "the notification was not deleted");
            let Some(window) = weak.upgrade() else { return };
            {
                let mut state = state.borrow_mut();
                let at = index.min(state.items.len());
                state.items.insert(at, (removed, kind));
            }
            show(&window, &state.borrow());
        },
    );
}

/// The release a row is about, if there is one to open.
#[must_use]
pub fn release_at(state: &Rc<RefCell<NotificationsState>>, index: usize) -> Option<i64> {
    state
        .borrow()
        .items
        .get(index)
        .and_then(|(n, _)| release_of(n))
}

/// Reads the switches from the server and shows them.
pub fn load_switches(window: &MainWindow, client: &Client) {
    if !client.is_authenticated() {
        window.set_notify_switches(slint::ModelRc::new(VecModel::<bool>::default()));
        return;
    }
    let weak = window.as_weak();
    let api = client.clone();
    tasks::spawn(
        async move { api.notification_preferences().await },
        move |result| {
            let Some(window) = weak.upgrade() else { return };
            match result {
                Ok(p) => {
                    let on = vec![
                        p.is_episode_notifications_enabled,
                        p.is_first_episode_notification_enabled,
                        p.is_comment_notifications_enabled,
                        p.is_related_release_notifications_enabled,
                        p.is_article_notifications_enabled,
                        p.is_my_article_comment_notifications_enabled,
                        p.is_my_collection_comment_notifications_enabled,
                        p.is_release_type_notifications_enabled,
                        p.is_report_process_notifications_enabled,
                    ];
                    window.set_notify_switches(slint::ModelRc::new(VecModel::from(on)));
                }
                Err(error) => tracing::debug!(%error, "the notification settings were not loaded"),
            }
        },
    );
}

/// Turns one switch over, then reads them all back: the request carries no
/// value, so the only way to know which way it went is to ask.
pub fn flip(window: &MainWindow, client: &Client, index: i32) {
    let Some(switch) = usize::try_from(index)
        .ok()
        .and_then(|at| SWITCHES.get(at))
        .copied()
    else {
        return;
    };
    let weak = window.as_weak();
    let api = client.clone();
    let mine = client.clone();
    tasks::spawn(
        async move { api.notification_switch(switch).await },
        move |result| {
            if let Err(error) = result {
                tracing::warn!(%error, ?switch, "the setting was not changed");
            }
            if let Some(window) = weak.upgrade() {
                load_switches(&window, &mine);
            }
        },
    );
}

/// Puts the rows on screen.
fn show(window: &MainWindow, state: &NotificationsState) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        });
    let items: Vec<NotificationItem> = state
        .items
        .iter()
        .map(|(n, kind)| item_for(n, *kind, now))
        .collect();
    window.set_notification_items(slint::ModelRc::new(VecModel::from(items)));
    window.set_notifications_has_more(state.has_more);
}

fn item_for(n: &Notification, kind: Kind, now: i64) -> NotificationItem {
    let (subject, detail) = match kind {
        Kind::Episode => n.episode.as_ref().map_or_else(Default::default, |e| {
            let source = [e.name.as_str(), e.source.dubber.name.as_str()]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            (e.release.title_ru.clone(), source)
        }),
        Kind::Friend => (
            n.by_profile
                .as_ref()
                .map(|p| p.login.clone())
                .unwrap_or_default(),
            String::new(),
        ),
        Kind::Article => n.article.as_ref().map_or_else(Default::default, |a| {
            let text = a
                .payload
                .blocks
                .iter()
                .map(anirust_api::ArticleBlock::plain_text)
                .find(|t| !t.trim().is_empty())
                .unwrap_or_default();
            (a.channel.title.clone(), text)
        }),
        Kind::Related => (
            n.release
                .as_ref()
                .map(|r| r.title_ru.clone())
                .unwrap_or_default(),
            String::new(),
        ),
        Kind::Reply | Kind::ArticleComment | Kind::CollectionComment => {
            n.comment.as_ref().map_or_else(Default::default, |c| {
                // A spoiler stays one here too: the notification names who
                // wrote, not what.
                let text = if c.is_spoiler {
                    String::new()
                } else {
                    c.message.clone()
                };
                (c.profile.login.clone(), text)
            })
        }
        Kind::Other => (String::new(), String::new()),
    };

    NotificationItem {
        kind: kind.name().into(),
        subject: subject.into(),
        detail: detail.into(),
        minutes_ago: crate::session::minutes_since(n.timestamp, now),
        unseen: n.is_new,
        can_delete: kind.delete().is_some(),
        can_open: release_of(n).is_some(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anirust_api::{CommentCompact, EpisodeCompact, ReleaseCompact};

    #[test]
    fn a_notification_opens_the_release_it_is_about() {
        let episode = Notification {
            episode: Some(EpisodeCompact {
                release: ReleaseCompact {
                    id: 7,
                    ..ReleaseCompact::default()
                },
                ..EpisodeCompact::default()
            }),
            ..Notification::default()
        };
        assert_eq!(release_of(&episode), Some(7));

        let reply = Notification {
            comment: Some(CommentCompact {
                release: Some(ReleaseCompact {
                    id: 9,
                    ..ReleaseCompact::default()
                }),
                ..CommentCompact::default()
            }),
            ..Notification::default()
        };
        assert_eq!(release_of(&reply), Some(9));
        assert_eq!(release_of(&Notification::default()), None);
    }

    #[test]
    fn a_kind_comes_from_the_type_or_else_from_the_list() {
        let typed = Notification {
            kind: "friend".into(),
            ..Notification::default()
        };
        assert_eq!(Kind::of(&typed, NotificationKind::All), Kind::Friend);
        let untyped = Notification::default();
        assert_eq!(
            Kind::of(&untyped, NotificationKind::ReleaseComments),
            Kind::Reply
        );
        assert_eq!(Kind::of(&untyped, NotificationKind::All), Kind::Other);
    }

    #[test]
    fn posts_cannot_be_deleted_one_at_a_time() {
        assert!(Kind::Article.delete().is_none());
        assert!(Kind::Episode.delete().is_some());
    }

    #[test]
    fn the_switches_and_the_menu_are_the_lengths_the_screen_expects() {
        assert_eq!(SWITCHES.len(), 9, "the profile screen has nine labels");
        assert_eq!(KINDS.len(), 8, "the sheet's menu has eight entries");
    }
}
