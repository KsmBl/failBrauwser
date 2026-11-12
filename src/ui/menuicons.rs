//! Icons in the right-click menus: each item gets the icon of the theme that fits its text,
//! from freedesktop.org's standard names first, then names other themes use for the same.

use gtk::prelude::*;
use gtk::{gio, glib};

/// The menu text without mnemonic underscores and the ellipsis of items that ask first.
pub fn plain_text(label: &str) -> String {
    label.replace('_', "").trim_end_matches('…').trim_end_matches("...").to_string()
}

/// Icon names for a menu item by its text, the preferred first (empty: no icon fits).
pub fn icon_names(label: &str) -> &'static [&'static str] {
    match plain_text(label).as_str() {
        "Open" => &["document-open", "folder-open"],
        "Open in New Tab" => &["tab-new", "window-new", "list-add"],
        "Open With" => &["system-run", "application-x-executable"],
        "Other Application" => &["application-x-executable", "system-run"],
        "Cut" => &["edit-cut"],
        "Copy" => &["edit-copy"],
        "Copy Address" => &["edit-copy", "insert-link"],
        "Paste" | "Paste Into Folder" => &["edit-paste"],
        "Move to Trash" => &["user-trash", "edit-delete"],
        "Delete" | "Delete Permanently" => &["edit-delete", "user-trash-full"],
        "Rename" => &["edit-rename", "document-edit", "accessories-text-editor"],
        "Edit" | "Edit Custom Actions" => &["document-edit", "edit-rename", "preferences-other"],
        "Extract Here" | "Extract To" => &["archive-extract", "extract-archive", "package-x-generic"],
        "Open as Archive" => &["package-x-generic", "application-x-archive", "archive-manager"],
        "Compress" => &["archive-insert", "package-x-generic", "application-x-archive"],
        "Compare" => &["view-split-left-right", "view-dual", "edit-find-replace"],
        "Mount as Drive" | "Mount" => &["media-mount", "drive-harddisk", "drive-removable-media"],
        "Unmount" => &["media-eject", "drive-harddisk"],
        "Eject" => &["media-eject"],
        "Unlock" => &["changes-allow", "dialog-password", "system-lock-screen"],
        "Retry" | "Reload" => &["view-refresh"],
        "Create Link on Desktop" => &["emblem-symbolic-link", "insert-link", "user-desktop"],
        "Create Folder" => &["folder-new"],
        "Create Document" => &["document-new"],
        "Select All" => &["edit-select-all"],
        "Show Hidden Files" | "Show Hidden Entry" => &["view-visible", "view-reveal-symbolic", "view-hidden"],
        "Hide" | "Hide Mounted Volumes" => &["view-hidden", "view-conceal-symbolic"],
        "Properties" => &["document-properties"],
        "Empty Trash" => &["user-trash-full", "edit-clear"],
        "Restore" => &["edit-undo", "document-revert"],
        "Pin to Side Panel" => &["view-pin", "bookmark-new", "list-add"],
        "Add Current Folder" | "Add Server" => &["list-add", "bookmark-new"],
        "Remove" => &["list-remove", "edit-delete"],
        "Move Up" => &["go-up"],
        "Move Down" => &["go-down"],
        _ => &[],
    }
}

/// The first of `names` the current icon theme has; else the first of their symbolic
/// forms (Adwaita has only those).
pub fn themed(names: &[&str]) -> Option<String> {
    let theme = gtk::IconTheme::default()?;
    with_symbolic(names).into_iter().find(|n| theme.has_icon(n))
}

/// `names`, then each of them as a symbolic icon.
pub fn with_symbolic(names: &[&str]) -> Vec<String> {
    let symbolic = names.iter().filter(|n| !n.ends_with("-symbolic")).map(|n| format!("{n}-symbolic"));
    names.iter().map(|n| n.to_string()).chain(symbolic).collect()
}

/// The icon for the menu text `label`, when the theme has one.
pub fn icon_for(label: &str) -> Option<gio::ThemedIcon> {
    themed(icon_names(label)).map(|n| gio::ThemedIcon::new(&n))
}

/// An item of a menu model with its action and icon.
pub fn model_item(label: &str, action: &str) -> gio::MenuItem {
    let item = gio::MenuItem::new(Some(label), Some(action));
    if let Some(icon) = icon_for(label) {
        item.set_icon(&icon);
    }
    item
}

/// A submenu of a menu model with its icon.
pub fn model_submenu(label: &str, sub: &gio::Menu) -> gio::MenuItem {
    let item = gio::MenuItem::new_submenu(Some(label), sub);
    if let Some(icon) = icon_for(label) {
        item.set_icon(&icon);
    }
    item
}

/// A menu item showing the icon for `label` (with a mnemonic) in front of it; where the
/// theme has none, an empty space of the same size keeps the texts lined up.
pub fn item(label: &str) -> gtk::MenuItem {
    let item = gtk::MenuItem::new();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let image = match themed(icon_names(label)) {
        Some(name) => gtk::Image::from_icon_name(Some(&name), gtk::IconSize::Menu),
        None => gtk::Image::new(),
    };
    image.set_pixel_size(16);
    image.set_size_request(16, 16);
    let text = gtk::AccelLabel::new("");
    text.set_text_with_mnemonic(label);
    text.set_mnemonic_widget(Some(&item));
    text.set_accel_widget(Some(&item));
    text.set_xalign(0.0);
    row.pack_start(&image, false, false, 0);
    row.pack_start(&text, true, true, 0);
    item.add(&row);
    item
}

/// The text of a menu item, with its mnemonic underscore, also of one made by [`item`]
/// (tests find items by it).
pub fn item_text(item: &gtk::MenuItem) -> Option<glib::GString> {
    item.label().or_else(|| {
        let row = item.child()?.downcast::<gtk::Box>().ok()?;
        row.children().into_iter().find_map(|c| c.downcast::<gtk::Label>().ok()).map(|l| l.label())
    })
}
