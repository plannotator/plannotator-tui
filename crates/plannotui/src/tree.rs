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
}

#[derive(Debug)]
pub(crate) struct Tree {
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
        walk(root, 0, &mut rows)?;
        Ok(Self { rows })
    }

    /// Index of the row for `path`, if it is in the tree.
    pub(crate) fn position(&self, path: &Path) -> Option<usize> {
        self.rows.iter().position(|r| r.path == path)
    }

    /// The first file in the tree, in display order.
    pub(crate) fn first_file(&self) -> Option<&Row> {
        self.rows.iter().find(|r| !r.is_dir)
    }
}

/// Append `dir`'s markdown files and non-empty subdirectories to `rows`, files first.
fn walk(dir: &Path, depth: usize, rows: &mut Vec<Row>) -> Result<()> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| !is_hidden(p))
        .collect();
    entries.sort();

    for path in entries.iter().filter(|p| p.is_file() && is_markdown(p)) {
        rows.push(Row { name: file_name(path), path: path.clone(), depth, is_dir: false });
    }
    for path in entries.iter().filter(|p| p.is_dir()) {
        let mark = rows.len();
        rows.push(Row { name: file_name(path), path: path.clone(), depth, is_dir: true });
        walk(path, depth + 1, rows)?;
        if rows.len() == mark + 1 {
            rows.pop(); // no markdown beneath: prune the directory row
        }
    }
    Ok(())
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    #[test]
    fn lists_markdown_files_prunes_empty_dirs_and_skips_hidden() {
        let root = std::env::temp_dir().join(format!("plannotui-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("docs/deep")).expect("mkdir");
        std::fs::create_dir_all(root.join("empty")).expect("mkdir");
        std::fs::create_dir_all(root.join(".hidden")).expect("mkdir");
        std::fs::write(root.join("b.md"), "").expect("write");
        std::fs::write(root.join("a.MD"), "").expect("write");
        std::fs::write(root.join("notes.txt"), "").expect("write");
        std::fs::write(root.join("docs/deep/plan.md"), "").expect("write");
        std::fs::write(root.join(".hidden/x.md"), "").expect("write");

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
        std::fs::remove_dir_all(&root).expect("cleanup");
    }
}
