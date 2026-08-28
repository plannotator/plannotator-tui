# Spec: annotate the agent's last message (`plannotator-tui last`)

Status: contract for phase 4, 2026-08-28. Detection and extraction rules are decision 9
(verified against Plannotator's source and this machine's `~/.claude` and `~/.codex`).

## Shape

- `plannotator-tui-hosts` (new lib crate): pure functions over strings and injected paths.
  Finds an agent's transcript and extracts its recent rendered messages. No spawning, no
  `~` lookups; the binary crate injects `sessions_dir`, `projects_dir`, a process-table
  snapshot, and env.
- `plannotator-tui last`: the CLI. Detects the host, finds the transcript, shows a picker of
  the newest messages, opens the chosen one as a transient document (`Provenance::AgentMessage`),
  and delivers feedback through the normal seam (clipboard standalone, the agent pane in Herdr).
- Herdr: `plannotator-tui herdr last` resolves the agent's pid from `herdr pane process-info`
  and opens the pane with `PLANNOTATOR_TUI_MESSAGE_PID`; `plannotator-tui herdr pane` is the
  pane entrypoint that reads the env and opens either a file or a message.

## `plannotator-tui-hosts` API (freeze this; F2 codes against it)

```rust
pub enum Host { ClaudeCode, Codex }
impl Host { pub fn label(self) -> &'static str }            // "claude", "codex"

pub enum Role { Human, Assistant }

/// One rendered message: every text block that shares the host's message id, in order.
pub struct Message { pub id: String, pub role: Role, pub text: String, pub at: Option<String> }

pub struct SessionMeta { pub pid: u32, pub session_id: String, pub cwd: PathBuf, pub started_at: u64 }

/// Env chain from decision 9; `PLANNOTATOR_TUI_HOST` overrides. Default Claude Code.
pub fn detect_host(env: impl Fn(&str) -> Option<String>) -> Host;

pub mod claude {
    pub fn parse_session_meta(json: &str) -> Option<SessionMeta>;
    pub fn project_slug(cwd: &Path) -> String;                       // [^A-Za-z0-9-] → '-'
    pub fn parse_ps(output: &str) -> Vec<(u32, u32)>;                // `ps -eo pid=,ppid=`
    /// The ladder (decision 9): ancestor pids → cwd scan → slug+mtime → parent dirs.
    /// `start_pid` is the process to walk up from (our ppid, or an explicit agent pid).
    pub fn find_transcript(
        sessions_dir: &Path, projects_dir: &Path, cwd: &Path,
        process_table: &[(u32, u32)], start_pid: u32,
    ) -> Option<PathBuf>;
    /// Newest `n` rendered messages on the active branch, newest first; falls back to file
    /// order when the chain is untrusted or the branch has no assistant text.
    pub fn parse_messages(jsonl: &str, n: usize) -> Vec<Message>;
}

pub mod codex {
    pub fn find_transcripts(codex_home: &Path, thread_id: Option<&str>) -> Vec<PathBuf>; // newest first, all files of the thread
    pub fn parse_messages(jsonl_files: &[String], n: usize) -> Vec<Message>;
}

pub enum HostError { NoTranscript(String), NoMessages(String), Io(std::io::Error) }
```

Human-prompt filter (decision 9) applies to `Role::Human`: not `isMeta`, not sidechain, not
`<local-command-…>` / `<command-name>` / `<system-reminder>` / `<system-notification>`
prefixes. The picker shows assistant messages by default; humans are kept for context only.

Fixtures: `crates/plannotator-tui-hosts/tests/fixtures/claude-code.jsonl` and `codex.jsonl`,
cut from real transcripts on this machine with every text body replaced by a short
placeholder that keeps the structure: a `parentUuid` branch (rewind), streamed chunks sharing
one `message.id`, `thinking`/`tool_use` blocks, a `tool_result` user entry, an `isMeta`
user entry, an `isSidechain` entry, bookkeeping entries without uuids written last, and a
`/compact` root. Tests are pure functions over those strings.

## Environment (adds to the phase-3 contract)

```
PLANNOTATOR_TUI_MESSAGE_PID   open the last message of the agent with this pid (launcher → pane)
PLANNOTATOR_TUI_HOST          claude | codex; overrides detection (any context)
PLANNOTATOR_TUI_SESSION       explicit transcript path; skips detection (any context)
```

`plannotator-tui herdr pane` precedence: `PLANNOTATOR_TUI_MESSAGE_PID` → `PLANNOTATOR_TUI_FILE`
→ `$PWD`. The delivery target is unchanged (`PLANNOTATOR_TUI_DELIVER_TO`).

## CLI

```
plannotator-tui last [--host H] [--pid N] [--session PATH] [--stdin] [--print] [--pick N]
```
- default: detect → find → picker of the newest 25 assistant messages → annotate → send.
- `--print`: newest message text on stdout, exit 0 (the delivery contract from decision 9).
- `--stdin`: the document is stdin; no detection.
- Errors name what was searched: "no Claude Code transcript for pid 1234 (looked in …)".

## Herdr

- Action `last` in the manifest → `plannotator-tui herdr last`: target pane = context focused
  pane (agent) or `HERDR_PANE_ID`; `herdr pane process-info --pane <id>` → the process whose
  name identifies the agent (`claude`, `codex`; else the foreground group leader) → pid; host
  from that name; then `plugin pane open` as `herdr open` does, with
  `PLANNOTATOR_TUI_MESSAGE_PID`, `PLANNOTATOR_TUI_HOST`, `PLANNOTATOR_TUI_DELIVER_TO`.
- In the pane, `find_transcript` starts at that pid (`sessions/<pid>.json` is a direct hit).
- Manifest pane command becomes `plannotator-tui herdr pane`.
