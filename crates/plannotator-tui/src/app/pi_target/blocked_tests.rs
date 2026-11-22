//! Choosing an agent that is at a dialog, in a document review: nothing is sent, nothing is
//! copied, the note stays pending, and the screen says why. Against a fake `herdr` and a fake
//! `rex` (POSIX shell scripts, so Unix-only). The review starts with the real clipboard
//! fallback and copying turned on; in tests [`Clipboard`] only records what it would copy.

#![allow(clippy::expect_used, clippy::panic, reason = "tests assert by panicking")]

use ratatui::crossterm::event::{Event, KeyCode, KeyEvent};

use super::super::review_test_support::{file_app, press};
use super::*;
use crate::delivery::{Clipboard, test_clipboard};
use crate::herdr::agents::{HerdrAgents, fake_herdr};
use crate::rex::agents::{FIXTURE_OWN_BLOCK, FIXTURE_SESSION, RexAgents, fake_rex};
use crate::ui::agent_picker::AgentFinder;
use crate::ui::send_button::SendState;

fn enter(app: &mut App) {
    app.handle_event(&Event::Key(KeyEvent::from(KeyCode::Enter))).expect("enter");
}

/// A file review that copies when nothing names a target, as a real one does, with one note.
fn review(tag: &str, finder: impl FnOnce(&Path) -> AgentFinder) -> (PathBuf, App) {
    let (root, mut app, _) = file_app(tag);
    app.delivery = Box::new(Clipboard);
    app.clipboard = true;
    let folder = agent_picker::real(&root.join("docs"));
    app.reach = Reach::start_with(finder(&root), folder.clone(), None);
    app.reach.find(&folder);
    app.add_quote_annotation("two", Kind::Comment, "tighten this".into()).expect("note");
    (root, app)
}

fn assert_waits(app: &App, target: &str) {
    assert_eq!(app.status.as_deref(), Some(format!("{target} is at a dialog · E retry").as_str()));
    assert!(matches!(app.send_state, SendState::Blocked(_)), "{:?}", app.send_state);
    assert_eq!(test_clipboard::copied(), [] as [String; 0], "nothing copied");
    assert_eq!(app.send_count(), 1, "the note is still pending");
    assert_eq!(app.delivery.describe(), target, "the agent stays the target for the retry");
}

#[test]
fn a_herdr_agent_at_a_dialog_gets_nothing_and_nothing_is_copied() {
    let (root, mut app) = review("blocked-herdr", |root| {
        let agents = serde_json::json!({"result": {"type": "agent_list", "agents": [
            {"agent": "claude", "agent_status": "blocked", "workspace_id": "w1", "tab_id": "w1:t1", "pane_id": "w1:p2", "cwd": "/elsewhere"},
        ]}});
        let fake = root.join("fake herdr");
        let bin = fake_herdr(&fake, &agents);
        std::fs::write(fake.join("blocked"), "").expect("blocked flag");
        AgentFinder {
            pi_root: Some(root.join("registry")),
            herdr: Some(HerdrAgents { bin, own_pane: None }),
            rex: None,
            ask_on_send: true,
        }
    });
    press(&mut app, 'E');
    assert_eq!(app.mode, Mode::PickTarget);
    enter(&mut app);
    assert_waits(&app, "claude in w1:p2");
    assert!(!root.join("fake herdr/prompts").exists());
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn a_rex_agent_at_a_dialog_gets_nothing_and_nothing_is_copied() {
    let (root, mut app) = review("blocked-rex", |root| {
        let bin = fake_rex(&root.join("fake rex"));
        AgentFinder {
            pi_root: Some(root.join("registry")),
            herdr: None,
            rex: Some(RexAgents {
                bin,
                session: FIXTURE_SESSION.into(),
                own_block: Some(FIXTURE_OWN_BLOCK.into()),
            }),
            ask_on_send: true,
        }
    });
    press(&mut app, 'E');
    assert_eq!(app.mode, Mode::PickTarget);
    // Copy, the two agents in this window, then the Claude at a permission prompt.
    press(&mut app, 'j');
    press(&mut app, 'j');
    enter(&mut app);
    assert_waits(&app, "claude in 63239c");
    assert!(!root.join("fake rex/pastes").exists());
    std::fs::remove_dir_all(root).expect("cleanup");
}
