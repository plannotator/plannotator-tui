//! The `OpenCode` 2.4 part reader: the `session_part` statement is prepared once per read and
//! reused per message id, and a failure names the message it was reading.

use std::path::Path;

use rusqlite::{Connection, Statement, params};
use serde_json::Value;

use super::payload::{injected, json_text};
use crate::HostError;

/// Reads the text parts of one reply at a time, or nothing when the database predates the
/// part table. Owned names keep the borrow on the connection alone.
pub(super) struct Parts<'connection> {
    statement: Option<Statement<'connection>>,
    database: String,
    session_id: String,
}

impl<'connection> Parts<'connection> {
    /// Prepare the reader for `session_id`, or a reader that yields nothing when the database
    /// has no part table (written before `OpenCode` 2.4).
    pub(super) fn new(
        connection: &'connection Connection,
        database: &Path,
        session_id: &str,
    ) -> Result<Self, HostError> {
        let session_id = session_id.to_owned();
        let database = database.display().to_string();
        let statement = if super::has_table(connection, "session_part")? {
            Some(
                connection
                    .prepare(
                        "SELECT data FROM session_part \
                         WHERE session_id = ?1 AND message_id = ?2 AND type = 'text' \
                         ORDER BY ordinal ASC",
                    )
                    .map_err(|e| part_error(&session_id, &database, None, &e))?,
            )
        } else {
            None
        };
        Ok(Self { statement, database, session_id })
    }

    /// The visible text of `message_id`'s parts, joined in `ordinal` order, or `None` when it
    /// has no visible text there. A part row that will not parse is an error: it was selected
    /// for this reply, so silence would drop text the user is owed.
    pub(super) fn text_for(&mut self, message_id: &str) -> Result<Option<String>, HostError> {
        let Self { statement, database, session_id } = self;
        let Some(statement) = statement.as_mut() else { return Ok(None) };
        let rows = statement
            .query_map(params![session_id.as_str(), message_id], |row| row.get::<_, String>(0))
            .map_err(|e| part_error(session_id, database, Some(message_id), &e))?;
        let mut texts = Vec::new();
        for row in rows {
            let data = row.map_err(|e| part_error(session_id, database, Some(message_id), &e))?;
            let json: Value = serde_json::from_str(&data)
                .map_err(|e| part_error(session_id, database, Some(message_id), &e))?;
            if let Some(text) = json_text(&json)
                && !injected(&json)
            {
                texts.push(text.to_owned());
            }
        }
        Ok((!texts.is_empty()).then(|| texts.join("\n\n")))
    }
}

/// A part read failure: names the operation, the session and message, and the database.
fn part_error(
    session_id: &str,
    database: &str,
    message_id: Option<&str>,
    err: &impl std::fmt::Display,
) -> HostError {
    let reply = message_id.map(|id| format!(" message {id}")).unwrap_or_default();
    HostError::NoTranscript(format!(
        "reading the text parts of OpenCode session {session_id}{reply} in {database}: {err}"
    ))
}
