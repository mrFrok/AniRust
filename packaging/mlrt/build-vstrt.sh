#!/bin/sh
# Builds vstrt_rtx — vs-mlrt's TensorRT-RTX filter for VapourSynth — for
# Linux, ported to VapourSynth's API 4 by vstrt-api4.patch.
#
# vs-mlrt (https://github.com/AmusementClub/vs-mlrt, GPL-3.0) still speaks
# API 3, which VapourSynth stopped loading in R73; the patch moves vstrt to
# API 4 without changing what it does or the names vsmlrt.py calls. The
# result is ours to ship under the GPL. TensorRT-RTX itself is not: the
# build links against NVIDIA's SDK, downloaded here for that alone, and at
# run time the player loads the copy the user fetched from NVIDIA.
#
# Needs cmake, ninja, a C++20 compiler, curl and tar. Writes
# $OUT_DIR/libvstrt_rtx.so (target/mlrt by default).

set -eu

vs_mlrt=${VS_MLRT_COMMIT:-44033da39922b7abebd9b680cd0cd42a9b6ca6de}
trt_rtx_url=${TRT_RTX_URL:-https://developer.nvidia.com/downloads/trt/rtx_sdk/secure/1.3/TensorRT-RTX-1.3.0.35-Linux-x86_64-cuda-13.1-Release-external.tar.gz}
cuda=${CUDA_REDIST:-13.1.80}
cccl=${CCCL_REDIST:-13.1.78}
vs_version=${VS_VERSION:-R72}
root=$(cd "$(dirname "$0")/../.." && pwd)
work=${WORK_DIR:-$root/target/mlrt-build}
out=${OUT_DIR:-$root/target/mlrt}

mkdir -p "$work" "$out"
cd "$work"

fetch() { # url file
    curl -fsSL --retry 3 -o "$2" "$1"
}

if [ ! -d vs-mlrt ]; then
    fetch "https://github.com/AmusementClub/vs-mlrt/archive/$vs_mlrt.tar.gz" vs-mlrt.tar.gz
    tar xzf vs-mlrt.tar.gz
    mv "vs-mlrt-$vs_mlrt" vs-mlrt
    (
        cd vs-mlrt
        patch -p1 < "$root/packaging/mlrt/vstrt-api4.patch"
        # vstrt's CMake names the build after `git describe`.
        git init -q
        git add -A
        git -c user.name=anirust -c user.email=anirust@localhost commit -qm "vs-mlrt $vs_mlrt"
        git tag "vs-mlrt-$vs_mlrt"
    )
fi

if [ ! -d trt-rtx ]; then
    fetch "$trt_rtx_url" trt-rtx.tar.gz
    tar xzf trt-rtx.tar.gz
    mv TensorRT-RTX-*/ trt-rtx
fi

# The CUDA pieces the build looks for — runtime, headers and the compiler
# CMake uses to recognise a toolkit — from NVIDIA's redistributables.
if [ ! -x cuda/bin/nvcc ]; then
    mkdir -p cuda
    redist=https://developer.download.nvidia.com/compute/cuda/redist
    for part in "cuda_cudart/linux-x86_64/cuda_cudart-linux-x86_64-$cuda" \
        "cuda_nvcc/linux-x86_64/cuda_nvcc-linux-x86_64-$cuda" \
        "cuda_crt/linux-x86_64/cuda_crt-linux-x86_64-$cuda" \
        "libnvvm/linux-x86_64/libnvvm-linux-x86_64-$cuda" \
        "cuda_cccl/linux-x86_64/cuda_cccl-linux-x86_64-$cccl"; do
        curl -fsSL --retry 3 "$redist/$part-archive.tar.xz" | tar xJ -C cuda --strip-components=1
    done
fi

if [ ! -d "vapoursynth-$vs_version" ]; then
    fetch "https://github.com/vapoursynth/vapoursynth/archive/refs/tags/$vs_version.tar.gz" vs.tar.gz
    tar xzf vs.tar.gz
fi

cmake -S vs-mlrt/vstrt -B build -G Ninja \
    -DCMAKE_BUILD_TYPE=Release \
    -DTENSORRT_HOME="$work/trt-rtx" \
    -DVAPOURSYNTH_INCLUDE_DIRECTORY="$work/vapoursynth-$vs_version/include" \
    -DCUDAToolkit_ROOT="$work/cuda"
ninja -C build
cp build/libvstrt_rtx.so "$out/libvstrt_rtx.so"
echo "$out/libvstrt_rtx.so"
