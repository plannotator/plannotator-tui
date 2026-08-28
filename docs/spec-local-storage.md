# Spec: local annotation storage and the folder experience

Status: reviewed with plannotator-ops 2026-08-28; conventions below are confirmed against the Plannotator source. Supersedes the sidecar-next-to-the-file storage of
phase 2 for durable records; the sidecar remains as a per-file working copy (see §4).

## Why this matters

People keep their annotations. Plannotator users review what they wrote over months —
the `plannotator-compound` skill reads the archive to find patterns in what they reject
and how their feedback evolves. On this machine `~/.plannotator` holds 1,267 submission
records and 41 plan feedback files. plannotui's annotations should land in that same
archive so they compound with everything else, and should add what Plannotator's records
lack: the structured data.

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

### 1. plannotui writes into the Plannotator data dir, under its own directory

```
~/.plannotator/
  clients/plannotui/                        # everything plannotui owns
    annotations/<project>/<slug>/
      annotations.json                      #   current structured set for the file
      submissions/<stamp>.json              #   structured record per export/submit
    recent.json
  history/<project>/<slug>/
    NNN.md                                  #   version snapshots (shared with Plannotator)
    submissions/<stamp>-plannotui.md        #   Plannotator's format, written by us too
```

Rules:
- Same data-dir resolution, same `project` and `slug` functions, ported exactly and
  pinned by tests. A file annotated in Plannotator and in plannotui shares one
  `history/<project>/<slug>/` directory.
- **Rulings from plannotator-ops, honored as written:**
  - Our top-level directory is `clients/plannotui/`, not `tui/`. Plannotator's
    `uninstall --purge` removes only its own top-level entries and reports anything else as
    "preserved (unrecognized custom entry)", so `clients/` survives a purge and plannotui
    owns its own uninstall story (`plannotui --uninstall-data`, documented). The copies we
    append under `history/` **are** purge-owned and will be deleted with it. Both halves
    are correct; this asymmetry is deliberate and documented here so nobody is surprised.
  - Submission files we write are named `<stamp>-plannotui.md` — the client suffix makes a
    cross-writer filename collision structurally impossible (Plannotator's own collision
    probe is `existsSync`, which is racy across processes) and shows provenance in a
    listing. Nothing parses submission filenames beyond listing them.
  - We write **only** into `history/<project>/<slug>/submissions/` and `NNN.md` snapshots;
    never any other file into the slug root, whose namespace readers assume is snapshots.
  - `- Client: plannotui` is **appended after** the existing three metadata lines, never
    reordering them. Nothing parses submission bodies strictly today; that is an
    observation, not a guarantee, so we stay format-compatible.
  - Snapshots are created with an exclusive-create open (`O_EXCL`); on `EEXIST` re-scan for
    the next number and retry. Dedupe is byte-equality against the latest version only,
    reusing its number. That closes a two-writer race Plannotator's own writer still has.
- We never write under `plans/`, `drafts/`, or any other Plannotator-owned path, and never
  modify a file we did not create.
- Gates, matching Plannotator's: `PLANNOTATOR_ANNOTATE_HISTORY=0|false` (env wins) or
  `config.json { "annotateHistory": false }` disables **both** snapshots and submission
  records under `history/`. Transient documents (an agent's last message, stdin, a URL)
  never get history or submissions. `clients/plannotui/` still holds the working set
  unless `PLANNOTUI_STATE=0`.

### 2. Two records per file, with different lifetimes

- **`annotations.json`** — the live set: the wire-shape `Annotation` array (phase 1),
  including resolved/orphaned state and the document version each anchor was made
  against. Rewritten atomically on every change. This is what the app loads.
- **`submissions/<ts>.json` + `history/…/submissions/<ts>.md`** — an immutable snapshot
  written on **export** (`E`), on **submit** to a host (phase 4), and on **sync** to a
  workspace (phase 3). The `.md` is the human/agent feedback text in Plannotator's format;
  the `.json` is `{ document: {path, project, version}, exported_at, feedback: <text>,
  annotations: [...] }`. Nothing ever rewrites a submission.

That split gives the compound use its raw material (many dated records, markdown, same
place as before) and gives future tooling — a plannotui `history` view, search, a
"what did I say about this file last month" command — real data.

### 3. Sidecars become optional

The phase-2 `<file>.annotations.json` next to the document is useful in exactly one case:
a repo that wants annotations checked in. It becomes opt-in (`--sidecar`, or a
`.plannotui.toml` in the folder), off by default, so annotating a repo leaves no trace in
it. When both exist, `tui/…/annotations.json` is authoritative and the sidecar is a copy.

### 4. The folder experience (local, no sharing)

What exists after phase 2: tree pane, `Tab` focus cycling, per-file open, per-file store.
What this slice adds:

- **Annotation marks in the tree.** Each file row shows its count: `plans/auth.md  3`,
  dimmed when zero, so a folder's review state is visible at a glance. Directories show
  the sum. Counts come from `tui/annotations/<project>/` — one stat per file at scan.
- **Tree that stays out of the way.** `Ctrl-b`-style toggle: `t` hides/shows the tree; it
  auto-hides under 120 columns and is remembered per session. Hidden tree + `Tab` still
  reaches it (it appears while focused, hides again on leaving).
- **Folder-wide export.** `E` in the tree exports feedback for every annotated file in the
  folder as one document with a `## File: <path>` heading per file (numbering global, as
  Plannotator does for multi-page), and writes one submission record per file.
- **Recent.** `plannotui` with no argument opens the most recently annotated folder or
  file (`tui/recent.json`, ≤20 entries); `plannotui --recent` lists them.
- **Version snapshots.** On first open of a file in a session, snapshot it to
  `history/<project>/<slug>/NNN.md` exactly as Plannotator's folder sessions do (ruling:
  yes, write them — same slug means a version saved by one tool is the other's diff
  baseline). Byte-equal to the latest → no write; else exclusive-create with retry.

### 5. Migration from phase-2 sidecars

On opening a file that has a sidecar and no `tui/` record, import the sidecar into
`tui/annotations/<project>/<slug>/annotations.json` and leave the sidecar in place. No
data is moved or deleted.

## Non-goals

- Reading or displaying Plannotator's `plans/` archive. Different object (plan reviews
  with verdicts); compound covers it.
- A history browser UI. The data is laid out so one can exist; building it is a later
  slice.
- Sharing. `docs/spec-share-folder.md`, deferred.

## Resolved with plannotator-ops (2026-08-28)

All seven questions answered from the source; the answers are folded into §1–§4 above:
slug hash and input rules, data-dir order, `sanitizeTag` incl. the null-under-2 rule,
second-writer safety (no lock; client-suffixed filenames; submissions dir only), snapshot
dedupe and `O_EXCL`, the history gates, and the `clients/plannotui/` name with its purge
asymmetry.

## Phasing

- **2b (this spec):** data-dir resolution, project/slug port with fixture tests,
  `clients/plannotui/annotations` store replacing the sidecar as authority, submission
  records in both formats on `E`, version snapshots, tree counts, tree toggle, folder
  export, recent, `--uninstall-data`.
- Later: history view, search over `tui/`.
