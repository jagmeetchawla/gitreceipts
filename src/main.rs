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
    about = "Read what your agent actually did.",
    long_about = "\
git receipts — read what your agent actually did.

An agent can produce a day of commits in an hour. Understanding what \
happened is the slow part, and the session log is megabytes nobody reads. \
gitreceipts turns it into an account you can read at reading speed: each \
prompt you typed, the work it drove, the commit it became.

  git receipts            what happened here (the default)
  git receipts audit      …and did every claimed edit really land?
  git receipts export     the same receipt as JSON

You can trust the account because it is checked, not narrated. The session \
log is the agent's story; git kept an independent record the agent cannot \
rewrite. Every file claim is verified against the actual commit blobs — so \
`audit` can say \"broken promises: 0\" and mean it.

Most useful when you can't reconstruct it by hand: unattended or overnight \
runs you weren't watching, or work from a while ago you no longer remember."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
}

/// WHAT to read. Identical for every command — sharing the struct is what
/// keeps them identical, rather than three hand-maintained copies that
/// drift (recap shipped without --verbose/--oneline exactly that way).
#[derive(clap::Args)]
struct Scope {
    /// Session file(s) to read. Default: every session for the target.
    sessions: Vec<PathBuf>,
    /// Just the most recent session.
    #[arg(long)]
    latest: bool,
    /// Every session (the default; an explicit synonym).
    #[arg(long)]
    all: bool,
    /// A single git repo. The value is optional — bare --repo means
    /// "this folder".
    #[arg(long, value_name = "DIR", conflicts_with = "project", num_args = 0..=1, default_missing_value = ".")]
    repo: Option<PathBuf>,
    /// A folder holding several repos. The value is optional — bare
    /// --project means "this folder".
    #[arg(long, value_name = "DIR", num_args = 0..=1, default_missing_value = ".")]
    project: Option<PathBuf>,
    /// Claude Code projects directory (default: ~/.claude/projects).
    #[arg(long, value_name = "DIR")]
    store: Option<PathBuf>,
    /// Which agent produced the sessions (recorded in the receipt).
    #[arg(long, value_enum, default_value_t = report::Agent::Claude)]
    agent: report::Agent,
    /// Scope to a single commit (short or full hash).
    #[arg(long, value_name = "REF")]
    commit: Option<String>,
    /// THIS live session, found by a unique marker you just echoed.
    #[arg(long, value_name = "MARKER", conflicts_with_all = ["latest", "all"])]
    this_session: Option<String>,
    /// Include your own commits this agent didn't make (hand-made, pulls,
    /// merges). Opens the WHEN axis; other people's commits stay out.
    #[arg(long)]
    full_history: bool,
    /// Include commits by OTHER people. Opens the WHO axis: by default a
    /// colleague's commit is theirs, and neither its unexplained files nor
    /// its verdict is yours.
    #[arg(long)]
    all_authors: bool,
    /// Also count this name or email as you (repeatable). Use when a repo's
    /// git identity differs from the one that made the commits.
    #[arg(long = "me", value_name = "NAME|EMAIL")]
    me: Vec<String>,
}

/// WHAT TO HIDE. Identical for every command — privacy that varied by
/// command would be a trap.
#[derive(clap::Args)]
struct Privacy {
    /// Drop your prompts (counts stay).
    #[arg(long)]
    no_prompt: bool,
    /// Drop the agent's prose.
    #[arg(long)]
    no_summary: bool,
    /// Drop both prompts and agent prose.
    #[arg(long)]
    no_intent: bool,
    /// Drop names and emails.
    #[arg(long)]
    no_identity: bool,
    /// Extra literal word to mask as ****. Repeatable.
    #[arg(long, value_name = "WORD")]
    redact: Vec<String>,
    /// Turn OFF the built-in secret/PII scanner (on by default).
    #[arg(long)]
    no_scan: bool,
}

/// HOW to show it. Shared by the two reading commands, so a view that
/// exists for one exists for both.
#[derive(clap::Args)]
struct View {
    /// Colors: auto (default), always, never.
    #[arg(long, value_enum, default_value_t = report::ColorMode::Auto)]
    color: report::ColorMode,
    /// text (default) or html — a self-contained page to keep or share.
    #[arg(long, value_enum, default_value_t = report::Format::Text)]
    format: report::Format,
    /// HTML only: which commits start expanded — auto, all, none.
    #[arg(long, value_enum, default_value_t = report::Expand::Auto)]
    expand: report::Expand,
    /// HTML only: a small page. Amber and red commits keep their full
    /// file/command lists; everything else collapses to its headline, and
    /// long prose is clipped. Every cap says how much it hid.
    #[arg(long)]
    compact: bool,
    /// The condensed table: one line per commit, findings named.
    #[arg(long)]
    summary: bool,
    /// One line per commit, every commit, no caps (like git log --oneline).
    #[arg(long)]
    oneline: bool,
    /// Full per-commit anatomy: files added/modified/deleted and the
    /// commands that ran.
    #[arg(short, long)]
    verbose: bool,
    /// Every command in full with its captured output (implies --verbose).
    #[arg(long)]
    with_output: bool,
    /// Print each commit's whole conversation, not just intent + summary.
    #[arg(long)]
    full: bool,
    /// Don't page through $PAGER.
    #[arg(long)]
    no_pager: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// Read a session as a story: what you asked, what the agent did, what
    /// landed. The default command.
    #[command(long_about = "\
Read a session as a story — the default command.

Recap answers \"what happened here\": the prompts you typed, the work each
one drove, and the commit it became. Findings are named where they occur
but never lead — for the verdict (broken promises, residue, the colored
balance) run `git receipts audit`.

Scopes exactly like audit: this repo by default, --project for a folder of
repos, --commit for one commit's full story, --this-session for the live
one.")]
    Recap {
        #[command(flatten)]
        scope: Scope,
        #[command(flatten)]
        privacy: Privacy,
        #[command(flatten)]
        view: View,
    },
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
        #[command(flatten)]
        scope: Scope,
        #[command(flatten)]
        privacy: Privacy,
        #[command(flatten)]
        view: View,
        /// Show only some intervals: all (default), red, amber, grey,
        /// green, or red-amber (the unanswered ones).
        #[arg(long, value_enum, default_value_t = report::Filter::All)]
        filter: report::Filter,
        /// Status dots as emoji (🟢⚪🟡🔴) instead of ANSI marks — for chat
        /// surfaces that strip terminal color.
        #[arg(long)]
        emoji: bool,
        /// Exit with the verdict: 0 green/grey, 1 amber, 2 red. For CI
        /// gates; a red-only gate tests >= 2. Default exit semantics are
        /// unchanged without this flag.
        #[arg(long)]
        exit_code: bool,
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
        /// Repo to reconcile against. The value is optional — bare --repo
        /// means "this folder", the same as passing nothing.
        /// Mutually exclusive with --project.
        #[arg(long, value_name = "DIR", conflicts_with = "project", num_args = 0..=1, default_missing_value = ".")]
        repo: Option<PathBuf>,
        /// A PROJECT folder holding several git repos: export a
        /// { project, repos: [receipt, …] } wrapper, one receipt per repo — the
        /// JSON twin of `audit --project`. Mutually exclusive with --repo.
        #[arg(long, value_name = "DIR", num_args = 0..=1, default_missing_value = ".")]
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
        /// Include the WHOLE in-window spine in the receipt, not just this
        /// agent's own commits (the default). See `audit --full-history`.
        #[arg(long)]
        full_history: bool,
        /// Include commits by other people. See `audit --all-authors`.
        #[arg(long)]
        all_authors: bool,
        /// Also count this name or email as you (repeatable).
        #[arg(long = "me", value_name = "NAME|EMAIL")]
        me: Vec<String>,
    },
    /// Write roff man pages for this CLI into a directory.
    ///
    /// Hidden: it exists so the release build can ship the same pages the
    /// binary's own `--help` describes — one source, never out of sync.
    /// git looks for `man git-receipts` when you type `git receipts
    /// --help`, so the packaged page is what makes that work.
    #[command(hide = true)]
    Man {
        /// Directory to write `git-receipts*.1` into (created if absent).
        #[arg(long, value_name = "DIR", default_value = "man")]
        out: PathBuf,
        /// Install into the first writable directory on your MANPATH
        /// instead, so `git receipts --help` works. For `cargo install`,
        /// which has no man-install step of its own.
        #[arg(long, conflicts_with = "out")]
        install: bool,
    },
}

/// The first `man1` directory on this machine's MANPATH we can actually
/// write to. `cargo install` ships no man page, so this is how a
/// cargo-installed binary can still make `git receipts --help` work.
/// Prefers a user-owned prefix; falls back to ~/.local/share/man/man1 and
/// says so, since that path needs to be on MANPATH to take effect.
fn man_install_dir() -> Result<PathBuf> {
    let raw = std::process::Command::new("manpath")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    for base in raw.split(':').filter(|s| !s.is_empty()) {
        // Skip the read-only system/SDK trees; only a prefix we can write
        // is a candidate, and probing beats hardcoding a list.
        let dir = PathBuf::from(base).join("man1");
        let probe = dir.join(".gitreceipts-write-probe");
        if std::fs::create_dir_all(&dir).is_ok() && std::fs::write(&probe, b"").is_ok() {
            let _ = std::fs::remove_file(&probe);
            return Ok(dir);
        }
    }

    let home = std::env::var("HOME").map_err(|_| anyhow::anyhow!("no HOME set"))?;
    let fallback = PathBuf::from(home).join(".local/share/man/man1");
    eprintln!(
        "note: no writable directory on your MANPATH — using {}.\n\
         If `git receipts --help` still fails, add it:  export MANPATH=\"$HOME/.local/share/man:$(manpath)\"",
        fallback.display()
    );
    Ok(fallback)
}

/// One place that turns the shared groups into report options, so the two
/// reading commands cannot drift in how they interpret the same flag.
fn options_from(
    privacy: &Privacy,
    view: &View,
    narrative: bool,
    filter: report::Filter,
    emoji: bool,
    commit_scoped: bool,
) -> report::Options {
    report::Options {
        color: view.color,
        show: report::Show {
            prompt: !privacy.no_prompt && !privacy.no_intent,
            summary: !privacy.no_summary && !privacy.no_intent,
        },
        show_identity: !privacy.no_identity,
        filter,
        format: view.format,
        expand: view.expand,
        // --with-output, --commit and --full all need the per-commit
        // anatomy, so any of them implies verbose for the console.
        verbose: view.verbose || view.with_output || view.full || commit_scoped,
        with_output: view.with_output,
        commit: None,
        oneline: view.oneline,
        // Recap's default view IS the condensed one; audit asks for it.
        summary: view.summary || (narrative && !view.oneline && !view.verbose && !commit_scoped),
        emoji,
        compact: view.compact,
        narrative,
        full: view.full || commit_scoped,
        project_section: false,
        siblings: Vec::new(),
    }
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
    // No subcommand → recap this repo. Bare `git receipts` only printed
    // help before, so giving it a default takes nothing away.
    // No subcommand → recap this repo, with every default. Bare
    // `git receipts` only printed help before, so this takes nothing away.
    let command = cli.command.unwrap_or_else(|| {
        use clap::Parser;
        // Parse an empty recap invocation rather than hand-listing every
        // default — a hand-written copy is exactly what drifts.
        match Cli::parse_from(["git-receipts", "recap"]).command {
            Some(c) => c,
            None => unreachable!("an explicit subcommand was given"),
        }
    });
    match command {
        Cmd::Recap {
            scope,
            privacy,
            view,
        } => {
            let opts = options_from(
                &privacy,
                &view,
                true,
                report::Filter::All,
                false,
                scope.commit.is_some(),
            );
            audit::run(
                scope.sessions,
                scope.latest,
                scope.all,
                scope.repo,
                scope.project,
                scope.store,
                view.no_pager,
                opts,
                scope.commit,
                privacy.redact,
                !privacy.no_scan,
                scope.agent,
                scope.full_history,
                scope.all_authors,
                &scope.me,
                false,
                scope.this_session,
            )
        }
        Cmd::Man { out, install } => {
            use clap::CommandFactory;
            let dir = if install { man_install_dir()? } else { out };
            std::fs::create_dir_all(&dir)?;
            clap_mangen::generate_to(Cli::command(), &dir)?;
            eprintln!("man pages written to {}", dir.display());
            if install {
                eprintln!("`git receipts --help` should work now.");
            }
            Ok(())
        }
        Cmd::Audit {
            scope,
            privacy,
            view,
            filter,
            emoji,
            exit_code,
        } => {
            let opts = options_from(
                &privacy,
                &view,
                false,
                filter,
                emoji,
                scope.commit.is_some(),
            );
            audit::run(
                scope.sessions,
                scope.latest,
                scope.all,
                scope.repo,
                scope.project,
                scope.store,
                view.no_pager,
                opts,
                scope.commit,
                privacy.redact,
                !privacy.no_scan,
                scope.agent,
                scope.full_history,
                scope.all_authors,
                &scope.me,
                exit_code,
                scope.this_session,
            )
        }
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
            full_history,
            all_authors,
            me,
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
            full_history,
            all_authors,
            &me,
        ),
    }
}
