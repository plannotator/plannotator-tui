//! One transcript line, reduced to what rendering and branch-walking need.

use serde_json::Value;

/// Entry types that never render and never carry a prompt.
const SKIPPED_TYPES: [&str; 6] =
    ["progress", "system", "file-history-snapshot", "file-history-delta", "queue-operation", "attachment"];
/// Prefixes that mark a `user` entry as machinery rather than a typed prompt.
const MACHINE_PREFIXES: [&str; 5] = [
    "<local-command-",
    "<command-name>",
    "<system-reminder>",
    "<system-notification>",
    "<task-notification>",
];

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools, reason = "each flag mirrors one field of the transcript line")]
pub(crate) struct Entry {
    pub(crate) uuid: Option<String>,
    pub(crate) parent: Option<String>,
    pub(crate) kind: String,
    pub(crate) role: Option<String>,
    pub(crate) message_id: Option<String>,
    /// `text` blocks (or the string content), in order. Thinking and tool blocks are dropped.
    pub(crate) texts: Vec<String>,
    pub(crate) timestamp: Option<String>,
    pub(crate) is_meta: bool,
    pub(crate) is_sidechain: bool,
    pub(crate) hidden: bool,
    pub(crate) is_compact_summary: bool,
}

impl Entry {
    /// Lenient: a line that is not a JSON object is `None`, never an error.
    pub(crate) fn parse(line: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(line.trim()).ok()?;
        let object = value.as_object()?;
        let string = |key: &str| object.get(key).and_then(Value::as_str).map(str::to_owned);
        let flag = |key: &str| object.get(key).and_then(Value::as_bool).unwrap_or(false);
        let message = object.get("message").and_then(Value::as_object);
        let texts = match message.and_then(|m| m.get("content")) {
            Some(Value::String(text)) => vec![text.clone()],
            Some(Value::Array(blocks)) => blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .map(str::to_owned)
                .collect(),
            _ => Vec::new(),
        };
        let hidden = matches!(
            object.get("visibility").and_then(Value::as_str),
            Some("llm_only" | "assistant_only" | "hidden")
        );
        Some(Self {
            uuid: string("uuid"),
            parent: string("parentUuid"),
            kind: string("type").unwrap_or_default(),
            role: message.and_then(|m| m.get("role")).and_then(Value::as_str).map(str::to_owned),
            message_id: message.and_then(|m| m.get("id")).and_then(Value::as_str).map(str::to_owned),
            texts,
            timestamp: string("timestamp"),
            is_meta: flag("isMeta"),
            is_sidechain: flag("isSidechain"),
            hidden,
            is_compact_summary: flag("isCompactSummary"),
        })
    }

    /// The role this entry speaks in: `type`, else `message.role`.
    pub(crate) fn effective_role(&self) -> Option<&str> {
        match self.kind.as_str() {
            "user" | "assistant" => Some(self.kind.as_str()),
            _ => self.role.as_deref(),
        }
    }

    pub(crate) fn is_skipped(&self) -> bool {
        SKIPPED_TYPES.contains(&self.kind.as_str()) || self.hidden || self.is_sidechain
    }

    /// A typed human prompt with text: not meta, not a compact summary, not machinery.
    pub(crate) fn is_human_prompt(&self) -> bool {
        if self.is_skipped()
            || self.is_meta
            || self.is_compact_summary
            || self.effective_role() != Some("user")
        {
            return false;
        }
        let text = self.texts.join("\n");
        let trimmed = text.trim_start();
        !trimmed.is_empty() && !MACHINE_PREFIXES.iter().any(|p| trimmed.starts_with(p))
    }

    pub(crate) fn is_assistant_text(&self) -> bool {
        !self.is_skipped() && self.effective_role() == Some("assistant") && !self.texts.is_empty()
    }
}
