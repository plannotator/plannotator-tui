//! The agents running in Rex blocks, for the agent picker (`docs/spec-pi-link.md`).
//!
//! Rex has no agent list, so this builds one from reads. `rex ls` names the sessions and
//! `rex window ls` each session's windows (Rex's tabs) and the blocks in them. Each terminal
//! block's `process` and `program_status` methods then say which agent runs there, in which
//! folder and in what state, and its `title` what the agent calls its work. A block whose
//! program is no agent is left out, and so is the review's own block. A read that fails
//! drops only what it would have shown: an unreadable session lists none of its agents, an
//! unreadable title leaves the name out.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::Value;

use super::command::{self, short_id};
use super::launch::{foreground, reported_host};

/// The creator of terminal blocks, the only blocks an agent runs in.
const TERMINAL: &str = "com.superlogical.terminal";
/// How many blocks are read at once. Each read is a `rex` process; twenty agents take one
/// round instead of sixty calls in a row.
const PARALLEL: usize = 16;
/// The longest title a row shows, in characters, so the place column stays on screen.
const NAME_CHARS: usize = 28;

/// An agent in a Rex block, as the picker shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BlockAgent {
    /// The session the block is in, by ID: Send scopes its calls to it.
    pub(crate) session_id: String,
    /// The session's label, else its short ID.
    pub(crate) session: String,
    /// The window's (tab's) label, else its position.
    pub(crate) window: String,
    /// The block Send pastes into.
    pub(crate) block: String,
    /// The agent: `claude`, `codex`, `pi`, ...
    pub(crate) agent: String,
    /// The agent's process. Send refuses once another program runs in the block.
    pub(crate) pid: u32,
    /// The block's label when someone gave it one, else the title the agent set.
    pub(crate) name: Option<String>,
    /// `idle`, `working`, `blocked`, `error`, `done`, or `unknown` when the agent has not
    /// reported a state to Rex.
    pub(crate) status: String,
    /// The agent's folder.
    pub(crate) cwd: Option<PathBuf>,
    /// In the window the review runs in.
    pub(crate) here: bool,
    /// Rex's own order: session, window, block. Breaks ties between rows with one label.
    pub(crate) rank: (usize, usize, usize),
}

impl BlockAgent {
    /// `claude`, or `claude · Fix login redirect` when the block has a name.
    pub(crate) fn label(&self) -> String {
        match &self.name {
            Some(name) => format!("{} · {name}", self.agent),
            None => self.agent.clone(),
        }
    }

    /// Where the block is: `session › window · block`, the block as `rex ls` prints it.
    pub(crate) fn place(&self) -> String {
        format!("{} › {} · {}", self.session, self.window, short_id(&self.block))
    }
}

/// A terminal block that may hold an agent, with where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Candidate {
    session_id: String,
    session: String,
    window: String,
    block: String,
    /// The block's label, when it is not the one Rex gives every block of its flavor.
    label: Option<String>,
    here: bool,
    rank: (usize, usize, usize),
}

/// Reads the agents from one Rex: its binary, the session and block the review runs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RexAgents {
    pub(crate) bin: PathBuf,
    /// `REX_SESSION`: its agents come first.
    pub(crate) session: String,
    /// `REX_BLOCK`: never listed, it is the review's own block. Its window comes first.
    pub(crate) own_block: Option<String>,
}

impl RexAgents {
    /// Inside Rex (`REX_SESSION` set), the agents Rex runs; else `None`. Rex sets
    /// `REX_SESSION`, `REX_BLOCK` and `REX_BIN_DIR` in every block, so the review's own pane
    /// has them whether a launcher opened it or you ran it there.
    pub(crate) fn from_lookup(env: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let non_empty = |k: &str| env(k).filter(|v| !v.is_empty());
        let session = non_empty("REX_SESSION")?;
        let own_block = non_empty("REX_BLOCK");
        Some(Self { bin: command::bin_from(&env), session, own_block })
    }

    /// Every block with an agent except our own, the review's session first, then in Rex's
    /// order. Empty when Rex cannot be read.
    pub(crate) fn list(&self) -> Vec<BlockAgent> {
        let listed = self.read(&["ls", "--json"]);
        let mut found = listed.as_ref().map(sessions).unwrap_or_default();
        if !found.iter().any(|(id, _)| *id == self.session) {
            found.push((self.session.clone(), short_id(&self.session).to_owned()));
        }
        // The review's session first; the sort is stable, so the rest keep Rex's order.
        found.sort_by_key(|(id, _)| *id != self.session);
        let candidates: Vec<Candidate> = found
            .iter()
            .enumerate()
            .filter_map(|(at, (id, label))| {
                let view = self.read(&["window", "ls", "-s", id, "--json"])?;
                Some(blocks(&view, at, label, self.own_block.as_deref()))
            })
            .flatten()
            .collect();
        candidates
            .chunks(PARALLEL)
            .flat_map(|chunk| {
                std::thread::scope(|scope| {
                    let reads: Vec<_> = chunk.iter().map(|c| scope.spawn(move || self.agent(c))).collect();
                    reads.into_iter().filter_map(|read| read.join().ok().flatten()).collect::<Vec<_>>()
                })
            })
            .collect()
    }

    /// The agent in `candidate`'s block, if one runs there. Only an agent's title is read.
    fn agent(&self, candidate: &Candidate) -> Option<BlockAgent> {
        let call = |method: &str| {
            let method = format!("{TERMINAL}.{method}");
            self.read(&["do", &method, "-b", &candidate.block, "-s", &candidate.session_id])
        };
        let process = call("process")?;
        let status = call("program_status");
        let fg = foreground(&process)?;
        status.as_ref().and_then(|s| reported_host(s, fg.pid)).or(fg.host)?;
        let title = call("title");
        agent(candidate, &process, status.as_ref(), title.as_ref())
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

/// The sessions in `rex ls --json`, as (ID, label or short ID), in Rex's order.
fn sessions(listed: &Value) -> Vec<(String, String)> {
    let listed = listed.get("sessions").and_then(Value::as_array).into_iter().flatten();
    listed
        .filter_map(|session| {
            let id = session.get("session_id").and_then(Value::as_str).filter(|id| !id.is_empty())?;
            Some((id.to_owned(), label_or(session, short_id(id))))
        })
        .collect()
}

/// `entry`'s label, trimmed, else `fallback`.
fn label_or(entry: &Value, fallback: &str) -> String {
    entry
        .get("label")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

/// The terminal blocks in one session's `rex window ls --json`, window by window, in every
/// layer (tiled and floating), leaving out `own_block`. The session is the `at`th listed.
fn blocks(view: &Value, at: usize, session: &str, own_block: Option<&str>) -> Vec<Candidate> {
    let Some(session_id) = view.get("session_id").and_then(Value::as_str) else { return Vec::new() };
    let windows = view.get("windows").and_then(Value::as_array).into_iter().flatten();
    let mut found = Vec::new();
    for (position, window) in windows.enumerate() {
        let in_window: Vec<&Value> = window
            .get("layers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .flat_map(|layer| layer.get("blocks").and_then(Value::as_array).into_iter().flatten())
            .collect();
        let id = |block: &Value| block.get("block_id").and_then(Value::as_str).map(str::to_owned);
        let here = own_block.is_some_and(|own| in_window.iter().any(|b| id(b).as_deref() == Some(own)));
        let name = label_or(window, &(position + 1).to_string());
        for (index, block) in in_window.into_iter().enumerate() {
            let Some(block_id) = id(block).filter(|b| Some(b.as_str()) != own_block) else { continue };
            if block.get("creator_name").and_then(Value::as_str).is_some_and(|c| c != TERMINAL) {
                continue;
            }
            found.push(Candidate {
                session_id: session_id.to_owned(),
                session: session.to_owned(),
                window: name.clone(),
                block: block_id,
                label: given_label(block),
                here,
                rank: (at, position, index),
            });
        }
    }
    found
}

/// A block's label when someone gave it one: not empty, and not the name of its flavor,
/// which is what Rex labels every new block (`shell` for `com.superlogical.terminal.shell`).
fn given_label(block: &Value) -> Option<String> {
    let label = block.get("label").and_then(Value::as_str).map(str::trim).filter(|l| !l.is_empty())?;
    let flavor = block.get("flavor").and_then(Value::as_str).unwrap_or_default();
    let default = flavor.rsplit('.').next().unwrap_or_default();
    (label != default).then(|| label.to_owned())
}

/// The agent in `candidate`'s block from its `process`, `program_status` and `title`
/// replies; `None` when its foreground program is no agent. The agent is the one its status
/// report names, else the one its process names, as a launcher finds it.
fn agent(
    candidate: &Candidate,
    process: &Value,
    status: Option<&Value>,
    title: Option<&Value>,
) -> Option<BlockAgent> {
    let fg = foreground(process)?;
    let host = status.and_then(|s| reported_host(s, fg.pid)).or(fg.host)?;
    let name = candidate.label.clone().or_else(|| title.and_then(|t| agent_title(t, fg.pid, host, status)));
    Some(BlockAgent {
        session_id: candidate.session_id.clone(),
        session: candidate.session.clone(),
        window: candidate.window.clone(),
        block: candidate.block.clone(),
        agent: host.to_owned(),
        pid: fg.pid,
        name,
        status: state(status, fg.pid),
        cwd: fg.cwd,
        here: candidate.here,
        rank: candidate.rank,
    })
}

/// The agent's state from `program_status`: of the records process `pid` owns, the one that
/// most needs you. `unknown` when it owns none: it has not reported, or another program did.
pub(crate) fn state(status: Option<&Value>, pid: u32) -> String {
    const URGENCY: [&str; 5] = ["blocked", "error", "working", "idle", "done"];
    let records = status.and_then(|s| s.get("records")).and_then(Value::as_array).into_iter().flatten();
    records
        .filter(|r| r.pointer("/owner/pid").and_then(Value::as_u64) == Some(u64::from(pid)))
        // Rex: treat a state you don't know as idle.
        .map(|r| r.get("state").and_then(Value::as_str).filter(|s| URGENCY.contains(s)).unwrap_or("idle"))
        .min_by_key(|s| URGENCY.iter().position(|u| u == s))
        .unwrap_or("unknown")
        .to_owned()
}

/// The title the agent, process `pid`, set: without the spinner or mark an agent puts in
/// front, cut to [`NAME_CHARS`]. `None` when another program set it, or it only names the
/// agent (`codex`, or `Claude Code` for `claude-code`).
fn agent_title(title: &Value, pid: u32, host: &str, status: Option<&Value>) -> Option<String> {
    let owner = title.pointer("/owner/pid").and_then(Value::as_u64);
    if owner.is_some_and(|owner| owner != u64::from(pid)) {
        return None;
    }
    let text = title.get("title").and_then(Value::as_str)?;
    let text = text.trim_start_matches(|c: char| !c.is_alphanumeric()).trim();
    let plain = text.to_lowercase();
    let app = status
        .and_then(|s| s.get("records"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|r| r.pointer("/owner/pid").and_then(Value::as_u64) == Some(u64::from(pid)))
        .find_map(|r| r.get("effective_app").or_else(|| r.get("app")).and_then(Value::as_str));
    if text.is_empty() || plain == host || app.is_some_and(|app| plain.replace(' ', "-") == app) {
        return None;
    }
    Some(if text.chars().count() > NAME_CHARS {
        let cut: String = text.chars().take(NAME_CHARS - 1).collect();
        format!("{}…", cut.trim_end())
    } else {
        text.to_owned()
    })
}

/// The recorded fixtures in `tests/fixtures/rex-agents`: one Rex with sessions `work`
/// (window `api`: a shell, Claude, Codex; window `web`: a Claude at a permission prompt, pi)
/// and `side` (a Claude in a block labelled `release`), read from Rex 0.1.0-preview.22 with
/// stand-in agents and paths rewritten; `live-window-ls.json` is from a live Rex
/// 0.1.0-preview.21 with one shell.
#[cfg(test)]
pub(crate) fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rex-agents")
}

/// The block in the fixtures where the review runs: the shell in `work › api`.
#[cfg(test)]
pub(crate) const FIXTURE_OWN_BLOCK: &str = "block:01a1f658-7c0b-702a-a296-443d917d7f85";
/// The fixtures' `work` session.
#[cfg(test)]
pub(crate) const FIXTURE_SESSION: &str = "session:01a1f658-7c0a-71ad-8ff6-a1bcbc31504f";

/// A fake `rex` in `dir` for tests that answers the reads from the fixtures and records a
/// send. Every call's arguments go to `dir/calls`; `api call` writes its stdin to
/// `dir/pastes`, one line per paste.
#[cfg(all(test, unix))]
#[allow(clippy::expect_used, reason = "test fixtures assert by panicking")]
pub(crate) fn fake_rex(dir: &std::path::Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).expect("fake rex dir");
    for entry in std::fs::read_dir(fixture_dir()).expect("fixtures") {
        let path = entry.expect("fixture").path();
        std::fs::copy(&path, dir.join(path.file_name().expect("name"))).expect("copy fixture");
    }
    let script = format!(
        r#"#!/bin/sh
d='{d}'
echo "$*" >> "$d/calls"
case "$1 $2" in
  'ls --json') cat "$d/ls.json" ;;
  'window ls') f=$(grep -l "\"session_id\": \"$4\"" "$d"/window-ls-*.json | head -1); [ -n "$f" ] || exit 3; cat "$f" ;;
  do\ com.superlogical.terminal.*)
    m=${{2#com.superlogical.terminal.}}; b=$4; f="$d/${{b#"${{b%??????}}"}}.$m.json"
    [ -f "$f" ] || exit 3; cat "$f" ;;
  'api call') cat >> "$d/pastes"; echo >> "$d/pastes"; echo '{{}}' ;;
  'send-key -b'|'focus '*) ;;
  *) exit 2 ;;
esac
"#,
        d = dir.display()
    );
    let bin = dir.join("rex");
    std::fs::write(&bin, script).expect("script");
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    bin
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]
mod tests {
    use std::path::Path;

    use serde_json::json;

    use super::*;

    fn fixture(name: &str) -> Value {
        let text = std::fs::read_to_string(fixture_dir().join(name)).expect("fixture");
        serde_json::from_str(&text).expect("fixture JSON")
    }

    /// The agents in the fixtures, read the way `list` reads them, without a `rex`.
    fn recorded(own_block: Option<&str>) -> Vec<BlockAgent> {
        let mut listed = sessions(&fixture("ls.json"));
        listed.sort_by_key(|(id, _)| id != FIXTURE_SESSION);
        listed
            .iter()
            .enumerate()
            .flat_map(|(at, (_, label))| {
                blocks(&fixture(&format!("window-ls-{label}.json")), at, label, own_block)
            })
            .filter_map(|c| {
                let short = short_id(&c.block).to_owned();
                let read = |m: &str| fixture(&format!("{short}.{m}.json"));
                agent(&c, &read("process"), Some(&read("program_status")), Some(&read("title")))
            })
            .collect()
    }

    #[test]
    fn every_agent_in_every_window_and_session_is_found_and_the_shells_are_not() {
        let agents = recorded(Some(FIXTURE_OWN_BLOCK));
        let rows: Vec<String> =
            agents.iter().map(|a| format!("{} | {} | {}", a.label(), a.status, a.place())).collect();
        assert_eq!(
            rows,
            [
                "claude · Fix login redirect | idle | work › api · 9ba798",
                "codex | working | work › api · 31496a",
                "claude · Migrate styles | blocked | work › web · 63239c",
                "pi · π - notes | idle | work › web · eda7cc",
                "claude · release | idle | side › Window 122adf3e · fd82e9",
            ]
        );
        let here: Vec<bool> = agents.iter().map(|a| a.here).collect();
        assert_eq!(here, [true, true, false, false, false], "the review runs in work › api");
        assert_eq!(agents[0].cwd.as_deref(), Some(Path::new("/Users/me/work/api")));
        assert_eq!(agents[1].cwd.as_deref(), Some(Path::new("/Users/me/work/api/src")));
        assert_eq!(agents[0].pid, 94903);
        assert_eq!(agents[0].session_id, FIXTURE_SESSION);
        assert_eq!(agents[3].agent, "pi", "pi runs under node; its status report names it");
    }

    #[test]
    fn the_reviews_own_block_is_never_listed_and_without_it_no_window_is_here() {
        let own = "block:01a1f658-997c-79c0-ae26-1bf77b9ba798";
        let agents = recorded(Some(own));
        assert!(!agents.iter().any(|a| a.block == own));
        assert_eq!(agents.iter().filter(|a| a.here).count(), 1, "the Codex beside it");
        let anywhere = recorded(None);
        assert_eq!(anywhere.len(), 5);
        assert_eq!(anywhere.iter().filter(|a| a.here).count(), 0);
    }

    #[test]
    fn a_rex_with_only_a_shell_has_one_block_to_look_at() {
        // Recorded from a live Rex (session label changed).
        let live = blocks(&fixture("live-window-ls.json"), 0, "home", None);
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].label, None, "`shell` is the flavor's own label");
        assert_eq!(live[0].window, "Window 0eab11cc");
        let shell = agent(
            &live[0],
            &fixture("7d7f85.process.json"),
            Some(&fixture("7d7f85.program_status.json")),
            None,
        );
        assert_eq!(shell, None, "a shell is no agent");
    }

    #[test]
    fn the_state_is_the_agents_own_most_urgent_record() {
        let pid = 7;
        let status = |records: Value| json!({ "records": records });
        assert_eq!(state(None, pid), "unknown");
        assert_eq!(state(Some(&status(json!([]))), pid), "unknown");
        let other = status(json!([{"state": "working", "owner": {"pid": 8}}]));
        assert_eq!(state(Some(&other), pid), "unknown", "another program's record says nothing");
        let mixed = status(json!([
            {"state": "idle", "owner": {"pid": 7}},
            {"id": "build", "state": "blocked", "kind": "question", "owner": {"pid": 7}},
            {"id": "x", "state": "working", "owner": {"pid": 7}},
        ]));
        assert_eq!(state(Some(&mixed), pid), "blocked");
        let odd = status(json!([{"state": "dreaming", "owner": {"pid": 7}}]));
        assert_eq!(state(Some(&odd), pid), "idle", "Rex: an unknown state reads as idle");
    }

    #[test]
    fn a_title_loses_its_spinner_and_is_kept_only_when_the_agent_set_it_and_says_more() {
        let title = |text: &str, owner: u32| json!({"title": text, "owner": {"pid": owner, "name": "x"}});
        let claude = json!({"records": [{"state": "idle", "app": "claude-code", "effective_app": "claude-code", "owner": {"pid": 7}}]});
        let named = |t: &Value| agent_title(t, 7, "claude", Some(&claude));
        assert_eq!(named(&title("⠐ Fix login redirect", 7)).as_deref(), Some("Fix login redirect"));
        assert_eq!(named(&title("✳ Claude Code", 7)), None, "only the agent's name");
        assert_eq!(named(&title("claude", 7)), None);
        assert_eq!(named(&title("✳ Fix login redirect", 8)), None, "set by another program");
        assert_eq!(named(&json!({"title": ""})), None);
        assert_eq!(named(&json!({"title": "no owner known"})).as_deref(), Some("no owner known"));
        let long = named(&title("✳ Refactor the session restore path for windows", 7)).expect("named");
        assert_eq!(long, "Refactor the session restor…");
        assert_eq!(long.chars().count(), NAME_CHARS);
    }

    #[test]
    fn only_a_label_someone_gave_names_the_block() {
        let block = |label: &str, flavor: &str| json!({"label": label, "flavor": flavor});
        assert_eq!(given_label(&block("shell", "com.superlogical.terminal.shell")), None);
        assert_eq!(given_label(&block(" ", "com.superlogical.terminal.shell")), None);
        assert_eq!(
            given_label(&block("reviewer", "com.superlogical.terminal.shell")).as_deref(),
            Some("reviewer")
        );
    }

    #[test]
    fn outside_rex_there_is_nothing_to_read() {
        let env = |vars: &'static [(&'static str, &'static str)]| {
            move |k: &str| vars.iter().find(|(name, _)| *name == k).map(|(_, v)| (*v).to_owned())
        };
        assert_eq!(RexAgents::from_lookup(env(&[("REX_BLOCK", "block:b")])), None);
        assert_eq!(RexAgents::from_lookup(env(&[("REX_SESSION", "")])), None);
        let inside =
            RexAgents::from_lookup(env(&[("REX_SESSION", "session:s"), ("REX_BIN_DIR", "/opt/rex")]))
                .expect("rex");
        assert_eq!(inside.bin, PathBuf::from("/opt/rex/rex"));
        assert_eq!(inside.own_block, None);
    }

    #[test]
    fn sessions_without_an_id_are_skipped_and_garbage_lists_none() {
        let listed =
            json!({"sessions": [{"session_id": "session:abcdef123456", "label": " "}, {"label": "x"}]});
        assert_eq!(sessions(&listed), [("session:abcdef123456".to_owned(), "123456".to_owned())]);
        assert_eq!(sessions(&json!("nonsense")), []);
        assert_eq!(blocks(&json!({"windows": []}), 0, "s", None), []);
    }

    #[cfg(unix)]
    #[test]
    fn list_reads_every_session_and_only_reads() {
        let dir = std::env::temp_dir().join(format!("plannotator-tui-rex-agents-{}", std::process::id()));
        let bin = fake_rex(&dir);
        let rex =
            RexAgents { bin, session: FIXTURE_SESSION.into(), own_block: Some(FIXTURE_OWN_BLOCK.into()) };
        assert_eq!(rex.list(), recorded(Some(FIXTURE_OWN_BLOCK)));
        let calls = std::fs::read_to_string(dir.join("calls")).expect("calls");
        let verbs: std::collections::BTreeSet<String> =
            calls.lines().map(|l| l.split_whitespace().take(2).collect::<Vec<_>>().join(" ")).collect();
        assert_eq!(
            verbs.into_iter().collect::<Vec<_>>(),
            [
                "do com.superlogical.terminal.process",
                "do com.superlogical.terminal.program_status",
                "do com.superlogical.terminal.title",
                "ls --json",
                "window ls",
            ],
            "only reads"
        );
        assert!(!calls.contains(&format!("-b {FIXTURE_OWN_BLOCK}")), "our own block is not read");
        let titles = calls.lines().filter(|l| l.contains(".title")).count();
        assert_eq!(titles, 5, "one title per agent: {calls}");
        let gone = RexAgents { bin: dir.join("absent"), session: FIXTURE_SESSION.into(), own_block: None };
        assert_eq!(gone.list(), []);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
