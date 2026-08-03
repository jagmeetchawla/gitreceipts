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
//! plus the header context, token estimate, provenance, blast radii, and
//! the git-identity roll-up.

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::extract::Session;
use crate::fmt::redact_home;
use crate::ingest::IngestStats;
use crate::reconcile::{Audit, Interval, Landing, LandingSummary, Status};
use crate::report::{Filter, Show};

/// Receipt schema version, independent of the tool version. Pre-1.0 and
/// expected to evolve: the shape is not yet stable, so consumers should not
/// assume compatibility across minor bumps. A future "1.0" is the stability
/// commitment; a breaking change before then bumps the minor.
pub const SCHEMA_VERSION: &str = "0.6";

/// The privacy caution stamped on every receipt (single-repo and project).
const NOTICE: &str = "Private audit report — built from chat/agent logs, git contents, \
     and command output. Meant for developers to audit their own work; \
     handle with extreme caution and treat as private before sharing.";

/// The whole receipt: the root object a consumer reads.
#[derive(Debug, Serialize)]
pub struct Receipt {
    pub schema_version: &'static str,
    /// Privacy caution. This receipt is built from chat/agent logs, git
    /// contents, and command output — private by default; handle with care
    /// before sharing (redaction flags reduce, never remove, exposure).
    pub notice: &'static str,
    pub tool: Tool,
    pub source: Source,
    /// What the built-in secret/PII scanner masked while building this receipt
    /// (`[redacted:<kind>]` in the command/output/MCP fields). Zero when
    /// `--no-scan`. A tally, not a guarantee of completeness.
    pub redacted: Redacted,
    pub summary: Summary,
    pub intervals: Vec<IntervalReceipt>,
    pub tail: Tail,
    /// File claims that resolve outside the audited repo (scratch dirs, other
    /// repos, memory files) — reported, never scored.
    pub out_of_repo: Vec<FileClaim>,
    /// Writes that reached a SIBLING project repo — a per-repo COUNT only, no
    /// paths (sibling protection). Populated only in `--project` exports, where
    /// the sibling roots are known; empty and omitted for a single-repo export.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sibling_repos: Vec<SiblingRepo>,
    /// The full chat — every prompt and assistant message, in order — present
    /// only under `--full`. Scoped to the commit's interval when `--commit` is
    /// also given, otherwise the whole session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript: Option<Vec<ChatMessage>>,
}

/// A sibling repo's write tally — no paths, so a single repo's shareable
/// receipt names the sibling without exposing its file tree.
#[derive(Debug, Serialize)]
pub struct SiblingRepo {
    pub name: String,
    pub files: usize,
    pub changes: usize,
}

/// Tally of what the secret/PII scanner masked in this receipt.
#[derive(Debug, Serialize)]
pub struct Redacted {
    pub secrets: usize,
    pub pii: usize,
}

/// One turn of the conversation, for the `--full` transcript.
#[derive(Debug, Serialize)]
pub struct ChatMessage {
    /// "user" | "assistant".
    pub role: &'static str,
    /// RFC 3339.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts: Option<String>,
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct Tool {
    pub name: &'static str,
    pub version: &'static str,
}

#[derive(Debug, Serialize)]
pub struct Source {
    /// The coding agent that produced the session (`"claude-code"`). Reserved
    /// for multi-agent support (`--agent`); today always `claude-code`.
    pub agent: &'static str,
    /// The model(s) that produced the session, each with its request count,
    /// most-used first. A mid-session model switch shows as several entries.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<ModelUse>,
    /// Reasoning effort observed across the session. Effort logging is sparse
    /// (newer logs only), so `coverage` says whether every request was tagged,
    /// some, or none. Omitted entirely when the log carries no effort at all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<EffortObs>,
    /// The session stem, or a summary when several were merged.
    pub session: String,
    pub repo: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<Window>,
    pub branches: Vec<String>,
    pub ingest: Ingest,
}

/// A model and how many deduplicated API requests it produced in some window.
#[derive(Debug, Serialize)]
pub struct ModelUse {
    pub id: String,
    pub requests: usize,
}

/// The agent work behind an interval: API requests and their output tokens,
/// attributed by the conversation window that led to the commit.
#[derive(Debug, Serialize)]
pub struct IntervalCost {
    pub requests: usize,
    pub output_tokens: u64,
}

/// Reasoning-effort levels seen in a window, and how completely the log tagged
/// them — never presented as authoritative, unlike `models`.
#[derive(Debug, Serialize)]
pub struct EffortObs {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub observed: Vec<String>,
    /// `"full"` (every request tagged) · `"partial"` · `"none"`.
    pub coverage: &'static str,
}

/// Build the receipt's model roll-up + effort observation for a window.
fn provenance(
    session: &Session,
    after: Option<chrono::DateTime<chrono::Utc>>,
    until: Option<chrono::DateTime<chrono::Utc>>,
) -> (Vec<ModelUse>, Option<EffortObs>) {
    let models = session
        .models_used(after, until)
        .into_iter()
        .map(|(id, requests)| ModelUse { id, requests })
        .collect();
    let (observed, coverage) = session.effort_seen(after, until);
    // Omit effort only when nothing at all was observed; a partial signal is
    // exactly what we want to keep (and label), never drop.
    let effort = (coverage != crate::extract::Coverage::None).then_some(EffortObs {
        observed,
        coverage: coverage.as_str(),
    });
    (models, effort)
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
    pub mcp_calls: usize,
    pub observations: usize,
    /// Execution-axis facts, per oracle (surfaced, not scored).
    pub execution: ExecutionFacts,
    /// Ledger claims across all intervals.
    pub claims_total: usize,
    /// Ledger claims that reached git (landed on time or late).
    pub claims_landed: usize,
    /// The headline: claims that never landed and nothing explains.
    pub broken_promises: usize,
    /// Unique files with unexplained residue in your OWN commits (deduped, not
    /// per-event; another contributor's keyframe residue excluded) — the honest
    /// "residue" figure. The per-event breakdown is in `exceptions`.
    pub residue_files: usize,
    /// The exception + attribution aggregates the console headlines under "what
    /// happened to every exception" — first-class here so the receipt carries
    /// every number the report shows, not just the four it always did.
    pub exceptions: ExceptionCounts,
    pub balance: Balance,
    pub provenance: Provenance,
    pub radii: Radii,
    pub tokens: Tokens,
    pub identities: Identities,
}

/// The console's "what happened to every exception" block + interval-spine
/// attribution, serialized. Mirrors [`crate::reconcile::Exceptions`] exactly.
#[derive(Debug, Serialize)]
pub struct ExceptionCounts {
    pub landed_late: usize,
    pub resolved_superseded: usize,
    pub resolved_deliberate: usize,
    pub resolved_persisted: usize,
    /// Genuine residue (unclaimed changes), before the who-split.
    pub residue: usize,
    /// Unclaimed total = residue + command-attributed + dismissed.
    pub unclaimed_total: usize,
    pub unclaimed_by_command: usize,
    pub unclaimed_other_contributor: usize,
    pub unclaimed_unexplained: usize,
    pub dismissed: usize,
    pub keyframes: usize,
    pub agent_committed: usize,
    pub created_elsewhere: usize,
    pub failed_commands_or_edits: usize,
}

impl From<crate::reconcile::Exceptions> for ExceptionCounts {
    fn from(e: crate::reconcile::Exceptions) -> Self {
        ExceptionCounts {
            landed_late: e.landed_late,
            resolved_superseded: e.resolved_superseded,
            resolved_deliberate: e.resolved_deliberate,
            resolved_persisted: e.resolved_persisted,
            residue: e.residue,
            unclaimed_total: e.unclaimed_total,
            unclaimed_by_command: e.unclaimed_by_command,
            unclaimed_other_contributor: e.unclaimed_other_contributor,
            unclaimed_unexplained: e.unclaimed_unexplained,
            dismissed: e.dismissed,
            keyframes: e.keyframes,
            agent_committed: e.agent_committed,
            created_elsewhere: e.created_elsewhere,
            failed_commands_or_edits: e.failed_commands_or_edits,
        }
    }
}

/// Execution-axis facts per oracle — what the executors reported, surfaced
/// not scored. `failed`/`errored` = the executor's error signal; `aborted` =
/// the subset that were user-stops (a rejection/interrupt), not agent failures.
#[derive(Debug, Serialize)]
pub struct ExecutionFacts {
    pub os_fs_commands: usize,
    pub os_fs_failed: usize,
    pub os_fs_aborted: usize,
    pub mcp_calls: usize,
    pub mcp_errored: usize,
    pub mcp_aborted: usize,
}

#[derive(Debug, Serialize)]
pub struct Balance {
    pub green: usize,
    /// Amber = worth a look: residue, a failed command, or an errored MCP.
    pub amber: usize,
    pub red: usize,
    pub total: usize,
}

/// Provenance — who attested each claim (a fact, not a grade). Ladder by
/// authority of the attester: claimed (agent's word) < receipted (an executor
/// returned a receipt) < landed (git durably verified). Only git lands.
#[derive(Debug, Serialize)]
pub struct Provenance {
    pub landed: usize,
    pub receipted: usize,
    pub claimed: usize,
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
    /// "green" | "amber" | "red".
    pub status: &'static str,
    pub agent_committed: bool,
    pub pushed: bool,
    /// The model(s) that drove this commit's interval — the conversation
    /// between the previous spine commit and this one. Usually one; a mid-work
    /// model switch shows two.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<ModelUse>,
    /// Reasoning effort observed in this interval, with coverage. Omitted when
    /// the interval's requests carried no effort tag.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<EffortObs>,
    /// API requests + output tokens behind this commit. Omitted for a keyframe
    /// interval that carried no requests (e.g. another contributor's commit).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<IntervalCost>,
    /// This commit's parent is not the previous spine commit (rebase/reset).
    pub spine_jump: bool,
    /// The prompts this commit answers to. Empty under `--no-prompt`/`--no-intent`.
    pub intents: Vec<String>,
    /// The agent's own post-commit summary — a natural-language claim, not
    /// proof. Omitted under `--no-summary`/`--no-intent`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub commands: Commands,
    /// The MCP tool calls in this interval (S3, execution axis) — each with the
    /// server's receipt (receipted vs errored).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub mcp: Vec<McpRunReceipt>,
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
pub struct McpRunReceipt {
    pub server: String,
    pub tool: String,
    /// The structured call input (compact JSON) — the claim's payload.
    pub input: String,
    /// The server's receipt reported an error.
    pub errored: bool,
    /// The server's response — the oracle's own words. Present for errored
    /// calls by default, and for every call under `--with-output`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<CommandOutput>,
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
    /// Build the receipt from a completed audit. `show.prompt`/`show.summary`
    /// gate the user prompt text and the agent's prose respectively (matching
    /// `--no-prompt`/`--no-summary`, both dropped by `--no-intent`) while keeping
    /// every count. `with_output` includes every command's captured output; by
    /// default only failed commands carry it (output is bulky and rebloats the
    /// receipt toward the raw log, but a failure's is worth keeping).
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        session_name: &str,
        repo: &str,
        agent: &'static str,
        session: &Session,
        stats: &IngestStats,
        audit: &Audit,
        show: Show,
        show_identity: bool,
        with_output: bool,
        commit: Option<&str>,
        filter: Filter,
        full: bool,
        siblings: &[std::path::PathBuf],
    ) -> Receipt {
        let (external_writes, sibling_writes) = audit.partition_out_of_repo(siblings);
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
            .enumerate()
            .filter(|(_, i)| commit.is_none_or(|h| i.commit.hash == h))
            .filter(|(_, i)| filter.keeps(i.status()))
            .map(|(idx, i)| {
                // The interval's conversation window: (previous spine commit, this
                // commit] — the same bounds the console/HTML use to attach prompts.
                let after = (idx > 0).then(|| audit.intervals[idx - 1].commit.ts);
                let (models, effort) = provenance(session, after, Some(i.commit.ts));
                let cost = session.cost_in(after, Some(i.commit.ts));
                interval_receipt(i, show, show_identity, with_output, models, effort, cost)
            })
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
        let amber = audit
            .intervals
            .iter()
            .filter(|i| i.status() == Status::Amber)
            .count();
        let red = audit
            .intervals
            .iter()
            .filter(|i| i.status() == Status::Red)
            .count();

        // --no-identity drops the names (like --no-intent drops prompt text):
        // the roll-up and per-commit author/co-authors go empty. agent_committed
        // still marks keyframes, so attribution survives without the identities.
        let identities = if show_identity {
            Identities {
                authors: dedup_sorted(audit.intervals.iter().map(|i| i.commit.author.clone())),
                co_authors: dedup_sorted(
                    audit
                        .intervals
                        .iter()
                        .flat_map(|i| i.commit.co_authors.iter().cloned()),
                ),
            }
        } else {
            Identities {
                authors: Vec::new(),
                co_authors: Vec::new(),
            }
        };

        let tk = &session.tokens;
        let summary = Summary {
            prompts: audit.prompts,
            commits: audit.intervals.len(),
            file_claims: audit.file_claims,
            commands: audit.commands,
            mcp_calls: audit.mcp_calls,
            observations: audit.observations,
            execution: ExecutionFacts {
                os_fs_commands: audit.commands,
                os_fs_failed: audit.cmd_failed,
                os_fs_aborted: audit.cmd_aborted,
                mcp_calls: audit.mcp_calls,
                mcp_errored: audit.mcp_errored,
                mcp_aborted: audit.mcp_aborted,
            },
            claims_total,
            claims_landed,
            broken_promises,
            residue_files: audit.residue_files(),
            exceptions: audit.exceptions().into(),
            balance: Balance {
                green,
                amber,
                red,
                total: audit.intervals.len(),
            },
            provenance: Provenance {
                landed: claims_landed,
                receipted: audit.grades.receipted + audit.grades.claimed + audit.mcp_calls,
                claimed: claims_total.saturating_sub(claims_landed) + audit.grades.dark,
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

        // The scanner tally is complete now — the intervals above have all been
        // built, and each ran its command/output/MCP text through redaction.
        let (secrets, pii) = crate::fmt::scan_counts();
        // Session-wide provenance roll-up (whole-session window).
        let (models, effort) = provenance(session, None, None);
        Receipt {
            schema_version: SCHEMA_VERSION,
            notice: NOTICE,
            redacted: Redacted { secrets, pii },
            tool: Tool {
                name: env!("CARGO_PKG_NAME"),
                version: env!("CARGO_PKG_VERSION"),
            },
            source: Source {
                agent,
                models,
                effort,
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
                intents: if show.prompt {
                    audit.tail_intents.iter().map(|s| redact_home(s)).collect()
                } else {
                    Vec::new()
                },
            },
            out_of_repo: external_writes
                .iter()
                .map(|(path, edits)| FileClaim {
                    path: redact_home(path),
                    edits: *edits,
                })
                .collect(),
            sibling_repos: sibling_writes
                .into_iter()
                .map(|s| SiblingRepo {
                    name: s.name,
                    files: s.files,
                    changes: s.changes,
                })
                .collect(),
            transcript: if full && (show.prompt || show.summary) {
                Some(transcript(session, audit, commit, show))
            } else {
                None
            },
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

/// A `--project` export: the JSON twin of `audit --project`. A masthead
/// (project path, repo count, overall verdict), a `landing` roll-up mirroring
/// the console's where-it-landed table, then one full receipt per repo. Same
/// numbers as the console, same schema version.
#[derive(Debug, Serialize)]
pub struct ProjectReceipt {
    pub schema_version: &'static str,
    pub notice: &'static str,
    /// Discriminator: `"project"`, versus a bare `Receipt` (single repo).
    pub kind: &'static str,
    /// The project folder, home-redacted for display.
    pub project: String,
    pub summary: ProjectSummary,
    /// One full receipt per repo, in the same order as `summary.landing`.
    pub repos: Vec<RepoReceipt>,
}

#[derive(Debug, Serialize)]
pub struct ProjectSummary {
    pub repos: usize,
    /// The strongest verdict across the repos (red > amber > green).
    pub verdict: &'static str,
    /// The where-it-landed table: one headline row per repo.
    pub landing: Vec<RepoLanding>,
}

/// One repo's headline row — the JSON of a console landing-table line.
#[derive(Debug, Serialize)]
pub struct RepoLanding {
    pub name: String,
    pub verdict: &'static str,
    pub commits: usize,
    pub claims: usize,
    pub landed: usize,
    pub broken: usize,
    pub residue_files: usize,
}

#[derive(Debug, Serialize)]
pub struct RepoReceipt {
    /// The repo's directory name — the join key to its `summary.landing` row.
    pub name: String,
    pub receipt: Receipt,
}

impl ProjectReceipt {
    /// Assemble the wrapper from each repo's name, built receipt, and landing
    /// summary. `project` is the already-home-redacted project path.
    pub fn build(
        project: String,
        entries: Vec<(String, Receipt, LandingSummary)>,
    ) -> ProjectReceipt {
        let verdict = if entries.iter().any(|(_, _, s)| s.verdict == Status::Red) {
            "red"
        } else if entries.iter().any(|(_, _, s)| s.verdict == Status::Amber) {
            "amber"
        } else {
            "green"
        };
        let landing: Vec<RepoLanding> = entries
            .iter()
            .map(|(name, _, s)| RepoLanding {
                name: name.clone(),
                verdict: status_str(s.verdict),
                commits: s.commits,
                claims: s.claims,
                landed: s.landed,
                broken: s.broken,
                residue_files: s.residue_files,
            })
            .collect();
        let repo_count = landing.len();
        let repos = entries
            .into_iter()
            .map(|(name, receipt, _)| RepoReceipt { name, receipt })
            .collect();
        ProjectReceipt {
            schema_version: SCHEMA_VERSION,
            notice: NOTICE,
            kind: "project",
            project,
            summary: ProjectSummary {
                repos: repo_count,
                verdict,
                landing,
            },
            repos,
        }
    }

    pub fn to_json(&self, pretty: bool) -> serde_json::Result<String> {
        if pretty {
            serde_json::to_string_pretty(self)
        } else {
            serde_json::to_string(self)
        }
    }
}

/// The full chat as `ChatMessage`s. Scoped to a commit's interval span
/// `(previous commit, this commit]` when `commit` is set (the conversation
/// that produced it), else the whole session.
fn transcript(
    session: &Session,
    audit: &Audit,
    commit: Option<&str>,
    show: Show,
) -> Vec<ChatMessage> {
    let (after, until): (Option<DateTime<Utc>>, Option<DateTime<Utc>>) = match commit {
        Some(h) => match audit.intervals.iter().position(|i| i.commit.hash == h) {
            Some(0) => (None, Some(audit.intervals[0].commit.ts)),
            Some(k) => (
                Some(audit.intervals[k - 1].commit.ts),
                Some(audit.intervals[k].commit.ts),
            ),
            None => (None, None),
        },
        None => (None, None),
    };
    session
        .conversation(after, until)
        .into_iter()
        .filter(|t| if t.user { show.prompt } else { show.summary })
        .map(|t| ChatMessage {
            role: if t.user { "user" } else { "assistant" },
            ts: t.ts.map(|ts| ts.to_rfc3339()),
            text: redact_home(t.text),
        })
        .collect()
}

fn interval_receipt(
    i: &Interval,
    show: Show,
    show_identity: bool,
    with_output: bool,
    models: Vec<ModelUse>,
    effort: Option<EffortObs>,
    cost: (usize, u64),
) -> IntervalReceipt {
    let c = &i.commit;
    let (creq, cout) = cost;
    IntervalReceipt {
        commit: CommitReceipt {
            hash: c.hash.clone(),
            short: c.short.clone(),
            subject: redact_home(&c.subject),
            // --no-identity: drop the committer name/email and co-authors.
            author: if show_identity {
                c.author.clone()
            } else {
                String::new()
            },
            co_authors: if show_identity {
                c.co_authors.clone()
            } else {
                Vec::new()
            },
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
        models,
        effort,
        cost: (creq > 0).then_some(IntervalCost {
            requests: creq,
            output_tokens: cout,
        }),
        spine_jump: i.spine_jump,
        intents: if show.prompt {
            i.intents.iter().map(|s| redact_home(s)).collect()
        } else {
            Vec::new()
        },
        summary: if show.summary {
            i.summary.as_deref().map(redact_home)
        } else {
            None
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
        mcp: i
            .mcp_runs
            .iter()
            .map(|m| McpRunReceipt {
                server: m.server.clone(),
                tool: m.tool.clone(),
                input: redact_home(&m.input),
                errored: m.errored,
                // Output for errored calls by default, all under --with-output.
                output: if with_output || m.errored {
                    m.output.as_ref().map(|o| CommandOutput {
                        is_error: o.is_error,
                        text: redact_home(&o.text),
                    })
                } else {
                    None
                },
            })
            .collect(),
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
        Status::Amber => "amber",
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
