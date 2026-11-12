#!/usr/bin/env python3
"""A GTK 3 application that makes GTK's own file dialog, as xfce4-terminal's "Save
Contents…" does, and writes what it got, one line: "<response> <file> … | <filter>".

    gtk3_dialog.py save|open|several|folder <out file> <folder> [<name>]

Saving answers with GTK_RESPONSE_ACCEPT, the others with GTK_RESPONSE_OK, as
applications do."""
import sys

import gi

gi.require_version("Gtk", "3.0")
from gi.repository import Gtk  # noqa: E402

kind, out, folder = sys.argv[1], sys.argv[2], sys.argv[3]
action = {"save": Gtk.FileChooserAction.SAVE, "folder": Gtk.FileChooserAction.SELECT_FOLDER}.get(kind, Gtk.FileChooserAction.OPEN)
title = "Save contents..." if kind == "save" else "Pick something"
dialog = Gtk.FileChooserDialog(title=title, action=action)
accept = Gtk.ResponseType.ACCEPT if kind == "save" else Gtk.ResponseType.OK
dialog.add_buttons("_Cancel", Gtk.ResponseType.CANCEL, "_Save" if kind == "save" else "_Pick", accept)
dialog.set_select_multiple(kind == "several")
dialog.set_current_folder(folder)
if len(sys.argv) > 4:
    dialog.set_current_name(sys.argv[4])
for name, pattern in [("Text files", "*.txt"), ("Logs", "*.log")]:
    f = Gtk.FileFilter()
    f.set_name(name)
    f.add_pattern(pattern)
    dialog.add_filter(f)
response = dialog.run()
files = " ".join(dialog.get_filenames()) if response == accept else ""
picked = dialog.get_filter().get_name() if dialog.get_filter() else ""
with open(out + ".part", "w") as fh:
    fh.write(f"{int(response)} {files} | {picked}".replace("  ", " "))
import os  # noqa: E402

os.rename(out + ".part", out)
