//! Antigravity CLI (`agy`): one directory per conversation under the app data directory
//! (`$ANTIGRAVITY_CLI_HOME`, default `~/.gemini/antigravity-cli`), its steps in
//! `brain/<conversation id>/.system_generated/logs/transcript_full.jsonl`.
//!
//! Each line is one step: `USER_INPUT` steps carry the prompt in `content`, `PLANNER_RESPONSE`
//! steps carry the reply in `content` (a step that only calls tools has none), and every other
//! type is a tool result or bookkeeping. `transcript.jsonl` next to it truncates long
//! `content`, so it is never read.
//!
//! A transcript does not record its folder. Two files do: `conversation_summaries.db`
//! (`SQLite`, `workspace_uris` as a JSON array of `file://` URIs per conversation) and
//! `history.jsonl` (one line per prompt, with `workspace` and, after the first prompt, the
//! `conversationId`). A conversation counts for a cwd only when one of them says so, so a
//! missing or unreadable database narrows the choice to what the history proves and never
//! widens it to the newest transcript of some other folder.

use std::io::BufRead as _;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::sqlite::{open_read_only, sql_error};
use crate::{HostError, Message, Role};

/// The app data directory relative to `$HOME` (`%USERPROFILE%` on Windows).
pub const DEFAULT_APP_DATA_DIR: &str = ".gemini/antigravity-cli";
/// Conversation id → workspace URIs, relative to the app data directory.
pub const SUMMARIES_DB: &str = "conversation_summaries.db";
/// One line per prompt, relative to the app data directory.
pub const HISTORY_FILE: &str = "history.jsonl";

const WHAT: &str = "Antigravity";

/// The untruncated transcript of `conversation_id`.
pub fn transcript_path(app_data: &Path, conversation_id: &str) -> PathBuf {
    app_data
        .join("brain")
        .join(conversation_id)
        .join(".system_generated")
        .join("logs")
        .join("transcript_full.jsonl")
}

/// The transcript of the conversation Herdr (or `--session-id`) named, when it exists.
pub fn find_transcript_by_id(app_data: &Path, conversation_id: &str) -> Result<Option<PathBuf>, HostError> {
    let id = crate::validate_session_id(conversation_id)?;
    let path = transcript_path(app_data, id);
    Ok(path.is_file().then_some(path))
}

/// The conversation id a transcript path carries: the `brain/<id>` directory above
/// `.system_generated/logs/`.
pub fn conversation_of(transcript: &Path) -> Option<String> {
    let logs = transcript.parent()?;
    let generated = logs.parent()?;
    (logs.file_name()? == "logs" && generated.file_name()? == ".system_generated")
        .then(|| generated.parent()?.file_name()?.to_str().map(str::to_owned))
        .flatten()
}

/// The conversation for `cwd` with the newest transcript that holds a message: one whose
/// workspace is `cwd`, else one whose workspace contains it. Workspaces come from the summaries
/// database and the prompt history together; when the database cannot be read the error says
/// so, and the history alone decides.
pub fn find_transcript(app_data: &Path, cwd: &Path) -> Result<PathBuf, HostError> {
    let wanted = normalize(&cwd.to_string_lossy());
    let db = app_data.join(SUMMARIES_DB);
    let history = app_data.join(HISTORY_FILE);
    let (mut workspaces, db_error) = match summaries(&db) {
        Ok(rows) => (rows, None),
        Err(err) => (Vec::new(), Some(err)),
    };
    workspaces.extend(history_workspaces(&history));
    let newest = |matches: &dyn Fn(&str) -> bool| {
        let ids = workspaces.iter().filter(|(_, workspace)| matches(workspace)).map(|(id, _)| id.as_str());
        newest_with_messages(app_data, ids)
    };
    if let Some(found) =
        newest(&|workspace| workspace == wanted).or_else(|| newest(&|w| is_ancestor(w, &wanted)))
    {
        return Ok(found);
    }
    let unread = db_error.map(|err| format!("; {err}")).unwrap_or_default();
    Err(HostError::NoTranscript(format!(
        "no Antigravity conversation started in {} (looked in {} and {}{unread})",
        cwd.display(),
        db.display(),
        history.display()
    )))
}

/// `(conversation id, workspace)` for every top-level conversation in the summaries database.
/// Subagent conversations (a non-empty `parent_conversation_id`, when the column exists) are
/// left out: their replies are their parent's tool results, not the agent's.
fn summaries(db: &Path) -> Result<Vec<(String, String)>, HostError> {
    let connection = open_read_only(db, WHAT)?;
    let has_parent: bool = connection
        .query_row(
            "SELECT count(*) FROM pragma_table_info('conversation_summaries') \
             WHERE name = 'parent_conversation_id'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|e| sql_error(WHAT, &e))?
        > 0;
    let filter = if has_parent { " WHERE coalesce(parent_conversation_id, '') = ''" } else { "" };
    let mut statement = connection
        .prepare(&format!("SELECT conversation_id, workspace_uris FROM conversation_summaries{filter}"))
        .map_err(|e| sql_error(WHAT, &e))?;
    let rows = statement
        .query_map([], |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?)))
        .map_err(|e| sql_error(WHAT, &e))?;
    let mut out = Vec::new();
    for row in rows {
        let (Some(id), Some(uris)) = row.map_err(|e| sql_error(WHAT, &e))? else { continue };
        let Ok(Value::Array(uris)) = serde_json::from_str::<Value>(&uris) else { continue };
        for uri in uris.iter().filter_map(Value::as_str).filter_map(path_of_file_uri) {
            out.push((id.clone(), normalize(&uri)));
        }
    }
    Ok(out)
}

/// `(conversation id, workspace)` for every history line that names both.
fn history_workspaces(history: &Path) -> Vec<(String, String)> {
    let Ok(file) = std::fs::File::open(history) else { return Vec::new() };
    std::io::BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| {
            let entry: Value = serde_json::from_str(line.trim()).ok()?;
            let id = entry.get("conversationId")?.as_str()?;
            let workspace = entry.get("workspace")?.as_str()?;
            Some((id.to_owned(), normalize(workspace)))
        })
        .collect()
}

/// The last-written transcript among `ids` that holds at least one message. Ids that are not
/// one safe path component are skipped, whatever wrote them.
fn newest_with_messages<'a>(app_data: &Path, ids: impl Iterator<Item = &'a str>) -> Option<PathBuf> {
    let mut ranked: Vec<(Option<std::time::SystemTime>, PathBuf)> = ids
        .filter(|id| crate::validate_session_id(id).is_ok())
        .map(|id| transcript_path(app_data, id))
        .filter(|path| path.is_file())
        .map(|path| (std::fs::metadata(&path).and_then(|data| data.modified()).ok(), path))
        .collect();
    ranked.sort_by(|(a_time, a), (b_time, b)| b_time.cmp(a_time).then_with(|| a.cmp(b)));
    ranked.dedup_by(|(_, a), (_, b)| a == b);
    ranked
        .into_iter()
        .map(|(_, path)| path)
        .find(|path| std::fs::read_to_string(path).is_ok_and(|text| !parse_messages(&text, 1).is_empty()))
}

/// The newest `n` prompts and replies, newest first. A reply is a `PLANNER_RESPONSE` step's
/// `content` as written; a prompt is a `USER_INPUT` step's `content`, wrappers and all (only
/// replies are shown). A line that does not parse, such as one still being written, is
/// skipped.
pub fn parse_messages(jsonl: &str, n: usize) -> Vec<Message> {
    jsonl.lines().rev().filter_map(step_message).take(n).collect()
}

fn step_message(line: &str) -> Option<Message> {
    let step: Value = serde_json::from_str(line.trim()).ok()?;
    let role = match step.get("type")?.as_str()? {
        "PLANNER_RESPONSE" => Role::Assistant,
        "USER_INPUT" => Role::Human,
        _ => return None,
    };
    let text = step.get("content")?.as_str().filter(|text| !text.trim().is_empty())?;
    Some(Message {
        id: step.get("step_index").and_then(Value::as_u64).map(|index| index.to_string()).unwrap_or_default(),
        role,
        text: text.to_owned(),
        at: step.get("created_at").and_then(Value::as_str).map(str::to_owned),
    })
}

/// The local path of a `file://` URI, percent-decoded: `file:///w/a%20b` → `/w/a b`, and the
/// Windows forms `file:///C:/w` and `file:///c%3A/w` → `C:/w` and `c:/w`. Other schemes and
/// remote hosts are not local paths.
pub fn path_of_file_uri(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let slash = rest.find('/')?;
    if !matches!(rest.get(..slash)?, "" | "localhost") {
        return None;
    }
    let path = percent_decode(rest.get(slash..)?);
    match path.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => path.get(1..).map(str::to_owned),
        _ => Some(path),
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&byte) = bytes.get(i) {
        let escaped = (byte == b'%')
            .then(|| text.get(i + 1..i + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        if let Some(decoded) = escaped {
            out.push(decoded);
            i += 3;
        } else {
            out.push(byte);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Paths compare without trailing separators; on Windows also up to case and slash direction.
fn normalize(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    if cfg!(windows) { trimmed.replace('\\', "/").to_lowercase() } else { trimmed.to_owned() }
}

fn is_ancestor(workspace: &str, cwd: &str) -> bool {
    !workspace.is_empty() && cwd.starts_with(workspace) && cwd.as_bytes().get(workspace.len()) == Some(&b'/')
}
