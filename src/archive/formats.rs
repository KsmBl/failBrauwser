//! Every format the archive helper can open, as a table generated from the helper itself
//! (`formats_table.rs`; regenerate with `FB_REGEN_FORMATS=1 cargo test --test archive_helper
//! formats_table`). Kept in the program so deciding what a file is never starts the helper.

/// What kind of container a format is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Holds files and usually folders.
    Archive,
    /// A disk or file system image (FAT, ext4, ISO, VHD, …).
    Filesystem,
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

/// The formats offered first when creating an archive, in this order.
pub const COMMON: &[&str] = &["Zip", "SevenZip", "TarGz", "TarXz", "TarZst", "Tar"];

/// The formats a new archive can be written in, beyond [`COMMON`], grouped for a menu
/// and sorted by name. Compressed files and encodings hold exactly one file, so they are
/// only offered for `single_file`.
pub fn create_groups(single_file: bool) -> Vec<(&'static str, Vec<&'static Format>)> {
    let pick = |kinds: &[Kind]| {
        let mut v: Vec<&'static Format> = FORMATS.iter().filter(|f| f.create && kinds.contains(&f.kind) && !COMMON.contains(&f.id)).collect();
        v.sort_by_key(|f| f.name.to_lowercase());
        v
    };
    let mut groups = Vec::new();
    if single_file {
        groups.push(("Compressed File", pick(&[Kind::Stream])));
    }
    groups.push(("Archives", pick(&[Kind::Archive, Kind::Tar])));
    groups.push(("Disk Images", pick(&[Kind::Filesystem])));
    if single_file {
        groups.push(("Encodings", pick(&[Kind::Wrapper])));
    }
    groups.retain(|(_, v)| !v.is_empty());
    groups
}

/// The file name for a new archive called `name`: the format's suffix is added unless the
/// name already ends with one of its suffixes.
pub fn file_name_for(name: &str, f: &Format) -> String {
    let lower = name.to_lowercase();
    if f.exts.iter().any(|e| lower.len() > e.len() && lower.ends_with(e)) { name.to_string() } else { format!("{name}{}", default_ext(f)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_formats_exist_and_can_be_created() {
        for id in COMMON {
            assert!(by_id(id).is_some_and(|f| f.create), "{id}");
        }
    }

    #[test]
    fn every_creatable_format_is_offered_once() {
        let offered: Vec<&str> = create_groups(true).into_iter().flat_map(|(_, v)| v).map(|f| f.id).chain(COMMON.iter().copied()).collect();
        let mut sorted = offered.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), offered.len(), "no format twice");
        let creatable = FORMATS.iter().filter(|f| f.create).count();
        assert_eq!(offered.len(), creatable);
    }

    #[test]
    fn single_file_formats_only_for_one_file() {
        let many = create_groups(false);
        assert!(many.iter().all(|(_, v)| v.iter().all(|f| !matches!(f.kind, Kind::Stream | Kind::Wrapper))));
        assert!(create_groups(true).iter().any(|(g, _)| *g == "Compressed File"));
    }

    #[test]
    fn file_names_get_the_suffix_once() {
        let gz = by_id("Gzip").unwrap();
        assert_eq!(file_name_for("notes.txt", gz), "notes.txt.gz");
        assert_eq!(file_name_for("notes.txt.gz", gz), "notes.txt.gz");
        assert_eq!(file_name_for("photos", by_id("TarGz").unwrap()), "photos.tar.gz");
    }
}
