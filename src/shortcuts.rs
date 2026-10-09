//! The entries of the side panel: built-in places (Drives, Home, Desktop, Trash, mounted
//! volumes) and folders the user added, in the user's order, each one hideable and
//! renameable. Stored in `$XDG_CONFIG_HOME/failbrauwser/shortcuts`, one entry per line.
//!
//! On the first start the folders bookmarked in GTK (`~/.config/gtk-3.0/bookmarks`, shared
//! with other file managers) are taken over.

use gtk::glib;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Drives,
    Home,
    Desktop,
    Trash,
    /// Mounted volumes (USB drives, network shares): as many rows as there are mounts.
    Mounts,
    Dir(PathBuf),
}

impl Kind {
    pub fn default_name(&self) -> String {
        match self {
            Kind::Drives => "Drives".into(),
            Kind::Home => "Home".into(),
            Kind::Desktop => "Desktop".into(),
            Kind::Trash => "Trash".into(),
            Kind::Mounts => "Mounted volumes".into(),
            Kind::Dir(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string_lossy().into_owned()),
        }
    }

    /// Built-in entries can be hidden but not removed.
    pub fn is_builtin(&self) -> bool {
        !matches!(self, Kind::Dir(_))
    }

    fn key(&self) -> String {
        match self {
            Kind::Drives => "drives".into(),
            Kind::Home => "home".into(),
            Kind::Desktop => "desktop".into(),
            Kind::Trash => "trash".into(),
            Kind::Mounts => "mounts".into(),
            Kind::Dir(p) => glib::filename_to_uri(p, None).map(|u| u.to_string()).unwrap_or_default(),
        }
    }

    fn from_key(key: &str) -> Option<Kind> {
        Some(match key {
            "drives" => Kind::Drives,
            "home" => Kind::Home,
            "desktop" => Kind::Desktop,
            "trash" => Kind::Trash,
            "mounts" => Kind::Mounts,
            uri => Kind::Dir(glib::filename_from_uri(uri).ok()?.0),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    pub kind: Kind,
    /// A name the user gave it; `None` shows the default name.
    pub name: Option<String>,
    pub hidden: bool,
}

impl Shortcut {
    pub fn new(kind: Kind) -> Self {
        Shortcut { kind, name: None, hidden: false }
    }

    pub fn label(&self) -> String {
        self.name.clone().unwrap_or_else(|| self.kind.default_name())
    }
}

/// The panel's entries, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcuts(pub Vec<Shortcut>);

impl Shortcuts {
    pub fn default_path() -> PathBuf {
        glib::user_config_dir().join("failbrauwser").join("shortcuts")
    }

    /// What a first start shows: the built-in places with the GTK bookmarks after Desktop.
    pub fn defaults(bookmarks: Vec<(PathBuf, Option<String>)>) -> Self {
        let mut v = vec![Shortcut::new(Kind::Drives), Shortcut::new(Kind::Home), Shortcut::new(Kind::Desktop)];
        for (p, name) in bookmarks {
            if !v.iter().any(|s| s.kind == Kind::Dir(p.clone())) {
                v.push(Shortcut { kind: Kind::Dir(p), name, hidden: false });
            }
        }
        v.push(Shortcut::new(Kind::Trash));
        v.push(Shortcut::new(Kind::Mounts));
        Shortcuts(v)
    }

    /// Reads the saved entries; without a file, the defaults (taking over GTK's bookmarks).
    /// Built-in entries missing from an older file are added at the end.
    pub fn load_from(path: &Path, gtk_bookmarks: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::defaults(read_gtk_bookmarks(gtk_bookmarks));
        };
        let mut v: Vec<Shortcut> = Vec::new();
        for line in text.lines() {
            // hidden <TAB> kind <TAB> name
            let mut f = line.splitn(3, '\t');
            let (Some(hidden), Some(key)) = (f.next(), f.next()) else { continue };
            let Some(kind) = Kind::from_key(key) else { continue };
            if v.iter().any(|s| s.kind == kind) {
                continue;
            }
            let name = f.next().filter(|n| !n.is_empty()).map(str::to_string);
            v.push(Shortcut { kind, name, hidden: hidden == "1" });
        }
        for k in [Kind::Drives, Kind::Home, Kind::Desktop, Kind::Trash, Kind::Mounts] {
            if !v.iter().any(|s| s.kind == k) {
                v.push(Shortcut::new(k));
            }
        }
        Shortcuts(v)
    }

    pub fn load() -> Self {
        Self::load_from(&Self::default_path(), &glib::user_config_dir().join("gtk-3.0").join("bookmarks"))
    }

    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        let mut out = String::new();
        for s in &self.0 {
            // Names are one line without tabs.
            let name = s.name.as_deref().unwrap_or("").replace(['\t', '\n', '\r'], " ");
            out.push_str(&format!("{}\t{}\t{}\n", if s.hidden { 1 } else { 0 }, s.kind.key(), name));
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, out)
    }

    pub fn save(&self) -> io::Result<()> {
        self.save_to(&Self::default_path())
    }

    /// Adds a folder before `at` (at the end for `None`). A folder already in the list is
    /// moved there and shown again. Returns its index.
    pub fn add_dir(&mut self, dir: PathBuf, at: Option<usize>) -> usize {
        let kind = Kind::Dir(dir);
        let mut entry = Shortcut::new(kind.clone());
        let mut at = at.unwrap_or(self.0.len()).min(self.0.len());
        if let Some(i) = self.0.iter().position(|s| s.kind == kind) {
            entry = self.0.remove(i);
            entry.hidden = false;
            if i < at {
                at -= 1;
            }
        }
        self.0.insert(at, entry);
        at
    }

    /// Moves entry `from` so that it ends up at index `to`.
    pub fn move_to(&mut self, from: usize, to: usize) {
        if from >= self.0.len() {
            return;
        }
        let s = self.0.remove(from);
        let to = to.min(self.0.len());
        self.0.insert(to, s);
    }

    /// Removes a folder the user added; built-in entries stay (they can be hidden).
    pub fn remove(&mut self, i: usize) -> bool {
        if self.0.get(i).is_some_and(|s| !s.kind.is_builtin()) {
            self.0.remove(i);
            true
        } else {
            false
        }
    }

    /// Gives an entry a name; empty or the default name goes back to the default.
    pub fn rename(&mut self, i: usize, name: &str) {
        if let Some(s) = self.0.get_mut(i) {
            let name = name.trim();
            s.name = (!name.is_empty() && name != s.kind.default_name()).then(|| name.to_string());
        }
    }

    pub fn set_hidden(&mut self, i: usize, hidden: bool) {
        if let Some(s) = self.0.get_mut(i) {
            s.hidden = hidden;
        }
    }
}

/// `file://` lines of a GTK bookmarks file, with their optional names. Network and other
/// non-local bookmarks are left out.
pub fn read_gtk_bookmarks(path: &Path) -> Vec<(PathBuf, Option<String>)> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    text.lines()
        .filter_map(|l| {
            let (uri, name) = match l.split_once(' ') {
                Some((u, n)) => (u, Some(n.trim().to_string()).filter(|n| !n.is_empty())),
                None => (l.trim(), None),
            };
            let p = glib::filename_from_uri(uri).ok()?.0;
            Some((p, name))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_start_takes_over_gtk_bookmarks() {
        let dir = tempfile::tempdir().unwrap();
        let bm = dir.path().join("bookmarks");
        std::fs::write(&bm, "file:///home/me/Games Spiele\nfile:///home/me/My%20Docs\nsmb://nas/share NAS\n").unwrap();
        let s = Shortcuts::load_from(&dir.path().join("none"), &bm);
        let labels: Vec<String> = s.0.iter().map(Shortcut::label).collect();
        assert_eq!(labels, ["Drives", "Home", "Desktop", "Spiele", "My Docs", "Trash", "Mounted volumes"]);
        assert_eq!(s.0[4].kind, Kind::Dir("/home/me/My Docs".into()));
    }

    #[test]
    fn saves_and_loads_order_names_and_hidden() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/shortcuts");
        let mut s = Shortcuts::defaults(Vec::new());
        let g = s.add_dir("/data/Games\twith tab".into(), Some(1));
        assert_eq!(g, 1);
        s.rename(g, "Spiele\tx");
        s.set_hidden(2, true); // Home
        s.move_to(0, 3); // Drives after Desktop
        s.save_to(&path).unwrap();
        let back = Shortcuts::load_from(&path, Path::new("/nonexistent"));
        assert_eq!(back, Shortcuts::load_from(&path, Path::new("/nonexistent")));
        let labels: Vec<(String, bool)> = back.0.iter().map(|s| (s.label(), s.hidden)).collect();
        assert_eq!(
            labels,
            [
                ("Spiele x".into(), false),
                ("Home".into(), true),
                ("Desktop".into(), false),
                ("Drives".into(), false),
                ("Trash".into(), false),
                ("Mounted volumes".into(), false)
            ]
        );
        assert_eq!(back.0[0].kind, Kind::Dir("/data/Games\twith tab".into()));
    }

    #[test]
    fn editing_rules() {
        let mut s = Shortcuts::defaults(Vec::new());
        assert!(!s.remove(0), "built-in entries are only hidden");
        let i = s.add_dir("/x".into(), None);
        assert_eq!(s.add_dir("/y".into(), Some(0)), 0);
        // Adding a folder that is there moves it and shows it again.
        s.set_hidden(i + 1, true);
        assert_eq!(s.add_dir("/x".into(), Some(1)), 1);
        assert_eq!(s.0[1].kind, Kind::Dir("/x".into()));
        assert!(!s.0[1].hidden);
        assert_eq!(s.0.iter().filter(|e| e.kind == Kind::Dir("/x".into())).count(), 1);
        s.rename(1, "Home base");
        assert_eq!(s.0[1].label(), "Home base");
        s.rename(1, "  ");
        assert_eq!(s.0[1].name, None);
        assert!(s.remove(1));
        assert!(!s.0.iter().any(|e| e.kind == Kind::Dir("/x".into())));
    }

    #[test]
    fn older_files_get_new_builtins_and_skip_junk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shortcuts");
        std::fs::write(&path, "0\thome\t\n1\ttrash\tBin\ngarbage\n0\tfile:///srv\t\n0\thome\t\n").unwrap();
        let s = Shortcuts::load_from(&path, Path::new("/nonexistent"));
        let keys: Vec<String> = s.0.iter().map(|e| e.label()).collect();
        assert_eq!(keys, ["Home", "Bin", "srv", "Drives", "Desktop", "Mounted volumes"]);
        assert!(s.0[1].hidden);
    }
}
