#!/bin/sh
# Builds vstrt_rtx — vs-mlrt's TensorRT-RTX filter for VapourSynth — for
# Linux, or for Windows from Git Bash in an MSVC environment, ported to
# VapourSynth's API 4 by port-api4.py.
#
# vs-mlrt (https://github.com/AmusementClub/vs-mlrt, GPL-3.0) still speaks
# API 3, which VapourSynth stopped loading in R73; the port moves vstrt to
# API 4 without changing what it does or the names vsmlrt.py calls. The
# result is ours to ship under the GPL. TensorRT-RTX itself is not: the
# build links against NVIDIA's SDK, downloaded here for that alone, and at
# run time the player loads the copy the user fetched from NVIDIA — the same
# version, so the two match (1.3 on Linux, 1.6 on Windows: the builds NVIDIA
# publishes for each).
#
# Needs cmake, ninja, git, a C++20 compiler, curl, tar or unzip, and python.
# Writes $OUT_DIR/libvstrt_rtx.so or vstrt_rtx.dll (target/mlrt by default).

set -eu

case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN*) windows=1 ;;
    *) windows=0 ;;
esac

vs_mlrt=${VS_MLRT_COMMIT:-44033da39922b7abebd9b680cd0cd42a9b6ca6de}
cuda=${CUDA_REDIST:-13.1.80}
cccl=${CCCL_REDIST:-13.1.78}
vs_version=${VS_VERSION:-R72}
if [ "$windows" = 1 ]; then
    trt_rtx_url=${TRT_RTX_URL:-https://developer.nvidia.com/downloads/trt/rtx_sdk/secure/1.6/TensorRT-RTX-1.6.1.120-Windows-amd64-cuda-13.4-Release-external.zip}
    platform=windows-x86_64
    ext=zip
else
    trt_rtx_url=${TRT_RTX_URL:-https://developer.nvidia.com/downloads/trt/rtx_sdk/secure/1.3/TensorRT-RTX-1.3.0.35-Linux-x86_64-cuda-13.1-Release-external.tar.gz}
    platform=linux-x86_64
    ext=tar.xz
fi
root=$(cd "$(dirname "$0")/../.." && pwd)
work=${WORK_DIR:-$root/target/mlrt-build}
out=${OUT_DIR:-$root/target/mlrt}
# Windows' "python3" may be the Microsoft Store's stub.
if [ "$windows" = 1 ]; then py=python; else py=$(command -v python3 || command -v python); fi

mkdir -p "$work" "$out"
cd "$work"
work=$(pwd)

fetch() { # url file
    curl -fsSL --retry 3 -o "$2" "$1"
}

# A path as the native tools take it: cmake on Windows does not read
# Git Bash's /d/a/... paths.
native() {
    if [ "$windows" = 1 ]; then cygpath -m "$1"; else printf '%s\n' "$1"; fi
}

unpack() { # archive directory — the archive's single top folder becomes it
    rm -rf unpack.tmp
    mkdir unpack.tmp
    case "$1" in
        # Git for Windows may lack unzip; the runners have 7-Zip.
        *.zip) if command -v unzip >/dev/null; then unzip -q "$1" -d unpack.tmp; else 7z x -y -ounpack.tmp "$1" >/dev/null; fi ;;
        *.tar.xz) tar xJf "$1" -C unpack.tmp ;;
        *) tar xzf "$1" -C unpack.tmp ;;
    esac
    mkdir -p "$2"
    cp -R unpack.tmp/*/. "$2/"
    rm -rf unpack.tmp
}

if [ ! -d vs-mlrt ]; then
    fetch "https://github.com/AmusementClub/vs-mlrt/archive/$vs_mlrt.tar.gz" vs-mlrt.tar.gz
    tar xzf vs-mlrt.tar.gz
    mv "vs-mlrt-$vs_mlrt" vs-mlrt
    (
        cd vs-mlrt
        # vs-mlrt's CMake names the build after `git describe`.
        git init -q
        git add -A
        git -c user.name=anirust -c user.email=anirust@localhost commit -qm "vs-mlrt $vs_mlrt"
        git tag "vs-mlrt-$vs_mlrt"
    )
fi
if [ ! -f vs-mlrt/vstrt/.anirust-ported ]; then
    "$py" "$(native "$root/packaging/mlrt/port-api4.py")" "$(native "$work/vs-mlrt/vstrt")"
    touch vs-mlrt/vstrt/.anirust-ported
fi

if [ ! -d trt-rtx ]; then
    case "$trt_rtx_url" in
        *.zip) archive=trt-rtx.zip ;;
        *) archive=trt-rtx.tar.gz ;;
    esac
    fetch "$trt_rtx_url" "$archive"
    unpack "$archive" trt-rtx
    rm "$archive"
fi

# The CUDA pieces the build looks for — runtime, headers and the compiler
# CMake uses to recognise a toolkit — from NVIDIA's redistributables.
if [ ! -d cuda/bin ]; then
    redist=https://developer.download.nvidia.com/compute/cuda/redist
    for part in "cuda_cudart/$platform/cuda_cudart-$platform-$cuda" \
        "cuda_nvcc/$platform/cuda_nvcc-$platform-$cuda" \
        "cuda_crt/$platform/cuda_crt-$platform-$cuda" \
        "libnvvm/$platform/libnvvm-$platform-$cuda" \
        "cuda_cccl/$platform/cuda_cccl-$platform-$cccl"; do
        fetch "$redist/$part-archive.$ext" "cuda-part.$ext"
        unpack "cuda-part.$ext" cuda
        rm "cuda-part.$ext"
    done
fi

if [ ! -d "vapoursynth-$vs_version" ]; then
    fetch "https://github.com/vapoursynth/vapoursynth/archive/refs/tags/$vs_version.tar.gz" vs.tar.gz
    tar xzf vs.tar.gz
fi

if [ "$windows" = 1 ]; then
    # TensorRT-RTX names its Windows libraries by version; the C runtime is
    # linked statically, as vs-mlrt builds it.
    set -- -DTENSORRT_LIBRARY_SUFFIX=_1_6 -DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded
    built=vstrt_rtx.dll
else
    set --
    built=libvstrt_rtx.so
fi
cmake -S "$(native "$work/vs-mlrt/vstrt")" -B "$(native "$work/build-vstrt")" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release \
    -DTENSORRT_HOME="$(native "$work/trt-rtx")" \
    -DVAPOURSYNTH_INCLUDE_DIRECTORY="$(native "$work/vapoursynth-$vs_version/include")" \
    -DCUDAToolkit_ROOT="$(native "$work/cuda")" \
    "$@"
ninja -C build-vstrt
cp "build-vstrt/$built" "$out/$built"
echo "$out/$built"
