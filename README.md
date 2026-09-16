# AniRust

A cross-platform desktop client for Anixart, written in Rust with Slint, and a
real player built on mpv.

*[Русская версия](README.ru.md)*

> AniRust is an unofficial project and is **not affiliated** with the
> developers of Anixart. The Anixart name and branding belong to their owners.

## Why

The official client is Android-only, closed source and ad-supported. AniRust is
a desktop client with a player of its own: playback speed, proper subtitle
rendering, hardware decoding, Anime4K upscaling, resume across episodes, and
progress synchronised with an Anixart account.

## Status

Early development. Done:

- `api/` — typed API client (search, releases, the episode chain, account,
  lists, history, favourites);
- `extract/` — resolves embedded player pages to direct streams: **Kodik**
  (96% of the catalogue), **AniLibria** and **Sibnet**;
- `cli/` — a probe for driving the live API.

Not covered: SovetRomantica, whose host is unreachable from the network this
was developed on, so its protocol could not be observed — and guessing it would
mean reading someone else's implementation. Allvideo, StudioMir, Myvi, VKVideo,
OK, RuTube and MailRu are listed by the official client but did not appear in
the sampled catalogue.

Next, by phase: the mpv player → the Slint GUI.

## Building

Needs Rust 1.90+ and, for the player, `libmpv` >= 2.

```sh
cargo build --release
```

## The probe

`anirust-cli` drives the live API and prints what comes back. Diagnostics go to
stderr and data to stdout, so output pipes cleanly into `jq`.

```sh
# Search
anirust-cli search "Fullmetal Alchemist"

# Walk the whole playback chain and report how many episodes need an extractor
anirust-cli chain 307 --all-dubbers

# Straight from a release id to a playable stream
anirust-cli stream 307 1

# A ready-to-run mpv command, headers and timeout already filled in
$(anirust-cli stream 307 1 --mpv)

# Hosts an extractor is implemented for
anirust-cli hosts

# Account
export ANIRUST_TOKEN=$(anirust-cli login <login>)
anirust-cli me
anirust-cli list watching
```

The CDN nodes these manifests redirect to can be very slow to accept a
connection — 19s was measured against a host that had answered in 14ms moments
earlier. ffmpeg's default timeout is shorter, so `--mpv` always passes
`--network-timeout`: without it a perfectly good stream looks like a broken
extractor.

## Architecture

```
api/       typed Anixart client (reqwest + serde)
extract/   the Extractor trait and per-host implementations
player/    libmpv wrapper rendering into a Slint GL texture   [phase 3]
core/      session, cache, playback orchestration             [phase 4]
gui/       the Slint application                              [phase 4]
cli/       the debugging probe
```

Getting from a release to something playable takes three calls, then usually an
extractor:

```
dubbers(release_id)                        -> voice-over tracks
sources(release_id, dubber_id)             -> Kodik, Sibnet, ...
episodes(release_id, dubber_id, source_id) -> episodes
```

Each episode carries a `url` and an `iframe` flag. Measured across 832 episodes
in 8 releases: 97% are embeds, and 96% of those go through Kodik.

**The `iframe` flag cannot be trusted.** Sibnet and AniLibria episodes arrive
with `iframe = false` even though their URLs (`shell.php?videoid=`,
`iframe.php?id=`) are player pages answering `text/html`. Routing is therefore
decided by host: if an extractor claims it, the URL goes through that extractor
regardless of the flag.

## Clean room

The API layer and the extractors are written against **observed protocol
behaviour**, not against decompiled sources of the official app. This is a
deliberate constraint, and it is honoured literally:

1. Decompiled material never enters the repository — see `.gitignore`.
2. Only interface facts are taken from the official app: host, path, method,
   field name, enum value. Those are facts, not expression.
3. Implementations are written against the live exchange, not someone's code.
4. Test fixtures are captured HTTP responses, never third-party sources.
5. mpv, ffmpeg and the Anime4K shaders come from upstream, not from the APK.

## Licence

[GPL-3.0-or-later](LICENSE).

Slint is used under its GPLv3 option, the one intended for open-source
applications. mpv (`GPL-2.0-or-later AND LGPL-2.1-or-later`) is linked
dynamically. The Anime4K shaders are MIT, taken from upstream.
