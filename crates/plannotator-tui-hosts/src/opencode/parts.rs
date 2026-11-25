//! The `OpenCode` 2.4 part reader: the `session_part` statement is prepared on the first reply
//! that needs it and reused per message id, and a failure names the message it was reading.

use std::path::Path;

use rusqlite::{Connection, Statement, params};
use serde_json::Value;

use super::payload::{injected, json_text};
use crate::HostError;

const QUERY: &str = "SELECT data FROM session_part \
                     WHERE session_id = ?1 AND message_id = ?2 AND type = 'text' \
                     ORDER BY ordinal ASC";

/// Reads the text parts of one reply at a time, or nothing when the database predates the
/// part table. Owned names keep the borrow on the connection alone.
pub(super) struct Parts<'connection> {
    connection: &'connection Connection,
    /// Whether the database has a part table (written by `OpenCode` 2.4 or later).
    table: bool,
    /// Prepared on first use, so a session whose replies are all inline never depends on the
    /// part table's columns.
    statement: Option<Statement<'connection>>,
    database: String,
    session_id: String,
}

impl<'connection> Parts<'connection> {
    /// A reader for `session_id`, which yields nothing when the database has no part table.
    pub(super) fn new(
        connection: &'connection Connection,
        database: &Path,
        session_id: &str,
    ) -> Result<Self, HostError> {
        Ok(Self {
            connection,
            table: super::has_table(connection, "session_part")?,
            statement: None,
            database: database.display().to_string(),
            session_id: session_id.to_owned(),
        })
    }

    /// The visible text of `message_id`'s parts, joined in `ordinal` order, or `None` when it
    /// has no visible text there. A part row that will not parse is an error: it was selected
    /// for this reply, so silence would drop text the user is owed.
    pub(super) fn text_for(&mut self, message_id: &str) -> Result<Option<String>, HostError> {
        let Self { connection, table, statement, database, session_id } = self;
        if !*table {
            return Ok(None);
        }
        let error = |e: &dyn std::fmt::Display| part_error(session_id, database, message_id, e);
        let statement = match statement {
            Some(statement) => statement,
            None => statement.insert(connection.prepare(QUERY).map_err(|e| error(&e))?),
        };
        let rows = statement
            .query_map(params![session_id.as_str(), message_id], |row| row.get::<_, String>(0))
            .map_err(|e| error(&e))?;
        let mut texts = Vec::new();
        for row in rows {
            let data = row.map_err(|e| error(&e))?;
            let json: Value = serde_json::from_str(&data).map_err(|e| error(&e))?;
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
fn part_error(session_id: &str, database: &str, message_id: &str, err: &dyn std::fmt::Display) -> HostError {
    HostError::NoTranscript(format!(
        "reading the text parts of OpenCode session {session_id} message {message_id} in {database}: {err}"
    ))
}
