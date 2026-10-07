#!/bin/sh
# Wraps the release build in AniRust.app with libmpv and everything it
# links copied inside, and puts it in a disk image under dist/.
#
#   packaging/macos/bundle.sh 1.0.0
#
# Needs `dylibbundler` (Homebrew) and the tools macOS ships: sips, iconutil,
# hdiutil, rsvg-convert from librsvg for the icon.
set -eu

version="$1"
root="$(cd "$(dirname "$0")/../.." && pwd)"
app="$root/dist/AniRust.app"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$app/Contents/Frameworks"
cp "$root/target/release/anirust" "$app/Contents/MacOS/"

# The icon: the SVG drawn at the sizes an .icns holds.
iconset="$root/dist/AniRust.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
  rsvg-convert -w "$size" -h "$size" "$root/packaging/icons/anirust.svg" -o "$iconset/icon_${size}x${size}.png"
  rsvg-convert -w "$((size * 2))" -h "$((size * 2))" "$root/packaging/icons/anirust.svg" -o "$iconset/icon_${size}x${size}@2x.png"
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/AniRust.icns"
rm -rf "$iconset"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>AniRust</string>
  <key>CFBundleDisplayName</key><string>AniRust</string>
  <key>CFBundleIdentifier</key><string>io.github.mrfrok.AniRust</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleExecutable</key><string>anirust</string>
  <key>CFBundleIconFile</key><string>AniRust</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Frame generation's RIFE plugin and models, when the caller fetched them
# (packaging/fetch-rife.sh macos-arm64 DIR; RIFE_DIR=DIR). In Resources, where
# the player looks for them (../Resources/rife from the executable): folders
# beside the executable read as nested bundles to codesign, and the models'
# folders are not bundles. The plugin is code, so it is signed itself — on
# Apple silicon an unsigned library does not load.
if [ -n "${RIFE_DIR:-}" ]; then
  cp -R "$RIFE_DIR" "$app/Contents/Resources/rife"
  codesign --force --sign - "$app/Contents/Resources/rife/librife.dylib"
fi

# libmpv and its dependencies, rewritten to load from inside the bundle.
dylibbundler -od -b -x "$app/Contents/MacOS/anirust" \
  -d "$app/Contents/Frameworks" -p @executable_path/../Frameworks/

hdiutil create -volname AniRust -srcfolder "$app" -ov -format UDZO \
  "$root/dist/anirust-$version-macos-arm64.dmg"
