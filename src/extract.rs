//! Stage 3: extract claims from the ordered records.
//!
//! A *claim* is anything the agent asserted it did to the world: a file
//! mutation (whose exact content is in the log) or a shell command (whose
//! effects are only as good as the receipt that came back). Tool results are
//! linked back to their claims by tool_use_id — they are the receipts.

use std::collections::HashMap;
use std::fmt;
use std::io::Read;

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

/// One deduplicated API request's provenance: which model produced it and, when
/// the log carries it, the reasoning-effort level. One per distinct
/// `(message.id, requestId)` — the same key token accounting dedups on — so
/// counts are requests, not streamed records.
#[derive(Debug, Clone)]
pub struct RequestMeta {
    pub ts: Option<DateTime<Utc>>,
    pub model: Option<String>,
    pub effort: Option<String>,
}

/// How much of a window's requests carried an `effort` tag — effort logging is
/// newer and sparse, so a receipt must say whether it saw all, some, or none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coverage {
    None,
    Partial,
    Full,
}

impl Coverage {
    pub fn as_str(self) -> &'static str {
        match self {
            Coverage::None => "none",
            Coverage::Partial => "partial",
            Coverage::Full => "full",
        }
    }
}

#[derive(Debug, Default)]
pub struct Session {
    pub claims: Vec<Claim>,
    pub prompts: Vec<Prompt>,
    /// Assistant prose messages, in order — the agent narrating its work.
    pub narrations: Vec<Narration>,
    pub tokens: Tokens,
    /// One entry per deduplicated API request — the model (and, where logged,
    /// the reasoning effort) behind each. Time-sorted, so a window can name the
    /// model(s) that drove any interval.
    pub requests: Vec<RequestMeta>,
    pub first_ts: Option<DateTime<Utc>>,
    pub last_ts: Option<DateTime<Utc>>,
    pub cwds: Vec<String>,
    pub branches: Vec<String>,
}

/// Half-open window predicate shared by `conversation` and the provenance
/// queries: `None` bounds are unbounded; with both `None` (whole session)
/// undated items are kept, but any bounded window drops them (unplaceable).
fn in_window(
    ts: Option<DateTime<Utc>>,
    after: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
) -> bool {
    match ts {
        Some(t) => after.is_none_or(|a| t > a) && until.is_none_or(|u| t <= u),
        None => after.is_none() && until.is_none(),
    }
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
        let in_window = |ts: Option<DateTime<Utc>>| in_window(ts, after, until);
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

    /// The model(s) that produced requests within `(after, until]`, each with
    /// its request count, most-used first (ties broken by id). `(None, None)`
    /// rolls up the whole session. Synthetic/unlabelled requests are excluded.
    pub fn models_used(
        &self,
        after: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Vec<(String, usize)> {
        let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for r in &self.requests {
            if in_window(r.ts, after, until)
                && let Some(m) = r.model.as_deref()
            {
                *counts.entry(m).or_default() += 1;
            }
        }
        let mut v: Vec<(String, usize)> = counts
            .into_iter()
            .map(|(k, c)| (k.to_string(), c))
            .collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        v
    }

    /// The distinct reasoning-effort levels observed within `(after, until]`,
    /// plus how much of the window's requests actually carried an effort tag.
    /// Effort logging is sparse, so `Coverage` is the honesty signal: `Full`
    /// only when every request in the window was tagged, `None` when none were.
    pub fn effort_seen(
        &self,
        after: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> (Vec<String>, Coverage) {
        let mut set: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        let (mut total, mut tagged) = (0usize, 0usize);
        for r in &self.requests {
            if in_window(r.ts, after, until) {
                total += 1;
                if let Some(e) = r.effort.as_deref() {
                    tagged += 1;
                    set.insert(e);
                }
            }
        }
        let coverage = if tagged == 0 {
            Coverage::None
        } else if tagged == total {
            Coverage::Full
        } else {
            Coverage::Partial
        };
        (set.into_iter().map(str::to_string).collect(), coverage)
    }
}

fn parse_ts(rec: &Record) -> Option<DateTime<Utc>> {
    rec.timestamp
        .as_deref()
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&Utc))
}

/// A large tool result is offloaded by the harness: the log holds a
/// `<persisted-output>` pointer + a 2KB preview, and the full content lives in
/// a sibling `tool-results/<id>.txt`. Follow the pointer to recover the full
/// receipt (the oracle) — a truncated preview can drop the very "error" or
/// confirmation line that matters. Returns None (keep the self-describing
/// preview) if the marker/path is absent, fails the guardrail, or is
/// unreadable.
///
/// Guardrail (session logs are untrusted input): only follow a path that looks
/// like a Claude tool-results file (`…/tool-results/*.txt`), is a *regular*
/// file (reject symlinks via `symlink_metadata`), and read at most one receipt
/// cap of it. (Auditing another machine's logs would want this further
/// restricted to the known store root — a later hardening.)
fn resolve_offload(text: &str) -> Option<String> {
    if !text.starts_with("<persisted-output>") {
        return None;
    }
    let path = text
        .lines()
        .find_map(|l| l.split_once("saved to: ").map(|(_, p)| p.trim()))?;
    if !path.contains("/tool-results/") || !path.ends_with(".txt") {
        return None;
    }
    if !std::fs::symlink_metadata(path).ok()?.is_file() {
        return None;
    }
    let mut buf = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(RECEIPT_TEXT_CAP as u64 + 1)
        .read_to_string(&mut buf)
        .ok()?;
    Some(buf)
}

pub fn extract(ordered: &[Record]) -> Session {
    // receipts first, keyed by tool_use_id
    let mut receipts: HashMap<String, Receipt> = HashMap::new();
    for rec in ordered {
        for res in rec.tool_results() {
            // Follow a `<persisted-output>` pointer to the full receipt; else
            // keep the (self-describing) preview text.
            let mut text = resolve_offload(&res.text()).unwrap_or_else(|| res.text());
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
    // Per-request provenance (model + effort), keyed the same way. Kept separate
    // from the usage map because effort can appear on a record that carries no
    // usage, so we must fold across *all* of a request's records, not just the
    // usage-bearing one.
    let mut per_meta: HashMap<(String, String), RequestMeta> = HashMap::new();

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

        // Provenance: model on (virtually) every assistant record, effort on
        // some. Fold across the whole request; first non-empty value wins, ts
        // is the earliest seen. `<synthetic>` is the harness's own turns — not a
        // model the user chose, so it never counts.
        if rec.kind == "assistant"
            && let Some(msg) = &rec.message
        {
            let fallback = || rec.uuid.clone().unwrap_or_default();
            let key = (
                msg.id.clone().unwrap_or_else(fallback),
                rec.request_id.clone().unwrap_or_else(fallback),
            );
            let meta = per_meta.entry(key).or_insert_with(|| RequestMeta {
                ts: None,
                model: None,
                effort: None,
            });
            if let Some(t) = ts
                && meta.ts.is_none_or(|s| t < s)
            {
                meta.ts = Some(t);
            }
            if meta.model.is_none()
                && let Some(m) = msg.model.as_deref()
                && m != "<synthetic>"
                && !m.is_empty()
            {
                meta.model = Some(m.to_string());
            }
            if meta.effort.is_none()
                && let Some(e) = rec.effort.as_deref()
                && !e.is_empty()
            {
                meta.effort = Some(e.to_string());
            }
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
    session.requests = per_meta.into_values().collect();
    session.requests.sort_by_key(|r| r.ts);
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
    use super::{Action, Coverage, classify_tool, extract, parse_mcp_name, resolve_offload};
    use crate::schema::Record;
    use chrono::{DateTime, Utc};
    use serde_json::json;

    fn ts(s: &str) -> Option<DateTime<Utc>> {
        Some(DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc))
    }

    /// An assistant record with a model, optional effort, keyed by (id, req).
    fn asst(ts: &str, id: &str, req: &str, model: &str, effort: Option<&str>) -> Record {
        let mut v = json!({
            "type": "assistant",
            "timestamp": ts,
            "requestId": req,
            "message": { "id": id, "model": model, "content": [] },
        });
        if let Some(e) = effort {
            v["effort"] = json!(e);
        }
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn model_and_effort_are_captured_per_request_with_honest_coverage() {
        // Two records share one request (id=a/req=1): the model is on both, the
        // effort tag only on the second — it must still be folded in. A third
        // request is a different model; a fourth is <synthetic> (harness noise).
        let records = vec![
            asst("2026-01-01T10:00:00Z", "a", "1", "claude-opus-4-8", None),
            asst(
                "2026-01-01T10:01:00Z",
                "a",
                "1",
                "claude-opus-4-8",
                Some("high"),
            ),
            asst(
                "2026-01-01T10:05:00Z",
                "b",
                "2",
                "claude-fable-5",
                Some("high"),
            ),
            asst("2026-01-01T10:06:00Z", "c", "3", "<synthetic>", None),
        ];
        let session = extract(&records);

        // Three deduped requests; the synthetic one carries no model.
        assert_eq!(session.requests.len(), 3, "one entry per (id, requestId)");

        // Roll-up: synthetic excluded; ties sort by id ascending.
        assert_eq!(
            session.models_used(None, None),
            vec![
                ("claude-fable-5".to_string(), 1),
                ("claude-opus-4-8".to_string(), 1)
            ]
        );

        // Effort observed on 2 of 3 requests → partial, honestly.
        let (efforts, coverage) = session.effort_seen(None, None);
        assert_eq!(efforts, vec!["high".to_string()]);
        assert_eq!(coverage, Coverage::Partial);

        // Windowing: (10:02, 10:10] excludes request `a` (earliest ts 10:00),
        // keeps fable (10:05); the synthetic request has no model to report.
        assert_eq!(
            session.models_used(ts("2026-01-01T10:02:00Z"), ts("2026-01-01T10:10:00Z")),
            vec![("claude-fable-5".to_string(), 1)]
        );
    }

    #[test]
    fn effort_coverage_is_none_when_the_log_never_tags_it() {
        // Older logs carry no effort at all — coverage must say `None`, not fake
        // completeness, and the observed set is empty.
        let records = vec![
            asst("2026-01-01T10:00:00Z", "a", "1", "claude-opus-4-8", None),
            asst("2026-01-01T10:05:00Z", "b", "2", "claude-opus-4-8", None),
        ];
        let session = extract(&records);
        let (efforts, coverage) = session.effort_seen(None, None);
        assert!(efforts.is_empty());
        assert_eq!(coverage, Coverage::None);
        // A single model across the whole session still rolls up cleanly.
        assert_eq!(
            session.models_used(None, None),
            vec![("claude-opus-4-8".to_string(), 2)]
        );
    }

    #[test]
    fn offload_pointer_is_followed_to_the_full_receipt() {
        let base = std::env::temp_dir().join(format!("gr-offload-{}", std::process::id()));
        let dir = base.join("tool-results");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("abc123.txt");
        std::fs::write(&file, "FULL RECEIPT\nerror at line 99\ntail").unwrap();

        let pointer = format!(
            "<persisted-output>\nOutput too large (44.6KB). Full output saved to: {}\n\nPreview (first 2KB):\ntruncated…",
            file.display()
        );
        let full = resolve_offload(&pointer).expect("should follow the pointer");
        assert!(
            full.contains("error at line 99"),
            "recovered the full receipt"
        );

        // guardrails: not-a-pointer → None; a path outside tool-results → None
        assert!(resolve_offload("plain output").is_none());
        assert!(
            resolve_offload("<persisted-output>\nFull output saved to: /etc/passwd\nx").is_none(),
            "only follows …/tool-results/*.txt"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

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
