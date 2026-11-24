//! Hiding whitespace changes (`W`, the menu, the header's `no ws` chip), the tab width and the
//! changed-text marks, on a Go file whose diff has a re-indented block with one real edit, a
//! hunk of trailing whitespace only, and a hunk whose only change is spaces put between
//! numbers. Notes, selections, the comment box and the moves around the review keep to the
//! right lines through every change.

use plannotator_tui_review::Side;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use super::jump_flow::settle_search;
use super::whole_flow::settle_whole;
use super::*;
use crate::config::review::{InlineChanges, ReviewConfig, Whitespace};
use crate::review::Mode;
use crate::review::diff::Lines;
use crate::review::settings::{ROWS as MENU, state};
use crate::ui::chips;

const COLS: u16 = 160;
const ROWS: u16 = 45;

/// Hunk 1 (lines 1-11): lines 4-8 re-indented from a tab to spaces, line 6 also edited.
/// Hunk 2 (17-24): trailing spaces on 20 and 21. Hunk 3 (29-33): `1,2,3` to `1, 2, 3`.
const GO_BEFORE: &str = "package main\n\nfunc total(items []int) int {\n\tsum := 0\n\tfor _, n := range items {\n\t\tsum += n\n\t}\n\treturn sum\n}\n\n// one\n// two\n// three\n// four\n// five\n// six\n// seven\n// eight\nfunc greet(name string) {\n\tprintln(\"hello\", name)\n\tprintln(\"bye\", name)\n}\n// a\n// b\n// c\n// d\n// e\n// f\n// g\n// h\nfunc main() {\n\tprintln(total([]int{1,2,3}))\n}\n";
const GO_AFTER: &str = "package main\n\nfunc total(items []int) int {\n    sum := 0\n    for _, n := range items {\n        sum += n * 2\n    }\n    return sum\n}\n\n// one\n// two\n// three\n// four\n// five\n// six\n// seven\n// eight\nfunc greet(name string) {\n\tprintln(\"hello\", name)  \n\tprintln(\"bye\", name)  \n}\n// a\n// b\n// c\n// d\n// e\n// f\n// g\n// h\nfunc main() {\n\tprintln(total([]int{1, 2, 3}))\n}\n";

fn fixture(tag: &str) -> Fixture {
    let fixture = Fixture::new(tag);
    fixture.write("ws.go", GO_BEFORE);
    fixture.git(&["add", "."]);
    fixture.git(&["-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-qm", "go"]);
    fixture.write("ws.go", GO_AFTER);
    fixture
}

fn buffer(screen: &mut Screen) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(COLS, ROWS)).expect("terminal");
    let mut drawn = Ok(());
    terminal.draw(|f| drawn = screen.draw(f)).expect("draw");
    drawn.expect("viewer draw");
    screen.after_draw();
    terminal.backend().buffer().clone()
}

fn text(buffer: &Buffer, y: u16) -> String {
    (0..buffer.area.width).filter_map(|x| buffer.cell((x, y)).map(|c| c.symbol().to_owned())).collect()
}

fn all(buffer: &Buffer) -> String {
    (0..buffer.area.height).map(|y| text(buffer, y)).collect::<Vec<_>>().join("\n")
}

/// The diff pane's rows of the last frame, as text.
fn pane(screen: &Screen, buffer: &Buffer) -> Vec<String> {
    let diff = screen.geometry.diff;
    (diff.top()..diff.bottom())
        .map(|y| {
            (diff.left()..diff.right())
                .filter_map(|x| buffer.cell((x, y)).map(ratatui::buffer::Cell::symbol))
                .collect()
        })
        .collect()
}

fn reveal(screen: &mut Screen, side: Side, line: u32) {
    screen.diff.reveal("ws.go", side, line).expect("the line is shown");
}

/// The screen row showing `line` of ws.go on `side` in the last frame, read on that side.
fn row_of(screen: &Screen, side: Side, line: u32) -> Option<u16> {
    let diff = screen.geometry.diff;
    let column = if side == Side::Old && screen.diff.split() { diff.x + 1 } else { diff.right() - 2 };
    (diff.top()..diff.bottom()).find(|&y| {
        screen.diff.row_at(column, y).is_some_and(|row| {
            let shown = if side == Side::Old { row.old_line } else { row.new_line };
            row.path == "ws.go" && row.kind == diffs_rs::engine::DiffRowKind::Code && shown == Some(line)
        })
    })
}

fn whitespace(screen: &Screen) -> diffs_rs::engine::Whitespace {
    screen.diff.drawn_with().2
}

fn remembered(fixture: &Fixture) -> ReviewConfig {
    state::load(&fixture.state(), ReviewConfig::DEFAULT).0
}

fn click(screen: &mut Screen, rect: Rect) {
    for kind in [MouseEventKind::Down(MouseButton::Left), MouseEventKind::Up(MouseButton::Left)] {
        let event = MouseEvent { kind, column: rect.x, row: rect.y, modifiers: KeyModifiers::NONE };
        screen.handle_event(Event::Mouse(event)).expect("mouse");
    }
}

fn menu_row(row: crate::review::settings::Row) -> usize {
    MENU.iter().position(|r| *r == row).expect("a menu row")
}

#[test]
fn w_hides_whitespace_only_hunks_keeps_a_real_edit_and_the_lit_chip_says_so_until_w_again() {
    let fixture = fixture("ws-toggle");
    let mut screen = fixture.open_configured();
    let shown = buffer(&mut screen);
    let header = text(&shown, 0);
    assert!(!header.contains(" no ws "), "{header:?}");
    assert!(all(&shown).contains("println(\"bye\", name)"), "the trailing-space hunk shows");
    keys(&mut screen, "W");
    // Held until the frame: the viewer changes once per frame however many keys came.
    assert_eq!(whitespace(&screen), diffs_rs::engine::Whitespace::Show);
    let hidden = buffer(&mut screen);
    assert_eq!(whitespace(&screen), diffs_rs::engine::Whitespace::IgnoreAll);
    let header = text(&hidden, 0);
    let chip = screen.geometry.whitespace_chip.expect("the chip");
    assert!(header.contains(" Wrap   no ws "), "beside the wrap switch: {header:?}");
    let lit = hidden.cell((chip.x, chip.y)).expect("cell");
    assert_eq!(lit.bg, chips::chips().whitespace_hidden.bg.expect("a background"));
    let drawn = all(&hidden);
    assert_eq!(drawn.matches("whitespace only").count(), 2, "two hunks are headings alone:\n{drawn}");
    assert!(!drawn.contains("println(\"bye\", name)"), "{drawn}");
    assert!(drawn.contains("sum += n * 2"), "the real edit stays:\n{drawn}");
    assert!(row_of(&screen, Side::New, 5).is_some() && row_of(&screen, Side::Old, 5).is_some(), "a pair");
    assert!(screen.status.as_deref().is_some_and(|s| s.starts_with("whitespace changes hidden")));
    assert_eq!(remembered(&fixture).whitespace, Whitespace::IgnoreAll, "remembered");
    // The header's counts are the patch's: hiding changes none of them.
    assert_eq!(header.split(" \u{b7} ").nth(1), text(&shown, 0).split(" \u{b7} ").nth(1));

    // A click on the chip shows them again, and the chip goes.
    click(&mut screen, chip);
    let again = buffer(&mut screen);
    assert_eq!(whitespace(&screen), diffs_rs::engine::Whitespace::Show);
    assert!(screen.geometry.whitespace_chip.is_none() && !text(&again, 0).contains("no ws"));
    assert_eq!(screen.status.as_deref(), Some("whitespace changes shown"));
    assert_eq!(remembered(&fixture).whitespace, Whitespace::Show);
}

#[test]
fn the_menu_steps_whitespace_tab_width_and_changed_text_live_and_reset_puts_them_back() {
    use crate::review::settings::Row;
    use diffs_rs::engine::InlineChanges as Engine;
    let fixture = fixture("ws-menu");
    let mut screen = fixture.open_configured();
    buffer(&mut screen);
    keys(&mut screen, ",");
    let drawn = all(&buffer(&mut screen));
    for (label, value) in [("Whitespace", "show"), ("Tab width", "4 columns"), ("Changed text", "words")] {
        assert!(drawn.contains(&format!(" {label:<14}{value} ")), "{label}:\n{drawn}");
    }
    for _ in 0..menu_row(Row::Whitespace) {
        keys(&mut screen, "j");
    }
    let mut seen = Vec::new();
    for _ in 0..3 {
        press(&mut screen, KeyCode::Right);
        buffer(&mut screen);
        seen.push((screen.setting_value(Row::Whitespace), whitespace(&screen)));
    }
    assert_eq!(
        seen,
        [
            ("hide (like git -w)".to_owned(), diffs_rs::engine::Whitespace::IgnoreAll),
            ("hide amount changes (git -b)".to_owned(), diffs_rs::engine::Whitespace::IgnoreChange),
            ("show".to_owned(), diffs_rs::engine::Whitespace::Show),
        ]
    );
    press(&mut screen, KeyCode::Left);
    buffer(&mut screen);
    assert_eq!(screen.settings.now.whitespace, Whitespace::IgnoreChange, "left goes back");
    assert!(all(&buffer(&mut screen)).contains("1, 2, 3"), "-b keeps a space put where there was none");

    keys(&mut screen, "j");
    let mut widths = Vec::new();
    for code in [KeyCode::Right, KeyCode::Right, KeyCode::Left, KeyCode::Left] {
        press(&mut screen, code);
        buffer(&mut screen);
        widths.push(screen.diff.drawn_with().0);
    }
    assert_eq!(widths, [8, 2, 8, 4]);
    keys(&mut screen, "j");
    let mut marks = Vec::new();
    for _ in 0..4 {
        press(&mut screen, KeyCode::Enter);
        buffer(&mut screen);
        marks.push((screen.setting_value(Row::InlineChanges), screen.diff.drawn_with().1));
    }
    assert_eq!(
        marks,
        [
            ("words, joined".to_owned(), Engine::WordAlt),
            ("characters".to_owned(), Engine::Char),
            ("off".to_owned(), Engine::None),
            ("words".to_owned(), Engine::Word),
        ]
    );
    press(&mut screen, KeyCode::Left);
    keys(&mut screen, "k");
    press(&mut screen, KeyCode::Right);
    buffer(&mut screen);
    let remembered = remembered(&fixture);
    assert_eq!(
        (remembered.whitespace, remembered.tab_width.get(), remembered.inline_changes),
        (Whitespace::IgnoreChange, 8, InlineChanges::None)
    );

    // Reset: the three go back with the rest.
    for _ in 0..MENU.len() {
        keys(&mut screen, "j");
    }
    press(&mut screen, KeyCode::Enter);
    buffer(&mut screen);
    assert_eq!(screen.settings.now, ReviewConfig::DEFAULT);
    assert_eq!(
        screen.diff.drawn_with(),
        (4, diffs_rs::engine::InlineChanges::Word, diffs_rs::engine::Whitespace::Show)
    );
    assert!(!fixture.state().exists());
}

#[test]
fn review_sets_them_by_default_w_hides_as_it_says_and_a_width_between_steps_reaches_its_neighbours() {
    let fixture = fixture("ws-config");
    fixture
        .write_config("[review]\nwhitespace = \"ignore-change\"\ntab_width = 3\ninline_changes = \"char\"\n");
    let mut screen = fixture.open_configured();
    let drawn = buffer(&mut screen);
    assert_eq!(
        screen.diff.drawn_with(),
        (3, diffs_rs::engine::InlineChanges::Char, diffs_rs::engine::Whitespace::IgnoreChange),
        "from the first frame"
    );
    assert!(text(&drawn, 0).contains(" no ws "), "hidden from the start, and said");
    keys(&mut screen, "W");
    buffer(&mut screen);
    assert_eq!(whitespace(&screen), diffs_rs::engine::Whitespace::Show);
    keys(&mut screen, "W");
    buffer(&mut screen);
    assert_eq!(whitespace(&screen), diffs_rs::engine::Whitespace::IgnoreChange, "as [review] hides it");
    assert_eq!(
        screen.status.as_deref(),
        Some("changes in the amount of whitespace hidden (git -b) \u{b7} W shows them")
    );
    assert_eq!(screen.setting_value(crate::review::settings::Row::TabWidth), "3 columns");
    keys(&mut screen, ",");
    for _ in 0..menu_row(crate::review::settings::Row::TabWidth) {
        keys(&mut screen, "j");
    }
    press(&mut screen, KeyCode::Right);
    buffer(&mut screen);
    assert_eq!(screen.diff.drawn_with().0, 4);
    // Reset goes back to the config file's values.
    keys(&mut screen, "jjjjjj");
    press(&mut screen, KeyCode::Enter);
    buffer(&mut screen);
    assert_eq!(screen.diff.drawn_with().0, 3);
    assert_eq!(screen.status.as_deref(), Some("settings back to [review] in the config file"));
}

#[test]
fn notes_stay_on_their_lines_and_a_comment_on_a_pair_saves_its_file_side_and_line() {
    let fixture = fixture("ws-notes");
    let mut screen = fixture.open_configured();
    buffer(&mut screen);
    // Before hiding: a note on the real edit, one on a trailing-space line, one on a removed
    // line that hiding pairs.
    for (side, line) in [(Side::New, 6), (Side::New, 20), (Side::Old, 5)] {
        reveal(&mut screen, side, line);
        keys(&mut screen, "c");
        type_and_save(&mut screen, &format!("on {side:?} {line}"));
    }
    reveal(&mut screen, Side::New, 1);
    keys(&mut screen, "W");
    let hidden = buffer(&mut screen);
    let marked = |buffer: &Buffer, y: Option<u16>| y.is_some_and(|y| text(buffer, y).contains('\u{258c}'));
    assert!(marked(&hidden, row_of(&screen, Side::New, 6)), "the edit's note");
    assert!(marked(&hidden, row_of(&screen, Side::Old, 5)), "the paired line's note, on its pair");
    assert_eq!(row_of(&screen, Side::New, 20), None, "its hunk is a heading");
    // In the rail, the hidden one says why the diff cannot show it.
    let at = screen.rail_order().iter().position(|&i| screen.store.annotations()[i].line_start == 20);
    screen.rail_cursor = at.expect("in the rail");
    screen.show_selected_note();
    assert_eq!(
        screen.status.as_deref(),
        Some("its lines changed only whitespace, hidden now \u{b7} W shows them")
    );

    // Unified, while hidden: a comment on a pair row is on its new line.
    keys(&mut screen, "u");
    buffer(&mut screen);
    assert!(!screen.diff.split());
    reveal(&mut screen, Side::New, 7);
    keys(&mut screen, "c");
    type_and_save(&mut screen, "on the pair, unified");
    // Split, while hidden: on the old side of a pair, the old line.
    keys(&mut screen, "u");
    buffer(&mut screen);
    reveal(&mut screen, Side::Old, 8);
    keys(&mut screen, "c");
    type_and_save(&mut screen, "on the pair, old side");
    let notes: Vec<(String, Side, u32)> =
        screen.store.annotations().iter().map(|n| (n.file_path.clone(), n.side, n.line_start)).collect();
    for want in [("ws.go", Side::New, 7), ("ws.go", Side::Old, 8)] {
        assert!(notes.contains(&(want.0.to_owned(), want.1, want.2)), "{want:?} in {notes:?}");
    }
    // Shown again, every note is on its line: the added, the removed and the trailing-space ones.
    keys(&mut screen, "W");
    reveal(&mut screen, Side::New, 1);
    let shown = buffer(&mut screen);
    for (side, line) in [(Side::New, 6), (Side::New, 20), (Side::Old, 5), (Side::New, 7), (Side::Old, 8)] {
        assert!(marked(&shown, row_of(&screen, side, line)), "{side:?} {line}:\n{}", all(&shown));
    }
}

#[test]
fn a_selection_keeps_its_lines_through_a_toggle_or_is_cleared_when_its_hunk_hides() {
    let fixture = fixture("ws-selection");
    let mut screen = fixture.open_configured();
    buffer(&mut screen);
    // Growing in the re-indented hunk: kept, still growing, still on screen.
    reveal(&mut screen, Side::New, 5);
    keys(&mut screen, "vj");
    keys(&mut screen, "W");
    let hidden = buffer(&mut screen);
    let want = Lines { path: "ws.go".into(), side: Side::New, first: 5, last: 6 };
    assert_eq!(screen.diff.selection(), Some(want.clone()));
    assert!(screen.diff.selecting());
    assert!(screen.diff.rows_showing(&want).is_some(), "on screen:\n{}", all(&hidden));
    keys(&mut screen, "j");
    assert_eq!(screen.diff.selection().map(|l| l.last), Some(7), "it grows on");
    keys(&mut screen, "v");
    keys(&mut screen, "W");
    buffer(&mut screen);
    assert_eq!(screen.diff.selection(), Some(Lines { last: 7, ..want }), "shown again, the same lines");

    // In the trailing-space hunk: hiding it clears the selection, and `c` has nothing lit to use.
    press(&mut screen, KeyCode::Esc);
    reveal(&mut screen, Side::New, 20);
    keys(&mut screen, "vjv");
    assert!(screen.diff.selection().is_some());
    keys(&mut screen, "W");
    buffer(&mut screen);
    assert_eq!(screen.diff.selection(), None);
    assert_eq!(
        screen.status.as_deref(),
        Some("selection cleared: its lines changed only whitespace, now hidden")
    );
    keys(&mut screen, "c");
    assert_eq!(screen.mode, Mode::Browse, "the cursor is on the hunk's heading: no line to comment on");
    // A heading alone has no line to start a selection on.
    keys(&mut screen, "v");
    assert_eq!(screen.diff.selection(), None);
}

#[test]
fn a_selection_stays_in_its_hunk_and_never_reaches_into_a_hidden_one() {
    let fixture = fixture("ws-one-hunk");
    let mut screen = fixture.open_configured();
    buffer(&mut screen);
    keys(&mut screen, "W");
    buffer(&mut screen);
    // `v` from the re-indented hunk's last line, then down past its edge and the hidden hunk.
    reveal(&mut screen, Side::New, 10);
    keys(&mut screen, "v");
    for _ in 0..8 {
        keys(&mut screen, "j");
    }
    let selection = screen.diff.selection().expect("a selection");
    assert_eq!((selection.first, selection.last), (10, 11), "the hunk ends at 11");
    assert_eq!(screen.status.as_deref(), Some("a selection stays in one hunk"));
    keys(&mut screen, "v");
    press(&mut screen, KeyCode::Esc);

    // A drag from line 9 down over the hidden hunk's heading and below.
    buffer(&mut screen);
    let from = row_of(&screen, Side::New, 9).expect("line 9 on screen");
    let column = screen.geometry.diff.right() - 10;
    let event = |kind, row| Event::Mouse(MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE });
    screen.handle_event(event(MouseEventKind::Down(MouseButton::Left), from)).expect("press");
    for row in from..from + 12 {
        screen.handle_event(event(MouseEventKind::Drag(MouseButton::Left), row)).expect("drag");
    }
    screen.handle_event(event(MouseEventKind::Up(MouseButton::Left), from + 12)).expect("release");
    let selection = screen.diff.selection().expect("a selection");
    assert_eq!((selection.path.as_str(), selection.first, selection.last), ("ws.go", 9, 11));
}

#[test]
fn the_comment_box_takes_the_keys_and_the_settings_stay_while_it_is_open() {
    let fixture = fixture("ws-compose");
    let mut screen = fixture.open_configured();
    buffer(&mut screen);
    keys(&mut screen, "W");
    buffer(&mut screen);
    reveal(&mut screen, Side::New, 6);
    keys(&mut screen, "c");
    assert!(matches!(screen.mode, Mode::Compose(_)));
    keys(&mut screen, "W,uw");
    let chip = screen.geometry.whitespace_chip.expect("the chip");
    click(&mut screen, chip);
    buffer(&mut screen);
    assert_eq!(screen.compose.value(), "W,uw");
    assert_eq!(whitespace(&screen), diffs_rs::engine::Whitespace::IgnoreAll);
    assert_eq!(screen.settings.now.whitespace, Whitespace::IgnoreAll);
    assert!(!screen.settings.now.wrap);
    press(&mut screen, KeyCode::Enter);
    let note = screen.store.annotations().last().expect("saved");
    assert_eq!((note.file_path.as_str(), note.side, note.line_start), ("ws.go", Side::New, 6));
}

#[test]
fn moving_around_the_review_and_reading_it_again_keep_the_three_settings() {
    let fixture = fixture("ws-keep");
    fixture.write_config("[review]\nwhitespace = \"ignore-all\"\ntab_width = 8\ninline_changes = \"char\"\n");
    let mut screen = fixture.open_configured();
    let want = (8, diffs_rs::engine::InlineChanges::Char, diffs_rs::engine::Whitespace::IgnoreAll);
    let check = |screen: &mut Screen, what: &str| {
        buffer(screen);
        assert_eq!(screen.diff.drawn_with(), want, "after {what}");
        assert_eq!(screen.settings.now.whitespace, Whitespace::IgnoreAll, "after {what}");
    };
    check(&mut screen, "opening");
    for (moves, what) in
        [("n", "n"), ("N", "N"), ("]", "]"), ("[", "["), ("x", "a viewed mark"), ("x", "unmarking")]
    {
        keys(&mut screen, moves);
        check(&mut screen, what);
    }
    // The file filter, then the kind filter.
    keys(&mut screen, "/ws");
    press(&mut screen, KeyCode::Enter);
    check(&mut screen, "the file filter");
    keys(&mut screen, "f");
    press(&mut screen, KeyCode::Esc);
    check(&mut screen, "clearing it");
    keys(&mut screen, "t");
    press(&mut screen, KeyCode::Enter);
    check(&mut screen, "the kind filter");
    keys(&mut screen, "t");
    press(&mut screen, KeyCode::Enter);
    // The kind filter leaves the file list focused; `W` is the diff's key, as `w` is.
    screen.focus = crate::review::Focus::Diff;
    // Hidden, line 32 has no row; shown, the cursor on it, hidden again: the cursor goes to
    // its hunk's heading.
    assert!(screen.diff.reveal("ws.go", Side::New, 32).is_err(), "its hunk is hidden");
    keys(&mut screen, "W");
    buffer(&mut screen);
    reveal(&mut screen, Side::New, 32);
    keys(&mut screen, "W");
    check(&mut screen, "hiding with the cursor on a hidden line");
    assert_eq!(screen.diff.cursor().expect("cursor"), None, "on the heading");
    // Go to definition from `return sum`, a line drawn unchanged.
    reveal(&mut screen, Side::New, 8);
    keys(&mut screen, "d");
    settle_search(&mut screen);
    // `sum` is set at line 4 and used at 6: a list asks which; Enter takes the first.
    if matches!(screen.mode, Mode::Pick(_)) {
        press(&mut screen, KeyCode::Enter);
    }
    assert_eq!(screen.mode, Mode::Browse);
    check(&mut screen, "go to definition");
    assert!(screen.whole.is_none(), "the diff shows line 4 and line 6");
    // The whole file: its own viewer, drawn with the same tab width and marks.
    keys(&mut screen, "o");
    settle_whole(&mut screen);
    let whole = screen.whole.as_ref().and_then(|w| w.diff()).expect("the whole file");
    assert_eq!(whole.drawn_with(), want);
    keys(&mut screen, "o");
    check(&mut screen, "the whole file");
    keys(&mut screen, "r");
    settle(&mut screen);
    check(&mut screen, "a reload");
    assert!(text(&buffer(&mut screen), 0).contains(" no ws "));
}

#[test]
fn fifty_toggles_in_one_burst_end_in_the_frame_of_none_and_fifty_one_in_that_of_one() {
    let fixture = fixture("ws-burst");
    let frame_after = |presses: usize| {
        let mut screen = fixture.open_configured();
        buffer(&mut screen);
        // On a line of the trailing-space hunk, which hiding takes off the screen.
        reveal(&mut screen, Side::New, 21);
        buffer(&mut screen);
        for _ in 0..presses {
            keys(&mut screen, "W");
        }
        let drawn = buffer(&mut screen);
        let cursor = screen.diff.cursor_row().map(|row| (row.kind, row.new_line));
        let rows = pane(&screen, &drawn);
        screen.close().expect("close");
        // Each review leaves the next one its choice: start each from none.
        let _ = std::fs::remove_file(fixture.state());
        (rows, cursor, text(&drawn, 0))
    };
    let (none, one) = (frame_after(0), frame_after(1));
    assert_ne!(none, one, "one toggle changes the frame");
    assert_eq!(frame_after(50), none);
    assert_eq!(frame_after(51), one);
}

#[test]
fn the_reviews_footer_says_once_that_the_config_file_was_not_loaded() {
    let fixture = fixture("ws-config-notice");
    fixture.write_config("[ui]\ncopy_on_select = false\n[review]\ninline_changes = \"words\"\n");
    let (config, notice) = crate::config::Config::load_for_screen(&fixture.config());
    assert_eq!(config.review, ReviewConfig::DEFAULT, "defaults, as before");
    let mut screen = fixture.open();
    screen.add_notice(notice.expect("a notice"));
    let drawn = buffer(&mut screen);
    let footer =
        (0..ROWS).map(|y| text(&drawn, y)).find(|row| row.contains("config not loaded")).expect("said");
    assert!(footer.trim_start().starts_with("config not loaded: unknown variant `words`"), "{footer:?}");
    assert!(footer.contains("; using defaults ("), "{footer:?}");
}

#[test]
fn go_to_definition_opens_a_line_of_a_hidden_hunk_in_the_whole_file() {
    let comments: String =
        (6..=13).flat_map(|n| ["// c".to_owned(), n.to_string(), "\n".to_owned()]).collect();
    let before = format!(
        "package main\n\nfunc helper() int {{\n\treturn 1\n}}\n{comments}func use() int {{\n\treturn helper()+0\n}}\n"
    );
    let after = format!(
        "package main\n\nfunc helper() int {{\n    return 1\n}}\n{comments}func use() int {{\n\treturn helper() + 1\n}}\n"
    );
    let fixture = Fixture::new("ws-jump");
    fixture.write("j.go", &before);
    fixture.git(&["add", "."]);
    fixture.git(&["-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-qm", "go"]);
    fixture.write("j.go", &after);
    let mut screen = fixture.open_configured();
    keys(&mut screen, "W");
    buffer(&mut screen);
    assert!(screen.diff.reveal("j.go", Side::New, 3).is_err(), "the definition's hunk is a heading");
    screen.diff.reveal("j.go", Side::New, 15).expect("the call is shown");
    keys(&mut screen, "d");
    settle_search(&mut screen);
    settle_whole(&mut screen);
    let whole = screen.whole.as_ref().expect("the whole file");
    assert_eq!(whole.path, "j.go");
    assert_eq!(whole.target().expect("target").map(|(first, _)| first), Some(3));
}
