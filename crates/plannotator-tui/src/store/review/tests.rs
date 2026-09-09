#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::collections::BTreeMap;
use std::path::PathBuf;

use plannotator_tui_schema::{Kind, Reply, State};

use super::*;
use crate::store::Location;

fn fixture(tag: &str) -> (PathBuf, Location, Document, Store) {
    let root = std::env::temp_dir().join(format!("plannotator-review-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let path = root.join("plan.md");
    let location = Location::for_file(&root.join("data"), "project", &path);
    let doc = Document::parse("one two three\n".to_owned());
    let store = Store::load(&location, &doc).expect("load empty store");
    (root, location, doc, store)
}

fn add(store: &mut Store, doc: &Document, quote: &str, body: &str) -> String {
    let start = doc.source.find(quote).expect("quote");
    store.add(doc, start..start + quote.len(), quote.into(), Kind::Comment, body.into()).expect("add");
    store.annotations.last().expect("annotation").id.clone()
}

#[test]
fn incremental_batches_and_edits_remain_correct_after_reopening() {
    let (root, location, doc, mut store) = fixture("batches");
    let a = add(&mut store, &doc, "one", "A");
    let b = add(&mut store, &doc, "two", "B");
    store.record_delivery("agent", &[a.clone(), b.clone()]).expect("send A and B");
    let c = add(&mut store, &doc, "three", "C");
    store.record_delivery("agent", &[c]).expect("send C");

    let mut store = Store::load(&location, &doc).expect("reopen");
    assert!(store.all_delivered(), "sending C must not make A and B pending again");
    store.edit_body(&b, "B revised".into()).expect("edit immediately after send");
    let store = Store::load(&location, &doc).expect("reopen after edit");
    let pending: Vec<&str> =
        store.annotations.iter().filter(|a| store.is_pending(a)).map(|a| a.id.as_str()).collect();
    assert_eq!(pending, [b.as_str()]);
    assert!(!store.is_pending(&store.annotations[0]));
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn an_edit_advances_past_a_delivery_even_if_the_clock_went_backwards() {
    let (_root, _location, doc, _) = fixture("clock");
    let mut store = Store::transient();
    let id = add(&mut store, &doc, "one", "original");
    store.annotations[0].updated_at = "2099-01-01T00:00:00Z".into();
    store.deliveries.push(Delivered {
        at: "2099-01-01T00:00:00Z".into(),
        target: "agent".into(),
        annotation_ids: vec![id.clone()],
    });
    assert!(!store.is_pending(&store.annotations[0]));
    assert!(!store.edit_body(&id, "original".into()).expect("unchanged body"));
    assert!(!store.is_pending(&store.annotations[0]));
    store.edit_body(&id, "edited".into()).expect("edit");
    assert!(store.is_pending(&store.annotations[0]));
    store.record_delivery("agent", &[id]).expect("send despite clock skew");
    assert!(!store.is_pending(&store.annotations[0]));
}

/// `YYYY-MM-DDTHH:MM:SS.mmmZ`: exactly three fractional digits and a trailing `Z`.
fn has_millis_shape(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 24
        && bytes[19] == b'.'
        && bytes[23] == b'Z'
        && bytes[20..23].iter().all(u8::is_ascii_digit)
        && parse_time(value).is_some()
}

#[test]
fn every_written_timestamp_has_three_fractional_digits_and_a_trailing_z() {
    let doc = Document::parse("one\n".into());
    let mut store = Store::transient();
    let id = add(&mut store, &doc, "one", "note");
    let fresh = store.annotations[0].updated_at.clone();
    assert!(has_millis_shape(&fresh), "fresh timestamp was {fresh:?}");
    assert_eq!(store.annotations[0].created_at, fresh);

    store.record_delivery("agent", std::slice::from_ref(&id)).expect("send");
    let sent = store.deliveries[0].at.clone();
    assert!(has_millis_shape(&sent), "delivery timestamp was {sent:?}");
    assert!(!store.is_pending(&store.annotations[0]));

    store.edit_body(&id, "edited".into()).expect("edit right after the send");
    let advanced = store.annotations[0].updated_at.clone();
    assert!(has_millis_shape(&advanced), "advanced timestamp was {advanced:?}");
    assert!(store.is_pending(&store.annotations[0]));

    // A clock that has not moved (or went backwards) since the send advances the edit by
    // one millisecond, the smallest step the stored shape can represent.
    store.annotations[0].updated_at = "2099-01-01T00:00:00Z".into();
    store.deliveries = vec![Delivered {
        at: "2099-01-01T00:00:00.000Z".into(),
        target: "agent".into(),
        annotation_ids: vec![id.clone()],
    }];
    store.edit_body(&id, "edited again".into()).expect("edit against a stalled clock");
    assert_eq!(store.annotations[0].updated_at, "2099-01-01T00:00:00.001Z");

    // Finer digits in an older record are read but never written back: a delivery after
    // such a timestamp rounds up so the annotation does not stay pending.
    store.annotations[0].updated_at = "2099-01-01T00:00:00.425677Z".into();
    store.record_delivery("agent", &[id]).expect("send after a fine-grained timestamp");
    assert_eq!(store.deliveries.last().expect("delivery").at, "2099-01-01T00:00:00.426Z");
    assert!(!store.is_pending(&store.annotations[0]));
}

#[test]
fn rfc3339_offsets_and_precision_are_compared_as_instants() {
    let doc = Document::parse("one\n".into());
    let mut store = Store::transient();
    let id = add(&mut store, &doc, "one", "note");
    for (updated, sent, pending) in [
        ("2026-09-09T05:00:00Z", "2026-09-09T05:00:00.000Z", false),
        ("2026-09-09T05:00:00.001Z", "2026-09-09T05:00:00Z", true),
        ("2026-09-09T13:00:00+08:00", "2026-09-09T05:00:00Z", false),
        ("2026-09-09T00:00:00-05:00", "2026-09-09T05:00:00Z", false),
        ("unknown", "2026-09-09T05:00:00Z", true),
        ("2026-09-09T05:00:00Z", "unknown", true),
    ] {
        store.annotations[0].updated_at = updated.into();
        store.deliveries =
            vec![Delivered { at: sent.into(), target: "agent".into(), annotation_ids: vec![id.clone()] }];
        assert_eq!(store.is_pending(&store.annotations[0]), pending, "{updated} vs {sent}");
    }
}

#[test]
fn archiving_keeps_pending_notes_and_restores_complete_annotations_and_history() {
    let (root, location, doc, mut store) = fixture("restore");
    let a = add(&mut store, &doc, "one", "A");
    let b = add(&mut store, &doc, "two", "B");
    let annotation = &mut store.annotations[0];
    annotation.state = State::Resolved;
    annotation.author = Some("reviewer".into());
    annotation.attachments = vec!["attachment.png".into()];
    annotation.other.insert("future_field".into(), serde_json::json!({"keep": true}));
    annotation.replies.push(Reply {
        id: "reply-1".into(),
        annotation_id: a.clone(),
        body: "keep this reply".into(),
        author: None,
        author_name: None,
        created_at: annotation.created_at.clone(),
        updated_at: annotation.updated_at.clone(),
        other: BTreeMap::default(),
    });
    let original = annotation.clone();
    store.record_delivery("agent", &[a.clone(), b.clone()]).expect("send");
    store.edit_body(&b, "B edited".into()).expect("edit B");
    add(&mut store, &doc, "three", "C");
    let history = store.deliveries.clone();

    assert_eq!(store.archive_sent().expect("archive").as_slice(), std::slice::from_ref(&a));
    assert_eq!(store.len(), 2, "B and C remain active");
    let mut store = Store::load(&location, &doc).expect("reopen archive");
    assert_eq!(store.archived(), std::slice::from_ref(&original));
    assert_eq!(store.deliveries, history);
    assert_eq!(store.restore_archived(&doc, std::slice::from_ref(&a)).expect("restore"), 1);
    let store = Store::load(&location, &doc).expect("reopen restored");
    let restored = store.annotations.iter().find(|a| a.id == original.id).expect("same id");
    assert_eq!(restored, &original);
    assert!(!store.is_pending(restored), "restoring must not resend it");
    assert!(store.archived().is_empty());
    assert_eq!(store.deliveries, history);
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn archive_and_restore_failures_leave_memory_and_disk_intact() {
    let (root, location, doc, mut store) = fixture("write-failure");
    let id = add(&mut store, &doc, "one", "keep me");
    store.record_delivery("agent", std::slice::from_ref(&id)).expect("send");
    let before = std::fs::read(&location.record).expect("record");
    let blocked_tmp = location.record.with_extension("json.tmp");
    std::fs::create_dir(&blocked_tmp).expect("block temporary file");
    assert!(store.archive_sent().is_err());
    assert_eq!(store.len(), 1);
    assert!(store.archived().is_empty());
    assert_eq!(std::fs::read(&location.record).expect("record"), before);
    std::fs::remove_dir(&blocked_tmp).expect("unblock");

    store.archive_sent().expect("archive");
    let before = std::fs::read(&location.record).expect("archived record");
    std::fs::create_dir(&blocked_tmp).expect("block temporary file");
    assert!(store.restore_archived(&doc, &[id]).is_err());
    assert_eq!(store.len(), 0);
    assert_eq!(store.archived().len(), 1);
    assert_eq!(std::fs::read(&location.record).expect("record"), before);
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn restoring_never_overwrites_an_active_annotation_with_the_same_id() {
    let doc = Document::parse("one\n".into());
    let mut store = Store::transient();
    let id = add(&mut store, &doc, "one", "newer body");
    let mut archived = store.annotations[0].clone();
    archived.body = "older body".into();
    store.archived.push(archived.clone());
    assert_eq!(store.restore_archived(&doc, &[id]).expect("restore"), 0);
    assert_eq!(store.annotations[0].body, "newer body");
    assert_eq!(store.archived(), [archived]);
}

#[test]
fn archive_only_records_are_discoverable_and_sent_orphans_can_be_finished() {
    let (root, location, doc, mut store) = fixture("orphan");
    let id = add(&mut store, &doc, "one", "remove this");
    store.record_delivery("agent", std::slice::from_ref(&id)).expect("send");
    let changed = Document::parse("different text\n".into());
    store.resolve_all(&changed);
    assert_eq!(store.orphans(), 1);
    assert_eq!(store.archive_sent().expect("finish").as_slice(), std::slice::from_ref(&id));
    assert_eq!(Store::annotated_documents(&root.join("data"), "project"), [root.join("plan.md")]);
    let mut store = Store::load(&location, &changed).expect("reopen");
    store.restore_archived(&changed, &[id]).expect("restore orphan");
    assert_eq!(store.orphans(), 1);
    assert!(store.all_delivered());
    std::fs::remove_dir_all(root).expect("cleanup");
}
