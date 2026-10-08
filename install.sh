#!/usr/bin/env bash
# AniRust installer for Linux and macOS: picks the package that suits the
# system, installs it for the current user, offers the system packages the
# player wants for the GPU it finds, and updates or removes it later.
#
#   curl -fsSL https://raw.githubusercontent.com/mrFrok/AniRust/main/install.sh | bash
#   ... | bash -s -- --update
#   ... | bash -s -- --uninstall [--purge]
#
# Options:
#   --update            install the latest release over the installed one
#   --uninstall         remove the program; --purge removes settings and
#                       history too
#   --version=X.Y.Z     a given release rather than the latest
#   --package=KIND      tarball, appimage or flatpak, instead of the choice
#                       made for this system
#   --no-deps           leave system packages alone
#   -y, --yes           answer yes to every question
#
# Which package (the same choice as the README's):
#   Arch and its kin                       the tarball, built on Arch
#   glibc 2.39 or newer                    the AppImage
#   older glibc, musl, anything else       the Flatpak
#   macOS on Apple silicon                 the disk image, into ~/Applications
#   NixOS                                  the flake (nix profile install)
#
# Everything goes into the user's home: ~/.local/opt/anirust, a link in
# ~/.local/bin, the desktop entry and icon in ~/.local/share. System packages
# — GPU drivers, VapourSynth — go through the package manager with sudo, and
# only after asking.

set -euo pipefail

REPO="mrFrok/AniRust"
APP_ID="io.github.mrfrok.AniRust"
PREFIX="${ANIRUST_PREFIX:-$HOME/.local/opt/anirust}"
BIN_DIR="$HOME/.local/bin"
DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
MARKER="$PREFIX/.install"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

info() { echo -e "${CYAN}$*${NC}"; }
ok()   { echo -e "${GREEN}✓${NC} $*"; }
warn() { echo -e "${YELLOW}⚠${NC} $*"; }
err()  { echo -e "${RED}✗${NC} $*" >&2; exit 1; }

ACTION=install
PURGE=false
VERSION=""
PACKAGE=""
DEPS=true
YES=false

for arg in "$@"; do
    case "$arg" in
        --update) ACTION=update ;;
        --uninstall) ACTION=uninstall ;;
        --purge) PURGE=true ;;
        --version=*) VERSION="${arg#*=}"; VERSION="${VERSION#v}" ;;
        --package=*) PACKAGE="${arg#*=}" ;;
        --no-deps) DEPS=false ;;
        -y | --yes) YES=true ;;
        -h | --help) sed -n '2,32p' "$0" 2>/dev/null | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) warn "Unknown option: $arg" ;;
    esac
done

# ---------------------------------------------------------------------------
# Asking. Piped into bash, the script's stdin is the script itself, so the
# answers come from the terminal; with no terminal, nothing is assumed.
# ---------------------------------------------------------------------------

ask() { # question -> 0 for yes
    $YES && return 0
    # A terminal that exists but cannot be opened (no controlling one) is no
    # terminal.
    { : < /dev/tty; } 2>/dev/null || return 1
    local answer
    printf '%b [Y/n] ' "$1" > /dev/tty
    read -r answer < /dev/tty || return 1
    case "$answer" in
        "" | [YyДд]*) return 0 ;;
        *) return 1 ;;
    esac
}

# ---------------------------------------------------------------------------
# Downloading and checking
# ---------------------------------------------------------------------------

fetch() { # url file
    if command -v curl &>/dev/null; then
        curl -fL --retry 3 --progress-bar "$1" -o "$2"
    elif command -v wget &>/dev/null; then
        wget -q --show-progress -O "$2" "$1"
    else
        err "curl or wget is needed"
    fi
}

sha256_of() {
    if command -v sha256sum &>/dev/null; then
        sha256sum "$1" | cut -d' ' -f1
    elif command -v shasum &>/dev/null; then
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

# Against the .sha256 the release publishes beside every file, and, when
# minisign is installed, against the release key's signature as well.
verify_asset() { # file
    local file="$1" expected actual
    fetch "$(asset_url "$(basename "$file").sha256")" "$file.sha256" 2>/dev/null ||
        err "No checksum published for $(basename "$file")"
    expected="$(cut -d' ' -f1 < "$file.sha256" | tr 'A-F' 'a-f')"
    case "$expected" in *[!0-9a-f]* | "") err "The checksum of $(basename "$file") is malformed" ;; esac
    [ "${#expected}" -eq 64 ] || err "The checksum of $(basename "$file") is malformed"
    actual="$(sha256_of "$file")"
    [ -n "$actual" ] || err "sha256sum or shasum is needed to check the download"
    [ "$actual" = "$expected" ] || err "$(basename "$file") does not match its checksum — not installing it"
    ok "Checksum verified"
    if command -v minisign &>/dev/null &&
        fetch "$(asset_url "$(basename "$file").minisig")" "$file.minisig" 2>/dev/null; then
        printf '%s\n' "untrusted comment: AniRust release key" \
            "RWTsVgZgS0HPjqK3vDoEBG+K19n1nsoOVdObXpaP1T3KMXa0yG/lm9d5" > "$file.pub"
        minisign -V -q -p "$file.pub" -m "$file" || err "$(basename "$file") is not signed by the AniRust release key"
        ok "Signature verified"
    fi
}

latest_version() {
    # The release page redirects to the newest tag: no API call, so no API
    # rate limit.
    local url
    url="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest" 2>/dev/null ||
        wget -q --max-redirect=5 -S --spider "https://github.com/$REPO/releases/latest" 2>&1 |
        sed -n 's/^ *Location: //p' | tail -n 1)"
    url="${url%$'\r'}"
    case "$url" in
        */tag/v*) echo "${url##*/tag/v}" ;;
        *) return 1 ;;
    esac
}

asset_url() { echo "https://github.com/$REPO/releases/download/v$VERSION/$1"; }
raw_url() { echo "https://raw.githubusercontent.com/$REPO/v$VERSION/$1"; }

# ---------------------------------------------------------------------------
# The system
# ---------------------------------------------------------------------------

OS=""
ARCH=""
OS_ID=""
OS_LIKE=""

detect_system() {
    case "$(uname -s)" in
        Linux) OS=linux ;;
        Darwin) OS=macos ;;
        *) err "AniRust runs on Linux, Windows and macOS; this is $(uname -s)" ;;
    esac
    case "$(uname -m)" in
        x86_64 | amd64) ARCH=x86_64 ;;
        aarch64 | arm64) ARCH=arm64 ;;
        *) ARCH="$(uname -m)" ;;
    esac
    if [ -r /etc/os-release ]; then
        # shellcheck disable=SC1091 # the system's, read where it is
        OS_ID="$(. /etc/os-release && echo "${ID:-}")"
        # shellcheck disable=SC1091
        OS_LIKE="$(. /etc/os-release && echo "${ID_LIKE:-}")"
    fi
}

is_musl() {
    [ -e "/lib/ld-musl-$(uname -m).so.1" ] || ldd --version 2>&1 | grep -qi musl
}

# glibc's version as a number: 2.39 -> 239.
glibc_version() {
    local v
    v="$(getconf GNU_LIBC_VERSION 2>/dev/null | awk '{print $2}')"
    [ -n "$v" ] || v="$(ldd --version 2>/dev/null | head -n 1 | grep -oE '[0-9]+\.[0-9]+$')"
    [ -n "$v" ] || { echo 0; return; }
    echo "$(( ${v%%.*} * 100 + 10#${v#*.} ))"
}

# Arch and the distributions built on its packages — Artix names no ID_LIKE,
# so pacman itself counts too.
is_arch_like() {
    case " $OS_ID $OS_LIKE " in *" arch "* | *" archlinux "* | *" artix "*) return 0 ;; esac
    command -v pacman &>/dev/null
}

# Immutable systems, where packages are not installed the usual way.
is_atomic() {
    case " $OS_ID $OS_LIKE " in
        *" silverblue "* | *" kinoite "* | *" sericea "* | *" onyx "* | *" bazzite "* | \
        *" aurora "* | *" bluefin "* | *" vanillaos "* | *" carbonos "* | *" steamos "*) return 0 ;;
    esac
    [ -d /ostree ] || command -v rpm-ostree &>/dev/null
}

is_nixos() { [ -e /etc/NIXOS ] || [ "$OS_ID" = nixos ]; }

package_manager() {
    local pm
    for pm in pacman apt-get dnf zypper xbps-install apk emerge; do
        if command -v "$pm" &>/dev/null; then
            case "$pm" in
                apt-get) echo apt ;;
                xbps-install) echo xbps ;;
                *) echo "$pm" ;;
            esac
            return
        fi
    done
}

# The PCI vendors of the display devices: 10de NVIDIA, 1002 AMD, 8086 Intel.
gpu_vendors() {
    cat /sys/class/drm/*/device/vendor 2>/dev/null | sort -u | tr '\n' ' '
}

# ---------------------------------------------------------------------------
# What the player wants from the system, by package manager
# ---------------------------------------------------------------------------

# Prints the packages for one need on one package manager, or nothing.
package_for() { # pm need
    case "$1:$2" in
        pacman:vapoursynth | apt:vapoursynth | dnf:vapoursynth | zypper:vapoursynth | xbps:vapoursynth) echo vapoursynth ;;
        pacman:intel-va | dnf:intel-va | zypper:intel-va | xbps:intel-va | apk:intel-va) echo intel-media-driver ;;
        apt:intel-va) echo intel-media-va-driver ;;
        pacman:intel-cl | dnf:intel-cl | xbps:intel-cl) echo intel-compute-runtime ;;
        apt:intel-cl) echo intel-opencl-icd ;;
        zypper:intel-cl) echo intel-opencl ;;
        pacman:amd-va) echo mesa ;;
        apt:amd-va | dnf:amd-va) echo mesa-va-drivers ;;
        zypper:amd-va) echo Mesa-dri ;;
        xbps:amd-va) echo mesa-vaapi ;;
        apk:amd-va) echo mesa-va-gallium ;;
        apt:fuse) if apt-cache show libfuse2t64 &>/dev/null; then echo libfuse2t64; else echo libfuse2; fi ;;
        dnf:fuse | zypper:fuse) echo fuse-libs ;;
        pacman:fuse) echo fuse2 ;;
    esac
}

install_command() { # pm
    case "$1" in
        pacman) echo "pacman -S --needed --noconfirm" ;;
        apt) echo "apt-get install -y" ;;
        dnf) echo "dnf install -y" ;;
        zypper) echo "zypper --non-interactive install" ;;
        xbps) echo "xbps-install -Sy" ;;
        apk) echo "apk add" ;;
        emerge) echo "emerge --noreplace" ;;
    esac
}

# Is a package already installed?
installed() { # pm package
    case "$1" in
        pacman) pacman -Q "$2" &>/dev/null ;;
        apt) dpkg-query -W -f='${Status}' "$2" 2>/dev/null | grep -q "install ok installed" ;;
        dnf | zypper) rpm -q "$2" &>/dev/null ;;
        xbps) xbps-query "$2" &>/dev/null ;;
        apk) apk info -e "$2" &>/dev/null ;;
        *) return 1 ;;
    esac
}

# Offers the system packages the chosen package and the GPUs want.
install_dependencies() { # package
    $DEPS || return 0
    [ "$OS" = linux ] || return 0
    local kind="$1" pm vendors needs=() wanted=() need pkg
    vendors="$(gpu_vendors)"
    case "$kind" in tarball | appimage) needs+=(vapoursynth) ;; esac
    [ "$kind" = appimage ] && needs+=(fuse)
    # The Flatpak's runtime carries its own drivers and VapourSynth.
    if [ "$kind" != flatpak ]; then
        case "$vendors" in *0x8086*) needs+=(intel-va intel-cl) ;; esac
        case "$vendors" in *0x1002*) needs+=(amd-va) ;; esac
    fi
    [ ${#needs[@]} -gt 0 ] || return 0

    if is_atomic; then
        warn "This system installs packages by layering. For hardware decoding and the"
        warn "neural networks, install the GPU's VA-API and OpenCL drivers with rpm-ostree"
        warn "(intel-media-driver and intel-compute-runtime for Intel, mesa-va-drivers for AMD)."
        return 0
    fi
    pm="$(package_manager)"
    if [ -z "$pm" ]; then
        warn "No known package manager; install VapourSynth and the GPU's VA-API driver yourself."
        return 0
    fi
    for need in "${needs[@]}"; do
        pkg="$(package_for "$pm" "$need")"
        [ -n "$pkg" ] || continue
        installed "$pm" "$pkg" || wanted+=("$pkg")
    done
    [ ${#wanted[@]} -gt 0 ] || { ok "System packages are in place"; return 0; }

    echo
    info "The player would like these system packages:"
    for pkg in "${wanted[@]}"; do echo "    $pkg"; done
    echo "  (VapourSynth for frame generation; VA-API for hardware decoding; Intel's"
    echo "   OpenCL for the neural networks on an Intel GPU)"
    local command
    command="$(install_command "$pm") ${wanted[*]}"
    if ask "Install them with ${BOLD}sudo $command${NC}?"; then
        local sudo=""
        [ "$(id -u)" -eq 0 ] || sudo=sudo
        [ "$pm" = apt ] && { $sudo apt-get update -qq || true; }
        # shellcheck disable=SC2086 # the command is words on purpose
        if $sudo $command; then
            ok "System packages installed"
        else
            warn "Some packages did not install; the player works without them, with less"
        fi
    else
        warn "Skipped. Later: sudo $command"
    fi
}

# ---------------------------------------------------------------------------
# Installing each kind of package
# ---------------------------------------------------------------------------

choose_package() {
    [ -n "$PACKAGE" ] && { echo "$PACKAGE"; return; }
    if [ "$OS" = macos ]; then echo dmg; return; fi
    if is_nixos; then echo nix; return; fi
    if ! is_atomic && is_arch_like && ! is_musl; then echo tarball; return; fi
    if ! is_musl && [ "$(glibc_version)" -ge 239 ]; then echo appimage; return; fi
    echo flatpak
}

# The desktop entry and icon, the same files the program writes for itself on
# first start (gui/src/desktop.rs), so the two never fight over them.
install_desktop_entry() { # exe
    local apps="$DATA_HOME/applications" icons="$DATA_HOME/icons/hicolor/scalable/apps" tmp
    mkdir -p "$apps" "$icons"
    tmp="$(mktemp)"
    if fetch "$(raw_url "packaging/linux/$APP_ID.desktop")" "$tmp" 2>/dev/null; then
        sed "s|^Exec=anirust %u|Exec=\"$1\" %u|" "$tmp" > "$apps/$APP_ID.desktop"
    else
        warn "The desktop entry could not be fetched; the program writes it on first start"
    fi
    fetch "$(raw_url packaging/icons/anirust.svg)" "$icons/$APP_ID.svg" 2>/dev/null || true
    rm -f "$tmp"
    if command -v update-desktop-database &>/dev/null; then
        update-desktop-database "$apps" 2>/dev/null || true
    fi
    if command -v gtk-update-icon-cache &>/dev/null; then
        gtk-update-icon-cache -f -t "$DATA_HOME/icons/hicolor" 2>/dev/null || true
    fi
    ok "Added to the applications menu"
}

link_into_bin() { # target name
    mkdir -p "$BIN_DIR"
    ln -sfn "$1" "$BIN_DIR/$2"
}

write_marker() { # kind
    mkdir -p "$PREFIX"
    printf 'package=%s\nversion=%s\n' "$1" "$VERSION" > "$MARKER"
}

install_tarball() {
    [ "$ARCH" = x86_64 ] || err "The Linux packages are built for x86_64 only"
    local name="anirust-$VERSION-linux-x86_64" tmp
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"; trap - RETURN' RETURN
    info "Downloading $name.tar.gz…"
    fetch "$(asset_url "$name.tar.gz")" "$tmp/$name.tar.gz" || err "Could not download $name.tar.gz"
    verify_asset "$tmp/$name.tar.gz"
    tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
    mkdir -p "$PREFIX"
    # Swapped in whole, so a failed copy never leaves half of each version.
    rm -rf "$PREFIX/app.new" "$PREFIX/app.old"
    mv "$tmp/$name" "$PREFIX/app.new"
    [ -d "$PREFIX/app" ] && mv "$PREFIX/app" "$PREFIX/app.old"
    mv "$PREFIX/app.new" "$PREFIX/app"
    rm -rf "$PREFIX/app.old"
    link_into_bin "$PREFIX/app/anirust" anirust
    link_into_bin "$PREFIX/app/anirust-cli" anirust-cli
    install_desktop_entry "$PREFIX/app/anirust"
    write_marker tarball
}

install_appimage() {
    [ "$ARCH" = x86_64 ] || err "The Linux packages are built for x86_64 only"
    local name="AniRust-$VERSION-x86_64.AppImage" tmp
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"; trap - RETURN' RETURN
    info "Downloading $name…"
    fetch "$(asset_url "$name")" "$tmp/$name" || err "Could not download $name"
    verify_asset "$tmp/$name"
    chmod 755 "$tmp/$name"
    mkdir -p "$PREFIX"
    mv -f "$tmp/$name" "$PREFIX/AniRust.AppImage"
    link_into_bin "$PREFIX/AniRust.AppImage" anirust
    install_desktop_entry "$PREFIX/AniRust.AppImage"
    write_marker appimage
}

install_flatpak() {
    [ "$ARCH" = x86_64 ] || err "The Flatpak is built for x86_64 only"
    if ! command -v flatpak &>/dev/null; then
        local pm
        pm="$(package_manager)"
        if [ -z "$pm" ] || is_atomic; then
            err "Flatpak is needed; install it, then run this again"
        fi
        ask "Flatpak is not installed. Install it with ${BOLD}sudo $(install_command "$pm") flatpak${NC}?" ||
            err "Flatpak is needed for this system"
        # shellcheck disable=SC2046
        sudo $(install_command "$pm") flatpak
    fi
    flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
    local name="anirust-$VERSION-x86_64.flatpak" tmp
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"; trap - RETURN' RETURN
    info "Downloading $name…"
    fetch "$(asset_url "$name")" "$tmp/$name" || err "Could not download $name"
    verify_asset "$tmp/$name"
    flatpak install --user -y --reinstall "$tmp/$name"
    # A command to type, as with the other packages; the menu entry is
    # Flatpak's own.
    mkdir -p "$BIN_DIR" "$PREFIX"
    printf '#!/bin/sh\nexec flatpak run %s "$@"\n' "$APP_ID" > "$PREFIX/anirust"
    chmod 755 "$PREFIX/anirust"
    link_into_bin "$PREFIX/anirust" anirust
    write_marker flatpak
}

install_dmg() {
    [ "$ARCH" = arm64 ] || err "The macOS build is for Apple silicon; on an Intel Mac, build from source (README)"
    local name="anirust-$VERSION-macos-arm64.dmg" tmp mount apps="$HOME/Applications"
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"; trap - RETURN' RETURN
    info "Downloading $name…"
    fetch "$(asset_url "$name")" "$tmp/$name" || err "Could not download $name"
    verify_asset "$tmp/$name"
    mount="$tmp/mount"
    mkdir -p "$mount" "$apps"
    hdiutil attach -nobrowse -readonly -mountpoint "$mount" "$tmp/$name" >/dev/null
    rm -rf "$apps/AniRust.app.new"
    ditto "$mount/AniRust.app" "$apps/AniRust.app.new"
    hdiutil detach "$mount" -quiet || true
    rm -rf "$apps/AniRust.app"
    mv "$apps/AniRust.app.new" "$apps/AniRust.app"
    # Downloaded by a browser it would be quarantined; by curl it is not, but
    # a copy from an earlier manual install may be.
    xattr -dr com.apple.quarantine "$apps/AniRust.app" 2>/dev/null || true
    write_marker dmg
    ok "Installed to $apps/AniRust.app"
    if command -v brew &>/dev/null && ! brew list vapoursynth &>/dev/null && $DEPS &&
        ask "Frame generation needs VapourSynth. Install it with ${BOLD}brew install vapoursynth${NC}?"; then
        brew install vapoursynth || warn "VapourSynth did not install; frame generation stays off"
    fi
}

suggest_path() {
    case ":$PATH:" in *":$BIN_DIR:"*) return ;; esac
    echo
    warn "$BIN_DIR is not in PATH, so \`anirust\` will not be found by name."
    echo "  Add this to your shell's profile: export PATH=\"$BIN_DIR:\$PATH\""
}

# ---------------------------------------------------------------------------
# The three actions
# ---------------------------------------------------------------------------

installed_package() { [ -f "$MARKER" ] && sed -n 's/^package=//p' "$MARKER"; }
installed_version() { [ -f "$MARKER" ] && sed -n 's/^version=//p' "$MARKER"; }

do_install() {
    local kind="$1"
    if [ -z "$VERSION" ]; then
        info "Looking up the latest release…"
        VERSION="$(latest_version)" || err "Could not find the latest release"
    fi
    ok "AniRust $VERSION, as $kind"
    case "$kind" in
        tarball) install_tarball ;;
        appimage) install_appimage ;;
        flatpak) install_flatpak ;;
        dmg) install_dmg ;;
        nix)
            echo
            info "On NixOS, AniRust comes from its flake:"
            echo "  nix profile install github:$REPO"
            echo "  (or try it: nix run github:$REPO)"
            exit 0
            ;;
        *) err "Unknown package: $kind (tarball, appimage or flatpak)" ;;
    esac
    install_dependencies "$kind"
    [ "$kind" = dmg ] || suggest_path
    echo
    ok "AniRust $VERSION is installed. Start it from the applications menu$([ "$kind" = dmg ] || echo " or with \`anirust\`")."
}

do_update() {
    local kind current
    kind="$(installed_package || true)"
    [ -n "$kind" ] || err "AniRust was not installed by this script; install it first"
    current="$(installed_version || true)"
    if [ -z "$VERSION" ]; then
        VERSION="$(latest_version)" || err "Could not find the latest release"
    fi
    if [ "$current" = "$VERSION" ]; then
        ok "AniRust $current is the latest"
        return
    fi
    info "Updating AniRust ${current:-?} → $VERSION"
    DEPS=false
    do_install "$kind"
}

do_uninstall() {
    local kind
    kind="$(installed_package || true)"
    if [ "$kind" = flatpak ]; then
        flatpak uninstall --user -y "$APP_ID" || true
    fi
    if [ "$kind" = dmg ]; then
        rm -rf "$HOME/Applications/AniRust.app"
    fi
    for name in anirust anirust-cli; do
        # Only links into our folder: an anirust from a package stays.
        case "$(readlink "$BIN_DIR/$name" 2>/dev/null)" in
            "$PREFIX"/*) rm -f "$BIN_DIR/$name" ;;
        esac
    done
    rm -rf "$PREFIX"
    if [ "$OS" = linux ] && grep -qs "$PREFIX" "$DATA_HOME/applications/$APP_ID.desktop"; then
        rm -f "$DATA_HOME/applications/$APP_ID.desktop" "$DATA_HOME/icons/hicolor/scalable/apps/$APP_ID.svg"
    fi
    ok "AniRust removed"
    if $PURGE; then
        if [ "$OS" = macos ]; then
            rm -rf "$HOME/Library/Application Support/anirust" "$HOME/Library/Caches/anirust"
        else
            rm -rf "${XDG_CONFIG_HOME:-$HOME/.config}/anirust" "$DATA_HOME/anirust" "${XDG_CACHE_HOME:-$HOME/.cache}/anirust"
        fi
        ok "Settings, history and cache removed"
        echo "  The session token stays in the system's keyring (service dev.anirust.client)."
    else
        echo "  Settings and history stay; --uninstall --purge removes them too."
    fi
}

main() {
    echo
    echo -e "${BOLD}AniRust${NC} — a desktop client for Anixart"
    echo
    detect_system
    case "$ACTION" in
        install) do_install "$(choose_package)" ;;
        update) do_update ;;
        uninstall) do_uninstall ;;
    esac
}

main
