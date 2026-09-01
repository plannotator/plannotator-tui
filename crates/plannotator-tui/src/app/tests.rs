//! Behaviour of the header's Send button and the quit confirmation, drawn into a
//! `TestBackend` the way the `--snapshot` CLI does.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::path::PathBuf;

use plannotator_tui_schema::{DocumentSource, Kind, Provenance};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use super::send::SendState;
use super::{App, Mode};
use crate::delivery::{Delivery, Discard, HerdrAgent};

/// A transient source: the app runs exactly as it does on a file, but nothing is written
/// to the Plannotator data directory.
fn app(delivery: Box<dyn Delivery>) -> App {
    let source =
        DocumentSource::new("# Plan\n\nfirst thing\n".to_owned(), "plan.md", true, Provenance::Stdin);
    App::open(source, 60, delivery).expect("app opens")
}

fn agent() -> Box<dyn Delivery> {
    Box::new(HerdrAgent::new(PathBuf::from("/nonexistent/herdr"), "w1:p1".into(), Some("claude".into())))
}

/// One frame, as one string per screen row.
fn draw(app: &mut App) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(80, 20)).expect("terminal");
    terminal.draw(|frame| app.draw(frame)).expect("draw");
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .filter_map(|x| buffer.cell((x, y)))
                .map(|c| c.symbol().to_owned())
                .collect()
        })
        .collect()
}

fn row(rows: &[String], index: usize) -> &str {
    rows.get(index).map_or("", String::as_str)
}

#[test]
fn the_header_draws_the_send_button_and_records_where_it_is() {
    let mut app = app(agent());
    app.add_block_annotation(0, Kind::Comment, "x".to_owned()).expect("annotation");
    let rows = draw(&mut app);
    let header = row(&rows, 0);
    assert!(header.contains("Send 1 to claude in w1:p1 ▸"), "header was {header:?}");
    let rect = app.geometry.send_button.expect("button rect recorded");
    assert_eq!(rect.y, 0);
    assert_eq!(rect.right(), 80, "the button sits on the right edge");
}

#[test]
fn clicking_the_send_button_sends() {
    let mut app = app(Box::new(Discard));
    app.add_block_annotation(0, Kind::Comment, "x".to_owned()).expect("annotation");
    draw(&mut app);
    let rect = app.geometry.send_button.expect("button rect recorded");
    let click = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: rect.x + rect.width / 2,
        row: rect.y,
        modifiers: KeyModifiers::NONE,
    });
    app.handle_event(&click).expect("click");
    assert_eq!(app.send_state, SendState::Sent);
}

#[test]
fn quitting_with_unsent_feedback_asks_before_it_quits() {
    let mut app = app(agent());
    app.add_block_annotation(0, Kind::Comment, "x".to_owned()).expect("annotation");
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Char('q')))).expect("q");
    assert_eq!(app.mode, Mode::ConfirmQuit);
    assert!(!app.quit, "the question is asked instead of quitting");
    let rows = draw(&mut app);
    let footer = row(&rows, 19);
    assert!(footer.contains("before quitting? y send · n quit · esc cancel"), "footer was {footer:?}");
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Char('n')))).expect("n");
    assert!(app.quit, "n quits without sending");
    assert_eq!(app.send_state, SendState::Ready, "nothing was sent");
}

fn candidates() -> Vec<plannotator_tui_hosts::Message> {
    use plannotator_tui_hosts::{Message, Role};
    let message = |id: &str, text: &str, at: &str| Message {
        id: id.to_owned(),
        role: Role::Assistant,
        text: text.to_owned(),
        at: Some(at.to_owned()),
    };
    vec![
        message("m3", "# Third\n\nnewest message\n", "2026-08-28T12:41:00.000Z"),
        message("m2", "# Second\n\nmiddle message\n", "2026-08-28T12:38:00.000Z"),
        message("m1", "# First\n\noldest message\n", "2026-08-28T12:30:00.000Z"),
    ]
}

#[test]
fn the_picker_lists_newest_first_and_opens_the_chosen_message() {
    let mut app = App::open_message("claude", "/tmp/transcript.jsonl", candidates(), 60, Box::new(Discard))
        .expect("opens");
    app.clock_offset = 0;
    assert_eq!(app.mode, Mode::Pick, "more than one candidate asks which");
    let rows = draw(&mut app);
    let listed: Vec<&str> = rows.iter().map(String::as_str).filter(|r| r.contains("12:")).collect();
    assert_eq!(listed.len(), 3, "{rows:?}");
    assert!(listed[0].contains("12:41  # Third"), "{:?}", listed[0]);
    assert!(listed[2].contains("12:30  # First"), "{:?}", listed[2]);

    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Char('j')))).expect("j");
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Enter))).expect("enter");
    assert_eq!(app.mode, Mode::Browse);
    assert_eq!(app.open.doc.source, "# Second\n\nmiddle message\n");
    assert!(app.open.store.is_transient(), "a message is never written to disk");
    app.add_block_annotation(0, Kind::Comment, "x".to_owned()).expect("annotate");
    assert!(app.open.store.is_transient());
}

#[test]
fn escaping_the_picker_keeps_the_newest_message() {
    let mut app = App::open_message("claude", "/tmp/transcript.jsonl", candidates(), 60, Box::new(Discard))
        .expect("opens");
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Esc))).expect("esc");
    assert_eq!(app.mode, Mode::Browse);
    assert_eq!(app.open.doc.source, "# Third\n\nnewest message\n");
    assert_eq!(app.open.source.name, "claude · last message");
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Char('p')))).expect("p");
    assert_eq!(app.mode, Mode::Pick, "p reopens the picker");
}

/// A folder of `count` Markdown files named `f00.md`, `f01.md`, … in a fresh temp dir.
fn folder(count: usize) -> PathBuf {
    let root = std::env::temp_dir().join(format!("plannotator-tui-folder-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("mkdir");
    for i in 0..count {
        std::fs::write(root.join(format!("f{i:02}.md")), format!("# File {i}\n")).expect("write");
    }
    root
}

/// One frame at `width` × `height`, as one string per screen row.
fn draw_sized(app: &mut App, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal.draw(|frame| app.draw(frame)).expect("draw");
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .filter_map(|x| buffer.cell((x, y)))
                .map(|c| c.symbol().to_owned())
                .collect()
        })
        .collect()
}

fn open_path(app: &App) -> String {
    match &app.open.source.provenance {
        Provenance::File { path } => path.file_name().expect("name").to_string_lossy().into_owned(),
        _ => String::new(),
    }
}

#[test]
fn the_tree_scrolls_to_keep_the_cursor_visible_and_hit_tests_through_the_offset() {
    let root = folder(30);
    let mut app = App::open_folder(&root, 100, Box::new(Discard)).expect("folder opens");
    // 140 columns shows the tree; 20 rows leaves 18 for the body (header + footer).
    draw_sized(&mut app, 140, 20);
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Tab))).expect("tab");
    for _ in 0..25 {
        app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Char('j')))).expect("j");
    }
    assert_eq!(app.tree_cursor, 25);
    let rows = draw_sized(&mut app, 140, 20);
    assert_eq!(app.tree_scroll, 8, "the window slides so row 25 is the last visible row");
    assert!(row(&rows, 1).contains("f08.md"), "first drawn tree row was {:?}", row(&rows, 1));
    assert!(row(&rows, 18).contains("f25.md"), "last drawn tree row was {:?}", row(&rows, 18));
    let tree_pane: Vec<String> = rows[1..=18].iter().map(|r| r.chars().take(28).collect()).collect();
    assert!(!tree_pane.iter().any(|r| r.contains("f00.md")), "tree pane was {tree_pane:#?}");

    // Clicking the third visible row opens the file at scroll + 2, not row 2.
    let click = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 2,
        row: 3,
        modifiers: KeyModifiers::NONE,
    });
    app.handle_event(&click).expect("click");
    assert_eq!(app.tree_cursor, 10);
    assert_eq!(open_path(&app), "f10.md");

    // Moving back up pulls the window with the cursor.
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Tab))).expect("tab");
    for _ in 0..5 {
        app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Char('k')))).expect("k");
    }
    draw_sized(&mut app, 140, 20);
    assert_eq!((app.tree_cursor, app.tree_scroll), (5, 5));

    // The wheel over the tree scrolls the tree, not the document, and leaves the cursor alone.
    let wheel = Event::Mouse(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 2,
        row: 5,
        modifiers: KeyModifiers::NONE,
    });
    app.handle_event(&wheel).expect("wheel");
    assert_eq!((app.tree_cursor, app.tree_scroll, app.scroll), (5, 8, 0));
    // Wheel scrolling is clamped to the last full window of rows.
    for _ in 0..20 {
        app.handle_event(&wheel).expect("wheel");
    }
    assert_eq!(app.tree_scroll, 12, "30 rows in 18 lines: the window stops at 12");
    let rows = draw_sized(&mut app, 140, 20);
    assert!(row(&rows, 18).contains("f29.md"), "last tree row was {:?}", row(&rows, 18));
    std::fs::remove_dir_all(&root).expect("cleanup");
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

fn click_at(column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

#[test]
fn double_clicking_a_block_offers_the_toolbar_for_the_whole_block() {
    let mut app = app(Box::new(Discard));
    draw(&mut app);
    // "first thing" is the second block; single click starts a selection, not a toolbar.
    let (col, row) = (5, 3);
    app.handle_event(&click_at(col, row)).expect("first click");
    let rows = draw(&mut app);
    assert!(!rows.iter().any(|r| r.contains("looks good")), "no toolbar after one click");
    app.handle_event(&click_at(col, row)).expect("second click");
    let rows = draw(&mut app);
    assert!(rows.iter().any(|r| r.contains("looks good")), "toolbar after a double-click: {rows:?}");
    // The toolbar acts on the whole block: the 'a' key approves it.
    let placed_before = app.open.store.placed().len();
    app.handle_event(&key(KeyCode::Char('a'), KeyModifiers::NONE)).expect("approve");
    assert_eq!(app.open.store.placed().len(), placed_before + 1);
    let rows = draw(&mut app);
    assert!(rows.iter().any(|r| r.contains("👍")), "rail shows the approval: {rows:?}");
}

#[test]
fn a_comment_can_span_lines_and_enter_saves_it() {
    let mut app = app(Box::new(Discard));
    draw(&mut app);
    let (col, row) = (5, 3);
    app.handle_event(&click_at(col, row)).expect("click");
    app.handle_event(&click_at(col, row)).expect("double click");
    app.handle_event(&key(KeyCode::Char('c'), KeyModifiers::NONE)).expect("open compose");
    for c in "first line".chars() {
        app.handle_event(&key(KeyCode::Char(c), KeyModifiers::NONE)).expect("type");
    }
    // Shift+Enter and Alt+Enter both insert a newline; Ctrl+J too.
    app.handle_event(&key(KeyCode::Enter, KeyModifiers::SHIFT)).expect("shift+enter");
    for c in "second".chars() {
        app.handle_event(&key(KeyCode::Char(c), KeyModifiers::NONE)).expect("type");
    }
    app.handle_event(&key(KeyCode::Enter, KeyModifiers::ALT)).expect("alt+enter");
    for c in "third".chars() {
        app.handle_event(&key(KeyCode::Char(c), KeyModifiers::NONE)).expect("type");
    }
    let rows = draw(&mut app);
    assert!(rows.iter().any(|r| r.contains("first line")), "compose shows line one: {rows:?}");
    assert!(rows.iter().any(|r| r.contains("second")), "compose shows line two");
    assert!(rows.iter().any(|r| r.contains("alt+enter new line")), "hint shows the fallback key");
    app.handle_event(&key(KeyCode::Enter, KeyModifiers::NONE)).expect("save");
    let placed = app.open.store.placed();
    assert_eq!(placed.last().expect("annotation").annotation.body, "first line\nsecond\nthird");
}

#[test]
fn pasting_into_the_comment_box_keeps_newlines() {
    let mut app = app(Box::new(Discard));
    draw(&mut app);
    app.handle_event(&click_at(5, 3)).expect("click");
    app.handle_event(&click_at(5, 3)).expect("double click");
    app.handle_event(&key(KeyCode::Char('c'), KeyModifiers::NONE)).expect("compose");
    app.handle_event(&Event::Paste("pasted one\r\npasted two".to_owned())).expect("paste");
    app.handle_event(&key(KeyCode::Enter, KeyModifiers::NONE)).expect("save");
    let placed = app.open.store.placed();
    assert_eq!(placed.last().expect("annotation").annotation.body, "pasted one\npasted two");
}
