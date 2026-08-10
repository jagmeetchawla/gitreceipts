//! Finding the session log and the repo when they don't line up.
//!
//! A session is recorded under the directory the agent was LAUNCHED in —
//! which is often not the repo: monorepo parents, container directories
//! holding several repos, `..`-launched sessions. Discovery searches the
//! session store for the repo's whole ancestor chain, and when no repo was
//! named, infers it from where the session's claims actually point.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::causal;
use crate::ingest::{self, IngestStats};
use crate::schema::Record;

/// The session store encodes a launch directory by replacing path
/// separators (and dots) with dashes.
fn encode(path: &Path) -> String {
    path.display()
        .to_string()
        .chars()
        .map(|c| if c == '/' || c == '.' { '-' } else { c })
        .collect()
}

/// The default store: the **Claude Code projects directory**,
/// `~/.claude/projects`. This is the ONLY directory gitreceipts ever reads —
/// the per-project session logs live here. It deliberately does NOT point at
/// `~/.claude`, so the tool never touches your settings, prompt history, MCP
/// auth caches, or anything else in that directory.
pub fn default_store() -> Option<PathBuf> {
    std::env::home_dir().map(|h| h.join(".claude").join("projects"))
}

/// Candidate session directories for a repo within the store (the Claude Code
/// projects directory): the repo's own encoding plus every ancestor (the
/// session may have been launched anywhere above the repo).
///
/// When nothing matches exactly — typical for a store mounted from
/// another machine, whose encoded names carry THAT machine's absolute
/// paths — fall back to matching project dirs whose encoded name ends
/// with an ancestor's directory name ("…-myapp" for a repo at any
/// /Volumes/mount/…/myapp), and say so.
/// Does this store directory hold at least one session file? Directory
/// existence is not the same question: the agent creates a directory for
/// any folder it is opened in, so plenty of them are empty shells.
fn holds_sessions(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries
        .flatten()
        .any(|e| e.path().extension().and_then(|x| x.to_str()) == Some("jsonl"))
}

pub fn session_dirs_for(store: &Path, repo: &Path) -> Vec<PathBuf> {
    let start = repo.canonicalize().unwrap_or_else(|_| repo.to_path_buf());

    // A directory that exists but holds no .jsonl is an empty shell — the
    // agent opened that folder once and recorded nothing. Treating it as a
    // hit made three unlike things go wrong: `--project` aborted the whole
    // roll-up on one such repo, the repo appeared in neither the audited
    // list nor the named no-sessions list, and the empty shell short-
    // circuited the cross-machine fallback below. Every caller means "has
    // usable sessions", so answer that question here, once.
    let exact: Vec<PathBuf> = start
        .ancestors()
        .map(|a| store.join(encode(a)))
        .filter(|d| holds_sessions(d))
        .collect();
    if !exact.is_empty() {
        return exact;
    }

    // Cross-machine fallback: suffix-match on the REPO's own basename only.
    // Matching every ancestor's name over-matched (`/data/dev` produced the
    // suffix `-dev`, which "matched" a store dir for `rusticplayground-dev`
    // — a different repo). The repo directory's name is the one identity
    // worth trusting across machines.
    let suffixes: Vec<String> = start
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| n.len() >= 3)
        .map(|n| format!("-{}", encode(Path::new(n))))
        .into_iter()
        .collect();
    let Ok(entries) = std::fs::read_dir(store) else {
        return Vec::new();
    };
    let matched: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| holds_sessions(p))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| suffixes.iter().any(|s| name.ends_with(s.as_str())))
        })
        .collect();
    for m in &matched {
        eprintln!(
            "note: no exact session directory for {} in this store; matched {} by name (a store recorded on another machine encodes that machine's paths)",
            repo.display(),
            m.display()
        );
    }
    matched
}

/// The git repos under a PROJECT folder that have sessions — the repos a
/// `--project` audit will report. Walks the folder (bounded depth), treats any
/// directory with a `.git` entry (dir OR file, so worktrees count) as a repo and
/// does not descend into it, then keeps only those the store has sessions for.
/// Sorted. A project folder that is itself a git repo yields just itself (the
/// monorepo case → project ≡ repo).
pub fn project_repos(project: &Path, store: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        if dir.join(".git").exists() {
            out.push(dir.to_path_buf());
            return; // a repo — don't descend into it (skip nested repos)
        }
        if depth == 0 {
            return;
        }
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut sub: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            sub.sort();
            for p in sub {
                walk(&p, depth - 1, out);
            }
        }
    }
    let start = project
        .canonicalize()
        .unwrap_or_else(|_| project.to_path_buf());
    let mut repos = Vec::new();
    walk(&start, 5, &mut repos);
    repos.retain(|r| !session_dirs_for(store, r).is_empty());
    repos
}

/// Repos under a project folder that have NO sessions in the store — the
/// ones `project_repos` filters out. They are still part of the project,
/// so the roll-up names them rather than dropping them: a table whose row
/// count differs from what is on disk looks complete and isn't.
pub fn project_repos_without_sessions(project: &Path, store: &Path) -> Vec<PathBuf> {
    let start = project
        .canonicalize()
        .unwrap_or_else(|_| project.to_path_buf());
    let mut all = Vec::new();
    walk_repos(&start, 5, &mut all);
    all.retain(|r| session_dirs_for(store, r).is_empty());
    all
}

/// Why a `--project` folder yielded nothing — so the error can say which
/// of three unlike problems the user actually has (a typo'd path, a folder
/// with no repos, or repos whose sessions this store doesn't hold).
pub enum ProjectMiss {
    NoSuchDir,
    NotADir,
    NoRepos,
    /// Repos are there; none has sessions in this store.
    NoSessions(Vec<PathBuf>),
}

pub fn diagnose_project(project: &Path, store: &Path) -> ProjectMiss {
    if !project.exists() {
        return ProjectMiss::NoSuchDir;
    }
    if !project.is_dir() {
        return ProjectMiss::NotADir;
    }
    let start = project
        .canonicalize()
        .unwrap_or_else(|_| project.to_path_buf());
    let mut repos = Vec::new();
    walk_repos(&start, 5, &mut repos);
    if repos.is_empty() {
        ProjectMiss::NoRepos
    } else {
        let _ = store;
        ProjectMiss::NoSessions(repos)
    }
}

/// A directory with this basename near the current one — checked against
/// the cwd's ancestors and their immediate children. Turns a "no such
/// directory: gitreceipts" dead end into the path the user meant, which
/// is nearly always a sibling or an ancestor of where they are standing.
pub fn nearby_dir_named(name: &Path) -> Option<PathBuf> {
    let name = name.file_name()?;
    let cwd = std::env::current_dir().ok()?;
    for anc in cwd.ancestors().take(6) {
        if anc.file_name() == Some(name) {
            return Some(anc.to_path_buf());
        }
        let candidate = anc.join(name);
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    None
}

/// Repos under a folder, ignoring whether the store has sessions for them.
fn walk_repos(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if dir.join(".git").exists() {
        out.push(dir.to_path_buf());
        return;
    }
    if depth == 0 {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut sub: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        sub.sort();
        for p in sub {
            walk_repos(&p, depth - 1, out);
        }
    }
}

/// Newest .jsonl across the candidate directories.
pub fn latest_session(store: &Path, repo: &Path) -> Result<PathBuf> {
    let dirs = session_dirs_for(store, repo);
    if dirs.is_empty() {
        bail!(
            "no session directories found for {} (or any parent) under {}",
            repo.display(),
            store.display()
        );
    }
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    for dir in &dirs {
        let entries = std::fs::read_dir(dir)
            .with_context(|| format!("read session dir {}", dir.display()))?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let modified = entry.metadata()?.modified()?;
            if newest.as_ref().is_none_or(|(t, _)| modified > *t) {
                newest = Some((modified, path));
            }
        }
    }
    newest.map(|(_, p)| p).with_context(|| {
        format!(
            "no .jsonl sessions in {}",
            dirs.iter()
                .map(|d| d.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })
}

/// Every session the store still has for this repo (and its ancestors),
/// oldest first. There is no local "archive": sessions live here until
/// the store's retention cleanup removes them — history older than that
/// is simply gone, and commits from it will show as unclaimed keyframes.
pub fn all_sessions(store: &Path, repo: &Path) -> Result<Vec<PathBuf>> {
    let dirs = session_dirs_for(store, repo);
    if dirs.is_empty() {
        bail!(
            "no session directories found for {} (or any parent) under {}",
            repo.display(),
            store.display()
        );
    }
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for dir in &dirs {
        for entry in
            std::fs::read_dir(dir).with_context(|| format!("read session dir {}", dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                found.push((entry.metadata()?.modified()?, path));
            }
        }
    }
    if found.is_empty() {
        bail!(
            "no .jsonl sessions found for {} or its parents",
            repo.display()
        );
    }
    found.sort_by_key(|(t, _)| *t);
    Ok(found.into_iter().map(|(_, p)| p).collect())
}

/// Ingest several session files into one causally ordered record stream.
///
/// Sessions can be forks of one conversation sharing a common prefix of
/// identical events — records are deduplicated by uuid so a forked
/// session cannot double-count its claims. Files merge oldest-first, so
/// the first occurrence of a shared event wins.
pub fn merge_sessions(paths: &[PathBuf]) -> Result<(Vec<Record>, IngestStats)> {
    let mut per_file: Vec<(Vec<Record>, IngestStats)> = Vec::new();
    for path in paths {
        let (records, stats) = ingest::ingest(path)?;
        per_file.push((causal::order(records), stats));
    }
    // oldest session first, by first event timestamp
    per_file.sort_by_key(|(records, _)| {
        records
            .iter()
            .find_map(|r| r.timestamp.clone())
            .unwrap_or_default()
    });

    let mut merged_stats = IngestStats::default();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut merged: Vec<Record> = Vec::new();
    for (records, stats) in per_file {
        merged_stats.lines += stats.lines;
        merged_stats.kept += stats.kept;
        merged_stats.skipped_types += stats.skipped_types;
        merged_stats.unparseable += stats.unparseable;
        for rec in records {
            match &rec.uuid {
                Some(id) => {
                    if seen.insert(id.clone()) {
                        merged.push(rec);
                    } else {
                        merged_stats.duplicates += 1;
                    }
                }
                None => merged.push(rec),
            }
        }
    }
    Ok((merged, merged_stats))
}

/// Find the ONE session file that contains `marker` — identity-based
/// targeting for `--this-session`. The caller has just emitted the marker
/// into its own live conversation, so exactly one file should carry it.
/// Session files flush asynchronously; one short retry absorbs write lag.
pub fn session_containing(store: &Path, marker: &str) -> anyhow::Result<PathBuf> {
    for attempt in 0..2 {
        let mut hits: Vec<PathBuf> = Vec::new();
        if let Ok(projects) = std::fs::read_dir(store) {
            for proj in projects.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
                if let Ok(files) = std::fs::read_dir(&proj) {
                    for f in files.flatten().map(|e| e.path()) {
                        if f.extension().and_then(|e| e.to_str()) == Some("jsonl")
                            && std::fs::read_to_string(&f).is_ok_and(|c| c.contains(marker))
                        {
                            hits.push(f);
                        }
                    }
                }
            }
        }
        match hits.len() {
            1 => return Ok(hits.remove(0)),
            0 if attempt == 0 => std::thread::sleep(std::time::Duration::from_secs(1)),
            0 => anyhow::bail!(
                "no session in {} contains the marker — echo it into the conversation first, then run this",
                store.display()
            ),
            _ => {
                hits.sort();
                anyhow::bail!(
                    "{} sessions contain the marker (it must be unique): {}",
                    hits.len(),
                    hits.last().unwrap().display()
                );
            }
        }
    }
    unreachable!()
}

/// Resolve a folder to THE repo it names — the one rule used both for
/// `--repo <dir>` and for the bare invocation's cwd.
///
/// The folder must itself contain `.git`. Nothing else resolves — not an
/// ancestor, not a child, not even a lone child. Repos found below are
/// NAMED in the error so the next command is obvious, but choosing one is
/// the user's call; `--project` is the switch for "all of them".
pub fn resolve_repo(folder: &Path) -> Result<PathBuf> {
    if !folder.exists() {
        bail!("no such directory: {}", folder.display());
    }
    if !folder.is_dir() {
        bail!("{} is a file, not a directory", folder.display());
    }
    // Errors name the real path: "." tells the reader nothing in a
    // scrollback, and --repo/--project both default to it.
    let shown = folder
        .canonicalize()
        .unwrap_or_else(|_| folder.to_path_buf());
    let folder = &shown;
    if folder.join(".git").exists() {
        return Ok(folder.to_path_buf());
    }

    let mut children: Vec<PathBuf> = std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir() && p.join(".git").exists())
                .collect()
        })
        .unwrap_or_default();
    children.sort();

    match children.len() {
        0 => bail!(
            "{} is not a git repo.\n       \
             Run this from a git repo, or name one with --repo <dir>.\n       \
             To audit every repo under a folder: --project <dir>",
            folder.display()
        ),
        n => {
            // No .git HERE, but repos below: by definition this folder is a
            // container, not a repo — a folder holds at most one .git, so
            // multiplicity only ever appears as separate subfolders. That
            // shape is what --project is for, whether it holds one repo or
            // ten. Name what was found; never choose.
            let names: Vec<String> = children
                .iter()
                .take(6)
                .map(|p| {
                    p.file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("?")
                        .to_string()
                })
                .collect();
            bail!(
                "{} is not a git repo. It holds {} ({}{}) — name one with \
                 --repo <dir>, or audit them all with --project {}",
                folder.display(),
                if n == 1 {
                    "1 repo".to_string()
                } else {
                    format!("{n} repos")
                },
                names.join(", "),
                if n > 6 { ", …" } else { "" },
                folder.display()
            )
        }
    }
}
