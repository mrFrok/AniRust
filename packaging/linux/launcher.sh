#!/bin/sh
# Starts AniRust from the Linux archive.
#
# The archive's libmpv, in lib/, has the VapourSynth filter frame generation
# needs, but it was built against one distribution's ffmpeg and libplacebo.
# It is used when the system has what it needs — `ldd` finds every library —
# and the system's own libmpv otherwise, so the program always starts and
# only frame generation depends on the match.
#
# Installed as both `anirust` and `anirust-cli`; runs bin/ of the same name.

here=$(dirname "$(readlink -f "$0")")
name=$(basename "$0")

if [ -f "$here/lib/libmpv.so.2" ] &&
    ! LD_LIBRARY_PATH="$here/lib" ldd "$here/lib/libmpv.so.2" 2>/dev/null | grep -q "not found"; then
    LD_LIBRARY_PATH="$here/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    export LD_LIBRARY_PATH
fi

# The desktop entry the program writes for itself should start it through
# here, not straight from bin/.
ANIRUST_LAUNCHER="$here/anirust"
export ANIRUST_LAUNCHER

exec "$here/bin/$name" "$@"
