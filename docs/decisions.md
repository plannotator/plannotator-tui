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

## 9. `plannotui last` design, from Plannotator's regressions (2026-08-28)

From plannotator-ops, who own the reference implementation (a few hundred lines of
extraction, several thousand of resolution machinery grown from regressions). Rules:

**Detection is a chain, deterministic, no skill required.**
1. Explicit override first: `PLANNOTUI_HOST` / `PLANNOTUI_SESSION` env vars (validated).
2. Per-host env fingerprints: `CODEX_THREAD_ID` (authoritative for Codex), `OMPCODE`, etc.
3. Claude Code: `~/.claude/projects/<slug(cwd)>/<session>.jsonl`, candidates ranked by
   mtime, with an ancestor-directory walk for a cwd below the project root.
4. Last resort only: ancestor-pid walk against registered sessions (Copilot sets nothing).
   Parse `ps` output through pure functions so tests never spawn it.
cwd alone and parent-process names are not identity. Nothing assumes a host sets anything.

**Claude Code extraction.** Entries form a TREE via `parentUuid`. `/rewind` writes nothing;
the newest entry is simply off the active branch. Walk from the newest uuid-bearing entry
back to root; only entries on that path count. Then: assistant entries only; skip
tool_use/tool_result, thinking blocks, partial/streaming entries, compaction summaries,
and subagent sidechain transcripts. The spec phrase is **"the last assistant entry on the
active branch that renders text"** — an assistant entry can be tool-calls-only.

**Codex extraction.** `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl`, thread id
is the uuid in the filename. One thread spans MULTIPLE files: collect all files for the
thread from day one, walk backward across them, and stop before the active turn (or you
annotate the message being written). Reference: Plannotator issue #1367 / PR #1387.

**Always offer a picker.** "Which message" is a UI problem as much as a resolution problem:
show the last N rendered messages, default to the resolved one. The picker forgave every
resolution bug Plannotator ever shipped.

**Explicit input first.** `plannotui last --stdin` and the env overrides ship before any
divination exists, so the tool is usable on day one and testable without a host.

**Delivery contract is frozen before code.** Plain mode: feedback on stdout, **exit 0
even on zero-resolve or handoff** (a non-zero exit from a Bash bang-prefix aborts the
prompt before the model sees anything). `--json`: one machine-readable decision record on
stdout, nothing else. Hook mode emits `{"decision":"block","reason":"<feedback>"}`. Plain
markdown, no bracketed paste, no size framing. Hosts may run us under a shell timeout that
kills the process group (OpenCode: 120 s); say so in skill text and surface submit failures.

**Fixtures.** Freeze: Claude Code JSONL entry shape, our stdout/exit contract, Codex
`response_item/message/output_text` entry shape. Do not freeze: Codex file layout,
Copilot session-state layout, anything derived from process tables. Every parser is a pure
function over a string; tests never touch the real `~/.claude` or `~/.codex`.
