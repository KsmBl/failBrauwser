#!/usr/bin/env bash
# Builds and installs failBrauwser.
#
#   ./install.sh                 install for the current user into ~/.local
#   ./install.sh --system        install for everyone into /usr/local (asks for sudo)
#   ./install.sh --prefix DIR    install into DIR
#   ./install.sh --no-autostart  do not start the background instance at login
#   ./install.sh --default       make failBrauwser the default file manager
#   ./install.sh --no-network    do not install the GVfs SMB backend (smb:// addresses)
#   ./install.sh --no-archives   do not build archive support (needs no .NET SDK then)
#   ./install.sh --uninstall     remove what an install with the same options put in place
#
# Archive support (opening, extracting, creating archives) is a helper built with the
# .NET 10 SDK from CompressionWorkbench (by Hawkynt, vendored in vendor/compressionworkbench).
# Without an SDK on the PATH the script downloads one into ~/.local/share/failbrauwser/dotnet
# (no root needed, only used for building).
#
# Typed smb:// addresses are mounted through GVfs; its SMB backend is installed with the
# system's package manager (asks for sudo) unless it is already there.
set -euo pipefail

APP_ID=org.failbrauwser.FailBrauwser
HERE=$(cd "$(dirname "$0")" && pwd)
PREFIX="$HOME/.local"
SUDO=""
AUTOSTART=1
MAKE_DEFAULT=0
NETWORK=1
ARCHIVES=1
UNINSTALL=0
CWB_ROOT=${CWB_ROOT:-"$HERE/vendor/compressionworkbench"}

while [ $# -gt 0 ]; do
    case "$1" in
        --system) PREFIX=/usr/local; SUDO=sudo ;;
        --prefix) PREFIX=$2; shift ;;
        --no-autostart) AUTOSTART=0 ;;
        --default) MAKE_DEFAULT=1 ;;
        --no-network) NETWORK=0 ;;
        --no-archives) ARCHIVES=0 ;;
        --uninstall) UNINSTALL=1 ;;
        -h|--help) sed -n '2,19p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done

BIN_DIR="$PREFIX/bin"
LIB_DIR="$PREFIX/lib/failbrauwser"
SHARE="$PREFIX/share"
AUTOSTART_FILE="${XDG_CONFIG_HOME:-$HOME/.config}/autostart/failbrauwser.desktop"

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }

refresh_caches() {
    command -v update-desktop-database >/dev/null && $SUDO update-desktop-database -q "$SHARE/applications" 2>/dev/null || true
    command -v gtk-update-icon-cache >/dev/null && $SUDO gtk-update-icon-cache -q -t -f "$SHARE/icons/hicolor" 2>/dev/null || true
}

if [ "$UNINSTALL" = 1 ]; then
    say "Stopping the background instance"
    "$BIN_DIR/failbrauwser" --quit 2>/dev/null || true
    say "Removing failBrauwser from $PREFIX"
    $SUDO rm -f "$BIN_DIR/failbrauwser" \
        "$SHARE/applications/$APP_ID.desktop" \
        "$SHARE/icons/hicolor/scalable/apps/$APP_ID.svg"
    $SUDO rm -f "$LIB_DIR/fb-archive"
    $SUDO rmdir "$LIB_DIR" 2>/dev/null || true
    rm -f "$AUTOSTART_FILE"
    refresh_caches
    say "Done. Settings in ~/.config/failbrauwser were kept."
    exit 0
fi

command -v cargo >/dev/null || { echo "cargo (Rust) is required: https://rustup.rs" >&2; exit 1; }
pkg-config --exists gtk+-3.0 || { echo "GTK 3 development files are required (gtk3)" >&2; exit 1; }

HELPER=""
HELPER_PID=""
HELPER_LOG="$HERE/target/helper-build.log"
DOTNET_HOME="${XDG_DATA_HOME:-$HOME/.local/share}/failbrauwser/dotnet"

# A .NET SDK that can build net10.0: on the PATH, from an earlier run, or downloaded now.
has_sdk10() { "$1" --list-sdks 2>/dev/null | grep -Eq '^(1[0-9]|[2-9][0-9])\.'; }
find_dotnet() {
    if command -v dotnet >/dev/null && has_sdk10 dotnet; then command -v dotnet; return; fi
    if [ -x "$DOTNET_HOME/dotnet" ] && has_sdk10 "$DOTNET_HOME/dotnet"; then echo "$DOTNET_HOME/dotnet"; return; fi
    say "Downloading the .NET 10 SDK into $DOTNET_HOME (for building the archive helper)" >&2
    local script
    script=$(mktemp)
    if command -v curl >/dev/null; then curl -fsSL https://dot.net/v1/dotnet-install.sh -o "$script"
    elif command -v wget >/dev/null; then wget -qO "$script" https://dot.net/v1/dotnet-install.sh
    else echo "curl or wget is needed to download the .NET SDK" >&2; rm -f "$script"; return 1
    fi || { rm -f "$script"; return 1; }
    bash "$script" --channel 10.0 --install-dir "$DOTNET_HOME" --no-path >&2 || { rm -f "$script"; return 1; }
    rm -f "$script"
    has_sdk10 "$DOTNET_HOME/dotnet" && echo "$DOTNET_HOME/dotnet"
}

if [ "$ARCHIVES" = 1 ] && [ -f "$CWB_ROOT/Compression.Lib/Compression.Lib.csproj" ]; then
    if DOTNET=$(find_dotnet); then
        # The helper is compiled to native code, which needs a C toolchain to link.
        AOT_LINKER=()
        if ! command -v clang >/dev/null; then
            if command -v gcc >/dev/null; then AOT_LINKER=(-p:CppCompilerAndLinker=gcc)
            else echo "warning: neither clang nor gcc found — the archive helper cannot be linked" >&2
            fi
        fi
        # Built alongside the app: most of the helper's native compile runs on one core,
        # so the Rust build uses the others meanwhile.
        say "Building the archive helper in the background (CompressionWorkbench at $CWB_ROOT; log: $HELPER_LOG)"
        mkdir -p "$HERE/target"
        (cd "$HERE" && PATH="$(dirname "$DOTNET"):$PATH" DOTNET_ROOT="$(dirname "$(readlink -f "$DOTNET")")" \
            DOTNET_CLI_TELEMETRY_OPTOUT=1 DOTNET_NOLOGO=1 \
            make helper CWB_ROOT="$(cd "$CWB_ROOT" && pwd)" HELPER_FLAGS="${AOT_LINKER[*]}") >"$HELPER_LOG" 2>&1 &
        HELPER_PID=$!
        # Do not leave it running when the app build fails.
        trap '[ -n "$HELPER_PID" ] && kill "$HELPER_PID" 2>/dev/null' EXIT
    else
        echo "warning: no .NET 10 SDK could be set up — installing without archive support" >&2
    fi
elif [ "$ARCHIVES" = 1 ]; then
    echo "warning: $CWB_ROOT has no CompressionWorkbench — installing without archive support" >&2
fi

say "Building failBrauwser"
(cd "$HERE" && cargo build --release --locked)

if [ -n "$HELPER_PID" ]; then
    say "Waiting for the archive helper"
    if wait "$HELPER_PID"; then
        HELPER="$HERE/target/helper/fb-archive"
    else
        grep -v 'warning CS' "$HELPER_LOG" | tail -20 >&2
        echo "warning: the archive helper did not build (log: $HELPER_LOG) — installing without archive support" >&2
    fi
    HELPER_PID=""
fi

say "Installing into $PREFIX"
$SUDO install -Dm755 "$HERE/target/release/failbrauwser" "$BIN_DIR/failbrauwser"
if [ -n "$HELPER" ]; then
    $SUDO install -Dm755 "$HELPER" "$LIB_DIR/fb-archive"
fi
$SUDO install -Dm644 "$HERE/data/$APP_ID.desktop" "$SHARE/applications/$APP_ID.desktop"
$SUDO install -Dm644 "$HERE/data/icons/$APP_ID.svg" "$SHARE/icons/hicolor/scalable/apps/$APP_ID.svg"
if [ "$PREFIX" != /usr ] && [ "$PREFIX" != /usr/local ]; then
    # Launchers need the absolute path when the prefix is not on everyone's PATH.
    $SUDO sed -i "s|^Exec=failbrauwser|Exec=$BIN_DIR/failbrauwser|; s|^TryExec=failbrauwser|TryExec=$BIN_DIR/failbrauwser|" "$SHARE/applications/$APP_ID.desktop"
fi
refresh_caches

# The GVfs SMB backend, for typed smb:// addresses.
if [ "$NETWORK" = 1 ] && [ ! -e /usr/share/gvfs/mounts/smb.mount ]; then
    if command -v pacman >/dev/null; then smb_install=(sudo pacman -S --needed gvfs-smb)
    elif command -v apt-get >/dev/null; then smb_install=(sudo apt-get install -y gvfs-backends)
    elif command -v dnf >/dev/null; then smb_install=(sudo dnf install -y gvfs-smb)
    elif command -v zypper >/dev/null; then smb_install=(sudo zypper install gvfs-backend-samba)
    else smb_install=()
    fi
    if [ ${#smb_install[@]} -gt 0 ]; then
        say "Installing the GVfs SMB backend (smb:// addresses): ${smb_install[*]}"
        if "${smb_install[@]}"; then
            # The running GVfs daemon only knows the backends it started with.
            systemctl --user restart gvfs-daemon 2>/dev/null || true
        else
            echo "warning: the GVfs SMB backend was not installed — smb:// addresses will not open" >&2
        fi
    else
        echo "warning: install your distribution's GVfs SMB backend for smb:// addresses" >&2
    fi
fi

if [ "$AUTOSTART" = 1 ]; then
    say "Starting a background instance at login (windows then open instantly)"
    install -Dm644 "$HERE/data/failbrauwser-autostart.desktop" "$AUTOSTART_FILE"
    sed -i "s|^Exec=failbrauwser|Exec=$BIN_DIR/failbrauwser|" "$AUTOSTART_FILE"
else
    rm -f "$AUTOSTART_FILE"
fi

if [ "$MAKE_DEFAULT" = 1 ]; then
    say "Making failBrauwser the default file manager"
    xdg-mime default "$APP_ID.desktop" inode/directory
fi

# Replace a running (older) instance and keep the new one ready right away.
"$BIN_DIR/failbrauwser" --quit 2>/dev/null || true
if [ "$AUTOSTART" = 1 ] && [ -n "${DBUS_SESSION_BUS_ADDRESS:-}" ]; then
    (setsid "$BIN_DIR/failbrauwser" --daemon >/dev/null 2>&1 &)
fi

say "Installed. Start it from your menu or run: failbrauwser"
case ":$PATH:" in *":$BIN_DIR:"*) ;; *) echo "note: $BIN_DIR is not on your PATH" ;; esac
