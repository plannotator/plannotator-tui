# Viewed marks (review progress)

`plannotator-tui review` lets you mark a file viewed with `x`. The marks use the
browser app's own review-progress records, so a review you start in one app continues
in the other. This format belongs to the browser app. When the browser changes it, this
document and `crates/plannotator-tui-review/src/progress.rs` must change with it.

Browser sources, read when this was written:

- `packages/shared/review-progress.ts`: the key, the record, loading and saving.
- `packages/shared/review-core.ts`: `parseRawDiffEntries`, `resolveReviewFileIdentities`
  (the file fingerprint), `runGitDiff` (the git command per diff type),
  `MAX_REVIEW_FILE_CONTENT_BYTES`.
- `packages/server/review.ts`: `captureProgress` (which diff type and base go into the
  key) and the `main` to `origin/main` base upgrade.
- `packages/shared/config.ts`: `resolveReviewProgress`.

## Where records live

```
<data-dir>/review-progress/<key>/<sha256(path)>.json
```

`<data-dir>` is the Plannotator data directory: `PLANNOTATOR_DATA_DIR`, else
`~/.plannotator` (see `plannotator-tui-schema::data_dir`). There is one small file per
reviewed file. `sha256(path)` is the hex SHA-256 of the file's repository-relative path
(the new path; for a deleted file, the old path), as UTF-8.

A record is `JSON.stringify` of this object, with the fields in this order:

```json
{"v":1,"path":"src/a.ts","fingerprint":"<64 hex>","viewed":true,"suppressed":false}
```

`suppressed` is always `!viewed` when either app writes a record. The browser uses it to
let an explicit "not viewed" override GitHub's viewed state.

Both apps write a record to `<record>.<unique>.tmp` with mode 0600, then rename it over
the record. Two writers never corrupt a record; the last rename wins. A missing,
unreadable or malformed record means "not viewed". A record counts only when `v` is 1,
`path` equals the file's path, all fields have the right types, and `fingerprint` equals
the file's fingerprint in the current diff.

## The key

```
key = sha256(JSON.stringify(["git", realpath(root), head, [diffType, base]]))
```

- `root` is `git rev-parse --show-toplevel`, passed through `realpath`. For example, on
  macOS `/var/tmp/x` becomes `/private/var/tmp/x`.
- `head` is `git symbolic-ref --quiet HEAD` (`refs/heads/feat`). If HEAD is detached, it
  is `git rev-parse HEAD` (the full sha).
- `base` is the compare-against ref for `since-base`, `branch` and `merge-base`. For all
  other diff types it is `null`.
- The JSON has no spaces. A missing value is `null`, never omitted.

How this app's scopes map to the browser's diff types (`Scope::diff_type`):

| tui scope | diffType | base |
|---|---|---|
| uncommitted | `uncommitted` | `null` |
| `--staged` | `staged` | `null` |
| `--base REF` (and the default branch review) | `since-base` | the ref as the header names it, e.g. `origin/main` |
| `--commit SHA` | `commit:<full sha>` | `null` |

The browser compares against `origin/main` when `origin/HEAD` points there. This app's
default base `origin/HEAD` resolves to the same name, so their default branch reviews
share a key. The browser also rewrites a bare `main` to `origin/main` in some cases (a
default base it did not pin, after a network check). If one app shows `main` and the other
shows `origin/main`, the keys differ and the marks are not shared.

## The fingerprint

```
fingerprint = sha256(identity)
identity    = JSON.stringify([oldPath, newPath, oldMode, newMode, oldOid, newOid])
```

The values come from `git diff --raw -z --no-abbrev`, run as the browser runs it for that
diff type:

| diffType | command |
|---|---|
| `uncommitted` | `git diff HEAD`, plus each untracked file |
| `staged` | `git diff --staged` |
| `since-base` | `git diff <merge-base(base, HEAD)>`, plus each untracked file |
| `commit:<sha>` | `git diff <sha>^..<sha>` (the empty tree for a root commit) |

Both apps run the diff with `-c core.bigFileThreshold=5242880`.

- `oldPath` is `null` for an added file. `newPath` is `null` for a deleted file.
- Modes and ids are written as git prints them (`100644`, 40 or 64 hex digits).
- An untracked file is `[null, path, "000000", mode, <zeros>, newOid]`. Its mode is
  `100644`, `100755` for an executable file, or `120000` for a symlink.
- When git prints an all-zero new id (a working-tree side), the id is
  `git hash-object --stdin-paths` of the file, run at the root. For a symlink it is
  `git hash-object --stdin` of the link text. The hash must match the new id on the
  patch's `index` line. A file with no `index` line (a mode-only change) must hash to its
  old id. If it does not match, the file changed after the diff was taken, and it gets no
  fingerprint.
- These files get no fingerprint: submodules (mode `160000`), any side larger than 5 MiB,
  and paths that `--stdin-paths` cannot take (a line break, a carriage return, or a
  leading `"`). You can still mark such a file viewed, but the mark lasts only for the
  session.

An edit to a file changes its fingerprint, so its mark stops counting. The old record
stays on disk, but it no longer matches.

## Pull requests

The browser keys a pull request review as `["pr", url, "layer" | "full-stack"]`. Its
fingerprints come from GitHub's patch text (abbreviated `index` lines, so it hashes
`[path, patch]`). This app fetches the commits and diffs them locally, so it never has
that text byte for byte. For this reason, this app keeps pull request marks in its own
folder:

```
<data-dir>/clients/plannotator-tui/review-progress/<sha256(["pr", url, "layer"])>/<sha256(path)>.json
```

The records have the same shape. The fingerprints use the local identity above, taken
from `git diff <merge-base>..<head>` in the clone or the cache repository. Nothing is sent
to GitHub; this app does not read or set GitHub's viewed state.

## Turning it off

`PLANNOTATOR_REVIEW_PROGRESS=0|false|off|disabled` turns saving off, and `1|true|on`
turns it on. Without the variable, `reviewProgress` in `<data-dir>/config.json` decides
(`true`/`false`, or `"true"`/`"1"`/`"false"`/`"0"`). The default is on. When saving is
off, marks last for the session only, and no records are read.

## Keys

| key | action | does |
|---|---|---|
| `x` | `toggle-viewed` | Marks the file under the cursor viewed, or not. |
| `ctrl+n` | `next-unviewed` | Goes to the next file not viewed, in diff order, wrapping around. |
| (none) | `previous-unviewed` | Goes to the previous file not viewed. Bind it under `[keys]`. |

`n`/`N` still move one file at a time. `ctrl+n` is `n` with a modifier, so it is "next
file", skipping the viewed ones. Most terminals send Ctrl+Shift+N as Ctrl+N, so the
backward jump has no default key.

The header counts the marks (`3/12 viewed`). The file list (`f`) shows `✓` after a viewed
file's counts and dims its row; a folder gets the tick when every file in it is viewed,
also when it is collapsed. `H` hides the viewed files from the list and the diff; a file
marked while they are hidden leaves both, and the diff goes on to the next one. The footer shows `✓ viewed` when the file under the cursor is
marked.
