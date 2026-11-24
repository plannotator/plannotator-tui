//! A release leaves the view where it is, whichever way the drag went. The diff is scrolled
//! well down from its top first, so a view that moved has somewhere to go: a drag up in the
//! middle of the pane once put the lines it selected on the pane's last rows when it was let
//! go. Up, down, a drag along one row, one after another, the reports one per frame or all
//! between two frames, split and unified, with wrapping on and off, small and large.

use ratatui::crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind};

use super::*;
use crate::review::diff::{Lines, Progress};

/// One file of 300 lines, every third changed, every seventh long enough to wrap: one hunk.
fn long(tag: &str) -> Fixture {
    let fixture = Fixture::new(tag);
    let line = |i: usize, changed: bool| {
        let tail =
            if i.is_multiple_of(7) { format!("  # {}", "long words ".repeat(16)) } else { String::new() };
        if changed { format!("line_{i} = {i} * 2{tail}\n") } else { format!("line_{i} = {i}{tail}\n") }
    };
    fixture.write("long.py", &(1..=300).map(|i| line(i, false)).collect::<String>());
    fixture.git(&["add", "."]);
    fixture.git(&["-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-qm", "long"]);
    fixture.write("long.py", &(1..=300).map(|i| line(i, i.is_multiple_of(3))).collect::<String>());
    fixture
}

type Size = (u16, u16);

fn draw(screen: &mut Screen, (cols, rows): Size) {
    let mut terminal = Terminal::new(TestBackend::new(cols, rows)).expect("terminal");
    let mut drawn = Ok(());
    terminal.draw(|f| drawn = screen.draw(f)).expect("draw");
    drawn.expect("viewer draw");
    screen.after_draw();
}

/// The column of the new side's code: near the pane's right edge, as a split's right pane
/// and a unified row both have it.
fn column(screen: &Screen) -> u16 {
    screen.geometry.diff.right() - 20
}

/// What each row of the pane shows, in a new frame: the new line and which of its rows.
fn view(screen: &mut Screen, size: Size) -> View {
    draw(screen, size);
    let pane = screen.geometry.diff;
    (pane.top()..pane.bottom())
        .map(|y| screen.diff.row_at(pane.right() - 2, y).map(|row| (row.new_line, row.part)))
        .collect()
}

/// The new line drawn on screen row `y` of the last frame, on its first row.
fn line_at(screen: &Screen, y: u16) -> Option<u32> {
    let row = screen.diff.row_at(screen.geometry.diff.right() - 2, y)?;
    (row.part == 0 && row.kind == diffs_rs::engine::DiffRowKind::Code).then_some(row.new_line).flatten()
}

fn report(screen: &mut Screen, kind: MouseEventKind, (column, row): (u16, u16), modifiers: KeyModifiers) {
    let event = MouseEvent { kind, column, row, modifiers };
    screen.handle_event(Event::Mouse(event)).expect("mouse");
}

/// Open the review at `size`, `keys` pressed, and scrolled `notches` of the wheel down.
fn opened(tag: &str, size: Size, keys: &str, notches: usize) -> (Fixture, Screen) {
    let fixture = long(tag);
    let screen = fixture.open();
    (fixture, ready_and_scrolled(screen, size, keys, notches))
}

/// `screen` with `keys` pressed, its viewer ready, and scrolled `notches` of the wheel down.
fn ready_and_scrolled(mut screen: Screen, size: Size, keys: &str, notches: usize) -> Screen {
    for key in keys.chars() {
        press(&mut screen, KeyCode::Char(key));
    }
    // The file's whole source arrives after its patch and lays the rows out again.
    let started = Instant::now();
    while screen.diff.progress() != Progress::Ready {
        assert!(started.elapsed() < Duration::from_secs(10), "the viewer never got ready");
        screen.tick().expect("tick");
        draw(&mut screen, size);
        std::thread::sleep(Duration::from_millis(10));
    }
    draw(&mut screen, size);
    let pane = screen.geometry.diff;
    let middle = (column(&screen), pane.y + pane.height / 2);
    for _ in 0..notches {
        report(&mut screen, MouseEventKind::ScrollDown, middle, KeyModifiers::NONE);
        draw(&mut screen, size);
    }
    screen
}

/// Two screen rows `rows` apart around the pane's middle that each start a line of code,
/// the upper one first.
fn rows_apart(screen: &mut Screen, size: Size, rows: u16) -> (u16, u16) {
    draw(screen, size);
    let pane = screen.geometry.diff;
    let start = pane.y + pane.height / 3;
    let y = (start..pane.bottom() - rows)
        .find(|&y| line_at(screen, y).is_some() && line_at(screen, y + rows).is_some())
        .unwrap_or_else(|| panic!("no two lines {rows} rows apart"));
    (y, y + rows)
}

/// Press on row `from`, go a row at a time to row `to` (along the row when they are the
/// same), let go there, with a frame after each report unless `burst`. The view before and
/// after, and the lines selected.
fn drag(screen: &mut Screen, size: Size, (from, to): (u16, u16), burst: bool) -> (View, View, Option<Lines>) {
    let before = view(screen, size);
    let x = column(screen);
    let (a, b) = (line_at(screen, from).expect("pressed line"), line_at(screen, to).expect("released line"));
    report(screen, MouseEventKind::Down(MouseButton::Left), (x, from), KeyModifiers::NONE);
    let rows: Vec<u16> = if to >= from { (from..=to).collect() } else { (to..=from).rev().collect() };
    let mut cells: Vec<(u16, u16)> = rows.into_iter().skip(1).map(|y| (x, y)).collect();
    if from == to {
        cells = (1..=8).map(|dx| (x + dx, to)).collect();
    }
    for &cell in &cells {
        report(screen, MouseEventKind::Drag(MouseButton::Left), cell, KeyModifiers::NONE);
        if !burst {
            draw(screen, size);
        }
    }
    let last = cells.last().copied().unwrap_or((x, to));
    report(screen, MouseEventKind::Up(MouseButton::Left), last, KeyModifiers::NONE);
    let after = view(screen, size);
    assert_eq!(
        screen.diff.selection(),
        Some(Lines { path: "long.py".into(), side: Side::New, first: a.min(b), last: a.max(b) })
    );
    assert!(screen.geometry.toolbar_bar.is_some(), "the toolbar is up");
    (before, after, screen.diff.selection())
}

type View = Vec<Option<(Option<u32>, usize)>>;

/// The rows both views have. A narrow footer can take a row more or less once lines are
/// selected (its hints change), so the pane can end a row higher or lower; what is drawn on
/// each row it keeps must not change.
fn kept(before: &View, after: &View) -> (View, View) {
    let rows = before.len().min(after.len());
    (before[..rows].to_vec(), after[..rows].to_vec())
}

const SIZES: [Size; 5] = [(200, 60), (250, 70), (180, 50), (120, 40), (100, 30)];

/// Every kind of drag in the middle of the pane, at every size, with `keys` pressed first.
fn every_drag_keeps_the_view(tag: &str, keys: &str) {
    for size in SIZES {
        for burst in [false, true] {
            let name = format!("{size:?} {keys:?} burst {burst}");
            let (_fixture, mut screen) =
                opened(&format!("{tag}-{}-{}-{burst}", size.0, size.1), size, keys, 8);
            let (top, bottom) = rows_apart(&mut screen, size, 2);
            for (what, rows) in [("up", (bottom, top)), ("down", (top, bottom)), ("along a row", (top, top))]
            {
                let (before, after, _) = drag(&mut screen, size, rows, burst);
                let (before, after) = kept(&before, &after);
                assert_eq!(after, before, "{name}: the view moved on the release of a drag {what}");
                press(&mut screen, KeyCode::Esc);
            }
            // A drag down, then one up while the first one's selection and toolbar stand.
            let (before, after, _) = drag(&mut screen, size, (top, bottom), burst);
            let (before, after) = kept(&before, &after);
            assert_eq!(after, before, "{name}: the first drag");
            let (before, after, _) = drag(&mut screen, size, (bottom, top), burst);
            let (before, after) = kept(&before, &after);
            assert_eq!(after, before, "{name}: a drag up after a drag down");
        }
    }
}

#[test]
fn a_release_in_a_split_diff_never_moves_the_view() {
    every_drag_keeps_the_view("release-split", "");
}

#[test]
fn a_release_in_a_unified_diff_never_moves_the_view() {
    every_drag_keeps_the_view("release-unified", "u");
}

#[test]
fn a_release_with_wrapping_on_never_moves_the_view() {
    every_drag_keeps_the_view("release-wrap", "w");
}

#[test]
fn a_release_in_a_unified_diff_with_wrapping_on_never_moves_the_view() {
    every_drag_keeps_the_view("release-unified-wrap", "uw");
}

#[test]
fn a_shift_click_above_the_line_first_pressed_leaves_the_view_where_it_is() {
    let size = (200, 60);
    let (_fixture, mut screen) = opened("release-shift", size, "", 8);
    let (top, bottom) = rows_apart(&mut screen, size, 6);
    drag(&mut screen, size, (bottom - 2, bottom), false);
    let before = view(&mut screen, size);
    let x = column(&screen);
    report(&mut screen, MouseEventKind::Down(MouseButton::Left), (x, top), KeyModifiers::SHIFT);
    report(&mut screen, MouseEventKind::Up(MouseButton::Left), (x, top), KeyModifiers::SHIFT);
    let first = before[usize::from(top - screen.geometry.diff.y)].and_then(|(line, _)| line);
    assert_eq!(screen.diff.selection().map(|lines| lines.first), first, "grew up to the clicked line");
    let (before, after) = kept(&before, &view(&mut screen, size));
    assert_eq!(after, before, "the view stayed");
}

/// One file of 300 tab-indented lines: the first six re-indented with spaces and nothing else
/// (a hunk of whitespace changes only), then from line 20 every third re-indented, every
/// ninth of those changed too: one hunk of pairs drawn as unchanged and real edits.
fn reindented(tag: &str) -> Fixture {
    let fixture = Fixture::new(tag);
    let before = (1..=300).map(|i| format!("\tline_{i} = {i}\n")).collect::<Vec<_>>().concat();
    let after: String = (1..=300)
        .map(|i| match i {
            1..=6 => format!("    line_{i} = {i}\n"),
            20.. if i % 9 == 0 => format!("    line_{i} = {i} * 2\n"),
            20.. if i % 3 == 0 => format!("    line_{i} = {i}\n"),
            _ => format!("\tline_{i} = {i}\n"),
        })
        .collect();
    fixture.write("long.py", &before);
    fixture.git(&["add", "."]);
    fixture.git(&["-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-qm", "tabs"]);
    fixture.write("long.py", &after);
    fixture
}

#[test]
fn with_whitespace_hidden_a_drag_up_after_scrolling_selects_its_lines_keeps_the_view_and_comments_on_them() {
    use diffs_rs::engine::{InlineChanges, Whitespace};
    let size = (160, 50);
    for (width, keys) in [(2, ""), (8, ""), (2, "u"), (8, "u")] {
        let name = format!("tab width {width}, keys {keys:?}");
        let fixture = reindented(&format!("release-ws-{width}-{keys}"));
        fixture.write_config(&format!(
            "[ui]\ncopy_on_select = false\n[review]\nwhitespace = \"ignore-all\"\ntab_width = {width}\ninline_changes = \"none\"\n"
        ));
        let mut screen = ready_and_scrolled(fixture.open_configured(), size, keys, 8);
        assert_eq!(screen.diff.drawn_with(), (width, InlineChanges::None, Whitespace::IgnoreAll), "{name}");
        assert!(screen.diff.reveal("long.py", Side::New, 3).is_err(), "{name}: the first hunk is a heading");
        for burst in [false, true] {
            let (top, bottom) = rows_apart(&mut screen, size, 6);
            let (before, after, lines) = drag(&mut screen, size, (bottom, top), burst);
            let (before, after) = kept(&before, &after);
            assert_eq!(after, before, "{name}, burst {burst}: the view moved on the release of a drag up");
            let lines = lines.expect("selected");
            let pair = (lines.first..=lines.last).find(|i| i % 3 == 0 && i % 9 != 0);
            assert!(pair.is_some(), "{name}: the drag took a pair drawn as unchanged: {lines:?}");
            press(&mut screen, KeyCode::Char('c'));
            type_and_save(&mut screen, &format!("{name} {burst}"));
            let note = screen.store.annotations().last().expect("saved");
            assert_eq!(
                (note.file_path.as_str(), note.side, note.line_start, note.line_end),
                ("long.py", Side::New, lines.first, lines.last),
                "{name}, burst {burst}"
            );
        }
    }
}
