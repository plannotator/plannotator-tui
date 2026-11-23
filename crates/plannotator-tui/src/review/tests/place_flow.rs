//! Which file the diff's cursor is in, as the app has it (the list, the footer, `x`, `C`,
//! `n`/`N`), against what the viewer says, after every event: on every row of a mix of every
//! kind of file, split and unified, wrapped or not, at several sizes, by every way of moving,
//! and with two events before a frame. Finding it never moves the cursor, the selection or
//! the view.

use diffs_rs::engine::{Command, DiffRowKind};
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};

use super::*;
use crate::review::diff::Lines;
use crate::review::{Draft, Mode};

/// Binary files, a file with two hunks and a gap before and between them, a file whose last
/// line has no newline, and a text file last.
pub(super) fn mixed(tag: &str) -> Fixture {
    let fixture = Fixture::new(tag);
    let lines = (1..=40).fold(String::new(), |text, n| text + "line " + &n.to_string() + "\n");
    fixture.write("nn.txt", &lines);
    fixture.write("nonl.txt", "first\nlast");
    fixture.write("zz.txt", "one\ntwo\n");
    fixture.git(&["add", "nn.txt", "nonl.txt", "zz.txt"]);
    commit(&fixture);
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

fn commit(fixture: &Fixture) {
    fixture.git(&["-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-qm", "more"]);
}

/// Every kind of file the reviewer tried: a binary file first, a mode change only, a rename,
/// a deleted file, a new one, a very long file with hunks far apart, a symlink replaced by a
/// file, a very wide line, and a last line with no newline.
fn every_kind(tag: &str) -> Fixture {
    let fixture = Fixture::new(tag);
    let long = (1..=300).fold(String::new(), |text, n| text + "row " + &n.to_string() + "\n");
    let wide = format!("{}\n", "wide ".repeat(90));
    fixture.write("bb.sh", "echo hi\n");
    fixture.write("cc_old.txt", "moved as it is\nand more\n");
    fixture.write("dd.txt", "gone one\ngone two\n");
    fixture.write("ff.txt", &long);
    fixture.write("hh.txt", &format!("short\n{wide}"));
    fixture.write("ii.txt", "first\nlast");
    std::os::unix::fs::symlink("app.py", fixture.repo().join("gg")).expect("symlink");
    fixture.git(&["add", "."]);
    commit(&fixture);
    std::fs::write(fixture.repo().join("aa.bin"), [0u8, 1, 2, 255, 0]).expect("binary");
    let mode = <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o755);
    std::fs::set_permissions(fixture.repo().join("bb.sh"), mode).expect("chmod");
    fixture.git(&["mv", "cc_old.txt", "cc_new.txt"]);
    std::fs::remove_file(fixture.repo().join("dd.txt")).expect("delete");
    fixture.write("ee.txt", "fresh one\nfresh two\n");
    fixture.write("ff.txt", &long.replace("row 5\n", "row five\n").replace("row 290\n", "row two-ninety\n"));
    std::fs::remove_file(fixture.repo().join("gg")).expect("unlink");
    fixture.write("gg", "now a file\n");
    fixture.write("hh.txt", &format!("short\n{}", wide.replace("wide wide", "WIDE wide")));
    fixture.write("ii.txt", "first\nchanged");
    fixture
}

fn ctrl_n(screen: &mut Screen) {
    let key = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL);
    screen.handle_event(Event::Key(key)).expect("ctrl+n");
}

fn mouse(screen: &mut Screen, kind: MouseEventKind, column: u16, row: u16) {
    let event = MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE };
    screen.handle_event(Event::Mouse(event)).expect("mouse");
}

/// The file the viewer says the cursor is in, by the capture's path.
fn engine_file(screen: &Screen) -> String {
    let row = screen.diff.cursor_row().expect("the viewer has a cursor");
    screen.diff.paths()[row.file].clone()
}

/// The app's file for the cursor is the viewer's: the list's and the footer's, and the one
/// `x`, `C` and the footer's tick take.
fn same_file(screen: &Screen, what: &str) -> String {
    let file = engine_file(screen);
    assert_eq!(screen.files.current.as_deref(), Some(file.as_str()), "{what}: the list's file");
    if screen.diff.selection().is_none() {
        assert_eq!(screen.file_under_cursor().as_deref(), Some(file.as_str()), "{what}: the cursor's file");
    }
    file
}

/// After a frame: the same, and `x` marks that file and `C` opens a note on it. Both are
/// undone, and nothing else moves.
fn acts_on(screen: &mut Screen, cols: u16, rows: u16, what: &str) {
    frame(screen, cols, rows);
    let file = same_file(screen, what);
    let before = screen.diff.cursor_row();
    let viewed = screen.progress.is_viewed(&file);
    keys(screen, "x");
    assert_eq!(screen.progress.is_viewed(&file), !viewed, "{what}: x marks {file}");
    keys(screen, "x");
    assert_eq!(screen.progress.is_viewed(&file), viewed, "{what}: x again");
    keys(screen, "C");
    assert_eq!(screen.mode, Mode::Compose(Draft::File(file.clone())), "{what}: C notes {file}");
    press(screen, KeyCode::Esc);
    assert_eq!(screen.mode, Mode::Browse, "{what}");
    assert_eq!(screen.diff.cursor_row(), before, "{what}: x and C moved nothing");
}

/// One key, and the checks.
fn step(screen: &mut Screen, key: KeyCode, cols: u16, rows: u16, what: &str) {
    press(screen, key);
    same_file(screen, what);
    acts_on(screen, cols, rows, what);
}

/// Up to the first row: the first file's heading.
fn to_top(screen: &mut Screen) {
    keys(screen, "gg");
    for _ in 0..40 {
        keys(screen, "k");
    }
}

fn walk(screen: &mut Screen, cols: u16, rows: u16, what: &str) {
    to_top(screen);
    acts_on(screen, cols, rows, &format!("{what} top"));
    let mut files = std::collections::HashSet::new();
    for n in 0..200 {
        let before = screen.diff.cursor_row();
        step(screen, KeyCode::Char('j'), cols, rows, &format!("{what} j {n}"));
        files.insert(screen.diff.cursor_row().expect("cursor").file);
        if screen.diff.cursor_row() == before {
            break;
        }
    }
    assert_eq!(files.len(), screen.diff.paths().len(), "{what}: every file walked");
    for n in 0..200 {
        let before = screen.diff.cursor_row();
        step(screen, KeyCode::Char('k'), cols, rows, &format!("{what} k {n}"));
        if screen.diff.cursor_row() == before {
            break;
        }
    }
    for (key, times) in [(KeyCode::PageDown, 12), (KeyCode::PageUp, 12)] {
        for n in 0..times {
            step(screen, key, cols, rows, &format!("{what} {key:?} {n}"));
        }
    }
    for (keys_, times) in [("G", 1), ("gg", 1), ("]", 12), ("[", 12), ("n", 10), ("N", 10)] {
        for n in 0..times {
            keys(screen, keys_);
            same_file(screen, &format!("{what} {keys_} {n}"));
            acts_on(screen, cols, rows, &format!("{what} {keys_} {n}"));
            step(screen, KeyCode::Char('j'), cols, rows, &format!("{what} {keys_} {n} then j"));
        }
    }
    // The wheel, then a click on every row of the diff, the sticky heading included.
    let pane = screen.geometry.diff;
    for (kind, times) in [(MouseEventKind::ScrollDown, 25), (MouseEventKind::ScrollUp, 25)] {
        for n in 0..times {
            mouse(screen, kind, pane.x + 20, pane.y + 2);
            same_file(screen, &format!("{what} {kind:?} {n}"));
            acts_on(screen, cols, rows, &format!("{what} {kind:?} {n}"));
        }
    }
    for page in 0..4 {
        frame(screen, cols, rows);
        let pane = screen.geometry.diff;
        for y in pane.top()..pane.bottom() {
            frame(screen, cols, rows);
            for x in [pane.x + 12, pane.right() - 12] {
                mouse(screen, MouseEventKind::Down(MouseButton::Left), x, y);
                mouse(screen, MouseEventKind::Up(MouseButton::Left), x, y);
                same_file(screen, &format!("{what} page {page} click {x},{y}"));
            }
            acts_on(screen, cols, rows, &format!("{what} page {page} click row {y}"));
        }
        step(screen, KeyCode::PageDown, cols, rows, &format!("{what} page {page}"));
    }
}

#[test]
fn every_row_of_every_kind_of_file_is_the_viewers_file_by_every_move() {
    let fixture = every_kind("place-every");
    for (cols, rows) in [(110, 18), (170, 30)] {
        for wrap in [false, true] {
            let mut screen = fixture.open();
            frame(&mut screen, cols, rows);
            let paths = screen.diff.paths().to_vec();
            assert_eq!(paths.len(), 10, "{paths:?}");
            assert_eq!(paths.first().map(String::as_str), Some("aa.bin"), "a binary file first");
            if wrap {
                keys(&mut screen, "w");
            }
            let what = format!("{cols}x{rows} wrap {wrap}");
            walk(&mut screen, cols, rows, &what);
            // The other layout.
            keys(&mut screen, "u");
            walk(&mut screen, cols, rows, &format!("{what} u"));
        }
    }
}

#[test]
fn two_events_before_a_frame_leave_the_viewers_file() {
    let fixture = every_kind("place-two");
    let mut screen = fixture.open();
    let sizes = [(170, 30), (100, 14), (130, 20), (90, 12)];
    frame(&mut screen, sizes[0].0, sizes[0].1);
    let pane = screen.geometry.diff;
    for n in 0..80 {
        let (cols, rows) = sizes[n % sizes.len()];
        let what = format!("pair {n} at {cols}x{rows}");
        // A resize or `u`, then the wheel, a click or `j`, all before the next frame.
        match n % 3 {
            0 => screen.handle_event(Event::Resize(cols, rows)).expect("resize"),
            1 => keys(&mut screen, "u"),
            _ => keys(&mut screen, "j"),
        }
        same_file(&screen, &format!("{what}: first"));
        match n % 4 {
            0 => mouse(&mut screen, MouseEventKind::ScrollDown, pane.x + 10, pane.y + 3),
            1 => {
                mouse(&mut screen, MouseEventKind::Down(MouseButton::Left), pane.x + 10, pane.y + 4);
                mouse(&mut screen, MouseEventKind::Up(MouseButton::Left), pane.x + 10, pane.y + 4);
            }
            2 => keys(&mut screen, "j"),
            _ => press(&mut screen, KeyCode::PageDown),
        }
        same_file(&screen, &format!("{what}: second"));
        acts_on(&mut screen, cols, rows, &what);
    }
}

#[test]
fn wrapped_a_page_down_from_a_binary_file_marks_and_notes_the_file_it_reaches() {
    // The reviewer's case: a binary file first, `w`, one page down at 120x40 puts the cursor
    // on long.txt's hunk heading. Short files in between bring it there; how many, and how
    // long the first, depends on the rows above, so the first mix that does is the case.
    let long =
        (1..=60).fold(String::new(), |text, n| text + &n.to_string() + " " + &"words ".repeat(30) + "\n");
    let open = |(between, first): (usize, usize)| {
        let fixture = Fixture::new("place-wrap-page");
        std::fs::write(fixture.repo().join("aa.bin"), [0u8, 1, 0, 255]).expect("binary");
        for n in 1..=between {
            fixture.write(&format!("b{n:02}.txt"), &"x\n".repeat(if n == 1 { first } else { 1 }));
        }
        fixture.write("long.txt", &long);
        let mut screen = fixture.open();
        frame(&mut screen, 120, 40);
        keys(&mut screen, "w");
        press(&mut screen, KeyCode::PageDown);
        (fixture, screen)
    };
    let on_heading = |screen: &Screen| {
        screen.diff.cursor_row().is_some_and(|row| row.kind == DiffRowKind::Hunk(0) && row.path == "long.txt")
    };
    let (_fixture, mut screen) = (1..=8)
        .flat_map(|between| (1..=3).map(move |first| (between, first)))
        .map(open)
        .find(|(_, screen)| on_heading(screen))
        .expect("a page down onto long.txt's hunk heading");
    acts_on(&mut screen, 120, 40, "wrap page down");
    keys(&mut screen, "x");
    assert!(screen.progress.is_viewed("long.txt") && !screen.progress.is_viewed("aa.bin"));
    keys(&mut screen, "x");
    // ctrl+n goes on from long.txt, wrapping round to aa.bin.
    ctrl_n(&mut screen);
    assert_eq!(engine_file(&screen), "aa.bin");
    same_file(&screen, "after ctrl+n");
    keys(&mut screen, "gg");
    press(&mut screen, KeyCode::PageDown);
    keys(&mut screen, "x");
    assert!(screen.progress.is_viewed("long.txt") && !screen.progress.is_viewed("aa.bin"));
}

#[test]
fn the_wheel_onto_a_heading_marks_notes_and_steps_from_that_file() {
    // The reviewer's case: 110x16, the wheel until the cursor is on nn.txt's heading.
    let fixture = mixed("place-wheel-heading");
    let mut screen = fixture.open();
    let (cols, rows) = (110, 16);
    frame(&mut screen, cols, rows);
    to_top(&mut screen);
    let pane = screen.geometry.diff;
    let mut reached = false;
    for _ in 0..60 {
        mouse(&mut screen, MouseEventKind::ScrollDown, pane.x + 20, pane.y + 2);
        frame(&mut screen, cols, rows);
        same_file(&screen, "wheel");
        let row = screen.diff.cursor_row().expect("cursor");
        if row.kind == DiffRowKind::Header && engine_file(&screen) == "nn.txt" {
            reached = true;
            break;
        }
    }
    assert!(reached, "the wheel put the cursor on nn.txt's heading");
    acts_on(&mut screen, cols, rows, "on nn.txt's heading");
    keys(&mut screen, "n");
    assert_eq!(engine_file(&screen), "nonl.txt", "n goes on from nn.txt");
    same_file(&screen, "after n");
    ctrl_n(&mut screen);
    assert_eq!(engine_file(&screen), "zz.txt", "ctrl+n goes on from nonl.txt");
    same_file(&screen, "after ctrl+n");
}

#[test]
fn a_click_on_the_sticky_heading_and_a_resize_onto_an_empty_side_keep_the_file() {
    let fixture = every_kind("place-sticky");
    let mut screen = fixture.open();
    let (cols, rows) = (170, 14);
    frame(&mut screen, cols, rows);
    screen.diff.reveal("ff.txt", Side::New, 290).expect("line 290");
    frame(&mut screen, cols, rows);
    let pane = screen.geometry.diff;
    let shown = frame(&mut screen, cols, rows);
    let top = diff_side(&screen, &shown[usize::from(pane.y)]);
    assert!(top.starts_with(" ff.txt"), "the sticky heading: {top:?}");
    let before = screen.diff.cursor_row();
    mouse(&mut screen, MouseEventKind::Down(MouseButton::Left), pane.x + 10, pane.y);
    mouse(&mut screen, MouseEventKind::Up(MouseButton::Left), pane.x + 10, pane.y);
    assert_eq!(screen.diff.cursor_row(), before, "the viewer takes no click there");
    acts_on(&mut screen, cols, rows, "a click on the sticky heading");
    // The old side of an added line: a split row's empty cell, then narrower, unified.
    screen.diff.reveal("ee.txt", Side::New, 1).expect("ee.txt");
    press(&mut screen, KeyCode::Char('h'));
    let row = screen.diff.cursor_row().expect("cursor");
    assert_eq!((row.side, row.old_line), (diffs_rs::engine::Side::Old, None), "{row:?}");
    acts_on(&mut screen, cols, rows, "an empty side");
    screen.handle_event(Event::Resize(100, rows)).expect("resize");
    same_file(&screen, "resized, before the frame");
    acts_on(&mut screen, 100, rows, "resized");
    // The "No newline" mark.
    screen.diff.reveal("ii.txt", Side::New, 2).expect("ii.txt");
    keys(&mut screen, "j");
    if screen.diff.cursor_row().is_some_and(|row| row.kind == DiffRowKind::NoNewline) {
        screen.handle_event(Event::Resize(170, rows)).expect("resize");
        acts_on(&mut screen, 170, rows, "on the No newline mark, resized");
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
        let shown = frame(screen, cols, rows);
        let pane = screen.geometry.diff;
        shown[usize::from(pane.top())..usize::from(pane.bottom())]
            .iter()
            .map(|row| diff_side(screen, row))
            .collect::<Vec<_>>()
    };
    for n in 0..70 {
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
    same_file(&screen, "after j");
}

#[test]
fn v_then_a_jump_to_another_hunk_ends_the_growing_and_j_moves_on() {
    let fixture = mixed("place-v-jump");
    let mut screen = fixture.open();
    frame(&mut screen, 110, 40);
    screen.diff.reveal("nn.txt", Side::New, 13).expect("line 13");
    keys(&mut screen, "vj");
    keys(&mut screen, "]");
    assert!(!screen.diff.selecting(), "the jump ended the growing");
    let lines = Lines { path: "nn.txt".into(), side: Side::New, first: 13, last: 14 };
    assert_eq!(screen.diff.selection(), Some(lines.clone()));
    let heading = screen.diff.cursor_row().expect("cursor");
    assert_eq!((heading.kind, heading.hunk), (DiffRowKind::Hunk(1), Some(1)));
    keys(&mut screen, "j");
    let row = screen.diff.cursor_row().expect("cursor");
    assert_eq!((row.hunk, row.new_line), (Some(1), Some(30)), "j went on into the next hunk");
    assert_eq!(screen.diff.selection(), Some(lines), "the selection stayed");
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

/// The diff's part of each row of a frame.
fn diff_rows(screen: &mut Screen, cols: u16, rows: u16) -> Vec<String> {
    let shown = frame(screen, cols, rows);
    shown.iter().map(|row| diff_side(screen, row)).collect()
}

#[test]
fn ctrl_n_goes_past_both_files_of_a_path_and_counts_the_path_once() {
    // gg is two files: the symlink deleted and a file added in its place.
    let fixture = every_kind("place-twice");
    let mut screen = fixture.open();
    frame(&mut screen, 120, 30);
    let paths = screen.diff.paths().to_vec();
    let gg: Vec<usize> = (0..paths.len()).filter(|&i| paths[i] == "gg").collect();
    assert_eq!(gg.len(), 2, "{paths:?}");
    for path in &paths {
        if path != "gg" && path != "hh.txt" {
            screen.progress.set(path, true).expect("mark");
        }
    }
    screen.goto_index(gg[0]).expect("gg");
    assert_eq!(screen.diff.cursor_file(), Some(gg[0]));
    for _ in 0..3 {
        ctrl_n(&mut screen);
        assert_eq!(engine_file(&screen), "hh.txt", "ctrl+n goes past both of gg's files");
        assert_eq!(screen.status.as_deref(), Some("hh.txt \u{b7} 2 not viewed"));
        same_file(&screen, "on hh.txt");
        ctrl_n(&mut screen);
        assert_eq!(screen.diff.cursor_file(), Some(gg[0]), "back round to gg's first file");
        assert_eq!(screen.status.as_deref(), Some("gg \u{b7} 2 not viewed"));
    }
    // From gg's second file too.
    screen.goto_index(gg[1]).expect("gg");
    ctrl_n(&mut screen);
    assert_eq!(engine_file(&screen), "hh.txt");
}
