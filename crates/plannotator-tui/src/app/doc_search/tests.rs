//! The search prompt, highlights and `n` / `N`, drawn into a `TestBackend` as the app draws.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use plannotator_tui_schema::{DocumentSource, Kind, Provenance};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent};

use crate::app::{App, Mode};
use crate::delivery::Discard;
use crate::theme::palette;

const PLAN: &str =
    "# Plan\n\nThe plan has three parts.\n\n## Later\n\nKeep the plan small, and say why the PLAN changed.\n";

fn scratch(label: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("plannotator-tui-doc-search-{label}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn app_with(source: &str, width: usize) -> App {
    let source = DocumentSource::new(source.to_owned(), "plan.md", true, Provenance::Stdin);
    let mut app = App::open(source, width, Box::new(Discard)).expect("app opens");
    app.data_dir = scratch("data");
    draw(&mut app, 80, 20);
    app
}

fn draw(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal.draw(|frame| app.draw(frame)).expect("draw");
    terminal.backend().buffer().clone()
}

fn line(buffer: &Buffer, y: u16) -> String {
    (0..buffer.area.width).filter_map(|x| buffer.cell((x, y))).map(ratatui::buffer::Cell::symbol).collect()
}

/// The footer's rows: its hints can flow up to three.
fn footer(app: &mut App) -> String {
    let buffer = draw(app, 80, 20);
    (17..20).map(|y| line(&buffer, y)).collect::<Vec<_>>().join("\n")
}

/// The text painted in `style`'s background, row by row, joined by `|`.
fn painted(buffer: &Buffer, style: ratatui::style::Style) -> String {
    let rows: Vec<String> = (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .filter_map(|x| buffer.cell((x, y)))
                .filter(|cell| cell.bg == style.bg.expect("a background"))
                .map(ratatui::buffer::Cell::symbol)
                .collect::<String>()
        })
        .filter(|row| !row.is_empty())
        .collect();
    rows.join("|")
}

fn press(app: &mut App, code: KeyCode) {
    app.handle_event(&Event::Key(KeyEvent::from(code))).expect("key");
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        press(app, KeyCode::Char(c));
    }
}

fn search(app: &mut App, query: &str) {
    press(app, KeyCode::Char('/'));
    assert_eq!(app.mode, Mode::Search);
    typed(app, query);
    press(app, KeyCode::Enter);
    assert_eq!(app.mode, Mode::Browse);
}

#[test]
fn the_prompt_opens_in_the_footer_and_typing_highlights_every_match_with_a_count() {
    let mut app = app_with(PLAN, 60);
    press(&mut app, KeyCode::Char('/'));
    typed(&mut app, "plan");
    let buffer = draw(&mut app, 80, 20);
    let footer = line(&buffer, 19);
    assert!(footer.starts_with(" /plan "), "{footer:?}");
    assert!(footer.trim_end().ends_with("1/4 · enter keep · esc cancel"), "{footer:?}");
    // Lower case ignores case: the heading and the shouted PLAN count too.
    assert_eq!(painted(&buffer, palette().search_current), "Plan");
    assert_eq!(painted(&buffer, palette().search), "plan|planPLAN");
}

#[test]
fn a_capital_in_the_query_makes_it_match_case() {
    let mut app = app_with(PLAN, 60);
    search(&mut app, "PLAN");
    assert!(footer(&mut app).contains("/PLAN 1/1"));
    assert_eq!(app.cursor_text(), "PLAN");
}

#[test]
fn enter_keeps_the_search_and_n_and_shift_n_step_through_it_wrapping_at_either_end() {
    let mut app = app_with(PLAN, 60);
    search(&mut app, "plan");
    assert!(footer(&mut app).contains("/plan 1/4"), "the count stays in the footer");
    press(&mut app, KeyCode::Char('n'));
    assert!(footer(&mut app).contains("/plan 2/4"));
    let buffer = draw(&mut app, 80, 20);
    assert_eq!(painted(&buffer, palette().search_current), "plan");
    assert_eq!(app.cursor_text(), "plan");
    press(&mut app, KeyCode::Char('n'));
    press(&mut app, KeyCode::Char('n'));
    assert!(footer(&mut app).contains("/plan 4/4"));
    press(&mut app, KeyCode::Char('n'));
    let wrapped = footer(&mut app);
    assert!(wrapped.contains("search wrapped to the top · /plan 1/4"), "{wrapped:?}");
    press(&mut app, KeyCode::Char('N'));
    let wrapped = footer(&mut app);
    assert!(wrapped.contains("search wrapped to the bottom · /plan 4/4"), "{wrapped:?}");
    press(&mut app, KeyCode::Char('N'));
    assert!(footer(&mut app).contains("/plan 3/4"));
    press(&mut app, KeyCode::Char('g'));
    press(&mut app, KeyCode::Char('N'));
    assert!(footer(&mut app).contains("search wrapped to the bottom"));
    press(&mut app, KeyCode::Esc);
    assert!(!footer(&mut app).contains("wrapped"), "clearing the search clears its word");
}

#[test]
fn typing_lands_on_the_first_match_at_or_after_the_cursor() {
    let mut app = app_with(PLAN, 60);
    press(&mut app, KeyCode::Char('j'));
    press(&mut app, KeyCode::Char('j'));
    assert_eq!(app.selected, 2, "on the second heading");
    search(&mut app, "plan");
    assert!(footer(&mut app).contains("/plan 3/4"));
    assert_eq!(app.selected, 3);
}

#[test]
fn esc_in_the_prompt_puts_back_the_view_the_cursor_and_the_search_before_it() {
    let mut source: String = (0..60).map(|i| format!("Paragraph {i}.")).collect::<Vec<_>>().join("\n\n");
    source.push_str("\n\nThe needle.\n");
    let mut app = app_with(&source, 60);
    search(&mut app, "paragraph 1");
    let before = (app.scroll, app.cursor, app.selected);
    press(&mut app, KeyCode::Char('/'));
    typed(&mut app, "needle");
    draw(&mut app, 80, 20);
    assert_ne!((app.scroll, app.cursor, app.selected), before, "typing moved to the match");
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.mode, Mode::Browse);
    assert_eq!((app.scroll, app.cursor, app.selected), before);
    assert!(footer(&mut app).contains("/paragraph 1 1/11"), "the earlier search is back");
}

#[test]
fn a_second_esc_clears_the_highlights_and_only_a_third_quits() {
    let mut app = app_with(PLAN, 60);
    search(&mut app, "plan");
    press(&mut app, KeyCode::Esc);
    let buffer = draw(&mut app, 80, 20);
    assert_eq!(painted(&buffer, palette().search), "");
    assert!(!line(&buffer, 19).contains("/plan"));
    assert!(!app.quit);
    press(&mut app, KeyCode::Esc);
    assert!(app.quit);
}

#[test]
fn an_empty_search_clears_the_highlights() {
    let mut app = app_with(PLAN, 60);
    search(&mut app, "plan");
    search(&mut app, "");
    let buffer = draw(&mut app, 80, 20);
    assert_eq!(painted(&buffer, palette().search), "");
    assert_eq!(painted(&buffer, palette().search_current), "");
}

#[test]
fn a_query_with_no_match_says_so_and_leaves_the_view_where_it_was() {
    let mut app = app_with(PLAN, 60);
    press(&mut app, KeyCode::Char('j'));
    let before = (app.scroll, app.cursor, app.selected);
    press(&mut app, KeyCode::Char('/'));
    typed(&mut app, "absent");
    assert!(footer(&mut app).contains("no matches · enter keep · esc cancel"));
    assert_eq!((app.scroll, app.cursor, app.selected), before);
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Char('n'));
    assert!(footer(&mut app).contains("no matches for “absent” · /absent no matches"));
}

#[test]
fn a_match_across_a_wrap_is_highlighted_on_both_rows_and_the_cursor_lands_on_it() {
    let source = "The rollout plan covers the staging cluster first and then production after a review.\n";
    let mut app = app_with(source, 30);
    let buffer = draw(&mut app, 40, 20);
    assert!(line(&buffer, 1).trim_end().ends_with("staging"), "the phrase wraps at this width");
    assert!(line(&buffer, 2).contains("cluster first"));
    press(&mut app, KeyCode::Char('/'));
    typed(&mut app, "staging cluster first");
    let buffer = draw(&mut app, 40, 20);
    assert_eq!(painted(&buffer, palette().search_current), "staging|cluster first");
    press(&mut app, KeyCode::Enter);
    // The cursor is on the match, so `v` selects from its first character.
    typed(&mut app, "ve");
    press(&mut app, KeyCode::Enter);
    let pending = app.pending.as_ref().expect("a selection");
    assert_eq!(app.open.doc.source.get(pending.range.clone()), Some("staging"));
}

#[test]
fn searching_keeps_a_waiting_selection_and_every_annotation() {
    let mut app = app_with(PLAN, 60);
    app.add_quote_annotation("three parts", Kind::Comment, "why three?".to_owned()).expect("note");
    // The first note opens the rail; let that reflow happen before selecting.
    draw(&mut app, 80, 20);
    app.select_quote_for_snapshot("Keep the").expect("selection");
    let pending = app.pending.as_ref().map(|p| p.range.clone());
    search(&mut app, "plan");
    press(&mut app, KeyCode::Char('n'));
    press(&mut app, KeyCode::Char('N'));
    assert_eq!(app.pending.as_ref().map(|p| p.range.clone()), pending, "the toolbar's selection stays");
    assert_eq!(app.open.store.len(), 1);
    let buffer = draw(&mut app, 80, 20);
    assert_eq!(painted(&buffer, palette().search_current), "Plan");
    let tint = ratatui::style::Style::new().bg(palette().comment_bg);
    assert_eq!(painted(&buffer, tint), "three parts", "the note keeps its own tint");
    // Esc clears the selection first; the search waits for the next one.
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.pending.as_ref().map(|p| p.range.clone()), None);
    let shown = footer(&mut app);
    assert!(shown.contains("/plan"), "{shown}");
}

#[test]
fn the_search_keys_are_rebindable_and_the_footer_names_them() {
    let mut app = app_with(PLAN, 60);
    app.keys =
        toml::from_str("search = [\"f\"]\nnext-match = [\"]\"]\nprevious-match = [\"[\"]\n").expect("keys");
    press(&mut app, KeyCode::Char('/'));
    assert_eq!(app.mode, Mode::Browse, "/ is no longer search");
    assert!(footer(&mut app).contains("f search"));
    press(&mut app, KeyCode::Char('f'));
    assert_eq!(app.mode, Mode::Search);
    typed(&mut app, "plan");
    press(&mut app, KeyCode::Enter);
    let shown = footer(&mut app);
    assert!(shown.contains("]/[ match"), "{shown:?}");
    press(&mut app, KeyCode::Char(']'));
    assert!(footer(&mut app).contains("/plan 2/4"));
    press(&mut app, KeyCode::Char('n'));
    assert!(footer(&mut app).contains("/plan 2/4"), "n is free again");
}

#[test]
fn n_before_any_search_says_how_to_start_one() {
    let mut app = app_with(PLAN, 60);
    press(&mut app, KeyCode::Char('n'));
    assert!(footer(&mut app).contains("no search: / starts one"));
}

#[test]
fn a_folder_review_searches_the_open_file_and_opening_another_drops_the_search() {
    let root = scratch("folder");
    std::fs::write(root.join("a.md"), "# A\n\nalpha shared\n").expect("a");
    std::fs::write(root.join("b.md"), "# B\n\nbeta shared\n").expect("b");
    let mut app = App::open_folder(&root, 60, Box::new(Discard)).expect("folder");
    app.data_dir = root.join("data");
    draw(&mut app, 140, 20);
    search(&mut app, "shared");
    assert!(footer(&mut app).contains("/shared 1/1"), "only the open file is searched");
    app.open_file_path(&root.join("b.md")).expect("open b");
    assert!(!footer(&mut app).contains("/shared"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn highlights_follow_the_reading_column_and_the_rail_as_they_reflow() {
    let source = "The rollout plan covers the staging cluster first and then production after a review \
of every metric the team agreed on beforehand.\n";
    let mut app = app_with(source, 60);
    app.add_quote_annotation("rollout", Kind::Comment, "when?".to_owned()).expect("note");
    search(&mut app, "staging cluster first and then production");
    let whole = "stagingclusterfirstandthenproduction";
    let current = |app: &mut App| painted(&draw(app, 100, 20), palette().search_current);
    let before = current(&mut app);
    assert_eq!(before.replace(['|', ' '], ""), whole);
    for key in ['<', '<', '<', '>', 'z', 'z'] {
        press(&mut app, KeyCode::Char(key));
        let now = current(&mut app);
        assert_eq!(now.replace(['|', ' '], ""), whole, "after {key}: {now:?}");
    }
    press(&mut app, KeyCode::Char('<'));
    press(&mut app, KeyCode::Char('<'));
    assert!(current(&mut app).contains('|'), "a narrower column splits the match across rows");
    press(&mut app, KeyCode::Char('n'));
    assert!(footer(&mut app).contains("/staging cluster first and then production 1/1"));
}

#[test]
fn a_match_in_coloured_code_keeps_its_syntax_colour_under_the_search_background() {
    // The second `let` sits in a code block nested in a list item.
    let source = "Intro.\n\n```rust\nlet total = 42;\n```\n\n- a step\n\n  ```rust\n  let more = 1;\n  ```\n";
    let mut app = app_with(source, 60);
    search(&mut app, "let");
    let buffer = draw(&mut app, 80, 20);
    assert_eq!(painted(&buffer, palette().search_current), "let");
    assert_eq!(painted(&buffer, palette().search), "let", "the nested block's match");
    // The match the cursor is not on keeps its keyword colour; only its background changes.
    let other: Vec<_> = (0..buffer.area.height)
        .flat_map(|y| (0..buffer.area.width).map(move |x| (x, y)))
        .filter_map(|xy| buffer.cell(xy))
        .filter(|cell| cell.bg == palette().search.bg.expect("bg"))
        .map(|cell| cell.fg)
        .collect();
    assert_eq!(other, [palette().code_keyword; 3]);
}

impl App {
    /// The source text of the match the cursor is on.
    fn cursor_text(&self) -> String {
        let search = self.open.search.as_ref().expect("a search");
        let range = search.current.and_then(|i| search.matches.get(i)).expect("a current match");
        self.open.doc.source[range.clone()].to_owned()
    }
}
