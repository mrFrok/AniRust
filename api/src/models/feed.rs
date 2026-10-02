// SPDX-License-Identifier: GPL-3.0-or-later
//
// The feed: posts, the channels they come from, and who wrote them.

use serde::{Deserialize, Serialize};

use crate::serde_ext::nullable;

// ---------------------------------------------------------------------------
// The feed
// ---------------------------------------------------------------------------

/// Who wrote something, as the feed names them: enough to show a face and a
/// name, and nothing that needs a second request.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ProfileSlim {
    #[serde(deserialize_with = "nullable")]
    pub id: i64,
    #[serde(deserialize_with = "nullable")]
    pub login: String,
    #[serde(deserialize_with = "nullable")]
    pub avatar: String,
}

/// A channel a post was published in. Channels are what a feed is
/// subscribed to; a blog is a channel that belongs to one account.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Channel {
    #[serde(deserialize_with = "nullable")]
    pub id: i64,
    #[serde(deserialize_with = "nullable")]
    pub title: String,
    #[serde(deserialize_with = "nullable")]
    pub description: String,
    #[serde(deserialize_with = "nullable")]
    pub avatar: String,
    #[serde(deserialize_with = "nullable")]
    pub is_blog: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_verified: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_subscribed: bool,
    #[serde(deserialize_with = "nullable")]
    pub subscriber_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub article_count: i64,
    /// The account follows the channel but hears nothing of its posts.
    #[serde(deserialize_with = "nullable")]
    pub is_muted: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_creator: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_administrator_or_higher: bool,
}

/// One block of a post's body.
///
/// Posts are written in a block editor, and the server hands the blocks over
/// as they were saved: a `type` and a `data` object whose shape depends on it.
/// This is deliberately not an enum of block kinds. A post with one block of a
/// kind this client has not met should still show its other nine, and a
/// struct that tolerates any `data` is what makes that possible; the accessors
/// below read the fields each kind is known to carry.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct ArticleBlock {
    #[serde(rename = "type", deserialize_with = "nullable")]
    pub kind: String,
    /// Kept as it came: its shape depends on `kind`.
    pub data: serde_json::Value,
}

impl ArticleBlock {
    /// The block's text, with the editor's inline markup taken out.
    ///
    /// Paragraphs, headers and quotes carry `text`; a list carries `items`,
    /// which are joined one to a line. Anything else has no text to give.
    #[must_use]
    pub fn plain_text(&self) -> String {
        match self.kind.as_str() {
            "paragraph" | "header" | "quote" => self
                .data
                .get("text")
                .and_then(serde_json::Value::as_str)
                .map(strip_markup)
                .unwrap_or_default(),
            "list" => self
                .data
                .get("items")
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| {
                            // Items are plain strings in the simple editor and
                            // objects with `content` in the nested one.
                            item.as_str()
                                .or_else(|| item.get("content").and_then(|c| c.as_str()))
                        })
                        .map(|item| format!("• {}", strip_markup(item)))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default(),
            _ => String::new(),
        }
    }

    /// Image URLs a media block carries, in order.
    #[must_use]
    pub fn media_urls(&self) -> Vec<String> {
        if self.kind != "media" {
            return Vec::new();
        }
        self.data
            .get("items")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.get("url").and_then(serde_json::Value::as_str))
                    .filter(|url| !url.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// A post's body.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct ArticlePayload {
    /// When the body was written, in milliseconds — the editor's own stamp.
    #[serde(deserialize_with = "nullable")]
    pub time: i64,
    /// The editor's format version, sent back as it came.
    #[serde(deserialize_with = "nullable")]
    pub version: String,
    #[serde(deserialize_with = "nullable")]
    pub blocks: Vec<ArticleBlock>,
}

/// A post in the feed.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Article {
    #[serde(deserialize_with = "nullable")]
    pub id: i64,
    #[serde(deserialize_with = "nullable")]
    pub channel: Channel,
    #[serde(deserialize_with = "nullable")]
    pub author: ProfileSlim,
    #[serde(deserialize_with = "nullable")]
    pub payload: ArticlePayload,
    /// Seconds since the epoch.
    #[serde(deserialize_with = "nullable")]
    pub creation_date: i64,
    #[serde(deserialize_with = "nullable")]
    pub comment_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub repost_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub vote_count: i64,
    #[serde(deserialize_with = "nullable")]
    pub is_pinned: bool,
    /// This account's vote on it: 0 none, 1 down, 2 up — the scale every vote
    /// in the service shares.
    #[serde(deserialize_with = "nullable")]
    pub vote: i32,
    #[serde(deserialize_with = "nullable")]
    pub is_muted: bool,
    /// Published with the author's name rather than only the channel's.
    #[serde(deserialize_with = "nullable")]
    pub is_signed: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_deleted: bool,
    /// The post this one reposts, when it is a repost.
    #[serde(deserialize_with = "nullable")]
    pub repost_article: Option<Box<Article>>,
}

impl Article {
    /// The post's text, block by block, a blank line between them.
    #[must_use]
    pub fn plain_text(&self) -> String {
        self.payload
            .blocks
            .iter()
            .map(ArticleBlock::plain_text)
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// The first picture in the post, which is the one a feed shows.
    #[must_use]
    pub fn first_image(&self) -> Option<String> {
        self.payload
            .blocks
            .iter()
            .flat_map(ArticleBlock::media_urls)
            .next()
    }
}

/// Text with the editor's inline HTML removed and its entities decoded.
///
/// The editor stores bold, italics and links as tags inside the text. A
/// line break becomes a newline; every other tag is dropped and its contents
/// kept. Only the handful of entities an editor actually emits are decoded —
/// this is not an HTML parser and does not need to be one.
fn strip_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('>') else {
            // An unclosed `<` is a less-than sign, not a tag.
            out.push_str(&rest[open..]);
            rest = "";
            break;
        };
        let tag = rest[open + 1..open + close].trim().to_ascii_lowercase();
        if tag.starts_with("br") {
            out.push('\n');
        }
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);

    out.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod feed_tests {
    use super::*;

    fn block(kind: &str, data: serde_json::Value) -> ArticleBlock {
        ArticleBlock {
            kind: kind.to_owned(),
            data,
        }
    }

    #[test]
    fn inline_markup_is_taken_out_and_its_text_kept() {
        assert_eq!(
            strip_markup("<b>Re:Zero</b> &amp; <a href=\"x\">клип</a>"),
            "Re:Zero & клип"
        );
    }

    #[test]
    fn a_line_break_survives_as_one() {
        assert_eq!(strip_markup("раз<br>два<br/>три"), "раз\nдва\nтри");
    }

    #[test]
    fn a_lone_less_than_sign_is_not_a_tag() {
        assert_eq!(strip_markup("1 < 2"), "1 < 2");
    }

    #[test]
    fn a_list_is_one_item_to_a_line() {
        let list = block(
            "list",
            serde_json::json!({ "items": ["a", { "content": "<i>b</i>" }] }),
        );
        assert_eq!(list.plain_text(), "• a\n• b");
    }

    #[test]
    fn an_unknown_block_has_no_text_and_spoils_nothing() {
        let article = Article {
            payload: ArticlePayload {
                blocks: vec![
                    block("paragraph", serde_json::json!({ "text": "первый" })),
                    block("poll", serde_json::json!({ "question": "?" })),
                    block(
                        "header",
                        serde_json::json!({ "text": "второй", "level": 2 }),
                    ),
                ],
                ..ArticlePayload::default()
            },
            ..Article::default()
        };
        assert_eq!(article.plain_text(), "первый\n\nвторой");
    }

    #[test]
    fn the_first_picture_is_the_first_media_url() {
        let article = Article {
            payload: ArticlePayload {
                blocks: vec![
                    block("paragraph", serde_json::json!({ "text": "x" })),
                    block(
                        "media",
                        serde_json::json!({ "items": [{ "url": "" }, { "url": "https://a/1.jpg" }] }),
                    ),
                ],
                ..ArticlePayload::default()
            },
            ..Article::default()
        };
        assert_eq!(article.first_image().as_deref(), Some("https://a/1.jpg"));
    }

    #[test]
    fn a_post_with_nulls_where_values_belong_still_reads() {
        let article: Article = serde_json::from_str(
            r#"{"id":7,"channel":null,"author":null,"payload":{"blocks":null},"creation_date":null}"#,
        )
        .expect("nulls degrade into defaults");
        assert_eq!(article.id, 7);
        assert!(article.payload.blocks.is_empty());
    }
}
