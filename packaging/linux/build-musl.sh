#!/bin/sh
# Build the native x86_64 musl archive on Alpine 3.24. It uses the system's
# libmpv, fontconfig and desktop libraries, like a distribution package.
set -eu

cd "$(dirname "$0")/../.."
if ! rustc -vV | grep -Eqx 'host: x86_64-[^-]+-linux-musl'; then
    echo 'Build this archive with a native x86_64 musl Rust toolchain.' >&2
    exit 1
fi

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
name="anirust-$version-linux-musl-x86_64"
target=${CARGO_TARGET_DIR:-target}
stage="$target/musl-stage/$name"

# libmpv and the GUI's dynamically loaded desktop libraries need musl's loader.
export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-C target-feature=-crt-static"
cargo build --release --locked -p anirust-gui -p anirust-cli

rm -rf "$stage"
mkdir -p "$stage/bin" dist
for bin in anirust anirust-cli; do
    readelf -l "$target/release/$bin" | grep -F '/lib/ld-musl-x86_64.so.1'
    ldd "$target/release/$bin" >/dev/null
    cp "$target/release/$bin" "$stage/bin/$bin"
    install -m755 packaging/linux/launcher.sh "$stage/$bin"
done
"$stage/anirust-cli" --version
cp packaging/icons/anirust.svg README.md README.ru.md LICENSE "$stage/"

tar -C "$stage/.." -czf "dist/$name.tar.gz" "$name"
(cd dist && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
