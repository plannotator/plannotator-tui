# plannotui

Annotate Markdown in the terminal: select text, comment, 👍 looks good, ✗ delete.

```bash
cargo build --release
./target/release/plannotui samples/plugins.md     # one file
./target/release/plannotui samples                # a folder: tree on the left
```

## Keys

| Where | Keys |
|---|---|
| anywhere | `Tab` cycle focus (tree · document · rail) · `E` copy feedback markdown · `r` reload · `q` quit |
| document | drag with the mouse, or `v` then `hjkl` / `w` `b` / `0` `$` to select; `Enter` confirms · `j`/`k` or click selects a block · `c` comments on the block · `x` clears the block's annotations |
| selection toolbar | `a` 👍 looks good · `c` 💬 comment (opens a box at the selection) · `d` ✗ delete · `Esc` clears |
| rail | `j`/`k` move · `e` / `Enter` edit body · `x` remove · click a bubble to focus it |
| tree | `j`/`k` move · `Enter` open |

Selections and exports are copied to the terminal clipboard (OSC 52).

## Where things live

Annotations are saved next to the file as `<name>.annotations.json`, in the same shape the
Plannotator Workspaces API uses (`plannotui-schema`). Documents opened as transient sources
(an agent's last message, stdin) are never persisted.

## Headless tools

```bash
plannotui --export <file.md>                     # feedback markdown to stdout
plannotui --bench <file.md>                      # parse / render / reflow timings
plannotui --blocks <file.md>                     # block index, kind, first row
plannotui --annotate <file.md> <quote> <text> [comment|looks_good|delete]
plannotui --annotate-block <file.md> <block> <text>
plannotui --snapshot <file|folder> [cols rows scroll] [select-quote]   # one frame as text + mark map
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
