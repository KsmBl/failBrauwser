//! Where features plug into windows and panes: the side panel, the drives page, archives
//! and file operations each add their part here.

use super::clipboard::ClipSource;
use super::model::Item;
use super::pane::Pane;
use super::window::Window;
use super::{dnd, fileops, util};
use failbrauwser::location::Location;
use failbrauwser::ops::copy::Mode;
use gtk::gio;
use gtk::prelude::*;
use std::rc::Rc;

pub fn window_created(w: &Rc<Window>) {
    fileops::install_actions(w);
    super::sidebar::attach(w);
}

/// Places that are not local folders (network shares mounted through GVfs without a path).
pub fn open_uri(w: &Window, uri: &str) {
    util::show_error(&w.win, "Cannot open location", &format!("“{uri}” is not a local folder."));
}

pub fn pane_created(w: &Window, p: &Rc<Pane>) {
    dnd::setup(w, p);
}

pub fn pane_contents_changed(_w: &Window, _p: &Rc<Pane>) {}

pub fn selection_changed(w: &Window) {
    fileops::update_sensitivity(w);
}

pub fn settings_changed(w: &Window) {
    if let Some(sb) = super::sidebar::for_window(w) {
        sb.apply_settings();
    }
}

/// Can files be created, changed or removed here?
pub fn location_writable(loc: &Location) -> bool {
    match loc {
        Location::Dir(p) => {
            use std::os::unix::ffi::OsStrExt;
            let Ok(c) = std::ffi::CString::new(p.as_os_str().as_bytes()) else { return false };
            unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
        }
        _ => false,
    }
}

/// Items that are not plain local files (entries inside archives).
pub fn open_non_local(_w: &Window, _item: &Item) {}

pub fn transfer_non_local(w: &Window, _sources: Vec<ClipSource>, _dest: Location, _mode: Mode, _after: Box<dyn FnOnce()>) {
    util::show_error(&w.win, "Not supported", "This kind of transfer is not supported.");
}

pub fn delete_non_local(_w: &Window, _pane: &Rc<Pane>) {}

pub fn rename_non_local(_w: &Window, _pane: &Rc<Pane>, _item: &Item, _new: &str) {}

pub fn create_non_local(_w: &Window, _loc: &Location, _name: &str, _folder: bool) {}

fn section(menu: &gio::Menu, items: &[(&str, &str)]) {
    let s = gio::Menu::new();
    for (label, action) in items {
        s.append(Some(label), Some(action));
    }
    menu.append_section(None, &s);
}

/// The right-click menu for the current selection (or the folder background).
pub fn context_menu_model(_w: &Window, pane: &Rc<Pane>, items: &[Item]) -> gio::Menu {
    let menu = gio::Menu::new();
    let local = matches!(pane.location(), Location::Dir(_));
    if items.is_empty() {
        section(&menu, &[("Create _Folder…", "win.new-folder"), ("Create _Document…", "win.new-file")]);
        section(&menu, &[("_Paste", "win.paste")]);
        section(&menu, &[("Select _All", "win.select-all"), ("Show _Hidden Files", "win.show-hidden"), ("_Reload", "win.reload")]);
        return menu;
    }
    let open = gio::Menu::new();
    open.append(Some("_Open"), Some("win.open"));
    if items.iter().all(Item::is_dir_like) {
        open.append(Some("Open in New _Tab"), Some("win.open-in-tab"));
    }
    if let Some(sub) = open_with_menu(items) {
        open.append_submenu(Some("Open _With"), &sub);
    }
    menu.append_section(None, &open);
    let mut edit = vec![("Cu_t", "win.cut"), ("_Copy", "win.copy")];
    if items.len() == 1 && items[0].is_dir_like() {
        edit.push(("_Paste Into Folder", "win.paste-into"));
    }
    section(&menu, &edit);
    let mut del = Vec::new();
    if local {
        del.push(("Move to T_rash", "win.trash"));
    }
    del.push(("_Delete", "win.delete"));
    del.push(("_Rename…", "win.rename"));
    section(&menu, &del);
    menu
}

/// Applications registered for the selection's type, plus "Other Application…".
pub fn open_with_menu(items: &[Item]) -> Option<gio::Menu> {
    let first = items.iter().find(|i| i.path().is_some() && !i.is_dir_like())?;
    let ct = first.content_type();
    let sub = gio::Menu::new();
    let apps = gio::AppInfo::recommended_for_type(&ct);
    for app in apps.iter().take(12) {
        let Some(id) = app.id() else { continue };
        let item = gio::MenuItem::new(Some(&app.display_name()), None);
        item.set_action_and_target_value(Some("win.open-with"), Some(&id.to_variant()));
        if let Some(icon) = app.icon() {
            item.set_icon(&icon);
        }
        sub.append_item(&item);
    }
    let other = gio::Menu::new();
    other.append(Some("Other _Application…"), Some("win.open-with-other"));
    sub.append_section(None, &other);
    Some(sub)
}
