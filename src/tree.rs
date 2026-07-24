use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// One visible row in the flattened tree view.
pub struct Node {
    pub path: PathBuf,

    pub name: String,

    pub is_dir: bool,

    pub depth: usize,

    pub expanded: bool,
}

/// A lazily-expanded directory tree rendered as a flat, scrollable list. A
/// directory's children are only read from disk the first time it is expanded.
pub struct FileTree {
    pub root: PathBuf,

    pub nodes: Vec<Node>,

    pub selected: usize,

    pub scroll: usize,

    /// Combined mtime of the root and every expanded folder; a change means the
    /// directory listing went stale and needs a re-read.
    fingerprint: u64,
}

impl FileTree {
    pub fn new(root: PathBuf) -> Self {
        let nodes = read_dir(&root, 0);

        let mut tree = Self {
            root,
            nodes,
            selected: 0,
            scroll: 0,
            fingerprint: 0,
        };

        tree.fingerprint = tree.compute_fingerprint();

        tree
    }

    pub fn selected_node(&self) -> Option<&Node> {
        self.nodes.get(self.selected)
    }

    /// Re-read the tree from disk, preserving which folders are expanded and the
    /// selected path. Picks up files created or removed while ocode is open.
    pub fn refresh(&mut self) {
        let expanded: HashSet<PathBuf> = self
            .nodes
            .iter()
            .filter(|n| n.is_dir && n.expanded)
            .map(|n| n.path.clone())
            .collect();

        let selected_path = self.selected_node().map(|n| n.path.clone());

        let mut nodes = Vec::new();

        build_tree(&self.root, 0, &expanded, &mut nodes);

        self.nodes = nodes;

        self.selected = selected_path
            .and_then(|p| self.nodes.iter().position(|n| n.path == p))
            .unwrap_or_else(|| self.selected.min(self.nodes.len().saturating_sub(1)));

        self.fingerprint = self.compute_fingerprint();
    }

    /// Refresh only if a watched directory changed on disk since the last check.
    /// Returns whether a refresh happened.
    pub fn poll(&mut self) -> bool {
        if self.compute_fingerprint() == self.fingerprint {
            return false;
        }

        self.refresh();

        true
    }

    fn compute_fingerprint(&self) -> u64 {
        let mut acc = dir_mtime(&self.root);

        for node in &self.nodes {
            if node.is_dir && node.expanded {
                acc = acc.wrapping_add(dir_mtime(&node.path));
            }
        }

        acc
    }

    /// Scroll the visible window by `delta` rows without touching the selection,
    /// clamped so the list never scrolls past its last screenful.
    pub fn scroll_view(&mut self, delta: isize, visible: usize) {
        let max = self.nodes.len().saturating_sub(visible.max(1)) as isize;

        self.scroll = (self.scroll as isize + delta).clamp(0, max.max(0)) as usize;
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.nodes.len() {
            self.selected += 1;
        }
    }

    pub fn expand(&mut self) {
        let Some(node) = self.nodes.get(self.selected) else {
            return;
        };

        if !node.is_dir || node.expanded {
            return;
        }

        let children = read_dir(&node.path, node.depth + 1);

        self.nodes[self.selected].expanded = true;

        let insert_at = self.selected + 1;

        for (offset, child) in children.into_iter().enumerate() {
            self.nodes.insert(insert_at + offset, child);
        }
    }

    pub fn collapse(&mut self) {
        let Some(node) = self.nodes.get(self.selected) else {
            return;
        };

        if !node.is_dir {
            return;
        }

        if !node.expanded {
            return;
        }

        let depth = node.depth;

        self.nodes[self.selected].expanded = false;

        let start = self.selected + 1;

        let mut end = start;

        while end < self.nodes.len() && self.nodes[end].depth > depth {
            end += 1;
        }

        self.nodes.drain(start..end);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_picks_up_new_and_removed_files() {
        let dir = std::env::temp_dir().join(format!("ocode_tree_refresh_{}", std::process::id()));

        let _ = fs::remove_dir_all(&dir);

        fs::create_dir_all(&dir).unwrap();

        fs::write(dir.join("a.txt"), "a").unwrap();

        let mut tree = FileTree::new(dir.clone());

        assert!(tree.nodes.iter().any(|n| n.name == "a.txt"));

        assert!(!tree.nodes.iter().any(|n| n.name == "b.txt"));

        fs::write(dir.join("b.txt"), "b").unwrap();

        fs::remove_file(dir.join("a.txt")).unwrap();

        tree.refresh();

        assert!(tree.nodes.iter().any(|n| n.name == "b.txt"), "new file appears");

        assert!(!tree.nodes.iter().any(|n| n.name == "a.txt"), "removed file is gone");

        fs::remove_dir_all(&dir).ok();
    }
}

fn build_tree(dir: &Path, depth: usize, expanded: &HashSet<PathBuf>, out: &mut Vec<Node>) {
    for mut node in read_dir(dir, depth) {
        let expand = node.is_dir && expanded.contains(&node.path);

        node.expanded = expand;

        let path = node.path.clone();

        out.push(node);

        if expand {
            build_tree(&path, depth + 1, expanded, out);
        }
    }
}

fn dir_mtime(dir: &Path) -> u64 {
    fs::metadata(dir)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

fn read_dir(dir: &Path, depth: usize) -> Vec<Node> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut nodes: Vec<Node> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();

            let name = entry.file_name().to_str()?.to_string();

            if name == ".git" {
                return None;
            }

            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);

            Some(Node {
                path,
                name,
                is_dir,
                depth,
                expanded: false,
            })
        })
        .collect();

    nodes.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    nodes
}
