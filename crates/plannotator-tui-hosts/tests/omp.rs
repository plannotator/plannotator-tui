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
