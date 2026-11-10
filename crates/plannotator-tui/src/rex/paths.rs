//! `plannotator-tui rex find`: the Markdown files a block's output names, newest first.

use std::path::{Path, PathBuf};

/// Characters that end a path in prose: whitespace, quotes, brackets and the markup around a
/// link or an inline-code span.
fn is_boundary(c: char) -> bool {
    c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '<' | '>' | ',' | '|' | '*')
}

/// The existing `.md` files `text` names, newest mention first and each once. A relative path
/// or one starting with `~/` resolves against `cwd` or `home`; a path that is not a file is
/// dropped, so a sentence that ends in `notes.md.` or a path from another machine never wins.
pub(crate) fn markdown_files(text: &str, cwd: Option<&Path>, home: Option<&Path>) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for token in text.split(is_boundary).rev() {
        let token = token.trim_end_matches(['.', ':', ';', '!', '?']);
        if !Path::new(token).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("md")) {
            continue;
        }
        let path = match (token.strip_prefix("~/"), home) {
            (Some(rest), Some(home)) => home.join(rest),
            (Some(_), None) => continue,
            (None, _) => PathBuf::from(token),
        };
        let path = match cwd {
            Some(cwd) if path.is_relative() => cwd.join(path),
            _ => path,
        };
        if path.is_file() && !found.contains(&path) {
            found.push(path);
        }
    }
    found
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    fn scratch(tag: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("plannotator-tui-paths-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        Scratch(dir)
    }

    #[test]
    fn the_newest_existing_file_comes_first_and_prose_punctuation_is_dropped() {
        let dir = scratch("order");
        for name in ["old.md", "new.md"] {
            std::fs::write(dir.path().join(name), "# x").expect("write");
        }
        let root = dir.path().display();
        let text =
            format!("see {root}/old.md, then `{root}/new.md`.\nnot here: {root}/gone.md and {root}/old.md");
        let found = markdown_files(&text, None, None);
        assert_eq!(found, vec![dir.path().join("old.md"), dir.path().join("new.md")]);
    }

    #[test]
    fn relative_and_home_paths_resolve_against_the_blocks_folder_and_home() {
        let dir = scratch("relative");
        std::fs::create_dir(dir.path().join("docs")).expect("mkdir");
        std::fs::write(dir.path().join("docs/plan.md"), "# x").expect("write");
        let found = markdown_files("docs/plan.md ~/docs/plan.md", Some(dir.path()), Some(dir.path()));
        assert_eq!(found, vec![dir.path().join("docs/plan.md")]);
    }
}
