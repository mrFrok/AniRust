#!/usr/bin/env bash
# Prepare the package files from a published AniRust release.
set -euo pipefail

version=${1:?usage: prepare-release-packages.sh VERSION OUTPUT_DIRECTORY}
version=${version#v}
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo 'Expected a stable version such as 1.1.1' >&2; exit 1; }
out=$(realpath -m "${2:?output directory required}")
root=$(cd "$(dirname "$0")/.." && pwd)
repo=mrFrok/AniRust
tag="v$version"
base="https://github.com/$repo/releases/download/$tag"
raw="https://raw.githubusercontent.com/$repo/$tag"
app_id=io.github.mrfrok.AniRust

release=$(gh release view "$tag" -R "$repo" --json tagName,isDraft,isPrerelease)
jq -e --arg tag "$tag" '.tagName == $tag and (.isDraft or .isPrerelease | not)' <<< "$release" >/dev/null

fetch() { curl --proto '=https' -fsSL --retry 3 "$1"; }

asset_sha() {
  local name=$1 sum file sidecar
  sidecar=$(fetch "$base/$name.sha256")
  read -r sum file <<< "$sidecar"
  if [[ ! $sum =~ ^[[:xdigit:]]{64}$ || ${file#\*} != "$name" ]]; then
    echo "Invalid checksum for $name" >&2
    return 1
  fi
  printf '%s\n' "$sum"
}

source_package="$out/aur/anirust"
binary_package="$out/aur/anirust-bin"
tap="$out/homebrew"
winget="$out/winget/manifests/m/mrFrok/AniRust/$version"
mkdir -p "$source_package" "$binary_package" "$tap/Casks" "$tap/Formula" "$winget"

scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
export SRCDEST="$scratch/sources" BUILDDIR="$scratch/build"
mkdir -p "$SRCDEST" "$BUILDDIR"

# makepkg downloads all architecture-specific sources and computes their hashes.
awk -v version="$version" '
  /^pkgver=/ { print "pkgver=" version; next }
  /^sha256sums(_[a-zA-Z0-9_]+)?=/ { next }
  { print }
' "$root/packaging/arch/PKGBUILD" > "$source_package/PKGBUILD"
(
  cd "$source_package"
  makepkg --geninteg >> "$scratch/aur-checksums"
  cat "$scratch/aur-checksums" >> PKGBUILD
  makepkg --printsrcinfo > .SRCINFO
)
source_sha=$(sha256sum "$SRCDEST/anirust-$version.tar.gz")
source_sha=${source_sha%% *}

desktop_sha=$(fetch "$raw/packaging/linux/$app_id.desktop" | sha256sum)
metainfo_sha=$(fetch "$raw/packaging/linux/$app_id.metainfo.xml" | sha256sum)
archive_x64_sha=$(asset_sha "anirust-$version-linux-x86_64.tar.gz")
archive_arm_sha=$(asset_sha "anirust-$version-linux-aarch64.tar.gz")
sed -e "s/@VERSION@/$version/g" \
    -e "s/@DESKTOP_SHA@/${desktop_sha%% *}/g" -e "s/@METAINFO_SHA@/${metainfo_sha%% *}/g" \
    -e "s/@ARCHIVE_X64_SHA@/$archive_x64_sha/g" -e "s/@ARCHIVE_ARM_SHA@/$archive_arm_sha/g" \
    "$root/packaging/arch/PKGBUILD-bin.in" > "$binary_package/PKGBUILD"
(cd "$binary_package" && makepkg --printsrcinfo > .SRCINFO)

dmg_sha=$(asset_sha "anirust-$version-macos-arm64.dmg")
cat > "$tap/Casks/anirust.rb" <<EOF
cask "anirust" do
  version "$version"
  sha256 "$dmg_sha"

  url "https://github.com/$repo/releases/download/v#{version}/anirust-#{version}-macos-arm64.dmg"
  name "AniRust"
  desc "Desktop client for Anixart with an mpv player"
  homepage "https://github.com/$repo"

  depends_on arch: :arm64
  depends_on macos: ">= :sequoia"

  app "AniRust.app"
end
EOF
cat > "$tap/Formula/anirust.rb" <<EOF
class Anirust < Formula
  desc "Desktop client for Anixart with an mpv player"
  homepage "https://github.com/$repo"
  url "https://github.com/$repo/archive/refs/tags/$tag.tar.gz"
  sha256 "$source_sha"
  license "GPL-3.0-or-later"

  depends_on :linux
  depends_on "cmake" => :build
  depends_on "pkgconf" => :build
  depends_on "rust" => :build
  depends_on "fontconfig"
  depends_on "libxkbcommon"
  depends_on "mpv"

  def install
    system "cargo", "build", "--release", "--locked", "-p", "anirust-gui", "-p", "anirust-cli"
    bin.install "target/release/anirust", "target/release/anirust-cli"
    (share/"applications").install "packaging/linux/$app_id.desktop"
    (share/"icons/hicolor/scalable/apps").install "packaging/icons/anirust.svg" => "$app_id.svg"
  end

  test do
    assert_match "AniRust #{version}", shell_output("#{bin}/anirust --version")
  end
end
EOF
ruby -c "$tap/Casks/anirust.rb"
ruby -c "$tap/Formula/anirust.rb"

winget_id=mrFrok.AniRust
windows_installer="AniRust-$version-setup.exe"
windows_sha=$(asset_sha "$windows_installer")
cat > "$winget/$winget_id.yaml" <<EOF
# yaml-language-server: \$schema=https://aka.ms/winget-manifest.version.1.12.0.schema.json
PackageIdentifier: $winget_id
PackageVersion: $version
DefaultLocale: en-US
ManifestType: version
ManifestVersion: 1.12.0
EOF
cat > "$winget/$winget_id.installer.yaml" <<EOF
# yaml-language-server: \$schema=https://aka.ms/winget-manifest.installer.1.12.0.schema.json
PackageIdentifier: $winget_id
PackageVersion: $version
InstallerType: inno
Scope: user
InstallModes:
- interactive
- silent
- silentWithProgress
AppsAndFeaturesEntries:
- DisplayName: AniRust
  Publisher: mrfrok
  ProductCode: '{59C3AE0C-ECA3-4C2B-912D-F985FB8B60A6}_is1'
Installers:
- Architecture: x64
  InstallerUrl: $base/$windows_installer
  InstallerSha256: $windows_sha
ManifestType: installer
ManifestVersion: 1.12.0
EOF
cat > "$winget/$winget_id.locale.en-US.yaml" <<EOF
# yaml-language-server: \$schema=https://aka.ms/winget-manifest.defaultLocale.1.12.0.schema.json
PackageIdentifier: $winget_id
PackageVersion: $version
PackageLocale: en-US
Publisher: mrfrok
PublisherUrl: https://github.com/$repo
PublisherSupportUrl: https://github.com/$repo/issues
PackageName: AniRust
PackageUrl: https://github.com/$repo
License: GPL-3.0-or-later
LicenseUrl: $raw/LICENSE
ShortDescription: Desktop client for Anixart with an mpv player
Moniker: anirust
ReleaseNotesUrl: https://github.com/$repo/releases/tag/$tag
ManifestType: defaultLocale
ManifestVersion: 1.12.0
EOF

schema_base=https://raw.githubusercontent.com/microsoft/winget-cli/master/schemas/JSON/manifests/v1.12.0
for schema in version installer defaultLocale; do
  fetch "$schema_base/manifest.$schema.1.12.0.json" > "$scratch/$schema.json"
done
check-jsonschema --schemafile "$scratch/version.json" "$winget/$winget_id.yaml"
check-jsonschema --schemafile "$scratch/installer.json" "$winget/$winget_id.installer.yaml"
check-jsonschema --schemafile "$scratch/defaultLocale.json" "$winget/$winget_id.locale.en-US.yaml"
