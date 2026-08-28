# plannotator-tui

Annotate Markdown in the terminal: select text, comment, 👍 looks good, ✗ delete.

```bash
cargo build --release
./target/release/plannotator-tui samples/plugins.md     # one file
./target/release/plannotator-tui samples                # a folder: tree on the left
```

## Keys

| Where | Keys |
|---|---|
| anywhere | `Tab` cycle focus (tree · document · rail) · `E` send feedback (clipboard) · `t` show/hide tree · `r` reload · `q` quit |
| document | drag with the mouse, or `v` then `hjkl` / `w` `b` / `0` `$` to select; `Enter` confirms · `j`/`k` or click selects a block · `c` comments on the block · `x` clears the block's annotations |
| selection toolbar | `a` 👍 looks good · `c` 💬 comment (opens a box at the selection) · `d` ✗ delete · `Esc` clears |
| rail | `j`/`k` move · `e` / `Enter` edit body · `x` remove · click a bubble to focus it |
| tree | `j`/`k` move · `Enter` open · `E` send feedback for every annotated file · counts show per file |

Selections and exports are copied to the terminal clipboard (OSC 52).

## Where things live

Every annotation is saved the moment it is made, as JSON, in the Plannotator data directory:

```
~/.plannotator/clients/plannotator-tui/annotations/<project>/<slug>/annotations.json
```

`<project>` and `<slug>` follow Plannotator's own rules (git repo name; basename + 8 hex of
sha256 of the path), so one file maps to one directory in both tools. The records are in the
Plannotator Workspaces wire shape (`plannotator-tui-schema`); any agent can read them. Nothing is
written next to your files. `PLANNOTATOR_DATA_DIR` relocates the directory. Transient
documents (an agent's last message, stdin) are never persisted.

## Headless tools

```bash
plannotator-tui --export <file.md>                     # feedback markdown to stdout
plannotator-tui --bench <file.md>                      # parse / render / reflow timings
plannotator-tui --blocks <file.md>                     # block index, kind, first row
plannotator-tui --annotate <file.md> <quote> <text> [comment|looks_good|delete]
plannotator-tui --annotate-block <file.md> <block> <text>
plannotator-tui --snapshot <file|folder> [cols rows scroll] [select-quote]   # one frame as text + mark map
```

## Measured (Apple Silicon, release build)

| corpus | blocks | rows | render + align | reflow on resize | row lookup |
|---|---|---|---|---|---|
| plugins.md (16 KB) | 60 | 305 | 14 ms | 0.7 ms | 20 ns |
| big.md (2.5 MB, 50k lines) | 14,100 | 56,999 | 352 ms | 50 ms | 15 ns |

Per frame, only visible rows are touched.

## Known limits

- Reference-style links and footnotes lose their target when a block is rendered alone.
- A whole list is one block for block-level commands; text selection is not affected.
