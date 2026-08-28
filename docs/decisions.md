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
