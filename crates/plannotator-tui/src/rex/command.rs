//! The `rex` binary, scoped to one session.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use serde_json::Value;

/// The exit status `rex` uses when a session or block it was given does not exist.
const NOT_FOUND: i32 = 3;

/// Why a `rex` call failed.
#[derive(Debug)]
pub(crate) enum RexError {
    /// The session or block is gone.
    NotFound(String),
    /// `rex` could not be started, or it failed for another reason.
    Failed(anyhow::Error),
}

impl std::fmt::Display for RexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(msg) => f.write_str(msg),
            Self::Failed(err) => write!(f, "{err:#}"),
        }
    }
}

impl std::error::Error for RexError {}

/// The `rex` to run. Inside a Rex terminal that is `$REX_BIN_DIR/rex`, the server's own copy.
/// A Rex action runs commands with a bare `PATH` and no `REX_BIN_DIR`, but its parent is the
/// server, so `parent` names that binary. Anywhere else, `rex` on `PATH`.
fn bin(env: impl Fn(&str) -> Option<String>, parent: impl FnOnce() -> Option<PathBuf>) -> PathBuf {
    env("REX_BIN_DIR")
        .filter(|dir| !dir.is_empty())
        .map(|dir| PathBuf::from(dir).join("rex"))
        .or_else(parent)
        .unwrap_or_else(|| PathBuf::from("rex"))
}

/// The parent process's executable, when it is a `rex`.
fn parent_rex() -> Option<PathBuf> {
    let exe = parent_exe()?;
    (exe.file_name()? == "rex").then_some(exe)
}

#[cfg(unix)]
fn parent_exe() -> Option<PathBuf> {
    let ppid = std::os::unix::process::parent_id();
    if let Ok(exe) = std::fs::read_link(format!("/proc/{ppid}/exe")) {
        return Some(exe);
    }
    // macOS has no /proc; `ps` prints the full path of the executable.
    let output = Command::new("/bin/ps").args(["-o", "comm=", "-p", &ppid.to_string()]).output().ok()?;
    let path = String::from_utf8(output.stdout).ok()?;
    let path = path.trim();
    (output.status.success() && path.starts_with('/')).then(|| PathBuf::from(path))
}

#[cfg(not(unix))]
fn parent_exe() -> Option<PathBuf> {
    None
}

/// The last six characters of a Rex ID, the short form `rex ls` prints.
pub(crate) fn short_id(id: &str) -> &str {
    id.char_indices().rev().nth(5).map_or(id, |(at, _)| id.get(at..).unwrap_or(id))
}

/// `rex`, with every call scoped to `session`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rex {
    bin: PathBuf,
    session: String,
}

impl Rex {
    /// `rex` as `bin` finds it, scoped to `session` when given, else `REX_SESSION`.
    pub(crate) fn from_lookup(env: impl Fn(&str) -> Option<String>, session: Option<String>) -> Result<Self> {
        let non_empty = |k: &str| env(k).filter(|v| !v.is_empty());
        let session = session
            .or_else(|| non_empty("REX_SESSION"))
            .context("no Rex session: pass --session or run inside Rex (REX_SESSION is not set)")?;
        Ok(Self { bin: bin(&env, parent_rex), session })
    }

    /// Run `rex ARGS -s SESSION`, writing `stdin` to it when given. Returns stdout.
    pub(crate) fn run(&self, args: &[&str], stdin: Option<&str>) -> Result<String, RexError> {
        let mut child = Command::new(&self.bin)
            .args(args)
            .args(["-s", &self.session])
            .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| RexError::Failed(anyhow::anyhow!("cannot run {}: {err}", self.bin.display())))?;
        if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
            pipe.write_all(text.as_bytes()).map_err(|err| RexError::Failed(err.into()))?;
        }
        let output = child.wait_with_output().map_err(|err| RexError::Failed(err.into()))?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = stderr.trim().trim_start_matches("Error: ").to_owned();
        let command = args.iter().take(2).copied().collect::<Vec<_>>().join(" ");
        let message = if message.is_empty() { format!("rex {command} failed") } else { message };
        Err(match output.status.code() {
            Some(NOT_FOUND) => RexError::NotFound(message),
            _ => RexError::Failed(anyhow::anyhow!(message)),
        })
    }

    /// Call one of a terminal block's methods, such as `process` or `program_status`.
    pub(crate) fn terminal_method(&self, block: &str, method: &str) -> Result<Value, RexError> {
        let method = format!("com.superlogical.terminal.{method}");
        json(&method, &self.run(&["do", &method, "-b", block], None)?)
    }

    /// Call a server API method with `args`, sent on stdin so no text lands in `ps`.
    pub(crate) fn api(&self, method: &str, args: &Value) -> Result<Value, RexError> {
        json(method, &self.run(&["api", "call", method, "-"], Some(&args.to_string()))?)
    }
}

/// What `method` printed, as JSON.
fn json(method: &str, stdout: &str) -> Result<Value, RexError> {
    serde_json::from_str(stdout).map_err(|err| {
        RexError::Failed(anyhow::anyhow!("{method} returned something other than JSON: {err}"))
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    fn rex(vars: &[(&str, &str)], session: Option<&str>) -> Result<Rex> {
        let owned: Vec<(String, String)> =
            vars.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
        Rex::from_lookup(
            |k| owned.iter().find(|(name, _)| name == k).map(|(_, v)| v.clone()),
            session.map(str::to_owned),
        )
    }

    #[test]
    fn the_terminals_rex_then_the_parent_rex_then_path() {
        let server = || Some(PathBuf::from("/app/MacOS/rex"));
        let in_terminal = |k: &str| (k == "REX_BIN_DIR").then(|| "/opt/rex/bin".to_owned());
        assert_eq!(bin(in_terminal, server), PathBuf::from("/opt/rex/bin/rex"));
        assert_eq!(bin(|_: &str| None, server), PathBuf::from("/app/MacOS/rex"));
        assert_eq!(bin(|_: &str| None, || None), PathBuf::from("rex"));
    }

    #[test]
    fn a_short_id_is_the_last_six_characters() {
        assert_eq!(short_id("block:01a17dd5-e47c-754f-b72e-ef739ea1d6db"), "a1d6db");
        assert_eq!(short_id("abc"), "abc");
    }

    #[test]
    fn an_explicit_session_wins_and_none_at_all_is_an_error() {
        let explicit = rex(&[("REX_SESSION", "session:a")], Some("session:b")).expect("rex");
        assert_eq!(explicit.session, "session:b");
        assert!(rex(&[], None).is_err());
    }
}
