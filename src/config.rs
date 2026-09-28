//! User settings, stored as a GLib key file in `$XDG_CONFIG_HOME/failbrauwser/settings.ini`.

use gtk::glib;
use std::path::{Path, PathBuf};

/// What the folder tree in the sidebar starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeRoot {
    /// The folder the tab shows.
    Current,
    Home,
    Filesystem,
}

impl TreeRoot {
    pub fn as_str(self) -> &'static str {
        match self {
            TreeRoot::Current => "current",
            TreeRoot::Home => "home",
            TreeRoot::Filesystem => "root",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "current" => Some(TreeRoot::Current),
            "home" => Some(TreeRoot::Home),
            "root" => Some(TreeRoot::Filesystem),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    List,
    Icons,
}

/// Columns of the detailed list, in display order. `Name` is always shown.
pub const COLUMN_IDS: &[&str] = &["name", "size", "dirsize", "type", "modified", "permissions", "owner", "group"];

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub show_hidden: bool,
    pub folders_first: bool,
    pub view_mode: ViewMode,
    /// Visible column ids from [`COLUMN_IDS`].
    pub columns: Vec<String>,
    pub sort_column: String,
    pub sort_descending: bool,
    pub tree_root: TreeRoot,
    pub show_tree: bool,
    /// Keep running after the last window closes so the next window opens instantly.
    pub daemon: bool,
    pub window_width: i32,
    pub window_height: i32,
    pub sidebar_width: i32,
    /// Position of the divider between shortcuts and folder tree.
    pub sidebar_split: i32,
    /// Throttle writes to removable and rotational drives so the page cache never fills up.
    pub smooth_writes: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            show_hidden: false,
            folders_first: true,
            view_mode: ViewMode::List,
            columns: ["name", "size", "type", "modified"].iter().map(|s| s.to_string()).collect(),
            sort_column: "name".into(),
            sort_descending: false,
            tree_root: TreeRoot::Home,
            show_tree: true,
            daemon: true,
            window_width: 1000,
            window_height: 650,
            sidebar_width: 220,
            sidebar_split: 260,
            smooth_writes: true,
        }
    }
}

const GROUP: &str = "General";

impl Settings {
    pub fn default_path() -> PathBuf {
        glib::user_config_dir().join("failbrauwser").join("settings.ini")
    }

    /// Loads settings; anything missing or malformed keeps its default.
    pub fn load_from(path: &Path) -> Settings {
        let mut s = Settings::default();
        let kf = glib::KeyFile::new();
        if kf.load_from_file(path, glib::KeyFileFlags::NONE).is_err() {
            return s;
        }
        let b = |key: &str, v: &mut bool| {
            if let Ok(x) = kf.boolean(GROUP, key) {
                *v = x;
            }
        };
        let i = |key: &str, v: &mut i32, min: i32| {
            if let Ok(x) = kf.integer(GROUP, key) {
                *v = x.max(min);
            }
        };
        b("show_hidden", &mut s.show_hidden);
        b("folders_first", &mut s.folders_first);
        b("sort_descending", &mut s.sort_descending);
        b("show_tree", &mut s.show_tree);
        b("daemon", &mut s.daemon);
        b("smooth_writes", &mut s.smooth_writes);
        i("window_width", &mut s.window_width, 300);
        i("window_height", &mut s.window_height, 200);
        i("sidebar_width", &mut s.sidebar_width, 0);
        i("sidebar_split", &mut s.sidebar_split, 0);
        if let Ok(v) = kf.string(GROUP, "view_mode") {
            s.view_mode = if v == "icons" { ViewMode::Icons } else { ViewMode::List };
        }
        if let Ok(v) = kf.string(GROUP, "tree_root") {
            if let Some(t) = TreeRoot::parse(&v) {
                s.tree_root = t;
            }
        }
        if let Ok(v) = kf.string(GROUP, "sort_column") {
            if COLUMN_IDS.contains(&v.as_str()) {
                s.sort_column = v.to_string();
            }
        }
        if let Ok(list) = kf.string_list(GROUP, "columns") {
            let mut cols: Vec<String> = list
                .iter()
                .map(|c| c.to_string())
                .filter(|c| COLUMN_IDS.contains(&c.as_str()))
                .collect();
            if !cols.iter().any(|c| c == "name") {
                cols.insert(0, "name".into());
            }
            cols.dedup();
            s.columns = cols;
        }
        s
    }

    pub fn load() -> Settings {
        Self::load_from(&Self::default_path())
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let kf = glib::KeyFile::new();
        kf.set_boolean(GROUP, "show_hidden", self.show_hidden);
        kf.set_boolean(GROUP, "folders_first", self.folders_first);
        kf.set_string(GROUP, "view_mode", if self.view_mode == ViewMode::Icons { "icons" } else { "list" });
        // Key file list syntax: items separated and terminated by ';'.
        kf.set_string(GROUP, "columns", &format!("{};", self.columns.join(";")));
        kf.set_string(GROUP, "sort_column", &self.sort_column);
        kf.set_boolean(GROUP, "sort_descending", self.sort_descending);
        kf.set_string(GROUP, "tree_root", self.tree_root.as_str());
        kf.set_boolean(GROUP, "show_tree", self.show_tree);
        kf.set_boolean(GROUP, "daemon", self.daemon);
        kf.set_boolean(GROUP, "smooth_writes", self.smooth_writes);
        kf.set_integer(GROUP, "window_width", self.window_width);
        kf.set_integer(GROUP, "window_height", self.window_height);
        kf.set_integer(GROUP, "sidebar_width", self.sidebar_width);
        kf.set_integer(GROUP, "sidebar_split", self.sidebar_split);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        kf.save_to_file(path).map_err(std::io::Error::other)
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&Self::default_path())
    }

    pub fn column_visible(&self, id: &str) -> bool {
        self.columns.iter().any(|c| c == id)
    }

    /// Shows or hides a column, keeping the canonical order. `name` cannot be hidden.
    pub fn set_column_visible(&mut self, id: &str, visible: bool) {
        if id == "name" || !COLUMN_IDS.contains(&id) {
            return;
        }
        if visible {
            if !self.column_visible(id) {
                self.columns.push(id.to_string());
                self.columns.sort_by_key(|c| COLUMN_IDS.iter().position(|x| x == c));
            }
        } else {
            self.columns.retain(|c| c != id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/settings.ini");
        let mut s = Settings::default();
        s.show_hidden = true;
        s.view_mode = ViewMode::Icons;
        s.tree_root = TreeRoot::Current;
        s.set_column_visible("dirsize", true);
        s.set_column_visible("type", false);
        s.window_width = 1234;
        s.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), s);
    }

    #[test]
    fn missing_or_broken_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Settings::load_from(&dir.path().join("none.ini")), Settings::default());
        let bad = dir.path().join("bad.ini");
        std::fs::write(&bad, "[General]\ncolumns=bogus;size\ntree_root=sideways\nwindow_width=-5\n").unwrap();
        let s = Settings::load_from(&bad);
        assert_eq!(s.columns, vec!["name", "size"]);
        assert_eq!(s.tree_root, TreeRoot::Home);
        assert_eq!(s.window_width, 300);
    }

    #[test]
    fn column_order_is_canonical_and_name_stays() {
        let mut s = Settings::default();
        s.set_column_visible("owner", true);
        s.set_column_visible("dirsize", true);
        s.set_column_visible("name", false);
        assert_eq!(s.columns, vec!["name", "size", "dirsize", "type", "modified", "owner"]);
    }
}
