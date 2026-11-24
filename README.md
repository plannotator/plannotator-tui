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

On x86_64 Linux, choose `plannotator-tui-x86_64-unknown-linux-musl` if the GNU
binary reports a missing `GLIBC_*` version, or if you use Alpine. This static
musl build does not require system glibc or a local Rust toolchain. Existing
GNU/Linux binaries remain available. Verify your download against `SHA256SUMS`
from the same release, make it executable, and put it on your `PATH` as
`plannotator-tui`.

## Use

```sh
plannotator-tui docs/plan.md      # one file
plannotator-tui docs               # a folder: file tree on the left, counts per file
plannotator-tui last               # your coding agent's recent replies, pick one, annotate it
```

Drag with the mouse (or `v`, move, then `Enter`) to select, then `a` 👍 · `c` 💬 · `d` ✗. `E`
copies the feedback to the clipboard as numbered annotations (`# Annotations on plan.md`,
`## Annotation 1 (line 12)`, …), or, when an agent is within reach, first asks which agent
to send to (see [Choosing the agent](#choosing-the-agent)). Every annotation is saved as JSON the moment you make it; `q`
closes.

`I` attaches a local image to the selected note (in the notes, or the selected block's note in
the document): type, paste or drop its path, then `Enter`. Quoted, `~` and `file://` paths
work; a relative path starts at the document's folder. PNG, JPEG, GIF, WebP, BMP and SVG files
are accepted. The feedback lists each one as `Image: /absolute/path.png` under its note, so the
agent opens it from your disk; nothing is copied or uploaded. `D` removes the last image added.
Adding or removing an image makes a sent note pending again.

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
note. With nothing pending, `E` reports “nothing new to send” and the button reads
`Sent ✓ ▸ claude in w1:p2` (`Copied ✓` for the clipboard) until there is a new note. When
the agent is at a dialog it reads `claude in w1:p2 at a dialog · retry (E)`. A failed send
keeps the notes pending for retry.

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
| anywhere | `Tab` cycle tree · document · notes; `z` hide or show the notes; `E` send; `P` choose the agent to send to; `t` tree; `r` reload; `o` open the file in `$EDITOR`; `q` quit |
| reply review | `S` send and close |
| gated review | `A` approve; `E` send and finish; `q` dismiss |
| document | `j`/`k` block; `c` comment on the block; `a` looks good; `d` delete; `x` clear its annotations; `X` clear every annotation on the document, after a `y`; `v` select with `hjkl` `w` `b` `e` `0` `$`, then `a`, `c` or `d` to act on it, `y` to copy it, or `Enter` for the toolbar; `V` select whole rows; `i` move the cursor with those keys first, then `v` to select from there; `I`/`D` attach or remove an image on the block's note; `/` search the text, then `n`/`N` next/previous match |
| toolbar | `a` looks good · `c` comment · `d` delete · `y` copy · `Esc` |
| comment box | `Enter` save; `Shift+Enter`, `Alt+Enter` or `Ctrl+J` new line; `↑`/`↓` move by display row; `Ctrl+←`/`Ctrl+→`, `Alt+←`/`Alt+→` or `Alt+B`/`Alt+F` jump by word; `Ctrl+A`/`Ctrl+E` line start/end; `Ctrl+W`, `Alt+Backspace` or `Ctrl+Backspace` delete the previous word; `Alt+D` or `Ctrl+Delete` delete the next word; `Ctrl+U`/`Ctrl+K` delete to line start/end; `Esc` cancel, asking first if the text changed |
| notes | `j`/`k`; `e` edit; `x` remove; `I` attach an image, `D` remove the last; click a bubble |
| file/folder review | `E` send new · `m` review menu (`R` resend all · `F` finish review · `U` undo · `H` archive) |
| folder review | `Ctrl+P` open a file by name, including in collapsed folders: type, `↑`/`↓` or click, `Enter`, `Esc` |
| tree | `j`/`k`; `Enter` open; `.` show/hide dot-prefixed entries (`.agents/`, `.github/`); `E` sends new notes across all reviewed files, including collapsed folders |
| code review | drag with the mouse or `v` select, `v` again to end; `c` comment; `y` copy the selected lines; `s` suggest code; `C` note on the file; `M` note on the whole change; `o`/`O` whole file now/before; `d` go to definition, `ctrl+o`/`ctrl+i` back/forward; `]`/`[` hunk; `n`/`N` file; `x` viewed; `ctrl+n` next not viewed; `h`/`l` old/new side; `u` split/unified; `w` wrap; `(`/`)` move the line between the sides (or drag it); `,` diff settings; `f` the file list; `/` filter files by name; `t` filter by kind; `H` hide viewed files; `<`/`>` list width; `{`/`}` notes width (or drag the line between the panes); `z` hide or show the notes; `r` read the diff again (the footer says when it changed on disk); `?` keys |

These are the defaults; every key can be rebound under `[keys]` (see [Keys](#keys)).

The notes rail on the right appears with the first note: a document or diff without notes
gets every column. `z` closes it, and it stays closed until `z` or `Tab` opens it again: a
new note, a wider window, another file or a reload leave it closed, and the header and
footer still count the notes, whose marks stay in the text. The next session starts the way
you left it. `notes_rail` in the config file's `[ui]` table changes how it starts; it is
described with the other `[ui]` settings.

### Approval gate

When a script or an agent must know whether you signed off, add `--gate`. The review gains
an **Approve** action (`A`, and an `Approve (A)` button on the header's edge) beside Send and
quit, and ends with one decision: Approve is `approved`, a successful Send is `annotated`, and
quitting is `dismissed`. Notes you have not sent yet ride along with an approval
(`Approve with 2 notes (A)`). Without `--gate` nothing changes.

```sh
plannotator-tui docs/plan.md --gate                    # prints the outcome when you finish
plannotator-tui docs/plan.md --gate --json --require-approval --result-file /tmp/plan-decision.json
```

The output is the one `plannotator annotate --gate` prints. Plain `--gate` prints `The user
approved.`, the approved-with-notes message (the notes plus the file they are about), the
feedback, or nothing for a dismissal. `--json` prints one line instead,
`{"decision":"approved|annotated|dismissed","feedback":"…"}`; `feedback` is absent from a
bare approval and a dismissal, and the line also carries `annotationCount` and the reviewed
`target`. `--json` without `--gate` decides between `annotated` and `dismissed` only.

For a gate a script can trust, add either or both strict flags (they need `--gate --json`):

- `--require-approval` exits 0 only on approval and 1 on `annotated` or `dismissed`.
- `--result-file <path>` also writes `{"decision":…,"feedback":…}` to `<path>`, atomically,
  and never over an existing file: the folder must exist and the file must not. A relative
  path starts at the current directory.

Under a strict flag a failure of the gate itself (a bad flag, a missing file, a result file
that cannot be written) exits 2, never 1. The review draws on stdout, so a gated review
refuses to start when stdout is not a terminal: read the decision from `--result-file`
rather than capturing stdout. Each decision is also added to the feedback archive as
`approved`, `approved-with-notes`, `feedback` or `dismissed`.

`o` hands the terminal to `$VISUAL` (or `$EDITOR`, which may carry flags such as `code -w`)
on the open file, at the current line for vim, nvim, nano, helix, emacs, micro and kak. When
the editor exits the viewer comes back and, if the file changed, reloads it as `r` does so
notes re-anchor. A GUI editor without its wait flag returns at once; save later and the
footer says the file changed. Reply and terminal reviews have no file, so `o` does nothing
there.

Hidden folders are out of the tree until `.` asks for them, and `.git`, `.hg`, `.svn`,
`.jj`, `.cache`, `.direnv`, `.venv`, `venv`, `__pycache__`, `node_modules`, `vendor`,
`target`, `build`, `dist` and `out` stay out either way — none of them is read at all.
Showing or hiding is a view choice: notes recorded for a file inside a hidden folder are
part of the review and are sent whether or not its folder is listed.

`Ctrl+P` matches file names, not contents, ignoring case and allowing gaps (`prd` finds
`product-requirements.md`). Results show the path within the folder, so files with the same
name are told apart, and opening one expands the tree down to it. It follows the tree's
hidden-entries setting and skips the same folders the tree does.

### Search

`/` opens a search line at the bottom of the document. Every match is highlighted as you
type, and the cursor moves to the first one at or after it; the footer counts them (`3/17`).
`Enter` keeps the search and closes the line, `Esc` closes it and puts the view back. `n` and
`N` then go to the next and previous match, wrapping at either end, and `Esc` clears the
highlights. An empty search clears them too.

A search is plain text, not a pattern, and ignores case unless it has a capital letter. It
matches the text as shown: Markdown markup is invisible to it, and a match can run across a
wrapped line, inside a table cell or a list item. It does not cross a hard line break or run
from one list item or table row into the next. The cursor lands on the match, so `v` selects
from it. A search moves only the cursor and the view: a waiting selection and every note stay
as they were.

In a folder review a search covers the open file only; opening another file drops it.

## Code review

```sh
plannotator-tui review                 # uncommitted changes in this repository
plannotator-tui review --staged        # only what is staged
plannotator-tui review --base main     # everything since this branch left main
plannotator-tui review --commit 1a2b3c # one commit
plannotator-tui review ../other-repo   # another repository
plannotator-tui review 123             # pull request #123 of this repository (also #123)
plannotator-tui review https://github.com/owner/repo/pull/123
```

`review` opens a git diff. Move with `j`/`k`, `]`/`[` for the next or previous hunk, and
`n`/`N` for the next or previous file. `h` and `l` go to the old and new side of a line.
`v` starts a selection and a second `v` ends it. Then:

- `c` writes a comment on the lines.
- `s` writes a suggestion: the box opens with the new lines, so you edit the code you want.
- `C` writes a note on the whole file.
- `M` writes a note on the whole change (also the first row of the Review menu). It sits
  at the top of the rail under **Whole change**, and the agent gets it first, under
  `## Global comments`, as the browser sends its global comments.

The mouse works too. The wheel scrolls the diff, the file list or the notes under the
pointer. A click puts the cursor on a line, on the old or new side you clicked; a click on a
file in the file list goes to that file, a click on a folder opens or closes it, and a
click on a note selects it and shows its lines. The Send and `Review ▾` buttons in the header are clickable. Drag over lines to
select them: a selection is whole lines on the side where the drag started, as in the
browser's code review, and the lines are shaded while you drag. Drag past the top or bottom
of the diff to scroll. When you let go, a small toolbar next to the lines offers `💬 comment`
and `✎ suggest` (click it, or press `c` or `s`), and the note is anchored on exactly those
lines, with their code quoted for the agent. `Esc` clears the selection. Drag works the
same in the whole-file view. Shift-click grows a selection to the clicked line, from the
line the drag started on. A selection stays inside the hunk it started in, as GitHub's
multi-line comments do: a drag or a `v` selection stops at the hunk's first or last line.
A key, a second click, or the window losing focus before you let go drops the drag, and
its shading with it.

Drag the line between the file list and the diff, or between the diff and the notes, to
make the list or the notes wider or narrower; the line lights up under the pointer and while
you drag it. `<`/`>` do the same for the list and `{`/`}` for the notes, four columns at a
time. The width you set stays for the rest of the review, through filters and `r`, and the
diff always keeps room to show its code; a window made narrower for a while draws the panes
narrower and gives them the width you set back when it widens. Double-click the line (two
clicks on one cell, without moving) to let the list or the notes fit themselves again.

Notes appear in the rail on the right, grouped by file, and a mark in the diff shows their
lines. Sent notes get a dimmer mark. `Tab` moves to the rail; `j`/`k` there moves to a
note and the diff moves with it; `e` edits and `x` removes it. The rail appears with the
first note; `z` closes it and gives the diff its columns, and `z` or `Tab` opens it again.
Closed, it stays closed, as in a document review.

`E` sends the new notes, `S` sends them and closes, and `P` picks the agent to send
to. Sends go to the same place a document review's do (Herdr pane, Rex, Pi or the
clipboard, or the agent you choose when nothing named one; see [Choosing the agent](#choosing-the-agent)), and are kept in the feedback archive. `q` asks first when notes are unsent.
`m` opens the Review menu: `M` comment on the whole change, `R` resend all, `F` finish
review, `U` undo.

`o` shows the whole file under the cursor as it is now, scrolled to the same line; `O`
shows it as it was before the change (the base), at the matching old line. The header says
`Whole file · new` or `Whole file · base (1a2b3c4)`, a bar in the gutter marks the lines the
change added (or removed), and `]`/`[` move between them. `n`/`N` go to the next or previous
file, and `o`, `O` or `Esc` go back to the diff where you left it. `c` in the current file
writes an ordinary diff note when the diff shows those lines; on lines outside every hunk it
writes a note **not in the diff**, which the rail lists under its own heading and the agent
gets with the code quoted, as the browser sends one. `Enter` on such a note in the rail
opens the whole file at its lines. The base is read-only for notes. Files over 5 MiB, binary
files, an added file's base and a deleted file's current version are not shown; the view
says which.

`d` goes to the definition of a word on the cursor's line, in the diff or the whole file.
With several identifiers on the line, a small list asks which (`1`-`9` or `Enter`). The
search (`git grep`, the same patterns and ranking as the browser's code navigation) runs in
the background on the side you are on: the new side searches the reviewed version, the old
side and the base view search the base; in a pull request it reads the PR's commits, also
from the cache repository. One clear answer is jumped to: in the diff when it shows that
line, otherwise in the whole file. Several are listed with `path:line`, definition or use,
and the line; `Enter` goes, `Esc` closes. `ctrl+o` goes back to where you jumped from and
`ctrl+i` forward again (in terminals that tell `ctrl+i` from Tab).

`r` reads the diff again after you or the agent change files. Notes follow their lines.
The review checks for changes by itself every few seconds (a `git status` and the times of
the changed files; never while you type, less often in a large repository), and the footer
then says `the diff changed on disk · r refreshes`. It never reloads by itself. Commit and
pull request reviews have nothing on disk to check.
A note whose lines changed goes to the **Outdated** group at the end of the rail; it is
still sent, marked as outdated. `u` switches between split and unified view, `w` wraps
long lines and `?` shows every key.

The header shows the layout as a switch, `Split  Unified` with the one in force lit, and a
`Wrap` switch beside it; a click on either does what `u` or `w` does. In a split diff, drag
the line between the old and the new side to give one side more of the width, or press `(`
and `)`; a double click on the line makes the sides even again. Each side keeps at least 20
columns. `,` (or the header's `Settings` button) opens the diff settings: layout, split
ratio, wrap, line numbers, colours and background, each with its value. `↑`/`↓` pick one,
`←`/`→` change it, `enter` or `space` moves it on (and evens the split ratio), a click on a row
does the same, and `esc` closes the menu. Every change applies at once and is remembered for
the next review, however you made it (the menu, the switches, a key or a drag); the menu's
last row, **Reset to defaults**, forgets those choices and goes back to
[`[review]`](#how-diffs-open) in the config file, else the defaults. A narrow header gives up
the key hints first, then the `Wrap` switch, the `Settings` button and the `Review` button,
and keeps the layout switch longest (as `S  U` when it must).

The changed files are listed left of the diff as a folder tree. A chain of folders with
one folder each is one row (`crates/app/src/`), and the list is as wide as its names
need, up to two fifths of the screen. A name that does not fit is cut in the middle, so
its end and extension stay readable; the diff heading shows the whole path. `<` and `>`
make the list narrower or wider. Where there is no room for it (80 columns beside the
notes) it hides, and `f` opens it, with the notes making way. In the list, `j`/`k` move
and show each file, `enter` goes to it, `h`/`l` close and open folders, and `f` closes
the list.

`/` filters the files by name: every word you type must be in the path (`review test`),
and the list follows each key. `enter` hands the filter to the diff, which then shows
only those files, so `n`/`N`, `]`/`[` and `ctrl+n` move among them. `esc` clears it.
`t` opens the kinds: code, tests, docs, config, generated and other, each with how many
files of it the change has. Space turns a kind on or off, `enter` shows that kind alone.
`H` hides the files you have marked viewed. The list's heading shows `Files · 12 of 340`
and the filter while one is on. Notes are never hidden: the rail keeps them, Send sends
them all, and the list says how many unsent notes are in hidden files.

The kinds follow the browser's rules for tests and generated files: test folders
(`tests/`, `__tests__/`, `spec/`, `fixtures/`, `src/androidTest/`, `Acme.Tests/`), test
names (`*.test.ts`, `*.spec.js`, `*_test.go`, `test_*.py`, `*.snap`, `*.stories.tsx`), and
generated files (lockfiles, `*.min.js`, `*.map`). Here `*.pb.go`, `*.g.dart`, `vendor/`,
`node_modules/` and `dist/` are generated too. Docs are prose (`*.md`, `*.rst`, `*.txt`,
`README`, `LICENSE`, `docs/`); config is data and build files (`*.json`, `*.yaml`,
`*.toml`, dotfiles, `Dockerfile`, `Makefile`, `*.config.ts`); code is source by
extension; the rest is other.

`x` marks the file under the cursor viewed, and `x` again unmarks it. The header counts
them (`3/12 viewed`), the file list ticks viewed files and dims them, and `ctrl+n` goes to
the next file you have not viewed. Marks are
saved in the browser app's own format under the data folder (`review-progress/`), so a
review started in the browser continues here and the other way round. A mark stops
counting once its file changes. Set `PLANNOTATOR_REVIEW_PROGRESS=0` to keep marks for the
session only. See [docs/spec-review-progress.md](docs/spec-review-progress.md).

### Pull requests

`review` with a pull request link, a number or `#123` (also `--pr PR`) reviews what GitHub
shows for that pull request: its head commit against where it left its base branch. It uses
your `gh` login (install the [GitHub CLI](https://cli.github.com) and run `gh auth login`). A
number is looked up in the GitHub repository of the folder you run it in; a link works from
any folder. A folder whose name is only digits is `./123`.

The pull request's commits are fetched into your clone when you run it inside one (any remote
naming the PR's repository), as the private ref `refs/plannotator-tui/pull/<n>/head`. Your
branch, index and working files are not touched. Outside a clone of that repository, they are
fetched into a cache repository under the data folder
(`clients/plannotator-tui/pulls/<host>/<owner>/<repo>.git`); the first review of a repository
there downloads its history once.

The header shows `PR #123 · title · base ← head`. `r` looks the pull request up again and
fetches new pushes; notes follow their lines as they do for local changes. Notes are saved
once per pull request, whichever clone you open it from. Send goes to the agent as for any
review, with the browser's pull request header (`# PR Review: owner/repo#123`, the title,
the branches and the link). Nothing is posted to GitHub.

A Pi session can leave notes on the diff too (see [Pi](#pi)). Notes are saved with the
other annotations, one record per repository and scope. Code review is not available on
Windows yet.

## Inside Herdr

Install [Herdr Annotate](https://github.com/plannotator/herdr-annotate); it bundles this binary,
opens it in a pane with `prefix+o` (folder) or `prefix+shift+o` (agent's last reply), and the
header button sends the review straight back to the agent as its next message:
`Send 3 new ▸ claude in w1:p2 (E)`. Ctrl-clicking a `file://…md` link opens it in Annotate only
if you install the add-on: `herdr plugin install plannotator/herdr-annotate/links`. Folder reviews show
`Send 3 new across 2 files` and send one combined feedback message.

```toml
# ~/.config/plannotator-tui/config.toml
[herdr]
placement = "overlay"   # overlay (full tab, default) | split | popup
```

`plannotator-tui herdr open plan.md --gate [--result-file <path>]` opens a gated review. Approve
sends the agent `The user approved.` (or the approved-with-notes message), Send sends the
numbered feedback as usual, and quitting sends nothing; the window closes on any of the three.
With `--result-file` the pane also writes the decision JSON there. The command returns as soon
as the pane opens, so a script waits for the file.

`plannotator-tui herdr last --newest` opens the agent's newest reply without asking which
one; without the flag the picker comes first, as it always has.

`plannotator-tui herdr terminal [--lines N]` opens the focused pane's recent output (the last
200 lines by default; Herdr caps a read at 1000) as a transient review, shown verbatim in one
code block. Send goes to the agent in that pane, as it does for a reply; a pane with no agent
copies instead. `--print` writes the document to stdout instead of opening a pane. Herdr
Annotate ships it as the `annotate.terminal` action with no default key.

`plannotator-tui herdr review [--staged | --base REF | --commit SHA | PR]` opens a [code
review](#code-review) of the focused pane's git repository (else the workspace's folder) in a
pane. Send goes to the agent in that pane. A folder that is not in a repository, or has no
changes to review, fails before any pane opens. The development manifest has it as the
`review` action with no default key, on macOS and Linux.

A Claude Code session is looked up in `CLAUDE_CONFIG_DIR`, else `~/.claude`. An agent's own
`CLAUDE_CONFIG_DIR` doesn't reach a pane Herdr opens, so if you run several profiles, list
them to have them searched for a session Herdr identifies by id:

```toml
[last]
claude_profile_dirs = ["~/.claude-max", "~/.claude-work"]
```

Finding a session by pid or folder still uses only the first two.

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

`[colors]` replaces individual roles of whichever palette is in force; a role left out keeps
its light or dark value. A colour is a name (`yellow`, `light-cyan`), a 256-colour index
(`238`, quoted or not) or `#rrggbb`. An unknown role or colour is an error that names it, and
`plannotator-tui config` prints the roles you set. Every role, with the dark palette's values
(apart from `selection`, which is reverse video by default):

```toml
[colors]
accent = "cyan"          # focused borders and gutter, the open file, menu and picker frames
heading = "cyan"         # every heading level
comment = "yellow"       # comment titles, the compose box, annotated rows in the gutter
looks-good = "green"
delete = "red"           # also the struck-through text
comment-bg = "58"        # the tint under commented text
looks-good-bg = "22"     # the tint under looks-good text
block = "236"            # the focused block and the focused tree row
toolbar = "238"          # the floating toolbar and the idle header buttons; 253 on light
cursor = "240"           # the keyboard cursor cell
selection = "#45475a"    # a character selection, a picker row, a menu row
code = "light-yellow"    # inline code, and fenced code without a bundled grammar; 94 on light
code-keyword = "magenta" # fenced code with a known language: keywords
code-string = "green"    # 28 on the light palette
code-number = "yellow"   # numbers and constants; 130 on light
code-function = "blue"
code-type = "cyan"       # 30 on light
code-comment = "dark-gray"  # comments (italic), the bar beside code and its language label
search = "24"            # every match of a document search
search-current = "208"   # the match the cursor is on (black text by default); 214 on light
```

`cursor`, `selection`, `search` and `search-current` become plain backgrounds when set: the
text on them keeps the terminal's colour. The text on the header buttons and the floating
toolbar is not a role: each theme picks it from the 256-colour table so that it reads at 4.5:1
or more on the default toolbar grey. A `toolbar` colour far from that grey can make it harder
to read.

The document's text sits in a reading column, centred in the columns the tree and the notes
leave, and `[ui]` sizes it and the app around it:

```toml
[ui]
doc_width = 80           # the reading column, in columns; 0 lets the text fill the pane
max_width = 160          # center the whole app in this many columns
horizontal_margin = 2    # blank columns on each side
```

`doc_width` defaults to `80`; `max_width` and `horizontal_margin` default to `0`, which
fills the terminal. `<` and `>` narrow and widen the column by eight for the session. The
margin and `max_width` yield before the app drops below 80 columns, so a narrow pane keeps
its notes rail; a narrow pane also lets the document shrink below `doc_width`.

Fenced code is drawn without its fences, beside a bar, and coloured by language when the
grammar is one of the bundled ones; other code keeps one colour. This holds inside a list
or a quote too. Top-level code also shows its language at the top right when there is
room, and a line too long for the column continues on the next row.

The comment box is 48 columns wide. `comment_width` changes that, down to a minimum of 38,
and a narrower pane still narrows it:

```toml
[ui]
comment_width = 72
```

The notes rail appears with a review's first note and stays away while there are none.
`notes_rail` changes how a session starts:

```toml
[ui]
notes_rail = "when-notes"   # default; or "shown" (always, notes or not) or "hidden" (closed)
```

`z` (`toggle-notes`) closes or opens the rail in the document and the code review alike,
and the last choice is remembered in `clients/plannotator-tui/ui-state.json` under the data
folder. A remembered choice wins over `notes_rail`; with `when-notes`, a rail remembered
open still waits for the first note before it takes any columns.

A mouse selection is copied to the clipboard when you release the button, in a document and
in the code review (the selected lines' code). To leave the clipboard alone, as with Herdr's
`ui.copy_on_select = false`, turn it off and press `y` on the selection when you want a copy:

```toml
[ui]
copy_on_select = false
```

### How diffs open

`[review]` sets how the code review draws its diff by default. Every key is optional, and a
file without the table opens diffs as before:

```toml
[review]
layout = "auto"         # auto (split from 120 columns, unified below; default) | split | unified
split_ratio = 0.5       # the old side's share of a split row, 0.1 to 0.9; the new side gets the rest
wrap = false            # wrap long lines instead of scrolling them sideways
line_numbers = true     # line numbers in the gutter; off keeps the +/- signs
theme = "auto"          # the diff's colours: auto (follow [ui] theme, default) | dark | light
background = "terminal" # terminal (the terminal's own background, default) | theme (the diff theme's)
```

The app never writes this file. What you choose in the review (the settings menu, the
header's switches, `u`, `w`, `(`/`)`, dragging the line between the sides) is remembered in
`clients/plannotator-tui/review-settings.toml` under the data folder, and wins over
`[review]` the next time; a key you never changed there keeps coming from `[review]`, then
the defaults. **Reset to defaults**, the menu's last row, forgets the remembered choices.
Each review writes only the keys it changed, so two reviews open at once keep each other's
choices; a drag is remembered when you let go, to two decimals. If the remembered file
cannot be read or written, the footer says so once and the review carries on with the
settings it has.

Each side of a split row keeps at least 20 columns, so on a narrow screen the line may sit
nearer the middle than the ratio says. `(` and `)` move it in steps of 0.05 from the nearest
step. The whole-file view is always unified. `theme` changes the diff only; the rest of the
app keeps `[ui] theme`.

### Keys

Every key is rebindable under `[keys]`: an action, and the keys that run it.

```toml
[keys]
left = ["j", "Left"]   # jkl; instead of hjkl
down = ["k", "Down"]
up = ["l", "Up"]
right = [";", "Right"]
line-start = ["^", "Home"]
```

An entry replaces that action's keys and leaves every other action at its default; `[]`
unbinds it. A key is one character (`G` is Shift+g), `ctrl+<character>`, or one of `Up`,
`Down`, `Left`, `Right`, `Home`, `End`, `PageUp`, `PageDown`, `Enter`, `Esc`, `Tab`,
`Delete`, `Backspace`. An action keeps its meaning everywhere it applies, so `down` moves
down in the document, the tree, the notes and every picker. A key bound to two actions that
are live at the same time is an error that names both, and the hints at the bottom of the
screen show your keys. `plannotator-tui config` prints every action with its keys. An action
added in a later release leaves its default key to whatever your file already bound it to
(a file that put `toggle-layout` on `(` keeps it there, and `split-left` gets no key).

| Action | Default | Does |
|---|---|---|
| `up` `down` `left` `right` | `k` `j` `h` `l`, arrows | move; `left`/`right` in the document start the cursor, `right` in the tree opens |
| `word-forward` `word-back` `word-end` | `w` `b` `e` | cursor to the next or previous word's start, or the next word's end |
| `line-start` `line-end` | `0` `$`, `Home` `End` | cursor to the start or end of the line |
| `page-down` `page-up` | `ctrl+d` `ctrl+u`, `PageDown` `PageUp` | half a page, of the document or the message picker |
| `top` `bottom` | `g` `G`, `Home` `End` | first or last block, or message in the picker |
| `confirm` `back` | `Enter` `Esc` | finish, open, run; cancel, close |
| `quit` `interrupt` | `q` `ctrl+c` | quit |
| `cycle-focus` | `Tab` | tree · document · notes |
| `send` `send-and-close` | `E` `S` | send; send and close a reply review |
| `approve` | `A` | approve a gated review (`--gate`) |
| `toggle-tree` `reload` `pick-message` | `t` `r` `p` | show/hide the tree; reload; choose another reply |
| `open-in-editor` | `o` | edit the open file in `$VISUAL` or `$EDITOR`, then reload it |
| `quick-open` | `ctrl+p` | find a file by name in a folder review |
| `review-menu` | `m` | the Review menu |
| `pick-target` | `P` | choose the agent Send goes to (see [Choosing the agent](#choosing-the-agent)) |
| `resend-all` `finish-review` `undo-finish` `archive` | `R` `F` `U` `H` | file and folder review actions |
| `toggle-hidden` | `.` | show/hide dot-prefixed entries in the tree |
| `edit` `remove` | `e` `x`, `Delete` | the selected note |
| `attach-image` `remove-image` | `I` `D` | an image on the selected note, or the selected block's |
| `visual` `visual-line` `move-cursor` | `v` `V` `i` | select with the cursor; select whole rows; move it first |
| `comment` `clear-block` | `c` `x` | on the block; clear its annotations |
| `clear-all` | `X` | every annotation on the document, after `yes` |
| `looks-good` `delete` | `a` `d` | toolbar, with `comment`; also on the selected block, unless another document action has the key |
| `copy` | `y` | copy the selection waiting in the toolbar |
| `copy-lines` | `y` | code review: copy the selected lines; a config that sets only `copy` gives its keys to this too |
| `search` `next-match` `previous-match` | `/` `n` `N` | search the open document; step through the matches |
| `yes` `no` | `y` `n`, `Y` `N` | the send-before-quitting and clear-all questions |
| `suggest` `file-comment` | `s` `C` | code review: suggest code for the lines; a note on the whole file |
| `change-comment` | `M` | code review: a note on the whole change |
| `next-hunk` `previous-hunk` `next-file` `previous-file` | `]` `[` `n` `N` | code review: move by hunk or file |
| `toggle-layout` `toggle-wrap` `toggle-files` | `u` `w` `f` | code review: split or unified view; wrap; open the file list and move to it, or close it from there |
| `diff-settings` | `,` | code review: the diff settings menu, remembered for the next review (see [`[review]`](#how-diffs-open)) |
| `split-left` `split-right` | `(` `)` | code review: move the line between a split diff's sides by a twentieth of the width |
| `filter-files` `filter-kinds` `hide-viewed` | `/` `t` `H` | code review: filter the file list and the diff by name; by kind; hide viewed files |
| `toggle-viewed` `next-unviewed` `previous-unviewed` | `x` `ctrl+n`, none | code review: mark the file viewed; the next or previous file not viewed |
| `whole-file` `whole-base` | `o` `O` | code review: the whole file as it is now, or before the change; again to go back |
| `go-to-definition` `jump-back` `jump-forward` | `d` `ctrl+o` `ctrl+i` | code review: search for the identifier on the line and go to its definition; back and forward through the places jumped from (`ctrl+i` needs a terminal that tells it from Tab) |
| `help` | `?` | code review: every key (`j`/`k` scroll it when it does not fit) |
| `narrow-column` `widen-column` | `<` `>` | the reading column, by eight columns |
| `narrow-files` `widen-files` | `<` `>` | code review: the file list, by four columns |
| `narrow-notes` `widen-notes` | `}` `{` | code review: the notes panel, by four columns |
| `toggle-notes` | `z` | close the notes rail, or open it again; remembered for the next session |

Actions forwarded by Herdr Mirror default to a split beside the invoking remote
pane. Mirror does not preserve overlay presentation, and Herdr 0.8.2 opens an
overlay in its server's active tab, which can differ from the tab you are viewing.
An explicit `--placement` or `PLANNOTATOR_TUI_PLACEMENT` still takes precedence.

## Rex Annotate

Rex Annotate gives [Rex](https://www.superlogical.com/rex) the same reviews as Herdr Annotate:
a key opens the focused pane's agent replies, its folder, its git changes, or its output in
plannotator-tui, and **Send** makes the review the agent's next message:
`Send 3 new ▸ claude in a1d6db (E)`. Rex has no plugin manifest, so Rex Annotate is a Lua module
that Rex's config file loads.

```sh
brew trust plannotator/tap && brew install plannotator/tap/plannotator-tui
plannotator-tui rex install    # writes ~/.config/rex/plannotator/annotate.lua
```

Add the line it prints to `~/.config/rex/init.lua`, then reload:

```lua
require("plannotator.annotate").setup{}
```

```sh
rex config reload
```

`rex install` never edits `init.lua`. It writes the module beside it (in the folder of
`$REX_CONFIG` when that is set), naming this binary by its full path, because Rex runs actions
with a bare `PATH`. It then checks the module with `rex config check` against a scratch copy.
Run it again after upgrading plannotator-tui. `--dry-run` says what it would write;
`--print` writes the module to stdout instead.

| Key | Action (`rex do …`, palette: Annotate) | Opens |
|---|---|---|
| `Shift+Cmd+O` | `annotate_last` | the agent's recent replies |
| `Alt+Shift+Cmd+O` | `annotate_newest` | the agent's newest reply, with no picker |
| `Ctrl+Cmd+O` | `annotate_open` | the pane's folder, with a file tree |
| | `annotate_gate` | the pane's folder, with **Approve** (`A`) |
| | `annotate_terminal` | the pane's recent output |
| | `annotate_find` | the newest `.md` file the pane's output names |
| | `annotate_review` | a [code review](#code-review) of the uncommitted changes in the pane's repository |

Rex for Mac binds none of these keys by default. Change or add keys, or bind none:

```lua
require("plannotator.annotate").setup{
  placement = "split",                       -- tab | split | popup; default: [rex] placement, else tab
  keys = { annotate_gate = "ctrl+cmd+a", annotate_last = false },
  -- keys = false binds none; the actions stay in the palette
}
```

A review opens full size in a tab of its own, as Herdr's overlay does. It can open instead in
a split to the right of the pane, so the agent stays in view, or in a floating popup over the
pane's window:

```toml
# ~/.config/plannotator-tui/config.toml
[rex]
placement = "tab"   # tab (full size, default) | split | popup
```

The tab is a new tab (a Rex window) in the agent's session, named `Annotate · ` and the file,
agent or folder it reviews. Send and `P` work as in a split. When the review ends, Rex closes
its tab and plannotator-tui shows the pane it was opened from again, unless you have moved to
another tab by then. Earlier releases opened a split by default; set `placement = "split"` to
keep that.

Everything goes through the Rex server, so the app's Remote Control setting can stay off.

### Launchers

The actions run these, and so can a script or an agent. Each opens the review and exits:

```sh
plannotator-tui rex last [--newest]         # the agent's replies
plannotator-tui rex open [file.md | folder] # a document, or the pane's folder
plannotator-tui rex terminal [--lines N]    # the pane's recent output; --print writes it out
plannotator-tui rex find [--lines N]        # the newest .md file the pane's output names
plannotator-tui rex review [--staged | --base REF | --commit SHA | PR] # a code review of the pane's repository
```

The pane is `--block <id>`, else `REX_BLOCK`, so an agent that runs a launcher in its own
shell reviews itself; that is how it hands you a plan. `--placement split|popup|tab` overrides the
config. `--paste` puts the feedback in the agent's prompt without pressing Enter and focuses the
agent's pane, so you can edit it first; the header says `claude prompt in a1d6db`.
`rex open --gate [--result-file <path>]` adds Approve for a sign-off, as `herdr open --gate` does.
`rex review` reviews the git repository of the pane's folder, as `herdr review` does: a folder
that is not in a repository, or has no changes to review, fails before any split opens. Code
review is not available on Windows yet.

Send goes to the agent in the pane the review was opened from, with no question asked. `P`
lists every other agent Rex runs, in every session and window, the ones in the review's own
window first, so a review opened from one agent can go to another (see
[Choosing the agent](#choosing-the-agent)). A review you start yourself in a Rex pane, with
no launcher, asks which agent on the first Send.

The agent is the one the pane's program reports to Rex (its program status app, such as `pi`
or `claude-code`), else the one its process is named after. So pi, which runs under `node`,
gets Send like any other agent. A pane with no agent copies instead. A send waits while Rex
reports the agent blocked on a permission prompt or a question, as it does in Herdr: nothing is
sent or copied, the notes stay pending, and `E` retries. It also
needs the agent to report its state to Rex (Claude Code, Codex and pi do): until it has, the
review copies instead, since Enter could answer a dialog Rex cannot see, and `--paste` still
works. Rex does not report its theme to programs, so with a light Rex theme set
`[ui] theme = "light"`.

## Inside herdr-core

[herdr-core](https://github.com/herdrdev/herdr-core) (`hc`) runs the terminals and
[herdr-app](https://github.com/herdrdev/herdr-app) is its window. Neither has Herdr's plugins, so
plannotator-tui is started by a command rather than a manifest. Run it in an hc terminal:

```sh
plannotator-tui hc last [--newest] [--session <transcript>]   # the agent's replies
plannotator-tui hc open [file.md | folder]                    # a document, or the terminal's folder
plannotator-tui hc open plan.md --gate [--result-file <path>] # adds Approve, as `herdr open --gate`
plannotator-tui hc review [--staged | --base REF | --commit SHA | PR] # a code review of the terminal's repository
```

Each starts plannotator-tui in a terminal of its own, `annotate-<terminal>`, and shows it in
herdr-app: over the tab for `overlay` and `popup` (the default), or in a pane beside the focused
one for `split`, from `--placement` or `[herdr] placement`. The review starts once herdr-app shows
it: until herdr-app tells a new terminal its colours, the terminal answers a colour query with the
core's dark default, so asking sooner would draw a dark palette on a light window. herdr-app needs
a moment to learn of a new terminal, so the launcher asks again for up to 3 s while it answers
"no running terminal"; any other refusal, or no herdr-app window at all, ends that at once. Then the
review starts anyway and cannot know your colours: the launcher prints herdr-app's reason and the
`hc attach` command. Set `[ui] theme` for that case. The terminal is removed when the review
ends, and hc does not type a note about it into the agent's prompt. One review per terminal at a
time. `hc review` reviews the git repository of the terminal's folder, as `herdr review` does: a
folder that is not in a repository, or has no changes to review, fails before any terminal starts.

The terminal is `--terminal <t>` (an address or a name), else `HC_TERMINAL_ID`, so an agent that
runs a launcher in its own terminal reviews itself; that is how it hands you a plan. The header
button sends the review to the agent as its next message with `hc send`: `Send 3 new ▸ pi in t7 (E)`.
A terminal with no agent copies instead.

A send needs the agent to still be the terminal's foreground program and to have reported its state
to hc (OSC 7501), because Enter could answer a dialog hc cannot see. `hc send` also refuses an agent
that reports it is blocked, and the review says so; nothing is copied and `E` retries.

hc has no exact session for an agent, so `hc last` reads the newest transcript in the agent's folder.
Another agent in that folder may have written one more recently; `--session` names the right
transcript. Terminals on another machine and `terminal` reviews (a terminal's recent output) are not
supported yet. The feedback travels as one argument to `hc send`, so the OS caps its size (128 KiB
on Linux, about 1 MiB on macOS). A longer review is not sent: the review copies it instead and says
why.

## Choosing the agent

`P` (`pick-target`) opens a picker of every agent Send can reach, in document and code
reviews alike:

- running Pi sessions with the [Pi extension](#pi);
- inside Herdr, every pane with an agent (Claude, Codex, Pi and the rest), named by agent,
  folder, state, and `workspace › tab · pane`, so two Claude panes are told apart. The
  review's own pane is never listed.
- inside Rex, every pane with an agent in every Rex session and window (tab), named by
  agent, folder, state, and `session › window · pane`. A pane you gave a label shows it
  (`claude · reviewer`); otherwise the row shows the title the agent set, such as the task
  Claude Code is on (`claude · Fix login redirect`). An agent that has not reported its state
  to Rex reads `no state`; Send refuses it, as it does for a launcher (see
  [Rex Annotate](#rex-annotate)). A Pi session in a Rex pane is one row, sent to over the
  Pi link and placed where Rex has it. The review's own pane is never listed.

```
╭ send to which agent? ─────────────────────────────────────────────────────────────────╮
│ claude · Fix login redirect  ~/work/api      idle     work › api · 9ba798             │
│ codex                        ~/work/api/src  working  work › api · 31496a             │
│ claude · Migrate styles      ~/work/web      blocked  work › web · 63239c             │
│ claude · release             ~/work/notes    idle     side › Window 122adf3e · fd82e9 │
│ pi · π - notes               ~/work/notes    idle     work › web · eda7cc             │
╰ ↑↓ move · enter send here · esc close ────────────────────────────────────────────────╯
```

Inside Rex, the agents in the review's own window come first. Then agents in the review's
folder, then by name; the rows do not move while the picker is open. In a narrow pane the
folders are shortened first, so the state and the place stay in view. The pick becomes the
Send target until you change it, and the last row goes back to the target the review started
with.

Launched from an agent (a Herdr, Rex or herdr-core launcher, or `--deliver-to`), Send goes to
that agent and `P` switches. Launched on its own, with no [delivery command](#any-other-agent-a-delivery-command),
Send opens the picker whenever an agent is within reach, and the button says
`Send 1 new ▸ choose agent (E)`. `Enter` sends to the agent under the cursor (the first one)
and keeps it as the target; `Copy to clipboard`, one row up, copies this once; `Esc` sends
nothing. With no agent within reach, Send copies as before. herdr-core agents are reached
only through their launcher.

## Pi

plannotator-tui can send a review to any running [Pi](https://github.com/earendil-works/pi)
session, and Pi can leave notes in the file you have open. Install the extension once:

```sh
plannotator-tui pi install   # copies it into ~/.pi/agent/extensions/
```

Restart your Pi sessions, or run `/reload` in each one. Run `plannotator-tui pi install`
again after upgrading plannotator-tui: the binary does not notice an older extension, and
some fixes need the new one (for example, `last` following Pi's place after `/tree`).
Then:

- **Send to a Pi:** press `P` (`pick-target`) to choose a session (see
  [Choosing the agent](#choosing-the-agent)). Send (`E`) then goes to that session, as a
  user message. A busy Pi reads it once its current work is done.
- **Notes from Pi:** Pi gets a `plannotator_tui_note` tool. It quotes text from a file you
  have open, optionally with a line number, and its note shows up beside that text marked
  `pi · <session name>`. The note is sent with the rest of your review.
- `plannotator-tui pi list` lists the sessions; `plannotator-tui pi send --session <id or name>`
  sends stdin to one.

Each side lists itself in `clients/plannotator-tui/sessions/` in the data folder (see below) and listens on
127.0.0.1 with a secret token; nothing leaves this computer. The protocol is in
[`docs/spec-pi-link.md`](docs/spec-pi-link.md) and the extension in
[`integrations/pi/`](integrations/pi/).

## Any other agent: a delivery command

Herdr, Rex and herdr-core are built in. For anything else, such as tmux, a script or another
agent's CLI, name a command that takes the feedback on stdin:

```sh
export PLANNOTATOR_TUI_DELIVER_CMD='tmux load-buffer - && tmux paste-buffer -p -t agent && tmux send-keys -t agent Enter'
```

or, for every review:

```toml
# ~/.config/plannotator-tui/config.toml
[deliver]
command = "~/bin/send-to-agent"
```

The environment variable wins over the config. Send (`E`) then runs the command and the header
names it by its first word: `Send 3 new ▸ cmd · tmux (E)`.

- **How it runs:** the value is one command line for the shell: `sh -c` on macOS and Linux,
  `cmd /S /C` on Windows. Pipelines, `&&` and variables work. The feedback is on stdin, the
  same text `E` would copy. `PLANNOTATOR_TUI_REVIEW_PATH` is the file or folder under review,
  when the review opened one, and the command inherits the review's environment.
- **The exit status is the outcome:**

  | Exit | Means | The review |
  |---|---|---|
  | `0` | delivered | marks the notes sent; the first line on stdout, if any, joins the status |
  | `75` | the agent cannot take a message now, e.g. it is at a dialog | keeps the notes pending and copies nothing; `E` retries |
  | `69` | there is no agent to send to | copies the feedback; `E` retries |
  | anything else | the send failed | shows the first line of stderr; `E` retries |

  75 and 69 are `EX_TEMPFAIL` and `EX_UNAVAILABLE` from `sysexits.h`. The first line of stderr
  (else stdout) says why in the status.
- **It should be quick.** The review waits for the command and stops it after 30 s. A child it
  leaves running in the background (`cmd &`) does not hold the review up.
- **Where it sits:** a target that a Herdr, Rex or herdr-core launcher named for this review
  comes first, because it names one agent for one review. The command replaces the clipboard
  everywhere else. A gated review (`--gate`) with no launcher still gives its decision to
  whoever ran it, and `P` can still switch Send to another agent.

For Pi, `plannotator-tui pi send` is already such a command (see [Pi](#pi)):

```sh
PLANNOTATOR_TUI_DELIVER_CMD='plannotator-tui pi send --session my-session' plannotator-tui plan.md
```

If that session has closed, or no session matches, `pi send` exits 69, so the review copies the
feedback instead.

## Agent replies

`plannotator-tui last` finds the transcript of the agent that launched your shell and shows a
picker of its recent replies. Hosts: Claude Code, Codex, pi, Oh My Pi, GitHub Copilot CLI,
Droid, Hermes CLI, OpenCode (1 and 2), Command Code (`cmd`), Antigravity CLI (`agy`). `--host`, `--pid`, `--session <transcript>` (format sniffed when
no host is named) and `--session-id <id>` (Hermes, OpenCode) override detection; `--stdin`
reads a document; `--newest` skips the picker and opens the newest reply straight away, with
`p` still opening the picker on the rest; the picker offers the 25 newest replies, or as many
as `pick` under `[last]` in the config file says, also inside Herdr and Rex, and `--pick N` for
one run;
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
