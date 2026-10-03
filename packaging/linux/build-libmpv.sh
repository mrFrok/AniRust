#!/bin/sh
# Builds libmpv with the VapourSynth filter, which frame generation needs and
# distribution builds of mpv usually leave out.
#
# The source is mpv's own release, with one patch
# (mpv-vapoursynth-dlopen.patch): VapourSynth is opened when the filter is
# first used instead of being linked, so this libmpv still loads, and plays
# everything else, on a machine without VapourSynth. NVIDIA's codec headers
# are fetched too, for CUDA decoding; they are headers only.
#
# Needs meson, ninja, a C compiler, and the development files of mpv's usual
# dependencies (ffmpeg, libplacebo, libass, ...) plus VapourSynth's headers —
# on Arch, what the mpv and vapoursynth packages already pulled in.
#
# Writes target/libmpv/libmpv.so.2. The Linux archive puts it in lib/ beside
# the program, which looks there first.

set -eu

mpv_version=${MPV_VERSION:-0.41.0}
nv_headers=${NV_HEADERS:-n13.0.19.0}
root=$(cd "$(dirname "$0")/../.." && pwd)
work=${WORK_DIR:-$root/target/libmpv-build}
out=${OUT_DIR:-$root/target/libmpv}

mkdir -p "$work" "$out"
cd "$work"

if [ ! -d "mpv-$mpv_version" ]; then
    curl -fsSL -o mpv.tar.gz "https://github.com/mpv-player/mpv/archive/refs/tags/v$mpv_version.tar.gz"
    tar xzf mpv.tar.gz
    (cd "mpv-$mpv_version" && patch -p1 < "$root/packaging/linux/mpv-vapoursynth-dlopen.patch")
fi

if [ ! -f deps/lib/pkgconfig/ffnvcodec.pc ]; then
    curl -fsSL -o nv-codec-headers.tar.gz \
        "https://github.com/FFmpeg/nv-codec-headers/archive/refs/tags/$nv_headers.tar.gz"
    tar xzf nv-codec-headers.tar.gz
    make -C "nv-codec-headers-$nv_headers" PREFIX="$work/deps" install >/dev/null
fi

cd "mpv-$mpv_version"
if [ ! -d build ]; then
    PKG_CONFIG_PATH="$work/deps/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}" \
        meson setup build --buildtype=release \
        -Dlibmpv=true -Dcplayer=false -Dvapoursynth=enabled \
        -Dlua=disabled -Djavascript=disabled \
        -Dmanpage-build=disabled -Dhtml-build=disabled -Dpdf-build=disabled
fi
ninja -C build
cp -L build/libmpv.so.2 "$out/libmpv.so.2"
echo "$out/libmpv.so.2"
