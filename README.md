# AniRust

A cross-platform desktop client for Anixart, written in Rust with Slint, and a
real player built on mpv.

[![Release](https://img.shields.io/github/v/release/mrFrok/AniRust)](https://github.com/mrFrok/AniRust/releases/latest)
[![CI](https://github.com/mrFrok/AniRust/actions/workflows/ci.yml/badge.svg)](https://github.com/mrFrok/AniRust/actions/workflows/ci.yml)

**[Download](https://github.com/mrFrok/AniRust/releases/latest)** for Windows,
macOS or Linux — which file to take is under [Installing](#installing).

*[Русская версия](README.ru.md)*

> AniRust is an unofficial project and is **not affiliated** with the
> developers of Anixart. The Anixart name and branding belong to their owners.

## Why

The official client is Android-only, closed source and ad-supported. AniRust is
a desktop client with a player of its own, built for what a desktop GPU can do:
Anime4K's full set of networks up to its heaviest, RIFE frame generation,
hardware decoding on every vendor's card, proper subtitle rendering, resume
across episodes, and progress synchronised with an Anixart account.

## Status

Released: 1.0.0 is the first version for everyday use. Browse or search, open
a release, pick a voice-over and an episode, and watch it — with the picture on
the release screen rather than on a screen of its own, so choosing the next
episode never means leaving what you are watching.

Working:

- **Browsing** — the catalogue cut by popularity, rating, films and
  announcements, narrowed by genre; the front page's curated cards,
  recommendations, what is being watched and discussed; the week's schedule;
  a random release. The search looks through whatever tab is up — a list,
  the history, collections — and from anywhere else finds releases, people
  and channels at once.
- **Playback** — Kodik, AniLibria and Sibnet resolved to direct streams;
  hardware decoding; quality, speed, subtitle and audio track selection; skip
  the opening; upscaling, frame generation and the rest of the player below.
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

## The player

**Upscaling.** Anime4K's six modes — A, B, C, A+A, B+B, C+A, each for a kind of
source — at four qualities: Fast (networks M then S), High (L, M), Max (VL, M,
Anime4K's own choice for high-end cards) and Ultra (a VL restore, a UL upscale,
then L). Every chain doubles twice around Anime4K's auto-downscale, so 720p
reaches 4K through two network passes rather than one and a stretch. "Render at
4K" upscales to 3840×2160 whatever the window. The shaders are embedded and
need no setup. Two of Anime4K's UL networks are left out: they need more
varying variables than OpenGL allows and fail to link on every GPU.

**Frame generation.** RIFE draws frames between the real ones, to ×2, 60 or the
screen's rate, with a fast network (RIFE 4.6) or a cleaner one (4.26). It runs
through mpv's VapourSynth filter and a Vulkan plugin, so on any vendor's card.
It is heavy: on an RTX 4070 Ti SUPER, RIFE 4.6 makes 74 frames a second at 720p
and 46 at 1080p, so sources taller than 720 lines are brought down for it and
the upscaling after it brings them back. It needs three things, none of them
linked, so the program runs without them: a libmpv with the VapourSynth filter,
VapourSynth itself, and the plugin with its models. Every package but the
`.deb` carries the libmpv and the plugin, and the Flatpak VapourSynth as well;
elsewhere VapourSynth is installed once (see [Installing](#installing)). When
one is missing the menu says so, and if the filter fails, mpv plays on without
it and the log says why.

**Neural networks on the matrix units.** The same networks run two to three
times faster on the units GPUs keep for them — NVIDIA's tensor cores, Intel's
XMX engines, AMD's matrix cores — through
[vs-mlrt](https://github.com/AmusementClub/vs-mlrt), whose plugins are ported
to VapourSynth's current API by `packaging/mlrt/port-api4.py` and built by the
release: TensorRT for NVIDIA RTX, OpenVINO for Intel, MIGraphX for AMD. Measured
on an RTX 4070 Ti SUPER with TensorRT, RIFE 4.26 makes 150 frames a second at
720p and 66 at 1080p — enough for 1080p at 60 and for 720p at 120 and more.
On any of the three, the menu offers **neural upscaling** too: Real-ESRGAN
AnimeVideo v3 doubles the picture before the shaders (720p to 1440p at 43
frames a second on that card). Each engine runs only on its maker's cards, so
the menu offers the ones for the GPUs the machine has — read from the kernel
on Linux and from DXGI on Windows — and marks the one that suits it best. The
vendors' runtimes
are not part of the program: TensorRT-RTX (NVIDIA's licence, about 90 MB) and
OpenVINO (Apache 2.0, 55 to 110 MB) are fetched from their vendors by the
player when asked, and MIGraphX comes with ROCm from the distribution
(`migraphx` on Arch). Until the runtime is there, Vulkan runs the networks. The
first time an engine meets a picture size it compiles the network for the card,
which takes ten seconds or more once; the player says so meanwhile. The
figures above are TensorRT's, the only ones measured so far: OpenVINO and
MIGraphX are built and load, but are yet to be timed on Intel and AMD cards.
macOS has no engine of this kind yet.

**Keeping up.** If frames start dropping — more than one a second over ten
seconds of playback — the load comes down a step by itself: neural upscaling
first, then the RIFE network, Anime4K's quality, the generated rate (the
screen's to half of it when that is still 60 or more, then 60, then ×2), and
then each of them off, with a line saying what changed. The step is kept. It
can be switched off.

**The rest**, much of it after the official Android player:

- hold the button on the picture to play at 2×; step a frame back or forward
  from buttons that appear when paused; repeat an episode;
- screenshots with or without subtitles, to Pictures/AniRust;
- another dub or subtitles from a file; audio and subtitle delay in tenths of
  a second; loudness evened out;
- subtitle size, the player's own style over ASS styles, and a fonts folder for
  the fonts fansubs name;
- colour presets: natural, brighter, vivid, soft, dark room;
- hardware decoding with the display handed to mpv, so VA-API works on AMD and
  Intel under Wayland and X11, D3D11 on Windows, VideoToolbox on macOS;
- the player's settings kept between runs, if wanted.

| Key | Does |
| --- | --- |
| Space, K | play or pause |
| ← → / J L | 5 s / 10 s back or forward |
| ↑ ↓, M | volume, mute |
| `<` `>` | slower, faster |
| `,` `.` | a frame back or forward |
| Shift+P, Shift+N | previous, next episode |
| F, T | full screen, theatre |
| C, S | subtitles on or off, skip the opening |
| U, I | next upscaling mode, interpolation |
| R | repeat the episode |
| P, Ctrl+P | screenshot with subtitles, without |
| Z X, Shift+Z X | subtitle delay, audio delay |

## Installing

Every release is on the
[releases page](https://github.com/mrFrok/AniRust/releases/latest), each file
with a `.sha256` beside it (`sha256sum -c FILE.sha256`).

| System | Take |
| --- | --- |
| Windows 10 and 11 | `anirust-<version>-windows-x86_64.zip` |
| macOS 14+ on Apple silicon | `anirust-<version>-macos-arm64.dmg` |
| Arch, CachyOS, Manjaro, openSUSE Tumbleweed | `anirust-<version>-linux-x86_64.tar.gz`, or the PKGBUILD |
| Ubuntu 24.04+, Debian 13+, Fedora 40+ and other glibc systems that recent | `AniRust-<version>-x86_64.AppImage` |
| any other Linux — older glibc, or none at all (Alpine, Void musl, Chimera) | `anirust-<version>-x86_64.flatpak` |

- **Windows** — unpack the zip anywhere and run `anirust.exe`. It carries
  libmpv, the RIFE plugin and vs-mlrt for TensorRT and OpenVINO. For frame
  generation, install VapourSynth from its
  [releases](https://github.com/vapoursynth/vapoursynth/releases). AMD cards
  run the networks through Vulkan on Windows: MIGraphX is Linux-only here. The
  program is not signed, so SmartScreen may stop its first start: "More info",
  then "Run anyway".
- **macOS** — open the disk image and copy AniRust to Applications. It carries
  libmpv and the RIFE plugin, which runs through Vulkan on Metal. Apple has not
  notarised it, so macOS refuses the first start: allow it under System
  Settings → Privacy & Security → "Open Anyway", or run
  `xattr -dr com.apple.quarantine /Applications/AniRust.app`. Frame generation
  has yet to be tried on a Mac.
- **The Linux tarball** — unpack it and run `anirust` inside; the first start
  adds it to the desktop's applications. It is built on Arch and carries its
  own libmpv, with the VapourSynth filter, in `lib/`, the RIFE plugin in
  `rife/` and vs-mlrt in `mlrt/`; the rest of libmpv's libraries come from the
  system, so it runs whole on rolling distributions. Elsewhere its launcher
  falls back to the system's libmpv and everything but frame generation works.
  For frame generation, install VapourSynth (`vapoursynth` on Arch); the first
  time it is switched on, the program runs `vapoursynth config`, which
  VapourSynth needs once to find its Python.
- **The PKGBUILD** in `packaging/arch/` builds the tagged release from source
  with `makepkg -si`, libmpv with the filter included; the RIFE plugin comes
  from upstream and vs-mlrt from the release
  (`anirust-<version>-mlrt-linux-x86_64.tar.gz` is that vs-mlrt alone).
- **The AppImage** — `chmod +x` it and run it; the first start adds it to the
  desktop's applications. One file with its own libmpv (with the filter), RIFE
  and vs-mlrt, built on Ubuntu 24.04. Frame generation uses the host's
  VapourSynth when it has one.
- **The Flatpak** carries the whole player in its own runtime — ffmpeg, mpv
  with the VapourSynth filter, VapourSynth, RIFE and vs-mlrt — so it runs on
  any Linux and frame generation works out of the box.
  `flatpak install --user anirust-<version>-x86_64.flatpak` installs it, and
  `flatpak run io.github.mrfrok.AniRust` or the applications menu starts it;
  the runtime comes from Flathub, and the GPU driver, NVIDIA's included, from
  Flatpak's GL extension.
- **The `.deb`** (`anirust_<version>-1_amd64.deb`, for Debian and Ubuntu,
  `sudo apt install ./anirust_<version>-1_amd64.deb`) plays through the
  system's libmpv, which those distributions build without the VapourSynth
  filter: everything but frame generation.

Outside the Flatpak, two things on Linux come from the system's drivers, and
the player says on screen when one is missing. Hardware decoding on Intel and
AMD cards goes through VA-API: `intel-media-driver` for Intel
(`intel-media-va-driver` on Debian and Ubuntu), Mesa's for AMD; without it the
player decodes on the CPU, and its decoder line reads SW. OpenVINO runs the
networks on an Intel GPU through Intel's OpenCL: `intel-compute-runtime`
(`intel-opencl-icd` on Debian and Ubuntu).

Releases are built by GitHub Actions (`.github/workflows/release.yml`): a tag
`v*` builds every package, each with a `.sha256`, and publishes the release;
started by hand, the workflow is a dry run that builds everything and publishes
nothing.

## Reporting problems

In the [issues](https://github.com/mrFrok/AniRust/issues), with the log when
something misbehaves. On Linux and macOS it goes to the terminal the program
was started from, in more detail with `RUST_LOG=anirust=debug`; on Windows it
is written to `%LOCALAPPDATA%\anirust\anirust.log`. Look it over before
posting: it names the releases and episodes played, and in 1.0.0 a request that
failed on the network shows the session token (`token=…`), which is to be cut
out.

## Building

```sh
git clone https://github.com/mrFrok/AniRust
```

Needs Rust 1.90+ and, for the player, `libmpv` >= 2.

- **Linux** — `libmpv-dev` on Debian and Ubuntu, `mpv` on Arch. The file
  chooser goes through the desktop portal (`xdg-desktop-portal`), and the
  token through the Secret Service.
- **macOS** — `brew install mpv`, and `LIBRARY_PATH="$(brew --prefix)/lib"`
  for the build.
- **Windows** — a libmpv build with an MSVC import library;
  `packaging/windows/fetch-libmpv.ps1` makes one from a Developer PowerShell,
  and `libmpv-2.dll` goes next to `anirust.exe`. Or from Linux, with MinGW:
  `packaging/windows/cross-build.sh` fetches libmpv and RIFE and writes the
  zip to `dist/`.

```sh
cargo build --release
./target/release/anirust            # browse
./target/release/anirust 2999       # straight into a release
```

Packages: `cargo deb -p anirust-gui` for Debian, `makepkg -si` in
`packaging/arch/`, `packaging/macos/bundle.sh VERSION` for an app bundle,
`packaging/linux/build-tarball.sh` for the Linux tarball (it builds libmpv with
the VapourSynth filter and fetches RIFE first).

vs-mlrt's plugins build with `packaging/mlrt/build-vstrt.sh` (TensorRT),
`build-vsov.sh` (OpenVINO) and `build-vsmigx.sh` (MIGraphX) — on Linux, and the
first two on Windows from Git Bash in an MSVC environment — and the networks
with `packaging/mlrt/prepare-models.py`, all into `target/mlrt`.
`ANIRUST_MLRT_DIR` points the program at such a folder, and
`ANIRUST_TRT_RTX_DIR`, `ANIRUST_OPENVINO_DIR` and `ROCM_PATH` at the vendors'
runtimes.

For frame generation in a development build, point the program at a libmpv
with the filter and at the plugin:

```sh
packaging/linux/build-libmpv.sh          # target/libmpv/libmpv.so.2
packaging/fetch-rife.sh linux target/rife-linux
LD_LIBRARY_PATH=target/libmpv ANIRUST_RIFE_DIR=target/rife-linux cargo run -p anirust-gui
```

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

# Play an episode headless through the real player: upscaling, frame generation
anirust-cli play 307 1 --upscale a+a --quality max
anirust-cli play 307 1 --generate 60      # prints the rate in and out

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
5. mpv, ffmpeg, the Anime4K shaders and RIFE come from upstream, not from the
   APK. The official player was a list of features to match, never a source.

## Licence

[GPL-3.0-or-later](LICENSE).

Slint is used under its GPLv3 option, the one intended for open-source
applications. mpv (`GPL-2.0-or-later AND LGPL-2.1-or-later`) is linked
dynamically; the libmpv the Linux tarball ships is built from mpv's release
with the one patch in `packaging/linux/`, which opens VapourSynth at run time
instead of linking it. The Anime4K shaders are MIT, taken from upstream. The
RIFE plugin (VapourSynth-RIFE-ncnn-Vulkan) and the RIFE models are MIT,
fetched from upstream at build time and shipped with their licences. vs-mlrt is
GPL-3.0; its plugins are built from its sources with our port to VapourSynth's
API 4, and ship with its licence, as do the networks: RIFE (MIT) and
Real-ESRGAN (BSD-3-Clause). The vendors' runtimes are not shipped: TensorRT-RTX
is NVIDIA's, under NVIDIA's licence, and is fetched from NVIDIA by the person
using the program when they ask; OpenVINO (Apache 2.0) likewise from Intel;
ROCm comes from the distribution. The Material 3 component set in
`gui/material-1.18.0/` is MIT, vendored from `ui-libraries/material` of
slint-ui/slint at the tag matching the `slint` dependency — it has no crates.io package, and a UI that changes shape when
someone else tags a release is not a UI anyone can review. One change is
ours, marked in place: the secondary tab bar gives every tab the same width,
so its indicator stays under the tab it marks.
