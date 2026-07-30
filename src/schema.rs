//! Raw session-record types for Claude Code JSONL logs.
//!
//! Parsing is tolerant by design: unknown fields are ignored, unknown record
//! types are skipped (and counted) upstream, and `message.content` stays a
//! `serde_json::Value` so schema drift in content blocks never fails a whole
//! record.

use serde::Deserialize;
use serde_json::Value;

/// One line of the session JSONL, envelope only.
#[derive(Debug, Deserialize)]
pub struct Record {
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub uuid: Option<String>,
    #[serde(rename = "parentUuid", default)]
    pub parent_uuid: Option<String>,
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(rename = "gitBranch", default)]
    pub git_branch: Option<String>,
    /// The API request that produced this record. A single request is
    /// streamed as several JSONL records sharing this id — token usage
    /// must be deduplicated on it (see `extract`).
    #[serde(rename = "requestId", default)]
    pub request_id: Option<String>,
    /// Reasoning-effort level for the turn (`low`..`max`), a top-level field on
    /// *some* assistant records — newer logs only, so it is sparse and may be
    /// absent for a whole session. Captured best-effort, never assumed complete.
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub message: Option<Message>,
}

#[derive(Debug, Deserialize)]
pub struct Message {
    /// The assistant message id — also stable across a request's streamed
    /// records; paired with `requestId` it keys token deduplication.
    #[serde(default)]
    pub id: Option<String>,
    /// The model that produced this assistant turn (e.g. `claude-opus-4-8`).
    /// Present on effectively every assistant record, so a mid-session model
    /// switch is captured turn-by-turn. `<synthetic>` marks harness-injected
    /// messages, filtered out in `extract`.
    #[serde(default)]
    pub model: Option<String>,
    /// Either a plain string (user prompts) or an array of content blocks.
    #[serde(default)]
    pub content: Value,
    /// Token usage for the assistant turn. Streaming records repeat this
    /// with monotonically increasing values, so the final (max) per
    /// request is the real count.
    #[serde(default)]
    pub usage: Option<Usage>,
}

/// Token counts from an assistant record's `message.usage`.
#[derive(Debug, Default, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
}

/// A `tool_use` content block, extracted from an assistant record.
#[derive(Debug, Deserialize)]
pub struct ToolUse {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub input: Value,
}

/// A `tool_result` content block, extracted from a user record.
#[derive(Debug, Deserialize)]
pub struct ToolResult {
    pub tool_use_id: String,
    #[serde(default)]
    pub is_error: Option<bool>,
    #[serde(default)]
    pub content: Value,
}

impl ToolResult {
    /// Flatten result content (string or blocks) to text, best effort.
    pub fn text(&self) -> String {
        match &self.content {
            Value::String(s) => s.clone(),
            Value::Array(blocks) => blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        }
    }
}

impl Record {
    /// Iterate the message content blocks, if content is an array.
    pub fn content_blocks(&self) -> impl Iterator<Item = &Value> {
        self.message
            .as_ref()
            .and_then(|m| m.content.as_array())
            .map(|a| a.iter())
            .into_iter()
            .flatten()
    }

    pub fn tool_uses(&self) -> Vec<ToolUse> {
        self.content_blocks()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
            .filter_map(|b| serde_json::from_value(b.clone()).ok())
            .collect()
    }

    pub fn tool_results(&self) -> Vec<ToolResult> {
        self.content_blocks()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
            .filter_map(|b| serde_json::from_value(b.clone()).ok())
            .collect()
    }
}
