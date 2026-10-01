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

use anirust_api::{
    Client, CommentModeration, CommentSort, CommentTarget, CommentVote, EpisodeSort, Filter,
    FilterSort, ProfileList, SearchBy,
};
use serde_json::{Value, json};
use wiremock::matchers::{body_json, body_string_contains, header, method, path, query_param};
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

// ---- A3: comments, on all three targets ----------------------------------

endpoint! {
    release_comments: "GET" "/release/comment/all/7/0",
    token: true,
    ["sort" = "3"]
    reply: page(),
    call: |c| c.comments(CommentTarget::Release, 7, 0, CommentSort::Popular),
}

endpoint! {
    release_comment: "GET" "/release/comment/55",
    token: true,
    reply: json!({ "code": 0, "id": 55 }),
    call: |c| c.comment(CommentTarget::Release, 55),
}

endpoint! {
    release_comment_replies: "POST" "/release/comment/replies/55/0",
    token: true,
    ["sort" = "2"]
    reply: page(),
    call: |c| c.comment_replies(CommentTarget::Release, 55, 0, CommentSort::Oldest),
}

endpoint! {
    release_profile_comments: "GET" "/release/comment/all/profile/5/0",
    token: true,
    ["sort" = "0"]
    reply: page(),
    call: |c| c.profile_comments(CommentTarget::Release, 5, 0, CommentSort::Newest),
}

endpoint! {
    release_comment_delete: "GET" "/release/comment/delete/55",
    token: true,
    reply: ack(),
    call: |c| c.comment_delete(CommentTarget::Release, 55),
}

endpoint! {
    release_comment_vote: "GET" "/release/comment/vote/55/2",
    token: true,
    reply: ack(),
    call: |c| c.comment_vote(CommentTarget::Release, 55, CommentVote::Up),
}

endpoint! {
    release_comment_votes: "GET" "/release/comment/votes/55/0",
    token: true,
    reply: page(),
    call: |c| c.comment_votes(CommentTarget::Release, 55, 0),
}

endpoint! {
    release_comment_moderate: "POST" "/release/comment/process/55",
    token: true,
    reply: ack(),
    call: |c| c.comment_moderate(CommentTarget::Release, 55, &CommentModeration::default()),
}

endpoint! {
    article_comments: "GET" "/article/comment/all/7/0",
    token: true,
    ["sort" = "3"]
    reply: page(),
    call: |c| c.comments(CommentTarget::Article, 7, 0, CommentSort::Popular),
}

endpoint! {
    article_comment: "GET" "/article/comment/55",
    token: true,
    reply: json!({ "code": 0, "id": 55 }),
    call: |c| c.comment(CommentTarget::Article, 55),
}

endpoint! {
    article_comment_replies: "POST" "/article/comment/replies/55/0",
    token: true,
    ["sort" = "2"]
    reply: page(),
    call: |c| c.comment_replies(CommentTarget::Article, 55, 0, CommentSort::Oldest),
}

endpoint! {
    article_profile_comments: "GET" "/article/comment/all/profile/5/0",
    token: true,
    ["sort" = "0"]
    reply: page(),
    call: |c| c.profile_comments(CommentTarget::Article, 5, 0, CommentSort::Newest),
}

endpoint! {
    article_comment_delete: "GET" "/article/comment/delete/55",
    token: true,
    reply: ack(),
    call: |c| c.comment_delete(CommentTarget::Article, 55),
}

endpoint! {
    article_comment_vote: "GET" "/article/comment/vote/55/2",
    token: true,
    reply: ack(),
    call: |c| c.comment_vote(CommentTarget::Article, 55, CommentVote::Up),
}

endpoint! {
    article_comment_votes: "POST" "/article/comment/votes/55/0",
    token: true,
    reply: page(),
    call: |c| c.comment_votes(CommentTarget::Article, 55, 0),
}

endpoint! {
    article_comment_moderate: "POST" "/article/comment/process/55",
    token: true,
    reply: ack(),
    call: |c| c.comment_moderate(CommentTarget::Article, 55, &CommentModeration::default()),
}

endpoint! {
    collection_comments: "GET" "/collection/comment/all/7/0",
    token: true,
    ["sort" = "3"]
    reply: page(),
    call: |c| c.comments(CommentTarget::Collection, 7, 0, CommentSort::Popular),
}

endpoint! {
    collection_comment: "GET" "/collection/comment/55",
    token: true,
    reply: json!({ "code": 0, "id": 55 }),
    call: |c| c.comment(CommentTarget::Collection, 55),
}

endpoint! {
    collection_comment_replies: "POST" "/collection/comment/replies/55/0",
    token: true,
    ["sort" = "2"]
    reply: page(),
    call: |c| c.comment_replies(CommentTarget::Collection, 55, 0, CommentSort::Oldest),
}

endpoint! {
    collection_profile_comments: "GET" "/collection/comment/all/profile/5/0",
    token: true,
    ["sort" = "0"]
    reply: page(),
    call: |c| c.profile_comments(CommentTarget::Collection, 5, 0, CommentSort::Newest),
}

endpoint! {
    collection_comment_delete: "GET" "/collection/comment/delete/55",
    token: true,
    reply: ack(),
    call: |c| c.comment_delete(CommentTarget::Collection, 55),
}

endpoint! {
    collection_comment_vote: "GET" "/collection/comment/vote/55/2",
    token: true,
    reply: ack(),
    call: |c| c.comment_vote(CommentTarget::Collection, 55, CommentVote::Up),
}

endpoint! {
    collection_comment_votes: "GET" "/collection/comment/votes/55/0",
    token: true,
    reply: page(),
    call: |c| c.comment_votes(CommentTarget::Collection, 55, 0),
}

endpoint! {
    collection_comment_moderate: "POST" "/collection/comment/process/55",
    token: true,
    reply: ack(),
    call: |c| c.comment_moderate(CommentTarget::Collection, 55, &CommentModeration::default()),
}

endpoint! {
    article_comments_popular: "GET" "/article/comment/all/9/popular",
    token: true,
    reply: page(),
    call: |c| c.article_comments_popular(9),
}

/// The body is the request here, so it is checked whole: a reply names both
/// the comment it answers and that comment's author.
#[tokio::test]
async fn comment_add_as_a_reply() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/release/comment/add/7"))
        .and(query_param("token", TOKEN))
        .and(body_json(json!({
            "message": "согласен",
            "spoiler": true,
            "parent_comment_id": 55,
            "reply_to_profile_id": 1001,
        })))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "code": 0, "comment": { "id": 56 } })),
        )
        .expect(1)
        .mount(&server)
        .await;

    let added = client(&server)
        .comment_add(
            CommentTarget::Release,
            7,
            "согласен",
            true,
            Some((55, 1001)),
        )
        .await
        .expect("the reply is the request the server expects");
    assert_eq!(added.id, 56);
}

/// A comment that is not a reply sends both reply fields as null, which is
/// what the app's request object serialises to.
#[tokio::test]
async fn comment_add_on_its_own() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/collection/comment/add/3"))
        .and(body_json(json!({
            "message": "хорошая подборка",
            "spoiler": false,
            "parent_comment_id": null,
            "reply_to_profile_id": null,
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "code": 0, "comment": {} })))
        .expect(1)
        .mount(&server)
        .await;

    client(&server)
        .comment_add(
            CommentTarget::Collection,
            3,
            "хорошая подборка",
            false,
            None,
        )
        .await
        .expect("a top-level comment");
}

#[tokio::test]
async fn comment_edit() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/article/comment/edit/55"))
        .and(body_json(
            json!({ "message": "исправлено", "spoiler": false }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(ack()))
        .expect(1)
        .mount(&server)
        .await;

    client(&server)
        .comment_edit(CommentTarget::Article, 55, "исправлено", false)
        .await
        .expect("an edit");
}

// ---- A4: posts and suggestions -------------------------------------------

endpoint! {
    article: "GET" "/article/9",
    token: true,
    reply: json!({ "code": 0, "article": { "id": 9 } }),
    call: |c| c.article(9),
}

endpoint! {
    article_vote: "GET" "/article/vote/9/2",
    token: true,
    reply: ack(),
    call: |c| c.article_vote(9, CommentVote::Up),
}

endpoint! {
    article_votes: "POST" "/article/votes/9/0",
    token: true,
    ["sort" = "0"]
    reply: page(),
    call: |c| c.article_votes(9, 0, 0),
}

endpoint! {
    article_reposts: "GET" "/article/reposts/9/0",
    token: true,
    ["sort" = "0"]
    reply: page(),
    call: |c| c.article_reposts(9, 0, 0),
}

endpoint! {
    article_mute: "GET" "/article/mute/9",
    token: true,
    reply: ack(),
    call: |c| c.article_mute(9),
}

endpoint! {
    article_unmute: "GET" "/article/unmute/9",
    token: true,
    reply: ack(),
    call: |c| c.article_unmute(9),
}

endpoint! {
    article_pin: "GET" "/article/edit/pinned/9",
    token: true,
    ["is_pinned" = "true"]
    reply: ack(),
    call: |c| c.article_pin(9, true),
}

endpoint! {
    article_delete: "POST" "/article/delete/9",
    token: true,
    reply: ack(),
    call: |c| c.article_delete(9),
}

endpoint! {
    suggestion: "GET" "/article/suggestion/9",
    token: true,
    reply: json!({ "code": 0, "article": {} }),
    call: |c| c.suggestion(9),
}

endpoint! {
    suggestion_delete: "POST" "/article/suggestion/delete/9",
    token: true,
    reply: ack(),
    call: |c| c.suggestion_delete(9),
}

endpoint! {
    suggestion_publish: "POST" "/article/suggestion/publish/9",
    token: true,
    ["is_signed" = "false"]
    reply: json!({ "code": 0, "article": {} }),
    call: |c| c.suggestion_publish(9, false),
}

#[tokio::test]
async fn article_event() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/article/event"))
        .and(body_json(
            json!({ "articles": [9, 10], "type": "VIEW", "entry_point": "FEED" }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(ack()))
        .expect(1)
        .mount(&server)
        .await;

    client(&server)
        .article_event(
            &[9, 10],
            anirust_api::ArticleEventKind::View,
            anirust_api::ArticleEntryPoint::Feed,
        )
        .await
        .expect("a view event");
}

/// The post goes up as a JSON *string* inside the JSON body, which is what
/// the app's `writeValueAsString` produces. Checked by decoding it back.
#[tokio::test]
async fn article_create_sends_the_body_as_a_string() {
    use wiremock::Request;

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/article/create/4"))
        .and(query_param("token", TOKEN))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "code": 0, "article": { "id": 77 } })),
        )
        .expect(1)
        .mount(&server)
        .await;

    let payload = anirust_api::ArticlePayload {
        blocks: vec![anirust_api::ArticleBlock {
            kind: "paragraph".into(),
            data: json!({ "text": "привет" }),
        }],
        ..anirust_api::ArticlePayload::default()
    };
    let created = client(&server)
        .article_create(4, &payload, true, None)
        .await
        .expect("a post");
    assert_eq!(created.id, 77);

    let sent: Vec<Request> = server.received_requests().await.unwrap_or_default();
    let body: serde_json::Value = serde_json::from_slice(&sent[0].body).expect("a JSON body");
    assert_eq!(body["is_signed"], json!(true));
    assert_eq!(body["repost_article_id"], json!(null));
    let inner = body["payload"].as_str().expect("the payload is a string");
    let decoded: serde_json::Value = serde_json::from_str(inner).expect("holding JSON");
    assert_eq!(decoded["blocks"][0]["data"]["text"], json!("привет"));
}

#[tokio::test]
async fn suggestions_name_the_channel_in_the_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/article/suggestion/all/0"))
        .and(body_json(json!({ "channel_id": 4 })))
        .respond_with(ResponseTemplate::new(200).set_body_json(page()))
        .expect(1)
        .mount(&server)
        .await;

    client(&server)
        .suggestions(4, 0)
        .await
        .expect("suggestions");
}
