//! The `audit` subcommand: discover the sessions and repo, merge and
//! extract, reconcile, then render (console, optionally paged, or HTML).
//!
//! [`load`] is the shared pipeline up to the reconciled [`Audit`]; both this
//! subcommand and `export` build on it, differing only in how they present
//! the result.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use gitreceipts::extract::Session;
use gitreceipts::ingest::IngestStats;
use gitreceipts::reconcile::Audit;
use gitreceipts::{discover, extract, html, reconcile, report};

use crate::pager::{finish_pager, start_pager};

/// A one-line progress note on stderr for the slow part (discover + reconcile
/// against git, seconds on a long session). Cleared when dropped — including on
/// the `?` error path — so it never collides with the report on stdout. No-op
/// unless stderr is a terminal, so pipes and redirects stay clean.
pub struct Status(bool);

impl Status {
    pub fn show(msg: &str) -> Self {
        let on = std::io::stderr().is_terminal();
        if on {
            eprint!("\r\x1b[2K{msg}");
            let _ = std::io::stderr().flush();
        }
        Status(on)
    }
}

impl Drop for Status {
    fn drop(&mut self) {
        if self.0 {
            eprint!("\r\x1b[2K");
            let _ = std::io::stderr().flush();
        }
    }
}

/// A completed audit plus the header context every presenter needs.
pub struct Loaded {
    /// Display name for the report header.
    pub name: String,
    /// The repo path, as a string for display.
    pub repo_display: String,
    /// Which agent produced the session (reserved for future parser selection;
    /// today always Claude). Recorded in the receipt's `source.agent`.
    pub agent: report::Agent,
    pub session: Session,
    pub stats: IngestStats,
    pub audit: Audit,
}

/// Resolve a commit ref (a short or full hash prefix, `lspci -s` style) to the
/// unique full hash of a spine interval, or fail with a helpful message.
pub fn resolve_commit(audit: &Audit, needle: &str) -> Result<String> {
    let needle = needle.to_ascii_lowercase();
    let matches: Vec<&str> = audit
        .intervals
        .iter()
        .map(|i| i.commit.hash.as_str())
        .filter(|h| h.starts_with(&needle))
        .collect();
    match matches.as_slice() {
        [] => bail!(
            "no commit matching '{needle}' in the audited spine — run `git receipts audit --oneline` to see the commits"
        ),
        [one] => Ok((*one).to_string()),
        many => bail!(
            "'{needle}' is ambiguous — it matches {} commits; use more of the hash",
            many.len()
        ),
    }
}

/// Walk up from `dir` to the nearest directory containing `.git`.
fn find_enclosing_repo(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .find(|a| a.join(".git").exists())
        .map(|a| a.to_path_buf())
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    sessions: Vec<PathBuf>,
    latest: bool,
    all: bool,
    repo: Option<PathBuf>,
    project: Option<PathBuf>,
    store: Option<PathBuf>,
    no_pager: bool,
    mut opts: report::Options,
    commit: Option<String>,
    redact: Vec<String>,
    scan: bool,
    agent: report::Agent,
) -> Result<()> {
    // Project mode: one header, then each repo under the folder reported in turn.
    if let Some(project) = project {
        return run_project(
            project, sessions, latest, all, store, no_pager, opts, redact, scan, agent,
        );
    }
    let loaded = {
        let _status = Status::show("git receipts: auditing… reconciling against git");
        load(sessions, latest, all, repo, store, &redact, scan, agent)?
    };
    let Loaded {
        name,
        repo_display,
        session,
        stats,
        audit,
        // agent is recorded in the receipt (export); the console/HTML views
        // don't surface it, so ignore it here.
        agent: _,
    } = &loaded;

    // Resolve --commit against the actual spine (fails on unknown/ambiguous).
    if let Some(needle) = &commit {
        opts.commit = Some(resolve_commit(audit, needle)?);
    }

    match opts.format {
        report::Format::Text => {
            // On a terminal, page through $PAGER (git's behavior). The pager
            // redirects stdout to a pipe, so auto color would strip itself —
            // force color on for the pager unless explicitly off.
            let pager = start_pager(no_pager);
            if pager.is_some() && opts.color == report::ColorMode::Auto {
                opts.color = report::ColorMode::Always;
            }
            report::print(name, repo_display, session, stats, audit, &opts);
            finish_pager(pager);
        }
        report::Format::Html => print!(
            "{}",
            html::render(
                name,
                repo_display,
                session,
                stats,
                audit,
                opts.show,
                opts.show_identity,
                opts.expand,
                opts.with_output,
                opts.commit.as_deref(),
                opts.full,
            )
        ),
    }
    Ok(())
}

/// Project mode: report every git repo under the project folder in turn — one
/// header, then each repo's own console report under a banner. Phase 1: console
/// only; each repo reuses the single-repo pipeline (`load` + `report::print`).
#[allow(clippy::too_many_arguments)]
fn run_project(
    project: PathBuf,
    sessions: Vec<PathBuf>,
    latest: bool,
    all: bool,
    store: Option<PathBuf>,
    no_pager: bool,
    mut opts: report::Options,
    redact: Vec<String>,
    scan: bool,
    agent: report::Agent,
) -> Result<()> {
    if opts.format == report::Format::Html {
        bail!("--project --format html is not supported yet — use the console view for now");
    }
    if !sessions.is_empty() {
        bail!("--project audits the folder's own sessions; don't also pass session file(s)");
    }
    let store = resolve_store(store)?;
    let repos = discover::project_repos(&project, &store);
    if repos.is_empty() {
        bail!(
            "no git repos with sessions found under {} — audit a single repo with --repo <dir>, or check the folder/store",
            project.display()
        );
    }

    let pager = start_pager(no_pager);
    if pager.is_some() && opts.color == report::ColorMode::Auto {
        opts.color = report::ColorMode::Always;
    }
    println!(
        "git receipts — project {}",
        gitreceipts::fmt::tilde(&project.display().to_string())
    );
    println!(
        "{} git repo{} with sessions under this project",
        repos.len(),
        if repos.len() == 1 { "" } else { "s" }
    );
    for repo in &repos {
        let name = repo
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("(repo)");
        println!("\n═══ {name} ═══════════════════════════════════════════════");
        let loaded = load(
            Vec::new(),
            latest,
            all,
            Some(repo.clone()),
            Some(store.clone()),
            &redact,
            scan,
            agent,
        )?;
        report::print(
            &loaded.name,
            &loaded.repo_display,
            &loaded.session,
            &loaded.stats,
            &loaded.audit,
            &opts,
        );
    }
    finish_pager(pager);
    Ok(())
}

/// Resolve the session store: `--store` if given (with the old-`.claude`-dir
/// nudge), else the default `~/.claude/projects`.
pub fn resolve_store(store: Option<PathBuf>) -> Result<PathBuf> {
    match store {
        Some(s) => {
            if !s.is_dir() {
                bail!("--store {} is not a directory", s.display());
            }
            // Nudge for the old habit: a dir that CONTAINS projects/ is the
            // `.claude` dir, not the projects dir the store now points at.
            if s.join("projects").is_dir() {
                bail!(
                    "--store points at the Claude Code projects directory itself now, not .claude — did you mean {}?",
                    s.join("projects").display()
                );
            }
            Ok(s)
        }
        None => discover::default_store()
            .context("cannot locate the home directory (default store: ~/.claude/projects)"),
    }
}

/// Run the pipeline for ONE repo — discover sessions and repo, merge, extract,
/// reconcile — and return the reconciled audit with its header context.
#[allow(clippy::too_many_arguments)]
pub fn load(
    sessions: Vec<PathBuf>,
    latest: bool,
    all: bool,
    repo: Option<PathBuf>,
    store: Option<PathBuf>,
    redact: &[String],
    scan: bool,
    agent: report::Agent,
) -> Result<Loaded> {
    let store = resolve_store(store)?;
    let anchor = || {
        repo.clone()
            .or_else(|| std::env::current_dir().ok())
            .context("cannot determine a directory to search for sessions")
    };
    // Session selection. The DEFAULT (no session, no flags) is ALL of the repo's
    // sessions — the complete picture, and it avoids the single-session trap
    // where your OTHER sessions' commits look like another contributor's
    // (inflated residue/keyframes). `--latest` is the deliberate one-session
    // shortcut; `--all` is kept as an explicit synonym for the default.
    let session_paths: Vec<PathBuf> = if !sessions.is_empty() {
        if latest || all {
            bail!("pass session path(s) OR --latest/--all, not both");
        }
        sessions
    } else if latest {
        if all {
            bail!("--latest and --all are mutually exclusive");
        }
        vec![discover::latest_session(&store, &anchor()?)?]
    } else {
        // default and --all: every session the store has for this repo
        discover::all_sessions(&store, &anchor()?)?
    };

    let (ordered, stats) = discover::merge_sessions(&session_paths)?;
    if ordered.is_empty() {
        bail!("no execution events in the given session(s)");
    }
    let session_data = extract::extract(&ordered);

    // Establish redaction from the LOG's own working directories (so the OS
    // username is masked wherever it appears, even auditing another machine's
    // sessions) plus any words the user asked to mask. Must precede any render.
    gitreceipts::fmt::set_redaction(&session_data.cwds, redact, scan);

    let repo_path = match repo {
        Some(r) => {
            if !r.join(".git").exists() {
                bail!("{} is not a git repo", r.display());
            }
            r
        }
        // No --repo: prefer the repo we're standing in; otherwise infer
        // from where the session's claims point.
        None => match std::env::current_dir()
            .ok()
            .and_then(|d| find_enclosing_repo(&d))
        {
            Some(here) => here,
            None => discover::infer_repo(&session_data)?,
        },
    };

    let audit = reconcile::reconcile(&repo_path, &session_data)?;
    Ok(Loaded {
        name: session_name(&session_paths),
        repo_display: repo_path.display().to_string(),
        agent,
        session: session_data,
        stats,
        audit,
    })
}

/// A display name for the report header: the session stem, or a summary
/// when several merged.
fn session_name(paths: &[PathBuf]) -> String {
    let short = |p: &PathBuf| {
        p.file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.chars().take(8).collect::<String>())
            .unwrap_or_else(|| "session".to_string())
    };
    match paths {
        [one] => one
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("session")
            .to_string(),
        many => format!(
            "{} sessions merged ({})",
            many.len(),
            many.iter().map(short).collect::<Vec<_>>().join(", ")
        ),
    }
}
