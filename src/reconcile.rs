//! Stage 4: the interval equation.
//!
//! Real commits are the spine. For each interval A→B, what commit B
//! introduced is the bank statement and the session's claimed file mutations
//! are the itemized ledger. Green means they balance. Red comes in two
//! flavors: claims that never landed in any statement, and residue — files
//! the statement says changed that the log never claimed (human edits, or
//! command side effects).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, Utc};

use crate::extract::{Action, Claim, Radius, Session};
use crate::gitio::{self, FileChange, SpineCommit};

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
    /// For a Late landing: Some(true) if the claimed content was found in
    /// a later commit's blob; None if there was nothing to check against.
    /// (A failed check never reaches Late — the claim stays Never.)
    pub late_verified: Option<bool>,
    /// For a Late landing: which commit it landed in, and how many commits
    /// after its own interval.
    pub landed_at: Option<(String, usize)>,
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

#[derive(Debug)]
pub struct Interval {
    pub commit: SpineCommit,
    pub agent_committed: bool,
    /// User prompts typed during this interval — the asks this commit
    /// answers to.
    pub intents: Vec<String>,
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
    pub fn never_landed(&self) -> impl Iterator<Item = &LedgerLine> {
        self.ledger.iter().filter(|l| l.landing == Landing::Never)
    }

    pub fn landed_late(&self) -> impl Iterator<Item = &LedgerLine> {
        self.ledger.iter().filter(|l| l.landing == Landing::Late)
    }

    pub fn status(&self) -> Status {
        if self.ledger.iter().any(|l| l.landing == Landing::Never) {
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
    fn add(&mut self, grade: Grade) {
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
    pub observations: usize,
}

/// Match a claimed absolute path against the audited repo.
///
/// The repo root itself always qualifies. A session cwd qualifies as a
/// *historical alias* (the repo was renamed or moved mid-session) only if
/// the majority of distinct paths claimed under it appear somewhere in the
/// repo's history — a scratch directory or a sibling repo fails that test.
struct Roots {
    valid: Vec<String>,
}

impl Roots {
    fn build(repo_canon: &str, session: &Session, history: &HashSet<String>) -> Roots {
        let mut candidates: Vec<String> = vec![repo_canon.to_string()];
        for cwd in &session.cwds {
            if !candidates.contains(cwd) {
                candidates.push(cwd.clone());
            }
        }

        // group distinct claimed paths by their longest-prefix candidate
        let mut claimed_under: HashMap<&str, HashSet<String>> = HashMap::new();
        for claim in &session.claims {
            if let Action::FileMutation { path, .. } = &claim.action
                && let Some((root, rel)) = longest_prefix(path, &candidates)
            {
                claimed_under.entry(root).or_default().insert(rel);
            }
        }

        let valid = candidates
            .iter()
            .filter(|root| {
                if root.as_str() == repo_canon {
                    return true;
                }
                match claimed_under.get(root.as_str()) {
                    Some(rels) if !rels.is_empty() => {
                        let hits = rels.iter().filter(|r| history.contains(*r)).count();
                        hits * 2 >= rels.len()
                    }
                    _ => false,
                }
            })
            .cloned()
            .collect();
        Roots { valid }
    }

    fn relativize(&self, path: &str) -> Option<String> {
        longest_prefix(path, &self.valid).map(|(_, rel)| rel)
    }
}

pub fn longest_prefix<'r>(path: &str, roots: &'r [String]) -> Option<(&'r str, String)> {
    // A rel with a `..` component could escape the repo when later joined
    // or handed to git — claimed paths are untrusted, so such a claim goes
    // to the out-of-repo bucket instead of the ledger.
    roots
        .iter()
        .filter_map(|root| {
            path.strip_prefix(root.as_str())
                .and_then(|rest| rest.strip_prefix('/'))
                .filter(|rel| !rel.split('/').any(|c| c == ".."))
                .map(|rel| (root.as_str(), rel.to_string()))
        })
        .max_by_key(|(root, _)| root.len())
}

/// Strings that would count as "this command named that file": the full
/// relative path, its pre-rename path, and every parent directory of
/// either that still contains a slash (so `git mv Sources/Clipbop
/// Sources/Clipbob` covers the whole tree, but a bare top-level word
/// cannot match by accident). Root-level files match by their full name.
fn name_candidates(change: &FileChange) -> Vec<String> {
    let mut out = Vec::new();
    let mut sides = vec![change.path.as_str()];
    if let Some(old) = &change.old_path {
        sides.push(old.as_str());
    }
    for side in sides {
        out.push(side.to_string());
        let mut parts: Vec<&str> = side.split('/').collect();
        while parts.len() > 1 {
            parts.pop();
            let ancestor = parts.join("/");
            // a bare top-level word ("src") could match prose by accident;
            // an extension-bearing name ("ClipBob.xcodeproj") cannot
            if parts.len() >= 2 || ancestor.contains('.') {
                out.push(ancestor);
            }
        }
    }
    out
}

fn grade_command(claim: &Claim, corroborated: bool) -> Grade {
    match &claim.receipt {
        None => Grade::Dark,
        Some(_) if corroborated => Grade::Receipted,
        Some(_) => Grade::Claimed,
    }
}

pub fn reconcile(repo: &Path, session: &Session) -> Result<Audit> {
    let from = session.first_ts.unwrap_or(DateTime::<Utc>::MIN_UTC);
    let to = session.last_ts.unwrap_or(Utc::now()) + chrono::Duration::minutes(2);
    let raw_spine = gitio::spine(repo, from, to)?;

    // Collapse amend chains: a `commit (amend)` sharing its parent with the
    // previous spine commit replaces it. The draft leaves the spine but its
    // size and lifetime are recorded on the surviving commit.
    let mut spine: Vec<SpineCommit> = Vec::with_capacity(raw_spine.len());
    let mut superseded_by_hash: Vec<(String, Superseded)> = Vec::new();
    for commit in raw_spine {
        if commit.is_amend()
            && let Some(prev) = spine.last()
            && prev.parent == commit.parent
        {
            let draft = spine.pop().expect("last() was Some");
            let files = gitio::commit_names(repo, &draft.hash)
                .map(|f| f.len())
                .unwrap_or(0);
            // drafts the popped draft had absorbed now belong to this amend
            for (owner, _) in superseded_by_hash.iter_mut() {
                if *owner == draft.hash {
                    *owner = commit.hash.clone();
                }
            }
            superseded_by_hash.push((
                commit.hash.clone(),
                Superseded {
                    short: draft.short,
                    files,
                    seconds_before_amend: (commit.ts - draft.ts).num_seconds(),
                },
            ));
        }
        spine.push(commit);
    }

    let repo_canon = repo
        .canonicalize()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| repo.display().to_string());
    let history = gitio::history_paths(repo)?;
    let roots = Roots::build(&repo_canon, session, &history);

    // Interval index for a timestamp: first spine commit at-or-after it.
    // Git truncates committer dates to whole seconds, so the event that
    // creates a commit can carry a timestamp a few hundred ms past it —
    // the boundary gets one second of slack to absorb that.
    let interval_of = |ts: Option<DateTime<Utc>>| -> Option<usize> {
        let ts = ts?;
        spine
            .iter()
            .position(|c| ts <= c.ts + chrono::Duration::seconds(1))
    };

    let mut intervals: Vec<Interval> = Vec::with_capacity(spine.len());
    for (i, commit) in spine.iter().enumerate() {
        let statement = gitio::commit_names(repo, &commit.hash)?;
        let spine_jump = i > 0 && commit.parent != spine[i - 1].hash;
        let superseded = superseded_by_hash
            .iter()
            .filter(|(owner, _)| *owner == commit.hash)
            .map(|(_, s)| Superseded {
                short: s.short.clone(),
                files: s.files,
                seconds_before_amend: s.seconds_before_amend,
            })
            .collect();
        intervals.push(Interval {
            commit: commit.clone(),
            agent_committed: false,
            intents: Vec::new(),
            spine_jump,
            superseded,
            statement,
            ledger: Vec::new(),
            residue: Vec::new(),
            attributed_residue: Vec::new(),
            dismissed_residue: Vec::new(),
            commands: 0,
            effectful_commands: 0,
        });
    }

    let mut audit = Audit {
        intervals,
        tail_claims: Vec::new(),
        out_of_repo: Vec::new(),
        tail_intents: Vec::new(),
        grades: GradeCount::default(),
        radii: RadiusCount::default(),
        prompts: session.prompts.len(),
        file_claims: 0,
        commands: 0,
        observations: 0,
    };

    for prompt in &session.prompts {
        match interval_of(prompt.ts) {
            Some(idx) => audit.intervals[idx].intents.push(prompt.text.clone()),
            None => audit.tail_intents.push(prompt.text.clone()),
        }
    }

    // per-interval ledgers accumulate as path -> (edits, frames, last probe)
    type LedgerAcc = BTreeMap<String, (usize, Vec<usize>, Option<String>)>;
    let mut ledgers: Vec<LedgerAcc> = (0..audit.intervals.len())
        .map(|_| BTreeMap::new())
        .collect();
    // effectful command text and output per interval, for residue attribution
    let mut cmd_corpus: Vec<String> = vec![String::new(); audit.intervals.len()];
    let mut out_corpus: Vec<String> = vec![String::new(); audit.intervals.len()];

    for claim in &session.claims {
        match &claim.action {
            Action::Observation => {
                audit.observations += 1;
                audit.radii.read_only += 1;
            }
            Action::FileMutation { path, probe } => {
                audit.file_claims += 1;
                audit.grades.add(Grade::Exact);
                audit.radii.local_fs += 1;
                if claim.receipt.as_ref().is_some_and(|r| r.is_error) {
                    audit.grades.failed += 1;
                    continue;
                }
                let Some(rel) = roots.relativize(path) else {
                    audit.out_of_repo.push((path.clone(), claim.frame));
                    continue;
                };
                match interval_of(claim.ts) {
                    Some(idx) => {
                        let entry = ledgers[idx].entry(rel).or_insert((0, Vec::new(), None));
                        entry.0 += 1;
                        entry.1.push(claim.frame);
                        if probe.is_some() {
                            entry.2 = probe.clone();
                        }
                    }
                    None => audit.tail_claims.push((rel, claim.frame)),
                }
            }
            Action::Command { command, radius } => {
                audit.commands += 1;
                match radius {
                    None => audit.radii.read_only += 1,
                    Some(Radius::LocalFs) => audit.radii.local_fs += 1,
                    Some(Radius::LocalGit) => audit.radii.local_git += 1,
                    Some(Radius::RemoteGit) => audit.radii.remote_git += 1,
                    Some(Radius::Network) => audit.radii.network += 1,
                }
                let failed = claim.receipt.as_ref().is_some_and(|r| r.is_error);
                if failed {
                    audit.grades.failed += 1;
                }
                let idx = interval_of(claim.ts);
                if let Some(idx) = idx {
                    audit.intervals[idx].commands += 1;
                    if radius.is_some() && !failed {
                        audit.intervals[idx].effectful_commands += 1;
                        cmd_corpus[idx].push_str(command);
                        cmd_corpus[idx].push('\n');
                        if let Some(r) = &claim.receipt {
                            out_corpus[idx].push_str(&r.text);
                            out_corpus[idx].push('\n');
                        }
                    }
                }
                // One Bash call can create several commits back to back; it
                // claims that many consecutive intervals.
                let commit_count = crate::extract::git_subcommands(command)
                    .iter()
                    .filter(|s| s.as_str() == "commit")
                    .count();
                let mut corroborated = false;
                if commit_count > 0
                    && !failed
                    && let Some(idx) = idx
                {
                    for k in idx..(idx + commit_count).min(audit.intervals.len()) {
                        audit.intervals[k].agent_committed = true;
                    }
                    corroborated = true;
                }
                if radius.is_some() && !failed {
                    audit.grades.add(grade_command(claim, corroborated));
                }
            }
        }
    }

    // settle each interval: match ledger against statement, find residue
    for (interval, ledger) in audit.intervals.iter_mut().zip(ledgers) {
        for (path, (edits, frames, probe)) in ledger {
            let landing = if interval.statement.iter().any(|c| c.path == path) {
                Landing::OnTime
            } else {
                Landing::Never
            };
            interval.ledger.push(LedgerLine {
                path,
                edits,
                frames,
                landing,
                probe,
                late_verified: None,
                landed_at: None,
                diagnosis: None,
            });
        }
        interval.residue = interval
            .statement
            .iter()
            .filter(|c| !interval.ledger.iter().any(|l| l.path == c.path))
            .cloned()
            .collect();
    }

    // Forward sweep: a claim can be swept into ANY later commit, not just
    // the next one — partial staging, batched commits, a file parked for a
    // day. For every unlanded claim with content to check, walk every
    // subsequent commit that touched the path and look for the claimed
    // content in its blob. Found: verified late landing, and the matching
    // residue there is explained away. Not found anywhere: the claim is
    // genuinely broken — and we can now SAY so, because we checked.
    let n = audit.intervals.len();
    let mut sweeps: Vec<(usize, usize, String, usize)> = Vec::new(); // (i, line_idx, path, j)
    for i in 0..n {
        for (line_idx, line) in audit.intervals[i].ledger.iter().enumerate() {
            if line.landing != Landing::Never {
                continue;
            }
            let Some(probe) = &line.probe else { continue };
            for (j, later) in audit.intervals.iter().enumerate().skip(i + 1) {
                if !later.statement.iter().any(|c| c.path == line.path) {
                    continue;
                }
                if gitio::blob_at(repo, &later.commit.hash, &line.path)
                    .is_some_and(|blob| blob.contains(probe.as_str()))
                {
                    sweeps.push((i, line_idx, line.path.clone(), j));
                    break;
                }
            }
        }
    }
    for (i, line_idx, path, j) in sweeps {
        let short = audit.intervals[j].commit.short.clone();
        let line = &mut audit.intervals[i].ledger[line_idx];
        line.landing = Landing::Late;
        line.late_verified = Some(true);
        line.landed_at = Some((short, j - i));
        if let Some(pos) = audit.intervals[j]
            .residue
            .iter()
            .position(|c| c.path == path)
        {
            audit.intervals[j].residue.remove(pos);
        }
    }

    // Claims with nothing to verify against stay on the old conservative
    // rule: next commit only, path match, labeled unverified.
    for i in 0..n.saturating_sub(1) {
        let (head, rest) = audit.intervals.split_at_mut(i + 1);
        let (cur, next) = (&mut head[i], &mut rest[0]);
        for line in cur
            .ledger
            .iter_mut()
            .filter(|l| l.landing == Landing::Never && l.probe.is_none())
        {
            if let Some(pos) = next.residue.iter().position(|c| c.path == line.path) {
                line.landing = Landing::Late;
                line.late_verified = None;
                line.landed_at = Some((next.commit.short.clone(), 1));
                next.residue.remove(pos);
            }
        }
    }

    // Dismiss residue that stopped mattering: the user has since
    // gitignored the path (local, global, or info/exclude — check-ignore
    // consults them all) or it is no longer tracked at all. Still listed,
    // but it no longer colors the interval.
    let tracked = gitio::tracked_paths(repo).unwrap_or_default();
    let mut dismissal: HashMap<String, &'static str> = HashMap::new();
    for interval in audit.intervals.iter_mut() {
        let (kept, dismissed): (Vec<FileChange>, Vec<FileChange>) =
            interval.residue.drain(..).partition(|c| {
                if dismissal.contains_key(&c.path) {
                    return false;
                }
                if gitio::is_ignored(repo, &c.path) {
                    dismissal.insert(c.path.clone(), "now gitignored");
                    false
                } else if !tracked.contains(&c.path) {
                    dismissal.insert(c.path.clone(), "no longer tracked today");
                    false
                } else {
                    true
                }
            });
        interval.residue = kept;
        interval.dismissed_residue = dismissed
            .into_iter()
            .map(|c| {
                let why = *dismissal.get(&c.path).expect("just inserted");
                (c, why)
            })
            .collect();
    }

    // Attribute what the commands account for: a residue file whose path
    // (or pre-rename path, or a parent directory of either) is named in an
    // effectful command this interval — or in that command's captured
    // output — was changed by the shell, not behind anyone's back. The
    // diff still isn't in the log, so this is claimed-grade evidence, and
    // the line says which kind.
    for (idx, interval) in audit.intervals.iter_mut().enumerate() {
        let (kept, attributed): (Vec<FileChange>, Vec<FileChange>) =
            interval.residue.drain(..).partition(|c| {
                !name_candidates(c)
                    .iter()
                    .any(|n| cmd_corpus[idx].contains(n) || out_corpus[idx].contains(n))
            });
        interval.residue = kept;
        interval.attributed_residue = attributed
            .into_iter()
            .map(|c| {
                let why = if name_candidates(&c)
                    .iter()
                    .any(|n| cmd_corpus[idx].contains(n))
                {
                    "named in this interval's commands"
                } else {
                    "named in a command's output this interval"
                };
                (c, why)
            })
            .collect();
    }

    // Diagnose the survivors: why did git never see this claim?
    for interval in audit.intervals.iter_mut() {
        for line in interval
            .ledger
            .iter_mut()
            .filter(|l| l.landing == Landing::Never)
        {
            // The forward sweep already checked every later commit, so
            // these are verified statements, not guesses.
            let on_disk_with_content = line.probe.as_ref().is_some_and(|probe| {
                std::fs::read_to_string(repo.join(&line.path))
                    .is_ok_and(|now| now.contains(probe.as_str()))
            });
            line.diagnosis = Some(if gitio::is_ignored(repo, &line.path) {
                "gitignored — the write was real but git never saw it"
            } else if on_disk_with_content {
                "the content is on disk right now, still uncommitted — it never reached a commit"
            } else if history.contains(&line.path) {
                "a tracked file, but this edit's content reached no later commit in this session — overwritten or reverted before landing"
            } else if repo.join(&line.path).exists() {
                "on disk, never committed — and today's file no longer carries this edit"
            } else {
                "deleted before any commit — written, used, thrown away"
            });
        }
    }

    Ok(audit)
}
