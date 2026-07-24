//! The `export` subcommand: run the same audit pipeline, then emit the
//! reconciled result as a versioned JSON receipt instead of a human report.
//!
//! This is the machine-readable half of the tool — the interchange artifact
//! meant to be committed, consolidated, or handed to another program. It
//! reuses [`crate::audit::load`] verbatim, so the receipt reflects exactly
//! what the console/HTML report would show.

use std::path::PathBuf;

use anyhow::Result;
use gitreceipts::receipt::Receipt;

use crate::audit::{self, Loaded};

pub fn run(
    sessions: Vec<PathBuf>,
    latest: bool,
    all: bool,
    repo: Option<PathBuf>,
    store: Option<PathBuf>,
    show_intent: bool,
    pretty: bool,
) -> Result<()> {
    let Loaded {
        name,
        repo_display,
        session,
        stats,
        audit,
    } = audit::load(sessions, latest, all, repo, store)?;

    let receipt = Receipt::build(&name, &repo_display, &session, &stats, &audit, show_intent);
    println!("{}", receipt.to_json(pretty)?);
    Ok(())
}
