//! The code review draws on the terminal's own background, as the document screen does: only
//! changed lines and hunk headings carry a colour of their own. The whole-file view's engine
//! does the same, and so does a light terminal's palette.

use std::time::Duration;

use plannotator_tui_review::Request;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Color;

use super::whole_flow::settle_whole;
use super::*;
use crate::review::diff::{Diff, Progress};

fn buffer(screen: &mut Screen) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(160, 40)).expect("terminal");
    let mut drawn = Ok(());
    terminal.draw(|f| drawn = screen.draw(f)).expect("draw");
    drawn.expect("viewer draw");
    screen.after_draw();
    terminal.backend().buffer().clone()
}

/// Every cell's symbol, foreground and background, row by row.
fn cells(buffer: &Buffer) -> Vec<Vec<(String, Color, Color)>> {
    let width = buffer.area.width;
    (0..buffer.area.height)
        .map(|y| {
            (0..width)
                .filter_map(|x| buffer.cell((x, y)).map(|c| (c.symbol().to_owned(), c.fg, c.bg)))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn row(buffer: &Buffer, text: &str) -> Vec<(String, Color, Color)> {
    let width = buffer.area.width;
    (0..buffer.area.height)
        .map(|y| {
            (0..width)
                .filter_map(|x| buffer.cell((x, y)).map(|c| (c.symbol().to_owned(), c.fg, c.bg)))
                .collect::<Vec<_>>()
        })
        .find(|cells| cells.iter().map(|(s, _, _)| s.as_str()).collect::<String>().contains(text))
        .unwrap_or_else(|| panic!("no row with {text:?}"))
}

#[test]
fn the_diff_leaves_context_and_empty_rows_to_the_terminal_background() {
    let fixture = Fixture::new("look");
    // Two files, so the viewer shows its file list.
    fixture.write("notes.txt", "changed\n");
    let mut screen = fixture.open();
    let buffer = buffer(&mut screen);
    // A plain context line and the file list: no background, and plain text in the
    // terminal's own colour.
    let context = row(&buffer, "def add(a, b):");
    assert!(context.iter().all(|(_, _, bg)| *bg == Color::Reset), "{context:?}");
    assert!(context.iter().any(|(s, fg, _)| s != " " && *fg == Color::Reset), "{context:?}");
    let files = row(&buffer, "Files");
    assert!(files.iter().all(|(_, _, bg)| *bg == Color::Reset), "{files:?}");
    // A changed line keeps its tint.
    let added = row(&buffer, "total = a + b");
    assert!(added.iter().any(|(_, _, bg)| matches!(bg, Color::Rgb(..))), "{added:?}");
}

#[test]
fn the_whole_file_view_leaves_its_lines_and_empty_rows_to_the_terminal_background_too() {
    let fixture = Fixture::new("look-whole");
    let mut screen = fixture.open();
    at(&mut screen, 9);
    keys(&mut screen, "o");
    settle_whole(&mut screen);
    let buffer = buffer(&mut screen);
    let rows = cells(&buffer);
    let text =
        |cells: &[(String, Color, Color)]| cells.iter().map(|(s, _, _)| s.as_str()).collect::<String>();
    assert!(text(&rows[0]).starts_with(" Whole file · new · app.py "), "{:?}", text(&rows[0]));
    // Below the header and the file's heading, everything the view draws: the file's lines,
    // changed or not, and the rows under its last line.
    let last = row(&buffer, "print(add(1, 2))");
    assert!(last.iter().all(|(_, _, bg)| *bg == Color::Reset), "{last:?}");
    assert!(last.iter().any(|(s, fg, _)| s != " " && *fg == Color::Reset), "{last:?}");
    let footer = rows.len() - 3;
    let below = rows.iter().position(|r| text(r).contains("print(add(1, 2))")).expect("last line") + 1;
    assert!(below < footer, "the file is shorter than the screen");
    for (y, cells) in rows.iter().enumerate().take(footer).skip(below) {
        assert!(cells.iter().all(|(_, _, bg)| *bg == Color::Reset), "row {y}: {cells:?}");
    }
}

/// The fixture's diff drawn by a viewer of its own, light or dark, at 160×40.
fn viewer(fixture: &Fixture, light: bool) -> Buffer {
    let (_repo, capture) =
        crate::review::capture(&fixture.repo(), &Request::Uncommitted { base: None }).expect("capture");
    let (wake, _ready) = std::sync::mpsc::sync_channel(1);
    let paths: Vec<String> = capture.files.iter().map(|f| f.path.clone()).collect();
    let settings = crate::config::review::ReviewConfig::DEFAULT;
    let mut diff = Diff::open(&capture, &paths, settings, light, wake).expect("viewer");
    let mut terminal = Terminal::new(TestBackend::new(160, 40)).expect("terminal");
    let started = Instant::now();
    loop {
        diff.poll(&capture).expect("poll");
        let mut drawn = Ok(());
        terminal.draw(|f| drawn = diff.draw(f, f.area())).expect("draw");
        drawn.expect("viewer draw");
        diff.after_draw();
        if diff.files_shown() == capture.files.len() && diff.progress() == Progress::Ready {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "the viewer never loaded");
        std::thread::sleep(Duration::from_millis(10));
    }
    let buffer = terminal.backend().buffer().clone();
    let _ = diff.close();
    buffer
}

/// How light a colour is, 0 to 255; `None` for one the terminal picks.
fn lightness(color: Color) -> Option<u32> {
    match color {
        Color::Rgb(r, g, b) => Some((u32::from(r) + u32::from(g) + u32::from(b)) / 3),
        _ => None,
    }
}

#[test]
fn a_light_terminal_keeps_its_own_background_with_light_tints_on_changed_lines() {
    let fixture = Fixture::new("look-light");
    fixture.write("notes.txt", "changed\n");
    for light in [true, false] {
        let buffer = viewer(&fixture, light);
        let context = row(&buffer, "def add(a, b):");
        assert!(context.iter().all(|(_, _, bg)| *bg == Color::Reset), "light {light}: {context:?}");
        let added = row(&buffer, "total = a + b");
        let tint = added.iter().find_map(|(_, _, bg)| lightness(*bg)).expect("a tint on the added line");
        if light {
            assert!(tint > 160, "a light tint on a light terminal: {added:?}");
        } else {
            assert!(tint < 96, "a dark tint on a dark terminal: {added:?}");
        }
        // The text on the tint is readable against it: dark on light, light on dark.
        let ink = added.iter().filter(|(s, _, _)| s.trim() != "").find_map(|(_, fg, _)| lightness(*fg));
        if let Some(ink) = ink {
            assert!(if light { ink < tint } else { ink > tint }, "light {light}: {added:?}");
        }
    }
}
