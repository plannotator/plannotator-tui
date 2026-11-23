//! The lines between the panes: dragging the one right of the file list or left of the
//! notes resizes that pane, within the same limits as `<`/`>` and `{`/`}`; the width stays
//! through filters and reloads; a double click fits the pane again; and a press on a line
//! never moves the diff's cursor or starts a selection.

use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use super::*;
use crate::review::files::fit;

const COLS: u16 = 160;
const ROWS: u16 = 30;

fn mouse(screen: &mut Screen, kind: MouseEventKind, column: u16, row: u16) {
    let event = MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE };
    screen.handle_event(Event::Mouse(event)).expect("mouse");
}

/// Press on `from`'s column and row, drag to `to`'s column, let go.
fn drag_line(screen: &mut Screen, from: u16, to: u16, row: u16) {
    mouse(screen, MouseEventKind::Down(MouseButton::Left), from, row);
    mouse(screen, MouseEventKind::Drag(MouseButton::Left), to, row);
    mouse(screen, MouseEventKind::Up(MouseButton::Left), to, row);
}

/// Two changed files, so the list shows, and a note, so the notes panel shows.
fn open(tag: &str) -> (Fixture, Screen) {
    let fixture = Fixture::new(tag);
    fixture.write("notes.txt", "changed\n");
    let mut screen = fixture.open();
    at(&mut screen, 9);
    keys(&mut screen, "c");
    type_and_save(&mut screen, "name the total");
    frame(&mut screen, COLS, ROWS);
    (fixture, screen)
}

fn list_line(screen: &Screen) -> u16 {
    screen.geometry.files.right()
}

fn notes_line(screen: &Screen) -> u16 {
    screen.geometry.rail.x - 1
}

#[test]
fn dragging_the_list_line_resizes_the_list_and_moves_nothing_in_the_diff() {
    let (_fixture, mut screen) = open("divider-files");
    let before = screen.geometry.files.width;
    let cursor = screen.diff.cursor().expect("cursor");
    let line = list_line(&screen);
    drag_line(&mut screen, line, line + 10, 5);
    frame(&mut screen, COLS, ROWS);
    assert_eq!(screen.geometry.files.width, before + 10);
    assert_eq!(screen.diff.cursor().expect("cursor"), cursor, "the cursor stays");
    assert!(screen.diff.selection().is_none());
    assert!(screen.drag.is_none());
    // `<` goes on from the dragged width.
    keys(&mut screen, "<");
    frame(&mut screen, COLS, ROWS);
    assert_eq!(screen.geometry.files.width, before + 6);
}

#[test]
fn the_list_line_stops_where_the_diff_would_lose_its_minimum_or_the_list_its_narrowest() {
    let (_fixture, mut screen) = open("divider-limits");
    let line = list_line(&screen);
    drag_line(&mut screen, line, COLS - 1, 5);
    frame(&mut screen, COLS, ROWS);
    assert!(screen.geometry.diff.width >= fit::DIFF_MIN_WIDTH, "{:?}", screen.geometry.diff);
    let line = list_line(&screen);
    drag_line(&mut screen, line, 2, 5);
    frame(&mut screen, COLS, ROWS);
    assert_eq!(screen.geometry.files.width, fit::MIN_WIDTH);
}

#[test]
fn dragging_the_notes_line_resizes_the_notes_and_the_keys_go_on_from_there() {
    let (_fixture, mut screen) = open("divider-notes");
    let before = screen.geometry.rail.width;
    let line = notes_line(&screen);
    drag_line(&mut screen, line, line - 8, 5);
    frame(&mut screen, COLS, ROWS);
    assert_eq!(screen.geometry.rail.width, before + 8);
    keys(&mut screen, "}");
    frame(&mut screen, COLS, ROWS);
    assert_eq!(screen.geometry.rail.width, before + 4);
    keys(&mut screen, "{{");
    frame(&mut screen, COLS, ROWS);
    assert_eq!(screen.geometry.rail.width, before + 12);
    // Never so wide that the diff loses its minimum.
    let line = notes_line(&screen);
    drag_line(&mut screen, line, 1, 5);
    frame(&mut screen, COLS, ROWS);
    assert!(screen.geometry.diff.width >= fit::DIFF_MIN_WIDTH, "{:?}", screen.geometry.diff);
    assert!(screen.geometry.files.width >= fit::MIN_WIDTH, "the list still shows");
}

#[test]
fn a_width_set_by_hand_stays_through_a_filter_and_a_reload_until_a_double_click() {
    let (fixture, mut screen) = open("divider-stays");
    let (files, rail) = (screen.geometry.files.width, screen.geometry.rail.width);
    let line = list_line(&screen);
    drag_line(&mut screen, line, line + 12, 5);
    let line = notes_line(&screen);
    drag_line(&mut screen, line, line - 6, 5);
    frame(&mut screen, COLS, ROWS);
    let set = (files + 12, rail + 6);
    assert_eq!((screen.geometry.files.width, screen.geometry.rail.width), set);

    // A name filter, then cleared; a reload after a change on disk.
    keys(&mut screen, "/app");
    press(&mut screen, KeyCode::Enter);
    frame(&mut screen, COLS, ROWS);
    assert_eq!((screen.geometry.files.width, screen.geometry.rail.width), set, "filtered");
    press(&mut screen, KeyCode::Esc);
    fixture.write("a-much-longer-file-name-than-before.txt", "new\n");
    keys(&mut screen, "r");
    frame(&mut screen, COLS, ROWS);
    assert_eq!((screen.geometry.files.width, screen.geometry.rail.width), set, "reloaded");

    // A double click on each line fits its pane again.
    for line in [list_line(&screen), notes_line(&screen)] {
        mouse(&mut screen, MouseEventKind::Down(MouseButton::Left), line, 5);
        mouse(&mut screen, MouseEventKind::Up(MouseButton::Left), line, 5);
        mouse(&mut screen, MouseEventKind::Down(MouseButton::Left), line, 5);
        mouse(&mut screen, MouseEventKind::Up(MouseButton::Left), line, 5);
        frame(&mut screen, COLS, ROWS);
    }
    assert_eq!(screen.files.chosen_width(), None);
    assert_eq!(screen.geometry.rail.width, rail);
}

#[test]
fn a_line_lights_up_under_the_pointer_and_while_dragged_in_colour_only() {
    let (_fixture, mut screen) = open("divider-hover");
    let line = list_line(&screen);
    let lit = |screen: &mut Screen, x: u16| {
        let mut terminal = Terminal::new(TestBackend::new(COLS, ROWS)).expect("terminal");
        terminal.draw(|f| screen.draw(f).expect("draw")).expect("draw");
        let cell = &terminal.backend().buffer()[(x, 5)];
        // The same glyph and weight lit or not: only the colour changes.
        assert_eq!(cell.symbol(), "\u{2502}");
        assert_eq!(cell.modifier, ratatui::style::Modifier::empty());
        cell.fg == crate::theme::palette().accent
    };
    assert!(!lit(&mut screen, line));
    let notes = notes_line(&screen);
    assert!(!lit(&mut screen, notes), "the notes line looks like the list's");
    mouse(&mut screen, MouseEventKind::Moved, line, 5);
    assert!(lit(&mut screen, line), "under the pointer");
    mouse(&mut screen, MouseEventKind::Moved, line + 20, 5);
    assert!(!lit(&mut screen, line), "the pointer left");
    mouse(&mut screen, MouseEventKind::Down(MouseButton::Left), line, 5);
    mouse(&mut screen, MouseEventKind::Drag(MouseButton::Left), line + 4, 5);
    assert!(lit(&mut screen, line + 4), "while dragged");
    mouse(&mut screen, MouseEventKind::Up(MouseButton::Left), line + 4, 5);
}
