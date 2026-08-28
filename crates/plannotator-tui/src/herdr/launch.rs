//! `plannotator-tui herdr open`: resolve what to open, where feedback goes, and how the pane is
//! placed, then run `herdr plugin pane open`. One command for humans (manifest actions)
//! and agents (the skill). `plan` and `argv` are pure; only `run` touches a process.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use super::context::{HerdrEnv, Target};
use crate::config::{Config, Placement, SplitDirection};

/// Command-line inputs to the launcher.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct OpenArgs {
    pub(crate) path: Option<PathBuf>,
    pub(crate) placement: Option<Placement>,
    pub(crate) deliver_to: Option<String>,
}

/// A fully resolved launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Launch {
    /// File or folder plannotator-tui opens.
    pub(crate) file: PathBuf,
    /// The pane's working directory: the file's folder, or the folder itself.
    pub(crate) cwd: PathBuf,
    pub(crate) placement: Placement,
    pub(crate) direction: SplitDirection,
    /// Popup `(width, height)`.
    pub(crate) popup: (String, String),
    /// The pane a split opens next to.
    pub(crate) target_pane: Option<String>,
    pub(crate) deliver: Option<Target>,
    /// The plugin id to open under: whatever plugin ships this binary.
    pub(crate) plugin: String,
}

/// A `file://` URL as a local path; anything else is not ours to open.
fn file_url_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    // `file:///abs` → `/abs`; `file://host/abs` → `/abs` for localhost only.
    let path = match rest.find('/') {
        Some(0) => rest,
        Some(slash) if matches!(&rest[..slash], "" | "localhost") => &rest[slash..],
        _ => return None,
    };
    let decoded = percent_decode(path);
    // Windows: `file:///C:/x` carries a leading slash before the drive letter.
    #[cfg(windows)]
    let decoded = match decoded.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => {
            decoded.get(1..).unwrap_or_default().to_owned()
        }
        _ => decoded,
    };
    Some(PathBuf::from(decoded))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let decoded = (bytes.get(i) == Some(&b'%'))
            .then(|| s.get(i + 1..i + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        if let Some(byte) = decoded {
            out.push(byte);
            i += 3;
        } else {
            out.extend(bytes.get(i..=i).unwrap_or_default());
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Resolve every input per the contract. `cwd` is the launcher process's directory.
pub(crate) fn plan(env: &HerdrEnv, config: &Config, args: OpenArgs, cwd: &Path) -> Result<Launch> {
    let context = env.context.as_ref();
    let placement = match (args.placement, env.placement.as_deref()) {
        (Some(p), _) => p,
        (None, Some(text)) => text.parse().context("PLANNOTATOR_TUI_PLACEMENT")?,
        (None, None) => config.herdr.placement,
    };

    let file = args
        .path
        .or_else(|| context.and_then(|c| c.clicked_url.as_deref()).and_then(file_url_path))
        .or_else(|| context.and_then(|c| c.focused_pane_cwd.clone()).map(PathBuf::from))
        .or_else(|| context.and_then(|c| c.workspace_cwd.clone()).map(PathBuf::from))
        .unwrap_or_else(|| cwd.to_path_buf());
    let file = if file.is_absolute() { file } else { cwd.join(file) };
    let dir = if file.is_dir() {
        file.clone()
    } else {
        file.parent().map_or_else(|| cwd.to_path_buf(), Path::to_path_buf)
    };

    let deliver = match args.deliver_to {
        Some(pane) => Some(Target { pane, agent: None }),
        // With a context (a manifest action) the focused pane counts only when an agent
        // runs there. Without one, the caller is an agent running the skill from its own
        // pane, and HERDR_PANE_ID is that pane.
        None if env.context.is_some() => env.focused_agent_pane(),
        None => env.pane_id.clone().map(|pane| Target { pane, agent: None }),
    };
    let target_pane = deliver
        .as_ref()
        .map(|t| t.pane.clone())
        .or_else(|| context.and_then(|c| c.focused_pane_id.clone()))
        .or_else(|| env.pane_id.clone());

    Ok(Launch {
        file,
        cwd: dir,
        placement,
        direction: config.herdr.split_direction,
        popup: (config.herdr.popup_width.clone(), config.herdr.popup_height.clone()),
        target_pane,
        deliver,
        plugin: env.plugin_id.clone().unwrap_or_else(|| "plannotator-tui".to_owned()),
    })
}

/// The `herdr` arguments for a launch.
pub(crate) fn argv(launch: &Launch) -> Vec<String> {
    let mut out: Vec<String> = ["plugin", "pane", "open", "--plugin", &launch.plugin, "--entrypoint", "doc"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    out.extend(["--placement".to_owned(), launch.placement.to_string()]);
    match launch.placement {
        Placement::Split => {
            out.extend(["--direction".to_owned(), launch.direction.to_string()]);
            if let Some(pane) = &launch.target_pane {
                out.extend(["--target-pane".to_owned(), pane.clone()]);
            }
        }
        Placement::Popup => {
            out.extend(["--width".to_owned(), launch.popup.0.clone()]);
            out.extend(["--height".to_owned(), launch.popup.1.clone()]);
        }
        Placement::Overlay => {}
    }
    out.push("--focus".to_owned());
    out.extend(["--cwd".to_owned(), launch.cwd.display().to_string()]);
    out.extend(["--env".to_owned(), format!("PLANNOTATOR_TUI_FILE={}", launch.file.display())]);
    if let Some(target) = &launch.deliver {
        out.extend(["--env".to_owned(), format!("PLANNOTATOR_TUI_DELIVER_TO={}", target.pane)]);
        if let Some(agent) = &target.agent {
            out.extend(["--env".to_owned(), format!("PLANNOTATOR_TUI_DELIVER_AGENT={agent}")]);
        }
    }
    out
}

/// Run the launch through `bin`. Herdr's own stdout/stderr pass through.
pub(crate) fn run(env: &HerdrEnv, launch: &Launch) -> Result<()> {
    if !env.in_herdr {
        anyhow::bail!("not inside Herdr (HERDR_ENV is not set)");
    }
    let status = Command::new(&env.bin)
        .args(argv(launch))
        .status()
        .with_context(|| format!("running {}", env.bin.display()))?;
    if !status.success() {
        anyhow::bail!("herdr plugin pane open failed with {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
