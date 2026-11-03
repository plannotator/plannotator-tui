//! An agent running in an hc terminal, as a delivery target.
//!
//! `hc send` pastes the feedback as the agent's next message and presses Enter, once nobody
//! has typed in the terminal for a moment. Before sending, the target is checked: the agent's
//! process must still be the terminal's foreground program, and it must have reported its
//! state at all. Without a report `hc` cannot say the agent is not at a dialog, and Enter
//! there would answer it. `hc send` itself refuses a program that reports it is blocked, the
//! counterpart of Herdr's `agent_blocked`.

use serde_json::Value;

use super::command::{Hc, HcError};
use crate::delivery::{Delivery, DeliveryError};

/// `PLANNOTATOR_TUI_HC_TERMINAL`: the terminal to send feedback to, set by `plannotator-tui hc`.
pub(crate) const TERMINAL_VAR: &str = "PLANNOTATOR_TUI_HC_TERMINAL";
/// `PLANNOTATOR_TUI_DELIVER_AGENT`: the agent's host label, for the footer and the archive.
pub(crate) const AGENT_VAR: &str = "PLANNOTATOR_TUI_DELIVER_AGENT";
/// `PLANNOTATOR_TUI_HC_PID`: the agent's process ID when the review opened.
pub(crate) const PID_VAR: &str = "PLANNOTATOR_TUI_HC_PID";

#[derive(Debug)]
pub(crate) struct HcAgent {
    hc: Hc,
    terminal: String,
    agent: Option<String>,
    /// The agent's process ID at launch. A send needs this process in the terminal's foreground.
    pid: u32,
}

impl HcAgent {
    /// The target `plannotator-tui hc` named in the environment, or `None` outside such a layer.
    pub(crate) fn from_lookup(env: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let non_empty = |k: &str| env(k).filter(|v| !v.is_empty());
        let terminal = non_empty(TERMINAL_VAR)?;
        let pid = non_empty(PID_VAR)?.parse().ok()?;
        Some(Self { hc: Hc::from_lookup(&env), terminal, agent: non_empty(AGENT_VAR), pid })
    }

    fn who(&self) -> &str {
        self.agent.as_deref().unwrap_or("the agent")
    }
}

impl Delivery for HcAgent {
    fn agent_host(&self) -> Option<&str> {
        self.agent.as_deref()
    }

    fn describe(&self) -> String {
        format!("{} in {}", self.agent.as_deref().unwrap_or("agent"), self.terminal)
    }

    fn is_agent(&self) -> bool {
        true
    }

    fn deliver(&self, feedback: &str) -> Result<(), DeliveryError> {
        let facts = self.hc.get(&self.terminal).map_err(delivery_error)?;
        if let Some(reason) = unready(&facts, self.pid, self.who(), &self.terminal) {
            return Err(DeliveryError::Unavailable(reason));
        }
        self.hc.send(&self.terminal, feedback).map_err(delivery_error)
    }
}

fn delivery_error(err: HcError) -> DeliveryError {
    match err {
        HcError::Blocked(message) => DeliveryError::Blocked(message),
        HcError::Gone(message) => DeliveryError::Unavailable(message),
        HcError::Taken(message) => DeliveryError::Failed(anyhow::anyhow!(message)),
        HcError::Failed(err) => DeliveryError::Failed(err),
    }
}

/// Why the agent cannot take a message, from `hc get` JSON; `None` when it can.
fn unready(facts: &Value, pid: u32, who: &str, terminal: &str) -> Option<String> {
    if facts.get("state").and_then(Value::as_str) != Some("live") {
        return Some(format!("{terminal} has finished"));
    }
    if facts.pointer("/foreground/pid").and_then(Value::as_u64) != Some(u64::from(pid)) {
        return Some(format!("{who} no longer runs in {terminal}"));
    }
    if facts.get("reported").and_then(Value::as_bool) != Some(true) {
        return Some(format!("{who} in {terminal} has not reported its state to hc"));
    }
    None
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use serde_json::json;

    use super::*;

    fn agent(vars: &[(&str, &str)]) -> Option<HcAgent> {
        let owned: Vec<(String, String)> =
            vars.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
        HcAgent::from_lookup(|k| owned.iter().find(|(name, _)| name == k).map(|(_, v)| v.clone()))
    }

    #[test]
    fn only_a_named_terminal_and_process_make_a_target() {
        assert!(agent(&[]).is_none());
        assert!(agent(&[(TERMINAL_VAR, "t7")]).is_none());
        assert!(agent(&[(PID_VAR, "7")]).is_none());
        assert!(agent(&[(TERMINAL_VAR, "t7"), (PID_VAR, "not-a-pid")]).is_none());
        let target = agent(&[(TERMINAL_VAR, "t7"), (PID_VAR, "7"), (AGENT_VAR, "pi")]).expect("a target");
        assert_eq!(target.describe(), "pi in t7");
        assert_eq!(target.agent_host(), Some("pi"));
    }

    #[test]
    fn an_agent_takes_a_message_only_while_it_runs_there_and_reports_its_state() {
        let live = json!({"state":"live","reported":true,"foreground":{"pid":7,"name":"pi"}});
        assert_eq!(unready(&live, 7, "pi", "t7"), None);

        let finished = json!({"state":"finished","reported":true});
        assert_eq!(unready(&finished, 7, "pi", "t7").as_deref(), Some("t7 has finished"));

        let shell = json!({"state":"live","reported":true,"foreground":{"pid":9,"name":"zsh"}});
        assert_eq!(unready(&shell, 7, "pi", "t7").as_deref(), Some("pi no longer runs in t7"));

        let silent = json!({"state":"live","reported":false,"foreground":{"pid":7,"name":"pi"}});
        assert_eq!(
            unready(&silent, 7, "pi", "t7").as_deref(),
            Some("pi in t7 has not reported its state to hc")
        );
    }
}
