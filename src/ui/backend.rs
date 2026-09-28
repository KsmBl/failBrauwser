//! Reading locations. Runs on worker threads; returns plain data.

use super::model::Item;
use failbrauwser::archive::is_browsable_name;
use failbrauwser::archive::vfs::Vfs;
use failbrauwser::fs::list_dir;
use failbrauwser::location::Location;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct LoadError {
    pub message: String,
    /// The archive is encrypted: ask for its password and read again.
    pub needs_password: bool,
}

impl From<String> for LoadError {
    fn from(message: String) -> Self {
        LoadError { message, needs_password: false }
    }
}

/// Reads everything at a location (hidden entries included).
pub fn read_location(loc: &Location) -> Result<Vec<Item>, LoadError> {
    match loc {
        Location::Dir(p) => list_dir(p)
            .map(|v| v.into_iter().map(Item::Fs).collect())
            .map_err(|e| format!("Cannot open “{}”: {}", p.display(), e).into()),
        Location::Archive(a) => {
            let archive = Vfs::global().archive(a).map_err(|e| LoadError {
                needs_password: e.needs_password(),
                message: format!("Cannot open “{}”: {}", Location::Archive(a.clone()).title(), e),
            })?;
            match archive.tree.children(&a.inner) {
                Some(children) => Ok(children.into_iter().cloned().map(Item::Archive).collect()),
                None => Err(format!("“{}” does not exist in this archive.", a.inner).into()),
            }
        }
        Location::Drives => Ok(Vec::new()),
    }
}

/// The file or directory whose changes should trigger a re-read of the location.
pub fn watch_path(loc: &Location) -> Option<PathBuf> {
    match loc {
        Location::Dir(p) => Some(p.clone()),
        // Any change to the archive file (ours or another program's) re-reads it.
        Location::Archive(a) => Some(a.file.clone()),
        Location::Drives => None,
    }
}

/// If activating this file should open it as a folder (an archive), the location to go to.
pub fn enter_as_folder(current: &Location, item: &Item) -> Option<Location> {
    if item.is_dir_like() || !is_browsable_name(&item.display_name()) {
        return None;
    }
    match (current, item) {
        (_, Item::Fs(e)) => Some(Location::Archive(failbrauwser::location::ArchiveLoc::root(e.path.clone()))),
        (Location::Archive(a), Item::Archive(n)) => Some(Location::Archive(a.enter_nested(&n.path))),
        _ => None,
    }
}
