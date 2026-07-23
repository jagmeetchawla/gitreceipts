//! Git access for the interval spine. We shell out to `git` on purpose:
//! its output is itself the receipt, and v0.1 needs nothing libgit2 offers.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct SpineCommit {
    pub hash: String,
    pub short: String,
    /// First parent, empty for a root commit.
    pub parent: String,
    pub ts: DateTime<Utc>,
    pub subject: String,
    /// The reflog action that created it (commit, commit (amend), rebase…).
    pub reflog_action: String,
    /// Still reachable from a branch, or only alive in the reflog?
    pub reachable: bool,
}

impl SpineCommit {
    pub fn is_amend(&self) -> bool {
        self.reflog_action.starts_with("commit (amend)")
    }
}

#[derive(Debug, Clone)]
pub struct FileChange {
    pub status: char,
    pub path: String,
}

pub fn git(repo: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .with_context(|| format!("run git {}", args.join(" ")))?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Commit-creating reflog entries inside the window, in creation order.
///
/// The reflog is the local truth of what HEAD did while the session ran —
/// it sees amends, rebases, and commits later reset away, which plain
/// `git log` does not.
pub fn spine(repo: &Path, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<SpineCommit>> {
    let raw = git(
        repo,
        &["log", "-g", "--format=%H%x00%h%x00%cI%x00%P%x00%gs%x00%s"],
    )?;
    let reachable_raw = git(repo, &["rev-list", "--branches", "--tags", "HEAD"])?;
    let reachable: std::collections::HashSet<&str> = reachable_raw.lines().collect();

    let mut commits: Vec<SpineCommit> = Vec::new();
    // `log -g` lists newest first; walk it reversed to get creation order,
    // which stays right even when timestamps tie (commit + amend in the
    // same second, rebase bursts).
    for line in raw.lines().rev() {
        let mut parts = line.split('\u{0}');
        let (Some(hash), Some(short), Some(cdate), Some(parents), Some(gs), Some(subject)) = (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ) else {
            continue;
        };
        let creates_commit = ["commit", "rebase", "cherry-pick"]
            .iter()
            .any(|p| gs.starts_with(p));
        if !creates_commit {
            continue;
        }
        let Ok(ts) = DateTime::parse_from_rfc3339(cdate) else {
            continue;
        };
        let ts = ts.with_timezone(&Utc);
        if ts < from || ts > to {
            continue;
        }
        if commits.iter().any(|c| c.hash == hash) {
            continue;
        }
        commits.push(SpineCommit {
            hash: hash.to_string(),
            short: short.to_string(),
            parent: parents.split_whitespace().next().unwrap_or("").to_string(),
            ts,
            subject: subject.to_string(),
            reflog_action: gs.split(':').next().unwrap_or(gs).to_string(),
            reachable: reachable.contains(hash),
        });
    }
    Ok(commits)
}

/// Is this repo-relative path matched by the repo's ignore rules?
///
/// `rel_path` originates in the session file, which is untrusted — the
/// `--` keeps a path like `--stdin` from being parsed as a git option
/// (without it, check-ignore would block forever reading the terminal).
pub fn is_ignored(repo: &Path, rel_path: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["check-ignore", "-q", "--", rel_path])
        .stdin(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Every path that ever appeared in the repo's history. Used to decide
/// whether a session cwd is a historical alias of this repo (it was renamed
/// or moved) or some other place entirely (a scratch dir, another repo).
pub fn history_paths(repo: &Path) -> Result<std::collections::HashSet<String>> {
    let raw = git(repo, &["log", "--all", "--format=", "--name-only"])?;
    Ok(raw
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}

/// What one commit introduced vs its first parent: the interval statement.
pub fn commit_names(repo: &Path, hash: &str) -> Result<Vec<FileChange>> {
    let raw = git(
        repo,
        &[
            "show",
            "--name-status",
            "-M",
            "--first-parent",
            "--format=",
            hash,
        ],
    )?;
    Ok(parse_name_status(&raw))
}

pub fn parse_name_status(raw: &str) -> Vec<FileChange> {
    let mut changes = Vec::new();
    for line in raw.lines() {
        let mut cols = line.split('\t');
        let Some(status) = cols.next().and_then(|s| s.chars().next()) else {
            continue;
        };
        if !status.is_ascii_uppercase() {
            continue;
        }
        // renames list old\tnew — the new path is the one that exists at B
        let path = match status {
            'R' | 'C' => cols.nth(1),
            _ => cols.next(),
        };
        if let Some(path) = path {
            changes.push(FileChange {
                status,
                path: path.to_string(),
            });
        }
    }
    changes
}
