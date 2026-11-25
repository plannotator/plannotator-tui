#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use super::*;
use crate::patch::{FilePatch, parse};

const ROOT: &str = "/work/app";

struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("pt-review-store-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Self { dir }
    }

    fn load(&self) -> ReviewStore {
        ReviewStore::load(&self.dir, "app", ROOT, &Scope::Uncommitted).expect("loads")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn file() -> FilePatch {
    let patch =
        "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,2 +1,3 @@\n one\n-two\n+TWO\n+three\n";
    parse(patch).pop().expect("a file")
}

fn note(text: &str) -> CodeAnnotation {
    CodeAnnotation::line_comment(&file(), crate::Side::New, 2, 3, text.into(), "key")
}

fn add(store: &mut ReviewStore, text: &str) -> String {
    let note = note(text);
    let id = note.id.clone();
    store.add(note).expect("adds");
    id
}

#[test]
fn the_record_lives_under_reviews_keyed_by_root_and_scope() {
    let data = Path::new("/data");
    let dir = review_dir(data, "app", ROOT, &Scope::Uncommitted);
    let hex = crate::sha256_hex(&[ROOT.as_bytes(), b"\0", b"uncommitted"], 8);
    assert_eq!(dir, data.join("clients/plannotator-tui/reviews/app").join(format!("uncommitted-{hex}")));
    let branch = review_dir(data, "app", ROOT, &Scope::Branch { base: "origin/main".into() });
    assert!(
        branch.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("branch-origin-main-"))
    );
    assert_ne!(
        review_dir(data, "app", "/work/app-wt", &Scope::Uncommitted),
        dir,
        "each worktree has its own"
    );
    assert_eq!(project_for(Path::new("/work/My App")), "my-app");
}

#[test]
fn every_change_is_saved_and_reloads_with_the_five_keys() {
    let f = Fixture::new("save");
    let mut store = f.load();
    assert!(!store.path().exists(), "loading writes nothing");
    let id = add(&mut store, "first");
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(store.path()).expect("written")).expect("JSON");
    let keys: Vec<&String> = json.as_object().expect("object").keys().collect();
    assert_eq!(keys, ["annotations", "archived", "deliveries", "repo", "scope"]);
    assert_eq!(json["repo"], ROOT);
    assert_eq!(json["scope"]["kind"], "uncommitted");
    assert_eq!(json["annotations"][0]["filePath"], "a.rs");

    assert!(
        store
            .edit(&id, |n| {
                n.text = Some("edited".into());
                true
            })
            .expect("edits")
    );
    let reloaded = f.load();
    assert_eq!(reloaded.annotations()[0].text.as_deref(), Some("edited"));
    let mut reloaded = reloaded;
    assert!(reloaded.remove(&id).expect("removes"));
    assert_eq!(f.load().annotations(), []);
    assert!(!store.path().with_extension("json.tmp").exists(), "the temp file is renamed away");
}

#[test]
fn sends_cover_notes_until_they_are_edited_across_reloads() {
    let f = Fixture::new("pending");
    let mut store = f.load();
    let a = add(&mut store, "a");
    let b = add(&mut store, "b");
    assert!(!store.all_delivered());
    store.record_delivery("claude in w1:p2", &[a.clone(), b.clone()]).expect("records");
    assert!(store.all_delivered());

    let mut store = f.load();
    assert!(store.pending().is_empty(), "the send survives a restart");
    // An edit in the same millisecond as the send still counts.
    store
        .edit(&b, |n| {
            n.text = Some("b2".into());
            true
        })
        .expect("edits");
    let reloaded = f.load();
    let pending: Vec<&str> = reloaded.pending().iter().map(|n| n.id.as_str()).collect();
    assert_eq!(pending, [b.as_str()]);
    assert!(!store.edit(&a, |_| false).expect("no-op"), "an edit that changes nothing is not saved");
    assert!(!store.is_pending(&store.annotations()[0]));
}

#[test]
fn a_send_is_never_dated_before_the_edits_it_covers() {
    let f = Fixture::new("future");
    let mut store = f.load();
    let mut ahead = note("from a clock running fast");
    ahead.plannotator_tui.updated_at = Some(now_millis() + 60_000);
    let id = ahead.id.clone();
    store.add(ahead).expect("adds");
    store.record_delivery("agent", std::slice::from_ref(&id)).expect("records");
    assert!(store.all_delivered());
    assert_eq!(parse_millis(&store.deliveries()[0].at), store.annotations()[0].plannotator_tui.updated_at);
}

#[test]
fn a_taken_note_goes_back_where_it_was_as_sent_as_it_was() {
    let f = Fixture::new("take");
    let mut store = f.load();
    let ids = [add(&mut store, "a"), add(&mut store, "b"), add(&mut store, "c")];
    let [a, b, c] = ids.clone();
    store.record_delivery("agent", std::slice::from_ref(&b)).expect("records");
    let (index, taken) = store.take(&b).expect("saves").expect("there");
    assert_eq!((index, taken.id.as_str()), (1, b.as_str()));
    assert!(store.take(&b).expect("saves").is_none(), "gone");
    let mut store = f.load();
    assert_eq!(store.annotations().len(), 2, "the removal was saved");
    assert!(store.put_back(index, taken.clone()).expect("saves"));
    let ids_now: Vec<&str> = store.annotations().iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids_now, [a.as_str(), b.as_str(), c.as_str()]);
    assert!(!store.is_pending(&store.annotations()[1]), "still sent");
    assert!(!store.put_back(index, taken).expect("no-op"), "never twice");
    assert_eq!(f.load().annotations().len(), 3, "and saved");
    // Fewer notes left than its place: it goes last.
    let (index, last) = store.take(&c).expect("saves").expect("there");
    store.take(&a).expect("saves");
    assert!(store.put_back(index, last).expect("saves"));
    assert_eq!(store.annotations().last().map(|n| n.id.clone()), Some(c));
}

#[test]
fn finishing_archives_sent_notes_and_restoring_brings_them_back() {
    let f = Fixture::new("archive");
    let mut store = f.load();
    let sent = add(&mut store, "sent");
    let unsent = add(&mut store, "unsent");
    store.record_delivery("agent", std::slice::from_ref(&sent)).expect("records");
    let archived = store.archive_sent().expect("archives");
    assert_eq!(archived, std::slice::from_ref(&sent));
    let store_ids = |s: &ReviewStore| s.annotations().iter().map(|n| n.id.clone()).collect::<Vec<_>>();
    assert_eq!(store_ids(&store), std::slice::from_ref(&unsent));

    let mut store = f.load();
    assert_eq!(store.archived().len(), 1);
    assert_eq!(store.restore_archived(&archived).expect("restores"), 1);
    assert_eq!(store_ids(&store), [unsent, sent.clone()]);
    assert_eq!(store.archived(), []);
    assert!(!store.is_pending(&store.annotations()[1]), "restoring keeps the send history");

    // Archiving again would duplicate an id already archived elsewhere: refused, unchanged.
    let mut copy = store.annotations()[1].clone();
    copy.text = Some("stale archived copy".into());
    store.record.archived.push(copy);
    let before = store.record().clone();
    assert!(matches!(store.archive_sent(), Err(Error::ArchiveConflict { ref id }) if *id == sent));
    assert_eq!(store.record(), &before);
}

#[test]
fn applied_resolutions_are_saved() {
    let f = Fixture::new("apply");
    let mut store = f.load();
    add(&mut store, "moves");
    add(&mut store, "goes stale");
    let changed = store
        .apply(&[Resolution::Moved { line_start: 7, line_end: 8 }, Resolution::Outdated])
        .expect("applies");
    assert!(changed);
    let reloaded = f.load();
    let notes = reloaded.annotations();
    assert_eq!((notes[0].line_start, notes[0].line_end), (7, 8));
    assert!(notes[1].outdated);
    assert!(
        !f.load()
            .apply(&[Resolution::Moved { line_start: 7, line_end: 8 }, Resolution::Outdated])
            .expect("ok")
    );
}

#[test]
fn a_failed_save_leaves_the_store_as_it_was() {
    let f = Fixture::new("fail");
    let mut store = f.load();
    // A file where the record's directory should be: creating it fails.
    std::fs::create_dir_all(&f.dir).expect("dir");
    std::fs::write(f.dir.join("clients"), "not a directory").expect("blocker");
    let err = store.add(note("lost?")).expect_err("cannot save");
    assert!(matches!(err, Error::Io { action: "creating", .. }), "{err}");
    assert!(store.annotations().is_empty(), "memory follows disk");
}

#[test]
fn times_read_back_to_the_millisecond_and_round_finer_digits_up() {
    assert_eq!(format_millis(1_793_000_000_123), "2026-10-26T07:33:20.123Z");
    assert_eq!(parse_millis("2026-10-26T07:33:20.123Z"), Some(1_793_000_000_123));
    assert_eq!(parse_millis("2026-10-26T07:33:20.1230001Z"), Some(1_793_000_000_124));
    assert_eq!(parse_millis("2026-10-26T08:33:20.123+01:00"), Some(1_793_000_000_123));
    assert_eq!(parse_millis("yesterday"), None);
}
