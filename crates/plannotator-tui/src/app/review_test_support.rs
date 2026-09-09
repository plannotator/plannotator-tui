//! Isolated file reviews and a recording stand-in for the external delivery transport.

#![allow(clippy::expect_used, reason = "test fixtures assert by panicking")]

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use plannotator_tui_schema::{DocumentSource, Provenance};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;

use super::{App, Open, read_file};
use crate::delivery::{Delivery, DeliveryError};
use crate::tree::Tree;

#[derive(Debug, Default, Clone, Copy)]
pub(super) enum Outcome {
    #[default]
    Success,
    Blocked,
    Unavailable,
    Failed,
}

#[derive(Debug, Default, Clone)]
pub(super) struct RecordingDelivery {
    pub(super) calls: Rc<RefCell<Vec<String>>>,
    pub(super) outcome: Rc<Cell<Outcome>>,
}

impl Delivery for RecordingDelivery {
    fn describe(&self) -> String {
        "test agent".into()
    }
    fn is_agent(&self) -> bool {
        true
    }
    fn deliver(&self, text: &str) -> Result<(), DeliveryError> {
        self.calls.borrow_mut().push(text.to_owned());
        match self.outcome.get() {
            Outcome::Success => Ok(()),
            Outcome::Blocked => Err(DeliveryError::Blocked("at a dialog".into())),
            Outcome::Unavailable => Err(DeliveryError::Unavailable("agent gone".into())),
            Outcome::Failed => Err(DeliveryError::Failed(anyhow::anyhow!("transport failed"))),
        }
    }
}

pub(super) fn file_app(tag: &str) -> (PathBuf, App, RecordingDelivery) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "plannotator-review-app-{tag}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("docs")).expect("docs");
    let path = root.join("docs/a.md");
    std::fs::write(&path, "# Plan\n\none\n\ntwo\n\nthree\n").expect("document");
    let delivery = RecordingDelivery::default();
    // Do not resolve a persistent source until the app has its private data directory.
    let source = DocumentSource::new(String::new(), "fixture", true, Provenance::Stdin);
    let mut app = App::open(source, 100, Box::new(delivery.clone())).expect("app");
    app.data_dir = root.join("data");
    app.project = "review-tests".into();
    app.open = Open::new(read_file(&path).expect("file"), 100, &app.data_dir, &app.project).expect("open");
    (root, app, delivery)
}

pub(super) fn folder_app(tag: &str) -> (PathBuf, App, RecordingDelivery) {
    let (root, mut app, delivery) = file_app(tag);
    app.tree = Some(Tree::scan(&root.join("docs")).expect("tree"));
    app.refresh_review_counts();
    (root, app, delivery)
}

pub(super) fn press(app: &mut App, ch: char) {
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Char(ch)))).expect("key");
}

pub(super) fn click(app: &mut App, rect: Rect) {
    app.handle_event(&Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: rect.x + rect.width / 2,
        row: rect.y,
        modifiers: KeyModifiers::NONE,
    }))
    .expect("click");
}

pub(super) fn draw(app: &mut App, width: u16, height: u16) -> String {
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

pub(super) fn reopen(app: &mut App) {
    let Provenance::File { path } = &app.open.source.provenance else { return };
    app.open = Open::new(read_file(path).expect("read"), 100, &app.data_dir, &app.project).expect("reopen");
    app.refresh_review_counts();
    app.derive_send_state();
}
