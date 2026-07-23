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

use crate::extract::{Action, Session};

/// The session store encodes a launch directory by replacing path
/// separators (and dots) with dashes.
fn encode(path: &Path) -> String {
    path.display()
        .to_string()
        .chars()
        .map(|c| if c == '/' || c == '.' { '-' } else { c })
        .collect()
}

/// Candidate session directories for a repo: the repo's own encoding plus
/// every ancestor up to (and including) the home directory — the session
/// may have been launched anywhere above the repo.
pub fn session_dirs_for(repo: &Path) -> Vec<PathBuf> {
    let Some(home) = std::env::home_dir() else {
        return Vec::new();
    };
    let store = home.join(".claude").join("projects");
    let start = repo.canonicalize().unwrap_or_else(|_| repo.to_path_buf());
    start
        .ancestors()
        .take_while(|a| a.starts_with(&home))
        .map(|a| store.join(encode(a)))
        .filter(|d| d.is_dir())
        .collect()
}

/// Newest .jsonl across the candidate directories.
pub fn latest_session(repo: &Path) -> Result<PathBuf> {
    let dirs = session_dirs_for(repo);
    if dirs.is_empty() {
        bail!(
            "no session directories found for {} (or any parent) under ~/.claude/projects",
            repo.display()
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
