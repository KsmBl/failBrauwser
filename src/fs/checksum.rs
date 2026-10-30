//! MD5 and SHA-256 of a file, read once for both, and comparing them with a checksum the
//! user copied (from a download page, a `sha256sum` line, …).

use md5::Md5;
use sha2::{Digest, Sha256};
use std::io::{self, Read};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checksums {
    pub md5: String,
    pub sha256: String,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Both checksums of `path`. `progress` gets the bytes read so far; a set `cancel` stops
/// with an `Interrupted` error.
pub fn file_checksums(path: &Path, cancel: &AtomicBool, mut progress: impl FnMut(u64)) -> io::Result<Checksums> {
    let mut f = std::fs::File::open(path)?;
    let (mut md5, mut sha) = (Md5::new(), Sha256::new());
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        md5.update(&buf[..n]);
        sha.update(&buf[..n]);
        done += n as u64;
        progress(done);
    }
    Ok(Checksums { md5: hex(&md5.finalize()), sha256: hex(&sha.finalize()) })
}

/// What a copied text says about `sums`: which checksum of the text matches (`Ok("SHA-256")`),
/// or `Err` with why not (no checksum in it, or one that differs).
pub fn compare(text: &str, sums: &Checksums) -> Result<&'static str, String> {
    // The hexadecimal words of the right lengths; "sha256:…" and "hash  name" lines too.
    let words: Vec<String> = text
        .split(|c: char| !c.is_ascii_hexdigit())
        .filter(|w| w.len() == 32 || w.len() == 64)
        .map(str::to_ascii_lowercase)
        .collect();
    if words.is_empty() {
        return Err("The clipboard holds no MD5 or SHA-256 checksum.".into());
    }
    for w in &words {
        if *w == sums.sha256 {
            return Ok("SHA-256");
        }
        if *w == sums.md5 {
            return Ok("MD5");
        }
    }
    let kind = if words[0].len() == 64 { "SHA-256" } else { "MD5" };
    Err(format!("The {kind} checksum in the clipboard does not match this file."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_checksums_and_progress() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("abc");
        std::fs::write(&p, "abc").unwrap();
        let mut seen = 0;
        let s = file_checksums(&p, &AtomicBool::new(false), |n| seen = n).unwrap();
        assert_eq!(s.md5, "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(s.sha256, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(seen, 3);
        // Bigger than one read, and cancelled.
        let big = dir.path().join("big");
        std::fs::write(&big, vec![0u8; 3 << 20]).unwrap();
        assert_eq!(file_checksums(&big, &AtomicBool::new(false), |_| {}).unwrap().md5.len(), 32);
        let err = file_checksums(&big, &AtomicBool::new(true), |_| {}).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Interrupted);
    }

    #[test]
    fn copied_checksums() {
        let s = Checksums { md5: "900150983cd24fb0d6963f7d28e17f72".into(), sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into() };
        assert_eq!(compare("BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD", &s), Ok("SHA-256"));
        assert_eq!(compare("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  abc.iso\n", &s), Ok("SHA-256"));
        assert_eq!(compare("md5: 900150983cd24fb0d6963f7d28e17f72", &s), Ok("MD5"));
        assert_eq!(compare(" sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad ", &s), Ok("SHA-256"));
        assert!(compare("0000000000000000000000000000000000000000000000000000000000000000", &s).unwrap_err().contains("SHA-256 checksum in the clipboard does not match"));
        assert!(compare("hello world", &s).unwrap_err().contains("no MD5 or SHA-256"));
    }
}
