# Spec: annotate a folder, share a selection of it as a workspace

Status: draft for review, 2026-08-28. Sources: Workspaces API (`api-design/spec.yaml`), the
`workspaces` sync CLI (`projects/workspaces-cli`), and the two workspaces-ops handoffs.

## The problem in one sentence

A user opens a folder in plannotui and wants a teammate to annotate some of those files —
in the web app or their own terminal — without accidentally publishing the whole repo.

## What already exists, and what it means for us

**A workspace is a folder.** One namespace of paths per workspace; each path is a document
or an asset (409 on collision). Sharing is always the whole workspace, so *the set of
documents in the workspace is exactly the set of files shared.* That is the safety
property we build on: what is in the workspace is what is shared, nothing else.

**The `workspaces` sync CLI already does folder ↔ workspace.** Go binary, `workspaces
connect <ws_id|--create name> <folder>`, `.workspaces/state.sqlite` marks a connected
folder, a background service keeps it in two-way sync, conflicts freeze as
`*.conflict-local-*` files, and it handles the ugly parts (live-edit quiet window, case /
NFD collisions, 412 version conflicts). plannotui must not reimplement any of that.

**But the sync CLI uploads everything.** Its walk has no extension filter: every file under
the folder that is not in a fixed ignore list (`.git`, `node_modules`, `target`, `vendor`,
`dist`, `.env`, private keys, binaries, >1.5 MB, non-UTF-8) is uploaded, stamped
`kind: markdown`. It does not read `.gitignore` on purpose. **Connecting a repo root is
sharing the repo.** This is the exact accident the user is worried about, and it is the
thing plannotui adds value on: choosing.

## Design

### 1. Two folders, not one

plannotui never connects the folder the user is *browsing*. It connects a **share folder**:

```
~/.local/share/plannotui/shares/<share-id>/      # the synced folder (has .workspaces/)
    docs/plans/auth.md                           # selected files, at their relative paths
    docs/plans/auth.md.annotations.json          # (see §5)
```

The browsed folder stays untouched — no `.workspaces/` dropped into the user's repo, no
background service watching their source tree, nothing to accidentally commit. The share
folder contains only what the user selected. `workspaces connect` runs against the share
folder, so the sync CLI's whole-folder semantics become *exactly right*: everything in it
is meant to be shared.

Selected files are **hard-linked** into the share folder when the filesystem allows it
(same volume; macOS and Linux), so an edit in the repo is the same bytes in the share folder
with no copy step and no drift. Fallback is a copy plus a plannotui-owned watcher that
re-copies on change. Either way the sync CLI sees a normal file.

### 2. Selection is explicit, visible, and remembered

The tree pane gains a share column. Each row is one of:

| mark | meaning |
|---|---|
| `○` | not shared |
| `●` | shared (in the workspace) |
| `◐` | directory with some shared descendants |

Keys in the tree: `s` toggles the file (or every eligible file under a directory), `S`
opens the share panel. Toggling is local until the user confirms in the panel; nothing hits
the network on a keypress.

**Defaults are conservative.** On first share plannotui pre-selects *nothing*. It offers
one-key presets in the panel — "this file", "this folder (n files)", "all markdown under
docs/ (n files)" — each showing the count and total size before confirmation. There is no
"everything" preset. Eligibility for the presets is markdown only (`.md`, `.markdown`,
`.mdx`); other file types can be added one at a time with `s` but never by a preset.

**Refusals, not warnings.** plannotui refuses to add: anything the sync CLI would ignore
(same list — one function, ported, with a test that pins it to the Go list); anything
matched by the user's `.gitignore` / `.git/info/exclude` when inside a git repo (the sync
CLI skips this on purpose for context folders; we are inside code repos, so we honor it);
files over the document limit; symlinks. A refused file shows `✕` and the reason in the
panel. This is the second safety property: the categories of file that are almost never
meant to be shared cannot be shared from the tree at all.

**Confirmation shows the manifest.** Before creating or changing a workspace the panel lists
every path that will be added, kept, or removed, with sizes and the total, and the
workspace's visibility. The user confirms with `y`. Nothing is created before that.

The selection is remembered per browsed folder in `HERDR_PLUGIN_STATE_DIR` /
`$XDG_STATE_HOME/plannotui/shares.json`: `{ browsed_root, share_id, workspace_id,
api_origin, selected: [relative paths], created_at }`. Opening the same folder later shows
the same marks and the share panel offers "update" and "stop sharing".

### 3. The share flow

```
S  →  share panel
      ┌──────────────────────────────────────────────────────┐
      │ Share to Workspaces                                  │
      │                                                      │
      │ 3 files selected · 41 KB                             │
      │   docs/plans/auth.md             12 KB               │
      │   docs/plans/sessions.md         18 KB               │
      │   docs/adr/0007-tokens.md        11 KB               │
      │                                                      │
      │ workspace   plannotui-herdr-docs   (new)             │
      │ visibility  link_edit  ·  anyone with the link       │
      │             can read and comment                     │
      │                                                      │
      │ y create and share   n cancel   e edit selection     │
      └──────────────────────────────────────────────────────┘
```

On `y`:
1. Create the share folder; link/copy the selected files at their relative paths.
2. `workspaces connect --create "<name>" <share-folder>` (or `workspaces connect <ws_id>
   <share-folder>` for an update). The CLI creates the workspace, uploads, installs the
   background sync. plannotui shells out to it via `PATH` and parses its output; if the
   binary is missing, the panel says so and links the install instructions. **plannotui
   does not talk to the documents API for sync** — one engine, the CLI's.
3. `POST /v1/workspaces/{ws}/shares` → `share_url`, copied to the clipboard and shown.
4. Record the share in `shares.json`.

Update = change the selection, confirm: files added are linked in, files removed are
deleted from the share folder (the sync CLI propagates the delete; annotations on a deleted
document are the server's to keep or drop — see open question 3). Stop sharing =
`workspaces disconnect <share-folder>` and, on request, delete the workspace.

### 4. Annotations round-trip through the workspace, not the sync folder

The sync CLI syncs bodies, not annotations. plannotui's Workspaces client (phase 3) does the
annotation side: for a document in a shared folder, annotations are read from and written
to `/v1/workspaces/{ws}/documents/{doc}/annotations`, keyed by the document id the sync
CLI recorded in `.workspaces/state.sqlite` (`files.document_id` by path). The local sidecar
becomes a cache of the server rows plus an offline outbox. The mapping "browsed file →
share folder path → document id" is one lookup.

### 5. What the teammate sees

- **Web:** a workspace with only the shared files, comments anchored by `originalText`
  (phase 1 guarantees the match), and `plannotui`'s kinds rendered as ordinary comments.
- **Terminal:** `plannotui open <share_url>` (phase 3) — plannotui resolves the link to the
  workspace, connects its *own* share folder for it (read side of the same mechanism), and
  opens it in folder mode. Their annotations post to the same documents. Same UI, no
  install of anything beyond plannotui + the sync CLI.

### 6. Visibility

Anonymous creation makes a `link_edit` workspace with a 30-day idle expiry and no owner;
`POST /v1/workspaces/{ws}/claim` by a signed-in user makes it permanent. Signed-in
creation makes it `private`; a share link then widens it. plannotui's panel always states
the resulting visibility in plain words before `y`, and defaults to the signed-in path when
a `wsk_live_` key or a `workspaces login` is present. Expiry is shown in the panel for
anonymous shares.

## Non-goals (this slice)

- Sharing non-markdown files by preset. Assets (images) are a later, explicit action.
- Reimplementing sync, conflicts, or the quiet window. The CLI owns them.
- Live presence. Phase 6.
- Editing bodies from plannotui. It is an annotator; the sync CLI carries edits made in
  the user's editor.

## Answers from the API (verified in the Workspaces source, 2026-08-28)

1. **Path rules** (`packages/core/src/paths.ts:27`): `/` only (backslash rejected), no
   leading/trailing/double slash, no `.`/`..`/`.git` segment, no control chars or `\ ? # %`,
   255 UTF-8 **bytes** per component, no depth or total cap, **case-sensitive and
   byte-exact**, `.md` **not** required. NFC-equal-but-not-identical paths → `409
   path_conflict`; macOS gives NFD names, so normalize to NFC before upload. Documents and
   assets share one namespace.
2. **Ids**: the workspace id may be client-minted (`ws_` + 26 Crockford chars) and is the
   only create-idempotency handle — but it is **permanently single-use, even after
   deletion**. Document ids are always server-minted. `kind` is never inferred from the
   extension; send `kind: "markdown"` explicitly.
3. **Deleting a document** is permanent, no trash; its annotations go with it (keyed on
   document id). Deleting the last document leaves an empty workspace. **Rename/move is
   `PATCH {doc_path}` and keeps the id**, so annotations follow — never delete-and-re-add.
4. **Share links**: `POST /v1/workspaces/{ws}/shares {mode: view|edit}` → `{id, token,
   share_url}`; the token is shown **once**, no expiry field exists, revoke is immediate.
   `share_url` is `…/w/{wsId}?share=<token>`, so the teammate path parses the id and token
   from the URL. A link holder on `edit` can add/move/delete documents and delete **any**
   comment; can never delete the workspace.
5. **Sync CLI state** is a Go-private `state.sqlite`; do not read or share it. Read the
   document ids from `GET /v1/workspaces/{ws}` (`documents[]` by `doc_path`) instead.
6. **Rate limits apply to anonymous traffic only**; a human session or API key is exempt.
   Anonymous: 10 workspace mints/min, 60 writes/min per IP, `429` with `Retry-After`.
7. **Credentials**: `POST /v1/api-keys` needs a browser session — plannotui cannot
   bootstrap its own key; the user pastes one. Better: the **`cli_token` flow**
   (`GET /cli/login` loopback + PKCE), which counts as the human's own hand — never gated
   by workspace rules, never turned into a proposal. That is the credential to implement.
8. **Two gotchas that change behaviour**: an agent credential's `PUT`/`PATCH` may return
   `202` (a proposal, not applied) — another reason for `cli_token`; and an **anonymous
   `link_edit` workspace has no owner and can never be deleted** by anyone — always create
   with a credential.
9. **`workspaces connect` requires org membership** — a link-shared workspace cannot be
   connected as a folder by the teammate. So the "teammate opens the link in their
   terminal" path (3c) must use the REST API directly, not the sync CLI.

Full report: `.local/prd/herdr-knowledge/report-workspaces-folder-model.md` (herdr repo).

## Phasing

- **3a** (this spec, local half): share column, presets, refusal rules, share folder with
  hard links, `shares.json`, panel UI. Testable without a server: assert the share folder
  contents and the manifest.
- **3b**: shell out to `workspaces connect/disconnect`, mint the share link, annotation
  round-trip via the API. Needs a real account and the web app open.
- **3c**: `plannotui open <share_url>` for the teammate side — REST only (see answer 9):
  fetch the document list and bodies with `?share=`, annotate, post comments. No local
  sync folder for link holders.
