// SPDX-License-Identifier: GPL-3.0-or-later

//! Writing posts: a new one in the account's blog or a channel it runs, a
//! change to one of its own, or a suggestion to someone else's channel.
//!
//! The editor writes the blocks the official one does — paragraphs,
//! headings, lists, quotes, lines between them. Pictures and embeds go up
//! through an upload this client has no way into, so it does not add them;
//! a post that already has some keeps them as they were when it is changed.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, VecModel};

use anirust_api::{Article, ArticleBlock, ArticlePayload, Channel, Client};

use crate::{DraftBlock, MainWindow, PickerOption, tasks};

/// What the editor is doing.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Mode {
    #[default]
    New,
    /// Changing a post: its id.
    Edit(i64),
    /// Suggesting a post to a channel: its id.
    Suggest(i64),
}

/// One block being written. Kinds this editor does not write are carried
/// whole, so changing a post does not lose them.
#[derive(Clone, Default)]
struct Draft {
    kind: String,
    text: String,
    caption: String,
    kept: Option<ArticleBlock>,
}

#[derive(Default)]
pub struct EditorState {
    mode: Mode,
    blocks: Vec<Draft>,
    /// Where a new post can go, and which of them is chosen.
    channels: Vec<Channel>,
    channel: usize,
}

/// Everything the editor's buttons need.
pub struct Context<'a> {
    pub window: &'a MainWindow,
    pub state: &'a Rc<RefCell<EditorState>>,
    pub client: &'a Rc<Client>,
    /// The account's profile id.
    pub me: i64,
}

fn show(window: &MainWindow, state: &EditorState) {
    let blocks: Vec<DraftBlock> = state
        .blocks
        .iter()
        .map(|d| DraftBlock {
            kind: d.kind.as_str().into(),
            text: d.text.as_str().into(),
            caption: d.caption.as_str().into(),
        })
        .collect();
    window.set_editor_blocks(slint::ModelRc::new(VecModel::from(blocks)));
}

fn begin(cx: &Context<'_>, mode: Mode, blocks: Vec<Draft>) {
    {
        let mut state = cx.state.borrow_mut();
        state.mode = mode;
        state.blocks = blocks;
        state.channels.clear();
        state.channel = 0;
        show(cx.window, &state);
    }
    let window = cx.window;
    window.set_editor_editing(matches!(mode, Mode::Edit(_)));
    window.set_editor_suggesting(matches!(mode, Mode::Suggest(_)));
    window.set_editor_error("".into());
    window.set_editor_busy(false);
    window.set_editor_signed(false);
    window.set_editor_can_sign(false);
    window.set_editor_open(true);
}

/// A new post. Offers the account's blog first, then the channels it may
/// write in; on a channel's page that the account writes in, that one.
pub fn open_new(cx: &Context<'_>, on_channel: Option<Channel>) {
    if let Some(channel) = on_channel.as_ref()
        && !channel.is_creator
        && !channel.is_administrator_or_higher
    {
        // Not the account's to post in: a suggestion, if the channel takes them.
        if channel.is_article_suggestion_enabled {
            begin(cx, Mode::Suggest(channel.id), vec![paragraph()]);
        }
        return;
    }
    begin(cx, Mode::New, vec![paragraph()]);
    cx.window.set_editor_loading(true);
    cx.window
        .set_editor_channels(slint::ModelRc::new(VecModel::<PickerOption>::default()));

    let weak = cx.window.as_weak();
    let state = Rc::clone(cx.state);
    let api = (**cx.client).clone();
    let me = cx.me;
    tasks::spawn(
        async move {
            let blog = api.blog(me).await.ok().filter(|b| b.id > 0);
            let mut channels: Vec<Channel> = blog.into_iter().collect();
            for channel in api.editor_channels(None).await.unwrap_or_default() {
                if channels.iter().all(|c| c.id != channel.id) {
                    channels.push(channel);
                }
            }
            channels
        },
        move |channels| {
            let Some(window) = weak.upgrade() else { return };
            window.set_editor_loading(false);
            let wanted = on_channel.map(|c| c.id);
            let chosen = wanted
                .and_then(|id| channels.iter().position(|c| c.id == id))
                .unwrap_or(0);
            let options: Vec<PickerOption> = channels
                .iter()
                .map(|c| PickerOption {
                    label: c.title.as_str().into(),
                    ..PickerOption::default()
                })
                .collect();
            window.set_editor_channels(slint::ModelRc::new(VecModel::from(options)));
            window.set_editor_channel(i32::try_from(chosen).unwrap_or(0));
            window.set_editor_can_sign(channels.get(chosen).is_some_and(|c| !c.is_blog));
            if channels.is_empty() {
                window.set_editor_error("no-channel".into());
            }
            let mut state = state.borrow_mut();
            state.channels = channels;
            state.channel = chosen;
        },
    );
}

/// Changes one of the account's posts.
pub fn open_edit(cx: &Context<'_>, article: &Article) {
    let blocks = article.payload.blocks.iter().map(draft_of).collect();
    begin(cx, Mode::Edit(article.id), blocks);
    cx.window.set_editor_can_sign(!article.channel.is_blog);
    cx.window.set_editor_signed(article.is_signed);
}

pub fn select_channel(cx: &Context<'_>, index: usize) {
    let mut state = cx.state.borrow_mut();
    if let Some(channel) = state.channels.get(index) {
        cx.window
            .set_editor_channel(i32::try_from(index).unwrap_or(0));
        cx.window.set_editor_can_sign(!channel.is_blog);
        state.channel = index;
    }
}

fn paragraph() -> Draft {
    Draft {
        kind: "paragraph".to_owned(),
        ..Draft::default()
    }
}

/// A stored block as the editor holds it.
fn draft_of(block: &ArticleBlock) -> Draft {
    let text = |key: &str| {
        block
            .data
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    match block.kind.as_str() {
        "paragraph" | "header" => Draft {
            kind: block.kind.clone(),
            text: text("text"),
            ..Draft::default()
        },
        "quote" => Draft {
            kind: "quote".to_owned(),
            text: text("text"),
            caption: text("caption"),
            kept: None,
        },
        // One item to a line, as the editor shows a list.
        "list" => Draft {
            kind: "list".to_owned(),
            text: block
                .plain_text()
                .lines()
                .map(|line| line.trim_start_matches("• "))
                .collect::<Vec<_>>()
                .join("\n"),
            ..Draft::default()
        },
        "delimiter" => Draft {
            kind: "delimiter".to_owned(),
            ..Draft::default()
        },
        other => Draft {
            kind: other.to_owned(),
            kept: Some(block.clone()),
            ..Draft::default()
        },
    }
}

/// The block as it is written back.
fn block_of(draft: &Draft) -> Option<ArticleBlock> {
    if let Some(kept) = &draft.kept {
        return Some(kept.clone());
    }
    let text = draft.text.trim();
    match draft.kind.as_str() {
        "delimiter" => Some(ArticleBlock::delimiter()),
        _ if text.is_empty() => None,
        "header" => Some(ArticleBlock::header(text, 2)),
        "quote" => Some(ArticleBlock::quote(text, draft.caption.trim())),
        "list" => {
            let items: Vec<String> = text
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_owned)
                .collect();
            Some(ArticleBlock::list(&items, false))
        }
        _ => Some(ArticleBlock::paragraph(text)),
    }
}

// ---------------------------------------------------------------------------
// Changing the blocks
// ---------------------------------------------------------------------------

pub fn add_block(cx: &Context<'_>, kind: &str) {
    let mut state = cx.state.borrow_mut();
    state.blocks.push(Draft {
        kind: kind.to_owned(),
        ..Draft::default()
    });
    show(cx.window, &state);
}

pub fn remove_block(cx: &Context<'_>, index: usize) {
    let mut state = cx.state.borrow_mut();
    if index < state.blocks.len() {
        state.blocks.remove(index);
        show(cx.window, &state);
    }
}

pub fn move_block(cx: &Context<'_>, index: usize, delta: i32) {
    let mut state = cx.state.borrow_mut();
    let Some(to) = index.checked_add_signed(delta as isize) else {
        return;
    };
    if index < state.blocks.len() && to < state.blocks.len() {
        state.blocks.swap(index, to);
        show(cx.window, &state);
    }
}

/// Keeps what is typed. The rows are not redrawn for it: redrawing under
/// the cursor would move the cursor.
pub fn set_text(state: &Rc<RefCell<EditorState>>, index: usize, text: &str) {
    if let Some(block) = state.borrow_mut().blocks.get_mut(index) {
        block.text = text.to_owned();
    }
}

pub fn set_caption(state: &Rc<RefCell<EditorState>>, index: usize, text: &str) {
    if let Some(block) = state.borrow_mut().blocks.get_mut(index) {
        block.caption = text.to_owned();
    }
}

// ---------------------------------------------------------------------------
// Sending it
// ---------------------------------------------------------------------------

/// Publishes, saves or suggests the post, then calls `done` to show it.
pub fn publish(cx: &Context<'_>, done: impl FnOnce(&MainWindow) + 'static) {
    let (mode, blocks, channel) = {
        let state = cx.state.borrow();
        (
            state.mode,
            state.blocks.iter().filter_map(block_of).collect::<Vec<_>>(),
            state.channels.get(state.channel).map(|c| c.id),
        )
    };
    let window = cx.window;
    if blocks.iter().all(|b| b.kind == "delimiter") {
        window.set_editor_error("empty".into());
        return;
    }
    if mode == Mode::New && channel.is_none() {
        window.set_editor_error("no-channel".into());
        return;
    }
    let payload = ArticlePayload::of(blocks);
    let signed = window.get_editor_signed() && window.get_editor_can_sign();
    window.set_editor_busy(true);
    window.set_editor_error("".into());

    let weak = window.as_weak();
    let api = (**cx.client).clone();
    tasks::spawn(
        async move {
            match mode {
                Mode::New => {
                    api.article_create(channel.unwrap_or_default(), &payload, signed, None)
                        .await
                }
                Mode::Edit(id) => api.article_edit(id, &payload, signed).await,
                Mode::Suggest(channel) => api.suggestion_create(channel, &payload).await,
            }
        },
        move |result| {
            let Some(window) = weak.upgrade() else { return };
            window.set_editor_busy(false);
            match result {
                Ok(_) => {
                    window.set_editor_open(false);
                    done(&window);
                }
                Err(error) => {
                    tracing::warn!(%error, "the post was not sent");
                    window.set_editor_error("failed".into());
                }
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(kind: &str, text: &str) -> Draft {
        Draft {
            kind: kind.to_owned(),
            text: text.to_owned(),
            ..Draft::default()
        }
    }

    #[test]
    fn empty_blocks_are_left_out_but_a_divider_is_not() {
        assert!(block_of(&draft("paragraph", "   ")).is_none());
        assert_eq!(block_of(&draft("delimiter", "")).unwrap().kind, "delimiter");
    }

    #[test]
    fn a_list_is_a_line_to_an_item() {
        let block = block_of(&draft("list", "раз\n\n два \nтри")).unwrap();
        assert_eq!(
            block.data["items"],
            serde_json::json!(["раз", "два", "три"])
        );
    }

    #[test]
    fn a_list_reads_back_into_lines() {
        let block = ArticleBlock::list(&["раз".to_owned(), "два".to_owned()], false);
        assert_eq!(draft_of(&block).text, "раз\nдва");
    }

    #[test]
    fn a_picture_survives_an_edit() {
        let media = ArticleBlock {
            kind: "media".to_owned(),
            data: serde_json::json!({ "items": [{ "url": "https://example.com/a.png" }] }),
            ..ArticleBlock::default()
        };
        let back = block_of(&draft_of(&media)).unwrap();
        assert_eq!(back.kind, "media");
        assert_eq!(back.data, media.data);
    }
}
