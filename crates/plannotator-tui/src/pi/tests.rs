use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

fn temp_root(tag: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "plannotator-tui-pi-{tag}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// A stand-in Pi: answers each connection with the next canned reply, recording requests.
fn fake_pi(replies: Vec<Value>) -> (u16, std::thread::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let handle = std::thread::spawn(move || {
        let mut seen = Vec::new();
        for reply in replies {
            let (stream, _) = listener.accept().expect("accept");
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).expect("read");
            seen.push(serde_json::from_str(&line).expect("json"));
            let mut out = serde_json::to_vec(&reply).expect("reply");
            out.push(b'\n');
            (&stream).write_all(&out).expect("write");
        }
        seen
    });
    (port, handle)
}

fn entry(root: &Path, port: u16, token: &str) -> PiEntry {
    let path = root.join("pi").join("s1-42.json");
    let value = json!({
        "v": 1, "host": "pi", "id": "s1", "pid": 42, "cwd": "/w/api", "name": "api",
        "transcript": null, "port": port, "token": token, "busy": false, "updated": 5,
    });
    write_entry(&path, &value).expect("entry");
    pi_entries(root).into_iter().next().expect("parsed")
}

/// A port nothing listens on. It lies below every ephemeral range, so no `bind(0)` in a test
/// running alongside is handed it, as it could be a port bound here and released.
const CLOSED_PORT: u16 = 1;

#[test]
fn the_root_is_the_override_or_sessions_in_the_data_dir() {
    assert_eq!(root_from(Some(" /x/s ".into()), || PathBuf::from("/d")), PathBuf::from("/x/s"));
    assert_eq!(
        root_from(Some(" ".into()), || PathBuf::from("/d")),
        PathBuf::from("/d/clients/plannotator-tui/sessions")
    );
    assert_eq!(root_from(None, || PathBuf::from("/d")), PathBuf::from("/d/clients/plannotator-tui/sessions"));
}

#[test]
fn entries_parse_skip_strangers_and_are_private() {
    let root = temp_root("entries");
    let parsed = entry(&root, 9, "t");
    assert_eq!((parsed.id.as_str(), parsed.label(), parsed.port), ("s1", "api".to_owned(), 9));
    std::fs::write(root.join("pi/junk.json"), "{").expect("junk");
    std::fs::write(root.join("pi/v2.json"), r#"{"v":2,"host":"pi"}"#).expect("v2");
    assert_eq!(pi_entries(&root).len(), 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = |p: &Path| std::fs::metadata(p).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode(&parsed.path), 0o600);
        assert_eq!(mode(&root.join("pi")), 0o700);
        assert_eq!(mode(&root), 0o700);
    }
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn a_label_falls_back_to_the_short_id() {
    let root = temp_root("label");
    let mut parsed = entry(&root, 9, "t");
    parsed.name = Some("  ".into());
    parsed.id = "0190abcdef".into();
    assert_eq!(parsed.label(), "0190abcd");
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn send_reports_started_or_queued_and_sends_the_spec_request() {
    let root = temp_root("send");
    let (port, pi) = fake_pi(vec![
        json!({"ok": true, "disposition": "started"}),
        json!({"ok": true, "disposition": "queued"}),
    ]);
    let session = entry(&root, port, "secret");
    assert_eq!(send(&session, "one").expect("one"), Disposition::Started);
    assert_eq!(send(&session, "two").expect("two"), Disposition::Queued);
    let seen = pi.join().expect("fake pi");
    assert_eq!(seen[0]["op"], "send");
    assert_eq!(seen[0]["token"], "secret");
    assert_eq!(seen[0]["text"], "one");
    assert_eq!(seen[0]["deliver"], "auto");
    assert_eq!(seen[1]["text"], "two");
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn a_rewritten_entry_is_read_again_after_a_bad_token() {
    let root = temp_root("retoken");
    let (port, pi) = fake_pi(vec![
        json!({"ok": false, "code": "bad_token"}),
        json!({"ok": true, "disposition": "started"}),
    ]);
    let stale = entry(&root, port, "old");
    entry(&root, port, "new");
    assert_eq!(send(&stale, "hi").expect("retried"), Disposition::Started);
    let seen = pi.join().expect("fake pi");
    assert_eq!((seen[0]["token"].as_str(), seen[1]["token"].as_str()), (Some("old"), Some("new")));
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn a_closed_session_is_gone_and_its_entry_removed() {
    let root = temp_root("gone");
    let session = entry(&root, CLOSED_PORT, "t");
    assert!(matches!(send(&session, "hi"), Err(SendError::Gone)));
    assert!(!session.path.exists());
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn a_pi_error_comes_back_with_its_code() {
    let root = temp_root("error");
    let (port, pi) = fake_pi(vec![json!({"ok": false, "code": "empty", "message": "nothing"})]);
    let session = entry(&root, port, "t");
    match send(&session, " ") {
        Err(SendError::Failed(message)) => assert_eq!(message, "empty: nothing"),
        other => panic!("{other:?}"),
    }
    pi.join().expect("fake pi");
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn live_sessions_keep_answering_ones_with_fresh_state_and_drop_closed_ones() {
    let root = temp_root("live");
    let (port, pi) = fake_pi(vec![json!({"ok": true, "busy": true, "name": "renamed", "id": "s1"})]);
    entry(&root, port, "t");
    let dead = root.join("pi").join("s2-43.json");
    write_entry(
        &dead,
        &json!({"v": 1, "host": "pi", "id": "s2", "pid": 43, "cwd": "/w", "port": CLOSED_PORT, "token": "t"}),
    )
    .expect("dead");
    let live = live_pi_sessions(&root);
    assert_eq!(live.len(), 1);
    assert!(live[0].busy);
    assert_eq!(live[0].label(), "renamed");
    assert!(!dead.exists(), "the closed session's entry is removed");
    assert_eq!(pi.join().expect("fake pi")[0]["op"], "ping");
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn the_link_lists_itself_answers_pings_and_hands_notes_to_the_app() {
    let root = temp_root("link");
    let doc = root.join("w").join("doc.md");
    let link = link::Link::start(&root, root.join("w"), vec![doc.clone()]).expect("link");
    let path = root.join("tui").join(format!("{}.json", std::process::id()));
    let listed: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("entry")).expect("json");
    assert_eq!(listed["host"], "plannotator-tui");
    assert_eq!(listed["files"], json!([doc]));
    let port = u16::try_from(listed["port"].as_u64().expect("port")).expect("u16");
    let token = listed["token"].as_str().expect("token").to_owned();
    assert_eq!(token.len(), 64);

    let ping = request(port, &json!({"v": 1, "token": token, "op": "ping"}), SEND_TIMEOUT).expect("ping");
    assert_eq!(ping["ok"], true);
    let bad = request(port, &json!({"v": 1, "token": "nope", "op": "ping"}), SEND_TIMEOUT).expect("bad");
    assert_eq!(bad["code"], "bad_token");
    let relative = json!({"v": 1, "token": token, "op": "note", "file": "doc.md", "note": "n"});
    assert_eq!(request(port, &relative, SEND_TIMEOUT).expect("relative")["code"], "bad_request");

    // The app's side: answer the note when it arrives, as the event loop would.
    let expected = doc.clone();
    let app = std::thread::spawn(move || {
        loop {
            if let Some(note) = link.take_notes().pop() {
                assert_eq!(note.file, expected);
                assert_eq!((note.quote.as_deref(), note.line, note.body.as_str()), (Some("q"), Some(3), "n"));
                assert_eq!(note.author, "pi · api");
                note.answer(json!({"ok": true}));
                return link;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    let request_note = json!({
        "v": 1, "token": token, "op": "note", "file": doc, "quote": "q", "line": 3,
        "note": " n ", "author": "pi · api",
    });
    assert_eq!(request(port, &request_note, SEND_TIMEOUT).expect("note"), json!({"ok": true}));
    let link = app.join().expect("app");
    let other = root.join("w").join("other.md");
    link.set_files(vec![other.clone()]);
    let listed: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("entry")).expect("json");
    assert_eq!(listed["files"], json!([other]));
    drop(link);
    assert!(!path.exists(), "the entry goes with the review");
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn the_extension_goes_in_the_pi_agent_dir() {
    let home = Path::new("/h");
    let path = |dir: Option<&str>| install::extension_path(|_| dir.map(str::to_owned), home);
    assert_eq!(path(None), PathBuf::from("/h/.pi/agent/extensions/plannotator-tui.ts"));
    assert_eq!(path(Some("~/pi")), PathBuf::from("/h/pi/extensions/plannotator-tui.ts"));
    assert_eq!(path(Some("/abs")), PathBuf::from("/abs/extensions/plannotator-tui.ts"));
}

#[test]
fn no_matching_session_is_a_no_pi_error_so_the_review_copies_instead() {
    let err = cli::pick_for_test(&[], "gone").expect_err("no session");
    assert!(err.is::<cli::NoPi>(), "{err:#}");
}
