#!/bin/sh
# Builds the Linux archive, dist/anirust-<version>-linux-x86_64.tar.gz:
#
#   anirust, anirust-cli   the programs; they look in lib/ beside themselves
#                          before the system's library directories
#   lib/libmpv.so.2        libmpv with the VapourSynth filter (build-libmpv.sh)
#   rife/                  the RIFE plugin and models (../fetch-rife.sh)
#   anirust.svg, README.md, README.ru.md, LICENSE
#
# The rest of libmpv's dependencies (ffmpeg, libplacebo, libass, ...) come
# from the system, as with any mpv, so the archive runs where this was built
# and on distributions as recent. VapourSynth is optional: without it, frame
# generation is unavailable and everything else works.

set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)

libmpv=${LIBMPV:-$root/target/libmpv/libmpv.so.2}
[ -f "$libmpv" ] || OUT_DIR="$(dirname "$libmpv")" packaging/linux/build-libmpv.sh

rife=${RIFE_DIR:-$root/target/rife-linux}
[ -f "$rife/librife.so" ] || packaging/fetch-rife.sh linux "$rife"

# $ORIGIN reaches the linker as written: lib/ beside the executable.
RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-Wl,-rpath,\$ORIGIN/lib" \
    cargo build --release --locked -p anirust-gui -p anirust-cli

name="anirust-$version-linux-x86_64"
stage="$root/target/linux-stage/$name"
rm -rf "$stage"
mkdir -p "$stage/lib" "$root/dist"
cp target/release/anirust target/release/anirust-cli "$stage/"
cp "$libmpv" "$stage/lib/libmpv.so.2"
cp -r "$rife" "$stage/rife"
cp packaging/icons/anirust.svg README.md README.ru.md LICENSE "$stage/"

tar -C "$stage/.." -czf "$root/dist/$name.tar.gz" "$name"
echo "dist/$name.tar.gz"
