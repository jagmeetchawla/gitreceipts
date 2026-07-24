//! `git receipts` — see what your agent actually did.

mod audit;
mod pager;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use gitreceipts::report;

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
    /// Audit one or more Claude Code sessions against a git repo.
    Audit {
        /// Paths to session .jsonl files (or use --latest / --all).
        sessions: Vec<PathBuf>,
        /// Audit the most recent session for the repo.
        #[arg(long)]
        latest: bool,
        /// Audit every session the store still has for this repo and its
        /// parent directories, merged into one ledger. There is no local
        /// archive: sessions older than the store's retention are gone,
        /// and commits from that era will show as unclaimed keyframes.
        #[arg(long)]
        all: bool,
        /// Repo to reconcile against (default: cwd recorded in the session).
        #[arg(long)]
        repo: Option<PathBuf>,
        /// Claude data directory holding the session store (default:
        /// ~/.claude). Point at a mounted drive to audit another
        /// machine's sessions: --store /Volumes/studio/Users/me/.claude
        #[arg(long)]
        store: Option<PathBuf>,
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
        /// Output format: text (console ledger) or html (a self-contained
        /// page — redirect it: `... --format html > audit.html`).
        #[arg(long, value_enum, default_value_t = report::Format::Text)]
        format: report::Format,
        /// HTML only: which commit drill-downs start expanded — auto
        /// (findings open, balanced collapsed), all, or none.
        #[arg(long, value_enum, default_value_t = report::Expand::Auto)]
        expand: report::Expand,
        /// Console: print each commit's full anatomy — files added/
        /// modified/deleted/renamed and the commands that ran.
        #[arg(short, long)]
        verbose: bool,
        /// Don't page the console report through $PAGER, even on a
        /// terminal. (By default, like git, a terminal gets a colored
        /// pager; a pipe or redirect never does.)
        #[arg(long)]
        no_pager: bool,
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
            sessions,
            latest,
            all,
            repo,
            store,
            color,
            no_intent,
            filter,
            format,
            expand,
            verbose,
            no_pager,
        } => audit::run(
            sessions,
            latest,
            all,
            repo,
            store,
            no_pager,
            report::Options {
                color,
                show_intent: !no_intent,
                filter,
                format,
                expand,
                verbose,
            },
        ),
    }
}
