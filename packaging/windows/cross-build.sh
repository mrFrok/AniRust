#!/bin/sh
# Builds the Windows client from Linux with MinGW and packs it:
# dist/anirust-<version>-windows-x86_64.zip with anirust.exe,
# anirust-cli.exe, libmpv-2.dll, the RIFE plugin and models in rife/, the
# READMEs and the licence.
#
# Needs the Rust target (`rustup target add x86_64-pc-windows-gnu`), a
# MinGW-w64 toolchain (`x86_64-w64-mingw32-gcc`), `7z`, `curl` and `zip`.
# libmpv is shinchiro's build, the one mpv's own site points Windows users
# at; its archive carries the DLL and the MinGW import library the linker
# takes as is. Set MPV_DIR to a folder holding `libmpv-2.dll` and
# `libmpv.dll.a` to skip the download.

set -eu

target=x86_64-pc-windows-gnu
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
mpv=${MPV_DIR:-$root/target/windows-mpv}

if [ ! -f "$mpv/libmpv.dll.a" ]; then
    echo "fetching libmpv for Windows"
    # With a token, if there is one: without, the API allows 60 requests an hour.
    url=$(curl -fsSL ${GITHUB_TOKEN:+-H "Authorization: Bearer $GITHUB_TOKEN"} \
        https://api.github.com/repos/shinchiro/mpv-winbuild-cmake/releases/latest |
        grep -o '"browser_download_url": *"[^"]*/mpv-dev-x86_64-[0-9]\{8\}-git-[0-9a-f]*\.7z"' |
        head -n 1 | sed 's/.*"\(https[^"]*\)"/\1/')
    [ -n "$url" ] || { echo "no mpv-dev-x86_64 archive in the latest release" >&2; exit 1; }
    mkdir -p "$mpv"
    curl -fL --retry 3 -o "$mpv/mpv-dev.7z" "$url"
    7z x "$mpv/mpv-dev.7z" "-o$mpv" -y >/dev/null
    rm "$mpv/mpv-dev.7z"
fi

# `-lmpv` resolves to libmpv.dll.a in this folder.
RUSTFLAGS="${RUSTFLAGS:-} -L native=$mpv" \
    cargo build --release --locked --target "$target" -p anirust-gui -p anirust-cli

name="anirust-$version-windows-x86_64"
stage="$root/target/windows-stage/$name"
rm -rf "$stage"
mkdir -p "$stage" "$root/dist"
cp "target/$target/release/anirust.exe" "target/$target/release/anirust-cli.exe" \
    "$mpv/libmpv-2.dll" README.md README.ru.md LICENSE "$stage/"

# Frame generation: the RIFE plugin and models, beside the program. VapourSynth
# itself is installed separately (README); shinchiro's libmpv has the filter.
rife=${RIFE_DIR:-$root/target/rife-windows}
[ -f "$rife/librife.dll" ] || packaging/fetch-rife.sh windows "$rife"
cp -r "$rife" "$stage/rife"

rm -f "$root/dist/$name.zip"
(cd "$stage/.." && zip -qr "$root/dist/$name.zip" "$name")
echo "dist/$name.zip"
