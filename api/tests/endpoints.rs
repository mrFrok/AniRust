// SPDX-License-Identifier: GPL-3.0-or-later
//
// Every endpoint, checked against a local server.
//
// Each test stands up a server that answers exactly one request — the method,
// the path and the parameters the official client sends — and fails if the
// call sends anything else, or nothing. That is the protocol written down
// once, as a table, and checked on every build without a network or an
// account.
//
// The answers are the smallest body each call accepts, not captured traffic:
// what a response looks like is tested against fixtures where one has been
// captured. These tests are about what goes out.

use anirust_api::{Client, EpisodeSort, Filter, FilterSort, ProfileList, SearchBy};
use serde_json::{Value, json};
use wiremock::matchers::{body_string_contains, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "test-token";

/// A client pointed at the local server, signed in, and never retrying: a
/// retry would turn one wrong request into several and blur the failure.
fn client(server: &MockServer) -> Client {
    Client::builder()
        .base_urls([server.uri()])
        .token(TOKEN)
        .max_retries(0)
        .build()
        .expect("a client for the local server")
}

/// The bare acknowledgement every write answers with.
fn ack() -> Value {
    json!({ "code": 0 })
}

/// An empty page, as every paged listing answers.
fn page() -> Value {
    json!({ "code": 0, "content": [], "current_page": 0, "total_page_count": 0, "total_count": 0 })
}

/// One test per endpoint.
///
/// `token` says whether the request must carry the account's token; the extra
/// `key = value` pairs are query parameters it must also carry.
macro_rules! endpoint {
    (
        $name:ident: $method:literal $path:literal,
        token: $token:literal,
        $([$key:literal = $value:literal])*
        reply: $reply:expr,
        call: |$client:ident| $call:expr $(,)?
    ) => {
        #[tokio::test]
        async fn $name() {
            let server = MockServer::start().await;
            let mut expected = Mock::given(method($method)).and(path($path));
            if $token {
                expected = expected.and(query_param("token", TOKEN));
            }
            $( expected = expected.and(query_param($key, $value)); )*
            expected
                .respond_with(ResponseTemplate::new(200).set_body_json($reply))
                .expect(1)
                .mount(&server)
                .await;

            let $client = client(&server);
            if let Err(error) = $call.await {
                panic!("{} failed against the expected request: {error}", stringify!($name));
            }
        }
    };
}

// ---- playback chain ------------------------------------------------------

endpoint! {
    dubbers: "GET" "/episode/7",
    token: true,
    reply: json!({ "code": 0, "types": [] }),
    call: |c| c.dubbers(7),
}

// The one call in the chain the app makes without a token.
endpoint! {
    sources: "GET" "/episode/7/3",
    token: false,
    reply: json!({ "code": 0, "sources": [] }),
    call: |c| c.sources(7, 3),
}

endpoint! {
    episodes: "GET" "/episode/7/3/11",
    token: true,
    ["sort" = "1"]
    reply: json!({ "code": 0, "episodes": [] }),
    call: |c| c.episodes(7, 3, 11, EpisodeSort::Descending),
}

endpoint! {
    mark_watched: "POST" "/episode/watch/7/11/4",
    token: true,
    reply: ack(),
    call: |c| c.mark_watched(7, 11, 4),
}

endpoint! {
    mark_unwatched: "POST" "/episode/unwatch/7/11/4",
    token: true,
    reply: ack(),
    call: |c| c.mark_unwatched(7, 11, 4),
}

endpoint! {
    history_add: "GET" "/history/add/7/11/4",
    token: true,
    reply: ack(),
    call: |c| c.history_add(7, 11, 4),
}

endpoint! {
    history_delete: "GET" "/history/delete/7",
    token: true,
    reply: ack(),
    call: |c| c.history_delete(7),
}

// ---- releases ------------------------------------------------------------

endpoint! {
    release: "GET" "/release/7",
    token: true,
    ["extended_mode" = "true"]
    reply: json!({ "code": 0, "release": {} }),
    call: |c| c.release(7, true),
}

endpoint! {
    random_release: "GET" "/release/random",
    token: true,
    ["extended_mode" = "false"]
    reply: json!({ "code": 0, "release": {} }),
    call: |c| c.random_release(false),
}

endpoint! {
    search_releases: "POST" "/search/releases/0",
    token: true,
    reply: json!({ "code": 0, "releases": [] }),
    call: |c| c.search_releases("re:zero", SearchBy::Title, 0),
}

endpoint! {
    discover_watching: "POST" "/discover/watching/0",
    token: true,
    reply: page(),
    call: |c| c.discover_watching(0),
}

endpoint! {
    discover_recommendations: "POST" "/discover/recommendations/1",
    token: true,
    ["previous_page" = "0"]
    reply: page(),
    call: |c| c.discover_recommendations(1, 0),
}

endpoint! {
    discover_discussing: "POST" "/discover/discussing",
    token: true,
    reply: page(),
    call: |c| c.discover_discussing(),
}

endpoint! {
    filter: "POST" "/filter/0",
    token: false,
    reply: page(),
    call: |c| c.filter(&Filter::sorted_by(FilterSort::Popularity), 0),
}

// ---- the account ---------------------------------------------------------

endpoint! {
    sign_in: "POST" "/auth/signIn",
    token: false,
    reply: json!({ "code": 0, "profile": {}, "profileToken": { "id": 1, "token": "t" } }),
    call: |c| c.sign_in("login", "password"),
}

endpoint! {
    profile: "GET" "/profile/5",
    token: true,
    reply: json!({ "code": 0, "profile": {} }),
    call: |c| c.profile(5),
}

endpoint! {
    profile_list: "GET" "/profile/list/all/2/0",
    token: true,
    reply: page(),
    call: |c| c.profile_list(ProfileList::Planned, 0, None),
}

endpoint! {
    profile_list_add: "GET" "/profile/list/add/3/7",
    token: true,
    reply: ack(),
    call: |c| c.profile_list_add(ProfileList::Watched, 7),
}

endpoint! {
    profile_list_delete: "GET" "/profile/list/delete/3/7",
    token: true,
    reply: ack(),
    call: |c| c.profile_list_delete(ProfileList::Watched, 7),
}

endpoint! {
    history: "GET" "/history/0",
    token: true,
    reply: page(),
    call: |c| c.history(0),
}

endpoint! {
    favorites: "GET" "/favorite/all/0",
    token: true,
    reply: page(),
    call: |c| c.favorites(0, None),
}

endpoint! {
    favorite_add: "GET" "/favorite/add/7",
    token: true,
    reply: ack(),
    call: |c| c.favorite_add(7),
}

endpoint! {
    favorite_delete: "GET" "/favorite/delete/7",
    token: true,
    reply: ack(),
    call: |c| c.favorite_delete(7),
}

// ---- the feed ------------------------------------------------------------

endpoint! {
    feed: "GET" "/feed/all/0",
    token: true,
    ["date" = "0"]
    reply: page(),
    call: |c| c.feed(0),
}

endpoint! {
    feed_latest: "GET" "/feed/latest/all/0",
    token: true,
    reply: page(),
    call: |c| c.feed_latest(0),
}

endpoint! {
    channel_subscribe: "POST" "/channel/subscribe/9",
    token: true,
    reply: ack(),
    call: |c| c.channel_subscribe(9),
}

endpoint! {
    channel_unsubscribe: "POST" "/channel/unsubscribe/9",
    token: true,
    reply: ack(),
    call: |c| c.channel_unsubscribe(9),
}

// ---- the account's settings ----------------------------------------------

/// The one multipart request so far, so it is checked by hand: the file has to
/// go up as the part the server reads, under its own name, with the empty
/// `name` part the app sends beside it.
#[tokio::test]
async fn avatar_edit() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/profile/preference/avatar/edit"))
        .and(query_param("token", TOKEN))
        .and(body_string_contains("name=\"image\"; filename=\"me.png\""))
        .and(body_string_contains("name=\"name\""))
        .and(body_string_contains("PNGDATA"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ack()))
        .expect(1)
        .mount(&server)
        .await;

    client(&server)
        .avatar_edit("me.png", "image/png", b"PNGDATA".to_vec())
        .await
        .expect("the upload is the request the server expects");
}

endpoint! {
    avatar_delete: "GET" "/profile/preference/avatar/delete",
    token: true,
    reply: ack(),
    call: |c| c.avatar_delete(),
}

// ---- A1: the release, and what can be done to it --------------------------

/// Checked by hand for the header: the franchise listing answers only to the
/// same API version the search does.
#[tokio::test]
async fn related() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/related/42/0"))
        .and(query_param("token", TOKEN))
        .and(header("API-Version", "v2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page()))
        .expect(1)
        .mount(&server)
        .await;

    client(&server)
        .related(42, 0)
        .await
        .expect("the franchise is asked for by its own id, with the version header");
}

endpoint! {
    release_vote: "GET" "/release/vote/add/7/4",
    token: true,
    reply: ack(),
    call: |c| c.release_vote(7, 4),
}

endpoint! {
    release_vote_out_of_range_is_clamped: "GET" "/release/vote/add/7/5",
    token: true,
    reply: ack(),
    call: |c| c.release_vote(7, 9),
}

endpoint! {
    release_vote_delete: "GET" "/release/vote/delete/7",
    token: true,
    reply: ack(),
    call: |c| c.release_vote_delete(7),
}

endpoint! {
    random_favorite: "GET" "/release/random/favorite",
    token: true,
    ["extended_mode" = "false"]
    reply: json!({ "code": 0, "release": {} }),
    call: |c| c.random_favorite(false),
}

endpoint! {
    random_from_list: "GET" "/release/random/profile/list/5/2",
    token: true,
    ["extended_mode" = "true"]
    reply: json!({ "code": 0, "release": {} }),
    call: |c| c.random_from_list(5, ProfileList::Planned, true),
}

endpoint! {
    random_from_collection: "GET" "/release/collection/3/random",
    token: true,
    ["extended_mode" = "false"]
    reply: json!({ "code": 0, "release": {} }),
    call: |c| c.random_from_collection(3, false),
}

endpoint! {
    streaming_platforms: "GET" "/release/streaming/platform/7",
    token: false,
    reply: page(),
    call: |c| c.streaming_platforms(7),
}

endpoint! {
    mark_all_watched: "POST" "/episode/watch/7/11",
    token: true,
    reply: ack(),
    call: |c| c.mark_all_watched(7, 11),
}

endpoint! {
    mark_all_unwatched: "POST" "/episode/unwatch/7/11",
    token: true,
    reply: ack(),
    call: |c| c.mark_all_unwatched(7, 11),
}

endpoint! {
    episode_target: "GET" "/episode/target/7/11/4",
    token: false,
    reply: json!({ "code": 0, "episode": {} }),
    call: |c| c.episode_target(7, 11, 4),
}

endpoint! {
    episode_updates: "GET" "/episode/updates/7/0",
    token: false,
    reply: page(),
    call: |c| c.episode_updates(7, 0),
}

endpoint! {
    all_dubbers: "GET" "/type/all",
    token: true,
    reply: json!({ "code": 0, "types": [] }),
    call: |c| c.all_dubbers(),
}

endpoint! {
    dubber_channel: "GET" "/type/3/channel",
    token: true,
    reply: json!({ "code": 0, "channel": null, "is_widget_eligible": true }),
    call: |c| c.dubber_channel(3),
}

endpoint! {
    dubber_pin: "GET" "/type/pin/7/3",
    token: true,
    reply: ack(),
    call: |c| c.dubber_pin(7, 3),
}

endpoint! {
    dubber_unpin: "GET" "/type/unpin/7/3",
    token: true,
    reply: ack(),
    call: |c| c.dubber_unpin(7, 3),
}

endpoint! {
    dubber_widget_hide: "GET" "/type/widget/hide/3",
    token: true,
    ["permanent" = "true"]
    reply: ack(),
    call: |c| c.dubber_widget_hide(3, true),
}

endpoint! {
    dubber_widget_unhide: "GET" "/type/widget/unhide/3",
    token: true,
    reply: ack(),
    call: |c| c.dubber_widget_unhide(3),
}

endpoint! {
    profile_list_of: "GET" "/profile/list/all/5/1/0",
    token: true,
    reply: page(),
    call: |c| c.profile_list_of(5, ProfileList::Watching, 0, None),
}
