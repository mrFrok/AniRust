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
    pub is_article_suggestion_enabled: bool,
    #[serde(deserialize_with = "nullable")]
    pub is_commenting_enabled: bool,
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
    /// The editor's id for the block, unique within the post.
    #[serde(
        deserialize_with = "nullable",
        skip_serializing_if = "String::is_empty"
    )]
    pub id: String,
    #[serde(rename = "type", deserialize_with = "nullable")]
    pub kind: String,
    /// Kept as it came: its shape depends on `kind`.
    pub data: serde_json::Value,
}

/// The editor's format version, as the official client writes it.
pub const ARTICLE_VERSION: &str = "2.26.5";

impl ArticleBlock {
    fn new(kind: &str, data: serde_json::Value) -> Self {
        Self {
            id: String::new(),
            kind: kind.to_owned(),
            data,
        }
    }

    /// A paragraph of text.
    #[must_use]
    pub fn paragraph(text: &str) -> Self {
        Self::new(
            "paragraph",
            serde_json::json!({ "text": text, "text_length": text.chars().count() }),
        )
    }

    /// A heading; `level` is 1 to 6, as in HTML.
    #[must_use]
    pub fn header(text: &str, level: u8) -> Self {
        Self::new(
            "header",
            serde_json::json!({
                "text": text,
                "level": level.clamp(1, 6),
                "text_length": text.chars().count(),
            }),
        )
    }

    /// A list, numbered or not.
    #[must_use]
    pub fn list(items: &[String], ordered: bool) -> Self {
        Self::new(
            "list",
            serde_json::json!({
                "style": if ordered { "ordered" } else { "unordered" },
                "items": items,
                "item_count": items.len(),
            }),
        )
    }

    /// A quotation, with whom it is by.
    #[must_use]
    pub fn quote(text: &str, caption: &str) -> Self {
        Self::new(
            "quote",
            serde_json::json!({
                "text": text,
                "caption": caption,
                "alignment": "left",
                "text_length": text.chars().count(),
                "caption_length": caption.chars().count(),
            }),
        )
    }

    /// A line between parts of a post.
    #[must_use]
    pub fn delimiter() -> Self {
        Self::new("delimiter", serde_json::json!({}))
    }

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
    #[serde(deserialize_with = "nullable")]
    pub block_count: usize,
}

impl ArticlePayload {
    /// A body of these blocks, written now. Blocks without an id are given
    /// one, since the editor tells blocks apart by it.
    #[must_use]
    pub fn of(blocks: Vec<ArticleBlock>) -> Self {
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| {
                i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
            });
        let blocks: Vec<ArticleBlock> = blocks
            .into_iter()
            .enumerate()
            .map(|(at, mut block)| {
                if block.id.is_empty() {
                    block.id = block_id(time, at);
                }
                block
            })
            .collect();
        Self {
            time,
            version: ARTICLE_VERSION.to_owned(),
            block_count: blocks.len(),
            blocks,
        }
    }
}

/// Ten characters from the stamp and the block's place, in the editor's
/// alphabet: unique within a post, which is all an id has to be.
fn block_id(time: i64, at: usize) -> String {
    const ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ_-";
    let mut n = (time.unsigned_abs() << 8) ^ (at as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    (0..10)
        .map(|_| {
            let c = ALPHABET[(n % 64) as usize] as char;
            n /= 64;
            n ^= 0x0005_DEEC_E66D;
            c
        })
        .collect()
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

    /// The first picture's shape, height over width, when the post says.
    #[must_use]
    pub fn first_image_ratio(&self) -> Option<f32> {
        self.payload
            .blocks
            .iter()
            .filter(|block| block.kind == "media")
            .filter_map(|block| block.data.get("items")?.as_array()?.first().cloned())
            .find_map(|item| {
                let width = item.get("width")?.as_f64()?;
                let height = item.get("height")?.as_f64()?;
                (width > 0.0 && height > 0.0).then(|| (height / width) as f32)
            })
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
            id: String::new(),
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

    #[test]
    fn a_written_post_reads_back_as_it_was_written() {
        let payload = ArticlePayload::of(vec![
            ArticleBlock::header("Анонс", 2),
            ArticleBlock::paragraph("Второй сезон выйдет весной."),
            ArticleBlock::list(&["раз".to_owned(), "два".to_owned()], false),
            ArticleBlock::quote("Скоро.", "студия"),
            ArticleBlock::delimiter(),
        ]);
        assert_eq!(payload.block_count, 5);
        assert_eq!(payload.version, ARTICLE_VERSION);
        let ids: std::collections::HashSet<_> =
            payload.blocks.iter().map(|b| b.id.clone()).collect();
        assert_eq!(ids.len(), 5, "every block has its own id");
        assert!(ids.iter().all(|id| id.len() == 10));

        let text = serde_json::to_string(&payload).unwrap();
        let back: ArticlePayload = serde_json::from_str(&text).unwrap();
        assert_eq!(back.blocks[0].kind, "header");
        assert_eq!(back.blocks[0].data["level"], 2);
        assert_eq!(back.blocks[1].plain_text(), "Второй сезон выйдет весной.");
        assert_eq!(back.blocks[2].plain_text(), "• раз\n• два");
        assert_eq!(back.blocks[3].data["caption"], "студия");
        assert_eq!(back.blocks[4].kind, "delimiter");
    }
}
