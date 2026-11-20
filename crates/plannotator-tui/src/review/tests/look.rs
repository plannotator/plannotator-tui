//! The code review draws on the terminal's own background, as the document screen does: only
//! changed lines and hunk headings carry a colour of their own.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Color;

use super::*;

fn buffer(screen: &mut Screen) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(160, 40)).expect("terminal");
    let mut drawn = Ok(());
    terminal.draw(|f| drawn = screen.draw(f)).expect("draw");
    drawn.expect("viewer draw");
    screen.after_draw();
    terminal.backend().buffer().clone()
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
