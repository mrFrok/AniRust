#!/bin/sh
# Builds vsov — vs-mlrt's OpenVINO filter for VapourSynth — for Linux, or
# for Windows from Git Bash in an MSVC environment, ported to VapourSynth's
# API 4 by port-api4.py.
#
# OpenVINO runs the networks on Intel GPUs (and on any CPU). vsov reads the
# ONNX networks itself, through onnx and protobuf built here as static
# libraries at the commits OpenVINO 2024.6 uses, so the plugin depends on
# nothing but OpenVINO's runtime. That runtime (Apache-2.0) is fetched by the
# player when asked, like TensorRT-RTX; the build links against the same
# archive.
#
# Needs cmake, ninja, git, a C++17 compiler, curl, tar or unzip, and python.
# Writes $OUT_DIR/libvsov.so or vsov.dll (target/mlrt by default).

set -eu

case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN*) windows=1 ;;
    *) windows=0 ;;
esac

vs_mlrt=${VS_MLRT_COMMIT:-44033da39922b7abebd9b680cd0cd42a9b6ca6de}
if [ "$windows" = 1 ]; then
    openvino_url=${OPENVINO_URL:-https://storage.openvinotoolkit.org/repositories/openvino/packages/2024.6/windows/w_openvino_toolkit_windows_2024.6.0.17404.4c0f47d2335_x86_64.zip}
else
    openvino_url=${OPENVINO_URL:-https://storage.openvinotoolkit.org/repositories/openvino/packages/2024.6/linux/l_openvino_toolkit_ubuntu24_2024.6.0.17404.4c0f47d2335_x86_64.tgz}
fi
protobuf_ref=f0dc78d7e6e331b8c6bb2d5283e06aa26883ca7c
onnx_ref=b8baa8446686496da4cc8fda09f2b6fe65c2a02c
vs_version=${VS_VERSION:-R72}
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
        *) tar xzf "$1" -C unpack.tmp ;;
    esac
    mkdir -p "$2"
    cp -R unpack.tmp/*/. "$2/"
    rm -rf unpack.tmp
}

checkout() { # repository commit directory
    if [ ! -d "$3" ]; then
        git init -q "$3"
        git -C "$3" fetch -q --depth 1 "https://github.com/$1" "$2"
        git -C "$3" checkout -q FETCH_HEAD
    fi
}

if [ "$windows" = 1 ]; then
    # Everything linked into the plugin uses the static C runtime, as
    # vs-mlrt builds it.
    runtime=-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded
    exe=.exe
else
    runtime=-DCMAKE_POSITION_INDEPENDENT_CODE=ON
    exe=
fi

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
if [ ! -f vs-mlrt/vsov/.anirust-ported ]; then
    "$py" "$(native "$root/packaging/mlrt/port-api4.py")" "$(native "$work/vs-mlrt/vsov")"
    touch vs-mlrt/vsov/.anirust-ported
fi

if [ ! -d protobuf/install ]; then
    checkout protocolbuffers/protobuf "$protobuf_ref" protobuf
    # From the top folder: the old cmake/ entry point starts the project
    # before protobuf chooses MSVC's runtime, so the library would come out
    # with the DLL runtime whatever is asked, and fail to link.
    cmake -S "$(native "$work/protobuf")" -B "$(native "$work/protobuf/build")" -G Ninja \
        -DCMAKE_BUILD_TYPE=Release -DCMAKE_POSITION_INDEPENDENT_CODE=ON "$runtime" \
        -Dprotobuf_BUILD_SHARED_LIBS=OFF -Dprotobuf_BUILD_TESTS=OFF \
        -Dprotobuf_MSVC_STATIC_RUNTIME=ON
    cmake --build "$(native "$work/protobuf/build")"
    cmake --install "$(native "$work/protobuf/build")" --prefix "$(native "$work/protobuf/install")"
fi

if [ ! -d onnx/install ]; then
    checkout onnx/onnx "$onnx_ref" onnx
    cmake -S "$(native "$work/onnx")" -B "$(native "$work/onnx/build")" -G Ninja \
        -DCMAKE_BUILD_TYPE=Release -DCMAKE_POSITION_INDEPENDENT_CODE=ON "$runtime" \
        -DProtobuf_PROTOC_EXECUTABLE="$(native "$work/protobuf/install/bin/protoc$exe")" \
        -DProtobuf_LITE_LIBRARY="$(native "$work/protobuf/install/lib")" \
        -DProtobuf_LIBRARIES="$(native "$work/protobuf/install/lib")" \
        -DONNX_USE_LITE_PROTO=ON -DONNX_USE_PROTOBUF_SHARED_LIBS=OFF \
        -DONNX_GEN_PB_TYPE_STUBS=OFF -DONNX_ML=0 -DONNX_USE_MSVC_STATIC_RUNTIME=1
    cmake --build "$(native "$work/onnx/build")"
    cmake --install "$(native "$work/onnx/build")" --prefix "$(native "$work/onnx/install")"
fi

if [ ! -d openvino ]; then
    case "$openvino_url" in
        *.zip) archive=openvino.zip ;;
        *) archive=openvino.tgz ;;
    esac
    fetch "$openvino_url" "$archive"
    unpack "$archive" openvino
    rm "$archive"
fi

if [ ! -d "vapoursynth-$vs_version" ]; then
    fetch "https://github.com/vapoursynth/vapoursynth/archive/refs/tags/$vs_version.tar.gz" vs.tar.gz
    tar xzf vs.tar.gz
fi

if [ "$windows" = 1 ]; then
    # OpenVINO as a DLL beside the plugin's own delay-loading, not linked in.
    set -- -DWIN32_SHARED_OPENVINO=ON "$runtime"
    built=vsov.dll
else
    set --
    built=libvsov.so
fi
cmake -S "$(native "$work/vs-mlrt/vsov")" -B "$(native "$work/build-vsov")" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release \
    -DVAPOURSYNTH_INCLUDE_DIRECTORY="$(native "$work/vapoursynth-$vs_version/include")" \
    -DOpenVINO_DIR="$(native "$work/openvino/runtime/cmake")" \
    -DONNX_DIR="$(native "$work/onnx/install/lib/cmake/ONNX")" \
    -DCMAKE_PREFIX_PATH="$(native "$work/protobuf/install");$(native "$work/onnx/install")" \
    "$@"
ninja -C build-vsov
cp "build-vsov/$built" "$out/$built"
echo "$out/$built"
