#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::cell::RefCell;
use std::rc::Rc;

use plannotator_tui_schema::{DocumentSource, Kind, Provenance};

use crate::app::review_test_support::{RecordingDelivery, file_app, folder_app, press};
use crate::app::send::SendState;
use crate::app::{Focus, Mode, Open};
use crate::delivery::{Delivery, DeliveryError};
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
