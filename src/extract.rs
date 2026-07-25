//! Stage 3: extract claims from the ordered records.
//!
//! A *claim* is anything the agent asserted it did to the world: a file
//! mutation (whose exact content is in the log) or a shell command (whose
//! effects are only as good as the receipt that came back). Tool results are
//! linked back to their claims by tool_use_id — they are the receipts.

use std::collections::HashMap;
use std::fmt;

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::schema::{Record, Usage};

/// How far a claim can reach. Ordered: later variants reach further.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Radius {
    LocalFs,
    LocalGit,
    RemoteGit,
    Network,
}

impl fmt::Display for Radius {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Radius::LocalFs => "local-fs",
            Radius::LocalGit => "local-git",
            Radius::RemoteGit => "remote-git",
            Radius::Network => "network",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone)]
pub enum Action {
    /// Write/Edit/MultiEdit/NotebookEdit — the diff itself is in the log.
    /// `probe` is the content the mutation claims to leave behind (a Write's
    /// body, an Edit's new_string) — used to verify a late landing really
    /// carries this edit and not a coincidental change to the same path.
    FileMutation { path: String, probe: Option<String> },
    /// Bash — an asserted effect with, at best, captured output as receipt.
    Command {
        command: String,
        radius: Option<Radius>,
    },
    /// An MCP tool call — `mcp__<server>__<tool>`. A first-class effectful
    /// action, NOT an observation: the agent asked a connected server to do
    /// something, and the server's tool_result (on the claim's receipt) is the
    /// oracle. `input` is the structured call payload (compact JSON).
    McpCall {
        server: String,
        tool: String,
        input: String,
    },
    /// Read/Grep/Glob/etc — observes, doesn't persist anything.
    Observation,
}

/// Split an MCP tool name `mcp__<server>__<tool>` into its parts. The server
/// is the first component after `mcp__`; everything after the next `__` is the
/// tool (server names use single `_`, so the first `__` is the boundary).
fn parse_mcp_name(name: &str) -> (String, String) {
    let rest = name.strip_prefix("mcp__").unwrap_or(name);
    match rest.split_once("__") {
        Some((server, tool)) => (server.to_string(), tool.to_string()),
        None => (rest.to_string(), String::new()),
    }
}

/// Captured output is capped: enough to carry every file list a command
/// prints, without holding a whole build log per receipt.
const RECEIPT_TEXT_CAP: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub struct Receipt {
    pub is_error: bool,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct Claim {
    /// Index into the causally ordered event list, for citing in the report.
    pub frame: usize,
    pub ts: Option<DateTime<Utc>>,
    pub action: Action,
    pub receipt: Option<Receipt>,
}

/// A real typed user prompt — the intent the work that follows answers to.
#[derive(Debug, Clone)]
pub struct Prompt {
    pub ts: Option<DateTime<Utc>>,
    pub text: String,
}

/// A substantial assistant prose message — the agent narrating what it did.
/// The one that follows a commit is that commit's own summary (a
/// natural-language claim, reconciled the same way the ledger is).
#[derive(Debug, Clone)]
pub struct Narration {
    pub ts: Option<DateTime<Utc>>,
    pub text: String,
}

/// Assistant prose is capped: a commit summary is a paragraph, not a log.
const NARRATION_CAP: usize = 4096;

/// Deduplicated token totals for the session. A single API request is
/// streamed as several JSONL records that repeat `message.usage` with
/// growing values; summing them raw overcounts 2–3×. We key each request
/// on `(message.id, requestId)`, keep the MAX of every field (the final
/// streamed value), and sum across distinct requests.
#[derive(Debug, Default)]
pub struct Tokens {
    pub requests: usize,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_creation: u64,
}

/// One turn of the conversation — a user prompt or an assistant message —
/// borrowed from the session for the `--full` transcript.
pub struct Turn<'a> {
    pub user: bool,
    pub ts: Option<DateTime<Utc>>,
    pub text: &'a str,
}

#[derive(Debug, Default)]
pub struct Session {
    pub claims: Vec<Claim>,
    pub prompts: Vec<Prompt>,
    /// Assistant prose messages, in order — the agent narrating its work.
    pub narrations: Vec<Narration>,
    pub tokens: Tokens,
    pub first_ts: Option<DateTime<Utc>>,
    pub last_ts: Option<DateTime<Utc>>,
    pub cwds: Vec<String>,
    pub branches: Vec<String>,
}

impl Session {
    /// The conversation — prompts and assistant narrations merged in time
    /// order — within the half-open window `(after, until]`. `None` bounds are
    /// unbounded; with both `None` (whole session) undated turns are kept, but
    /// a bounded (scoped) window drops them since they can't be placed.
    pub fn conversation(
        &self,
        after: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Vec<Turn<'_>> {
        let in_window = |ts: Option<DateTime<Utc>>| match ts {
            Some(t) => after.is_none_or(|a| t > a) && until.is_none_or(|u| t <= u),
            None => after.is_none() && until.is_none(),
        };
        let mut turns: Vec<Turn> = Vec::new();
        for p in &self.prompts {
            if in_window(p.ts) {
                turns.push(Turn {
                    user: true,
                    ts: p.ts,
                    text: &p.text,
                });
            }
        }
        for n in &self.narrations {
            if in_window(n.ts) {
                turns.push(Turn {
                    user: false,
                    ts: n.ts,
                    text: &n.text,
                });
            }
        }
        turns.sort_by_key(|t| t.ts);
        turns
    }
}

fn parse_ts(rec: &Record) -> Option<DateTime<Utc>> {
    rec.timestamp
        .as_deref()
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&Utc))
}

pub fn extract(ordered: &[Record]) -> Session {
    // receipts first, keyed by tool_use_id
    let mut receipts: HashMap<String, Receipt> = HashMap::new();
    for rec in ordered {
        for res in rec.tool_results() {
            let mut text = res.text();
            if text.len() > RECEIPT_TEXT_CAP {
                let mut cut = RECEIPT_TEXT_CAP;
                while !text.is_char_boundary(cut) {
                    cut -= 1;
                }
                text.truncate(cut);
            }
            receipts.insert(
                res.tool_use_id.clone(),
                Receipt {
                    is_error: res.is_error.unwrap_or(false),
                    text,
                },
            );
        }
    }

    // Token usage, deduplicated per request: streaming records repeat
    // message.usage with growing values, so we keep the MAX of each field
    // per (message.id, requestId) and sum across distinct requests.
    let mut per_request: HashMap<(String, String), Usage> = HashMap::new();

    let mut session = Session::default();
    for (frame, rec) in ordered.iter().enumerate() {
        let ts = parse_ts(rec);
        if let Some(t) = ts {
            if session.first_ts.is_none_or(|f| t < f) {
                session.first_ts = Some(t);
            }
            if session.last_ts.is_none_or(|l| t > l) {
                session.last_ts = Some(t);
            }
        }
        if let Some(cwd) = rec.cwd.as_deref()
            && !session.cwds.iter().any(|c| c == cwd)
        {
            session.cwds.push(cwd.to_string());
        }
        if let Some(b) = rec.git_branch.as_deref()
            && !b.is_empty()
            && !session.branches.iter().any(|x| x == b)
        {
            session.branches.push(b.to_string());
        }

        if rec.kind == "user"
            && let Some(text) = prompt_text(rec)
        {
            session.prompts.push(Prompt { ts, text });
        }

        if rec.kind == "assistant"
            && let Some(text) = assistant_text(rec)
        {
            session.narrations.push(Narration { ts, text });
        }

        if rec.kind == "assistant"
            && let Some(msg) = &rec.message
            && let Some(usage) = &msg.usage
        {
            // Key on the ids stable across a request's streamed records;
            // fall back to the record uuid so a missing id can't collapse
            // distinct requests together.
            let fallback = || rec.uuid.clone().unwrap_or_default();
            let key = (
                msg.id.clone().unwrap_or_else(fallback),
                rec.request_id.clone().unwrap_or_else(fallback),
            );
            let slot = per_request.entry(key).or_default();
            slot.input_tokens = slot.input_tokens.max(usage.input_tokens);
            slot.output_tokens = slot.output_tokens.max(usage.output_tokens);
            slot.cache_read_input_tokens = slot
                .cache_read_input_tokens
                .max(usage.cache_read_input_tokens);
            slot.cache_creation_input_tokens = slot
                .cache_creation_input_tokens
                .max(usage.cache_creation_input_tokens);
        }

        for tu in rec.tool_uses() {
            let action = classify_tool(&tu.name, &tu.input);
            session.claims.push(Claim {
                frame,
                ts,
                action,
                receipt: receipts.get(&tu.id).cloned(),
            });
        }
    }

    session.tokens.requests = per_request.len();
    for u in per_request.values() {
        session.tokens.input += u.input_tokens;
        session.tokens.output += u.output_tokens;
        session.tokens.cache_read += u.cache_read_input_tokens;
        session.tokens.cache_creation += u.cache_creation_input_tokens;
    }
    session
}

/// A user record counts as intent only when a person typed it: plain text,
/// no tool_result blocks, and not harness bookkeeping (command wrappers,
/// caveats, injected reminders). Returns the first human-looking line.
fn prompt_text(rec: &Record) -> Option<String> {
    let content = &rec.message.as_ref()?.content;
    let raw = match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => {
            if blocks
                .iter()
                .any(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
            {
                return None;
            }
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        }
        _ => return None,
    };
    raw.lines()
        .map(str::trim)
        .find(|l| {
            !l.is_empty() && !l.starts_with('<') && !l.starts_with("Caveat:") && !l.starts_with('[')
        })
        .map(|l| l.chars().take(200).collect())
}

/// Substantial assistant prose from a record — the text blocks joined,
/// trimmed, and capped. Skips trivial one-liners (a real commit summary is a
/// paragraph); returns None for tool-only or tiny messages.
fn assistant_text(rec: &Record) -> Option<String> {
    let content = &rec.message.as_ref()?.content;
    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return None,
    };
    let text = text.trim();
    if text.chars().count() < 40 {
        return None;
    }
    Some(text.chars().take(NARRATION_CAP).collect())
}

fn input_str<'v>(input: &'v Value, key: &str) -> Option<&'v str> {
    input.get(key).and_then(Value::as_str)
}

/// The content this mutation claims to leave in the file.
fn mutation_probe(tool: &str, input: &Value) -> Option<String> {
    match tool {
        "Write" => input_str(input, "content").map(str::to_string),
        "Edit" => input_str(input, "new_string").map(str::to_string),
        "MultiEdit" => input
            .get("edits")
            .and_then(Value::as_array)
            .and_then(|edits| edits.last())
            .and_then(|e| e.get("new_string"))
            .and_then(Value::as_str)
            .map(str::to_string),
        _ => None,
    }
}

fn classify_tool(name: &str, input: &Value) -> Action {
    match name {
        "Write" | "Edit" | "MultiEdit" => Action::FileMutation {
            path: input_str(input, "file_path")
                .unwrap_or_default()
                .to_string(),
            probe: mutation_probe(name, input),
        },
        "NotebookEdit" => Action::FileMutation {
            path: input_str(input, "notebook_path")
                .unwrap_or_default()
                .to_string(),
            probe: input_str(input, "new_source").map(str::to_string),
        },
        "Bash" => {
            let command = input_str(input, "command").unwrap_or_default().to_string();
            let radius = command_radius(&command);
            Action::Command { command, radius }
        }
        // MCP tool call — the non-shell way agents act. First-class effectful
        // action; the tool_result on the receipt is the oracle.
        n if n.starts_with("mcp__") => {
            let (server, tool) = parse_mcp_name(n);
            Action::McpCall {
                server,
                tool,
                input: serde_json::to_string(input).unwrap_or_default(),
            }
        }
        _ => Action::Observation,
    }
}

/// Every git subcommand invoked anywhere in a compound shell command.
/// Token-based so `git -C path commit`, `git -c k=v push`, and chains
/// behind `&&`/`;`/`|` are all seen.
pub fn git_subcommands(command: &str) -> Vec<String> {
    let mut subs = Vec::new();
    let tokens: Vec<&str> = command.split_whitespace().collect();
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i] == "git" {
            let mut j = i + 1;
            while j < tokens.len() {
                let t = tokens[j];
                if t == "-C" || t == "-c" {
                    j += 2;
                } else if t.starts_with('-') {
                    j += 1;
                } else {
                    subs.push(t.to_string());
                    break;
                }
            }
            i = j;
        }
        i += 1;
    }
    subs
}

const GIT_REMOTE: [&str; 6] = ["push", "pull", "fetch", "clone", "ls-remote", "remote"];
const GIT_LOCAL_WRITE: [&str; 18] = [
    "commit",
    "add",
    "rm",
    "mv",
    "tag",
    "branch",
    "checkout",
    "switch",
    "restore",
    "reset",
    "rebase",
    "merge",
    "stash",
    "cherry-pick",
    "init",
    "revert",
    "am",
    "apply",
];

/// Heuristic blast radius for a shell command; None means read-only.
/// Compound commands get the furthest radius any part reaches.
pub fn command_radius(command: &str) -> Option<Radius> {
    let mut radius: Option<Radius> = None;
    let mut bump = |r: Radius| {
        if radius.is_none_or(|cur| r > cur) {
            radius = Some(r);
        }
    };

    for sub in git_subcommands(command) {
        if GIT_REMOTE.contains(&sub.as_str()) {
            bump(Radius::RemoteGit);
        } else if GIT_LOCAL_WRITE.contains(&sub.as_str()) {
            bump(Radius::LocalGit);
        }
    }

    let has = |needle: &str| command.contains(needle);
    for net in [
        "curl",
        "wget",
        "gh ",
        "ssh ",
        "scp ",
        "rsync",
        "brew install",
        "npm install",
        "npm publish",
        "pip install",
        "cargo install",
        "cargo publish",
        "cargo add",
    ] {
        if has(net) {
            bump(Radius::Network);
        }
    }
    // check the first token of every segment, not just the whole command —
    // scripts routinely open with `cd` and mutate on later lines
    let fs_starts = [
        "rm", "mv", "cp", "mkdir", "touch", "chmod", "chown", "ln", "tee", "install",
    ];
    for segment in command
        .lines()
        .flat_map(|l| l.split("&&"))
        .flat_map(|l| l.split(';'))
        .flat_map(|l| l.split('|'))
    {
        let first_word = segment.split_whitespace().next().unwrap_or("");
        if fs_starts.contains(&first_word) {
            bump(Radius::LocalFs);
        }
    }
    for fs in [
        "sed -i",
        " > ",
        " >> ",
        "tee ",
        "mkdir ",
        " rm ",
        " mv ",
        " cp ",
        "touch ",
        "cargo build",
        "cargo fmt",
        "cargo test",
        "swift build",
        "xcodebuild",
        "make ",
        "npm run",
        "swift test",
        "codesign",
        "xcrun",
        "defaults write",
        "plutil",
    ] {
        if has(fs) {
            bump(Radius::LocalFs);
        }
    }
    radius
}

#[cfg(test)]
mod tests {
    use super::{Action, classify_tool, parse_mcp_name};
    use serde_json::json;

    #[test]
    fn mcp_name_splits_into_server_and_tool() {
        assert_eq!(
            parse_mcp_name("mcp__ccd_session_mgmt__list_sessions"),
            ("ccd_session_mgmt".to_string(), "list_sessions".to_string())
        );
        assert_eq!(
            parse_mcp_name("mcp__pg__query"),
            ("pg".to_string(), "query".to_string())
        );
    }

    #[test]
    fn mcp_call_is_a_first_class_action_not_an_observation() {
        match classify_tool("mcp__postgres__query", &json!({"sql": "SELECT 1"})) {
            Action::McpCall {
                server,
                tool,
                input,
            } => {
                assert_eq!(server, "postgres");
                assert_eq!(tool, "query");
                assert!(input.contains("SELECT 1"));
            }
            other => panic!("expected McpCall, got {other:?}"),
        }
        // a plain read-only tool is still an observation
        assert!(matches!(
            classify_tool("Read", &json!({"file_path": "x"})),
            Action::Observation
        ));
    }
}
