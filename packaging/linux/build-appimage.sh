#!/bin/sh
# Builds dist/AniRust-<version>-x86_64.AppImage: one file that runs on glibc
# distributions as old as the one it is built on — the release builds it on
# Ubuntu 24.04 (glibc 2.39), so Ubuntu 24.04, Debian 13, Fedora 40 and newer.
# Distributions with no glibc take the Flatpak instead.
#
# Inside: the programs, libmpv with the VapourSynth filter (build-libmpv.sh)
# and every library it links but the ones that must match the host's GPU
# driver, the RIFE plugin and models, and vs-mlrt for TensorRT, OpenVINO and
# MIGraphX. VapourSynth itself, when the host has it, is the host's.
#
# Needs what build-libmpv.sh and the mlrt scripts need, Rust, rsvg-convert,
# and the windowing libraries the program loads at run time (see
# DLOPENED below) installed so linuxdeploy can find and bundle them.

set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
arch=x86_64

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

appdir="$root/target/AppDir"
rm -rf "$appdir"
mkdir -p "$appdir/usr/bin"
cp target/release/anirust target/release/anirust-cli "$appdir/usr/bin/"

tools="$root/target/appimage-tools"
mkdir -p "$tools"
# Pinned releases with checked hashes, not the moving "continuous" build:
# these tools package what ships, so a swapped upstream binary would poison
# the published AppImage.
ld_tag=1-alpha-20251107-1
plugin_tag=1-alpha-20250213-1
ld_sha=c20cd71e3a4e3b80c3483cef793cda3f4e990aca14014d23c544ca3ce1270b4d
plugin_sha=992d502a248e14ab185448ddf6f6e7d25558cb84d4623c354c3af350c25fccb3
if [ ! -x "$tools/linuxdeploy" ]; then
    curl -fsSLo "$tools/linuxdeploy" \
        "https://github.com/linuxdeploy/linuxdeploy/releases/download/$ld_tag/linuxdeploy-$arch.AppImage"
    curl -fsSLo "$tools/linuxdeploy-plugin-appimage" \
        "https://github.com/linuxdeploy/linuxdeploy-plugin-appimage/releases/download/$plugin_tag/linuxdeploy-plugin-appimage-$arch.AppImage"
    (cd "$tools" && printf '%s  %s\n' "$ld_sha" linuxdeploy "$plugin_sha" linuxdeploy-plugin-appimage | sha256sum -c -)
    chmod +x "$tools/linuxdeploy" "$tools/linuxdeploy-plugin-appimage"
fi
PATH="$tools:$PATH"
# Where FUSE is missing (CI runners, containers) the tools unpack themselves.
APPIMAGE_EXTRACT_AND_RUN=1
export PATH APPIMAGE_EXTRACT_AND_RUN

# The windowing libraries winit loads with dlopen, which ldd — and so
# linuxdeploy — never sees. libGL, libEGL, libvulkan, libxcb, libX11-xcb and
# the wayland libraries stay out on purpose: they must match the host's GPU
# driver, and the AppImage exclude list forbids bundling them.
DLOPENED="libxkbcommon.so.0 libxkbcommon-x11.so.0 libxcb-xkb.so.1 libXcursor.so.1 libXi.so.6 libXrandr.so.2"
set --
for lib in $DLOPENED; do
    path=$(ldconfig -p | awk -v l="$lib" '$1 == l { print $NF; exit }')
    [ -n "$path" ] || { echo "$lib is not installed" >&2; exit 1; }
    set -- "$@" --library "$path"
done

icons="$root/target/appimage-icons"
mkdir -p "$icons"
cp packaging/icons/anirust.svg "$icons/io.github.mrfrok.AniRust.svg"
rsvg-convert -w 256 -h 256 packaging/icons/anirust.svg -o "$icons/io.github.mrfrok.AniRust.png"

# libmpv as built here, with the filter, ahead of any the system has.
LD_LIBRARY_PATH="$(dirname "$libmpv")${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export LD_LIBRARY_PATH
cd "$root/target"
rm -f ./*.AppImage
linuxdeploy \
    --appdir "$appdir" \
    --executable "$appdir/usr/bin/anirust" \
    --executable "$appdir/usr/bin/anirust-cli" \
    --library "$libmpv" \
    "$@" \
    --desktop-file "$root/packaging/linux/io.github.mrfrok.AniRust.desktop" \
    --icon-file "$icons/io.github.mrfrok.AniRust.png" \
    --icon-file "$icons/io.github.mrfrok.AniRust.svg"

# The plugins go in after the libraries: they link to the vendors' runtimes
# (TensorRT-RTX, OpenVINO, ROCm) and to Vulkan, which are the host's or
# fetched by the player, never bundled, and linuxdeploy would stop on them.
# Where the program looks for them: ../lib/anirust from its own folder.
mkdir -p "$appdir/usr/lib/anirust/mlrt"
cp -r "$rife" "$appdir/usr/lib/anirust/rife"
cp -r "$mlrt/vsmlrt.py" "$mlrt/models" "$mlrt"/LICENSE-* "$mlrt"/lib*.so "$appdir/usr/lib/anirust/mlrt/"
linuxdeploy-plugin-appimage --appdir "$appdir"

mkdir -p "$root/dist"
name="AniRust-$version-$arch.AppImage"
mv ./*.AppImage "$root/dist/$name"
echo "dist/$name"
