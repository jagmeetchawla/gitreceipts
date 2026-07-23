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

use crate::schema::Record;

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
    FileMutation { path: String },
    /// Bash — an asserted effect with, at best, captured output as receipt.
    Command {
        command: String,
        radius: Option<Radius>,
    },
    /// Read/Grep/Glob/etc — observes, doesn't persist anything.
    Observation,
}

#[derive(Debug, Clone)]
pub struct Receipt {
    pub is_error: bool,
}

#[derive(Debug, Clone)]
pub struct Claim {
    /// Index into the causally ordered event list, for citing in the report.
    pub frame: usize,
    pub ts: Option<DateTime<Utc>>,
    pub action: Action,
    pub receipt: Option<Receipt>,
}

#[derive(Debug, Default)]
pub struct Session {
    pub claims: Vec<Claim>,
    pub first_ts: Option<DateTime<Utc>>,
    pub last_ts: Option<DateTime<Utc>>,
    pub cwds: Vec<String>,
    pub branches: Vec<String>,
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
            receipts.insert(
                res.tool_use_id.clone(),
                Receipt {
                    is_error: res.is_error.unwrap_or(false),
                },
            );
        }
    }

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
    session
}

fn input_str<'v>(input: &'v Value, key: &str) -> Option<&'v str> {
    input.get(key).and_then(Value::as_str)
}

fn classify_tool(name: &str, input: &Value) -> Action {
    match name {
        "Write" | "Edit" | "MultiEdit" => Action::FileMutation {
            path: input_str(input, "file_path")
                .unwrap_or_default()
                .to_string(),
        },
        "NotebookEdit" => Action::FileMutation {
            path: input_str(input, "notebook_path")
                .unwrap_or_default()
                .to_string(),
        },
        "Bash" => {
            let command = input_str(input, "command").unwrap_or_default().to_string();
            let radius = command_radius(&command);
            Action::Command { command, radius }
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
fn command_radius(command: &str) -> Option<Radius> {
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
    let first_word = command.split_whitespace().next().unwrap_or("");
    let fs_starts = [
        "rm", "mv", "cp", "mkdir", "touch", "chmod", "chown", "ln", "tee", "install",
    ];
    if fs_starts.contains(&first_word) {
        bump(Radius::LocalFs);
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
    use super::*;

    #[test]
    fn git_subcommands_sees_through_global_flags() {
        assert_eq!(git_subcommands("git -C /x commit -m hi"), vec!["commit"]);
        assert_eq!(git_subcommands("git -c a=b push origin main"), vec!["push"]);
        assert_eq!(git_subcommands("ls -la && echo git"), Vec::<String>::new());
    }

    #[test]
    fn git_subcommands_counts_every_invocation() {
        let script = "git add -A\ngit commit -q -F - <<'MSG'\nfirst\nMSG\ngit commit -q -aF - <<'MSG'\nsecond\nMSG";
        let subs = git_subcommands(script);
        assert_eq!(subs.iter().filter(|s| *s == "commit").count(), 2);
    }

    #[test]
    fn radius_orders_by_reach() {
        assert_eq!(command_radius("git status"), None);
        assert_eq!(
            command_radius("git add -A && git commit -m x"),
            Some(Radius::LocalGit)
        );
        assert_eq!(
            command_radius("git commit -m x && git push"),
            Some(Radius::RemoteGit)
        );
        assert_eq!(command_radius("mkdir -p build"), Some(Radius::LocalFs));
        assert_eq!(
            command_radius("curl -s https://example.com"),
            Some(Radius::Network)
        );
    }
}
