//! The mouse over the file list: the wheel scrolls the list and leaves the diff, a click on
//! a file shows it in the diff, a click on a folder closes or opens it. None of it reaches
//! the viewer, whose own list is hidden.

use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use super::viewed_flow::entry;
use super::*;
use crate::review::Focus;

const COLS: u16 = 160;
const ROWS: u16 = 24;

/// Thirty changed files in three folders, so the list is longer than the screen.
fn many(tag: &str) -> Fixture {
    let fixture = Fixture::new(tag);
    for folder in ["alpha", "beta", "gamma"] {
        std::fs::create_dir_all(fixture.repo().join(folder)).expect("folder");
        for n in 0..10 {
            fixture.write(&format!("{folder}/file-{n:02}.ts"), &format!("export const n = {n};\n"));
        }
    }
    fixture
}

fn mouse(screen: &mut Screen, kind: MouseEventKind, column: u16, row: u16) {
    let event = MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE };
    screen.handle_event(Event::Mouse(event)).expect("mouse");
}

/// The screen row whose list part shows `name`.
fn list_row(rows: &[String], name: &str) -> u16 {
    let at = rows.iter().position(|row| row.split('\u{2502}').next().is_some_and(|left| left.contains(name)));
    u16::try_from(at.unwrap_or_else(|| panic!("{name} is not in the list: {rows:#?}"))).expect("row")
}

#[test]
fn the_wheel_over_the_list_scrolls_the_list_and_not_the_diff() {
    let fixture = many("files-wheel");
    let mut screen = fixture.open();
    let rows = frame(&mut screen, COLS, ROWS);
    assert!(rows.iter().any(|r| entry_has(r, "alpha/")), "{rows:#?}");
    let diff_before: Vec<String> = rows.iter().map(|r| diff_side(&screen, r)).collect();
    mouse(&mut screen, MouseEventKind::ScrollDown, 3, 5);
    mouse(&mut screen, MouseEventKind::ScrollDown, 3, 5);
    let rows = frame(&mut screen, COLS, ROWS);
    assert!(!rows.iter().any(|r| entry_has(r, "alpha/")), "the list scrolled: {rows:#?}");
    let diff_after: Vec<String> = rows.iter().map(|r| diff_side(&screen, r)).collect();
    assert_eq!(diff_before, diff_after, "the diff stays");
    mouse(&mut screen, MouseEventKind::ScrollUp, 3, 5);
    mouse(&mut screen, MouseEventKind::ScrollUp, 3, 5);
    assert!(frame(&mut screen, COLS, ROWS).iter().any(|r| entry_has(r, "alpha/")));
}

#[test]
fn a_click_on_a_file_shows_it_in_the_diff_and_selects_nothing() {
    let fixture = many("files-click");
    let mut screen = fixture.open();
    let rows = frame(&mut screen, COLS, ROWS);
    let row = list_row(&rows, "file-03.ts");
    mouse(&mut screen, MouseEventKind::Down(MouseButton::Left), 6, row);
    mouse(&mut screen, MouseEventKind::Up(MouseButton::Left), 6, row);
    assert_eq!(screen.focus, Focus::Files);
    assert_eq!(screen.files.current.as_deref(), Some("alpha/file-03.ts"));
    assert_eq!(screen.file_under_cursor().expect("cursor").as_deref(), Some("alpha/file-03.ts"));
    assert!(screen.diff.selection().is_none());
    let rows = frame(&mut screen, COLS, ROWS);
    assert!(rows.iter().any(|r| diff_side(&screen, r).contains("alpha/file-03.ts")), "{rows:#?}");
}

#[test]
fn a_click_on_a_folder_closes_it_and_another_opens_it() {
    let fixture = many("files-fold");
    let mut screen = fixture.open();
    let rows = frame(&mut screen, COLS, ROWS);
    let row = list_row(&rows, "alpha/");
    mouse(&mut screen, MouseEventKind::Down(MouseButton::Left), 3, row);
    let rows = frame(&mut screen, COLS, ROWS);
    assert!(entry(&rows, "alpha/").contains('\u{25b8}'), "closed: {rows:#?}");
    let next = rows.get(usize::from(row) + 1).expect("a row");
    assert!(entry_has(next, "beta/"), "its files are hidden: {rows:#?}");
    mouse(&mut screen, MouseEventKind::Down(MouseButton::Left), 3, row);
    assert!(entry(&frame(&mut screen, COLS, ROWS), "alpha/").contains('\u{25be}'), "open again");
}

/// Whether the list's part of `row` names `name`.
fn entry_has(row: &str, name: &str) -> bool {
    row.split('\u{2502}').next().is_some_and(|left| left.contains(name))
}
