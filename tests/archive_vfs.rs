//! Archive folders through the VFS, with the real helper: nested archives and write-back.

use failbrauwser::archive::client::locate_helper;
use failbrauwser::archive::vfs::Vfs;
use failbrauwser::archive::Helper;
use failbrauwser::location::ArchiveLoc;
use std::fs;
use std::path::Path;

fn vfs(cache: &Path) -> Option<Vfs> {
    let exe = locate_helper();
    if !exe.is_file() {
        eprintln!("fb-archive helper not built, skipping");
        return None;
    }
    Some(Vfs::new(Helper::new(exe), cache.to_path_buf()))
}

fn names(v: &Vfs, loc: &ArchiveLoc, dir: &str) -> Vec<String> {
    let a = v.archive(loc).unwrap();
    a.tree.children(dir).unwrap_or_default().iter().map(|n| n.name.clone()).collect()
}

/// outer.zip containing docs/inner.tar.gz containing notes/a.txt
fn nested_fixture(d: &Path, v: &Vfs) -> ArchiveLoc {
    fs::create_dir_all(d.join("src/notes")).unwrap();
    fs::write(d.join("src/notes/a.txt"), "inner text").unwrap();
    let inner = d.join("inner.tar.gz");
    v.helper().add(&inner, &[(d.join("src/notes"), "notes".into())], None).unwrap();
    let outer = d.join("outer.zip");
    fs::write(d.join("readme"), "top").unwrap();
    v.helper().add(&outer, &[(inner, "docs/inner.tar.gz".into()), (d.join("readme"), "readme".into())], None).unwrap();
    ArchiveLoc::root(outer)
}

#[test]
fn browses_into_nested_archives() {
    let d = tempfile::tempdir().unwrap();
    let Some(v) = vfs(&d.path().join("cache")) else { return };
    let outer = nested_fixture(d.path(), &v);
    assert_eq!(names(&v, &outer, ""), vec!["docs", "readme"]);
    let inner = outer.enter_nested("docs/inner.tar.gz");
    assert_eq!(names(&v, &inner, ""), vec!["notes"]);
    assert_eq!(names(&v, &inner, "notes"), vec!["a.txt"]);
    let out = d.path().join("out");
    let got = v.extract(&inner, &["notes/a.txt".into()], &out).unwrap();
    assert_eq!(fs::read_to_string(&got[0]).unwrap(), "inner text");
}

#[test]
fn edits_in_nested_archive_reach_the_outer_file() {
    let d = tempfile::tempdir().unwrap();
    let Some(v) = vfs(&d.path().join("cache")) else { return };
    let outer = nested_fixture(d.path(), &v);
    let inner = outer.enter_nested("docs/inner.tar.gz");
    fs::write(d.path().join("new.txt"), "added deep").unwrap();
    v.add(&inner, &[(d.path().join("new.txt"), "notes/new.txt".into())]).unwrap();
    v.mkdir(&inner, "fresh").unwrap();
    v.rename(&inner, "notes/a.txt", "notes/b.txt").unwrap();

    // A new VFS with an empty cache sees the change through the outer zip.
    let v2 = vfs(&d.path().join("cache2")).unwrap();
    assert_eq!(names(&v2, &inner, ""), vec!["fresh", "notes"]);
    assert_eq!(names(&v2, &inner, "notes"), vec!["b.txt", "new.txt"]);
    assert_eq!(names(&v2, &outer, ""), vec!["docs", "readme"]);

    v.remove(&inner, &["notes".into()]).unwrap();
    let v3 = vfs(&d.path().join("cache3")).unwrap();
    assert_eq!(names(&v3, &inner, ""), vec!["fresh"]);
}

#[test]
fn listing_is_cached_until_the_file_changes() {
    let d = tempfile::tempdir().unwrap();
    let Some(v) = vfs(&d.path().join("cache")) else { return };
    let outer = nested_fixture(d.path(), &v);
    let a1 = v.archive(&outer).unwrap();
    let a2 = v.archive(&outer).unwrap();
    assert!(std::sync::Arc::ptr_eq(&a1, &a2));
    assert!(v.cached(&outer).is_some());
    fs::write(d.path().join("more"), "m").unwrap();
    v.add(&outer, &[(d.path().join("more"), "more".into())]).unwrap();
    let a3 = v.archive(&outer).unwrap();
    assert!(!std::sync::Arc::ptr_eq(&a1, &a3));
    assert_eq!(names(&v, &outer, ""), vec!["docs", "more", "readme"]);
}

#[test]
fn extract_all_makes_one_folder() {
    let d = tempfile::tempdir().unwrap();
    let Some(v) = vfs(&d.path().join("cache")) else { return };
    let outer = nested_fixture(d.path(), &v);
    let dest = d.path().join("dest");
    fs::create_dir(&dest).unwrap();
    // Two top-level entries: into a folder named after the archive.
    let made = v.extract_all(&outer, &dest).unwrap();
    assert_eq!(made, dest.join("outer"));
    assert_eq!(fs::read_to_string(dest.join("outer/readme")).unwrap(), "top");
    // Again: the name is taken, a free one is used.
    let made2 = v.extract_all(&outer, &dest).unwrap();
    assert_eq!(made2, dest.join("outer (2)"));
    // A single top folder is extracted as it is.
    let inner = outer.enter_nested("docs/inner.tar.gz");
    assert_eq!(v.extract_all(&inner, &dest).unwrap(), dest.join("notes"));
}

#[test]
fn read_only_formats_refuse_edits() {
    let d = tempfile::tempdir().unwrap();
    let Some(v) = vfs(&d.path().join("cache")) else { return };
    let st = std::process::Command::new("sh").arg("-c").arg(format!("echo hi | gzip > {}", d.path().join("x.gz").display())).status().unwrap();
    assert!(st.success());
    let loc = ArchiveLoc::root(d.path().join("x.gz"));
    let a = v.archive(&loc).unwrap();
    assert!(!a.writable);
    let err = v.mkdir(&loc, "nope").unwrap_err();
    assert_eq!(err.code, "unsupported");
}

#[test]
fn failed_extraction_leaves_nothing_behind() {
    let d = tempfile::tempdir().unwrap();
    let Some(v) = vfs(&d.path().join("cache")) else { return };
    fs::write(d.path().join("f.txt"), "x").unwrap();
    let zip = d.path().join("locked.zip");
    v.helper().add(&zip, &[(d.path().join("f.txt"), "f.txt".into())], Some("pw")).unwrap();
    let dest = d.path().join("dest");
    fs::create_dir(&dest).unwrap();
    let loc = ArchiveLoc::root(zip);
    assert!(v.extract_all(&loc, &dest).unwrap_err().needs_password());
    assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
    v.set_password(&loc, "pw");
    assert_eq!(v.extract_all(&loc, &dest).unwrap(), dest.join("locked"));
}
