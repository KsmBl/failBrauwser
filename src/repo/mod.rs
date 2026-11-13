//! Version-control working copies: which tool owns a folder, and the client its commands run
//! (M15). A folder is a working copy because of a marker the tool itself left there; nothing
//! here reads a repository's own data, so a working copy is never understood behind its tool's
//! back. The Repository menu (M15) is the only caller that turns this into actions.

pub mod verbs;

use std::path::{Path, PathBuf};

/// The tools whose working copies this program recognises (M15).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Git,
    Mercurial,
    Subversion,
    Cvs,
    VisualSourceSafe,
}

impl Tool {
    /// The command-line client this tool's own commands run (M15).
    pub fn cli(&self) -> &'static str {
        match self {
            Tool::Git => "git",
            Tool::Mercurial => "hg",
            Tool::Subversion => "svn",
            Tool::Cvs => "cvs",
            Tool::VisualSourceSafe => "ss",
        }
    }
}

/// A working copy: the tool that owns it, and the folder its marker sits in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkingCopy {
    pub tool: Tool,
    pub root: PathBuf,
}

/// The tool owning this folder itself, by whatever marker the folder carries (M15). The order
/// the tools are asked in is the one the specification fixes for a folder carrying more than
/// one marker.
pub fn marker_in(dir: &Path) -> Option<Tool> {
    marker(dir, true)
}

/// The tool whose working copy a folder is inside: its own marker first, then its parents'
/// (M15). A Visual Source Safe binding reaches no folder but the one holding its `.scc` file,
/// because VSS records the binding folder by folder.
pub fn find(dir: &Path) -> Option<WorkingCopy> {
    if let Some(tool) = marker_in(dir) {
        return Some(WorkingCopy { tool, root: dir.to_path_buf() });
    }
    let mut parent = dir.parent();
    while let Some(p) = parent {
        if let Some(tool) = marker(p, false) {
            return Some(WorkingCopy { tool, root: p.to_path_buf() });
        }
        parent = p.parent();
    }
    None
}

/// How much of a `.git` file is worth reading to find out whether it names a repository: a real
/// pointer is a line or two, so a file bigger than this is not one (M15).
const POINTER_READ: u64 = 4096;

/// One folder's own markers. `per_folder_too` says whether a Visual Source Safe `.scc` file
/// counts: it does for the folder asked about, and not for a parent being walked past (M15).
fn marker(dir: &Path, per_folder_too: bool) -> Option<Tool> {
    if dir.join(".git").is_dir() || names_a_repository(&dir.join(".git")) {
        return Some(Tool::Git);
    }
    if dir.join(".hg").is_dir() {
        return Some(Tool::Mercurial);
    }
    if dir.join(".svn").is_dir() {
        return Some(Tool::Subversion);
    }
    // CVS leaves a CVS folder in every folder it tracks, and its Repository file names the
    // module: without that file a folder of that name is just a folder.
    if dir.join("CVS").join("Repository").is_file() {
        return Some(Tool::Cvs);
    }
    if per_folder_too && (dir.join("vssver.scc").is_file() || dir.join("vssver2.scc").is_file()) {
        return Some(Tool::VisualSourceSafe);
    }
    None
}

/// A linked worktree or a submodule keeps its repository elsewhere and leaves a `.git` file
/// naming where; a file of that name that names nothing is no marker (M15).
fn names_a_repository(pointer: &Path) -> bool {
    let Ok(meta) = pointer.metadata() else { return false };
    if !meta.is_file() || meta.len() > POINTER_READ {
        return false;
    }
    std::fs::read_to_string(pointer).is_ok_and(|s| s.trim_start().starts_with("gitdir:"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fresh() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn folders(base: &Path, names: &[&str]) {
        for n in names {
            fs::create_dir_all(base.join(n)).unwrap();
        }
    }

    fn files(base: &Path, named: &[(&str, &str)]) {
        for (n, content) in named {
            let p = base.join(n);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, content).unwrap();
        }
    }

    #[test]
    fn a_git_folder_marks_its_own_folder_as_the_working_copy() {
        let d = fresh();
        folders(d.path(), &[".git", "src"]);
        assert_eq!(marker_in(d.path()), Some(Tool::Git));
        assert_eq!(find(d.path()), Some(WorkingCopy { tool: Tool::Git, root: d.path().to_path_buf() }));
    }

    /// A linked worktree or a submodule keeps its repository elsewhere and leaves a `.git` file
    /// naming that place; the folder holding the file is still the working copy.
    #[test]
    fn a_git_file_naming_a_repository_marks_its_folder_too() {
        let d = fresh();
        files(d.path(), &[(".git", "gitdir: /elsewhere/repo/.git/worktrees/one\n")]);
        assert_eq!(find(d.path()), Some(WorkingCopy { tool: Tool::Git, root: d.path().to_path_buf() }));
    }

    #[test]
    fn a_git_file_naming_no_repository_is_no_working_copy() {
        let d = fresh();
        files(d.path(), &[(".git", "I am a note, not a pointer\n")]);
        assert_eq!(marker_in(d.path()), None);
        assert_ne!(find(d.path()).map(|w| w.root), Some(d.path().to_path_buf()));
    }

    /// A marker has to be read, not swallowed whole: a folder named `.git` used as a scratch
    /// file could otherwise cost a menu a read of whatever is in it.
    #[test]
    fn an_overgrown_git_file_is_not_read_to_find_out_it_is_not_a_pointer() {
        let d = fresh();
        files(d.path(), &[(".git", &"x".repeat(1_000_000))]);
        assert_eq!(marker_in(d.path()), None);
    }

    #[test]
    fn an_hg_folder_marks_a_mercurial_working_copy() {
        let d = fresh();
        folders(d.path(), &[".hg/store"]);
        assert_eq!(find(d.path()), Some(WorkingCopy { tool: Tool::Mercurial, root: d.path().to_path_buf() }));
    }

    #[test]
    fn an_svn_folder_marks_a_subversion_working_copy() {
        let d = fresh();
        folders(d.path(), &[".svn/pristine", "trunk"]);
        assert_eq!(find(d.path()), Some(WorkingCopy { tool: Tool::Subversion, root: d.path().to_path_buf() }));
    }

    /// Before 1.7 Subversion left a `.svn` in every folder, so the deepest one is what a folder
    /// of such a tree answers — and the folder it sits in is that folder's own working copy.
    #[test]
    fn an_svn_tree_of_the_old_nested_layout_is_found_at_the_folder_its_marker_sits_in() {
        let d = fresh();
        folders(d.path(), &["a/.svn", "a/b/.svn"]);
        let deep = d.path().join("a/b");
        assert_eq!(find(&deep), Some(WorkingCopy { tool: Tool::Subversion, root: deep.clone() }));
    }

    #[test]
    fn a_cvs_folder_counts_once_it_names_its_repository() {
        let d = fresh();
        folders(d.path(), &["CVS"]);
        assert_eq!(marker_in(d.path()), None);
        files(d.path(), &[("CVS/Repository", "module/name\n")]);
        assert_eq!(find(d.path()), Some(WorkingCopy { tool: Tool::Cvs, root: d.path().to_path_buf() }));
    }

    /// VSS keeps its binding per folder, in a `.scc` file: the folder holding it is the working
    /// copy, and a folder of its own is not bound until it holds one too.
    #[test]
    fn a_vss_binding_is_the_folder_holding_its_scc_file() {
        for name in ["vssver.scc", "vssver2.scc"] {
            let d = fresh();
            folders(d.path(), &["bound"]);
            files(d.path(), &[(&format!("bound/{name}"), "")]);
            let bound = d.path().join("bound");
            assert_eq!(find(&bound), Some(WorkingCopy { tool: Tool::VisualSourceSafe, root: bound.clone() }), "{name} should mark a Visual Source Safe binding");
            let under = bound.join("deeper");
            fs::create_dir_all(&under).unwrap();
            assert_eq!(marker_in(&under), None, "{name} should not bind a folder below it");
            assert_ne!(find(&under).map(|w| w.tool), Some(Tool::VisualSourceSafe), "{name} should not reach a folder below the bound one");
        }
    }

    #[test]
    fn a_folder_inside_a_working_copy_answers_the_nearest_marker_above_it() {
        let d = fresh();
        folders(d.path(), &[".git", "a/b/c"]);
        assert_eq!(find(&d.path().join("a/b/c")), Some(WorkingCopy { tool: Tool::Git, root: d.path().to_path_buf() }));
    }

    /// One folder, several markers: the order the specification fixes settles which tool answers.
    #[test]
    fn one_folder_with_several_markers_answers_the_agreed_order() {
        let all = fresh();
        folders(all.path(), &[".git", ".hg", ".svn", "CVS"]);
        files(all.path(), &[("CVS/Repository", "m\n"), ("vssver.scc", "")]);
        assert_eq!(marker_in(all.path()), Some(Tool::Git));

        let no_git = fresh();
        folders(no_git.path(), &[".hg", ".svn", "CVS"]);
        files(no_git.path(), &[("CVS/Repository", "m\n"), ("vssver.scc", "")]);
        assert_eq!(marker_in(no_git.path()), Some(Tool::Mercurial));

        let no_distributed = fresh();
        folders(no_distributed.path(), &[".svn", "CVS"]);
        files(no_distributed.path(), &[("CVS/Repository", "m\n"), ("vssver.scc", "")]);
        assert_eq!(marker_in(no_distributed.path()), Some(Tool::Subversion));

        let central = fresh();
        folders(central.path(), &["CVS"]);
        files(central.path(), &[("CVS/Repository", "m\n"), ("vssver.scc", "")]);
        assert_eq!(marker_in(central.path()), Some(Tool::Cvs));
    }

    /// A marker deeper down belongs to the folder under it, not to the one holding it.
    #[test]
    fn a_folder_that_only_holds_a_working_copy_deeper_down_is_not_one() {
        let d = fresh();
        folders(&d.path().join("proj"), &[".git"]);
        assert_eq!(marker_in(d.path()), None);
        assert_ne!(find(d.path()).map(|w| w.root), Some(d.path().to_path_buf()));
    }

    /// A bare repository — the tool's own data without a working tree — is not a working copy.
    #[test]
    fn a_bare_repository_is_not_a_working_copy() {
        let d = fresh();
        folders(d.path(), &["objects", "refs"]);
        files(d.path(), &[("HEAD", "ref: refs/heads/main\n"), ("config", "[core]\n")]);
        assert_eq!(marker_in(d.path()), None);
    }

    /// Detection against a repository `git` made rather than against folders named like its
    /// markers, and against a linked worktree of it, which keeps its repository elsewhere and
    /// leaves a `.git` file naming where it is. Skipped with a printed reason where no git runs.
    #[test]
    fn a_working_copy_of_git_as_git_leaves_it_is_recognised() {
        let Some(git) = installed("git") else {
            eprintln!("no git client on this machine, skipping the real-repository scenario");
            return;
        };
        let d = fresh();
        ask(&git, d.path(), &["init", "-q"]);
        assert_eq!(find(d.path()), Some(WorkingCopy { tool: Tool::Git, root: d.path().to_path_buf() }), "a folder git made should answer Git");

        let hermetic = ["-c", "user.name=fixture", "-c", "user.email=fixture@example", "-c", "core.hooksPath="];
        let mut commit = hermetic.to_vec();
        commit.extend(["commit", "-q", "--no-verify", "--allow-empty", "-m", "one"]);
        ask(&git, d.path(), &commit);
        ask(&git, d.path(), &["worktree", "add", "-q", "--detach", "work", "HEAD"]);
        let work = d.path().join("work");
        assert_eq!(find(&work), Some(WorkingCopy { tool: Tool::Git, root: work.clone() }), "a linked worktree names its repository from a file");
        assert!(work.join(".git").is_file(), "the worktree's own marker should be a file, not a folder");
    }

    fn installed(program: &str) -> Option<String> {
        match std::process::Command::new(program).arg("--version").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status() {
            Ok(status) if status.success() => Some(program.to_string()),
            _ => None,
        }
    }

    fn ask(program: &str, dir: &Path, args: &[&str]) {
        let status = std::process::Command::new(program).current_dir(dir).args(args).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().expect("the client could not run");
        assert!(status.success(), "{} {args:?} in {} did not answer", program, dir.display());
    }

    #[test]
    fn each_tool_runs_its_own_client() {
        assert_eq!(Tool::Git.cli(), "git");
        assert_eq!(Tool::Mercurial.cli(), "hg");
        assert_eq!(Tool::Subversion.cli(), "svn");
        assert_eq!(Tool::Cvs.cli(), "cvs");
        assert_eq!(Tool::VisualSourceSafe.cli(), "ss");
    }
}
