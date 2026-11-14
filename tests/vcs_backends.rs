//! One behaviour, every tool: a working copy each tool's own client made answers the questions
//! the TortoiseGit windows ask — states for the overlays, changes to commit, history, blame, a
//! file's text before and after — the same way, whatever tool owns it. A tool whose client is not
//! installed says so and is skipped; the README records which ones are measured.

use failbrauwser::vcs::backend::{self, Backend};
use failbrauwser::vcs::state::Status;
use failbrauwser::vcs::{self as vcs, Tool};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

fn have(program: &str) -> bool {
    Command::new(program).arg("--version").output().is_ok_and(|o| o.status.success())
}

/// Runs a client the way a fixture needs it: hermetic, in English, failing loudly.
fn sh(dir: &Path, program: &str, args: &[&str]) -> String {
    let out = Command::new(program)
        .current_dir(dir)
        .args(args)
        .env("LC_ALL", "C")
        .env("HGPLAIN", "1")
        .env("HGRCPATH", "")
        .env("HGUSER", "fixture <fixture@example>")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("CVS_RSH", "false")
        .output()
        .unwrap_or_else(|e| panic!("{program} could not start: {e}"));
    assert!(out.status.success(), "{program} {args:?} in {}: {}{}", dir.display(), String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A fixture: a working copy of `tool` holding `one.txt` committed once ("first") and changed
/// in a second commit ("second"), plus what the tool's client needs to commit again.
struct Fixture {
    _keep: tempfile::TempDir,
    wc: PathBuf,
    tool: Tool,
}

impl Fixture {
    fn make(tool: Tool) -> Option<Fixture> {
        let needs: &[&str] = match tool {
            Tool::Git => &["git"],
            Tool::Mercurial => &["hg"],
            Tool::Subversion => &["svn", "svnadmin"],
            Tool::Cvs => &["cvs"],
            Tool::VisualSourceSafe => return None,
        };
        if let Some(missing) = needs.iter().find(|p| !have(p)) {
            eprintln!("{missing} is not installed, skipping {}", tool.label());
            return None;
        }
        let keep = tempfile::tempdir().unwrap();
        let base = keep.path().to_path_buf();
        let wc = base.join("wc");
        match tool {
            Tool::Git => {
                std::fs::create_dir_all(&wc).unwrap();
                sh(&wc, "git", &["init", "-q", "-b", "main"]);
                for (k, v) in [("user.name", "fixture"), ("user.email", "fixture@example"), ("commit.gpgsign", "false"), ("core.hooksPath", "")] {
                    sh(&wc, "git", &["config", k, v]);
                }
            }
            Tool::Mercurial => {
                sh(&base, "hg", &["init", "wc"]);
                std::fs::write(wc.join(".hg/hgrc"), "[ui]\nusername = fixture <fixture@example>\n").unwrap();
            }
            Tool::Subversion => {
                sh(&base, "svnadmin", &["create", "repo"]);
                let url = format!("file://{}/repo", base.display());
                sh(&base, "svn", &["mkdir", "-q", "-m", "layout", &format!("{url}/trunk"), &format!("{url}/branches"), &format!("{url}/tags")]);
                sh(&base, "svn", &["checkout", "-q", &format!("{url}/trunk"), "wc"]);
            }
            Tool::Cvs => {
                let root = base.join("cvsroot");
                sh(&base, "cvs", &["-d", root.to_str().unwrap(), "init"]);
                let import = base.join("import");
                std::fs::create_dir_all(&import).unwrap();
                sh(&import, "cvs", &["-d", root.to_str().unwrap(), "import", "-m", "start", "mod", "vendor", "start"]);
                sh(&base, "cvs", &["-d", root.to_str().unwrap(), "checkout", "-d", "wc", "mod"]);
            }
            Tool::VisualSourceSafe => unreachable!(),
        }
        let f = Fixture { _keep: keep, wc, tool };
        f.write("one.txt", "alpha\nbeta\n");
        f.add("one.txt");
        f.commit("first");
        // A tool that stamps files by second could not tell the two versions apart.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        f.write("one.txt", "alpha\nbeta changed\ngamma\n");
        f.commit("second");
        Some(f)
    }

    fn write(&self, name: &str, text: &str) {
        let p = self.wc.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn client(&self, args: &[&str]) -> String {
        sh(&self.wc, self.tool.cli(), args)
    }

    fn add(&self, name: &str) {
        self.client(&["add", name]);
    }

    fn commit(&self, message: &str) {
        match self.tool {
            Tool::Git => self.client(&["commit", "-q", "-a", "-m", message]),
            _ => self.client(&["commit", "-m", message]),
        };
        if self.tool == Tool::Subversion {
            // svn log and blame read the revision the working copy was updated to.
            self.client(&["update", "-q"]);
        }
    }

    fn backend(&self) -> Arc<dyn Backend> {
        let wc = vcs::find(&self.wc).expect("the fixture is a working copy");
        assert_eq!(wc.tool, self.tool);
        backend::open(&wc).unwrap_or_else(|| panic!("{} has no backend", self.tool.label()))
    }
}

/// Runs `check` against every tool whose client is installed.
fn each_tool(check: impl Fn(&Fixture, &dyn Backend)) {
    for tool in Tool::all() {
        let Some(f) = Fixture::make(*tool) else { continue };
        let b = f.backend();
        assert_eq!(b.tool(), *tool);
        check(&f, b.as_ref());
    }
}

fn state(b: &dyn Backend, dir: &Path, name: &str) -> Option<Status> {
    b.folder_status(dir).unwrap_or_else(|e| panic!("{}: {e}", b.tool().label())).get(&OsString::from(name)).copied()
}

#[test]
fn a_committed_file_is_normal_and_a_changed_one_modified() {
    each_tool(|f, b| {
        assert_eq!(state(b, &f.wc, "one.txt"), Some(Status::Normal), "{}", b.tool().label());
        std::thread::sleep(std::time::Duration::from_millis(1100));
        f.write("one.txt", "alpha\nbeta changed\ngamma\ndelta\n");
        assert_eq!(state(b, &f.wc, "one.txt"), Some(Status::Modified), "{}", b.tool().label());
        assert_eq!(b.summary().unwrap(), Status::Modified, "{}", b.tool().label());
    });
}

#[test]
fn a_new_file_is_unversioned_until_added() {
    each_tool(|f, b| {
        f.write("new.txt", "fresh\n");
        assert_eq!(state(b, &f.wc, "new.txt"), Some(Status::Unversioned), "{}", b.tool().label());
        f.add("new.txt");
        assert_eq!(state(b, &f.wc, "new.txt"), Some(Status::Added), "{}", b.tool().label());
        let changes = b.changes(&[], true, false).unwrap();
        let new = changes.iter().find(|c| c.path == "new.txt").unwrap_or_else(|| panic!("{}: {changes:?}", b.tool().label()));
        assert_eq!(new.status, Status::Added, "{}", b.tool().label());
    });
}

#[test]
fn a_folder_shows_the_most_important_state_inside() {
    each_tool(|f, b| {
        f.write("sub/inner.txt", "inner\n");
        if f.tool == Tool::Cvs {
            f.add("sub");
        }
        f.add(if f.tool == Tool::Cvs { "sub/inner.txt" } else { "sub" });
        assert_eq!(state(b, &f.wc, "sub"), Some(Status::Modified), "a folder holding an added file reads as changed ({})", b.tool().label());
        assert_eq!(state(b, &f.wc.join("sub"), "inner.txt"), Some(Status::Added), "{}", b.tool().label());
    });
}

#[test]
fn unversioned_changes_are_listed_only_when_asked() {
    each_tool(|f, b| {
        f.write("loose.txt", "x\n");
        assert!(b.changes(&[], false, false).unwrap().iter().all(|c| c.path != "loose.txt"), "{}", b.tool().label());
        assert!(b.changes(&[], true, false).unwrap().iter().any(|c| c.path == "loose.txt" && c.status == Status::Unversioned), "{}", b.tool().label());
    });
}

#[test]
fn the_history_is_newest_first_with_its_messages() {
    each_tool(|_, b| {
        let log = b.log(&[], 100).unwrap();
        let subjects: Vec<&str> = log.iter().map(|c| c.subject()).filter(|s| *s == "first" || *s == "second").collect();
        assert_eq!(subjects, ["second", "first"], "{}: {log:?}", b.tool().label());
        let second = log.iter().find(|c| c.subject() == "second").unwrap();
        assert!(second.author.contains("fixture") || !second.author.is_empty(), "{}", b.tool().label());
        assert!(second.time > 0, "{}: a revision has a time", b.tool().label());
        let changed = b.revision_changes(&second.hash).unwrap();
        assert!(changed.iter().any(|c| c.path.ends_with("one.txt") && c.status == Status::Modified), "{}: {changed:?}", b.tool().label());
    });
}

#[test]
fn the_history_of_one_file_names_its_two_versions() {
    each_tool(|_, b| {
        let (latest, before) = b.last_two_versions("one.txt").unwrap_or_else(|| panic!("{}", b.tool().label()));
        assert_eq!(String::from_utf8_lossy(&b.text_at(&latest, "one.txt")), "alpha\nbeta changed\ngamma\n", "{}", b.tool().label());
        assert_eq!(String::from_utf8_lossy(&b.text_at(&before, "one.txt")), "alpha\nbeta\n", "{}", b.tool().label());
        assert_eq!(b.parent(&latest).as_deref().map(|p| b.text_at(p, "one.txt")), Some(b"alpha\nbeta\n".to_vec()), "{}", b.tool().label());
    });
}

#[test]
fn the_working_file_is_compared_with_its_last_commit() {
    each_tool(|f, b| {
        f.write("one.txt", "edited\n");
        assert_eq!(String::from_utf8_lossy(&b.base_text("one.txt")), "alpha\nbeta changed\ngamma\n", "{}", b.tool().label());
        assert!(b.base_text("never-there.txt").is_empty(), "{}", b.tool().label());
    });
}

#[test]
fn blame_names_the_revision_of_each_line() {
    each_tool(|_, b| {
        let lines = b.blame("one.txt").unwrap();
        let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["alpha", "beta changed", "gamma"], "{}", b.tool().label());
        assert_eq!(lines.iter().map(|l| l.line).collect::<Vec<_>>(), [1, 2, 3], "{}", b.tool().label());
        assert_ne!(lines[0].hash, lines[1].hash, "{}: alpha is older than its neighbour", b.tool().label());
        assert_eq!(lines[1].hash, lines[2].hash, "{}", b.tool().label());
        assert!(lines.iter().all(|l| !l.author.is_empty()), "{}", b.tool().label());
    });
}

#[test]
fn a_clean_working_copy_has_no_conflicts() {
    each_tool(|_, b| {
        assert!(!b.has_conflicts(), "{}", b.tool().label());
        assert_eq!(b.summary().unwrap(), Status::Normal, "{}", b.tool().label());
    });
}

#[test]
fn each_tool_names_where_its_working_copy_is() {
    each_tool(|_, b| {
        let want = match b.tool() {
            Tool::Git => "main",
            Tool::Mercurial => "default",
            Tool::Subversion => "^/trunk",
            Tool::Cvs => "HEAD",
            Tool::VisualSourceSafe => unreachable!(),
        };
        assert_eq!(b.branch_label(), want, "{}", b.tool().label());
    });
}
