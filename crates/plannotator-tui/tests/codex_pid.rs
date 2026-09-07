//! Linux PID discovery through the CLI, with real open transcript descriptors.

#![cfg(target_os = "linux")]
#![allow(clippy::expect_used, reason = "tests assert by panicking")]

use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

const SELECTED_ID: &str = "11111111-1111-4111-8111-111111111111";
const OTHER_ID: &str = "22222222-2222-4222-8222-222222222222";
const SUBAGENT_ID: &str = "33333333-3333-4333-8333-333333333333";

#[derive(Debug)]
struct Fixture {
    directory: PathBuf,
    selected: PathBuf,
    other: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let directory =
            std::env::temp_dir().join(format!("plannotator codex pid {tag} ü-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("fixture directory");
        let selected = Self::rollout(&directory, "01", SELECTED_ID, "SELECTED PANE");
        let other = Self::rollout(&directory, "02", OTHER_ID, "OTHER PANE");
        Self { directory, selected, other }
    }

    fn rollout(directory: &Path, day: &str, id: &str, text: &str) -> PathBuf {
        Self::write_rollout(directory, day, id, &[Self::assistant_message(id, text)])
    }

    /// A rollout Codex wrote for one of its subagents (a review, a guardian): the session
    /// meta on the first line names `source.subagent`.
    fn subagent_rollout(&self) -> PathBuf {
        let meta = serde_json::json!({
            "type": "session_meta",
            "payload": {"id": SUBAGENT_ID, "source": {"subagent": "review"}}
        });
        let message = Self::assistant_message(SUBAGENT_ID, "SUBAGENT PANE");
        Self::write_rollout(&self.directory, "03", SUBAGENT_ID, &[meta, message])
    }

    fn write_rollout(directory: &Path, day: &str, id: &str, lines: &[serde_json::Value]) -> PathBuf {
        let path =
            directory.join(format!("sessions/2026/09/{day}/rollout-2026-09-{day}T01-00-00-{id}.jsonl"));
        std::fs::create_dir_all(path.parent().expect("parent")).expect("session directory");
        let mut text = String::new();
        for line in lines {
            text.push_str(&line.to_string());
            text.push('\n');
        }
        std::fs::write(&path, text).expect("transcript");
        path
    }

    fn assistant_message(id: &str, text: &str) -> serde_json::Value {
        serde_json::json!({
            "type": "response_item",
            "payload": {"type": "message", "role": "assistant", "id": id,
                        "content": [{"type": "output_text", "text": text}]}
        })
    }

    fn command(&self, pid: u32) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_plannotator-tui"));
        command
            .env("CODEX_HOME", &self.directory)
            // An inherited id from another pane must not override the explicitly selected PID.
            .env("CODEX_THREAD_ID", OTHER_ID)
            .args(["last", "--host", "codex", "--pid", &pid.to_string(), "--print"]);
        command
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

#[derive(Debug)]
struct HoldingProcess(Child);

impl HoldingProcess {
    fn new(first: &Path, second: Option<&Path>) -> Self {
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "exec 3<\"$1\"; if [ -n \"${2:-}\" ]; then exec 4<\"$2\"; fi; printf 'ready\\n'; read -r line",
            "hold-transcripts",
        ]).arg(first);
        if let Some(path) = second {
            command.arg(path);
        }
        let mut process = Self(command.stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().expect("holder"));
        let mut ready = String::new();
        BufReader::new(process.0.stdout.take().expect("stdout")).read_line(&mut ready).expect("ready");
        assert_eq!(ready.trim(), "ready");
        process
    }
}

impl Drop for HoldingProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn pid_selects_its_transcript_over_newer_rollout_and_inherited_thread() {
    let fixture = Fixture::new("selected");
    let process = HoldingProcess::new(&fixture.selected, None);
    let out = fixture.command(process.0.id()).output().expect("runs");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "SELECTED PANE");
    assert!(out.stderr.is_empty(), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn duplicate_descriptors_do_not_make_one_transcript_ambiguous() {
    let fixture = Fixture::new("duplicates");
    let process = HoldingProcess::new(&fixture.selected, Some(&fixture.selected));
    let out = fixture.command(process.0.id()).output().expect("runs");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "SELECTED PANE");
    assert!(out.stderr.is_empty(), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn open_subagent_rollouts_do_not_make_one_transcript_ambiguous() {
    let fixture = Fixture::new("subagent");
    let subagent = fixture.subagent_rollout();
    let process = HoldingProcess::new(&fixture.selected, Some(&subagent));
    let out = fixture.command(process.0.id()).output().expect("runs");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "SELECTED PANE");
    assert!(out.stderr.is_empty(), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn ambiguous_pid_does_not_read_the_newest_session() {
    let fixture = Fixture::new("ambiguous");
    let process = HoldingProcess::new(&fixture.selected, Some(&fixture.other));
    let out = fixture.command(process.0.id()).output().expect("runs");
    assert!(out.status.success(), "--print reports discovery failures without aborting its caller");
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("2 open transcripts"));
}

#[test]
fn pid_without_an_open_transcript_does_not_read_another_session() {
    let fixture = Fixture::new("missing");
    let process = HoldingProcess::new(Path::new("/dev/null"), None);
    let out = fixture.command(process.0.id()).output().expect("runs");
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("0 open transcripts"));
}

#[test]
fn exact_session_id_still_precedes_pid_discovery() {
    let fixture = Fixture::new("exact");
    let process = HoldingProcess::new(&fixture.other, None);
    let out = fixture.command(process.0.id()).args(["--session-id", SELECTED_ID]).output().expect("runs");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "SELECTED PANE");
    assert!(out.stderr.is_empty(), "{}", String::from_utf8_lossy(&out.stderr));
}
