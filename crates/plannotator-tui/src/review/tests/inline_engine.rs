//! What the viewer keeps still around the notes' boxes, as a reviewer met it in the real
//! program: a box edited shorter, after the cursor came up onto it or a click on its bottom
//! border, keeps its line's row; so does a resize with the cursor on a wrapped box's last
//! row. The cursor keeps its side past a box of the other side, and switching the Notes
//! setting round near the top of the diff ends where it started.

use plannotator_tui_review::Side;
use ratatui::crossterm::event::{KeyCode, MouseButton, MouseEventKind};

use super::compose_near::{draw, report, view};
use super::inline_flow::{SIZE, opened, set_notes};
use super::inline_support::{box_saying, check_boxes, comment, row_of_spot};
use super::*;
use crate::config::review::Notes;
use crate::review::{Draft, Mode};

/// The screen row of new line `line` of file0.py (a changed line) in the last frame.
fn row_of_new(screen: &Screen, line: u32) -> Option<u16> {
    let spot = ("file0.py".to_owned(), None, Some(line), "Code".to_owned());
    row_of_spot(screen, &spot)
}

const LONG: &str = "a note long enough to take several rows of its box, so that saving it shorter \
                    gives back rows under its line, which must come back without moving the line";

/// In the comment box: delete everything typed but `keep` characters, by keys, and save.
fn shorten_and_save(screen: &mut Screen, keep: usize) {
    press(screen, KeyCode::End);
    let len = screen.compose.value().chars().count();
    for _ in keep..len {
        press(screen, KeyCode::Backspace);
    }
    press(screen, KeyCode::Enter);
    assert_eq!(screen.mode, Mode::Browse, "saved");
}

#[test]
fn a_box_edited_shorter_after_coming_up_onto_it_keeps_its_line_still() {
    let (_fixture, mut screen) = opened("engine-up", "layout = \"unified\"\nnotes = \"inline\"");
    comment(&mut screen, "file0.py", Side::New, (20, 20), LONG);
    draw(&mut screen, SIZE);
    screen.diff.reveal("file0.py", Side::New, 21).expect("line 21");
    draw(&mut screen, SIZE);
    press(&mut screen, KeyCode::Char('k'));
    assert!(screen.cursor_note().is_some(), "up onto the box");
    draw(&mut screen, SIZE);
    let line = row_of_new(&screen, 20).expect("its line on screen");
    press(&mut screen, KeyCode::Enter);
    shorten_and_save(&mut screen, 6);
    let buffer = draw(&mut screen, SIZE);
    check_boxes(&screen, &buffer, "shorter");
    assert_eq!(row_of_new(&screen, 20), Some(line), "its line did not move");
}

#[test]
fn a_box_edited_shorter_after_a_click_on_its_bottom_border_keeps_its_line_still() {
    let (_fixture, mut screen) = opened("engine-click", "layout = \"unified\"\nnotes = \"inline\"");
    comment(&mut screen, "file0.py", Side::New, (20, 20), LONG);
    screen.diff.reveal("file0.py", Side::New, 20).expect("line 20");
    draw(&mut screen, SIZE);
    let boxed = box_saying(&screen, LONG).expect("drawn");
    let at = (boxed.x + 3, boxed.bottom() - 1);
    report(&mut screen, MouseEventKind::Down(MouseButton::Left), at);
    report(&mut screen, MouseEventKind::Up(MouseButton::Left), at);
    assert!(screen.cursor_note().is_some(), "on the box");
    draw(&mut screen, SIZE);
    let line = row_of_new(&screen, 20).expect("its line on screen");
    press(&mut screen, KeyCode::Enter);
    shorten_and_save(&mut screen, 6);
    let buffer = draw(&mut screen, SIZE);
    check_boxes(&screen, &buffer, "shorter");
    assert_eq!(row_of_new(&screen, 20), Some(line), "its line did not move");
}

#[test]
fn a_resize_with_the_cursor_on_a_wrapped_boxs_last_row_keeps_its_line_still() {
    let (_fixture, mut screen) = opened("engine-resize", "layout = \"unified\"\nnotes = \"inline\"");
    comment(&mut screen, "file0.py", Side::New, (20, 20), LONG);
    let narrow = (80, SIZE.1);
    draw(&mut screen, narrow);
    screen.diff.reveal("file0.py", Side::New, 20).expect("line 20");
    draw(&mut screen, narrow);
    let boxed = box_saying(&screen, LONG).expect("drawn");
    assert!(boxed.height > 3, "wrapped: {boxed:?}");
    let at = (boxed.x + 3, boxed.bottom() - 1);
    report(&mut screen, MouseEventKind::Down(MouseButton::Left), at);
    report(&mut screen, MouseEventKind::Up(MouseButton::Left), at);
    assert!(screen.cursor_note().is_some(), "on the box's last row");
    draw(&mut screen, narrow);
    let line = row_of_new(&screen, 20).expect("its line on screen");
    let buffer = draw(&mut screen, SIZE);
    check_boxes(&screen, &buffer, "wide");
    assert_eq!(row_of_new(&screen, 20), Some(line), "its line did not move");
}

#[test]
fn a_resize_after_a_box_on_the_cursor_shrank_keeps_its_line_still() {
    let (_fixture, mut screen) = opened("engine-shrunk", "layout = \"unified\"\nnotes = \"inline\"");
    comment(&mut screen, "file0.py", Side::New, (20, 20), LONG);
    let narrow = (80, SIZE.1);
    draw(&mut screen, narrow);
    screen.diff.reveal("file0.py", Side::New, 20).expect("line 20");
    draw(&mut screen, narrow);
    let boxed = box_saying(&screen, LONG).expect("drawn");
    let at = (boxed.x + 3, boxed.bottom() - 1);
    report(&mut screen, MouseEventKind::Down(MouseButton::Left), at);
    report(&mut screen, MouseEventKind::Up(MouseButton::Left), at);
    assert!(screen.cursor_note().is_some(), "on the box's last row");
    draw(&mut screen, narrow);
    let line = row_of_new(&screen, 20).expect("its line on screen");
    // One row shorter, at the same width: the line stays.
    let id = screen.store.annotations().first().map(|n| n.id.clone()).expect("the note");
    let shorter = LONG.split_once(", which").map(|(head, _)| head.to_owned()).expect("a comma");
    screen
        .store
        .edit(&id, |note| {
            note.text = Some(shorter);
            true
        })
        .expect("edited");
    draw(&mut screen, narrow);
    assert_eq!(row_of_new(&screen, 20), Some(line), "the shorter box kept its line");
    // Two columns wider, the box as tall: nothing to move for.
    let buffer = draw(&mut screen, (82, SIZE.1));
    check_boxes(&screen, &buffer, "82");
    assert_eq!(row_of_new(&screen, 20), Some(line), "its line did not move");
}

#[test]
fn the_cursor_keeps_its_side_past_a_box_of_the_other_side() {
    let (_fixture, mut screen) = opened("engine-side", "layout = \"split\"\nnotes = \"inline\"");
    comment(&mut screen, "file0.py", Side::Old, (32, 32), "only on the old side");
    screen.diff.reveal("file0.py", Side::New, 32).expect("new 32");
    draw(&mut screen, SIZE);
    assert_eq!(screen.diff.line_on(Side::New).expect("line"), Some(("file0.py", 32)));
    press(&mut screen, KeyCode::Down);
    press(&mut screen, KeyCode::Down);
    draw(&mut screen, SIZE);
    keys(&mut screen, "c");
    let Mode::Compose(Draft::Comment(lines)) = screen.mode.clone() else {
        panic!("a comment: {:?}", screen.status)
    };
    assert_eq!((lines.side, lines.first), (Side::New, 33), "the new side of 33");
    press(&mut screen, KeyCode::Esc);
    press(&mut screen, KeyCode::Up);
    press(&mut screen, KeyCode::Up);
    draw(&mut screen, SIZE);
    keys(&mut screen, "c");
    let Mode::Compose(Draft::Comment(lines)) = screen.mode.clone() else { panic!("a comment") };
    assert_eq!((lines.side, lines.first), (Side::New, 32), "back on new 32");
    press(&mut screen, KeyCode::Esc);
}

#[test]
fn the_notes_setting_round_and_back_near_the_top_ends_where_it_started() {
    let (_fixture, mut screen) = opened("engine-round", "layout = \"unified\"\nnotes = \"inline\"");
    comment(&mut screen, "file0.py", Side::New, (2, 2), "near the top");
    comment(&mut screen, "file0.py", Side::New, (5, 6), "a little lower");
    screen.diff.reveal("file0.py", Side::New, 4).expect("line 4");
    let mut seen = vec![(Notes::Inline, view(&mut screen, SIZE))];
    for notes in [Notes::Panel, Notes::Inline, Notes::Both, Notes::Panel] {
        set_notes(&mut screen, notes);
        let now = view(&mut screen, SIZE);
        if let Some((_, before)) = seen.iter().find(|(seen, _)| *seen == notes) {
            assert_eq!(&now, before, "{notes:?} again: where it was");
        }
        seen.push((notes, now));
    }
}
