#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Fetches what the player's TensorRT path needs besides the plugin.

- vsmlrt.py, vs-mlrt's Python front end (GPL-3.0), at the commit the plugin
  is built from;
- the networks, from vs-mlrt's model releases: RIFE 4.26 and 4.25 lite for
  frame generation (MIT, Practical-RIFE), Real-ESRGAN AnimeVideo v3 ×2 for
  upscaling (BSD-3-Clause, Real-ESRGAN).

The networks are converted to fp16 here, keeping fp32 inputs and outputs.
TensorRT-RTX runs a network at the precision it is written in, and vsmlrt
would otherwise convert it on the user's machine, which takes the `onnx`
Python package there; done once at build time, the player needs nothing but
VapourSynth.

Needs Python with `onnx` and `onnxconverter-common`, and `7z`.

    packaging/mlrt/prepare-models.py target/mlrt
"""

import pathlib
import shutil
import subprocess
import sys
import tempfile
import urllib.request
import warnings

VS_MLRT_COMMIT = "44033da39922b7abebd9b680cd0cd42a9b6ca6de"
RELEASES = "https://github.com/AmusementClub/vs-mlrt/releases/download"

# (release tag, archive, the file inside it, under models/)
MODELS = [
    ("external-models", "rife_v4.26.7z", "rife/rife_v4.26.onnx"),
    ("external-models", "rife_v4.25_lite.7z", "rife/rife_v4.25_lite.onnx"),
    ("model-20211209", "RealESRGANv2_v1.7z", "RealESRGANv2/RealESRGANv2-animevideo-xsx2.onnx"),
]

LICENSES = {
    "LICENSE-vs-mlrt": f"https://raw.githubusercontent.com/AmusementClub/vs-mlrt/{VS_MLRT_COMMIT}/LICENSE",
    "LICENSE-RIFE": "https://raw.githubusercontent.com/hzwer/Practical-RIFE/main/LICENSE",
    "LICENSE-Real-ESRGAN": "https://raw.githubusercontent.com/xinntao/Real-ESRGAN/master/LICENSE",
}


def fetch(url: str, path: pathlib.Path) -> None:
    with urllib.request.urlopen(url) as response, open(path, "wb") as out:
        shutil.copyfileobj(response, out)


def to_fp16(source: pathlib.Path, target: pathlib.Path) -> None:
    import onnx
    from onnxconverter_common.float16 import convert_float_to_float16

    model = onnx.load(source)
    with warnings.catch_warnings():
        # Every weight too small for fp16 is reported; there are thousands,
        # and rounding them to the nearest fp16 value is the point.
        warnings.simplefilter("ignore")
        model = convert_float_to_float16(model, keep_io_types=True)
    target.parent.mkdir(parents=True, exist_ok=True)
    onnx.save(model, target)


def main() -> None:
    out = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "target/mlrt")
    out.mkdir(parents=True, exist_ok=True)

    fetch(
        f"https://raw.githubusercontent.com/AmusementClub/vs-mlrt/{VS_MLRT_COMMIT}/scripts/vsmlrt.py",
        out / "vsmlrt.py",
    )
    for name, url in LICENSES.items():
        fetch(url, out / name)

    with tempfile.TemporaryDirectory() as tmp:
        tmp = pathlib.Path(tmp)
        for tag, archive, member in MODELS:
            target = out / "models" / member
            if target.is_file():
                continue
            fetch(f"{RELEASES}/{tag}/{archive}", tmp / archive)
            subprocess.run(
                ["7z", "x", "-y", f"-o{tmp / 'x'}", str(tmp / archive)],
                check=True,
                stdout=subprocess.DEVNULL,
            )
            to_fp16(tmp / "x" / member, target)
            print(target)


if __name__ == "__main__":
    main()
