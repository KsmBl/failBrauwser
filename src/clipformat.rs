//! The clipboard format file managers share (`x-special/gnome-copied-files`, used by
//! Nautilus, Thunar, Nemo, Caja, PCManFM): an operation line, then one URI per line.

use std::path::PathBuf;

pub const COPIED_FILES: &str = "x-special/gnome-copied-files";

pub fn format_copied_files(paths: &[PathBuf], cut: bool) -> String {
    let mut s = String::from(if cut { "cut" } else { "copy" });
    for p in paths {
        if let Ok(uri) = gtk::glib::filename_to_uri(p, None) {
            s.push('\n');
            s.push_str(&uri);
        }
    }
    s
}

/// Parses the shared format; `None` when the text is something else.
pub fn parse_copied_files(text: &str) -> Option<(bool, Vec<PathBuf>)> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let cut = match lines.next()? {
        "cut" => true,
        "copy" => false,
        _ => return None,
    };
    let paths = lines.filter_map(uri_to_path).collect();
    Some((cut, paths))
}

/// Accepts `file://` URIs and absolute paths (plain text pasted from a terminal).
pub fn uri_to_path(s: &str) -> Option<PathBuf> {
    let s = s.trim();
    if s.starts_with("file://") {
        gtk::glib::filename_from_uri(s).ok().map(|(p, _)| p)
    } else if s.starts_with('/') {
        Some(PathBuf::from(s))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_with_special_characters() {
        let paths = vec![PathBuf::from("/tmp/a b"), PathBuf::from("/tmp/ä#%")];
        let text = format_copied_files(&paths, true);
        assert!(text.starts_with("cut\nfile:///tmp/a%20b"));
        assert_eq!(parse_copied_files(&text), Some((true, paths)));
    }

    #[test]
    fn rejects_foreign_text() {
        assert_eq!(parse_copied_files("hello\nworld"), None);
        assert_eq!(parse_copied_files(""), None);
        assert_eq!(parse_copied_files("copy\nhttps://example.org/x\n/abs/path"), Some((false, vec![PathBuf::from("/abs/path")])));
    }
}
