//! The data types the interval equation produces — the audit result and
//! everything hanging off it. No logic here beyond trivial accessors.

use crate::extract::{Radius, Receipt};
use crate::gitio::{FileChange, SpineCommit};

/// Evidence grade for an effectful claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grade {
    /// The mutation's content is in the log itself.
    Exact,
    /// Independently corroborated by git (e.g. commit found in the reflog).
    Receipted,
    /// The agent asserted it and captured output, but nothing corroborates.
    Claimed,
    /// No receipt came back at all — we cannot know what happened.
    Dark,
}

/// Where a claimed file mutation ended up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Landing {
    /// In this interval's statement.
    OnTime,
    /// Not staged here, but in the very next commit — the check cleared
    /// after the statement date.
    Late,
    Never,
}

#[derive(Debug)]
pub struct LedgerLine {
    pub path: String,
    pub edits: usize,
    pub frames: Vec<usize>,
    pub landing: Landing,
    /// The content the last edit claimed to leave behind, for verifying a
    /// late landing.
    pub probe: Option<String>,
    /// For a Late landing: which commit it landed in (content-verified
    /// there), and how many commits after its own interval.
    pub landed_at: Option<(String, usize)>,
    /// A benign explanation for a never-landed claim: superseded by the
    /// file's own later landed edits, deliberately removed by the
    /// session's commands, or persisted on disk outside git's view.
    /// A resolved line is still listed, but it is not a broken promise.
    pub resolution: Option<String>,
    /// For never-landed claims: why git never saw it, when we can tell.
    pub diagnosis: Option<&'static str>,
}

/// A draft commit replaced by an amend moments later — collapsed out of the
/// spine, but worth telling: the diff between draft and final is often the
/// most interesting receipt in the session.
#[derive(Debug)]
pub struct Superseded {
    pub short: String,
    pub files: usize,
    pub seconds_before_amend: i64,
}

/// One effectful command that ran in an interval, for the drill-down.
#[derive(Debug)]
pub struct CommandRun {
    /// The full command text. The console/HTML views truncate it to a
    /// one-line summary (`fmt::command_summary`); the JSON receipt keeps it
    /// whole — it is the actual claim, and the only evidence for the
    /// un-verifiable tail (network calls, deploys).
    pub command: String,
    pub radius: Option<Radius>,
    pub committed: bool,
    pub pushed: bool,
    pub failed: bool,
    /// The command's captured output, when the log carried one. Held for the
    /// receipt's opt-in `--with-output`; the reports never render it.
    pub output: Option<Receipt>,
}

/// One MCP tool call that ran in an interval — the execution-axis counterpart
/// to `CommandRun`. The receipt (the server's tool_result) is the oracle: it
/// says receipted (`!errored`) vs errored.
#[derive(Debug)]
pub struct McpRun {
    pub server: String,
    pub tool: String,
    /// The structured call input (compact JSON) — the claim's payload.
    pub input: String,
    /// The server returned an error result (`is_error`).
    pub errored: bool,
    /// The server's response — the oracle's own words, capped upstream.
    pub output: Option<Receipt>,
}

#[derive(Debug)]
pub struct Interval {
    pub commit: SpineCommit,
    pub agent_committed: bool,
    /// This commit is reachable from a remote-tracking ref — pushed as of
    /// the repo's last fetch.
    pub pushed: bool,
    /// The effectful commands the agent ran in this interval.
    pub commands_run: Vec<CommandRun>,
    /// The MCP tool calls the agent made in this interval (S3, execution axis).
    pub mcp_runs: Vec<McpRun>,
    /// User prompts typed during this interval — the asks this commit
    /// answers to.
    pub intents: Vec<String>,
    /// The agent's own account of this commit: the first prose message it
    /// wrote right after committing. A natural-language claim — the readable
    /// counterpart to the verified ledger, never proof itself.
    pub summary: Option<String>,
    /// This commit's parent is not the previous spine commit (rebase,
    /// reset, branch switch) — the timeline has a seam here.
    pub spine_jump: bool,
    pub superseded: Vec<Superseded>,
    pub statement: Vec<FileChange>,
    pub ledger: Vec<LedgerLine>,
    pub residue: Vec<FileChange>,
    /// Residue explained by this interval's own commands: the path (or its
    /// pre-rename path, or a parent directory) is named in a command the
    /// agent ran, or in that command's captured output. Weaker evidence
    /// than an exact claim — the diff isn't in the log — but the change is
    /// accounted for, so it does not make the interval yellow.
    pub attributed_residue: Vec<(FileChange, &'static str)>,
    /// Residue whose path is gitignored or untracked TODAY — listed for
    /// honesty, but dismissed: it does not make the interval yellow.
    pub dismissed_residue: Vec<(FileChange, &'static str)>,
    pub commands: usize,
    /// Commands with any write radius — the usual explanation for residue.
    pub effectful_commands: usize,
}

/// How an interval settled. Red is a broken promise (a claim that never
/// landed); residue alone is a warning (something changed unclaimed —
/// usually command fallout), not a lie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Green,
    ResidueOnly,
    Red,
}

impl Interval {
    /// Broken promises: never landed AND nothing explains it.
    pub fn never_landed(&self) -> impl Iterator<Item = &LedgerLine> {
        self.ledger
            .iter()
            .filter(|l| l.landing == Landing::Never && l.resolution.is_none())
    }

    /// Never landed, but benignly explained — not red.
    pub fn resolved_never(&self) -> impl Iterator<Item = &LedgerLine> {
        self.ledger
            .iter()
            .filter(|l| l.landing == Landing::Never && l.resolution.is_some())
    }

    pub fn landed_late(&self) -> impl Iterator<Item = &LedgerLine> {
        self.ledger.iter().filter(|l| l.landing == Landing::Late)
    }

    pub fn status(&self) -> Status {
        // The verdict is git-only: red = a claimed edit git never got (the one
        // unambiguous signal). Execution-axis errors are NOT verdicts — a
        // non-zero exit is usually benign (grep no-match, a probe, a retried
        // command), so failed commands / errored MCP calls are SURFACED as
        // facts (the ✗failed flag, the errored tag, the receipt) for the auditor
        // to judge, never manufactured into red or yellow. (S3 finding.)
        if self
            .ledger
            .iter()
            .any(|l| l.landing == Landing::Never && l.resolution.is_none())
        {
            Status::Red
        } else if !self.residue.is_empty() {
            Status::ResidueOnly
        } else {
            Status::Green
        }
    }

    pub fn balanced(&self) -> bool {
        self.status() == Status::Green
    }
}

#[derive(Debug, Default)]
pub struct GradeCount {
    pub exact: usize,
    pub receipted: usize,
    pub claimed: usize,
    pub dark: usize,
    pub failed: usize,
}

impl GradeCount {
    pub(crate) fn add(&mut self, grade: Grade) {
        match grade {
            Grade::Exact => self.exact += 1,
            Grade::Receipted => self.receipted += 1,
            Grade::Claimed => self.claimed += 1,
            Grade::Dark => self.dark += 1,
        }
    }
}

#[derive(Debug, Default)]
pub struct RadiusCount {
    pub local_fs: usize,
    pub local_git: usize,
    pub remote_git: usize,
    pub network: usize,
    pub read_only: usize,
}

#[derive(Debug)]
pub struct Audit {
    pub intervals: Vec<Interval>,
    /// File claims after the last spine commit — never committed, so the
    /// equation cannot check them; reported, not judged.
    pub tail_claims: Vec<(String, usize)>,
    /// File claims that resolve outside the repo (scratch dirs, other
    /// repos, memory files, …).
    pub out_of_repo: Vec<(String, usize)>,
    /// Prompts typed after the last commit.
    pub tail_intents: Vec<String>,
    pub grades: GradeCount,
    pub radii: RadiusCount,
    pub prompts: usize,
    pub file_claims: usize,
    pub commands: usize,
    /// MCP tool calls — first-class effectful actions (S3), no longer folded
    /// into observations.
    pub mcp_calls: usize,
    // Execution-axis FACTS (surfaced, not verdicts), broken down by oracle.
    // "errored/failed" = the executor's error signal; "aborted" = the subset
    // that were user-stops (a rejection/interrupt), not agent failures.
    /// OS/FS oracle: commands the OS reported a non-zero exit for.
    pub cmd_failed: usize,
    pub cmd_aborted: usize,
    /// MCP oracle: calls whose server returned an error result.
    pub mcp_errored: usize,
    pub mcp_aborted: usize,
    pub observations: usize,
}
