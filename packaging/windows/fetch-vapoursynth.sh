#!/bin/sh
# Puts a VapourSynth of its own into the Windows package, so that frame
# generation and the neural networks work without installing anything:
# VapourSynth R73's portable build and the embeddable Python it runs on, in
# one folder the program points libmpv at (VSSCRIPT_PATH).
#
#   packaging/windows/fetch-vapoursynth.sh DEST
#
# R73 is the last VapourSynth whose portable build is self-contained: its
# VSScript.dll finds Python beside itself (portable.vs). From R74 on it reads
# the Python to use from a per-user config file, which a second VapourSynth on
# the machine would share. Our plugins speak API 4, which R73 is.
#
# What pip would do with VapourSynth's wheel is done by hand: it is a zip, and
# its two files go into site-packages. The installer's own script
# (Install-Portable-VapourSynth-R73.ps1) is the reference for the layout.
#
# VapourSynth is LGPL-2.1 and Python is under the PSF licence; both licences
# go along.
#
# Needs curl, sha256sum and 7z or unzip.

set -eu

dest=${1:?destination folder}
vs=73
python=3.13.16
vs_sha=3326f10d0fdcdec45649a474cbc9810795ab3da422634d0f134bca6089afbb91
python_sha=97dae5274cc54867065e8d5a3226e48c35017ed332a0fdb0e27d5b5821961297

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

unzip_to() { # archive directory
    if command -v 7z >/dev/null; then
        7z x -y "-o$2" "$1" >/dev/null
    else
        unzip -q -o "$1" -d "$2"
    fi
}

curl -fsSL --retry 3 -o "$work/python.zip" \
    "https://www.python.org/ftp/python/$python/python-$python-embed-amd64.zip"
curl -fsSL --retry 3 -o "$work/vs.zip" \
    "https://github.com/vapoursynth/vapoursynth/releases/download/R$vs/VapourSynth64-Portable-R$vs.zip"
(cd "$work" && printf '%s  %s\n' "$python_sha" python.zip "$vs_sha" vs.zip | sha256sum -c - >/dev/null)

rm -rf "$dest"
mkdir -p "$dest"
unzip_to "$work/python.zip" "$dest"
unzip_to "$work/vs.zip" "$dest"

# The wheel, installed: the module and the core into site-packages, its
# record beside them.
site="$dest/Lib/site-packages"
mkdir -p "$site"
unzip_to "$dest/wheel/vapoursynth-$vs-cp312-abi3-win_amd64.whl" "$work/wheel"
cp "$work/wheel/vapoursynth.pyd" "$site/"
cp "$work/wheel/vapoursynth-$vs.data/data/Lib/site-packages/vapoursynth.dll" "$site/"
cp -R "$work/wheel/vapoursynth-$vs.dist-info" "$site/"

# The embeddable Python reads only what its ._pth file lists.
pth=$(ls "$dest"/python3*._pth)
printf 'Lib\\site-packages\r\n' >>"$pth"

# What a player has no use for: the Python 3.8 variant, the wheel now
# installed, the documentation and SDK, the archiver, the plugin manager, and
# the AVFS and Video for Windows bridges with Pismo's installer for them.
rm -rf "$dest/VSScriptPython38.dll" "$dest/wheel" "$dest/doc" "$dest/sdk" \
    "$dest/7z.exe" "$dest/7z.dll" "$dest/AVFS.exe" "$dest"/pfm-*.exe \
    "$dest/VSVFW.dll" "$dest/vsrepo.py" "$dest/vsgenstubs.py" "$dest/vsgenstubs4" \
    "$dest/MANIFEST.in" "$dest/setup.py"
[ -f "$dest/portable.vs" ] || { echo "no portable.vs in VapourSynth R$vs" >&2; exit 1; }
[ -f "$dest/VSScript.dll" ] || { echo "no VSScript.dll in VapourSynth R$vs" >&2; exit 1; }

cp "$work/wheel/vapoursynth-$vs.dist-info/licenses/COPYING.LESSER" "$dest/LICENSE-VapourSynth" 2>/dev/null ||
    cp "$work/wheel/vapoursynth-$vs.dist-info/COPYING.LESSER" "$dest/LICENSE-VapourSynth"
mv "$dest/LICENSE.txt" "$dest/LICENSE-Python"
echo "$dest"
