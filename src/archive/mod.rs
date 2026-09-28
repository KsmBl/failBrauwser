//! Archives as directories: typed requests to the helper and the directory tree of a listing.

pub mod client;
pub mod tree;
pub mod vfs;

pub use client::{ArchiveError, Helper, Result};
pub use tree::{ArchiveNode, ArchiveTree};

use serde_json::{Value, json};
use std::path::Path;

/// One entry as the helper lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct ArchiveEntry {
    /// The name exactly as stored; needed nowhere but kept for diagnostics.
    pub raw: String,
    /// Normalized: relative, forward slashes, no trailing slash.
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub compressed: u64,
    pub mtime: Option<i64>,
    pub encrypted: bool,
    pub method: String,
}

#[derive(Debug, Clone)]
pub struct Listing {
    pub format: String,
    pub writable: bool,
    pub entries: Vec<ArchiveEntry>,
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn with_password(mut req: Value, password: Option<&str>) -> Value {
    if let Some(pw) = password {
        req["password"] = json!(pw);
    }
    req
}

fn parse_listing(v: &Value) -> Listing {
    let entries = v["entries"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|e| ArchiveEntry {
                    raw: e["raw"].as_str().unwrap_or_default().to_string(),
                    name: e["name"].as_str().unwrap_or_default().to_string(),
                    is_dir: e["dir"].as_bool().unwrap_or(false),
                    size: e["size"].as_i64().unwrap_or(0).max(0) as u64,
                    compressed: e["csize"].as_i64().unwrap_or(0).max(0) as u64,
                    mtime: e["mtime"].as_i64(),
                    encrypted: e["enc"].as_bool().unwrap_or(false),
                    method: e["method"].as_str().unwrap_or_default().to_string(),
                })
                .filter(|e| !e.name.is_empty())
                .collect()
        })
        .unwrap_or_default();
    Listing {
        format: v["format"].as_str().unwrap_or("Unknown").to_string(),
        writable: v["writable"].as_bool().unwrap_or(false),
        entries,
    }
}

impl Helper {
    pub fn list(&self, archive: &Path, password: Option<&str>) -> Result<Listing> {
        let v = self.request(with_password(json!({"cmd": "list", "archive": path_str(archive)}), password))?;
        Ok(parse_listing(&v))
    }

    /// Is this file something the helper can open as a folder?
    pub fn probe(&self, path: &Path) -> Result<(bool, String, bool)> {
        let v = self.request(json!({"cmd": "probe", "path": path_str(path)}))?;
        Ok((
            v["archive"].as_bool().unwrap_or(false),
            v["format"].as_str().unwrap_or_default().to_string(),
            v["writable"].as_bool().unwrap_or(false),
        ))
    }

    /// Extracts the given entries (or everything for `None`) below `dest`, keeping their
    /// archive-relative paths. A folder brings its whole subtree.
    pub fn extract(&self, archive: &Path, entries: Option<&[String]>, dest: &Path, password: Option<&str>) -> Result<()> {
        let mut req = json!({"cmd": "extract", "archive": path_str(archive), "dest": path_str(dest)});
        if let Some(e) = entries {
            req["entries"] = json!(e);
        }
        self.request(with_password(req, password)).map(|_| ())
    }

    /// Adds files or folders from disk under the given entry names. A missing archive is created.
    pub fn add(&self, archive: &Path, items: &[(std::path::PathBuf, String)], password: Option<&str>) -> Result<()> {
        let items: Vec<Value> = items.iter().map(|(src, name)| json!({"src": path_str(src), "name": name})).collect();
        self.request(with_password(json!({"cmd": "add", "archive": path_str(archive), "items": items}), password)).map(|_| ())
    }

    pub fn remove(&self, archive: &Path, names: &[String], password: Option<&str>) -> Result<()> {
        self.request(with_password(json!({"cmd": "remove", "archive": path_str(archive), "names": names}), password)).map(|_| ())
    }

    pub fn rename(&self, archive: &Path, from: &str, to: &str, password: Option<&str>) -> Result<()> {
        self.request(with_password(json!({"cmd": "rename", "archive": path_str(archive), "from": from, "to": to}), password)).map(|_| ())
    }

    pub fn mkdir(&self, archive: &Path, name: &str, password: Option<&str>) -> Result<()> {
        self.request(with_password(json!({"cmd": "mkdir", "archive": path_str(archive), "name": name}), password)).map(|_| ())
    }
}

/// Archive types that open as a folder on double click. Anything else (office documents,
/// jars, disk images, …) opens with its application; "Open as Archive" still probes it.
const BROWSABLE_SUFFIXES: &[&str] = &[
    ".tar.gz", ".tar.bz2", ".tar.xz", ".tar.zst", ".tar.lz", ".tar.lzma", ".tar.lz4", ".tar.z", ".tgz", ".tbz",
    ".tbz2", ".txz", ".tzst", ".tlz", ".zip", ".7z", ".rar", ".tar", ".cpio", ".ar", ".deb", ".rpm", ".cab", ".lzh",
    ".lha", ".arj", ".cbz", ".cbr", ".cb7", ".cbt", ".xar", ".wim", ".zpaq", ".ace", ".zoo", ".arc", ".sit", ".sitx",
    ".gz", ".bz2", ".xz", ".zst", ".lz", ".lzma", ".lz4",
];

/// True when a file with this name opens as a folder on activation.
pub fn is_browsable_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    BROWSABLE_SUFFIXES.iter().any(|s| lower.len() > s.len() && lower.ends_with(s))
}

/// File name suffixes a new archive can be created with from the UI.
pub const CREATE_FORMATS: &[(&str, &str)] = &[
    ("ZIP", ".zip"),
    ("7-Zip", ".7z"),
    ("Tar + Gzip", ".tar.gz"),
    ("Tar + XZ", ".tar.xz"),
    ("Tar + Zstandard", ".tar.zst"),
    ("Tar", ".tar"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browsable_names() {
        assert!(is_browsable_name("a.ZIP"));
        assert!(is_browsable_name("x.tar.gz"));
        assert!(!is_browsable_name(".zip"));
        assert!(!is_browsable_name("doc.odt"));
        assert!(!is_browsable_name("zip"));
    }

    #[test]
    fn listing_is_parsed_and_empty_names_dropped() {
        let v = json!({"format": "Zip", "writable": true, "entries": [
            {"raw": "a/", "name": "a", "dir": true, "size": 0, "csize": 0, "enc": false, "method": "Store"},
            {"raw": "/", "name": "", "dir": true, "size": 0, "csize": 0, "enc": false, "method": ""},
            {"raw": "a/b", "name": "a/b", "dir": false, "size": 5, "csize": 3, "enc": true, "method": "Deflate", "mtime": 7}
        ]});
        let l = parse_listing(&v);
        assert_eq!(l.format, "Zip");
        assert!(l.writable);
        assert_eq!(l.entries.len(), 2);
        assert_eq!(l.entries[1].mtime, Some(7));
        assert!(l.entries[1].encrypted);
    }
}
