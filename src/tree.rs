use std::fs;
use std::path::PathBuf;

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
}

impl FileTree {
    pub fn new(root: PathBuf) -> Self {
        let nodes = read_dir(&root, 0);

        Self {
            root,
            nodes,
            selected: 0,
            scroll: 0,
        }
    }

    pub fn selected_node(&self) -> Option<&Node> {
        self.nodes.get(self.selected)
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

fn read_dir(dir: &PathBuf, depth: usize) -> Vec<Node> {
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
