# plannotator-tui for Pi

A Pi extension that links Pi sessions with plannotator-tui. The extension file is
`crates/plannotator-tui/src/pi/plannotator-tui.ts`. The plannotator-tui binary embeds it.

- **Feedback to Pi.** Each Pi session lists itself in a registry folder and listens on
  127.0.0.1 with a secret token. In plannotator-tui, press `P` to pick a Pi session. Send
  then delivers your review to that session as a user message. If Pi is busy, the message
  waits in Pi's queue until the current work is done.
- **Notes from Pi.** The model can leave a note in a plannotator-tui that has a file open,
  or on a changed line in a code review (`plannotator-tui review`). The note shows up next
  to the text, with the author "pi · <session name>".

The registry and wire format are in [`docs/spec-pi-link.md`](../../docs/spec-pi-link.md).
All traffic stays on this computer.

## Install

```sh
plannotator-tui pi install
```

This command copies the extension to `~/.pi/agent/extensions/plannotator-tui.ts`. If
`PI_CODING_AGENT_DIR` is set, it copies to that folder instead. You can also copy the file
there yourself. Then restart your Pi sessions, or run `/reload` in each one.

## In Pi

- Tool `plannotator_tui_note` with `{ file, quote?, line?, note }`. `file` is absolute or
  relative to the session's folder. `quote` must be copied exactly from the file.
  `line` is a 1-based line number. When something goes wrong, the tool returns the reason
  as a code with a hint, for example
  `quote_not_found: copy the quote exactly from the file's source text, or leave quote out`.
  Then the model can try again.
- Command `/plannotator-tui` lists the running plannotator-tui apps and their files.

## Test

```sh
node integrations/pi/test.mjs
```

The test runs the installed `pi` in RPC mode, with a fake model and a fake plannotator-tui.
All of its files go in a new folder under `/var/tmp`, so your real `~/.pi` and
`~/.plannotator` are not touched. It needs Pi 0.80.4 or newer for `agent_settled`.
`PI_LINK_EXTENSION=<file>` tests a different copy of the extension. `KEEP=1` keeps the
temp folder.
