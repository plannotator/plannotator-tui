# plannotui

Annotate Markdown in the terminal: select text, comment, 👍 looks good, ✗ delete.
A standalone app, and a Herdr plugin that knows how to open it.

- `crates/plannotui-schema` - annotation and anchor types, wire-compatible with
  Plannotator Workspaces (phase 1).
- `crates/plannotui` - the TUI application.
- `herdr/` - the Herdr plugin manifest; `skills/` - the agent skill.
- `samples/` - documents for development and benchmarks.

## Run

```bash
cargo build --release
./target/release/plannotui samples/plugins.md
```

## Inside Herdr

Link the plugin once (`herdr plugin link ./herdr`), build it (`cargo build --release`), and
bind a key:

```toml
# ~/.config/herdr/config.toml
[[keys.command]]
key = "prefix+a"
type = "plugin_action"
command = "plannotui.open"
```

`prefix+a` opens the focused pane's folder for review; Ctrl-click on a `file://…md` link an
agent printed opens that file; an agent runs `plannotui herdr open <file>` from its own pane
(see `skills/plannotui/SKILL.md`). In every case the header button says where feedback goes
— `Send 3 to claude in w1:p2 ▸` — and clicking it (or `E`) makes the review the agent's next
message. Where plannotui opens is yours to choose:

```toml
# ~/.config/plannotui/config.toml
[herdr]
placement = "overlay"   # overlay (full tab, default) | split | popup
```

`plannotui config` prints the file's path and the values in effect.

See `crates/plannotui/README.md` for keys, headless tools, and measurements;
`docs/decisions.md` for the design record; `AGENTS.md` for engineering rules.
