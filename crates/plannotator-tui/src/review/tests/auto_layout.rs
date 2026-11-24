//! `layout = "auto"` follows the terminal's width, not the diff pane's: saving the first note,
//! opening the notes panel then, closing or opening the file list or the notes, and dragging
//! the lines between the panes never flip it. Resizing the terminal across the threshold
//! does, and a layout chosen with `u` holds at any width.

use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use super::*;
use crate::review::settings::Row;

const ROWS: u16 = 40;

/// A review of two changed files (`listed`, so the file list shows) or of one.
fn review(tag: &str, listed: bool) -> (Fixture, Screen) {
    let fixture = Fixture::new(tag);
    if listed {
        fixture.write("notes.txt", "changed\n");
    }
    let screen = fixture.open();
    (fixture, screen)
}

/// Split or not, drawn at `cols`, and the settings menu's layout row says the same.
fn split_at(screen: &mut Screen, cols: u16) -> bool {
    frame(screen, cols, ROWS);
    let split = screen.diff.split();
    let said = if split { "auto \u{b7} split now" } else { "auto \u{b7} unified now" };
    assert_eq!(screen.setting_value(Row::Layout), said, "the menu says what is drawn");
    split
}

fn mouse(screen: &mut Screen, kind: MouseEventKind, column: u16, row: u16) {
    let event = MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE };
    screen.handle_event(Event::Mouse(event)).expect("mouse");
}

fn drag_line(screen: &mut Screen, from: u16, to: u16) {
    mouse(screen, MouseEventKind::Down(MouseButton::Left), from, 5);
    mouse(screen, MouseEventKind::Drag(MouseButton::Left), to, 5);
    mouse(screen, MouseEventKind::Up(MouseButton::Left), to, 5);
}

#[test]
fn saving_the_first_note_never_flips_an_auto_layout() {
    for listed in [true, false] {
        for cols in [120, 125, 140, 160] {
            let (_fixture, mut screen) = review(&format!("auto-note-{cols}-{listed}"), listed);
            let before = split_at(&mut screen, cols);
            assert_eq!(screen.geometry.rail.width, 0, "no notes yet");
            // Split as the review always opened: 120 columns for the diff beside the list.
            assert_eq!(before, !listed || cols >= 141, "{cols} columns, list {listed}");
            at(&mut screen, 9);
            press(&mut screen, KeyCode::Char('c'));
            frame(&mut screen, cols, ROWS);
            type_and_save(&mut screen, "name the total");
            let after = split_at(&mut screen, cols);
            assert_eq!(screen.geometry.rail.width, 0, "{cols}: a saved note opens no panel");
            assert_eq!(after, before, "{cols} columns, list {listed}: the layout held");
            open_notes(&mut screen);
            let opened = split_at(&mut screen, cols);
            assert!(screen.geometry.rail.width > 0, "{cols}: the notes panel opened by hand");
            assert_eq!(opened, before, "{cols} columns, list {listed}: the layout held");
        }
    }
}

#[test]
fn closing_opening_or_dragging_the_side_panels_never_flips_an_auto_layout() {
    for cols in [140, 160] {
        let (_fixture, mut screen) = review(&format!("auto-panels-{cols}"), true);
        at(&mut screen, 9);
        press(&mut screen, KeyCode::Char('c'));
        type_and_save(&mut screen, "name the total");
        open_notes(&mut screen);
        let split = split_at(&mut screen, cols);
        assert_eq!(split, cols == 160);
        let check = |screen: &mut Screen, what: &str| {
            assert_eq!(split_at(screen, cols), split, "{cols} columns, {what}");
        };
        // `f` moves to the list, `f` again closes it, and a third opens it.
        for (key, what) in [('f', "list focused"), ('f', "list closed"), ('f', "list open")] {
            press(&mut screen, KeyCode::Char(key));
            check(&mut screen, what);
        }
        press(&mut screen, KeyCode::Esc);
        assert!(screen.geometry.files.width > 0 && screen.geometry.rail.width > 0);
        press(&mut screen, KeyCode::Char('z'));
        check(&mut screen, "notes hidden");
        assert_eq!(screen.geometry.rail.width, 0);
        press(&mut screen, KeyCode::Char('z'));
        check(&mut screen, "notes shown");
        let list = screen.geometry.files.right();
        drag_line(&mut screen, list, cols - 1);
        check(&mut screen, "list dragged wide");
        assert_eq!(screen.geometry.diff.width, crate::review::files::fit::DIFF_MIN_WIDTH);
        let list = screen.geometry.files.right();
        drag_line(&mut screen, list, 2);
        check(&mut screen, "list dragged narrow");
        let notes = screen.geometry.rail.x - 1;
        drag_line(&mut screen, notes, 2);
        check(&mut screen, "notes dragged wide");
        let notes = screen.geometry.rail.x - 1;
        drag_line(&mut screen, notes, cols - 1);
        check(&mut screen, "notes dragged narrow");
    }
}

#[test]
fn resizing_the_terminal_across_the_threshold_switches_auto_and_never_a_chosen_layout() {
    let (_fixture, mut screen) = review("auto-resize", true);
    let shown: Vec<bool> = [160, 140, 141, 120, 200].map(|cols| split_at(&mut screen, cols)).into();
    assert_eq!(shown, [true, false, true, false, true]);
    let (_one, mut one) = review("auto-resize-one", false);
    assert_eq!((split_at(&mut one, 119), split_at(&mut one, 120)), (false, true));

    // Chosen with `u`: unified at any width, then split at any width.
    press(&mut screen, KeyCode::Char('u'));
    for cols in [200, 160, 141] {
        frame(&mut screen, cols, ROWS);
        assert!(!screen.diff.split(), "unified as chosen at {cols}");
    }
    press(&mut screen, KeyCode::Char('u'));
    for cols in [140, 100, 160] {
        frame(&mut screen, cols, ROWS);
        assert!(screen.diff.split(), "split as chosen at {cols}");
    }
}
