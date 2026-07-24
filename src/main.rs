//! `git receipts` — see what your agent actually did.

use std::path::PathBuf;

use gitreceipts::{discover, extract, html, reconcile, report};

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

/// Page the console report through `$PAGER` when writing to a terminal —
/// git's own behavior, so colors survive and short reports don't linger.
/// Returns the pager child (kept alive until [`finish_pager`]); `None`
/// means output goes straight to stdout.
#[cfg(unix)]
fn start_pager(no_pager: bool) -> Option<std::process::Child> {
    use std::io::IsTerminal;
    use std::os::unix::io::AsRawFd;
    use std::process::{Command, Stdio};

    if no_pager || !std::io::stdout().is_terminal() {
        return None;
    }
    let pager = std::env::var("GIT_PAGER")
        .or_else(|_| std::env::var("PAGER"))
        .unwrap_or_else(|_| "less".to_string());
    if pager.trim().is_empty() || pager.trim() == "cat" {
        return None;
    }
    // git's defaults: R = render ANSI color, F = quit if one screen,
    // X = don't clear the screen on exit.
    if std::env::var_os("LESS").is_none() {
        unsafe { std::env::set_var("LESS", "FRX") };
    }
    let child = Command::new("sh")
        .arg("-c")
        .arg(&pager)
        .stdin(Stdio::piped())
        .spawn()
        .ok()?;
    // Redirect our stdout to the pager's stdin, so every println! flows to
    // the pager without threading a writer through the report code.
    let fd = child.stdin.as_ref()?.as_raw_fd();
    unsafe { libc::dup2(fd, libc::STDOUT_FILENO) };
    Some(child)
}

/// Close the pipe to the pager (both fd copies) so it sees EOF, then wait
/// for the user to quit it.
#[cfg(unix)]
fn finish_pager(child: Option<std::process::Child>) {
    use std::io::Write;
    if let Some(mut child) = child {
        let _ = std::io::stdout().flush();
        drop(child.stdin.take());
        unsafe { libc::close(libc::STDOUT_FILENO) };
        let _ = child.wait();
    }
}

#[cfg(not(unix))]
fn start_pager(_no_pager: bool) -> Option<std::process::Child> {
    None
}
#[cfg(not(unix))]
fn finish_pager(_child: Option<std::process::Child>) {}

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
        } => audit(
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

fn audit(
    sessions: Vec<PathBuf>,
    latest: bool,
    all: bool,
    repo: Option<PathBuf>,
    store: Option<PathBuf>,
    no_pager: bool,
    mut opts: report::Options,
) -> Result<()> {
    let store = match store {
        Some(s) => {
            if !s.join("projects").is_dir() {
                bail!(
                    "{} has no projects/ directory — --store expects the .claude directory itself",
                    s.display()
                );
            }
            s
        }
        None => discover::default_store().context("cannot locate the home directory")?,
    };
    let anchor = || {
        repo.clone()
            .or_else(|| std::env::current_dir().ok())
            .context("cannot determine a directory to search for sessions")
    };
    let session_paths: Vec<PathBuf> = match (sessions.is_empty(), latest, all) {
        (false, false, false) => sessions,
        (true, true, false) => vec![discover::latest_session(&store, &anchor()?)?],
        (true, false, true) => discover::all_sessions(&store, &anchor()?)?,
        (true, false, false) => bail!("pass session path(s), --latest, or --all"),
        _ => bail!("pass session path(s), --latest, or --all — not a combination"),
    };

    let (ordered, stats) = discover::merge_sessions(&session_paths)?;
    if ordered.is_empty() {
        bail!("no execution events in the given session(s)");
    }
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
    let short = |p: &PathBuf| {
        p.file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.chars().take(8).collect::<String>())
            .unwrap_or_else(|| "session".to_string())
    };
    let name = match session_paths.as_slice() {
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
    };
    let name = name.as_str();
    match opts.format {
        report::Format::Text => {
            // On a terminal, page through $PAGER (git's behavior). The
            // pager redirects stdout to a pipe, so auto color would strip
            // itself — force color on for the pager unless explicitly off.
            let pager = start_pager(no_pager);
            if pager.is_some() && opts.color == report::ColorMode::Auto {
                opts.color = report::ColorMode::Always;
            }
            report::print(
                name,
                &repo_path.display().to_string(),
                &session_data,
                &stats,
                &audit,
                &opts,
            );
            finish_pager(pager);
        }
        report::Format::Html => print!(
            "{}",
            html::render(
                name,
                &repo_path.display().to_string(),
                &session_data,
                &stats,
                &audit,
                opts.show_intent,
                opts.expand,
            )
        ),
    }
    Ok(())
}
