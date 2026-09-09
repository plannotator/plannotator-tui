#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::cell::RefCell;
use std::rc::Rc;

use plannotator_tui_schema::Kind;

use crate::app::review_test_support::{RecordingDelivery, file_app, folder_app, press};
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
