//! The versioned JSON receipt — the machine-readable form of an audit.
//!
//! `report`/`html` render the reconciled [`Audit`] for a human; this module
//! serializes the *same* facts for a machine. It is a deliberate, stable
//! interchange schema: a dedicated set of owned, `serde`-friendly types that
//! map from the internal engine types, so the on-disk shape is decoupled from
//! internal refactors. Bump [`SCHEMA_VERSION`] on any breaking change.
//!
//! Everything the verbose console/HTML views show is present here: per-commit
//! statement, ledger, residue (attributed and dismissed), commands, intents,
//! plus the header context, token estimate, evidence grades, blast radii, and
//! the git-identity roll-up.

use serde::Serialize;

use crate::extract::Session;
use crate::fmt::redact_home;
use crate::ingest::IngestStats;
use crate::reconcile::{Audit, Interval, Landing, Status};
use crate::report::Filter;

/// Receipt schema version, independent of the tool version. Pre-1.0 and
/// expected to evolve: the shape is not yet stable, so consumers should not
/// assume compatibility across minor bumps. A future "1.0" is the stability
/// commitment; a breaking change before then bumps the minor.
pub const SCHEMA_VERSION: &str = "0.1";

/// The whole receipt: the root object a consumer reads.
#[derive(Debug, Serialize)]
pub struct Receipt {
    pub schema_version: &'static str,
    pub tool: Tool,
    pub source: Source,
    pub summary: Summary,
    pub intervals: Vec<IntervalReceipt>,
    pub tail: Tail,
    /// File claims that resolve outside the audited repo (scratch dirs, other
    /// repos, memory files) — reported, never scored.
    pub out_of_repo: Vec<FileClaim>,
}

#[derive(Debug, Serialize)]
pub struct Tool {
    pub name: &'static str,
    pub version: &'static str,
}

#[derive(Debug, Serialize)]
pub struct Source {
    /// The session stem, or a summary when several were merged.
    pub session: String,
    pub repo: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<Window>,
    pub branches: Vec<String>,
    pub ingest: Ingest,
}

#[derive(Debug, Serialize)]
pub struct Window {
    /// RFC 3339.
    pub start: String,
    pub end: String,
    pub days: i64,
    pub hours: i64,
}

#[derive(Debug, Serialize)]
pub struct Ingest {
    pub lines: usize,
    /// Execution events kept (fork-duplicates already removed).
    pub events_kept: usize,
    pub bookkeeping: usize,
    pub unparseable: usize,
    pub fork_duplicates: usize,
}

/// The headline numbers — the same arithmetic the console `intent → outcome`
/// block prints, so the receipt and the report never disagree.
#[derive(Debug, Serialize)]
pub struct Summary {
    pub prompts: usize,
    pub commits: usize,
    pub file_claims: usize,
    pub commands: usize,
    pub observations: usize,
    /// Ledger claims across all intervals.
    pub claims_total: usize,
    /// Ledger claims that reached git (landed on time or late).
    pub claims_landed: usize,
    /// The headline: claims that never landed and nothing explains.
    pub broken_promises: usize,
    pub balance: Balance,
    pub grades: Grades,
    pub radii: Radii,
    pub tokens: Tokens,
    pub identities: Identities,
}

#[derive(Debug, Serialize)]
pub struct Balance {
    pub green: usize,
    pub residue_only: usize,
    pub red: usize,
    pub total: usize,
}

#[derive(Debug, Serialize)]
pub struct Grades {
    pub exact: usize,
    pub receipted: usize,
    pub claimed: usize,
    pub dark: usize,
    pub failed: usize,
}

#[derive(Debug, Serialize)]
pub struct Radii {
    pub local_fs: usize,
    pub local_git: usize,
    pub remote_git: usize,
    pub network: usize,
    pub read_only: usize,
}

/// Estimated from the log's usage records, deduplicated per request — not
/// billing. Mirrors the console's "est., not billing" caveat.
#[derive(Debug, Serialize)]
pub struct Tokens {
    pub requests: usize,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_creation: u64,
}

/// Git identity roll-up — who committed, and any declared co-authors. Identity
/// only: a name never says agent-vs-hand; a co-author is present-only evidence.
#[derive(Debug, Serialize)]
pub struct Identities {
    pub authors: Vec<String>,
    pub co_authors: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct IntervalReceipt {
    pub commit: CommitReceipt,
    /// "green" | "residue_only" | "red".
    pub status: &'static str,
    pub agent_committed: bool,
    pub pushed: bool,
    /// This commit's parent is not the previous spine commit (rebase/reset).
    pub spine_jump: bool,
    /// The prompts this commit answers to. Empty when redacted (`--no-intent`).
    pub intents: Vec<String>,
    pub commands: Commands,
    /// What git recorded for this commit (its diff).
    pub statement: Vec<FileChangeReceipt>,
    /// The reconciled claims — one line per claimed path.
    pub ledger: Vec<LedgerReceipt>,
    /// Changed without a matching edit claim.
    pub residue: Vec<FileChangeReceipt>,
    /// Residue named by a command that ran — accounted for, not a warning.
    pub attributed_residue: Vec<AttributedChange>,
    /// Residue whose path is gitignored/untracked today — dismissed as noise.
    pub dismissed_residue: Vec<AttributedChange>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub superseded: Vec<Superseded>,
}

#[derive(Debug, Serialize)]
pub struct CommitReceipt {
    pub hash: String,
    pub short: String,
    pub subject: String,
    pub author: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub co_authors: Vec<String>,
    pub parent: String,
    /// Author date, RFC 3339 (clamped into sequence on a clock anomaly).
    pub authored: String,
    /// Committer date the object itself claims, RFC 3339.
    pub committed: String,
    pub reachable: bool,
    pub from_history: bool,
    pub clock_anomaly: bool,
    pub reflog_action: String,
}

#[derive(Debug, Serialize)]
pub struct Commands {
    pub total: usize,
    pub effectful: usize,
    pub runs: Vec<CommandRunReceipt>,
}

#[derive(Debug, Serialize)]
pub struct CommandRunReceipt {
    /// The full command text (the reports show a one-line summary; the
    /// receipt keeps it whole).
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius: Option<String>,
    pub committed: bool,
    pub pushed: bool,
    pub failed: bool,
    /// Captured output — the agent's own receipt for the un-verifiable tail,
    /// not proof. Present for failed commands by default, and for every
    /// command under `--with-output`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<CommandOutput>,
}

#[derive(Debug, Serialize)]
pub struct CommandOutput {
    pub is_error: bool,
    /// Capped at 64 KB upstream (in extract).
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct FileChangeReceipt {
    /// Git status letter: A/M/D/R/C…
    pub status: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LedgerReceipt {
    pub path: String,
    pub edits: usize,
    pub frames: Vec<usize>,
    /// "on_time" | "late" | "never".
    pub landing: &'static str,
    /// never-landed AND nothing explains it — the only thing that turns an
    /// interval red.
    pub broken: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub landed_at: Option<LandedAt>,
    /// A benign explanation for a never-landed claim (superseded, removed on
    /// record, persisted outside git).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    /// Why git never saw it, when we can tell.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnosis: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LandedAt {
    pub commit: String,
    pub commits_after: usize,
}

#[derive(Debug, Serialize)]
pub struct AttributedChange {
    pub change: FileChangeReceipt,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct Superseded {
    pub short: String,
    pub files: usize,
    pub seconds_before_amend: i64,
}

#[derive(Debug, Serialize)]
pub struct FileClaim {
    pub path: String,
    pub edits: usize,
}

#[derive(Debug, Serialize)]
pub struct Tail {
    /// File claims after the last commit — never committed, so unverifiable.
    pub claims: Vec<FileClaim>,
    /// Prompts typed after the last commit.
    pub intents: Vec<String>,
}

impl Receipt {
    /// Build the receipt from a completed audit. `show_intent` false drops the
    /// quoted prompt text (matching `--no-intent`) while keeping every count.
    /// `with_output` includes every command's captured output; by default
    /// only failed commands carry it (output is bulky and rebloats the receipt
    /// toward the raw log, but a failure's is worth keeping).
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        session_name: &str,
        repo: &str,
        session: &Session,
        stats: &IngestStats,
        audit: &Audit,
        show_intent: bool,
        with_output: bool,
        commit: Option<&str>,
        filter: Filter,
    ) -> Receipt {
        let window = match (session.first_ts, session.last_ts) {
            (Some(a), Some(b)) => {
                let dur = b - a;
                Some(Window {
                    start: a.to_rfc3339(),
                    end: b.to_rfc3339(),
                    days: dur.num_days(),
                    hours: dur.num_hours() % 24,
                })
            }
            _ => None,
        };

        // Scope the intervals to one commit and/or a status filter when asked;
        // the summary below is still computed over the whole session, matching
        // the console.
        let intervals: Vec<IntervalReceipt> = audit
            .intervals
            .iter()
            .filter(|i| commit.is_none_or(|h| i.commit.hash == h))
            .filter(|i| filter.keeps(i.status()))
            .map(|i| interval_receipt(i, show_intent, with_output))
            .collect();

        let claims_total: usize = audit.intervals.iter().map(|i| i.ledger.len()).sum();
        let claims_landed: usize = audit
            .intervals
            .iter()
            .flat_map(|i| i.ledger.iter())
            .filter(|l| l.landing != Landing::Never)
            .count();
        let broken_promises: usize = audit
            .intervals
            .iter()
            .flat_map(|i| i.never_landed())
            .count();
        let green = audit.intervals.iter().filter(|i| i.balanced()).count();
        let residue_only = audit
            .intervals
            .iter()
            .filter(|i| i.status() == Status::ResidueOnly)
            .count();
        let red = audit
            .intervals
            .iter()
            .filter(|i| i.status() == Status::Red)
            .count();

        let identities = Identities {
            authors: dedup_sorted(audit.intervals.iter().map(|i| i.commit.author.clone())),
            co_authors: dedup_sorted(
                audit
                    .intervals
                    .iter()
                    .flat_map(|i| i.commit.co_authors.iter().cloned()),
            ),
        };

        let tk = &session.tokens;
        let summary = Summary {
            prompts: audit.prompts,
            commits: audit.intervals.len(),
            file_claims: audit.file_claims,
            commands: audit.commands,
            observations: audit.observations,
            claims_total,
            claims_landed,
            broken_promises,
            balance: Balance {
                green,
                residue_only,
                red,
                total: audit.intervals.len(),
            },
            grades: Grades {
                exact: audit.grades.exact,
                receipted: audit.grades.receipted,
                claimed: audit.grades.claimed,
                dark: audit.grades.dark,
                failed: audit.grades.failed,
            },
            radii: Radii {
                local_fs: audit.radii.local_fs,
                local_git: audit.radii.local_git,
                remote_git: audit.radii.remote_git,
                network: audit.radii.network,
                read_only: audit.radii.read_only,
            },
            tokens: Tokens {
                requests: tk.requests,
                input: tk.input,
                output: tk.output,
                cache_read: tk.cache_read,
                cache_creation: tk.cache_creation,
            },
            identities,
        };

        Receipt {
            schema_version: SCHEMA_VERSION,
            tool: Tool {
                name: env!("CARGO_PKG_NAME"),
                version: env!("CARGO_PKG_VERSION"),
            },
            source: Source {
                session: session_name.to_string(),
                // Collapse home to ~ everywhere a path can carry it — the
                // receipt is meant to be committed/shared, and this matches
                // the console's privacy default.
                repo: redact_home(repo),
                window,
                branches: session.branches.clone(),
                ingest: Ingest {
                    lines: stats.lines,
                    events_kept: stats.kept - stats.duplicates,
                    bookkeeping: stats.skipped_types,
                    unparseable: stats.unparseable,
                    fork_duplicates: stats.duplicates,
                },
            },
            summary,
            intervals,
            tail: Tail {
                claims: audit
                    .tail_claims
                    .iter()
                    .map(|(path, edits)| FileClaim {
                        path: redact_home(path),
                        edits: *edits,
                    })
                    .collect(),
                intents: if show_intent {
                    audit.tail_intents.iter().map(|s| redact_home(s)).collect()
                } else {
                    Vec::new()
                },
            },
            out_of_repo: audit
                .out_of_repo
                .iter()
                .map(|(path, edits)| FileClaim {
                    path: redact_home(path),
                    edits: *edits,
                })
                .collect(),
        }
    }

    /// Serialize to a JSON string. `pretty` selects indented output.
    pub fn to_json(&self, pretty: bool) -> serde_json::Result<String> {
        if pretty {
            serde_json::to_string_pretty(self)
        } else {
            serde_json::to_string(self)
        }
    }
}

fn interval_receipt(i: &Interval, show_intent: bool, with_output: bool) -> IntervalReceipt {
    let c = &i.commit;
    IntervalReceipt {
        commit: CommitReceipt {
            hash: c.hash.clone(),
            short: c.short.clone(),
            subject: c.subject.clone(),
            author: c.author.clone(),
            co_authors: c.co_authors.clone(),
            parent: c.parent.clone(),
            authored: c.ts.to_rfc3339(),
            committed: c.committer_ts.to_rfc3339(),
            reachable: c.reachable,
            from_history: c.from_history,
            clock_anomaly: c.clock_anomaly,
            reflog_action: c.reflog_action.clone(),
        },
        status: status_str(i.status()),
        agent_committed: i.agent_committed,
        pushed: i.pushed,
        spine_jump: i.spine_jump,
        intents: if show_intent {
            i.intents.iter().map(|s| redact_home(s)).collect()
        } else {
            Vec::new()
        },
        commands: Commands {
            total: i.commands,
            effectful: i.effectful_commands,
            runs: i
                .commands_run
                .iter()
                .map(|r| CommandRunReceipt {
                    command: redact_home(&r.command),
                    radius: r.radius.as_ref().map(|rad| rad.to_string()),
                    committed: r.committed,
                    pushed: r.pushed,
                    failed: r.failed,
                    // Output when asked (--with-output) or when the command
                    // failed — a failure's output is the receipt you always
                    // want, and it's low-volume.
                    output: if with_output || r.failed {
                        r.output.as_ref().map(|o| CommandOutput {
                            is_error: o.is_error,
                            text: redact_home(&o.text),
                        })
                    } else {
                        None
                    },
                })
                .collect(),
        },
        statement: i.statement.iter().map(file_change).collect(),
        ledger: i
            .ledger
            .iter()
            .map(|l| LedgerReceipt {
                path: l.path.clone(),
                edits: l.edits,
                frames: l.frames.clone(),
                landing: landing_str(l.landing),
                broken: l.landing == Landing::Never && l.resolution.is_none(),
                landed_at: l.landed_at.as_ref().map(|(commit, after)| LandedAt {
                    commit: commit.clone(),
                    commits_after: *after,
                }),
                resolution: l.resolution.clone(),
                diagnosis: l.diagnosis.map(str::to_string),
            })
            .collect(),
        residue: i.residue.iter().map(file_change).collect(),
        attributed_residue: i
            .attributed_residue
            .iter()
            .map(|(fc, reason)| AttributedChange {
                change: file_change(fc),
                reason: (*reason).to_string(),
            })
            .collect(),
        dismissed_residue: i
            .dismissed_residue
            .iter()
            .map(|(fc, reason)| AttributedChange {
                change: file_change(fc),
                reason: (*reason).to_string(),
            })
            .collect(),
        superseded: i
            .superseded
            .iter()
            .map(|s| Superseded {
                short: s.short.clone(),
                files: s.files,
                seconds_before_amend: s.seconds_before_amend,
            })
            .collect(),
    }
}

fn file_change(fc: &crate::gitio::FileChange) -> FileChangeReceipt {
    FileChangeReceipt {
        status: fc.status.to_string(),
        path: fc.path.clone(),
        old_path: fc.old_path.clone(),
    }
}

fn status_str(s: Status) -> &'static str {
    match s {
        Status::Green => "green",
        Status::ResidueOnly => "residue_only",
        Status::Red => "red",
    }
}

fn landing_str(l: Landing) -> &'static str {
    match l {
        Landing::OnTime => "on_time",
        Landing::Late => "late",
        Landing::Never => "never",
    }
}

/// Collect, sort, dedup — the identity roll-up order the console uses.
fn dedup_sorted(it: impl Iterator<Item = String>) -> Vec<String> {
    let mut v: Vec<String> = it.collect();
    v.sort_unstable();
    v.dedup();
    v
}
