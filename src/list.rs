//! The `list` subcommand: enumerate the commit spine — a two-line session
//! summary and one line per commit — like `lspci` over the address bus. Each
//! row's short hash is the handle you pass to `audit`/`export --commit`.
//!
//! Reuses [`crate::audit::load`] and pages through $PAGER like `audit`.

use std::path::PathBuf;

use anyhow::Result;
use gitreceipts::report::{self, ColorMode, Filter};

use crate::audit::{self, Loaded};
use crate::pager::{finish_pager, start_pager};

#[allow(clippy::too_many_arguments)]
pub fn run(
    sessions: Vec<PathBuf>,
    latest: bool,
    all: bool,
    repo: Option<PathBuf>,
    store: Option<PathBuf>,
    no_pager: bool,
    mut color: ColorMode,
    filter: Filter,
) -> Result<()> {
    let Loaded {
        name,
        repo_display,
        session,
        stats: _,
        audit,
    } = audit::load(sessions, latest, all, repo, store)?;

    let pager = start_pager(no_pager);
    if pager.is_some() && color == ColorMode::Auto {
        color = ColorMode::Always;
    }
    report::list(&name, &repo_display, &session, &audit, color, filter);
    finish_pager(pager);
    Ok(())
}
