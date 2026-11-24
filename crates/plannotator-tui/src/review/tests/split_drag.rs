//! The line between a split diff's sides: a drag shares the width between them, within each
//! side's least width, and never moves the cursor, scrolls or selects; a burst of drag events
//! before a frame ends where one event a frame does; a double click (two clicks on one cell)
//! makes the sides even; a key drops the drag and keeps the ratio so far; the line lights
//! under the pointer; `(`/`)` move it; and the ratio is written once, when the drag ends.

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::style::Color;

use super::*;
use crate::config::review::ReviewConfig;

const COLS: u16 = 160;
const ROWS: u16 = 30;

fn buffer(screen: &mut Screen, cols: u16, rows: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(cols, rows)).expect("terminal");
    let mut drawn = Ok(());
    terminal.draw(|f| drawn = screen.draw(f)).expect("draw");
    drawn.expect("viewer draw");
    screen.after_draw();
    terminal.backend().buffer().clone()
}

fn mouse(screen: &mut Screen, kind: MouseEventKind, column: u16, row: u16) {
    let event = MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE };
    screen.handle_event(Event::Mouse(event)).expect("mouse");
}

const DOWN: MouseEventKind = MouseEventKind::Down(MouseButton::Left);
const DRAG: MouseEventKind = MouseEventKind::Drag(MouseButton::Left);
const UP: MouseEventKind = MouseEventKind::Up(MouseButton::Left);

/// A split review, wrapped or not, drawn once at 160×30.
fn open(tag: &str, wrap: bool) -> (Fixture, Screen) {
    let fixture = Fixture::new(tag);
    fixture.write_config(&format!("[review]\nlayout = \"split\"\nwrap = {wrap}\n"));
    let mut screen = fixture.open_configured();
    buffer(&mut screen, COLS, ROWS);
    (fixture, screen)
}

/// The line's column and a row of code it is drawn on, from the last frame.
fn line(screen: &mut Screen) -> (u16, u16) {
    let drawn = buffer(screen, COLS, ROWS);
    let column = screen.diff.split_line().expect("split");
    let row = (1..ROWS)
        .find(|&y| drawn.cell((column, y)).is_some_and(|c| c.symbol() == "\u{2502}"))
        .expect("a row with the line");
    (column, row)
}

/// The split ratio the next review opens with, in thousandths: the remembered one.
fn ratio_written(fixture: &Fixture) -> u16 {
    let (settings, notice) = crate::review::settings::state::load(&fixture.state(), ReviewConfig::DEFAULT);
    assert_eq!(notice, None);
    settings.split_ratio.permille()
}

/// The ratio a drag to `cells` of `room` remembers: to the cell, then to two decimals.
fn remembered(cells: u32, room: u32) -> u16 {
    let permille = u16::try_from((cells * 1000).div_ceil(room)).expect("small");
    (permille + 5) / 10 * 10
}

/// The first columns of the diff on each row: the old side's line numbers and the start of its
/// code, left of anywhere the line can be (each side keeps 20 columns). The scroll position,
/// independent of where the line is.
fn rows_shown(screen: &mut Screen) -> Vec<String> {
    let rows = frame(screen, COLS, ROWS);
    let left = usize::from(screen.geometry.diff.x);
    rows.into_iter().map(|row| row.chars().skip(left).take(16).collect()).collect()
}

#[test]
fn dragging_the_line_shares_the_width_and_moves_nothing_else() {
    for wrap in [false, true] {
        let (fixture, mut screen) = open(&format!("split-drag-{wrap}"), wrap);
        at(&mut screen, 9);
        let (column, row) = line(&mut screen);
        assert_eq!(column, (COLS - 1) / 2, "even to start with");
        let cursor = screen.diff.cursor().expect("cursor");
        let before = rows_shown(&mut screen);
        let cursor_row = screen.diff.cursor_row();
        mouse(&mut screen, DOWN, column, row);
        assert!(screen.dividers.dragging_split());
        mouse(&mut screen, DRAG, column - 30, row);
        mouse(&mut screen, DRAG, column - 40, row + 3);
        assert!(!fixture.state().exists(), "nothing written mid-drag");
        mouse(&mut screen, UP, column - 40, row + 3);
        let (moved, _) = line(&mut screen);
        assert_eq!(moved, column - 40, "{wrap}");
        assert_eq!(screen.diff.cursor().expect("cursor"), cursor, "{wrap}: the cursor stays");
        assert_eq!(screen.diff.cursor_row().map(|r| (r.file, r.side)), cursor_row.map(|r| (r.file, r.side)));
        assert!(screen.diff.selection().is_none() && screen.drag.is_none() && screen.toolbar.is_none());
        if !wrap {
            assert_eq!(rows_shown(&mut screen), before, "nothing scrolled");
        }
        assert_eq!(ratio_written(&fixture), remembered(39, 159), "to two decimals");
    }
}

#[test]
fn a_burst_of_drag_events_before_a_frame_ends_where_one_event_a_frame_ends() {
    for wrap in [false, true] {
        let (_a, mut burst) = open(&format!("split-burst-{wrap}"), wrap);
        let (_b, mut paced) = open(&format!("split-paced-{wrap}"), wrap);
        for screen in [&mut burst, &mut paced] {
            keys(screen, "jjjj");
            buffer(screen, COLS, ROWS);
        }
        let (column, row) = line(&mut burst);
        let path: Vec<u16> = (0..100).map(|n| (column + 60).saturating_sub(n * 7 % 130)).collect();
        for (screen, paced) in [(&mut burst, false), (&mut paced, true)] {
            mouse(screen, DOWN, column, row);
            for (n, &x) in path.iter().enumerate() {
                mouse(screen, DRAG, x, row + (n % 5) as u16);
                if paced {
                    buffer(screen, COLS, ROWS);
                }
            }
            mouse(screen, UP, *path.last().expect("a path"), row);
        }
        assert_eq!(frame(&mut burst, COLS, ROWS), frame(&mut paced, COLS, ROWS), "{wrap}");
        assert_eq!(burst.diff.split_line(), paced.diff.split_line());
        assert_eq!(burst.diff.cursor().expect("cursor"), paced.diff.cursor().expect("cursor"));
        assert!(burst.diff.selection().is_none() && burst.drag.is_none());
    }
}

#[test]
fn the_line_stops_where_either_side_would_lose_its_least_width() {
    let (fixture, mut screen) = open("split-limits", false);
    let (column, row) = line(&mut screen);
    mouse(&mut screen, DOWN, column, row);
    mouse(&mut screen, DRAG, 0, row);
    mouse(&mut screen, UP, 0, row);
    assert_eq!(line(&mut screen).0, 20, "the old side keeps 20 columns");
    let (column, row) = line(&mut screen);
    mouse(&mut screen, DOWN, column, row);
    mouse(&mut screen, DRAG, COLS - 1, row);
    mouse(&mut screen, UP, COLS - 1, row);
    assert_eq!(line(&mut screen).0, COLS - 1 - 20, "and so does the new side");
    // Narrower, the ratio set holds and the line keeps the sides' least width.
    let drawn = buffer(&mut screen, 60, ROWS);
    let narrow = screen.diff.split_line().expect("split");
    assert_eq!(narrow, 39);
    assert_eq!(drawn.cell((narrow, row)).map(|c| c.symbol().to_owned()), Some("\u{2502}".to_owned()));
    assert_eq!(ratio_written(&fixture), 880, "139 of 159 cells: 0.875, then 0.88");
}

#[test]
fn a_double_click_on_one_cell_evens_the_sides_and_a_drag_back_is_no_click() {
    let (fixture, mut screen) = open("split-double", false);
    let (column, row) = line(&mut screen);
    mouse(&mut screen, DOWN, column, row);
    mouse(&mut screen, DRAG, column + 25, row);
    mouse(&mut screen, UP, column + 25, row);
    let (moved, row) = line(&mut screen);
    assert_eq!(moved, column + 25);
    // Two presses that each drag away and back are no clicks.
    for _ in 0..2 {
        mouse(&mut screen, DOWN, moved, row);
        mouse(&mut screen, DRAG, moved + 2, row);
        mouse(&mut screen, DRAG, moved, row);
        mouse(&mut screen, UP, moved, row);
    }
    assert_eq!(line(&mut screen).0, moved);
    for _ in 0..2 {
        mouse(&mut screen, DOWN, moved, row);
        mouse(&mut screen, UP, moved, row);
    }
    assert_eq!(line(&mut screen).0, column, "even again");
    assert_eq!(ratio_written(&fixture), 500);
    assert!(screen.diff.selection().is_none());
}

#[test]
fn a_key_drops_the_drag_and_keeps_the_ratio_dragged_to_so_far() {
    let (fixture, mut screen) = open("split-cancel", false);
    let (column, row) = line(&mut screen);
    let cursor = screen.diff.cursor().expect("cursor");
    mouse(&mut screen, DOWN, column, row);
    mouse(&mut screen, DRAG, column - 10, row);
    press(&mut screen, KeyCode::Esc);
    assert!(!screen.dividers.dragging_split());
    mouse(&mut screen, DRAG, column - 50, row);
    mouse(&mut screen, UP, column - 50, row);
    assert_eq!(line(&mut screen).0, column - 10, "the later drag moves nothing");
    assert_eq!(screen.diff.cursor().expect("cursor"), cursor);
    assert!(screen.diff.selection().is_none() && screen.drag.is_none());
    assert_eq!(ratio_written(&fixture), remembered(69, 159));
}

#[test]
fn the_line_lights_under_the_pointer_and_while_dragged() {
    let (_fixture, mut screen) = open("split-hover", false);
    let (column, row) = line(&mut screen);
    let accent = crate::theme::palette().accent;
    let colour = |screen: &mut Screen| -> Color {
        buffer(screen, COLS, ROWS).cell((screen.diff.split_line().expect("split"), row)).expect("cell").fg
    };
    let plain = colour(&mut screen);
    assert_ne!(plain, accent);
    mouse(&mut screen, MouseEventKind::Moved, column, row);
    assert_eq!(colour(&mut screen), accent);
    mouse(&mut screen, MouseEventKind::Moved, column + 5, row);
    assert_eq!(colour(&mut screen), plain);
    mouse(&mut screen, DOWN, column, row);
    mouse(&mut screen, DRAG, column + 9, row + 2);
    assert_eq!(colour(&mut screen), accent, "lit while dragged, wherever the pointer");
    mouse(&mut screen, UP, column + 9, row + 2);
    screen.handle_event(Event::FocusLost).expect("focus");
    assert_eq!(colour(&mut screen), plain);
}

#[test]
fn parentheses_move_the_line_and_a_unified_diff_says_they_need_the_split() {
    let (fixture, mut screen) = open("split-keys", false);
    let (column, _) = line(&mut screen);
    keys(&mut screen, ")");
    assert_eq!(line(&mut screen).0, 87);
    keys(&mut screen, "((");
    assert_eq!(line(&mut screen).0, 71);
    assert_eq!(ratio_written(&fixture), 450);
    assert!(column > 71);
    keys(&mut screen, "u");
    buffer(&mut screen, COLS, ROWS);
    keys(&mut screen, ")");
    assert_eq!(screen.status.as_deref(), Some("the split ratio applies to the split layout"));
    assert_eq!(ratio_written(&fixture), 450);
}

#[test]
fn selected_lines_are_shaded_up_to_the_line_wherever_it_is() {
    let (_fixture, mut screen) = open("split-shade", false);
    let (column, row) = line(&mut screen);
    mouse(&mut screen, DOWN, column, row);
    mouse(&mut screen, DRAG, column - 35, row);
    mouse(&mut screen, UP, column - 35, row);
    let (line_at, _) = line(&mut screen);
    at(&mut screen, 9);
    keys(&mut screen, "vjv");
    let drawn = buffer(&mut screen, COLS, ROWS);
    let shade = crate::review::mouse::selection_bg();
    let shaded: Vec<u16> =
        (0..ROWS).filter(|&y| drawn.cell((line_at + 5, y)).is_some_and(|c| c.bg == shade)).collect();
    assert_eq!(shaded.len(), 2, "two new lines selected");
    for y in shaded {
        assert_eq!(drawn.cell((line_at, y)).map(|c| c.symbol().to_owned()), Some("\u{2502}".to_owned()));
        assert_ne!(drawn.cell((line_at, y)).map(|c| c.bg), Some(shade), "the line is not shaded");
        assert_ne!(drawn.cell((line_at - 1, y)).map(|c| c.bg), Some(shade), "nor the old side");
        assert_eq!(drawn.cell((COLS - 1, y)).map(|c| c.bg), Some(shade), "the new side to its end");
    }
}

#[test]
fn the_line_dragged_past_the_diffs_edges_or_off_the_screen_never_scrolls() {
    let (fixture, mut screen) = open("split-edges", false);
    let (column, row) = line(&mut screen);
    let before = rows_shown(&mut screen);
    let cursor = screen.diff.cursor().expect("cursor");
    mouse(&mut screen, DOWN, column, row);
    mouse(&mut screen, DRAG, column - 20, ROWS - 1);
    // A report no terminal reporting cells sends is ignored; it is not the pointer past an edge.
    mouse(&mut screen, DRAG, 900, 900);
    assert!(!screen.held_at_edge(), "a line's drag has no edge to hold");
    for step in 1..=10u32 {
        let later = Instant::now() + crate::ui::drag::STEP * step;
        assert!(!screen.step_at_edge(later).expect("step"));
    }
    mouse(&mut screen, DRAG, column - 20, 0);
    mouse(&mut screen, UP, column - 20, 0);
    assert_eq!(line(&mut screen).0, column - 20, "the column the pointer was in counts, any row");
    assert_eq!(screen.diff.cursor().expect("cursor"), cursor);
    assert!(screen.diff.selection().is_none() && screen.drag.is_none());
    // The line moved, so compare what stays left of both places it was drawn.
    let after = rows_shown(&mut screen);
    assert_eq!(after, before, "nothing scrolled");
    assert_eq!(ratio_written(&fixture), remembered(59, 159));
}
