# Spec: plannotui inside Herdr

Status: draft for review, 2026-08-28. Every Herdr fact below was verified in the Herdr
source (`~/oss/herdr/herdr`); the two things that need a live check are marked.

## The shape

Herdr never renders plannotui. It does four things for it: **opens it in a pane** with the
right file and the right context, **tells it which agent** to send feedback to, **routes
clicks and keys** to it, and **delivers feedback** into an agent's pane. plannotui stays a
standalone app; the plugin is a manifest plus a few shell lines. Delivery is the seam from
decision 11: `HerdrAgent { pane }` = `$HERDR_BIN_PATH agent prompt <pane> <feedback>`.

## Five ways in

| # | Who starts it | How the file is chosen | How the target agent is known | Herdr mechanism |
|---|---|---|---|---|
| 1 | Human, keybind | The tree: plannotui opens on the pane's cwd in folder mode with the tree focused — that *is* the file dialog | `HERDR_PLUGIN_CONTEXT_JSON.focused_pane_id` (the agent's pane, snapshotted before plannotui's pane spawns) | `[[keys.command]] type="plugin_action"` → action → `plugin pane open --placement overlay` |
| 3 | **Agent**, via a skill | The agent wrote the plan to a file and names it | The agent passes its own `$HERDR_PANE_ID` | agent runs `herdr plugin pane open … --placement split --target-pane $HERDR_PANE_ID --env …` |
| 4 | Human, Ctrl-click | The clicked `file://…md` link the agent printed (OSC 8) | focused pane at click time | `[[link_handlers]]` → action → pane open |
| 5 | Human, keybind | The agent's **last message**, extracted from its transcript | focused pane; the agent process pid from `herdr pane process-info` finds the session file directly | action → `plannotui last --deliver-to <pane>` (phase 4 hosts crate) |
| 6 | Human, quick | One file, in a **popup** (modal, no pane id) | same as 1 | `[[panes]] placement="popup"` — a second entrypoint for "review this one thing and send" |

Not built: a Herdr-side path prompt. Herdr has no input dialog for plugins; a popup picker
that then opens a pane runs into the popup singleton (`ui_busy`, needs a detached helper).
The tree-as-dialog in #1 is the same gesture with none of that.

## The contract between the manifest and the app

Environment the pane command reads, in precedence order:

```
PLANNOTUI_FILE          file or folder to open       (explicit, from an action or the agent)
PLANNOTUI_DELIVER_TO    pane id to send feedback to  (explicit)
HERDR_PLUGIN_CONTEXT_JSON
  .focused_pane_id      fallback delivery target: the agent's pane when a human triggered it
  .focused_pane_agent   shown in the footer: "send → claude in w1:p2"
  .workspace_cwd        fallback folder for #1
  .clicked_url          #4
HERDR_ENV=1             selects HerdrAgent delivery; absent → clipboard
HERDR_BIN_PATH          the herdr binary (fallback: `herdr` on PATH)
```

`plannotui` resolves: `PLANNOTUI_FILE` → `clicked_url` (as a path) → `workspace_cwd`. Delivery: `PLANNOTUI_DELIVER_TO` → `focused_pane_id` →
clipboard. The footer always names the target before the user presses `E`.

## Manifest

```toml
id = "plannotui"
name = "plannotui"
version = "0.2.0"
min_herdr_version = "0.8.0"
platforms = ["macos", "linux"]

[[build]]
command = ["bash", "herdr/install.sh"]          # prebuilt binary per platform, checksummed

[[panes]]                                       # #1 #3 #4: a real pane
id = "doc"
title = "plannotui"
placement = "overlay"
command = ["sh", "-c", "exec \"$HERDR_PLUGIN_ROOT/bin/plannotui\" \"${PLANNOTUI_FILE:-$PWD}\""]

[[panes]]                                       # #6: quick modal review
id = "quick"
title = "plannotui"
placement = "popup"
width = "90%"
height = "85%"
command = ["sh", "-c", "exec \"$HERDR_PLUGIN_ROOT/bin/plannotui\" \"${PLANNOTUI_FILE:-$PWD}\""]

[[actions]]                                     # #1
id = "open"
title = "Annotate: open here"
contexts = ["workspace", "pane"]
command = ["bash", "herdr/open.sh"]             # pane open doc, cwd from context, tree focused

[[actions]]                                     # #4 target
id = "open-link"
title = "Annotate: linked file"
contexts = ["pane"]
command = ["bash", "herdr/open.sh", "--from-link"]

[[link_handlers]]
id = "markdown-file"
title = "Annotate this file"
pattern = "^file://.*\\.(md|markdown|mdx)$"
action = "open-link"

[[actions]]                                     # #5
id = "last"
title = "Annotate: agent's last message"
contexts = ["pane"]
command = ["bash", "herdr/last.sh"]
```

`open.sh` is ~20 lines: read the context JSON, pick the file per the precedence above, then
`exec "$HERDR_BIN_PATH" plugin pane open --plugin plannotui --entrypoint doc --focus
--cwd "$folder" --env PLANNOTUI_FILE="$file" --env PLANNOTUI_DELIVER_TO="$pane"`.

Keybindings the user adds (Herdr has no manifest keybindings):

```toml
[[keys.command]]
key = "prefix+a"
type = "plugin_action"
command = "plannotui.open"

[[keys.command]]
key = "prefix+l"
type = "plugin_action"
command = "plannotui.last"
```

## The agent side (#3): one skill

`skills/plannotui/SKILL.md`, installed like Herdr's own skill. The whole instruction:

> When you want the human to review a plan or document: write it to a file, then run
> `herdr plugin pane open --plugin plannotui --entrypoint doc --placement split
> --direction right --target-pane "$HERDR_PANE_ID" --focus --env PLANNOTUI_FILE=<path>
> --env PLANNOTUI_DELIVER_TO="$HERDR_PANE_ID"`, and **end your turn**. The review arrives
> as your next user message, as numbered feedback (`## 1. (line 12) Feedback on: "…"`).
> Address each item. Only when `HERDR_ENV=1`.

The agent never waits on plannotui and never parses its output. Feedback is a normal turn.

## The Send button

Herdr is mouse-first, so sending is a visible control, not only a key. The rail header
holds one button whose label names the target and the count, and whose existence is
decided by the same rule as the footer:

```
target is an agent pane   →  [ Send 3 to claude ▸ ]     click or E
target is the clipboard   →  [ Copy 3 as feedback ]     click or E
no annotations yet        →  button shown dimmed, does nothing
```

After a click the button reads `Sent ▸ claude` for a few seconds, then returns. On
`agent_blocked` it reads `claude is at a dialog — copied instead` and stays clickable so
a second click retries. The button and `E` call the same `App::send`; there is no second
path. Dropped: #2 (selection), it was a worse #4 and a worse #5.

## Delivery

`HerdrAgent::deliver`: `$HERDR_BIN_PATH agent prompt <pane> <feedback>`. Verified
semantics: the text is one argument; Herdr encodes it with the pane's live bracketed-paste
mode and sends Enter 300 ms later, so multi-line feedback is safe. Outcomes the app shows:

- `ok` → `sent 3 annotations → claude in w1:p2`.
- `agent_blocked` (the agent is at a dialog) → not sent; status says so; `E` again retries;
  feedback is also copied to the clipboard so nothing is lost.
- `agent_not_found` / pane closed → clipboard, status names it.

`q` with unsent annotations asks once: `send to claude in w1:p2? (y/n/esc)`.

## What each placement means for the user

- **overlay** (#1, #4): full-tab, restores the previous focus and zoom on exit. Real
  pane: can be unzoomed or moved. Default.
- **split** (#3): beside the agent; both visible; you watch the agent react to the review.
- **popup** (#6): modal, singleton, swallows Escape and the prefix key, no pane id. Good for
  a quick one-file review, wrong for a folder session.

## Verified vs to confirm

Verified in Herdr source: overlay/split context is snapshotted before the new pane spawns
(`src/app/api/plugins/panes.rs:51`); `agent prompt` semantics; `file://` OSC 8 links reach
link handlers (`src/app/input/mod.rs:598-617`); actions have no TTY; popup is a singleton
that returns `ui_busy` outside plain terminal mode; `--env` cannot override `HERDR_*`.

To confirm live, in your terminal: mouse drag inside an overlay pane (selection UX depends
on it); `agent prompt` into Claude Code with a multi-paragraph feedback body.

## Order of work

1. `HerdrAgent` delivery + env resolution in the app (~80 lines, decision 11).
2. Manifest + `open.sh` + keybinding docs: #1, #4, #6.
3. The skill file: #3.
4. #5 after the hosts crate exists (phase 4).
