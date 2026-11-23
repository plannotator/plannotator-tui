//! The file list on the left of the diff: folder chains on one row, whole names when they
//! fit and middle cuts when they don't, and the filters by name, kind and viewed mark, which
//! the diff, `n`/`N` and `ctrl+n` follow while notes in hidden files are counted and sent.

use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

use super::viewed_flow::entry;
use super::*;
use crate::review::{Focus, Mode};

const PANEL: &str = "packages/review-editor/components/ReviewAgentJobDetailPanel.tsx";
const PANEL_TEST: &str = "packages/review-editor/components/ReviewAgentJobDetailPanel.test.tsx";
const GUIDE: &str = "docs/guides/keyboard-shortcuts.md";

/// The fixture's app.py change, plus a component, its test and a guide, all new.
fn monorepo(tag: &str) -> Fixture {
    let fixture = Fixture::new(tag);
    for path in [PANEL, PANEL_TEST, GUIDE] {
        let full = fixture.repo().join(path);
        std::fs::create_dir_all(full.parent().expect("a folder")).expect("folders");
        std::fs::write(full, format!("// {path}\nexport const one = 1;\n")).expect("write");
    }
    fixture
}

fn ctrl(screen: &mut Screen, c: char) {
    screen.handle_event(Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))).expect("key");
}

/// Wait for the diff to take what the filter shows.
fn shown(screen: &mut Screen) -> Vec<String> {
    settle(screen);
    screen.diff.paths().to_vec()
}

#[test]
fn folder_chains_are_one_row_and_names_show_whole_with_their_counts() {
    let fixture = monorepo("files-look");
    let mut screen = fixture.open();
    let rows = frame(&mut screen, 160, 30);
    assert!(row_with(&rows, "Files \u{b7} 4").starts_with(" Files \u{b7} 4 "), "{rows:#?}");
    assert!(entry(&rows, "packages/").contains("\u{25be} packages/review-editor/components/"), "{rows:#?}");
    let panel = entry(&rows, "ReviewAgentJobDetailPanel.tsx");
    assert!(panel.starts_with("   ReviewAgentJobDetailPanel.tsx "), "one level in, no gutter: {panel:?}");
    assert!(panel.trim_end().ends_with("A +2 \u{2212}0"), "{panel:?}");
    // The list fits its rows: no wider than the longest needs, its divider right after.
    let width = usize::from(screen.files.drawn_width);
    assert!(width < 60, "{width}");
    // The diff shows the files in the list's order: folders first, then files.
    assert_eq!(screen.diff.paths(), [GUIDE, PANEL_TEST, PANEL, "app.py"]);
}

#[test]
fn a_name_too_long_for_the_list_is_cut_in_the_middle_and_keeps_its_extension() {
    let fixture = monorepo("files-cut");
    let mut screen = fixture.open();
    // 80 columns and no notes: the list gets a little under two fifths.
    let rows = frame(&mut screen, 80, 30);
    let test = entry(&rows, ".test.tsx");
    assert!(test.contains("Rev") && test.contains(".test.tsx"), "{rows:#?}");
    assert!(usize::from(screen.files.drawn_width) <= 32);
    // `>` widens the list until the name fits whole; `<` narrows it again.
    keys(&mut screen, ">>>>>>");
    let rows = frame(&mut screen, 120, 30);
    assert!(
        entry(&rows, "ReviewAgentJobDetailPanel.test.tsx").contains("ReviewAgentJobDetailPanel.test.tsx")
    );
    keys(&mut screen, "<<<<<<<<<<<<");
    let _ = frame(&mut screen, 120, 30);
    assert_eq!(screen.files.drawn_width, 20);
}

#[test]
fn typing_a_filter_narrows_the_list_and_enter_narrows_the_diff_and_esc_clears_it() {
    let fixture = monorepo("files-filter");
    let mut screen = fixture.open();
    keys(&mut screen, "/panel");
    assert_eq!(screen.mode, Mode::FilterFiles);
    let rows = frame(&mut screen, 160, 30);
    assert!(row_with(&rows, "Files \u{b7} 2 of 4").contains("\u{2502}"), "{rows:#?}");
    assert!(row_with(&rows, "/panel").contains("/panel"));
    assert!(!rows.iter().any(|r| entry_has(r, "app.py")), "the list follows each key: {rows:#?}");
    // The diff waits for Enter.
    assert_eq!(screen.diff.paths().len(), 4);
    press(&mut screen, KeyCode::Enter);
    assert_eq!(shown(&mut screen), [PANEL_TEST, PANEL]);
    let rows = frame(&mut screen, 160, 30);
    assert!(rows[0].contains(" 2 of 4 files +4 \u{2212}0 "), "{:?}", rows[0]);
    // n and N move among the files shown.
    assert_eq!(screen.files.current.as_deref(), Some(PANEL_TEST));
    keys(&mut screen, "n");
    assert_eq!(screen.files.current.as_deref(), Some(PANEL));
    keys(&mut screen, "n");
    assert_eq!(screen.status.as_deref(), Some("last file"));
    // Esc in the list clears the filter; every file comes back.
    keys(&mut screen, "f");
    press(&mut screen, KeyCode::Esc);
    assert_eq!(shown(&mut screen).len(), 4);
    assert_eq!(screen.files.current.as_deref(), Some(PANEL), "the diff stays on the file");
    // Esc while typing clears too.
    keys(&mut screen, "/zzz");
    let rows = frame(&mut screen, 160, 30);
    assert!(row_with(&rows, "No file matches").contains('\u{2502}'));
    press(&mut screen, KeyCode::Esc);
    assert_eq!(screen.mode, Mode::Browse);
    assert_eq!(screen.files.filter.query, "");
}

/// Whether the list's part of `row` names `name`.
fn entry_has(row: &str, name: &str) -> bool {
    row.split('\u{2502}').next().is_some_and(|left| left.contains(name))
}

#[test]
fn the_kind_picker_counts_each_kind_and_enter_shows_one_alone() {
    let fixture = monorepo("files-kinds");
    let mut screen = fixture.open();
    keys(&mut screen, "t");
    let rows = frame(&mut screen, 160, 30);
    for (kind, count) in [("Code", 2), ("Tests", 1), ("Docs", 1), ("Generated", 0)] {
        let row = row_with(&rows, &format!(" {kind} "));
        assert!(row.contains(&format!(" {count} ")), "{kind}: {row:?}");
    }
    // Down to Tests, Enter: tests alone.
    keys(&mut screen, "jj");
    press(&mut screen, KeyCode::Enter);
    assert_eq!(shown(&mut screen), [PANEL_TEST]);
    let rows = frame(&mut screen, 160, 30);
    assert!(row_with(&rows, "Files \u{b7} 1 of 4").contains('\u{2502}'));
    assert!(entry_has(row_with(&rows, " Tests"), "Tests"), "the filter line names the kind: {rows:#?}");
    // Space turns Docs on as well; the picker closing hands it to the diff.
    keys(&mut screen, "tjjj ");
    press(&mut screen, KeyCode::Esc);
    assert_eq!(shown(&mut screen), [GUIDE, PANEL_TEST]);
    // All kinds again.
    keys(&mut screen, "t");
    press(&mut screen, KeyCode::Enter);
    assert_eq!(shown(&mut screen).len(), 4);
}

#[test]
fn notes_on_hidden_files_are_counted_on_screen_and_still_sent() {
    let fixture = monorepo("files-notes");
    let mut screen = fixture.open();
    at(&mut screen, 9);
    keys(&mut screen, "c");
    type_and_save(&mut screen, "name the total");
    keys(&mut screen, "/panel");
    press(&mut screen, KeyCode::Enter);
    assert_eq!(shown(&mut screen).len(), 2);
    let rows = frame(&mut screen, 160, 30);
    assert!(entry_has(row_with(&rows, "1 unsent note in hidden files"), "hidden files"), "{rows:#?}");
    // The rail keeps the note; going to it says why the diff cannot show it.
    assert!(row_with(&rows, "name the total").contains('\u{2502}'));
    // The list hidden for want of room: the footer says it instead.
    let rows = frame(&mut screen, 80, 30);
    assert_eq!(screen.files.drawn_width, 0, "{rows:#?}");
    assert!(rows.iter().any(|r| r.contains("filter: 2 of 4 files")), "{rows:#?}");
    press(&mut screen, KeyCode::Tab);
    assert_eq!(screen.status.as_deref(), Some("app.py is hidden by the file filter"));
    // Send sends every note, hidden or not.
    keys(&mut screen, "E");
    assert!(
        fixture.sent().first().is_some_and(|sent| sent.contains("name the total")),
        "{:?}",
        fixture.sent()
    );
}

#[test]
fn hiding_viewed_files_drops_a_file_once_marked_and_goes_on_to_the_next() {
    let fixture = monorepo("files-viewed");
    let mut screen = fixture.open();
    keys(&mut screen, "H");
    assert_eq!(screen.status.as_deref(), Some("hiding 0 viewed files"));
    // The first file, marked: it leaves, and the diff moves on.
    assert_eq!(screen.files.current.as_deref(), Some(GUIDE));
    keys(&mut screen, "x");
    assert_eq!(shown(&mut screen), [PANEL_TEST, PANEL, "app.py"]);
    assert_eq!(screen.files.current.as_deref(), Some(PANEL_TEST));
    let rows = frame(&mut screen, 160, 30);
    assert!(row_with(&rows, "Files \u{b7} 3 of 4").contains('\u{2502}'), "{rows:#?}");
    assert!(entry_has(row_with(&rows, "not viewed"), "not viewed"));
    // ctrl+n stays among the files shown.
    ctrl(&mut screen, 'n');
    assert_eq!(screen.files.current.as_deref(), Some(PANEL));
    // H again: every file back.
    keys(&mut screen, "H");
    assert_eq!(shown(&mut screen).len(), 4);
}

#[test]
fn the_list_moves_the_diff_as_its_cursor_moves_and_enter_goes_there() {
    let fixture = monorepo("files-keys");
    let mut screen = fixture.open();
    keys(&mut screen, "f");
    assert_eq!(screen.focus, Focus::Files);
    // Rows: docs/guides/, its file, packages/…/components/, two files, app.py.
    // The cursor starts on the file the diff shows, the guide.
    keys(&mut screen, "jj");
    assert_eq!(screen.files.current.as_deref(), Some(PANEL_TEST));
    assert_eq!(screen.file_under_cursor().as_deref(), Some(PANEL_TEST));
    keys(&mut screen, "j");
    press(&mut screen, KeyCode::Enter);
    assert_eq!(screen.focus, Focus::Diff);
    assert_eq!(screen.files.current.as_deref(), Some(PANEL));
    // h on a file goes to its folder; h there closes it.
    keys(&mut screen, "fhh");
    let rows = frame(&mut screen, 160, 30);
    assert!(entry(&rows, "components/").contains("\u{25b8} "), "closed: {rows:#?}");
    assert!(!rows.iter().any(|r| entry_has(r, "DetailPanel.tsx")));
    // f from the list closes it; f again opens it and moves there.
    keys(&mut screen, "f");
    assert_eq!(screen.focus, Focus::Diff);
    let _ = frame(&mut screen, 160, 30);
    assert_eq!(screen.files.drawn_width, 0);
    keys(&mut screen, "f");
    assert_eq!(screen.focus, Focus::Files);
}

#[test]
fn at_eighty_columns_the_list_hides_beside_the_notes_and_the_rail_makes_way_for_it() {
    let fixture = monorepo("files-narrow");
    let mut screen = fixture.open();
    at(&mut screen, 9);
    keys(&mut screen, "c");
    type_and_save(&mut screen, "name the total");
    let rows = frame(&mut screen, 80, 30);
    assert_eq!(screen.files.drawn_width, 0, "{rows:#?}");
    assert!(rows.iter().any(|r| r.contains("files hidden: f shows them")), "{rows:#?}");
    assert!(row_with(&rows, "name the total").contains('\u{2502}'), "the rail stays");
    keys(&mut screen, "f");
    let rows = frame(&mut screen, 80, 30);
    assert!(screen.files.drawn_width >= 20, "{rows:#?}");
    assert!(!rows.iter().any(|r| r.contains("name the total")), "the rail made way: {rows:#?}");
    // Back to the diff: the rail returns and the list hides again.
    press(&mut screen, KeyCode::Esc);
    let rows = frame(&mut screen, 80, 30);
    assert!(rows.iter().any(|r| r.contains("name the total")), "{rows:#?}");
}

#[test]
fn a_folder_whose_files_are_all_viewed_is_ticked_open_or_closed() {
    let fixture = monorepo("files-folder-tick");
    let mut screen = fixture.open();
    keys(&mut screen, "nx");
    let rows = frame(&mut screen, 160, 30);
    assert!(!entry(&rows, "components/").trim_end().ends_with('\u{2713}'), "one of two: {rows:#?}");
    keys(&mut screen, "nx");
    let rows = frame(&mut screen, 160, 30);
    assert!(entry(&rows, "components/").trim_end().ends_with('\u{2713}'), "{rows:#?}");
    // Closed, it keeps the tick: h goes up to the folder, h again closes it.
    keys(&mut screen, "fhh");
    let rows = frame(&mut screen, 160, 30);
    let folder = entry(&rows, "components/");
    assert!(folder.contains('\u{25b8}') && folder.trim_end().ends_with('\u{2713}'), "{rows:#?}");
}

#[test]
fn every_file_row_shows_the_same_columns_so_they_line_up() {
    let fixture = monorepo("files-columns");
    let mut screen = fixture.open();
    for cols in [80, 100, 160] {
        let rows = frame(&mut screen, cols, 30);
        let width = usize::from(screen.files.drawn_width);
        // A file row's text ends at the same column: the counts' right edge, or the status
        // letter's when the counts give way.
        let ends: Vec<usize> = rows
            .iter()
            .skip(2)
            .filter(|row| row.chars().nth(width) == Some('\u{2502}'))
            .map(|row| row.chars().take(width).collect::<String>())
            .filter(|left| {
                !left.trim().is_empty() && !left.contains('\u{25be}') && !left.contains('\u{25b8}')
            })
            .map(|left| left.trim_end().chars().count())
            .collect();
        assert!(ends.len() >= 4, "{cols}: {rows:#?}");
        assert!(ends.windows(2).all(|w| w.first() == w.last()), "{cols} columns: {ends:?} {rows:#?}");
    }
}

#[test]
fn the_footer_hints_take_one_row_on_a_wide_screen_and_two_on_a_narrow_one() {
    let fixture = monorepo("files-footer");
    let mut screen = fixture.open();
    for (cols, most) in [(200, 1), (160, 1), (100, 2), (80, 2)] {
        let rows = frame(&mut screen, cols, 30);
        let footer: Vec<&String> = rows.iter().rev().take_while(|r| !r.contains('\u{2502}')).collect();
        assert!(footer.len() <= most, "{cols} columns: {footer:#?}");
        let last = footer.first().expect("a footer");
        assert!(last.contains("? help") && last.contains("q quit"), "{cols} columns: {last:?}");
        // No row holds a hint alone.
        assert!(footer.iter().all(|r| r.contains(" \u{b7} ")), "{cols} columns: {footer:#?}");
    }
}
