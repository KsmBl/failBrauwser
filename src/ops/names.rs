//! Finding unused file names: "report (copy).pdf", "report (copy 2).pdf", "photo (2).jpg".

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::Path;

/// Splits "archive.tar.gz" into ("archive", ".tar.gz") and "a.txt" into ("a", ".txt").
/// Hidden files without a further dot keep their whole name as stem.
pub fn split_extension(name: &[u8]) -> (&[u8], &[u8]) {
    let lower = name.to_ascii_lowercase();
    if let Some(i) = find_subslice(&lower, b".tar.") {
        if i > 0 {
            return (&name[..i], &name[i..]);
        }
    }
    match name.iter().rposition(|&b| b == b'.') {
        Some(0) | None => (name, &[]),
        Some(i) => (&name[..i], &name[i..]),
    }
}

fn find_subslice(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

/// A name for `name` that does not exist in `dir`. With a label ("copy") the candidates are
/// "stem (copy).ext", "stem (copy 2).ext", …; without, "stem (2).ext", "stem (3).ext", ….
pub fn free_name(dir: &Path, name: &OsStr, label: Option<&str>) -> OsString {
    candidates(name, label).find(|c| std::fs::symlink_metadata(dir.join(c)).is_err()).expect("endless candidates")
}

/// Same as [`free_name`] but checks against a set of names instead of the disk.
pub fn free_name_in(existing: &dyn Fn(&OsStr) -> bool, name: &OsStr, label: Option<&str>) -> OsString {
    candidates(name, label).find(|c| !existing(c)).expect("endless candidates")
}

fn candidates<'a>(name: &'a OsStr, label: Option<&'a str>) -> impl Iterator<Item = OsString> + 'a {
    let (stem, ext) = split_extension(name.as_bytes());
    (1u64..).map(move |n| {
        let tag = match (label, n) {
            (Some(l), 1) => l.to_string(),
            (Some(l), n) => format!("{l} {n}"),
            (None, n) => (n + 1).to_string(),
        };
        let mut v = stem.to_vec();
        v.extend_from_slice(format!(" ({tag})").as_bytes());
        v.extend_from_slice(ext);
        OsString::from_vec(v)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions() {
        assert_eq!(split_extension(b"a.txt"), (&b"a"[..], &b".txt"[..]));
        assert_eq!(split_extension(b"x.TAR.gz"), (&b"x"[..], &b".TAR.gz"[..]));
        assert_eq!(split_extension(b".bashrc"), (&b".bashrc"[..], &b""[..]));
        assert_eq!(split_extension(b"noext"), (&b"noext"[..], &b""[..]));
        assert_eq!(split_extension(b".config.json"), (&b".config"[..], &b".json"[..]));
    }

    #[test]
    fn free_names_skip_existing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("r (copy).pdf"), "").unwrap();
        assert_eq!(free_name(dir.path(), OsStr::new("r.pdf"), Some("copy")), "r (copy 2).pdf");
        assert_eq!(free_name(dir.path(), OsStr::new("r.pdf"), None), "r (2).pdf");
        let taken = |n: &OsStr| n == "d (2)";
        assert_eq!(free_name_in(&taken, OsStr::new("d"), None), "d (3)");
    }
}
