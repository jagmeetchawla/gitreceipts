//! `git receipts` — see what your agent actually did.

mod audit;
mod export;
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

Each commit is colored: green when it balances and nothing failed, amber (!)
when something is worth a look (residue, a failed command, or an errored MCP),
or red (x) for a broken promise. Only red is a verdict — a claimed edit git
never got; amber is a fact, never a lie. git is the oracle: what git can witness
is verified; side-effects beyond it (network, deploy, out-of-repo writes) are
surfaced as blast radius, never as proof.",
        after_long_help = "\
EXAMPLES:
  git receipts audit --latest                audit the newest session for this repo
  git receipts audit --latest --oneline      the commit spine, one line per commit
  git receipts audit --latest --commit 6d6cdc4  drill into a single commit
  git receipts audit --all                   merge every session the store has for it
  git receipts audit sess.jsonl --repo ~/app a specific session vs a specific repo
  git receipts audit --latest --filter red   only the broken promises
  git receipts audit --latest --no-prompt    hide your prompts before sharing (counts stay)
  git receipts audit --latest --verbose      per-commit anatomy (A/M/D/R + commands)
  git receipts audit --latest --format html > audit.html    self-contained HTML report
  git receipts audit --all --store /Volumes/studio/Users/me/.claude/projects --repo /Volumes/studio/.../app
                                             another machine's sessions from a mounted drive

On a terminal the report auto-pages through $PAGER with color (like git);
a pipe or redirect never pages. Use --no-pager to opt out."
    )]
    Audit {
        /// Paths to session .jsonl files. With none given, audits ALL of the
        /// repo's sessions (the default) — pass a file to audit just that one.
        sessions: Vec<PathBuf>,
        /// Audit ONLY the most recent session, not all of them. Deliberate
        /// shortcut for "what my last run did". Caveat: a single-session audit
        /// reconciles against the whole repo, so commits from your OTHER
        /// sessions match nothing and show up as residue / unclaimed keyframes.
        /// Audit all sessions (the default) for a clean picture.
        #[arg(long)]
        latest: bool,
        /// Audit every session for this repo, merged into one ledger. This is
        /// now the DEFAULT (kept as an explicit synonym). There is no local
        /// archive: sessions older than the store's retention are gone, and
        /// commits from that era show as unclaimed keyframes.
        #[arg(long)]
        all: bool,
        /// A single git repo to reconcile against (default: the cwd's repo).
        /// Mutually exclusive with --project.
        #[arg(long, conflicts_with = "project")]
        repo: Option<PathBuf>,
        /// A PROJECT folder holding several git repos (e.g. a public repo beside
        /// private ops/config repos, driven in one session). Discovers every git
        /// repo under it that has sessions and reports each — a project header
        /// plus one section per repo. Mutually exclusive with --repo.
        #[arg(long)]
        project: Option<PathBuf>,
        /// The Claude Code projects directory — where session logs live, and
        /// the ONLY directory gitreceipts reads (never your settings, prompt
        /// history, or MCP auth). Default: ~/.claude/projects. To audit
        /// another machine's sessions, point at a mounted copy:
        /// --store /Volumes/studio/Users/me/.claude/projects
        #[arg(long)]
        store: Option<PathBuf>,
        /// Which coding agent produced the session. Only `claude` (the
        /// default) is supported today; the switch reserves the contract so
        /// other agents (e.g. codex) become an additive change, not a
        /// breaking one. Recorded in the receipt's source.agent.
        #[arg(long, value_enum, default_value_t = report::Agent::Claude)]
        agent: report::Agent,
        /// When to emit ANSI colors. `always` keeps them through pipes,
        /// for `| bat`, `| less -R`, or saving a colored transcript.
        #[arg(long, value_enum, default_value_t = report::ColorMode::Auto)]
        color: report::ColorMode,
        /// Suppress your prompt text (intent lines, and your turns in --full).
        /// Counts stay. Use before sharing a report you'd rather not attach
        /// your own words to.
        #[arg(long)]
        no_prompt: bool,
        /// Suppress the agent's prose — its post-commit summaries (and its
        /// turns in --full). The verified ledger and counts stay.
        #[arg(long)]
        no_summary: bool,
        /// Suppress BOTH your prompts and the agent's prose (= --no-prompt
        /// --no-summary). Counts stay. The blunt "share safely" switch.
        #[arg(long)]
        no_intent: bool,
        /// Suppress git-identity names and emails — the "who touched this
        /// repo" roll-up and per-commit committer/co-author lines (counts and
        /// keyframe attribution stay). Use before sharing a report from a
        /// repo with contributors you'd rather not name.
        #[arg(long)]
        no_identity: bool,
        /// Filter the spine purely by color: all, red (broken promises), amber
        /// (residue, failed command, or errored MCP), green, or red-amber.
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
        /// Terse spine: the session summary, then one line per commit —
        /// short hash, status, subject, landed/claimed — instead of the full
        /// per-commit drill-down. Like `git log --oneline`. Console only.
        #[arg(long)]
        oneline: bool,
        /// Print each commit's full conversation (every prompt and assistant
        /// message), not just intent + summary. Most useful scoped with
        /// --commit — the whole session gets long. Implies --verbose.
        #[arg(long)]
        full: bool,
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
        /// `lspci -s` style — see `audit --oneline` for the hashes. The
        /// header, summary, and balance still cover the whole session.
        /// Implies --verbose for the console.
        #[arg(long, value_name = "REF")]
        commit: Option<String>,
        /// Extra literal word to mask as **** everywhere in the report — on
        /// top of the automatic home-directory/username redaction. Repeatable.
        /// For names, hostnames, or client ids the tool can't infer:
        /// --redact acme-corp --redact staging.internal
        #[arg(long, value_name = "WORD")]
        redact: Vec<String>,
        /// Turn OFF the built-in secret/PII scanner (on by default). The
        /// scanner masks API keys, tokens, private keys, and validated PII
        /// (SSN/card/IBAN) it finds in commands, output, and MCP results.
        /// Only pass this when you trust the audience with raw values.
        #[arg(long)]
        no_scan: bool,
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
  git receipts export --latest --no-intent        drop prompts + agent prose, keep counts
  git receipts export --latest --with-output       include each command's output
  git receipts export --latest --full             the whole chat + all output
  git receipts export --latest --commit 6d6cdc4 --full   one commit's conversation"
    )]
    Export {
        /// Paths to session .jsonl files. With none given, exports ALL of the
        /// repo's sessions (the default) — pass a file to export just that one.
        sessions: Vec<PathBuf>,
        /// Export ONLY the most recent session, not all of them. Caveat: a
        /// single-session view reconciles against the whole repo, so commits
        /// from your other sessions show as residue / unclaimed keyframes.
        #[arg(long)]
        latest: bool,
        /// Export every session for this repo, merged. Now the DEFAULT (kept as
        /// an explicit synonym).
        #[arg(long)]
        all: bool,
        /// Repo to reconcile against (default: cwd recorded in the session).
        /// Mutually exclusive with --project.
        #[arg(long, conflicts_with = "project")]
        repo: Option<PathBuf>,
        /// A PROJECT folder holding several git repos: export a
        /// { project, repos: [receipt, …] } wrapper, one receipt per repo — the
        /// JSON twin of `audit --project`. Mutually exclusive with --repo.
        #[arg(long)]
        project: Option<PathBuf>,
        /// The Claude Code projects directory — where session logs live, and
        /// the ONLY directory gitreceipts reads. Default: ~/.claude/projects.
        /// For another machine's sessions, point at a mounted copy:
        /// --store /Volumes/studio/Users/me/.claude/projects
        #[arg(long)]
        store: Option<PathBuf>,
        /// Which coding agent produced the session. Only `claude` (the
        /// default) is supported today; reserves the contract so other agents
        /// become additive. Recorded in the receipt's source.agent.
        #[arg(long, value_enum, default_value_t = report::Agent::Claude)]
        agent: report::Agent,
        /// Omit your prompt text from the receipt (intents + your --full
        /// turns); counts stay. Before committing or sharing a receipt.
        #[arg(long)]
        no_prompt: bool,
        /// Omit the agent's prose — post-commit summaries (+ its --full turns).
        /// The verified ledger and counts stay.
        #[arg(long)]
        no_summary: bool,
        /// Omit BOTH your prompts and the agent's prose (= --no-prompt
        /// --no-summary); counts stay.
        #[arg(long)]
        no_intent: bool,
        /// Omit git-identity names/emails from the receipt — the identity
        /// roll-up and per-commit author/co-authors go empty (keyframe
        /// attribution via agent_committed stays). Use before committing or
        /// sharing a receipt from a repo with contributors.
        #[arg(long)]
        no_identity: bool,
        /// Filter the intervals purely by color: all, red (broken promises),
        /// amber (residue/failed command/errored MCP), green, or red-amber. The summary
        /// still covers the whole session.
        #[arg(long, value_enum, default_value_t = report::Filter::All)]
        filter: report::Filter,
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
        /// Extra literal word to mask as **** everywhere in the receipt — on
        /// top of the automatic home-directory/username redaction. Repeatable.
        /// For names, hostnames, or client ids the tool can't infer:
        /// --redact acme-corp --redact staging.internal
        #[arg(long, value_name = "WORD")]
        redact: Vec<String>,
        /// Turn OFF the built-in secret/PII scanner (on by default). The
        /// scanner masks API keys, tokens, private keys, and validated PII
        /// (SSN/card/IBAN) found in the captured commands, output, and MCP
        /// results before they enter the receipt.
        #[arg(long)]
        no_scan: bool,
        /// The maximal export: add the full chat transcript (every prompt and
        /// assistant message, in order) and imply --with-output. With --commit,
        /// the transcript is scoped to that commit's conversation.
        #[arg(long)]
        full: bool,
        /// Emit single-line JSON instead of indented (for streaming/piping).
        #[arg(long)]
        compact: bool,
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
            project,
            store,
            agent,
            color,
            no_prompt,
            no_summary,
            no_intent,
            no_identity,
            filter,
            format,
            expand,
            oneline,
            full,
            verbose,
            with_output,
            commit,
            redact,
            no_scan,
            no_pager,
        } => audit::run(
            sessions,
            latest,
            all,
            repo,
            project,
            store,
            no_pager,
            report::Options {
                color,
                show: report::Show {
                    prompt: !no_prompt && !no_intent,
                    summary: !no_summary && !no_intent,
                },
                show_identity: !no_identity,
                filter,
                format,
                expand,
                // --with-output, --commit, and --full all need the per-commit
                // anatomy, so any of them implies verbose for the console.
                verbose: verbose || with_output || full || commit.is_some(),
                with_output,
                commit: None,
                oneline,
                full,
                project_section: false,
            },
            commit,
            redact,
            !no_scan,
            agent,
        ),
        Cmd::Export {
            sessions,
            latest,
            all,
            repo,
            project,
            store,
            agent,
            no_prompt,
            no_summary,
            no_intent,
            no_identity,
            filter,
            with_output,
            commit,
            redact,
            no_scan,
            full,
            compact,
        } => export::run(
            sessions,
            latest,
            all,
            repo,
            project,
            store,
            report::Show {
                prompt: !no_prompt && !no_intent,
                summary: !no_summary && !no_intent,
            },
            !no_identity,
            filter,
            with_output,
            commit,
            full,
            !compact,
            redact,
            !no_scan,
            agent,
        ),
    }
}
