//! Boxes in the diff never move what the person reads. Unified and split (50:50, 30:70), wrap
//! on and off, the cursor near the top, in the middle and on the last page, with none, one or
//! many notes above and below it: after each thing a person does (a note added, edited,
//! removed, one from the agent, the Notes setting switched, the panel, the file list, a resize,
//! the line between the sides, the layout, wrapping, whitespace, the tab width, a viewed mark,
//! a drag) the cursor's line is on the screen row it was on, every line's code starts at the
//! column it did, no row is left empty, and every box is inside its rows, over no code, under
//! its own note's line, saying that note's words (`inline_support`).
//!
//! The exceptions are the documented ones: at the very top of the diff the top stays and the
//! cursor's line moves as boxes above it come and go; wrapping turned on or off, reading the diff again, the file filter and jumping to a
//! note from the panel move the view by design (wrapping as it does with no boxes: the
//! cursor's file's heading comes to the top), and there only the boxes and the columns are
//! checked.

use plannotator_tui_review::Side;
use ratatui::crossterm::event::{Event, KeyCode, MouseButton, MouseEventKind};

use super::compose_near::{draw, report};
use super::inline_flow::set_notes;
use super::inline_support::{Spot, cells, check_boxes, comment, no_holes, row_of_cursor, spot};
use super::save_keeps_view::{Columns, PATH, SIZE, columns, source};
use super::*;
use crate::config::review::Notes;
use crate::review::diff::Progress;

/// Screen.kt changed from line 60 to 140, notes.txt changed, opened as `config` says with the
/// notes inline.
fn kotlin(tag: &str, config: &str) -> (Fixture, Screen) {
    let fixture = Fixture::new(tag);
    fixture.write(PATH, &source(false));
    fixture.git(&["add", "."]);
    fixture.git(&["-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-qm", "kotlin"]);
    fixture.write(PATH, &source(true));
    fixture.write("notes.txt", "changed\n");
    fixture.write_config(&format!("[review]\nnotes = \"inline\"\n{config}\n"));
    let mut screen = fixture.open_configured();
    ready(&mut screen);
    (fixture, screen)
}

/// Draw and poll until the viewer has `files` files, coloured.
fn ready_with(screen: &mut Screen, files: usize) {
    let started = Instant::now();
    loop {
        screen.tick().expect("tick");
        draw(screen, SIZE);
        if screen.diff.progress() == Progress::Ready && screen.diff.files_shown() >= files {
            screen.tick().expect("tick");
            draw(screen, SIZE);
            return;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "the viewer never got ready");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Draw and poll until the viewer has both files, coloured.
fn ready(screen: &mut Screen) {
    ready_with(screen, 2);
}

/// Where the cursor starts.
#[derive(Debug, Clone, Copy)]
enum At {
    /// On the first line of the hunk, at the top of the diff.
    Top,
    /// In the middle of the pane, on line 100.
    Middle,
    /// The last line of the diff (`G`).
    LastPage,
}

/// How many notes the review has before the first step.
#[derive(Debug, Clone, Copy)]
enum Count {
    None,
    One,
    Several,
}

/// What a frame shows: the cursor's line and row, and the code's columns.
struct Look {
    cursor: Spot,
    row: Option<u16>,
    columns: Columns,
    at_start: bool,
    /// The pane's text, to show when a check fails.
    text: String,
}

fn look(screen: &mut Screen, case: &str) -> Look {
    let buffer = draw(screen, SIZE);
    check_boxes(screen, &buffer, case);
    no_holes(screen, case);
    let cursor = spot(screen);
    let row = row_of_cursor(screen);
    let pane = screen.geometry.diff;
    let top_row = screen.diff.row_at(pane.right() - 2, pane.y);
    let at_start = top_row
        .as_ref()
        .is_some_and(|row| row.file == 0 && matches!(row.kind, diffs_rs::engine::DiffRowKind::Header));
    let text = (pane.top()..pane.bottom())
        .map(|y| cells(&buffer, pane.x, y, pane.width))
        .collect::<Vec<_>>()
        .join("\n");
    Look { cursor, row, columns: columns(screen, &buffer), at_start, text }
}

/// How a step may change the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Keeps {
    /// The cursor's line on its row (moving up only at the very start of the diff).
    Cursor,
    /// Neither: the view moves by design. The boxes and the columns still hold.
    Nothing,
}

fn kept(before: &Look, after: &Look, keeps: Keeps, case: &str) {
    for (key, column) in &before.columns {
        if let Some(now) = after.columns.get(key) {
            assert_eq!(column, now, "{case}: line {key:?} scrolled sideways");
        }
    }
    let common = before.columns.keys().filter(|k| after.columns.contains_key(*k)).count();
    assert!(
        after.columns.is_empty() || common > 0 || keeps == Keeps::Nothing,
        "{case}: no line of before is on screen"
    );
    let side_columns = |look: &Look| {
        let mut by_side: std::collections::BTreeMap<&str, u16> = std::collections::BTreeMap::new();
        for ((side, _), column) in &look.columns {
            let first = by_side.entry(side).or_insert(*column);
            assert_eq!(first, column, "{case}: every line of a side from its start");
        }
    };
    side_columns(after);
    match keeps {
        Keeps::Cursor => {
            assert_eq!(after.cursor, before.cursor, "{case}: the cursor stays on its line");
            match (before.row, after.row) {
                // At the very top of the diff the top stays the top: boxes coming or going
                // above the cursor move its line instead.
                (Some(_), Some(_)) if before.at_start && after.at_start => {}
                (was, now) => assert_eq!(
                    now, was,
                    "{case}: the cursor's line keeps its screen row\nbefore:\n{}\nafter:\n{}",
                    before.text, after.text
                ),
            }
        }
        Keeps::Nothing => {}
    }
}

/// The line of Screen.kt `delta` lines from the cursor's (or from line 120 when the cursor
/// is in the other file), kept inside the hunk.
fn near(screen: &Screen, delta: i64) -> u32 {
    let row = screen.diff.cursor_row().expect("cursor");
    let line = if row.path == PATH { row.new_line.or(row.old_line).unwrap_or(100) } else { 130 };
    (i64::from(line) + delta).clamp(58, 142) as u32
}

fn notes(screen: &mut Screen, many: Count) {
    match many {
        Count::None => {}
        Count::One => {
            let line = near(screen, -2);
            comment(screen, PATH, Side::New, (line, line), "one note here");
        }
        Count::Several => {
            keys(screen, "C");
            type_and_save(screen, "about the whole file");
            for (side, lines, text) in [
                (Side::New, (61, 62), "a range near the top"),
                (Side::Old, (64, 64), "the old side of 64"),
                (Side::New, (90, 90), "first on ninety"),
                (Side::New, (90, 90), "second on ninety"),
                (Side::Old, (100, 100), "old hundred"),
                (Side::New, (110, 112), "a range in the middle and a longer text that wraps in a box"),
                (Side::New, (125, 125), "near the end"),
                (Side::New, (138, 138), "last one in the hunk"),
            ] {
                comment(screen, PATH, side, lines, text);
            }
            comment(screen, "notes.txt", Side::New, (1, 1), "the other file");
        }
    }
}

fn start(screen: &mut Screen, at: At) {
    match at {
        At::Top => {
            screen.diff.show_heading(0).expect("heading");
            for _ in 0..3 {
                press(screen, KeyCode::Char('j'));
            }
        }
        At::Middle => {
            screen.diff.reveal(PATH, Side::New, 118).expect("line 118");
            draw(screen, SIZE);
            for _ in 0..12 {
                press(screen, KeyCode::Char('k'));
            }
        }
        At::LastPage => press(screen, KeyCode::Char('G')),
    }
}

/// One thing a person does, and what it keeps.
type Step = (&'static str, Keeps, fn(&mut Screen, &Fixture));

fn steps() -> Vec<Step> {
    vec![
        ("a note above, saved", Keeps::Cursor, |s, _| {
            let line = near(s, -3);
            comment(s, PATH, Side::New, (line, line), "added above the cursor");
        }),
        ("a note below, saved", Keeps::Cursor, |s, _| {
            let line = near(s, 2);
            comment(s, PATH, Side::New, (line, line), "added below the cursor");
        }),
        ("c on the cursor's line, typed, saved", Keeps::Cursor, |s, _| {
            if s.diff.cursor().ok().flatten().is_some() {
                press(s, KeyCode::Char('c'));
                type_and_save(s, "typed on the cursor line");
            }
        }),
        ("a note above edited taller", Keeps::Cursor, |s, _| {
            if let Some(id) = above(s) {
                s.edit_note(&id);
                press(s, KeyCode::End);
                type_and_save(s, &" and much more text that makes this box taller than before".repeat(3));
            }
        }),
        ("a note above edited shorter", Keeps::Cursor, |s, _| {
            if let Some(id) = above(s) {
                s.edit_note(&id);
                s.compose = crate::ui::compose::Compose::default();
                type_and_save(s, "short");
            }
        }),
        ("a note above removed", Keeps::Cursor, |s, _| {
            if let Some(id) = above(s) {
                s.remove_note(&id).expect("removed");
            }
        }),
        ("a note from the agent", Keeps::Cursor, |s, fixture| {
            let line = near(s, -4);
            let file = fixture.repo().join(PATH);
            let (note, _answer) = crate::pi::link::Note::for_test(file, None, Some(line as usize), "from pi");
            s.take_pi_note(&note);
        }),
        ("Notes: both", Keeps::Cursor, |s, _| set_notes(s, Notes::Both)),
        ("a note saved with both", Keeps::Cursor, |s, _| {
            let line = near(s, -1);
            comment(s, PATH, Side::Old, (line, line), "saved with both");
        }),
        ("Notes: panel", Keeps::Cursor, |s, _| set_notes(s, Notes::Panel)),
        ("a note saved with the panel", Keeps::Cursor, |s, _| {
            let line = near(s, -5);
            comment(s, PATH, Side::New, (line, line), "saved with the panel");
        }),
        ("Notes: inline", Keeps::Cursor, |s, _| set_notes(s, Notes::Inline)),
        ("z opens the panel", Keeps::Cursor, |s, _| keys(s, "z")),
        ("z closes it", Keeps::Cursor, |s, _| keys(s, "z")),
        ("f twice closes the list", Keeps::Cursor, |s, _| keys(s, "ff")),
        ("f opens it", Keeps::Cursor, |s, _| {
            keys(s, "f");
            press(s, KeyCode::Esc);
        }),
        ("narrower and back", Keeps::Cursor, |s, _| {
            draw(s, (150, SIZE.1));
            draw(s, SIZE);
        }),
        ("(", Keeps::Cursor, |s, _| keys(s, "(")),
        (")", Keeps::Cursor, |s, _| keys(s, ")")),
        ("u twice", Keeps::Cursor, |s, _| {
            keys(s, "u");
            draw(s, SIZE);
            keys(s, "u");
        }),
        ("W twice", Keeps::Cursor, |s, _| {
            keys(s, "W");
            draw(s, SIZE);
            keys(s, "W");
        }),
        ("the tab width", Keeps::Cursor, |s, _| {
            let row = crate::review::settings::ROWS
                .iter()
                .position(|row| *row == crate::review::settings::Row::TabWidth)
                .expect("the row");
            keys(s, ",");
            for _ in 0..row {
                keys(s, "j");
            }
            press(s, KeyCode::Right);
            press(s, KeyCode::Esc);
        }),
        ("x marks the file viewed", Keeps::Cursor, |s, _| keys(s, "x")),
        ("x again", Keeps::Cursor, |s, _| keys(s, "x")),
        ("a drag over lines and a box, let go", Keeps::Nothing, |s, _| drag_over(s)),
        ("w", Keeps::Nothing, |s, _| keys(s, "w")),
        ("w back", Keeps::Nothing, |s, _| keys(s, "w")),
        ("a note picked in the panel", Keeps::Nothing, |s, _| {
            keys(s, "z");
            press(s, KeyCode::Tab);
            keys(s, "jj");
            press(s, KeyCode::Tab);
            keys(s, "z");
        }),
        ("the file filter and back", Keeps::Nothing, |s, _| {
            keys(s, "/Screen");
            press(s, KeyCode::Enter);
            ready_with(s, 1);
            keys(s, "/");
            press(s, KeyCode::Esc);
            ready(s);
        }),
        ("r reads the diff again", Keeps::Nothing, |s, _| {
            keys(s, "r");
            ready(s);
        }),
    ]
}

/// A line note on Screen.kt above the cursor's line, the nearest.
fn above(screen: &Screen) -> Option<String> {
    let row = screen.diff.cursor_row()?;
    let line = if row.path == PATH { row.new_line.or(row.old_line)? } else { 200 };
    screen
        .store
        .annotations()
        .iter()
        .filter(|n| {
            n.file_path == PATH && n.scope == plannotator_tui_review::NoteScope::Line && n.line_end < line
        })
        .max_by_key(|n| n.line_end)
        .map(|n| n.id.clone())
}

/// A drag down over three rows of code from the cursor's row, which crosses a box when one
/// hangs there, and the release: the view does not move (0.11.3).
fn drag_over(screen: &mut Screen) {
    let buffer = draw(screen, SIZE);
    drop(buffer);
    let pane = screen.geometry.diff;
    let x = pane.right() - 20;
    let rows = |screen: &Screen| -> Vec<_> {
        (pane.top()..pane.bottom())
            .map(|y| {
                screen
                    .diff
                    .row_at(pane.right() - 2, y)
                    .map(|r| (r.path, r.old_line, r.new_line, r.part, r.kind))
            })
            .collect()
    };
    let Some(from) = (pane.top() + 3..pane.bottom() - 8)
        .find(|&y| screen.diff.row_at(x, y).is_some_and(|r| r.kind == diffs_rs::engine::DiffRowKind::Code))
    else {
        return;
    };
    let before = rows(screen);
    report(screen, MouseEventKind::Down(MouseButton::Left), (x, from));
    for y in from + 1..from + 6 {
        report(screen, MouseEventKind::Drag(MouseButton::Left), (x, y));
        draw(screen, SIZE);
    }
    report(screen, MouseEventKind::Up(MouseButton::Left), (x, from + 5));
    draw(screen, SIZE);
    assert_eq!(rows(screen), before, "letting go moved the view");
    screen.handle_event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))).expect("esc");
}

fn every_step(name: &str, layout: &str) {
    for wrap in [false, true] {
        for at in [At::Top, At::Middle, At::LastPage] {
            for many in [Count::None, Count::One, Count::Several] {
                let case = format!("{name} wrap {wrap} {at:?} {many:?}");
                let tag = format!("inline-view-{}-{wrap}-{at:?}-{many:?}", name.replace([' ', ':'], "-"));
                let (fixture, mut screen) = kotlin(&tag, &format!("{layout}\nwrap = {wrap}"));
                notes(&mut screen, many);
                start(&mut screen, at);
                let mut before = look(&mut screen, &format!("{case}: start"));
                for (what, keeps, step) in steps() {
                    let case = format!("{case}: {what}");
                    step(&mut screen, &fixture);
                    let after = look(&mut screen, &case);
                    kept(&before, &after, keeps, &case);
                    before = after;
                }
            }
        }
    }
}

#[test]
fn unified_boxes_never_move_the_view() {
    every_step("unified", "layout = \"unified\"");
}

#[test]
fn split_50_boxes_never_move_the_view() {
    every_step("split 50", "layout = \"split\"");
}

#[test]
fn split_30_boxes_never_move_the_view() {
    every_step("split 30", "layout = \"split\"\nsplit_ratio = 0.3");
}
