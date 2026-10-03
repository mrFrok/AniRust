// SPDX-License-Identifier: GPL-3.0-or-later

//! Renders the interface to a PNG without a display.
//!
//! Layout mistakes are cheap to make and expensive to spot by description: a
//! hidden element that still holds its place, a column that collapses, a row
//! whose glyphs sit on three different centre lines. All of those are obvious
//! in a picture and invisible in the source. This draws the real components
//! with the software renderer so they can be looked at from a terminal, in CI,
//! or over SSH.
//!
//! The video surface stays empty — it is a borrowed GL texture, and there is no
//! GL here. Everything around it is the real thing, with one difference worth
//! knowing before a bug is chased that is not there: the software renderer
//! clips to a rectangle, ignoring `border-radius`. Anything drawn inside a
//! rounded box with `clip: true` — the selected segment of a segmented
//! button, a poster in its card — has square corners here and rounded ones in
//! the application, which draws through OpenGL.
//!
//! ```text
//! cargo run -p anirust-gui --example screenshot -- out.png [width height] [state]
//! ```
//!
//! `state` is `home`, `home-signed-in`, `release` (default), `playing`,
//! `theatre`, `downloads`, `sign-in`, `failed`, `saved`, `loading`,
//! `refreshing`, `empty`, `nothing`, `profile`, `profile-signed-in`,
//! `profile-light`, `home-downloading`, `feed`, `feed-latest`,
//! `feed-signed-out`, `feed-empty`, `feed-channels`, `feed-channel`, `collections`, `collection`,
//! `collection-editor`, `collection-picker`, `search`, `sign-up`,
//! `sign-up-code`, `restore-code`, `settings`, `settings-email`,
//! `settings-standing`, `report`, `editor`, `feed-own`, `admin`, `admin-suggested`,
//! `admin-new`, `sign-out`, `app-settings`, `release-announced`,
//! `release-note`, `franchise`, `profile-teal`,
//! `profile-teal-glass`, `deletion`, `deletion-pending`, `comments`, `comments-replying`,
//! `comments-signed-out`, `light` or `amoled`. Narrow is a width, not a state: pass
//! one below 900.
//!
//! `ANIRUST_SHOT_POINTER=x,y` parks the pointer anywhere, for the states
//! that only show under it.
//!
//! `home-hover-account` parks the pointer on the account button, which is the
//! only way to see what it offers: the action it performs is in a tooltip,
//! and a tooltip is drawn for a pointer that is not there in a screenshot.

use std::rc::Rc;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter, WindowEvent};
use slint::{LogicalPosition, PhysicalSize, PlatformError};

slint::include_modules!();

/// A platform with no windowing system behind it.
struct Headless {
    window: Rc<MinimalSoftwareWindow>,
}

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| "screenshot.png".to_owned());
    let width: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(1440);
    let height: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(900);
    let state = args.next().unwrap_or_else(|| "release".to_owned());

    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless {
        window: window.clone(),
    }))?;

    let ui = MainWindow::new()?;
    populate(&ui);
    populate_home(&ui);
    ui.set_screen(
        if state.starts_with("home")
            || state.starts_with("sign-")
            || state.starts_with("restore")
            || state == "downloads"
            || state == "saved"
            || state == "loading"
            || state == "empty"
            || state == "refreshing"
            || state.starts_with("profile")
            || state == "light"
            || state == "amoled"
            || state.starts_with("feed")
            || state == "schedule"
            || state.starts_with("notifications")
            || state.starts_with("collection")
            || state == "search"
            || state.starts_with("settings")
            || state.starts_with("report")
            || state == "editor"
            || state.starts_with("admin")
            || state == "sign-out"
            || state.starts_with("app-settings")
            || state.starts_with("deletion")
        {
            "home".into()
        } else {
            "release".into()
        },
    );
    if state.starts_with("home-signed-in") || state == "home-hover-account" {
        ui.set_signed_in(true);
        ui.set_account_name("mrfrok".into());
    }
    if state == "downloads" {
        ui.set_destination(5);
        ui.set_downloads_working(2);
        ui.set_downloads(slint::ModelRc::new(slint::VecModel::from(vec![
            DownloadItem {
                title: "Демоны старшей школы — 3".into(),
                dubber: "AniLibria".into(),
                status: "done".into(),
                progress: 1.0,
                detail: "/home/you/Видео/AniRust/Демоны старшей школы - 03 [AniLibria].mp4".into(),
            },
            DownloadItem {
                title: "Демоны старшей школы — 4".into(),
                dubber: "AniLibria".into(),
                status: "running".into(),
                progress: 0.42,
                detail: "".into(),
            },
            DownloadItem {
                title: "Демоны старшей школы — 5".into(),
                dubber: "AniLibria".into(),
                status: "queued".into(),
                progress: 0.0,
                detail: "".into(),
            },
        ])));
    }
    // A description short enough that the franchise and the platforms under
    // it are in view without scrolling the panel.
    if state == "release-links" {
        ui.set_release_description("Короткое описание.".into());
    }
    // The comment sheet over the release, with every kind of row in it.
    if state.starts_with("comments") {
        ui.set_signed_in(state != "comments-signed-out");
        ui.set_comments_open(true);
        ui.set_comments_title("Демоны старшей школы".into());
        ui.set_comments_total(1241);
        ui.set_comments_sort(2);
        ui.set_comments_has_more(true);
        ui.set_comments(slint::ModelRc::new(
            slint::VecModel::from(sample_comments()),
        ));
        if state == "comments-replying" {
            ui.set_comments_reply_to("user2".into());
            ui.set_comment_draft("Согласен, но вторая половина сезона сильнее.".into());
        }
    }
    // The account's view of a release: in a list, a favourite, rated.
    if state == "release-rated" {
        ui.set_signed_in(true);
        ui.set_release_list(2);
        ui.set_release_favourite(true);
        ui.set_release_vote(4);
    }
    if state == "failed" {
        ui.set_episode_failed(true);
    }
    // Nothing to start: no episodes behind the voice-over that was chosen.
    if state == "nothing" {
        ui.set_episodes(slint::ModelRc::new(
            slint::VecModel::<EpisodeItem>::default(),
        ));
        ui.set_resume_episode(0);
    }
    // The destination with the most tabs, which is where a bar that divides
    // its width between them is worth looking at.
    if state == "saved" {
        ui.set_destination(2);
        ui.set_tab(5);
    }
    // The feed, with posts of each shape it has to lay out: long text that
    // folds, a picture, a short line, a blog, a channel already followed.
    if state.starts_with("feed") {
        ui.set_destination(3);
        ui.set_signed_in(state != "feed-signed-out");
        ui.set_feed_tab(match state.as_str() {
            "feed-latest" => 1,
            "feed-channels" => 2,
            _ => 0,
        });
        if state != "feed-signed-out" && state != "feed-empty" {
            let mut posts = sample_posts();
            // The account's own post, with what it may do to it.
            if state == "feed-own" {
                posts[0].can_edit = true;
                posts[0].can_delete = true;
                posts[0].can_pin = true;
            }
            ui.set_posts(slint::ModelRc::new(slint::VecModel::from(posts)));
        }
        if state == "feed-channels" {
            ui.set_feed_channels(slint::ModelRc::new(
                slint::VecModel::from(sample_channels()),
            ));
            ui.set_subscription_count(2);
        }
        // A channel's own page: its header over its posts.
        if state == "feed-channel" {
            let mut channel = sample_channels().swap_remove(0);
            channel.muted = true;
            ui.set_open_channel_item(channel);
            ui.set_channel_open(true);
        }
    }
    // Collections: the grid of them, one open over its releases, and the
    // two sheets for the account's own.
    if state == "collections" {
        ui.set_destination(1);
        ui.set_tab(4);
        ui.set_genres(slint::ModelRc::new(
            slint::VecModel::<slint::SharedString>::default(),
        ));
    }
    if state == "collection" || state.starts_with("collection-") {
        ui.set_destination(2);
        ui.set_tab(7);
        ui.set_signed_in(true);
        ui.set_collection_open(state == "collection");
        ui.set_open_collection_item(CollectionItem {
            title: "Лучшее за десятилетие".into(),
            description: "То, что стоит посмотреть каждому: без филлеров, без затянутых арок."
                .into(),
            creator: "user7".into(),
            image: slint::Image::default(),
            image_loaded: false,
            favourite: true,
            favourites: 214,
            comments: 18,
            private: true,
            mine: true,
        });
    }
    if state == "collection-editor" {
        ui.set_collection_editor_open(true);
        ui.set_collection_editing(true);
        ui.set_collection_draft_title("Лучшее за десятилетие".into());
        ui.set_collection_draft_private(true);
    }
    if state == "collection-picker" {
        ui.set_collection_picker_open(true);
        ui.set_my_collection_titles(slint::ModelRc::new(slint::VecModel::from(
            ["Лучшее за десятилетие", "На вечер", "Пересмотреть"]
                .iter()
                .map(|t| slint::SharedString::from(*t))
                .collect::<Vec<_>>(),
        )));
    }
    // The account's settings over its profile, with a change refused.
    if state.starts_with("settings") {
        ui.set_destination(4);
        ui.set_signed_in(true);
        ui.set_settings_open(true);
        ui.set_account_settings(AccountSettings {
            status: "Смотрю всё подряд".into(),
            email_hint: "m***k@g***.com".into(),
            telegram: "mrfrok".into(),
            privacy_stats: 1,
            privacy_social: 2,
            privacy_friend_requests: 0,
            incognito: true,
            login_change_available: state == "settings",
            login_next_change: "12 ноября 2026".into(),
            ..AccountSettings::default()
        });
        ui.set_settings_message(
            if state == "settings" {
                "login-taken"
            } else {
                "saved"
            }
            .into(),
        );
        ui.set_email_code_pending(state == "settings-email");
    }
    // Another accent, and the window see-through.
    if state.starts_with("profile-teal") {
        ui.set_accent(Accent::Teal);
        ui.set_accent_index(3);
        ui.set_translucent(state == "profile-teal-glass");
    }
    // A release that is only announced; a team note; the whole franchise.
    if state == "release-announced" {
        ui.set_release_watchable(false);
        ui.set_release_rateable(false);
        ui.set_release_score("".into());
    }
    if state == "release-note" || state == "franchise" {
        ui.set_release_note(
            "Фильм является краткой версией истории двух сезонов. К просмотру не обязателен."
                .into(),
        );
    }
    if state == "franchise" {
        ui.set_franchise_open(true);
        let item = |title: &str, detail: &str, current: bool| FranchiseItem {
            title: title.into(),
            detail: detail.into(),
            poster: stand_in_avatar(),
            poster_loaded: true,
            current,
        };
        ui.set_franchise_items(slint::ModelRc::new(slint::VecModel::from(vec![
            item("Демоны старшей школы", "ТВ-сериал · 2012", true),
            item("Демоны старшей школы: OVA", "OVA · 2012", false),
            item("Демоны старшей школы: Новая", "ТВ-сериал · 2013", false),
            item("Демоны старшей школы: Рождённые", "ТВ-сериал · 2015", false),
            item("Демоны старшей школы: Герой", "ТВ-сериал · 2018", false),
        ])));
    }
    // The settings destination, signed in, with its switches.
    if state.starts_with("app-settings") {
        ui.set_destination(6);
        ui.set_signed_in(state == "app-settings");
        ui.set_account_name("mrfrok".into());
        ui.set_app_version("0.1.0".into());
        ui.set_notify_switches(slint::ModelRc::new(slint::VecModel::from(vec![
            true, false, true, true, false, true, true, false, false,
        ])));
    }
    // The question before signing out.
    if state == "sign-out" {
        ui.set_destination(4);
        ui.set_signed_in(true);
        ui.set_account_name("mrFrok".into());
        ui.set_confirm_sign_out(true);
    }
    // Running a channel: its settings, and the queue of suggested posts.
    if state.starts_with("admin") {
        ui.set_destination(3);
        ui.set_signed_in(true);
        ui.set_admin_open(true);
        ui.set_admin_creator(true);
        ui.set_admin_title("Новостной канал".into());
        ui.set_admin_description("Анонсы, даты выхода и новости индустрии.".into());
        ui.set_admin_suggestions(true);
        if state == "admin-suggested" {
            ui.set_admin_tab(1);
            let item = |author: &str, text: &str| SuggestionItem {
                author: author.into(),
                text: text.into(),
            };
            ui.set_admin_suggested(slint::ModelRc::new(slint::VecModel::from(vec![
                item("user2", "Вышел трейлер второго сезона, дата — апрель."),
                item(
                    "user5",
                    "Подборка опенингов этого сезона: десять штук, от лучшего к худшему.",
                ),
            ])));
        }
        if state == "admin-new" {
            ui.set_admin_creating(true);
            ui.set_admin_title("".into());
            ui.set_admin_description("".into());
        }
    }
    // The post editor, a few blocks in, one of them a picture it keeps.
    if state == "editor" {
        ui.set_destination(3);
        ui.set_signed_in(true);
        ui.set_editor_open(true);
        ui.set_editor_channels(slint::ModelRc::new(slint::VecModel::from(vec![
            PickerOption {
                label: "mrfrok".into(),
                ..PickerOption::default()
            },
        ])));
        ui.set_editor_can_sign(false);
        let block = |kind: &str, text: &str, caption: &str| DraftBlock {
            kind: kind.into(),
            text: text.into(),
            caption: caption.into(),
        };
        ui.set_editor_blocks(slint::ModelRc::new(slint::VecModel::from(vec![
            block("header", "Что посмотреть этой осенью", ""),
            block(
                "paragraph",
                "Короткий список того, что вышло и что стоит начать.",
                "",
            ),
            block("list", "Первое\nВторое\nТретье", ""),
            block("media", "", ""),
            block("quote", "Скоро.", "студия"),
            block("delimiter", "", ""),
        ])));
    }
    // A report about a comment, a reason picked; and the deletion sheet,
    // before a request and with one in.
    if state == "report" {
        ui.set_report_open(true);
        ui.set_report_subject("Согласен, вторая половина сильнее.".into());
        ui.set_report_reasons(slint::ModelRc::new(slint::VecModel::from(
            ["Спам", "Оскорбления", "Спойлеры", "Другое"]
                .iter()
                .map(|r| slint::SharedString::from(*r))
                .collect::<Vec<_>>(),
        )));
    }
    if state.starts_with("deletion") {
        ui.set_destination(4);
        ui.set_signed_in(true);
        ui.set_account(Account {
            login: "mrfrok".into(),
            ..Account::default()
        });
        ui.set_deletion_open(true);
        ui.set_deletion_pending(state == "deletion-pending");
        ui.set_deletion_at("2 ноября 2026".into());
    }
    if state == "settings-standing" {
        ui.set_standing("Блокировок: 1. Ограничение до 12 ноября 2026.".into());
        ui.set_enforcements(slint::ModelRc::new(slint::VecModel::from(vec![
            EnforcementItem {
                reason: "Оскорбления в комментариях".into(),
                date: "28 сентября 2026".into(),
                status: "".into(),
                answer: "".into(),
                can_appeal: true,
            },
            EnforcementItem {
                reason: "Спойлеры без пометки".into(),
                date: "3 августа 2026".into(),
                status: "rejected".into(),
                answer: "Спойлер был в первой строке.".into(),
                can_appeal: false,
            },
        ])));
    }
    // A search of everything: people and channels over the releases.
    if state == "search" {
        ui.set_query("демон".into());
        ui.set_searching(true);
        let face = |name: &str, online: bool| PersonItem {
            login: name.into(),
            avatar: slint::Image::default(),
            avatar_loaded: false,
            online,
        };
        ui.set_found_people(slint::ModelRc::new(slint::VecModel::from(vec![
            face("demon_lord", true),
            face("демонёнок", false),
            face("issei_hyodo", false),
        ])));
        ui.set_found_channels(slint::ModelRc::new(slint::VecModel::from(vec![face(
            "Демоны старшей школы — фан-канал",
            false,
        )])));
    }
    // The bell with a count, and its sheet open over the grid.
    if state.starts_with("notifications") {
        ui.set_signed_in(true);
        ui.set_account_name("mrfrok".into());
        ui.set_unseen(3);
        ui.set_notifications_open(state == "notifications");
        let n = |kind: &str, subject: &str, detail: &str, minutes: i32, unseen: bool| {
            NotificationItem {
                kind: kind.into(),
                subject: subject.into(),
                detail: detail.into(),
                minutes_ago: minutes,
                unseen,
                can_delete: kind != "article",
                can_open: kind == "episode" || kind == "reply" || kind == "related",
            }
        };
        ui.set_notification_items(slint::ModelRc::new(slint::VecModel::from(vec![
            n(
                "episode",
                "Re:Zero. Жизнь с нуля 4",
                "17 серия · AniLibria",
                40,
                true,
            ),
            n(
                "reply",
                "user2",
                "Согласен, вторая половина сильнее.",
                300,
                true,
            ),
            n("friend", "user5", "", 1500, true),
            n(
                "article",
                "Новостной канал",
                "Анонс второго сезона.",
                3000,
                false,
            ),
            n("related", "Демоны старшей школы: Герой", "", 9000, false),
        ])));
    }
    // The schedule: the home tabs at their fullest, and the chip row as days.
    if state == "schedule" {
        ui.set_destination(0);
        ui.set_tab(7);
        ui.set_genre(4);
        ui.set_chips_are_days(true);
        ui.set_genres(slint::ModelRc::new(slint::VecModel::from(
            ["пн", "вт", "ср", "чт", "пт", "сб", "вс"]
                .iter()
                .map(|day| slint::SharedString::from(*day))
                .collect::<Vec<_>>(),
        )));
    }
    // Something in the queue, so the badge on the toolbar has a number in it.
    if state == "home-downloading" {
        ui.set_downloads_working(3);
    }
    // Both languages count differently, and the profile screen is where the
    // counting words are: three forms in Russian, two in English.
    if state.ends_with("-en") {
        ui.set_lang("en".into());
    }
    // The appearances that can be seen without a desktop to ask: the browsing
    // screen is the one with the most of the palette on it at once.
    if state == "light" || state == "profile-light" {
        ui.set_appearance(Appearance::Light);
    }
    if state == "amoled" {
        ui.set_appearance(Appearance::Amoled);
    }
    // The screen about the application rather than about what to watch.
    if state.starts_with("profile") {
        ui.set_destination(4);
        // `profile` alone is the screen with nobody on it; every other spelling
        // of it has an account behind it.
        ui.set_signed_in(state != "profile");
        ui.set_account_name("mrfrok".into());
        ui.set_appearance_choice(3);
        ui.set_account(Account {
            login: "mrfrok".into(),
            status: "статус, который аккаунт написал о себе".into(),
            registered: "2019-05-12".into(),
            verified: true,
            watching: 12,
            planned: 148,
            watched: 306,
            hold: 4,
            dropped: 9,
            votes: 271,
            comments: 18,
            collections: 3,
            friends: 26,
            episodes_watched: 2824,
            minutes_watched: 67_140,
            genres: "экшен 11%, фэнтези 9%, комедия 8%".into(),
            audiences: "сёнен 8%".into(),
            themes: "школа 5%".into(),
            dynamics: slint::ModelRc::new(slint::VecModel::from(vec![0, 0, 0, 0, 2, 0, 0, 0])),
            dynamics_days: slint::ModelRc::new(slint::VecModel::from(vec![
                12, 13, 14, 15, 16, 17, 18, 19,
            ])),
            stats_hidden: false,
            friend_status: -1,
            blocked: false,
            friend_requests_closed: false,
            online: true,
        });
        ui.set_avatar(stand_in_avatar());
        ui.set_avatar_loaded(true);
        let person = |login: &str, online: bool| PersonItem {
            login: login.into(),
            avatar: stand_in_avatar(),
            avatar_loaded: true,
            online,
        };
        ui.set_profile_friends(slint::ModelRc::new(slint::VecModel::from(vec![
            person("maryfoxloza", true),
            person("A5tepXd", false),
        ])));
        ui.set_friend_requests(slint::ModelRc::new(slint::VecModel::from(vec![person(
            "user7", false,
        )])));
        if state == "profile-other" {
            ui.set_profile_is_mine(false);
            ui.set_friend_requests(slint::ModelRc::new(slint::VecModel::<PersonItem>::default()));
        }
        ui.set_notify_switches(slint::ModelRc::new(slint::VecModel::from(vec![
            true, false, true, true, false, true, true, false, false,
        ])));
        ui.set_recent(slint::ModelRc::new(slint::VecModel::from(vec![
            recent("Реинкарнация безработного 3", 13, 64),
            recent("Re:Zero. Жизнь с нуля 4", 17, 2_890),
            recent("НищеБог-же", 2, 2_896),
            recent("Военная хроника маленькой девочки 2", 10, 6_210),
            recent("Комендант общежития богинь", 1, 85_320),
        ])));
    }
    // A tab switched: the last tab's cards stay up while the new ones are
    // fetched, which is the only case where loading is said over a full grid.
    if state == "refreshing" {
        ui.set_results_loading(true);
    }
    // Waiting for the first page of results, and having asked for something
    // there is none of: the two empty states of the browsing screen.
    if state == "loading" || state == "empty" {
        ui.set_results(slint::ModelRc::new(
            slint::VecModel::<ReleaseCard>::default(),
        ));
        ui.set_results_loading(state == "loading");
    }
    if state == "empty" {
        ui.set_searching(true);
    }
    if state == "sign-in" {
        ui.set_show_sign_in(true);
        ui.set_login("mrfrok".into());
        ui.set_password("hunter2".into());
        ui.set_sign_in_error("wrong-password".into());
    }
    // Registering: a taken login with the service's suggestions, then the
    // code; and restoring, at the code with the new password beside it.
    if state == "sign-up" {
        ui.set_show_sign_in(true);
        ui.set_sign_in_mode("sign-up".into());
        ui.set_login("mrfrok".into());
        ui.set_sign_up_email("me@example.com".into());
        ui.set_password("hunter2".into());
        ui.set_sign_in_error("login-taken".into());
        ui.set_login_suggestions(slint::ModelRc::new(slint::VecModel::from(vec![
            slint::SharedString::from("mrfrok1"),
            "mrfrok_2026".into(),
        ])));
    }
    if state == "sign-up-code" {
        ui.set_show_sign_in(true);
        ui.set_sign_in_mode("sign-up-code".into());
        ui.set_sign_in_code("12345".into());
        ui.set_sign_in_error("code-expired".into());
    }
    if state == "restore-code" {
        ui.set_show_sign_in(true);
        ui.set_sign_in_mode("restore-code".into());
    }
    window.set_size(PhysicalSize::new(width, height));

    let mut pixels = vec![slint::Rgb8Pixel { r: 0, g: 0, b: 0 }; (width * height) as usize];
    let mut draw = |window: &MinimalSoftwareWindow| {
        slint::platform::update_timers_and_animations();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, width as usize);
        });
        window.request_redraw();
    };

    // The first pass is where `init` runs, the breakpoint is measured and the
    // list learns its own height. Only then does starting an episode mean
    // anything — scrolling to it needs a list that has been laid out.
    draw(&window);
    if state == "playing"
        || state == "theatre"
        || state == "upscale"
        || state == "paused"
        || state == "player-settings"
    {
        start_playing(&ui, state == "theatre");
    }
    // The settings sheet, with some sound and subtitle choices made.
    if state == "player-settings" {
        ui.set_audio_delay_label("+0,3".into());
        ui.set_subtitle_scale(2);
        ui.set_normalize(true);
        ui.set_player_sheet("settings".into());
    }
    // Paused, which is when the frame steps appear beside play.
    if state == "paused" {
        ui.set_paused(true);
        ui.set_has_video(true);
    }
    // The upscale sheet, a mode and a quality picked.
    if state == "upscale" {
        ui.set_upscale(4);
        ui.set_upscale_quality(2);
        ui.set_frames_available(true);
        ui.set_frame_rate(2);
        ui.set_player_sheet("anime4k".into());
    }
    // A pointer put where the account button is, so the tooltip that says what
    // clicking it does is drawn. Measured from the right edge, which is where
    // the button sits whatever the window is.
    if state == "home-hover-account" {
        window.dispatch_event(WindowEvent::PointerMoved {
            position: LogicalPosition::new(width as f32 - 62.0, 36.0),
        });
    }
    // A pointer anywhere, for hover states that only show under it:
    // `ANIRUST_SHOT_POINTER=x,y`.
    if let Some((x, y)) = std::env::var("ANIRUST_SHOT_POINTER").ok().and_then(|at| {
        let (x, y) = at.split_once(',')?;
        Some((x.trim().parse::<f32>().ok()?, y.trim().parse::<f32>().ok()?))
    }) {
        window.dispatch_event(WindowEvent::PointerMoved {
            position: LogicalPosition::new(x, y),
        });
        // And a turn of the wheel there, for what is below the fold of a
        // scrolled area: `ANIRUST_SHOT_SCROLL=pixels`, positive downwards.
        if let Some(pixels) = std::env::var("ANIRUST_SHOT_SCROLL")
            .ok()
            .and_then(|pixels| pixels.trim().parse::<f32>().ok())
        {
            draw(&window);
            window.dispatch_event(WindowEvent::PointerScrolled {
                position: LogicalPosition::new(x, y),
                delta_x: 0.0,
                delta_y: -pixels,
            });
        }
    }
    draw(&window);
    draw(&window);
    // Animations are driven by the clock, and three frames drawn in the same
    // microsecond leave every one of them at its first step: an indicator
    // half-way to the tab it belongs to, a control mid-stretch. Half a second
    // of real time puts the picture in the state a viewer would actually see.
    for _ in 0..12 {
        std::thread::sleep(std::time::Duration::from_millis(40));
        draw(&window);
    }

    let mut buffer = image::RgbImage::new(width, height);
    for (pixel, out) in pixels.iter().zip(buffer.pixels_mut()) {
        *out = image::Rgb([pixel.r, pixel.g, pixel.b]);
    }
    buffer.save(&path)?;

    println!("wrote {path} ({width}x{height}, {state})");
    Ok(())
}

/// Fills the window with a release that exercises the cases worth looking at:
/// a long description, several voice-overs, episodes both watched and
/// part-watched, and one with a title of its own.
fn populate(ui: &MainWindow) {
    ui.set_lang("ru".into());
    ui.set_release_title("Демоны старшей школы".into());
    ui.set_release_original_title("High School DxD".into());
    ui.set_release_year("2012".into());
    ui.set_release_genres("комедия, романтика, сверхъестественное, экшен, этти, гарем".into());
    ui.set_release_studio("TNK".into());
    ui.set_release_status("Вышел".into());
    ui.set_release_score("4.6".into());
    ui.set_release_episodes_label("24/24".into());
    ui.set_release_description(
        "Что нужно от жизни простому 17-летнему японскому школьнику? Иссэй Хёдо отлично \
         знает ответ, ведь ради этого он и записался в бывшую женскую академию Куо! Хёдо \
         наивно полагал, что после начала совместного обучения в условиях дефицита парней \
         станет королем и познает весну жизни, однако идет второй год."
            .into(),
    );
    ui.set_release_loading(false);
    let link = |label: &str, detail: &str| LinkItem {
        label: label.into(),
        detail: detail.into(),
    };
    ui.set_release_related(slint::ModelRc::new(slint::VecModel::from(vec![
        link("Демоны старшей школы: Новая", "2013"),
        link("Демоны старшей школы: Рожденные", "2015"),
        link("Демоны старшей школы: Герой", "2018"),
        link("OVA", "2012"),
    ])));
    ui.set_release_platforms(slint::ModelRc::new(slint::VecModel::from(vec![
        link("Crunchyroll", ""),
        link("Wink", ""),
    ])));
    let video = |title: &str, detail: &str| VideoItem {
        title: title.into(),
        detail: detail.into(),
        image: stand_in_avatar(),
        image_loaded: true,
    };
    ui.set_release_videos(slint::ModelRc::new(slint::VecModel::from(vec![
        video("Трейлер", "Трейлеры · YouTube"),
        video("Опенинг «Trip -innocent of D-»", "Опенинги · YouTube"),
        video("Эндинг", "Эндинги · YouTube"),
    ])));

    ui.set_dubbers(slint::ModelRc::new(slint::VecModel::from(vec![
        option("AniLibria", 14, false),
        option("AniDUB", 12, false),
        option("SHIZA Project", 12, false),
        option("Субтитры", 12, true),
    ])));
    ui.set_sources(slint::ModelRc::new(slint::VecModel::from(vec![
        option("Kodik", 12, false),
        option("Libria", 12, false),
    ])));

    // Long enough that the list has to scroll, which is the case worth
    // looking at: resuming episode 13 should not start the list at episode 1.
    let episodes: Vec<EpisodeItem> = (1..=24)
        .map(|position| EpisodeItem {
            position,
            name: if position == 7 {
                "Рождение".into()
            } else {
                "".into()
            },
            watched: position < 13,
            resume_at: if position == 13 {
                "8:21".into()
            } else {
                "".into()
            },
            filler: position == 9,
        })
        .collect();
    ui.set_episodes(slint::ModelRc::new(slint::VecModel::from(episodes)));
    ui.set_resume_episode(13);
}

/// Puts an episode in the player, as clicking one would.
fn start_playing(ui: &MainWindow, theatre: bool) {
    {
        ui.set_playing(true);
        ui.set_current_episode(13);
        ui.set_current_index(12);
        ui.set_episode_label("13 - AniLibria".into());
        ui.set_state("playing".into());
        ui.set_position_text("8:21".into());
        ui.set_duration_text("23:40".into());
        ui.set_progress(0.353);
        ui.set_buffered(0.48);
        ui.set_quality_label("720p → 1440p".into());
        ui.set_decoder_label("NVDEC".into());
        ui.set_upscale(2);
        ui.set_has_skip(true);
        ui.set_has_next(true);
        ui.set_has_previous(true);
        ui.set_qualities(slint::ModelRc::new(slint::VecModel::from(vec![
            slint::SharedString::from("1080p"),
            slint::SharedString::from("720p"),
            slint::SharedString::from("480p"),
        ])));
        ui.set_theatre(theatre);
    }
}

/// A grid of results, so the browsing screen can be looked at too.
fn populate_home(ui: &MainWindow) {
    let titles = [
        ("Демоны старшей школы", "2012 · 12/12", "4.6"),
        ("Стальной алхимик: Братство", "2009 · 64/64", "4.9"),
        ("Магическая битва", "2020 · 24/24", "4.8"),
        ("Клинок, рассекающий демонов", "2019 · 26/26", "4.8"),
        ("Атака титанов", "2013 · 25/25", "4.9"),
        ("Ван-Пис", "1999 · 1122", "4.7"),
        ("Наруто: Ураганные хроники", "2007 · 500/500", "4.5"),
        ("Токийский гуль", "2014 · 12/12", "4.4"),
        ("Код Гиас: Восставший Лелуш", "2006 · 25/25", "4.8"),
        ("Тетрадь смерти", "2006 · 37/37", "4.9"),
        ("Re:Zero", "2016 · 25/25", "4.7"),
        ("Доктор Стоун", "2019 · 24/24", "4.6"),
    ];

    let cards: Vec<ReleaseCard> = titles
        .iter()
        .map(|(title, subtitle, score)| ReleaseCard {
            title: (*title).into(),
            subtitle: (*subtitle).into(),
            score: (*score).into(),
            poster: slint::Image::default(),
            poster_loaded: false,
        })
        .collect();

    ui.set_results(slint::ModelRc::new(slint::VecModel::from(cards)));

    ui.set_genres(slint::ModelRc::new(slint::VecModel::from(
        anirust_gui_genres()
            .iter()
            .map(|name| slint::SharedString::from(*name))
            .collect::<Vec<_>>(),
    )));
    ui.set_destination(1);
    ui.set_tab(1);
    ui.set_genre(5);
}

/// The chips the browsing screen offers. Repeated here rather than imported:
/// an example cannot reach into the binary crate it renders.
fn anirust_gui_genres() -> [&'static str; 8] {
    [
        "экшен",
        "фэнтези",
        "приключения",
        "драма",
        "комедия",
        "романтика",
        "школа",
        "исэкай",
    ]
}

/// A picture standing in for the account's own.
///
/// Drawn here rather than fetched: there is no network in a screenshot, and a
/// screen that always shows the placeholder says nothing about how it looks
/// with a picture in it. It is deliberately a flat gradient — nobody should
/// mistake it for proof that the real one loads.
fn stand_in_avatar() -> slint::Image {
    const SIZE: u32 = 128;
    let mut buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(SIZE, SIZE);
    let width = buffer.width();
    for (index, pixel) in buffer.make_mut_slice().iter_mut().enumerate() {
        let x = (index as u32 % width) as f32 / SIZE as f32;
        let y = (index as u32 / width) as f32 / SIZE as f32;
        *pixel = slint::Rgba8Pixel {
            r: (120.0 + 90.0 * x) as u8,
            g: (70.0 + 40.0 * y) as u8,
            b: (190.0 + 50.0 * (1.0 - x)) as u8,
            a: 255,
        };
    }
    slint::Image::from_rgba8(buffer)
}

/// Posts written for the screenshot. Their wording is invented and says so;
/// only their shapes matter.
fn sample_posts() -> Vec<FeedPost> {
    let post = |channel: &str, minutes: i32, text: &str| FeedPost {
        channel: channel.into(),
        channel_avatar: stand_in_avatar(),
        channel_avatar_loaded: true,
        subscribed: false,
        can_subscribe: true,
        minutes_ago: minutes,
        text: text.into(),
        picture: slint::Image::default(),
        picture_loaded: false,
        has_picture: false,
        comments: 4,
        votes: 37,
        my_vote: 0,
        vote_refused: false,
        pinned: false,
        can_edit: false,
        can_delete: false,
        can_pin: false,
        picture_ratio: 0.0,
    };
    vec![
        FeedPost {
            pinned: true,
            subscribed: true,
            ..post(
                "Новостной канал",
                90,
                "Закреплённая запись канала. Длинный текст, чтобы было видно, как запись сворачивается после шести строк и предлагает показать остальное. Ещё одно предложение, чтобы строк точно хватило. И ещё одно, на всякий случай, потому что окно бывает широким. Здесь текст продолжается и продолжается, как продолжаются настоящие анонсы, в которых всё самое важное — в последнем абзаце.

Второй абзац записи, отделённый пустой строкой, как и блоки редактора.

 Третий абзац, которого в свёрнутом виде уже не видно.",
            )
        },
        FeedPost {
            my_vote: 2,
            vote_refused: true,
            has_picture: true,
            picture: stand_in_avatar(),
            picture_loaded: true,
            ..post("Канал с картинками", 2_900, "Короткая подпись к картинке.")
        },
        FeedPost {
            can_subscribe: false,
            ..post("mrFrok", 6_300, "Запись из личного блога: на блог не подписываются, поэтому кнопки нет.")
        },
    ]
}

/// A thread with one of each kind of row: plain, a spoiler, the account's
/// own, an opened reply chain, and a deleted comment. Invented wording.
fn sample_comments() -> Vec<CommentItem> {
    let base = |author: &str, minutes: i32, text: &str| CommentItem {
        author: author.into(),
        avatar: stand_in_avatar(),
        avatar_loaded: true,
        minutes_ago: minutes,
        text: text.into(),
        spoiler: false,
        revealed: false,
        edited: false,
        deleted: false,
        score: 0,
        my_vote: 0,
        replies: 0,
        expanded: false,
        is_reply: false,
        mine: false,
        episode: 0,
    };
    vec![
        CommentItem {
            score: 884,
            my_vote: 2,
            replies: 2,
            expanded: true,
            ..base(
                "user1",
                60 * 24 * 300,
                "Комментарий с высоким рейтингом. Текст длиной в несколько строк, чтобы было видно, как он переносится в узкой панели и как под ним стоят голоса и действия.",
            )
        },
        CommentItem {
            is_reply: true,
            score: 12,
            ..base(
                "user2",
                60 * 24 * 290,
                "Ответ на него — с отступом, аватар меньше.",
            )
        },
        CommentItem {
            is_reply: true,
            mine: true,
            edited: true,
            score: -3,
            my_vote: 0,
            ..base(
                "mrfrok",
                60 * 5,
                "Свой ответ: его можно изменить и удалить.",
            )
        },
        CommentItem {
            spoiler: true,
            score: 45,
            replies: 7,
            episode: 12,
            ..base("user3", 60 * 26, "Скрытый текст спойлера.")
        },
        CommentItem {
            deleted: true,
            replies: 3,
            ..base("user4", 60 * 24 * 3, "")
        },
    ]
}

fn recent(title: &str, episode: i32, minutes_ago: i32) -> HistoryItem {
    HistoryItem {
        title: title.into(),
        episode,
        poster: slint::Image::default(),
        poster_loaded: false,
        minutes_ago,
    }
}

fn option(label: &str, episodes: i32, is_sub: bool) -> PickerOption {
    PickerOption {
        label: label.into(),
        episodes,
        is_sub,
        pinned: false,
    }
}

fn sample_channels() -> Vec<ChannelItem> {
    let c = |title: &str, description: &str, subscribers: i32, subscribed: bool, blog: bool| {
        ChannelItem {
            title: title.into(),
            description: description.into(),
            avatar: slint::Image::default(),
            avatar_loaded: false,
            subscribers,
            subscribed,
            muted: false,
            blog,
            verified: !blog,
            manageable: false,
        }
    };
    vec![
        c(
            "Новостной канал",
            "Анонсы, даты выхода и новости индустрии.",
            48_210,
            true,
            false,
        ),
        c("user7", "Пишу о том, что смотрю.", 312, true, true),
        c(
            "Обзоры сезона",
            "Что смотреть этой весной: коротко и по делу, без спойлеров.",
            9_870,
            false,
            false,
        ),
        c("Клипы и опенинги", "", 1_204, false, false),
    ]
}
