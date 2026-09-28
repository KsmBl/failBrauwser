//! Where features plug into windows and panes: the side panel, the drives page, archives
//! and file operations each add their part here.

use super::model::Item;
use super::pane::Pane;
use super::window::Window;
use gtk::gio;
use gtk::prelude::*;
use std::rc::Rc;

pub fn window_created(_w: &Rc<Window>) {}

pub fn pane_contents_changed(_w: &Window, _p: &Rc<Pane>) {}

pub fn settings_changed(_w: &Window) {}

/// Items that are not plain local files (entries inside archives).
pub fn open_non_local(_w: &Window, _item: &Item) {}

/// The right-click menu for the current selection (or the folder background).
pub fn context_menu_model(w: &Window, pane: &Rc<Pane>, items: &[Item]) -> gio::Menu {
    let menu = gio::Menu::new();
    let open = gio::Menu::new();
    if items.is_empty() {
        let s = gio::Menu::new();
        s.append(Some("Select _All"), Some("win.select-all"));
        s.append(Some("Show _Hidden Files"), Some("win.show-hidden"));
        s.append(Some("_Reload"), Some("win.reload"));
        menu.append_section(None, &s);
        let _ = (w, pane);
        return menu;
    }
    open.append(Some("_Open"), Some("win.open"));
    if items.iter().all(Item::is_dir_like) {
        open.append(Some("Open in New _Tab"), Some("win.open-in-tab"));
    }
    if let Some(sub) = open_with_menu(items) {
        open.append_submenu(Some("Open _With"), &sub);
    }
    menu.append_section(None, &open);
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
