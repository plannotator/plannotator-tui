//! Process-level facts the store needs: the data directory and the project name.
//!
//! Both are resolved once per run from the environment, the home directory, and `git`,
//! then handed to the pure functions in `plannotator_tui_schema::datadir`.

use std::path::{Component, Path, PathBuf};
use std::process::Command;

/// The path a document is keyed by: absolute and lexically normalized (`.` and `..`
/// folded), symlinks left alone — the same rule as Node's `path.resolve`, which is what
/// Plannotator hashes, so both tools find one record for one spelling of a file.
pub(crate) fn absolute(path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")).join(path)
    };
    let mut out = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// The Plannotator data directory for this process.
pub(crate) fn data_dir() -> PathBuf {
    let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    plannotator_tui_schema::data_dir(|k| std::env::var(k).ok(), &home, Path::exists)
}

/// Plannotator's project name for a folder: the enclosing git repo's name, else the
/// folder's own name.
pub(crate) fn project_name(folder: &Path) -> String {
    let toplevel = Command::new("git")
        .args(["-C", &folder.to_string_lossy(), "rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()));
    plannotator_tui_schema::project_name(toplevel.as_deref(), folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_and_dotted_spellings_resolve_to_one_absolute_path() {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        assert_eq!(absolute(Path::new("docs/plan.md")), cwd.join("docs/plan.md"));
        assert_eq!(absolute(Path::new("./docs/../docs/plan.md")), cwd.join("docs/plan.md"));
        assert_eq!(absolute(Path::new("/a/b/../c/./d.md")), PathBuf::from("/a/c/d.md"));
    }
}
