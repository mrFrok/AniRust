#!/bin/sh
set -eu

appdir=${APPDIR:-$(dirname "$(readlink -f "$0")")}
fallback="$appdir/usr/lib/anirust/vulkan-loader"

# Prefer the loader installed alongside the graphics driver. The copy in the
# image is only for hosts that have no Vulkan loader, so libmpv can still start.
if [ -f "$fallback/libvulkan.so.1" ] &&
    ! PATH="$PATH:/sbin:/usr/sbin" ldconfig -p 2>/dev/null |
        grep -qE 'libvulkan\.so\.1 \([^)]*x86-64[^)]*\)'; then
    export LD_LIBRARY_PATH="${LD_LIBRARY_PATH:+$LD_LIBRARY_PATH:}$fallback"
fi

exec "$appdir/usr/bin/anirust" "$@"
