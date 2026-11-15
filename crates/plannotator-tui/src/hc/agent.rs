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
        // Neither reached the agent, so the review copies instead.
        HcError::Gone(message) | HcError::TooLong(message) => DeliveryError::Unavailable(message),
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

    /// An `hc` that answers `get` with the file `get` in its folder and logs each `send`;
    /// `blocked` there makes `send` refuse the way `hc` does for an agent at a dialog.
    #[cfg(unix)]
    fn fake_hc(tag: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("plannotator-tui-hc-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let script = format!(
            r#"#!/bin/sh
case "$1" in
  get) cat {dir}/get ;;
  send)
    if [ -f {dir}/blocked ]; then
      echo '{{"error":{{"kind":"blocked","message":"t7 is blocked on a permission prompt"}}}}' >&2; exit 1
    fi
    echo "$2" >> {dir}/sent ;;
esac
"#,
            dir = dir.display()
        );
        std::fs::write(dir.join("hc"), script).expect("write hc");
        std::fs::set_permissions(dir.join("hc"), std::fs::Permissions::from_mode(0o755)).expect("chmod");
        dir
    }

    #[cfg(unix)]
    #[test]
    fn a_send_reaches_only_a_ready_agent_and_every_refusal_lets_the_review_copy_or_retry() {
        let dir = fake_hc("send");
        let bin = dir.join("hc").display().to_string();
        let target = agent(&[
            (TERMINAL_VAR, "t7"),
            (PID_VAR, "7"),
            (AGENT_VAR, "pi"),
            (crate::hc::command::BIN_VAR, &bin),
        ])
        .expect("a target");
        let facts = |json: Value| std::fs::write(dir.join("get"), json.to_string()).expect("get");
        let sent = || std::fs::read_to_string(dir.join("sent")).map_or(0, |s| s.lines().count());

        facts(json!({"state":"live","reported":true,"foreground":{"pid":7,"name":"pi"}}));
        target.deliver("notes").expect("a ready agent takes it");
        assert_eq!(sent(), 1);

        std::fs::write(dir.join("blocked"), "").expect("flag");
        let blocked = target.deliver("notes").expect_err("refused at a dialog");
        assert!(matches!(&blocked, DeliveryError::Blocked(m) if m.contains("permission")), "{blocked}");
        std::fs::remove_file(dir.join("blocked")).expect("flag");

        // Over the argument limit, `hc` never runs; the review copies instead.
        let long = target.deliver(&"x".repeat(4 << 20)).expect_err("too long");
        assert!(
            matches!(&long, DeliveryError::Unavailable(m) if m.ends_with("too much text for one hc call"))
        );

        facts(json!({"state":"live","reported":true,"foreground":{"pid":9,"name":"zsh"}}));
        let gone = target.deliver("notes").expect_err("the agent left");
        assert!(matches!(&gone, DeliveryError::Unavailable(m) if m == "pi no longer runs in t7"), "{gone}");
        assert_eq!(sent(), 1, "nothing but the first send was typed");
        std::fs::remove_dir_all(dir).ok();
    }
}
