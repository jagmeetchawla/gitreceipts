//! The data types the interval equation produces — the audit result and
//! everything hanging off it. No logic here beyond trivial accessors.

use std::path::{Path, PathBuf};

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

/// How an interval settled, by color.
/// - **Red** = a broken promise: a claimed edit git never got. A lie. The one
///   verdict; the trustworthy number.
/// - **Amber** = worth a look: genuine residue, a failed command, or an errored
///   MCP call. Not a lie — but not "nothing to see" either.
/// - **Green** = clean: nothing to look at on either axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Green,
    Amber,
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
        // RED is git-only and means exactly one thing: a claimed edit git never
        // got. A non-zero exit is not a lie, so execution errors NEVER reach red
        // — the red count stays the trustworthy number.
        //
        // But green is reserved for "nothing to look at". We can't know a failed
        // command or an errored MCP call was harmless (the file may be in git yet
        // something around it broke), so those lift green → AMBER, alongside
        // genuine residue. User-aborts are your own stop, not a failure, so they
        // do not count. (This refines the earlier S3 stance: errors stay out of
        // red, but they do tint amber — "look here.")
        if self
            .ledger
            .iter()
            .any(|l| l.landing == Landing::Never && l.resolution.is_none())
        {
            Status::Red
        } else if !self.residue.is_empty()
            || self.commands_run.iter().any(|c| c.failed)
            || self.mcp_runs.iter().any(|m| m.errored)
        {
            Status::Amber
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

/// The exception aggregates the console headlines under "what happened to every
/// exception" plus the interval-spine attribution counts. Computed once from the
/// `Audit` so the console, HTML, and JSON receipt all render the SAME numbers —
/// the three surfaces are just formats of one receipt. See [`Audit::exceptions`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Exceptions {
    /// Claims that landed a commit or two late (content-verified there).
    pub landed_late: usize,
    /// Never landed but benignly explained, by kind:
    pub resolved_superseded: usize,
    pub resolved_deliberate: usize,
    pub resolved_persisted: usize,
    /// Landed at a DIFFERENT path (moved before its first commit; content-verified).
    pub resolved_relocated: usize,
    /// Genuine residue (unclaimed changes git recorded), before the who-split.
    pub residue: usize,
    /// Unclaimed total = residue + command-attributed + dismissed.
    pub unclaimed_total: usize,
    /// …explained by a command this agent ran (attributed residue).
    pub unclaimed_by_command: usize,
    /// …in a commit this session did not make (another contributor, by identity).
    pub unclaimed_other_contributor: usize,
    /// …inside an agent commit with nothing to explain it.
    pub unclaimed_unexplained: usize,
    /// Residue whose path is gitignored/untracked today — listed, dismissed.
    pub dismissed: usize,
    /// Commits not made by this session (keyframes) vs made by it.
    pub keyframes: usize,
    pub agent_committed: usize,
    /// Commits created elsewhere (pulled/fetched — absent from the reflog).
    pub created_elsewhere: usize,
    /// Commands or edits the oracle reported as failed.
    pub failed_commands_or_edits: usize,
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
    /// Audit the WHOLE in-window spine, including commits this agent did not
    /// make (teammate commits, pulls/merges, pre-agent history swept into the
    /// window). Default false: the verdict/balance/spine cover only the agent's
    /// OWN commits — the honest picture when agentic development sits on top of
    /// an existing codebase. `--full-history` sets it true (the old behavior).
    /// Set by the caller after `reconcile` (which always leaves it false).
    pub full_history: bool,
}

/// The headline numbers, computed over the equation set once so console, HTML,
/// and JSON never disagree. "Equation set" = the agent's own commits by default,
/// or every interval under `--full-history`.
#[derive(Debug, Clone, Copy)]
pub struct Counts {
    pub green: usize,
    pub amber: usize,
    pub red: usize,
    pub total: usize,
    pub claims_total: usize,
    pub claims_landed: usize,
    /// Never-landed unexplained claims (the broken-promise headline).
    pub broken: usize,
    /// Non-agent commits excluded from the equation (0 under --full-history) —
    /// reported as context, never as the agent's residue or broken promises.
    pub keyframes_excluded: usize,
}

impl Audit {
    /// Compute the exception + attribution aggregates once, so every surface
    /// renders identical numbers. Mirrors the console's own arithmetic exactly.
    pub fn exceptions(&self) -> Exceptions {
        let lines = || self.intervals.iter().flat_map(|i| i.ledger.iter());
        let resolved = |needle: &str| {
            lines()
                .filter(|l| l.resolution.as_deref().is_some_and(|r| r.contains(needle)))
                .count()
        };
        let residue: usize = self.intervals.iter().map(|i| i.residue.len()).sum();
        let by_command: usize = self
            .intervals
            .iter()
            .map(|i| i.attributed_residue.len())
            .sum();
        let dismissed: usize = self
            .intervals
            .iter()
            .map(|i| i.dismissed_residue.len())
            .sum();
        // Residue in a commit this session did not make is another contributor's.
        let other_contributor: usize = self
            .intervals
            .iter()
            .filter(|i| !i.agent_committed)
            .map(|i| i.residue.len())
            .sum();
        let keyframes = self.intervals.iter().filter(|i| !i.agent_committed).count();
        Exceptions {
            landed_late: lines().filter(|l| l.landing == Landing::Late).count(),
            resolved_superseded: resolved("superseded"),
            resolved_deliberate: resolved("deliberately"),
            resolved_persisted: resolved("persisted outside git"),
            resolved_relocated: resolved("relocated before its first commit"),
            residue,
            unclaimed_total: residue + by_command + dismissed,
            unclaimed_by_command: by_command,
            unclaimed_other_contributor: other_contributor,
            unclaimed_unexplained: residue - other_contributor,
            dismissed,
            keyframes,
            agent_committed: self.intervals.len() - keyframes,
            created_elsewhere: self
                .intervals
                .iter()
                .filter(|i| i.commit.from_history)
                .count(),
            failed_commands_or_edits: self.grades.failed,
        }
    }

    /// Unique files with unexplained residue in THIS session's OWN commits — the
    /// residue that actually warrants a look. Deduplicated (a file counted once,
    /// not once per commit it recurs in) and scoped to agent-committed intervals
    /// (another contributor's keyframe residue is theirs, attributed by identity,
    /// not your unexplained residue). This is the honest "residue" headline; the
    /// per-event `Exceptions` breakdown is the fuller unclaimed-changes picture.
    pub fn residue_files(&self) -> usize {
        let mut set = std::collections::HashSet::new();
        for i in &self.intervals {
            if i.agent_committed {
                for r in &i.residue {
                    set.insert(r.path.as_str());
                }
            }
        }
        set.len()
    }

    /// The intervals that shape the verdict/balance: the agent's OWN commits,
    /// or every interval under `--full-history`. Non-agent keyframes (teammate
    /// commits, pulls, pre-agent history) are context, not the agent's account.
    pub fn equation(&self) -> impl Iterator<Item = &Interval> {
        let full = self.full_history;
        self.intervals
            .iter()
            .filter(move |i| full || i.agent_committed)
    }

    /// Non-agent commits held OUT of the equation (0 under --full-history) —
    /// surfaced as a context count, never counted as the agent's work.
    pub fn keyframes_excluded(&self) -> usize {
        if self.full_history {
            0
        } else {
            self.intervals.iter().filter(|i| !i.agent_committed).count()
        }
    }

    /// The whole audit's verdict = the strongest color across the equation set:
    /// Red if any equation interval is red (a broken promise), else Amber if any
    /// is amber, else Green. A teammate's keyframe never sets your verdict.
    pub fn verdict(&self) -> Status {
        if self.equation().any(|i| i.status() == Status::Red) {
            Status::Red
        } else if self.equation().any(|i| i.status() == Status::Amber) {
            Status::Amber
        } else {
            Status::Green
        }
    }

    /// The headline numbers over the equation set — the single source console,
    /// HTML, and JSON all render, so they cannot drift.
    pub fn counts(&self) -> Counts {
        let mut c = Counts {
            green: 0,
            amber: 0,
            red: 0,
            total: 0,
            claims_total: 0,
            claims_landed: 0,
            broken: 0,
            keyframes_excluded: self.keyframes_excluded(),
        };
        for i in self.equation() {
            c.total += 1;
            match i.status() {
                Status::Green => c.green += 1,
                Status::Amber => c.amber += 1,
                Status::Red => c.red += 1,
            }
            c.claims_total += i.ledger.len();
            c.claims_landed += i
                .ledger
                .iter()
                .filter(|l| l.landing != Landing::Never)
                .count();
            c.broken += i.never_landed().count();
        }
        c
    }

    /// One repo's headline numbers — the row a `--project` roll-up prints and
    /// the JSON wrapper carries. Computed once (over the equation set) so console
    /// and JSON agree.
    pub fn landing_summary(&self) -> LandingSummary {
        let c = self.counts();
        LandingSummary {
            verdict: self.verdict(),
            commits: c.total,
            claims: c.claims_total,
            landed: c.claims_landed,
            broken: c.broken,
            residue_files: self.residue_files(),
        }
    }
}

/// A repo's headline numbers for a project roll-up — shared by the console
/// landing table and the JSON project wrapper so the two never disagree.
#[derive(Debug, Clone, Copy)]
pub struct LandingSummary {
    pub verdict: Status,
    pub commits: usize,
    pub claims: usize,
    pub landed: usize,
    pub broken: usize,
    pub residue_files: usize,
}

/// Writes that a repo's session made into a SIBLING project repo — a per-repo
/// COUNT only, never the paths. This is the sibling-protection payoff: a single
/// repo's shareable view can say "14 changes reached the `ops` repo — audit it
/// directly" without exposing that private repo's file tree.
#[derive(Debug, Clone)]
pub struct SiblingWrites {
    pub name: String,
    /// Distinct paths touched in that sibling.
    pub files: usize,
    /// Total write claims (a file touched twice counts twice).
    pub changes: usize,
}

/// Resolve `.` and `..` lexically (no filesystem access — the path may be a
/// historical claim whose target no longer exists), so a `repo/../ops/x` claim
/// still matches the `ops` sibling root.
fn normalize_lexical(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

impl Audit {
    /// Split `out_of_repo` writes into (truly external, sibling-repo). External
    /// writes (scratch dirs, memory files, unrelated repos) keep their paths;
    /// writes that land inside one of the given `siblings` collapse to a
    /// per-sibling count with NO paths. With an empty `siblings` list — every
    /// non-project audit — nothing is a sibling, so all writes stay external
    /// and this is a no-op partition.
    pub fn partition_out_of_repo(
        &self,
        siblings: &[PathBuf],
    ) -> (Vec<(String, usize)>, Vec<SiblingWrites>) {
        let roots: Vec<(PathBuf, String)> = siblings
            .iter()
            .map(|s| {
                let name = s
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("(repo)")
                    .to_string();
                (normalize_lexical(s), name)
            })
            .collect();

        let mut external: Vec<(String, usize)> = Vec::new();
        // name -> (distinct paths, write-claim count). One out_of_repo entry is
        // one write claim; a file written twice is two claims, one path.
        let mut sib: std::collections::BTreeMap<
            String,
            (std::collections::HashSet<String>, usize),
        > = std::collections::BTreeMap::new();
        for (path, frame) in &self.out_of_repo {
            let norm = normalize_lexical(Path::new(path));
            let home = roots
                .iter()
                .find(|(root, _)| norm.starts_with(root))
                .map(|(_, name)| name);
            match home {
                Some(name) => {
                    let e = sib.entry(name.clone()).or_default();
                    e.0.insert(path.clone());
                    e.1 += 1;
                }
                // Display the lexically-normalized path: a `sibling/../audited`
                // detour must not leave a sibling's NAME in an external path.
                None => external.push((norm.to_string_lossy().into_owned(), *frame)),
            }
        }
        let siblings = sib
            .into_iter()
            .map(|(name, (paths, changes))| SiblingWrites {
                name,
                files: paths.len(),
                changes,
            })
            .collect();
        (external, siblings)
    }
}
