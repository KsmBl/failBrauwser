//! Reading directories and comparing two readings of the same directory.

use super::entry::FileEntry;
use std::collections::HashMap;
use std::ffi::OsString;
use std::io;
use std::path::Path;

/// Reads a directory. Entries that vanish while reading are skipped silently.
pub fn list_dir(dir: &Path) -> io::Result<Vec<FileEntry>> {
    let rd = std::fs::read_dir(dir)?;
    let mut out = Vec::new();
    for item in rd {
        let Ok(item) = item else { continue };
        let path = item.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
        out.push(FileEntry::from_metadata(path, item.file_name(), &meta));
    }
    Ok(out)
}

/// How a directory changed between two readings.
#[derive(Debug, Default, PartialEq)]
pub struct Diff {
    pub added: Vec<FileEntry>,
    pub removed: Vec<OsString>,
    pub changed: Vec<FileEntry>,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }
}

/// Compares entries by name; an entry counts as changed when anything shown about it differs.
pub fn diff(old: &[FileEntry], new: &[FileEntry]) -> Diff {
    let old_by_name: HashMap<&OsString, &FileEntry> = old.iter().map(|e| (&e.name, e)).collect();
    let new_names: std::collections::HashSet<&OsString> = new.iter().map(|e| &e.name).collect();
    let mut d = Diff::default();
    for e in new {
        match old_by_name.get(&e.name) {
            None => d.added.push(e.clone()),
            Some(o) if **o != *e => d.changed.push(e.clone()),
            _ => {}
        }
    }
    for e in old {
        if !new_names.contains(&e.name) {
            d.removed.push(e.name.clone());
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn lists_all_entries_including_hidden() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a"), "1").unwrap();
        fs::write(dir.path().join(".h"), "1").unwrap();
        fs::create_dir(dir.path().join("d")).unwrap();
        let mut names: Vec<_> = list_dir(dir.path()).unwrap().into_iter().map(|e| e.display_name()).collect();
        names.sort();
        assert_eq!(names, vec![".h", "a", "d"]);
    }

    #[test]
    fn listing_missing_dir_fails() {
        assert!(list_dir(Path::new("/definitely/not/here")).is_err());
    }

    #[test]
    fn diff_detects_add_remove_change() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        fs::write(p.join("keep"), "1").unwrap();
        fs::write(p.join("gone"), "1").unwrap();
        fs::write(p.join("grow"), "1").unwrap();
        let before = list_dir(p).unwrap();
        fs::remove_file(p.join("gone")).unwrap();
        fs::write(p.join("grow"), "12345").unwrap();
        fs::write(p.join("new"), "1").unwrap();
        let after = list_dir(p).unwrap();
        let d = diff(&before, &after);
        assert_eq!(d.added.len(), 1);
        assert_eq!(d.added[0].display_name(), "new");
        assert_eq!(d.removed, vec![OsString::from("gone")]);
        assert_eq!(d.changed.len(), 1);
        assert_eq!(d.changed[0].display_name(), "grow");
        assert!(diff(&after, &after).is_empty());
    }
}
