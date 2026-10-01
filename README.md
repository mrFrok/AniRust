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

Usable. Browse or search, open a release, pick a voice-over and an episode, and
watch it — with the picture on the release screen rather than on a screen of its
own, so choosing the next episode never means leaving what you are watching.

Working:

- **Browsing** — what other people are watching, and search by title.
- **Playback** — Kodik, AniLibria and Sibnet resolved to direct streams;
  hardware decoding; quality, speed, subtitle and audio track selection;
  Anime4K upscaling and frame interpolation; skip the opening.
- **Continuing** — where an episode was left off is remembered, the list opens
  there, and an episode that ends is followed by the next one.
- **An account** — signing in syncs watched episodes and history; the token
  goes to the platform's secret store. The profile screen shows the account
  itself: what is in each list as a ring, what it has had to say, and what was
  watched lately — each of them a way into the list or the release behind it.
- **A release, from the account's side** — its list, favourite and rating
  out of five; episodes ticked by hand, one or all; the voice-over it opens
  with; the rest of its franchise, and the services that also carry it.
- **The feed** — posts from the channels the account follows, and the latest
  from every channel; following a channel from one of its posts.
- **Appearance** — light, dark, AMOLED, or whatever the desktop is set to,
  chosen on the profile screen and kept in `~/.config/anirust/settings.json`.

Not covered: SovetRomantica, whose host is unreachable from the network this
was developed on, so its protocol could not be observed — and guessing it would
mean reading someone else's implementation. Allvideo, StudioMir, Myvi, VKVideo,
OK, RuTube and MailRu are listed by the official client but did not appear in
the sampled catalogue.

The aim is parity with the official client. Its interface declares 286
distinct endpoints; **47** are implemented, each checked against a local
server by `api/tests/endpoints.rs`. The rest are being added area by area —
comments, discovery, notifications, people, channels, collections, settings
— and then packaged builds.

## Building

Needs Rust 1.90+ and, for the player, `libmpv` >= 2. On Debian and Ubuntu that
is `libmpv-dev`; on Arch, `mpv`.

```sh
cargo build --release
./target/release/anirust            # browse
./target/release/anirust 2999       # straight into a release
```

### Looking at the interface without a display

Layout mistakes are cheap to make and hard to spot by description. The interface
renders to a PNG with the software renderer, no window server involved:

```sh
cargo run -p anirust-gui --example screenshot -- out.png 1440 900 release
```

The last argument is one of `home`, `home-signed-in`, `home-hover-account`,
`release`, `playing`, `theatre`, `downloads`, `sign-in`, `failed`, `saved`,
`loading`, `refreshing`, `empty`, `nothing`, `profile`, `profile-signed-in`,
`profile-light`, `profile-en`, `home-downloading`, `feed`, `feed-latest`,
`feed-signed-out`, `feed-empty`, `light` or `amoled`; a width below 900 gives
the stacked layout. The states past the obvious ones are the ones worth having:
each is a screen that is easy to leave untested and easy to get wrong —
nothing loaded yet, nothing found, nothing to play.

One difference from the running application is worth knowing before a bug is
chased that is not there: the software renderer clips to a rectangle and
ignores `border-radius`, so anything inside a rounded box with `clip: true`
has square corners in a PNG and rounded ones on screen.

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
player/    libmpv wrapper rendering into a Slint GL texture
gui/       the Slint application
cli/       the debugging probe
```

mpv renders into a framebuffer the GUI owns, and Slint borrows that as a
texture — no frame is ever copied through the CPU. Two details cost a long
evening each and are worth knowing before touching `gui/src/video.rs`: the
window has to be repainted from `AfterRendering` rather than on a timer, or
mpv's estimate of the display rate is wrong and interpolation tears the picture
apart; and mpv's default `rgba16f` intermediate buffers are unusable in the
OpenGL ES context Slint provides, which shows up as bands of torn colour
rather than as an error.

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
dynamically. The Anime4K shaders are MIT, taken from upstream. So is the
Material 3 component set in `gui/material-1.18.0/`, vendored from
`ui-libraries/material` of slint-ui/slint at the tag matching the `slint`
dependency — it has no crates.io package, and a UI that changes shape when
someone else tags a release is not a UI anyone can review.
