//! Saving a note leaves the diff where it is: the same line on the top row, and every line's
//! code from the same column. Nothing in the review scrolls the code sideways, so that column
//! is each line's start. The notes panel does not open for a saved note (the gutter mark, the
//! footer and the header's Send count say it was saved), and opening or closing the panel or
//! the file list, a resize, or moving the line between the sides scrolls nothing sideways.
//!
//! The case the person hit: a wide terminal, the line between the sides at 30:70, wrapping
//! off, lines longer than any pane, the file list open and the panel closed. The first note
//! opened the panel, the diff got narrower, and the viewer scrolled every line to the column
//! the selection had last clicked, at the new side's right edge: only tails of the longest
//! lines were left.

use std::collections::BTreeMap;

use plannotator_tui_review::Side;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, MouseButton, MouseEventKind};

use super::compose_near::{View, draw, report, view};
use super::*;
use crate::review::diff::{Lines, Progress};

const SIZE: (u16, u16) = (190, 50);
const PATH: &str = "Screen.kt";

/// Line `i` of `Screen.kt`, before or after the change: every fourth from line 2 a short comment, the
/// others longer than any pane, each with its own `rowNNN` to find it by.
fn kotlin_line(i: usize, after: bool) -> String {
    if i % 4 == 2 {
        return format!("        // section {i}\n");
    }
    let changed = after && (60..=140).contains(&i) && i % 3 != 1;
    let call = if changed { "draw" } else { "render" };
    format!(
        "        val row{i:03} = {call}(state.thread.items.filter {{ it.kind == Kind.Comment }}.map \
         {{ Row(it.id, it.author, it.body) }}, label = \"row {i}\", width = {i}, color = Color.Gray)\n"
    )
}

fn source(after: bool) -> String {
    (1..=200).map(|i| kotlin_line(i, after)).collect()
}

/// The usual fixture with `notes.txt` changed (so the file list shows) and `Screen.kt`, committed and changed in one
/// hunk from line 60 to 140, with `[review]` as `config` says.
fn kotlin(tag: &str, config: &str) -> (Fixture, Screen) {
    let fixture = Fixture::new(tag);
    fixture.write(PATH, &source(false));
    fixture.git(&["add", "."]);
    fixture.git(&["-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-qm", "kotlin"]);
    fixture.write(PATH, &source(true));
    fixture.write("notes.txt", "changed\n");
    fixture.write_config(config);
    let mut screen = fixture.open_configured();
    let started = Instant::now();
    while screen.diff.progress() != Progress::Ready {
        assert!(started.elapsed() < Duration::from_secs(10), "the viewer never got ready");
        screen.tick().expect("tick");
        draw(&mut screen, SIZE);
        std::thread::sleep(Duration::from_millis(10));
    }
    // Line 80 near the top, then three notches of the wheel, as a person reading down.
    screen.diff.reveal(PATH, Side::New, 80).expect("line 80");
    draw(&mut screen, SIZE);
    let pane = screen.geometry.diff;
    for _ in 0..3 {
        report(&mut screen, MouseEventKind::ScrollDown, (pane.x + pane.width / 2, pane.y + pane.height / 2));
        draw(&mut screen, SIZE);
    }
    (fixture, screen)
}

/// The screen row of `line` of `Screen.kt` on `side`, in the last frame.
fn row_of(screen: &Screen, side: Side, line: u32) -> u16 {
    let pane = screen.geometry.diff;
    let x = if side == Side::Old && screen.diff.split() { pane.x + 1 } else { pane.right() - 2 };
    (pane.top()..pane.bottom())
        .find(|&y| {
            screen.diff.row_at(x, y).is_some_and(|row| {
                let shown = if side == Side::Old { row.old_line } else { row.new_line };
                row.path == PATH && shown == Some(line) && row.part == 0
            })
        })
        .unwrap_or_else(|| panic!("{side:?} line {line} is not on screen"))
}

/// A column inside the code of `side`'s pane, where a person would press.
fn code_column(screen: &Screen, side: Side) -> u16 {
    let pane = screen.geometry.diff;
    match screen.diff.split_line() {
        Some(line) if side == Side::Old => pane.x + (line - pane.x) / 2,
        Some(line) => line + (pane.right() - line) / 2,
        None => pane.x + pane.width / 2,
    }
}

/// Drag over `first..=last` on `side`, from `last` up when `up`, and let go.
fn drag(screen: &mut Screen, side: Side, (first, last): (u32, u32), up: bool) {
    draw(screen, SIZE);
    let x = code_column(screen, side);
    let (from, to) = if up { (last, first) } else { (first, last) };
    let (from, to) = (row_of(screen, side, from), row_of(screen, side, to));
    report(screen, MouseEventKind::Down(MouseButton::Left), (x, from));
    let rows: Vec<u16> = if to >= from { (from..=to).collect() } else { (to..=from).rev().collect() };
    for y in rows.into_iter().skip(1) {
        report(screen, MouseEventKind::Drag(MouseButton::Left), (x, y));
        draw(screen, SIZE);
    }
    report(screen, MouseEventKind::Up(MouseButton::Left), (x, to));
    assert_eq!(screen.diff.selection(), Some(Lines { path: PATH.into(), side, first, last }));
}

/// Where each `rowNNN` is drawn on each side, from that side's left edge: the code's
/// columns. Equal maps mean no line scrolled sideways (and none left the screen).
type Columns = BTreeMap<(&'static str, u32), u16>;

fn columns(screen: &Screen, buffer: &Buffer) -> Columns {
    let pane = screen.geometry.diff;
    let sides: Vec<(&'static str, u16, u16)> = match screen.diff.split_line() {
        Some(line) => vec![("old", pane.x, line), ("new", line + 1, pane.right())],
        None => vec![("any", pane.x, pane.right())],
    };
    let mut found = Columns::new();
    for y in pane.top()..pane.bottom() {
        for &(side, left, right) in &sides {
            let cells: String = (left..right)
                .map(|x| buffer.cell((x, y)).and_then(|c| c.symbol().chars().next()).unwrap_or(' '))
                .collect();
            let Some(byte) = cells.find("val row") else { continue };
            let number = cells.get(byte + 7..byte + 10).and_then(|n| n.parse().ok()).expect("a row number");
            let column = cells.get(..byte).map_or(0, |b| b.chars().count()) as u16;
            found.insert((side, number), column);
        }
    }
    found
}

/// The view (which line on each row, and the cursor's) and the code's columns, in a new frame.
fn look(screen: &mut Screen) -> (View, Columns) {
    let shown = view(screen, SIZE);
    let buffer = draw(screen, SIZE);
    let columns = columns(screen, &buffer);
    (shown, columns)
}

/// The lines on screen in both, at the columns they had: every line's code still starts
/// where it did, and that is the line's start, the same column for every line of a side.
fn same_columns(before: &Columns, after: &Columns, case: &str) {
    let both: Vec<_> = before.keys().filter(|k| after.contains_key(*k)).collect();
    assert!(
        both.len() >= 10,
        "{case}: the code of most lines left the screen, scrolled sideways: {before:?} then {after:?}"
    );
    for key in both {
        assert_eq!(
            before[key], after[key],
            "{case}: line {key:?} scrolled sideways: {before:?} then {after:?}"
        );
    }
    for side in ["old", "new", "any"] {
        let mut columns = before.iter().filter(|((s, _), _)| *s == side).map(|(_, c)| *c);
        if let Some(first) = columns.next() {
            assert!(columns.all(|c| c == first), "{case}: every {side} line from its start: {before:?}");
        }
    }
}

/// What each row of the pane shows, without the cursor: a selection moves the cursor.
fn rows(view: &View) -> &Rows {
    &view.0
}

type Rows = Vec<Option<(String, Option<u32>, Option<u32>)>>;

/// The header counts unsent notes and the footer says the last was saved.
fn acknowledged(screen: &mut Screen, notes: usize, case: &str) {
    let frame = frame(screen, SIZE.0, SIZE.1);
    assert!(frame[0].contains(&format!("{notes} new")), "{case}: the header counts it: {:?}", frame[0]);
    let footer = frame.iter().rev().take(2).cloned().collect::<Vec<_>>().join(" ");
    assert!(footer.contains("note saved"), "{case}: the footer says so: {footer:?}");
}

/// A click on `line` of `side`, left button down and up where a person would click.
fn click(screen: &mut Screen, side: Side, line: u32, x: u16) {
    draw(screen, SIZE);
    let y = row_of(screen, side, line);
    report(screen, MouseEventKind::Down(MouseButton::Left), (x, y));
    report(screen, MouseEventKind::Up(MouseButton::Left), (x, y));
}

const SPLIT_30: &str = "[review]\nlayout = \"split\"\nsplit_ratio = 0.3\nwrap = false\n";

#[test]
fn saving_a_comment_on_a_mouse_selection_keeps_the_diff_where_it_was() {
    for up in [true, false] {
        let case = format!("drag {}", if up { "up" } else { "down" });
        let (_fixture, mut screen) = kotlin(&format!("save-keeps-{up}"), SPLIT_30);
        draw(&mut screen, SIZE);
        assert!(!screen.rail_shown() && screen.files.drawn_width > 0, "{case}: list open, panel closed");
        let (before, columns_before) = look(&mut screen);
        drag(&mut screen, Side::New, (100, 103), up);
        press(&mut screen, KeyCode::Char('c'));
        let boxed = look(&mut screen);
        assert_eq!(rows(&boxed.0), rows(&before), "{case}: the box opened over the diff, which stayed");
        type_and_save(&mut screen, "is this the right label?");
        let (after, columns_after) = look(&mut screen);
        same_columns(&columns_before, &columns_after, &case);
        assert_eq!(rows(&after), rows(&before), "{case}: the top line and every row below it");
        assert_eq!(screen.store.annotations().len(), 1, "{case}: saved");
        assert!(!screen.rail_shown() && screen.geometry.rail.width == 0, "{case}: the panel stays closed");
        acknowledged(&mut screen, 1, &case);
        let marked = frame(&mut screen, SIZE.0, SIZE.1);
        let line = row_of(&screen, Side::New, 101);
        assert!(marked[usize::from(line)].contains('\u{258c}'), "{case}: the gutter marks the note's lines");
    }
}

/// Lines of `Screen.kt` on screen on `side`, from `tenth` tenths down the pane, `count` of
/// them in a row.
#[derive(Debug, Clone, Copy)]
struct Span {
    side: Side,
    tenth: usize,
    count: u32,
}

impl Span {
    fn lines(self, screen: &mut Screen) -> (u32, u32) {
        draw(screen, SIZE);
        let pane = screen.geometry.diff;
        let x = if self.side == Side::Old && screen.diff.split() { pane.x + 1 } else { pane.right() - 2 };
        let shown: Vec<u32> = (pane.top()..pane.bottom())
            .filter_map(|y| screen.diff.row_at(x, y))
            .filter(|row| {
                row.path == PATH && row.part == 0 && row.kind == diffs_rs::engine::DiffRowKind::Code
            })
            .filter_map(|row| if self.side == Side::Old { row.old_line } else { row.new_line })
            .collect();
        let first = shown[shown.len() * self.tenth / 10];
        let last = first + self.count - 1;
        assert!(
            (first..=last).all(|n| shown.contains(&n)),
            "{self:?}: {first}..={last} on screen in {shown:?}"
        );
        (first, last)
    }
}

/// One way of making a note, or of not making one.
#[derive(Debug, Clone, Copy)]
enum Act {
    /// Drag over the lines, up or down, `c`, type, `Enter`.
    Comment(Span, bool),
    /// Drag over new lines, `s`, `Enter` with the code as it is.
    Suggest(Span),
    /// Click the first new line, `v`, `j` down to the last, `v`, `c`, type, `Enter`.
    Keys(Span),
    /// Drag, `c`, `Esc`: nothing saved.
    Cancel(Span),
}

/// Do `act` and check the diff did not move: the same rows, the code from the same columns.
fn act_keeps_the_view(screen: &mut Screen, act: Act, case: &str) {
    let (before, columns_before) = look(screen);
    let notes = screen.store.annotations().len();
    let panel = screen.rail_shown();
    let saves = match act {
        Act::Comment(span, up) => {
            let lines = span.lines(screen);
            drag(screen, span.side, lines, up);
            press(screen, KeyCode::Char('c'));
            type_and_save(screen, "why here?");
            true
        }
        Act::Suggest(span) => {
            let lines = span.lines(screen);
            drag(screen, Side::New, lines, false);
            press(screen, KeyCode::Char('s'));
            draw(screen, SIZE);
            press(screen, KeyCode::Enter);
            true
        }
        Act::Keys(span) => {
            let (first, last) = span.lines(screen);
            let x = code_column(screen, Side::New);
            click(screen, Side::New, first, x);
            press(screen, KeyCode::Char('v'));
            for _ in 0..20 {
                if screen
                    .diff
                    .cursor()
                    .expect("cursor")
                    .is_some_and(|at| at.side == Side::New && at.first >= last)
                {
                    break;
                }
                press(screen, KeyCode::Char('j'));
            }
            press(screen, KeyCode::Char('v'));
            let selected = Some(Lines { path: PATH.into(), side: Side::New, first, last });
            assert_eq!(screen.diff.selection(), selected, "{case}: {:?}", screen.diff.cursor());
            press(screen, KeyCode::Char('c'));
            draw(screen, SIZE);
            type_and_save(screen, "and this?");
            true
        }
        Act::Cancel(span) => {
            let lines = span.lines(screen);
            drag(screen, Side::New, lines, true);
            press(screen, KeyCode::Char('c'));
            draw(screen, SIZE);
            press(screen, KeyCode::Esc);
            false
        }
    };
    let (after, columns_after) = look(screen);
    let case = format!("{case} {act:?}");
    assert_eq!(screen.mode, crate::review::Mode::Browse, "{case}: the box closed");
    assert_eq!(screen.store.annotations().len(), notes + usize::from(saves), "{case}");
    assert_eq!(screen.rail_shown(), panel, "{case}: the panel neither opens nor closes");
    same_columns(&columns_before, &columns_after, &case);
    assert_eq!(rows(&after), rows(&before), "{case}: the top line and every row below it");
    if saves {
        acknowledged(screen, notes + 1, &case);
    }
}

#[test]
fn no_note_moves_the_diff_at_any_split_or_layout_with_wrapping_on_or_off() {
    let layouts = [
        ("split 30", "layout = \"split\"\nsplit_ratio = 0.3"),
        ("split 50", "layout = \"split\""),
        ("split 70", "layout = \"split\"\nsplit_ratio = 0.7"),
        ("unified", "layout = \"unified\""),
    ];
    for (name, layout) in layouts {
        for wrap in [false, true] {
            for panel in [false, true] {
                let case = format!("{name} wrap {wrap} panel {panel}");
                let tag = format!("save-every-{}-{wrap}-{panel}", name.replace(' ', "-"));
                let (_fixture, mut screen) = kotlin(&tag, &format!("[review]\n{layout}\nwrap = {wrap}\n"));
                if panel {
                    press(&mut screen, KeyCode::Char('z'));
                    assert!(screen.rail_shown(), "{case}: opened by hand");
                }
                let new = |tenth, count| Span { side: Side::New, tenth, count };
                let other = if screen.diff.split() {
                    Act::Comment(Span { side: Side::Old, tenth: 2, count: 3 }, false)
                } else {
                    Act::Keys(new(2, 3))
                };
                for act in [
                    Act::Comment(new(5, 4), true),
                    other,
                    Act::Comment(new(1, 2), false),
                    Act::Suggest(new(7, 2)),
                    Act::Keys(new(4, 2)),
                    Act::Cancel(new(6, 2)),
                ] {
                    act_keeps_the_view(&mut screen, act, &case);
                }
                if panel {
                    let shown = frame(&mut screen, SIZE.0, SIZE.1).join("\n");
                    assert!(shown.contains("why here?"), "{case}: the open panel shows the notes:\n{shown}");
                }
            }
        }
    }
}

#[test]
fn opening_the_panel_or_the_list_resizing_or_moving_the_middle_line_never_scrolls_sideways() {
    for selection in [false, true] {
        let case = format!("selection {selection}");
        let (_fixture, mut screen) = kotlin(&format!("save-by-hand-{selection}"), SPLIT_30);
        // A click far right on a long line, as a person reading it would: the column the
        // viewer would scroll to once the pane got narrower.
        let right = screen.geometry.diff.right() - 3;
        click(&mut screen, Side::New, 101, right);
        if selection {
            drag(&mut screen, Side::New, (100, 103), true);
        }
        let (before, columns) = look(&mut screen);
        let check = |screen: &mut Screen, what: &str, same_rows: bool| {
            let (after, columns_after) = look(screen);
            same_columns(&columns, &columns_after, &format!("{case}: {what}"));
            if same_rows {
                assert_eq!(rows(&after), rows(&before), "{case}: {what}: the top line and the rows");
            }
        };
        press(&mut screen, KeyCode::Char('z'));
        assert!(screen.rail_shown() && screen.geometry.rail.width == 0, "{case}: drawn next frame");
        check(&mut screen, "the panel opened", true);
        assert!(screen.geometry.rail.width > 0);
        press(&mut screen, KeyCode::Char('z'));
        check(&mut screen, "the panel closed", true);
        // `f` goes to the list, `f` again closes it, a third opens it with the focus.
        press(&mut screen, KeyCode::Char('f'));
        press(&mut screen, KeyCode::Char('f'));
        check(&mut screen, "the list closed", true);
        assert_eq!(screen.files.drawn_width, 0);
        press(&mut screen, KeyCode::Char('f'));
        check(&mut screen, "the list opened", true);
        assert!(screen.files.drawn_width > 0);
        press(&mut screen, KeyCode::Esc);
        assert_eq!(screen.focus, crate::review::Focus::Diff, "back to the diff");
        draw(&mut screen, (150, 50));
        draw(&mut screen, (120, 50));
        check(&mut screen, "the terminal narrowed and widened again", false);
        for key in [')', ')', '(', '(', '(', '('] {
            press(&mut screen, KeyCode::Char(key));
            check(&mut screen, &format!("the middle line moved with {key}"), true);
        }
        press(&mut screen, KeyCode::Char('z'));
        check(&mut screen, "the panel opened again", true);
    }
}

/// The first row of the pane that shows a line, by its file and line numbers: the top line.
/// A wrapped line's later rows show the same numbers.
fn top_line(view: &View) -> Option<(String, Option<u32>, Option<u32>)> {
    rows(view).iter().flatten().find(|(_, old, new)| old.is_some() || new.is_some()).cloned()
}

/// Every line shown starts at its first character: each side shows its lines' code from one
/// column, and at least `least` of them, which a view scrolled past the code's start lacks.
fn from_start(columns: &Columns, least: usize, case: &str) {
    assert!(columns.len() >= least, "{case}: the lines' code left the screen sideways: {columns:?}");
    for side in ["old", "new", "any"] {
        let mut shown = columns.iter().filter(|((s, _), _)| *s == side).map(|(_, c)| *c);
        if let Some(first) = shown.next() {
            assert!(shown.all(|c| c == first), "{case}: every {side} line from its start: {columns:?}");
        }
    }
}

const WIDE: &str = "Wide.kt";

/// `Wide.kt`, sixty lines like `Screen.kt`'s, the last without a newline: lines 47-49
/// removed and three long lines added after line 57, near enough the end for the hunk to
/// reach it. The diff has rows with only an old line
/// and rows with only a new one, so one half of a split row is empty, and the
/// "No newline at end of file" mark under the last line.
fn wide(tag: &str, config: &str) -> (Fixture, Screen) {
    let fixture = Fixture::new(tag);
    let before: String = (1..=60).map(|i| kotlin_line(i, false)).collect();
    let after: String = (1..=60)
        .filter(|i| !(47..=49).contains(i))
        .flat_map(|i| {
            let added: &[usize] = if i == 57 { &[901, 903, 905] } else { &[] };
            std::iter::once(kotlin_line(i, false)).chain(added.iter().map(|&n| kotlin_line(n, false)))
        })
        .collect();
    fixture.write(WIDE, before.trim_end_matches('\n'));
    fixture.git(&["add", "."]);
    fixture.git(&["-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-qm", "wide"]);
    fixture.write(WIDE, after.trim_end_matches('\n'));
    fixture.write("notes.txt", "changed\n");
    fixture.write_config(config);
    let mut screen = fixture.open_configured();
    let started = Instant::now();
    while screen.diff.progress() != Progress::Ready {
        assert!(started.elapsed() < Duration::from_secs(10), "the viewer never got ready");
        screen.tick().expect("tick");
        draw(&mut screen, SIZE);
        std::thread::sleep(Duration::from_millis(10));
    }
    screen.diff.reveal(WIDE, Side::New, 57).expect("line 57");
    draw(&mut screen, SIZE);
    (fixture, screen)
}

/// The screen row of the first row of `Wide.kt` that `wanted` accepts, in the last frame.
fn wide_row(screen: &Screen, wanted: impl Fn(&diffs_rs::engine::DiffRow) -> bool) -> u16 {
    let pane = screen.geometry.diff;
    (pane.top()..pane.bottom())
        .find(|&y| {
            screen.diff.row_at(pane.right() - 2, y).is_some_and(|row| row.path == WIDE && wanted(&row))
        })
        .expect("the row is on screen")
}

/// The column just inside the right edge of the old side, or of the new side (or of a
/// unified row).
fn far_right(screen: &Screen, old: bool) -> u16 {
    let pane = screen.geometry.diff;
    match screen.diff.split_line() {
        Some(line) if old => line - 2,
        _ => pane.right() - 3,
    }
}

fn press_at(screen: &mut Screen, (x, y): (u16, u16)) {
    report(screen, MouseEventKind::Down(MouseButton::Left), (x, y));
    report(screen, MouseEventKind::Up(MouseButton::Left), (x, y));
}

/// Where a click lands that the viewer used to scroll to once the pane got narrower.
#[derive(Debug, Clone, Copy)]
enum Far {
    /// Far right in the empty old half of an added row.
    AddedRowsEmptyHalf,
    /// Far right in the empty new half of a removed row.
    RemovedRowsEmptyHalf,
    /// Far right on the "No newline at end of file" mark, on the old or the new side.
    NoNewline(bool),
    /// An old-side drag from a context line down over the added rows, then `Esc`.
    OldDragOverAdded,
}

fn far_click(screen: &mut Screen, far: Far) {
    use diffs_rs::engine::DiffRowKind;
    draw(screen, SIZE);
    let code = |row: &diffs_rs::engine::DiffRow| row.kind == DiffRowKind::Code && row.part == 0;
    match far {
        Far::AddedRowsEmptyHalf => {
            let y = wide_row(screen, |row| code(row) && row.new_line == Some(55) && row.old_line.is_none());
            press_at(screen, (far_right(screen, true), y));
        }
        Far::RemovedRowsEmptyHalf => {
            let y = wide_row(screen, |row| code(row) && row.old_line == Some(48) && row.new_line.is_none());
            press_at(screen, (far_right(screen, false), y));
        }
        Far::NoNewline(old) => {
            let y = wide_row(screen, |row| row.kind == DiffRowKind::NoNewline);
            press_at(screen, (far_right(screen, old), y));
        }
        Far::OldDragOverAdded => {
            let x = far_right(screen, true);
            let from = wide_row(screen, |row| code(row) && row.new_line == Some(53));
            let to = wide_row(screen, |row| code(row) && row.new_line == Some(56));
            report(screen, MouseEventKind::Down(MouseButton::Left), (x, from));
            for y in from + 1..=to {
                report(screen, MouseEventKind::Drag(MouseButton::Left), (x, y));
                draw(screen, SIZE);
            }
            press(screen, KeyCode::Esc);
            report(screen, MouseEventKind::Up(MouseButton::Left), (x, to));
        }
    }
}

const LAYOUTS: [(&str, &str); 3] = [
    ("split 30", "layout = \"split\"\nsplit_ratio = 0.3"),
    ("split 50", "layout = \"split\""),
    ("unified", "layout = \"unified\""),
];

/// What a person does after the click that narrows, widens or redraws the diff: the notes
/// panel, the file list, a resize, the line between the sides, the layout and whitespace.
fn reflow_by_hand(screen: &mut Screen, check: &dyn Fn(&mut Screen, &str)) {
    for (keys, what) in [
        ("z", "the panel opened"),
        ("z", "the panel closed"),
        ("ff", "the list closed"),
        ("f", "the list opened"),
    ] {
        for key in keys.chars() {
            press(screen, KeyCode::Char(key));
        }
        check(screen, what);
    }
    press(screen, KeyCode::Esc);
    draw(screen, (150, 50));
    draw(screen, (120, 50));
    check(screen, "the terminal narrowed and widened again");
    for key in [')', ')', '(', '(', '(', 'u', 'u', 'W', 'W'] {
        press(screen, KeyCode::Char(key));
        check(screen, &format!("then {key}"));
    }
}

#[test]
fn a_click_in_an_empty_half_or_on_the_no_newline_mark_never_scrolls_the_diff_sideways() {
    for (name, layout) in LAYOUTS {
        let fars = [
            Far::AddedRowsEmptyHalf,
            Far::RemovedRowsEmptyHalf,
            Far::NoNewline(true),
            Far::NoNewline(false),
            Far::OldDragOverAdded,
        ];
        for far in fars {
            let case = format!("{name} {far:?}");
            let tag = format!("wide-{}-{far:?}", name.replace(' ', "-")).replace(['(', ')'], "");
            let (_fixture, mut screen) = wide(&tag, &format!("[review]\n{layout}\nwrap = false\n"));
            far_click(&mut screen, far);
            let (before, columns) = look(&mut screen);
            from_start(&columns, 10, &format!("{case}: clicked"));
            let top = top_line(&before);
            reflow_by_hand(&mut screen, &|screen, what| {
                let (after, columns) = look(screen);
                from_start(&columns, 10, &format!("{case}: {what}"));
                // The same line: a pair's row in a split diff is two rows unified, old first.
                let after = top_line(&after);
                let same = match (&after, &top) {
                    (Some((path, old, new)), Some((was, old_was, new_was))) => {
                        path == was
                            && ((old.is_some() && old == old_was) || (new.is_some() && new == new_was))
                    }
                    _ => false,
                };
                assert!(same, "{case}: {what}: the top line {top:?} became {after:?}");
            });
        }
    }
}

/// How the cursor gets onto a wrapped line's second or later row before `w`.
#[derive(Debug, Clone, Copy)]
enum OnPart {
    /// A click far right on the new side, or on the old side.
    Click(bool),
    /// `j` until the cursor is on one.
    Down,
}

#[test]
fn turning_wrapping_off_on_a_wrapped_row_keeps_the_top_line_and_every_line_from_its_start() {
    for (name, layout) in LAYOUTS {
        for on in [OnPart::Click(false), OnPart::Click(true), OnPart::Down] {
            let case = format!("{name} {on:?}");
            let tag = format!("unwrap-{}-{on:?}", name.replace(' ', "-")).replace(['(', ')'], "");
            let (_fixture, mut screen) = kotlin(&tag, &format!("[review]\n{layout}\nwrap = true\n"));
            draw(&mut screen, SIZE);
            match on {
                OnPart::Click(old) => {
                    let pane = screen.geometry.diff;
                    let y = (pane.top() + 10..pane.bottom())
                        .find(|&y| screen.diff.row_at(pane.right() - 2, y).is_some_and(|row| row.part > 0))
                        .expect("a wrapped row");
                    let x = far_right(&screen, old);
                    press_at(&mut screen, (x, y));
                }
                OnPart::Down => {
                    for _ in 0..8 {
                        press(&mut screen, KeyCode::Char('j'));
                    }
                    while screen.diff.cursor_row().is_some_and(|row| row.part == 0) {
                        press(&mut screen, KeyCode::Char('j'));
                    }
                }
            }
            let (before, _) = look(&mut screen);
            assert!(
                screen.diff.cursor_row().is_some_and(|row| row.part > 0),
                "{case}: the cursor is on a wrapped row"
            );
            let top = top_line(&before);
            press(&mut screen, KeyCode::Char('w'));
            let (after, columns) = look(&mut screen);
            from_start(&columns, 20, &format!("{case}: wrapping off"));
            assert_eq!(top_line(&after), top, "{case}: the top line, now on one row");
            reflow_by_hand(&mut screen, &|screen, what| {
                let (_, columns) = look(screen);
                from_start(&columns, 20, &format!("{case}: {what}"));
            });
        }
    }
}
