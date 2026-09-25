<img src="docs/assets/banner.webp" alt="Plannotator TUI" width="720">

Annotate Markdown in the terminal. Select text, leave a 💬 comment, mark it 👍 looks good or
✗ delete, and hand the review to a coding agent as numbered feedback. One static binary,
no runtime. Rust + ratatui.

[![crates.io](https://img.shields.io/crates/v/plannotator-tui?style=flat-square)](https://crates.io/crates/plannotator-tui)
[![release](https://img.shields.io/github/v/release/plannotator/plannotator-tui?style=flat-square)](https://github.com/plannotator/plannotator-tui/releases)
[![ci](https://img.shields.io/github/actions/workflow/status/plannotator/plannotator-tui/post-merge.yml?branch=main&style=flat-square&label=main)](https://github.com/plannotator/plannotator-tui/actions/workflows/post-merge.yml)

```text
                                                                    Send 2 to claude in w1:p1 ▸
  Herdr plugins are shareable, executable workflow packages. A
  plugin can be a Bash script, JavaScript app, Lua script, Rust
▍ binary, or any other argv command your machine can run. Herdr     ╭ 💬  8d3e8 ────────────────╮
▍ owns the host surface: installation, manifest validation,         │ Say which parts the      │
  keybindings, terminal panes, events, invocation context, and      │ plugin can override.     │
  socket access. The plugin owns its implementation language,       ╰──────────────────────────╯
  dependencies, files, and durable state.
    👍  looks good (a)  💬  comment (c)  ✗ delete (d)
▍ Plugins exist so Herdr can stay lean. The core stays focused on   ╭ 👍  25300 ────────────────╮
▍ terminal workspaces, panes, agents, and a stable CLI/socket API.  │ looks good               │
▍ Plugins turn that existing extension surface into reusable        ╰──────────────────────────╯
▍ workflows that people can build, install, and share without
▍ adding every workflow to Herdr itself.

 plugins.md · 2 annotations · selected 36 chars a looks good · c comment · d delete · esc clear
```

Watch it inside Herdr: [demo](https://x.com/plannotator/status/2093419561077154287).

## Install

```sh
brew trust plannotator/tap && brew install plannotator/tap/plannotator-tui   # macOS, Linux
cargo install plannotator-tui                                                # anywhere with Rust
```

Homebrew 6 asks you to trust a third-party tap once before installing from it.

Prebuilt binaries for macOS, Linux and Windows are on the
[releases page](https://github.com/plannotator/plannotator-tui/releases).

## Use

```sh
plannotator-tui docs/plan.md      # one file
plannotator-tui docs               # a folder: file tree on the left, counts per file
plannotator-tui last               # your coding agent's recent replies, pick one, annotate it
```

Drag with the mouse (or `v` and move) to select, then `a` 👍 · `c` 💬 · `d` ✗. `E` copies the
feedback to the clipboard as numbered annotations (`# Annotations on plan.md`, `## Annotation 1
(line 12)`, …). Every annotation is saved as JSON the moment you make it; `q` closes.

Copies go to the clipboard as OSC 52, which is the terminal you are looking at, so on Herdr 0.9.0
they reach your own machine even when the app runs on a remote server; Herdr Annotate's global
`copy-context` and `copy-archive` actions do not, because they run outside a pane.

On an agent's reply, `S` sends the feedback and closes the window in one key, the same as
`q` then `y`. `E` still sends and leaves the window open. When the agent is at a dialog the
window stays open and the footer says why; press `S` again to retry. With nothing to send,
or with a review the agent already has, `S` closes like `q`. File and folder reviews have
no `S`; they finish with `F`.

For file and folder reviews, `E` sends only new or edited annotations. Send A and B, then
add C: the next send includes just C. Sent notes stay visible with a marker; editing one
makes it pending again, including after a restart. `R` **Resend all** includes every active
note. With nothing pending, `E` reports “nothing new to send”. A failed send keeps the notes
pending for retry.

`F` **Finish review** archives sent, unchanged notes and leaves pending ones in place.
`U` undoes the last finish during this session. `H` opens the archive, where Enter or a
click restores a note even after reopening the app. Restoring keeps its original id and
sent status. The archive is stored with the annotations and works even when feedback
history is turned off. The header holds the send button and a `Review ▾ (m)` button whose
menu lists these four actions with live counts (`R` resend all · 3 sent, `F` finish review
· archive 3 sent, `U` undo finish, `H` archive · 2 notes); rows with nothing to act on are
dimmed. The keys also work without opening the menu.

| Where | Keys |
|---|---|
| anywhere | `Tab` cycle tree · document · notes; `E` send; `t` tree; `r` reload; `q` quit |
| reply review | `S` send and close |
| document | `j`/`k` block; `c` comment on the block; `x` clear its annotations; `v` select with `hjkl` `w` `b` `0` `$`; `i` move the cursor with those keys first, then `v` to select from there |
| toolbar | `a` looks good · `c` comment · `d` delete · `Esc` |
| notes | `j`/`k`; `e` edit; `x` remove; click a bubble |
| file/folder review | `E` send new · `m` review menu (`R` resend all · `F` finish review · `U` undo · `H` archive) |
| tree | `j`/`k`; `Enter` open; `.` show/hide dot-prefixed entries (`.agents/`, `.github/`); `E` sends new notes across all reviewed files, including collapsed folders |

Hidden folders are out of the tree until `.` asks for them, and `.git`, `.hg`, `.svn`,
`.jj`, `.cache`, `.direnv`, `.venv`, `venv`, `__pycache__`, `node_modules`, `vendor`,
`target`, `build`, `dist` and `out` stay out either way — none of them is read at all.
Showing or hiding is a view choice: notes recorded for a file inside a hidden folder are
part of the review and are sent whether or not its folder is listed.

## Inside Herdr

Install [Herdr Annotate](https://github.com/plannotator/herdr-annotate); it bundles this binary,
opens it in a pane with `prefix+o` (folder) or `prefix+shift+o` (agent's last reply) or by
Ctrl-clicking a `file://…md` link, and the header button sends the review straight back to
the agent as its next message: `Send 3 new ▸ claude in w1:p2 (E)`. Folder reviews show
`Send 3 new across 2 files` and send one combined feedback message.

```toml
# ~/.config/plannotator-tui/config.toml
[herdr]
placement = "overlay"   # overlay (full tab, default) | split | popup
```

`plannotator-tui herdr last --newest` opens the agent's newest reply without asking which
one; without the flag the picker comes first, as it always has.

`plannotator-tui herdr terminal [--lines N]` opens the focused pane's recent output (the last
200 lines by default; Herdr caps a read at 1000) as a transient review, shown verbatim in one
code block. Send goes to the agent in that pane, as it does for a reply; a pane with no agent
copies instead. `--print` writes the document to stdout instead of opening a pane. Herdr
Annotate ships it as the `annotate.terminal` action with no default key.

`plannotator-tui config` prints the file's path and the values in effect. The `herdr/`
directory in this repo is the development manifest; users should install Herdr Annotate.

The same file chooses the theme:

```toml
[ui]
theme = "auto"   # auto (ask the terminal, default) | light | dark
```

On `auto` the viewer asks the terminal for its background colour once at startup and uses a
light palette when it finds one; a terminal that does not answer keeps the dark palette it
has always used. The question costs one round trip before the screen is drawn, and a key
pressed into that window is read along with the reply and lost, so set the theme outright if
you habitually type ahead. `light` and `dark` skip the
question, and `PLANNOTATOR_TUI_THEME=light|dark` does the same for one run — the variable
wins over the file, and `plannotator-tui config` prints whichever is in effect. Only the
backgrounds plannotator-tui paints itself change; the document keeps your terminal's own
colours either way.

Actions forwarded by Herdr Mirror default to a split beside the invoking remote
pane. Mirror does not preserve overlay presentation, and Herdr 0.8.2 opens an
overlay in its server's active tab, which can differ from the tab you are viewing.
An explicit `--placement` or `PLANNOTATOR_TUI_PLACEMENT` still takes precedence.

## Agent replies

`plannotator-tui last` finds the transcript of the agent that launched your shell and shows a
picker of its recent replies. Hosts: Claude Code, Codex, pi, Oh My Pi, GitHub Copilot CLI,
Droid, Hermes CLI, OpenCode (1 and 2). `--host`, `--pid`, `--session <transcript>` (format sniffed when
no host is named) and `--session-id <id>` (Hermes, OpenCode) override detection; `--stdin`
reads a document; `--newest` skips the picker and opens the newest reply straight away, with
`p` still opening the picker on the rest;
`--print` writes the newest reply to stdout and always exits 0 (for hooks and scripts).
A reply review keeps its annotations in memory only; nothing about it survives the run, but
the feedback you send or copy is archived like any other (see Feedback archive below).

On Linux, an explicit Codex `--pid` selects the rollout opened by that process. If it
cannot be identified uniquely, `last` reports the failure instead of choosing an unrelated
session. `--session` and `--session-id` keep precedence over PID discovery.

Inside Herdr, the exact-session path needs the session id Herdr reports for the pane. Herdr's
Claude Code integration registers on `SessionStart`, so a session reports its id only when it
started after `herdr integration install claude`; a session that was already running when the
integration was installed reports none. Without an id, `last` shows the newest transcript for
the folder, which is a guess when several sessions share one directory, and says so in the
status line.

## Where annotations live

```
~/.plannotator/clients/plannotator-tui/annotations/<project>/<slug>/annotations.json
```

`<project>` is the git repo name, `<slug>` the file's basename plus 8 hex of the sha256 of
its path: Plannotator's own layout, so both tools see one record per file. The JSON is the
Plannotator Workspaces wire shape; any agent can read it. Nothing is written next to your
files. `PLANNOTATOR_DATA_DIR` relocates the directory.

### Feedback archive

A successful Send or Copy also appends what was submitted (the feedback text, the quoted
selections and their annotations, and the file, folder or agent session it was about) to
`{data_dir}/feedback/<project>/index.jsonl`, with a Markdown copy under `records/`. The data
dir is `PLANNOTATOR_DATA_DIR`, else an existing `~/.plannotator`, else
`$XDG_DATA_HOME/plannotator`, else `~/.plannotator`. File, folder and reply reviews are all
archived; a send that fails or is refused is not. The format is the one the Plannotator
browser app writes, so both tools share one history. To turn it off, set
`PLANNOTATOR_FEEDBACK_HISTORY=0` (once the variable is set, only `1` or `true` enable) or put
`"feedbackHistory": false` in `{data_dir}/config.json`; the variable wins over the file.

## Headless

```sh
plannotator-tui --export <file|folder>                          # all active notes, to stdout (no delivery recorded)
plannotator-tui --annotate <file> <quote> <text> [comment|looks_good|delete] [--occurrence N]
plannotator-tui --snapshot <file|folder> [cols rows scroll] [quote]   # one frame as text
plannotator-tui --bench <file>                                  # parse / layout timings
plannotator-tui herdr terminal [--lines N] --print              # a Herdr pane's recent output as a document
```

## Repository

- `crates/plannotator-tui`: the app. `crates/plannotator-tui-schema`: annotation and anchor
  types, wire-compatible with Plannotator Workspaces. `crates/plannotator-tui-hosts`: agent
  transcript readers.
- `docs/decisions.md` is the design record; `AGENTS.md` the engineering rules;
  `crates/plannotator-tui/README.md` the full key reference and measurements.

MIT.
