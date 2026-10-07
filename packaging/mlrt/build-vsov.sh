#!/bin/sh
# Builds vsov — vs-mlrt's OpenVINO filter for VapourSynth — for Linux,
# ported to VapourSynth's API 4 by port-api4.py.
#
# OpenVINO runs the networks on Intel GPUs (and on any CPU). vsov reads the
# ONNX networks itself, through onnx and protobuf built here as static
# libraries at the commits OpenVINO 2024.6 uses, so the plugin depends on
# nothing but OpenVINO's runtime. That runtime (Apache-2.0) is fetched by the
# player when asked, like TensorRT-RTX; the build links against the same
# archive.
#
# Needs cmake, ninja, git, a C++17 compiler, curl and tar. Writes
# $OUT_DIR/libvsov.so (target/mlrt by default).

set -eu

vs_mlrt=${VS_MLRT_COMMIT:-44033da39922b7abebd9b680cd0cd42a9b6ca6de}
openvino_url=${OPENVINO_URL:-https://storage.openvinotoolkit.org/repositories/openvino/packages/2024.6/linux/l_openvino_toolkit_ubuntu24_2024.6.0.17404.4c0f47d2335_x86_64.tgz}
protobuf_ref=f0dc78d7e6e331b8c6bb2d5283e06aa26883ca7c
onnx_ref=b8baa8446686496da4cc8fda09f2b6fe65c2a02c
vs_version=${VS_VERSION:-R72}
root=$(cd "$(dirname "$0")/../.." && pwd)
work=${WORK_DIR:-$root/target/mlrt-build}
out=${OUT_DIR:-$root/target/mlrt}

mkdir -p "$work" "$out"
cd "$work"

fetch() { # url file
    curl -fsSL --retry 3 -o "$2" "$1"
}

checkout() { # repository commit directory
    if [ ! -d "$3" ]; then
        git init -q "$3"
        git -C "$3" fetch -q --depth 1 "https://github.com/$1" "$2"
        git -C "$3" checkout -q FETCH_HEAD
    fi
}

if [ ! -d vs-mlrt ]; then
    fetch "https://github.com/AmusementClub/vs-mlrt/archive/$vs_mlrt.tar.gz" vs-mlrt.tar.gz
    tar xzf vs-mlrt.tar.gz
    mv "vs-mlrt-$vs_mlrt" vs-mlrt
    (
        cd vs-mlrt
        git init -q
        git add -A
        git -c user.name=anirust -c user.email=anirust@localhost commit -qm "vs-mlrt $vs_mlrt"
        git tag "vs-mlrt-$vs_mlrt"
    )
fi
if [ ! -f vs-mlrt/vsov/.anirust-ported ]; then
    python3 "$root/packaging/mlrt/port-api4.py" vs-mlrt/vsov
    touch vs-mlrt/vsov/.anirust-ported
fi

if [ ! -d protobuf/install ]; then
    checkout protocolbuffers/protobuf "$protobuf_ref" protobuf
    cmake -S protobuf/cmake -B protobuf/build -G Ninja \
        -DCMAKE_BUILD_TYPE=Release -DCMAKE_POSITION_INDEPENDENT_CODE=ON \
        -Dprotobuf_BUILD_SHARED_LIBS=OFF -Dprotobuf_BUILD_TESTS=OFF
    cmake --build protobuf/build
    cmake --install protobuf/build --prefix protobuf/install
fi

if [ ! -d onnx/install ]; then
    checkout onnx/onnx "$onnx_ref" onnx
    cmake -S onnx -B onnx/build -G Ninja \
        -DCMAKE_BUILD_TYPE=Release -DCMAKE_POSITION_INDEPENDENT_CODE=ON \
        -DProtobuf_PROTOC_EXECUTABLE="$work/protobuf/install/bin/protoc" \
        -DProtobuf_LITE_LIBRARY="$work/protobuf/install/lib" \
        -DProtobuf_LIBRARIES="$work/protobuf/install/lib" \
        -DONNX_USE_LITE_PROTO=ON -DONNX_USE_PROTOBUF_SHARED_LIBS=OFF \
        -DONNX_GEN_PB_TYPE_STUBS=OFF -DONNX_ML=0
    cmake --build onnx/build
    cmake --install onnx/build --prefix onnx/install
fi

if [ ! -d openvino ]; then
    fetch "$openvino_url" openvino.tgz
    tar xzf openvino.tgz
    mv l_openvino_toolkit_* openvino
fi

if [ ! -d "vapoursynth-$vs_version" ]; then
    fetch "https://github.com/vapoursynth/vapoursynth/archive/refs/tags/$vs_version.tar.gz" vs.tar.gz
    tar xzf vs.tar.gz
fi

cmake -S vs-mlrt/vsov -B build-vsov -G Ninja \
    -DCMAKE_BUILD_TYPE=Release \
    -DVAPOURSYNTH_INCLUDE_DIRECTORY="$work/vapoursynth-$vs_version/include" \
    -DOpenVINO_DIR="$work/openvino/runtime/cmake" \
    -DProtobuf_DIR="$work/protobuf/install/lib/cmake/protobuf" \
    -DONNX_DIR="$work/onnx/install/lib/cmake/ONNX" \
    -DCMAKE_PREFIX_PATH="$work/protobuf/install;$work/onnx/install"
ninja -C build-vsov
cp build-vsov/libvsov.so "$out/libvsov.so"
echo "$out/libvsov.so"
