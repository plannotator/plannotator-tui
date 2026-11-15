# Pi link: plannotator-tui and Pi sessions talk to each other

Status: v1, built in this repo. The extension lives in `integrations/pi/`.

plannotator-tui sends review feedback to any running Pi session the person picks, and a Pi
session can leave notes in a plannotator-tui that has the file open. Both sides list
themselves in a registry folder and listen on loopback TCP with a per-process secret.
Nothing goes over the network.

## Registry

Root: `PLANNOTATOR_TUI_SESSIONS_DIR` when set and non-empty (trimmed; `~` is not
expanded), otherwise
`<Plannotator data dir>/clients/plannotator-tui/sessions`, beside plannotator-tui's notes
(Plannotator itself uses `<data dir>/sessions`). The data dir follows Plannotator's rule
(`PLANNOTATOR_DATA_DIR` → an existing `~/.plannotator` → `$XDG_DATA_HOME/plannotator` →
`~/.plannotator`), the same one plannotator-tui keeps notes in.

- `<root>/pi/<session id>-<pid>.json`: one per Pi session.
- `<root>/tui/<pid>.json`: one per running plannotator-tui.

Folders are created mode 0700 and entries mode 0600. Writers write a temporary file in the
same folder and rename it over the entry. Each side removes its own entry when it ends.

Paths in entries are as each process saw them. Resolve both sides to real paths before
comparing them (on macOS `/var/…` is `/private/var/…`).

Entries can be stale (a crash or `kill -9` leaves them). A reader treats an entry as live
only if its `pid` is running; for a stale entry the reader may delete it. A refused
connection also means stale.

### Pi entry

```json
{ "v": 1, "host": "pi", "id": "<session id>", "pid": 4121, "cwd": "/abs/dir",
  "name": "api-refactor", "transcript": "/abs/session.jsonl",
  "port": 50123, "token": "<64 hex chars>", "busy": false, "updated": 1763190000000 }
```

`name` and `transcript` may be `null`. `busy` is true from `agent_start` until
`agent_settled` (or `agent_end` where there is no `agent_settled`). `updated` is epoch
milliseconds of the last rewrite. The entry is rewritten when busy or the name changes, and
on every `session_start` (a `/new`, `/resume` or `/fork` removes the old entry first).

### plannotator-tui entry

```json
{ "v": 1, "host": "plannotator-tui", "pid": 5300, "cwd": "/abs/dir",
  "port": 50124, "token": "<64 hex chars>", "files": ["/abs/doc.md"],
  "updated": 1763190000000 }
```

`files` lists the absolute paths of the documents it can take notes for: the open file
first. It is rewritten when the open file changes. In a code review (`plannotator-tui
review`) it lists every changed file in the diff, and is rewritten when `r` recaptures it.

## Wire

Connect to `127.0.0.1:<port>`, write one JSON object and `\n`, read one JSON line back,
close. A request line is at most 1 MiB. Any request whose `token` differs from the entry's
gets `{"ok":false,"code":"bad_token"}`. Unknown fields are ignored; an unknown `op` gets
`{"ok":false,"code":"bad_request"}`.

### To a Pi session

Send feedback as a user message:

```json
{ "v": 1, "token": "…", "op": "send", "text": "…", "deliver": "auto", "source": "plannotator-tui" }
```

- `deliver: "auto"` (default): when Pi is idle, the text starts a turn; when busy, it waits
  in Pi's follow-up queue and Pi reads it when the current work is done.
- `deliver: "steer"`: when busy, it goes in after the current tool calls.
- Reply: `{"ok":true,"disposition":"started"|"queued"}`, or
  `{"ok":false,"code":"empty"|"failed","message":"…"}`.
- `ok` means Pi accepted the message, not that a model answered it.

Check a session:

```json
{ "v": 1, "token": "…", "op": "ping" }
```

Reply: `{"ok":true,"busy":false,"name":"…","id":"…"}`.

### To a plannotator-tui

`ping` answers `{"ok":true,"pid":…}`. Leave a note:

```json
{ "v": 1, "token": "…", "op": "note", "file": "/abs/doc.md", "quote": "exact text",
  "line": 42, "note": "…", "author": "pi · api-refactor" }
```

- `file` is absolute. `quote` is text copied exactly from the file's source. `line` is an
  optional 1-based source line. With both, the note goes on the occurrence of `quote`
  nearest that line; with only `quote`, on its first occurrence; with only `line`, on the
  block that holds that line; with neither, on the first block.
- A code review takes the same fields. `file` is a changed file. `line` is a line of the
  new version, and the diff must show it (an added or context line in a hunk); the note is
  a comment on that line. With `quote`, the note goes on the new-side line holding it that
  is nearest `line`. With neither, the note is on the whole file. A deleted file takes only
  file notes.
- Reply: `{"ok":true}`, or `{"ok":false,"code":…,"message":"…"}` with `code` one of
  `not_open` (that file is not the open document, or not a changed file of the code
  review), `not_in_diff` (a code review does not show that line), `quote_not_found`,
  `empty`, `busy` (the person is typing a note; try again shortly), `bad_request`.

## The Pi extension

`integrations/pi/plannotator-tui.ts`, one file, no dependencies beyond Node and Pi.
`plannotator-tui pi install` copies it to `~/.pi/agent/extensions/plannotator-tui.ts`
(honouring `PI_CODING_AGENT_DIR`).

- Registers the session on `session_start`, removes it on `session_shutdown`, keeps `busy`
  and `name` current.
- Answers `send` and `ping` as above.
- Adds the tool `plannotator_tui_note` `{ file, quote?, line?, note }`: it finds a live
  plannotator-tui whose `files` include the file (resolved against the session's cwd) and
  sends `note`, trying the next candidate on `not_open` or a refused connection. Its
  result says where the note went or why it could not; besides the wire codes it can be
  `no_plannotator_tui` (no running plannotator-tui has the file open) or `failed: <reason>`.
- Adds the command `/plannotator-tui`, which lists the running plannotator-tui apps and the
  files they have open.

## plannotator-tui

- Registers itself on start in an interactive review and removes the entry on exit.
- `P` (`pick-target`) opens the Pi picker: live Pi sessions, the ones in the document's folder first. The
  pick becomes the Send target until changed; the picker also offers going back to the
  target the review started with.
- A note from Pi is added to the open document, or to the diff in a code review, with the
  author shown, and the footer says so. Notes from Pi are sent with the rest of the review like any other note.
- `plannotator-tui pi list` prints live Pi sessions. `plannotator-tui pi send --session <id>`
  sends stdin to one.
