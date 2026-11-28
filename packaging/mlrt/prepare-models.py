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
RIFE_WARP_STAMP = "models/rife/.split-warp"

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


def patch_vsmlrt(path: pathlib.Path) -> None:
    """Lets an fp16 network be built as it is, without converting it.

    vsmlrt expects every TensorRT-RTX network to be fp32 and converts one
    itself when asked for fp16. The AnimeJaNai network the player fetches is
    fp16 already, with fp16 inputs and outputs, and its author publishes no
    fp32 copy. The conversion would need the `onnx` package on the viewer's
    machine; the plugin builds the network as it stands if the guard is
    lifted, which is what TensorRT-RTX does best anyway.
    """
    old = (
        "    elif fp16_io:\n"
        "        raise ValueError('tensorrt_rtx: \"fp16\" must be True.')"
    )
    new = (
        "    elif fp16_io:\n"
        "        # AniRust: the network is fp16 already; only its I/O\n"
        "        # formats need saying. No conversion, no onnx at run time.\n"
        "        pass"
    )
    text = path.read_text(encoding="utf-8")
    if old not in text:
        raise SystemExit(f"{path} has no fp16 guard to patch")
    path.write_text(text.replace(old, new), encoding="utf-8")


def split_rife_warp(model) -> None:
    """Warp the two frames separately in RIFE's first GridSample.

    TensorRT-RTX 1.3 miscompiles the batched first warp at 720p, producing
    distorted edges even between two identical frames. A pair of batch-one
    warps gives the same result as the ONNX graph without this artefact.
    """
    from onnx import TensorProto, helper

    graph = model.graph
    index = next(i for i, node in enumerate(graph.node) if node.op_type == "GridSample")
    warp = graph.node[index]
    fields = {attr.name: helper.get_attribute_value(attr) for attr in warp.attribute}
    graph.initializer.append(helper.make_tensor("warp_split", TensorProto.INT64, [2], [1, 1]))
    nodes = [
        helper.make_node(
            "Split", [warp.input[0], "warp_split"], ["warp_frame_a", "warp_frame_b"], axis=0
        ),
        helper.make_node(
            "Split", [warp.input[1], "warp_split"], ["warp_grid_a", "warp_grid_b"], axis=0
        ),
        helper.make_node("GridSample", ["warp_frame_a", "warp_grid_a"], ["warped_a"], **fields),
        helper.make_node("GridSample", ["warp_frame_b", "warp_grid_b"], ["warped_b"], **fields),
        helper.make_node("Concat", ["warped_a", "warped_b"], list(warp.output), axis=0),
    ]
    del graph.node[index]
    for offset, node in enumerate(nodes):
        graph.node.insert(index + offset, node)


def to_fp16(source: pathlib.Path, target: pathlib.Path, *, rife: bool) -> None:
    import onnx
    from onnxconverter_common.float16 import convert_float_to_float16

    model = onnx.load(source)
    if rife:
        split_rife_warp(model)
    with warnings.catch_warnings():
        # Every weight too small for fp16 is reported; there are thousands,
        # and rounding them to the nearest fp16 value is the point.
        warnings.simplefilter("ignore")
        model = convert_float_to_float16(model, keep_io_types=True)
    onnx.checker.check_model(model)
    target.parent.mkdir(parents=True, exist_ok=True)
    onnx.save(model, target)


def main() -> None:
    out = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "target/mlrt")
    out.mkdir(parents=True, exist_ok=True)

    fetch(
        f"https://raw.githubusercontent.com/AmusementClub/vs-mlrt/{VS_MLRT_COMMIT}/scripts/vsmlrt.py",
        out / "vsmlrt.py",
    )
    patch_vsmlrt(out / "vsmlrt.py")
    for name, url in LICENSES.items():
        fetch(url, out / name)

    with tempfile.TemporaryDirectory() as tmp:
        tmp = pathlib.Path(tmp)
        stamp = out / RIFE_WARP_STAMP
        for tag, archive, member in MODELS:
            target = out / "models" / member
            rife = member.startswith("rife/")
            if target.is_file() and (not rife or stamp.is_file()):
                continue
            fetch(f"{RELEASES}/{tag}/{archive}", tmp / archive)
            subprocess.run(
                ["7z", "x", "-y", f"-o{tmp / 'x'}", str(tmp / archive)],
                check=True,
                stdout=subprocess.DEVNULL,
            )
            to_fp16(tmp / "x" / member, target, rife=rife)
            print(target)
        stamp.touch()


if __name__ == "__main__":
    main()
