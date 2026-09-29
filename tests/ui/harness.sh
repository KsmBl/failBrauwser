#!/bin/bash
# Runs failBrauwser with a self-test script inside a headless sway session.
#   harness.sh <binary> <script> <workdir>
# The script may use $T for <workdir>. Config, data and trash live below <workdir>, so the
# user's settings and trash are never touched; the GTK theme settings are copied over.
set -u
BIN=$1; SCRIPT=$2; T=$3
# Parallel runs would pick up each other's compositor socket: one at a time.
exec 9>"${TMPDIR:-/tmp}/failbrauwser-ui-test.lock"
flock 9
mkdir -p "$T/config" "$T/data" "$T/cache"
# The look comes from the user's desktop (even when HOME is pointed at demo data).
TH=${FB_THEME_HOME:-$HOME}
[ -d "$TH/.config/gtk-3.0" ] && cp -r "$TH/.config/gtk-3.0" "$T/config/" 2>/dev/null
# On Wayland GTK takes theme, icons and fonts from GSettings (the dconf database).
[ -d "$TH/.config/dconf" ] && cp -r "$TH/.config/dconf" "$T/config/" 2>/dev/null
# Icon themes installed per user live in the data dir; keep them visible.
[ -d "$TH/.local/share/icons" ] && ln -sfn "$TH/.local/share/icons" "$T/data/icons"

sed "s|\$T|$T|g" "$SCRIPT" > "$T/script.fbt"
CONF=$(mktemp)
cat > "$CONF" <<SWAY
output HEADLESS-1 resolution 1280x800 bg #2e3440 solid_color
default_border normal
SWAY
# Before the private bus starts: services it activates (Tumbler, …) use the test folders too.
export XDG_CONFIG_HOME="$T/config" XDG_DATA_HOME="$T/data" XDG_CACHE_HOME="$T/cache" GTK_A11Y=none NO_AT_BRIDGE=1
exec dbus-run-session -- bash -c '
  unset DISPLAY
  before=$(ls "$XDG_RUNTIME_DIR" | grep -E "^wayland-[0-9]+$" | sort)
  WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman sway -c "'"$CONF"'" >"'"$T"'/sway.log" 2>&1 &
  SP=$!
  for i in $(seq 100); do
    new=$(comm -13 <(echo "$before") <(ls "$XDG_RUNTIME_DIR" | grep -E "^wayland-[0-9]+$" | sort) | head -1)
    [ -n "$new" ] && break; sleep 0.05
  done
  export WAYLAND_DISPLAY=$new GDK_BACKEND=wayland FB_SELFTEST="'"$T"'/script.fbt"
  timeout 60 "'"$BIN"'"
  rc=$?
  kill $SP 2>/dev/null; wait $SP 2>/dev/null
  exit $rc
' 2>&1 | grep -v -E "dbus-daemon|SpiRegistry|fusermount"
rc=${PIPESTATUS[0]}
rm -f "$CONF"
exit $rc
