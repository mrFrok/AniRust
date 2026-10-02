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

- **Browsing** — the catalogue cut by popularity, rating, films and
  announcements, narrowed by genre; the front page's curated cards,
  recommendations, what is being watched and discussed; the week's schedule;
  a random release. The search looks through whatever tab is up — a list,
  the history, collections — and from anywhere else finds releases, people
  and channels at once.
- **Playback** — Kodik, AniLibria and Sibnet resolved to direct streams;
  hardware decoding; quality, speed, subtitle and audio track selection;
  Anime4K upscaling and frame interpolation; skip the opening.
- **Continuing** — where an episode was left off is remembered, the list opens
  there, and an episode that ends is followed by the next one.
- **A release, from the account's side** — its list, favourite and rating
  out of five; episodes ticked by hand, one or all; the voice-over it opens
  with; the rest of its franchise, the services that also carry it, its
  trailers and openings, and putting it in a collection.
- **Comments** — on releases, posts and collections: sorted, threaded,
  spoilers hidden, voted on, written, changed and deleted.
- **The feed and channels** — posts from followed channels and the latest
  from all of them; channel pages; following, muting; writing posts in a
  block editor, changing, pinning and deleting them; suggesting posts to
  other channels; making a channel or a blog and running it — settings,
  pictures, the suggestion queue, administrators, blocks.
- **Collections** — everybody's and the account's own; favourited,
  commented, made, changed, given a cover, deleted.
- **People** — profiles of others, friends and requests, blocking.
- **Notifications** — a bell with a count, the list by kind, and which kinds
  arrive at all.
- **The account** — signing in, registering and restoring a password by
  emailed code; the token goes to the platform's secret store. Picture,
  status, privacy, incognito, social links, login, password and email;
  standing with the service and appeals; bookmarks in from Shikimori and out
  as CSV; deleting the account, asked for twice.
- **Reports** — on releases, comments, posts, channels, collections and
  people, with the service's own reasons.
- **Appearance** — light, dark, AMOLED, or whatever the desktop is set to,
  kept in the platform's config directory.

Not covered: SovetRomantica, whose host is unreachable from the network this
was developed on, so its protocol could not be observed — and guessing it would
mean reading someone else's implementation. Allvideo, StudioMir, Myvi, VKVideo,
OK, RuTube and MailRu are listed by the official client but did not appear in
the sampled catalogue. Signing in through Google, VK, Telegram or Yandex is in
the API layer but not in the interface: those take a token their own SDKs give
only to the official app. Pictures inside posts are kept when a post is
changed, but not added: they go up through an upload this client has no way
into.

The official client's interface declares 286 distinct endpoints; **all 286**
are implemented, each checked against a local server by
`api/tests/endpoints.rs`, and `api/tests/parity.rs` fails if one goes missing.

## Installing

Tagged releases build a `.deb`, a Linux tarball, a Windows zip with libmpv
beside the program, and a macOS disk image; Arch has a PKGBUILD in
`packaging/arch/`.

## Building

Needs Rust 1.90+ and, for the player, `libmpv` >= 2.

- **Linux** — `libmpv-dev` on Debian and Ubuntu, `mpv` on Arch. The file
  chooser goes through the desktop portal (`xdg-desktop-portal`), and the
  token through the Secret Service.
- **macOS** — `brew install mpv`, and `LIBRARY_PATH="$(brew --prefix)/lib"`
  for the build.
- **Windows** — a libmpv build with an MSVC import library;
  `packaging/windows/fetch-libmpv.ps1` makes one from a Developer PowerShell,
  and `libmpv-2.dll` goes next to `anirust.exe`.

```sh
cargo build --release
./target/release/anirust            # browse
./target/release/anirust 2999       # straight into a release
```

Packages: `cargo deb -p anirust-gui` for Debian, `makepkg -si` in
`packaging/arch/`, `packaging/macos/bundle.sh VERSION` for an app bundle.

### Looking at the interface without a display

Layout mistakes are cheap to make and hard to spot by description. The interface
renders to a PNG with the software renderer, no window server involved:

```sh
cargo run -p anirust-gui --example screenshot -- out.png 1440 900 release
```

The last argument names a state; the full list is at the top of
`gui/examples/screenshot.rs`. A width below 900 gives the stacked layout.
`ANIRUST_SHOT_POINTER=x,y` parks the pointer, and `ANIRUST_SHOT_SCROLL=px`
turns the wheel under it, for what is below a fold. The states past the
obvious ones are the ones worth having: each is a screen that is easy to
leave untested and easy to get wrong — nothing loaded yet, nothing found,
nothing to play.

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
