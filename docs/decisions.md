# Decisions

Short records of choices that shape the code. Newest last. Each has the reason and the
source, so a future change can revisit the reason rather than the conclusion.

## 1. Anchor wire shape (2026-08-28)

Validated with workspaces-ops against the Workspaces code.

```json
{
  "originalText": "<rendered selection text>",
  "quote": "<same>",
  "plannotator-tui": {
    "kind": "comment" | "looks_good" | "delete",
    "source": { "start": <byte>, "end": <byte>, "version": "<git blob sha of the raw markdown>" },
    "prefix": "<up to 32 chars of RAW source before>",
    "suffix": "<up to 32 chars of RAW source after>",
    "block": <top-level block index hint>
  }
}
```

- The server stores the anchor opaquely (16 KiB cap; only a top-level `point` is inspected).
- The web client renders from `originalText` (falls back to `quote`): exact substring over the
  rendered text, first occurrence, whitespace-collapsed fallback. It ignores everything else.
- Everything plannotator-tui-only lives under one `plannotator_tui` key. Top level carries only
  `originalText`, `quote`, and that namespace. Never claim `startMeta`, `endMeta`,
  `htmlAnchor`, `htmlAdditionalTargets`, `point`, `kind`, `state`, `source` at top level.
- `prefix`/`suffix` are RAW source, same coordinate system as the byte offsets. One system
  per object.
- The spec.yaml example `{block_id, start_offset, original_text}` is stale: it saves and
  renders as an unanchored card. Not used.

## 2. Kinds are not bodies (2026-08-28)

`looks_good` and `delete` are tags in the namespace, not body text. The body is the human
sentence a teammate should read (may be empty for looks_good). The web shows an ordinary
comment; plannotator-tui shows the glyph. No verdict convention exists on the annotation surface
(verdicts are approval rounds, a different object).

## 3. Cross-block selections join with no separator (2026-08-28)

Verified by workspaces-ops with an executed DOM test through the real viewer: the rendered
DOM contributes no character between blocks (`textContent` is `"...words.The second..."`),
so a cross-paragraph `originalText` resolves only when the block texts are joined with
**no separator**. `"\n\n"` and space joins both fail (the whitespace-collapse fallback
normalizes toward a space the haystack never has). This is what the browser's own
`Range.toString()` produces, which is why in-app cross-block annotations already work.

So: `originalText`/`quote` = rendered block texts concatenated directly. The raw-source
form with real newlines stays under `plannotator-tui.source`. No clamping. An upstream ask is
filed so `"\n\n"` eventually works too; we do not wait on it.

## 4. Document version = git blob sha (2026-08-28)

The document `ETag` is `sha1("blob " + len + "\0" + bytes)` over the exact bytes returned —
byte-identical to `git hash-object`. Computed locally; used as `plannotator-tui.source.version`.
When comparing, strip `W/` and quotes first.

## 5. Polling cannot use `?since=` for updates (2026-08-28)

`since` filters on created time. Resolves and edits do not appear. Phase 3 polls the full
per-document list (small) and diffs by id + state + updated_at. If that gets expensive it
becomes a contract ask for a changed-signal.

## 6. Documents are sources, not files (2026-08-28)

From plannotator-ops on the "last message" capability: the app opens a *document source* —
content, display name, `transient` flag, opaque provenance — not a path. A file is one
source; an agent's last message handed in as a string is another. Transient sources have
no sidecar, no history, no drafts. Delivery is a separate seam: on submit, feedback goes
back to wherever the source came from (a file's sidecar, a Workspaces document, an agent
pane via Herdr). Transcript extraction is its own crate behind the source seam (decision
8); the app core never sees transcript formats.

## 7. No abstraction ahead of a second caller (2026-08-28)

Traits exist only at seams to external systems: document source, delivery, Workspaces
client, clipboard. Everything else is concrete.

## 8. plannotator-tui owns "last message" extraction (2026-08-28)

A standalone app cannot depend on the Plannotator CLI (a Bun + browser install) to read an
agent's last reply. `plannotator-tui last` extracts it itself, in a `plannotator-tui-hosts` crate: one
small module per agent behind one trait — `detect`, `last_message`, `deliver`. Claude Code
first (JSONL session log: last assistant entry on the active branch, text blocks joined),
Codex second; others on demand. Each host module carries one real transcript fixture and
one test. The Plannotator sources (`apps/hook/server/session-log.ts`, `codex-session.ts`)
are the format reference — knowledge copied, not code. Decision 6 is unchanged: the
message is a transient document source, delivery is a seam, and inside Herdr `deliver`
may target the pane's agent.

## 9. `plannotator-tui last`: detection and extraction, from the Plannotator source (2026-08-28)

Read directly from `/Users/ramos/plannotator/plannotator` (`apps/hook/server/index.ts:505-520`,
`:1348-1520`; `session-log.ts`; `codex-session.ts`) and verified on this machine. This is the
reference for `plannotator-tui-hosts`; knowledge copied, not code.

**Host detection is an env-var chain, then a fallback.** `PLANNOTATOR_ORIGIN` override
(validated) > `CODEX_THREAD_ID` > `COPILOT_CLI` > `OPENCODE` > `GEMINI_CLI` > `OMPCODE`
(last: OMP exports it into every shell it spawns) > default Claude Code. We mirror it with
`PLANNOTATOR_TUI_HOST` as the override. `cwd` is never identity.

**Claude Code session resolution — the deterministic part.** Claude Code writes
`~/.claude/sessions/<pid>.json` per running session: `{pid, sessionId, cwd, startedAt, …}`
(verified: `1421.json` here belongs to the plannotator-ops session). Ladder, most precise
first, first hit wins:
1. **Ancestor-PID walk** (`resolveSessionLogByAncestorPids`): snapshot the process table
   once (`ps -eo pid=,ppid=`, parsed by a pure function), walk ≤8 parents from our ppid,
   read `sessions/<pid>.json` at each hop; match its `sessionId` to
   `~/.claude/projects/<slug(cwd)>/<sessionId>.jsonl`. Ghost check: if a NEWER jsonl exists
   in that project dir with no registered metadata, it is a `/clear` session — prefer it.
   This is why `plannotator last` works from a bare shell: the shell's ancestor IS claude.
2. **Cwd scan of session metadata**: all `sessions/*.json` whose `cwd` matches, newest
   `startedAt` first, matched to a jsonl.
3. **Slug + mtime**: `~/.claude/projects/<cwd with [^A-Za-z0-9-] → '-'>/*.jsonl`, newest
   first; case-insensitive dir fallback (Windows lowercases the slug).
4. **Ancestor-directory walk**: try each parent directory's slug (user `cd`'d deeper).
Each candidate is tried until one yields a message ("no messages" means "wrong file").

**Claude Code extraction** (`resolveActiveBranchIndices`, `extractRecentRenderedMessages`):
- Parse JSONL leniently (skip malformed lines). Entries carry `uuid`/`parentUuid`; bookkeeping
  types (`last-prompt`, `ai-title`, `mode`, `file-history-snapshot`) have none and are often
  written last, so "newest entry" = newest entry WITH a uuid.
- Active branch = walk `parentUuid` from that entry to the root (`parentUuid: null`).
  Untrusted chain (no ids, dangling parent, cycle) → fall back to file order.
- `/compact` writes a new root, so right after it the active branch may hold no assistant
  text: an empty result falls back to file order ("fail open, never fail empty").
- Skip: `progress`, `system`, `file-history-snapshot`, `queue-operation`; hidden visibility
  (`llm_only`, `assistant_only`, `hidden`); `isSidechain` subagent entries; non-text blocks
  (`thinking`, `tool_use`). Role = `type` or `message.role`.
- A rendered message = all `text` blocks of the same `message.id` (streamed chunks share
  it), concatenated in file order. Collect the newest N such messages for the picker
  (Plannotator uses 25); the newest is the default.
- A human prompt = `user` role, not hidden, has text, and does not start with
  `<local-command-`, `<command-name>`, `<local-command-stdout|stderr>`, `<system-reminder>`,
  `<system-notification>`.

**Codex.** `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl`; thread id = uuid in
the filename; scan date dirs newest-first. Entries: `type == "response_item"`,
`payload.type == "message"`, `payload.role == "assistant"`, text from `output_text` blocks.
Active turn: newest `event_msg` turn-start after the newest turn-complete → walk backward
from just before it. Multi-file threads (issue #1367, PR #1387 unmerged): collect ALL files
for the thread from day one and walk backward across them.

**Delivery contract** (`index.ts:338-350`, ours to freeze): plain mode prints feedback on
stdout, empty on close, "The user approved." on approve, **exit 0 always** (a non-zero exit
from a Bash bang-prefix aborts the prompt before the model reads it). `--json` prints one
`{"decision":"approved|dismissed|annotated","feedback":"…"}`. `--hook` prints nothing on
approve/close and `{"decision":"block","reason":"…"}` on annotate. No size framing, no
bracketed paste. Hosts may kill our process group on a shell timeout (OpenCode: 120 s).

**Explicit input first.** `plannotator-tui last --stdin` and `PLANNOTATOR_TUI_HOST`/`PLANNOTATOR_TUI_SESSION`
overrides ship before any detection, so the tool is usable and testable without a host.

**Tests.** Every resolver and parser is a pure function over strings/paths with injected
`sessions_dir`, `projects_dir`, and a `parent_pid` closure — never the real `~/.claude`,
never a spawned `ps`. Plannotator's `session-log.test.ts` (1,621 lines) is the regression
inventory: slug rules, human-prompt filtering, last-message extraction edge cases, picker,
active branch, rewind, compact, ancestor walk, ancestor pids, cwd scan, `ps` parsing,
cross-platform cwd compare. Freeze the JSONL entry shape and our stdout contract; do not
freeze Codex file layout or anything derived from process tables.

## 10. The anchor stores the raw quote; context only ranks (2026-08-28)

Found by a test in phase 1: reconstructing the quote from prefix/suffix is unsound — an
empty suffix matches anywhere, so a deleted quote "resolved" to whatever now sits after the
prefix. `plannotator-tui.quote` holds the selected raw-source text and is the only thing the
resolver searches for. The byte range is a shortcut (trusted only when it still holds the
quote between its context); prefix/suffix and the block hint only rank occurrences. A quote
that is not in the document is an orphan, always.

## 11. Submit is delivery, and delivery is a seam (2026-08-28)

The standalone app has no submit step: every annotation is saved to JSON as it is made, and
`E` copies the feedback text to the clipboard. "Send this review back to the agent" is a
Herdr concern, because only Herdr knows which agent sits in which pane.

The app is built so that is a plug, not a rewrite:

```rust
/// Where rendered feedback goes when the user sends it.
pub(crate) trait Delivery {
    fn describe(&self) -> String;              // shown in the footer: "send → clipboard"
    fn deliver(&self, feedback: &str) -> Result<()>;
}
```

- `Clipboard` is the only implementation in 2b, and the default everywhere.
- The Herdr plugin (phase 5) adds `HerdrAgent { pane_id }`: `herdr agent prompt <pane>
  <feedback>`. Verified in Herdr's source (2026-08-28): an overlay/split pane's
  `HERDR_PLUGIN_CONTEXT_JSON` is snapshotted **before** the new pane spawns
  (`src/app/api/plugins/panes.rs:51`), so `focused_pane_id` / `focused_pane_agent` name the
  pane that was focused when plannotator-tui was triggered — the agent's pane. An explicit
  `--deliver-to <pane>` overrides. `agent prompt` takes the text as one argument, honors
  the pane's bracketed-paste mode, sends Enter after 300 ms, and returns `agent_blocked`
  without sending if the agent is waiting on a dialog; the footer shows that. The footer
  always names the target.
- `plannotator-tui last` (phase 4) adds the host deliveries (stdout with exit 0, hook JSON).
- The feedback text is produced by one function (`export::feedback`) regardless of target,
  so what an agent receives from Herdr is byte-identical to what the clipboard gets.

This is decision 6's delivery seam made concrete. Nothing else in the app knows about
Herdr; `HERDR_ENV=1` only selects the implementation.

## 12. Placement is the user's, and one launcher serves humans and agents (2026-08-28)

Where plannotator-tui opens inside Herdr — full-screen overlay, split beside the agent, or a
modal popup — is a preference, not a property of the entry point. It lives in plannotator-tui's
own config (`[herdr] placement`, `~/.config/plannotator-tui/config.toml`), because Herdr has no
per-user plugin settings and a manifest cannot express "the user prefers". The parser is
strict: an unknown key is an error that names it, so a typo never silently falls back.
`PLANNOTATOR_TUI_PLACEMENT` and `--placement` override for one launch; the agent skill uses
`split` because watching the agent react is the point of that path.

All entry points run the same command, `plannotator-tui herdr open [PATH]`. A manifest action runs
it with Herdr's invocation context (the focused pane's folder, or the Ctrl-clicked
`file://` link, and the focused pane's agent as the feedback target); an agent runs it from
its own pane, where `HERDR_PANE_ID` is the caller and therefore the target. The launcher
resolves file, target and placement, then execs `herdr plugin pane open` with explicit
`PLANNOTATOR_TUI_FILE` / `PLANNOTATOR_TUI_DELIVER_TO` / `PLANNOTATOR_TUI_DELIVER_AGENT`, so the pane never
has to guess where it came from. Inside the pane `HERDR_PANE_ID` is plannotator-tui's own pane
and is never a target. `argv` construction is a pure function with exact-string tests.

Verified live (Herdr 0.8.2, disposable named session, a `cat` renamed `claude` standing in
for the agent): `agent prompt` delivers a multi-line feedback body verbatim; the manifest
action opens the overlay in folder mode on the agent's cwd; `--placement` on `plugin pane
open` overrides the manifest, so one `doc` entrypoint serves all three placements.

## 13. Sending is a button, and the record remembers it (2026-08-28)

Herdr is mouse-first, so the send is a visible control in the header, not only `E`. Its
label is derived from the delivery target and the send state — `Send 3 to claude ▸`,
`Sent ▸ claude`, `claude at a dialog · copied · click to retry`, or `Copy 3 as feedback`
outside Herdr — so the user always knows where feedback goes before pressing anything.
The button and `E` call the same function; there is no second path.

Delivery outcomes are typed (`Blocked`, `Unavailable`, `Failed`) because the app reacts
differently: a blocked agent means "retry later", so the text is also put on the clipboard
and nothing is lost; a missing agent means the clipboard is the target now.

`annotations.json` gains `deliveries: [{at, target, annotation_ids}]`. It is what lets the
button say "Sent" after a restart and, later, lets Workspaces know which comments the agent
has already seen. It is append-only and absent until the first send, so records written by
earlier builds load unchanged. A folder-wide send is recorded on the open file only; the
per-file state is a UI convenience, not an audit log.

## 14. What the real transcripts changed in decision 9 (2026-08-28)

`plannotator-tui-hosts` was built against decision 9 and then checked against this
machine's live Claude Code and Codex files. Four things the rules did not say:

- `attachment` entries carry `uuid`/`parentUuid` and sit inside the parent chain. The branch
  walk passes through them and rendering skips them; skipping them from the walk would make
  every chain look dangling.
- `/compact` is a `system` entry with `subtype: "compact_boundary"`, `parentUuid: null` and a
  `logicalParentUuid`, followed by a `user` entry flagged `isCompactSummary` /
  `isVisibleInTranscriptOnly`. The summary is not a human prompt. The file-order fallback
  after a compact works as specified.
- `<task-notification>` is another machine-written prefix on `user` entries; it joins the
  human-prompt filter.
- Codex assistant messages carry `phase` (`commentary` | `final_answer`); both are returned
  newest first so the final answer is the default pick. Subagent rollouts
  (`payload.source.subagent`) have their own thread ids and are excluded when choosing a
  thread — grouping by `session_id` would merge a reviewer's output into the main thread.

`visibility` and `isSidechain` did not appear in the sampled transcripts; the rules stay,
covered by synthesized fixture entries. The stdout contract (`--print`: newest reply, exit 0
always, errors on stderr) is frozen as decision 9 required.


Peer-reviewed against Plannotator's source at main by the plannotator-ops session
(2026-08-28): the attachment-in-chain rule matches Plannotator exactly; the compact-summary
exclusion, the `<task-notification>` prefix, the explicit Codex subagent skip, and
multi-rollout grouping are stricter than Plannotator (which has an open bug, #1367, on the
last one). Keep them; do not regress to match.

Reference survey (plannotator-ops, 2026-08-28, Plannotator at main): only Claude Code,
Codex, Copilot CLI and Droid have on-disk transcript readers there. Copilot sessions
(`~/.copilot/session-state/<uuid>/`) are found through `inuse.<pid>.lock` files matched
against the ancestor pids — in Herdr the pane's pid is a direct hit — with a cwd ladder as
the fallback (`copilot-session.ts`); a lock only counts while its pid still names a Copilot
process. Droid (`~/.factory/sessions/<slug>/`) reuses Claude's entry shape with `id` /
`parentId`, has no rewind tree and no per-process metadata, so its current session is the
newest log for the cwd's slug (else the first ancestor directory with logs), read in file
order and never falling through to an older sibling. Both are mirrored here with fixtures
cut from real sessions on this machine. OpenCode is an API bridge in Plannotator and Gemini
CLI is env-detected only; neither has a format to mirror, so in Herdr they fall back to the
pane's screen text.

**Pi** (2026-08-28). Sessions live in `~/.pi/agent/sessions/--<cwd with its leading slash
dropped and `/`, `\`, `:` as `-`>--/<timestamp>_<uuid>.jsonl` (`PI_CODING_AGENT_SESSION_DIR`
or `PI_CODING_AGENT_DIR` override; legacy flat files directly under `sessions/` still
exist). Every entry carries `id`/`parentId` — messages, model and thinking-level changes,
compactions, custom entries — so the active branch is the chain from the newest entry with
an id, which is what pi's own `getBranch()` returns (no lane records are written to v3 files;
operation records are in-memory only, so there is no on-disk turn marker). Unlike Claude
Code there is **no file-order fallback**: a chain that cannot be reconstructed yields nothing
rather than the wrong messages. Rendering matches Plannotator's `pi-extension/assistant-message.ts`
exactly: a `message` entry whose `content` is an array, text = the `text` blocks joined with
`\n`, whitespace-only (toolCall-only) entries skipped, one entry = one message keyed by the
entry id, timestamps normalized to ISO (numbers are Unix ms). There is
no pid registry, so a running pi is matched by cwd (the Herdr launcher passes the agent
pane's cwd as `PLANNOTATOR_TUI_CWD`), newest first, skipping sessions that hold no message
yet. pi exports `PI_CODING_AGENT=true` and `AI_AGENT=pi` into the shells it spawns; both
select the pi host after the Codex marker.

**Oh My Pi and Hermes CLI** (2026-08-29, plannotator-tui#24, #25). OMP is a pi harness: same
entry format, same encoded-cwd layout, rooted at `~/.omp/agent/sessions`, and it reuses pi's
`PI_CODING_AGENT_SESSION_DIR` / `PI_CODING_AGENT_DIR` overrides (`oh-my-pi/packages/coding-agent/src/cli/args.ts`),
so `omp` is pi's reader over another root. Its `OMPCODE` marker selects it last of all the
markers, as Plannotator orders them, and `AI_AGENT=omp` first, before pi's own flag which OMP
also sets. Hermes CLI has no transcript files: conversations are rows in SQLite
(`~/.hermes/state.db`, `HERMES_HOME` override, WAL mode), addressed by the session id Herdr
reports. The reader opens the database read-only (`mode=ro`, so the live WAL is visible),
falls back to `immutable=1` only when the shared-memory index cannot be mapped (a stale read
beats touching a running agent's store), and issues one query over
`idx_messages_session_active`, newest first. Both arrive from Herdr through `agent_session`
(`kind: path | id`) rather than host+pid discovery; a transcript path handed over without a
host name is recognised by its first lines (`sniff`), so any Herdr-integrated agent that
writes one of the known formats works without a host table entry.

Source-verified 2026-08-29 against `NousResearch/hermes-agent` (`hermes_state_common.py`,
`hermes_state.py`, `hermes_state_search.py`, `hermes_constants.py`) and `oh-my-pi`
(`packages/utils/src/procmgr.ts`): Hermes writes `messages.timestamp` with `time.time()`
(Unix seconds); `active=0, compacted=0` rows are rewinds the user took back and
`active=0, compacted=1` rows are compaction archives, so `active = 1` is the right filter;
the session id Herdr reports is `sessions.id`; `HERMES_HOME` else `~/.hermes`
(`%LOCALAPPDATA%\hermes` on Windows). omp exports only `OMPCODE=1` and `CLAUDECODE=1` to
child shells, no pi marker, so the marker chain reaches `OMPCODE` correctly.
