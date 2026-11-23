//! A drag as a terminal in a window reports it: a report for each pixel the pointer moves, so
//! many for one cell, and many between two frames. Inside the pane the view never moves; held
//! past its edge it scrolls a line per step of a timer, and no further than the drag's hunk.
//! On a long diff at 160×45, where the diff is split.

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use super::*;
use crate::review::diff::Lines;
use crate::review::mouse::edge::STEP;

const COLS: u16 = 160;
const ROWS: u16 = 45;

/// Four files of 200 lines, each changed in two blocks: its first hunk is lines 1 to 92.
fn long(tag: &str) -> Fixture {
    let fixture = Fixture::new(tag);
    let line = |f: usize, i: usize, changed: bool| {
        if changed {
            format!("f{f}_line_{i} = {i} * 2  # changed\n")
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

fn open(tag: &str) -> (Fixture, Screen) {
    let fixture = long(tag);
    let mut screen = fixture.open();
    draw(&mut screen);
    (fixture, screen)
}

fn draw(screen: &mut Screen) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(COLS, ROWS)).expect("terminal");
    let mut drawn = Ok(());
    terminal.draw(|f| drawn = screen.draw(f)).expect("draw");
    drawn.expect("viewer draw");
    screen.after_draw();
    terminal.backend().buffer().clone()
}

/// What each row of the pane shows, as the viewer drew it: the file and the new line.
fn view(screen: &mut Screen) -> Vec<Option<(String, Option<u32>)>> {
    draw(screen);
    let pane = screen.geometry.diff;
    (pane.top()..pane.bottom())
        .map(|y| screen.diff.row_at(pane.right() - 2, y).map(|row| (row.path, row.new_line)))
        .collect()
}

/// The cell on the new side where new line `line` of file0.py is drawn.
fn cell_of(screen: &mut Screen, line: u32) -> (u16, u16) {
    draw(screen);
    let pane = screen.geometry.diff;
    let y = (pane.top()..pane.bottom())
        .find(|&y| {
            screen
                .diff
                .row_at(pane.right() - 2, y)
                .is_some_and(|row| row.path == "file0.py" && row.new_line == Some(line))
        })
        .unwrap_or_else(|| panic!("line {line} is not on screen"));
    (pane.right() - 20, y)
}

fn report(screen: &mut Screen, kind: MouseEventKind, (column, row): (u16, u16)) {
    let event = MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE };
    screen.handle_event(Event::Mouse(event)).expect("mouse");
}

fn moved(screen: &mut Screen, at: (u16, u16)) {
    report(screen, MouseEventKind::Drag(MouseButton::Left), at);
}

fn new_lines(first: u32, last: u32) -> Lines {
    Lines { path: "file0.py".into(), side: Side::New, first, last }
}

/// A pointer going from `from` to `to` a row at a time, reported five times in each cell and
/// over two columns of it, as a terminal reporting pixels does.
fn wiggle(from: (u16, u16), to: (u16, u16)) -> Vec<(u16, u16)> {
    let rows: Vec<u16> =
        if to.1 >= from.1 { (from.1..=to.1).collect() } else { (to.1..=from.1).rev().collect() };
    rows.into_iter().flat_map(|y| [0, 0, 1, 1, 0].map(|dx| (to.0 + dx, y))).collect()
}

#[test]
fn a_burst_of_drag_reports_inside_the_pane_moves_no_view_and_selects_press_to_pointer() {
    let (_fixture, mut screen) = open("burst-middle");
    let before = view(&mut screen);
    let from = cell_of(&mut screen, 20);
    let to = cell_of(&mut screen, 27);
    report(&mut screen, MouseEventKind::Down(MouseButton::Left), from);
    // Past the pointer's line and back, with no frame between the reports.
    for at in wiggle(from, (to.0, to.1 + 4)).into_iter().chain(wiggle((to.0, to.1 + 4), to)) {
        moved(&mut screen, at);
    }
    assert_eq!(screen.selected_lines(), Some(new_lines(20, 27)));
    assert_eq!(view(&mut screen), before, "the view stayed");
    report(&mut screen, MouseEventKind::Up(MouseButton::Left), to);
    assert_eq!(screen.diff.selection(), Some(new_lines(20, 27)));
    assert_eq!(view(&mut screen), before, "the view stayed after the release");
}

#[test]
fn reports_in_one_burst_end_as_the_same_reports_one_per_frame() {
    let (_fixture, mut framed) = open("burst-framed");
    let (_other, mut burst) = open("burst-burst");
    let from = cell_of(&mut framed, 12);
    let pane = framed.geometry.diff;
    // Down past the bottom, up past the top, and back to the middle.
    let path: Vec<(u16, u16)> = [
        wiggle(from, (from.0, pane.bottom() + 1)),
        wiggle((from.0, pane.bottom() + 1), (from.0, pane.y - 1)),
        wiggle((from.0, pane.y - 1), (from.0, from.1 + 6)),
    ]
    .concat();
    report(&mut framed, MouseEventKind::Down(MouseButton::Left), from);
    report(&mut burst, MouseEventKind::Down(MouseButton::Left), from);
    for &at in &path {
        moved(&mut framed, at);
        draw(&mut framed);
        moved(&mut burst, at);
    }
    assert_eq!(burst.selected_lines(), framed.selected_lines());
    assert_eq!(burst.selected_lines(), Some(new_lines(12, 18)));
    assert_eq!(view(&mut burst), view(&mut framed));
}

#[test]
fn reports_for_the_pressed_cell_leave_a_click_a_click() {
    let (_fixture, mut screen) = open("burst-click");
    let at = cell_of(&mut screen, 20);
    report(&mut screen, MouseEventKind::Down(MouseButton::Left), at);
    for _ in 0..50 {
        moved(&mut screen, at);
    }
    report(&mut screen, MouseEventKind::Up(MouseButton::Left), at);
    assert_eq!(screen.diff.selection(), None);
    assert_eq!(screen.diff.cursor().expect("cursor"), Some(new_lines(20, 20)));
}

#[test]
fn a_press_on_any_row_of_the_pane_leaves_the_view_where_it_is() {
    let (_fixture, mut screen) = open("burst-press");
    let before = view(&mut screen);
    let pane = screen.geometry.diff;
    for y in pane.top()..pane.bottom() {
        report(&mut screen, MouseEventKind::Down(MouseButton::Left), (pane.right() - 20, y));
        report(&mut screen, MouseEventKind::Up(MouseButton::Left), (pane.right() - 20, y));
        assert_eq!(view(&mut screen), before, "after a press on row {y}");
    }
}

#[test]
fn held_past_the_bottom_the_view_scrolls_a_line_per_step_however_many_reports() {
    let (_fixture, mut screen) = open("burst-edge");
    let from = cell_of(&mut screen, 20);
    let pane = screen.geometry.diff;
    report(&mut screen, MouseEventKind::Down(MouseButton::Left), from);
    for at in wiggle(from, (from.0, pane.bottom() - 1)) {
        moved(&mut screen, at);
    }
    let before = view(&mut screen);
    let last =
        |view: &[Option<(String, Option<u32>)>]| view.last().cloned().flatten().and_then(|(_, line)| line);
    let bottom = last(&before).expect("a line on the last row");
    // Hundreds of reports past the bottom, along it and below it: none scrolls by itself.
    for k in 0..300u16 {
        moved(&mut screen, (from.0 + k % 7, pane.bottom() + k % 2));
    }
    assert_eq!(view(&mut screen), before);
    assert!(screen.held_at_edge());
    // The timer scrolls a line per step, and not again within a step.
    let start = Instant::now();
    for (offset, scrolls) in [(0, true), (10, false), (49, false), (50, true), (60, false), (100, true)] {
        let stepped = screen.step_at_edge(start + Duration::from_millis(offset)).expect("step");
        assert_eq!(stepped, scrolls, "at {offset} ms");
        draw(&mut screen);
    }
    assert_eq!(last(&view(&mut screen)), Some(bottom + 3));
    assert_eq!(screen.selected_lines(), Some(new_lines(20, bottom + 3)));
    // Back inside the pane the timer stops.
    moved(&mut screen, (from.0, pane.bottom() - 2));
    assert!(!screen.held_at_edge());
    let still = view(&mut screen);
    assert!(!screen.step_at_edge(start + STEP * 10).expect("step"));
    assert_eq!(view(&mut screen), still);
}

#[test]
fn held_past_the_bottom_the_view_stops_at_the_end_of_the_drags_hunk() {
    let (_fixture, mut screen) = open("burst-end");
    let from = cell_of(&mut screen, 20);
    let pane = screen.geometry.diff;
    report(&mut screen, MouseEventKind::Down(MouseButton::Left), from);
    moved(&mut screen, (from.0, pane.bottom() + 1));
    let reached = screen.selected_lines().expect("dragging").last;
    let start = Instant::now();
    let mut steps = 0;
    for n in 0..200 {
        steps += u32::from(screen.step_at_edge(start + STEP * n).expect("step"));
        draw(&mut screen);
    }
    assert_eq!(steps, 92 - reached, "a line a step from line {reached}, to the hunk's end");
    assert_eq!(screen.selected_lines(), Some(new_lines(20, 92)));
    let shown = view(&mut screen);
    assert!(
        shown.iter().flatten().all(|(path, _)| path == "file0.py"),
        "the view stayed in file0.py: {shown:?}"
    );
    report(&mut screen, MouseEventKind::Up(MouseButton::Left), (from.0, pane.bottom() + 1));
    assert_eq!(screen.diff.selection(), Some(new_lines(20, 92)));
}

#[test]
fn other_buttons_the_horizontal_wheel_and_focus_reports_during_a_drag_scroll_nothing() {
    let (_fixture, mut screen) = open("burst-odd");
    let from = cell_of(&mut screen, 20);
    let to = cell_of(&mut screen, 24);
    let pane = screen.geometry.diff;
    report(&mut screen, MouseEventKind::Down(MouseButton::Left), from);
    for at in wiggle(from, to) {
        moved(&mut screen, at);
    }
    let before = view(&mut screen);
    let kinds = [
        MouseEventKind::Drag(MouseButton::Right),
        MouseEventKind::Drag(MouseButton::Middle),
        MouseEventKind::Up(MouseButton::Right),
        MouseEventKind::Up(MouseButton::Middle),
        MouseEventKind::ScrollLeft,
        MouseEventKind::ScrollRight,
    ];
    for kind in kinds {
        for row in [pane.y - 1, from.1, pane.bottom() - 1, pane.bottom(), pane.bottom() + 5] {
            report(&mut screen, kind, (to.0, row));
        }
    }
    screen.handle_event(Event::FocusGained).expect("focus");
    assert!(!screen.held_at_edge());
    assert_eq!(view(&mut screen), before);
    report(&mut screen, MouseEventKind::Up(MouseButton::Left), to);
    assert_eq!(screen.diff.selection(), Some(new_lines(20, 24)));
}
