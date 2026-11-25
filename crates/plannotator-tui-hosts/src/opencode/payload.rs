//! Text out of an `OpenCode` `data` payload. One set of rules decides what is reply text,
//! whether the block came from an `OpenCode` 1 `part` row, an `OpenCode` 2 message payload or a
//! 2.4 `session_part` row: type `text`, not blank, not injected context.

use serde_json::Value;

/// The text of a JSON block that is real reply text: type `text` and not blank.
pub(super) fn json_text(json: &Value) -> Option<&str> {
    if json.get("type").and_then(Value::as_str) != Some("text") {
        return None;
    }
    json.get("text").and_then(Value::as_str).filter(|text| !text.trim().is_empty())
}

/// Is the block injected context rather than something the user or the model wrote?
pub(super) fn injected(json: &Value) -> bool {
    let flagged = |key: &str| json.get(key).and_then(Value::as_bool).unwrap_or(false);
    flagged("synthetic") || flagged("ignored")
}

/// The text of an `OpenCode` 1 `part.data` payload when it is real text.
pub(super) fn part_text(data: &str) -> Option<String> {
    let json: Value = serde_json::from_str(data).ok()?;
    let text = json_text(&json)?;
    (!injected(&json)).then(|| text.to_owned())
}
