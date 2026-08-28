# plannotui

Prototype answering one question: can we render Markdown with an off-the-shelf renderer,
lay it out per block, and pin comments to blocks - performantly, inside a Herdr pane?

Stack: `ratatui` + `pulldown-cmark` (parser) + `tui-markdown` (events → styled text).
We own only: block splitting (`doc.rs`), word wrap that carries a source offset per cell
(`wrap.rs`), the row map (`layout.rs`), quote-based anchors (`comments.rs`), and the
alignment that recovers source offsets from rendered text (`srcmap.rs`) - a character diff
between a block's source and what the renderer produced, so the renderer never needs to
know about positions.

## Run

```bash
cargo build --release
./target/release/plannotui samples/plugins.md
```

**Drag with the mouse to select text.** A toolbar appears beside the selection:
`👍 looks good (a)` · `💬 comment (c)` · `✗ delete (d)`. Click an item or press its key.
Comment opens a small box at the selection; Enter saves, Esc cancels. Looks-good tints the
text green, delete strikes it through, comment tints it yellow; all three get a bubble in
the rail. The selection is also copied to the clipboard (OSC 52).

Without a selection: `j`/`k` or click selects a block · `c` comments on the whole block ·
`x` clears the block's annotations · wheel / PgUp / PgDn scrolls · Esc clears the
selection · `r` reloads the file from disk (annotations re-resolve; unfound ones are
counted as orphaned) · `q` quits.

Comments are saved next to the file as `<name>.annotations.json`.

## Inside Herdr

```bash
herdr plugin link "$PWD"                              # once
herdr plugin pane open --plugin ramos.plannotui --entrypoint doc --focus
# side-by-side instead of full screen:
herdr plugin pane open --plugin ramos.plannotui --entrypoint doc --placement split --focus
# another file:
herdr plugin pane open --plugin ramos.plannotui --entrypoint doc --focus --env ANNOTATE_FILE=/path/to/doc.md
```

## Headless tools

```bash
plannotui --bench <file.md>                     # parse / render / reflow timings
plannotui --blocks <file.md>                    # block index, kind, first row
plannotui --add-comment <file.md> <block> <text>
plannotui --add-quote-comment <file.md> <quote> <text> [comment|approve|delete]
plannotui --snapshot <file.md> [cols rows scroll] [select-quote]  # one frame as text + mark map (# comment, + approve, - delete, % selected)
```

## Measured (Apple Silicon, release build)

| corpus | blocks | rows | render + align | reflow on resize | row lookup | cells mapped |
|---|---|---|---|---|---|---|
| plugins.md (16 KB) | 60 | 305 | 14 ms | 0.7 ms | 20 ns | 99% |
| big.md (2.5 MB, 50k lines) | 14,100 | 56,999 | 374 ms | 50 ms | 15 ns | 94% |

Unmapped cells are renderer decoration (bullets, table borders, code fences).

Per-frame cost touches only visible rows.

## Known limits of the per-block trick

- Reference-style links (`[text][id]` with the definition elsewhere) and footnotes lose
  their target when a block is rendered alone.
- A whole list is one block; commenting on a single list item is not yet possible.
- A selection is clamped to the block it starts in.
- Keyboard-only selection (shift+arrows) is not implemented yet; keyboard users comment
  per block.
