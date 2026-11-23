//! The file list's rows: changed paths as a folder tree, folders before files, each sorted by
//! name, as the browser's tree sorts them (`packages/ui/utils/diffFileTree.ts`). A chain of
//! folders that hold nothing but one folder is one row, `crates/plannotator-tui/src/`, as
//! the browser and VS Code compact them. The diff shows its files in the same order, so the
//! list and `n`/`N` agree.

use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Node {
    Folder {
        expanded: bool,
    },
    /// A file, by its index in the paths the rows were built from.
    File(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) depth: usize,
    /// A folder's path below its parent row, ending in `/`; a file's name.
    pub(crate) label: String,
    /// What the row is known by from one build to the next: a folder's whole path ending in
    /// `/`, a file's path.
    pub(crate) key: String,
    pub(crate) node: Node,
}

#[derive(Debug, Default)]
struct Folder<'a> {
    folders: BTreeMap<&'a str, Folder<'a>>,
    files: Vec<(&'a str, usize)>,
}

/// The rows for `paths`, with the folders in `collapsed` (by key) closed.
pub(crate) fn rows(paths: &[&str], collapsed: &BTreeSet<String>) -> Vec<Row> {
    let mut root = Folder::default();
    for (index, path) in paths.iter().enumerate() {
        let mut segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let Some(name) = segments.pop() else { continue };
        let folder = segments
            .into_iter()
            .fold(&mut root, |folder, segment| folder.folders.entry(segment).or_default());
        folder.files.push((name, index));
    }
    let mut rows = Vec::new();
    walk(&root, "", 0, collapsed, &mut rows);
    rows
}

/// The file indexes of `paths` in the order the rows list them, every folder open.
pub(crate) fn order(paths: &[&str]) -> Vec<usize> {
    rows(paths, &BTreeSet::new())
        .into_iter()
        .filter_map(|row| match row.node {
            Node::File(index) => Some(index),
            Node::Folder { .. } => None,
        })
        .collect()
}

/// Names as the browser's `localeCompare` roughly orders them: case aside first.
fn by_name(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(b))
}

fn walk(folder: &Folder<'_>, prefix: &str, depth: usize, collapsed: &BTreeSet<String>, rows: &mut Vec<Row>) {
    let mut folders: Vec<(&str, &Folder<'_>)> = folder.folders.iter().map(|(name, f)| (*name, f)).collect();
    folders.sort_by(|a, b| by_name(a.0, b.0));
    for (name, mut child) in folders {
        let mut label = format!("{name}/");
        while child.files.is_empty()
            && child.folders.len() == 1
            && let Some((only, grandchild)) = child.folders.iter().next()
        {
            label = format!("{label}{only}/");
            child = grandchild;
        }
        let key = format!("{prefix}{label}");
        let expanded = !collapsed.contains(&key);
        rows.push(Row { depth, label, key: key.clone(), node: Node::Folder { expanded } });
        if expanded {
            walk(child, &key, depth + 1, collapsed, rows);
        }
    }
    let mut files = folder.files.clone();
    files.sort_by(|a, b| by_name(a.0, b.0));
    for (name, index) in files {
        rows.push(Row {
            depth,
            label: name.to_owned(),
            key: format!("{prefix}{name}"),
            node: Node::File(index),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each row as `indent + label`, two spaces a level.
    fn drawn(paths: &[&str], collapsed: &[&str]) -> Vec<String> {
        let collapsed = collapsed.iter().map(|s| (*s).to_owned()).collect();
        rows(paths, &collapsed).iter().map(|r| format!("{}{}", "  ".repeat(r.depth), r.label)).collect()
    }

    #[test]
    fn a_chain_of_single_folders_is_one_row() {
        let paths = [
            "crates/plannotator-tui/src/review/draw.rs",
            "crates/plannotator-tui/src/review/files/kind.rs",
            "crates/plannotator-tui/src/main.rs",
            "README.md",
        ];
        assert_eq!(
            drawn(&paths, &[]),
            [
                "crates/plannotator-tui/src/",
                "  review/",
                "    files/",
                "      kind.rs",
                "    draw.rs",
                "  main.rs",
                "README.md",
            ]
        );
    }

    #[test]
    fn a_folder_with_a_file_beside_its_one_folder_is_not_merged() {
        let paths = ["a/b/c/one.rs", "a/top.rs"];
        assert_eq!(drawn(&paths, &[]), ["a/", "  b/c/", "    one.rs", "  top.rs"]);
    }

    #[test]
    fn folders_come_before_files_and_names_sort_without_regard_to_case() {
        let paths = ["src/b.rs", "src/A.rs", "Zeta/x.rs", "alpha/y.rs", "src/util/z.rs"];
        assert_eq!(
            drawn(&paths, &[]),
            ["alpha/", "  y.rs", "src/", "  util/", "    z.rs", "  A.rs", "  b.rs", "Zeta/", "  x.rs"]
        );
    }

    #[test]
    fn a_collapsed_folder_hides_its_rows_and_keeps_its_key() {
        let paths = ["web/src/app.ts", "web/src/view.ts", "web/README.md"];
        let rows = rows(&paths, &["web/src/".to_owned()].into());
        let keys: Vec<&str> = rows.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys, ["web/", "web/src/", "web/README.md"]);
        assert_eq!(rows.get(1).map(|r| r.node), Some(Node::Folder { expanded: false }));
    }

    #[test]
    fn the_order_is_the_rows_order_whatever_is_collapsed() {
        let paths = ["README.md", "src/main.rs", "src/a/b.rs"];
        assert_eq!(order(&paths), [2, 1, 0]);
    }
}
