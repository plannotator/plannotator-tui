<img src="docs/assets/banner.webp" alt="Plannotator TUI" width="720">

Annotate Markdown in the terminal. Select text, leave a 💬 comment, mark it 👍 looks good or
✗ delete, and hand the review to a coding agent as numbered feedback. One static binary,
no runtime. Rust + ratatui.

[![crates.io](https://img.shields.io/crates/v/plannotator-tui?style=flat-square)](https://crates.io/crates/plannotator-tui)
[![release](https://img.shields.io/github/v/release/plannotator/plannotator-tui?style=flat-square)](https://github.com/plannotator/plannotator-tui/releases)
[![ci](https://img.shields.io/github/actions/workflow/status/plannotator/plannotator-tui/post-merge.yml?branch=main&style=flat-square&label=main)](https://github.com/plannotator/plannotator-tui/actions/workflows/post-merge.yml)

```text
                                                                             Copy 2 as feedback
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
review to the clipboard as numbered annotations (`# Annotations on plan.md`, `## Annotation 1
(line 12)`, …). Every annotation is saved as JSON the moment you make it; `q` closes.

| Where | Keys |
|---|---|
| anywhere | `Tab` cycle tree · document · notes; `E` send; `t` tree; `r` reload; `q` quit |
| document | `j`/`k` block; `c` comment on the block; `x` clear its annotations; `v` select with `hjkl` `w` `b` `0` `$` |
| toolbar | `a` looks good · `c` comment · `d` delete · `Esc` |
| notes | `j`/`k`; `e` edit; `x` remove; click a bubble |
| tree | `j`/`k`; `Enter` open; `E` sends every annotated file |

## Inside Herdr

Install [Herdr Annotate](https://github.com/plannotator/herdr-annotate); it bundles this binary,
opens it in a pane with `prefix+o` (folder) or `prefix+shift+o` (agent's last reply) or by
Ctrl-clicking a `file://…md` link, and the header button sends the review straight back to
the agent as its next message: `Send 3 to claude in w1:p2 ▸`.

```toml
# ~/.config/plannotator-tui/config.toml
[herdr]
placement = "overlay"   # overlay (full tab, default) | split | popup
```

`plannotator-tui config` prints the file's path and the values in effect. The `herdr/`
directory in this repo is the development manifest; users should install Herdr Annotate.

## Agent replies

`plannotator-tui last` finds the transcript of the agent that launched your shell and shows a
picker of its recent replies. Hosts: Claude Code, Codex, pi, Oh My Pi, GitHub Copilot CLI,
Droid, Hermes CLI, OpenCode (1 and 2). `--host`, `--pid`, `--session <transcript>` (format sniffed when
no host is named) and `--session-id <id>` (Hermes, OpenCode) override detection; `--stdin`
reads a document;
`--print` writes the newest reply to stdout and always exits 0 (for hooks and scripts).
Reply reviews are never written to disk.

## Where annotations live

```
~/.plannotator/clients/plannotator-tui/annotations/<project>/<slug>/annotations.json
```

`<project>` is the git repo name, `<slug>` the file's basename plus 8 hex of the sha256 of
its path: Plannotator's own layout, so both tools see one record per file. The JSON is the
Plannotator Workspaces wire shape; any agent can read it. Nothing is written next to your
files. `PLANNOTATOR_DATA_DIR` relocates the directory.

## Headless

```sh
plannotator-tui --export <file|folder>                          # the review, to stdout
plannotator-tui --annotate <file> <quote> <text> [comment|looks_good|delete]
plannotator-tui --snapshot <file|folder> [cols rows scroll] [quote]   # one frame as text
plannotator-tui --bench <file>                                  # parse / layout timings
```

## Repository

- `crates/plannotator-tui`: the app. `crates/plannotator-tui-schema`: annotation and anchor
  types, wire-compatible with Plannotator Workspaces. `crates/plannotator-tui-hosts`: agent
  transcript readers.
- `docs/decisions.md` is the design record; `AGENTS.md` the engineering rules;
  `crates/plannotator-tui/README.md` the full key reference and measurements.

MIT.
