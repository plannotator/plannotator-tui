#![allow(clippy::expect_used, reason = "tests assert by panicking")]
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use plannotator_tui_schema::{DocumentSource, Kind, Provenance};
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};

use crate::app::{App, Focus, Mode};
use crate::delivery::Discard;
use crate::tree::Tree;

fn fixture() -> (PathBuf, App) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "plannotator-search-app-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(root.join("docs/deep")).expect("mkdir");
    for (name, content) in [("plan.md", "# Root\n\none\n"), ("docs/deep/plan.md", "# Nested\n\ntwo\n")] {
        std::fs::write(root.join(name), content).expect("write");
    }
    let mut app = App::open(
        DocumentSource::new("# Original".into(), "original", true, Provenance::Stdin),
        60,
        Box::new(Discard),
    )
    .expect("app");
    app.data_dir = root.join("data");
    app.tree = Some(Tree::scan(&root).expect("tree"));
    (root, app)
}

fn key(app: &mut App, code: KeyCode) {
    app.handle_event(&Event::Key(KeyEvent::from(code))).expect("key");
}

fn start(app: &mut App) {
    app.handle_event(&Event::Key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL))).expect("start");
    assert_eq!(app.mode, Mode::FileSearch);
}

fn finish_scan(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.file_search.as_ref().is_some_and(|s| !s.done) {
        app.poll_file_search();
        assert!(Instant::now() < deadline, "scan must finish");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn screen(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal.draw(|frame| app.draw(frame)).expect("draw");
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            (0..width)
                .filter_map(|x| buffer.cell((x, y)))
                .map(ratatui::buffer::Cell::symbol)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn cancelling_keeps_document_focus_scroll_and_selection_and_discards_old_scan() {
    let (root, mut app) = fixture();
    app.focus = Focus::Tree;
    app.scroll = 7;
    app.selected = 3;
    app.cursor = (7, 2);
    start(&mut app);
    key(&mut app, KeyCode::Char('j'));
    assert_eq!(app.file_search.as_ref().map(|s| s.query.as_str()), Some("j"));
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.mode, Mode::Browse);
    assert_eq!(app.focus, Focus::Tree);
    assert_eq!((app.scroll, app.selected, app.cursor), (7, 3, (7, 2)));
    assert_eq!(app.open.source.name, "original");
    start(&mut app);
    finish_scan(&mut app);
    assert_eq!(app.file_search.as_ref().map(|s| s.files.len()), Some(2));
    key(&mut app, KeyCode::Esc);
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn duplicate_names_open_the_selected_path_reveal_ancestors_and_keep_annotations() {
    let (root, mut app) = fixture();
    start(&mut app);
    finish_scan(&mut app);
    // Shown with the platform's separator.
    let nested = std::path::Path::new("docs").join("deep").join("plan.md");
    assert!(screen(&mut app, 100, 24).contains(nested.to_string_lossy().as_ref()));
    key(&mut app, KeyCode::Enter);
    assert!(
        matches!(&app.open.source.provenance, Provenance::File { path } if path == &root.join("docs/deep/plan.md"))
    );
    assert_eq!(
        app.tree.as_ref().and_then(|t| t.position(&root.join("docs/deep/plan.md"))),
        Some(app.tree_cursor)
    );
    app.add_block_annotation(0, Kind::Comment, "keep me".into()).expect("annotate");
    start(&mut app);
    finish_scan(&mut app);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    assert!(
        matches!(&app.open.source.provenance, Provenance::File { path } if path == &root.join("plan.md"))
    );
    start(&mut app);
    finish_scan(&mut app);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.open.store.len(), 1);
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn a_deleted_result_reports_its_path_without_leaving_the_original_document() {
    let (root, mut app) = fixture();
    start(&mut app);
    finish_scan(&mut app);
    std::fs::remove_file(root.join("docs/deep/plan.md")).expect("remove");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.mode, Mode::FileSearch);
    assert_eq!(app.open.source.name, "original");
    assert!(app.file_search.as_ref().and_then(|s| s.error.as_ref()).is_some_and(|s| s.contains("plan.md")));
    key(&mut app, KeyCode::Esc);
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn the_selected_result_stays_visible_when_scrolling() {
    let (root, mut app) = fixture();
    for index in 0..40 {
        std::fs::write(root.join(format!("note-{index:02}.md")), "# Note").expect("write");
    }
    start(&mut app);
    finish_scan(&mut app);
    for _ in 0..25 {
        key(&mut app, KeyCode::Down);
    }
    let selected = app.file_search.as_ref().and_then(super::FileSearch::selected_path).expect("selected");
    let relative = selected.strip_prefix(&root).expect("relative").to_string_lossy();
    assert!(screen(&mut app, 80, 10).contains(relative.as_ref()));
    key(&mut app, KeyCode::Esc);
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn a_failed_scan_is_reported_as_unreadable_not_as_no_files() {
    let (root, mut app) = fixture();
    std::fs::remove_dir_all(&root).expect("remove root");
    start(&mut app);
    finish_scan(&mut app);
    assert_eq!(app.file_search.as_ref().map(|s| s.errors), Some(1));
    assert!(screen(&mut app, 100, 24).contains("1 unreadable"));
    key(&mut app, KeyCode::Esc);
}

#[test]
fn paste_unicode_empty_results_and_tiny_resizes_are_safe() {
    let (root, mut app) = fixture();
    start(&mut app);
    finish_scan(&mut app);
    app.handle_event(&Event::Paste("แผน\nงาน".into())).expect("paste");
    assert_eq!(app.file_search.as_ref().map(|s| s.query.as_str()), Some("แผนงาน"));
    assert!(screen(&mut app, 100, 24).contains("no matching files"));
    for size in [(1, 1), (10, 4), (80, 20)] {
        screen(&mut app, size.0, size.1);
    }
    key(&mut app, KeyCode::Backspace);
    assert_eq!(app.file_search.as_ref().map(|s| s.query.as_str()), Some("แผนงา"));
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.open.source.name, "original");
    key(&mut app, KeyCode::Esc);
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn the_cursor_stays_on_the_best_match_while_results_stream_in_until_the_user_moves_it() {
    let (root, mut app) = fixture();
    start(&mut app);
    finish_scan(&mut app);
    let search = app.file_search.as_mut().expect("search");
    // Replay the scan as two batches: the root's file first, then the nested one, which
    // sorts above it.
    let mut files = std::mem::take(&mut search.files);
    files.sort_by(|a, b| b.relative.cmp(&a.relative));
    let nested = files.pop().expect("nested");
    search.files = files;
    search.rematch();
    search.files.push(nested);
    search.rematch();
    assert_eq!(search.selected_path(), Some(root.join("docs/deep/plan.md")));
    // Once the user has picked a row, a later batch must not move it.
    search.move_cursor(1);
    search.rematch();
    assert_eq!(search.selected_path(), Some(root.join("plan.md")));
    key(&mut app, KeyCode::Esc);
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn clicking_a_result_opens_it_and_ctrl_c_closes_without_quitting() {
    let (root, mut app) = fixture();
    start(&mut app);
    finish_scan(&mut app);
    screen(&mut app, 100, 24);
    let (rect, _) = *app.geometry.search_rows.get(1).expect("second row");
    let click = ratatui::crossterm::event::MouseEvent {
        kind: ratatui::crossterm::event::MouseEventKind::Down(ratatui::crossterm::event::MouseButton::Left),
        column: rect.x + 1,
        row: rect.y,
        modifiers: KeyModifiers::NONE,
    };
    app.handle_event(&Event::Mouse(click)).expect("click");
    assert_eq!(app.mode, Mode::Browse);
    assert!(
        matches!(&app.open.source.provenance, Provenance::File { path } if path == &root.join("plan.md"))
    );
    start(&mut app);
    app.handle_event(&Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))).expect("ctrl+c");
    assert_eq!(app.mode, Mode::Browse);
    assert!(!app.quit);
    assert!(app.file_search.is_none());
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn ctrl_p_does_nothing_outside_a_folder_review() {
    let (root, mut app) = fixture();
    app.tree = None;
    app.handle_event(&Event::Key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL))).expect("ctrl+p");
    assert_eq!(app.mode, Mode::Browse);
    assert!(app.file_search.is_none());
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn a_long_path_keeps_its_file_name_visible() {
    assert_eq!(super::tail("docs/deep/plan.md", 40), "docs/deep/plan.md");
    assert_eq!(super::tail("docs/deep/plan.md", 10), "…p/plan.md");
    // Wide characters count two columns.
    assert_eq!(super::tail("文档/计划.md", 8), "…计划.md");
}
