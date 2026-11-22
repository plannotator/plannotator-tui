//! The Send button follows the agent finder's first answer without a key press, in a document
//! review opened on its own inside Rex or Herdr. The finder runs a fake `rex` or `herdr`, held
//! by a gate until the test lets it answer. The fakes are POSIX shell scripts, so these tests
//! are Unix-only.

#![allow(clippy::expect_used, clippy::panic, reason = "tests assert by panicking")]

use std::time::{Duration, Instant};

use serde_json::json;

use super::super::review_test_support::{draw, file_app};
use super::*;
use crate::herdr::agents::{HerdrAgents, fake_herdr};
use crate::rex::agents::{FIXTURE_OWN_BLOCK, FIXTURE_SESSION, RexAgents, fake_rex};
use crate::ui::agent_picker::{AgentFinder, gated};

/// Send with nothing named: the clipboard, recording instead of copying.
#[derive(Debug, Default, Clone)]
struct FallbackCopy;

impl crate::delivery::Delivery for FallbackCopy {
    fn describe(&self) -> String {
        "clipboard".into()
    }
    fn is_fallback(&self) -> bool {
        true
    }
    fn deliver(&self, _feedback: &str) -> Result<(), crate::delivery::DeliveryError> {
        Ok(())
    }
}

/// The header row, which holds the Send button.
fn header(app: &mut App) -> String {
    let screen = draw(app, 100, 24);
    screen.lines().find(|line| line.contains("(E)")).unwrap_or_default().to_owned()
}

/// Open a review whose finder `finder` makes, hold its first look, then let it answer, doing
/// what the event loop does on each wake and never pressing a key.
fn first_look_redraws(tag: &str, finder: impl FnOnce(&Path, &Path) -> AgentFinder) {
    let (root, mut app, _) = file_app(tag);
    app.delivery = Box::new(FallbackCopy);
    let folder = agent_picker::real(&root.join("docs"));
    let gate = root.join("gate");
    let finder = finder(&root, &gate);
    app.reach = Reach::start_with(finder, folder, None);
    while app.poll_background() {}
    assert!(header(&mut app).contains("Copy 0 new (E)"), "{}", header(&mut app));
    assert!(app.background_pending(), "the loop checks often while the finder looks");
    assert!(!app.poll_background(), "nothing to redraw before the finder answers");

    std::fs::remove_file(&gate).expect("open the gate");
    let started = Instant::now();
    while app.background_pending() {
        assert!(started.elapsed() < Duration::from_secs(20), "the finder never answered");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(app.poll_background(), "the answer redraws the screen without a key");
    let after = header(&mut app);
    assert!(after.contains("Send 0 new ▸ choose agent (E)"), "{after}");
    assert!(!app.poll_background(), "the answer redraws once; an idle review stays idle");
    assert!(!app.background_pending());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn in_rex_the_send_button_asks_as_soon_as_the_finder_answers() {
    first_look_redraws("first-look-rex", |root, gate| {
        let bin = fake_rex(&root.join("fake rex"));
        AgentFinder {
            pi_root: Some(root.join("registry")),
            herdr: None,
            rex: Some(RexAgents {
                bin: gated(&bin, gate),
                session: FIXTURE_SESSION.into(),
                own_block: Some(FIXTURE_OWN_BLOCK.into()),
            }),
            ask_on_send: true,
        }
    });
}

#[test]
fn in_herdr_the_send_button_asks_as_soon_as_the_finder_answers() {
    first_look_redraws("first-look-herdr", |root, gate| {
        let agents = json!({"result": {"type": "agent_list", "agents": [
            {"agent": "claude", "agent_status": "idle", "workspace_id": "w1", "tab_id": "w1:t1", "pane_id": "w1:p2", "cwd": "/elsewhere"},
        ]}});
        let bin = fake_herdr(&root.join("fake herdr"), &agents);
        AgentFinder {
            pi_root: Some(root.join("registry")),
            herdr: Some(HerdrAgents { bin: gated(&bin, gate), own_pane: Some("w1:p9".into()) }),
            rex: None,
            ask_on_send: true,
        }
    });
}
