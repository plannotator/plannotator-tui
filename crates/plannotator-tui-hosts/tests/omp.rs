//! OMP reads exactly like pi, from its own root.

#![allow(clippy::expect_used, reason = "tests assert by panicking")]

use std::path::{Path, PathBuf};

use plannotator_tui_hosts::{Role, omp, pi};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

#[test]
fn omp_sessions_resolve_and_parse_with_pis_rules() {
    let root = fixtures().join("pi-sessions");
    let found = omp::find_transcript(&root, Path::new("/work/project")).expect("a session for the cwd");
    assert_eq!(found, pi::find_transcript(&root, Path::new("/work/project")).expect("pi agrees"));
    let text = std::fs::read_to_string(&found).expect("session");
    let messages = omp::parse_messages(&text, 25);
    assert_eq!(messages, pi::parse_messages(&text, 25));
    assert!(messages.iter().any(|m| m.role == Role::Assistant));
    assert_eq!(omp::DEFAULT_AGENT_DIR, ".omp/agent");
}

/// OMP discovery is pi's function, so the last-written session wins here too.
#[test]
fn omp_ranks_candidates_by_last_write_like_pi() {
    let root = std::env::temp_dir().join(format!("plannotator-tui-omp-mtime-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let bucket = root.join(pi::encoded_dir(Path::new("/work/project")));
    std::fs::create_dir_all(&bucket).expect("bucket");
    let session = |id: &str| {
        format!(
            concat!(
                r#"{{"type":"session","version":3,"id":"{0}","cwd":"/work/project"}}"#,
                "\n",
                r#"{{"type":"message","id":"m-{0}","parentId":null,"#,
                r#""message":{{"role":"assistant","content":[{{"type":"text","text":"{0}"}}]}}}}"#,
                "\n",
            ),
            id
        )
    };
    let resumed = bucket.join("2026-08-28T10-00-00-000Z_01a00000-0000-7000-8000-00000000000a.jsonl");
    let idle = bucket.join("2026-08-29T10-00-00-000Z_01a00000-0000-7000-8000-00000000000b.jsonl");
    std::fs::write(&resumed, session("a")).expect("write");
    std::fs::write(&idle, session("b")).expect("write");
    let at = |seconds: u64| std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds);
    let set = |path: &PathBuf, seconds: u64| {
        std::fs::File::options()
            .write(true)
            .open(path)
            .expect("open")
            .set_modified(at(seconds))
            .expect("mtime");
    };
    set(&idle, 1_000_000);
    set(&resumed, 1_000_060);

    assert_eq!(omp::find_transcript(&root, Path::new("/work/project")).expect("found"), resumed);
    std::fs::remove_dir_all(&root).expect("cleanup");
}
