// SPDX-License-Identifier: GPL-3.0-or-later

//! What a file contains, and which parts of it are playing.
//!
//! A player needs more than "set subtitle 2": it has to offer a menu, which
//! means knowing what exists, what each entry is called, and what is currently
//! chosen.
//!
//! mpv exposes this as a `track-list` node. Rather than deserialise a node
//! tree, each field is read through its own property path
//! (`track-list/0/title` and so on): the values are then plain strings, flags
//! and integers, which keeps this free of a bespoke JSON shape that mpv is
//! free to change underneath us.

use libmpv2::Mpv;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Video,
    Audio,
    Subtitle,
}

impl TrackKind {
    fn from_mpv(value: &str) -> Option<Self> {
        Some(match value {
            "video" => Self::Video,
            "audio" => Self::Audio,
            "sub" => Self::Subtitle,
            _ => return None,
        })
    }

    /// The property that selects a track of this kind.
    pub(crate) fn selector(self) -> &'static str {
        match self {
            Self::Video => "vid",
            Self::Audio => "aid",
            Self::Subtitle => "sid",
        }
    }
}

/// One stream inside the file.
#[derive(Debug, Clone)]
pub struct Track {
    /// Identifier to pass back when selecting. Unique per kind, not globally.
    pub id: i64,
    pub kind: TrackKind,
    /// Human title, when the file carries one.
    pub title: Option<String>,
    /// Language tag, usually ISO 639.
    pub lang: Option<String>,
    pub selected: bool,
    /// Loaded from a separate file rather than the container.
    pub external: bool,
    /// Codec, for telling otherwise identical tracks apart.
    pub codec: Option<String>,
}

impl Track {
    /// A label for a menu, falling back through title, language and codec
    /// before giving up and naming the id.
    ///
    /// Anime releases habitually ship several subtitle tracks with no titles
    /// and the same language, so the fallbacks matter more than they look.
    #[must_use]
    pub fn label(&self) -> String {
        if let Some(title) = self.title.as_deref().filter(|t| !t.is_empty()) {
            return match self.lang.as_deref().filter(|l| !l.is_empty()) {
                Some(lang) => format!("{title} [{lang}]"),
                None => title.to_owned(),
            };
        }
        if let Some(lang) = self.lang.as_deref().filter(|l| !l.is_empty()) {
            return lang.to_owned();
        }
        if let Some(codec) = self.codec.as_deref().filter(|c| !c.is_empty()) {
            return format!("#{} ({codec})", self.id);
        }
        format!("#{}", self.id)
    }
}

/// Reads the whole track list.
///
/// A missing or unreadable entry is skipped rather than failing the list: a
/// menu with one odd track missing is better than no menu.
pub(crate) fn list(mpv: &Mpv) -> Vec<Track> {
    let count = mpv.get_property::<i64>("track-list/count").unwrap_or(0);

    (0..count)
        .filter_map(|index| {
            let kind = TrackKind::from_mpv(&string_at(mpv, index, "type")?)?;
            Some(Track {
                id: mpv
                    .get_property::<i64>(&format!("track-list/{index}/id"))
                    .ok()?,
                kind,
                title: string_at(mpv, index, "title"),
                lang: string_at(mpv, index, "lang"),
                selected: flag_at(mpv, index, "selected"),
                external: flag_at(mpv, index, "external"),
                codec: string_at(mpv, index, "codec"),
            })
        })
        .collect()
}

fn string_at(mpv: &Mpv, index: i64, field: &str) -> Option<String> {
    mpv.get_property::<String>(&format!("track-list/{index}/{field}"))
        .ok()
        .filter(|value| !value.is_empty())
}

fn flag_at(mpv: &Mpv, index: i64, field: &str) -> bool {
    mpv.get_property::<bool>(&format!("track-list/{index}/{field}"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(title: Option<&str>, lang: Option<&str>, codec: Option<&str>) -> Track {
        Track {
            id: 3,
            kind: TrackKind::Subtitle,
            title: title.map(str::to_owned),
            lang: lang.map(str::to_owned),
            selected: false,
            external: false,
            codec: codec.map(str::to_owned),
        }
    }

    #[test]
    fn a_title_and_language_are_shown_together() {
        assert_eq!(
            track(Some("Signs"), Some("rus"), None).label(),
            "Signs [rus]"
        );
    }

    #[test]
    fn a_title_alone_is_enough() {
        assert_eq!(track(Some("Full"), None, None).label(), "Full");
    }

    #[test]
    fn language_stands_in_for_a_missing_title() {
        assert_eq!(track(None, Some("jpn"), None).label(), "jpn");
    }

    #[test]
    fn codec_stands_in_when_nothing_else_does() {
        assert_eq!(track(None, None, Some("ass")).label(), "#3 (ass)");
    }

    #[test]
    fn an_anonymous_track_is_still_named() {
        assert_eq!(track(None, None, None).label(), "#3");
    }

    #[test]
    fn empty_strings_count_as_absent() {
        // mpv reports a missing field as an empty string, not as no field.
        assert_eq!(track(Some(""), Some(""), Some("")).label(), "#3");
    }

    #[test]
    fn kinds_map_to_their_selector_properties() {
        assert_eq!(TrackKind::Video.selector(), "vid");
        assert_eq!(TrackKind::Audio.selector(), "aid");
        assert_eq!(TrackKind::Subtitle.selector(), "sid");
    }

    #[test]
    fn unknown_kinds_are_rejected() {
        assert_eq!(TrackKind::from_mpv("sub"), Some(TrackKind::Subtitle));
        assert_eq!(TrackKind::from_mpv("attachment"), None);
    }
}
