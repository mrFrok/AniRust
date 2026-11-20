#!/bin/sh
# Build an archive against the system's libmpv and desktop libraries.
# Use Alpine for musl and Ubuntu for the aarch64 glibc archive.
set -eu

cd "$(dirname "$0")/../.."
arch=$(uname -m)
case "$arch:${1:?libc: musl or glibc}" in
    x86_64:musl | aarch64:musl)
        platform=linux-musl
        rust_libc=musl
        interpreter="/lib/ld-musl-$arch.so.1"
        # libmpv and the desktop libraries need the dynamic loader.
        export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-C target-feature=-crt-static"
        ;;
    aarch64:glibc)
        platform=linux
        rust_libc=gnu
        interpreter=/lib/ld-linux-aarch64.so.1
        ;;
    *) echo "Unsupported architecture and libc: $arch $1" >&2; exit 1 ;;
esac
if ! rustc -vV | grep -Eqx "host: $arch-[^-]+-linux-$rust_libc"; then
    echo "Build this archive with a native $arch Linux toolchain for $1." >&2
    exit 1
fi

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
name="anirust-$version-$platform-$arch"
target=${CARGO_TARGET_DIR:-target}
stage="$target/native-stage/$name"

cargo build --release --locked -p anirust-gui -p anirust-cli

rm -rf "$stage"
mkdir -p "$stage/bin" dist
for bin in anirust anirust-cli; do
    readelf -l "$target/release/$bin" | grep -F "Requesting program interpreter: $interpreter]"
    ldd "$target/release/$bin" >/dev/null
    cp "$target/release/$bin" "$stage/bin/$bin"
    install -m755 packaging/linux/launcher.sh "$stage/$bin"
done
"$stage/anirust-cli" --version
cp packaging/icons/anirust.svg README.md README.ru.md LICENSE "$stage/"

tar -C "$stage/.." -czf "dist/$name.tar.gz" "$name"
(cd dist && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
