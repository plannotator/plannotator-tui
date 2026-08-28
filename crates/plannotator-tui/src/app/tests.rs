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
