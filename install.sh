#!/usr/bin/env bash
# Builds and installs failBrauwser.
#
#   ./install.sh                 install for the current user into ~/.local
#   ./install.sh --system        install for everyone into /usr/local (asks for sudo)
#   ./install.sh --prefix DIR    install into DIR
#   ./install.sh --no-autostart  do not start the background instance at login
#   ./install.sh --default       make failBrauwser the default file manager
#   ./install.sh --no-network    do not install the GVfs SMB backend (smb:// addresses)
#   ./install.sh --uninstall     remove what an install with the same options put in place
#
# Archive support needs the .NET 10 SDK; it builds CompressionWorkbench (by Hawkynt,
# vendored in vendor/compressionworkbench). Without dotnet failBrauwser is installed
# without archive support.
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
UNINSTALL=0
CWB_ROOT=${CWB_ROOT:-"$HERE/vendor/compressionworkbench"}

while [ $# -gt 0 ]; do
    case "$1" in
        --system) PREFIX=/usr/local; SUDO=sudo ;;
        --prefix) PREFIX=$2; shift ;;
        --no-autostart) AUTOSTART=0 ;;
        --default) MAKE_DEFAULT=1 ;;
        --no-network) NETWORK=0 ;;
        --uninstall) UNINSTALL=1 ;;
        -h|--help) sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
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

say "Building failBrauwser"
(cd "$HERE" && cargo build --release --locked)

HELPER=""
if command -v dotnet >/dev/null && [ -f "$CWB_ROOT/Compression.Lib/Compression.Lib.csproj" ]; then
    say "Building the archive helper (CompressionWorkbench at $CWB_ROOT)"
    (cd "$HERE" && make helper CWB_ROOT="$(cd "$CWB_ROOT" && pwd)")
    HELPER="$HERE/target/helper/fb-archive"
else
    echo "warning: dotnet not found — installing without archive support" >&2
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
