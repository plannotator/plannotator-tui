//! Notes drawn in the diff (`[review] notes = "inline"` or `"both"`): a box under each note's
//! last line, over its side of a split diff, stacked in the order made; the file note under
//! its heading; what the cursor, the keys and the mouse do on a box; and the default,
//! `panel`, drawing exactly as before. On the long four-file fixture at 160×45.

use plannotator_tui_review::{CodeAnnotation, Side};
use ratatui::crossterm::event::{KeyCode, MouseButton, MouseEventKind};
use ratatui::style::Modifier;

use super::compose_near::{COLS, ROWS, draw, long, report, the_box};
use super::inline_support::{box_saying, cells, check_boxes, comment, row_of_spot, spot};
use super::whitespace_flow;
use super::*;
use crate::config::review::Notes;
use crate::review::{Draft, Mode};
use crate::theme::palette;

const SIZE: (u16, u16) = (COLS, ROWS);

/// The long fixture opened with `[review]` as `config` says, the cursor on new line 30 of
/// file0.py, near the bottom: new lines 8 to 30 are on screen.
fn opened(tag: &str, config: &str) -> (Fixture, Screen) {
    let fixture = long(tag, false);
    fixture.write_config(&format!("[review]\n{config}\n"));
    let mut screen = fixture.open_configured();
    draw(&mut screen, SIZE);
    screen.diff.reveal("file0.py", Side::New, 30).expect("line 30");
    draw(&mut screen, SIZE);
    (fixture, screen)
}

/// The Notes setting to `notes`, by the settings menu.
pub(super) fn set_notes(screen: &mut Screen, notes: Notes) {
    let row = crate::review::settings::ROWS
        .iter()
        .position(|row| *row == crate::review::settings::Row::Notes)
        .expect("a Notes row");
    keys(screen, ",");
    for _ in 0..row {
        keys(screen, "j");
    }
    while screen.settings.now.notes != notes {
        press(screen, KeyCode::Right);
    }
    press(screen, KeyCode::Esc);
}

/// The diff pane's cells in `buffer`.
fn pane(screen: &Screen, buffer: &ratatui::buffer::Buffer) -> Vec<String> {
    let pane = screen.geometry.diff;
    (pane.top()..pane.bottom()).map(|y| cells(buffer, pane.x, y, pane.width)).collect()
}

fn all(buffer: &ratatui::buffer::Buffer) -> String {
    (0..buffer.area.height).map(|y| cells(buffer, 0, y, buffer.area.width)).collect::<Vec<_>>().join("\n")
}

#[test]
fn the_panel_setting_keeps_no_rows_for_notes_and_draws_as_before() {
    let (_fixture, mut screen) = opened("inline-panel", "layout = \"split\"");
    let before = draw(&mut screen, SIZE);
    comment(&mut screen, "file0.py", Side::New, (9, 10), "a panel note");
    comment(&mut screen, "file0.py", Side::Old, (12, 12), "an old one");
    let noted = draw(&mut screen, SIZE);
    assert!(screen.diff.boxes_set().is_empty(), "no rows kept");
    assert!(screen.geometry.boxes.is_empty());
    assert!(!all(&noted).contains("comment \u{b7} L"), "no box:\n{}", all(&noted));
    // Inline and back: the diff is the one panel drew.
    set_notes(&mut screen, Notes::Inline);
    assert!(!screen.diff.boxes_set().is_empty());
    draw(&mut screen, SIZE);
    set_notes(&mut screen, Notes::Panel);
    let back = draw(&mut screen, SIZE);
    assert!(screen.diff.boxes_set().is_empty());
    assert_eq!(pane(&screen, &back), pane(&screen, &noted), "panel again draws what it drew");
    assert_ne!(pane(&screen, &before), pane(&screen, &noted), "the notes' marks");
}

#[test]
fn boxes_hang_under_their_notes_last_line_on_its_side_in_the_order_made() {
    let (_fixture, mut screen) = opened("inline-under", "layout = \"unified\"\nnotes = \"inline\"");
    comment(&mut screen, "file0.py", Side::New, (10, 12), "first range");
    comment(&mut screen, "file0.py", Side::New, (12, 12), "second on twelve");
    comment(&mut screen, "file0.py", Side::Old, (12, 12), "old twelve");
    comment(&mut screen, "file1.py", Side::New, (12, 12), "other file twelve");
    let buffer = draw(&mut screen, SIZE);
    check_boxes(&screen, &buffer, "unified");
    let first = box_saying(&screen, "first range").expect("drawn");
    let second = box_saying(&screen, "second on twelve").expect("drawn");
    assert_eq!(second.y, first.bottom(), "stacked under the earlier one");
    assert!(cells(&buffer, first.x, first.y, first.width).contains("comment \u{b7} L10\u{2013}12"));
    assert!(box_saying(&screen, "old twelve").is_none(), "the removed lines are further up");
    // Unified, a box starts at the code's first column, as wide as the code or 84.
    let area = screen.diff.box_areas().first().map(|a| a.area).expect("an area");
    assert_eq!((first.x, first.width), (area.x, area.width.min(84)));
    // The border is the notes' colour, the title bold.
    let corner = buffer.cell((first.x, first.y)).expect("corner");
    assert_eq!(corner.fg, palette().comment);
    // The old line's box is under the removed line 12, with the old side named.
    screen.diff.reveal("file0.py", Side::Old, 12).expect("old line 12");
    let buffer = draw(&mut screen, SIZE);
    check_boxes(&screen, &buffer, "old");
    let old = box_saying(&screen, "old twelve").expect("drawn");
    assert!(cells(&buffer, old.x, old.y, old.width).contains("comment \u{b7} L12 old"));
    // The other file's box is under its own line 12.
    screen.diff.reveal("file1.py", Side::New, 16).expect("file1 line 16");
    let buffer = draw(&mut screen, SIZE);
    check_boxes(&screen, &buffer, "file1");
    assert!(box_saying(&screen, "other file twelve").is_some());
    assert!(box_saying(&screen, "first range").is_none(), "file0 is off screen");
}

#[test]
fn split_a_box_sits_over_its_sides_half_or_across_when_that_half_is_narrow() {
    let (_fixture, mut screen) = opened("inline-split", "layout = \"split\"\nnotes = \"inline\"");
    comment(&mut screen, "file0.py", Side::Old, (12, 12), "old side note");
    comment(&mut screen, "file0.py", Side::New, (12, 12), "new side note");
    let buffer = draw(&mut screen, SIZE);
    check_boxes(&screen, &buffer, "split 50");
    let line = screen.diff.split_line().expect("split");
    let old = box_saying(&screen, "old side note").expect("old");
    let new = box_saying(&screen, "new side note").expect("new");
    assert!(old.right() <= line, "the old box over the old side: {old:?} {line}");
    assert!(new.x > line, "the new box over the new side: {new:?} {line}");
    assert_eq!(new.y, old.bottom(), "both hang under the row, in the order made");
    // 30:70 leaves the old side under 48 columns: its box runs across.
    keys(&mut screen, "((((");
    let buffer = draw(&mut screen, SIZE);
    check_boxes(&screen, &buffer, "split 30");
    let line = screen.diff.split_line().expect("split");
    assert!(line - screen.geometry.diff.x < 48, "narrow old side");
    let old = box_saying(&screen, "old side note").expect("old");
    let area = screen.diff.box_areas().first().map(|a| a.area).expect("an area");
    assert_eq!((old.x, old.width), (area.x, area.width.min(84)), "across from the old code");
    assert!(old.right() > line);
    let new = box_saying(&screen, "new side note").expect("new");
    assert!(new.x > line);
}

#[test]
fn a_file_note_sits_under_its_heading_and_notes_with_no_box_are_counted() {
    let (_fixture, mut screen) = opened("inline-file", "layout = \"unified\"\nnotes = \"inline\"");
    keys(&mut screen, "C");
    type_and_save(&mut screen, "whole file thoughts");
    keys(&mut screen, "M");
    type_and_save(&mut screen, "about everything");
    screen.diff.show_heading(0).expect("heading");
    let buffer = draw(&mut screen, SIZE);
    check_boxes(&screen, &buffer, "file note");
    let boxed = box_saying(&screen, "whole file thoughts").expect("drawn");
    assert!(cells(&buffer, boxed.x, boxed.y, boxed.width).contains("file comment"));
    let footer = all(&buffer);
    assert!(footer.contains("1 note not shown inline \u{b7} z"), "{footer}");
    open_notes(&mut screen);
    let buffer = draw(&mut screen, SIZE);
    assert!(all(&buffer).contains("1 note not shown inline") && !all(&buffer).contains("inline \u{b7} z"));
}

#[test]
fn inline_starts_without_the_panel_whatever_the_notes_and_both_keeps_its_rule() {
    let (fixture, mut screen) = opened("inline-rail", "notes = \"inline\"");
    comment(&mut screen, "file0.py", Side::New, (9, 9), "a note");
    let mut again = fixture.open_configured();
    draw(&mut again, SIZE);
    assert!(!again.rail_shown(), "inline: no panel at the start, notes or not");
    press(&mut again, KeyCode::Char('z'));
    assert!(again.rail_shown(), "z opens it");
    press(&mut again, KeyCode::Char('z'));
    assert!(!again.rail_shown());
    press(&mut again, KeyCode::Tab);
    assert!(again.rail_shown() && again.focus == crate::review::Focus::Notes, "Tab goes there");
    fixture.write_config("[review]\nnotes = \"both\"\n");
    let both = fixture.open_configured();
    assert!(both.rail_shown(), "both: the panel's rule, shown with notes");
    // Switching to inline puts the panel away; back to both brings it.
    let mut screen = both;
    draw(&mut screen, SIZE);
    set_notes(&mut screen, Notes::Inline);
    assert!(!screen.rail_shown());
    set_notes(&mut screen, Notes::Both);
    assert!(screen.rail_shown());
}

#[test]
fn on_a_box_enter_edits_it_over_the_box_delete_removes_it_and_x_stays_viewed() {
    let (_fixture, mut screen) = opened("inline-keys", "layout = \"split\"\nnotes = \"both\"");
    open_notes(&mut screen);
    comment(&mut screen, "file0.py", Side::New, (9, 9), "fix this");
    comment(&mut screen, "file0.py", Side::New, (20, 20), "other one");
    screen.diff.reveal("file0.py", Side::New, 9).expect("line 9");
    press(&mut screen, KeyCode::Char('j'));
    assert!(screen.cursor_note().is_some_and(|n| n.text.as_deref() == Some("fix this")), "on the box");
    let buffer = draw(&mut screen, SIZE);
    let boxed = box_saying(&screen, "fix this").expect("drawn");
    let corner = buffer.cell((boxed.x, boxed.y)).expect("corner");
    assert_eq!(corner.fg, palette().accent, "the cursor's box is lit");
    assert!(corner.modifier.contains(Modifier::BOLD));
    let text = all(&buffer);
    assert!(text.contains("enter/e edit") && text.contains("delete remove"), "{text}");
    // `x` is still viewed: the note stays.
    press(&mut screen, KeyCode::Char('x'));
    assert_eq!(screen.store.annotations().len(), 2);
    assert!(screen.status.as_deref().is_some_and(|s| s.contains("viewed")), "{:?}", screen.status);
    // Enter: the comment box over the note's box, with its text.
    press(&mut screen, KeyCode::Enter);
    assert!(matches!(&screen.mode, Mode::Compose(Draft::Edit(_))));
    assert_eq!(screen.compose.value(), "fix this");
    let buffer = draw(&mut screen, SIZE);
    // (`x` ticked the file in the list, which got wider: the box with it.)
    let boxed = box_saying(&screen, "fix this").expect("still drawn");
    let corner = cells(&buffer, boxed.x, boxed.y, 7);
    assert_eq!(corner, "\u{256d} edit ", "the comment box over the note's box");
    press(&mut screen, KeyCode::End);
    type_and_save(&mut screen, " now, and say why at some length so the box needs a second line of text");
    let buffer = draw(&mut screen, SIZE);
    check_boxes(&screen, &buffer, "edited");
    assert!(screen.cursor_note().is_some(), "the cursor stays on the box");
    // `e` edits too; Esc keeps it.
    press(&mut screen, KeyCode::Char('e'));
    assert!(matches!(&screen.mode, Mode::Compose(Draft::Edit(_))));
    press(&mut screen, KeyCode::Esc);
    assert_eq!(screen.mode, Mode::Browse);
    // Delete removes it, and the cursor is back on its line.
    press(&mut screen, KeyCode::Delete);
    assert_eq!(screen.store.annotations().len(), 1);
    assert_eq!(screen.status.as_deref(), Some("note removed"));
    assert_eq!(screen.diff.line_on(Side::New).expect("line"), Some(("file0.py", 9)));
    let buffer = draw(&mut screen, SIZE);
    check_boxes(&screen, &buffer, "removed");
    assert!(box_saying(&screen, "fix this").is_none());
}

#[test]
fn a_click_on_a_box_selects_its_note_and_a_drag_never_starts_or_lands_on_one() {
    let (_fixture, mut screen) = opened("inline-mouse", "layout = \"unified\"\nnotes = \"both\"");
    open_notes(&mut screen);
    comment(&mut screen, "file0.py", Side::New, (12, 12), "first");
    comment(&mut screen, "file0.py", Side::New, (20, 20), "second");
    draw(&mut screen, SIZE);
    let boxed = box_saying(&screen, "second").expect("drawn");
    let at = (boxed.x + 4, boxed.y + 1);
    report(&mut screen, MouseEventKind::Down(MouseButton::Left), at);
    assert!(screen.drag.is_none(), "no selection starts on a box");
    report(&mut screen, MouseEventKind::Up(MouseButton::Left), at);
    assert!(screen.cursor_note().is_some_and(|n| n.text.as_deref() == Some("second")));
    assert_eq!(screen.selected_note().and_then(|n| n.text.as_deref()), Some("second"), "the panel follows");
    assert_eq!(screen.diff.selection(), None);
    // A drag from line 11 down over line 12's box to 14 selects the lines only.
    let first = box_saying(&screen, "first").expect("drawn");
    let x = first.x + 10;
    let from = first.y - 2;
    report(&mut screen, MouseEventKind::Down(MouseButton::Left), (x, from));
    for y in from + 1..first.bottom() + 3 {
        report(&mut screen, MouseEventKind::Drag(MouseButton::Left), (x, y));
        draw(&mut screen, SIZE);
    }
    report(&mut screen, MouseEventKind::Up(MouseButton::Left), (x, first.bottom() + 2));
    let selected = screen.diff.selection().expect("a selection");
    assert_eq!((selected.side, selected.first), (Side::New, 11), "{selected:?}");
    assert!(selected.last >= 13, "past the box: {selected:?}");
}

#[test]
fn a_new_comment_opens_below_the_boxes_under_its_line_and_its_own_box_follows() {
    let (_fixture, mut screen) = opened("inline-compose", "layout = \"unified\"\nnotes = \"inline\"");
    comment(&mut screen, "file0.py", Side::New, (12, 12), "already here");
    screen.diff.reveal("file0.py", Side::New, 12).expect("line 12");
    let buffer = draw(&mut screen, SIZE);
    let existing = box_saying(&screen, "already here").expect("drawn");
    let before = spot(&screen);
    let row = row_of_spot(&screen, &before).expect("on screen");
    press(&mut screen, KeyCode::Char('c'));
    let buffer_open = draw(&mut screen, SIZE);
    let compose = the_box(&buffer_open, "comment on");
    assert_eq!(compose.y, existing.bottom(), "below the box, covering no code");
    drop(buffer);
    type_and_save(&mut screen, "one more");
    let buffer = draw(&mut screen, SIZE);
    check_boxes(&screen, &buffer, "saved");
    let added = box_saying(&screen, "one more").expect("its box");
    assert_eq!(added.y, existing.bottom(), "under the earlier box");
    assert_eq!(row_of_spot(&screen, &before), Some(row), "the line kept its row");
}

#[test]
fn a_review_opened_with_notes_starts_at_the_top_with_the_first_files_box() {
    let (fixture, mut screen) = opened("inline-open", "notes = \"inline\"\nlayout = \"unified\"");
    keys(&mut screen, "C");
    type_and_save(&mut screen, "about file zero");
    comment(&mut screen, "file0.py", Side::Old, (1, 1), "on line one");
    let mut again = fixture.open_configured();
    let buffer = draw(&mut again, SIZE);
    check_boxes(&again, &buffer, "opened");
    let pane = again.geometry.diff;
    let top = again.diff.row_at(pane.right() - 2, pane.y).expect("the top row");
    assert_eq!(top.kind, diffs_rs::engine::DiffRowKind::Header, "the first heading on top");
    let file = box_saying(&again, "about file zero").expect("the file's box shows");
    assert_eq!(file.y, pane.y + 1, "right under it");
    let cursor = again.diff.cursor_row().expect("a cursor");
    assert_eq!((cursor.file, cursor.old_line, cursor.part), (0, Some(1), 0), "on the first line");
    assert!(box_saying(&again, "on line one").is_some());
}

#[test]
fn a_note_from_the_agent_gets_its_box_credited_and_suggestions_show_their_code() {
    let (fixture, mut screen) = opened("inline-pi", "notes = \"inline\"\nlayout = \"unified\"");
    let file = fixture.repo().join("file0.py");
    let (note, _answer) = crate::pi::link::Note::for_test(file, None, Some(9), "check this");
    screen.take_pi_note(&note);
    let key = screen.capture.patch_key.clone();
    let patch = screen.file("file0.py").expect("file");
    let suggestion = CodeAnnotation::suggestion(patch, 11, 11, "f0_line_11 = 11 * 3".into(), None, &key);
    screen.add_note(suggestion).expect("saved");
    let buffer = draw(&mut screen, SIZE);
    check_boxes(&screen, &buffer, "agent");
    let credited = box_saying(&screen, "check this").expect("drawn");
    assert!(
        cells(&buffer, credited.x, credited.y, credited.width)
            .contains("comment \u{b7} L9 \u{b7} pi \u{b7} test")
    );
    let text = all(&buffer);
    assert!(text.contains("suggestion \u{b7} L11"), "{text}");
    assert!(
        text.contains("- f0_line_11 = 11 * 2  # changed") && text.contains("+ f0_line_11 = 11 * 3"),
        "{text}"
    );
}

#[test]
fn a_box_in_a_hunk_hidden_with_whitespace_goes_is_counted_and_comes_back() {
    let fixture = whitespace_flow::fixture("inline-ws");
    fixture.write_config("[review]\nnotes = \"inline\"\nlayout = \"unified\"\n");
    let mut screen = fixture.open_configured();
    draw(&mut screen, SIZE);
    comment(&mut screen, "ws.go", Side::New, (21, 21), "trailing spaces here");
    comment(&mut screen, "ws.go", Side::New, (6, 6), "doubled");
    let buffer = draw(&mut screen, SIZE);
    check_boxes(&screen, &buffer, "shown");
    assert!(box_saying(&screen, "trailing spaces here").is_some());
    assert!(!all(&buffer).contains("not shown inline"));
    keys(&mut screen, "W");
    let buffer = draw(&mut screen, SIZE);
    check_boxes(&screen, &buffer, "whitespace hidden");
    assert!(box_saying(&screen, "trailing spaces here").is_none(), "its hunk is a heading now");
    assert!(box_saying(&screen, "doubled").is_some(), "the other stays");
    assert!(all(&buffer).contains("1 note not shown inline \u{b7} z"), "{}", all(&buffer));
    keys(&mut screen, "W");
    let buffer = draw(&mut screen, SIZE);
    check_boxes(&screen, &buffer, "whitespace shown");
    assert!(box_saying(&screen, "trailing spaces here").is_some(), "back");
    assert!(!all(&buffer).contains("not shown inline"));
}

#[test]
fn three_hundred_notes_lay_out_once_and_draw_only_what_shows() {
    let (_fixture, mut screen) = opened("inline-many", "layout = \"split\"\nnotes = \"inline\"");
    for i in 0..300u32 {
        let file = format!("file{}.py", i % 4);
        let line = 1 + (i / 4) % 89;
        let side = if i % 3 == 0 { Side::Old } else { Side::New };
        comment(&mut screen, &file, side, (line, line), &format!("note {i} with a few words to wrap"));
    }
    draw(&mut screen, SIZE);
    assert_eq!(screen.diff.boxes_set().len(), 300);
    let started = Instant::now();
    for _ in 0..40 {
        press(&mut screen, KeyCode::Char('j'));
        let buffer = draw(&mut screen, SIZE);
        assert!(screen.geometry.boxes.len() <= usize::from(ROWS), "only the boxes on screen");
        drop(buffer);
    }
    let buffer = draw(&mut screen, SIZE);
    check_boxes(&screen, &buffer, "300 notes");
    // Debug build, 41 frames with 300 boxes set: well under a second each.
    assert!(started.elapsed() < Duration::from_secs(20), "{:?}", started.elapsed());
}

#[test]
fn the_notes_setting_comes_from_the_config_file_is_remembered_and_reset() {
    let (fixture, mut screen) = opened("inline-setting", "notes = \"both\"");
    assert_eq!(screen.settings.now.notes, Notes::Both, "the config file's");
    set_notes(&mut screen, Notes::Inline);
    let mut next = fixture.open_configured();
    draw(&mut next, SIZE);
    assert_eq!(next.settings.now.notes, Notes::Inline, "remembered over the config file");
    keys(&mut next, ",");
    for _ in 0..crate::review::settings::ROWS.len() - 1 {
        keys(&mut next, "j");
    }
    press(&mut next, KeyCode::Enter);
    assert_eq!(next.settings.now.notes, Notes::Both, "Reset: the config file's again");
    fixture.write_config("[review]\nnotes = \"sideways\"\n");
    let review = crate::config::Config::load_from(&fixture.config());
    assert!(review.is_err_and(|e| format!("{e:#}").contains("sideways")), "a bad value is named");
}
