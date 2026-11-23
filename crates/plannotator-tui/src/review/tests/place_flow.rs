//! Which row and file the diff's cursor is on, kept without moving the viewer: on every row,
//! after rows, pages, hunk and file jumps, clicks and the wheel, split and unified, with a
//! selection open; and finding it never moves the cursor, the selection or the view.

use diffs_rs::engine::Command;
use ratatui::crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind};

use super::*;
use crate::review::diff::Lines;

/// Binary files, a file with two hunks and a gap before and between them, a file whose last
/// line has no newline, and a text file last, so every row has a line somewhere below it.
pub(super) fn mixed(tag: &str) -> Fixture {
    let fixture = Fixture::new(tag);
    let lines = (1..=40).fold(String::new(), |text, n| text + "line " + &n.to_string() + "\n");
    fixture.write("nn.txt", &lines);
    fixture.write("nonl.txt", "first\nlast");
    fixture.write("zz.txt", "one\ntwo\n");
    fixture.git(&["add", "nn.txt", "nonl.txt", "zz.txt"]);
    fixture.git(&["-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-qm", "more"]);
    fixture.write(
        "nn.txt",
        &lines.replace("line 12\n", "line twelve\n").replace("line 33\n", "line 33a\nline 33b\n"),
    );
    fixture.write("nonl.txt", "first\nchanged");
    fixture.write("zz.txt", "one\ntwo\nthree\n");
    for name in ["aa.bin", "mm.bin"] {
        std::fs::write(fixture.repo().join(name), [0u8, name.len() as u8, 0, 255]).expect("binary");
    }
    fixture
}

/// The diff's part of each row of a frame.
fn diff_rows(screen: &mut Screen, cols: u16, rows: u16) -> Vec<String> {
    let shown = frame(screen, cols, rows);
    shown.iter().map(|row| diff_side(screen, row)).collect()
}

fn mouse(screen: &mut Screen, kind: MouseEventKind, column: u16, row: u16) {
    let event = MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE };
    screen.handle_event(Event::Mouse(event)).expect("mouse");
}

/// The row the screen keeps, the row found by moving, and the file the list shows: all three
/// must agree.
fn check(screen: &mut Screen, what: &str) {
    let tracked = screen.diff.tracked();
    let probed = screen.diff.probe();
    assert!(probed.is_some(), "{what}: the probe found the cursor");
    assert_eq!(tracked, probed, "{what}");
    let file = probed.map(|(file, _)| screen.diff.paths()[file].clone());
    assert_eq!(screen.files.current, file, "{what}: the list's file");
}

/// Up to the first row: the first file's heading.
fn to_top(screen: &mut Screen) {
    keys(screen, "g");
    for _ in 0..12 {
        keys(screen, "k");
    }
}

#[test]
fn every_row_down_and_up_is_known_in_both_layouts() {
    let fixture = mixed("place-walk");
    for (cols, split) in [(110, false), (170, true)] {
        let mut screen = fixture.open();
        frame(&mut screen, cols, 120);
        assert_eq!(screen.diff.split(), split);
        assert_eq!(screen.diff.paths(), ["aa.bin", "app.py", "mm.bin", "nn.txt", "nonl.txt", "zz.txt"]);
        to_top(&mut screen);
        check(&mut screen, "the top");
        for n in 0..80 {
            keys(&mut screen, "j");
            check(&mut screen, &format!("{cols}: j {n}"));
        }
        for n in 0..80 {
            keys(&mut screen, "k");
            check(&mut screen, &format!("{cols}: k {n}"));
        }
        // Hunk and file jumps, from wherever the rows left the cursor.
        for (key, times) in [("]", 9), ("[", 9), ("n", 6), ("N", 6)] {
            for n in 0..times {
                keys(&mut screen, key);
                check(&mut screen, &format!("{cols}: {key} {n}"));
                keys(&mut screen, "j");
                check(&mut screen, &format!("{cols}: {key} {n} then j"));
            }
        }
        // `u` lays the rows out the other way.
        keys(&mut screen, "nj");
        keys(&mut screen, "u");
        check(&mut screen, &format!("{cols}: u"));
        keys(&mut screen, "jj");
        check(&mut screen, &format!("{cols}: u then jj"));
    }
}

#[test]
fn pages_wheels_and_clicks_land_on_known_rows() {
    let fixture = mixed("place-pages");
    let mut screen = fixture.open();
    let (cols, rows) = (110, 14);
    frame(&mut screen, cols, rows);
    to_top(&mut screen);
    for n in 0..8 {
        press(&mut screen, KeyCode::PageDown);
        check(&mut screen, &format!("page down {n}"));
        frame(&mut screen, cols, rows);
    }
    for n in 0..8 {
        press(&mut screen, KeyCode::PageUp);
        check(&mut screen, &format!("page up {n}"));
        frame(&mut screen, cols, rows);
    }
    // The wheel pushes the cursor along the top, then along the bottom.
    let x = screen.geometry.diff.x + 20;
    let y = screen.geometry.diff.y + 2;
    for n in 0..20 {
        mouse(&mut screen, MouseEventKind::ScrollDown, x, y);
        check(&mut screen, &format!("wheel down {n}"));
        frame(&mut screen, cols, rows);
    }
    for n in 0..20 {
        mouse(&mut screen, MouseEventKind::ScrollUp, x, y);
        check(&mut screen, &format!("wheel up {n}"));
        frame(&mut screen, cols, rows);
    }
    // A click on every row of the diff, a few screens down.
    for page in 0..5 {
        frame(&mut screen, cols, rows);
        let diff = screen.geometry.diff;
        for row in diff.top()..diff.bottom() {
            frame(&mut screen, cols, rows);
            mouse(&mut screen, MouseEventKind::Down(MouseButton::Left), diff.x + 30, row);
            mouse(&mut screen, MouseEventKind::Up(MouseButton::Left), diff.x + 30, row);
            check(&mut screen, &format!("page {page}: click on row {row}"));
        }
        press(&mut screen, KeyCode::PageDown);
    }
}

#[test]
fn finding_the_file_moves_nothing_with_a_selection_open_on_every_row() {
    let fixture = mixed("place-still");
    let mut screen = fixture.open();
    // The same review, moved by the viewer's own commands only, never asked which file.
    let mut bare = fixture.open();
    let (cols, rows) = (110, 16);
    for screen in [&mut screen, &mut bare] {
        frame(screen, cols, rows);
        to_top(screen);
        frame(screen, cols, rows);
    }
    // The diff's pane only, at one height: the footer says, on a second row, when a
    // selection stopped at a hunk's edge.
    let diff = |screen: &mut Screen| {
        screen.status = None;
        let shown = diff_rows(screen, cols, rows);
        let pane = screen.geometry.diff;
        shown[usize::from(pane.top())..usize::from(pane.bottom())].to_vec()
    };
    for n in 0..70 {
        // `v` on every line the walk reaches, then a step on: the selection grows on its
        // own side only, as the viewer grows it.
        if screen.diff.cursor().expect("cursor").is_some() && screen.diff.selection().is_none() {
            keys(&mut screen, "v");
            bare.diff.begin_or_end_selection().expect("v");
        }
        keys(&mut screen, "j");
        bare.diff.run(Command::Down).expect("down");
        assert_eq!(screen.diff.selection(), bare.diff.selection(), "step {n}");
        assert_eq!(diff(&mut screen), diff(&mut bare), "step {n}: the view");
        if n % 7 == 6 {
            keys(&mut screen, "v");
            bare.diff.begin_or_end_selection().expect("v");
            press(&mut screen, KeyCode::Esc);
            bare.diff.clear_selection().expect("esc");
        }
    }
}

#[test]
fn v_then_j_at_a_hunks_end_keeps_the_cursor_and_the_selection_in_the_hunk() {
    let fixture = mixed("place-gap");
    let mut screen = fixture.open();
    frame(&mut screen, 110, 40);
    // nn.txt's first hunk ends on line 15; the row below is the gap before the next.
    screen.diff.reveal("nn.txt", Side::New, 15).expect("line 15");
    keys(&mut screen, "v");
    keys(&mut screen, "j");
    let line = Lines { path: "nn.txt".into(), side: Side::New, first: 15, last: 15 };
    assert_eq!(screen.diff.cursor().expect("cursor"), Some(line.clone()), "not on the gap");
    assert_eq!(screen.diff.selection(), Some(line));
    check(&mut screen, "after j");
}

#[test]
fn n_puts_the_heading_on_the_top_row_and_a_click_on_a_heading_scrolls_nothing() {
    let fixture = mixed("place-top");
    let mut screen = fixture.open();
    let (cols, rows) = (110, 14);
    frame(&mut screen, cols, rows);
    to_top(&mut screen);
    for path in ["app.py", "mm.bin", "nn.txt", "nonl.txt", "zz.txt"] {
        keys(&mut screen, "n");
        let shown = frame(&mut screen, cols, rows);
        let top = diff_side(&screen, &shown[usize::from(screen.geometry.diff.y)]);
        assert!(top.starts_with(&format!(" {path}")), "{path} at the top: {shown:#?}");
        assert_eq!(screen.files.current.as_deref(), Some(path));
    }
    // A click on the heading at the top, press and release before the next frame, then a
    // slow one: the view does not move, and the list names that file.
    keys(&mut screen, "N");
    let before = diff_rows(&mut screen, cols, rows);
    keys(&mut screen, "jj");
    frame(&mut screen, cols, rows);
    let diff = screen.geometry.diff;
    for _ in 0..2 {
        mouse(&mut screen, MouseEventKind::Down(MouseButton::Left), diff.x + 10, diff.y);
        mouse(&mut screen, MouseEventKind::Up(MouseButton::Left), diff.x + 10, diff.y);
    }
    assert_eq!(screen.files.current.as_deref(), Some("nonl.txt"));
    // Below the header row: the cursor's bar leaves the line `jj` reached, nothing else.
    let after = diff_rows(&mut screen, cols, rows);
    let unmarked =
        |rows: &[String]| -> Vec<String> { rows.iter().map(|r| r.replace('\u{258e}', " ")).collect() };
    assert_eq!(unmarked(&after), unmarked(&before), "nothing scrolled");
    mouse(&mut screen, MouseEventKind::Down(MouseButton::Left), diff.x + 10, diff.y);
    frame(&mut screen, cols, rows);
    mouse(&mut screen, MouseEventKind::Up(MouseButton::Left), diff.x + 10, diff.y);
    assert_eq!(unmarked(&diff_rows(&mut screen, cols, rows)), unmarked(&before), "nor now");
    assert_eq!(screen.files.current.as_deref(), Some("nonl.txt"));
}
