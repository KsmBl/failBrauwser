//! Every format the archive helper can open, as a table generated from the helper itself
//! (`formats_table.rs`; regenerate with `FB_REGEN_FORMATS=1 cargo test --test archive_helper
//! formats_table`). Kept in the program so deciding what a file is never starts the helper.

/// What kind of container a format is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Holds files and usually folders: archives and file system images.
    Archive,
    /// A tar archive inside a single-file compressor (`.tar.gz`, …).
    Tar,
    /// A single-file compressor (`.gz`, `.xz`, …): one file inside.
    Stream,
    /// An encoding of one file (`.uue`, `.hqx`, …).
    Wrapper,
}

#[derive(Debug, Clone, Copy)]
pub struct Format {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: Kind,
    /// A new one can be written.
    pub create: bool,
    /// Entries can be added to and removed from an existing one directly.
    pub modify: bool,
    /// Can be encrypted with a password.
    pub password: bool,
    /// Keeps folders.
    pub dirs: bool,
    /// Holds more than one file.
    pub multi: bool,
    /// File name suffixes, lower case with the dot, longest (compound) first.
    pub exts: &'static [&'static str],
}

include!("formats_table.rs");

/// The format a file name suggests, by its longest matching suffix.
pub fn by_name(name: &str) -> Option<&'static Format> {
    let lower = name.to_lowercase();
    let mut best: Option<(&'static Format, usize)> = None;
    for f in FORMATS {
        for e in f.exts {
            if lower.len() > e.len() && lower.ends_with(e) && best.is_none_or(|(_, l)| e.len() > l) {
                best = Some((f, e.len()));
            }
        }
    }
    best.map(|(f, _)| f)
}

/// The format with this helper id (`Zip`, `SevenZip`, …).
pub fn by_id(id: &str) -> Option<&'static Format> {
    FORMATS.iter().find(|f| f.id == id)
}

/// The suffix new files of this format get.
pub fn default_ext(f: &Format) -> &'static str {
    f.exts.first().copied().unwrap_or("")
}
