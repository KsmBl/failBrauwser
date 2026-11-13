//! What the Repository menu offers and what it runs (M15). The tool behind a folder answers
//! which commands exist at all — a central-store tool has nothing to push, Visual Source Safe
//! keeps no ignore list and no blame — and the selection answers which of those the folder can
//! carry right now: a rename wants one item, a delete wants at least one, and *Clone* only
//! belongs where no working copy reaches.
//!
//! Every command is spelled out as the argument list of that tool's own client, run inside the
//! working copy, so the menu never speaks to a repository except through its own tool. The lists
//! are pinned here for all five tools: git's answers are measured against a real repository by
//! the scenarios of `super`, and the other four stay unmeasured on a machine that runs only
//! `git`, exactly as the README records.

use super::Tool;
use std::path::Path;

/// A command of the Repository submenu, in the order the submenu shows them (M15).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Clone,
    Commit,
    Pull,
    Push,
    Sync,
    Stash,
    SwitchCheckout,
    Revert,
    CheckModifications,
    Diff,
    DiffWithPrevious,
    ShowLog,
    BrowseRepository,
    Blame,
    Rename,
    Delete,
    DeleteKeepLocal,
    DeleteAndIgnore,
    Settings,
}

/// What a selection is made of, as far as the Repository menu cares (M15).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub files: usize,
    pub folders: usize,
}

impl Selection {
    pub fn total(&self) -> usize {
        self.files + self.folders
    }

    fn names_one(&self) -> bool {
        self.total() == 1
    }
}

/// The extra words a command needs before its client can run: the commit message, the branch or
/// revision to switch to, the address to clone, the name a rename moves to. A command that needs
/// one and is not given it says so by answering no arguments at all (M15).
#[derive(Clone, Copy, Debug, Default)]
pub struct Request<'w> {
    pub message: Option<&'w str>,
    pub reference: Option<&'w str>,
    pub url: Option<&'w str>,
    pub new_name: Option<&'w str>,
}

impl Command {
    /// The label the submenu shows (M15).
    pub fn label(&self) -> &'static str {
        match self {
            Command::Clone => "Clone…",
            Command::Commit => "Commit",
            Command::Pull => "Pull",
            Command::Push => "Push",
            Command::Sync => "Sync",
            Command::Stash => "Stash",
            Command::SwitchCheckout => "Switch/Checkout…",
            Command::Revert => "Revert",
            Command::CheckModifications => "Check for modifications",
            Command::Diff => "Diff",
            Command::DiffWithPrevious => "Diff with previous version",
            Command::ShowLog => "Show Log",
            Command::BrowseRepository => "Browse Repository",
            Command::Blame => "Blame",
            Command::Rename => "Rename",
            Command::Delete => "Delete",
            Command::DeleteKeepLocal => "Delete (keep local)",
            Command::DeleteAndIgnore => "Delete and add to ignore list",
            Command::Settings => "Settings",
        }
    }

    /// The commands in the order the submenu shows them, working-copy part first, then reading,
    /// then what the selected items get done to, and Settings last (M15).
    pub fn all() -> &'static [Command] {
        &[
            Command::Commit,
            Command::Pull,
            Command::Push,
            Command::Sync,
            Command::Stash,
            Command::SwitchCheckout,
            Command::Revert,
            Command::CheckModifications,
            Command::Diff,
            Command::DiffWithPrevious,
            Command::ShowLog,
            Command::BrowseRepository,
            Command::Blame,
            Command::Rename,
            Command::Delete,
            Command::DeleteKeepLocal,
            Command::DeleteAndIgnore,
            Command::Settings,
            Command::Clone,
        ]
    }

    /// Whether this tool has the command at all; a command it has no equivalent for stays out of
    /// the submenu rather than appearing greyed out (M15).
    pub fn supports(&self, tool: Tool) -> bool {
        match self {
            // A central store keeps its history on the server: there is nothing local to send
            // out, and nothing of one's own to fetch and merge back.
            Command::Push | Command::Sync | Command::Stash => matches!(tool, Tool::Git | Tool::Mercurial),
            // VSS can say who last wrote a file, but it keeps no blame, has no rename of its own,
            // no ignore list and no settings a folder could carry.
            Command::Clone | Command::Blame | Command::Rename | Command::DeleteAndIgnore => !matches!(tool, Tool::VisualSourceSafe),
            Command::Settings => matches!(tool, Tool::Git | Tool::Mercurial | Tool::Subversion),
            _ => true,
        }
    }

    /// Whether the folder can carry this command with the selection it has right now, given that
    /// a working copy does reach it (`in_working_copy`) or not (M15).
    pub fn applies(&self, tool: Tool, selection: &Selection, in_working_copy: bool) -> bool {
        if matches!(self, Command::Clone) {
            return !in_working_copy;   // a folder the tool already owns has nothing to clone into
        }
        if !in_working_copy || !self.supports(tool) {
            return false;
        }
        match self {
            // Nothing selected answers the folder's own diff; several folders at once are not one
            // thing to read.
            Command::Diff => selection.total() == 0 || selection.files > 0 || selection.folders == 1,
            Command::DiffWithPrevious | Command::Blame | Command::Rename => selection.names_one(),
            Command::Delete | Command::DeleteKeepLocal | Command::DeleteAndIgnore => selection.total() > 0,
            _ => true,
        }
    }

    /// The argument lists this command runs, one per client call, against this tool, with the
    /// selected paths (empty for the folder itself). Empty answers nothing: the command is not
    /// this tool's, or the words it needs are missing (M15).
    pub fn runs(&self, tool: Tool, what: &Request, targets: &[&Path]) -> Vec<Vec<String>> {
        if !self.supports(tool) {
            return Vec::new();
        }
        let names: Vec<String> = targets.iter().map(|p| p.display().to_string()).collect();
        // A command either speaks about the folder or about the selected items; Git and Mercurial
        // want a separator before the items, the others take them plain.
        let asked = |first: &[&str], about_items: bool| -> Vec<String> {
            let mut argv: Vec<String> = first.iter().map(|s| s.to_string()).collect();
            if about_items && !names.is_empty() {
                if matches!(tool, Tool::Git) {
                    argv.push("--".to_string());
                }
                argv.extend(names.iter().cloned());
            }
            argv
        };
        match (self, tool) {
            (Command::Clone, _) => match what.url {
                None => Vec::new(),
                Some(url) => {
                    let dir = sole(targets);
                    match tool {
                        Tool::Git | Tool::Mercurial => vec![vec!["clone".into(), url.into(), dir]],
                        Tool::Subversion => vec![vec!["checkout".into(), url.into(), dir]],
                        // CVS reads its module from the address and names the folder after the flag.
                        Tool::Cvs => vec![vec!["checkout".into(), "-d".into(), dir, url.into()]],
                        Tool::VisualSourceSafe => Vec::new(),
                    }
                }
            },
            (Command::Commit, _) => match what.message {
                // Without the message the client would open an editor of its own on a keyboard
                // nobody is sitting in front of.
                None => Vec::new(),
                Some(message) => match tool {
                    Tool::Git => vec![asked(&["commit", "-m", message], true)],
                    Tool::Mercurial | Tool::Subversion | Tool::Cvs => vec![asked(&["commit", "-m", message], true)],
                    Tool::VisualSourceSafe => vec![asked(&["Checkin", "-I-Y", "-C", message], true)],
                },
            },
            (Command::Pull, _) => vec![asked(match tool {
                Tool::Git | Tool::Mercurial => &["pull"],
                Tool::Subversion => &["update"],
                Tool::Cvs => &["update", "-d"],
                Tool::VisualSourceSafe => &["Get", "-R", "-I-Y"],
            }, false)],
            (Command::Push, _) => vec![asked(&["push"], false)],
            (Command::Sync, Tool::Git) => vec![asked(&["fetch"], false), asked(&["pull"], false), asked(&["push"], false)],
            (Command::Sync, _) => vec![asked(&["pull"], false), asked(&["push"], false)],
            (Command::Stash, Tool::Git) => match what.message {
                Some(message) => vec![asked(&["stash", "push", "-m", message], false)],
                None => vec![asked(&["stash", "push"], false)],
            },
            (Command::Stash, _) => match what.message {
                Some(message) => vec![asked(&["shelve", "-m", message], false)],
                None => vec![asked(&["shelve"], false)],
            },
            (Command::SwitchCheckout, Tool::Subversion) => what.url.map_or_else(Vec::new, |url| vec![asked(&["switch", url], false)]),
            (Command::SwitchCheckout, Tool::VisualSourceSafe) => what.reference.map_or_else(Vec::new, |r| vec![asked(&["Get", "-R", "-I-Y", &format!("-V{r}")], false)]),
            (Command::SwitchCheckout, _) => match what.reference {
                None => Vec::new(),
                Some(r) => vec![match tool {
                    Tool::Git => asked(&["switch", r], false),
                    Tool::Mercurial => asked(&["update", r], false),
                    Tool::Cvs => asked(&["update", "-r", r], false),
                    _ => Vec::new(),
                }],
            },
            // VSS has no single revert: a checked-out file goes back to the store and the stored
            // version is read again over the folder.
            (Command::Revert, Tool::VisualSourceSafe) => vec![asked(&["UndoCheckout", "-I-Y"], true), asked(&["Restore", "-R", "-I-Y"], true)],
            (Command::Revert, _) => vec![asked(match tool {
                Tool::Git => &["restore"],
                Tool::Mercurial | Tool::Subversion => &["revert"],
                Tool::Cvs => &["update", "-C"],
                _ => &[],
            }, true)],
            (Command::CheckModifications, _) => vec![asked(match tool {
                Tool::Git => &["status", "--porcelain"],
                Tool::Mercurial | Tool::Subversion => &["status"],
                Tool::Cvs => &["update", "-n", "-d"],
                Tool::VisualSourceSafe => &["Status"],
            }, false)],
            (Command::Diff, _) => vec![asked(match tool { Tool::Cvs => &["diff", "-u"], _ => &["diff"] }, true)],
            (Command::DiffWithPrevious, _) => previous(tool, what, &names),
            (Command::ShowLog, _) => vec![log_of(tool, &names)],
            (Command::BrowseRepository, _) => vec![log_of(tool, &[])],
            (Command::Blame, _) => vec![asked(match tool { Tool::Git => &["blame"], _ => &["annotate"] }, true)],
            (Command::Rename, _) => match (what.new_name, names.first()) {
                (Some(new), Some(old)) => vec![vec![match tool { Tool::Git => "mv", _ => "rename" }.to_string(), old.clone(), new.to_string()]],
                _ => Vec::new(),
            },
            (Command::Delete, _) => vec![asked(match tool {
                Tool::Git => &["rm", "-r"],
                Tool::Mercurial => &["remove"],
                Tool::Subversion => &["delete"],
                Tool::Cvs => &["remove", "-f"],
                Tool::VisualSourceSafe => &["Delete", "-I-Y"],
            }, true)],
            (Command::DeleteKeepLocal, _) => vec![asked(match tool {
                Tool::Git => &["rm", "--cached"],
                Tool::Mercurial => &["remove", "--keep-files"],
                Tool::Subversion => &["delete", "--keep-local"],
                // CVS wants the file gone from the folder, so the window that asks for this puts
                // the copy back once the client has had its way (M15).
                Tool::Cvs => &["remove"],
                Tool::VisualSourceSafe => &["DeleteKeepLocal", "-I-Y"],
            }, true)],
            (Command::DeleteAndIgnore, _) => Command::Delete.runs(tool, what, targets),
            (Command::Settings, Tool::Git) => vec![asked(&["config", "--list", "--local"], false)],
            (Command::Settings, Tool::Mercurial) => vec![asked(&["config", "--list"], false)],
            (Command::Settings, _) => vec![asked(&["proplist", "--verbose"], true)],
        }
    }

    /// The file a deletion adds its name to, where the tool keeps such a list in a file (M15).
    pub fn ignore_file(tool: Tool) -> Option<&'static str> {
        match tool {
            Tool::Git => Some(".gitignore"),
            Tool::Mercurial => Some(".hgignore"),
            Tool::Cvs => Some(".cvsignore"),
            // Subversion keeps its ignore list as a property of the folder, and VSS keeps none.
            Tool::Subversion | Tool::VisualSourceSafe => None,
        }
    }

    /// The client call that records a name as ignored where there is no file for it (M15).
    pub fn ignore_command(tool: Tool, folder: &str, name: &str) -> Vec<String> {
        match tool {
            Tool::Subversion => vec!["propset".into(), "svn:ignore".into(), name.into(), folder.into()],
            _ => Vec::new(),
        }
    }
}

/// The folder a command works in when it was called on one: the single selected place, or the
/// working copy itself.
fn sole(targets: &[&Path]) -> String {
    match targets {
        [one] => one.display().to_string(),
        _ => ".".into(),
    }
}

/// One file between its previous version and this one (M15). Git and Mercurial name a revision
/// in their own words and always have one, so Git falls back to the tip; the central tools count
/// in numbers, and their previous version is the number below the one they were given — without
/// that number there is nothing to ask them for.
fn previous(tool: Tool, what: &Request, names: &[String]) -> Vec<Vec<String>> {
    let revision = |offset: u64| what.reference.and_then(|r| r.parse::<u64>().ok()).map(|r| format!("-r{}:{}", r - offset, r));
    let mut argv: Vec<String> = match tool {
        Tool::Git => {
            let reference = what.reference.unwrap_or("HEAD");
            vec!["diff".into(), format!("{reference}~1").into(), reference.into(), "--".into()]
        }
        Tool::Mercurial => vec!["diff".into(), "--change".into(), ".".into()],
        Tool::Subversion => match revision(1) {
            Some(range) => vec!["diff".into(), range],
            None => return Vec::new(),
        },
        Tool::Cvs => match what.reference.and_then(|r| r.parse::<u64>().ok()) {
            Some(r) => vec!["diff".into(), "-u".into(), format!("-r{}", r - 1), format!("-r{r}")],
            None => return Vec::new(),
        },
        Tool::VisualSourceSafe => match what.reference.and_then(|r| r.parse::<u64>().ok()) {
            Some(r) => vec!["Diff".into(), format!("-r{}", r - 1), format!("-r{r}")],
            None => return Vec::new(),
        },
    };
    argv.extend(names.iter().cloned());
    vec![argv]
}

/// The history of a folder or of selected items, in a shape the window can read: fields between
/// files are separated by a unit mark and entries by a record mark, so a message carrying a
/// newline of its own still arrives as one entry (M15).
fn log_of(tool: Tool, names: &[String]) -> Vec<String> {
    let mut argv: Vec<String> = match tool {
        Tool::Git => vec!["log".into(), "--format=%H\x1f%an\x1f%aI\x1f%s\x1e".into()],
        Tool::Mercurial => vec!["log".into(), "--template".into(), "{node}\x1f{author}\x1f{date|isodate}\x1f{desc|firstline}\x1e".into()],
        Tool::Subversion => vec!["log".into(), "--xml".into()],
        Tool::Cvs => vec!["log".into()],
        Tool::VisualSourceSafe => vec!["History".into(), "-I".into()],
    };
    if matches!(tool, Tool::Git) && !names.is_empty() {
        argv.push("--".to_string());
    }
    argv.extend(names.iter().cloned());
    argv
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(name)
    }

    fn targets(names: &[&str]) -> Vec<std::path::PathBuf> {
        names.iter().map(|&n| path(n)).collect()
    }

    fn refs(targets: &[std::path::PathBuf]) -> Vec<&Path> {
        targets.iter().map(|p| p.as_path()).collect()
    }

    /// The tools, in the order the README table names them.
    const TOOLS: [(Tool, &str); 5] = [
        (Tool::Git, "Git"),
        (Tool::Mercurial, "Mercurial"),
        (Tool::Subversion, "Subversion"),
        (Tool::Cvs, "CVS"),
        (Tool::VisualSourceSafe, "Visual Source Safe"),
    ];

    fn one(cmd: Command, tool: Tool, what: Request, target: &str) -> Vec<String> {
        let t = targets(&[target]);
        let mut runs = cmd.runs(tool, &what, &refs(&t));
        assert_eq!(runs.len(), 1, "{:?} on {} should answer exactly one call", cmd, tool.cli());
        runs.remove(0)
    }

    #[test]
    fn the_menu_shows_every_command_of_the_order_the_specification_fixes() {
        let shown: Vec<&str> = Command::all().iter().map(|c| c.label()).collect();
        assert_eq!(
            shown,
            [
                "Commit",
                "Pull",
                "Push",
                "Sync",
                "Stash",
                "Switch/Checkout…",
                "Revert",
                "Check for modifications",
                "Diff",
                "Diff with previous version",
                "Show Log",
                "Browse Repository",
                "Blame",
                "Rename",
                "Delete",
                "Delete (keep local)",
                "Delete and add to ignore list",
                "Settings",
                "Clone…"
            ]
        );
    }

    /// A central-store tool has no local commits to send out, and Visual Source Safe keeps no
    /// ignore list of its own and no blame. Those commands are left out rather than greyed.
    #[test]
    fn a_tool_without_a_command_leaves_it_out_of_the_submenu() {
        let absent: Vec<(&str, &str)> = [Command::Push, Command::Sync, Command::Stash, Command::Clone]
            .iter()
            .flat_map(|c| TOOLS.iter().filter(|(t, _)| !c.supports(*t)).map(move |(_, name)| (c.label(), *name)))
            .collect();
        for (label, name) in [("Push", "Subversion"), ("Push", "CVS"), ("Push", "Visual Source Safe"), ("Sync", "Subversion"), ("Sync", "CVS"), ("Sync", "Visual Source Safe"), ("Stash", "Subversion"), ("Stash", "CVS"), ("Stash", "Visual Source Safe"), ("Clone…", "Visual Source Safe")] {
            assert!(absent.contains(&(label, name)), "{label} should be left out for {name}");
        }
        for tool in [Tool::VisualSourceSafe] {
            for cmd in [Command::Blame, Command::Rename, Command::DeleteAndIgnore, Command::Settings] {
                assert!(!cmd.supports(tool), "{} should be left out for {}", cmd.label(), tool.cli());
            }
        }
        for cmd in [Command::Commit, Command::Diff, Command::ShowLog, Command::Revert, Command::Delete, Command::DeleteKeepLocal, Command::CheckModifications] {
            for (tool, name) in TOOLS {
                assert!(cmd.supports(tool), "{} should be there for {}", cmd.label(), name);
            }
        }
    }

    /// A command needs the selection its tool can act on: a rename or a blame wants exactly one
    /// item, a delete wants at least one, and a folder-wide diff is what no selection answers.
    #[test]
    fn what_the_selection_carries_decides_which_of_them_the_folder_offers() {
        let none = Selection::default();
        let one_file = Selection { files: 1, folders: 0 };
        let one_folder = Selection { files: 0, folders: 1 };
        let many = Selection { files: 2, folders: 3 };

        for cmd in [Command::DiffWithPrevious, Command::Blame, Command::Rename] {
            assert!(cmd.applies(Tool::Git, &one_file, true), "{} wants one item and has it", cmd.label());
            assert!(cmd.applies(Tool::Git, &one_folder, true), "a folder is still one item for {}", cmd.label());
            for empty in [none, many] {
                assert!(!cmd.applies(Tool::Git, &empty, true), "{} should wait for exactly one item", cmd.label());
            }
        }
        for cmd in [Command::Delete, Command::DeleteKeepLocal, Command::DeleteAndIgnore] {
            assert!(!cmd.applies(Tool::Git, &none, true), "{} wants something selected", cmd.label());
            assert!(cmd.applies(Tool::Git, &many, true), "{} takes the whole selection", cmd.label());
        }
        assert!(Command::Diff.applies(Tool::Git, &none, true), "no selection answers the folder's own diff");
        assert!(Command::Diff.applies(Tool::Git, &many, true));
        assert!(!Command::Diff.applies(Tool::Git, &Selection { files: 0, folders: 2 }, true), "several folders at once are not one diff to read");

        for cmd in [Command::Commit, Command::Pull, Command::CheckModifications, Command::ShowLog, Command::BrowseRepository] {
            assert!(cmd.applies(Tool::Git, &none, true), "{} works on the folder itself", cmd.label());
            assert!(!cmd.applies(Tool::Git, &none, false), "{} wants a working copy", cmd.label());
        }
        assert!(Command::Clone.applies(Tool::Git, &none, false), "a plain folder is where Clone lives");
        for tool in [Tool::Git, Tool::Mercurial, Tool::Subversion, Tool::Cvs] {
            assert!(!Command::Clone.applies(tool, &none, true), "a folder already inside a {} working copy needs no Clone", tool.cli());
        }
    }

    #[test]
    fn git_answers_the_arguments_a_repository_of_its_own_understands() {
        let file = targets(&["src/a.rs"]);
        assert_eq!(one(Command::Commit, Tool::Git, Request { message: Some("a change"), ..Default::default() }, "src/a.rs"), ["commit", "-m", "a change", "--", "src/a.rs"]);
        assert_eq!(one(Command::Pull, Tool::Git, Request::default(), "."), ["pull"]);
        assert_eq!(one(Command::Push, Tool::Git, Request::default(), "."), ["push"]);
        assert_eq!(one(Command::Stash, Tool::Git, Request { message: Some("work"), ..Default::default() }, "."), ["stash", "push", "-m", "work"]);
        assert_eq!(one(Command::SwitchCheckout, Tool::Git, Request { reference: Some("next"), ..Default::default() }, "."), ["switch", "next"]);
        assert_eq!(one(Command::Revert, Tool::Git, Request::default(), "src/a.rs"), ["restore", "--", "src/a.rs"]);
        assert_eq!(one(Command::CheckModifications, Tool::Git, Request::default(), "."), ["status", "--porcelain"]);
        assert_eq!(Command::Diff.runs(Tool::Git, &Request::default(), &refs(&file)), vec![vec!["diff".to_string(), "--".to_string(), "src/a.rs".to_string()]]);
        assert_eq!(Command::Diff.runs(Tool::Git, &Request::default(), &[]), vec![vec!["diff".to_string()]]);
        assert_eq!(one(Command::DiffWithPrevious, Tool::Git, Request { reference: Some("HEAD"), ..Default::default() }, "src/a.rs"), ["diff", "HEAD~1", "HEAD", "--", "src/a.rs"]);
        assert_eq!(one(Command::Blame, Tool::Git, Request::default(), "src/a.rs"), ["blame", "--", "src/a.rs"]);
        assert_eq!(one(Command::Rename, Tool::Git, Request { new_name: Some("b.rs"), ..Default::default() }, "src/a.rs"), ["mv", "src/a.rs", "b.rs"]);
        assert_eq!(one(Command::Delete, Tool::Git, Request::default(), "docs"), ["rm", "-r", "--", "docs"]);
        assert_eq!(one(Command::DeleteKeepLocal, Tool::Git, Request::default(), "src/a.rs"), ["rm", "--cached", "--", "src/a.rs"]);
        assert_eq!(Command::BrowseRepository.runs(Tool::Git, &Request::default(), &[]), Command::ShowLog.runs(Tool::Git, &Request::default(), &[]));
        assert_eq!(one(Command::Settings, Tool::Git, Request::default(), "."), ["config", "--list", "--local"]);
        assert_eq!(one(Command::Clone, Tool::Git, Request { url: Some("https://example.invalid/x.git"), ..Default::default() }, "."), ["clone", "https://example.invalid/x.git", "."]);
        assert_eq!(Command::Sync.runs(Tool::Git, &Request::default(), &[]), vec![vec!["fetch".to_string()], vec!["pull".to_string()], vec!["push".to_string()]]);
        assert!(Command::Commit.runs(Tool::Git, &Request::default(), &refs(&file)).is_empty(), "a commit without a message has nothing to run, and no editor of the client's own gets opened");
        assert_eq!(Command::ignore_file(Tool::Git), Some(".gitignore"));
    }

    #[test]
    fn the_other_four_tools_are_answered_by_their_own_clients_verbs() {
        // Mercurial.
        assert_eq!(one(Command::Commit, Tool::Mercurial, Request { message: Some("a change"), ..Default::default() }, "a.txt"), ["commit", "-m", "a change", "a.txt"]);
        assert_eq!(one(Command::Pull, Tool::Mercurial, Request::default(), "."), ["pull"]);
        assert_eq!(Command::Sync.runs(Tool::Mercurial, &Request::default(), &[]), vec![vec!["pull".to_string()], vec!["push".to_string()]], "Mercurial syncs by pulling then pushing");
        assert_eq!(one(Command::Stash, Tool::Mercurial, Request { message: Some("work"), ..Default::default() }, "."), ["shelve", "-m", "work"]);
        assert_eq!(one(Command::SwitchCheckout, Tool::Mercurial, Request { reference: Some("next"), ..Default::default() }, "."), ["update", "next"]);
        assert_eq!(one(Command::Revert, Tool::Mercurial, Request::default(), "a.txt"), ["revert", "a.txt"]);
        assert_eq!(one(Command::CheckModifications, Tool::Mercurial, Request::default(), "."), ["status"]);
        assert_eq!(one(Command::DiffWithPrevious, Tool::Mercurial, Request::default(), "a.txt"), ["diff", "--change", ".", "a.txt"]);
        assert_eq!(one(Command::Blame, Tool::Mercurial, Request::default(), "a.txt"), ["annotate", "a.txt"]);
        assert_eq!(one(Command::Rename, Tool::Mercurial, Request { new_name: Some("b.txt"), ..Default::default() }, "a.txt"), ["rename", "a.txt", "b.txt"]);
        assert_eq!(one(Command::Delete, Tool::Mercurial, Request::default(), "a.txt"), ["remove", "a.txt"]);
        assert_eq!(one(Command::DeleteKeepLocal, Tool::Mercurial, Request::default(), "a.txt"), ["remove", "--keep-files", "a.txt"]);
        assert_eq!(one(Command::Settings, Tool::Mercurial, Request::default(), "."), ["config", "--list"]);
        assert_eq!(Command::ignore_file(Tool::Mercurial), Some(".hgignore"));

        // Subversion.
        assert_eq!(one(Command::Commit, Tool::Subversion, Request { message: Some("a change"), ..Default::default() }, "a.txt"), ["commit", "-m", "a change", "a.txt"]);
        assert_eq!(one(Command::Pull, Tool::Subversion, Request::default(), "."), ["update"]);
        assert_eq!(one(Command::SwitchCheckout, Tool::Subversion, Request { url: Some("https://example.invalid/repo/trunk"), ..Default::default() }, "."), ["switch", "https://example.invalid/repo/trunk"]);
        assert_eq!(one(Command::Revert, Tool::Subversion, Request::default(), "a.txt"), ["revert", "a.txt"]);
        assert_eq!(one(Command::CheckModifications, Tool::Subversion, Request::default(), "."), ["status"]);
        assert_eq!(one(Command::DiffWithPrevious, Tool::Subversion, Request { reference: Some("41"), ..Default::default() }, "a.txt"), ["diff", "-r40:41", "a.txt"]);
        assert_eq!(one(Command::Blame, Tool::Subversion, Request::default(), "a.txt"), ["annotate", "a.txt"]);
        assert_eq!(one(Command::Rename, Tool::Subversion, Request { new_name: Some("b.txt"), ..Default::default() }, "a.txt"), ["rename", "a.txt", "b.txt"]);
        assert_eq!(one(Command::Delete, Tool::Subversion, Request::default(), "a.txt"), ["delete", "a.txt"]);
        assert_eq!(one(Command::DeleteKeepLocal, Tool::Subversion, Request::default(), "a.txt"), ["delete", "--keep-local", "a.txt"]);
        assert_eq!(one(Command::DeleteAndIgnore, Tool::Subversion, Request::default(), "a.txt"), ["delete", "a.txt"], "the ignore property arrives after the delete, so the client keeps the two straight");
        assert_eq!(Command::ignore_file(Tool::Subversion), None, "Subversion keeps its ignore list as a property, not a file");
        assert_eq!(one(Command::Settings, Tool::Subversion, Request::default(), "."), ["proplist", "--verbose", "."]);
        assert_eq!(one(Command::Clone, Tool::Subversion, Request { url: Some("https://example.invalid/repo/trunk"), ..Default::default() }, "trunk"), ["checkout", "https://example.invalid/repo/trunk", "trunk"]);

        // CVS.
        assert_eq!(one(Command::Commit, Tool::Cvs, Request { message: Some("a change"), ..Default::default() }, "a.txt"), ["commit", "-m", "a change", "a.txt"]);
        assert_eq!(one(Command::Pull, Tool::Cvs, Request::default(), "."), ["update", "-d"]);
        assert_eq!(one(Command::SwitchCheckout, Tool::Cvs, Request { reference: Some("RELEASE_2"), ..Default::default() }, "."), ["update", "-r", "RELEASE_2"]);
        assert_eq!(one(Command::Revert, Tool::Cvs, Request::default(), "a.txt"), ["update", "-C", "a.txt"]);
        assert_eq!(one(Command::CheckModifications, Tool::Cvs, Request::default(), "."), ["update", "-n", "-d"]);
        assert_eq!(one(Command::Blame, Tool::Cvs, Request::default(), "a.txt"), ["annotate", "a.txt"]);
        assert_eq!(one(Command::Rename, Tool::Cvs, Request { new_name: Some("b.txt"), ..Default::default() }, "a.txt"), ["rename", "a.txt", "b.txt"]);
        assert_eq!(one(Command::Delete, Tool::Cvs, Request::default(), "a.txt"), ["remove", "-f", "a.txt"]);
        assert_eq!(one(Command::DeleteKeepLocal, Tool::Cvs, Request::default(), "a.txt"), ["remove", "a.txt"]);
        assert_eq!(Command::ignore_file(Tool::Cvs), Some(".cvsignore"));
        assert_eq!(one(Command::Clone, Tool::Cvs, Request { url: Some("module/name"), ..Default::default() }, "checkout-here"), ["checkout", "-d", "checkout-here", "module/name"]);
        let dotted = Command::DiffWithPrevious.runs(Tool::Cvs, &Request { reference: Some("1.3"), ..Default::default() }, &refs(&targets(&["a.txt"])));
        assert!(dotted.is_empty(), "CVS counts a previous version as two plain numbers, so a dotted 1.3 answers nothing rather than a wrong pair");
        assert_eq!(one(Command::DiffWithPrevious, Tool::Cvs, Request { reference: Some("41"), ..Default::default() }, "a.txt"), ["diff", "-u", "-r40", "-r41", "a.txt"]);

        // Visual Source Safe.
        assert_eq!(one(Command::Commit, Tool::VisualSourceSafe, Request { message: Some("a change"), ..Default::default() }, "a.txt"), ["Checkin", "-I-Y", "-C", "a change", "a.txt"], "the comment travels with the call, or ss.exe would stop and ask for one");
        assert_eq!(one(Command::Pull, Tool::VisualSourceSafe, Request::default(), "."), ["Get", "-R", "-I-Y"]);
        assert_eq!(one(Command::SwitchCheckout, Tool::VisualSourceSafe, Request { reference: Some("$/proj/main"), ..Default::default() }, "."), ["Get", "-R", "-I-Y", "-V$/proj/main"]);
        assert_eq!(Command::Revert.runs(Tool::VisualSourceSafe, &Request::default(), &refs(&targets(&["a.txt"]))), vec![
            vec!["UndoCheckout".to_string(), "-I-Y".to_string(), "a.txt".to_string()],
            vec!["Restore".to_string(), "-R".to_string(), "-I-Y".to_string(), "a.txt".to_string()],
        ], "a VSS revert undoes the checkout and then reads the stored version back over it");
        assert_eq!(one(Command::CheckModifications, Tool::VisualSourceSafe, Request::default(), "."), ["Status"]);
        assert_eq!(one(Command::DiffWithPrevious, Tool::VisualSourceSafe, Request { reference: Some("41"), ..Default::default() }, "a.txt"), ["Diff", "-r40", "-r41", "a.txt"]);
        assert_eq!(one(Command::ShowLog, Tool::VisualSourceSafe, Request::default(), "a.txt"), ["History", "-I", "a.txt"]);
        assert_eq!(one(Command::Delete, Tool::VisualSourceSafe, Request::default(), "a.txt"), ["Delete", "-I-Y", "a.txt"]);
        assert_eq!(one(Command::DeleteKeepLocal, Tool::VisualSourceSafe, Request::default(), "a.txt"), ["DeleteKeepLocal", "-I-Y", "a.txt"]);
        assert_eq!(Command::ignore_file(Tool::VisualSourceSafe), None);
        assert_eq!(Command::ignore_command(Tool::Subversion, "docs", "a.txt"), ["propset", "svn:ignore", "a.txt", "docs"]);
        for tool in [Tool::Git, Tool::Mercurial, Tool::Cvs, Tool::VisualSourceSafe] {
            assert!(Command::ignore_command(tool, "docs", "a.txt").is_empty(), "{} keeps its ignore list elsewhere", tool.cli());
        }
    }

    /// A command that needs a word it was not given runs nothing, rather than starting the
    /// client's own dialogue in a process whose keyboard nobody is looking at.
    #[test]
    fn a_command_missing_the_words_it_needs_starts_nothing() {
        for (cmd, tool) in [
            (Command::Commit, Tool::Git),
            (Command::Commit, Tool::Cvs),
            (Command::Commit, Tool::VisualSourceSafe),
            (Command::Clone, Tool::Git),
            (Command::SwitchCheckout, Tool::Git),
            (Command::SwitchCheckout, Tool::Subversion),
            (Command::Rename, Tool::Git),
            (Command::DiffWithPrevious, Tool::Subversion),
        ] {
            assert!(cmd.runs(tool, &Request::default(), &[]).is_empty(), "{} on {} should wait for its words", cmd.label(), tool.cli());
        }
    }
}
