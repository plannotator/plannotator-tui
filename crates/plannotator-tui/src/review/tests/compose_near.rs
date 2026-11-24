//! The comment box opens next to the lines it is about: under them, else above them, over
//! their side of a split diff, never over them while there is room, and never past the
//! pane. A file's note opens under its heading, the whole change's in the middle. Every
//! position is read from the frame the screen drew. On a long diff, split at 160×45.

use plannotator_tui_review::Side;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use super::*;
use crate::review::diff::Lines;
use crate::review::{Draft, Mode};

pub(super) const COLS: u16 = 160;
pub(super) const ROWS: u16 = 45;

/// Four files of 200 lines, each changed in two blocks, lines 1 to 89 and 100 to 189; their
/// first hunk runs to line 92. With `wide`, lines 30 to 32 run past any pane.
pub(super) fn long(tag: &str, wide: bool) -> Fixture {
    let fixture = Fixture::new(tag);
    let line = |f: usize, i: usize, changed: bool| {
        let tail = if wide && (30..=32).contains(&i) { " and then some".repeat(12) } else { String::new() };
        if changed {
            format!("f{f}_line_{i} = {i} * 2  # changed{tail}\n")
        } else {
            format!("f{f}_line_{i} = {i}\n")
        }
    };
    for f in 0..4 {
        fixture.write(&format!("file{f}.py"), &(1..=200).map(|i| line(f, i, false)).collect::<String>());
    }
    fixture.git(&["add", "."]);
    fixture.git(&["-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-qm", "long"]);
    for f in 0..4 {
        fixture
            .write(&format!("file{f}.py"), &(1..=200).map(|i| line(f, i, i % 100 < 90)).collect::<String>());
    }
    fixture
}

pub(super) fn draw(screen: &mut Screen, (cols, rows): (u16, u16)) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(cols, rows)).expect("terminal");
    let mut drawn = Ok(());
    terminal.draw(|f| drawn = screen.draw(f)).expect("draw");
    drawn.expect("viewer draw");
    screen.after_draw();
    terminal.backend().buffer().clone()
}

/// What each row of the pane shows (the file, the old and the new line) and the cursor's row:
/// equal views mean nothing scrolled and the cursor did not move.
pub(super) type View = (Vec<Option<(String, Option<u32>, Option<u32>)>>, Option<String>);

pub(super) fn view(screen: &mut Screen, size: (u16, u16)) -> View {
    draw(screen, size);
    let pane = screen.geometry.diff;
    let rows = (pane.top()..pane.bottom())
        .map(|y| screen.diff.row_at(pane.right() - 2, y).map(|row| (row.path, row.old_line, row.new_line)))
        .collect();
    let cursor = screen.diff.cursor_row().map(|row| {
        format!(
            "{} {:?} {:?} {} {:?} {:?} {:?}",
            row.file, row.kind, row.hunk, row.part, row.old_line, row.new_line, row.side
        )
    });
    (rows, cursor)
}

/// The cell where `line` of file0.py is drawn on `side`: on the new side, in a split diff,
/// that side's pane.
pub(super) fn cell_of(screen: &mut Screen, frame_size: (u16, u16), side: Side, line: u32) -> (u16, u16) {
    draw(screen, frame_size);
    let pane = screen.geometry.diff;
    let x = if side == Side::Old { pane.x + 20 } else { pane.right() - 12 };
    let y = (pane.top()..pane.bottom())
        .find(|&y| {
            screen.diff.row_at(x, y).is_some_and(|row| {
                let shown = if side == Side::Old { row.old_line } else { row.new_line };
                row.path == "file0.py" && shown == Some(line) && row.part == 0
            })
        })
        .unwrap_or_else(|| panic!("line {line} is not on screen"));
    (x, y)
}

pub(super) fn report(screen: &mut Screen, kind: MouseEventKind, (column, row): (u16, u16)) {
    screen
        .handle_event(Event::Mouse(MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE }))
        .expect("mouse");
}

/// Drag from `first` to `last` of file0.py on `side` as a terminal reports it, several
/// reports a cell, and let go: the selection and its toolbar.
pub(super) fn drag(screen: &mut Screen, frame_size: (u16, u16), side: Side, first: u32, last: u32) {
    let from = cell_of(screen, frame_size, side, first);
    let to = cell_of(screen, frame_size, side, last);
    report(screen, MouseEventKind::Down(MouseButton::Left), from);
    for y in from.1..=to.1 {
        for dx in [0, 1, 1, 0] {
            report(screen, MouseEventKind::Drag(MouseButton::Left), (to.0 + dx, y));
        }
    }
    report(screen, MouseEventKind::Up(MouseButton::Left), to);
    assert_eq!(screen.diff.selection(), Some(file0(side, first, last)));
}

pub(super) fn file0(side: Side, first: u32, last: u32) -> Lines {
    Lines { path: "file0.py".into(), side, first, last }
}

/// The box drawn in `buffer` whose title starts with `verb`, from its rounded corners.
pub(super) fn the_box(buffer: &Buffer, verb: &str) -> Rect {
    let area = buffer.area;
    let symbol = |x: u16, y: u16| buffer.cell((x, y)).map_or("", |c| c.symbol());
    for y in area.top()..area.bottom() {
        let row: String = (area.left()..area.right()).map(|x| symbol(x, y)).collect();
        let Some(byte) = row.find(&format!(" {verb}")) else { continue };
        let column = row.get(..byte).map_or(0, |before| before.chars().count()) as u16;
        let x =
            (area.left()..=column).rev().find(|&x| symbol(x, y) == "\u{256d}").expect("a top-left corner");
        let right = (x..area.right()).find(|&c| symbol(c, y) == "\u{256e}").expect("a top-right corner");
        let bottom = (y..area.bottom()).find(|&r| symbol(x, r) == "\u{2570}").expect("a bottom-left corner");
        assert_eq!(symbol(right, bottom), "\u{256f}", "the corners make a box");
        return Rect { x, y, width: right - x + 1, height: bottom - y + 1 };
    }
    panic!("no box titled {verb:?}");
}

/// The rows of `pane` drawn in the selection's colour.
pub(super) fn tinted(buffer: &Buffer, pane: Rect) -> Vec<u16> {
    let tint = crate::review::mouse::selection_bg();
    (pane.top()..pane.bottom())
        .filter(|&y| (pane.left()..pane.right()).any(|x| buffer.cell((x, y)).is_some_and(|c| c.bg == tint)))
        .collect()
}

/// `inner` lies inside `outer`.
pub(super) fn within(inner: Rect, outer: Rect) -> bool {
    outer.union(inner) == outer
}

/// Open the box with `key` and check it against the selected rows: inside the pane, at most
/// 84 wide, and on none of them. Returns the box and the selected rows.
fn open(screen: &mut Screen, size: (u16, u16), key: char, verb: &str) -> (Rect, Vec<u16>) {
    let selected = tinted(&draw(screen, size), screen.geometry.diff);
    press(screen, KeyCode::Char(key));
    let buffer = draw(screen, size);
    let pane = screen.geometry.diff;
    let found = the_box(&buffer, verb);
    assert!(within(found, pane), "{found:?} in {pane:?}");
    assert!(found.width <= 84);
    assert_eq!(tinted(&buffer, pane), selected, "every selected row still shows, still selected");
    assert!(selected.iter().all(|y| !(found.top()..found.bottom()).contains(y)), "{found:?} {selected:?}");
    (found, selected)
}

/// The pane of a split diff that shows the new side: from after the middle line.
fn new_pane(pane: Rect) -> u16 {
    pane.x + pane.width.saturating_sub(1) / 2 + 1
}

#[test]
fn a_split_diffs_box_opens_under_lines_at_the_top_and_middle_over_their_side() {
    let (cols, rows) = (COLS, ROWS);
    for (side, first, last) in [(Side::New, 2, 3), (Side::New, 18, 21), (Side::Old, 18, 21)] {
        let fixture = long("near-split", false);
        let mut screen = fixture.open();
        drag(&mut screen, (cols, rows), side, first, last);
        let (found, selected) = open(&mut screen, (cols, rows), 'c', "comment on");
        let pane = screen.geometry.diff;
        assert!(screen.diff.split(), "the diff is split at {cols} columns");
        assert_eq!(found.y, selected.last().expect("selected") + 1, "{side:?} {first}-{last}: right under");
        let left = if side == Side::New { new_pane(pane) } else { pane.x };
        assert_eq!(found.x, left, "{side:?}: over its side");
        assert!(found.right() <= if side == Side::New { pane.right() } else { new_pane(pane) - 1 });
        type_and_save(&mut screen, "a note");
        let note = screen.store.annotations().first().expect("saved");
        assert_eq!((note.side, note.line_start, note.line_end), (side, first, last));
    }
}

#[test]
fn a_unified_diffs_box_opens_under_the_lines_from_the_panes_left_edge() {
    let size = (COLS, ROWS);
    let fixture = long("near-unified", false);
    let mut screen = fixture.open();
    draw(&mut screen, size);
    for _ in 0..3 {
        if !screen.diff.split() {
            break;
        }
        press(&mut screen, KeyCode::Char('u'));
        draw(&mut screen, size);
    }
    // Unified, the removed lines come first, the added ones under them.
    screen.diff.reveal("file0.py", Side::New, 20).expect("line 20");
    let shown: Vec<u32> =
        view(&mut screen, size).0.iter().flatten().filter(|r| r.1.is_none()).filter_map(|r| r.2).collect();
    let first = shown[shown.len() / 2];
    drag(&mut screen, size, Side::New, first, first + 2);
    let verb = format!("comment on file0.py:{first}-{}", first + 2);
    let (found, selected) = open(&mut screen, size, 'c', &verb);
    assert!(!screen.diff.split());
    assert_eq!(found.y, selected.last().expect("selected") + 1);
    assert_eq!((found.x, found.width), (screen.geometry.diff.x, 84));
}

#[test]
fn with_wrap_on_the_box_opens_under_the_last_part_of_a_wrapped_line() {
    let size = (COLS, ROWS);
    let fixture = long("near-wrap", true);
    let mut screen = fixture.open();
    press(&mut screen, KeyCode::Char('w'));
    draw(&mut screen, size);
    // A reveal leaves a line already on screen where it is, near the pane's foot here, where
    // the wrapped lines would run past it: the wheel brings line 30 up to the top third.
    let line_30 = Lines { path: "file0.py".into(), side: Side::New, first: 30, last: 30 };
    let pane = screen.geometry.diff;
    for _ in 0..40 {
        if screen.diff.rows_showing(&line_30).is_some_and(|(first, _)| first < pane.y + pane.height / 3) {
            break;
        }
        let at = (pane.x + pane.width / 2, pane.y + pane.height / 2);
        let wheel = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: at.0,
            row: at.1,
            modifiers: KeyModifiers::NONE,
        };
        screen.handle_event(Event::Mouse(wheel)).expect("wheel");
        draw(&mut screen, size);
    }
    screen.diff.reveal("file0.py", Side::New, 30).expect("line 30");
    draw(&mut screen, size);
    press(&mut screen, KeyCode::Char('v'));
    // `j` steps through a wrapped line's rows.
    for _ in 0..40 {
        if screen.diff.selection().is_some_and(|lines| lines.last == 32) {
            break;
        }
        press(&mut screen, KeyCode::Char('j'));
        draw(&mut screen, size);
    }
    let (found, selected) = open(&mut screen, size, 'c', "comment on file0.py:30-32");
    assert!(selected.len() > 3, "the long lines wrap: {selected:?}");
    assert_eq!(found.y, selected.last().expect("selected") + 1);
    assert_eq!(found.x, new_pane(screen.geometry.diff));
}

#[test]
fn lines_on_the_last_rows_get_the_box_above_them() {
    let size = (COLS, ROWS);
    let fixture = long("near-bottom", false);
    let mut screen = fixture.open();
    let shown = view(&mut screen, size).0;
    let bottom =
        shown.iter().rev().find_map(|row| row.as_ref().and_then(|row| row.2)).expect("a line at the bottom");
    drag(&mut screen, size, Side::New, bottom - 1, bottom);
    let pane = screen.geometry.diff;
    let (found, selected) = open(&mut screen, size, 'c', "comment on");
    assert_eq!(selected.last(), Some(&(pane.bottom() - 1)), "the lines end on the last row");
    assert_eq!(found.bottom(), selected[0], "right above the first selected row");
    // The suggestion box, which opens with the code, goes there too.
    press(&mut screen, KeyCode::Esc);
    let (suggest, _) = open(&mut screen, size, 's', "suggest code for file0.py");
    assert_eq!(suggest.bottom(), selected[0]);
    assert_eq!(suggest.height, 4, "two lines of code and the borders");
}

#[test]
fn lines_taller_than_the_pane_keep_the_box_beside_the_end_last_moved_to() {
    let size = (COLS, ROWS);
    for down in [true, false] {
        let fixture = long("near-tall", false);
        let mut screen = fixture.open();
        screen.diff.reveal("file0.py", Side::New, if down { 5 } else { 85 }).expect("line");
        press(&mut screen, KeyCode::Char('v'));
        for _ in 0..70 {
            press(&mut screen, KeyCode::Char(if down { 'j' } else { 'k' }));
            draw(&mut screen, size);
        }
        let (cursor, lines) = if down { (75, (5, 75)) } else { (15, (15, 85)) };
        assert_eq!(screen.diff.selection(), Some(file0(Side::New, lines.0, lines.1)));
        let shown: Vec<u32> = view(&mut screen, size).0.iter().flatten().filter_map(|row| row.2).collect();
        assert!(shown.iter().all(|n| (lines.0..=lines.1).contains(n)), "the lines fill the pane: {shown:?}");
        let at = cell_of(&mut screen, size, Side::New, cursor).1;
        press(&mut screen, KeyCode::Char('c'));
        let buffer = draw(&mut screen, size);
        let found = the_box(&buffer, "comment on");
        assert!(within(found, screen.geometry.diff));
        if down {
            assert_eq!(found.bottom(), at, "right above the cursor's line, the end dragged to");
        } else {
            assert_eq!(found.y, at + 1, "right under the cursor's line, the end dragged to");
        }
    }
}

#[test]
fn the_box_keeps_its_side_and_scrolls_inside_as_the_text_grows() {
    let size = (COLS, ROWS);
    let fixture = long("near-grow", false);
    let mut screen = fixture.open();
    drag(&mut screen, size, Side::New, 18, 19);
    let (opened, selected) = open(&mut screen, size, 'c', "comment on");
    assert_eq!(opened.height, 3);
    for n in 0..12 {
        screen.handle_event(Event::Paste(format!("line {n}"))).expect("paste");
        screen.handle_event(Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT))).expect("key");
    }
    let buffer = draw(&mut screen, size);
    let grown = the_box(&buffer, "comment on");
    assert_eq!((grown.x, grown.y), (opened.x, opened.y), "it grew down from where it opened");
    assert_eq!(grown.height, 10, "eight rows of text at most");
    assert_eq!(tinted(&buffer, screen.geometry.diff), selected);
    let text: Vec<String> = (grown.top()..grown.bottom())
        .map(|y| (grown.left()..grown.right()).map(|x| buffer[(x, y)].symbol()).collect())
        .collect();
    assert!(text.iter().any(|r| r.contains("line 11")), "the last line shows:\n{}", text.join("\n"));
    assert!(!text.iter().any(|r| r.contains("line 0 ")), "the first scrolled away");
}

#[test]
fn at_60_by_12_the_box_fits_the_pane_under_or_above_the_lines() {
    let size = (60, 12);
    let fixture = long("near-small", false);
    let mut screen = fixture.open();
    draw(&mut screen, size);
    assert_eq!(screen.geometry.files.width, 0, "the file list is hidden");
    assert_eq!(screen.geometry.rail.width, 0, "no notes yet");
    screen.diff.reveal("file0.py", Side::New, 3).expect("line 3");
    keys(&mut screen, "vj");
    let (found, selected) = open(&mut screen, size, 'c', "comment on");
    let pane = screen.geometry.diff;
    assert!(
        found.y == selected[selected.len() - 1] + 1 || found.bottom() == selected[0],
        "{found:?} {selected:?}"
    );
    assert_eq!((found.x, found.width), (pane.x, pane.width));
    screen
        .handle_event(Event::Paste("a longer comment that wraps onto a few rows of the box".into()))
        .expect("paste");
    let found = the_box(&draw(&mut screen, size), "comment on");
    assert!(within(found, pane) && found.height >= 3, "{found:?} in {pane:?}");
    press(&mut screen, KeyCode::Enter);
    let note = screen.store.annotations().first().expect("saved");
    assert_eq!((note.line_start, note.line_end), (3, 4));
    // Lines on the last rows: above them.
    let shown = view(&mut screen, size).0;
    let bottom = shown.iter().rev().find_map(|row| row.as_ref().and_then(|row| row.2)).expect("a line");
    screen.diff.reveal("file0.py", Side::New, bottom).expect("bottom");
    press(&mut screen, KeyCode::Char('c'));
    let buffer = draw(&mut screen, size);
    let found = the_box(&buffer, "comment on");
    let at = cell_of(&mut screen, size, Side::New, bottom).1;
    assert!(within(found, screen.geometry.diff), "{found:?}");
    assert_eq!(found.bottom(), at, "above the cursor's line on the last row");
}

#[test]
fn a_file_note_opens_under_the_files_heading_and_the_whole_changes_in_the_middle() {
    let size = (COLS, ROWS);
    let fixture = long("near-file", false);
    let mut screen = fixture.open();
    let buffer = draw(&mut screen, size);
    let pane = screen.geometry.diff;
    let heading = (pane.top()..pane.bottom())
        .find(|&y| {
            (pane.left()..pane.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .contains("file0.py")
        })
        .expect("file0.py's heading");
    press(&mut screen, KeyCode::Char('C'));
    assert_eq!(screen.mode, Mode::Compose(Draft::File("file0.py".into())));
    let found = the_box(&draw(&mut screen, size), "note on file0.py");
    assert_eq!((found.x, found.y), (pane.x, heading + 1));
    press(&mut screen, KeyCode::Esc);
    // Scrolled into the file, its heading is the one kept over the top row.
    screen.diff.reveal("file0.py", Side::New, 70).expect("line 70");
    press(&mut screen, KeyCode::Char('C'));
    let found = the_box(&draw(&mut screen, size), "note on file0.py");
    assert_eq!(found.y, pane.y + 1, "under the heading on the top row");
    press(&mut screen, KeyCode::Esc);
    press(&mut screen, KeyCode::Char('M'));
    let found = the_box(&draw(&mut screen, size), "comment on the whole change");
    let centre = |a: u16, b: u16| (i32::from(a) - i32::from(b)).abs() <= 1;
    assert!(centre(found.x + found.width / 2, pane.x + pane.width / 2), "{found:?} {pane:?}");
    assert!(centre(found.y + found.height / 2, pane.y + pane.height / 2), "{found:?} {pane:?}");
}

#[test]
fn editing_a_note_from_the_rail_opens_the_box_under_its_lines() {
    let size = (COLS, ROWS);
    let fixture = long("near-edit", false);
    let mut screen = fixture.open();
    drag(&mut screen, size, Side::New, 20, 21);
    press(&mut screen, KeyCode::Char('c'));
    type_and_save(&mut screen, "first take");
    press(&mut screen, KeyCode::Tab);
    press(&mut screen, KeyCode::Char('e'));
    let buffer = draw(&mut screen, size);
    let found = the_box(&buffer, "edit");
    let last = cell_of(&mut screen, size, Side::New, 21).1;
    assert_eq!(found.y, last + 1);
    let pane = screen.geometry.diff;
    assert_eq!(found.x, if screen.diff.split() { new_pane(pane) } else { pane.x });
}
