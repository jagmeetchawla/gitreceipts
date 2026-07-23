//! `git receipts` — see what your agent actually did.

use std::path::PathBuf;

use gitreceipts::{causal, discover, extract, ingest, reconcile, report};

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

/// Walk up from `dir` to the nearest directory containing `.git`.
fn find_enclosing_repo(dir: &std::path::Path) -> Option<PathBuf> {
    dir.ancestors()
        .find(|a| a.join(".git").exists())
        .map(|a| a.to_path_buf())
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

fn audit(
    session: Option<PathBuf>,
    latest: bool,
    repo: Option<PathBuf>,
    opts: report::Options,
) -> Result<()> {
    let session_path = match (session, latest) {
        (Some(p), false) => p,
        (None, true) => {
            let anchor = repo
                .clone()
                .or_else(|| std::env::current_dir().ok())
                .context("cannot determine a directory for --latest")?;
            discover::latest_session(&anchor)?
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
