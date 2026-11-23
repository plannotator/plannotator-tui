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
    // Held at the limit, not fitted again: the width asked for stays set.
    assert_eq!(screen.geometry.diff.width, fit::DIFF_MIN_WIDTH, "{:?}", screen.geometry.diff);
    assert!(screen.files.chosen_width().is_some_and(|w| w > screen.geometry.files.width));
    let line = list_line(&screen);
    drag_line(&mut screen, line, 2, 5);
    frame(&mut screen, COLS, ROWS);
    assert_eq!(screen.geometry.files.width, fit::MIN_WIDTH);
    assert_eq!(screen.files.chosen_width(), Some(fit::MIN_WIDTH));
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
    // Never so wide that the diff loses its minimum or the list its narrowest: held at the
    // limit, not fitted again.
    let line = notes_line(&screen);
    drag_line(&mut screen, line, 1, 5);
    frame(&mut screen, COLS, ROWS);
    assert_eq!(screen.geometry.diff.width, fit::DIFF_MIN_WIDTH, "{:?}", screen.geometry.diff);
    assert_eq!(screen.geometry.files.width, fit::MIN_WIDTH, "the list still shows");
    assert!(screen.geometry.rail.width > before + 12);
}

#[test]
fn a_double_click_is_two_clicks_on_one_cell_with_no_move() {
    let (_fixture, mut screen) = open("divider-double");
    let line = list_line(&screen);
    drag_line(&mut screen, line, line + 6, 5);
    frame(&mut screen, COLS, ROWS);
    let set = screen.files.chosen_width();
    assert!(set.is_some());
    let line = list_line(&screen);
    let click = |screen: &mut Screen, down: u16, up: u16| {
        mouse(screen, MouseEventKind::Down(MouseButton::Left), down, 5);
        mouse(screen, MouseEventKind::Up(MouseButton::Left), up, 5);
    };
    // Two drags in a row, each ending where it started.
    for _ in 0..2 {
        mouse(&mut screen, MouseEventKind::Down(MouseButton::Left), line, 5);
        mouse(&mut screen, MouseEventKind::Drag(MouseButton::Left), line + 3, 5);
        mouse(&mut screen, MouseEventKind::Drag(MouseButton::Left), line, 5);
        mouse(&mut screen, MouseEventKind::Up(MouseButton::Left), line, 5);
    }
    assert_eq!(screen.files.chosen_width(), set, "two drags are no double click");
    // A click, then a press on the line let go on the next row down.
    click(&mut screen, line, line);
    mouse(&mut screen, MouseEventKind::Down(MouseButton::Left), line, 5);
    mouse(&mut screen, MouseEventKind::Up(MouseButton::Left), line, 6);
    assert_eq!(screen.files.chosen_width(), set, "the second release left the cell");
    // A click on one row, then one on another.
    click(&mut screen, line, line);
    mouse(&mut screen, MouseEventKind::Down(MouseButton::Left), line, 7);
    mouse(&mut screen, MouseEventKind::Up(MouseButton::Left), line, 7);
    assert_eq!(screen.files.chosen_width(), set, "two cells");
    click(&mut screen, line, line);
    click(&mut screen, line, line);
    assert_eq!(screen.files.chosen_width(), None, "a double click fits the list again");
}

#[test]
fn a_width_chosen_stays_the_choice_through_a_narrow_window() {
    let (_fixture, mut screen) = open("divider-narrow");
    let line = list_line(&screen);
    drag_line(&mut screen, line, line + 20, 5);
    let line = notes_line(&screen);
    drag_line(&mut screen, line, line - 20, 5);
    frame(&mut screen, COLS, ROWS);
    let wide = (screen.geometry.files.width, screen.geometry.rail.width);
    // The window narrows for a moment: both panes give room, the choice stays.
    frame(&mut screen, 120, ROWS);
    assert!(screen.geometry.files.width < wide.0 || screen.geometry.rail.width < wide.1);
    frame(&mut screen, COLS, ROWS);
    assert_eq!((screen.geometry.files.width, screen.geometry.rail.width), wide, "back as chosen");
    // A key pressed while narrow steps from the width shown.
    let cols = (100..COLS)
        .rev()
        .find(|&cols| {
            frame(&mut screen, cols, ROWS);
            screen.geometry.files.width + 8 < wide.0
        })
        .expect("a window narrow enough");
    let narrow = screen.geometry.files.width;
    keys(&mut screen, "<");
    frame(&mut screen, cols, ROWS);
    assert_eq!(screen.geometry.files.width, narrow - 4);
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
    // Off the line, and the window left: unlit.
    mouse(&mut screen, MouseEventKind::Moved, line + 4, 5);
    assert!(lit(&mut screen, line + 4));
    screen.handle_event(Event::FocusLost).expect("focus lost");
    assert!(!lit(&mut screen, line + 4), "the window lost focus");
}
