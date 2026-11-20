//! The agents running in Herdr panes, for the agent picker (`docs/spec-pi-link.md`).
//!
//! `herdr agent list` names every pane with a detected agent; `herdr workspace list` and
//! `herdr tab list` give the labels that tell two Claude panes apart. All three are reads.
//! A label that cannot be read falls back to its id; an agent list that cannot be read lists
//! no Herdr agents at all, so the picker still offers the rest.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;
use serde_json::Value;

/// An agent in a Herdr pane, as the picker shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PaneAgent {
    /// The pane id Send prompts, e.g. `w1:p3`.
    pub(crate) pane: String,
    /// The agent Herdr detected: `claude`, `codex`, `pi`, ...
    pub(crate) agent: String,
    /// The name the person gave the agent in Herdr, if any.
    pub(crate) name: Option<String>,
    /// `idle`, `working`, `blocked`, `done` or `unknown`.
    pub(crate) status: String,
    /// The agent's folder: its foreground process's, else the pane's.
    pub(crate) cwd: Option<PathBuf>,
    /// The workspace's label, else its id.
    pub(crate) workspace: String,
    /// The tab's label, else its id.
    pub(crate) tab: String,
    /// The session Herdr knows the agent by (a transcript path or an id), to spot a Pi the
    /// Pi link already lists.
    pub(crate) session: Option<String>,
}

impl PaneAgent {
    /// `claude`, or `claude · reviewer` when the agent has a name.
    pub(crate) fn label(&self) -> String {
        match self.name.as_deref().map(str::trim).filter(|name| !name.is_empty()) {
            Some(name) => format!("{} · {name}", self.agent),
            None => self.agent.clone(),
        }
    }

    /// Where the pane is: `workspace › tab`.
    pub(crate) fn place(&self) -> String {
        format!("{} › {}", self.workspace, self.tab)
    }
}

/// One entry of `herdr agent list`. Herdr omits nulls and may add fields.
#[derive(Debug, Deserialize)]
struct Listed {
    pane_id: String,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    display_agent: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    agent_status: Option<String>,
    #[serde(default)]
    workspace_id: String,
    #[serde(default)]
    tab_id: String,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    foreground_cwd: Option<String>,
    #[serde(default)]
    agent_session: Option<Session>,
}

#[derive(Debug, Deserialize)]
struct Session {
    #[serde(default)]
    value: Option<String>,
}

/// Reads the agents from one Herdr: its binary and the pane this review runs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HerdrAgents {
    pub(crate) bin: PathBuf,
    /// `HERDR_PANE_ID`: never listed, it is the review's own pane.
    pub(crate) own_pane: Option<String>,
}

impl HerdrAgents {
    /// Every pane with an agent except our own, in Herdr's order. Empty on any failure.
    pub(crate) fn list(&self) -> Vec<PaneAgent> {
        let Some(agents) = self.read(&["agent", "list"]) else { return Vec::new() };
        let workspaces = self.read(&["workspace", "list"]);
        let tabs = self.read(&["tab", "list"]);
        parse(&agents, workspaces.as_ref(), tabs.as_ref(), self.own_pane.as_deref())
    }

    fn read(&self, args: &[&str]) -> Option<Value> {
        let output = Command::new(&self.bin)
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())?;
        serde_json::from_slice(&output.stdout).ok()
    }
}

/// The agents in `agents` (`herdr agent list`'s reply) with their workspace and tab labels,
/// leaving out `own_pane` and any entry without a pane or an agent.
pub(crate) fn parse(
    agents: &Value,
    workspaces: Option<&Value>,
    tabs: Option<&Value>,
    own_pane: Option<&str>,
) -> Vec<PaneAgent> {
    let label = |list: Option<&Value>, key: &str, field: &str, id: &str| -> String {
        list.and_then(|v| v.pointer(&format!("/result/{key}")))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|entry| entry.get(field).and_then(Value::as_str) == Some(id))
            .and_then(|entry| entry.get("label").and_then(Value::as_str))
            .map(str::trim)
            .filter(|label| !label.is_empty())
            .map_or_else(|| id.to_owned(), str::to_owned)
    };
    let listed = agents.pointer("/result/agents").and_then(Value::as_array).into_iter().flatten();
    listed
        .filter_map(|entry| serde_json::from_value::<Listed>(entry.clone()).ok())
        .filter(|entry| !entry.pane_id.is_empty() && Some(entry.pane_id.as_str()) != own_pane)
        .filter_map(|entry| {
            let agent = [&entry.agent, &entry.display_agent]
                .into_iter()
                .flatten()
                .map(|agent| agent.trim())
                .find(|agent| !agent.is_empty())?
                .to_owned();
            let cwd = [&entry.foreground_cwd, &entry.cwd]
                .into_iter()
                .flatten()
                .find(|cwd| !cwd.is_empty())
                .map(PathBuf::from);
            Some(PaneAgent {
                agent,
                name: entry.name,
                status: entry.agent_status.unwrap_or_else(|| "unknown".to_owned()),
                cwd,
                workspace: label(workspaces, "workspaces", "workspace_id", &entry.workspace_id),
                tab: label(tabs, "tabs", "tab_id", &entry.tab_id),
                session: entry.agent_session.and_then(|s| s.value).filter(|v| !v.is_empty()),
                pane: entry.pane_id,
            })
        })
        .collect()
}

/// Whether `agent` is the Pi session the Pi link lists as `id` with `transcript`: Herdr
/// records a Pi's session as its transcript path or its id.
pub(crate) fn is_pi_session(agent: &PaneAgent, id: &str, transcript: Option<&Path>) -> bool {
    agent.agent == "pi"
        && agent
            .session
            .as_deref()
            .is_some_and(|session| session == id || transcript.is_some_and(|path| Path::new(session) == path))
}

/// A fake `herdr` in `dir` for tests: `agent list` answers `agents`, `workspace list` and
/// `tab list` answer fixed labels, and `agent prompt <pane> <text>` appends the pane and the
/// text to `dir/prompts`, or, while `dir/blocked` exists, refuses as Herdr does for an agent
/// at a dialog (`agent_blocked`). Every call's arguments go to `dir/calls`.
#[cfg(all(test, unix))]
#[allow(clippy::expect_used, reason = "test fixtures assert by panicking")]
pub(crate) fn fake_herdr(dir: &Path, agents: &Value) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).expect("fake herdr dir");
    let workspaces = serde_json::json!({"result": {"type": "workspace_list", "workspaces": [
        {"workspace_id": "w1", "number": 1, "label": "api"},
        {"workspace_id": "w2", "number": 2, "label": " "},
    ]}});
    let tabs = serde_json::json!({"result": {"type": "tab_list", "tabs": [
        {"tab_id": "w1:t1", "workspace_id": "w1", "number": 1, "label": "main"},
        {"tab_id": "w1:t2", "workspace_id": "w1", "number": 2, "label": "tests"},
    ]}});
    std::fs::write(dir.join("agents.json"), agents.to_string()).expect("agents");
    std::fs::write(dir.join("workspaces.json"), workspaces.to_string()).expect("workspaces");
    std::fs::write(dir.join("tabs.json"), tabs.to_string()).expect("tabs");
    let script = format!(
        "#!/bin/sh\nd='{d}'\ncase \"$1 $2\" in\n  'agent prompt') printf '%s\\n' \"$1 $2 $3\" >> \"$d/calls\"; if [ -f \"$d/blocked\" ]; then echo '{{\"error\":{{\"code\":\"agent_blocked\",\"message\":\"agent is waiting for permission\"}}}}' >&2; exit 1; fi; printf '%s\\n%s\\n---\\n' \"$3\" \"$4\" >> \"$d/prompts\" ;;\n  'agent list') echo \"$*\" >> \"$d/calls\"; cat \"$d/agents.json\" ;;\n  'workspace list') echo \"$*\" >> \"$d/calls\"; cat \"$d/workspaces.json\" ;;\n  'tab list') echo \"$*\" >> \"$d/calls\"; cat \"$d/tabs.json\" ;;\n  *) echo \"$*\" >> \"$d/calls\"; exit 2 ;;\nesac\n",
        d = dir.display()
    );
    let bin = dir.join("herdr");
    std::fs::write(&bin, script).expect("script");
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    bin
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]
mod tests {
    use serde_json::json;

    use super::*;

    fn agents() -> Value {
        json!({"id": "cli:agent:list", "result": {"type": "agent_list", "agents": [
            {"terminal_id": "t1", "agent": "claude", "agent_status": "idle", "workspace_id": "w1",
             "tab_id": "w1:t1", "pane_id": "w1:p1", "focused": false, "cwd": "/work/api",
             "foreground_cwd": "/work/api/sub", "revision": 3, "future_field": {"x": 1}},
            {"terminal_id": "t2", "agent": "claude", "name": "reviewer", "agent_status": "working",
             "workspace_id": "w2", "tab_id": "w2:t1", "pane_id": "w2:p4", "focused": false, "revision": 1},
            {"terminal_id": "t3", "agent": "codex", "agent_status": "blocked", "workspace_id": "w1",
             "tab_id": "w1:t2", "pane_id": "w1:p9", "focused": true, "revision": 1},
            {"terminal_id": "t4", "agent_status": "unknown", "workspace_id": "w1",
             "tab_id": "w1:t1", "pane_id": "w1:p5", "focused": false, "revision": 1},
            {"terminal_id": "t5", "agent": "pi", "agent_status": "idle", "workspace_id": "w1",
             "tab_id": "w1:t1", "pane_id": "w1:p6", "focused": false, "revision": 1,
             "agent_session": {"source": "herdr:pi", "agent": "pi", "kind": "path", "value": "/s/pi.jsonl"}},
        ]}})
    }

    fn workspaces() -> Value {
        json!({"result": {"type": "workspace_list", "workspaces": [
            {"workspace_id": "w1", "number": 1, "label": "api"},
            {"workspace_id": "w2", "number": 2, "label": " "},
        ]}})
    }

    fn tabs() -> Value {
        json!({"result": {"type": "tab_list", "tabs": [
            {"tab_id": "w1:t1", "workspace_id": "w1", "number": 1, "label": "main"},
            {"tab_id": "w1:t2", "workspace_id": "w1", "number": 2, "label": "tests"},
        ]}})
    }

    #[test]
    fn agents_carry_their_workspace_and_tab_labels_and_skip_our_own_pane() {
        let listed = parse(&agents(), Some(&workspaces()), Some(&tabs()), Some("w1:p9"));
        let panes: Vec<&str> = listed.iter().map(|a| a.pane.as_str()).collect();
        assert_eq!(panes, ["w1:p1", "w2:p4", "w1:p6"], "our pane and the pane without an agent are left out");
        let first = &listed[0];
        assert_eq!(first.label(), "claude");
        assert_eq!(first.place(), "api › main");
        assert_eq!(first.cwd.as_deref(), Some(Path::new("/work/api/sub")), "the agent's own folder wins");
        assert_eq!(first.status, "idle");
        let second = &listed[1];
        assert_eq!(second.label(), "claude · reviewer");
        assert_eq!(second.place(), "w2 › w2:t1", "a blank or missing label falls back to the id");
        assert_eq!(second.cwd, None);
    }

    #[test]
    fn without_labels_the_ids_stand_in_and_garbage_lists_nothing() {
        let listed = parse(&agents(), None, None, None);
        assert_eq!(listed.len(), 4);
        assert_eq!(listed[2].place(), "w1 › w1:t2");
        assert_eq!(parse(&json!({"error": {"code": "server_not_running"}}), None, None, None), []);
        assert_eq!(parse(&json!("nonsense"), None, None, None), []);
    }

    #[test]
    fn a_herdr_pi_matches_the_pi_link_session_by_transcript_or_id() {
        let listed = parse(&agents(), None, None, None);
        let pi = listed.iter().find(|a| a.agent == "pi").expect("pi pane");
        assert!(is_pi_session(pi, "other", Some(Path::new("/s/pi.jsonl"))));
        assert!(!is_pi_session(pi, "other", Some(Path::new("/s/else.jsonl"))));
        let claude = &listed[0];
        assert!(!is_pi_session(claude, "x", None));
    }

    #[cfg(unix)]
    #[test]
    fn list_reads_the_three_herdr_lists() {
        let dir = std::env::temp_dir().join(format!("plannotator-tui-herdr-agents-{}", std::process::id()));
        let bin = fake_herdr(&dir, &agents());
        let listed = HerdrAgents { bin, own_pane: Some("w1:p1".into()) }.list();
        assert_eq!(listed.iter().map(|a| a.pane.as_str()).collect::<Vec<_>>(), ["w2:p4", "w1:p9", "w1:p6"]);
        assert_eq!(listed[1].place(), "api › tests");
        let calls = std::fs::read_to_string(dir.join("calls")).expect("calls");
        assert_eq!(calls, "agent list\nworkspace list\ntab list\n", "only reads");
        let missing = HerdrAgents { bin: dir.join("absent"), own_pane: None };
        assert_eq!(missing.list(), []);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
