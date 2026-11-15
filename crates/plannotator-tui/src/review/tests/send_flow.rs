//! Sending, the way a file review sends: new and edited notes, a blocked agent, quitting
//! with notes unsent, finishing, and the shared archive.

use ratatui::crossterm::event::KeyCode;

use super::*;
use crate::review::Mode;
use crate::ui::send_button::SendState;

fn note(screen: &mut Screen, line: u32, text: &str) {
    at(screen, line);
    keys(screen, "c");
    type_and_save(screen, text);
}

#[test]
fn send_carries_new_notes_and_resending_after_an_edit_carries_only_the_edit() {
    let fixture = Fixture::new("send");
    let mut screen = fixture.open();
    note(&mut screen, 9, "name the total");
    note(&mut screen, 14, "argv may be empty");
    keys(&mut screen, "E");
    let sent = fixture.sent();
    let [first] = sent.as_slice() else { panic!("one send") };
    assert!(first.starts_with(&format!("Review directory: {}", screen.root().display())), "{first}");
    assert!(first.contains("**Diff:** Uncommitted changes"));
    assert!(
        first.contains(
            "### Line 9 (new)\nname the total\n\n**Highlighted text:**\n```\n    total = a + b\n```"
        )
    );
    assert!(first.contains("argv may be empty"));
    assert_eq!(screen.send_state, SendState::Sent);
    keys(&mut screen, "E");
    assert_eq!(fixture.sent().len(), 1, "nothing new to send");

    press(&mut screen, KeyCode::Tab);
    keys(&mut screen, "j");
    keys(&mut screen, "e");
    screen.handle_event(Event::Paste(", or missing".into())).expect("paste");
    press(&mut screen, KeyCode::Enter);
    keys(&mut screen, "E");
    let sent = fixture.sent();
    let second = sent.last().expect("second send");
    assert!(
        second.contains("argv may be empty, or missing") && !second.contains("name the total"),
        "{second}"
    );
    keys(&mut screen, "R");
    let resent = fixture.sent();
    let all = resent.last().expect("resend");
    assert!(all.contains("name the total") && all.contains("or missing"), "R resends every note");
}

#[test]
fn a_blocked_agent_keeps_the_notes_pending_and_says_so() {
    let fixture = Fixture::new("blocked");
    fixture.agent.outcome.set(Outcome::Blocked);
    let mut screen = fixture.open();
    note(&mut screen, 9, "name the total");
    keys(&mut screen, "E");
    assert!(matches!(screen.send_state, SendState::Blocked(_)));
    assert_eq!(screen.store.pending().len(), 1);
    assert_eq!(screen.status.as_deref(), Some("test agent is at a dialog \u{b7} E retry"));
    fixture.agent.outcome.set(Outcome::Delivered);
    keys(&mut screen, "E");
    assert_eq!(screen.store.pending().len(), 0);
}

#[test]
fn quitting_with_unsent_notes_asks_first_and_send_and_close_sends() {
    let fixture = Fixture::new("quit");
    let mut screen = fixture.open();
    keys(&mut screen, "q");
    assert!(screen.wants_quit(), "nothing unsent: q quits");

    let mut screen = fixture.open();
    note(&mut screen, 9, "name the total");
    keys(&mut screen, "q");
    assert_eq!(screen.mode, Mode::ConfirmQuit);
    let rows = frame(&mut screen, 120, 40);
    assert!(
        rows[39].contains("send notes to test agent before quitting? y send \u{b7} n quit \u{b7} esc cancel")
    );
    press(&mut screen, KeyCode::Esc);
    assert!(!screen.wants_quit());
    keys(&mut screen, "S");
    assert!(screen.wants_quit());
    assert_eq!(fixture.sent().len(), 1);
}

#[test]
fn finishing_archives_sent_notes_and_undo_brings_them_back() {
    let fixture = Fixture::new("finish");
    let mut screen = fixture.open();
    note(&mut screen, 9, "name the total");
    keys(&mut screen, "E");
    note(&mut screen, 14, "argv may be empty");
    keys(&mut screen, "F");
    let texts: Vec<_> = screen.store.annotations().iter().filter_map(|n| n.text.clone()).collect();
    assert_eq!(texts, ["argv may be empty"], "the pending note stays");
    assert_eq!(screen.store.archived().len(), 1);
    keys(&mut screen, "m");
    assert_eq!(screen.mode, Mode::Menu(0));
    keys(&mut screen, "U");
    assert_eq!(screen.store.annotations().len(), 2);
    assert_eq!(screen.store.archived().len(), 0);
}

#[test]
fn a_send_is_archived_as_a_review_of_this_changeset() {
    let fixture = Fixture::new("archive");
    let mut screen = fixture.open();
    note(&mut screen, 9, "name the total");
    keys(&mut screen, "E");
    let index = fixture.data().join("feedback").join("repo").join("index.jsonl");
    let line: serde_json::Value = serde_json::from_str(read(&index).trim()).expect("one JSON line");
    assert_eq!(line["surface"], "review");
    assert_eq!(line["origin"], "claude-code");
    let review = &line["target"]["review"];
    assert_eq!(review["diffType"], "uncommitted");
    assert_eq!(review["changedFiles"], 1);
    assert_eq!(review["patchBytes"], screen.capture.patch_bytes());
    assert_eq!(review["cwd"], screen.root().display().to_string());
    let note = &line["annotations"][0];
    assert_eq!(
        (note["file"].as_str(), note["lineStart"].as_u64(), note["side"].as_str()),
        (Some("app.py"), Some(9), Some("new"))
    );
}
