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
