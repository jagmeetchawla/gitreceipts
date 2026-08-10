//! The `export` subcommand: run the same audit pipeline, then emit the
//! reconciled result as a versioned JSON receipt instead of a human report.
//!
//! This is the machine-readable half of the tool — the interchange artifact
//! meant to be committed, consolidated, or handed to another program. It
//! reuses [`crate::audit::load`] verbatim, so the receipt reflects exactly
//! what the console/HTML report would show. With `--project` it emits a
//! `ProjectReceipt` wrapper — the JSON twin of `audit --project`.

use std::path::PathBuf;

use anyhow::{Result, bail};
use gitreceipts::receipt::{ProjectReceipt, Receipt};
use gitreceipts::report::Filter;
use gitreceipts::{discover, fmt};

use crate::audit::{self, Loaded};

#[allow(clippy::too_many_arguments)]
pub fn run(
    sessions: Vec<PathBuf>,
    latest: bool,
    all: bool,
    repo: Option<PathBuf>,
    project: Option<PathBuf>,
    store: Option<PathBuf>,
    show: gitreceipts::report::Show,
    show_identity: bool,
    filter: Filter,
    with_output: bool,
    commit: Option<String>,
    full: bool,
    pretty: bool,
    redact: Vec<String>,
    scan: bool,
    agent: gitreceipts::report::Agent,
    full_history: bool,
    all_authors: bool,
    me: &[String],
) -> Result<()> {
    if let Some(project) = project {
        return run_project(
            project,
            sessions,
            latest,
            all,
            store,
            show,
            show_identity,
            filter,
            with_output,
            commit,
            full,
            pretty,
            redact,
            scan,
            agent,
            full_history,
            all_authors,
            me,
        );
    }

    let Loaded {
        name,
        repo_display,
        agent,
        session,
        stats,
        audit,
    } = {
        let _status = audit::Status::show("git receipts: preparing data for export…");
        audit::load(
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

    // Resolve --commit against the actual spine (fails on unknown/ambiguous).
    let commit = match &commit {
        Some(needle) => Some(audit::resolve_commit(&audit, needle)?),
        None => None,
    };

    let receipt = Receipt::build(
        &name,
        &repo_display,
        agent.source(),
        &session,
        &stats,
        &audit,
        show,
        show_identity,
        // --full is the maximal export: the whole chat AND every command's
        // output.
        with_output || full,
        commit.as_deref(),
        filter,
        full,
        // Single-repo export: no project context, so no sibling roots to mask.
        &[],
    );
    println!("{}", receipt.to_json(pretty)?);
    Ok(())
}

/// Project mode: a `{ project, summary, repos: [receipt, …] }` wrapper, one
/// receipt per git repo under the project folder — the JSON twin of
/// `audit --project`. Reuses the single-repo `load` + `Receipt::build` per repo,
/// so every number matches the console.
#[allow(clippy::too_many_arguments)]
fn run_project(
    project: PathBuf,
    sessions: Vec<PathBuf>,
    latest: bool,
    all: bool,
    store: Option<PathBuf>,
    show: gitreceipts::report::Show,
    show_identity: bool,
    filter: Filter,
    with_output: bool,
    commit: Option<String>,
    full: bool,
    pretty: bool,
    redact: Vec<String>,
    scan: bool,
    agent: gitreceipts::report::Agent,
    full_history: bool,
    all_authors: bool,
    me: &[String],
) -> Result<()> {
    if !sessions.is_empty() {
        bail!("--project exports the folder's own sessions; don't also pass session file(s)");
    }
    if commit.is_some() {
        bail!("--commit scopes a single repo's spine; it can't be combined with --project");
    }
    let store = audit::resolve_store(store)?;
    let repos = discover::project_repos(&project, &store);
    if repos.is_empty() {
        bail!(
            "no git repos with sessions found under {} — export a single repo with --repo <dir>, or check the folder/store",
            project.display()
        );
    }

    let _status = audit::Status::show("git receipts: preparing project export…");
    let mut entries: Vec<(String, Receipt, gitreceipts::reconcile::LandingSummary)> = Vec::new();
    for repo in &repos {
        let name = repo
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("(repo)")
            .to_string();
        // Every OTHER project repo is a sibling: its writes collapse to a count
        // in this repo's receipt, so each per-repo receipt is safe to share.
        let siblings: Vec<PathBuf> = repos.iter().filter(|r| *r != repo).cloned().collect();
        let loaded = audit::load(
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
        )?;
        let landing = loaded.audit.landing_summary();
        let receipt = Receipt::build(
            &loaded.name,
            &loaded.repo_display,
            loaded.agent.source(),
            &loaded.session,
            &loaded.stats,
            &loaded.audit,
            show,
            show_identity,
            with_output || full,
            None,
            filter,
            full,
            &siblings,
        );
        entries.push((name, receipt, landing));
    }

    // Redaction is established by the loads above, so this collapses $HOME.
    let project_display = fmt::redact_home(&project.display().to_string());
    let wrapper = ProjectReceipt::build(project_display, entries);
    println!("{}", wrapper.to_json(pretty)?);
    Ok(())
}
