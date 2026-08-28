# Spec: local annotation storage and the folder experience

Status: reviewed with plannotator-ops 2026-08-28; conventions below are confirmed against the Plannotator source. Supersedes the sidecar-next-to-the-file storage of
phase 2 for durable records; the sidecar remains as a per-file working copy (see §4).

## Why this matters

People keep their annotations and review them over time; that data is valuable. plannotator-tui
saves every annotation as JSON, automatically, in the Plannotator data dir under its own
directory, keyed the same way Plannotator keys files. Any agent can read the JSON.

## How Plannotator stores annotations today (verified in the source)

Data dir: `$PLANNOTATOR_DATA_DIR` if set; else an existing `~/.plannotator`; else
`$XDG_DATA_HOME/plannotator` when that is set; else `~/.plannotator`.
(`packages/shared/data-dir.ts`.)

```
~/.plannotator/
  plans/                                    # plan reviews: <slug>-<date>-{approved,denied}.md
    <slug>-<date>.annotations.md            #   the feedback markdown for that review
  history/<project>/<slug>/                 # annotate sessions, keyed by file path
    001.md, 002.md, …                       #   version snapshots of the annotated file
    submissions/<ISO timestamp>.md          #   one durable record per submitted review
  projects.json                             # [{name, cwd, lastSeen, parentCwd?, branch?}]
```

- `project` = `git rev-parse --show-toplevel` basename, else the cwd basename, else
  `_unknown`; each candidate passes through `sanitizeTag` (`packages/core/project.ts:15`):
  lowercase, trim, spaces and underscores → `-`, strip anything outside `[a-z0-9-]`,
  collapse hyphens, trim edge hyphens, cap at 30, and **null under 2 chars** (which falls
  through to the next candidate). (`packages/server/project.ts:15-45`.)
- `slug` = `annotate-<base>-<8hex>` where base = basename lowercased, `[^a-z0-9]+` → `-`,
  edge hyphens trimmed, ≤60, fallback `document`; and the 8 hex chars are the first 8 of
  `sha256(path)` hex (`contentHash` is `sha256.hex[..16]`, the slug takes `[..8]` of that).
  **The hash input is the path exactly as `path.resolve` produced it — not realpath'd, not
  lowercased.** Keyed by path, so one file always lands in one directory.
  (`packages/shared/annotate-history.ts:51-58`, `packages/shared/draft.ts:29`.)
- A submission record is markdown: a `# Annotate feedback` header, `- Source: <path>`,
  `- Decision: feedback | approved (with notes)`, `- Submitted: <ISO>`, `---`, then the
  exported feedback text — the same `## N. Feedback on: "…"` document an agent receives.
  (`annotate-history.ts:134`.) No structured JSON is kept when text exists. Filename:
  `toISOString()` with `:` and `.` → `-`, plus `.md` (`storage.ts:279`).
- Version snapshots dedupe by **byte equality against the latest version only**; padding
  is `padStart(3, "0")` but readers accept any width (`storage.ts:236-245`). The version
  browser lists only `/^\d+\.md$/` in the slug root (`storage.ts:331`).
- There is no lock, index, or manifest anywhere in `history/`. Everything is convention.
- Folder sessions snapshot each file on first open (version history) but write no
  submission records.
- Compound reads `plans/*-denied.md` and the feedback files; it is a markdown consumer.

## Design

**One rule: every annotation is saved to JSON, automatically, the moment it is made.**

```
~/.plannotator/
  clients/plannotator-tui/annotations/<project>/<slug>/annotations.json
```

- Same data-dir resolution and the same `project` / `slug` rules as Plannotator (above),
  so one file maps to one directory, and a future tool can join the two archives by path.
- `annotations.json` is the wire-shape `Annotation` array (phase 1) with the document
  version each anchor was made against. Rewritten atomically on every change — add, edit,
  remove, 👍, ✗. There is no submit, no export step, no session boundary.
- We write nothing anywhere else in the data dir. `history/`, `plans/`, and the rest are
  Plannotator's. No markdown records: the JSON is the record, and an agent reads JSON.
- `clients/plannotator-tui/` survives Plannotator's `uninstall --purge` (it only removes its own
  known entries); deleting it is the user's call.
- No sidecar next to the file. An existing phase-2 sidecar is imported on first open and
  left alone. Annotating a repo leaves no trace in it.
- Transient documents (an agent's last message, stdin) are never saved.

`E` copies the feedback text to the clipboard for pasting into an agent. It is a
convenience, not storage.

## Folder experience (local, no sharing)

What exists after phase 2: tree pane, `Tab` focus cycling, per-file open, per-file store.
This slice adds:

- **Annotation counts in the tree.** Each file row shows its count, dimmed at zero;
  directories show the sum.
- **Tree that stays out of the way.** `t` hides/shows it; auto-hides under 120 columns;
  `Tab` still reaches it.
- **Folder-wide export.** `E` in the tree copies feedback for every annotated file, with
  a `## File: <path>` heading per file.

## Not built

Version snapshots, submission records, a history view, `recent`, `--uninstall-data`,
sidecar opt-in. Each waits for a reader that needs it.

## Phasing

- **2b:** data-dir resolution, project/slug port with fixture tests, the
  `clients/plannotator-tui` JSON store, sidecar import, tree counts, tree toggle, folder export.
