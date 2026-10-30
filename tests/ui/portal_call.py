#!/usr/bin/env python3
"""Calls failBrauwser's file chooser portal backend as xdg-desktop-portal does and writes
the answer, one line: "<response> <uri> <uri> …".

    portal_call.py <OpenFile|SaveFile|SaveFiles|Close> <request id> <out file> [title] [options as JSON]
    portal_call.py Frontend <token> <out file> [title]   (through xdg-desktop-portal itself)

Options: multiple, directory (booleans), current_name (text), current_folder (path),
files (names), filters ([[name, [pattern, …]], …]; a pattern with a slash is a type).
(The gdbus tool garbles longer byte strings, so the call is made here.)"""
import json
import sys

from gi.repository import Gio, GLib

BUS = "org.freedesktop.impl.portal.desktop.failbrauwser"
OBJECT = "/org/freedesktop/portal/desktop"
method, request, out = sys.argv[1], sys.argv[2], sys.argv[3]
handle = f"{OBJECT}/request/1/{request}"
conn = Gio.bus_get_sync(Gio.BusType.SESSION, None)
if method == "Frontend":
    # As an application asks: through xdg-desktop-portal, answered by a Response signal.
    title = sys.argv[4] if len(sys.argv) > 4 else ""
    loop = GLib.MainLoop()
    token = request
    sender = conn.get_unique_name()[1:].replace(".", "_")
    path = f"/org/freedesktop/portal/desktop/request/{sender}/{token}"

    def response(_c, _s, _p, _i, _sig, params):
        code, results = params.unpack()
        with open(out, "w") as f:
            f.write(" ".join([str(code)] + results.get("uris", [])) + "\n")
        loop.quit()

    conn.signal_subscribe("org.freedesktop.portal.Desktop", "org.freedesktop.portal.Request", "Response", path, None, Gio.DBusSignalFlags.NONE, response)
    opts = {"handle_token": GLib.Variant("s", token)}
    conn.call_sync("org.freedesktop.portal.Desktop", OBJECT, "org.freedesktop.portal.FileChooser", "OpenFile", GLib.Variant("(ssa{sv})", ("", title, opts)), None, Gio.DBusCallFlags.NONE, 60000, None)
    GLib.timeout_add_seconds(60, loop.quit)
    loop.run()
    sys.exit(0)
if method == "Close":
    conn.call_sync(BUS, handle, "org.freedesktop.impl.portal.Request", "Close", None, None, Gio.DBusCallFlags.NONE, -1, None)
    sys.exit(0)
title = sys.argv[4] if len(sys.argv) > 4 else ""
given = json.loads(sys.argv[5]) if len(sys.argv) > 5 else {}
opts = {}
for key, value in given.items():
    if key in ("multiple", "directory"):
        opts[key] = GLib.Variant("b", value)
    elif key == "current_name":
        opts[key] = GLib.Variant("s", value)
    elif key == "current_folder":
        opts[key] = GLib.Variant("ay", value.encode() + b"\0")
    elif key == "files":
        opts[key] = GLib.Variant("aay", [n.encode() + b"\0" for n in value])
    elif key == "filters":
        opts[key] = GLib.Variant("a(sa(us))", [(name, [(1 if "/" in p else 0, p) for p in pats]) for name, pats in value])
args = GLib.Variant("(osssa{sv})", (handle, "org.example.App", "", title, opts))
reply = conn.call_sync(BUS, OBJECT, "org.freedesktop.impl.portal.FileChooser", method, args, GLib.VariantType("(ua{sv})"), Gio.DBusCallFlags.NONE, 600000, None)
code, results = reply.unpack()
with open(out, "w") as f:
    f.write(" ".join([str(code)] + results.get("uris", [])) + "\n")
