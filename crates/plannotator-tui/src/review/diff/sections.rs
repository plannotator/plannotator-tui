//! The capture's patch cut into one section per file, so the diff can show the files a
//! filter lets through, in the list's order.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

/// Each file's section of a patch: from its `diff --git` line to the next one. A path can
/// have two (a file replaced by a symlink is deleted, then added); a section the patch
/// parser cannot name is kept apart, to show whatever the filter.
#[derive(Debug, Default)]
pub(super) struct Sections {
    named: HashMap<String, Vec<Range<usize>>>,
    unnamed: Vec<Range<usize>>,
}

impl Sections {
    pub(super) fn of(patch: &[u8]) -> Self {
        let mut starts = Vec::new();
        let mut offset = 0;
        for line in patch.split_inclusive(|b| *b == b'\n') {
            if line.starts_with(b"diff --git ") {
                starts.push(offset);
            }
            offset += line.len();
        }
        let ends = starts.iter().skip(1).copied().chain([patch.len()]);
        let mut sections = Self::default();
        for (from, to) in starts.iter().copied().zip(ends) {
            let text = String::from_utf8_lossy(patch.get(from..to).unwrap_or_default());
            match plannotator_tui_review::patch::parse(&text).into_iter().next() {
                Some(file) => sections.named.entry(file.path).or_default().push(from..to),
                None => sections.unnamed.push(from..to),
            }
        }
        sections
    }

    /// The sections of `paths`, in that order, each path's in patch order, then the ones
    /// with no name, as one patch.
    pub(super) fn select(&self, patch: &[u8], paths: &[String]) -> Vec<u8> {
        let mut seen = HashSet::new();
        paths
            .iter()
            .filter(|path| seen.insert(path.as_str()))
            .filter_map(|path| self.named.get(path))
            .flatten()
            .chain(&self.unnamed)
            .filter_map(|range| patch.get(range.clone()))
            .flatten()
            .copied()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATCH: &str = "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-a\n+b\n\
                         diff --git a/link b/link\ndeleted file mode 100644\n--- a/link\n+++ /dev/null\n@@ -1 +0,0 @@\n-x\n\
                         diff --git a/link b/link\nnew file mode 120000\n--- /dev/null\n+++ b/link\n@@ -0,0 +1 @@\n+a.txt\n\
                         diff --git ???\n";

    fn select(paths: &[&str]) -> String {
        let paths: Vec<String> = paths.iter().map(|p| (*p).to_owned()).collect();
        String::from_utf8(Sections::of(PATCH.as_bytes()).select(PATCH.as_bytes(), &paths)).unwrap_or_default()
    }

    #[test]
    fn a_path_named_twice_brings_both_sections_once_in_patch_order() {
        let link = select(&["link", "link"]);
        assert_eq!(link.matches("diff --git a/link").count(), 2, "{link}");
        assert!(link.find("deleted file").zip(link.find("new file")).is_some_and(|(d, n)| d < n), "{link}");
        assert!(!link.contains("a/a.txt"), "{link}");
    }

    #[test]
    fn a_section_with_no_name_is_shown_whatever_is_picked() {
        assert!(select(&[]).contains("diff --git ???"));
        let both = select(&["link", "a.txt"]);
        assert!(both.ends_with("diff --git ???\n"), "{both}");
        assert!(
            both.find("a/link").zip(both.find("a/a.txt")).is_some_and(|(l, a)| l < a),
            "in the order asked: {both}"
        );
    }
}
