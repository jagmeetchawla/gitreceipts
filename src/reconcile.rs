//! Stage 4: the interval equation.
//!
//! Real commits are the spine. For each interval A→B, what commit B
//! introduced is the bank statement and the session's claimed file mutations
//! are the itemized ledger. Green means they balance. Red comes in two
//! flavors: claims that never landed in any statement, and residue — files
//! the statement says changed that the log never claimed (human edits, or
//! command side effects).
//!
//! This module root holds the engine (`reconcile` + its passes); the data
//! types live in [`types`] and the path/command matching in [`matching`].

mod matching;
mod types;

pub use matching::longest_prefix;
pub use types::{
    Audit, CommandRun, Exceptions, Grade, GradeCount, Interval, Landing, LandingSummary,
    LedgerLine, McpRun, RadiusCount, Status, Superseded,
};

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, Utc};

use crate::extract::{Action, Claim, Radius, Session};
use crate::gitio::{self, FileChange, SpineCommit};
use matching::{Roots, change_paths, command_names_path, command_removes_path, usable_probe};

/// Result text that says the USER stopped the action (a rejection or interrupt)
/// rather than the agent/executor failing. Used only to COUNT aborts as a
/// distinct fact — never as a verdict.
fn is_user_abort(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    const MARKERS: &[&str] = &[
        "user doesn't want to proceed",
        "user rejected",
        "tool use was rejected",
        "request interrupted",
        "interrupted by user",
        "cancelled by the user",
        "canceled by the user",
        "user chose not to",
        "operation was aborted",
    ];
    MARKERS.iter().any(|m| t.contains(m))
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

    // Batch every commit's statement (name-status) in ONE `git show` up front,
    // rather than a subprocess per commit — the dominant reconcile cost on long
    // sessions. Keyed by hash; both the amend-collapse and interval loops below
    // read from this map.
    let statements = gitio::commit_statements(
        repo,
        &raw_spine.iter().map(|c| c.hash.clone()).collect::<Vec<_>>(),
    )
    .unwrap_or_default();

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
            let files = statements.get(&draft.hash).map(|f| f.len()).unwrap_or(0);
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
    let pushed_set = gitio::pushed_commits(repo);
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
        let statement = statements.get(&commit.hash).cloned().unwrap_or_default();
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
            pushed: pushed_set.contains(&commit.hash),
            commands_run: Vec::new(),
            mcp_runs: Vec::new(),
            intents: Vec::new(),
            summary: None,
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
        mcp_calls: 0,
        cmd_failed: 0,
        cmd_aborted: 0,
        mcp_errored: 0,
        mcp_aborted: 0,
        observations: 0,
    };

    for prompt in &session.prompts {
        match interval_of(prompt.ts) {
            Some(idx) => audit.intervals[idx].intents.push(prompt.text.clone()),
            None => audit.tail_intents.push(prompt.text.clone()),
        }
    }

    // Each commit's summary: the first assistant narration after this commit
    // but before the next — the agent's own account of what it just landed.
    let mut narrations: Vec<&crate::extract::Narration> = session
        .narrations
        .iter()
        .filter(|n| n.ts.is_some())
        .collect();
    narrations.sort_by_key(|n| n.ts);
    for i in 0..audit.intervals.len() {
        let after = audit.intervals[i].commit.ts;
        let before = audit.intervals.get(i + 1).map(|iv| iv.commit.ts);
        audit.intervals[i].summary = narrations
            .iter()
            .filter(|n| n.ts.is_some_and(|t| t > after))
            .find(|n| before.is_none_or(|b| n.ts.is_some_and(|t| t < b)))
            .map(|n| n.text.clone());
    }

    // per-interval ledgers accumulate as path -> (edits, frames, last probe)
    type LedgerAcc = BTreeMap<String, (usize, Vec<usize>, Option<String>)>;
    let mut ledgers: Vec<LedgerAcc> = (0..audit.intervals.len())
        .map(|_| BTreeMap::new())
        .collect();
    // Effectful command TEXT per interval, one entry per command, for
    // residue attribution and deliberate-removal resolution. We do NOT
    // pool captured output: a command's stdout listing a filename (git
    // status, ls, a commit summary) is not evidence the command changed
    // that file — only the command text naming a path is causal.
    let mut cmd_corpus: Vec<Vec<String>> = vec![Vec::new(); audit.intervals.len()];

    for claim in &session.claims {
        match &claim.action {
            Action::Observation => {
                audit.observations += 1;
                audit.radii.read_only += 1;
            }
            // MCP call — first-class effectful action (S3). Retained on its
            // interval with the server's receipt (the oracle): receipted vs
            // errored. Surfacing + the errored→broken reconciliation follow.
            Action::McpCall {
                server,
                tool,
                input,
            } => {
                audit.mcp_calls += 1;
                let errored = claim.receipt.as_ref().is_some_and(|r| r.is_error);
                if errored {
                    audit.mcp_errored += 1;
                    if claim
                        .receipt
                        .as_ref()
                        .is_some_and(|r| is_user_abort(&r.text))
                    {
                        audit.mcp_aborted += 1;
                    }
                }
                if let Some(idx) = interval_of(claim.ts) {
                    audit.intervals[idx].mcp_runs.push(McpRun {
                        server: server.clone(),
                        tool: tool.clone(),
                        input: input.clone(),
                        errored,
                        output: claim.receipt.clone(),
                    });
                }
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
                        // Only keep a probe specific enough to verify a
                        // landing by content — a one-char edit like "}"
                        // would "match" almost any blob.
                        if probe.as_deref().is_some_and(usable_probe) {
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
                    audit.cmd_failed += 1;
                    if claim
                        .receipt
                        .as_ref()
                        .is_some_and(|r| is_user_abort(&r.text))
                    {
                        audit.cmd_aborted += 1;
                    }
                }
                let subs = crate::extract::git_subcommands(command);
                let has_push = subs.iter().any(|s| s == "push");
                // One Bash call can create several commits back to back; it
                // claims that many consecutive intervals.
                let commit_count = subs.iter().filter(|s| s.as_str() == "commit").count();

                let idx = interval_of(claim.ts);
                if let Some(idx) = idx {
                    audit.intervals[idx].commands += 1;
                    if radius.is_some() && !failed {
                        audit.intervals[idx].effectful_commands += 1;
                        cmd_corpus[idx].push(command.clone());
                    }
                    audit.intervals[idx].commands_run.push(CommandRun {
                        command: command.clone(),
                        radius: *radius,
                        committed: commit_count > 0,
                        pushed: has_push,
                        failed,
                        output: claim.receipt.clone(),
                    });
                }
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
                landed_at: None,
                resolution: None,
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

    // Forward sweep: a claim can land in ANY later commit, not just the next
    // one — partial staging, batched commits, a file parked for a day. For each
    // unlanded claim, walk subsequent commits and take the first whose statement
    // touches the path: that is a late landing, and the matching residue there is
    // explained away. This is the SAME path-level bar the on-time check uses
    // (a file in the commit = landed) — landing is landing, on time or late, and
    // no blob read is needed.
    let n = audit.intervals.len();
    let mut sweeps: Vec<(usize, usize, String, usize)> = Vec::new(); // (i, line_idx, path, j)
    for i in 0..n {
        for (line_idx, line) in audit.intervals[i].ledger.iter().enumerate() {
            if line.landing != Landing::Never {
                continue;
            }
            for (j, later) in audit.intervals.iter().enumerate().skip(i + 1) {
                if later.statement.iter().any(|c| c.path == line.path) {
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
        line.landed_at = Some((short, j - i));
        if let Some(pos) = audit.intervals[j]
            .residue
            .iter()
            .position(|c| c.path == path)
        {
            audit.intervals[j].residue.remove(pos);
        }
    }

    // Dismiss residue that stopped mattering: the user has since
    // gitignored the path (local, global, or info/exclude — check-ignore
    // consults them all) or it is no longer tracked at all. Still listed,
    // but it no longer colors the interval.
    let tracked = gitio::tracked_paths(repo).unwrap_or_default();
    // Batch check-ignore over every residue path in ONE subprocess, rather than
    // one `git check-ignore` per path (the other big per-item cost).
    let ignored = {
        let all: Vec<String> = audit
            .intervals
            .iter()
            .flat_map(|iv| iv.residue.iter().map(|c| c.path.clone()))
            .collect();
        gitio::ignored_paths(repo, &all)
    };
    let mut dismissal: HashMap<String, &'static str> = HashMap::new();
    for interval in audit.intervals.iter_mut() {
        let (kept, dismissed): (Vec<FileChange>, Vec<FileChange>) =
            interval.residue.drain(..).partition(|c| {
                if dismissal.contains_key(&c.path) {
                    return false;
                }
                if ignored.contains(&c.path) {
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
    // (or pre-rename path, or a directory that moved it) is named as a
    // whole token in an effectful command this interval was changed by the
    // shell, not behind anyone's back. The diff still isn't in the log, so
    // this is claimed-grade evidence. Matching is whole-token only, and
    // against command TEXT only — a command's stdout listing a filename is
    // not proof the command touched it.
    for (idx, interval) in audit.intervals.iter_mut().enumerate() {
        let named = |c: &FileChange| {
            change_paths(c)
                .iter()
                .any(|p| cmd_corpus[idx].iter().any(|cmd| command_names_path(cmd, p)))
        };
        let (kept, attributed): (Vec<FileChange>, Vec<FileChange>) =
            interval.residue.drain(..).partition(|c| !named(c));
        interval.residue = kept;
        interval.attributed_residue = attributed
            .into_iter()
            .map(|c| (c, "named in this interval's commands"))
            .collect();
    }

    // Resolve the never-landed claims that are not actually broken
    // promises, in order of evidence strength.
    //
    // 1. Superseded: a LATER claimed edit to the same file landed — the
    //    red edit was an intermediate state in a chain that kept its
    //    promise, not lost work.
    let mut landed_after: HashMap<String, Vec<(usize, String)>> = HashMap::new();
    for (j, interval) in audit.intervals.iter().enumerate() {
        for line in &interval.ledger {
            let commit = match line.landing {
                Landing::OnTime => interval.commit.short.clone(),
                Landing::Late => line
                    .landed_at
                    .as_ref()
                    .map(|(s, _)| s.clone())
                    .unwrap_or_else(|| interval.commit.short.clone()),
                Landing::Never => continue,
            };
            landed_after
                .entry(line.path.clone())
                .or_default()
                .push((j, commit));
        }
    }
    let n_intervals = audit.intervals.len();
    for i in 0..n_intervals {
        // (split borrows: read the landing map, then mutate interval i)
        let mut resolutions: Vec<(usize, String)> = Vec::new();
        for (line_idx, line) in audit.intervals[i].ledger.iter().enumerate() {
            if line.landing != Landing::Never || line.resolution.is_some() {
                continue;
            }
            if let Some(hits) = landed_after.get(&line.path)
                && let Some((_, commit)) = hits.iter().find(|(j, _)| *j > i)
            {
                resolutions.push((
                    line_idx,
                    format!(
                        "superseded — a later claimed edit to this file landed (in {commit}); an intermediate state, not lost work"
                    ),
                ));
            }
        }
        for (line_idx, why) in resolutions {
            audit.intervals[i].ledger[line_idx].resolution = Some(why);
        }
    }

    // 2. Persisted outside git: the path is ignored, but the claimed
    //    content is sitting on disk right now. The promise was KEPT —
    //    git just cannot see it.
    for interval in audit.intervals.iter_mut() {
        for line in interval
            .ledger
            .iter_mut()
            .filter(|l| l.landing == Landing::Never && l.resolution.is_none())
        {
            let persisted = line.probe.as_deref().is_some_and(|probe| {
                usable_probe(probe)
                    && std::fs::read_to_string(repo.join(&line.path))
                        .is_ok_and(|now| now.contains(probe))
            });
            if persisted && gitio::is_ignored(repo, &line.path) {
                line.resolution = Some(
                    "gitignored, but the claimed content is on disk today — the promise persisted outside git's view"
                        .to_string(),
                );
            }
        }
    }

    // 3. Deliberately removed: from the claim's interval onward, one of
    //    the session's own commands actually removes or moves this exact
    //    path (rm/mv/git rm/git mv/git clean naming it as a whole token).
    //    A mere mention is not enough — a later `echo done > x.log` must
    //    not resolve a genuinely lost `x`.
    for i in 0..n_intervals {
        let mut resolutions: Vec<usize> = Vec::new();
        for (line_idx, line) in audit.intervals[i].ledger.iter().enumerate() {
            if line.landing != Landing::Never || line.resolution.is_some() {
                continue;
            }
            let removed = (i..n_intervals).any(|j| {
                cmd_corpus[j]
                    .iter()
                    .any(|cmd| command_removes_path(cmd, &line.path))
            });
            if removed {
                resolutions.push(line_idx);
            }
        }
        for line_idx in resolutions {
            audit.intervals[i].ledger[line_idx].resolution = Some(
                "a later command in this session removes or moves this exact path — deleted or relocated deliberately, on record, not silently lost".to_string(),
            );
        }
    }

    // Diagnose the survivors: why did git never see this claim?
    for interval in audit.intervals.iter_mut() {
        for line in interval
            .ledger
            .iter_mut()
            .filter(|l| l.landing == Landing::Never && l.resolution.is_none())
        {
            // The forward sweep already checked every later commit, so
            // these are verified statements, not guesses.
            let on_disk_with_content = line.probe.as_deref().is_some_and(|probe| {
                usable_probe(probe)
                    && std::fs::read_to_string(repo.join(&line.path))
                        .is_ok_and(|now| now.contains(probe))
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
