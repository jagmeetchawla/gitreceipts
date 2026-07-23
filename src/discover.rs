//! Finding the session log and the repo when they don't line up.
//!
//! A session is recorded under the directory the agent was LAUNCHED in —
//! which is often not the repo: monorepo parents, container directories
//! holding several repos, `..`-launched sessions. Discovery searches the
//! session store for the repo's whole ancestor chain, and when no repo was
//! named, infers it from where the session's claims actually point.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::causal;
use crate::extract::{Action, Session};
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

/// The default store: `~/.claude`.
pub fn default_store() -> Option<PathBuf> {
    std::env::home_dir().map(|h| h.join(".claude"))
}

/// Candidate session directories for a repo inside a given store's
/// `projects/` dir: the repo's own encoding plus every ancestor (the
/// session may have been launched anywhere above the repo).
///
/// When nothing matches exactly — typical for a store mounted from
/// another machine, whose encoded names carry THAT machine's absolute
/// paths — fall back to matching project dirs whose encoded name ends
/// with an ancestor's directory name ("…-myapp" for a repo at any
/// /Volumes/mount/…/myapp), and say so.
pub fn session_dirs_for(store: &Path, repo: &Path) -> Vec<PathBuf> {
    let projects = store.join("projects");
    let start = repo.canonicalize().unwrap_or_else(|_| repo.to_path_buf());

    let exact: Vec<PathBuf> = start
        .ancestors()
        .map(|a| projects.join(encode(a)))
        .filter(|d| d.is_dir())
        .collect();
    if !exact.is_empty() {
        return exact;
    }

    // cross-machine fallback: suffix-match on directory basenames
    let suffixes: Vec<String> = start
        .ancestors()
        .filter_map(|a| a.file_name())
        .filter_map(|n| n.to_str())
        .filter(|n| n.len() >= 3)
        .map(|n| format!("-{}", encode(Path::new(n))))
        .collect();
    let Ok(entries) = std::fs::read_dir(&projects) else {
        return Vec::new();
    };
    let matched: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
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

/// Newest .jsonl across the candidate directories.
pub fn latest_session(store: &Path, repo: &Path) -> Result<PathBuf> {
    let dirs = session_dirs_for(store, repo);
    if dirs.is_empty() {
        bail!(
            "no session directories found for {} (or any parent) under {}",
            repo.display(),
            store.join("projects").display()
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
            store.join("projects").display()
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

/// No `--repo` given: infer it from the session itself.
///
/// Candidates are every git repo the session could mean: each recorded cwd
/// that is a repo, plus each immediate child of a recorded cwd that is a
/// repo (the container-directory layout). The winner is the candidate the
/// most file claims resolve under. A tie or an empty field is an error
/// that names the candidates — guessing wrong would audit the wrong repo.
pub fn infer_repo(session: &Session) -> Result<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let mut push = |p: PathBuf| {
        if p.join(".git").exists()
            && let Ok(canon) = p.canonicalize()
            && !candidates.contains(&canon)
        {
            candidates.push(canon);
        }
    };
    for cwd in &session.cwds {
        let cwd = PathBuf::from(cwd);
        push(cwd.clone());
        if let Ok(children) = std::fs::read_dir(&cwd) {
            for child in children.flatten() {
                let path = child.path();
                if path.is_dir() {
                    push(path);
                }
            }
        }
    }
    if candidates.is_empty() {
        bail!("the session's recorded directories contain no git repo; pass --repo");
    }
    if candidates.len() == 1 {
        return Ok(candidates.remove(0));
    }

    let mut scores: HashMap<&Path, usize> = HashMap::new();
    for claim in &session.claims {
        if let Action::FileMutation { path, .. } = &claim.action {
            // longest matching candidate wins the claim
            let best = candidates
                .iter()
                .filter(|c| Path::new(path).starts_with(c))
                .max_by_key(|c| c.as_os_str().len());
            if let Some(best) = best {
                *scores.entry(best.as_path()).or_default() += 1;
            }
        }
    }
    let mut ranked: Vec<(&Path, usize)> = candidates
        .iter()
        .map(|c| (c.as_path(), scores.get(c.as_path()).copied().unwrap_or(0)))
        .collect();
    ranked.sort_by_key(|(_, n)| std::cmp::Reverse(*n));

    match ranked.as_slice() {
        [(top, n), (_, m), ..] if *n > *m => {
            eprintln!(
                "note: repo inferred from the session's claims: {} ({n} file claims resolve here; pass --repo to override)",
                top.display()
            );
            Ok(top.to_path_buf())
        }
        _ => bail!(
            "the session touches several git repos and the claims don't single one out — pass --repo. candidates:\n{}",
            ranked
                .iter()
                .map(|(p, n)| format!("  {} ({n} file claims)", p.display()))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    }
}
