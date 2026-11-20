//! The reading column on screen: code selected and noted through its bar and label, and the
//! rail, header and footer around a centred column at the widths people use.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use plannotator_tui_schema::{DocumentSource, Kind, Provenance};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{Event, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use crate::app::App;
use crate::delivery::Discard;
use crate::highlight::Role;
use crate::theme::palette;

const DOC: &str = "# Reading\n\nBefore the code.\n\n```rust\nfn main() {\n    let n = 42; // answer\n}\n```\n\nAfter the code.\n";
const CODE: &str = "fn main() {\n    let n = 42; // answer\n}";

fn reading_app() -> App {
    let source = DocumentSource::new(DOC.to_owned(), "code.md", true, Provenance::Stdin);
    let mut app = App::open(source, 60, Box::new(Discard)).expect("app opens");
    app.data_dir = std::env::temp_dir().join(format!("plannotator-tui-reading-{}", std::process::id()));
    app
}

fn frame(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal.draw(|frame| app.draw(frame)).expect("draw");
    terminal.backend().buffer().clone()
}

fn rows(buffer: &Buffer) -> Vec<String> {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .filter_map(|x| buffer.cell((x, y)))
                .map(ratatui::buffer::Cell::symbol)
                .collect()
        })
        .collect()
}

/// Screen column and row where `text` first starts.
fn find(buffer: &Buffer, text: &str) -> (u16, u16) {
    rows(buffer)
        .iter()
        .enumerate()
        .find_map(|(y, row)| {
            let at = row.find(text)?;
            let x = row[..at].chars().count();
            Some((u16::try_from(x).expect("fits"), u16::try_from(y).expect("fits")))
        })
        .expect("the text is on screen")
}

fn drag(app: &mut App, from: (u16, u16), to: (u16, u16)) {
    for (kind, (column, row)) in [
        (MouseEventKind::Down(MouseButton::Left), from),
        (MouseEventKind::Drag(MouseButton::Left), to),
        (MouseEventKind::Up(MouseButton::Left), to),
    ] {
        app.handle_event(&Event::Mouse(MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE }))
            .expect("mouse");
    }
}

fn selected(app: &App) -> &str {
    let pending = app.pending.as_ref().expect("a selection");
    &app.open.doc.source[pending.range.clone()]
}

#[test]
fn a_drag_over_a_whole_code_block_selects_its_code_and_not_its_fences() {
    let mut app = reading_app();
    let buffer = frame(&mut app, 120, 30);
    let (_, top) = find(&buffer, "fn main");
    let (_, bottom) = find(&buffer, "▐ }");
    // From the bar on the first row to past the label's column on the last.
    let doc = app.geometry.doc;
    drag(&mut app, (doc.x, top), (doc.right() - 1, bottom));
    assert_eq!(selected(&app), CODE);
}

#[test]
fn a_selection_inside_code_maps_to_the_source_characters_under_it() {
    let mut app = reading_app();
    let buffer = frame(&mut app, 120, 30);
    let (x, y) = find(&buffer, "42; // ans");
    drag(&mut app, (x, y), (x + 9, y));
    assert_eq!(selected(&app), "42; // ans");
}

#[test]
fn a_selection_across_a_code_block_runs_from_the_text_before_it_to_the_text_after() {
    let mut app = reading_app();
    let buffer = frame(&mut app, 120, 30);
    let start = find(&buffer, "Before");
    let (x, y) = find(&buffer, "After the code.");
    drag(&mut app, start, (x + 14, y));
    assert_eq!(selected(&app), &DOC[DOC.find("Before").expect("there")..DOC.len() - 1]);
}

#[test]
fn a_note_on_code_tints_it_and_the_rail_header_and_footer_keep_their_places() {
    for width in [60, 80, 120, 200] {
        let mut app = reading_app();
        app.add_quote_annotation("let n = 42", Kind::Comment, "check the answer".to_owned()).expect("note");
        let buffer = frame(&mut app, width, 30);
        let screen = rows(&buffer);
        let doc = app.geometry.doc;
        // Code is coloured on screen, and the note's tint sits on the code it covers.
        let (x, y) = find(&buffer, "fn main");
        assert_eq!(buffer[(x, y)].fg, Role::Keyword.style().fg.expect("a colour"), "{width}");
        let (x, y) = find(&buffer, "let n = 42");
        assert_eq!(buffer[(x, y)].bg, palette().comment_bg, "{width}");
        assert_eq!(buffer[(x + 9, y)].fg, Role::Number.style().fg.expect("a colour"), "{width}");
        // The column stays in the document pane, and the pane ends before the rail.
        let (title, _) = find(&buffer, "Reading");
        assert_eq!(title, doc.x, "{width}");
        let bubbles = &app.geometry.bubbles;
        if width < 80 {
            assert_eq!(bubbles.len(), 0, "{width}");
            assert!(screen.last().expect("footer").contains("rail hidden"), "{width}");
        } else {
            let (rect, _) = bubbles.first().expect("a bubble beside the column");
            assert!(rect.x > doc.right() && rect.right() == width, "{width}: {rect:?} {doc:?}");
            assert!(screen.iter().any(|row| row.contains("check the answer")), "{width}");
            // Centred: the margins either side of the column differ by at most one.
            let (left, right) = (doc.x - 2, rect.x - 1 - doc.right());
            assert!(left.abs_diff(right) <= 1, "{width}: {left} {right}");
        }
        assert!(screen[0].contains("Copy"), "{width}: header {:?}", screen[0]);
        assert!(screen.last().expect("footer").contains("code.md"), "{width}");
    }
}

/// A reading column narrower than the app must not make `t` think the tree is hidden: the
/// first press on a wide screen hides the tree that is showing.
#[test]
fn t_hides_a_folder_tree_shown_beside_a_reading_column() {
    let (root, mut app, _) = crate::app::review_test_support::folder_app("reading-tree");
    frame(&mut app, 200, 30);
    assert!(app.geometry.tree.width > 0, "the tree shows on a wide screen");
    crate::app::review_test_support::press(&mut app, 't');
    frame(&mut app, 200, 30);
    assert_eq!(app.geometry.tree.width, 0, "one press hides it");
    std::fs::remove_dir_all(&root).expect("cleanup");
}
