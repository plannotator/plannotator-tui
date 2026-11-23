//! The review screen against real git repositories: a temporary repo per test, the real
//! diff viewer, a `TestBackend` frame, and a stand-in for the agent (the one external seam).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "tests assert by panicking")]

#[cfg(unix)]
mod big;
mod blocked_send;
mod disk_flow;
mod divider_mouse;
mod file_kinds;
mod file_list;
mod file_list_mouse;
mod file_moves;
#[cfg(unix)]
mod first_look;
mod jump_flow;
mod look;
mod mouse_flow;
mod mouse_guard;
mod notes_flow;
mod notes_rail;
mod pi_notes;
mod place_flow;
#[cfg(unix)]
mod pull_flow;
mod rex_picker;
mod send_flow;
mod viewed_flow;
mod whole_flow;

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use plannotator_tui_review::{Request, Side};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

use super::Screen;
use crate::delivery::{Delivery, DeliveryError};

/// The committed file, and the working copy each test starts from: an import added at the
/// top, one line changed, one line split in two, one call changed.
const BEFORE: &str = "def greet(name):\n    print(\"hello\", name)\n\n\ndef add(a, b):\n    return a + b\n\n\ndef main():\n    greet(\"world\")\n    print(add(1, 2))\n";
pub(super) const AFTER: &str = "import sys\n\n\ndef greet(name):\n    print(\"hello,\", name)\n\n\ndef add(a, b):\n    total = a + b\n    return total\n\n\ndef main():\n    greet(sys.argv[1])\n    print(add(1, 2))\n";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Outcome {
    #[default]
    Delivered,
    Blocked,
}

/// The agent: records what it was sent.
#[derive(Debug, Clone, Default)]
pub(super) struct Agent {
    pub(super) sent: Rc<RefCell<Vec<String>>>,
    pub(super) outcome: Rc<Cell<Outcome>>,
}

impl Delivery for Agent {
    fn describe(&self) -> String {
        "test agent".into()
    }
    fn is_agent(&self) -> bool {
        true
    }
    fn agent_host(&self) -> Option<&str> {
        Some("claude")
    }
    fn deliver(&self, text: &str) -> Result<(), DeliveryError> {
        self.sent.borrow_mut().push(text.to_owned());
        match self.outcome.get() {
            Outcome::Delivered => Ok(()),
            Outcome::Blocked => Err(DeliveryError::Blocked("at a dialog".into())),
        }
    }
}

/// A temporary repository with `app.py` committed and changed, and a data folder.
pub(super) struct Fixture {
    pub(super) dir: PathBuf,
    pub(super) agent: Agent,
}

impl Fixture {
    pub(super) fn new(tag: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "plannotator-review-screen-{tag}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("repo")).expect("repo dir");
        let fixture = Self { dir, agent: Agent::default() };
        fixture.git(&["init", "-q", "-b", "main"]);
        fixture.write("app.py", BEFORE);
        fixture.write("notes.txt", "unchanged\n");
        fixture.git(&["add", "."]);
        fixture.git(&["-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-qm", "init"]);
        fixture.write("app.py", AFTER);
        fixture
    }

    pub(super) fn repo(&self) -> PathBuf {
        self.dir.join("repo")
    }

    pub(super) fn data(&self) -> PathBuf {
        self.dir.join("data")
    }

    pub(super) fn write(&self, path: &str, text: &str) {
        std::fs::write(self.repo().join(path), text).expect("write");
    }

    pub(super) fn git(&self, args: &[&str]) {
        let status = Command::new("git").arg("-C").arg(self.repo()).args(args).status().expect("git");
        assert!(status.success(), "git {args:?}");
    }

    /// The review screen on the uncommitted changes, drawn once at 120×40 and loaded.
    pub(super) fn open(&self) -> Screen {
        let (repo, capture) =
            super::capture(&self.repo(), &Request::Uncommitted { base: None }).expect("capture");
        let (wake, _ready) = std::sync::mpsc::sync_channel(1);
        let mut screen =
            Screen::open(repo, capture, &self.data(), Box::new(self.agent.clone()), wake).expect("screen");
        settle(&mut screen);
        screen
    }

    pub(super) fn sent(&self) -> Vec<String> {
        self.agent.sent.borrow().clone()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Draw and poll until the viewer has every file, as the event loop would.
pub(super) fn settle(screen: &mut Screen) {
    let started = Instant::now();
    loop {
        screen.tick().expect("tick");
        let _ = frame(screen, 120, 40);
        let loaded = screen.diff.files_shown() >= screen.files.shown().len()
            && screen.diff.progress() != super::diff::Progress::Loading;
        if loaded {
            // A file a filter waits to show is gone to once its rows are drawn.
            screen.tick().expect("tick");
            return;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "the viewer never loaded");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// One frame at `cols`×`rows`, as text rows.
pub(super) fn frame(screen: &mut Screen, cols: u16, rows: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(cols, rows)).expect("terminal");
    let mut drawn = Ok(());
    terminal.draw(|f| drawn = screen.draw(f)).expect("draw");
    drawn.expect("viewer draw");
    screen.after_draw();
    let buffer = terminal.backend().buffer();
    (0..rows)
        .map(|y| (0..cols).filter_map(|x| buffer.cell((x, y)).map(|c| c.symbol().to_owned())).collect())
        .collect()
}

pub(super) fn press(screen: &mut Screen, code: KeyCode) {
    screen.handle_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE))).expect("key");
}

pub(super) fn keys(screen: &mut Screen, text: &str) {
    for c in text.chars() {
        press(screen, KeyCode::Char(c));
    }
}

/// Put the diff's cursor on new-side `line` of app.py.
pub(super) fn at(screen: &mut Screen, line: u32) {
    screen.diff.reveal("app.py", Side::New, line).expect("line is in the diff");
}

/// Type `text` into the open comment box and save it.
pub(super) fn type_and_save(screen: &mut Screen, text: &str) {
    screen.handle_event(Event::Paste(text.to_owned())).expect("paste");
    press(screen, KeyCode::Enter);
}

/// The part of a drawn row right of the file list and its divider: what the diff drew.
pub(super) fn diff_side(screen: &Screen, row: &str) -> String {
    let list = match screen.files.drawn_width {
        0 => 0,
        width => usize::from(width) + 1,
    };
    row.chars().skip(list).collect()
}

pub(super) fn row_with<'a>(rows: &'a [String], needle: &str) -> &'a str {
    rows.iter()
        .find(|r| r.contains(needle))
        .unwrap_or_else(|| panic!("no row with {needle:?}:\n{}", rows.join("\n")))
}

pub(super) fn read(path: &Path) -> String {
    std::fs::read_to_string(path).expect("read")
}
