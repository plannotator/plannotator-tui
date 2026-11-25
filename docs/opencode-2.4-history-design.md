# Restore the reply picker for OpenCode 2.4

## Problem

Herdr's `annotate.last` calls `plannotator-tui herdr last`. The OpenCode V2 reader in `crates/plannotator-tui-hosts/src/opencode.rs` reads assistant text only from `session_message.data.content[]`. In observed OpenCode 2.4.0/2.4.1 sessions, `content` is empty and completed replies have text rows in `session_part`. A resumed session can contain both formats, so the picker can silently fall back to older replies. A new session can instead report that there are no assistant messages.

## Proposed change

Keep the existing session resolution and the `session_message` scan in descending `seq` order. In `messages_v2`:

1. Check once whether `session_part` exists. If it does, prepare a reusable read-only statement: `SELECT data FROM session_part WHERE message_id = ?1 AND session_id = ?2 AND type = 'text' ORDER BY ordinal ASC`. The primary key starts with `message_id`, so the query does not need a session-wide scan or a new index.
2. For each assistant message, extract visible inline text exactly as today. If that produces no nonblank text and the parts statement is available, run it for that message and extract `data.text` using the existing `part_text` helper. Join qualifying parts with the existing paragraph separator (`\n\n`). Choose one source per message; do not concatenate them and risk duplicate replies.
3. Leave user prompts, message IDs, timestamps, ordering and the reply limit unchanged. Blank, ignored, synthetic, reasoning-only and tool-only parts do not count as replies. Preserve normal SQL error reporting; an absent optional table is the expected legacy case, not an error.

Detect the format by available text and tables rather than `session_v2.version`: resumed sessions can retain their original version while storing new replies as parts. Do not modify the OpenCode database or change Herdr shortcuts, delivery or the V1 reader.

## Verification

Add a synthetic mixed-history fixture in `crates/plannotator-tui-hosts/tests/opencode.rs`: an older inline reply; a newer multipart reply with empty inline content; a reply seeded in both formats; a user prompt; and a more recent assistant row with only non-displayable parts. Insert multipart rows out of order. Assert that the reader returns the new reply first, assembles its parts in ordinal order, emits the dual-format reply once, preserves prompt/ID/timestamp behavior and applies the limit to visible assistant replies. Keep the existing V1 and V2-without-`session_part` tests.

Run `cargo test -p plannotator-tui-hosts --test opencode`. Then compare `plannotator-tui last --host opencode --session-id <id> --print` with the newest visible reply in a known affected session, without sharing its contents. Finally, try `Ctrl+B`, then `Shift+O` in the focused OpenCode pane with the patched binary actually used by the Herdr plugin; the Homebrew binary is a separate installation.
