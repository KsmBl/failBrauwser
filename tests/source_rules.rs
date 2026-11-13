//! Rules for the source code that tests of the running program cannot catch.

use std::path::Path;

fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// GTK's `iter_next(&iter)` / `iter_previous(&iter)` change an iterator Rust sees as borrowed
/// unchanged: an optimised build may keep reading the first row and loop forever (0.8.1 froze
/// entering folders so). Rows are stepped through with `util::child_iters` instead.
#[test]
fn no_tree_iterators_changed_in_place() {
    let mut files = Vec::new();
    rust_files(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    let mut found = Vec::new();
    for f in files {
        for (n, line) in std::fs::read_to_string(&f).unwrap().lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            if code.contains(".iter_next(") || code.contains(".iter_previous(") {
                found.push(format!("{}:{}", f.display(), n + 1));
            }
        }
    }
    assert!(found.is_empty(), "use util::child_iters instead of iter_next/iter_previous: {found:?}");
}
