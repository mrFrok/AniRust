#!/bin/sh
# Builds libmpv with the VapourSynth filter, which frame generation needs and
# distribution builds of mpv usually leave out.
#
# The source is mpv's own release, with one patch
# (mpv-vapoursynth-dlopen.patch): VapourSynth is opened when the filter is
# first used instead of being linked, so this libmpv still loads, and plays
# everything else, on a machine without VapourSynth. NVIDIA's codec headers
# come along for CUDA decoding, and VapourSynth's headers when the system has
# none; all three are headers only.
#
# Needs meson, ninja, a C compiler, and the development files of mpv's usual
# dependencies (ffmpeg, libplacebo, libass, ...) — on Arch, what the mpv
# package already pulled in.
#
# Sources already unpacked in WORK_DIR (mpv-$MPV_VERSION,
# nv-codec-headers-$NV_HEADERS, vapoursynth-$VS_VERSION) are used as they
# are, which is how a PKGBUILD hands them over; anything missing is fetched.
#
# Writes $OUT_DIR/libmpv.so.2 (target/libmpv by default).

set -eu

mpv_version=${MPV_VERSION:-0.41.0}
nv_headers=${NV_HEADERS:-n13.0.19.0}
vs_version=${VS_VERSION:-R72}
root=$(cd "$(dirname "$0")/../.." && pwd)
work=${WORK_DIR:-$root/target/libmpv-build}
out=${OUT_DIR:-$root/target/libmpv}
patch_file="$root/packaging/linux/mpv-vapoursynth-dlopen.patch"

mkdir -p "$work" "$out"
cd "$work"

fetch() { # url file
    curl -fsSL --retry 3 -o "$2" "$1"
}

if [ ! -d "mpv-$mpv_version" ]; then
    fetch "https://github.com/mpv-player/mpv/archive/refs/tags/v$mpv_version.tar.gz" mpv.tar.gz
    tar xzf mpv.tar.gz
fi
if [ ! -f "mpv-$mpv_version/.anirust-patched" ]; then
    (
        cd "mpv-$mpv_version"
        # A tree patched before the marker existed is left as it is.
        if ! patch -p1 -R --dry-run -s < "$patch_file" >/dev/null 2>&1; then
            patch -p1 < "$patch_file"
        fi
        touch .anirust-patched
    )
fi

deps="$work/deps"
mkdir -p "$deps/lib/pkgconfig"

if [ ! -f "$deps/lib/pkgconfig/ffnvcodec.pc" ]; then
    if [ ! -d "nv-codec-headers-$nv_headers" ]; then
        fetch "https://github.com/FFmpeg/nv-codec-headers/archive/refs/tags/$nv_headers.tar.gz" nv.tar.gz
        tar xzf nv.tar.gz
    fi
    make -C "nv-codec-headers-$nv_headers" PREFIX="$deps" install >/dev/null
fi

# The filter needs VapourSynth's headers and nothing else of it. Debian and
# Ubuntu do not always package them; VapourSynth's source has them.
if ! pkg-config --exists vapoursynth && [ ! -f "$deps/lib/pkgconfig/vapoursynth.pc" ]; then
    if [ ! -d "vapoursynth-$vs_version" ]; then
        fetch "https://github.com/vapoursynth/vapoursynth/archive/refs/tags/$vs_version.tar.gz" vs.tar.gz
        tar xzf vs.tar.gz
    fi
    mkdir -p "$deps/include/vapoursynth"
    cp "vapoursynth-$vs_version"/include/*.h "$deps/include/vapoursynth/"
    cat > "$deps/lib/pkgconfig/vapoursynth.pc" <<PC
Name: vapoursynth
Description: VapourSynth headers, for mpv's filter
Version: ${vs_version#R}
Cflags: -I$deps/include/vapoursynth
PC
fi

cd "mpv-$mpv_version"
if [ ! -d build ]; then
    PKG_CONFIG_PATH="$deps/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}" \
        meson setup build --buildtype=release \
        -Dlibmpv=true -Dcplayer=false -Dvapoursynth=enabled \
        -Dlua=disabled -Djavascript=disabled \
        -Dmanpage-build=disabled -Dhtml-build=disabled -Dpdf-build=disabled
fi
ninja -C build
cp -L build/libmpv.so.2 "$out/libmpv.so.2"
echo "$out/libmpv.so.2"
