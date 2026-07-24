//! Path and command matching: which repo a claimed path belongs to, and
//! whether a command named or removed a path. Whole-token matching only —
//! `config.rs` must never match inside `config.rs.log` (that was a
//! false-green; see VERDICT §4.3/§5.2).

use std::collections::{HashMap, HashSet};

use crate::extract::{Action, Session};
use crate::gitio::FileChange;

/// Match a claimed absolute path against the audited repo.
///
/// The repo root itself always qualifies. A session cwd qualifies as a
/// *historical alias* (the repo was renamed or moved mid-session) only if
/// the majority of distinct paths claimed under it appear somewhere in the
/// repo's history — a scratch directory or a sibling repo fails that test.
pub(crate) struct Roots {
    valid: Vec<String>,
}

impl Roots {
    pub(crate) fn build(repo_canon: &str, session: &Session, history: &HashSet<String>) -> Roots {
        let mut candidates: Vec<String> = vec![repo_canon.to_string()];
        for cwd in &session.cwds {
            if !candidates.contains(cwd) {
                candidates.push(cwd.clone());
            }
        }

        // group distinct claimed paths by their longest-prefix candidate
        let mut claimed_under: HashMap<&str, HashSet<String>> = HashMap::new();
        for claim in &session.claims {
            if let Action::FileMutation { path, .. } = &claim.action
                && let Some((root, rel)) = longest_prefix(path, &candidates)
            {
                claimed_under.entry(root).or_default().insert(rel);
            }
        }

        let valid = candidates
            .iter()
            .filter(|root| {
                if root.as_str() == repo_canon {
                    return true;
                }
                match claimed_under.get(root.as_str()) {
                    Some(rels) if !rels.is_empty() => {
                        let hits = rels.iter().filter(|r| history.contains(*r)).count();
                        hits * 2 >= rels.len()
                    }
                    _ => false,
                }
            })
            .cloned()
            .collect();
        Roots { valid }
    }

    pub(crate) fn relativize(&self, path: &str) -> Option<String> {
        longest_prefix(path, &self.valid).map(|(_, rel)| rel)
    }
}

pub fn longest_prefix<'r>(path: &str, roots: &'r [String]) -> Option<(&'r str, String)> {
    // A rel with a `..` component could escape the repo when later joined
    // or handed to git — claimed paths are untrusted, so such a claim goes
    // to the out-of-repo bucket instead of the ledger.
    roots
        .iter()
        .filter_map(|root| {
            path.strip_prefix(root.as_str())
                .and_then(|rest| rest.strip_prefix('/'))
                .filter(|rel| !rel.split('/').any(|c| c == ".."))
                .map(|rel| (root.as_str(), rel.to_string()))
        })
        .max_by_key(|(root, _)| root.len())
}

/// A probe (claimed post-edit content) is specific enough to verify a
/// landing by content only if it carries real signal — a one-character
/// edit like `}` or `"` would "match" almost any blob.
pub(crate) fn usable_probe(probe: &str) -> bool {
    let t = probe.trim();
    t.len() >= 12 && t.chars().any(|c| c.is_alphanumeric())
}

/// Split a shell command into path-like tokens: whitespace-separated,
/// with surrounding quotes and trailing shell punctuation stripped. Used
/// for WHOLE-TOKEN path matching, so `config.rs` never matches inside
/// `config.rs.log`.
fn command_tokens(command: &str) -> Vec<&str> {
    command
        .split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '=' | '(' | ')' | ';'))
        .map(|t| t.trim_matches(|c: char| matches!(c, ',' | ':')))
        .filter(|t| !t.is_empty())
        .collect()
}

/// The paths this change is known by: its current path and its pre-rename
/// path. Matching is done against whole command tokens, plus directory
/// containment, so no substring accidents.
pub(crate) fn change_paths(change: &FileChange) -> Vec<&str> {
    let mut v = vec![change.path.as_str()];
    if let Some(old) = &change.old_path {
        v.push(old.as_str());
    }
    v
}

/// Does `command` name `path` as a token — exactly, or via a directory it
/// moved/removed (a token that is an ancestor directory of the path, as in
/// `git mv src/OldName src/NewName` covering every child)?
pub(crate) fn command_names_path(command: &str, path: &str) -> bool {
    command_tokens(command).iter().any(|t| {
        *t == path
            || path.strip_prefix(t).is_some_and(|r| r.starts_with('/'))
            || t.strip_prefix(path).is_some_and(|r| r.starts_with('/'))
    })
}

const REMOVAL_VERBS: [&str; 3] = ["rm", "mv", "unlink"];

/// Did some segment of `command` actually remove or move `path` — a
/// removal verb (`rm`, `mv`, `git rm`, `git mv`, `git clean`) whose
/// tokens name the path? Mentioning the name is not enough; a genuine
/// broken promise must not resolve just because a later `echo` prints it.
pub(crate) fn command_removes_path(command: &str, path: &str) -> bool {
    for segment in command
        .lines()
        .flat_map(|l| l.split("&&"))
        .flat_map(|l| l.split(';'))
        .flat_map(|l| l.split('|'))
    {
        let first = segment.split_whitespace().next().unwrap_or("");
        let subs = crate::extract::git_subcommands(segment);
        let removes = REMOVAL_VERBS.contains(&first)
            || subs
                .iter()
                .any(|s| matches!(s.as_str(), "rm" | "mv" | "clean"));
        if removes && command_names_path(segment, path) {
            return true;
        }
    }
    false
}
