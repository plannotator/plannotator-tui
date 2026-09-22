#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::cell::RefCell;
use std::rc::Rc;

use plannotator_tui_schema::{DocumentSource, Kind, Provenance};

use crate::app::review_test_support::{
    Outcome, RecordingDelivery, draw, file_app, folder_app, press, reply_app,
};
use crate::app::send::SendState;
use crate::app::{Focus, Mode, Open};
use crate::delivery::{Delivery, DeliveryError, Discard};
use crate::doc::Document;
use crate::store::{Location, Store};

struct EditingDelivery {
    transport: RecordingDelivery,
    location: Location,
    document: Document,
    annotation_id: String,
    saved: Rc<RefCell<Vec<u8>>>,
}

impl Delivery for EditingDelivery {
    fn describe(&self) -> String {
        self.transport.describe()
    }

    fn is_agent(&self) -> bool {
        true
    }

    fn deliver(&self, feedback: &str) -> Result<(), DeliveryError> {
        let mut store = Store::load(&self.location, &self.document).expect("second writer");
        store.edit_body(&self.annotation_id, "edited during send".into()).expect("edit");
        let start = self.document.source.find("two").expect("quote");
        store
            .add(&self.document, start..start + 3, "two".into(), Kind::Comment, "added during send".into())
            .expect("add");
        *self.saved.borrow_mut() = std::fs::read(&self.location.record).expect("newer record");
        self.transport.deliver(feedback)
    }
}

#[test]
fn send_writeback_preserves_changes_made_while_the_transport_is_running() {
    for folder in [false, true] {
        let (root, mut app, delivery) =
            if folder { folder_app("concurrent-folder") } else { file_app("concurrent-file") };
        let path = if folder {
            app.add_quote_annotation("one", Kind::Comment, "unchanged open file".into()).expect("A");
            let path = root.join("docs/b.md");
            std::fs::write(&path, "one\n\ntwo\n").expect("unopened file");
            let (doc, mut store) = app.load_review_file(&path).expect("B store");
            store.add(&doc, 0..3, "one".into(), Kind::Comment, "original note".into()).expect("B");
            path
        } else {
            app.add_quote_annotation("one", Kind::Comment, "original note".into()).expect("note");
            root.join("docs/a.md")
        };
        let (document, store) = app.load_review_file(&path).expect("review");
        let id = store.placed()[0].annotation.id.clone();
        let location = Location::for_file(&app.data_dir, &app.project, &path);
        let saved = Rc::new(RefCell::new(Vec::new()));
        app.delivery = Box::new(EditingDelivery {
            transport: delivery.clone(),
            location: location.clone(),
            document,
            annotation_id: id,
            saved: saved.clone(),
        });

        press(&mut app, 'E');

        let (_, store) = app.load_review_file(&path).expect("reload record");
        assert_eq!(store.len(), 2, "the note added during delivery must survive");
        assert_eq!(std::fs::read(&location.record).expect("record"), *saved.borrow());
        assert!(store.placed().iter().all(|p| store.is_pending(p.annotation)));
        assert_eq!(app.send_count(), 2, "new and edited notes stay pending in the UI");
        let status = app.status.as_deref().expect("status");
        assert!(status.contains("changed while sending"), "{status}");
        assert!(status.contains("next send may repeat it"), "{status}");
        assert!(status.contains(if folder { "b.md" } else { "a.md" }), "{status}");
        assert_eq!(delivery.calls.borrow().len(), 1);
        let sent = delivery.calls.borrow()[0].clone();
        assert!(sent.contains("original note"));
        assert!(!sent.contains("during send"));
        if folder {
            assert!(app.open.store.all_delivered(), "unchanged files still record the send");
        }

        app.delivery = Box::new(delivery.clone());
        press(&mut app, 'E');
        assert_eq!(app.send_count(), 0, "the newer notes can be sent without reopening");
        let resent = delivery.calls.borrow()[1].clone();
        assert!(resent.contains("edited during send") && resent.contains("added during send"));
        assert!(!resent.contains("unchanged open file"));
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}

/// The rule: removing a delivered note creates nothing new to send. After A and B were
/// sent and B is removed, the review stays `Sent` and `q` quits without asking. This
/// holds for file reviews and reply reviews alike; 0.7.0 re-armed a reply review to
/// "Send 1" here, which would have resent A although it was never changed.
#[test]
fn removing_a_sent_annotation_leaves_the_review_sent() {
    for reply in [false, true] {
        let (root, mut app, delivery) = file_app("remove-after-send");
        if reply {
            let source = DocumentSource::new(
                "one\n\ntwo\n".into(),
                "agent reply",
                true,
                Provenance::AgentMessage {
                    host: "claude".into(),
                    session: None,
                    message_id: Some("message-1".into()),
                },
            );
            app.open = Open::new(source, 80, &app.data_dir, &app.project).expect("reply");
        }
        app.add_quote_annotation("one", Kind::Comment, "A".into()).expect("A");
        app.add_quote_annotation("two", Kind::Comment, "B".into()).expect("B");
        assert!(app.has_unsent(), "reply={reply}");
        press(&mut app, 'E');
        assert_eq!(delivery.calls.borrow().len(), 1, "reply={reply}");
        assert_eq!(app.send_state, SendState::Sent, "reply={reply}");

        app.focus = Focus::Rail;
        app.rail_cursor = 1;
        press(&mut app, 'x');
        assert_eq!(app.open.store.len(), 1, "reply={reply}");
        assert_eq!(app.status.as_deref(), Some("annotation removed"), "reply={reply}");
        assert_eq!(app.send_state, SendState::Sent, "reply={reply}: removing B is not a change to send");
        assert!(!app.has_unsent(), "reply={reply}");
        if !reply {
            assert_eq!(app.send_count(), 0, "A was delivered and is still unchanged");
        }
        press(&mut app, 'q');
        assert!(app.quit, "reply={reply}: nothing to send, so no confirmation");
        assert_eq!(app.mode, Mode::Browse, "reply={reply}");
        assert_eq!(delivery.calls.borrow().len(), 1, "reply={reply}: no send was triggered");
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}

#[test]
fn s_sends_a_reply_review_and_closes() {
    let (root, mut app, delivery) = reply_app("s-sends");
    app.add_quote_annotation("one", Kind::Comment, "A".into()).expect("A");
    press(&mut app, 'S');
    assert_eq!(delivery.calls.borrow().len(), 1);
    assert!(app.quit);
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn s_keeps_the_window_open_while_the_agent_is_at_a_dialog() {
    let (root, mut app, delivery) = reply_app("s-blocked");
    app.add_quote_annotation("one", Kind::Comment, "A".into()).expect("A");
    delivery.outcome.set(Outcome::Blocked);
    press(&mut app, 'S');
    assert!(!app.quit, "the footer stays up to say why");
    let status = app.status.as_deref().expect("status");
    assert!(status.contains("at a dialog"), "{status}");

    delivery.outcome.set(Outcome::Success);
    press(&mut app, 'S');
    assert_eq!(delivery.calls.borrow().len(), 2, "a blocked send is retried");
    assert!(app.quit);
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn s_closes_a_reply_review_with_nothing_to_send() {
    let (root, mut app, delivery) = reply_app("s-empty");
    press(&mut app, 'S');
    assert!(app.quit, "an empty review closes like q");
    assert!(delivery.calls.borrow().is_empty());
    std::fs::remove_dir_all(root).expect("cleanup");
}

/// A reply review sends every note, so `S` after `E` would repeat the whole review.
#[test]
fn s_after_a_send_closes_without_sending_again() {
    let (root, mut app, delivery) = reply_app("s-after-e");
    app.add_quote_annotation("one", Kind::Comment, "A".into()).expect("A");
    press(&mut app, 'E');
    press(&mut app, 'S');
    assert_eq!(delivery.calls.borrow().len(), 1, "the agent already has it");
    assert!(app.quit);
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn s_copies_then_closes_when_the_target_is_not_an_agent() {
    let (root, mut app, _) = reply_app("s-copy");
    app.delivery = Box::new(Discard);
    app.add_quote_annotation("one", Kind::Comment, "A".into()).expect("A");
    press(&mut app, 'S');
    let status = app.status.as_deref().expect("status");
    assert!(status.starts_with("copied 1 annotation(s)"), "{status}");
    assert!(app.quit);
    std::fs::remove_dir_all(root).expect("cleanup");
}

/// File and folder reviews finish with `F`; `S` is not theirs.
#[test]
fn s_does_nothing_in_a_file_review() {
    for folder in [false, true] {
        let (root, mut app, delivery) = if folder { folder_app("s-folder") } else { file_app("s-file") };
        app.add_quote_annotation("one", Kind::Comment, "A".into()).expect("A");
        press(&mut app, 'S');
        assert!(!app.quit, "folder={folder}");
        assert_eq!(app.mode, Mode::Browse, "folder={folder}");
        assert!(delivery.calls.borrow().is_empty(), "folder={folder}");
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}

/// The longest key help still fits at 80 columns, and names `S` only where it works.
#[test]
fn the_footer_names_s_only_in_a_reply_review_and_fits_at_80_columns() {
    for reply in [true, false] {
        let (root, mut app, _) = if reply { reply_app("s-footer") } else { file_app("s-footer-file") };
        app.roam = true;
        let screen = draw(&mut app, 80, 24);
        let footer = screen.lines().last().expect("footer");
        let help = if reply {
            "hjkl move · v select · c comment · esc blocks · S send+quit · q quit"
        } else {
            "hjkl move · v select · c comment · esc blocks · q quit"
        };
        assert!(footer.contains(help), "reply={reply}: {footer:?}");
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}
