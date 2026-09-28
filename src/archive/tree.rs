//! The folder structure of an archive listing. Many archives store only files, so parent
//! folders are synthesized from the paths.

use super::ArchiveEntry;
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, PartialEq)]
pub struct ArchiveNode {
    /// Full normalized path inside the archive.
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub compressed: u64,
    pub mtime: Option<i64>,
    pub encrypted: bool,
    /// Synthesized folder without an entry of its own.
    pub implied: bool,
}

#[derive(Debug, Default, Clone)]
pub struct ArchiveTree {
    nodes: HashMap<String, ArchiveNode>,
    /// Folder path ("" is the root) → child names, sorted.
    children: HashMap<String, BTreeMap<String, ()>>,
}

fn parent_of(path: &str) -> &str {
    path.rfind('/').map(|i| &path[..i]).unwrap_or("")
}

fn leaf_of(path: &str) -> &str {
    path.rfind('/').map(|i| &path[i + 1..]).unwrap_or(path)
}

impl ArchiveTree {
    pub fn build(entries: &[ArchiveEntry]) -> Self {
        let mut t = ArchiveTree::default();
        t.children.insert(String::new(), BTreeMap::new());
        for e in entries {
            let path = e.name.trim_matches('/');
            if path.is_empty() || path.split('/').any(|p| p.is_empty() || p == "." || p == "..") {
                continue;
            }
            t.ensure_parents(path);
            let node = ArchiveNode {
                path: path.to_string(),
                name: leaf_of(path).to_string(),
                is_dir: e.is_dir,
                size: if e.is_dir { 0 } else { e.size },
                compressed: e.compressed,
                mtime: e.mtime,
                encrypted: e.encrypted,
                implied: false,
            };
            if node.is_dir {
                t.children.entry(path.to_string()).or_default();
            }
            // A folder entry must not be downgraded by a later file of the same name, and a real
            // entry replaces an implied folder.
            match t.nodes.get(path) {
                Some(existing) if existing.is_dir && !node.is_dir && !existing.implied => {}
                _ => {
                    t.nodes.insert(path.to_string(), node);
                }
            }
            t.children.entry(parent_of(path).to_string()).or_default().insert(leaf_of(path).to_string(), ());
        }
        t
    }

    fn ensure_parents(&mut self, path: &str) {
        let mut parent = parent_of(path);
        let mut chain = Vec::new();
        while !parent.is_empty() {
            chain.push(parent.to_string());
            parent = parent_of(parent);
        }
        for dir in chain.into_iter().rev() {
            if !self.nodes.contains_key(&dir) {
                self.nodes.insert(
                    dir.clone(),
                    ArchiveNode {
                        path: dir.clone(),
                        name: leaf_of(&dir).to_string(),
                        is_dir: true,
                        size: 0,
                        compressed: 0,
                        mtime: None,
                        encrypted: false,
                        implied: true,
                    },
                );
            }
            self.children.entry(dir.clone()).or_default();
            self.children.entry(parent_of(&dir).to_string()).or_default().insert(leaf_of(&dir).to_string(), ());
        }
    }

    pub fn get(&self, path: &str) -> Option<&ArchiveNode> {
        self.nodes.get(path.trim_matches('/'))
    }

    pub fn is_dir(&self, path: &str) -> bool {
        let p = path.trim_matches('/');
        p.is_empty() || self.nodes.get(p).is_some_and(|n| n.is_dir)
    }

    /// Children of a folder; `None` when there is no such folder.
    pub fn children(&self, dir: &str) -> Option<Vec<&ArchiveNode>> {
        let dir = dir.trim_matches('/');
        let names = self.children.get(dir)?;
        Some(
            names
                .keys()
                .filter_map(|n| {
                    let p = if dir.is_empty() { n.clone() } else { format!("{dir}/{n}") };
                    self.nodes.get(&p)
                })
                .collect(),
        )
    }

    /// Total uncompressed size below a folder (or the file size).
    pub fn total_size(&self, path: &str) -> u64 {
        let p = path.trim_matches('/');
        let prefix = format!("{p}/");
        self.nodes
            .values()
            .filter(|n| !n.is_dir && (p.is_empty() || n.path == p || n.path.starts_with(&prefix)))
            .map(|n| n.size)
            .sum()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(name: &str, dir: bool, size: u64) -> ArchiveEntry {
        ArchiveEntry {
            raw: name.into(),
            name: name.into(),
            is_dir: dir,
            size,
            compressed: size,
            mtime: None,
            encrypted: false,
            method: String::new(),
        }
    }

    #[test]
    fn synthesizes_missing_parents() {
        let t = ArchiveTree::build(&[e("a/b/c.txt", false, 3), e("top.txt", false, 1)]);
        let root: Vec<_> = t.children("").unwrap().iter().map(|n| n.name.clone()).collect();
        assert_eq!(root, vec!["a", "top.txt"]);
        assert!(t.get("a").unwrap().implied);
        assert!(t.is_dir("a/b"));
        assert_eq!(t.children("a/b").unwrap()[0].path, "a/b/c.txt");
        assert_eq!(t.total_size(""), 4);
        assert_eq!(t.total_size("a"), 3);
    }

    #[test]
    fn explicit_folder_replaces_implied_one() {
        let t = ArchiveTree::build(&[e("a/x", false, 1), e("a", true, 0)]);
        assert!(!t.get("a").unwrap().implied);
        assert_eq!(t.children("").unwrap().len(), 1);
    }

    #[test]
    fn rejects_traversal_and_empty_components() {
        let t = ArchiveTree::build(&[e("../evil", false, 1), e("a//b", false, 1), e("ok", false, 1)]);
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn unknown_folder_has_no_children() {
        let t = ArchiveTree::build(&[e("a", false, 1)]);
        assert!(t.children("nope").is_none());
        assert!(t.children("").is_some());
    }

    #[test]
    fn prefix_sizes_do_not_leak_to_siblings() {
        let t = ArchiveTree::build(&[e("ab/x", false, 5), e("a/y", false, 2)]);
        assert_eq!(t.total_size("a"), 2);
    }
}
