//! Where rendered feedback goes when the user sends it (decision 11).
//!
//! The app renders feedback with one function regardless of target; only the target
//! varies. Clipboard is the standalone default. Inside Herdr the target is an agent's pane
//! and the transport is `herdr agent prompt`. Nothing else in the app knows which is in use;
//! it only distinguishes the three outcomes below.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Why a send did not land. The app reacts differently to each.
#[derive(Debug)]
pub(crate) enum DeliveryError {
    /// The agent is at a dialog and Herdr refused the prompt; retry later.
    Blocked(String),
    /// No agent to send to: pane gone, agent not detected yet, herdr binary missing.
    Unavailable(String),
    /// Anything else, with whatever the transport said.
    Failed(anyhow::Error),
}

impl std::fmt::Display for DeliveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Blocked(msg) | Self::Unavailable(msg) => f.write_str(msg),
            Self::Failed(err) => write!(f, "{err:#}"),
        }
    }
}

impl From<std::io::Error> for DeliveryError {
    fn from(err: std::io::Error) -> Self {
        Self::Failed(err.into())
    }
}

pub(crate) trait Delivery {
    /// Shown in the footer and on the button, e.g. `clipboard` or `claude in w1:p1`.
    fn describe(&self) -> String;
    /// True when the target is an agent that will act on the feedback.
    fn is_agent(&self) -> bool {
        false
    }
    fn deliver(&self, feedback: &str) -> Result<(), DeliveryError>;
}

/// OSC 52: hand text to the terminal's clipboard so Cmd-V works outside the app.
#[derive(Debug, Default)]
pub(crate) struct Clipboard;

impl Delivery for Clipboard {
    fn describe(&self) -> String {
        "clipboard".to_owned()
    }

    fn deliver(&self, feedback: &str) -> Result<(), DeliveryError> {
        let mut out = std::io::stdout().lock();
        write!(out, "\x1b]52;c;{}\x07", crate::base64::encode(feedback.as_bytes()))?;
        out.flush()?;
        Ok(())
    }
}

/// Headless runs: nothing leaves the process.
#[derive(Debug, Default)]
pub(crate) struct Discard;

impl Delivery for Discard {
    fn describe(&self) -> String {
        "nowhere (headless)".to_owned()
    }

    fn deliver(&self, _feedback: &str) -> Result<(), DeliveryError> {
        Ok(())
    }
}

/// An agent running in a Herdr pane. `herdr agent prompt` pastes the feedback as the
/// agent's next message and presses Enter.
#[derive(Debug)]
pub(crate) struct HerdrAgent {
    bin: PathBuf,
    pane: String,
    agent: Option<String>,
}

impl HerdrAgent {
    pub(crate) fn new(bin: PathBuf, pane: String, agent: Option<String>) -> Self {
        Self { bin, pane, agent }
    }
}

impl Delivery for HerdrAgent {
    fn describe(&self) -> String {
        match &self.agent {
            Some(agent) => format!("{agent} in {}", self.pane),
            None => self.pane.clone(),
        }
    }

    fn is_agent(&self) -> bool {
        true
    }

    fn deliver(&self, feedback: &str) -> Result<(), DeliveryError> {
        let output = Command::new(&self.bin)
            .args(["agent", "prompt", &self.pane, feedback])
            .stdin(Stdio::null())
            .output()
            .map_err(|err| DeliveryError::Unavailable(format!("cannot run {}: {err}", self.bin.display())))?;
        parse_response(
            output.status.success(),
            &String::from_utf8_lossy(&output.stdout),
            &String::from_utf8_lossy(&output.stderr),
        )
    }
}

/// Map a `herdr agent prompt` exit into an outcome. Herdr prints a JSON envelope
/// `{"error":{"code","message"}}` on stderr when it refuses.
pub(crate) fn parse_response(success: bool, _stdout: &str, stderr: &str) -> Result<(), DeliveryError> {
    if success {
        return Ok(());
    }
    let envelope: Option<serde_json::Value> = serde_json::from_str(stderr.trim()).ok();
    let error = envelope.as_ref().and_then(|v| v.get("error"));
    let code = error.and_then(|e| e.get("code")).and_then(serde_json::Value::as_str);
    let message = error
        .and_then(|e| e.get("message"))
        .and_then(serde_json::Value::as_str)
        .map_or_else(|| stderr.trim().to_owned(), str::to_owned);
    match code {
        Some("agent_blocked") => Err(DeliveryError::Blocked(message)),
        Some("agent_not_found" | "agent_not_ready" | "pane_not_found" | "empty_agent_prompt") => {
            Err(DeliveryError::Unavailable(message))
        }
        Some(code) => Err(DeliveryError::Failed(anyhow::anyhow!("{code}: {message}"))),
        None => Err(DeliveryError::Failed(anyhow::anyhow!(
            "herdr agent prompt failed: {}",
            if message.is_empty() { "no output".to_owned() } else { message }
        ))),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    fn envelope(code: &str, message: &str) -> String {
        format!(r#"{{"id":"cli:agent:prompt","error":{{"code":"{code}","message":"{message}"}}}}"#)
    }

    #[test]
    fn success_is_ok_regardless_of_output() {
        assert!(parse_response(true, r#"{"id":"x","result":{}}"#, "").is_ok());
    }

    #[test]
    fn blocked_agent_is_blocked_with_herdrs_message() {
        let err =
            parse_response(false, "", &envelope("agent_blocked", "agent w1:p1 is blocked")).unwrap_err();
        assert!(matches!(&err, DeliveryError::Blocked(m) if m == "agent w1:p1 is blocked"));
    }

    #[test]
    fn missing_or_unready_agent_is_unavailable() {
        for code in ["agent_not_found", "agent_not_ready", "pane_not_found", "empty_agent_prompt"] {
            let err = parse_response(false, "", &envelope(code, "gone")).unwrap_err();
            assert!(matches!(err, DeliveryError::Unavailable(_)), "{code}");
        }
    }

    #[test]
    fn unknown_code_and_garbage_are_failures_that_keep_the_text() {
        let err = parse_response(false, "", &envelope("socket_error", "boom")).unwrap_err();
        assert!(matches!(&err, DeliveryError::Failed(e) if e.to_string() == "socket_error: boom"));
        let err = parse_response(false, "", "connection refused\n").unwrap_err();
        assert!(matches!(&err, DeliveryError::Failed(e) if e.to_string().contains("connection refused")));
    }

    #[test]
    fn describe_names_the_agent_when_known() {
        let bin = PathBuf::from("herdr");
        assert_eq!(
            HerdrAgent::new(bin.clone(), "w1:p1".into(), Some("claude".into())).describe(),
            "claude in w1:p1"
        );
        assert_eq!(HerdrAgent::new(bin, "w1:p1".into(), None).describe(), "w1:p1");
    }
}
