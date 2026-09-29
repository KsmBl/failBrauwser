//! Runs the scripted UI tests in tests/ui/*.fbt inside a headless sway session.
//! Skipped (with a notice) when sway, grim or dbus-run-session are not installed.

use std::path::Path;
use std::process::Command;

fn have(tool: &str) -> bool {
    Command::new("sh").arg("-c").arg(format!("command -v {tool}")).output().is_ok_and(|o| o.status.success())
}

fn run(script: &str, setup: impl FnOnce(&Path)) {
    if !["sway", "grim", "dbus-run-session"].iter().all(|t| have(t)) {
        eprintln!("headless UI tools missing, skipping {script}");
        return;
    }
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    setup(dir.path());
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = Command::new(root.join("tests/ui/harness.sh"))
        .arg(env!("CARGO_BIN_EXE_failbrauwser"))
        .arg(root.join("tests/ui").join(script))
        .arg(dir.path())
        .output()
        .unwrap();
    let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success() && log.contains("SELFTEST OK"), "{script} failed:\n{log}");
}

#[test]
fn file_operations() {
    run("file_ops.fbt", |t| {
        std::fs::create_dir_all(t.join("files/sub")).unwrap();
        std::fs::create_dir_all(t.join("dest")).unwrap();
        std::fs::write(t.join("files/a.txt"), "a").unwrap();
        std::fs::write(t.join("files/b.txt"), "b").unwrap();
    });
}

#[test]
fn sidebar_tree() {
    run("sidebar.fbt", |t| {
        std::fs::create_dir_all(t.join("a/b/c/d")).unwrap();
        std::fs::create_dir_all(t.join("a/other/inside")).unwrap();
    });
}

#[test]
fn total_size_column() {
    run("dirsize.fbt", |t| {
        let root = t.join("root");
        std::fs::create_dir_all(root.join("photos/2024/summer")).unwrap();
        std::fs::write(root.join("photos/a.jpg"), vec![0u8; 1500]).unwrap();
        std::fs::write(root.join("photos/2024/summer/b.jpg"), vec![0u8; 2000]).unwrap();
        std::fs::create_dir_all(root.join("with-link")).unwrap();
        std::fs::write(root.join("with-link/small"), vec![0u8; 10]).unwrap();
        std::fs::write(t.join("big"), vec![0u8; 100_000]).unwrap();
        // A link pointing at a big file outside: its target must not be counted.
        std::os::unix::fs::symlink(t.join("big"), root.join("with-link/big-link")).unwrap();
        std::fs::write(root.join("note.txt"), "hello").unwrap();
    });
}

#[test]
fn drives_page() {
    run("drives.fbt", |t| {
        std::fs::create_dir(t.join("d")).unwrap();
        std::fs::write(t.join("d/file"), "x").unwrap();
    });
}

#[test]
fn archives_as_folders() {
    if !failbrauwser::archive::client::locate_helper().is_file() {
        eprintln!("fb-archive helper not built, skipping");
        return;
    }
    run("archives.fbt", |t| {
        let w = t.join("w");
        std::fs::create_dir_all(w.join("out")).unwrap();
        std::fs::write(w.join("new.txt"), "new file").unwrap();
        let script = format!(
            r#"
import io, tarfile, zipfile
buf = io.BytesIO()
with tarfile.open(fileobj=buf, mode="w:gz") as t:
    data = b"deep content"
    info = tarfile.TarInfo("deep/file.txt"); info.size = len(data)
    t.addfile(info, io.BytesIO(data))
with zipfile.ZipFile("{w}/test.zip", "w", zipfile.ZIP_DEFLATED) as z:
    z.writestr("a.txt", "alpha")
    z.writestr("sub/b.txt", "beta")
    z.writestr("inner.tar.gz", buf.getvalue())
"#,
            w = w.display()
        );
        let ok = std::process::Command::new("python3").arg("-c").arg(script).status().unwrap();
        assert!(ok.success());
    });
}

#[test]
fn iso_images() {
    let have_xorriso = Command::new("sh").arg("-c").arg("command -v xorriso").output().is_ok_and(|o| o.status.success());
    if !have_xorriso || !failbrauwser::archive::client::locate_helper().is_file() {
        eprintln!("xorriso or archive helper missing, skipping");
        return;
    }
    run("iso.fbt", |t| {
        let w = t.join("w");
        std::fs::create_dir_all(w.join("src/docs")).unwrap();
        std::fs::create_dir_all(w.join("out")).unwrap();
        std::fs::write(w.join("src/readme.txt"), "hello").unwrap();
        std::fs::write(w.join("src/docs/a.txt"), "inside the disc").unwrap();
        std::fs::write(w.join("new.txt"), "added").unwrap();
        let ok = Command::new("xorriso")
            .args(["-as", "mkisofs", "-R", "-J", "-V", "DISC", "-o"])
            .arg(w.join("disc.iso"))
            .arg(w.join("src"))
            .output()
            .unwrap();
        assert!(ok.status.success());
        std::fs::remove_dir_all(w.join("src")).unwrap();
    });
}

#[test]
fn path_bar() {
    run("pathbar.fbt", |t| {
        std::fs::create_dir_all(t.join("a/b/c/a-rather-long-folder-name/another-long-folder-name/and-the-last-one")).unwrap();
        std::fs::create_dir_all(t.join("x")).unwrap();
    });
}
