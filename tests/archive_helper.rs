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
