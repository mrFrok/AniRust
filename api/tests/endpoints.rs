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

// ---- A2: discovery, schedule, config, search -----------------------------

endpoint! {
    discover_interesting: "POST" "/discover/interesting",
    token: false,
    reply: page(),
    call: |c| c.discover_interesting(),
}

endpoint! {
    discover_comments: "POST" "/discover/comments",
    token: false,
    reply: page(),
    call: |c| c.discover_comments(),
}

endpoint! {
    schedule: "GET" "/schedule",
    token: false,
    reply: json!({ "code": 0, "monday": [] }),
    call: |c| c.schedule(),
}

endpoint! {
    config_player: "GET" "/config/anixplayer",
    token: false,
    reply: json!({ "code": 0 }),
    call: |c| c.config_player(),
}

endpoint! {
    config_toggles: "GET" "/config/toggles",
    token: true,
    ["version_code" = "25100"] ["is_beta" = "false"] ["is_api_alt" = "false"]
    reply: json!({ "code": 0 }),
    call: |c| c.config_toggles(25100, false, false),
}

endpoint! {
    config_urls: "GET" "/config/urls",
    token: true,
    ["version_code" = "25100"] ["is_beta" = "false"]
    reply: json!({ "code": 0 }),
    call: |c| c.config_urls(25100, false),
}

endpoint! {
    search_profiles: "POST" "/search/profiles/0",
    token: true,
    reply: page(),
    call: |c| c.search_profiles("mrfrok", 0),
}

endpoint! {
    search_articles: "POST" "/search/articles/0",
    token: true,
    reply: page(),
    call: |c| c.search_articles("re:zero", None, 0),
}

endpoint! {
    search_channels: "POST" "/search/channels/0",
    token: true,
    reply: page(),
    call: |c| c.search_channels("news", &anirust_api::ChannelSearch::default(), 0),
}

endpoint! {
    search_subscribers: "POST" "/search/channel/4/subscribers/0",
    token: true,
    reply: page(),
    call: |c| c.search_subscribers(4, "a", 0),
}

endpoint! {
    search_collections: "POST" "/search/collections/0",
    token: true,
    reply: page(),
    call: |c| c.search_collections("isekai", 0),
}

endpoint! {
    search_favorite_collections: "POST" "/search/favoriteCollections/0",
    token: true,
    reply: page(),
    call: |c| c.search_favorite_collections("", 0),
}

endpoint! {
    search_profile_collections: "POST" "/search/profileCollections/5/0",
    token: true,
    ["release_id" = "7"]
    reply: page(),
    call: |c| c.search_profile_collections(5, 7, "", 0),
}

endpoint! {
    search_favorites: "POST" "/search/favorites/0",
    token: true,
    reply: page(),
    call: |c| c.search_favorites("re", 0),
}

endpoint! {
    search_history: "POST" "/search/history/0",
    token: true,
    reply: page(),
    call: |c| c.search_history("re", 0),
}

endpoint! {
    search_list: "POST" "/search/profile/list/2/0",
    token: true,
    reply: page(),
    call: |c| c.search_list(ProfileList::Planned, "re", 0),
}

endpoint! {
    search_feed: "POST" "/search/feed/0",
    token: true,
    reply: json!({ "code": 0, "articles": { "content": [] }, "channels": null }),
    call: |c| c.search_feed("re", 0),
}

/// A post search names the channel in the body, 0 meaning anywhere.
#[tokio::test]
async fn search_articles_in_one_channel() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search/articles/0"))
        .and(body_json(json!({ "query": "клип", "channel_id": 4 })))
        .respond_with(ResponseTemplate::new(200).set_body_json(page()))
        .expect(1)
        .mount(&server)
        .await;

    client(&server)
        .search_articles("клип", Some(4), 0)
        .await
        .expect("a search");
}

// ---- A8: notifications and what brings them ------------------------------

endpoint! {
    notifications_all: "GET" "/notification/all/0",
    token: true,
    reply: page(),
    call: |c| c.notifications(anirust_api::NotificationKind::All, 0),
}

endpoint! {
    notifications_episodes: "GET" "/notification/episodes/0",
    token: true,
    reply: page(),
    call: |c| c.notifications(anirust_api::NotificationKind::Episodes, 0),
}

endpoint! {
    notifications_friends: "GET" "/notification/friends/0",
    token: true,
    reply: page(),
    call: |c| c.notifications(anirust_api::NotificationKind::Friends, 0),
}

endpoint! {
    notifications_release_comments: "GET" "/notification/releaseComments/0",
    token: true,
    reply: page(),
    call: |c| c.notifications(anirust_api::NotificationKind::ReleaseComments, 0),
}

endpoint! {
    notifications_related_releases: "GET" "/notification/related/release/0",
    token: true,
    reply: page(),
    call: |c| c.notifications(anirust_api::NotificationKind::RelatedReleases, 0),
}

endpoint! {
    notifications_articles: "GET" "/notification/articles/0",
    token: true,
    reply: page(),
    call: |c| c.notifications(anirust_api::NotificationKind::Articles, 0),
}

endpoint! {
    notifications_article_comments: "GET" "/notification/article/comments/0",
    token: true,
    reply: page(),
    call: |c| c.notifications(anirust_api::NotificationKind::ArticleComments, 0),
}

endpoint! {
    notifications_collection_comments: "GET" "/notification/collectionComments/0",
    token: true,
    reply: page(),
    call: |c| c.notifications(anirust_api::NotificationKind::CollectionComments, 0),
}

endpoint! {
    notification_delete_episode: "GET" "/notification/episode/delete/31",
    token: true,
    reply: ack(),
    call: |c| c.notification_delete(anirust_api::NotificationDelete::Episode, 31),
}

endpoint! {
    notification_delete_friend: "GET" "/notification/friend/delete/31",
    token: true,
    reply: ack(),
    call: |c| c.notification_delete(anirust_api::NotificationDelete::Friend, 31),
}

endpoint! {
    notification_delete_release_comment: "GET" "/notification/releaseComment/delete/31",
    token: true,
    reply: ack(),
    call: |c| c.notification_delete(anirust_api::NotificationDelete::ReleaseComment, 31),
}

endpoint! {
    notification_delete_related_release: "GET" "/notification/related/release/delete/31",
    token: true,
    reply: ack(),
    call: |c| c.notification_delete(anirust_api::NotificationDelete::RelatedRelease, 31),
}

endpoint! {
    notification_delete_article_comment: "GET" "/notification/article/comment/delete/31",
    token: true,
    reply: ack(),
    call: |c| c.notification_delete(anirust_api::NotificationDelete::ArticleComment, 31),
}

endpoint! {
    notification_delete_collection_comment: "GET" "/notification/collectionComment/delete/31",
    token: true,
    reply: ack(),
    call: |c| c.notification_delete(anirust_api::NotificationDelete::CollectionComment, 31),
}

endpoint! {
    notification_delete_my_article_comment: "GET" "/notification/my/article/comment/delete/31",
    token: true,
    reply: ack(),
    call: |c| c.notification_delete(anirust_api::NotificationDelete::MyArticleComment, 31),
}

endpoint! {
    notification_delete_my_collection_comment: "GET" "/notification/my/collection/comment/delete/31",
    token: true,
    reply: ack(),
    call: |c| c.notification_delete(anirust_api::NotificationDelete::MyCollectionComment, 31),
}

endpoint! {
    notification_switch_episodes: "GET" "/profile/preference/notification/episode/edit",
    token: true,
    reply: ack(),
    call: |c| c.notification_switch(anirust_api::NotificationSwitch::Episodes),
}

endpoint! {
    notification_switch_first_episode: "GET" "/profile/preference/notification/episode/first/edit",
    token: true,
    reply: ack(),
    call: |c| c.notification_switch(anirust_api::NotificationSwitch::FirstEpisode),
}

endpoint! {
    notification_switch_comments: "GET" "/profile/preference/notification/comment/edit",
    token: true,
    reply: ack(),
    call: |c| c.notification_switch(anirust_api::NotificationSwitch::Comments),
}

endpoint! {
    notification_switch_related_releases: "GET" "/profile/preference/notification/related/release/edit",
    token: true,
    reply: ack(),
    call: |c| c.notification_switch(anirust_api::NotificationSwitch::RelatedReleases),
}

endpoint! {
    notification_switch_articles: "GET" "/profile/preference/notification/article/edit",
    token: true,
    reply: ack(),
    call: |c| c.notification_switch(anirust_api::NotificationSwitch::Articles),
}

endpoint! {
    notification_switch_my_article_comments: "GET" "/profile/preference/notification/my/article/comment/edit",
    token: true,
    reply: ack(),
    call: |c| c.notification_switch(anirust_api::NotificationSwitch::MyArticleComments),
}

endpoint! {
    notification_switch_my_collection_comments: "GET" "/profile/preference/notification/my/collection/comment/edit",
    token: true,
    reply: ack(),
    call: |c| c.notification_switch(anirust_api::NotificationSwitch::MyCollectionComments),
}

endpoint! {
    notification_switch_selected_releases: "GET" "/profile/preference/notification/selected/releases/edit",
    token: true,
    reply: ack(),
    call: |c| c.notification_switch(anirust_api::NotificationSwitch::SelectedReleases),
}

endpoint! {
    notification_switch_report_outcomes: "GET" "/profile/preference/notification/report/process/edit",
    token: true,
    reply: ack(),
    call: |c| c.notification_switch(anirust_api::NotificationSwitch::ReportOutcomes),
}

endpoint! {
    notification_count: "GET" "/notification/count",
    token: true,
    reply: json!({ "code": 0, "count": 3 }),
    call: |c| c.notification_count(),
}

endpoint! {
    notifications_read: "GET" "/notification/read",
    token: true,
    reply: ack(),
    call: |c| c.notifications_read(),
}

endpoint! {
    notifications_delete_all: "GET" "/notification/delete/all",
    token: true,
    reply: ack(),
    call: |c| c.notifications_delete_all(),
}

endpoint! {
    notification_preferences: "GET" "/profile/preference/notification/my",
    token: true,
    reply: json!({ "code": 0, "is_episode_notifications_enabled": true }),
    call: |c| c.notification_preferences(),
}

endpoint! {
    notification_releases: "GET" "/profile/preference/notification/release/all/0",
    token: true,
    reply: page(),
    call: |c| c.notification_releases(0),
}

endpoint! {
    notification_release_dubbers: "GET" "/profile/preference/notification/release/type/7",
    token: true,
    reply: json!({ "code": 0, "profile_release_type_notification_preferences": [{ "type": { "id": 3 } }] }),
    call: |c| c.notification_release_dubbers(7),
}

#[tokio::test]
async fn notification_preference_bodies() {
    let server = MockServer::start().await;
    for (route, body) in [
        (
            "/profile/preference/notification/release/type/edit",
            json!({ "release_id": 7, "profile_release_type_notification_preferences": [3, 4] }),
        ),
        (
            "/profile/preference/notification/status/edit",
            json!({ "profile_status_notification_preferences": [1, 2] }),
        ),
        (
            "/profile/preference/notification/type/edit",
            json!({ "profile_type_notification_preferences": [3] }),
        ),
    ] {
        Mock::given(method("POST"))
            .and(path(route))
            .and(query_param("token", TOKEN))
            .and(body_json(body))
            .respond_with(ResponseTemplate::new(200).set_body_json(ack()))
            .expect(1)
            .mount(&server)
            .await;
    }

    let c = client(&server);
    c.notification_release_dubbers_edit(7, &[3, 4])
        .await
        .expect("per release");
    c.notification_statuses_edit(&[1, 2])
        .await
        .expect("by list");
    c.notification_dubbers_edit(&[3])
        .await
        .expect("by voice-over");
}

// ---- A6: people ------------------------------------------------------------

endpoint! {
    profile_info: "GET" "/profile/info",
    token: true,
    reply: json!({ "code": 0, "rating_score": 5 }),
    call: |c| c.profile_info(),
}

endpoint! {
    profile_socials: "GET" "/profile/social/5",
    token: true,
    reply: json!({ "code": 0, "tg_page": "x" }),
    call: |c| c.profile_socials(5),
}

endpoint! {
    login_history: "GET" "/profile/login/history/all/5/0",
    token: true,
    reply: page(),
    call: |c| c.login_history(5, 0),
}

endpoint! {
    profile_moderate: "POST" "/profile/process/5",
    token: true,
    reply: ack(),
    call: |c| c.profile_moderate(5, false, None, None),
}

endpoint! {
    friends: "GET" "/profile/friend/all/5/0",
    token: true,
    reply: page(),
    call: |c| c.friends(5, 0),
}

endpoint! {
    friend_recommendations: "GET" "/profile/friend/recommendations",
    token: true,
    reply: page(),
    call: |c| c.friend_recommendations(),
}

endpoint! {
    friend_requests_in: "GET" "/profile/friend/requests/in/0",
    token: true,
    reply: page(),
    call: |c| c.friend_requests_in(0),
}

endpoint! {
    friend_requests_in_last: "GET" "/profile/friend/requests/in/last",
    token: true,
    ["count" = "3"]
    reply: page(),
    call: |c| c.friend_requests_in_last(3),
}

endpoint! {
    friend_requests_out: "GET" "/profile/friend/requests/out/0",
    token: true,
    reply: page(),
    call: |c| c.friend_requests_out(0),
}

endpoint! {
    friend_requests_out_last: "GET" "/profile/friend/requests/out/last",
    token: true,
    ["count" = "3"]
    reply: page(),
    call: |c| c.friend_requests_out_last(3),
}

endpoint! {
    friend_request_hide: "GET" "/profile/friend/request/hide/5",
    token: true,
    reply: ack(),
    call: |c| c.friend_request_hide(5),
}

endpoint! {
    rated_releases: "GET" "/profile/vote/release/voted/5/0",
    token: true,
    reply: page(),
    call: |c| c.rated_releases(5, 0, None),
}

endpoint! {
    unrated_releases: "GET" "/profile/vote/release/unvoted/0",
    token: true,
    reply: page(),
    call: |c| c.unrated_releases(0),
}

endpoint! {
    unrated_releases_last: "GET" "/profile/vote/release/unvoted/last",
    token: true,
    reply: page(),
    call: |c| c.unrated_releases_last(),
}

endpoint! {
    badges: "GET" "/profile/preference/badge/all/0",
    token: true,
    reply: json!({ "code": 0, "content": [{ "id": 1, "name": "b", "image_url": "u", "type": 1 }], "profile": {} }),
    call: |c| c.badges(0),
}

endpoint! {
    badge_wear: "GET" "/profile/preference/badge/edit/1",
    token: true,
    reply: ack(),
    call: |c| c.badge_wear(1),
}

endpoint! {
    badge_remove: "GET" "/profile/preference/badge/remove",
    token: true,
    reply: ack(),
    call: |c| c.badge_remove(),
}

endpoint! {
    blocked: "GET" "/profile/blocklist/all/0",
    token: true,
    reply: page(),
    call: |c| c.blocked(0),
}

endpoint! {
    unblock: "GET" "/profile/blocklist/remove/5",
    token: true,
    reply: ack(),
    call: |c| c.unblock(5),
}

endpoint! {
    role_holders: "GET" "/role/all/0/3",
    token: true,
    reply: page(),
    call: |c| c.role_holders(3, 0),
}

/// The friend endpoints answer success with codes of their own. Each code is
/// served in turn and read back as what it means.
#[tokio::test]
async fn friend_requests_read_their_own_codes() {
    use anirust_api::FriendOutcome;

    async fn answer(route: &str, code: i32) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(route))
            .and(query_param("token", TOKEN))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "code": code })))
            .mount(&server)
            .await;
        server
    }

    let sent = answer("/profile/friend/request/send/5", 3).await;
    assert_eq!(
        client(&sent).friend_request_send(5).await.ok(),
        Some(FriendOutcome::Sent)
    );

    let confirmed = answer("/profile/friend/request/send/5", 2).await;
    assert_eq!(
        client(&confirmed).friend_request_send(5).await.ok(),
        Some(FriendOutcome::Confirmed)
    );

    let limit = answer("/profile/friend/request/send/5", 6).await;
    assert!(
        client(&limit).friend_request_send(5).await.is_err(),
        "a limit is a refusal"
    );

    let unfriended = answer("/profile/friend/request/remove/5", 3).await;
    assert_eq!(
        client(&unfriended).friend_request_remove(5).await.ok(),
        Some(FriendOutcome::FriendshipRemoved)
    );

    let already = answer("/profile/blocklist/add/5", 2).await;
    assert!(
        client(&already).block(5).await.is_ok(),
        "already blocked is what was asked for"
    );
}

// ---- A5: channels -----------------------------------------------------------

endpoint! {
    channel: "GET" "/channel/4",
    token: true,
    reply: json!({ "code": 0, "channel": { "id": 4 } }),
    call: |c| c.channel(4),
}

endpoint! {
    blog: "GET" "/channel/blog/5",
    token: true,
    reply: json!({ "code": 0, "channel": {} }),
    call: |c| c.blog(5),
}

endpoint! {
    channel_articles: "POST" "/channel/4/article/all/0",
    token: true,
    reply: page(),
    call: |c| c.channel_articles(4, 0),
}

endpoint! {
    channels: "POST" "/channel/all/0",
    token: true,
    reply: page(),
    call: |c| c.channels(&anirust_api::ChannelFilter::default(), 0),
}

endpoint! {
    channel_recommendations: "GET" "/channel/recommendations/0",
    token: true,
    ["is_blog" = "false"]
    reply: page(),
    call: |c| c.channel_recommendations(0, Some(false), None),
}

endpoint! {
    subscriptions: "GET" "/channel/subscription/all/0",
    token: true,
    ["sort" = "1"]
    reply: page(),
    call: |c| c.subscriptions(0, 1),
}

endpoint! {
    subscription_count: "GET" "/channel/subscription/count",
    token: true,
    reply: json!({ "code": 0, "subscription_count": 4 }),
    call: |c| c.subscription_count(),
}

endpoint! {
    channel_mute: "POST" "/channel/mute/4",
    token: true,
    reply: ack(),
    call: |c| c.channel_mute(4),
}

endpoint! {
    channel_unmute: "POST" "/channel/unmute/4",
    token: true,
    reply: ack(),
    call: |c| c.channel_unmute(4),
}

endpoint! {
    muted_channels: "GET" "/channel/mute/all/0",
    token: true,
    reply: page(),
    call: |c| c.muted_channels(0),
}

endpoint! {
    channel_create: "POST" "/channel/create",
    token: true,
    reply: json!({ "code": 0, "channel": { "id": 8 } }),
    call: |c| c.channel_create(&anirust_api::ChannelSettings::default()),
}

endpoint! {
    blog_create: "POST" "/channel/blog/create",
    token: true,
    reply: json!({ "code": 0, "channel": {} }),
    call: |c| c.blog_create(),
}

endpoint! {
    channel_edit: "POST" "/channel/edit/4",
    token: true,
    reply: json!({ "code": 0, "channel": {} }),
    call: |c| c.channel_edit(4, &anirust_api::ChannelSettings::default()),
}

endpoint! {
    channel_cover_delete: "POST" "/channel/cover/delete/4",
    token: true,
    reply: ack(),
    call: |c| c.channel_cover_delete(4),
}

endpoint! {
    editor_available: "GET" "/channel/4/editor/available",
    token: true,
    ["is_suggestion" = "false"] ["is_edit_mode" = "true"]
    reply: json!({ "code": 0, "media_upload_token": "t" }),
    call: |c| c.editor_available(4, false, true),
}

endpoint! {
    editor_channels: "GET" "/channel/editor/available/all",
    token: true,
    reply: json!({ "code": 0, "channels": [] }),
    call: |c| c.editor_channels(None),
}

endpoint! {
    channel_members: "POST" "/channel/4/permission/all/0",
    token: true,
    reply: page(),
    call: |c| c.channel_members(4, 1, 0),
}

endpoint! {
    channel_blocked: "GET" "/channel/4/block/all/0",
    token: true,
    reply: page(),
    call: |c| c.channel_blocked(4, 0),
}

endpoint! {
    channel_block: "GET" "/channel/4/block/5",
    token: true,
    reply: json!({ "code": 0, "channel_block": null }),
    call: |c| c.channel_block(4, 5),
}

#[tokio::test]
async fn channel_management_bodies() {
    let server = MockServer::start().await;
    for (route, body) in [
        (
            "/channel/4/permission/manage",
            json!({ "target_profile_id": 5, "permission": 2 }),
        ),
        (
            "/channel/4/block/manage",
            json!({
                "target_profile_id": 5, "is_blocked": true, "is_perm_blocked": false,
                "reason": "спам", "is_reason_showing_enabled": true, "expire_date": null
            }),
        ),
        (
            "/channel/create",
            json!({
                "title": "Канал", "description": "", "is_commenting_enabled": true,
                "is_article_suggestion_enabled": false, "is_episode_channel_widget_enabled": null,
                "episode_channel_widget_article_count": null,
                "episode_channel_widget_popularity_period": null,
                "episode_channel_widget_sort": null
            }),
        ),
    ] {
        Mock::given(method("POST"))
            .and(path(route))
            .and(body_json(body))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "code": 0, "channel": {} })),
            )
            .expect(1)
            .mount(&server)
            .await;
    }
    let c = client(&server);
    c.channel_permission(4, 5, Some(2))
        .await
        .expect("a permission");
    c.channel_block_manage(
        4,
        &anirust_api::ChannelBlockRequest {
            target_profile_id: 5,
            is_blocked: true,
            reason: "спам".into(),
            is_reason_showing_enabled: true,
            ..Default::default()
        },
    )
    .await
    .expect("a block");
    c.channel_create(&anirust_api::ChannelSettings {
        title: "Канал".into(),
        is_commenting_enabled: true,
        ..Default::default()
    })
    .await
    .expect("a channel");
}

#[tokio::test]
async fn channel_pictures_go_up_as_the_image_part() {
    let server = MockServer::start().await;
    for route in ["/channel/avatar/upload/4", "/channel/cover/upload/4"] {
        Mock::given(method("POST"))
            .and(path(route))
            .and(body_string_contains("name=\"image\"; filename=\"pic.png\""))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "code": 0, "url": "https://x/y.png" })),
            )
            .expect(1)
            .mount(&server)
            .await;
    }
    let c = client(&server);
    let url = c
        .channel_avatar_upload(4, "pic.png", "image/png", b"P".to_vec())
        .await
        .expect("an avatar");
    assert_eq!(url, "https://x/y.png");
    c.channel_cover_upload(4, "pic.png", "image/png", b"P".to_vec())
        .await
        .expect("a cover");
}
