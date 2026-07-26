//! Git access for the interval spine. We shell out to `git` on purpose:
//! its output is itself the receipt, and v0.1 needs nothing libgit2 offers.

use std::collections::{HashMap, HashSet};
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

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
    /// Who git records as the author: "Name <email>". Identity, not
    /// authorship method — a name never tells you agent-vs-hand-coded.
    pub author: String,
    /// `Co-Authored-By:` trailer values from the commit message, e.g.
    /// "Claude Opus 4.8 <noreply@anthropic.com>". Present-only evidence of
    /// co-authorship (an agent, a pair); absence proves nothing.
    pub co_authors: Vec<String>,
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

/// Split the `Co-Authored-By` trailer values (git joins them with 0x1f).
fn parse_co_authors(raw: &str) -> Vec<String> {
    raw.split('\u{1f}')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
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
            "--format=%H%x00%h%x00%cI%x00%gd%x00%P%x00%gs%x00%an%x00%ae%x00%(trailers:key=Co-authored-by,valueonly,separator=%x1f)%x00%s",
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
            Some(aname),
            Some(aemail),
            Some(co_raw),
            Some(subject),
        ) = (
            parts.next(),
            parts.next(),
            parts.next(),
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
                author: format!("{aname} <{aemail}>"),
                co_authors: parse_co_authors(co_raw),
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
        &[
            "log",
            "--all",
            "--format=%H%x00%h%x00%cI%x00%P%x00%an%x00%ae%x00%(trailers:key=Co-authored-by,valueonly,separator=%x1f)%x00%s",
        ],
    )?;
    let mut extra: Vec<SpineCommit> = Vec::new();
    for line in hist_raw.lines() {
        let mut parts = line.split(NUL);
        let (
            Some(hash),
            Some(short),
            Some(cdate),
            Some(parents),
            Some(aname),
            Some(aemail),
            Some(co_raw),
            Some(subject),
        ) = (
            parts.next(),
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
            author: format!("{aname} <{aemail}>"),
            co_authors: parse_co_authors(co_raw),
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

/// Commits reachable from any remote-tracking ref — i.e. pushed, as of the
/// repo's last fetch. A local-only commit is absent. This is a local check
/// (no network), so it reflects the last-known remote state, not live.
pub fn pushed_commits(repo: &Path) -> std::collections::HashSet<String> {
    git(repo, &["rev-list", "--remotes"])
        .map(|raw| raw.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// Of `paths`, the subset matched by the repo's ignore rules — in ONE
/// `git check-ignore --stdin` instead of a spawn per path. Paths originate in
/// the untrusted session file; feeding them over stdin (never as argv) means a
/// path like `--stdin` can't be parsed as a git option, and `-z` uses NUL
/// separators both ways so odd characters aren't quoted or split.
pub fn ignored_paths(repo: &Path, paths: &[String]) -> HashSet<String> {
    if paths.is_empty() {
        return HashSet::new();
    }
    let mut child = match Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["check-ignore", "-z", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return HashSet::new(),
    };
    if let Some(mut stdin) = child.stdin.take() {
        for p in paths {
            let _ = stdin.write_all(p.as_bytes());
            let _ = stdin.write_all(&[0]);
        }
    }
    let out = match child.wait_with_output() {
        Ok(o) => o,
        Err(_) => return HashSet::new(),
    };
    String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Is this single repo-relative path matched by the repo's ignore rules? For
/// the rare per-claim diagnosis checks; the bulk residue pass uses
/// [`ignored_paths`]. `rel_path` comes from the untrusted session file, so `--`
/// keeps one like `--stdin` from being parsed as a git option.
pub fn is_ignored(repo: &Path, rel_path: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["check-ignore", "-q", "--", rel_path])
        .stdin(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Paths currently tracked by git (the index). A residue file that is no
/// longer here was later untracked or deleted — yesterday's noise, not
/// today's problem.
pub fn tracked_paths(repo: &Path) -> Result<HashSet<String>> {
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

/// The statements (name-status vs first parent) for MANY commits in a single
/// `git show`, keyed by full hash — the batched form of [`commit_names`], one
/// subprocess instead of one per commit. A record-separator (`\x1e`) before each
/// commit's `%H` lets us split the output back into per-commit blocks; the
/// `parse_name_status` line filter ignores the marker/blank lines.
pub fn commit_statements(
    repo: &Path,
    hashes: &[String],
) -> Result<HashMap<String, Vec<FileChange>>> {
    let mut map = HashMap::new();
    if hashes.is_empty() {
        return Ok(map);
    }
    let mut args: Vec<&str> = vec![
        "show",
        "--name-status",
        "-M",
        "--first-parent",
        "--format=%x1e%H",
    ];
    args.extend(hashes.iter().map(String::as_str));
    let raw = git(repo, &args)?;
    for chunk in raw.split('\u{1e}') {
        let mut parts = chunk.splitn(2, '\n');
        let hash = parts.next().unwrap_or("").trim();
        if hash.len() != 40 {
            continue; // the empty leading chunk / anything malformed
        }
        map.insert(
            hash.to_string(),
            parse_name_status(parts.next().unwrap_or("")),
        );
    }
    Ok(map)
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
