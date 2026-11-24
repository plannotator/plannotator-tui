//! The comment box draws over the diff and moves nothing under it: the view and the cursor are
//! the same before it opens, while it is open, after a save and after Esc. While it is open
//! the mouse cannot lose the draft or scroll the lines from under it, however its reports
//! arrive: a click on the text moves the text cursor, and nothing else does anything, one
//! report a frame or a hundred before one.

use plannotator_tui_review::Side;
use ratatui::crossterm::event::{KeyCode, MouseButton, MouseEventKind};
use ratatui::layout::Rect;

use super::compose_near::{COLS, ROWS, View, drag, draw, file0, long, report, the_box, tinted, view};
use super::*;
use crate::review::{Draft, Focus, Mode};

const SIZE: (u16, u16) = (COLS, ROWS);

/// A screen with a note already saved, so the notes rail is open, and lines 20 to 22
/// selected with the toolbar's comment button clicked.
fn composing(tag: &str) -> (Fixture, Screen) {
    let fixture = long(tag, false);
    let mut screen = fixture.open();
    screen.diff.reveal("file0.py", Side::New, 3).expect("line 3");
    press(&mut screen, KeyCode::Char('c'));
    type_and_save(&mut screen, "an earlier note");
    drag(&mut screen, SIZE, Side::New, 20, 22);
    let comment = locate(&draw(&mut screen, SIZE), "comment (c)");
    report(&mut screen, MouseEventKind::Down(MouseButton::Left), comment);
    report(&mut screen, MouseEventKind::Up(MouseButton::Left), comment);
    assert_eq!(screen.mode, Mode::Compose(Draft::Comment(file0(Side::New, 20, 22))));
    (fixture, screen)
}

fn locate(buffer: &ratatui::buffer::Buffer, needle: &str) -> (u16, u16) {
    let area = buffer.area;
    for y in area.top()..area.bottom() {
        let cells: Vec<&str> = (area.left()..area.right()).map(|x| buffer[(x, y)].symbol()).collect();
        if let Some(byte) = cells.concat().find(needle) {
            let mut width = 0;
            let x = cells.iter().take_while(|c| {
                width += c.len();
                width <= byte
            });
            return (x.count() as u16, y);
        }
    }
    panic!("no {needle:?} on screen");
}

#[test]
fn opening_typing_in_saving_and_closing_the_box_move_neither_the_view_nor_the_cursor() {
    let fixture = long("still-keys", false);
    let mut screen = fixture.open();
    screen.diff.reveal("file0.py", Side::New, 3).expect("line 3");
    press(&mut screen, KeyCode::Char('c'));
    type_and_save(&mut screen, "an earlier note");
    screen.diff.reveal("file0.py", Side::New, 30).expect("line 30");
    keys(&mut screen, "vjj");
    let before: View = view(&mut screen, SIZE);
    for (close, what) in [(Some('y'), "esc and discard"), (None, "save")] {
        press(&mut screen, KeyCode::Char('c'));
        assert_eq!(view(&mut screen, SIZE), before, "{what}: open");
        keys(&mut screen, "a draft");
        assert_eq!(view(&mut screen, SIZE), before, "{what}: typing");
        match close {
            None => press(&mut screen, KeyCode::Enter),
            Some(answer) => {
                press(&mut screen, KeyCode::Esc);
                press(&mut screen, KeyCode::Char(answer));
            }
        }
        assert_eq!(screen.mode, Mode::Browse, "{what}");
        // The save makes the selection the note and clears it; the cursor and the rows stay.
        let after = view(&mut screen, SIZE);
        assert_eq!(after, before, "{what}: closed");
        assert_eq!(screen.focus, Focus::Diff, "{what}: the rail took no focus");
        if close.is_none() {
            let note = screen.store.annotations().iter().find(|n| n.text.as_deref() == Some("a draft"));
            assert_eq!(note.map(|n| (n.line_start, n.line_end)), Some((30, 32)));
        }
    }
}

#[test]
fn the_first_note_keeps_its_lines_on_their_rows_when_the_rail_opens() {
    let fixture = long("still-rail", false);
    let mut screen = fixture.open();
    drag(&mut screen, SIZE, Side::New, 20, 22);
    let rows = |screen: &mut Screen| {
        let shown = view(screen, SIZE).0;
        let pane = screen.geometry.diff;
        [20u32, 22].map(|n| {
            shown.iter().position(|r| r.as_ref().is_some_and(|r| r.2 == Some(n))).map(|i| pane.y + i as u16)
        })
    };
    let before = rows(&mut screen);
    assert_eq!(screen.geometry.rail.width, 0);
    press(&mut screen, KeyCode::Char('c'));
    type_and_save(&mut screen, "first");
    assert!(draw(&mut screen, SIZE).area.width > 0 && screen.geometry.rail.width > 0, "the rail opened");
    assert_eq!(rows(&mut screen), before);
    assert_eq!(screen.focus, Focus::Diff);
}

/// Mouse reports over everything on the screen but the box's text: presses, drags, releases,
/// moves and the wheel, on the diff, the selected lines, the file list, the notes, the Send
/// button and the box's border.
fn strays(screen: &Screen, at_box: Rect) -> Vec<(MouseEventKind, (u16, u16))> {
    let pane = screen.geometry.diff;
    let send = screen.geometry.send_button.expect("the send button");
    let cells = [
        (pane.x + 3, pane.y + 2),
        (pane.right() - 5, pane.bottom() - 1),
        (screen.geometry.files.x + 2, screen.geometry.files.y + 3),
        (screen.geometry.rail.x + 2, screen.geometry.rail.y + 2),
        (send.x, send.y),
        (at_box.x, at_box.y),
        (at_box.right() - 1, at_box.bottom() - 1),
    ];
    let kinds = [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Drag(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
        MouseEventKind::Moved,
        MouseEventKind::ScrollDown,
        MouseEventKind::ScrollUp,
        MouseEventKind::ScrollDown,
        MouseEventKind::Down(MouseButton::Right),
    ];
    (0..100).map(|i| (kinds[i % kinds.len()], cells[(i / kinds.len()) % cells.len()])).collect()
}

#[test]
fn stray_mouse_reports_keep_the_draft_and_the_view_in_a_burst_or_one_a_frame() {
    let mut ends = Vec::new();
    for framed in [false, true] {
        let (fixture, mut screen) = composing(if framed { "still-framed" } else { "still-burst" });
        keys(&mut screen, "keep me");
        let buffer = draw(&mut screen, SIZE);
        let at_box = the_box(&buffer, "comment on file0.py:20-22");
        let before = (view(&mut screen, SIZE), tinted(&buffer, screen.geometry.diff));
        for (kind, at) in strays(&screen, at_box) {
            report(&mut screen, kind, at);
            if framed {
                draw(&mut screen, SIZE);
            }
        }
        let buffer = draw(&mut screen, SIZE);
        assert_eq!(screen.mode, Mode::Compose(Draft::Comment(file0(Side::New, 20, 22))));
        assert_eq!(screen.compose.value(), "keep me");
        assert_eq!(the_box(&buffer, "comment on file0.py:20-22"), at_box, "the box stayed by its lines");
        assert_eq!((view(&mut screen, SIZE), tinted(&buffer, screen.geometry.diff)), before);
        assert!(fixture.sent().is_empty(), "the Send button took no click");
        press(&mut screen, KeyCode::Enter);
        let note = screen.store.annotations().iter().find(|n| n.text.as_deref() == Some("keep me"));
        assert_eq!(note.map(|n| (n.line_start, n.line_end)), Some((20, 22)));
        ends.push(view(&mut screen, SIZE));
    }
    assert_eq!(ends[0], ends[1], "a burst ends where reports one a frame do");
}

#[test]
fn a_click_on_the_boxs_text_moves_the_text_cursor_there() {
    let (_fixture, mut screen) = composing("still-click");
    keys(&mut screen, "hello world");
    let buffer = draw(&mut screen, SIZE);
    let (x, y) = locate(&buffer, "world");
    // A burst of reports for the click, as a terminal sends them.
    report(&mut screen, MouseEventKind::Down(MouseButton::Left), (x, y));
    for _ in 0..20 {
        report(&mut screen, MouseEventKind::Drag(MouseButton::Left), (x, y));
    }
    report(&mut screen, MouseEventKind::Up(MouseButton::Left), (x, y));
    keys(&mut screen, "big ");
    assert_eq!(screen.compose.value(), "hello big world");
}
