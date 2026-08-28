# Spec: local annotation storage and the folder experience

Status: draft for review, 2026-08-28. Supersedes the sidecar-next-to-the-file storage of
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

- `project` = git repo name of the cwd (`git rev-parse --show-toplevel`, basename), else
  the directory name, else `_unknown`. (`packages/server/project.ts:23`.)
- `slug` = `annotate-<file basename lowercased, [^a-z0-9]+ → '-', trimmed, ≤60>-<first 8
  hex of contentHash(resolved absolute path)>`. Keyed by PATH, so the same file always
  lands in the same directory. (`packages/shared/annotate-history.ts:51`.)
- A submission record is markdown: a `# Annotate feedback` header, `- Source: <path>`,
  `- Decision: feedback | approved (with notes)`, `- Submitted: <ISO>`, `---`, then the
  exported feedback text — the same `## N. Feedback on: "…"` document an agent receives.
  (`annotate-history.ts:134`.) No structured JSON is kept when text exists.
- Folder sessions snapshot each file on first open (version history) but write no
  submission records.
- Compound reads `plans/*-denied.md` and the feedback files; it is a markdown consumer.

## Design

### 1. plannotui writes into the Plannotator data dir, under its own directory

```
~/.plannotator/
  tui/                                      # everything plannotui owns
    annotations/<project>/<slug>/
      annotations.json                      #   current structured set for the file
      submissions/<ISO timestamp>.json      #   structured record per export/submit
  history/<project>/<slug>/
    submissions/<ISO timestamp>.md          #   Plannotator's format, written by us too
```

Rules:
- Same data-dir resolution, same `project` and `slug` functions, ported exactly and
  pinned by tests against Plannotator's own fixtures. A file annotated in Plannotator and
  in plannotui shares one `history/<project>/<slug>/` directory.
- We never write under `plans/`, `drafts/`, or any other Plannotator-owned path, and never
  modify a file we did not create. `tui/` is ours; `history/…/submissions/*.md` we append
  to with the same format, with `- Client: plannotui` added as a fourth metadata line so a
  record's origin is visible and the compound skill parses it unchanged.
- The `contentHash` used in the slug is Plannotator's; we implement the same function (it
  is a short hash of the path string — see open question 1 for the exact algorithm).
- Respect `PLANNOTATOR_ANNOTATE_HISTORY=0` / `config.json { "annotateHistory": false }`:
  when history is disabled we write nothing under `history/`; `tui/` still holds the
  working set unless `PLANNOTUI_STATE=0`.

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
  `history/<project>/<slug>/NNN.md` exactly as Plannotator's folder sessions do, so the
  "what changed since I annotated" diff works in either tool. Deduped against the latest
  stored version by content.

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

## Open questions (plannotator-ops)

1. `contentHash` in `deriveAnnotateHistorySlug`: which function (sha256? fnv?) over which
   bytes, so our slugs are byte-identical.
2. Is `history/<project>/<slug>/submissions/*.md` safe for a second writer? Any index or
   lock we must honor, or a reader that assumes one `Source` per directory?
3. Does the compound skill (or anything else) parse the submission metadata lines
   strictly enough that an added `- Client:` line would break it?
4. Version snapshot dedupe: exact rule (`saveToHistory` compares content to the latest
   version only?) and the zero-padding width of `NNN.md`.
5. Should `tui/` be the name, or is there a naming convention for sibling clients?

## Phasing

- **2b (this spec):** data-dir resolution, project/slug port with fixture tests,
  `tui/annotations` store replacing the sidecar as authority, submission records in both
  formats on `E`, version snapshots, tree counts, tree toggle, folder export, recent.
- Later: history view, search over `tui/`.
