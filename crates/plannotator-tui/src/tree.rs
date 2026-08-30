//! A folder of Markdown files, as a flat list of tree rows for the left pane.
//!
//! Directories are walked eagerly (docs folders are small), hidden entries and non-Markdown
//! files are skipped, and empty directories are pruned. Rows carry their depth so the view
//! can indent; the tree is otherwise just an ordered list.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub(crate) struct Row {
    pub(crate) path: PathBuf,
    pub(crate) name: String,
    pub(crate) depth: usize,
    pub(crate) is_dir: bool,
    /// Annotations recorded for this file; for a directory, the sum beneath it.
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

impl Tree {
    pub(crate) fn scan(root: &Path) -> Result<Self> {
        let mut rows = Vec::new();
        let mut budget = WALK_BUDGET;
        walk(root, 0, &mut rows, &mut budget)?;
        Ok(Self { root: root.to_path_buf(), rows })
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// Index of the row for `path`, if it is in the tree.
    pub(crate) fn position(&self, path: &Path) -> Option<usize> {
        self.rows.iter().position(|r| r.path == path)
    }

    /// The first file in the tree, in display order.
    pub(crate) fn first_file(&self) -> Option<&Row> {
        self.rows.iter().find(|r| !r.is_dir)
    }

    /// Set each file's annotation count from `count`, then roll sums up into directories.
    #[allow(clippy::indexing_slicing, reason = "indices come from enumerate over the same vec")]
    pub(crate) fn set_counts(&mut self, count: impl Fn(&Path) -> usize) {
        for row in &mut self.rows {
            row.annotations = if row.is_dir { 0 } else { count(&row.path) };
        }
        // Rows are in walk order: a directory precedes its descendants, which all have a
        // greater depth until the next row at the directory's depth or shallower.
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

/// Directories that hold dependencies or build output, never docs worth listing.
const SKIPPED_DIRS: [&str; 8] =
    ["node_modules", "target", "vendor", "dist", "build", "out", "__pycache__", "venv"];

/// The walk visits at most this many directory entries; a folder bigger than that is not a
/// docs folder, and scanning it eagerly would hold a blank pane for minutes.
const WALK_BUDGET: usize = 50_000;

/// Append `dir`'s markdown files and non-empty subdirectories to `rows`, files first.
/// Dependency and build directories are skipped, symlinked directories are not followed
/// (cycles), and the walk stops with an error when `budget` runs out.
fn walk(dir: &Path, depth: usize, rows: &mut Vec<Row>, budget: &mut usize) -> Result<()> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| !is_hidden(p))
        .collect();
    entries.sort();
    *budget = budget.checked_sub(entries.len()).with_context(|| {
        format!(
            "{} holds too many files to scan (over {WALK_BUDGET}); open a docs subfolder instead",
            dir.display()
        )
    })?;

    for path in entries.iter().filter(|p| p.is_file() && is_markdown(p)) {
        rows.push(Row { name: file_name(path), path: path.clone(), depth, is_dir: false, annotations: 0 });
    }
    for path in entries.iter().filter(|p| is_walkable_dir(p)) {
        let mark = rows.len();
        rows.push(Row { name: file_name(path), path: path.clone(), depth, is_dir: true, annotations: 0 });
        walk(path, depth + 1, rows, budget)?;
        if rows.len() == mark + 1 {
            rows.pop(); // no markdown beneath: prune the directory row
        }
    }
    Ok(())
}

/// A real (non-symlinked) directory that is not a dependency or build tree.
fn is_walkable_dir(path: &Path) -> bool {
    let by_name = path.file_name().and_then(|n| n.to_str()).is_none_or(|n| !SKIPPED_DIRS.contains(&n));
    by_name && std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_dir())
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    #[test]
    fn lists_markdown_files_prunes_empty_dirs_and_skips_hidden() {
        let root = std::env::temp_dir().join(format!("plannotator-tui-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("docs/deep")).expect("mkdir");
        std::fs::create_dir_all(root.join("empty")).expect("mkdir");
        std::fs::create_dir_all(root.join(".hidden")).expect("mkdir");
        std::fs::write(root.join("b.md"), "").expect("write");
        std::fs::write(root.join("a.MD"), "").expect("write");
        std::fs::write(root.join("notes.txt"), "").expect("write");
        std::fs::write(root.join("docs/deep/plan.md"), "").expect("write");
        std::fs::write(root.join(".hidden/x.md"), "").expect("write");
        std::fs::create_dir_all(root.join("node_modules/pkg")).expect("mkdir");
        std::fs::write(root.join("node_modules/pkg/readme.md"), "").expect("write");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&root, root.join("loop")).expect("symlink");

        let tree = Tree::scan(&root).expect("scan");
        let shape: Vec<(String, usize, bool)> =
            tree.rows.iter().map(|r| (r.name.clone(), r.depth, r.is_dir)).collect();
        assert_eq!(
            shape,
            [
                ("a.MD".to_owned(), 0, false),
                ("b.md".to_owned(), 0, false),
                ("docs".to_owned(), 0, true),
                ("deep".to_owned(), 1, true),
                ("plan.md".to_owned(), 2, false),
            ]
        );
        assert_eq!(tree.first_file().map(|r| r.name.as_str()), Some("a.MD"));

        let mut tree = tree;
        tree.set_counts(|p| usize::from(p.ends_with("plan.md")) * 3 + usize::from(p.ends_with("b.md")));
        let counts: Vec<usize> = tree.rows.iter().map(|r| r.annotations).collect();
        assert_eq!(counts, [0, 1, 3, 3, 3], "directories sum their descendants");
        std::fs::remove_dir_all(&root).expect("cleanup");
    }
}
