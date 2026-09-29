//! The side panel: shortcuts (GTK's own places sidebar: home, bookmarks, drives with
//! mount/unmount/eject) above a folder tree.

use super::util;
use super::window::Window;
use failbrauwser::config::TreeRoot;
use failbrauwser::location::Location;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

const T_ICON: u32 = 0;
const T_NAME: u32 = 1;
const T_PATH: u32 = 2;
/// Children have been read (a placeholder child is shown until then).
const T_LOADED: u32 = 3;

pub struct Sidebar {
    pub places: gtk::PlacesSidebar,
    /// "Drives" above the places (GTK's places list cannot show custom locations).
    drives: gtk::ListBox,
    tree: gtk::TreeView,
    store: gtk::TreeStore,
    tree_box: gtk::ScrolledWindow,
    split: gtk::Paned,
    window: Weak<Window>,
    root: RefCell<Option<PathBuf>>,
    /// Ignore selection changes we cause ourselves while revealing the current folder.
    syncing: std::cell::Cell<bool>,
}

thread_local! {
    static SIDEBARS: RefCell<Vec<(Weak<Window>, Rc<Sidebar>)>> = const { RefCell::new(Vec::new()) };
}

/// The sidebar belonging to a window.
pub fn for_window(w: &Window) -> Option<Rc<Sidebar>> {
    SIDEBARS.with(|s| {
        let mut s = s.borrow_mut();
        s.retain(|(w, _)| w.strong_count() > 0);
        s.iter().find(|(x, _)| x.upgrade().is_some_and(|x| std::ptr::eq(&*x, w))).map(|(_, sb)| sb.clone())
    })
}

pub fn attach(w: &Rc<Window>) {
    let places = gtk::PlacesSidebar::new();
    places.set_show_recent(false);
    places.set_show_trash(true);
    places.set_show_other_locations(false);
    places.set_show_starred_location(false);
    places.set_show_enter_location(false);
    places.set_show_desktop(true);
    places.set_open_flags(gtk::PlacesOpenFlags::NORMAL | gtk::PlacesOpenFlags::NEW_TAB | gtk::PlacesOpenFlags::NEW_WINDOW);

    let store = gtk::TreeStore::new(&[gio::Icon::static_type(), glib::Type::STRING, glib::Type::STRING, glib::Type::BOOL]);
    let tree = gtk::TreeView::with_model(&store);
    tree.set_headers_visible(false);
    tree.set_enable_search(true);
    tree.set_search_column(T_NAME as i32);
    tree.set_show_expanders(true);
    tree.set_level_indentation(0);
    let col = gtk::TreeViewColumn::new();
    let pix = gtk::CellRendererPixbuf::new();
    let text = gtk::CellRendererText::new();
    text.set_ellipsize(gtk::pango::EllipsizeMode::End);
    TreeViewColumnExt::pack_start(&col, &pix, false);
    TreeViewColumnExt::add_attribute(&col, &pix, "gicon", T_ICON as i32);
    TreeViewColumnExt::pack_start(&col, &text, true);
    TreeViewColumnExt::add_attribute(&col, &text, "text", T_NAME as i32);
    tree.append_column(&col);
    let tree_box = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    tree_box.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    tree_box.add(&tree);

    let drives = gtk::ListBox::new();
    drives.style_context().add_class("sidebar");
    drives.style_context().add_class("fb-shortcuts");
    drives.set_selection_mode(gtk::SelectionMode::Single);
    drives.set_activate_on_single_click(true);
    let row = gtk::ListBoxRow::new();
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let icon = gtk::Image::from_gicon(&gio::ThemedIcon::from_names(&["drive-harddisk-symbolic", "drive-harddisk"]), gtk::IconSize::Menu);
    icon.style_context().add_class("sidebar-icon");
    let label = gtk::Label::new(Some("Drives"));
    label.set_xalign(0.0);
    label.style_context().add_class("sidebar-label");
    content.pack_start(&icon, false, false, 0);
    content.pack_start(&label, true, true, 0);
    row.add(&content);
    row.set_tooltip_text(Some("All drives with their fill levels (Alt+D)"));
    drives.add(&row);
    let top = gtk::Box::new(gtk::Orientation::Vertical, 0);
    top.pack_start(&drives, false, false, 0);
    top.pack_start(&places, true, true, 0);

    let split = gtk::Paned::new(gtk::Orientation::Vertical);
    split.pack1(&top, true, false);
    split.pack2(&tree_box, true, false);
    split.set_position(w.app.settings.borrow().sidebar_split);
    w.side.pack_start(&split, true, true, 0);
    w.side.show_all();

    let sb = Rc::new(Sidebar {
        places,
        drives,
        tree,
        store,
        tree_box,
        split,
        window: Rc::downgrade(w),
        root: RefCell::new(None),
        syncing: std::cell::Cell::new(false),
    });
    sb.connect();
    SIDEBARS.with(|s| s.borrow_mut().push((Rc::downgrade(w), sb.clone())));
    sb.apply_settings();
    let weak = Rc::downgrade(&sb);
    w.connect_location_changed(move |_| {
        if let Some(sb) = weak.upgrade() {
            sb.location_changed();
        }
    });
}

impl Sidebar {
    /// Activates the Drives entry as a click would (tests).
    pub fn click_drives(&self) {
        if let Some(r) = self.drives.row_at_index(0) {
            r.activate();
        }
    }

    pub fn drives_selected(&self) -> bool {
        self.drives.selected_row().is_some()
    }

    /// The folder at the top of the tree.
    pub fn root_path(&self) -> Option<PathBuf> {
        self.root.borrow().clone()
    }

    /// The folder selected in the tree.
    pub fn selected_path(&self) -> Option<PathBuf> {
        let (model, iter) = self.tree.selection().selected()?;
        let p: String = model.value(&iter, T_PATH as i32).get().ok()?;
        Some(PathBuf::from(p))
    }

    fn tree_path_of(&self, dir: &Path) -> Option<gtk::TreePath> {
        let mut found = None;
        self.store.foreach(|m, path, iter| {
            let p: String = m.value(iter, T_PATH as i32).get().unwrap_or_default();
            if Path::new(&p) == dir {
                found = Some(path.clone());
                return true;
            }
            false
        });
        found
    }

    /// Re-reads the subfolders of `dir` if the tree already shows them (after changes).
    pub fn refresh_dir(&self, dir: &Path) {
        let Some(p) = self.tree_path_of(dir) else { return };
        let Some(iter) = self.store.iter(&p) else { return };
        let loaded: bool = self.store.value(&iter, T_LOADED as i32).get().unwrap_or(false);
        let has_children = self.store.iter_children(Some(&iter)).is_some();
        if !loaded || !has_children && !self.tree.row_expanded(&p) {
            // Never read, or known to be empty and closed: read when opened.
            if !has_children {
                self.add_placeholder(&iter);
                self.store.set_value(&iter, T_LOADED, &false.to_value());
            }
            return;
        }
        self.store.set_value(&iter, T_LOADED, &false.to_value());
        self.load_children(&iter, None);
    }

    /// The subfolder names the tree shows below `dir` (tests).
    pub fn children_of(&self, dir: &Path) -> Vec<String> {
        let Some(iter) = self.tree_path_of(dir).and_then(|p| self.store.iter(&p)) else { return Vec::new() };
        let mut out = Vec::new();
        let mut child = self.store.iter_children(Some(&iter));
        while let Some(c) = child {
            let path: String = self.store.value(&c, T_PATH as i32).get().unwrap_or_default();
            if !path.is_empty() {
                out.push(self.store.value(&c, T_NAME as i32).get().unwrap_or_default());
            }
            child = if self.store.iter_next(&c) { Some(c) } else { None };
        }
        out
    }

    /// Expands a folder once, like a click on its triangle (it must be shown).
    pub fn expand(&self, dir: &Path) -> bool {
        match self.tree_path_of(dir) {
            Some(p) => {
                self.tree.expand_row(&p, false);
                true
            }
            None => false,
        }
    }

    /// The folder is expanded and shows its real subfolders (not the placeholder).
    pub fn expanded_with_children(&self, dir: &Path) -> bool {
        let Some(p) = self.tree_path_of(dir) else { return false };
        let Some(iter) = self.store.iter(&p) else { return false };
        let first: Option<String> = self.store.iter_children(Some(&iter)).map(|c| self.store.value(&c, T_PATH as i32).get().unwrap_or_default());
        self.tree.row_expanded(&p) && first.is_some_and(|f| !f.is_empty())
    }

    /// Selects a folder in the tree as if clicked (it must be shown).
    pub fn click(&self, dir: &Path) -> bool {
        let mut found = None;
        self.store.foreach(|m, path, iter| {
            let p: String = m.value(iter, T_PATH as i32).get().unwrap_or_default();
            if Path::new(&p) == dir {
                found = Some(path.clone());
                return true;
            }
            false
        });
        match found {
            Some(p) => {
                self.tree.selection().select_path(&p);
                true
            }
            None => false,
        }
    }

    fn connect(self: &Rc<Self>) {
        let win = self.window.clone();
        self.places.connect_open_location(move |_, file, flags| {
            let Some(w) = win.upgrade() else { return };
            let loc = match file.path() {
                Some(p) => Location::Dir(p),
                None => {
                    super::extensions::open_uri(&w, &file.uri());
                    return;
                }
            };
            if flags.contains(gtk::PlacesOpenFlags::NEW_WINDOW) {
                w.app.open_window(vec![loc]);
            } else if flags.contains(gtk::PlacesOpenFlags::NEW_TAB) {
                w.add_tab(loc, true);
            } else {
                w.navigate(loc);
            }
        });

        let win = self.window.clone();
        self.drives.connect_row_activated(move |_, _| {
            if let Some(w) = win.upgrade() {
                w.navigate(Location::Drives);
            }
        });

        let me = Rc::downgrade(self);
        self.tree.connect_test_expand_row(move |_, iter, _| {
            if let Some(sb) = me.upgrade() {
                sb.load_children(iter, None);
            }
            glib::Propagation::Proceed
        });
        let me = Rc::downgrade(self);
        self.tree.selection().connect_changed(move |sel| {
            let Some(sb) = me.upgrade() else { return };
            if sb.syncing.get() {
                return;
            }
            let Some((model, iter)) = sel.selected() else { return };
            let path: String = model.value(&iter, T_PATH as i32).get().unwrap_or_default();
            if path.is_empty() {
                return;
            }
            if let Some(w) = sb.window.upgrade() {
                let target = Location::Dir(PathBuf::from(path));
                if w.current_location() != target {
                    w.current_pane().navigate(target);
                }
            }
        });
        let me = Rc::downgrade(self);
        self.tree.connect_button_press_event(move |tree, ev| {
            let Some(sb) = me.upgrade() else { return glib::Propagation::Proceed };
            if ev.button() == 2 && ev.event_type() == gdk::EventType::ButtonPress {
                let (x, y) = ev.position();
                if let Some((Some(path), ..)) = tree.path_at_pos(x as i32, y as i32) {
                    if let Some(iter) = sb.store.iter(&path) {
                        let p: String = sb.store.value(&iter, T_PATH as i32).get().unwrap_or_default();
                        if let Some(w) = sb.window.upgrade() {
                            w.add_tab(Location::Dir(PathBuf::from(p)), false);
                        }
                    }
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        let win = self.window.clone();
        self.split.connect_position_notify(move |p| {
            if let Some(w) = win.upgrade() {
                w.app.settings.borrow_mut().sidebar_split = p.position();
                w.app.save_settings_soon();
            }
        });
    }

    /// Show or hide the tree and pick its root after a settings change.
    pub fn apply_settings(&self) {
        let Some(w) = self.window.upgrade() else { return };
        let show = w.app.settings.borrow().show_tree;
        self.tree_box.set_visible(show);
        *self.root.borrow_mut() = None;
        self.location_changed();
    }

    fn wanted_root(&self, current: Option<&Path>) -> Option<PathBuf> {
        let w = self.window.upgrade()?;
        let mode = w.app.settings.borrow().tree_root;
        match mode {
            TreeRoot::Home => Some(glib::home_dir()),
            TreeRoot::Filesystem => Some(PathBuf::from("/")),
            TreeRoot::Current => current.map(Path::to_path_buf),
        }
    }

    fn location_changed(&self) {
        let Some(w) = self.window.upgrade() else { return };
        let loc = w.current_location();
        let dir = loc.nearest_dir();
        match (&loc, loc.local_path()) {
            (Location::Trash, _) => self.places.set_location(Some(&gio::File::for_uri(failbrauwser::location::TRASH_URI))),
            (_, Some(p)) => self.places.set_location(Some(&gio::File::for_path(p))),
            _ => self.places.set_location(None::<&gio::File>),
        }
        if loc == Location::Drives {
            if let Some(r) = self.drives.row_at_index(0) {
                self.drives.select_row(Some(&r));
            }
        } else {
            self.drives.unselect_all();
        }
        if !w.app.settings.borrow().show_tree {
            return;
        }
        let Some(root) = self.wanted_root(dir.as_deref()) else { return };
        if self.root.borrow().as_ref() != Some(&root) {
            self.set_root(&root);
        }
        if let Some(d) = dir {
            self.reveal(&d);
        }
    }

    fn set_root(&self, root: &Path) {
        *self.root.borrow_mut() = Some(root.to_path_buf());
        self.store.clear();
        let name = if root == Path::new("/") {
            "File System".to_string()
        } else if root == glib::home_dir() {
            "Home".to_string()
        } else {
            root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| root.display().to_string())
        };
        let icon: gio::Icon = if root == Path::new("/") {
            gio::ThemedIcon::from_names(&["drive-harddisk", "folder"]).upcast()
        } else if root == glib::home_dir() {
            gio::ThemedIcon::from_names(&["user-home", "folder"]).upcast()
        } else {
            util::folder_icon()
        };
        let iter = self.store.insert_with_values(None, None, &[(T_ICON, &icon), (T_NAME, &name), (T_PATH, &root.to_string_lossy().to_string()), (T_LOADED, &false)]);
        self.add_placeholder(&iter);
        if let Some(p) = self.store.path(&iter) {
            self.load_children(&iter, None);
            self.tree.expand_row(&p, false);
        }
    }

    fn add_placeholder(&self, parent: &gtk::TreeIter) {
        self.store.insert_with_values(Some(parent), None, &[(T_NAME, &"…"), (T_PATH, &String::new()), (T_LOADED, &true)]);
    }

    /// Reads the subfolders of a node (once), then continues with `then`.
    fn load_children(&self, iter: &gtk::TreeIter, then: Option<Box<dyn FnOnce(&Sidebar)>>) {
        let loaded: bool = self.store.value(iter, T_LOADED as i32).get().unwrap_or(false);
        if loaded {
            if let Some(f) = then {
                f(self);
            }
            return;
        }
        self.store.set_value(iter, T_LOADED, &true.to_value());
        let path: String = self.store.value(iter, T_PATH as i32).get().unwrap_or_default();
        let show_hidden = self.window.upgrade().is_some_and(|w| w.app.settings.borrow().show_hidden);
        let Some(tree_path) = self.store.path(iter) else { return };
        let row = gtk::TreeRowReference::new(&self.store, &tree_path);
        let me = SIDEBARS.with(|s| s.borrow().iter().find(|(_, sb)| std::ptr::eq(&**sb, self)).map(|(_, sb)| Rc::downgrade(sb)));
        let Some(me) = me else { return };
        glib::spawn_future_local(async move {
            let dirs = gio::spawn_blocking(move || subfolders(Path::new(&path), show_hidden)).await.unwrap_or_default();
            let Some(sb) = me.upgrade() else { return };
            let Some(iter) = row.and_then(|r| r.path()).and_then(|p| sb.store.iter(&p)) else { return };
            // Add the real subfolders first, then drop the placeholder: a row that is left
            // without children for a moment gets collapsed by GTK, which made the first
            // click on an expander look like it did nothing.
            let mut old = Vec::new();
            let mut child = sb.store.iter_children(Some(&iter));
            while let Some(c) = child {
                old.push(c.clone());
                child = if sb.store.iter_next(&c) { Some(c) } else { None };
            }
            for d in dirs {
                let icon = super::util::icon_for_entry(&d.entry);
                let child = sb.store.insert_with_values(Some(&iter), None, &[(T_ICON, &icon), (T_NAME, &d.name), (T_PATH, &d.entry.path.to_string_lossy().to_string()), (T_LOADED, &!d.has_children)]);
                if d.has_children {
                    sb.add_placeholder(&child);
                }
            }
            for c in old {
                sb.store.remove(&c);
            }
            if let Some(f) = then {
                f(&sb);
            }
        });
    }

    /// Expands the tree down to `dir` and selects it.
    fn reveal(&self, dir: &Path) {
        let Some(root) = self.root.borrow().clone() else { return };
        let Some(root_iter) = self.store.iter_first() else { return };
        let Ok(rest) = dir.strip_prefix(&root) else {
            self.syncing.set(true);
            self.tree.selection().unselect_all();
            self.syncing.set(false);
            return;
        };
        let parts: Vec<String> = rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        self.reveal_step(root_iter, parts, 0);
    }

    fn reveal_step(&self, iter: gtk::TreeIter, parts: Vec<String>, i: usize) {
        if i == parts.len() {
            if let Some(p) = self.store.path(&iter) {
                self.syncing.set(true);
                self.tree.expand_to_path(&p);
                self.tree.selection().select_path(&p);
                self.tree.scroll_to_cell(Some(&p), None::<&gtk::TreeViewColumn>, false, 0.0, 0.0);
                self.syncing.set(false);
            }
            return;
        }
        let Some(path) = self.store.path(&iter) else { return };
        let row = gtk::TreeRowReference::new(&self.store, &path);
        self.load_children(
            &iter,
            Some(Box::new(move |sb: &Sidebar| {
                let Some(iter) = row.and_then(|r| r.path()).and_then(|p| sb.store.iter(&p)) else { return };
                let mut child = sb.store.iter_children(Some(&iter));
                while let Some(c) = child {
                    let name: String = sb.store.value(&c, T_NAME as i32).get().unwrap_or_default();
                    if name == parts[i] {
                        sb.reveal_step(c, parts, i + 1);
                        return;
                    }
                    child = if sb.store.iter_next(&c) { Some(c) } else { None };
                }
                // A hidden folder while hidden files are off: show just this one, since the
                // user is in it.
                let parent: String = sb.store.value(&iter, T_PATH as i32).get().unwrap_or_default();
                let full = Path::new(&parent).join(&parts[i]);
                if let Ok(entry) = failbrauwser::fs::FileEntry::stat(&full) {
                    if entry.is_dir_like() {
                        let icon = util::icon_for_entry(&entry);
                        let c = sb.store.insert_with_values(Some(&iter), None, &[(T_ICON, &icon), (T_NAME, &parts[i]), (T_PATH, &full.to_string_lossy().to_string()), (T_LOADED, &false)]);
                        sb.add_placeholder(&c);
                        sb.reveal_step(c, parts, i + 1);
                        return;
                    }
                }
                if let Some(p) = sb.store.path(&iter) {
                    sb.syncing.set(true);
                    sb.tree.expand_to_path(&p);
                    sb.tree.selection().select_path(&p);
                    sb.syncing.set(false);
                }
            })),
        );
    }
}

struct Subfolder {
    name: String,
    entry: failbrauwser::fs::FileEntry,
    has_children: bool,
}

/// The subfolders of `dir`, sorted, each with whether it has subfolders itself.
/// Runs on a worker thread.
fn subfolders(dir: &Path, show_hidden: bool) -> Vec<Subfolder> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<Subfolder> = rd
        .filter_map(Result::ok)
        .filter(|e| show_hidden || !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| failbrauwser::fs::FileEntry::stat(&e.path()).ok())
        .filter(|e| e.is_dir_like())
        .map(|e| Subfolder { name: e.display_name(), has_children: has_subfolders(&e.path, show_hidden), entry: e })
        .collect();
    v.sort_by(|a, b| failbrauwser::fs::format::natural_cmp(&a.name, &b.name));
    v
}

/// Cheap check whether a folder has any subfolder (stops at the first).
fn has_subfolders(dir: &Path, show_hidden: bool) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else { return false };
    rd.filter_map(Result::ok).take(2000).any(|e| {
        (show_hidden || !e.file_name().to_string_lossy().starts_with('.'))
            && e.file_type().map(|t| t.is_dir()).unwrap_or(false)
    })
}
