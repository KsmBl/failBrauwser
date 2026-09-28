//! The local filesystem: entries, listings, formatting.

pub mod dirsize;
pub mod entry;
pub mod format;
pub mod listing;

pub use entry::{FileEntry, Kind};
pub use listing::{Diff, diff, list_dir};
