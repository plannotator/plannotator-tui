//! A mouse selection as a terminal in a window reports it: a report for each pixel the
//! pointer moves, so many for one cell, and many between two frames. Inside the text nothing
//! scrolls; held past its edge the document scrolls a row per step of a timer, as far as its
//! end; a report outside the terminal is ignored.

#![allow(clippy::expect_used, reason = "tests assert by panicking")]

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{Event, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use super::App;
use super::review_test_support::{draw, file_app, reopen};
use crate::ui::drag::STEP;

const COLS: u16 = 120;
const ROWS: u16 = 40;

/// A file app on `paragraphs` paragraphs, drawn once at `COLS`×`ROWS`.
fn long_app(tag: &str, paragraphs: usize) -> (std::path::PathBuf, App) {
    let (root, mut app, _) = file_app(tag);
    let mut text = String::new();
    for i in 0..paragraphs {
        writeln!(text, "Paragraph {i} has some words in it for a selection to cover.\n").expect("write");
    }
    std::fs::write(root.join("docs/a.md"), text).expect("document");
    reopen(&mut app);
    app.ui.copy_on_select = false;
    draw(&mut app, COLS, ROWS);
    (root, app)
}

fn report(app: &mut App, kind: MouseEventKind, (column, row): (u16, u16)) {
    app.handle_event(&Event::Mouse(MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE }))
        .expect("mouse");
}

fn moved(app: &mut App, at: (u16, u16)) {
    report(app, MouseEventKind::Drag(MouseButton::Left), at);
}

/// The selection's ends in document (row, column).
fn selected(app: &App) -> Option<((usize, usize), (usize, usize))> {
    app.selection.as_ref().map(super::selection::Selection::ordered)
}

/// A pointer going from `from` to `to` a row at a time, reported five times in each cell and
/// over two columns of it, as a terminal reporting pixels does.
fn wiggle(from: (u16, u16), to: (u16, u16)) -> Vec<(u16, u16)> {
    let rows: Vec<u16> =
        if to.1 >= from.1 { (from.1..=to.1).collect() } else { (to.1..=from.1).rev().collect() };
    rows.into_iter().flat_map(|y| [0, 0, 1, 1, 0].map(|dx| (to.0 + dx, y))).collect()
}

#[test]
fn drag_reports_in_a_burst_scroll_nothing_and_end_as_the_same_reports_one_per_frame() {
    let (root, mut framed) = long_app("drag-framed", 60);
    let (other, mut burst) = long_app("drag-burst", 60);
    let doc = framed.geometry.doc;
    let from = (doc.x + 2, doc.y + 6);
    // Down past the bottom, up past the top, and back to the middle.
    let path: Vec<(u16, u16)> = [
        wiggle(from, (from.0 + 10, doc.bottom())),
        wiggle((from.0 + 10, doc.bottom()), (from.0 + 10, doc.y - 1)),
        wiggle((from.0 + 10, doc.y - 1), (from.0 + 10, from.1 + 5)),
    ]
    .concat();
    report(&mut framed, MouseEventKind::Down(MouseButton::Left), from);
    report(&mut burst, MouseEventKind::Down(MouseButton::Left), from);
    for &at in &path {
        moved(&mut framed, at);
        draw(&mut framed, COLS, ROWS);
        moved(&mut burst, at);
    }
    assert_eq!((burst.scroll, framed.scroll), (0, 0));
    assert_eq!(selected(&burst), selected(&framed));
    assert_eq!(selected(&burst), Some(((6, 2), (11, 12))));
    let end = (from.0 + 10, from.1 + 5);
    report(&mut framed, MouseEventKind::Up(MouseButton::Left), end);
    report(&mut burst, MouseEventKind::Up(MouseButton::Left), end);
    let range = |app: &App| app.pending.as_ref().map(|p| p.range.clone());
    assert_eq!(range(&burst), range(&framed));
    assert!(range(&burst).is_some(), "the release selected text");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&other);
}

#[test]
fn held_past_the_bottom_the_document_scrolls_a_row_per_step_however_many_reports() {
    let (root, mut app) = long_app("drag-edge", 60);
    let doc = app.geometry.doc;
    report(&mut app, MouseEventKind::Down(MouseButton::Left), (doc.x + 2, doc.y + 6));
    for k in 0..300u16 {
        moved(&mut app, (doc.x + 2 + k % 7, doc.bottom()));
    }
    assert_eq!(app.scroll, 0, "no report scrolls");
    assert!(app.held_at_edge());
    let start = Instant::now();
    for (offset, scrolls) in [(0, true), (10, false), (49, false), (50, true), (60, false), (100, true)] {
        assert_eq!(app.step_at_edge(start + Duration::from_millis(offset)), scrolls, "at {offset} ms");
        draw(&mut app, COLS, ROWS);
    }
    assert_eq!(app.scroll, 3);
    let last_row = usize::from(doc.height) - 1;
    assert_eq!(
        selected(&app).map(|(_, head)| head.0),
        Some(3 + last_row),
        "the head is on the last row shown"
    );
    // Back inside the text the timer stops.
    moved(&mut app, (doc.x + 2, doc.bottom() - 2));
    assert!(!app.held_at_edge());
    assert!(!app.step_at_edge(start + STEP * 10));
    assert_eq!(app.scroll, 3);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn held_past_the_bottom_the_document_stops_at_its_end() {
    let (root, mut app) = long_app("drag-end", 40);
    let doc = app.geometry.doc;
    report(&mut app, MouseEventKind::Down(MouseButton::Left), (doc.x + 2, doc.y + 6));
    moved(&mut app, (doc.x + 2, doc.bottom()));
    let start = Instant::now();
    let mut steps = 0;
    for n in 0..300 {
        steps += usize::from(app.step_at_edge(start + STEP * n));
        draw(&mut app, COLS, ROWS);
    }
    let end = app.open.layout.total_rows - usize::from(doc.height);
    assert_eq!((steps, app.scroll), (end, end), "a row a step, to the document's end");
    report(&mut app, MouseEventKind::Up(MouseButton::Left), (doc.x + 2, doc.bottom()));
    assert!(app.pending.is_some(), "the release selected text");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn reports_outside_the_terminal_are_ignored_and_a_release_there_still_ends_the_drag() {
    let (root, mut app) = long_app("drag-off-screen", 60);
    let doc = app.geometry.doc;
    report(&mut app, MouseEventKind::Down(MouseButton::Left), (doc.x + 2, doc.y + 6));
    moved(&mut app, (doc.x + 8, doc.y + 9));
    let before = selected(&app);
    // Pixel positions, rows past the terminal, and the largest a report can hold.
    let nonsense = [(doc.x + 8, ROWS), (doc.x + 8, 900), (1200, doc.y + 3), (u16::MAX, u16::MAX)];
    for at in nonsense {
        for kind in [
            MouseEventKind::Drag(MouseButton::Left),
            MouseEventKind::Down(MouseButton::Left),
            MouseEventKind::ScrollDown,
        ] {
            report(&mut app, kind, at);
        }
    }
    assert!(!app.held_at_edge());
    assert!(!app.step_at_edge(Instant::now()));
    assert_eq!(app.scroll, 0);
    assert_eq!(selected(&app), before);
    assert!(app.selection.is_some_and(|s| s.dragging), "still dragging");
    report(&mut app, MouseEventKind::Up(MouseButton::Left), (doc.x + 8, 900));
    assert!(app.selection.is_none_or(|s| !s.dragging), "the release ended it");
    assert!(app.pending.is_some(), "the release selected text");
    let _ = std::fs::remove_dir_all(&root);
}
