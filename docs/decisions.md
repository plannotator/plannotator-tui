# Decisions

Short records of choices that shape the code. Newest last. Each has the reason and the
source, so a future change can revisit the reason rather than the conclusion.

## 1. Anchor wire shape (2026-08-28)

Validated with workspaces-ops against the Workspaces code.

```json
{
  "originalText": "<rendered selection text>",
  "quote": "<same>",
  "plannotui": {
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
- Everything plannotui-only lives under one `plannotui` key. Top level carries only
  `originalText`, `quote`, and that namespace. Never claim `startMeta`, `endMeta`,
  `htmlAnchor`, `htmlAdditionalTargets`, `point`, `kind`, `state`, `source` at top level.
- `prefix`/`suffix` are RAW source, same coordinate system as the byte offsets. One system
  per object.
- The spec.yaml example `{block_id, start_offset, original_text}` is stale: it saves and
  renders as an unanchored card. Not used.

## 2. Kinds are not bodies (2026-08-28)

`looks_good` and `delete` are tags in the namespace, not body text. The body is the human
sentence a teammate should read (may be empty for looks_good). The web shows an ordinary
comment; plannotui shows the glyph. No verdict convention exists on the annotation surface
(verdicts are approval rounds, a different object).

## 3. Cross-block selections join with no separator (2026-08-28)

Verified by workspaces-ops with an executed DOM test through the real viewer: the rendered
DOM contributes no character between blocks (`textContent` is `"...words.The second..."`),
so a cross-paragraph `originalText` resolves only when the block texts are joined with
**no separator**. `"\n\n"` and space joins both fail (the whitespace-collapse fallback
normalizes toward a space the haystack never has). This is what the browser's own
`Range.toString()` produces, which is why in-app cross-block annotations already work.

So: `originalText`/`quote` = rendered block texts concatenated directly. The raw-source
form with real newlines stays under `plannotui.source`. No clamping. An upstream ask is
filed so `"\n\n"` eventually works too; we do not wait on it.

## 4. Document version = git blob sha (2026-08-28)

The document `ETag` is `sha1("blob " + len + "\0" + bytes)` over the exact bytes returned —
byte-identical to `git hash-object`. Computed locally; used as `plannotui.source.version`.
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

## 8. plannotui owns "last message" extraction (2026-08-28)

A standalone app cannot depend on the Plannotator CLI (a Bun + browser install) to read an
agent's last reply. `plannotui last` extracts it itself, in a `plannotui-hosts` crate: one
small module per agent behind one trait — `detect`, `last_message`, `deliver`. Claude Code
first (JSONL session log: last assistant entry on the active branch, text blocks joined),
Codex second; others on demand. Each host module carries one real transcript fixture and
one test. The Plannotator sources (`apps/hook/server/session-log.ts`, `codex-session.ts`)
are the format reference — knowledge copied, not code. Decision 6 is unchanged: the
message is a transient document source, delivery is a seam, and inside Herdr `deliver`
may target the pane's agent.

## 9. `plannotui last`: detection and extraction, from the Plannotator source (2026-08-28)

Read directly from `/Users/ramos/plannotator/plannotator` (`apps/hook/server/index.ts:505-520`,
`:1348-1520`; `session-log.ts`; `codex-session.ts`) and verified on this machine. This is the
reference for `plannotui-hosts`; knowledge copied, not code.

**Host detection is an env-var chain, then a fallback.** `PLANNOTATOR_ORIGIN` override
(validated) > `CODEX_THREAD_ID` > `COPILOT_CLI` > `OPENCODE` > `GEMINI_CLI` > `OMPCODE`
(last: OMP exports it into every shell it spawns) > default Claude Code. We mirror it with
`PLANNOTUI_HOST` as the override. `cwd` is never identity.

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

**Explicit input first.** `plannotui last --stdin` and `PLANNOTUI_HOST`/`PLANNOTUI_SESSION`
overrides ship before any detection, so the tool is usable and testable without a host.

**Tests.** Every resolver and parser is a pure function over strings/paths with injected
`sessions_dir`, `projects_dir`, and a `parent_pid` closure — never the real `~/.claude`,
never a spawned `ps`. Plannotator's `session-log.test.ts` (1,621 lines) is the regression
inventory: slug rules, human-prompt filtering, last-message extraction edge cases, picker,
active branch, rewind, compact, ancestor walk, ancestor pids, cwd scan, `ps` parsing,
cross-platform cwd compare. Freeze the JSONL entry shape and our stdout contract; do not
freeze Codex file layout or anything derived from process tables.
