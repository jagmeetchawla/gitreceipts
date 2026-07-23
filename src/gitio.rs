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
    /// Timestamp used for windowing and interval mapping. For a commit
    /// with a clock anomaly this is clamped into sequence, because its
    /// own dates cannot be trusted.
    pub ts: DateTime<Utc>,
    /// What the commit object itself claims (committer date).
    pub committer_ts: DateTime<Utc>,
    pub subject: String,
    /// The reflog action that created it (commit, commit (amend), rebase…).
    pub reflog_action: String,
    /// Still reachable from a branch, or only alive in the reflog?
    pub reachable: bool,
    /// The reflog places this commit between in-window commits, but its
    /// own timestamps say otherwise. Dates are trivially forgeable
    /// (GIT_COMMITTER_DATE forges the reflog stamp too); creation ORDER
    /// is not. We keep the commit and say the clock cannot be trusted.
    pub clock_anomaly: bool,
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
///
/// Windowing is the sandwich rule: an entry belongs to the session if its
/// own timestamps fall in the window, OR if it was created between two
/// entries that do. Timestamps are forgeable; the reflog's creation order
/// is not, so a backdated commit cannot slip out of the audit — it stays
/// in, flagged as a clock anomaly.
pub fn spine(repo: &Path, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<SpineCommit>> {
    let raw = git(
        repo,
        &[
            "log",
            "-g",
            "--date=iso-strict",
            "--format=%H%x00%h%x00%cI%x00%gd%x00%P%x00%gs%x00%s",
        ],
    )?;
    let reachable_raw = git(repo, &["rev-list", "--branches", "--tags", "HEAD"])?;
    let reachable: std::collections::HashSet<&str> = reachable_raw.lines().collect();

    // parse every commit-creating entry in creation order first
    let mut all: Vec<(SpineCommit, bool)> = Vec::new(); // (commit, in_window)
    // `log -g` lists newest first; walk it reversed to get creation order,
    // which stays right even when timestamps tie (commit + amend in the
    // same second, rebase bursts).
    for line in raw.lines().rev() {
        let mut parts = line.split('\u{0}');
        let (
            Some(hash),
            Some(short),
            Some(cdate),
            Some(gdate),
            Some(parents),
            Some(gs),
            Some(subject),
        ) = (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        )
        else {
            continue;
        };
        let creates_commit = ["commit", "rebase", "cherry-pick"]
            .iter()
            .any(|p| gs.starts_with(p));
        if !creates_commit {
            continue;
        }
        let Ok(committer_ts) = DateTime::parse_from_rfc3339(cdate) else {
            continue;
        };
        let committer_ts = committer_ts.with_timezone(&Utc);
        // %gd with iso-strict renders "HEAD@{<iso>}"
        let reflog_ts = gdate
            .split('{')
            .nth(1)
            .and_then(|s| s.strip_suffix('}'))
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or(committer_ts);
        if all.iter().any(|(c, _)| c.hash == hash) {
            continue;
        }
        let in_window = |t: DateTime<Utc>| t >= from && t <= to;
        let inside = in_window(committer_ts) || in_window(reflog_ts);
        all.push((
            SpineCommit {
                hash: hash.to_string(),
                short: short.to_string(),
                parent: parents.split_whitespace().next().unwrap_or("").to_string(),
                ts: committer_ts,
                committer_ts,
                subject: subject.to_string(),
                reflog_action: gs.split(':').next().unwrap_or(gs).to_string(),
                reachable: reachable.contains(hash),
                clock_anomaly: false,
            },
            inside,
        ));
    }

    // sandwich: keep everything from the first in-window entry to the last
    let first = all.iter().position(|(_, inside)| *inside);
    let last = all.iter().rposition(|(_, inside)| *inside);
    let (Some(first), Some(last)) = (first, last) else {
        return Ok(Vec::new());
    };
    let mut commits: Vec<SpineCommit> = Vec::new();
    for (mut commit, inside) in all.into_iter().take(last + 1).skip(first) {
        if !inside {
            commit.clock_anomaly = true;
            // its own dates are untrusted; clamp into sequence so interval
            // mapping stays monotonic
            commit.ts = commits.last().map(|p: &SpineCommit| p.ts).unwrap_or(from);
        }
        if let Some(prev) = commits.last()
            && commit.ts < prev.ts
        {
            commit.ts = prev.ts;
        }
        commits.push(commit);
    }
    Ok(commits)
}

/// The content of `path` as committed in `hash`, if it exists there.
/// Used to verify that a claimed edit's content actually reached a commit.
pub fn blob_at(repo: &Path, hash: &str, path: &str) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .arg("show")
        .arg(format!("{hash}:{path}"))
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
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

/// Paths currently tracked by git (the index). A residue file that is no
/// longer here was later untracked or deleted — yesterday's noise, not
/// today's problem.
pub fn tracked_paths(repo: &Path) -> Result<std::collections::HashSet<String>> {
    let raw = git(repo, &["ls-files"])?;
    Ok(raw
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
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
