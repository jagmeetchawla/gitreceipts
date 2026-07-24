//! `git receipts` — see what your agent actually did.

mod audit;
mod export;
mod list;
mod pager;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use gitreceipts::report;

#[derive(Parser)]
#[command(
    name = "git-receipts",
    version,
    about = "See what your agent actually did.",
    long_about = "\
git receipts — see what your agent actually did.

git is the oracle. A coding agent tells you what it did; git kept its own \
record of what actually persisted — a log the agent can't fabricate after \
the fact. gitreceipts reconciles the two, claim by claim, against that \
record. Not the agent's word. Git's.

Most useful when you can't reconstruct it by hand: unattended or overnight \
runs you weren't watching, or work from a while ago you no longer remember."
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Audit one or more Claude Code sessions against a git repo.
    #[command(
        long_about = "\
Audit one or more Claude Code sessions against a git repo.

Reads the session log(s) and the repo, then reconciles per commit:
  claimed vs landed  file edits verified content-level against commit blobs
  broken promises    claims that never landed and nothing explains — the headline
  unclaimed changes  git recorded it, no matching claim (attributed to a command,
                     to another contributor by git identity, or left unexplained)
  intent -> outcome  the prompts you typed, agent effort, blast radius

Each commit is green when it balances, residue-only (!) as a warning, or red
(x) for a broken promise. git is the oracle: what git can witness is verified;
side-effects beyond it (network, deploy, out-of-repo writes) are surfaced as
blast radius, never as proof.",
        after_long_help = "\
EXAMPLES:
  git receipts audit --latest                audit the newest session for this repo
  git receipts audit --all                   merge every session the store has for it
  git receipts audit sess.jsonl --repo ~/app a specific session vs a specific repo
  git receipts audit --latest --filter red   only the broken promises
  git receipts audit --latest --verbose      per-commit anatomy (A/M/D/R + commands)
  git receipts audit --latest --format html > audit.html    self-contained HTML report
  git receipts audit --all --store /Volumes/studio/Users/me/.claude --repo /Volumes/studio/.../app
                                             another machine's sessions from a mounted drive

On a terminal the report auto-pages through $PAGER with color (like git);
a pipe or redirect never pages. Use --no-pager to opt out."
    )]
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
        /// Use this before sharing a report from a session you don't
        /// fully remember.
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
        /// Show every command in full with its captured output, console and
        /// HTML alike. By default only FAILED commands expand this way (their
        /// output is the one you always want); this shows it for all. Implies
        /// --verbose for the console.
        #[arg(long)]
        with_output: bool,
        /// Scope the drill-down to a single commit (short or full hash),
        /// `lspci -s` style — see `git receipts list` for the hashes. The
        /// header, summary, and balance still cover the whole session.
        /// Implies --verbose for the console.
        #[arg(long, value_name = "REF")]
        commit: Option<String>,
        /// Don't page the console report through $PAGER, even on a
        /// terminal. (By default, like git, a terminal gets a colored
        /// pager; a pipe or redirect never does.)
        #[arg(long)]
        no_pager: bool,
    },
    /// Export the reconciled audit as a versioned JSON receipt.
    #[command(
        long_about = "\
Export the reconciled audit as a versioned JSON receipt.

Runs the same pipeline as `audit`, then emits the result as machine-readable
JSON instead of a human report — the same facts the verbose console/HTML
views show: per-commit statement, ledger, residue, commands, and intents,
plus the header context, token estimate, evidence grades, blast radii, and
the git-identity roll-up. Every headline number matches the report.

This is the interchange artifact: commit it beside the code, feed it to
another program, or hand it to a model to interpret. The schema is versioned
(`schema_version`); redirect it to a file with `> receipt.json`.",
        after_long_help = "\
EXAMPLES:
  git receipts export --latest > receipt.json     newest session, pretty JSON
  git receipts export --latest --compact          single-line JSON, for streaming
  git receipts export --all --repo ~/app          every session for a repo, merged
  git receipts export --latest --no-intent        drop quoted prompts, keep counts
  git receipts export --latest --with-output       include each command's output"
    )]
    Export {
        /// Paths to session .jsonl files (or use --latest / --all).
        sessions: Vec<PathBuf>,
        /// Export the most recent session for the repo.
        #[arg(long)]
        latest: bool,
        /// Export every session the store still has for this repo and its
        /// parent directories, merged into one ledger.
        #[arg(long)]
        all: bool,
        /// Repo to reconcile against (default: cwd recorded in the session).
        #[arg(long)]
        repo: Option<PathBuf>,
        /// Claude data directory holding the session store (default:
        /// ~/.claude). Point at a mounted drive to audit another machine's
        /// sessions: --store /Volumes/studio/Users/me/.claude
        #[arg(long)]
        store: Option<PathBuf>,
        /// Omit the quoted prompt text from intents (counts stay). Use this
        /// before committing or sharing a receipt.
        #[arg(long)]
        no_intent: bool,
        /// Include every command's captured output (stdout/stderr as logged).
        /// By default only FAILED commands carry output — output is bulky and
        /// rebloats the receipt, but a failure's is worth keeping. The command
        /// text is always present regardless.
        #[arg(long)]
        with_output: bool,
        /// Scope the receipt's intervals to a single commit (short or full
        /// hash), `lspci -s` style. The summary still covers the whole
        /// session; only the `intervals` array is restricted to this commit.
        #[arg(long, value_name = "REF")]
        commit: Option<String>,
        /// Emit single-line JSON instead of indented (for streaming/piping).
        #[arg(long)]
        compact: bool,
    },
    /// List the commit spine — one line per commit, with a session summary.
    #[command(
        long_about = "\
List the commit spine — one line per commit, like `lspci` over the address bus.

Prints a two-line session summary, then one row per commit: its short hash,
status (✔ green · ! residue · ✘ red), subject, landed/claimed count, and any
broken (✘N) or residue (●N) hints. The short hash is the handle you pass to
`audit --commit <ref>` or `export --commit <ref>` to drill into one commit.",
        after_long_help = "\
EXAMPLES:
  git receipts list --latest                 the spine for the newest session
  git receipts list --latest --filter red    only commits with broken promises
  git receipts list --all --repo ~/app       every session for a repo, merged"
    )]
    List {
        /// Paths to session .jsonl files (or use --latest / --all).
        sessions: Vec<PathBuf>,
        /// List the spine for the most recent session for the repo.
        #[arg(long)]
        latest: bool,
        /// Merge every session the store still has for this repo.
        #[arg(long)]
        all: bool,
        /// Repo to reconcile against (default: cwd recorded in the session).
        #[arg(long)]
        repo: Option<PathBuf>,
        /// Claude data directory holding the session store (default: ~/.claude).
        #[arg(long)]
        store: Option<PathBuf>,
        /// When to emit ANSI colors: auto, always, or never.
        #[arg(long, value_enum, default_value_t = report::ColorMode::Auto)]
        color: report::ColorMode,
        /// Which commits to list: all, red (broken promises), or red-residue.
        #[arg(long, value_enum, default_value_t = report::Filter::All)]
        filter: report::Filter,
        /// Don't page through $PAGER, even on a terminal.
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
            with_output,
            commit,
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
                // --with-output and --commit both need the per-commit anatomy,
                // so either implies verbose for the console.
                verbose: verbose || with_output || commit.is_some(),
                with_output,
                commit: None,
            },
            commit,
        ),
        Cmd::Export {
            sessions,
            latest,
            all,
            repo,
            store,
            no_intent,
            with_output,
            commit,
            compact,
        } => export::run(
            sessions,
            latest,
            all,
            repo,
            store,
            !no_intent,
            with_output,
            commit,
            !compact,
        ),
        Cmd::List {
            sessions,
            latest,
            all,
            repo,
            store,
            color,
            filter,
            no_pager,
        } => list::run(sessions, latest, all, repo, store, no_pager, color, filter),
    }
}
