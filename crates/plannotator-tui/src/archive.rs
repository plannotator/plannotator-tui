//! The shared feedback archive: one submitted-feedback index that Plannotator and this
//! TUI both write, distinguished by the `client` field.
//!
//! Contract: `packages/shared/feedback-archive.ts` in Plannotator (schema v1, frozen at
//! `443e1fcf`). One JSON object per line appended to
//! `{data_dir}/feedback/{project}/index.jsonl`, plus a human-readable markdown sidecar
//! under `records/` for content-bearing records, named with a `-plannotator-tui` suffix.
//!
//! Concurrency: both writers use lock-free `O_APPEND` whole-line writes. The line is
//! handed to ONE `write` call — never `write_all`, which loops on partial writes and
//! could splice bytes between another writer's — retried only on `Interrupted` (no bytes
//! written). After a partial write the rest is never sent; a lone `\n` is appended
//! best-effort to terminate the torn fragment (readers skip blank and unparsable lines)
//! and the record is reported as not archived.
//!
//! Archiving never fails a send: the public entry point swallows every error, because a
//! full disk must not turn a reviewer's submit into a failure (the annotations remain in
//! the annotation store either way).

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::store::civil_from_days;

/// The `client` label this writer stamps on every record. `herdr-annotate` is reserved
/// for the Lite plugin should it ever submit; never change either.
pub(crate) const CLIENT: &str = "plannotator-tui";

const RECORD_VERSION: u32 = 1;

/// Plannotator's host spellings for the `origin` and `target.agent.host` fields
/// (`PLANNOTATOR_ORIGIN` value family). Our labels differ for three hosts.
pub(crate) fn origin_label(host: &str) -> &str {
    match host {
        "claude" => "claude-code",
        "copilot" => "copilot-cli",
        "omp" => "oh-my-pi",
        other => other,
    }
}

/// The archive knob, shared with Plannotator so one setting governs both writers:
/// `PLANNOTATOR_FEEDBACK_HISTORY` env, else `feedbackHistory` in the data dir's
/// `config.json`, else on. Env semantics match `resolveFeedbackHistory`: when set,
/// only `"1"` and (case-insensitive) `"true"` enable.
pub(crate) fn enabled(env: impl Fn(&str) -> Option<String>, data_dir: &Path) -> bool {
    if let Some(value) = env("PLANNOTATOR_FEEDBACK_HISTORY") {
        return value == "1" || value.to_lowercase() == "true";
    }
    let Ok(text) = std::fs::read_to_string(data_dir.join("config.json")) else { return true };
    let Ok(config) = serde_json::from_str::<serde_json::Value>(&text) else { return true };
    match config.get("feedbackHistory") {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::String(s)) => !matches!(s.as_str(), "0" | "false"),
        _ => true,
    }
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentTarget {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) session: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) transcript: Option<String>,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Target {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) file_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) agent: Option<AgentTarget>,
}

impl Target {
    pub(crate) fn file(path: &Path) -> Self {
        Self { file_path: Some(path.display().to_string()), ..Self::default() }
    }

    pub(crate) fn agent(host: &str, session: Option<String>, transcript: Option<String>) -> Self {
        Self {
            agent: Some(AgentTarget { host: Some(host.to_owned()), session, transcript }),
            ..Self::default()
        }
    }

    fn is_empty(&self) -> bool {
        self.file_path.is_none() && self.agent.is_none()
    }
}

/// One submitted annotation, in the archive's lenient shallow shape.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnnotationRecord {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) id: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub(crate) kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) original_text: Option<String>,
}

#[derive(Serialize)]
struct Counts {
    annotations: usize,
    external: usize,
    images: usize,
}

/// The index line. Field order mirrors Plannotator's construction order; `recordFile`
/// stays last because it is decided after the sidecar exists.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
// The field is `recordFile` in the v1 contract; the prefix collision with `Record` is the
// contract's, not a naming choice here.
#[allow(clippy::struct_field_names)]
struct Record<'a> {
    v: u32,
    ts: String,
    client: &'static str,
    client_version: &'static str,
    project: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    origin: Option<&'a str>,
    surface: &'a str,
    decision: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<&'a Target>,
    #[serde(skip_serializing_if = "str::is_empty")]
    feedback: &'a str,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    annotations: &'a [AnnotationRecord],
    counts: Counts,
    #[serde(skip_serializing_if = "Option::is_none")]
    record_file: Option<String>,
}

pub(crate) struct Submission<'a> {
    pub(crate) data_dir: &'a Path,
    pub(crate) project: &'a str,
    /// "annotate" | "annotate-folder" | "annotate-last".
    pub(crate) surface: &'a str,
    /// Receiving agent host in Plannotator's spelling; absent for clipboard delivery.
    pub(crate) origin: Option<String>,
    pub(crate) target: Target,
    pub(crate) feedback: &'a str,
    /// Per-annotation records; empty in folder mode, where `count` still carries the total.
    pub(crate) annotations: Vec<AnnotationRecord>,
    pub(crate) count: usize,
    /// Milliseconds since the epoch; tests pin it, callers pass `None` for now.
    pub(crate) now_ms: Option<u128>,
}

/// Archive one submission. Never fails the caller; `None` means nothing durable was
/// written (the annotation store remains the recovery copy, matching Plannotator's
/// keep-the-draft contract).
pub(crate) fn append(submission: &Submission<'_>) -> Option<PathBuf> {
    try_append(submission).ok()
}

fn try_append(submission: &Submission<'_>) -> std::io::Result<PathBuf> {
    let now_ms = submission
        .now_ms
        .unwrap_or_else(|| SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis()));
    let ts = iso_millis(now_ms);
    let project_dir = submission.data_dir.join("feedback").join(submission.project);
    std::fs::create_dir_all(&project_dir)?;

    let mut record = Record {
        v: RECORD_VERSION,
        ts: ts.clone(),
        client: CLIENT,
        client_version: env!("CARGO_PKG_VERSION"),
        project: submission.project,
        origin: submission.origin.as_deref(),
        surface: submission.surface,
        decision: "feedback",
        target: (!submission.target.is_empty()).then_some(&submission.target),
        feedback: submission.feedback,
        annotations: &submission.annotations,
        counts: Counts { annotations: submission.count, external: 0, images: 0 },
        record_file: None,
    };

    let has_content = !submission.feedback.trim().is_empty() || !submission.annotations.is_empty();
    if has_content {
        record.record_file = Some(write_sidecar(&project_dir, submission, &ts)?);
    }

    let index = project_dir.join("index.jsonl");
    let line =
        format!("{}\n", serde_json::to_string(&record).map_err(|e| std::io::Error::other(e.to_string()))?);
    append_line(&index, line.as_bytes())?;
    Ok(index)
}

/// Write the markdown sidecar with an exclusive create, bumping a counter on collision,
/// and return its project-relative path. Written before the index line so a `recordFile`
/// can never name a missing file; an orphan sidecar after a failed append is harmless.
fn write_sidecar(project_dir: &Path, submission: &Submission<'_>, ts: &str) -> std::io::Result<String> {
    let records_dir = project_dir.join("records");
    std::fs::create_dir_all(&records_dir)?;
    let base = format!("{}-{}-feedback-{CLIENT}", ts.replace([':', '.'], "-"), submission.surface);
    let body = render_markdown(submission, ts);
    for n in 1..=100u32 {
        let name = if n == 1 { format!("{base}.md") } else { format!("{base}-{n}.md") };
        match std::fs::OpenOptions::new().write(true).create_new(true).open(records_dir.join(&name)) {
            Ok(mut file) => {
                file.write_all(body.as_bytes())?;
                return Ok(format!("records/{name}"));
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(err),
        }
    }
    Err(std::io::Error::other(format!(
        "100 sidecar name collisions for {base}: clock stuck or a runaway writer"
    )))
}

/// Append one whole line with a single `write` call under `O_APPEND`.
fn append_line(path: &Path, line: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    loop {
        match file.write(line) {
            Ok(n) if n == line.len() => return Ok(()),
            Ok(0) => return Err(std::io::ErrorKind::WriteZero.into()),
            Ok(_) => {
                // Torn line: never send the remainder (it could splice into a concurrent
                // writer's line). Terminate the fragment so the file heals for the next
                // record; at worst this leaves a blank line, which readers skip.
                let _ = file.write(b"\n");
                return Err(std::io::Error::other("partial append; record not archived"));
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
            Err(err) => return Err(err),
        }
    }
}

fn render_markdown(submission: &Submission<'_>, ts: &str) -> String {
    let title = match submission.surface {
        "annotate-folder" => "Annotate feedback (folder)",
        "annotate-last" => "Annotate feedback (agent message)",
        _ => "Annotate feedback",
    };
    let mut lines = vec![format!("# {title}"), String::new()];
    lines.push(format!("- Submitted: {ts}"));
    lines.push(format!("- Surface: {}", submission.surface));
    lines.push("- Decision: feedback".to_owned());
    lines.push(format!("- Project: {}", submission.project));
    if let Some(origin) = &submission.origin {
        lines.push(format!("- Origin: {origin}"));
    }
    if let Some(path) = &submission.target.file_path {
        lines.push(format!("- File: {path}"));
    }
    if let Some(agent) = &submission.target.agent {
        let mut bits = Vec::new();
        if let Some(host) = &agent.host {
            bits.push(host.clone());
        }
        if let Some(session) = &agent.session {
            bits.push(format!("session {session}"));
        }
        if !bits.is_empty() {
            lines.push(format!("- Agent: {}", bits.join(", ")));
        }
        if let Some(transcript) = &agent.transcript {
            lines.push(format!("- Transcript: {transcript}"));
        }
    }
    lines.push(format!("- Annotations: {}", submission.count));
    lines.extend([String::new(), "---".to_owned(), String::new()]);
    let feedback = submission.feedback.trim();
    lines.push(if feedback.is_empty() {
        "_No feedback text submitted._".to_owned()
    } else {
        submission.feedback.to_owned()
    });
    lines.push(String::new());
    lines.join("\n")
}

/// `Date.prototype.toISOString` shape: UTC with milliseconds.
fn iso_millis(now_ms: u128) -> String {
    let secs = (now_ms / 1000) as u64;
    let millis = (now_ms % 1000) as u64;
    let (year, month, day) = civil_from_days(secs / 86_400);
    let (hour, minute, second) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::unwrap_used,
    reason = "tests assert by panicking"
)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("plannotator-archive-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn submission<'a>(data_dir: &'a Path, feedback: &'a str) -> Submission<'a> {
        Submission {
            data_dir,
            project: "demo",
            surface: "annotate",
            origin: Some("claude-code".to_owned()),
            target: Target::file(Path::new("/work/plan.md")),
            feedback,
            annotations: vec![AnnotationRecord {
                id: Some("a1".to_owned()),
                kind: Some("comment".to_owned()),
                text: Some("tighten this".to_owned()),
                original_text: Some("the selected text".to_owned()),
            }],
            count: 1,
            now_ms: Some(1_788_242_400_123),
        }
    }

    #[test]
    fn the_index_line_matches_the_v1_contract_shape() {
        let d = dir("shape");
        let index = try_append(&submission(&d, "1. tighten this")).unwrap();
        let line = std::fs::read_to_string(index).unwrap();
        assert!(line.ends_with('\n'));
        let expected = format!(
            "{{\"v\":1,\"ts\":\"2026-09-01T06:00:00.123Z\",\"client\":\"plannotator-tui\",\
             \"clientVersion\":\"{}\",\"project\":\"demo\",\"origin\":\"claude-code\",\
             \"surface\":\"annotate\",\"decision\":\"feedback\",\
             \"target\":{{\"filePath\":\"/work/plan.md\"}},\"feedback\":\"1. tighten this\",\
             \"annotations\":[{{\"id\":\"a1\",\"type\":\"comment\",\"text\":\"tighten this\",\
             \"originalText\":\"the selected text\"}}],\
             \"counts\":{{\"annotations\":1,\"external\":0,\"images\":0}},\
             \"recordFile\":\"records/2026-09-01T06-00-00-123Z-annotate-feedback-plannotator-tui.md\"}}\n",
            env!("CARGO_PKG_VERSION"),
        );
        assert_eq!(line, expected);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_foreign_reader_sees_valid_json_with_a_numeric_v() {
        let d = dir("foreign");
        let index = try_append(&submission(&d, "text")).unwrap();
        let line = std::fs::read_to_string(index).unwrap();
        let value: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(value["v"], 1);
        assert_eq!(value["client"], "plannotator-tui");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn same_millisecond_submissions_bump_the_sidecar_counter() {
        let d = dir("collide");
        try_append(&submission(&d, "one")).unwrap();
        let index = try_append(&submission(&d, "two")).unwrap();
        let lines = std::fs::read_to_string(index).unwrap();
        assert_eq!(lines.lines().count(), 2);
        let second: serde_json::Value = serde_json::from_str(lines.lines().nth(1).unwrap()).unwrap();
        assert_eq!(
            second["recordFile"],
            "records/2026-09-01T06-00-00-123Z-annotate-feedback-plannotator-tui-2.md"
        );
        for line in lines.lines() {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            let sidecar = d.join("feedback/demo").join(value["recordFile"].as_str().unwrap());
            assert!(sidecar.is_file(), "{} missing", sidecar.display());
        }
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_sidecar_is_skipped_for_a_record_without_content() {
        let d = dir("bare");
        let mut sub = submission(&d, "");
        sub.annotations = Vec::new();
        sub.count = 0;
        let index = try_append(&sub).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(std::fs::read_to_string(index).unwrap().trim()).unwrap();
        assert!(value.get("recordFile").is_none());
        assert!(value.get("feedback").is_none());
        assert!(!d.join("feedback/demo/records").exists());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_sidecar_carries_the_feedback_the_agent_received() {
        let d = dir("sidecar");
        try_append(&submission(&d, "1. tighten this")).unwrap();
        let body = std::fs::read_to_string(
            d.join("feedback/demo/records/2026-09-01T06-00-00-123Z-annotate-feedback-plannotator-tui.md"),
        )
        .unwrap();
        assert!(body.starts_with("# Annotate feedback\n"));
        assert!(body.contains("- Origin: claude-code"));
        assert!(body.contains("- File: /work/plan.md"));
        assert!(body.ends_with("---\n\n1. tighten this\n"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_env_knob_follows_plannotators_resolver_semantics() {
        let d = dir("knob");
        let with = |v: &'static str| move |_: &str| Some(v.to_owned());
        assert!(enabled(with("1"), &d));
        assert!(enabled(with("true"), &d));
        assert!(enabled(with("TRUE"), &d));
        assert!(!enabled(with("0"), &d));
        assert!(!enabled(with("yes"), &d));
        assert!(!enabled(with(""), &d));
        assert!(enabled(|_| None, &d));
        std::fs::write(d.join("config.json"), r#"{"feedbackHistory": false}"#).unwrap();
        assert!(!enabled(|_| None, &d));
        assert!(enabled(with("1"), &d), "env wins over config");
        std::fs::write(d.join("config.json"), r#"{"feedbackHistory": "0"}"#).unwrap();
        assert!(!enabled(|_| None, &d));
        std::fs::write(d.join("config.json"), "not json").unwrap();
        assert!(enabled(|_| None, &d), "unreadable config falls back to on");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn host_labels_map_to_plannotators_spellings() {
        assert_eq!(origin_label("claude"), "claude-code");
        assert_eq!(origin_label("copilot"), "copilot-cli");
        assert_eq!(origin_label("omp"), "oh-my-pi");
        for unchanged in ["codex", "opencode", "pi", "droid", "hermes"] {
            assert_eq!(origin_label(unchanged), unchanged);
        }
    }

    #[test]
    fn an_agent_message_submission_records_agent_provenance() {
        let d = dir("agent");
        let sub = Submission {
            target: Target::agent("claude-code", Some("s-1".to_owned()), Some("/t.jsonl".to_owned())),
            surface: "annotate-last",
            ..submission(&d, "feedback")
        };
        let index = try_append(&sub).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(std::fs::read_to_string(index).unwrap().trim()).unwrap();
        assert_eq!(value["target"]["agent"]["host"], "claude-code");
        assert_eq!(value["target"]["agent"]["session"], "s-1");
        assert_eq!(value["target"]["agent"]["transcript"], "/t.jsonl");
        assert!(value["target"].get("filePath").is_none());
        let _ = std::fs::remove_dir_all(d);
    }
}
