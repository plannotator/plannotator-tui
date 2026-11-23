//! The mouse never ends the review and never leaves a drag behind: a release over lines no
//! longer shown applies nothing, any other event first drops the drag and its highlight, a
//! drag and `v` stay in their hunk, and the toolbar, the shift-click and the shading after a
//! drag keep to the lines selected.

use plannotator_tui_review::Side;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind};

use super::mouse_flow::{COLS, ROWS, drag, draw, lines, locate, long_fixture, mouse, release};
use super::place_flow::mixed;
use super::*;
use crate::review::diff::Lines;
use crate::review::mouse::{Drag, selection_bg};
use crate::review::{Draft, Mode};

/// The screen rows of the diff pane with any cell in the selection colour, in a new frame.
fn lit(screen: &mut Screen) -> Vec<u16> {
    let buffer = draw(screen);
    tinted(screen, &buffer)
}

/// The screen rows of the diff pane with any cell in the selection colour.
fn tinted(screen: &Screen, buffer: &Buffer) -> Vec<u16> {
    let pane = screen.geometry.diff;
    (pane.top()..pane.bottom())
        .filter(|&y| (pane.left()..pane.right()).any(|x| buffer[(x, y)].bg == selection_bg()))
        .collect()
}

fn down(screen: &mut Screen, at: (u16, u16)) {
    mouse(screen, MouseEventKind::Down(MouseButton::Left), at);
}

/// A drag over new lines 8 to 10 of app.py, the button still held.
fn hold_a_drag(screen: &mut Screen) -> (u16, u16) {
    let from = locate(screen, "def add(a, b):", true);
    let to = locate(screen, "return total", true);
    drag(screen, from, to);
    assert_eq!(screen.selected_lines(), Some(lines(Side::New, 8, 10)));
    assert_eq!(lit(screen).len(), 3, "the drag is lit");
    to
}

#[test]
fn a_release_over_files_reloaded_under_the_drag_selects_nothing_and_the_review_stays() {
    let fixture = Fixture::new("guard-reload");
    let mut screen = fixture.open();
    let to = hold_a_drag(&mut screen);
    // app.py back as committed, another file changed: the reload drops app.py from the
    // diff. Nothing but the reload happens between the press and the release.
    fixture.write("app.py", BEFORE);
    fixture.write("notes.txt", "changed\n");
    screen.reload().expect("reload");
    settle(&mut screen);
    assert_eq!(screen.diff.paths(), ["notes.txt"]);
    release(&mut screen, to);
    assert_eq!(screen.diff.selection(), None);
    assert!(!screen.wants_quit());
    assert_eq!(screen.status.as_deref(), Some("those lines are no longer shown; nothing selected"));
}

#[test]
fn an_error_from_the_mouse_is_said_in_the_footer_and_the_review_stays() {
    let fixture = Fixture::new("guard-error");
    let mut screen = fixture.open();
    // Lines the diff does not show, in the diff it does: the viewer refuses to select them.
    let generation = screen.diff.generation();
    screen.drag = Some(Drag::over(lines(Side::New, 400, 402), generation));
    let at = locate(&mut screen, "return total", true);
    let event = MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: at.0,
        row: at.1,
        modifiers: KeyModifiers::NONE,
    };
    screen.handle_event(Event::Mouse(event)).expect("the review goes on");
    assert!(screen.status.is_some(), "the footer says what failed");
    assert!(screen.drag.is_none());
    assert_eq!(screen.diff.selection(), None);
    assert!(!screen.wants_quit());
}

#[test]
fn the_reported_ways_to_end_the_review_by_mouse_end_nothing() {
    let fixture = Fixture::new("guard-repro");
    let mut screen = fixture.open();
    // A drag, `?` with the button held, the release over the help, Esc, a filter that
    // matches nothing, a click.
    let to = hold_a_drag(&mut screen);
    keys(&mut screen, "?");
    assert_eq!(screen.mode, Mode::Help);
    release(&mut screen, to);
    press(&mut screen, KeyCode::Esc);
    keys(&mut screen, "/zzz");
    press(&mut screen, KeyCode::Enter);
    down(&mut screen, to);
    release(&mut screen, to);
    assert!(!screen.wants_quit());
    assert_eq!(screen.diff.selection(), None);
    keys(&mut screen, "/");
    press(&mut screen, KeyCode::Esc);
    settle(&mut screen);

    // `r` with the button held, then the release.
    let to = hold_a_drag(&mut screen);
    keys(&mut screen, "r");
    settle(&mut screen);
    release(&mut screen, to);
    assert_eq!(screen.diff.selection(), None);
    assert!(screen.drag.is_none());

    // `o` and `O` with the button held, then the release over the whole file.
    for key in ["o", "O"] {
        let to = hold_a_drag(&mut screen);
        keys(&mut screen, key);
        assert!(screen.whole.is_some(), "{key} opened the whole file");
        assert!(screen.drag.is_none(), "{key} dropped the drag");
        whole_flow::settle_whole(&mut screen);
        release(&mut screen, to);
        assert!(screen.drag.is_none());
        assert_eq!(
            screen.active_diff().and_then(crate::review::diff::Diff::selection),
            None,
            "{key}: nothing selected"
        );
        press(&mut screen, KeyCode::Esc);
        assert!(screen.whole.is_none());
        settle(&mut screen);
    }
    assert!(!screen.wants_quit());
}

#[test]
fn anything_but_the_release_drops_a_drag_and_its_highlight() {
    let fixture = Fixture::new("guard-cancel");
    let paste = Event::Paste("x".into());
    let key = Event::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    let help = Event::Key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
    let moved = |at: (u16, u16)| {
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column: at.0,
            row: at.1,
            modifiers: KeyModifiers::NONE,
        })
    };
    for (what, event) in [
        ("a key", Some(key)),
        ("the help", Some(help)),
        ("a paste", Some(paste)),
        ("focus lost", Some(Event::FocusLost)),
        ("a resize", Some(Event::Resize(COLS, ROWS))),
        ("a move with no button", None),
    ] {
        let mut screen = fixture.open();
        let to = hold_a_drag(&mut screen);
        screen.handle_event(event.unwrap_or_else(|| moved(to))).expect(what);
        assert!(screen.drag.is_none(), "{what}: the drag is gone");
        if screen.mode != Mode::Browse {
            press(&mut screen, KeyCode::Esc);
        }
        assert_eq!(lit(&mut screen), Vec::<u16>::new(), "{what}: nothing lit");
        release(&mut screen, to);
        assert_eq!(screen.diff.selection(), None, "{what}: the release selects nothing");
        // What `c` takes is what is lit: the cursor's line alone.
        let cursor = screen.diff.cursor().expect("cursor").expect("on a line");
        keys(&mut screen, "c");
        assert_eq!(screen.mode, Mode::Compose(Draft::Comment(cursor)), "{what}");
    }

    // A second press while dragging (the first release lost) starts afresh from its line.
    let mut screen = fixture.open();
    let to = hold_a_drag(&mut screen);
    let other = locate(&mut screen, "print(add(1, 2))", true);
    down(&mut screen, other);
    mouse(&mut screen, MouseEventKind::Drag(MouseButton::Left), (other.0, other.1 - 1));
    release(&mut screen, (other.0, other.1 - 1));
    assert_eq!(screen.diff.selection(), Some(lines(Side::New, 14, 15)), "not from {to:?}");
}

#[test]
fn a_lost_release_on_a_line_between_panes_leaves_the_next_drag_its_own() {
    let fixture = Fixture::new("guard-divider");
    fixture.write("notes.txt", "changed\n");
    let mut screen = fixture.open();
    frame(&mut screen, COLS, ROWS);
    let list = screen.geometry.files.right();
    let width = screen.geometry.files.width;
    down(&mut screen, (list, 5));
    mouse(&mut screen, MouseEventKind::Drag(MouseButton::Left), (list + 4, 5));
    // The release went elsewhere; the next press is on a line of code.
    frame(&mut screen, COLS, ROWS);
    assert_eq!(screen.geometry.files.width, width + 4);
    let from = locate(&mut screen, "def add(a, b):", true);
    let to = locate(&mut screen, "return total", true);
    drag(&mut screen, from, to);
    release(&mut screen, to);
    assert_eq!(screen.diff.selection(), Some(lines(Side::New, 8, 10)));
    frame(&mut screen, COLS, ROWS);
    assert_eq!(screen.geometry.files.width, width + 4, "the list kept the width it was dragged to");
    assert!(!screen.dividers.dragging());
}

/// nn.txt's new lines: its first hunk runs from 9 to 15 (line 12 changed), the second from
/// 30 to 37 (line 33 split in two).
fn nn(tag: &str) -> (Fixture, Screen) {
    let fixture = mixed(tag);
    let mut screen = fixture.open();
    press(&mut screen, KeyCode::Char('u'));
    screen.diff.reveal("nn.txt", Side::New, 12).expect("line 12");
    draw(&mut screen);
    (fixture, screen)
}

fn nn_lines(first: u32, last: u32) -> Lines {
    Lines { path: "nn.txt".into(), side: Side::New, first, last }
}

#[test]
fn a_drag_stops_at_the_edge_of_the_hunk_it_started_in() {
    let (_fixture, mut screen) = nn("guard-hunk");
    let from = locate(&mut screen, "line twelve", true);
    let to = locate(&mut screen, "line 33b", true);
    drag(&mut screen, from, to);
    assert_eq!(screen.selected_lines(), Some(nn_lines(12, 15)), "while dragging");
    release(&mut screen, to);
    assert_eq!(screen.diff.selection(), Some(nn_lines(12, 15)));
    // And up, from the second hunk.
    press(&mut screen, KeyCode::Esc);
    let from = locate(&mut screen, "line twelve", true);
    let to = locate(&mut screen, "line 33b", true);
    drag(&mut screen, to, from);
    release(&mut screen, from);
    assert_eq!(screen.diff.selection(), Some(nn_lines(30, 34)));
}

#[test]
fn the_shading_between_selected_rows_leaves_gaps_and_headings_alone() {
    let (_fixture, mut screen) = nn("guard-gap-tint");
    // Lines across both hunks, as a note made before could hold.
    screen.diff.select(&nn_lines(13, 31)).expect("select");
    let shaded = lit(&mut screen);
    for needle in ["line 13", "line 15", "line 30", "line 31"] {
        let row = locate(&mut screen, needle, true).1;
        assert!(shaded.contains(&row), "{needle} is shaded: {shaded:?}");
    }
    let header = locate(&mut screen, "@@ -30,", true).1;
    assert!(!shaded.contains(&header), "the hunk's heading is not shaded");
    assert!(!shaded.contains(&(header - 1)), "the gap above it is not shaded");
}

#[test]
fn v_stops_at_the_edge_of_its_hunk_and_keeps_growing_inside_it() {
    let (_fixture, mut screen) = nn("guard-v");
    screen.diff.reveal("nn.txt", Side::New, 13).expect("line 13");
    keys(&mut screen, "v");
    keys(&mut screen, "jjjjjjjjjj");
    assert_eq!(screen.diff.selection(), Some(nn_lines(13, 15)));
    assert_eq!(screen.diff.cursor().expect("cursor"), Some(nn_lines(15, 15)));
    assert_eq!(screen.status.as_deref(), Some("a selection stays in one hunk"));
    assert!(screen.diff.selecting(), "still growing");
    keys(&mut screen, "k");
    assert_eq!(screen.diff.selection(), Some(nn_lines(13, 14)));
    // Up past the hunk's first line, from below.
    keys(&mut screen, "kkkkkkkkkk");
    assert_eq!(screen.diff.selection(), Some(nn_lines(9, 13)));
    keys(&mut screen, "v");
    assert_eq!(screen.diff.selection(), Some(nn_lines(9, 13)));
}

#[test]
fn a_click_on_the_toolbar_beside_its_buttons_does_nothing() {
    let fixture = Fixture::new("guard-toolbar-pad");
    let mut screen = fixture.open();
    let to = hold_a_drag(&mut screen);
    release(&mut screen, to);
    draw(&mut screen);
    let bar = screen.geometry.toolbar_bar.expect("the toolbar");
    let cursor = screen.diff.cursor().expect("cursor");
    let pad = (bar.x, bar.y);
    assert!(!screen.geometry.toolbar.iter().any(|(rect, _)| rect.contains(pad.into())), "padding");
    down(&mut screen, pad);
    release(&mut screen, pad);
    assert_eq!(screen.mode, Mode::Browse);
    assert_eq!(screen.diff.selection(), Some(lines(Side::New, 8, 10)));
    assert_eq!(screen.diff.cursor().expect("cursor"), cursor);
    assert!(frame(&mut screen, COLS, ROWS).iter().any(|r| r.contains("comment (c)")), "the toolbar stays");
}

#[test]
fn after_a_drag_up_to_the_top_row_the_toolbar_covers_no_selected_row() {
    let fixture = long_fixture("guard-toolbar-top");
    let mut screen = fixture.open();
    let first = locate(&mut screen, "v20 = 20", true);
    for _ in 0..4 {
        mouse(&mut screen, MouseEventKind::ScrollDown, first);
    }
    let from = locate(&mut screen, "v30 = 30", true);
    let top = screen.geometry.diff.y;
    drag(&mut screen, from, (from.0, top));
    release(&mut screen, (from.0, top));
    let shaded = lit(&mut screen);
    let bar = screen.geometry.toolbar_bar.expect("the toolbar");
    assert!(!shaded.contains(&bar.y), "the toolbar on row {} covers none of {shaded:?}", bar.y);
    assert!(shaded.len() > 3);
}

#[test]
fn an_upward_drag_that_scrolled_keeps_the_view_where_it_went_on_release() {
    let fixture = long_fixture("guard-up-scroll");
    let mut screen = fixture.open();
    let first = locate(&mut screen, "v20 = 20", true);
    for _ in 0..10 {
        mouse(&mut screen, MouseEventKind::ScrollDown, first);
    }
    let from = locate(&mut screen, "v50 = 50", true);
    let top = screen.geometry.diff.y;
    drag(&mut screen, from, (from.0, top + 1));
    for _ in 0..20 {
        mouse(&mut screen, MouseEventKind::Drag(MouseButton::Left), (from.0, top - 1));
        draw(&mut screen);
    }
    let before: Vec<String> = frame(&mut screen, COLS, ROWS).iter().map(|r| diff_side(&screen, r)).collect();
    let selected = screen.selected_lines().expect("dragging");
    release(&mut screen, (from.0, top - 1));
    assert_eq!(screen.diff.selection(), Some(selected));
    let after: Vec<String> = frame(&mut screen, COLS, ROWS).iter().map(|r| diff_side(&screen, r)).collect();
    let bar = screen.geometry.toolbar_bar.expect("the toolbar");
    let pane = screen.geometry.diff;
    for (y, (before, after)) in before.iter().zip(&after).enumerate() {
        let y16 = u16::try_from(y).expect("row");
        if y16 != bar.y && (pane.top()..pane.bottom()).contains(&y16) {
            assert_eq!(before, after, "row {y} stayed");
        }
    }
}

#[test]
fn a_shift_click_after_a_drag_up_grows_from_the_line_first_pressed() {
    let fixture = Fixture::new("guard-shift");
    let mut screen = fixture.open();
    let from = locate(&mut screen, "return total", true);
    let to = locate(&mut screen, "def add(a, b):", true);
    drag(&mut screen, from, to);
    release(&mut screen, to);
    assert_eq!(screen.diff.selection(), Some(lines(Side::New, 8, 10)));
    let further = locate(&mut screen, "print(add(1, 2))", true);
    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: further.0,
        row: further.1,
        modifiers: KeyModifiers::SHIFT,
    };
    screen.handle_event(Event::Mouse(click)).expect("shift-click");
    assert_eq!(screen.diff.selection(), Some(lines(Side::New, 10, 15)));
}

#[test]
fn a_bar_in_the_code_at_the_middle_column_leaves_a_unified_row_whole() {
    let fixture = Fixture::new("guard-bar");
    let bars = "\u{2502}".repeat(110);
    fixture.write("app.py", &AFTER.replace("import sys", &format!("bars = \"{bars}\"")));
    let mut screen = fixture.open();
    press(&mut screen, KeyCode::Char('u'));
    at(&mut screen, 1);
    keys(&mut screen, "vj");
    let buffer = draw(&mut screen);
    let pane = screen.geometry.diff;
    let y = locate(&mut screen, "bars = ", true).1;
    for x in pane.left()..pane.right() {
        assert_eq!(buffer[(x, y)].bg, selection_bg(), "column {x} of the row");
    }
}

#[test]
fn the_line_under_the_pointer_unlights_when_the_window_loses_focus() {
    let fixture = Fixture::new("guard-unlight");
    fixture.write("notes.txt", "changed\n");
    let mut screen = fixture.open();
    draw(&mut screen);
    let line = screen.geometry.files.right();
    let lit = |screen: &mut Screen| draw(screen)[(line, 5)].fg == crate::theme::palette().accent;
    mouse(&mut screen, MouseEventKind::Moved, (line, 5));
    assert!(lit(&mut screen));
    screen.handle_event(Event::FocusLost).expect("focus lost");
    assert!(!lit(&mut screen));
}
