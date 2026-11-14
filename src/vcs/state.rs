//! The words every version-control tool shares (DDD: the domain's own language): the state a
//! file or folder is in, a change waiting to be committed, a revision of the history and its
//! labels, a line of blame, a row of a side-by-side diff and a row of the revision graph. Nothing
//! here runs a client or needs a display; each backend (`git`, `hg`, `svn`, `cvs`, `vss`) answers
//! in these words.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;

/// The state of a file or folder, as its overlay icon shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Status {
    Normal,
    Ignored,
    Unversioned,
    Added,
    Renamed,
    Deleted,
    Modified,
    Conflicted,
}

impl Status {
    /// The word TortoiseGit's lists show.
    pub fn label(self) -> &'static str {
        match self {
            Status::Normal => "Normal",
            Status::Ignored => "Ignored",
            Status::Unversioned => "Unversioned",
            Status::Added => "Added",
            Status::Renamed => "Renamed",
            Status::Deleted => "Deleted",
            Status::Modified => "Modified",
            Status::Conflicted => "Conflicted",
        }
    }
    /// Changed in a way a commit records (not unversioned, ignored or unchanged).
    pub fn is_change(self) -> bool {
        matches!(self, Status::Added | Status::Renamed | Status::Deleted | Status::Modified | Status::Conflicted)
    }
}

/// A changed file of the working tree, relative to the top of the working tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub path: String,
    /// Where a renamed file was before.
    pub old_path: Option<String>,
    pub status: Status,
    /// Its changes are in the index already.
    pub staged: bool,
    /// Lines added and removed (None for binary files and those not known).
    pub added: Option<u64>,
    pub removed: Option<u64>,
}

impl Change {
    /// The extension TortoiseGit shows in its own column (".rs").
    pub fn extension(&self) -> String {
        let name = self.path.rsplit('/').next().unwrap_or(&self.path);
        match name.rfind('.') {
            Some(i) if i > 0 => name[i..].to_string(),
            _ => String::new(),
        }
    }
}

/// The overlay status of the entries of the folder `rel` (relative to the top of the tree)
/// from its status lines and the tracked files under it.
pub fn folder_status_from<'a>(rel: &str, changes: &[Change], tracked: impl Iterator<Item = &'a str>) -> HashMap<OsString, Status> {
    let prefix = if rel.is_empty() { String::new() } else { format!("{rel}/") };
    let child = |path: &str| -> Option<(String, bool)> {
        let rest = path.strip_prefix(&prefix)?;
        Some(match rest.split_once('/') {
            Some((first, _)) => (first.to_string(), true),
            None => (rest.to_string(), false),
        })
    };
    let mut out: HashMap<OsString, Status> = HashMap::new();
    for t in tracked {
        if let Some((name, _)) = child(t) {
            out.entry(OsString::from(name)).or_insert(Status::Normal);
        }
    }
    for c in changes {
        let Some((name, inside)) = child(&c.path) else { continue };
        let key = OsString::from(&name);
        let st = match (c.status, inside) {
            // What lies inside a folder: changes make it modified, a conflict conflicted;
            // unversioned or ignored files inside do not change a versioned folder.
            (Status::Conflicted, true) => Status::Conflicted,
            (s, true) if s.is_change() => Status::Modified,
            (Status::Unversioned | Status::Ignored, true) => {
                out.entry(key).or_insert(c.status);
                continue;
            }
            (s, _) => s,
        };
        let e = out.entry(key).or_insert(st);
        // A deleted file shown again (re-created, untracked) stays deleted; otherwise the
        // more important state wins.
        if rank(st) > rank(*e) {
            *e = st;
        }
    }
    out
}

/// How much a state matters when one folder holds several.
fn rank(s: Status) -> u8 {
    match s {
        Status::Ignored => 0,
        Status::Unversioned => 1,
        Status::Normal => 2,
        Status::Added | Status::Renamed => 3,
        Status::Deleted => 4,
        Status::Modified => 5,
        Status::Conflicted => 6,
    }
}

/// Branches, tags and the labels shown next to the commits they point at.
#[derive(Debug, Default, Clone)]
pub struct Refs {
    pub branches: Vec<String>,
    pub remote_branches: Vec<String>,
    pub tags: Vec<String>,
    pub by_commit: HashMap<String, Vec<RefLabel>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefLabel {
    Branch(String),
    Remote(String),
    Tag(String),
    Stash,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    pub time: i64,
    pub committer: String,
    pub commit_time: i64,
    /// The whole message; its first line is the subject.
    pub message: String,
}

impl Commit {
    pub fn subject(&self) -> &str {
        self.message.lines().next().unwrap_or("")
    }
    pub fn short(&self) -> &str {
        &self.hash[..self.hash.len().min(8)]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameLine {
    pub hash: String,
    pub author: String,
    pub time: i64,
    pub summary: String,
    /// The line number in the file shown (from 1).
    pub line: usize,
    pub text: String,
}

/// One line of a side-by-side diff: what the left (old) and right (new) side show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffRow {
    pub left: Option<(usize, String)>,
    pub right: Option<(usize, String)>,
    pub kind: DiffKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    Same,
    /// Only on the left: removed.
    Removed,
    /// Only on the right: added.
    Added,
    /// Changed: the old line left, the new one right.
    Changed,
}

/// Lines `old` and `new` side by side, removed and added lines paired where a block of
/// them replaces another.
pub fn side_by_side(old: &str, new: &str) -> Vec<DiffRow> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let ops = diff_lines(&a, &b);
    let mut rows = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    let mut k = 0;
    while k < ops.len() {
        match ops[k] {
            Op::Same => {
                rows.push(DiffRow { left: Some((i + 1, a[i].to_string())), right: Some((j + 1, b[j].to_string())), kind: DiffKind::Same });
                i += 1;
                j += 1;
                k += 1;
            }
            _ => {
                // A run of removals and additions, paired line by line.
                let (mut dels, mut adds) = (Vec::new(), Vec::new());
                while k < ops.len() && ops[k] != Op::Same {
                    if ops[k] == Op::Del {
                        dels.push(i);
                        i += 1;
                    } else {
                        adds.push(j);
                        j += 1;
                    }
                    k += 1;
                }
                for n in 0..dels.len().max(adds.len()) {
                    let l = dels.get(n).map(|&x| (x + 1, a[x].to_string()));
                    let r = adds.get(n).map(|&x| (x + 1, b[x].to_string()));
                    let kind = match (&l, &r) {
                        (Some(_), Some(_)) => DiffKind::Changed,
                        (Some(_), None) => DiffKind::Removed,
                        _ => DiffKind::Added,
                    };
                    rows.push(DiffRow { left: l, right: r, kind });
                }
            }
        }
    }
    rows
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Op {
    Same,
    Del,
    Add,
}

/// A shortest line diff (Myers) of `a` and `b`, with equal lines at both ends taken off
/// first so large files with small changes stay quick.
pub(super) fn diff_lines(a: &[&str], b: &[&str]) -> Vec<Op> {
    let pre = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let suf = a[pre..].iter().rev().zip(b[pre..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (ma, mb) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    let mut ops = vec![Op::Same; pre];
    ops.extend(myers(ma, mb));
    ops.extend(std::iter::repeat_n(Op::Same, suf));
    ops
}

fn myers(a: &[&str], b: &[&str]) -> Vec<Op> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    if n == 0 || m == 0 {
        return std::iter::repeat_n(Op::Del, n as usize).chain(std::iter::repeat_n(Op::Add, m as usize)).collect();
    }
    // Too big for the quadratic trace: everything replaced.
    if (n + m) as usize > 40_000 {
        return std::iter::repeat_n(Op::Del, n as usize).chain(std::iter::repeat_n(Op::Add, m as usize)).collect();
    }
    let max = n + m;
    let off = max;
    let mut v = vec![0isize; (2 * max + 2) as usize];
    let mut trace: Vec<Vec<isize>> = Vec::new();
    'outer: for d in 0..=max {
        trace.push(v.clone());
        let mut k = -d;
        while k <= d {
            let idx = (k + off) as usize;
            let mut x = if k == -d || (k != d && v[idx - 1] < v[idx + 1]) { v[idx + 1] } else { v[idx - 1] + 1 };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[idx] = x;
            if x >= n && y >= m {
                trace.push(v.clone());
                break 'outer;
            }
            k += 2;
        }
    }
    // Walk back through the trace.
    let mut ops = Vec::new();
    let (mut x, mut y) = (n, m);
    for d in (1..trace.len() - 1).rev() {
        let v = &trace[d];
        let d = d as isize;
        let k = x - y;
        let idx = (k + off) as usize;
        let prev_k = if k == -d || (k != d && v[idx - 1] < v[idx + 1]) { k + 1 } else { k - 1 };
        let prev_x = v[(prev_k + off) as usize];
        let prev_y = prev_x - prev_k;
        while x > prev_x && y > prev_y {
            ops.push(Op::Same);
            x -= 1;
            y -= 1;
        }
        if x == prev_x {
            ops.push(Op::Add);
        } else {
            ops.push(Op::Del);
        }
        x = prev_x;
        y = prev_y;
    }
    while x > 0 && y > 0 {
        ops.push(Op::Same);
        x -= 1;
        y -= 1;
    }
    ops.reverse();
    ops
}

/// Where a commit sits in the revision graph and which lines pass its row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRow {
    /// The column of the commit's dot.
    pub lane: usize,
    /// Lines from the row above: (column above, column here).
    pub top: Vec<(usize, usize)>,
    /// Lines to the row below: (column here, column below).
    pub bottom: Vec<(usize, usize)>,
    /// Columns in use in this row.
    pub width: usize,
}

/// Lays out the graph of `commits` (newest first, each with its parents) in columns.
pub fn graph(commits: &[(String, Vec<String>)]) -> Vec<GraphRow> {
    // lanes[i]: the commit the line in column i is heading for.
    let mut lanes: Vec<Option<String>> = Vec::new();
    let mut rows = Vec::with_capacity(commits.len());
    for (hash, parents) in commits {
        let before = lanes.clone();
        let lane = match lanes.iter().position(|l| l.as_deref() == Some(hash)) {
            Some(i) => i,
            None => match lanes.iter().position(Option::is_none) {
                Some(i) => {
                    lanes[i] = Some(hash.clone());
                    i
                }
                None => {
                    lanes.push(Some(hash.clone()));
                    lanes.len() - 1
                }
            },
        };
        // Lines from above: every column that was heading somewhere continues; those
        // heading for this commit end in its dot.
        let mut top = Vec::new();
        for (i, l) in before.iter().enumerate() {
            match l {
                Some(h) if h == hash => top.push((i, lane)),
                Some(_) => top.push((i, i)),
                None => {}
            }
        }
        // Other columns that also waited for this commit are free now.
        for (i, l) in lanes.iter_mut().enumerate() {
            if i != lane && l.as_deref() == Some(hash) {
                *l = None;
            }
        }
        // The first parent takes over the commit's column; others get columns of their own.
        lanes[lane] = parents.first().cloned();
        let mut bottom = Vec::new();
        if !parents.is_empty() {
            bottom.push((lane, lane));
        }
        for p in parents.iter().skip(1) {
            let target = match lanes.iter().position(|l| l.as_deref() == Some(p)) {
                Some(i) => i,
                None => match lanes.iter().position(Option::is_none) {
                    Some(i) => {
                        lanes[i] = Some(p.clone());
                        i
                    }
                    None => {
                        lanes.push(Some(p.clone()));
                        lanes.len() - 1
                    }
                },
            };
            bottom.push((lane, target));
        }
        // Every other column in use goes on straight down, unless this row only just opened
        // it for one of its parents (then its line comes from the dot).
        for (i, l) in lanes.iter().enumerate() {
            if i == lane || l.is_none() {
                continue;
            }
            let ran_on = before.get(i).is_some_and(|b| b.as_ref().is_some_and(|h| h != hash));
            if ran_on || !bottom.iter().any(|&(f, t)| f == lane && t == i) {
                bottom.push((i, i));
            }
        }
        bottom.sort();
        bottom.dedup();
        while lanes.last().is_some_and(Option::is_none) {
            lanes.pop();
        }
        let width = top.iter().chain(&bottom).flat_map(|&(a, b)| [a, b]).chain([lane]).max().unwrap_or(0) + 1;
        rows.push(GraphRow { lane, top, bottom, width });
    }
    rows
}

/// Adds `pattern` to an ignore file (creating it), unless it is there already.
pub fn add_ignore(file: &Path, pattern: &str) -> std::io::Result<()> {
    let text = std::fs::read_to_string(file).unwrap_or_default();
    if text.lines().any(|l| l.trim() == pattern) {
        return Ok(());
    }
    if let Some(d) = file.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut new = text;
    if !new.is_empty() && !new.ends_with('\n') {
        new.push('\n');
    }
    new.push_str(pattern);
    new.push('\n');
    std::fs::write(file, new)
}

/// The patterns TortoiseGit's "Add to ignore list" offers for a file: its name, and
/// "*.ext" for its extension.
pub fn ignore_patterns(name: &str) -> Vec<String> {
    let mut out = vec![name.to_string()];
    if let Some(i) = name.rfind('.').filter(|&i| i > 0) {
        out.push(format!("*{}", &name[i..]));
    }
    out
}

