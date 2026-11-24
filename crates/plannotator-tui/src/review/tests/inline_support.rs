//! What every frame with notes drawn in the diff must hold, read from the frame and the
//! viewer's rows: each box is inside the rows kept for it, over no code, under the line of
//! its own note (by file, side and line), saying that note's words; and where the cursor's
//! line is, to tell it did not move.

use diffs_rs::engine::DiffRowKind;
use plannotator_tui_review::{CodeAnnotation, NoteScope, Side};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::*;

/// The text of `buffer`'s row `y` from `x` for `width` cells.
pub(super) fn cells(buffer: &Buffer, x: u16, y: u16, width: u16) -> String {
    (x..x + width).filter_map(|x| buffer.cell((x, y)).map(|c| c.symbol().to_owned())).collect()
}

/// Every box drawn in the last frame holds: see the module.
pub(super) fn check_boxes(screen: &Screen, buffer: &Buffer, case: &str) {
    let pane = screen.geometry.diff;
    let areas = screen.diff.box_areas();
    assert_eq!(
        areas.iter().map(|a| a.id).collect::<Vec<_>>(),
        screen.geometry.boxes.iter().map(|(_, id)| *id).collect::<Vec<_>>(),
        "{case}: every box the viewer kept rows for is drawn"
    );
    for (&(rect, id), area) in screen.geometry.boxes.iter().zip(areas) {
        let note = screen.note_of_box(id).unwrap_or_else(|| panic!("{case}: box {id} has a note"));
        let what = format!("{case}: box {id} ({})", Screen::box_label(note));
        assert!(rect.width > 0 && rect.height > 0, "{what}: drawn");
        assert_eq!(rect.intersection(area.area), rect, "{what}: inside its rows {area:?}");
        assert!(
            pane.contains(rect.as_position())
                && rect.right() <= pane.right()
                && rect.bottom() <= pane.bottom()
        );
        for y in rect.top()..rect.bottom() {
            for x in [rect.left(), rect.right() - 1, pane.left(), pane.right() - 1] {
                assert_eq!(screen.diff.box_at(x, y), Some(id), "{what}: row {y} is its own, over no code");
            }
        }
        let left = buffer.cell((rect.x, rect.y)).map(|c| c.symbol().to_owned()).unwrap_or_default();
        let top = cells(buffer, rect.x, rect.y, rect.width);
        if area.clipped_top == 0 {
            assert_eq!(left, "\u{256d}", "{what}: its top-left corner first: {top:?}");
            assert!(top.contains(&Screen::box_label(note)), "{what}: titled for its note: {top:?}");
            if let Some(word) = note.text.as_deref().and_then(|t| t.split_whitespace().next())
                && rect.height > 1
            {
                let body = cells(buffer, rect.x, rect.y + 1, rect.width);
                assert!(body.contains(word), "{what}: says its note's words: {body:?}");
            }
            anchored(screen, note, rect.y, &what);
        }
        let bottom_cut = area.height - area.clipped_top - area.area.height;
        if bottom_cut == 0 {
            let corner = buffer.cell((rect.x, rect.bottom() - 1)).map(|c| c.symbol().to_owned());
            assert_eq!(corner.as_deref(), Some("\u{2570}"), "{what}: its bottom border shows");
        }
    }
}

/// The first row above screen row `y` that is no box shows the note's line, or its file's
/// heading for a file note.
fn anchored(screen: &Screen, note: &CodeAnnotation, y: u16, what: &str) {
    let pane = screen.geometry.diff;
    let column = pane.right() - 2;
    let above = (pane.top()..y).rev().find(|&row| screen.diff.box_at(column, row).is_none());
    let Some(above) = above else { return };
    let row = screen.diff.row_at(column, above).unwrap_or_else(|| panic!("{what}: a row above"));
    assert_eq!(row.path, note.file_path, "{what}: under a row of its file");
    // Its line can be the one the sticky heading covers.
    if row.kind == DiffRowKind::StickyHeader && note.scope == NoteScope::Line {
        return;
    }
    if note.scope == NoteScope::File {
        assert!(matches!(row.kind, DiffRowKind::Header | DiffRowKind::StickyHeader), "{what}: {row:?}");
        return;
    }
    let line = if note.side == Side::Old { row.old_line } else { row.new_line };
    assert_eq!(row.kind, DiffRowKind::Code, "{what}: under code: {row:?}");
    assert_eq!(line, Some(note.line_end), "{what}: under its last line on its side: {row:?}");
}

/// Where the cursor is, by what it shows: file, lines and kind. (Not the wrapped part: a
/// line wrapped again at another width can put the cursor on another of its parts.)
pub(super) type Spot = (String, Option<u32>, Option<u32>, String);

pub(super) fn spot(screen: &Screen) -> Spot {
    let row = screen.diff.cursor_row().expect("a cursor");
    (row.path.to_string(), row.old_line, row.new_line, format!("{:?}", row.kind))
}

/// The screen row of the last frame that shows `spot`, its first part when wrapped.
pub(super) fn row_of_spot(screen: &Screen, spot: &Spot) -> Option<u16> {
    let pane = screen.geometry.diff;
    let column = |side| match screen.diff.split_line() {
        Some(line) if side == Side::Old => pane.x + (line - pane.x) / 2,
        _ => pane.right() - 2,
    };
    let side = screen
        .diff
        .cursor_row()
        .map(|row| if row.side == diffs_rs::engine::Side::Old { Side::Old } else { Side::New });
    let x = column(side.unwrap_or(Side::New));
    (pane.top()..pane.bottom()).find(|&y| {
        screen.diff.row_at(x, y).is_some_and(|row| {
            (row.path.to_string(), row.old_line, row.new_line, format!("{:?}", row.kind)) == *spot
        })
    })
}

/// The screen row of the last frame the cursor is on: the part of its line it is on, which
/// the viewer keeps on its row when the line wraps again at another width.
pub(super) fn row_of_cursor(screen: &Screen) -> Option<u16> {
    let cursor = screen.diff.cursor_row()?;
    let spot = spot(screen);
    let first = row_of_spot(screen, &spot)?;
    Some(first + u16::try_from(cursor.part).ok()?)
}

/// Rows of the pane the viewer drew nothing on must all be at its bottom, past the diff's end.
pub(super) fn no_holes(screen: &Screen, case: &str) {
    let pane = screen.geometry.diff;
    let shown: Vec<bool> =
        (pane.top()..pane.bottom()).map(|y| screen.diff.row_at(pane.right() - 2, y).is_some()).collect();
    let first_empty = shown.iter().position(|s| !s).unwrap_or(shown.len());
    assert!(shown.iter().skip(first_empty).all(|s| !s), "{case}: an empty row inside the diff: {shown:?}");
}

/// A note made as `c` makes one, on `first..=last` of `side` of `path`.
pub(super) fn comment(screen: &mut Screen, path: &str, side: Side, (first, last): (u32, u32), text: &str) {
    let key = screen.capture.patch_key.clone();
    let file = screen.file(path).expect("the file");
    let note = CodeAnnotation::line_comment(file, side, first, last, text.to_owned(), &key);
    screen.add_note(note).expect("saved");
}

/// The rectangle the last frame drew the box of the note saying `text` in.
pub(super) fn box_saying(screen: &Screen, text: &str) -> Option<Rect> {
    screen.geometry.boxes.iter().find_map(|&(rect, id)| {
        screen.note_of_box(id).filter(|note| note.text.as_deref() == Some(text)).map(|_| rect)
    })
}
