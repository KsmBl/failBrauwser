//! Renaming many files at once: find and replace (plain or a regular expression), a name
//! template with a counter and the date, and the letter case. Every row is checked before
//! anything is renamed; the renames go through temporary names, so swapping two names works.

use regex::{Regex, RegexBuilder};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Case {
    #[default]
    Keep,
    Lower,
    Upper,
    /// Every word of the name (not the extension) starts with a capital.
    Title,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub find: String,
    pub replace: String,
    pub regex: bool,
    pub ignore_case: bool,
    /// `{name}` the name without extension, `{ext}` the extension with its dot, `{n}` the
    /// counter (`{n:3}` or `{n:03}` padded to three digits), `{date}` the day it was modified.
    pub template: String,
    pub start: i64,
    pub case: Case,
}

impl Default for Rule {
    fn default() -> Self {
        Rule { find: String::new(), replace: String::new(), regex: false, ignore_case: false, template: "{name}{ext}".into(), start: 1, case: Case::Keep }
    }
}

/// A file to rename: its name and when it was modified (Unix time).
#[derive(Debug, Clone)]
pub struct Source {
    pub name: String,
    pub mtime: i64,
}

fn finder(rule: &Rule) -> Result<Option<Regex>, String> {
    if rule.find.is_empty() {
        return Ok(None);
    }
    let pattern = if rule.regex { rule.find.clone() } else { regex::escape(&rule.find) };
    RegexBuilder::new(&pattern).case_insensitive(rule.ignore_case).build().map(Some).map_err(|e| {
        // The first line says what is wrong; the rest draws arrows under the pattern.
        e.to_string().lines().last().unwrap_or("invalid pattern").trim().to_string()
    })
}

fn date(mtime: i64) -> String {
    gtk::glib::DateTime::from_unix_local(mtime).and_then(|d| d.format("%Y-%m-%d")).map(|s| s.to_string()).unwrap_or_default()
}

fn title_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut start = true;
    for c in s.chars() {
        if start { out.extend(c.to_uppercase()) } else { out.extend(c.to_lowercase()) }
        start = !c.is_alphanumeric() && c != '\'';
    }
    out
}

/// Fills in the template for one file.
fn fill(template: &str, stem: &str, ext: &str, n: i64, mtime: i64) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let token = &after[..close];
        match token {
            "name" => out.push_str(stem),
            "ext" => out.push_str(ext),
            "date" => out.push_str(&date(mtime)),
            "n" => out.push_str(&n.to_string()),
            t if t.starts_with("n:") && t[2..].trim_start_matches('0').parse::<usize>().is_ok() || t == "n:0" => {
                let width: usize = t[2..].trim_start_matches('0').parse().unwrap_or(0);
                out.push_str(&format!("{n:0width$}"));
            }
            // Not a token: kept as it is.
            _ => {
                out.push('{');
                out.push_str(token);
                out.push('}');
            }
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

/// The new names, in the order of `sources`; an error when the pattern is invalid.
pub fn plan(rule: &Rule, sources: &[Source]) -> Result<Vec<String>, String> {
    let re = finder(rule)?;
    Ok(sources
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let replaced = match &re {
                Some(re) if rule.regex => re.replace_all(&s.name, rule.replace.as_str()).into_owned(),
                Some(re) => re.replace_all(&s.name, regex::NoExpand(&rule.replace)).into_owned(),
                None => s.name.clone(),
            };
            let (stem, ext) = super::names::split_extension(replaced.as_bytes());
            let (stem, ext) = (String::from_utf8_lossy(stem).into_owned(), String::from_utf8_lossy(ext).into_owned());
            let name = fill(&rule.template, &stem, &ext, rule.start + i as i64, s.mtime);
            match rule.case {
                Case::Keep => name,
                Case::Lower => name.to_lowercase(),
                Case::Upper => name.to_uppercase(),
                Case::Title => {
                    let (stem, ext) = super::names::split_extension(name.as_bytes());
                    format!("{}{}", title_case(&String::from_utf8_lossy(stem)), String::from_utf8_lossy(ext))
                }
            }
        })
        .collect())
}

/// What is wrong with each row (None: fine). `old` and `new` are names in one folder;
/// `exists` tells whether a name is taken in it.
pub fn problems(old: &[String], new: &[String], exists: impl Fn(&str) -> bool) -> Vec<Option<String>> {
    let mut count: HashMap<&str, usize> = HashMap::new();
    for n in new {
        *count.entry(n.as_str()).or_default() += 1;
    }
    old.iter()
        .zip(new)
        .map(|(o, n)| {
            if n.is_empty() || n.contains('/') || n == "." || n == ".." || n.contains('\0') {
                Some("not a valid name".into())
            } else if count[n.as_str()] > 1 {
                Some("two files would get this name".into())
            } else if n != o && exists(n) && !old.iter().any(|x| x == n) {
                Some("a file with this name is already there".into())
            } else {
                None
            }
        })
        .collect()
}

/// Renames each `(file, new name)` in its folder. Names that stay are skipped; the others
/// first get temporary names, then their new ones, so names may be swapped. On an error
/// the renames done so far are taken back. Returns `(old path, new path)` of each rename.
pub fn apply(renames: &[(PathBuf, String)]) -> io::Result<Vec<(PathBuf, PathBuf)>> {
    let todo: Vec<(PathBuf, PathBuf)> = renames
        .iter()
        .filter(|(p, n)| p.file_name().is_some_and(|f| f != n.as_str()))
        .map(|(p, n)| (p.clone(), p.with_file_name(n)))
        .collect();
    let tag = format!(".fb-rename-{}-", std::process::id());
    let mut parked: Vec<(PathBuf, PathBuf)> = Vec::new(); // (original, temporary)
    let undo_parked = |parked: &[(PathBuf, PathBuf)]| {
        for (orig, tmp) in parked.iter().rev() {
            let _ = std::fs::rename(tmp, orig);
        }
    };
    for (i, (from, _)) in todo.iter().enumerate() {
        let tmp = from.with_file_name(format!("{tag}{i}"));
        if let Err(e) = super::copy::rename_noreplace(from, &tmp) {
            undo_parked(&parked);
            return Err(e);
        }
        parked.push((from.clone(), tmp));
    }
    let mut done: Vec<(PathBuf, PathBuf)> = Vec::new();
    for ((from, to), (_, tmp)) in todo.iter().zip(&parked) {
        if let Err(e) = super::copy::rename_noreplace(tmp, to) {
            for (f, t) in done.iter().rev() {
                let _ = std::fs::rename(t, f);
            }
            let finished: Vec<PathBuf> = done.iter().map(|(f, _)| f.clone()).collect();
            let rest: Vec<(PathBuf, PathBuf)> = parked.iter().filter(|(o, _)| !finished.contains(o)).cloned().collect();
            undo_parked(&rest);
            return Err(io::Error::new(e.kind(), format!("{}: {e}", to.display())));
        }
        done.push((from.clone(), to.clone()));
    }
    Ok(done)
}

/// The folder of each file, for checking names folder by folder.
pub fn folder_of(p: &Path) -> PathBuf {
    p.parent().map(Path::to_path_buf).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(names: &[&str]) -> Vec<Source> {
        names.iter().map(|n| Source { name: n.to_string(), mtime: 1_700_000_000 }).collect()
    }

    #[test]
    fn templates_counters_and_case() {
        let s = src(&["IMG_0001.JPG", "IMG_0002.JPG", "notes.tar.gz"]);
        let r = Rule { template: "Urlaub {n:03}{ext}".into(), start: 7, ..Default::default() };
        assert_eq!(plan(&r, &s).unwrap(), ["Urlaub 007.JPG", "Urlaub 008.JPG", "Urlaub 009.tar.gz"]);
        let r = Rule { template: "{name}{ext}".into(), case: Case::Lower, ..Default::default() };
        assert_eq!(plan(&r, &s).unwrap()[0], "img_0001.jpg");
        let r = Rule { case: Case::Title, ..Default::default() };
        assert_eq!(plan(&r, &src(&["my HOLIDAY photo.JPG"])).unwrap(), ["My Holiday Photo.JPG"]);
        let r = Rule { template: "{date} {name}{ext} {unknown}".into(), ..Default::default() };
        let out = plan(&r, &src(&["a.txt"])).unwrap();
        assert!(out[0].starts_with("2023-11-1") && out[0].ends_with(" a.txt {unknown}"), "{out:?}");
        assert_eq!(plan(&Rule { template: "{n}-{n:2}".into(), ..Default::default() }, &src(&["x"])).unwrap(), ["1-01"]);
    }

    #[test]
    fn find_and_replace() {
        let s = src(&["IMG_0001.jpg", "img_0002.jpg"]);
        let r = Rule { find: "IMG_".into(), replace: "Photo ".into(), ..Default::default() };
        assert_eq!(plan(&r, &s).unwrap(), ["Photo 0001.jpg", "img_0002.jpg"]);
        let r = Rule { ignore_case: true, ..r };
        assert_eq!(plan(&r, &s).unwrap(), ["Photo 0001.jpg", "Photo 0002.jpg"]);
        // A plain search takes "$1" literally; a regular expression fills it in.
        let r = Rule { find: "0+(\\d)".into(), replace: "#$1".into(), regex: true, ..Default::default() };
        assert_eq!(plan(&r, &s).unwrap(), ["IMG_#1.jpg", "img_#2.jpg"]);
        let r = Rule { find: "(".into(), regex: true, ..Default::default() };
        assert!(plan(&r, &s).is_err());
        let r = Rule { find: "(".into(), replace: "[".into(), ..Default::default() };
        assert_eq!(plan(&r, &src(&["a (1).txt"])).unwrap(), ["a [1).txt"]);
    }

    #[test]
    fn problems_per_row() {
        let old: Vec<String> = ["a.txt", "b.txt", "c.txt"].map(String::from).to_vec();
        let new: Vec<String> = ["b.txt", "a.txt", "taken.txt"].map(String::from).to_vec();
        let p = problems(&old, &new, |n| ["a.txt", "b.txt", "c.txt", "taken.txt"].contains(&n));
        // Swapping a and b is fine; taken.txt is in the way.
        assert_eq!(p[0], None);
        assert_eq!(p[1], None);
        assert!(p[2].as_deref().unwrap().contains("already there"));
        let p = problems(&old, &["x".into(), "x".into(), "".into()], |_| false);
        assert!(p[0].as_deref().unwrap().contains("two files") && p[2].as_deref().unwrap().contains("not a valid"));
    }

    #[test]
    fn swaps_and_rollback() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        for (n, c) in [("a", "A"), ("b", "B"), ("c", "C")] {
            std::fs::write(d.join(n), c).unwrap();
        }
        let done = apply(&[(d.join("a"), "b".into()), (d.join("b"), "a".into()), (d.join("c"), "c".into())]).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("a")).unwrap(), "B");
        assert_eq!(std::fs::read_to_string(d.join("b")).unwrap(), "A");
        assert_eq!(done.len(), 2);
        // Something in the way: nothing changes.
        std::fs::write(d.join("blocker"), "X").unwrap();
        assert!(apply(&[(d.join("a"), "new".into()), (d.join("c"), "blocker".into())]).is_err());
        let mut names: Vec<String> = std::fs::read_dir(d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["a", "b", "blocker", "c"]);
        assert_eq!(std::fs::read_to_string(d.join("c")).unwrap(), "C");
    }
}
