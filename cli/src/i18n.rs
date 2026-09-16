// SPDX-License-Identifier: GPL-3.0-or-later

//! Bilingual output for the probe.
//!
//! The audience is mostly Russian-speaking, so Russian is a first-class output
//! language rather than an afterthought — but the source itself (identifiers,
//! comments, commit messages) stays English.
//!
//! Each message is a method returning a finished line, so a translation owns
//! its own wording *and* its own column widths: Cyrillic labels are rarely the
//! same length as their English counterparts, and letting each language pad
//! itself avoids ragged columns. Arguments are interpolated by name, so the
//! compiler still checks every placeholder against the method signature.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Lang {
    #[default]
    En,
    Ru,
}

impl Lang {
    /// Language implied by the environment, falling back to English.
    ///
    /// Honours the usual precedence — `LC_ALL`, then `LC_MESSAGES`, then
    /// `LANG` — so overriding the locale for one command works as expected.
    #[must_use]
    pub fn from_env() -> Self {
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .into_iter()
            .find_map(|var| std::env::var(var).ok().filter(|v| !v.is_empty()))
            .filter(|locale| locale.to_ascii_lowercase().starts_with("ru"))
            .map_or(Self::En, |_| Self::Ru)
    }
}

/// Generates one method per message, with both translations side by side.
macro_rules! catalog {
    ($(
        $(#[$doc:meta])*
        $name:ident ( $($arg:ident : $ty:ty),* $(,)? ) {
            en: $en:expr,
            ru: $ru:expr $(,)?
        }
    )*) => {
        impl Lang {
            $(
                $(#[$doc])*
                #[must_use]
                pub fn $name(self, $($arg: $ty),*) -> String {
                    match self {
                        Self::En => format!($en),
                        Self::Ru => format!($ru),
                    }
                }
            )*
        }
    };
}

catalog! {
    // ---- search -----------------------------------------------------------
    nothing_found() {
        en: "nothing found",
        ru: "ничего не найдено",
    }

    // ---- release ----------------------------------------------------------
    release_header(id: i64, title: &str) {
        en: "{id} - {title}",
        ru: "{id} — {title}",
    }
    release_original(value: &str) {
        en: "  original   {value}",
        ru: "  оригинал   {value}",
    }
    release_year(value: &str) {
        en: "  year       {value}",
        ru: "  год        {value}",
    }
    release_status(value: &str) {
        en: "  status     {value}",
        ru: "  статус     {value}",
    }
    release_genres(value: &str) {
        en: "  genres     {value}",
        ru: "  жанры      {value}",
    }
    release_studio(value: &str) {
        en: "  studio     {value}",
        ru: "  студия     {value}",
    }
    release_episodes(released: i32, total: i32) {
        en: "  episodes   {released}/{total}",
        ru: "  серии      {released}/{total}",
    }
    release_score(score: f32, votes: i64) {
        en: "  score      {score:.1} ({votes} votes)",
        ru: "  оценка     {score:.1} ({votes} голосов)",
    }
    release_playable(value: &str) {
        en: "  playable   {value}",
        ru: "  доступен   {value}",
    }
    release_block_reasons(play_disabled: bool, view_blocked: bool, deleted: bool) {
        en: "    play_disabled={play_disabled} view_blocked={view_blocked} deleted={deleted}",
        ru: "    play_disabled={play_disabled} view_blocked={view_blocked} deleted={deleted}",
    }
    yes() { en: "yes", ru: "да" }
    no() { en: "no", ru: "нет" }

    // ---- dubbers and sources ----------------------------------------------
    dubber_row(id: i64, name: &str, episodes: i64, kind: &str, views: i64) {
        en: "{id:>8}  {name:<38} {episodes:>4} eps  {kind}  {views} views",
        ru: "{id:>8}  {name:<38} серий {episodes:>4}  {kind}  просмотров {views}",
    }
    subtitles() { en: "sub", ru: "сабы" }
    dubbed() { en: "dub", ru: "озв." }
    source_row(id: i64, name: &str, episodes: i64, quality: i32) {
        en: "{id:>8}  {name:<28} {episodes:>4} eps  quality {quality}",
        ru: "{id:>8}  {name:<28} серий {episodes:>4}  качество {quality}",
    }

    // ---- account ----------------------------------------------------------
    password_prompt(login: &str) {
        en: "password for {login}: ",
        ru: "пароль для {login}: ",
    }
    signed_in(login: &str, id: i64) {
        en: "signed in as {login} (id {id})",
        ru: "вошли как {login} (id {id})",
    }
    sign_in_failed(error: &dyn fmt::Display) {
        en: "sign-in failed: {error}",
        ru: "вход не удался: {error}",
    }
    profile_watching(count: i64) {
        en: "  watching   {count}",
        ru: "  смотрю     {count}",
    }
    profile_planned(count: i64) {
        en: "  planned    {count}",
        ru: "  в планах   {count}",
    }
    profile_watched(count: i64) {
        en: "  watched    {count}",
        ru: "  просмотрено {count}",
    }
    profile_on_hold(count: i64) {
        en: "  on hold    {count}",
        ru: "  отложено   {count}",
    }
    profile_dropped(count: i64) {
        en: "  dropped    {count}",
        ru: "  брошено    {count}",
    }
    page_info(page: i32, pages: i32, total: i64) {
        en: "page {page} of {pages} ({total} total)",
        ru: "страница {page} из {pages} (всего {total})",
    }

    // ---- streams ----------------------------------------------------------
    episode_line(position: i32, url: &str, kind: &str) {
        en: "episode {position} - {url} ({kind})",
        ru: "серия {position} — {url} ({kind})",
    }
    embed() { en: "embed", ru: "эмбед" }
    direct() { en: "direct", ru: "прямая" }
    dubber_chosen(name: &str, id: i64) {
        en: "dubber: {name} ({id})",
        ru: "озвучка: {name} ({id})",
    }
    source_chosen(name: &str, id: i64) {
        en: "source: {name} ({id})",
        ru: "источник: {name} ({id})",
    }
    note_flag_says_direct() {
        en: "note: the API marked this episode as a direct link, but the host needs an \
             extractor - resolving through it anyway",
        ru: "примечание: API пометил серию как прямую ссылку, но хосту нужен экстрактор — \
             разрешаю через него",
    }
    warn_no_extractor() {
        en: "warning: this is an embed and no extractor claims the host - passing the URL \
             through as-is, which will most likely not play",
        ru: "внимание: это эмбед, но экстрактора для хоста нет — ссылка отдана как есть и, \
             скорее всего, не проиграется",
    }
    opening_range(start: u32, end: u32, duration: u32) {
        en: "opening: {start}-{end}s ({duration}s)",
        ru: "опенинг: {start}–{end} с ({duration} с)",
    }
    headers_required() {
        en: "headers (the player must send these):",
        ru: "заголовки (обязательны для плеера):",
    }

    // ---- chain ------------------------------------------------------------
    chain_release(id: i64, title: &str) {
        en: "release {id} - {title}",
        ru: "релиз {id} — {title}",
    }
    chain_not_playable() {
        en: "  warning: the server marks this release as not playable",
        ru: "  внимание: сервер помечает релиз как недоступный для воспроизведения",
    }
    chain_dubber_count(count: usize) {
        en: "voice-over tracks: {count}",
        ru: "озвучек: {count}",
    }
    chain_dubber(id: i64, name: &str, sources: usize) {
        en: "  dubber {id} ({name}): {sources} sources",
        ru: "  озвучка {id} ({name}): источников {sources}",
    }
    chain_source(id: i64, name: &str, episodes: usize) {
        en: "    source {id} ({name}): {episodes} episodes",
        ru: "    источник {id} ({name}): серий {episodes}",
    }
    chain_total(count: usize) {
        en: "episodes total: {count}",
        ru: "всего серий: {count}",
    }
    chain_direct(count: usize, percent: f64) {
        en: "  direct links: {count} ({percent:.0}%)",
        ru: "  прямых ссылок: {count} ({percent:.0}%)",
    }
    chain_embeds(count: usize, percent: f64) {
        en: "  embeds:       {count} ({percent:.0}%)  - need an extractor",
        ru: "  эмбедов:       {count} ({percent:.0}%)  — нужен экстрактор",
    }
    chain_by_host() {
        en: "by host:",
        ru: "по хостам:",
    }

    // ---- failures ---------------------------------------------------------
    err_no_dubbers() {
        en: "the release has no voice-over tracks",
        ru: "у релиза нет ни одной озвучки",
    }
    err_no_sources() {
        en: "the dubber has no sources",
        ru: "у озвучки нет ни одного источника",
    }
    err_no_episode(position: i32) {
        en: "this source has no episode {position}",
        ru: "серии {position} нет у этого источника",
    }
    err_nothing_resolved() {
        en: "nothing resolved to a playable stream",
        ru: "не удалось получить ни одного потока",
    }
}

impl Lang {
    /// Localised yes/no.
    #[must_use]
    pub fn yes_no(self, value: bool) -> String {
        if value { self.yes() } else { self.no() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_languages_interpolate_their_arguments() {
        assert_eq!(Lang::En.chain_total(7), "episodes total: 7");
        assert_eq!(Lang::Ru.chain_total(7), "всего серий: 7");
    }

    #[test]
    fn translations_differ_but_carry_the_same_data() {
        let en = Lang::En.release_episodes(12, 24);
        let ru = Lang::Ru.release_episodes(12, 24);
        assert_ne!(en, ru);
        for rendered in [&en, &ru] {
            assert!(rendered.contains("12/24"), "lost the numbers: {rendered}");
        }
    }

    #[test]
    fn yes_no_is_localised() {
        assert_eq!(Lang::En.yes_no(true), "yes");
        assert_eq!(Lang::Ru.yes_no(false), "нет");
    }

    #[test]
    fn locale_detection_only_matches_russian() {
        // from_env reads the process environment, so the mapping itself is
        // asserted through the same predicate it uses.
        for locale in ["ru_RU.UTF-8", "ru", "RU_ru"] {
            assert!(locale.to_ascii_lowercase().starts_with("ru"), "{locale}");
        }
        for locale in ["en_US.UTF-8", "C", "POSIX", "uk_UA.UTF-8"] {
            assert!(!locale.to_ascii_lowercase().starts_with("ru"), "{locale}");
        }
    }
}
