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
    /// Known from commit history rather than the local reflog. When a
    /// session-era reflog exists alongside, this usually means the commit
    /// was created elsewhere and pulled in — useful attribution. With no
    /// reflog at all it is simply how every commit is known.
    pub from_history: bool,
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
    /// For renames/copies: where the file lived before. Commands that did
    /// the moving name this side.
    pub old_path: Option<String>,
}

const NUL: char = '\u{0}';

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

/// The commit spine for a session window.
///
/// PRIMARY: commit history (`git log --all`), windowed on committer date —
/// the one source every repo has, clone or original.
///
/// ENRICHMENT: the local reflog, when it exists. It contributes what
/// history cannot: amended drafts and reset-away commits (objects that
/// never reached a ref), true creation order for tied timestamps, and the
/// sandwich rule — an entry created between two in-window entries belongs
/// to the session no matter what its (forgeable) dates claim, flagged as
/// a clock anomaly. A repo without a useful reflog simply gets the
/// history spine, quietly.
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
                from_history: false,
            },
            inside,
        ));
    }

    // sandwich: keep everything from the first in-window entry to the last
    let first = all.iter().position(|(_, inside)| *inside);
    let last = all.iter().rposition(|(_, inside)| *inside);
    let mut commits: Vec<SpineCommit> = Vec::new();
    if let (Some(first), Some(last)) = (first, last) {
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
    }

    // PRIMARY source: every window commit from history that the reflog
    // walk above didn't already place. With no reflog this is the whole
    // spine; with one, it adds commits created elsewhere (pulls, fetches).
    let known: std::collections::HashSet<String> = commits.iter().map(|c| c.hash.clone()).collect();
    let hist_raw = git(
        repo,
        &["log", "--all", "--format=%H%x00%h%x00%cI%x00%P%x00%s"],
    )?;
    let mut extra: Vec<SpineCommit> = Vec::new();
    for line in hist_raw.lines() {
        let mut parts = line.split(NUL);
        let (Some(hash), Some(short), Some(cdate), Some(parents), Some(subject)) = (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ) else {
            continue;
        };
        if known.contains(hash) {
            continue;
        }
        let Ok(ts) = DateTime::parse_from_rfc3339(cdate) else {
            continue;
        };
        let ts = ts.with_timezone(&Utc);
        if ts < from || ts > to {
            continue;
        }
        extra.push(SpineCommit {
            hash: hash.to_string(),
            short: short.to_string(),
            parent: parents.split_whitespace().next().unwrap_or("").to_string(),
            ts,
            committer_ts: ts,
            subject: subject.to_string(),
            reflog_action: "history".to_string(),
            reachable: reachable.contains(hash),
            clock_anomaly: false,
            from_history: true,
        });
    }
    extra.sort_by_key(|c| c.ts);
    for commit in extra {
        let pos = commits
            .iter()
            .position(|c| c.ts > commit.ts)
            .unwrap_or(commits.len());
        commits.insert(pos, commit);
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

/// Commits reachable from any remote-tracking ref — i.e. pushed, as of the
/// repo's last fetch. A local-only commit is absent. This is a local check
/// (no network), so it reflects the last-known remote state, not live.
pub fn pushed_commits(repo: &Path) -> std::collections::HashSet<String> {
    git(repo, &["rev-list", "--remotes"])
        .map(|raw| raw.lines().map(str::to_string).collect())
        .unwrap_or_default()
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
        let (old_path, path) = match status {
            'R' | 'C' => {
                let old = cols.next();
                (old.map(str::to_string), cols.next())
            }
            _ => (None, cols.next()),
        };
        if let Some(path) = path {
            changes.push(FileChange {
                status,
                path: path.to_string(),
                old_path,
            });
        }
    }
    changes
}
