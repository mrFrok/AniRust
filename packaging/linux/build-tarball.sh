#!/bin/sh
# Builds the Linux archive, dist/anirust-<version>-linux-x86_64.tar.gz:
#
#   anirust, anirust-cli   launchers (launcher.sh): they use lib/libmpv.so.2
#                          when the system has its libraries, the system's
#                          libmpv otherwise, and run bin/ of the same name
#   bin/                   the programs
#   lib/libmpv.so.2        libmpv with the VapourSynth filter (build-libmpv.sh)
#   rife/                  the RIFE plugin and models (../fetch-rife.sh)
#   mlrt/                  vs-mlrt for the GPUs' matrix units: vsmlrt.py, the
#                          networks (../mlrt/prepare-models.py) and the
#                          plugins for TensorRT, OpenVINO and MIGraphX
#                          (../mlrt/build-*)
#   anirust.svg, README.md, README.ru.md, LICENSE
#
# The archive's libmpv only matches distributions with the same ffmpeg and
# libplacebo as the one it was built on; elsewhere the launchers fall back to
# the system's libmpv, and everything but frame generation works the same.
# VapourSynth is optional either way, and so are the vendors' runtimes the
# vs-mlrt plugins load: the player fetches TensorRT-RTX or OpenVINO when
# asked, and finds ROCm where the distribution installed it.

set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)

libmpv=${LIBMPV:-$root/target/libmpv/libmpv.so.2}
[ -f "$libmpv" ] || OUT_DIR="$(dirname "$libmpv")" packaging/linux/build-libmpv.sh

rife=${RIFE_DIR:-$root/target/rife-linux}
[ -f "$rife/librife.so" ] || packaging/fetch-rife.sh linux "$rife"

mlrt=${MLRT_DIR:-$root/target/mlrt}
[ -f "$mlrt/vsmlrt.py" ] || python3 packaging/mlrt/prepare-models.py "$mlrt"
[ -f "$mlrt/libvstrt_rtx.so" ] || OUT_DIR="$mlrt" packaging/mlrt/build-vstrt.sh
[ -f "$mlrt/libvsov.so" ] || OUT_DIR="$mlrt" packaging/mlrt/build-vsov.sh
[ -f "$mlrt/libvsmigx.so" ] || OUT_DIR="$mlrt" packaging/mlrt/build-vsmigx.sh

cargo build --release --locked -p anirust-gui -p anirust-cli

name="anirust-$version-linux-x86_64"
stage="$root/target/linux-stage/$name"
rm -rf "$stage"
mkdir -p "$stage/bin" "$stage/lib" "$root/dist"
cp target/release/anirust target/release/anirust-cli "$stage/bin/"
install -m755 packaging/linux/launcher.sh "$stage/anirust"
install -m755 packaging/linux/launcher.sh "$stage/anirust-cli"
cp "$libmpv" "$stage/lib/libmpv.so.2"
cp -r "$rife" "$stage/rife"
mkdir -p "$stage/mlrt"
cp -r "$mlrt/vsmlrt.py" "$mlrt/models" "$mlrt"/LICENSE-* "$stage/mlrt/"
cp "$mlrt"/lib*.so "$stage/mlrt/"
cp packaging/icons/anirust.svg README.md README.ru.md LICENSE "$stage/"

tar -C "$stage/.." -czf "$root/dist/$name.tar.gz" "$name"
echo "dist/$name.tar.gz"
