//! The `hc` binary: herdr-core's CLI.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::Result;
use serde_json::Value;

/// `PLANNOTATOR_TUI_HC_BIN`: the `hc` the launcher found. A terminal the core starts may carry
/// a shorter `PATH` than the shell that asked for the review.
pub(crate) const BIN_VAR: &str = "PLANNOTATOR_TUI_HC_BIN";

/// Why an `hc` call failed.
#[derive(Debug)]
pub(crate) enum HcError {
    /// `hc send` refused: the program is at a dialog, and typing could answer it.
    Blocked(String),
    /// The terminal is gone or finished, or `hc` could not be started.
    Gone(String),
    /// `hc run` was given a name a live terminal already holds.
    Taken(String),
    /// Anything else, with whatever `hc` said.
    Failed(anyhow::Error),
}

impl std::fmt::Display for HcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Blocked(msg) | Self::Gone(msg) | Self::Taken(msg) => f.write_str(msg),
            Self::Failed(err) => write!(f, "{err:#}"),
        }
    }
}

impl std::error::Error for HcError {}

/// `hc`, run as the terminal already has it set up: `HC_TERMINAL_SOCKET` and `HC_SESSION` pick
/// the core, so nothing here names one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Hc {
    bin: PathBuf,
}

impl Hc {
    /// The `hc` named by [`BIN_VAR`], else `hc` on `PATH`.
    pub(crate) fn from_lookup(env: impl Fn(&str) -> Option<String>) -> Self {
        let bin = env(BIN_VAR).filter(|v| !v.is_empty()).map_or_else(|| PathBuf::from("hc"), PathBuf::from);
        Self { bin }
    }

    pub(crate) fn bin(&self) -> &std::path::Path {
        &self.bin
    }

    /// Run `hc ARGS` and return its stdout.
    pub(crate) fn run<S: AsRef<std::ffi::OsStr>>(&self, args: &[S]) -> Result<String, HcError> {
        let output = Command::new(&self.bin).args(args).stdin(Stdio::null()).output().map_err(|err| {
            match err.kind() {
                // Text travels as one argument; the OS caps its size.
                std::io::ErrorKind::ArgumentListTooLong => {
                    HcError::Failed(anyhow::anyhow!("too much text for one hc call: {err}"))
                }
                _ => HcError::Gone(format!("cannot run {}: {err}", self.bin.display())),
            }
        })?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }
        Err(classify(&String::from_utf8_lossy(&output.stderr)))
    }

    /// One terminal's facts (`hc get`).
    pub(crate) fn get(&self, terminal: &str) -> Result<Value, HcError> {
        let out = self.run(&["get", "--json", terminal])?;
        serde_json::from_str(&out).map_err(|err| HcError::Failed(anyhow::anyhow!("hc get {terminal}: {err}")))
    }

    /// Paste `text` into a terminal and press Enter (`hc send`). `hc` refuses a program that
    /// reports it is blocked, so nothing here passes `--force`.
    pub(crate) fn send(&self, terminal: &str, text: &str) -> Result<(), HcError> {
        self.run(&["send", terminal, text]).map(drop)
    }
}

/// Map `hc`'s failure to an outcome. It prints `{"error":{"kind","message","hint"}}` on stderr.
fn classify(stderr: &str) -> HcError {
    let envelope: Option<Value> = serde_json::from_str(stderr.trim()).ok();
    let error = envelope.as_ref().and_then(|v| v.get("error"));
    let text = |key: &str| error.and_then(|e| e.get(key)).and_then(Value::as_str);
    let message = text("message").map_or_else(|| stderr.trim().to_owned(), str::to_owned);
    match text("kind") {
        Some("blocked") => HcError::Blocked(message),
        Some("not_found" | "finished" | "machine_unknown" | "unavailable") => HcError::Gone(message),
        Some("name_taken") => HcError::Taken(message),
        Some(kind) => HcError::Failed(anyhow::anyhow!("{kind}: {message}")),
        None if message.is_empty() => HcError::Failed(anyhow::anyhow!("hc failed with no output")),
        None => HcError::Failed(anyhow::anyhow!("hc failed: {message}")),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    fn envelope(kind: &str, message: &str) -> String {
        format!(r#"{{"error":{{"hint":"see `hc ls --all`","kind":"{kind}","message":"{message}"}}}}"#)
    }

    #[test]
    fn a_blocked_program_is_told_apart_from_a_terminal_that_is_gone() {
        assert!(
            matches!(classify(&envelope("blocked", "t7 is blocked")), HcError::Blocked(m) if m == "t7 is blocked")
        );
        for kind in ["not_found", "finished", "machine_unknown", "unavailable"] {
            assert!(matches!(classify(&envelope(kind, "no terminal")), HcError::Gone(_)), "{kind}");
        }
    }

    #[test]
    fn a_name_a_live_terminal_holds_is_told_apart_from_other_failures() {
        assert!(matches!(classify(&envelope("name_taken", "taken")), HcError::Taken(m) if m == "taken"));
    }

    #[test]
    fn any_other_failure_keeps_what_hc_said() {
        let other = classify(&envelope("invalid_params", "bad name")).to_string();
        assert_eq!(other, "invalid_params: bad name");
        assert_eq!(classify("boom\n").to_string(), "hc failed: boom");
        assert_eq!(classify("").to_string(), "hc failed with no output");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn text_over_the_argument_limit_is_a_size_problem_not_a_missing_terminal() {
        let hc = Hc { bin: PathBuf::from("/bin/true") };
        let err = hc.send("t7", &"x".repeat(300_000)).expect_err("too long for one argument");
        assert!(matches!(err, HcError::Failed(_)), "{err}");
        assert!(err.to_string().starts_with("too much text for one hc call"), "{err}");
    }

    #[test]
    fn the_launcher_can_pin_the_hc_binary() {
        let pinned = Hc::from_lookup(|k| (k == BIN_VAR).then(|| "/opt/hc".to_owned()));
        assert_eq!(pinned.bin(), std::path::Path::new("/opt/hc"));
        assert_eq!(Hc::from_lookup(|_| None).bin(), std::path::Path::new("hc"));
        assert_eq!(Hc::from_lookup(|_| Some(String::new())).bin(), std::path::Path::new("hc"));
    }
}
