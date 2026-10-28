#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use plannotator_tui_schema::Kind;

use crate::app::Mode;
use crate::app::review_test_support::{Outcome, click, draw, file_app, folder_app, press};
use crate::gate::{Decision, Gate};

fn archived(app: &crate::app::App) -> Vec<serde_json::Value> {
    let index = app.data_dir.join("feedback").join(&app.project).join("index.jsonl");
    std::fs::read_to_string(index)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).expect("json line"))
        .collect()
}

#[test]
fn without_a_gate_nothing_changes() {
    let (root, mut app, delivery) = file_app("ungated");
    app.add_quote_annotation("one", Kind::Comment, "note".into()).expect("note");
    let screen = draw(&mut app, 100, 12);
    assert!(!screen.contains("Approve"), "{screen}");
    assert!(app.geometry.approve_button.is_none());
    assert!(!screen.lines().last().unwrap_or_default().contains("approve"));

    press(&mut app, 'A');
    assert!(delivery.calls.borrow().is_empty(), "A means nothing outside a gate");
    assert!(!app.quit);
    press(&mut app, 'E');
    assert_eq!(delivery.calls.borrow().len(), 1);
    assert!(!app.quit, "an ordinary send leaves the review open");
    assert_eq!(app.take_decision(), None);
    assert!(archived(&app).iter().all(|record| record["decision"] == "feedback"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_bare_approval_tells_the_agent_and_ends_the_review() {
    let (root, mut app, delivery) = file_app("bare");
    app.gate = Gate::Approve;
    press(&mut app, 'A');
    assert_eq!(*delivery.calls.borrow(), ["The user approved."]);
    assert!(app.quit);
    assert_eq!(app.take_decision(), Some(Decision::Approved { feedback: String::new(), count: 0 }));
    let records = archived(&app);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["decision"], "approved");
    assert!(records[0].get("recordFile").is_none(), "a bare approval is a decision-only line");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unsent_notes_ride_along_as_an_approval_with_notes() {
    let (root, mut app, delivery) = file_app("notes");
    app.gate = Gate::Approve;
    app.add_quote_annotation("two", Kind::Comment, "keep it short".into()).expect("note");
    let screen = draw(&mut app, 100, 12);
    assert!(screen.lines().next().unwrap_or_default().contains("Approve with 1 note (A)"), "{screen}");

    press(&mut app, 'A');
    let message = delivery.calls.borrow()[0].clone();
    let path = root.join("docs/a.md");
    assert!(message.starts_with("# Approved with Notes\n\nThe artifact is approved."), "{message}");
    assert!(message.contains(&format!("File: {}\n\n# Annotations on a.md", path.display())), "{message}");
    assert!(message.contains("> keep it short"), "{message}");
    assert_eq!(app.send_count(), 0, "the notes went with the approval");
    let decision = app.take_decision();
    assert!(
        matches!(&decision, Some(Decision::Approved { feedback, count: 1 })
            if feedback.starts_with("# Annotations on a.md")),
        "{decision:?}"
    );
    assert_eq!(archived(&app)[0]["decision"], "approved-with-notes");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_gated_send_ends_the_review_as_annotated() {
    let (root, mut app, delivery) = file_app("send");
    app.gate = Gate::Approve;
    app.add_quote_annotation("one", Kind::Comment, "fix".into()).expect("note");
    press(&mut app, 'E');
    assert!(app.quit);
    let sent = delivery.calls.borrow()[0].clone();
    assert!(sent.starts_with("# Annotations on a.md"), "the numbered feedback, as an ordinary send");
    assert_eq!(app.take_decision(), Some(Decision::Annotated { feedback: sent, count: 1 }));
    assert_eq!(archived(&app)[0]["decision"], "feedback");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn quitting_a_gated_review_dismisses_it() {
    let (root, mut app, delivery) = file_app("quit");
    app.gate = Gate::Approve;
    press(&mut app, 'q');
    assert!(app.quit);
    assert_eq!(app.take_decision(), Some(Decision::Dismissed));
    assert!(delivery.calls.borrow().is_empty(), "a dismissal tells the agent nothing");
    assert_eq!(archived(&app)[0]["decision"], "dismissed");

    // Unsent notes ask first; declining the send still dismisses.
    let (other, mut app, delivery) = file_app("quit-notes");
    app.gate = Gate::Approve;
    app.add_quote_annotation("one", Kind::Comment, "fix".into()).expect("note");
    press(&mut app, 'q');
    assert_eq!(app.mode, Mode::ConfirmQuit);
    press(&mut app, 'n');
    assert!(app.quit);
    assert_eq!(app.take_decision(), Some(Decision::Dismissed));
    assert!(delivery.calls.borrow().is_empty());
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(other);
}

#[test]
fn a_refused_approval_keeps_the_review_open() {
    let (root, mut app, delivery) = file_app("blocked");
    app.gate = Gate::Approve;
    delivery.outcome.set(Outcome::Blocked);
    press(&mut app, 'A');
    assert!(!app.quit);
    assert!(app.status.as_deref().is_some_and(|s| s.contains("at a dialog · A retry")), "{:?}", app.status);
    assert!(archived(&app).is_empty(), "nothing was decided");
    delivery.outcome.set(Outcome::Success);
    press(&mut app, 'A');
    assert!(app.quit);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_approve_button_sits_on_the_edge_and_approves_when_clicked() {
    let (root, mut app, delivery) = file_app("button");
    app.gate = Gate::Approve;
    let screen = draw(&mut app, 100, 12);
    let header = screen.lines().next().unwrap_or_default();
    assert!(header.trim_end().ends_with("Approve (A)"), "{header}");
    let footer = screen.lines().last().unwrap_or_default();
    assert!(footer.contains("A approve"), "{footer}");
    let rect = app.geometry.approve_button.expect("drawn");
    assert_eq!(rect.right(), 100);
    click(&mut app, rect);
    assert_eq!(*delivery.calls.borrow(), ["The user approved."]);
    assert!(app.quit);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn json_alone_decides_without_an_approve() {
    let (root, mut app, delivery) = file_app("decision");
    app.gate = Gate::Decision;
    let screen = draw(&mut app, 100, 12);
    assert!(!screen.contains("Approve"), "{screen}");
    press(&mut app, 'A');
    assert!(delivery.calls.borrow().is_empty() && !app.quit);
    press(&mut app, 'q');
    assert_eq!(app.take_decision(), Some(Decision::Dismissed));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_folder_approval_names_the_folder() {
    let (root, mut app, delivery) = folder_app("folder");
    app.gate = Gate::Approve;
    app.add_quote_annotation("one", Kind::Comment, "fix".into()).expect("note");
    app.refresh_review_counts();
    press(&mut app, 'A');
    let message = delivery.calls.borrow()[0].clone();
    assert!(message.contains(&format!("Folder: {}\n\n", root.join("docs").display())), "{message}");
    assert!(matches!(app.take_decision(), Some(Decision::Approved { count: 1, .. })));
    let _ = std::fs::remove_dir_all(root);
}
