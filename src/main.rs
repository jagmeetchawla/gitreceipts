//! `git receipts` — see what your agent actually did.

use std::path::PathBuf;

use gitreceipts::{causal, extract, ingest, reconcile, report};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "git-receipts",
    version,
    about = "See what your agent actually did."
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Audit a Claude Code session against its git repo.
    Audit {
        /// Path to the session .jsonl (or use --latest).
        session: Option<PathBuf>,
        /// Audit the most recent session for the repo.
        #[arg(long)]
        latest: bool,
        /// Repo to reconcile against (default: cwd recorded in the session).
        #[arg(long)]
        repo: Option<PathBuf>,
        /// When to emit ANSI colors. `always` keeps them through pipes,
        /// for `| bat`, `| less -R`, or saving a colored transcript.
        #[arg(long, value_enum, default_value_t = report::ColorMode::Auto)]
        color: report::ColorMode,
        /// Suppress the quoted prompt text on intent lines (counts stay).
        /// Prompts are where pasted secrets live; use this before sharing
        /// a report from a session you don't fully remember.
        #[arg(long)]
        no_intent: bool,
        /// Which intervals to list: all, red (broken promises only), or
        /// red-residue (red plus unclaimed-change intervals).
        #[arg(long, value_enum, default_value_t = report::Filter::All)]
        filter: report::Filter,
    },
}

fn main() -> Result<()> {
    // A report exists to be piped (`| head`, `| less` quit early). Restore
    // default SIGPIPE so a closed pipe ends the process quietly instead of
    // panicking mid-print.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let cli = Cli::parse();
    match cli.command {
        Cmd::Audit {
            session,
            latest,
            repo,
            color,
            no_intent,
            filter,
        } => audit(
            session,
            latest,
            repo,
            report::Options {
                color,
                show_intent: !no_intent,
                filter,
            },
        ),
    }
}

/// The `~/.claude/projects` directory encodes a repo path by replacing
/// path separators (and dots) with dashes.
fn sessions_dir_for(repo: &std::path::Path) -> Option<PathBuf> {
    let home = std::env::home_dir()?;
    let canon = repo.canonicalize().ok()?;
    let encoded: String = canon
        .display()
        .to_string()
        .chars()
        .map(|c| if c == '/' || c == '.' { '-' } else { c })
        .collect();
    Some(home.join(".claude").join("projects").join(encoded))
}

fn latest_session(repo: &std::path::Path) -> Result<PathBuf> {
    let dir = sessions_dir_for(repo).context("cannot locate ~/.claude/projects")?;
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(&dir)
        .with_context(|| format!("no sessions found for this repo at {}", dir.display()))?
    {
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
    newest
        .map(|(_, p)| p)
        .with_context(|| format!("no .jsonl sessions in {}", dir.display()))
}

fn audit(
    session: Option<PathBuf>,
    latest: bool,
    repo: Option<PathBuf>,
    opts: report::Options,
) -> Result<()> {
    let session_path = match (session, latest) {
        (Some(p), false) => p,
        (None, true) => {
            let repo = repo
                .clone()
                .or_else(|| std::env::current_dir().ok())
                .context("cannot determine repo for --latest")?;
            latest_session(&repo)?
        }
        (Some(_), true) => bail!("pass a session path or --latest, not both"),
        (None, false) => bail!("pass a session path or --latest"),
    };

    let (records, stats) = ingest::ingest(&session_path)?;
    if records.is_empty() {
        bail!("no execution events in {}", session_path.display());
    }
    let ordered = causal::order(records);
    let session_data = extract::extract(&ordered);

    let repo_path = match repo {
        Some(r) => r,
        None => PathBuf::from(
            session_data
                .cwds
                .first()
                .context("session records no cwd; pass --repo")?,
        ),
    };
    if !repo_path.join(".git").exists() {
        bail!(
            "{} is not a git repo (the session's original cwd may have moved; pass --repo)",
            repo_path.display()
        );
    }

    let audit = reconcile::reconcile(&repo_path, &session_data)?;
    let name = session_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("session");
    report::print(
        name,
        &repo_path.display().to_string(),
        &session_data,
        &stats,
        &audit,
        &opts,
    );
    Ok(())
}
