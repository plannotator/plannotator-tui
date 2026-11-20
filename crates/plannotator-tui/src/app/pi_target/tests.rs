#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "tests assert by panicking")]

use super::super::review_test_support::file_app;
use super::*;
use crate::pi::PiEntry;
use crate::pi::link::Note;
use crate::ui::agent_picker::Reachable;

fn note(app: &mut App, file: PathBuf, quote: Option<&str>, line: Option<usize>) -> Value {
    let (note, _answer) = Note::for_test(file, quote, line, "check this");
    app.take_pi_note(&note)
}

#[test]
fn a_pi_note_lands_on_the_quote_with_its_author() {
    let (root, mut app, _) = file_app("pi-note");
    let reply = note(&mut app, root.join("docs/a.md"), Some("two"), None);
    assert_eq!(reply, json!({"ok": true}));
    let placed = app.open.store.placed();
    let [one] = placed.as_slice() else { panic!("one note: {placed:?}") };
    assert_eq!(&app.open.doc.source[one.range.clone()], "two");
    assert_eq!(one.annotation.body, "check this");
    assert_eq!(one.annotation.author_name.as_deref(), Some("pi · test"));
    drop(placed);
    assert_eq!(app.status.as_deref(), Some("note from pi · test"));
    let feedback = app.feedback();
    assert!(feedback.contains("From: pi · test"), "{feedback}");
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn a_line_picks_the_nearest_occurrence_and_alone_picks_its_block() {
    let (root, mut app, _) = file_app("pi-line");
    let path = root.join("docs/a.md");
    std::fs::write(&path, "# Plan\n\nsame\n\nother\n\nsame\n").expect("rewrite");
    app.open = super::super::Open::new(
        super::super::read_file(&path).expect("file"),
        100,
        &app.data_dir,
        &app.project,
    )
    .expect("reopen");
    assert_eq!(note(&mut app, path.clone(), Some("same"), Some(7)), json!({"ok": true}));
    assert_eq!(note(&mut app, path.clone(), None, Some(5)), json!({"ok": true}));
    let source = app.open.doc.source.clone();
    let mut starts: Vec<(usize, String)> =
        app.open.store.placed().iter().map(|p| (p.range.start, source[p.range.clone()].to_owned())).collect();
    starts.sort();
    assert_eq!(starts, vec![(14, "other".to_owned()), (21, "same".to_owned())], "{source:?}");
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn a_pi_note_says_why_it_did_not_land() {
    let (root, mut app, _) = file_app("pi-refuse");
    let other = root.join("docs/b.md");
    std::fs::write(&other, "b\n").expect("b");
    assert_eq!(note(&mut app, other, Some("b"), None)["code"], "not_open");
    assert_eq!(note(&mut app, root.join("docs/a.md"), Some("nowhere"), None)["code"], "quote_not_found");
    app.mode = Mode::Compose;
    assert_eq!(note(&mut app, root.join("docs/a.md"), Some("two"), None)["code"], "busy");
    assert!(app.open.store.placed().is_empty());
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn picking_a_pi_moves_send_and_the_last_row_moves_it_back() {
    let (root, mut app, _) = file_app("pi-pick");
    let before = app.delivery.describe();
    let entry = PiEntry {
        v: 1,
        host: "pi".into(),
        id: "0190abcd-0000".into(),
        pid: 1,
        cwd: root.clone(),
        name: Some("api".into()),
        transcript: None,
        port: 1,
        token: "t".into(),
        busy: false,
        updated: 0,
        path: PathBuf::new(),
    };
    app.agent_picker = AgentPicker {
        agents: vec![
            Reachable::Pi(entry.clone()),
            Reachable::Pi(PiEntry { name: Some("web".into()), ..entry }),
        ],
        cursor: 0,
        copy_row: false,
    };
    app.choose_target(1).expect("choose");
    assert_eq!(app.delivery.describe(), "pi · web");
    app.choose_target(0).expect("choose");
    assert_eq!(app.delivery.describe(), "pi · api");
    assert_eq!(app.pi_rows(), 3, "two sessions and the way back");
    app.choose_target(2).expect("choose");
    assert_eq!(app.delivery.describe(), before);
    assert!(app.start_delivery.is_none());
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn line_start_counts_from_one() {
    assert_eq!(line_start("a\nb\nc", 1), Some(0));
    assert_eq!(line_start("a\nb\nc", 3), Some(4));
    assert_eq!(line_start("a\nb\nc", 4), None);
}

/// Send asking which agent, against a fake `herdr`. The fake is a POSIX shell script, which
/// Windows cannot run, so these tests are Unix-only like `tests/herdr_review.rs`.
#[cfg(unix)]
mod in_herdr {
    use super::*;

    /// The clipboard a review falls back to, recording what it would have copied, so no test
    /// writes to the real terminal clipboard.
    #[derive(Debug, Default, Clone)]
    struct FallbackCopy {
        copied: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
    }

    impl crate::delivery::Delivery for FallbackCopy {
        fn describe(&self) -> String {
            "clipboard".into()
        }
        fn is_fallback(&self) -> bool {
            true
        }
        fn deliver(&self, feedback: &str) -> Result<(), crate::delivery::DeliveryError> {
            self.copied.borrow_mut().push(feedback.to_owned());
            Ok(())
        }
    }

    /// An isolated file review whose Send falls back to the clipboard, inside a fake Herdr with
    /// a Claude in the review's folder, a Codex elsewhere, and a Claude in our own pane.
    fn isolated_in_herdr(tag: &str, with_agents: bool) -> (PathBuf, App, FallbackCopy, PathBuf) {
        use crate::herdr::agents::{HerdrAgents, fake_herdr};
        use crate::ui::agent_picker::AgentFinder;
        let (root, mut app, _) = file_app(tag);
        let copy = FallbackCopy::default();
        app.delivery = Box::new(copy.clone());
        let folder = agent_picker::real(&root.join("docs"));
        let agents = if with_agents {
            json!({"result": {"type": "agent_list", "agents": [
                {"agent": "codex", "agent_status": "idle", "workspace_id": "w2", "tab_id": "w2:t1", "pane_id": "w2:p1", "cwd": "/elsewhere"},
                {"agent": "claude", "agent_status": "idle", "workspace_id": "w1", "tab_id": "w1:t1", "pane_id": "w1:p2", "cwd": folder},
                {"agent": "claude", "agent_status": "working", "workspace_id": "w1", "tab_id": "w1:t2", "pane_id": "w1:p9", "cwd": folder},
            ]}})
        } else {
            json!({"result": {"type": "agent_list", "agents": []}})
        };
        let fake = root.join("fake herdr");
        let bin = fake_herdr(&fake, &agents);
        let finder = AgentFinder {
            pi_root: Some(root.join("registry")),
            herdr: Some(HerdrAgents { bin, own_pane: Some("w1:p9".into()) }),
            ask_on_send: true,
        };
        app.reach = Reach::start_with(finder, folder.clone());
        // The first look runs on a thread; look once more here so the button is settled.
        app.reach.find(&folder);
        (root, app, copy, fake)
    }

    fn enter(app: &mut App) {
        use ratatui::crossterm::event::{Event, KeyCode, KeyEvent};
        app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Enter))).expect("enter");
    }

    #[test]
    fn send_with_no_target_asks_which_agent_and_sends_to_the_one_picked() {
        use crate::app::review_test_support::{draw, press};
        let (root, mut app, copy, fake) = isolated_in_herdr("ask-send", true);
        app.add_quote_annotation("two", Kind::Comment, "tighten this".into()).expect("note");
        assert_eq!(app.send_label(), "Send 1 new ▸ choose agent (E)");

        press(&mut app, 'E');
        assert_eq!(app.mode, Mode::PickTarget, "Send opens the picker instead of copying");
        assert!(app.agent_picker.copy_row);
        let screen = draw(&mut app, 100, 24);
        if let Some(dir) = std::env::var_os("AGENT_PICKER_PROOF_DIR") {
            std::fs::create_dir_all(&dir).expect("proof dir");
            std::fs::write(PathBuf::from(dir).join("doc-review-send-opens-picker.txt"), &screen)
                .expect("proof");
        }
        assert!(screen.contains("Copy to clipboard"), "{screen}");
        assert!(screen.contains("claude"), "{screen}");
        assert!(!screen.contains("w1:p9"), "our own pane is never offered:\n{screen}");
        let calls = std::fs::read_to_string(fake.join("calls")).expect("calls");
        assert!(!calls.contains("prompt"), "nothing sent yet: {calls}");

        // The cursor starts on the first agent: the Claude in the review's folder.
        enter(&mut app);
        assert_eq!(app.mode, Mode::Browse);
        let prompts = std::fs::read_to_string(fake.join("prompts")).expect("prompts");
        assert!(prompts.starts_with("w1:p2\n"), "{prompts}");
        assert!(prompts.contains("tighten this"), "{prompts}");
        assert_eq!(app.status.as_deref(), Some("sent 1 annotation(s) → claude in w1:p2"));
        assert_eq!(copy.copied.borrow().len(), 0);
        // Send goes there from now on, and `P` offers the way back to the clipboard.
        assert_eq!(app.delivery.describe(), "claude in w1:p2");
        assert!(app.send_label().contains("▸ claude in w1:p2"), "{}", app.send_label());
        app.open_pi_picker();
        assert!(!app.agent_picker.copy_row);
        assert_eq!(app.pi_rows(), 3, "two agents and the way back");
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn copying_is_one_row_up_and_escape_sends_nothing() {
        use crate::app::review_test_support::press;
        use ratatui::crossterm::event::{Event, KeyCode, KeyEvent};
        let (root, mut app, copy, fake) = isolated_in_herdr("ask-copy", true);
        app.add_quote_annotation("two", Kind::Comment, "tighten this".into()).expect("note");

        press(&mut app, 'E');
        app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Esc))).expect("esc");
        assert_eq!(app.mode, Mode::Browse);
        assert_eq!(app.status.as_deref(), Some("nothing sent"));
        assert_eq!(copy.copied.borrow().len(), 0);

        press(&mut app, 'E');
        press(&mut app, 'k');
        enter(&mut app);
        assert_eq!(copy.copied.borrow().len(), 1, "the copy row copies");
        assert_eq!(app.status.as_deref(), Some("copied 1 annotation(s) → clipboard"));
        assert!(!fake.join("prompts").exists(), "no agent was prompted");
        // The clipboard is still the target, so the next Send asks again.
        app.add_quote_annotation("three", Kind::Comment, "and this".into()).expect("note");
        press(&mut app, 'E');
        assert_eq!(app.mode, Mode::PickTarget);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn with_no_agent_within_reach_send_copies_as_before() {
        use crate::app::review_test_support::press;
        let (root, mut app, copy, _fake) = isolated_in_herdr("ask-none", false);
        app.add_quote_annotation("two", Kind::Comment, "tighten this".into()).expect("note");
        assert_eq!(app.send_label(), "Copy 1 new (E)");
        press(&mut app, 'E');
        assert_eq!(app.mode, Mode::Browse);
        assert_eq!(copy.copied.borrow().len(), 1);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn a_launcher_target_sends_straight_there_without_asking() {
        use crate::app::review_test_support::press;
        let (root, mut app, copy, fake) = isolated_in_herdr("ask-launched", true);
        let launched = crate::app::review_test_support::RecordingDelivery::default();
        app.delivery = Box::new(launched.clone());
        app.add_quote_annotation("two", Kind::Comment, "tighten this".into()).expect("note");
        press(&mut app, 'E');
        assert_eq!(app.mode, Mode::Browse);
        assert_eq!(launched.calls.borrow().len(), 1);
        assert_eq!(copy.copied.borrow().len(), 0);
        assert!(!fake.join("prompts").exists());
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}
