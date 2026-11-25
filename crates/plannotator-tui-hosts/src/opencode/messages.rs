//! The message readers: `OpenCode` 1 (`message` + `part`) and `OpenCode` 2 (`session_message`,
//! whose payload may hold the text or, since 2.4, leave it to `session_part` rows).

use std::path::Path;

use rusqlite::params;
use serde_json::Value;

use super::parts::Parts;
use super::payload::{json_text, part_text};
use super::{WHAT, replies};
use crate::sqlite::{open_read_only, sql_error};
use crate::{HostError, Message, Role};

/// The user and assistant messages of `session_id` that carry text, newest first, through the
/// `n`th newest assistant reply.
pub fn messages_for_session(
    db: &Path,
    session_id: &str,
    schema: super::Schema,
    n: usize,
) -> Result<Vec<Message>, HostError> {
    crate::validate_session_id(session_id)?;
    match schema {
        super::Schema::V1 => messages_v1(db, session_id, n),
        super::Schema::V2 => messages_v2(db, session_id, n),
    }
}

fn messages_v1(db: &Path, session_id: &str, n: usize) -> Result<Vec<Message>, HostError> {
    let connection = open_read_only(db, WHAT)?;
    // One indexed pass: every text part of the session, grouped by message, newest message
    // first and parts in creation order within it.
    let mut statement = connection
        .prepare(
            "SELECT m.id, m.data, p.data FROM message m \
             JOIN part p ON p.message_id = m.id \
             WHERE m.session_id = ?1 AND json_extract(p.data, '$.type') = 'text' \
             ORDER BY m.time_created DESC, m.id DESC, p.time_created ASC, p.id ASC",
        )
        .map_err(|e| sql_error(WHAT, &e))?;
    let rows = statement
        .query_map(params![session_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })
        .map_err(|e| sql_error(WHAT, &e))?;

    let mut messages: Vec<Message> = Vec::new();
    let mut current: Option<(String, Role, Option<String>, Vec<String>)> = None;
    let flush = |current: &mut Option<(String, Role, Option<String>, Vec<String>)>,
                 out: &mut Vec<Message>| {
        if let Some((id, role, at, parts)) = current.take()
            && !parts.is_empty()
        {
            out.push(Message { id, role, text: parts.join("\n\n"), at });
        }
    };
    for row in rows {
        let (id, message_data, part_data) = row.map_err(|e| sql_error(WHAT, &e))?;
        if current.as_ref().is_none_or(|c| c.0 != id) {
            flush(&mut current, &mut messages);
            if replies(&messages) >= n {
                break;
            }
            let Some((role, at)) = message_meta(&message_data) else { continue };
            current = Some((id, role, at, Vec::new()));
        }
        if let Some((_, _, _, parts)) = current.as_mut()
            && let Some(text) = part_text(&part_data)
        {
            parts.push(text);
        }
    }
    if replies(&messages) < n {
        flush(&mut current, &mut messages);
    }
    empty_is_an_error(messages, db, session_id)
}

/// `OpenCode` 2: one `session_message` row per message; the payload carries the text directly,
/// or, since 2.4 writes each part as its own row, in the message's `session_part` rows.
fn messages_v2(db: &Path, session_id: &str, n: usize) -> Result<Vec<Message>, HostError> {
    let connection = open_read_only(db, WHAT)?;
    // The part table arrived in 2.4; a database written before it has none, and the message
    // payload is then the only source. The part query is prepared on the first reply without
    // payload text and then serves every such reply.
    let mut parts = Parts::new(&connection, db, session_id)?;
    let mut statement = connection
        .prepare(
            "SELECT id, type, data, time_created FROM session_message \
             WHERE session_id = ?1 AND type IN ('user', 'assistant') \
             ORDER BY seq DESC",
        )
        .map_err(|e| sql_error(WHAT, &e))?;
    let rows = statement
        .query_map(params![session_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(|e| sql_error(WHAT, &e))?;
    let mut messages = Vec::new();
    for row in rows {
        if replies(&messages) >= n {
            break;
        }
        let (id, kind, data, created) = row.map_err(|e| sql_error(WHAT, &e))?;
        // A message payload that will not parse is skipped, as it always has been: it was not
        // selected for its parts, and one bad row must not hide the rest of the session.
        let Ok(json) = serde_json::from_str::<Value>(&data) else { continue };
        let (role, text) = match kind.as_str() {
            "assistant" => (Role::Assistant, assistant_text(&json, &id, &mut parts)?),
            "user" => (Role::Human, json.get("text").and_then(Value::as_str).unwrap_or_default().to_owned()),
            _ => continue,
        };
        if text.trim().is_empty() {
            continue;
        }
        let at = json
            .pointer("/time/created")
            .and_then(Value::as_u64)
            .or_else(|| u64::try_from(created).ok())
            .map(crate::time::iso_from_unix_ms);
        messages.push(Message { id, role, text, at });
    }
    empty_is_an_error(messages, db, session_id)
}

/// A v2 assistant reply's text. The format is decided per reply: text in the payload wins, and
/// only a reply without visible text falls back to its parts. Never both, never repeated.
fn assistant_text(json: &Value, id: &str, parts: &mut Parts<'_>) -> Result<String, HostError> {
    let payload = json
        .get("content")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(json_text).collect::<Vec<_>>().join("\n\n"))
        .unwrap_or_default();
    if !payload.trim().is_empty() {
        return Ok(payload);
    }
    Ok(parts.text_for(id)?.unwrap_or(payload))
}

fn empty_is_an_error(messages: Vec<Message>, db: &Path, session_id: &str) -> Result<Vec<Message>, HostError> {
    if messages.is_empty() {
        return Err(HostError::NoMessages(format!(
            "no messages for OpenCode session {session_id} in {}",
            db.display()
        )));
    }
    Ok(messages)
}

/// Role and creation time from a `message.data` payload; `None` for roles we do not render.
fn message_meta(data: &str) -> Option<(Role, Option<String>)> {
    let json: Value = serde_json::from_str(data).ok()?;
    let role = match json.get("role").and_then(|r| r.as_str())? {
        "assistant" => Role::Assistant,
        "user" => Role::Human,
        _ => return None,
    };
    let at = json.pointer("/time/created").and_then(Value::as_u64).map(crate::time::iso_from_unix_ms);
    Some((role, at))
}
