#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Ports a vs-mlrt plugin from VapourSynth's API 3 to API 4.

vs-mlrt's filters (vstrt, vsov, vsmigx) are written against API 3, which
VapourSynth stopped loading in R73. The move is mechanical — renamed types and
functions, a filter created with its dependencies instead of an init
callback, plugins registered through VSPLUGINAPI — and this does it the same
way every time, so the result can be regenerated when vs-mlrt moves on:

    packaging/mlrt/port-api4.py path/to/vs-mlrt/vstrt

The functions keep their names, arguments and results, so vsmlrt.py calls
the ported plugins as it did the originals. Strings a plugin hands back are
marked binary, as API 3 handed every string back, because vsmlrt.py decodes
them as bytes.
"""

import pathlib
import re
import sys


def split_args(text: str) -> list[str]:
    """Splits a call's argument text at its top-level commas."""
    args, depth, current, quote = [], 0, [], None
    for c in text:
        if quote:
            current.append(c)
            if c == quote and (len(current) < 2 or current[-2] != "\\"):
                quote = None
            continue
        if c in "\"'":
            quote = c
        elif c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
        elif c == "," and depth == 0:
            args.append("".join(current))
            current = []
            continue
        current.append(c)
    args.append("".join(current))
    return args


def matching_paren(source: str, open_at: int) -> int:
    """The index of the `)` closing the `(` at `open_at`, skipping string and
    character literals and comments, where parentheses do not count."""
    depth, i = 0, open_at
    while i < len(source):
        c = source[i]
        if c in "\"'":
            i += 1
            while source[i] != c:
                i += 2 if source[i] == "\\" else 1
        elif source.startswith("//", i):
            i = source.index("\n", i)
        elif source.startswith("/*", i):
            i = source.index("*/", i) + 1
        elif c == "(":
            depth += 1
        elif c == ")":
            depth -= 1
            if depth == 0:
                return i
        i += 1
    raise ValueError(f"unbalanced parentheses from {open_at}")


def rewrite_calls(source: str, name: str, rewrite) -> str:
    """Rewrites every `name(...)` call with `rewrite(args) -> text`."""
    out, at = [], 0
    while True:
        start = source.find(name + "(", at)
        if start < 0:
            out.append(source[at:])
            return "".join(out)
        open_at = start + len(name)
        end = matching_paren(source, open_at)
        out.append(source[at:start])
        out.append(rewrite(split_args(source[open_at + 1 : end])))
        at = end + 1


RENAMES = [
    ("#include <VapourSynth.h>", "#include <VapourSynth4.h>"),
    ("#include <VSHelper.h>", "#include <VSHelper4.h>"),
    ("VSNodeRef", "VSNode"),
    ("VSFrameRef", "VSFrame"),
    ("VSFuncRef", "VSFunction"),
    ("vsapi->propNumElements(", "vsapi->mapNumElements("),
    ("vsapi->propNumKeys(", "vsapi->mapNumKeys("),
    ("vsapi->propGetKey(", "vsapi->mapGetKey("),
    ("vsapi->propGetType(", "vsapi->mapGetType("),
    ("vsapi->propGetNode(", "vsapi->mapGetNode("),
    ("vsapi->propGetDataSize(", "vsapi->mapGetDataSize("),
    ("vsapi->propGetData(", "vsapi->mapGetData("),
    ("vsapi->propGetInt(", "vsapi->mapGetInt("),
    ("vsapi->propGetFloat(", "vsapi->mapGetFloat("),
    ("vsapi->propGetFunc(", "vsapi->mapGetFunction("),
    ("vsapi->propSetInt(", "vsapi->mapSetInt("),
    ("vsapi->propSetIntArray(", "vsapi->mapSetIntArray("),
    ("vsapi->propSetFrame(", "vsapi->mapSetFrame("),
    ("vsapi->setError(", "vsapi->mapSetError("),
    ("vsapi->getError(", "vsapi->mapGetError("),
    ("vsapi->freeFunc(", "vsapi->freeFunction("),
    ("vsapi->getCoreInfo2(", "vsapi->getCoreInfo("),
    ("vsapi->getFrameFormat(", "vsapi->getVideoFrameFormat("),
    ("vsapi->getFramePropsRW(", "vsapi->getFramePropertiesRW("),
    ("vsapi->getFramePropsRO(", "vsapi->getFramePropertiesRO("),
    ("paReplace", "maReplace"),
    ("paAppend", "maAppend"),
    ("->format->", "->format."),
    ("isConstantFormat(", "vsh::isConstantVideoFormat("),
    ("vs_bitblt(", "vsh::bitblt("),
    ("static const VSPlugin * myself", "static VSPlugin * myself"),
    ('"clips:clip[];"', '"clips:vnode[];"'),
]


def port(source: str) -> str:
    for old, new in RENAMES:
        source = source.replace(old, new)
    source = re.sub(r"(?<![:\w])int64ToIntS\(", "vsh::int64ToIntS(", source)
    # Strides are ptrdiff_t now; vs-mlrt keeps them in ints, which MSVC
    # refuses to narrow to inside a braced initialiser.
    source = re.sub(
        r"\.pitch = (vsapi->getStride\([^()]*\))",
        r".pitch = static_cast<int>(\1)",
        source,
    )
    # A property's type is an enum now, not a character to print.
    source = source.replace('+ type + ")"', '+ std::to_string(type) + ")"')

    # Formats live inside the video info now, filled in by a query.
    source = re.sub(
        r"(\w+(?:->\w+)*)->format = vsapi->registerFormat\(cm(\w+),",
        r"vsapi->queryVideoFormat(&\1->format, cf\2,",
        source,
    )
    source = re.sub(r"newVideoFrame\(\s*(\w+(?:->\w+)*)->format,", r"newVideoFrame(&\1->format,", source)

    # Strings: binary, as API 3 gave them, for vsmlrt.py's `.decode()`.
    def set_data(args):
        return "vsapi->mapSetData(" + ",".join(args[:-1]) + ", dtBinary," + args[-1] + ")"

    source = rewrite_calls(source, "vsapi->propSetData", set_data)

    def call_function(args):
        return "vsapi->callFunction(" + ",".join(args[:3]) + ")"

    source = rewrite_calls(source, "vsapi->callFunc", call_function)

    def log_message(args):
        if len(args) >= 3:  # ported already
            return "vsapi->logMessage(" + ",".join(args) + ")"
        return "vsapi->logMessage(" + ",".join(args) + ", core)"

    source = rewrite_calls(source, "vsapi->logMessage", log_message)

    # No init callback: the output's video info goes to createVideoFilter.
    source = re.sub(
        r"static void VS_CC \w+Init\(\s*VSMap \*in,.*?\n\}\n\n?",
        "",
        source,
        flags=re.S,
    )
    source = re.sub(
        r"static const VSFrame \*VS_CC (\w+)\(\s*int n,\s*int activationReason,\s*void \*\*instanceData,",
        r"static const VSFrame *VS_CC \1(\n    int n,\n    int activationReason,\n    void *instanceData,",
        source,
    )
    source = re.sub(
        r"static_cast<(\w+) \*>\(\*instanceData\)",
        r"static_cast<\1 *>(instanceData)",
        source,
    )

    def create_filter(args):
        _in, out, name, _init, get_frame, free, mode, _flags, data, core = (a.strip() for a in args)
        return (
            "[&] {\n"
            "        std::vector<VSFilterDependency> deps;\n"
            "        for (const auto & node : d->nodes) {\n"
            "            deps.push_back({node, rpStrictSpatial});\n"
            "        }\n"
            "        auto out_vi = d->out_vi.get();\n"
            f"        vsapi->createVideoFilter({out}, {name}, out_vi, {get_frame}, {free}, {mode},\n"
            f"            deps.data(), static_cast<int>(deps.size()), {data}, {core});\n"
            "    }()"
        )

    source = rewrite_calls(source, "vsapi->createFilter", create_filter)

    # Registration through VSPLUGINAPI.
    source = re.sub(
        r"VS_EXTERNAL_API\(void\) VapourSynthPluginInit\(\s*VSConfigPlugin configFunc,\s*VSRegisterFunction registerFunc,\s*VSPlugin \*plugin\s*\)",
        "VS_EXTERNAL_API(void) VapourSynthPluginInit2(\n    VSPlugin *plugin,\n    const VSPLUGINAPI *vspapi\n)",
        source,
    )

    def config_plugin(args):
        # The identifier and names may sit in preprocessor branches, and the
        # branches end inside the arguments that change, so tokens are
        # replaced in place: the API version gains the plugin's version in
        # front of it, and the read-only flag becomes the flags, 0.
        *names, api, readonly, plugin = args
        api = api.replace(
            "VAPOURSYNTH_API_VERSION", "VS_MAKE_VERSION(1, 0), VAPOURSYNTH_API_VERSION"
        )
        readonly = readonly.replace("1", "0")
        return "vspapi->configPlugin(" + ",".join([*names, api, readonly, plugin]) + ")"

    source = rewrite_calls(source, "configFunc", config_plugin)

    def register(args):
        name = args[0].strip()
        returns = '"clip:vnode;"' if name == '"Model"' else '"any"'
        return "vspapi->registerFunction(" + ",".join(args[:2] + [" " + returns] + args[2:]) + ")"

    source = rewrite_calls(source, "registerFunc", register)
    return source


def main() -> None:
    plugin = pathlib.Path(sys.argv[1])
    for path in sorted(plugin.glob("*.cpp")) + sorted(plugin.glob("*.h")):
        # UTF-8 whatever the platform's own encoding: on Windows Python
        # would read cp1252 and choke on vs-mlrt's sources.
        before = path.read_text(encoding="utf-8")
        after = port(before)
        if after != before:
            path.write_text(after, encoding="utf-8", newline="\n")
            print(f"ported {path}")


if __name__ == "__main__":
    main()
