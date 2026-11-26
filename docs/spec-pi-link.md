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

Entries can be stale (a crash or `kill -9` leaves them). plannotator-tui treats a Pi entry
as live only if its port answers a `ping`; a refused connection means stale, and it deletes
that entry. (The extension checks a plannotator-tui entry's `pid` the same way.)

### Pi entry

```json
{ "v": 1, "host": "pi", "id": "<session id>", "pid": 4121, "cwd": "/abs/dir",
  "name": "api-refactor", "transcript": "/abs/session.jsonl",
  "leaf": "0c0c0c0c", "tip": "0c0c0c0c",
  "port": 50123, "token": "<64 hex chars>", "busy": false, "updated": 1763190000000 }
```

`name` and `transcript` may be `null`. `busy` is true from `agent_start` until
`agent_settled` (or `agent_end` where there is no `agent_settled`). `updated` is epoch
milliseconds of the last rewrite. The entry is rewritten when busy or the name changes, on
`/tree` navigation (`session_tree`), and on every `session_start` (a `/new`, `/resume` or
`/fork` removes the old entry first).

`leaf` is the session's current entry id (`null` before the first) and `tip` the id of its
newest entry, both as of the last rewrite. `/tree` moves the leaf without writing to the
transcript, so the transcript's newest entry can be on another branch. A reader that finds
the transcript's newest entry is still `tip` reads the branch ending at `leaf`; otherwise
Pi has appended since, every append moves its leaf to the new entry, and the newest entry's
branch is the one. An entry without `tip` (an older extension) says nothing about the tree.

`plannotator-tui last --pid <pid>` (the Rex, Herdr and herdr-core launchers pass it) reads
the transcript of the live entry with that pid, and takes the tree position from that same
entry: two Pis can resume one session file, each at its own place. When that session has
no reply saved yet, or is not saved at all (`pi --no-session`), `last` says so rather than
open another session's transcript. A transcript read without a pid takes the position from
the live entry written last that names it.

A Rex `last` review of a Pi records the live entry's session `id`, and the registry root it
was found in, for the review pane. Send checks the live entry for that pid still has that
id. If Pi moved to another session it refuses: `/new` and `/resume` obviously, and `/fork`
and `/clone` too, since Pi gives both a new session (reason `fork`, a new file whose header
names the old one) and whether it holds the reviewed reply depends on where it branched.
If no live entry answers (Pi reloading, the extension removed), it refuses and says to
retry or reinstall. Either refusal copies nothing and keeps the notes pending for `E`, as
for an agent at a dialog. A registry folder the pane cannot read at all means no check.

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
- `P` (`pick-target`) opens the agent picker: live Pi sessions and, inside Herdr
  (`HERDR_ENV=1`), every Herdr pane with an agent (`herdr agent list`, labelled with
  `herdr workspace list` and `herdr tab list`). A row shows the agent, its folder, its state,
  and where it runs (`pid N` for a Pi, `workspace › tab · pane` for a Herdr pane). The
  review's own pane (`HERDR_PANE_ID`) is never listed, and a Pi that Herdr also lists (same
  transcript or session id) is shown once, as the Pi session.
- Inside Rex (`REX_BLOCK` or `REX_SESSION` set), the picker also lists every Rex block with an agent, in
  every session. Rex has no agent list, so it is built from reads only: `rex ls --json`
  (sessions), `rex window ls -s <session> --json` (windows, their layers and blocks), and for
  each terminal block `rex do com.superlogical.terminal.process`, `…program_status` and, for
  an agent, `…title`, scoped with `-b <block> -s <session>`. Blocks are read in parallel. The
  agent is the program status app its foreground process reports (`claude-code` is
  `claude`), else the name of that process, as the Rex launchers find it. The state is the
  most urgent record that process owns (`blocked`, `error`, `working`, `idle`, `done`), or
  `unknown` when it owns none. A row shows the agent with the block's label when it is not
  the flavor's default (`shell`), else the terminal title the agent set (leading spinner
  removed, cut to 28 characters, left out when it only names the agent); the folder (the
  foreground process's cwd); the state; and `session › window · block` (labels, else the
  short session id or the window's position; the block's short id). An agent that owns no
  record shows `no state`: Send refuses it as a Rex launcher's target does (Enter could answer
  a dialog Rex cannot see). The review's own block (`REX_BLOCK`) is never listed. A Pi the Pi
  link lists with the same pid is shown once, as the Pi session, with the Rex block's place
  instead of `pid N`. A Rex pick sends as a Rex launcher's target does: it checks the agent's
  process still runs in the block and is not blocked, pastes with `block.event`, and presses
  Enter.
- Order: inside Rex, the agents in the window that holds `REX_BLOCK` first (a Pi there
  too). Then agents in the review's folder, then by name, then by pid, pane id or Rex's own
  order; the list is read once on open and does not move while the picker is open. Rows
  wider than the pane shorten the folder column from the left, then the names from the
  right, so the state and the place stay in view. The pick becomes the Send target until
  changed (a Herdr pick sends with `herdr agent prompt`); the picker also offers going back
  to the target the review started with. Document and code reviews share the picker.
- When nothing names a target (no launcher, `--deliver-to` or delivery command, so Send
  would copy), Send opens the same picker if an agent is within reach, with
  `Copy to clipboard` as the first row and the cursor on the first agent below it. Enter
  sends there and makes it the Send target; the copy row copies this once, and the next
  Send asks again; Esc sends nothing. With no agent within reach, Send copies as before.
  The Send button says `Send N new ▸ choose agent (E)` while Send would ask. The first look
  for agents runs in the background at start; Send looks again each time it is pressed.
  herdr-core agents are not listed: their targets are tied to the agent process a launcher
  named. A Rex launcher's target is a named target too, so Send goes there without asking
  and `P` switches.
- A note from Pi is added to the open document, or to the diff in a code review, with the
  author shown, and the footer says so. Notes from Pi are sent with the rest of the review like any other note.
- `plannotator-tui pi list` prints live Pi sessions. `plannotator-tui pi send --session <id>`
  sends stdin to one.
