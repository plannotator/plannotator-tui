//! The mouse: the wheel, clicks on lines, files, notes and buttons, and a drag that selects
//! lines for a comment, in the diff and in the whole file. Every position is found in the
//! frame the screen drew, as a terminal's click would land on it.

use plannotator_tui_review::Side;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind};
use ratatui::style::Modifier;

use super::whole_flow::settle_whole;
use super::*;
use crate::review::diff::Lines;
use crate::review::{Draft, Focus, Mode};

const COLS: u16 = 120;
const ROWS: u16 = 40;

fn draw(screen: &mut Screen) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(COLS, ROWS)).expect("terminal");
    let mut drawn = Ok(());
    terminal.draw(|f| drawn = screen.draw(f)).expect("draw");
    drawn.expect("viewer draw");
    screen.after_draw();
    terminal.backend().buffer().clone()
}

/// The cell where `needle` starts in a fresh frame: the leftmost match, or with `last` the
/// rightmost (the new side of a split diff).
fn locate(screen: &mut Screen, needle: &str, last: bool) -> (u16, u16) {
    let buffer = draw(screen);
    let mut found = Vec::new();
    for y in 0..ROWS {
        let cells: Vec<String> = (0..COLS).map(|x| buffer[(x, y)].symbol().to_owned()).collect();
        let row = cells.concat();
        for (byte, _) in row.match_indices(needle) {
            let mut width = 0;
            let mut x = 0u16;
            while width < byte {
                width += cells[usize::from(x)].len();
                x += 1;
            }
            found.push((x, y));
        }
    }
    let at = if last { found.last() } else { found.first() };
    *at.unwrap_or_else(|| panic!("no {needle:?} on screen"))
}

fn mouse(screen: &mut Screen, kind: MouseEventKind, (column, row): (u16, u16)) {
    let event = MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE };
    screen.handle_event(Event::Mouse(event)).expect("mouse");
}

/// Press at `from` and move, a row at a time, to `to`, without letting go.
fn drag(screen: &mut Screen, from: (u16, u16), to: (u16, u16)) {
    mouse(screen, MouseEventKind::Down(MouseButton::Left), from);
    let rows: Vec<u16> =
        if to.1 >= from.1 { (from.1..=to.1).collect() } else { (to.1..=from.1).rev().collect() };
    for row in rows {
        mouse(screen, MouseEventKind::Drag(MouseButton::Left), (to.0, row));
    }
}

fn release(screen: &mut Screen, at: (u16, u16)) {
    mouse(screen, MouseEventKind::Up(MouseButton::Left), at);
}

fn lines(side: Side, first: u32, last: u32) -> Lines {
    Lines { path: "app.py".into(), side, first, last }
}

fn selection(screen: &Screen) -> Option<Lines> {
    screen.diff.selection()
}

#[test]
fn a_drag_selects_whole_lines_on_its_side_and_the_comment_lands_on_them() {
    let fixture = Fixture::new("mouse-drag");
    let mut screen = fixture.open();
    let from = locate(&mut screen, "def add(a, b):", true);
    let to = locate(&mut screen, "return total", true);
    drag(&mut screen, (from.0 + 3, from.1), to);
    release(&mut screen, to);
    assert_eq!(selection(&screen), Some(lines(Side::New, 8, 10)));
    press(&mut screen, KeyCode::Char('c'));
    assert_eq!(screen.mode, Mode::Compose(Draft::Comment(lines(Side::New, 8, 10))));
    type_and_save(&mut screen, "split this up");
    let note = screen.store.annotations().first().expect("a note");
    assert_eq!((note.line_start, note.line_end, note.side), (8, 10, Side::New));
    assert_eq!(note.file_path, "app.py");
}

#[test]
fn a_drag_that_starts_on_the_old_side_stays_on_it() {
    let fixture = Fixture::new("mouse-old");
    let mut screen = fixture.open();
    let from = locate(&mut screen, "def add(a, b):", false);
    let to = locate(&mut screen, "return a + b", false);
    // The pointer wanders to the new side; the lines stay old ones.
    drag(&mut screen, from, (COLS - 10, to.1));
    release(&mut screen, to);
    assert_eq!(selection(&screen), Some(lines(Side::Old, 5, 6)));
}

#[test]
fn dragging_up_selects_from_the_pointer_down_to_the_pressed_line() {
    let fixture = Fixture::new("mouse-up");
    let mut screen = fixture.open();
    let from = locate(&mut screen, "return total", true);
    let to = locate(&mut screen, "def add(a, b):", true);
    drag(&mut screen, from, to);
    release(&mut screen, to);
    assert_eq!(selection(&screen), Some(lines(Side::New, 8, 10)));
}

#[test]
fn a_click_moves_the_cursor_and_picks_the_side_without_selecting() {
    let fixture = Fixture::new("mouse-click");
    let mut screen = fixture.open();
    let new = locate(&mut screen, "return total", true);
    mouse(&mut screen, MouseEventKind::Down(MouseButton::Left), new);
    release(&mut screen, new);
    assert_eq!(screen.diff.cursor().expect("cursor"), Some(lines(Side::New, 10, 10)));
    assert_eq!(selection(&screen), None);
    let old = locate(&mut screen, "return a + b", false);
    mouse(&mut screen, MouseEventKind::Down(MouseButton::Left), old);
    release(&mut screen, old);
    assert_eq!(screen.diff.cursor().expect("cursor"), Some(lines(Side::Old, 6, 6)));
}

#[test]
fn a_new_press_clears_the_last_selection() {
    let fixture = Fixture::new("mouse-reselect");
    let mut screen = fixture.open();
    let from = locate(&mut screen, "def add(a, b):", true);
    let to = locate(&mut screen, "return total", true);
    drag(&mut screen, from, to);
    release(&mut screen, to);
    mouse(&mut screen, MouseEventKind::Down(MouseButton::Left), from);
    release(&mut screen, from);
    assert_eq!(selection(&screen), None);
}

/// The new-side pane of the row holding `needle`: from the cell after the split's middle
/// line to the frame's edge.
fn new_pane(buffer: &Buffer, (x, y): (u16, u16)) -> std::ops::Range<u16> {
    let middle = (0..x).rev().find(|&c| buffer[(c, y)].symbol() == "\u{2502}").expect("a split row");
    middle + 1..COLS
}

/// Each of `rows` is one block of the selection colour on its new side, edge to edge: no
/// bar in the gutter, no underline, and the old side and the middle line left alone.
fn assert_selected_block(buffer: &Buffer, rows: &[(u16, u16)]) {
    let tint = crate::review::mouse::selection_bg();
    for &at in rows {
        let y = at.1;
        let pane = new_pane(buffer, at);
        assert_eq!(buffer[(pane.start, y)].symbol(), " ", "no bar on row {y}");
        for x in pane.clone() {
            let cell = &buffer[(x, y)];
            assert_eq!(cell.bg, tint, "row {y} column {x} is filled");
            assert!(!cell.modifier.contains(Modifier::UNDERLINED), "row {y} column {x} has no underline");
        }
        assert_ne!(buffer[(pane.start - 1, y)].bg, tint, "the middle line of row {y}");
        assert_ne!(buffer[(pane.start - 2, y)].bg, tint, "the old side of row {y}");
    }
}

#[test]
fn a_selection_is_one_block_of_colour_while_dragging_after_the_release_and_with_v() {
    let fixture = Fixture::new("mouse-tint");
    let mut screen = fixture.open();
    let rows = ["def add(a, b):", "total = a + b", "return total"];
    let marks: Vec<(u16, u16)> = rows.iter().map(|r| locate(&mut screen, r, true)).collect();
    let outside = locate(&mut screen, "def main():", true);
    let tint = crate::review::mouse::selection_bg();

    drag(&mut screen, marks[0], marks[2]);
    let buffer = draw(&mut screen);
    assert_selected_block(&buffer, &marks);
    assert_ne!(buffer[(outside.0, outside.1)].bg, tint);

    release(&mut screen, marks[2]);
    assert_selected_block(&draw(&mut screen), &marks);

    // The keyboard's selection looks the same, the cursor's row included.
    press(&mut screen, KeyCode::Esc);
    assert_eq!(selection(&screen), None);
    at(&mut screen, 8);
    keys(&mut screen, "vjj");
    assert_eq!(selection(&screen), Some(lines(Side::New, 8, 10)));
    let buffer = draw(&mut screen);
    assert_selected_block(&buffer, &marks);
    assert_ne!(buffer[(outside.0, outside.1)].bg, tint);
}

#[test]
fn in_the_unified_view_a_removed_line_inside_new_lines_is_shaded_but_not_selected() {
    let fixture = Fixture::new("mouse-unified");
    let mut screen = fixture.open();
    press(&mut screen, KeyCode::Char('u'));
    let tint = crate::review::mouse::selection_bg();
    let from = locate(&mut screen, "def add(a, b):", true);
    let to = locate(&mut screen, "return total", true);
    let removed = locate(&mut screen, "return a + b", true);
    assert!(from.1 < removed.1 && removed.1 < to.1, "the removed line sits among the new ones");
    let shaded = |buffer: &Buffer, (x, y): (u16, u16)| buffer[(x, y)].bg == tint;
    let pane = screen.geometry.diff;
    let check = |buffer: &Buffer| {
        for at in [from, removed, to] {
            assert!(shaded(buffer, at), "row {} is shaded", at.1);
        }
        // The whole row, from the pane's first column to its last.
        assert!(shaded(buffer, (pane.x, removed.1)) && shaded(buffer, (pane.right() - 1, removed.1)));
        // Nothing above the first selected row or below the last.
        assert!(!shaded(buffer, (from.0, from.1 - 1)) && !shaded(buffer, (to.0, to.1 + 1)));
    };

    drag(&mut screen, from, to);
    check(&draw(&mut screen));
    release(&mut screen, to);
    check(&draw(&mut screen));

    // The keyboard's selection looks the same.
    press(&mut screen, KeyCode::Esc);
    at(&mut screen, 8);
    // In the unified view `j` steps over the removed row too.
    keys(&mut screen, "vjjj");
    assert_eq!(selection(&screen), Some(lines(Side::New, 8, 10)));
    check(&draw(&mut screen));

    // Only the look: the note is on new lines 8-10 and quotes them alone.
    press(&mut screen, KeyCode::Char('c'));
    type_and_save(&mut screen, "split this up");
    let note = screen.store.annotations().first().expect("a note");
    assert_eq!((note.line_start, note.line_end, note.side), (8, 10, Side::New));
    assert_eq!(note.anchor_text.as_deref(), Some("def add(a, b):\n    total = a + b\n    return total"));
}

#[test]
fn the_toolbar_after_a_drag_comments_on_the_selection() {
    let fixture = Fixture::new("mouse-toolbar");
    let mut screen = fixture.open();
    let from = locate(&mut screen, "def add(a, b):", true);
    let to = locate(&mut screen, "return total", true);
    drag(&mut screen, from, to);
    release(&mut screen, to);
    let comment = locate(&mut screen, "comment (c)", false);
    assert_eq!(comment.1, to.1 + 1, "the toolbar sits under the release");
    mouse(&mut screen, MouseEventKind::Down(MouseButton::Left), comment);
    assert_eq!(screen.mode, Mode::Compose(Draft::Comment(lines(Side::New, 8, 10))));
}

#[test]
fn a_key_takes_the_toolbar_away_and_keeps_the_selection() {
    let fixture = Fixture::new("mouse-toolbar-key");
    let mut screen = fixture.open();
    let from = locate(&mut screen, "def add(a, b):", true);
    let to = locate(&mut screen, "return total", true);
    drag(&mut screen, from, to);
    release(&mut screen, to);
    assert!(frame(&mut screen, COLS, ROWS).iter().any(|r| r.contains("comment (c)")));
    press(&mut screen, KeyCode::Char('j'));
    assert!(!frame(&mut screen, COLS, ROWS).iter().any(|r| r.contains("comment (c)")));
    assert_eq!(selection(&screen), Some(lines(Side::New, 8, 10)));
}

#[test]
fn y_copies_the_selected_code_when_copy_on_select_is_off() {
    let fixture = Fixture::new("mouse-copy");
    let mut screen = fixture.open();
    screen.set_copy_on_select(false);
    let from = locate(&mut screen, "def add(a, b):", true);
    let to = locate(&mut screen, "return total", true);
    drag(&mut screen, from, to);
    release(&mut screen, to);
    assert!(row_with(&frame(&mut screen, COLS, ROWS), "esc clear").contains("y copy"));
    let picked = selection(&screen).expect("selected");
    assert_eq!(
        screen.selected_code(&picked).as_deref(),
        Some("def add(a, b):\n    total = a + b\n    return total")
    );
    press(&mut screen, KeyCode::Char('y'));
    assert_eq!(screen.status.as_deref(), Some("copied 3 lines"));
}

/// The fixture with `app.py` long enough that its diff runs past one screen.
fn long_fixture(tag: &str) -> Fixture {
    let fixture = Fixture::new(tag);
    let tail = (1..=60).map(|i| format!("v{i} = {i}")).collect::<Vec<_>>().join("\n");
    fixture.write("app.py", &format!("{AFTER}{tail}\n"));
    fixture
}

#[test]
fn dragging_past_the_bottom_scrolls_and_keeps_selecting() {
    let fixture = long_fixture("mouse-scroll");
    let mut screen = fixture.open();
    let from = locate(&mut screen, "import sys", true);
    let bottom = screen.geometry.diff.bottom();
    drag(&mut screen, from, (from.0, bottom - 1));
    let on_screen = screen.selected_lines().expect("dragging").last;
    for _ in 0..10 {
        mouse(&mut screen, MouseEventKind::Drag(MouseButton::Left), (from.0, bottom + 2));
        let _ = draw(&mut screen);
    }
    release(&mut screen, (from.0, bottom + 2));
    let picked = selection(&screen).expect("selected");
    assert_eq!(picked.first, 1);
    assert!(picked.last > on_screen + 5, "{} grew past {on_screen}", picked.last);
}

#[test]
fn a_drag_in_the_whole_file_selects_its_lines_for_a_comment() {
    let fixture = Fixture::new("mouse-whole");
    let mut screen = fixture.open();
    at(&mut screen, 1);
    press(&mut screen, KeyCode::Char('o'));
    settle_whole(&mut screen);
    let from = locate(&mut screen, "def greet(name):", true);
    let to = locate(&mut screen, "print(\"hello,\", name)", true);
    drag(&mut screen, from, to);
    release(&mut screen, to);
    // Drawn as in the diff: a block of colour with no bar and no underline.
    let buffer = draw(&mut screen);
    for (x, y) in [from, to] {
        let row: String = (0..COLS).map(|c| buffer[(c, y)].symbol()).collect();
        assert!(!row.contains(['\u{2503}', '\u{258e}']), "no bar: {row}");
        let cell = &buffer[(x, y)];
        assert_eq!(cell.bg, crate::review::mouse::selection_bg(), "{row}");
        assert!(!cell.modifier.contains(Modifier::UNDERLINED), "{row}");
    }
    let whole = screen.whole.as_ref().expect("still open");
    assert_eq!(whole.target().expect("target"), Some((4, 5)));
    press(&mut screen, KeyCode::Char('c'));
    assert_eq!(screen.mode, Mode::Compose(Draft::Comment(lines(Side::New, 4, 5))));
}

#[test]
fn the_wheel_scrolls_the_diff_under_the_pointer() {
    let fixture = long_fixture("mouse-wheel");
    let mut screen = fixture.open();
    let first = locate(&mut screen, "import sys", true);
    mouse(&mut screen, MouseEventKind::ScrollDown, first);
    assert!(!frame(&mut screen, COLS, ROWS).iter().any(|r| r.contains("import sys")));
    mouse(&mut screen, MouseEventKind::ScrollUp, first);
    assert!(frame(&mut screen, COLS, ROWS).iter().any(|r| r.contains("import sys")));
}

#[test]
fn a_click_in_the_file_list_opens_the_file_and_starts_no_selection() {
    let fixture = Fixture::new("mouse-files");
    fixture.write("notes.txt", "changed\n");
    let mut screen = fixture.open();
    draw(&mut screen);
    assert_eq!(screen.files.current.as_deref(), Some("app.py"));
    // The list's own row, not the diff's heading for the file.
    let y = screen.geometry.file_rows.iter().find(|(_, key)| key == "notes.txt").expect("a list row").0;
    let row = (screen.geometry.files.x + 2, y);
    drag(&mut screen, row, (row.0, row.1 + 3));
    release(&mut screen, (row.0, row.1 + 3));
    assert_eq!(selection(&screen), None);
    assert_eq!(screen.files.current.as_deref(), Some("notes.txt"));
    assert_eq!(screen.focus, Focus::Files);
}

#[test]
fn a_click_on_a_note_selects_it_and_shows_its_line() {
    let fixture = Fixture::new("mouse-note");
    let mut screen = fixture.open();
    at(&mut screen, 9);
    press(&mut screen, KeyCode::Char('c'));
    type_and_save(&mut screen, "first");
    at(&mut screen, 14);
    press(&mut screen, KeyCode::Char('c'));
    type_and_save(&mut screen, "second");
    at(&mut screen, 1);
    let bubble = locate(&mut screen, "first", true);
    mouse(&mut screen, MouseEventKind::Down(MouseButton::Left), bubble);
    assert_eq!(screen.focus, Focus::Notes);
    assert_eq!(screen.selected_note().map(|n| n.line_start), Some(9));
    assert_eq!(screen.diff.cursor().expect("cursor"), Some(lines(Side::New, 9, 9)));
}

#[test]
fn the_header_buttons_send_and_open_the_review_menu() {
    let fixture = Fixture::new("mouse-buttons");
    let mut screen = fixture.open();
    at(&mut screen, 9);
    press(&mut screen, KeyCode::Char('c'));
    type_and_save(&mut screen, "a note");
    let review = locate(&mut screen, "Review", false);
    mouse(&mut screen, MouseEventKind::Down(MouseButton::Left), review);
    assert_eq!(screen.mode, Mode::Menu(0));
    press(&mut screen, KeyCode::Esc);
    let send = locate(&mut screen, "Send 1 new", false);
    mouse(&mut screen, MouseEventKind::Down(MouseButton::Left), send);
    assert_eq!(fixture.sent().len(), 1);
}
