//! Which file the diff is in, whatever moved its cursor there: `n`/`N`, the list's
//! highlight and the footer start from it, a file with no lines at the end does not throw
//! the cursor back, and a filter cancelled with nothing changed leaves everything where it
//! was.

use ratatui::crossterm::event::KeyCode;

use super::*;
use crate::review::{Focus, Mode};

/// app.py's change, then two binary files at the root, which have no lines.
fn binaries(tag: &str) -> Fixture {
    let fixture = Fixture::new(tag);
    std::fs::write(fixture.repo().join("yy.bin"), [0u8, 1, 2, 0, 255]).expect("binary");
    std::fs::write(fixture.repo().join("zz.bin"), [0u8, 9, 2, 0, 255]).expect("binary");
    fixture
}

/// app.py's change and two new files before it in the list.
fn three(tag: &str) -> Fixture {
    let fixture = Fixture::new(tag);
    std::fs::create_dir_all(fixture.repo().join("docs")).expect("folder");
    fixture.write("docs/guide.md", "# Guide\n\nRead me.\n");
    fixture.write("docs/intro.md", "# Intro\n\nHello.\n");
    fixture
}

fn current(screen: &Screen) -> Option<&str> {
    screen.files.current.as_deref()
}

#[test]
fn n_and_shift_n_start_from_the_file_a_note_in_the_rail_moved_the_diff_to() {
    let fixture = three("moves-rail");
    let mut screen = fixture.open();
    assert_eq!(screen.diff.paths(), ["docs/guide.md", "docs/intro.md", "app.py"]);
    at(&mut screen, 9);
    keys(&mut screen, "c");
    type_and_save(&mut screen, "name the total");
    keys(&mut screen, "g");
    assert_eq!(current(&screen), Some("docs/guide.md"));
    // The rail moves the diff to the note; the list and the footer follow it.
    press(&mut screen, KeyCode::Tab);
    press(&mut screen, KeyCode::Tab);
    assert_eq!(screen.focus, Focus::Diff);
    assert_eq!(current(&screen), Some("app.py"));
    keys(&mut screen, "N");
    assert_ne!(screen.status.as_deref(), Some("first file"));
    assert_eq!(current(&screen), Some("docs/intro.md"));
    keys(&mut screen, "N");
    assert_eq!(current(&screen), Some("docs/guide.md"));
    keys(&mut screen, "N");
    assert_eq!(screen.status.as_deref(), Some("first file"));
    assert_eq!(current(&screen), Some("docs/guide.md"));
}

#[test]
fn j_on_a_file_with_no_lines_at_the_end_goes_down_and_never_back_up() {
    let fixture = binaries("moves-tail");
    let mut screen = fixture.open();
    assert_eq!(screen.diff.paths(), ["app.py", "yy.bin", "zz.bin"]);
    keys(&mut screen, "nn");
    assert_eq!(current(&screen), Some("zz.bin"));
    for _ in 0..4 {
        keys(&mut screen, "j");
        assert_eq!(current(&screen), Some("zz.bin"));
        assert_eq!(screen.diff.cursor().expect("cursor"), None, "still on zz.bin's rows");
    }
    keys(&mut screen, "n");
    assert_eq!(screen.status.as_deref(), Some("last file"));
    // Back up through the rows: yy.bin's note and heading, then app.py's last line.
    keys(&mut screen, "k");
    assert_eq!(current(&screen), Some("zz.bin"), "its heading");
    keys(&mut screen, "k");
    assert_eq!(current(&screen), Some("yy.bin"), "yy.bin's note");
    keys(&mut screen, "kk");
    assert_eq!(current(&screen), Some("app.py"));
    assert!(screen.diff.cursor().expect("cursor").is_some_and(|l| l.path == "app.py" && l.first == 15));
    // And down again, one row at a time.
    keys(&mut screen, "j");
    assert_eq!(current(&screen), Some("yy.bin"));
    keys(&mut screen, "jj");
    assert_eq!(current(&screen), Some("zz.bin"));
    // N from zz.bin's heading goes to yy.bin, not past it.
    keys(&mut screen, "N");
    assert_eq!(current(&screen), Some("yy.bin"));
}

#[test]
fn x_on_a_binary_file_heading_marks_that_file() {
    let fixture = binaries("moves-mark");
    let mut screen = fixture.open();
    keys(&mut screen, "n");
    assert_eq!(current(&screen), Some("yy.bin"));
    keys(&mut screen, "x");
    assert!(screen.progress.is_viewed("yy.bin"), "{:?}", screen.status);
    assert!(!screen.progress.is_viewed("zz.bin"));
}

#[test]
fn esc_on_a_filter_that_changed_nothing_keeps_the_selection_the_cursor_and_the_focus() {
    let fixture = three("moves-cancel");
    let mut screen = fixture.open();
    at(&mut screen, 9);
    keys(&mut screen, "vk");
    let selected = screen.diff.selection().expect("a selection");
    keys(&mut screen, "/");
    assert_eq!(screen.mode, Mode::FilterFiles);
    press(&mut screen, KeyCode::Esc);
    assert_eq!(screen.mode, Mode::Browse);
    assert_eq!(screen.focus, Focus::Diff, "back where / was pressed");
    assert_eq!(screen.diff.selection(), Some(selected.clone()));
    // The kind picker closed with nothing changed, and H with nothing viewed: the same.
    keys(&mut screen, "t");
    press(&mut screen, KeyCode::Esc);
    assert_eq!(screen.diff.selection(), Some(selected.clone()));
    keys(&mut screen, "H");
    assert_eq!(screen.diff.selection(), Some(selected));
}

#[test]
fn typing_the_filter_takes_a_paste_and_ctrl_c_still_quits() {
    let fixture = three("moves-paste");
    let mut screen = fixture.open();
    keys(&mut screen, "/");
    screen.handle_event(Event::Paste("intro\n".into())).expect("paste");
    assert_eq!(screen.files.filter.query, "intro");
    press(&mut screen, KeyCode::Enter);
    settle(&mut screen);
    assert_eq!(screen.diff.paths(), ["docs/intro.md"]);
    keys(&mut screen, "/");
    let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    screen.handle_event(Event::Key(ctrl_c)).expect("key");
    assert!(screen.quit || screen.mode == Mode::ConfirmQuit, "{:?}", screen.mode);
}

#[test]
fn walking_every_row_down_and_up_names_each_file_in_turn() {
    let fixture = Fixture::new("moves-walk");
    let lines = (1..=30).fold(String::new(), |text, n| text + "line " + &n.to_string() + "\n");
    fixture.write("nn.txt", &lines);
    fixture.git(&["add", "nn.txt"]);
    fixture.git(&["-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-qm", "nn"]);
    // A change far into nn.txt, so its hunk starts after a gap.
    fixture.write("nn.txt", &lines.replace("line 20\n", "line twenty\n"));
    for name in ["aa.bin", "mm.bin", "zz.bin"] {
        std::fs::write(fixture.repo().join(name), [0u8, name.len() as u8, 0, 255]).expect("binary");
    }
    let mut screen = fixture.open();
    let paths = ["aa.bin", "app.py", "mm.bin", "nn.txt", "zz.bin"];
    assert_eq!(screen.diff.paths(), paths);
    keys(&mut screen, "g");
    for _ in 0..12 {
        keys(&mut screen, "k");
    }
    assert_eq!(current(&screen), Some("aa.bin"));
    let mut seen = vec![current(&screen).map(str::to_owned)];
    // On a screen tall enough for all of it, looking for the file never scrolls the diff.
    let top = |screen: &mut Screen| {
        let rows = frame(screen, 160, 90);
        rows.get(1).map(|r| diff_side(screen, r)).unwrap_or_default()
    };
    let first = top(&mut screen);
    assert!(first.contains("aa.bin"), "{first}");
    for _ in 0..60 {
        keys(&mut screen, "j");
        seen.push(current(&screen).map(str::to_owned));
        assert_eq!(top(&mut screen), first);
    }
    let mut down: Vec<String> = seen.iter().flatten().cloned().collect();
    // Each row's file, top to bottom: the files in order, and two rows for each binary one.
    let count = |name: &str| down.iter().filter(|p| *p == name).count();
    assert_eq!(count("aa.bin"), 2, "{down:?}");
    assert_eq!(count("mm.bin"), 2, "{down:?}");
    assert!(count("app.py") > 10 && count("nn.txt") > 3, "{down:?}");
    down.dedup();
    assert_eq!(down, ["aa.bin", "app.py", "mm.bin", "nn.txt", "zz.bin"]);
    // And back up from the end.
    let mut up = vec![current(&screen).map(str::to_owned)];
    for _ in 0..60 {
        keys(&mut screen, "k");
        up.push(current(&screen).map(str::to_owned));
    }
    let mut up: Vec<String> = up.into_iter().flatten().collect();
    up.dedup();
    assert_eq!(up, ["zz.bin", "nn.txt", "mm.bin", "app.py", "aa.bin"]);
    // n from each heading goes to the next file.
    for path in &paths[1..] {
        keys(&mut screen, "n");
        assert_eq!(current(&screen), Some(*path));
    }
}

#[test]
fn a_file_replaced_by_a_symlink_is_one_row_and_the_diff_shows_both_sides() {
    let fixture = Fixture::new("moves-type");
    fixture.write("link", "hello from the old file\n");
    fixture.git(&["add", "link"]);
    fixture.git(&["-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-qm", "link"]);
    std::fs::remove_file(fixture.repo().join("link")).expect("remove");
    std::os::unix::fs::symlink("app.py", fixture.repo().join("link")).expect("symlink");
    let mut screen = fixture.open();
    assert_eq!(screen.files.shown(), ["app.py", "link"]);
    assert_eq!(screen.diff.paths(), ["app.py", "link", "link"], "both sections");
    let rows = frame(&mut screen, 160, 40);
    let listed: Vec<&String> = rows.iter().filter(|r| entry_has(r, "link")).collect();
    assert_eq!(listed.len(), 1, "{rows:#?}");
    let row = listed.first().map(|r| r.split('\u{2502}').next().unwrap_or_default()).unwrap_or_default();
    assert!(row.contains(" T ") && row.contains("+1") && row.contains("\u{2212}1"), "{row:?}");
    // The old file's line and the link's target, each once.
    let diff: Vec<String> = rows.iter().map(|r| diff_side(&screen, r)).collect();
    assert_eq!(diff.iter().filter(|r| r.contains("hello from the old file")).count(), 1, "{diff:#?}");
    assert_eq!(
        diff.iter().filter(|r| r.contains("app.py") && !r.contains("Modified")).count(),
        1,
        "{diff:#?}"
    );
}

/// Whether the list's part of `row` names `name`.
fn entry_has(row: &str, name: &str) -> bool {
    row.split('\u{2502}').next().is_some_and(|left| left.contains(name))
}

#[test]
fn a_note_on_a_file_no_longer_changed_is_not_counted_as_hidden_by_a_filter() {
    let fixture = three("moves-reverted");
    let mut screen = fixture.open();
    at(&mut screen, 9);
    keys(&mut screen, "c");
    type_and_save(&mut screen, "name the total");
    // app.py back as it was: the note stays, and no filter hides anything.
    fixture.git(&["checkout", "--", "app.py"]);
    keys(&mut screen, "r");
    settle(&mut screen);
    assert_eq!(screen.files.shown(), ["docs/guide.md", "docs/intro.md"]);
    assert_eq!(screen.store.pending().len(), 1);
    assert_eq!(screen.notes_in_hidden_files(), 0);
    let rows = frame(&mut screen, 160, 30);
    assert!(!rows.iter().any(|r| r.contains("hidden files")), "{rows:#?}");
    // A filter that hides a file with a note still counts it.
    let fixture = three("moves-hidden-note");
    let mut screen = fixture.open();
    at(&mut screen, 9);
    keys(&mut screen, "c");
    type_and_save(&mut screen, "name the total");
    keys(&mut screen, "/guide");
    press(&mut screen, KeyCode::Enter);
    settle(&mut screen);
    assert_eq!(screen.notes_in_hidden_files(), 1);
}

#[test]
fn the_list_keys_work_over_the_whole_file_view() {
    let fixture = three("moves-whole");
    let mut screen = fixture.open();
    at(&mut screen, 9);
    keys(&mut screen, "o");
    whole_flow::settle_whole(&mut screen);
    // > and < widen and narrow the list; f moves to it and f again closes it.
    let before = screen.files.drawn_width;
    keys(&mut screen, ">");
    let _ = frame(&mut screen, 160, 30);
    assert!(screen.files.drawn_width > before, "{} > {before}", screen.files.drawn_width);
    keys(&mut screen, "<");
    let _ = frame(&mut screen, 160, 30);
    assert_eq!(screen.files.drawn_width, before);
    keys(&mut screen, "f");
    assert_eq!(screen.focus, Focus::Files);
    keys(&mut screen, "f");
    assert!(screen.files.closed);
    keys(&mut screen, "t");
    assert!(matches!(screen.mode, Mode::Kinds(_)), "{:?}", screen.mode);
    press(&mut screen, KeyCode::Esc);
    keys(&mut screen, "H");
    assert!(screen.files.filter.hide_viewed);
    keys(&mut screen, "/");
    assert_eq!(screen.mode, Mode::FilterFiles);
    keys(&mut screen, "intro");
    press(&mut screen, KeyCode::Enter);
    settle(&mut screen);
    assert_eq!(screen.diff.paths(), ["docs/intro.md"]);
    assert!(screen.whole.is_some(), "the whole file stays open");
}

#[test]
fn a_screen_too_narrow_for_the_list_says_so_and_shows_the_filter_being_typed() {
    let fixture = three("moves-small");
    let mut screen = fixture.open();
    let rows = frame(&mut screen, 40, 10);
    assert_eq!(screen.files.drawn_width, 0);
    let footer = rows.last().cloned().unwrap_or_default();
    assert!(footer.contains("no room for the file list"), "{rows:#?}");
    keys(&mut screen, "f");
    assert_eq!(screen.status.as_deref(), Some("too narrow for the file list"));
    keys(&mut screen, "/int");
    let rows = frame(&mut screen, 40, 10);
    assert!(rows.iter().any(|r| r.contains("filter: int\u{258f}")), "{rows:#?}");
    press(&mut screen, KeyCode::Enter);
    settle(&mut screen);
    assert_eq!(screen.diff.paths(), ["docs/intro.md"]);
}

#[test]
fn x_in_the_list_with_viewed_files_hidden_leaves_the_cursor_on_the_next_file() {
    let fixture = three("moves-hide-x");
    let mut screen = fixture.open();
    keys(&mut screen, "H");
    keys(&mut screen, "f");
    assert_eq!(screen.files.cursor.as_deref(), Some("docs/guide.md"));
    keys(&mut screen, "x");
    settle(&mut screen);
    assert_eq!(screen.files.shown(), ["docs/intro.md", "app.py"]);
    assert_eq!(screen.files.cursor.as_deref(), Some("docs/intro.md"), "not its folder");
    assert_eq!(current(&screen), Some("docs/intro.md"));
}

#[test]
fn enter_on_a_filter_puts_the_files_heading_at_the_top_with_the_cursor_on_it() {
    let fixture = Fixture::new("moves-filter-top");
    let lines = (1..=30).fold(String::new(), |text, n| text + "line " + &n.to_string() + "\n");
    fixture.write("b.txt", &lines);
    fixture.git(&["add", "b.txt"]);
    fixture.git(&["-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-qm", "b"]);
    // A change past line one: a gap and the hunk's heading sit between heading and lines.
    fixture.write("b.txt", &lines.replace("line 9\n", "line nine\n"));
    let mut screen = fixture.open();
    keys(&mut screen, "/b.t");
    press(&mut screen, KeyCode::Enter);
    settle(&mut screen);
    assert_eq!(screen.diff.paths(), ["b.txt"]);
    assert_eq!(screen.diff.cursor().expect("cursor"), None, "on the heading");
    assert_eq!(current(&screen), Some("b.txt"));
    let rows = frame(&mut screen, 120, 20);
    let first = rows.get(1).map(|r| diff_side(&screen, r)).unwrap_or_default();
    assert!(first.trim_start().starts_with("b.txt"), "{rows:#?}");
    assert!(rows.iter().any(|r| r.contains("Context not included")), "{rows:#?}");
}
