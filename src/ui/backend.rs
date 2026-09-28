//! Reading locations. Runs on worker threads; returns plain data.

use super::model::Item;
use failbrauwser::fs::list_dir;
use failbrauwser::location::Location;
use std::path::PathBuf;

/// Reads everything at a location (hidden entries included).
pub fn read_location(loc: &Location) -> Result<Vec<Item>, String> {
    match loc {
        Location::Dir(p) => list_dir(p)
            .map(|v| v.into_iter().map(Item::Fs).collect())
            .map_err(|e| format!("Cannot open “{}”: {}", p.display(), e)),
        Location::Archive(_) => Err("Archives cannot be opened yet.".into()),
        Location::Drives => Ok(Vec::new()),
    }
}

/// The directory whose changes should trigger a re-read of the location.
pub fn watch_path(loc: &Location) -> Option<PathBuf> {
    match loc {
        Location::Dir(p) => Some(p.clone()),
        _ => None,
    }
}

/// If activating this file should open it as a folder (an archive), the location to go to.
pub fn enter_as_folder(_current: &Location, _item: &Item) -> Option<Location> {
    None
}
