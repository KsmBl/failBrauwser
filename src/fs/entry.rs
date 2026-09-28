//! A directory entry with everything the views need, gathered off the UI thread.

use std::ffi::OsString;
use std::fs::Metadata;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
    Symlink,
    /// Sockets, fifos, device nodes.
    Special,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileEntry {
    pub name: OsString,
    pub path: PathBuf,
    /// What the entry itself is (a symlink stays `Symlink`).
    pub kind: Kind,
    /// For symlinks: whether the target is a directory.
    pub target_is_dir: bool,
    /// For symlinks: the target does not exist.
    pub broken: bool,
    /// Size of the file (a symlink reports its target's size).
    pub size: u64,
    pub mtime: i64,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub dev: u64,
    pub ino: u64,
    pub nlink: u64,
    /// MIME type, guessed from the name only (no content sniffing, no extra I/O).
    pub content_type: String,
}

impl FileEntry {
    /// Builds an entry from `lstat` data, following a symlink once to learn what it points to.
    pub fn from_metadata(path: PathBuf, name: OsString, lmeta: &Metadata) -> Self {
        let ft = lmeta.file_type();
        let kind = if ft.is_symlink() {
            Kind::Symlink
        } else if ft.is_dir() {
            Kind::Dir
        } else if ft.is_file() {
            Kind::File
        } else {
            Kind::Special
        };
        let mut size = lmeta.len();
        let mut target_is_dir = false;
        let mut broken = false;
        if kind == Kind::Symlink {
            match std::fs::metadata(&path) {
                Ok(m) => {
                    target_is_dir = m.is_dir();
                    size = if m.is_file() { m.len() } else { 0 };
                }
                Err(_) => broken = true,
            }
        }
        if matches!(kind, Kind::Dir | Kind::Special) {
            size = 0;
        }
        let content_type = guess_content_type(&name, kind, target_is_dir);
        FileEntry {
            name,
            path,
            kind,
            target_is_dir,
            broken,
            size,
            mtime: lmeta.mtime(),
            mode: lmeta.mode(),
            uid: lmeta.uid(),
            gid: lmeta.gid(),
            dev: lmeta.dev(),
            ino: lmeta.ino(),
            nlink: lmeta.nlink(),
            content_type,
        }
    }

    pub fn stat(path: &Path) -> std::io::Result<Self> {
        let meta = std::fs::symlink_metadata(path)?;
        let name = path.file_name().map(|n| n.to_os_string()).unwrap_or_else(|| path.as_os_str().to_os_string());
        Ok(Self::from_metadata(path.to_path_buf(), name, &meta))
    }

    /// A directory, or a symlink to one: something you can open as a folder.
    pub fn is_dir_like(&self) -> bool {
        self.kind == Kind::Dir || (self.kind == Kind::Symlink && self.target_is_dir)
    }

    pub fn is_hidden(&self) -> bool {
        let b = self.name.as_bytes();
        b.first() == Some(&b'.') || b.last() == Some(&b'~')
    }

    pub fn display_name(&self) -> String {
        self.name.to_string_lossy().into_owned()
    }
}

/// MIME type from the file name, the way GIO does it but without touching the file.
pub fn guess_content_type(name: &std::ffi::OsStr, kind: Kind, target_is_dir: bool) -> String {
    if kind == Kind::Dir || (kind == Kind::Symlink && target_is_dir) {
        return "inode/directory".into();
    }
    if kind == Kind::Special {
        return "inode/special".into();
    }
    let (ct, _uncertain) = gtk::gio::content_type_guess(Some(Path::new(name)), None);
    ct.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn classifies_files_dirs_and_links() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        fs::create_dir(d.join("folder")).unwrap();
        fs::write(d.join("note.txt"), "hello").unwrap();
        std::os::unix::fs::symlink(d.join("folder"), d.join("link-dir")).unwrap();
        std::os::unix::fs::symlink(d.join("note.txt"), d.join("link-file")).unwrap();
        std::os::unix::fs::symlink(d.join("nowhere"), d.join("broken")).unwrap();

        let folder = FileEntry::stat(&d.join("folder")).unwrap();
        assert_eq!(folder.kind, Kind::Dir);
        assert_eq!(folder.content_type, "inode/directory");

        let note = FileEntry::stat(&d.join("note.txt")).unwrap();
        assert_eq!(note.kind, Kind::File);
        assert_eq!(note.size, 5);
        assert_eq!(note.content_type, "text/plain");

        let ld = FileEntry::stat(&d.join("link-dir")).unwrap();
        assert_eq!(ld.kind, Kind::Symlink);
        assert!(ld.is_dir_like());

        let lf = FileEntry::stat(&d.join("link-file")).unwrap();
        assert_eq!(lf.size, 5);
        assert!(!lf.is_dir_like());

        let b = FileEntry::stat(&d.join("broken")).unwrap();
        assert!(b.broken);
    }

    #[test]
    fn hidden_names() {
        let e = |n: &str| FileEntry {
            name: n.into(),
            path: n.into(),
            kind: Kind::File,
            target_is_dir: false,
            broken: false,
            size: 0,
            mtime: 0,
            mode: 0,
            uid: 0,
            gid: 0,
            dev: 0,
            ino: 0,
            nlink: 1,
            content_type: String::new(),
        };
        assert!(e(".bashrc").is_hidden());
        assert!(e("file~").is_hidden());
        assert!(!e("file").is_hidden());
    }
}
