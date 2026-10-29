//! An agent running in a Rex block, as a delivery target.
//!
//! Sending pastes the feedback into the block as a paste event, so Rex wraps it in the
//! bracketed-paste markers the agent asked for, then presses Enter. Once the paste lands the
//! feedback is the agent's: if Enter fails, it waits in the prompt, the block takes focus, and
//! the send still counts, so nothing pastes it twice. In paste mode it leaves
//! Enter to you: the feedback waits in the agent's prompt and the block takes focus, ready to
//! edit. Before writing anything it checks that the agent's process still runs in the block,
//! so a block that fell back to a shell never receives the text, and asks the block's
//! `program_status` whether the agent is at a dialog, the same refusal Herdr gives with
//! `agent_blocked`. A send also needs the agent to have reported its state there at all: an
//! agent that has not could be at a dialog Rex cannot see.

use std::cell::RefCell;

use serde_json::{Value, json};

use super::command::{Rex, RexError, short_id};
use super::launch::foreground;
use crate::delivery::{Delivery, DeliveryError};

/// `PLANNOTATOR_TUI_REX_BLOCK`: the block to send feedback to, set by `plannotator-tui rex`.
pub(crate) const BLOCK_VAR: &str = "PLANNOTATOR_TUI_REX_BLOCK";
/// `PLANNOTATOR_TUI_DELIVER_AGENT`: the agent's host label, for the footer and the archive.
pub(crate) const AGENT_VAR: &str = "PLANNOTATOR_TUI_DELIVER_AGENT";
/// `PLANNOTATOR_TUI_REX_PID`: the agent's process ID when the review opened.
pub(crate) const PID_VAR: &str = "PLANNOTATOR_TUI_REX_PID";
/// `PLANNOTATOR_TUI_REX_SUBMIT=0`: paste into the agent's prompt without pressing Enter.
pub(crate) const SUBMIT_VAR: &str = "PLANNOTATOR_TUI_REX_SUBMIT";

#[derive(Debug)]
pub(crate) struct RexAgent {
    rex: Rex,
    block: String,
    agent: Option<String>,
    /// The agent's process ID at launch. A send needs this process in the block's foreground.
    pid: u32,
    /// Press Enter after the paste; `false` leaves the feedback in the prompt to edit.
    submit: bool,
    /// What the last send left for you to do: Enter, when it did not reach the agent.
    notice: RefCell<Option<String>>,
}

impl RexAgent {
    /// The target `plannotator-tui rex` named in the environment, or `None` outside such a layer.
    pub(crate) fn from_lookup(env: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let non_empty = |k: &str| env(k).filter(|v| !v.is_empty());
        let block = non_empty(BLOCK_VAR)?;
        let pid = non_empty(PID_VAR)?.parse().ok()?;
        let rex = Rex::from_lookup(&env, None).ok()?;
        let submit = env(SUBMIT_VAR).as_deref() != Some("0");
        Some(Self { rex, block, agent: non_empty(AGENT_VAR), pid, submit, notice: RefCell::default() })
    }
}

impl Delivery for RexAgent {
    fn agent_host(&self) -> Option<&str> {
        self.agent.as_deref()
    }

    fn describe(&self) -> String {
        let block = short_id(&self.block);
        let who = self.agent.as_deref().unwrap_or("agent");
        if self.submit { format!("{who} in {block}") } else { format!("{who} prompt in {block}") }
    }

    fn is_agent(&self) -> bool {
        true
    }

    fn deliver(&self, feedback: &str) -> Result<(), DeliveryError> {
        let process = self.rex.terminal_method(&self.block, "process").map_err(unavailable)?;
        if foreground(&process).map(|fg| fg.pid) != Some(self.pid) {
            let who = self.agent.as_deref().unwrap_or("the agent");
            return Err(DeliveryError::Unavailable(format!(
                "{who} no longer runs in {}",
                short_id(&self.block)
            )));
        }
        let status = self.rex.terminal_method(&self.block, "program_status").map_err(unavailable)?;
        if let Some(reason) = blocked_reason(&status) {
            return Err(DeliveryError::Blocked(format!("{} is {reason}", self.describe())));
        }
        // Without a report from the agent, Rex cannot say it is not at a dialog, and Enter
        // there would answer it. Herdr refuses an agent it cannot read the same way.
        if self.submit && !reports_state(&status, self.pid) {
            let who = self.agent.as_deref().unwrap_or("the agent");
            return Err(DeliveryError::Unavailable(format!(
                "{who} in {} has not reported its state to Rex",
                short_id(&self.block)
            )));
        }
        self.notice.take();
        let paste = json!({
            "block_id": self.block,
            "event": { "type": "paste", "payload": { "text": feedback } },
        });
        self.rex.api("block.event", &paste).map_err(unavailable)?;
        // The feedback has landed in the agent's prompt. A failed Enter leaves it there, sent
        // but unsubmitted: a failure here would invite a second paste on top of it.
        if self.submit {
            if self.rex.run(&["send-key", "-b", &self.block, "enter"], None).is_ok() {
                return Ok(());
            }
            *self.notice.borrow_mut() = Some("Enter did not reach it; press Enter in its pane".to_owned());
        }
        // Focus only helps you edit or submit the prompt.
        let _ = self.rex.run(&["focus", &self.block], None);
        Ok(())
    }

    fn notice(&self) -> Option<String> {
        self.notice.take()
    }
}

fn unavailable(err: RexError) -> DeliveryError {
    match err {
        RexError::NotFound(message) => DeliveryError::Unavailable(message),
        RexError::Failed(err) => DeliveryError::Failed(err),
    }
}

/// What a blocked agent is waiting for, from `program_status` JSON; `None` when it can take a
/// message. A record Rex marks `blocked` is the agent at a permission prompt or a question.
pub(crate) fn blocked_reason(status: &Value) -> Option<String> {
    let records = status.get("records")?.as_array()?;
    let blocked = records.iter().find(|r| r.get("state").and_then(Value::as_str) == Some("blocked"))?;
    Some(match blocked.get("kind").and_then(Value::as_str) {
        Some("permission") => "waiting for a permission answer".to_owned(),
        Some("question") => "waiting for an answer to its question".to_owned(),
        Some("auth") => "waiting for a credential".to_owned(),
        _ => "waiting for you".to_owned(),
    })
}

/// Whether the agent, process `pid`, has reported its state in `program_status` JSON. Rex
/// keeps a `done` record after its program exits, so only a record the agent owns counts.
pub(crate) fn reports_state(status: &Value, pid: u32) -> bool {
    let records = status.get("records").and_then(Value::as_array);
    records.is_some_and(|records| {
        records.iter().any(|r| r.pointer("/owner/pid").and_then(Value::as_u64) == Some(u64::from(pid)))
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    fn agent(vars: &[(&str, &str)]) -> Option<RexAgent> {
        let owned: Vec<(String, String)> =
            vars.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
        RexAgent::from_lookup(|k| owned.iter().find(|(name, _)| name == k).map(|(_, v)| v.clone()))
    }

    #[test]
    fn only_a_named_block_inside_a_rex_session_is_a_target() {
        let block = "block:01a17dd5-e47c-754f-b72e-ef739ea1d6db";
        assert!(agent(&[("REX_SESSION", "session:a")]).is_none());
        assert!(agent(&[(BLOCK_VAR, block), (PID_VAR, "7")]).is_none());
        assert!(agent(&[("REX_SESSION", "session:a"), (BLOCK_VAR, block)]).is_none());
        let target =
            agent(&[("REX_SESSION", "session:a"), (BLOCK_VAR, block), (PID_VAR, "7"), (AGENT_VAR, "claude")])
                .expect("a target");
        assert_eq!(target.describe(), "claude in a1d6db");
        assert_eq!(target.agent_host(), Some("claude"));
    }

    #[test]
    fn only_an_explicit_zero_turns_sending_into_pasting_and_the_target_says_so() {
        let block = "block:01a17dd5-e47c-754f-b72e-ef739ea1d6db";
        let with = |submit: &'static str| {
            agent(&[
                ("REX_SESSION", "s"),
                (BLOCK_VAR, block),
                (PID_VAR, "7"),
                (AGENT_VAR, "claude"),
                (SUBMIT_VAR, submit),
            ])
            .expect("a target")
        };
        assert!(with("1").submit && with("").submit);
        assert!(!with("0").submit);
        assert_eq!(with("0").describe(), "claude prompt in a1d6db");
    }

    #[test]
    fn a_working_or_finished_agent_takes_messages_and_a_blocked_one_does_not() {
        let working = json!({"records":[{"state":"working","app":"claude-code"}]});
        assert_eq!(blocked_reason(&working), None);
        assert_eq!(blocked_reason(&json!({"records":[]})), None);
        let asking = json!({"records":[{"state":"done"},{"id":"x","state":"blocked","kind":"permission"}]});
        assert_eq!(blocked_reason(&asking).as_deref(), Some("waiting for a permission answer"));
        let unknown = json!({"records":[{"state":"blocked","kind":"telepathy"}]});
        assert_eq!(blocked_reason(&unknown).as_deref(), Some("waiting for you"));
    }

    #[test]
    fn only_a_record_the_agent_owns_says_it_has_reported_its_state() {
        assert!(!reports_state(&json!({"records":[]}), 42));
        assert!(!reports_state(&json!({}), 42));
        let mine = json!({"records":[{"state":"idle","owner":{"pid":42,"name":"2.1.312"}}]});
        assert!(reports_state(&mine, 42));
        // A `done` record outlives its program; one left by an earlier process says nothing.
        let earlier = json!({"records":[{"state":"done","owner":{"pid":41,"name":"2.1.312"}}]});
        assert!(!reports_state(&earlier, 42));
        assert!(!reports_state(&json!({"records":[{"state":"idle"}]}), 42));
    }

    /// A `rex` that logs its arguments and answers the calls a send makes. The foreground
    /// process is pid 42 and reports itself idle; `silent` in the directory makes it report
    /// nothing, and `enter-fails` makes `send-key` fail.
    #[cfg(unix)]
    fn fake_rex(tag: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("plannotator-tui-rex-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let script = format!(
            r#"#!/bin/sh
echo "$1 $2" >> {dir}/log
case "$1 $2" in
  "do com.superlogical.terminal.process") echo '{{"foreground":{{"pid":42,"argv0":"claude"}}}}' ;;
  "do com.superlogical.terminal.program_status")
    if [ -f {dir}/silent ]; then echo '{{"records":[]}}'
    else echo '{{"records":[{{"state":"idle","owner":{{"pid":42,"name":"claude"}}}}]}}'; fi ;;
  "api call") cat > /dev/null; echo '{{}}' ;;
  "send-key -b") if [ -f {dir}/enter-fails ]; then echo boom >&2; exit 1; fi ;;
esac
"#,
            dir = dir.display()
        );
        std::fs::write(dir.join("rex"), script).expect("write rex");
        std::fs::set_permissions(dir.join("rex"), std::fs::Permissions::from_mode(0o755)).expect("chmod");
        dir
    }

    #[cfg(unix)]
    fn sender(dir: &std::path::Path, pid: &str) -> RexAgent {
        sender_with(dir, pid, &[])
    }

    #[cfg(unix)]
    fn sender_with(dir: &std::path::Path, pid: &str, extra: &[(&str, &str)]) -> RexAgent {
        let bin = dir.display().to_string();
        let mut vars = vec![
            ("REX_BIN_DIR", bin.as_str()),
            ("REX_SESSION", "session:a"),
            (BLOCK_VAR, "block:b"),
            (PID_VAR, pid),
            (AGENT_VAR, "claude"),
        ];
        vars.extend_from_slice(extra);
        agent(&vars).expect("a target")
    }

    #[cfg(unix)]
    fn pastes(dir: &std::path::Path) -> usize {
        std::fs::read_to_string(dir.join("log")).expect("log").lines().filter(|l| *l == "api call").count()
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_enter_after_the_paste_still_counts_as_sent_and_says_so() {
        let dir = fake_rex("enter");
        let target = sender(&dir, "42");
        std::fs::write(dir.join("enter-fails"), "").expect("flag");
        target.deliver("notes").expect("the paste landed, so the send counts");
        assert_eq!(target.notice().as_deref(), Some("Enter did not reach it; press Enter in its pane"));
        assert_eq!(target.notice(), None, "a notice is shown once");
        let log = std::fs::read_to_string(dir.join("log")).expect("log");
        assert!(log.lines().any(|l| l == "focus block:b"), "the block takes focus: {log}");
        std::fs::remove_file(dir.join("enter-fails")).expect("flag");
        target.deliver("more notes").expect("a later send");
        assert_eq!(target.notice(), None);
        assert_eq!(pastes(&dir), 2, "each send pastes once");
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn an_agent_that_has_not_reported_its_state_gets_no_enter_but_can_take_a_paste() {
        let dir = fake_rex("silent");
        std::fs::write(dir.join("silent"), "").expect("flag");
        let target = sender(&dir, "42");
        assert!(matches!(target.deliver("notes"), Err(DeliveryError::Unavailable(_))));
        assert_eq!(pastes(&dir), 0);
        let paste = sender_with(&dir, "42", &[(SUBMIT_VAR, "0")]);
        paste.deliver("notes").expect("a paste leaves Enter to you");
        assert_eq!(pastes(&dir), 1);
        let log = std::fs::read_to_string(dir.join("log")).expect("log");
        assert!(!log.lines().any(|l| l.starts_with("send-key")), "no Enter: {log}");
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn nothing_is_written_once_the_agent_has_left_the_block() {
        let dir = fake_rex("gone");
        let target = sender(&dir, "43");
        let err = target.deliver("notes").expect_err("refused");
        let named_once = |m: &str| m.starts_with("claude no longer runs in ") && !m.contains("claude in");
        assert!(matches!(&err, DeliveryError::Unavailable(m) if named_once(m)), "{err}");
        assert_eq!(pastes(&dir), 0);
        std::fs::remove_dir_all(dir).ok();
    }
}
