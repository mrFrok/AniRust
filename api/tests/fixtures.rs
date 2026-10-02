// SPDX-License-Identifier: GPL-3.0-or-later
//
// Responses captured from the live service, decoded.
//
// Where the endpoint table checks what goes out, these check what comes back.
// Each fixture is a real response; where it named other people — their logins,
// pictures and words — those were replaced by placeholders before it was
// committed, keeping every key and every type as the server sent them.

use anirust_api::{Comment, Embedded};
use serde::Deserialize;

/// The paging envelope, as the fixtures carry it.
#[derive(Deserialize)]
struct Paged<T> {
    content: Vec<T>,
    total_count: i64,
}

fn load<T: for<'de> Deserialize<'de>>(name: &str) -> T {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name} does not decode: {e}"))
}

/// `GET release/comment/all/2999/0?sort=3`, the two most popular comments.
///
/// The release they are on is the reason this fixture exists. The server
/// writes it in full under the first comment, with an `@id`, and under every
/// comment after that writes only the number — Jackson's object identity.
/// Both forms have to decode.
#[test]
fn release_comments_carry_their_release_once_and_then_by_reference() {
    let page: Paged<Comment> = load("release_comments.json");
    assert_eq!(page.content.len(), 2);
    assert!(page.total_count > 2);

    let first = &page.content[0];
    assert!(
        matches!(first.release, Some(Embedded::Full(_))),
        "the first carries the release"
    );
    assert!(
        matches!(page.content[1].release, Some(Embedded::Ref(1))),
        "the second points back at it"
    );

    assert!(first.vote_count > 0);
    assert!(first.reply_count > 0);
    assert_eq!(first.profile.login, "user1");
    assert!(!first.is_spoiler);
}

/// `GET schedule`, one release kept per day. Sunday has one more item added by
/// hand: a reference to Monday's release, as the service writes a release it
/// has given already in the same response. It is dropped, not fatal.
#[test]
fn the_schedule_has_seven_days_and_survives_a_repeat() {
    let schedule: anirust_api::Schedule = load("schedule.json");
    let days = schedule.days();
    assert!(
        days.iter().all(|day| day.len() == 1),
        "one release a day, the reference dropped"
    );
    assert!(
        days.iter()
            .all(|day| day[0].id > 0 && !day[0].title().is_empty())
    );
}

/// `POST discover/interesting`, two cards. Every card seen was kind 1, a
/// release id written as a string.
#[test]
fn interesting_cards_lead_to_releases() {
    let page: Paged<anirust_api::Interesting> = load("interesting.json");
    assert_eq!(page.content.len(), 2);
    for card in &page.content {
        assert!(card.release_id().is_some_and(|id| id > 0), "{card:?}");
        assert!(!card.title.is_empty());
    }
}

/// A mixed page of notifications decodes whatever kinds are in it, including
/// one this client has never met. Hand-written: notifications need an
/// account, and these are the shapes the app's classes declare.
#[test]
fn a_page_of_notifications_of_every_kind() {
    let page: Paged<anirust_api::Notification> = serde_json::from_value(serde_json::json!({
        "content": [
            { "type": "episode", "id": 1, "timestamp": 1, "is_new": true,
              "episode": { "name": "13", "release": { "id": 7, "title_ru": "Релиз" },
                           "source": { "name": "Kodik", "type": { "name": "AniLibria" } } } },
            { "type": "friend", "id": 2, "timestamp": 2, "by_profile": { "id": 5, "login": "user5" }, "value": 1 },
            { "type": "article", "id": 3, "timestamp": 3,
              "article": { "id": 9, "channel": { "id": 4, "title": "Канал" }, "payload": { "blocks": [] } } },
            { "type": "something_new", "id": 4, "timestamp": 4, "whatever": { "nested": true } }
        ],
        "total_count": 4
    }))
    .expect("every kind decodes");

    let episode = page.content[0].episode.as_ref().expect("an episode");
    assert_eq!(episode.release.id, 7);
    assert_eq!(episode.source.dubber.name, "AniLibria");
    assert_eq!(page.content[1].by_profile.as_ref().map(|p| p.id), Some(5));
    assert_eq!(
        page.content[2]
            .article
            .as_ref()
            .map(|a| a.channel.title.as_str()),
        Some("Канал")
    );
    assert_eq!(
        page.content[3].kind, "something_new",
        "an unknown kind keeps its id and time"
    );
    assert_eq!(page.content[3].id, 4);
}
