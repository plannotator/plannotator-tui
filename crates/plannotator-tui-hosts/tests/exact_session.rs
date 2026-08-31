//! Exact session ids select one named conversation before any heuristic discovery.

#![allow(clippy::expect_used, reason = "tests assert by panicking")]

use std::path::{Path, PathBuf};

use plannotator_tui_hosts::{HostError, claude, codex, copilot, droid, hermes, omp, opencode, pi};
use rusqlite::{Connection, params};

const ID: &str = "11111111-1111-4111-8111-111111111111";
const OTHER_ID: &str = "22222222-2222-4222-8222-222222222222";

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("plannotator exact {tag} ü-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn touch(path: &Path) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("parent");
    std::fs::write(path, "{}\n").expect("file");
}

#[cfg(windows)]
fn lookup_spelling(path: &Path) -> PathBuf {
    let mut text = path.to_string_lossy().replace('/', "\\");
    if text.as_bytes().get(1) == Some(&b':') {
        let first = text.get(..1).unwrap_or_default().to_ascii_lowercase();
        text.replace_range(..1, &first);
    }
    PathBuf::from(text)
}

#[cfg(not(windows))]
fn lookup_spelling(path: &Path) -> PathBuf {
    path.to_path_buf()
}

#[test]
fn exact_file_host_ids_prefer_the_current_windows_path_bucket() {
    let root = temp_dir("file hosts");
    let home = root.join("home with spaces ü");
    let cwd = home.join("work tree");
    std::fs::create_dir_all(&cwd).expect("cwd");
    let lookup_cwd = lookup_spelling(&cwd);

    let claude_projects = root.join("claude root/projects");
    let claude_current = claude_projects.join(claude::project_slug(&cwd)).join(format!("{ID}.jsonl"));
    let claude_other = claude_projects.join("other-project").join(format!("{ID}.jsonl"));
    touch(&claude_current);
    touch(&claude_other);
    touch(&claude_projects.join("newest-unrelated").join(format!("{OTHER_ID}.jsonl")));
    assert_eq!(
        claude::find_transcript_by_id(&claude_projects, &lookup_cwd, ID).expect("claude"),
        Some(claude_current)
    );
    assert_eq!(claude::find_transcript_by_id(&claude_projects, &lookup_cwd, "missing").expect("miss"), None);

    let codex_home = root.join("codex root");
    let codex_file =
        codex_home.join("sessions/2026/08/30").join(format!("rollout-2026-08-30T10-00-00-{ID}.jsonl"));
    touch(&codex_file);
    touch(
        &codex_home.join("sessions/2026/08/31").join(format!("rollout-2026-08-31T10-00-00-{OTHER_ID}.jsonl")),
    );
    assert_eq!(codex::find_transcripts_by_id(&codex_home, ID).expect("codex"), vec![codex_file]);
    assert!(codex::find_transcripts_by_id(&codex_home, "missing").expect("miss").is_empty());

    let copilot_home = root.join("copilot root");
    let copilot_session = copilot_home.join("session-state").join(ID);
    touch(&copilot_session.join("events.jsonl"));
    touch(&copilot_home.join("session-state").join(OTHER_ID).join("events.jsonl"));
    assert_eq!(copilot::find_session_by_id(&copilot_home, ID).expect("copilot"), Some(copilot_session));
    assert_eq!(copilot::find_session_by_id(&copilot_home, "missing").expect("miss"), None);

    let factory = root.join("factory root");
    let droid_current = factory.join("sessions").join(claude::project_slug(&cwd)).join(format!("{ID}.jsonl"));
    let droid_other = factory.join("sessions/other-project").join(format!("{ID}.jsonl"));
    touch(&droid_current);
    touch(&droid_other);
    touch(&factory.join("sessions/newest-unrelated").join(format!("{OTHER_ID}.jsonl")));
    assert_eq!(droid::find_transcript_by_id(&factory, &lookup_cwd, ID).expect("droid"), Some(droid_current));
    assert_eq!(droid::find_transcript_by_id(&factory, &lookup_cwd, "missing").expect("miss"), None);

    let pi_sessions = root.join("pi sessions");
    let pi_current =
        pi_sessions.join(pi::encoded_dir(&cwd)).join(format!("2026-08-30T10-00-00-000Z_{ID}.jsonl"));
    let pi_other = pi_sessions.join("--other--").join(format!("2026-08-31T10-00-00-000Z_{ID}.jsonl"));
    touch(&pi_current);
    touch(&pi_other);
    touch(&pi_sessions.join("--newest--").join(format!("2026-09-01T10-00-00-000Z_{OTHER_ID}.jsonl")));
    assert_eq!(pi::find_transcript_by_id(&pi_sessions, &lookup_cwd, ID).expect("pi"), Some(pi_current));
    assert_eq!(pi::find_transcript_by_id(&pi_sessions, &lookup_cwd, "missing").expect("miss"), None);

    let omp_sessions = root.join("omp sessions");
    let omp_current = omp_sessions.join("-work tree").join(format!("2026-08-30T10-00-00-000Z_{ID}.jsonl"));
    let omp_other = omp_sessions.join("--other--").join(format!("2026-08-31T10-00-00-000Z_{ID}.jsonl"));
    touch(&omp_current);
    touch(&omp_other);
    touch(&omp_sessions.join("--newest--").join(format!("2026-09-01T10-00-00-000Z_{OTHER_ID}.jsonl")));
    assert_eq!(
        omp::find_transcript_by_id(
            &omp_sessions,
            &lookup_cwd,
            &lookup_spelling(&home),
            &std::env::temp_dir(),
            ID,
        )
        .expect("omp"),
        Some(omp_current)
    );
    assert_eq!(
        omp::find_transcript_by_id(
            &omp_sessions,
            &lookup_cwd,
            &lookup_spelling(&home),
            &std::env::temp_dir(),
            "missing",
        )
        .expect("miss"),
        None
    );

    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn exact_database_ids_do_not_select_a_newer_unrelated_session() {
    let root = temp_dir("database hosts");

    let hermes_db = root.join("Hermes Home ü/state.db");
    std::fs::create_dir_all(hermes_db.parent().expect("Hermes parent")).expect("Hermes parent");
    let hermes_writer = Connection::open(&hermes_db).expect("Hermes database");
    hermes_writer
        .execute_batch(
            "CREATE TABLE messages (
                id INTEGER PRIMARY KEY,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT,
                timestamp REAL NOT NULL,
                active INTEGER NOT NULL
            );",
        )
        .expect("Hermes schema");
    for (id, text, timestamp) in [(ID, "named", 1.0), (OTHER_ID, "newer unrelated", 2.0)] {
        hermes_writer
            .execute(
                "INSERT INTO messages (session_id, role, content, timestamp, active) \
                 VALUES (?1, 'assistant', ?2, ?3, 1)",
                params![id, text, timestamp],
            )
            .expect("Hermes message");
    }
    assert_eq!(
        hermes::messages_for_session(&hermes_db, ID, 1)
            .expect("Hermes exact")
            .first()
            .expect("Hermes message")
            .text,
        "named"
    );
    assert!(matches!(
        hermes::messages_for_session(&hermes_db, "missing", 1),
        Err(HostError::NoMessages(message)) if message.contains(&hermes_db.display().to_string())
    ));
    drop(hermes_writer);

    let opencode_db = root.join("OpenCode Data ü/opencode.db");
    std::fs::create_dir_all(opencode_db.parent().expect("OpenCode parent")).expect("OpenCode parent");
    let opencode_writer = Connection::open(&opencode_db).expect("OpenCode database");
    opencode_writer
        .execute_batch(
            "CREATE TABLE session (
                id TEXT PRIMARY KEY,
                parent_id TEXT,
                directory TEXT NOT NULL,
                time_updated INTEGER NOT NULL,
                time_archived INTEGER
            );",
        )
        .expect("OpenCode schema");
    for (id, updated) in [(ID, 1), (OTHER_ID, 2)] {
        opencode_writer
            .execute(
                "INSERT INTO session (id, directory, time_updated) VALUES (?1, 'C:/work tree', ?2)",
                params![id, updated],
            )
            .expect("OpenCode session");
    }
    assert_eq!(opencode::schema_of(&opencode_db, ID).expect("OpenCode exact"), opencode::Schema::V1);
    assert!(matches!(
        opencode::schema_of(&opencode_db, "missing"),
        Err(HostError::NoMessages(message)) if message.contains(&opencode_db.display().to_string())
    ));
    drop(opencode_writer);

    std::fs::remove_dir_all(root).expect("cleanup");
}

fn assert_invalid<T>(result: &Result<T, HostError>) {
    assert!(
        matches!(result, Err(HostError::InvalidSessionId(_))),
        "filesystem/database access must not replace validation"
    );
}

#[test]
fn malformed_ids_are_rejected_before_any_store_access() {
    let missing = Path::new("root that does not exist ü");
    for id in ["", " ", "..", "a..b", "../id", "dir/id", "dir\\id", "nul\0id"] {
        assert_invalid(&claude::find_transcript_by_id(missing, missing, id));
        assert_invalid(&codex::find_transcripts_by_id(missing, id));
        assert_invalid(&copilot::find_session_by_id(missing, id));
        assert_invalid(&droid::find_transcript_by_id(missing, missing, id));
        assert_invalid(&pi::find_transcript_by_id(missing, missing, id));
        assert_invalid(&omp::find_transcript_by_id(missing, missing, missing, missing, id));
        assert_invalid(&hermes::messages_for_session(missing, id, 1));
        assert_invalid(&opencode::schema_of(missing, id));
    }
}
