//! `plannotator-tui pi install`: copy the Pi link extension into Pi's extensions folder.
//!
//! The extension is embedded in this binary, so running install again after an upgrade
//! keeps the two in step. Pi loads every `.ts` file in `<agent dir>/extensions/`.

#![allow(clippy::print_stdout, reason = "install reports to the person who ran it")]

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// The extension (`docs/spec-pi-link.md`).
pub(crate) const EXTENSION: &str = include_str!("plannotator-tui.ts");
const FILE_NAME: &str = "plannotator-tui.ts";

pub(crate) fn run(args: &[String]) -> Result<()> {
    let (mut dry_run, mut print) = (false, false);
    for arg in args {
        match arg.as_str() {
            "--dry-run" => dry_run = true,
            "--print" => print = true,
            other => anyhow::bail!(
                "unexpected argument {other:?}\nusage: plannotator-tui pi install [--dry-run | --print]"
            ),
        }
    }
    if print {
        print!("{EXTENSION}");
        return Ok(());
    }
    let home = std::env::home_dir().context("finding the home folder")?;
    let target = extension_path(|k| std::env::var(k).ok(), &home);
    let verb = match (std::fs::read_to_string(&target), dry_run) {
        (Ok(existing), _) if existing == EXTENSION => "unchanged",
        (Ok(_), true) => "would update",
        (Err(_), true) => "would create",
        (existing, false) => {
            let dir = target.parent().context("the extension path has no folder")?;
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
            std::fs::write(&target, EXTENSION).with_context(|| format!("writing {}", target.display()))?;
            if existing.is_ok() { "updated" } else { "created" }
        }
    };
    println!("{verb} {}", target.display());
    if !dry_run && verb != "unchanged" {
        println!("Restart your Pi sessions, or run /reload in each, to load it.");
    }
    Ok(())
}

/// `<Pi agent dir>/extensions/plannotator-tui.ts`. The agent dir is
/// `PI_CODING_AGENT_DIR` (a leading `~` expands), else `~/.pi/agent`.
pub(crate) fn extension_path(env: impl Fn(&str) -> Option<String>, home: &Path) -> PathBuf {
    let agent_dir = match env("PI_CODING_AGENT_DIR").map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()) {
        Some(dir) if dir == "~" => home.to_path_buf(),
        Some(dir) => match dir.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None => PathBuf::from(dir),
        },
        None => home.join(".pi").join("agent"),
    };
    agent_dir.join("extensions").join(FILE_NAME)
}
