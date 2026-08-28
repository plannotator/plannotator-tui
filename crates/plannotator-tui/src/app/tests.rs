//! Behaviour of the header's Send button and the quit confirmation, drawn into a
//! `TestBackend` the way the `--snapshot` CLI does.

#![allow(clippy::expect_used, reason = "tests assert by panicking")]

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
    assert!(header.contains("plannotator-tui"), "the brand still fits: {header:?}");
    let rect = app.geometry.send_button.expect("button rect recorded");
    assert_eq!(rect.y, 0);
    assert!(rect.right() <= 80);
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
