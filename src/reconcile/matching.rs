//! Path and command matching: which repo a claimed path belongs to, and
//! whether a command named or removed a path. Whole-token matching only —
//! `config.rs` must never match inside `config.rs.log` (that was a
//! false-green; see VERDICT §4.3/§5.2).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::extract::{Action, Session};
use crate::gitio::FileChange;

/// Match a claimed absolute path against the audited repo.
///
/// The repo root itself always qualifies. A session cwd qualifies as a
/// *historical alias* (the repo was renamed or moved mid-session, or is
/// another checkout of the same repo) only if it survives two gates:
///
/// 1. **Ownership.** A cwd owned by a DIFFERENT live git repo can alias
///    only when that repo shares a root commit with this one (a clone or
///    second checkout). A sibling checked out beside this repo shares no
///    history and can never alias — shared scaffold basenames must not
///    drag its claims into this ledger as false broken promises.
/// 2. **Content-verified majority.** Where claims carry usable probes,
///    the majority of claimed CONTENT under the candidate must appear in
///    this repo's history — filenames are shared across scaffolds
///    (CLAUDE.md, package.json); bytes are not. Only probe-less claim
///    sets fall back to the filename-majority test.
pub(crate) struct Roots {
    valid: Vec<String>,
}

impl Roots {
    pub(crate) fn build(
        repo: &Path,
        repo_canon: &str,
        session: &Session,
        history: &HashSet<String>,
    ) -> Roots {
        let mut candidates: Vec<String> = vec![repo_canon.to_string()];
        for cwd in &session.cwds {
            if candidates.contains(cwd) {
                continue;
            }
            match owning_git_root(cwd) {
                Some(owner) if owner != repo_canon => {
                    if shares_root_commit(repo, Path::new(&owner)) {
                        candidates.push(cwd.clone());
                    }
                }
                _ => candidates.push(cwd.clone()),
            }
        }

        // group distinct claimed paths (and their last usable probe) by
        // their longest-prefix candidate
        let mut claimed_under: HashMap<&str, HashSet<String>> = HashMap::new();
        let mut probes_under: HashMap<&str, HashMap<String, String>> = HashMap::new();
        for claim in &session.claims {
            if let Action::FileMutation { path, probe } = &claim.action
                && let Some((root, rel)) = longest_prefix(path, &candidates)
            {
                claimed_under.entry(root).or_default().insert(rel.clone());
                if let Some(p) = probe
                    && usable_probe(p)
                {
                    probes_under.entry(root).or_default().insert(rel, p.clone());
                }
            }
        }

        let valid = candidates
            .iter()
            .filter(|root| {
                if root.as_str() == repo_canon {
                    return true;
                }
                let Some(rels) = claimed_under.get(root.as_str()) else {
                    return false;
                };
                if rels.is_empty() {
                    return false;
                }
                if let Some(probes) = probes_under.get(root.as_str()) {
                    // Every probed rel votes — a truncated sample would bias
                    // the vote (alphabetical caps like CLAUDE.md/MEMORY.md
                    // sort first and are the least likely to have landed,
                    // under-crediting a true rename).
                    let mut sample: Vec<(&String, &String)> = probes.iter().collect();
                    sample.sort();
                    if !sample.is_empty() {
                        let hits = sample
                            .iter()
                            .filter(|(rel, probe)| content_in_history(repo, rel, probe))
                            .count();
                        return hits * 2 >= sample.len();
                    }
                }
                let hits = rels.iter().filter(|r| history.contains(*r)).count();
                hits * 2 >= rels.len()
            })
            .cloned()
            .collect();
        Roots { valid }
    }

    pub(crate) fn relativize(&self, path: &str) -> Option<String> {
        longest_prefix(path, &self.valid).map(|(_, rel)| rel)
    }
}

/// The canonical root of the git repo that owns `path`, if any — the
/// nearest ancestor (or `path` itself) containing a `.git` entry.
fn owning_git_root(path: &str) -> Option<String> {
    let mut p = std::path::Path::new(path);
    loop {
        if p.join(".git").exists() {
            return p.canonicalize().ok().map(|c| c.display().to_string());
        }
        p = p.parent()?;
    }
}

/// Two working directories belong to the same repo family iff they share a
/// root commit — true for a clone or second checkout, never for a sibling
/// project that merely shares scaffold filenames.
fn shares_root_commit(repo: &Path, other: &Path) -> bool {
    let roots_of = |p: &Path| {
        crate::gitio::git(p, &["rev-list", "--max-parents=0", "HEAD"])
            .map(|raw| raw.lines().map(str::to_string).collect::<HashSet<String>>())
            .unwrap_or_default()
    };
    let a = roots_of(repo);
    !a.is_empty() && !a.is_disjoint(&roots_of(other))
}

/// Did this claimed content ever land at `rel` in the repo's history?
/// Every commit that touched `rel` is a candidate — a claim lands in its
/// own era and later edits overwrite it (a product-wide rename rewriting
/// the very bytes must not un-verify the claims that built the file).
/// Same content-level standard the landing checks use, applied to alias
/// qualification. Walk capped: an alias vote needs a hit, not a census.
fn content_in_history(repo: &Path, rel: &str, probe: &str) -> bool {
    let Ok(raw) = crate::gitio::git(repo, &["log", "--all", "--format=%H", "--", rel]) else {
        return false;
    };
    raw.lines()
        .take(30)
        .map(str::trim)
        .filter(|h| !h.is_empty())
        .any(|hash| {
            crate::gitio::file_at_commit(repo, hash, rel).is_some_and(|c| c.contains(probe))
        })
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

/// Presentation-only failure triage (VERDICT: can never flip a verdict).
/// Classifies a failed command by cited evidence; ambiguous stays "genuine".
pub(crate) fn triage_failure(
    command: &str,
    output: &str,
    user_abort: bool,
    retried_ok: bool,
) -> crate::reconcile::FailureTriage {
    use crate::reconcile::FailureTriage;
    if user_abort {
        return FailureTriage {
            class: "user-abort",
            evidence: "stopped by the user — their stop, not the agent's failure".into(),
        };
    }
    if retried_ok {
        return FailureTriage {
            class: "retried-and-passed",
            evidence: "the same command ran again later and passed".into(),
        };
    }
    // Exit-code conventions: nonzero that documented behavior defines as a
    // RESULT, not an error. Program = first token, skipping VAR= prefixes.
    let exit = output
        .lines()
        .next()
        .and_then(|l| l.strip_prefix("Exit code "))
        .and_then(|n| n.trim().parse::<u32>().ok());
    let program = command
        .split_whitespace()
        .find(|t| !t.contains('='))
        .unwrap_or("");
    let program = program.rsplit('/').next().unwrap_or(program);
    if exit == Some(1) {
        let convention = match program {
            "grep" | "rg" | "egrep" | "fgrep" | "ugrep" => Some("no match"),
            "diff" | "cmp" => Some("inputs differ"),
            "test" | "[" | "[[" => Some("condition false"),
            _ => None,
        };
        if let Some(meaning) = convention {
            return FailureTriage {
                class: "expected-nonzero",
                evidence: format!("exit 1 — {program}'s documented \"{meaning}\" result"),
            };
        }
    }
    if command.contains("||") || command.trim_start().starts_with("! ") {
        return FailureTriage {
            class: "guarded",
            evidence: "the command guards its own nonzero path (`||`/`!`)".into(),
        };
    }
    let low = output.to_ascii_lowercase();
    if low.contains("operation not permitted") || low.contains("sandbox") {
        return FailureTriage {
            class: "sandbox-denial",
            evidence: "blocked by the sandbox/permissions, not by the command".into(),
        };
    }
    // No output excerpt here: evidence strings travel un-redacted into
    // every surface, and raw output can carry home paths and usernames
    // (the QA leak check caught exactly that). The output field itself
    // ships alongside, through the normal redaction pipeline.
    let _ = output;
    FailureTriage {
        class: "genuine",
        evidence: "unclassified failure — its captured output is attached".into(),
    }
}

/// How a claimed probe was found in a committed blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProbeMatch {
    /// Byte-for-byte.
    Exact,
    /// Identical once whitespace is collapsed — a formatter (`cargo fmt`,
    /// prettier, gofmt) rewrapped the agent's text between the write and
    /// the commit.
    Reformatted,
}

/// Does this blob carry the claimed content?
///
/// Exact first. Failing that, compare with **all whitespace removed**: a
/// formatter changes spacing, not meaning, and refusing to see through it
/// turned a kept promise into a broken one (our own `scan_wiring.rs`,
/// 2026-08-09 — `cargo fmt` wrapped one call across three lines between
/// the write and the commit).
///
/// Whitespace must be *removed*, not collapsed: a formatter inserts space
/// where the author had none (`f(` + newline + `"x"`), which collapsing
/// cannot undo. Every non-whitespace byte must still match in order, and
/// probes already clear a specificity floor (§usable_probe), so nothing
/// else is loosened.
pub(crate) fn probe_in(content: &str, probe: &str) -> Option<ProbeMatch> {
    if content.contains(probe) {
        return Some(ProbeMatch::Exact);
    }
    let squeeze = |s: &str| -> String { s.chars().filter(|c| !c.is_whitespace()).collect() };
    let (p, c) = (squeeze(probe), squeeze(content));
    if c.contains(&p) {
        return Some(ProbeMatch::Reformatted);
    }
    // Formatters do more than respace: rustfmt adds a trailing comma when
    // it wraps a call, so even whitespace-free text differs by a byte or
    // two. Allow ONE small local difference, measured as the matching head
    // plus the matching tail: identical except in one place. Scattered
    // edits fail this and stay unverified, which is the honest default.
    let head = p.bytes().zip(c.bytes()).take_while(|(a, b)| a == b).count();
    let tail = p
        .bytes()
        .rev()
        .zip(c.bytes().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let matched = (head + tail).min(p.len());
    if !p.is_empty() && matched * 100 >= p.len() * 98 {
        return Some(ProbeMatch::Reformatted);
    }
    None
}
