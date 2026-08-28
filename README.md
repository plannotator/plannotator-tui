<img src="docs/assets/banner.webp" alt="Plannotator TUI" width="720">

Annotate Markdown in the terminal: select text, comment, 👍 looks good, ✗ delete.
A standalone app, and a Herdr plugin that knows how to open it.

- `crates/plannotator-tui-schema` - annotation and anchor types, wire-compatible with
  Plannotator Workspaces (phase 1).
- `crates/plannotator-tui` - the TUI application.
- `herdr/` - the Herdr plugin manifest; `skills/` - the agent skill.
- `samples/` - documents for development and benchmarks.

## Run

```bash
cargo build --release
./target/release/plannotator-tui samples/plugins.md
```

## Inside Herdr

Link the plugin once (`herdr plugin link ./herdr`), build it (`cargo build --release`), and
bind a key:

```toml
# ~/.config/herdr/config.toml
[[keys.command]]
key = "prefix+a"
type = "plugin_action"
command = "plannotator-tui.open"

[[keys.command]]
key = "prefix+shift+d"
type = "plugin_action"
command = "plannotator-tui.last"
```

`prefix+a` opens the focused pane's folder for review; Ctrl-click on a `file://…md` link an
agent printed opens that file; an agent runs `plannotator-tui herdr open <file>` from its own pane
(see `skills/plannotator-tui/SKILL.md`). In every case the header button says where feedback goes
— `Send 3 to claude in w1:p2 ▸` — and clicking it (or `E`) makes the review the agent's next
message. Where plannotator-tui opens is yours to choose:

```toml
# ~/.config/plannotator-tui/config.toml
[herdr]
placement = "overlay"   # overlay (full tab, default) | split | popup
```

`plannotator-tui config` prints the file's path and the values in effect.

See `crates/plannotator-tui/README.md` for keys, headless tools, and measurements;
`docs/decisions.md` for the design record; `AGENTS.md` for engineering rules.
