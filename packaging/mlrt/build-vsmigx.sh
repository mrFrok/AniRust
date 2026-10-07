#!/bin/sh
# Builds vsmigx — vs-mlrt's MIGraphX filter for VapourSynth — for Linux,
# ported to VapourSynth's API 4 by port-api4.py.
#
# MIGraphX runs the networks on AMD GPUs through ROCm. Unlike TensorRT-RTX
# and OpenVINO it is not fetched by the player: ROCm is installed from the
# distribution, and the plugin links to two of its libraries by soname,
# libmigraphx_c.so.3 and libamdhip64.so.7 — ROCm 7, as Arch and CachyOS
# have it.
#
# The plugin needs only those two libraries and their headers to build, not
# the whole of ROCm and the gigabytes MIGraphX's CMake package pulls in, so
# it is compiled directly. With ROCM_PATH unset and no /opt/rocm, the four
# Arch packages that hold them are fetched from an Arch mirror and unpacked
# under the work folder — nothing is installed.
#
# Needs a C++20 compiler, curl, tar with zstd, and python3. Writes
# $OUT_DIR/libvsmigx.so (target/mlrt by default).

set -eu

vs_mlrt=${VS_MLRT_COMMIT:-44033da39922b7abebd9b680cd0cd42a9b6ca6de}
vs_version=${VS_VERSION:-R72}
mirror=${ARCH_MIRROR:-https://geo.mirror.pkgbuild.com}
root=$(cd "$(dirname "$0")/../.." && pwd)
work=${WORK_DIR:-$root/target/mlrt-build}
out=${OUT_DIR:-$root/target/mlrt}
cxx=${CXX:-c++}

mkdir -p "$work" "$out"
cd "$work"

fetch() { # url file
    curl -fsSL --retry 3 -o "$2" "$1"
}

if [ ! -d vs-mlrt ]; then
    fetch "https://github.com/AmusementClub/vs-mlrt/archive/$vs_mlrt.tar.gz" vs-mlrt.tar.gz
    tar xzf vs-mlrt.tar.gz
    mv "vs-mlrt-$vs_mlrt" vs-mlrt
fi
if [ ! -f vs-mlrt/vsmigx/.anirust-ported ]; then
    python3 "$root/packaging/mlrt/port-api4.py" vs-mlrt/vsmigx
    touch vs-mlrt/vsmigx/.anirust-ported
fi

if [ ! -d "vapoursynth-$vs_version" ]; then
    fetch "https://github.com/vapoursynth/vapoursynth/archive/refs/tags/$vs_version.tar.gz" vs.tar.gz
    tar xzf vs.tar.gz
fi

rocm=${ROCM_PATH:-/opt/rocm}
if [ ! -f "$rocm/include/migraphx/migraphx.h" ]; then
    rocm="$work/rocm/opt/rocm"
    if [ ! -f "$rocm/include/migraphx/migraphx.h" ]; then
        mkdir -p "$work/rocm/db"
        fetch "$mirror/extra/os/x86_64/extra.db" "$work/rocm/extra.db"
        tar xzf "$work/rocm/extra.db" -C "$work/rocm/db"
        for package in migraphx hip-runtime-amd rocm-core hsa-rocr; do
            desc=$(ls -d "$work/rocm/db/$package"-[0-9]*/desc 2>/dev/null | head -n 1)
            [ -n "$desc" ] || { echo "$package is not in Arch's extra repository" >&2; exit 1; }
            file=$(sed -n '/%FILENAME%/{n;p}' "$desc")
            fetch "$mirror/extra/os/x86_64/$file" "$work/rocm/$file"
            tar --zstd -xf "$work/rocm/$file" -C "$work/rocm"
        done
    fi
fi

mkdir -p build-vsmigx
printf '#define VERSION "vs-mlrt-%s"\n' "$vs_mlrt" > build-vsmigx/config.h
"$cxx" -std=c++20 -O2 -shared -fPIC -D__HIP_PLATFORM_AMD__ \
    -I"$work/vapoursynth-$vs_version/include" -I"$rocm/include" -Ibuild-vsmigx \
    vs-mlrt/vsmigx/vs_migraphx.cpp \
    -L"$rocm/lib" -Wl,--no-undefined -lmigraphx_c -lamdhip64 \
    -o build-vsmigx/libvsmigx.so
cp build-vsmigx/libvsmigx.so "$out/libvsmigx.so"
echo "$out/libvsmigx.so"
