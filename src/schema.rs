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
    #[serde(default)]
    pub message: Option<Message>,
}

#[derive(Debug, Deserialize)]
pub struct Message {
    /// Either a plain string (user prompts) or an array of content blocks.
    #[serde(default)]
    pub content: Value,
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
