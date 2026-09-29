//! End-to-end tests against the real `fb-archive` helper. They are skipped (with a notice)
//! when the helper has not been built; `make helper` builds it.

use failbrauwser::archive::{ArchiveTree, Helper, client::locate_helper};
use std::fs;
use std::path::Path;
use std::process::Command;

fn helper() -> Option<Helper> {
    let exe = locate_helper();
    if !exe.is_file() {
        eprintln!("fb-archive helper not built, skipping ({})", exe.display());
        return None;
    }
    Some(Helper::new(exe))
}

fn sample_tree(root: &Path) {
    fs::create_dir_all(root.join("sub/deeper")).unwrap();
    fs::write(root.join("a.txt"), "root file").unwrap();
    fs::write(root.join("sub/a.txt"), "nested same name").unwrap();
    fs::write(root.join("sub/deeper/c.bin"), vec![7u8; 10_000]).unwrap();
}

fn names(h: &Helper, archive: &Path) -> Vec<String> {
    let mut n: Vec<String> = h.list(archive, None).unwrap().entries.into_iter().map(|e| e.name).collect();
    n.sort();
    n
}

#[test]
fn create_list_and_extract_roundtrip() {
    let Some(h) = helper() else { return };
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    sample_tree(&src);
    let archive = dir.path().join("new.zip");
    h.add(&archive, &[(src.clone(), "src".into())], None).unwrap();

    let listing = h.list(&archive, None).unwrap();
    assert_eq!(listing.format, "Zip");
    assert!(listing.writable);
    let tree = ArchiveTree::build(&listing.entries);
    assert!(tree.is_dir("src/sub/deeper"));
    assert_eq!(tree.get("src/sub/deeper/c.bin").unwrap().size, 10_000);

    let out = dir.path().join("out");
    h.extract(&archive, Some(&["src/sub".to_string()]), &out, None).unwrap();
    assert_eq!(fs::read_to_string(out.join("src/sub/a.txt")).unwrap(), "nested same name");
    assert_eq!(fs::read(out.join("src/sub/deeper/c.bin")).unwrap().len(), 10_000);
}

#[test]
fn remove_is_exact_and_folders_go_completely() {
    let Some(h) = helper() else { return };
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    sample_tree(&src);
    let archive = dir.path().join("t.tar.gz");
    let st = Command::new("tar").arg("czf").arg(&archive).arg("-C").arg(&src).arg("a.txt").arg("sub").status().unwrap();
    assert!(st.success());

    h.remove(&archive, &["a.txt".into()], None).unwrap();
    assert_eq!(names(&h, &archive), vec!["sub", "sub/a.txt", "sub/deeper", "sub/deeper/c.bin"]);

    h.remove(&archive, &["sub/deeper".into()], None).unwrap();
    assert_eq!(names(&h, &archive), vec!["sub", "sub/a.txt"]);
}

#[test]
fn mkdir_rename_and_replace() {
    let Some(h) = helper() else { return };
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    sample_tree(&src);
    let archive = dir.path().join("t.7z");
    h.add(&archive, &[(src.join("a.txt"), "a.txt".into()), (src.join("sub"), "sub".into())], None).unwrap();

    h.mkdir(&archive, "empty", None).unwrap();
    assert!(names(&h, &archive).contains(&"empty".to_string()));

    h.rename(&archive, "sub", "moved/inner", None).unwrap();
    let n = names(&h, &archive);
    assert!(n.contains(&"moved/inner/deeper/c.bin".to_string()), "{n:?}");
    assert!(!n.iter().any(|x| x.starts_with("sub")), "{n:?}");

    // Adding over an existing name replaces it instead of duplicating it.
    let newer = dir.path().join("a2.txt");
    fs::write(&newer, "replaced").unwrap();
    h.add(&archive, &[(newer, "a.txt".into())], None).unwrap();
    let listing = h.list(&archive, None).unwrap();
    let a: Vec<_> = listing.entries.iter().filter(|e| e.name == "a.txt").collect();
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].size, 8);
}

#[test]
fn errors_are_reported_not_fatal() {
    let Some(h) = helper() else { return };
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.zip");
    let err = h.list(&missing, None).unwrap_err();
    assert_eq!(err.code, "notfound");
    // The helper survives a failed request.
    let bogus = dir.path().join("plain.txt");
    fs::write(&bogus, "not an archive").unwrap();
    let (is_archive, _, _) = h.probe(&bogus).unwrap();
    assert!(!is_archive);
    assert!(h.is_running());
    h.shutdown();
    assert!(!h.is_running());
}

#[test]
fn long_operations_report_progress() {
    let Some(h) = helper() else { return };
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("many");
    std::fs::create_dir_all(&src).unwrap();
    // Random content compresses slowly enough for a few progress samples.
    let mut seed: u64 = 42;
    for i in 0..96 {
        let data: Vec<u8> = (0..256 * 1024)
            .map(|_| {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                (seed >> 33) as u8
            })
            .collect();
        fs::write(src.join(format!("f{i:03}.bin")), data).unwrap();
    }
    let archive = dir.path().join("big.tar.gz");
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let s2 = seen.clone();
    failbrauwser::archive::client::with_progress(move |p| s2.borrow_mut().push(p.clone()), || {
        h.add(&archive, &[(src.clone(), "many".into())], None).unwrap();
    });
    let seen = seen.borrow();
    assert!(!seen.is_empty(), "no progress reported");
    for p in seen.iter() {
        assert!(p.total >= 24 << 20, "{p:?}");
        assert!(p.done <= p.total, "{p:?}");
        assert!(p.phase == "adding" || p.phase == "writing", "{p:?}");
    }
}

#[test]
fn encrypted_archives_stay_encrypted_when_edited() {
    let Some(h) = helper() else { return };
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a.txt"), dir.path().join("b.txt"));
    fs::write(&a, "secret a").unwrap();
    fs::write(&b, "secret b").unwrap();
    let zip = dir.path().join("e.zip");
    h.add(&zip, &[(a.clone(), "a.txt".into())], Some("pw")).unwrap();
    // Adding with the password must not add a plain entry.
    h.add(&zip, &[(b.clone(), "sub/b.txt".into())], Some("pw")).unwrap();
    let l = h.list(&zip, None).unwrap();
    assert!(l.entries.iter().filter(|e| !e.is_dir).all(|e| e.encrypted), "{:?}", l.entries);
    assert_eq!(l.entries.iter().filter(|e| !e.is_dir).count(), 2);
    h.remove(&zip, &["a.txt".into()], Some("pw")).unwrap();
    assert!(h.list(&zip, None).unwrap().entries.iter().filter(|e| !e.is_dir).all(|e| e.encrypted));
    // Wrong or missing passwords are reported as such.
    let out = dir.path().join("out");
    assert!(h.extract(&zip, None, &out, None).unwrap_err().needs_password());
    assert!(h.extract(&zip, None, &out, Some("nope")).unwrap_err().needs_password());
    h.extract(&zip, None, &out, Some("pw")).unwrap();
    assert_eq!(fs::read_to_string(out.join("sub/b.txt")).unwrap(), "secret b");
    let sz = dir.path().join("e.7z");
    h.add(&sz, &[(a, "a.txt".into())], Some("pw")).unwrap();
    assert!(h.extract(&sz, None, &dir.path().join("o7"), None).unwrap_err().needs_password());
}
