//! Process-level facts the store needs: the data directory and the project name.
//!
//! Both are resolved once per run from the environment, the home directory, and `git`,
//! then handed to the pure functions in `plannotui_schema::datadir`.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The Plannotator data directory for this process.
pub(crate) fn data_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from);
    plannotui_schema::data_dir(|k| std::env::var(k).ok(), &home, Path::exists)
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
    plannotui_schema::project_name(toplevel.as_deref(), folder)
}
