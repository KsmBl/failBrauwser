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
    // SQLite databases are listed as tables, never written.
    let db = d.path().join("x.sqlite");
    let st = std::process::Command::new("python3")
        .arg("-c")
        .arg(format!("import sqlite3; c = sqlite3.connect({:?}); c.execute('create table t (a)'); c.commit()", db.display().to_string()))
        .status();
    if !st.is_ok_and(|s| s.success()) {
        eprintln!("python3 with sqlite3 missing, skipping");
        return;
    }
    let loc = ArchiveLoc::root(db);
    let a = v.archive(&loc).unwrap();
    assert!(!a.writable);
    let err = v.mkdir(&loc, "nope").unwrap_err();
    assert_eq!(err.code, "unsupported");
}

/// A compressed single file (.gz, .xz, …) shows the file inside, which can be edited and
/// saved back, and extracts to that file itself.
#[test]
fn compressed_files_are_edited_and_extracted() {
    let d = tempfile::tempdir().unwrap();
    let Some(v) = vfs(&d.path().join("cache")) else { return };
    fs::write(d.path().join("notes.txt"), "first version\n").unwrap();
    fs::write(d.path().join("edited"), "second version, longer than the first\n").unwrap();
    for ext in ["gz", "bz2", "xz", "zst", "br", "lz4", "lzma", "lz", "z", "uue"] {
        let file = d.path().join(format!("notes.txt.{ext}"));
        v.helper().add(&file, &[(d.path().join("notes.txt"), "notes.txt".into())], None).unwrap();
        let loc = ArchiveLoc::root(file.clone());
        let a = v.archive(&loc).unwrap();
        assert!(a.writable, "{ext} is writable");
        assert_eq!(names(&v, &loc, ""), ["notes.txt"], "{ext}");
        // Saving an edited copy recompresses the file.
        v.add(&loc, &[(d.path().join("edited"), "notes.txt".into())]).unwrap();
        let out = d.path().join(format!("out-{ext}"));
        v.extract(&loc, &["notes.txt".into()], &out).unwrap();
        assert_eq!(fs::read_to_string(out.join("notes.txt")).unwrap(), "second version, longer than the first\n", "{ext}");
        // Nothing else fits in.
        assert!(v.add(&loc, &[(d.path().join("edited"), "other.txt".into())]).is_err(), "{ext}");
        assert!(v.remove(&loc, &["notes.txt".into()]).is_err(), "{ext}");
        assert!(v.rename(&loc, "notes.txt", "x.txt").is_err(), "{ext}");
        assert!(v.mkdir(&loc, "dir").is_err(), "{ext}");
        // Extract Here gives the file, not a folder around it.
        let here = d.path().join(format!("here-{ext}"));
        fs::create_dir(&here).unwrap();
        assert_eq!(v.extract_all(&loc, &here).unwrap(), here.join("notes.txt"), "{ext}");
        assert!(here.join("notes.txt").is_file(), "{ext}");
    }
    // Other programs read what was written.
    if std::process::Command::new("gzip").arg("--version").output().is_ok() {
        let out = std::process::Command::new("gzip").arg("-dc").arg(d.path().join("notes.txt.gz")).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "second version, longer than the first\n");
    }
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
    let left: Vec<_> = fs::read_dir(&dest).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert!(left.is_empty(), "left behind: {left:?}");
    v.set_password(&loc, "pw");
    // A single file lands as itself, not in "f (2).txt".
    assert_eq!(v.extract_all(&loc, &dest).unwrap(), dest.join("f.txt"));
}
