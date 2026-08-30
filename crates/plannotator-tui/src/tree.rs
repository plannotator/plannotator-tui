//! A folder of Markdown files, listed lazily: one directory level at a time.
//!
//! Scanning eagerly held a blank pane for minutes on big trees (plannotator-tui#44 review),
//! so the tree lists only the root's entries at open, and a directory's children when it is
//! expanded. Hidden entries, non-Markdown files, dependency/build directories and symlinked
//! directories are skipped. Rows carry their depth and expansion state; the vec stays the
//! flattened visible list, so the view and hit-testing stay a plain slice.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Directories that hold dependencies or build output, never docs worth listing.
const SKIPPED_DIRS: [&str; 8] =
    ["node_modules", "target", "vendor", "dist", "build", "out", "__pycache__", "venv"];

#[derive(Debug, Clone)]
pub(crate) struct Row {
    pub(crate) path: PathBuf,
    pub(crate) name: String,
    pub(crate) depth: usize,
    pub(crate) is_dir: bool,
    /// A directory's children are in the list only while it is expanded.
    pub(crate) expanded: bool,
    /// Annotations recorded for this file; for a directory, the sum of its listed children.
    pub(crate) annotations: usize,
}

#[derive(Debug)]
pub(crate) struct Tree {
    root: PathBuf,
    pub(crate) rows: Vec<Row>,
}

fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "md" | "markdown" | "mdx"))
}

fn is_hidden(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.'))
}

/// A real (non-symlinked) directory that is not a dependency or build tree.
fn is_walkable_dir(path: &Path) -> bool {
    let by_name = path.file_name().and_then(|n| n.to_str()).is_none_or(|n| !SKIPPED_DIRS.contains(&n));
    by_name && std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_dir())
}

/// One directory's rows at `depth`: markdown files first, then subdirectories, both sorted.
fn list(dir: &Path, depth: usize) -> Result<Vec<Row>> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| !is_hidden(p))
        .collect();
    entries.sort();
    let mut rows = Vec::new();
    for path in entries.iter().filter(|p| p.is_file() && is_markdown(p)) {
        rows.push(Row {
            name: file_name(path),
            path: path.clone(),
            depth,
            is_dir: false,
            expanded: false,
            annotations: 0,
        });
    }
    for path in entries.iter().filter(|p| is_walkable_dir(p)) {
        rows.push(Row {
            name: file_name(path),
            path: path.clone(),
            depth,
            is_dir: true,
            expanded: false,
            annotations: 0,
        });
    }
    Ok(rows)
}

impl Tree {
    /// The root's own entries; nothing beneath is touched until a directory is expanded.
    pub(crate) fn scan(root: &Path) -> Result<Self> {
        Ok(Self { root: root.to_path_buf(), rows: list(root, 0)? })
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// Index of the row for `path`, if it is currently listed.
    pub(crate) fn position(&self, path: &Path) -> Option<usize> {
        self.rows.iter().position(|r| r.path == path)
    }

    /// The first listed file, in display order.
    #[cfg(test)]
    pub(crate) fn first_file(&self) -> Option<&Row> {
        self.rows.iter().find(|r| !r.is_dir)
    }

    /// Expand a collapsed directory row (listing its children) or collapse an expanded one
    /// (dropping every deeper row beneath it). Returns whether anything changed.
    pub(crate) fn toggle(&mut self, index: usize) -> Result<bool> {
        let Some(row) = self.rows.get(index) else { return Ok(false) };
        if !row.is_dir {
            return Ok(false);
        }
        let (depth, path, expanded) = (row.depth, row.path.clone(), row.expanded);
        if expanded {
            let end = self.end_of_subtree(index);
            self.rows.drain(index + 1..end);
        } else {
            let children = list(&path, depth + 1)?;
            let at = index + 1;
            self.rows.splice(at..at, children);
        }
        if let Some(row) = self.rows.get_mut(index) {
            row.expanded = !expanded;
        }
        Ok(true)
    }

    /// The exclusive end of the rows beneath `index` (rows strictly deeper than it).
    fn end_of_subtree(&self, index: usize) -> usize {
        let depth = self.rows.get(index).map_or(0, |r| r.depth);
        self.rows
            .iter()
            .enumerate()
            .skip(index + 1)
            .find(|(_, r)| r.depth <= depth)
            .map_or(self.rows.len(), |(i, _)| i)
    }

    /// Set each listed file's annotation count from `count`, then roll sums up into the
    /// listed directories (an unexpanded directory sums nothing; expand to see beneath it).
    #[allow(clippy::indexing_slicing, reason = "indices come from enumerate over the same vec")]
    pub(crate) fn set_counts(&mut self, count: impl Fn(&Path) -> usize) {
        for row in &mut self.rows {
            row.annotations = if row.is_dir { 0 } else { count(&row.path) };
        }
        for i in 0..self.rows.len() {
            if !self.rows[i].is_dir {
                continue;
            }
            let depth = self.rows[i].depth;
            let sum: usize = self.rows[i + 1..]
                .iter()
                .take_while(|r| r.depth > depth)
                .filter(|r| !r.is_dir)
                .map(|r| r.annotations)
                .sum();
            self.rows[i].annotations = sum;
        }
    }
}

/// The first markdown file at or near the top of `root`: the shallowest match, found by a
/// breadth-first look that gives up after `budget` entries. Big trees stay fast; the caller
/// shows a placeholder when nothing shallow exists.
pub(crate) fn first_file_shallow(root: &Path, budget: usize) -> Option<PathBuf> {
    let mut queue = std::collections::VecDeque::from([root.to_path_buf()]);
    let mut seen = 0usize;
    while let Some(dir) = queue.pop_front() {
        let Ok(rows) = list(&dir, 0) else { continue };
        seen += rows.len();
        if let Some(file) = rows.iter().find(|r| !r.is_dir) {
            return Some(file.path.clone());
        }
        if seen >= budget {
            return None;
        }
        queue.extend(rows.iter().filter(|r| r.is_dir).map(|r| r.path.clone()));
    }
    None
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    fn fixture(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("plannotator-tui-tree-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("docs/deep")).expect("mkdir");
        std::fs::create_dir_all(root.join("empty")).expect("mkdir");
        std::fs::create_dir_all(root.join(".hidden")).expect("mkdir");
        std::fs::create_dir_all(root.join("node_modules/pkg")).expect("mkdir");
        std::fs::write(root.join("b.md"), "").expect("write");
        std::fs::write(root.join("a.MD"), "").expect("write");
        std::fs::write(root.join("notes.txt"), "").expect("write");
        std::fs::write(root.join("docs/deep/plan.md"), "").expect("write");
        std::fs::write(root.join(".hidden/x.md"), "").expect("write");
        std::fs::write(root.join("node_modules/pkg/readme.md"), "").expect("write");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&root, root.join("loop")).expect("symlink");
        root
    }

    fn shape(tree: &Tree) -> Vec<(String, usize, bool)> {
        tree.rows.iter().map(|r| (r.name.clone(), r.depth, r.is_dir)).collect()
    }

    fn row(name: &str, depth: usize, is_dir: bool) -> (String, usize, bool) {
        (name.to_owned(), depth, is_dir)
    }

    #[test]
    fn the_root_lists_one_level_and_expanding_descends_lazily() {
        let root = fixture("lazy");
        let mut tree = Tree::scan(&root).expect("scan");
        // Top level only: markdown files then real directories; hidden entries, node_modules
        // and the symlink are gone. `empty` shows because nothing beneath it was scanned.
        assert_eq!(
            shape(&tree),
            [row("a.MD", 0, false), row("b.md", 0, false), row("docs", 0, true), row("empty", 0, true)]
        );
        let docs = tree.position(root.join("docs").as_path()).expect("docs row");
        assert!(tree.toggle(docs).expect("expand docs"));
        assert_eq!(shape(&tree)[3], row("deep", 1, true));
        let deep = tree.position(root.join("docs/deep").as_path()).expect("deep row");
        assert!(tree.toggle(deep).expect("expand deep"));
        assert_eq!(shape(&tree)[4], row("plan.md", 2, false));
        // Collapsing docs drops everything beneath it, however deep.
        assert!(tree.toggle(docs).expect("collapse docs"));
        assert_eq!(
            shape(&tree),
            [row("a.MD", 0, false), row("b.md", 0, false), row("docs", 0, true), row("empty", 0, true)]
        );
        // Counts roll up over what is listed.
        assert!(tree.toggle(docs).expect("re-expand docs"));
        let deep = tree.position(root.join("docs/deep").as_path()).expect("deep again");
        assert!(tree.toggle(deep).expect("re-expand deep"));
        tree.set_counts(|p| usize::from(p.ends_with("plan.md")) * 3 + usize::from(p.ends_with("b.md")));
        let counts: Vec<usize> = tree.rows.iter().map(|r| r.annotations).collect();
        assert_eq!(counts, [0, 1, 3, 3, 3, 0], "directories sum their listed descendants");
        assert_eq!(tree.first_file().map(|r| r.name.as_str()), Some("a.MD"));
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn toggling_a_file_row_changes_nothing() {
        let root = fixture("file-toggle");
        let mut tree = Tree::scan(&root).expect("scan");
        let before = shape(&tree);
        assert!(!tree.toggle(0).expect("file"));
        assert_eq!(shape(&tree), before);
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn the_shallowest_markdown_is_found_without_walking_everything() {
        let root = fixture("shallow");
        assert_eq!(first_file_shallow(&root, 2000), Some(root.join("a.MD")));
        // A tree whose markdown is only deep inside still resolves breadth-first.
        std::fs::remove_file(root.join("a.MD")).expect("rm");
        std::fs::remove_file(root.join("b.md")).expect("rm");
        assert_eq!(first_file_shallow(&root, 2000), Some(root.join("docs/deep/plan.md")));
        // A zero budget gives up instead of scanning.
        assert_eq!(first_file_shallow(&root, 0), None);
        std::fs::remove_dir_all(&root).expect("cleanup");
    }
}
