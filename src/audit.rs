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

#[allow(clippy::too_many_arguments)]
/// `--exit-code`: encode the verdict in the process exit status.
/// 0 = green or grey (the equation balances) · 1 = amber · 2 = red.
/// Grey never raises — explained findings are not failures.
fn exit_with_verdict(v: gitreceipts::reconcile::Status) -> ! {
    use gitreceipts::reconcile::Status as S;
    std::process::exit(match v {
        S::Red => 2,
        S::Amber => 1,
        S::Green | S::Grey => 0,
    })
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
    full_history: bool,
    all_authors: bool,
    me: &[String],
    exit_code: bool,
    this_session: Option<String>,
) -> Result<()> {
    let mut sessions = sessions;
    if let Some(marker) = &this_session {
        let store_dir = resolve_store(store.clone())?;
        sessions = vec![discover::session_containing(&store_dir, marker)?];
    }
    // Project mode: one header, then each repo under the folder reported in turn.
    if let Some(project) = project {
        return run_project(
            project,
            sessions,
            latest,
            all,
            store,
            no_pager,
            opts,
            redact,
            scan,
            agent,
            full_history,
            all_authors,
            me,
            exit_code,
        );
    }
    let loaded = {
        let _status = Status::show("git receipts: auditing… reconciling against git");
        load(
            sessions,
            latest,
            all,
            repo,
            store,
            &redact,
            scan,
            agent,
            full_history,
            all_authors,
            me,
        )?
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

    // Not one commit in the window is yours. Refuse rather than render an
    // empty green: an audit that filtered everything out looks identical to
    // an audit that found nothing wrong, and only one of those is true.
    // --all-authors is the way to see it anyway, so nothing is unreachable.
    if audit.identity_matched_nothing && !all_authors {
        bail!(
            "none of the {} commits in this window are yours by git's record.\n\n\
             \x20 gitreceipts is a git tool: it asks GIT who you are. Whose commits \
             count as yours comes from git config user.name / user.email, exactly as \
             `git log` and `git blame` see them.\n\
             \x20 this repo resolves that to: {}\n\n\
             \x20 check it:  git -C <repo> config user.email\n\
             \x20 fix it:    git -C <repo> config user.email you@example.com\n\
             \x20 or override just this run:  --me <name|email>  (repeatable)\n\
             \x20 or audit everyone:          --all-authors\n\n\
             \x20 refusing rather than printing an empty green report — an audit that \
             filtered everything out looks exactly like one that found nothing wrong.",
            audit.intervals.len(),
            audit.identity_described
        );
    }

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
                opts.narrative,
                opts.compact,
            )
        ),
    }
    if exit_code {
        exit_with_verdict(audit.verdict());
    }
    Ok(())
}

/// Project mode: report every git repo under the project folder — one project
/// masthead, a "where it landed" roll-up table, then each repo's own console
/// section (with the redundant per-repo header suppressed). A project folder
/// with a single repo (the monorepo case) collapses to the plain single-repo
/// report — no project chrome for one repo.
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
    full_history: bool,
    all_authors: bool,
    me: &[String],
    exit_code: bool,
) -> Result<()> {
    if !sessions.is_empty() {
        bail!("--project audits the folder's own sessions; don't also pass session file(s)");
    }
    let store = resolve_store(store)?;
    let repos = discover::project_repos(&project, &store);
    if repos.is_empty() {
        // Three unlike problems used to share one message. Name which.
        use discover::ProjectMiss::*;
        match discover::diagnose_project(&project, &store) {
            NoSuchDir => match discover::nearby_dir_named(&project) {
                Some(found) => bail!(
                    "no such directory: {} — did you mean {}?\n\
                     (--project takes a FOLDER holding git repos; run it again \
                     with that path, or use --repo <dir> for a single repo)",
                    project.display(),
                    found.display()
                ),
                None => bail!(
                    "no such directory: {} (--project takes a FOLDER holding git \
                     repos; for one repo use --repo <dir>)",
                    project.display()
                ),
            },
            NotADir => bail!(
                "{} is a file, not a directory — --project takes a folder holding \
                 git repos; to audit one session file, pass it as an argument",
                project.display()
            ),
            NoRepos => bail!(
                "no git repos under {} (searched 5 levels) — is this the right folder?",
                project.display()
            ),
            NoSessions(found) => {
                let names: Vec<String> = found
                    .iter()
                    .take(4)
                    .map(|r| {
                        r.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("?")
                            .to_string()
                    })
                    .collect();
                bail!(
                    "found {} git repo(s) under {} ({}{}) — but none has sessions in {}. \
                     Sessions are matched by the path they were recorded at, so a repo \
                     audited on another machine (or moved since) won't match; see \
                     KNOWN-LIMITATIONS.md §8.",
                    found.len(),
                    project.display(),
                    names.join(", "),
                    if found.len() > 4 { ", …" } else { "" },
                    store.display()
                )
            }
        }
    }

    // Reconcile every repo up front — the roll-up table needs all their numbers
    // before the first section prints. `set_redaction` inside each `load` is
    // cumulative, so the masking that protects one repo's paths also covers its
    // siblings by the time anything is rendered.
    let loaded: Vec<Loaded> = {
        let _status =
            Status::show("git receipts: auditing project… reconciling each repo against git");
        repos
            .iter()
            .map(|repo| {
                load(
                    Vec::new(),
                    latest,
                    all,
                    Some(repo.clone()),
                    Some(store.clone()),
                    &redact,
                    scan,
                    agent,
                    full_history,
                    all_authors,
                    me,
                )
            })
            .collect::<Result<Vec<_>>>()?
    };

    let repo_name = |repo: &Path| -> String {
        repo.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("(repo)")
            .to_string()
    };

    // HTML: one self-contained page — masthead + roll-up + a section per repo.
    // (A single-repo project collapses to the ordinary single-repo report.)
    if opts.format == report::Format::Html {
        if let [only] = loaded.as_slice() {
            print!(
                "{}",
                html::render(
                    &only.name,
                    &only.repo_display,
                    &only.session,
                    &only.stats,
                    &only.audit,
                    opts.show,
                    opts.show_identity,
                    opts.expand,
                    opts.with_output,
                    opts.commit.as_deref(),
                    opts.full,
                    opts.narrative,
                    opts.compact,
                )
            );
            return Ok(());
        }
        let names: Vec<String> = repos.iter().map(|r| repo_name(r)).collect();
        let sections: Vec<html::HtmlSection> = names
            .iter()
            .zip(&loaded)
            .map(|(name, l)| html::HtmlSection {
                name,
                session_name: &l.name,
                repo: &l.repo_display,
                session: &l.session,
                stats: &l.stats,
                audit: &l.audit,
            })
            .collect();
        print!(
            "{}",
            html::render_project(
                &project.display().to_string(),
                &sections,
                opts.show,
                opts.show_identity,
                opts.expand,
                opts.with_output,
                opts.full,
                opts.narrative,
                opts.compact,
            )
        );
        return Ok(());
    }

    let pager = start_pager(no_pager);
    if pager.is_some() && opts.color == report::ColorMode::Auto {
        opts.color = report::ColorMode::Always;
    }

    // One repo → project ≡ repo: just the normal single-repo report.
    if let [only] = loaded.as_slice() {
        report::print(
            &only.name,
            &only.repo_display,
            &only.session,
            &only.stats,
            &only.audit,
            &opts,
        );
        finish_pager(pager);
        if exit_code {
            exit_with_verdict(only.audit.verdict());
        }
        return Ok(());
    }

    if opts.summary {
        // Project summary: the roll-up, then one condensed table per repo
        // with commits — the same grammar as single-repo `--summary`.
        let sections: Vec<(String, &gitreceipts::reconcile::Audit)> = repos
            .iter()
            .zip(&loaded)
            .map(|(repo, l)| {
                let name = repo
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("(repo)")
                    .to_string();
                (name, &l.audit)
            })
            .collect();
        report::print_project_summary(&sections, &opts);
        finish_pager(pager);
        if exit_code {
            let worst = loaded
                .iter()
                .map(|l| l.audit.verdict())
                .max()
                .unwrap_or(gitreceipts::reconcile::Status::Green);
            exit_with_verdict(worst);
        }
        return Ok(());
    }
    report::project_header(&project.display().to_string(), loaded.len(), opts.color);
    let rows: Vec<(String, gitreceipts::reconcile::LandingSummary)> = repos
        .iter()
        .zip(&loaded)
        .map(|(repo, l)| {
            let name = repo
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("(repo)")
                .to_string();
            (name, l.audit.landing_summary())
        })
        .collect();
    let no_sessions: Vec<String> = discover::project_repos_without_sessions(&project, &store)
        .iter()
        .filter_map(|r| r.file_name().and_then(|n| n.to_str()).map(String::from))
        .collect();
    report::landing_table(&rows, &no_sessions, opts.color);

    opts.project_section = true;
    for (i, ((name, _), l)) in rows.iter().zip(&loaded).enumerate() {
        // Every OTHER project repo is a sibling: its writes are named and
        // counted in this section, never pathed — so each section is shareable.
        opts.siblings = repos
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, r)| r.clone())
            .collect();
        println!("\n═══ {name} ═══════════════════════════════════════════════");
        report::print(
            &l.name,
            &l.repo_display,
            &l.session,
            &l.stats,
            &l.audit,
            &opts,
        );
    }
    finish_pager(pager);
    if exit_code {
        let worst = loaded
            .iter()
            .map(|l| l.audit.verdict())
            .max()
            .unwrap_or(gitreceipts::reconcile::Status::Green);
        exit_with_verdict(worst);
    }
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
    full_history: bool,
    all_authors: bool,
    me: &[String],
) -> Result<Loaded> {
    let store = resolve_store(store)?;
    // Resolve the target repo BEFORE looking for sessions, so the sessions
    // we merge are the ones belonging to the repo we will reconcile. One
    // rule, one direction: the named folder (or cwd), then one level down,
    // never upward, never a guess between candidates.
    let start = match repo {
        Some(r) => r,
        None => std::env::current_dir().context("cannot determine the current directory")?,
    };
    let repo_path = discover::resolve_repo(&start)?;
    let anchor = || -> Result<PathBuf> { Ok(repo_path.clone()) };
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

    // Who is "you" in THIS repo: git's own user.name/user.email, plus any
    // identity the user named. Resolved per repo, because a work clone and a
    // personal one legitimately carry different identities.
    let identity = gitreceipts::identity::Identity::resolve(&repo_path, me);
    let mut audit = reconcile::reconcile_as(&repo_path, &session_data, &identity)?;
    // reconcile always leaves these false; the caller sets them from the flags.
    audit.full_history = full_history;
    audit.all_authors = all_authors;

    // The silent-empty-green guard. An identity that matches NOTHING while
    // the window holds commits means we are filtering by the wrong person —
    // a fresh clone, a work-vs-personal email, a repo-local override. Left
    // quiet, that renders a confident, empty, green audit: the worst output
    // this tool can produce. Recorded here; the CALLER decides how loud to
    // be, because a project run must not die on one repo (see the empty
    // session-dir abort this same release fixed).
    audit.identity_matched_nothing =
        identity.known && !audit.intervals.is_empty() && audit.intervals.iter().all(|i| !i.mine);

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
