//! The Send button follows the agent finder's first answer without a key press, in a code
//! review opened on its own inside Rex or Herdr: the answer wakes the event loop and the next
//! tick redraws. The finder runs a fake `rex` or `herdr`, held by a gate until the test lets
//! it answer.

use std::sync::mpsc;

use serde_json::json;

use super::*;
use crate::delivery::Clipboard;
use crate::herdr::agents::{HerdrAgents, fake_herdr};
use crate::rex::agents::{FIXTURE_OWN_BLOCK, FIXTURE_SESSION, RexAgents, fake_rex};
use crate::ui::agent_picker::{AgentFinder, Reach, gated};

/// The header row, which holds the Send button.
fn header(screen: &mut Screen) -> String {
    frame(screen, 120, 40).into_iter().find(|row| row.contains("(E)")).unwrap_or_default()
}

/// Open a review whose finder `finder` makes, hold its first look, then let it answer, doing
/// what the event loop does on each wake and never pressing a key.
fn first_look_wakes_and_redraws(tag: &str, finder: impl FnOnce(&Path, &Path) -> AgentFinder) {
    let fixture = Fixture::new(tag);
    let mut screen = fixture.open();
    screen.delivery = Box::new(Clipboard);
    let gate = fixture.dir.join("gate");
    let finder = finder(&fixture.dir, &gate);
    let (wake, ready) = mpsc::sync_channel(1);
    screen.reach = Reach::start_with(finder, screen.real_root(), Some(wake));
    assert!(header(&mut screen).contains("Copy 0 new (E)"), "{}", header(&mut screen));
    assert!(ready.recv_timeout(Duration::from_millis(100)).is_err(), "no wake before the answer");
    quiet(&mut screen);

    std::fs::remove_file(&gate).expect("open the gate");
    ready.recv_timeout(Duration::from_secs(20)).expect("the answer wakes the event loop");
    assert!(screen.tick().expect("tick"), "the answer redraws the screen without a key");
    let after = header(&mut screen);
    assert!(after.contains("Send 0 new ▸ choose agent (E)"), "{after}");
    // The answer redraws once; then an idle review stays idle.
    quiet(&mut screen);
    assert!(!screen.reach.poll());
}

/// Tick and draw until the viewer has nothing more to draw, as the loop does after opening.
fn quiet(screen: &mut Screen) {
    for _ in 0..50 {
        if !screen.tick().expect("tick") {
            return;
        }
        let _ = frame(screen, 120, 40);
    }
    panic!("the review kept asking to redraw");
}

#[test]
fn in_rex_the_send_button_asks_as_soon_as_the_finder_answers() {
    first_look_wakes_and_redraws("first-look-rex", |dir, gate| {
        let bin = fake_rex(&dir.join("fake rex"));
        AgentFinder {
            pi_root: Some(dir.join("registry")),
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
    first_look_wakes_and_redraws("first-look-herdr", |dir, gate| {
        let agents = json!({"result": {"type": "agent_list", "agents": [
            {"agent": "claude", "agent_status": "idle", "workspace_id": "w1", "tab_id": "w1:t1", "pane_id": "w1:p2", "cwd": "/elsewhere"},
        ]}});
        let bin = fake_herdr(&dir.join("fake herdr"), &agents);
        AgentFinder {
            pi_root: Some(dir.join("registry")),
            herdr: Some(HerdrAgents { bin: gated(&bin, gate), own_pane: Some("w1:p9".into()) }),
            rex: None,
            ask_on_send: true,
        }
    });
}
