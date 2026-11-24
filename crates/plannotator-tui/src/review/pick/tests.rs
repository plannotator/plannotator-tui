//! The change picker against real repositories (the review tests' fixture), drawn on a
//! `TestBackend`, and against a stand-in source that is slow or fails on purpose.
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "tests assert by panicking")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use plannotator_tui_review::{Capture, Repo, Scope};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use super::rows::Sized;
use super::shown::{Opener, Shown, Start};
use super::source::{GitSource, Listing, PAGE, Size, Source};
use super::{Picker, Row};
use crate::review::tests::{Fixture, at, type_and_save};

fn opener(fixture: &Fixture) -> Opener {
    let agent = fixture.agent.clone();
    let (wake, _ready) = std::sync::mpsc::sync_channel(1);
    Opener {
        data_dir: fixture.data(),
        wake,
        review: crate::config::review::ReviewConfig::DEFAULT,
        remembered: None,
        notes_rail: crate::notes_rail::RailStart::default(),
        keys: crate::keys::Keymap::default(),
        shift_enter: false,
        copy_on_select: true,
        notice: None,
        delivery: Box::new(move || Box::new(agent.clone())),
        live: false,
    }
}

fn repo(fixture: &Fixture) -> Repo {
    Repo::discover(&fixture.repo()).expect("repo")
}

/// The picker on its own, as `review --pick` opens it.
fn picking(fixture: &Fixture) -> Shown {
    Shown::start(Start::Pick(repo(fixture)), opener(fixture)).expect("picker")
}

/// The review of the uncommitted changes, as `review` opens it.
fn reviewing(fixture: &Fixture) -> Shown {
    let (repo, capture) =
        crate::review::capture(&fixture.repo(), &plannotator_tui_review::Request::Uncommitted { base: None })
            .expect("capture");
    let mut shown = Shown::start(Start::Review(repo, Box::new(capture)), opener(fixture)).expect("review");
    let screen = shown.screen_mut().expect("a review");
    crate::review::tests::settle(screen);
    shown
}

fn draw(shown: &mut Shown, cols: u16, rows: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(cols, rows)).expect("terminal");
    let mut drawn = Ok(());
    terminal.draw(|f| drawn = shown.draw(f)).expect("draw");
    drawn.expect("draw");
    shown.after_draw();
    let buffer = terminal.backend().buffer();
    (0..rows)
        .map(|y| (0..cols).filter_map(|x| buffer.cell((x, y)).map(|c| c.symbol().to_owned())).collect())
        .collect()
}

fn press(shown: &mut Shown, code: KeyCode) {
    shown.handle_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE))).expect("key");
}

fn keys(shown: &mut Shown, text: &str) {
    for c in text.chars() {
        press(shown, KeyCode::Char(c));
    }
}

/// Tick and draw until every row's size is known and nothing is being opened.
fn settle(shown: &mut Shown, cols: u16, rows: u16) -> Vec<String> {
    let started = Instant::now();
    loop {
        shown.tick().expect("tick");
        let drawn = draw(shown, cols, rows);
        let Some(picker) = shown.picker_mut() else { return drawn };
        let busy = !picker.listed
            || picker.opening.is_some()
            || picker.more == super::More::Loading
            || picker
                .rows
                .iter()
                .any(|row| matches!(row.size, Sized::Waiting | Sized::Asking) && row.scope().is_some());
        if !busy {
            return drawn;
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "the picker never settled:\n{}",
            drawn.join("\n")
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Tick until the picker has closed (a review opened) or `give_up` passes.
fn until_open(shown: &mut Shown) {
    let started = Instant::now();
    while shown.picker_mut().is_some_and(|p| p.opening.is_some()) {
        shown.tick().expect("tick");
        assert!(started.elapsed() < Duration::from_secs(20), "never opened");
        std::thread::sleep(Duration::from_millis(5));
    }
    shown.tick().expect("tick");
}

fn row_with<'a>(rows: &'a [String], needle: &str) -> &'a str {
    crate::review::tests::row_with(rows, needle)
}

fn position(rows: &[String], needle: &str) -> usize {
    rows.iter()
        .position(|r| r.contains(needle))
        .unwrap_or_else(|| panic!("no {needle:?}:\n{}", rows.join("\n")))
}

fn commit(fixture: &Fixture, message: &str) {
    fixture.git(&["add", "-A"]);
    fixture.git(&["-c", "user.name=Ada", "-c", "user.email=a@example.com", "commit", "-qm", message]);
}

#[test]
fn after_the_agent_commits_the_rows_keep_their_order_the_tree_is_dimmed_and_the_cursor_is_on_the_commit() {
    let fixture = Fixture::new("pick-clean");
    commit(&fixture, "feat: greet from argv");
    let mut shown = picking(&fixture);
    let rows = settle(&mut shown, 120, 20);
    let order =
        ["Uncommitted", "Unstaged", "Staged", "feat: greet from argv", "init"].map(|t| position(&rows, t));
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}\n{}", rows.join("\n"));
    for empty in ["Uncommitted", "Unstaged", "Staged"] {
        assert!(row_with(&rows, empty).contains("(no changes)"), "{}", rows.join("\n"));
    }
    assert!(!rows.iter().any(|r| r.contains("Branch vs")), "no origin, no branch row");
    let latest = row_with(&rows, "feat: greet from argv");
    assert!(latest.starts_with("▸ "), "{}", rows.join("\n"));
    assert!(
        latest.contains("+7 −3 · 1 file") && latest.contains("now") && latest.contains("Ada"),
        "{latest}"
    );
    let footer = "No uncommitted changes · pick a commit below";
    assert!(rows.iter().any(|r| r.contains(footer)), "{}", rows.join("\n"));
    assert!(!rows.iter().any(|r| r.contains("Nothing to review")), "{}", rows.join("\n"));
}

#[test]
fn with_uncommitted_staged_and_unstaged_changes_the_cursor_starts_on_uncommitted() {
    let fixture = Fixture::new("pick-parts");
    fixture.git(&["add", "app.py"]);
    fixture.write("notes.txt", "changed, not staged\n");
    fixture.write("new.txt", "untracked\n");
    let mut shown = picking(&fixture);
    let rows = settle(&mut shown, 120, 20);
    assert!(row_with(&rows, "Uncommitted").starts_with("▸ "), "{}", rows.join("\n"));
    assert!(row_with(&rows, "Uncommitted").contains("3 files"), "{}", rows.join("\n"));
    assert!(row_with(&rows, "Unstaged").contains("2 files"), "{}", rows.join("\n"));
    assert!(row_with(&rows, "Staged").contains("+7 −3 · 1 file"), "{}", rows.join("\n"));
}

#[test]
fn enter_reviews_the_row_exactly_and_an_empty_row_only_says_so() {
    let fixture = Fixture::new("pick-enter");
    let mut shown = picking(&fixture);
    settle(&mut shown, 120, 20);
    keys(&mut shown, "jj"); // Staged: nothing staged
    press(&mut shown, KeyCode::Enter);
    let rows = draw(&mut shown, 120, 20);
    assert!(rows.iter().any(|r| r.contains("Staged has no changes")), "{}", rows.join("\n"));
    assert!(shown.screen().is_none(), "still picking");

    keys(&mut shown, "k"); // Unstaged
    press(&mut shown, KeyCode::Enter);
    until_open(&mut shown);
    let screen = shown.screen().expect("the review opened");
    assert_eq!(screen.capture.scope, Scope::Unstaged);
    assert_eq!(screen.delivery.describe(), "test agent");
}

#[test]
fn esc_on_its_own_says_q_quits_and_q_quits() {
    let fixture = Fixture::new("pick-esc");
    let mut shown = picking(&fixture);
    press(&mut shown, KeyCode::Esc);
    let rows = draw(&mut shown, 80, 12);
    assert!(rows.iter().any(|r| r.contains("q quits")), "{}", rows.join("\n"));
    assert!(!shown.wants_quit() && shown.picker_mut().is_some());
    keys(&mut shown, "q");
    assert!(shown.wants_quit());
}

#[test]
fn p_opens_the_picker_over_the_review_and_a_new_target_keeps_the_old_ones_unsent_notes() {
    let fixture = Fixture::new("pick-over");
    commit(&fixture, "second");
    fixture.write("app.py", crate::review::tests::AFTER.replace("import sys", "import os").as_str());
    let mut shown = reviewing(&fixture);
    {
        let screen = shown.screen_mut().expect("review");
        at(screen, 1);
        crate::review::tests::keys(screen, "c");
        type_and_save(screen, "why os?");
    }
    // `p` while the comment box is open is text.
    crate::review::tests::keys(shown.screen_mut().expect("review"), "c");
    keys(&mut shown, "p");
    assert!(shown.picker_mut().is_none(), "p went into the draft");
    press(&mut shown, KeyCode::Esc);
    keys(&mut shown, "y"); // discard the draft

    keys(&mut shown, "p");
    let rows = settle(&mut shown, 120, 30);
    let badge = |r: &&String| r.contains("Uncommitted") && r.contains("open · 1 unsent");
    assert!(rows.iter().any(|r| badge(&r)), "{}", rows.join("\n"));
    press(&mut shown, KeyCode::Esc);
    assert!(shown.picker_mut().is_none(), "Esc goes back to the review");
    assert_eq!(shown.screen().expect("review").capture.scope, Scope::Uncommitted);

    keys(&mut shown, "p");
    settle(&mut shown, 120, 30);
    keys(&mut shown, "jjj"); // the commit `second`
    press(&mut shown, KeyCode::Enter);
    until_open(&mut shown);
    let screen = shown.screen_mut().expect("the commit's review");
    assert!(matches!(screen.capture.scope, Scope::Commit { .. }), "{:?}", screen.capture.scope);
    assert_eq!(screen.delivery.describe(), "test agent", "the delivery came along");
    assert_eq!(screen.store.annotations().len(), 0, "the commit has notes of its own: none yet");
    let rows = crate::review::tests::frame(screen, 120, 30);
    assert!(rows.iter().any(|r| r.contains("1 unsent note kept on Uncommitted")), "{}", rows.join("\n"));

    keys(&mut shown, "p");
    settle(&mut shown, 120, 30);
    keys(&mut shown, "g");
    press(&mut shown, KeyCode::Enter);
    until_open(&mut shown);
    let screen = shown.screen().expect("back on uncommitted");
    assert_eq!(screen.capture.scope, Scope::Uncommitted);
    assert_eq!(screen.store.annotations().len(), 1, "the note waited on its own scope");
}

#[test]
fn r_lists_a_new_commit_and_a_click_opens_the_row_under_it() {
    let fixture = Fixture::new("pick-reload");
    let mut shown = picking(&fixture);
    settle(&mut shown, 120, 20);
    commit(&fixture, "landed while picking");
    keys(&mut shown, "r");
    let rows = settle(&mut shown, 120, 20);
    let y = position(&rows, "landed while picking");
    assert!(row_with(&rows, "landed while picking").starts_with("▸ "), "the tree is clean now");
    assert!(row_with(&rows, "Uncommitted").contains("(no changes)"));
    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 10,
        row: y as u16 + 1, // `init`, below it
        modifiers: KeyModifiers::NONE,
    };
    shown.handle_event(Event::Mouse(click)).expect("click");
    until_open(&mut shown);
    let screen = shown.screen().expect("opened");
    assert_eq!(screen.capture.commit_subject.as_deref(), Some("init"));
}

#[test]
fn more_lists_older_commits_the_wheel_scrolls_and_small_screens_cut_titles() {
    let fixture = Fixture::new("pick-more");
    commit(&fixture, "change 0");
    for n in 1..=24 {
        fixture.write("notes.txt", &format!("{n}\n"));
        commit(
            &fixture,
            &format!("change {n} with a subject long enough to need cutting on a narrow screen"),
        );
    }
    let mut shown = picking(&fixture);
    let rows = settle(&mut shown, 120, 40);
    assert!(rows.iter().any(|r| r.contains("more…")), "{}", rows.join("\n"));
    assert!(!rows.iter().any(|r| r.contains("change 0")), "21 or more commits: 20 shown");
    keys(&mut shown, "G");
    press(&mut shown, KeyCode::Enter);
    let rows = settle(&mut shown, 120, 40);
    assert!(rows.iter().any(|r| r.contains("change 0")), "{}", rows.join("\n"));
    assert!(row_with(&rows, "init").contains("+12 −0"), "{}", rows.join("\n"));
    assert!(!rows.iter().any(|r| r.contains("more…")), "nothing older");

    for (cols, lines) in [(40, 10), (60, 14), (200, 12)] {
        let rows = draw(&mut shown, cols, lines);
        let shown_rows: Vec<&String> = rows.iter().filter(|r| r.contains("change 2")).collect();
        assert!(!shown_rows.is_empty() || cols == 40, "{cols}:\n{}", rows.join("\n"));
        for row in &rows {
            assert!(unicode_width::UnicodeWidthStr::width(row.as_str()) <= usize::from(cols));
        }
        if cols < 120 {
            assert!(
                rows.iter().any(|r| r.contains("change") && r.contains('…')),
                "{cols}:\n{}",
                rows.join("\n")
            );
        }
        if cols == 60 {
            // The author gives way before the subject is cut to a stub.
            assert!(rows.iter().any(|r| r.contains("with a subje")), "{cols}:\n{}", rows.join("\n"));
        }
    }
    // The wheel scrolls the list, the cursor staying on screen.
    keys(&mut shown, "g");
    draw(&mut shown, 60, 14);
    let wheel = |kind| MouseEvent { kind, column: 5, row: 5, modifiers: KeyModifiers::NONE };
    shown.handle_event(Event::Mouse(wheel(MouseEventKind::ScrollDown))).expect("wheel");
    let rows = draw(&mut shown, 60, 14);
    assert!(!rows.iter().any(|r| r.contains("Uncommitted")), "scrolled past the top:\n{}", rows.join("\n"));
    assert!(rows.iter().any(|r| r.starts_with("▸ ")), "the cursor came along:\n{}", rows.join("\n"));
}

/// A source whose sizes wait until the test lets them go, and whose captures can fail.
struct Gated {
    inner: GitSource,
    open: Arc<(Mutex<bool>, Condvar)>,
    capture_error: Option<String>,
}

impl Source for Gated {
    fn name(&self) -> String {
        self.inner.name()
    }
    fn list(&self, skip: usize, count: usize) -> Result<Listing, String> {
        self.inner.list(skip, count)
    }
    fn size(&self, scope: &Scope) -> Result<Size, String> {
        let (lock, cvar) = &*self.open;
        let mut open = lock.lock().expect("lock");
        while !*open {
            open = cvar.wait(open).expect("wait");
        }
        drop(open);
        self.inner.size(scope)
    }
    fn capture(&self, scope: &Scope) -> Result<(Repo, Capture), String> {
        match &self.capture_error {
            Some(error) => Err(error.clone()),
            None => self.inner.capture(scope),
        }
    }
}

fn gated(fixture: &Fixture, capture_error: Option<&str>) -> (Shown, Arc<(Mutex<bool>, Condvar)>) {
    let open = Arc::new((Mutex::new(false), Condvar::new()));
    let source = Gated {
        inner: GitSource { repo: repo(fixture), data_dir: fixture.data() },
        open: Arc::clone(&open),
        capture_error: capture_error.map(str::to_owned),
    };
    let listing = source.list(0, PAGE).ok();
    let opener = opener(fixture);
    let picker = Picker::open(Arc::new(source), listing, None, opener.keys.clone(), opener.wake.clone());
    (Shown::picking(picker, opener), open)
}

fn release(open: &Arc<(Mutex<bool>, Condvar)>) {
    *open.0.lock().expect("lock") = true;
    open.1.notify_all();
}

#[test]
fn a_slow_source_never_holds_up_a_frame_or_a_key() {
    let fixture = Fixture::new("pick-slow");
    let (mut shown, open) = gated(&fixture, None);
    let started = Instant::now();
    let rows = draw(&mut shown, 100, 16);
    shown.tick().expect("tick");
    keys(&mut shown, "jj");
    let rows_after = draw(&mut shown, 100, 16);
    assert!(started.elapsed() < Duration::from_secs(1), "{:?}", started.elapsed());
    assert!(row_with(&rows, "Uncommitted").trim_end().ends_with('…'), "{}", rows.join("\n"));
    assert!(row_with(&rows, "init").trim_end().ends_with('t'), "the commits are there at once");
    assert!(row_with(&rows_after, "Staged").starts_with("▸ "), "{}", rows_after.join("\n"));
    release(&open);
    let rows = settle(&mut shown, 100, 16);
    assert!(row_with(&rows, "Uncommitted").contains("1 file"), "{}", rows.join("\n"));
    assert!(row_with(&rows, "Staged").starts_with("▸ "), "a moved cursor stays where it was put");
}

#[test]
fn a_capture_that_fails_says_why_and_the_picker_stays() {
    let fixture = Fixture::new("pick-fails");
    let (mut shown, open) = gated(&fixture, Some("the patch is larger than 8 MiB"));
    release(&open);
    settle(&mut shown, 100, 16);
    press(&mut shown, KeyCode::Enter);
    let rows = settle(&mut shown, 100, 16);
    assert!(shown.screen().is_none());
    assert!(
        rows.iter().any(|r| r.contains("Uncommitted: the patch is larger than 8 MiB")),
        "{}",
        rows.join("\n")
    );
}

#[cfg(unix)]
#[test]
fn thousands_of_untracked_files_do_not_delay_the_first_frame() {
    let fixture = Fixture::new("pick-many");
    let many = fixture.repo().join("generated");
    std::fs::create_dir_all(&many).expect("dir");
    for n in 0..4000 {
        std::fs::write(many.join(format!("f{n}.txt")), "x\n").expect("write");
    }
    let started = Instant::now();
    let mut shown = picking(&fixture);
    let rows = draw(&mut shown, 100, 16);
    let first_frame = started.elapsed();
    let sized = Instant::now();
    repo(&fixture).diff_stat(&Scope::Uncommitted).expect("size");
    let size_takes = sized.elapsed();
    assert!(rows.iter().any(|r| r.contains("init")), "{}", rows.join("\n"));
    assert!(first_frame < size_takes, "first frame {first_frame:?}, one size {size_takes:?}");
    let rows = settle(&mut shown, 100, 16);
    assert!(row_with(&rows, "Uncommitted").contains("4001 files"), "{}", rows.join("\n"));
}

#[test]
fn rows_name_their_scope_as_the_review_header_does() {
    let commit = plannotator_tui_review::Commit {
        sha: "1a2b3c4d5e6f".into(),
        subject: "fix: login".into(),
        author: "Ada".into(),
        time: 0,
    };
    let row = Row::new(super::rows::Kind::Commit(commit));
    assert_eq!(row.name(), "Commit 1a2b3c4");
    assert_eq!(row.title(), (Some("1a2b3c4".into()), "fix: login".into()));
    assert_eq!(row.scope(), Some(Scope::Commit { sha: "1a2b3c4d5e6f".into() }));
}

/// A source that counts how many of its calls run at once, each taking a while.
struct Counting {
    inner: GitSource,
    now: Arc<AtomicUsize>,
    most: Arc<AtomicUsize>,
}

impl Counting {
    fn call<T>(&self, work: impl FnOnce() -> T) -> T {
        let now = self.now.fetch_add(1, Ordering::SeqCst) + 1;
        self.most.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(30));
        let out = work();
        self.now.fetch_sub(1, Ordering::SeqCst);
        out
    }
}

impl Source for Counting {
    fn name(&self) -> String {
        self.inner.name()
    }
    fn list(&self, skip: usize, count: usize) -> Result<Listing, String> {
        self.call(|| self.inner.list(skip, count))
    }
    fn size(&self, scope: &Scope) -> Result<Size, String> {
        self.call(|| self.inner.size(scope))
    }
    fn capture(&self, scope: &Scope) -> Result<(Repo, Capture), String> {
        self.call(|| self.inner.capture(scope))
    }
}

#[test]
fn pressing_r_again_and_again_never_runs_more_than_two_jobs_at_once() {
    let fixture = Fixture::new("pick-spam");
    let most = Arc::new(AtomicUsize::new(0));
    let source = Counting {
        inner: GitSource { repo: repo(&fixture), data_dir: fixture.data() },
        now: Arc::new(AtomicUsize::new(0)),
        most: Arc::clone(&most),
    };
    let opener = opener(&fixture);
    let picker = Picker::open(Arc::new(source), None, None, opener.keys.clone(), opener.wake.clone());
    let mut shown = Shown::picking(picker, opener);
    for _ in 0..25 {
        keys(&mut shown, "r");
        shown.tick().expect("tick");
        let running = shown.picker_mut().expect("picking").work.running();
        assert!(running <= super::source::AT_ONCE, "{running} jobs running");
    }
    let rows = settle(&mut shown, 100, 16);
    assert!(most.load(Ordering::SeqCst) <= super::source::AT_ONCE, "{} at once", most.load(Ordering::SeqCst));
    assert!(row_with(&rows, "Uncommitted").contains("1 file"), "{}", rows.join("\n"));
}

/// The pid and index copy of every run of a clean filter that takes 30 s, from its log.
#[cfg(unix)]
fn filter_runs(log: &std::path::Path) -> Vec<(i32, String)> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let (pid, index) = line.split_once(' ')?;
            Some((pid.parse().ok()?, index.to_owned()))
        })
        .collect()
}

#[cfg(unix)]
#[test]
fn quitting_while_sizes_are_counted_leaves_no_git_running_and_no_index_copy() {
    let fixture = Fixture::new("pick-quit");
    let log = fixture.dir.join("filter.log");
    let filter = format!("sh -c 'echo $$ $GIT_INDEX_FILE >> {}; sleep 30; cat'", log.display());
    fixture.git(&["config", "filter.slow.clean", &filter]);
    fixture.write(".gitattributes", "*.py filter=slow\n");
    let mut shown = picking(&fixture);
    let started = Instant::now();
    while filter_runs(&log).is_empty() {
        shown.tick().expect("tick");
        assert!(started.elapsed() < Duration::from_secs(20), "the filter never ran");
        std::thread::sleep(Duration::from_millis(10));
    }
    let runs = filter_runs(&log);
    let quitting = Instant::now();
    keys(&mut shown, "q");
    assert!(shown.wants_quit());
    drop(shown);
    assert!(quitting.elapsed() < Duration::from_secs(3), "quitting waited {:?}", quitting.elapsed());
    for (pid, index) in runs {
        let pid = rustix::process::Pid::from_raw(pid).expect("a pid");
        let gone = (0..100).any(|_| {
            std::thread::sleep(Duration::from_millis(10));
            rustix::process::test_kill_process(pid) == Err(rustix::io::Errno::SRCH)
        });
        assert!(gone, "the filter git started, {pid:?}, still runs");
        let copy = std::path::Path::new(&index).parent().expect("the copy's folder");
        assert!(!index.is_empty() && !copy.exists(), "{} was left behind", copy.display());
    }
}

#[test]
fn a_click_while_the_keys_box_is_open_only_closes_it() {
    let fixture = Fixture::new("pick-help-click");
    let mut shown = picking(&fixture);
    let rows = settle(&mut shown, 100, 16);
    let y = position(&rows, "init");
    keys(&mut shown, "?");
    let rows = draw(&mut shown, 100, 16);
    assert!(rows.iter().any(|r| r.contains("picker keys")), "{}", rows.join("\n"));
    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 50,
        row: y as u16,
        modifiers: KeyModifiers::NONE,
    };
    shown.handle_event(Event::Mouse(click)).expect("click");
    let wheel = MouseEvent { kind: MouseEventKind::ScrollDown, ..click };
    keys(&mut shown, "?");
    shown.handle_event(Event::Mouse(wheel)).expect("wheel");
    let picker = shown.picker_mut().expect("still picking");
    assert!(picker.opening.is_none(), "nothing under the box was opened");
    assert_eq!(picker.scroll, 0, "nor scrolled");
    let rows = settle(&mut shown, 100, 16);
    assert!(rows.iter().any(|r| r.contains("picker keys")), "the wheel left the box open");
    shown.handle_event(Event::Mouse(click)).expect("click");
    let rows = draw(&mut shown, 100, 16);
    assert!(!rows.iter().any(|r| r.contains("picker keys")), "a click closed it");
    assert!(shown.screen().is_none());
}

#[test]
fn the_keys_box_lines_its_descriptions_up_after_the_widest_keys() {
    let fixture = Fixture::new("pick-help-cols");
    let mut shown = picking(&fixture);
    keys(&mut shown, "?");
    let rows = draw(&mut shown, 100, 20);
    let column = |what: &str| {
        let row = row_with(&rows, what);
        row.find(what).map(|byte| unicode_width::UnicodeWidthStr::width(&row[..byte]))
    };
    let first = column("next / previous row");
    assert!(first.is_some(), "{}", rows.join("\n"));
    for what in ["a page down / up", "first / last row", "quit"] {
        assert_eq!(column(what), first, "{what}:\n{}", rows.join("\n"));
    }
}

#[test]
fn over_a_review_the_cursor_starts_on_it_and_enter_there_goes_back_untouched() {
    let fixture = Fixture::new("pick-same");
    commit(&fixture, "second");
    fixture.write("app.py", crate::review::tests::AFTER.replace("import sys", "import os").as_str());
    let mut shown = reviewing(&fixture);
    crate::review::tests::keys(shown.screen_mut().expect("review"), "jjjj");
    let before = draw(&mut shown, 120, 30);
    keys(&mut shown, "p");
    let rows = settle(&mut shown, 120, 30);
    let open = rows.iter().find(|r| r.contains("open")).expect("the open row");
    assert!(open.contains("▸ Uncommitted"), "{}", rows.join("\n"));
    press(&mut shown, KeyCode::Enter);
    assert!(shown.picker_mut().is_none(), "the picker closed at once");
    // The diff as it was; the footer may say the colours are still being worked out.
    let after = draw(&mut shown, 120, 30);
    assert_eq!(after[..29], before[..29], "the review is as it was");

    // Over the commit's review, its row is the one the cursor starts on.
    keys(&mut shown, "p");
    settle(&mut shown, 120, 30);
    keys(&mut shown, "jjj");
    press(&mut shown, KeyCode::Enter);
    until_open(&mut shown);
    keys(&mut shown, "p");
    let rows = settle(&mut shown, 120, 30);
    assert!(row_with(&rows, "second").contains("▸ ") && row_with(&rows, "second").contains("open"));
}

#[test]
fn quitting_counts_the_notes_left_unsent_on_another_review_and_can_go_to_it() {
    let fixture = Fixture::new("pick-quit-elsewhere");
    commit(&fixture, "second");
    fixture.write("app.py", crate::review::tests::AFTER.replace("import sys", "import os").as_str());
    let mut shown = reviewing(&fixture);
    {
        let screen = shown.screen_mut().expect("review");
        at(screen, 1);
        crate::review::tests::keys(screen, "c");
        type_and_save(screen, "why os?");
    }
    keys(&mut shown, "p");
    settle(&mut shown, 120, 30);
    keys(&mut shown, "jjj"); // the commit `second`
    press(&mut shown, KeyCode::Enter);
    until_open(&mut shown);
    keys(&mut shown, "q");
    assert!(!shown.wants_quit(), "it asks first");
    let rows = draw(&mut shown, 140, 30);
    let prompt = rows.last().expect("footer");
    assert!(prompt.contains("1 unsent note on Uncommitted"), "{prompt}");
    assert!(
        prompt.contains("n quit") && prompt.contains("esc go to it") && !prompt.contains("y send"),
        "{prompt}"
    );
    keys(&mut shown, "y");
    assert!(!shown.wants_quit(), "nothing here to send");

    // Back: the picker, on the review with the note.
    press(&mut shown, KeyCode::Esc);
    let rows = settle(&mut shown, 120, 30);
    assert!(row_with(&rows, "▸ Uncommitted").contains("1 unsent"), "{}", rows.join("\n"));
    press(&mut shown, KeyCode::Esc);

    // A note here too: send is offered for it, and the other is still named.
    {
        let screen = shown.screen_mut().expect("the commit's review");
        crate::review::tests::keys(screen, "jjjc");
        type_and_save(screen, "and here");
    }
    keys(&mut shown, "q");
    let rows = draw(&mut shown, 160, 30);
    let prompt = rows.last().expect("footer");
    assert!(
        prompt.contains("send notes to test agent before quitting? (1 unsent note on Uncommitted too)")
            && prompt.contains("y send"),
        "{prompt}"
    );
    keys(&mut shown, "n");
    assert!(shown.wants_quit());
}

#[test]
fn a_two_column_box_over_a_review_draws_without_panicking() {
    let fixture = Fixture::new("pick-tiny");
    let mut shown = reviewing(&fixture);
    keys(&mut shown, "p");
    for (cols, rows) in [(2, 3), (3, 2), (1, 1), (40, 10)] {
        draw(&mut shown, cols, rows);
    }
    keys(&mut shown, "?");
    draw(&mut shown, 2, 2);
}

#[test]
fn control_characters_in_a_subject_or_author_keep_the_columns_lined_up() {
    let fixture = Fixture::new("pick-control");
    fixture.git(&["add", "-A"]);
    fixture.git(&[
        "-c",
        "user.name=Ada\tL\u{1b}[31m",
        "-c",
        "user.email=a@example.com",
        "commit",
        "-qm",
        "fix:\ttabs\u{7} and \u{1b}[1mbold",
    ]);
    let mut shown = picking(&fixture);
    let rows = settle(&mut shown, 100, 16);
    let dot = |needle: &str| {
        let row = row_with(&rows, needle);
        row.find(" · ").map(|byte| unicode_width::UnicodeWidthStr::width(&row[..byte]))
    };
    assert!(dot("tabs").is_some(), "{}", rows.join("\n"));
    assert_eq!(dot("tabs"), dot("init"), "{}", rows.join("\n"));
    assert!(row_with(&rows, "tabs").contains("fix: tabs and [1mbold"), "{}", rows.join("\n"));
    assert!(!rows.iter().any(|r| r.contains('\u{1b}') || r.contains('\t')));
}

#[test]
fn a_commit_gone_since_the_list_was_read_says_to_reload() {
    let fixture = Fixture::new("pick-gone");
    commit(&fixture, "second");
    let (mut shown, open) = gated(&fixture, Some("git show failed: fatal: bad object 0123abc"));
    release(&open);
    settle(&mut shown, 100, 16);
    keys(&mut shown, "g");
    keys(&mut shown, "jjj"); // the commit `second`
    press(&mut shown, KeyCode::Enter);
    let rows = settle(&mut shown, 100, 16);
    assert!(rows.iter().any(|r| r.contains("bad object 0123abc · press r to reload")), "{}", rows.join("\n"));
}

#[test]
fn a_long_footer_message_ends_in_an_ellipsis() {
    let fixture = Fixture::new("pick-long");
    let (mut shown, open) = gated(&fixture, Some(&"x".repeat(300)));
    release(&open);
    settle(&mut shown, 60, 12);
    press(&mut shown, KeyCode::Enter);
    let rows = settle(&mut shown, 60, 12);
    let footer = row_with(&rows, "Uncommitted: x");
    assert!(footer.trim_end().ends_with('…'), "{}", rows.join("\n"));
}

#[test]
fn a_repository_with_no_commits_says_there_is_nothing() {
    let fixture = Fixture::new("pick-none");
    let fresh = fixture.dir.join("fresh");
    std::fs::create_dir_all(&fresh).expect("dir");
    let status =
        std::process::Command::new("git").arg("-C").arg(&fresh).args(["init", "-q"]).status().expect("git");
    assert!(status.success());
    let repo = Repo::discover(&fresh).expect("repo");
    let mut shown = Shown::start(Start::Pick(repo), opener(&fixture)).expect("picker");
    let rows = settle(&mut shown, 80, 12);
    assert!(rows.iter().any(|r| r.contains("Nothing to review in fresh")), "{}", rows.join("\n"));
    assert!(!rows.iter().any(|r| r.contains("pick a commit")), "{}", rows.join("\n"));
}
