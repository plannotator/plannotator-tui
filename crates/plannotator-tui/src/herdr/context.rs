//! The environment Herdr hands a plugin process, and the delivery target derived from it.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

/// `HERDR_PLUGIN_CONTEXT_JSON`: a snapshot taken before our pane spawned, so the focused
/// pane is the one the user was in — the agent's, when a human triggered us from it.
/// Every field is optional; Herdr omits nulls and may add fields.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub(crate) struct HerdrContext {
    pub(crate) workspace_id: Option<String>,
    pub(crate) workspace_cwd: Option<String>,
    pub(crate) focused_pane_id: Option<String>,
    pub(crate) focused_pane_cwd: Option<String>,
    pub(crate) focused_pane_agent: Option<String>,
    pub(crate) invocation_source: Option<String>,
    pub(crate) clicked_url: Option<String>,
}

/// Where feedback goes: a Herdr pane, and the agent in it when known (for the label).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Target {
    pub(crate) pane: String,
    pub(crate) agent: Option<String>,
}

/// Everything plannotator-tui reads from its environment when Herdr is involved.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct HerdrEnv {
    /// `HERDR_ENV=1`.
    pub(crate) in_herdr: bool,
    /// `HERDR_BIN_PATH`, else `herdr` on `PATH`.
    pub(crate) bin: PathBuf,
    /// `HERDR_SESSION`: Herdr's named session. Explicit `--session` beats inherited socket overrides.
    pub(crate) session_name: Option<String>,
    /// `HERDR_PANE_ID`: the pane this process runs in. Inside the app that is our own
    /// pane; only the launcher may treat it as "the caller".
    pub(crate) pane_id: Option<String>,
    pub(crate) context: Option<HerdrContext>,
    /// `PLANNOTATOR_TUI_FILE`, `PLANNOTATOR_TUI_DELIVER_TO`, `PLANNOTATOR_TUI_DELIVER_AGENT`,
    /// `PLANNOTATOR_TUI_PLACEMENT`: set by the launcher or an agent.
    pub(crate) file: Option<PathBuf>,
    pub(crate) deliver_to: Option<String>,
    pub(crate) deliver_agent: Option<String>,
    pub(crate) placement: Option<String>,
    /// `HERDR_PLUGIN_ID`: the plugin this binary ships in. Herdr sets it for plugin actions;
    /// unset (an agent's shell, a script) it is `annotate`, the id Herdr Annotate installs
    /// under, so `plannotator-tui herdr open` reaches the shipped plugin.
    pub(crate) plugin_id: Option<String>,
    /// `PLANNOTATOR_TUI_MESSAGE_PID`: open this agent's last message instead of a file.
    pub(crate) message_pid: Option<u32>,
    /// `PLANNOTATOR_TUI_HOST`: which agent's transcript format to read.
    pub(crate) host: Option<String>,
    /// `PLANNOTATOR_TUI_SESSION`: the agent's transcript path, when Herdr knew it.
    pub(crate) session: Option<PathBuf>,
    /// `PLANNOTATOR_TUI_SESSION_ID`: the agent's session id, for hosts without transcript files.
    pub(crate) session_id: Option<String>,
    /// `PLANNOTATOR_TUI_NEWEST=1`: open the newest reply straight away, no picker.
    pub(crate) newest: bool,
    /// `PLANNOTATOR_TUI_TERMINAL_PANE`: open this pane's recent output instead of a file.
    pub(crate) terminal_pane: Option<String>,
    /// `PLANNOTATOR_TUI_TERMINAL_LINES`: how many recent lines to read from that pane.
    pub(crate) terminal_lines: Option<u32>,
}

impl HerdrEnv {
    pub(crate) fn from_env() -> Self {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    pub(crate) fn from_lookup(env: impl Fn(&str) -> Option<String>) -> Self {
        let non_empty = |k: &str| env(k).filter(|v| !v.is_empty());
        let context =
            non_empty("HERDR_PLUGIN_CONTEXT_JSON").and_then(|json| serde_json::from_str(&json).ok());
        Self {
            in_herdr: env("HERDR_ENV").as_deref() == Some("1"),
            bin: non_empty("HERDR_BIN_PATH").map_or_else(|| PathBuf::from("herdr"), PathBuf::from),
            session_name: non_empty("HERDR_SESSION"),
            pane_id: non_empty("HERDR_PANE_ID"),
            context,
            file: non_empty("PLANNOTATOR_TUI_FILE").map(PathBuf::from),
            deliver_to: non_empty("PLANNOTATOR_TUI_DELIVER_TO"),
            deliver_agent: non_empty("PLANNOTATOR_TUI_DELIVER_AGENT"),
            placement: non_empty("PLANNOTATOR_TUI_PLACEMENT"),
            plugin_id: non_empty("HERDR_PLUGIN_ID"),
            message_pid: non_empty("PLANNOTATOR_TUI_MESSAGE_PID").and_then(|v| v.parse().ok()),
            host: non_empty("PLANNOTATOR_TUI_HOST"),
            session: non_empty("PLANNOTATOR_TUI_SESSION").map(PathBuf::from),
            session_id: non_empty("PLANNOTATOR_TUI_SESSION_ID"),
            newest: env("PLANNOTATOR_TUI_NEWEST").as_deref() == Some("1"),
            terminal_pane: non_empty("PLANNOTATOR_TUI_TERMINAL_PANE"),
            terminal_lines: non_empty("PLANNOTATOR_TUI_TERMINAL_LINES").and_then(|v| v.parse().ok()),
        }
    }

    /// The focused pane from the context, only when Herdr saw an agent in it.
    pub(crate) fn focused_agent_pane(&self) -> Option<Target> {
        let context = self.context.as_ref()?;
        let agent = context.focused_pane_agent.clone().filter(|agent| !agent.trim().is_empty())?;
        Some(Target { pane: context.focused_pane_id.clone()?, agent: Some(agent) })
    }

    /// The in-app rule: an explicit `PLANNOTATOR_TUI_DELIVER_TO`, else the context's focused
    /// pane when an agent runs there, else nothing. Never `HERDR_PANE_ID` — that is us.
    pub(crate) fn delivery_target(&self) -> Option<Target> {
        if let Some(pane) = &self.deliver_to {
            return Some(Target { pane: pane.clone(), agent: self.deliver_agent.clone() });
        }
        self.focused_agent_pane()
    }

    /// Whether the pane entrypoint opens an agent message rather than a file.
    pub(crate) fn has_message_source(&self) -> bool {
        self.session.is_some() || self.session_id.is_some() || self.message_pid.is_some()
    }

    /// The pane whose output the user asked to review: the focused pane when a plugin action
    /// started us, else the caller's own pane (an agent or a script running the command).
    pub(crate) fn reviewed_pane(&self) -> Option<String> {
        match &self.context {
            Some(context) => context.focused_pane_id.clone(),
            None => self.pane_id.clone(),
        }
    }

    /// The agent Herdr saw in `pane` when it snapshotted the context, if that pane was focused.
    pub(crate) fn context_agent_in(&self, pane: &str) -> Option<String> {
        let context = self.context.as_ref()?;
        (context.focused_pane_id.as_deref() == Some(pane))
            .then(|| context.focused_pane_agent.clone())
            .flatten()
            .filter(|agent| !agent.trim().is_empty())
    }

    /// Ask Herdr which agent runs in `pane` (`herdr pane get`), for the label when the
    /// launcher only knew the pane id. One short process at startup; `None` on any failure.
    pub(crate) fn agent_in_pane(&self, pane: &str) -> Option<String> {
        let output = herdr_command(&self.bin, self.session_name.as_deref())
            .args(["pane", "get", pane])
            .stdin(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())?;
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
        json.pointer("/result/pane/agent")?.as_str().map(str::to_owned)
    }
}

pub(crate) fn herdr_command(bin: &Path, session_name: Option<&str>) -> Command {
    let mut command = Command::new(bin);
    if let Some(session) = session_name.filter(|session| !session.is_empty()) {
        command.args(["--session", session]);
    }
    command
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    fn env(vars: &[(&str, &str)]) -> HerdrEnv {
        let owned: Vec<(String, String)> =
            vars.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
        HerdrEnv::from_lookup(|k| owned.iter().find(|(name, _)| name == k).map(|(_, v)| v.clone()))
    }

    #[test]
    fn context_with_omitted_and_unknown_fields_parses() {
        let json = r#"{"focused_pane_id":"w1:p2","focused_pane_agent":"claude","link_handler_id":"x","worktree":{"a":1}}"#;
        let context: HerdrContext = serde_json::from_str(json).expect("parses");
        assert_eq!(context.focused_pane_id.as_deref(), Some("w1:p2"));
        assert_eq!(context.focused_pane_agent.as_deref(), Some("claude"));
        assert_eq!(context.workspace_cwd, None);
    }

    #[test]
    fn outside_herdr_nothing_is_set() {
        let env = env(&[]);
        assert!(!env.in_herdr);
        assert_eq!(env.bin, PathBuf::from("herdr"));
        assert_eq!(env.session_name, None);
        assert_eq!(env.delivery_target(), None);
    }

    #[test]
    fn named_herdr_session_is_preserved_for_child_cli_calls() {
        let named = env(&[("HERDR_SESSION", "personal")]);
        assert_eq!(named.session_name.as_deref(), Some("personal"));
        assert_eq!(env(&[("HERDR_SESSION", "")]).session_name, None);
    }

    #[test]
    fn herdr_command_prepends_explicit_session_when_present() {
        let mut command = herdr_command(Path::new("herdr"), Some("personal"));
        command.args(["pane", "list"]);
        let args: Vec<_> = command.get_args().map(|arg| arg.to_string_lossy().into_owned()).collect();
        assert_eq!(args, ["--session", "personal", "pane", "list"]);

        let mut command = herdr_command(Path::new("herdr"), None);
        command.args(["pane", "list"]);
        let args: Vec<_> = command.get_args().map(|arg| arg.to_string_lossy().into_owned()).collect();
        assert_eq!(args, ["pane", "list"]);
    }

    #[test]
    fn explicit_deliver_to_wins_and_carries_the_agent_label() {
        let env = env(&[
            ("HERDR_ENV", "1"),
            ("PLANNOTATOR_TUI_DELIVER_TO", "w1:p1"),
            ("PLANNOTATOR_TUI_DELIVER_AGENT", "claude"),
            ("HERDR_PLUGIN_CONTEXT_JSON", r#"{"focused_pane_id":"w1:p9","focused_pane_agent":"codex"}"#),
        ]);
        assert_eq!(
            env.delivery_target(),
            Some(Target { pane: "w1:p1".into(), agent: Some("claude".into()) })
        );
    }

    #[test]
    fn context_pane_is_a_target_only_when_an_agent_runs_there() {
        let with_agent = env(&[
            ("HERDR_ENV", "1"),
            ("HERDR_PLUGIN_CONTEXT_JSON", r#"{"focused_pane_id":"w1:p2","focused_pane_agent":"claude"}"#),
        ]);
        assert_eq!(
            with_agent.delivery_target(),
            Some(Target { pane: "w1:p2".into(), agent: Some("claude".into()) })
        );
        let shell =
            env(&[("HERDR_ENV", "1"), ("HERDR_PLUGIN_CONTEXT_JSON", r#"{"focused_pane_id":"w1:p2"}"#)]);
        assert_eq!(shell.delivery_target(), None);
        let mirror_shell =
            env(&[("HERDR_PLUGIN_CONTEXT_JSON", r#"{"focused_pane_id":"w1:p2","focused_pane_agent":""}"#)]);
        assert_eq!(mirror_shell.delivery_target(), None);
    }

    #[test]
    fn own_pane_id_is_never_a_target() {
        let env = env(&[("HERDR_ENV", "1"), ("HERDR_PANE_ID", "w1:p3")]);
        assert_eq!(env.pane_id.as_deref(), Some("w1:p3"));
        assert_eq!(env.delivery_target(), None);
    }

    #[test]
    fn any_exact_session_or_pid_selects_the_message_entrypoint() {
        for vars in [
            vec![("PLANNOTATOR_TUI_SESSION", "C:\\sessions\\one.jsonl")],
            vec![("PLANNOTATOR_TUI_SESSION_ID", "session-one")],
            vec![("PLANNOTATOR_TUI_MESSAGE_PID", "42")],
        ] {
            assert!(env(&vars).has_message_source());
        }
        assert!(!env(&[]).has_message_source());
    }

    #[test]
    fn only_an_exact_newest_flag_skips_the_picker() {
        assert!(env(&[("PLANNOTATOR_TUI_NEWEST", "1")]).newest);
        assert!(!env(&[]).newest);
        assert!(!env(&[("PLANNOTATOR_TUI_NEWEST", "")]).newest);
        assert!(!env(&[("PLANNOTATOR_TUI_NEWEST", "0")]).newest);
    }

    #[test]
    fn the_reviewed_pane_is_the_focused_one_from_an_action_else_the_caller() {
        let action = env(&[
            ("HERDR_PANE_ID", "w1:p9"),
            ("HERDR_PLUGIN_CONTEXT_JSON", r#"{"focused_pane_id":"w1:p2","focused_pane_agent":"codex"}"#),
        ]);
        assert_eq!(action.reviewed_pane().as_deref(), Some("w1:p2"));
        assert_eq!(action.context_agent_in("w1:p2").as_deref(), Some("codex"));
        assert_eq!(action.context_agent_in("w1:p3"), None);
        assert_eq!(env(&[("HERDR_PANE_ID", "w1:p9")]).reviewed_pane().as_deref(), Some("w1:p9"));
        assert_eq!(env(&[("HERDR_PLUGIN_CONTEXT_JSON", "{}")]).reviewed_pane(), None);
    }

    #[test]
    fn the_terminal_pane_and_line_count_come_from_the_launcher() {
        let env =
            env(&[("PLANNOTATOR_TUI_TERMINAL_PANE", "w1:p2"), ("PLANNOTATOR_TUI_TERMINAL_LINES", "80")]);
        assert_eq!(env.terminal_pane.as_deref(), Some("w1:p2"));
        assert_eq!(env.terminal_lines, Some(80));
    }
}
