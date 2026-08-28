# plannotui

Annotate Markdown in the terminal: select text, comment, 👍 looks good, ✗ delete.
A standalone app, and a Herdr plugin that knows how to open it.

- `crates/plannotui-schema` - annotation and anchor types, wire-compatible with
  Plannotator Workspaces (phase 1).
- `crates/plannotui` - the TUI application.
- `herdr/` - the Herdr plugin manifest and launch scripts.
- `samples/` - documents for development and benchmarks.

## Run

```bash
cargo build --release
./target/release/plannotui samples/plugins.md
```

See `crates/plannotui/README.md` for keys, headless tools, and measurements;
`docs/decisions.md` for the design record; `AGENTS.md` for engineering rules.
