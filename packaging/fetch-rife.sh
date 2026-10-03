#!/bin/sh
# Fetches the RIFE plugin for VapourSynth and the two models the player
# offers, into a folder the player finds (`rife` beside the executable, or
# ANIRUST_RIFE_DIR).
#
#   packaging/fetch-rife.sh linux|windows|macos-arm64|macos-x86_64 DEST
#
# Plugin: VapourSynth-RIFE-ncnn-Vulkan, MIT. Models: RIFE 4.6 and 4.26 from
# Practical-RIFE, MIT. Both licences go along as LICENSE-*.

set -eu

release=${RIFE_RELEASE:-r9_mod_v33}
repo=styler00dollar/VapourSynth-RIFE-ncnn-Vulkan
platform=${1:?platform: linux, windows, macos-arm64 or macos-x86_64}
dest=${2:?destination folder}

case "$platform" in
    linux) asset=librife_linux_x86-64.so; name=librife.so ;;
    windows) asset=librife_windows_x86-64.dll; name=librife.dll ;;
    macos-arm64) asset=librife_macos_arm64.dylib; name=librife.dylib ;;
    macos-x86_64) asset=librife_macos_x86-64.dylib; name=librife.dylib ;;
    *) echo "unknown platform: $platform" >&2; exit 1 ;;
esac

mkdir -p "$dest/models"
curl -fsSL -o "$dest/$name" "https://github.com/$repo/releases/download/$release/$asset"
curl -fsSL -o "$dest/LICENSE-RIFE-plugin" "https://raw.githubusercontent.com/$repo/master/LICENSE"
curl -fsSL -o "$dest/LICENSE-RIFE" "https://raw.githubusercontent.com/hzwer/Practical-RIFE/main/LICENSE"

for model in rife-v4.6_ensembleFalse rife-v4.26_ensembleFalse; do
    mkdir -p "$dest/models/$model"
    for file in flownet.param flownet.bin; do
        curl -fsSL -o "$dest/models/$model/$file" \
            "https://raw.githubusercontent.com/$repo/master/models/$model/$file"
    done
done
echo "$dest"
