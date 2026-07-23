//! Terminal report: the honest console statement of what balanced and what
//! didn't. Red lines are itemized — a shrug is not a report.

use std::io::IsTerminal;

use crate::extract::Session;
use crate::ingest::IngestStats;
use crate::reconcile::Audit;

/// Follows the git convention: `auto` colors a terminal and strips colors
/// from pipes; `always` keeps ANSI escapes flowing into `bat`, `less -R`,
/// a file for later `cat` — anything that renders them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

struct Style {
    on: bool,
}

impl Style {
    fn new(mode: ColorMode) -> Self {
        let on = match mode {
            ColorMode::Always => true,
            ColorMode::Never => false,
            ColorMode::Auto => {
                std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
            }
        };
        Style { on }
    }
    fn paint(&self, code: &str, s: &str) -> String {
        if self.on && !s.is_empty() {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }
    fn green(&self, s: &str) -> String {
        self.paint("32", s)
    }
    fn red(&self, s: &str) -> String {
        self.paint("31", s)
    }
    fn yellow(&self, s: &str) -> String {
        self.paint("33", s)
    }
    fn dim(&self, s: &str) -> String {
        self.paint("2", s)
    }
    fn bold(&self, s: &str) -> String {
        self.paint("1", s)
    }
    fn cyan(&self, s: &str) -> String {
        self.paint("36", s)
    }
}

/// Collapse the home directory to `~` in a displayed path. Reports get
/// screenshotted and shared; the OS username does not need to ship with
/// them.
fn tilde(path: &str) -> String {
    std::env::home_dir()
        .map(|h| h.display().to_string())
        .and_then(|h| path.strip_prefix(&h).map(|rest| format!("~{rest}")))
        .unwrap_or_else(|| path.to_string())
}

pub fn print(
    session_name: &str,
    repo: &str,
    session: &Session,
    stats: &IngestStats,
    audit: &Audit,
    color: ColorMode,
    show_intent: bool,
) {
    let st = Style::new(color);

    println!("{}", st.bold(&format!("git receipts — {session_name}")));
    println!(
        "repo: {}   branches seen: {}",
        tilde(repo),
        session.branches.join(", ")
    );
    if let (Some(a), Some(b)) = (session.first_ts, session.last_ts) {
        let dur = b - a;
        println!(
            "window: {} → {}  ({}d {}h)",
            a.format("%Y-%m-%d %H:%MZ"),
            b.format("%Y-%m-%d %H:%MZ"),
            dur.num_days(),
            dur.num_hours() % 24
        );
    }
    println!(
        "events: {} kept / {} lines ({} bookkeeping, {} unparseable)",
        stats.kept, stats.lines, stats.skipped_types, stats.unparseable
    );
    println!(
        "claims: {} file mutations · {} commands · {} observations",
        audit.file_claims, audit.commands, audit.observations
    );
    println!(
        "evidence: {} exact · {} receipted · {} claimed · {} dark   ({} failed)",
        st.green(&audit.grades.exact.to_string()),
        st.green(&audit.grades.receipted.to_string()),
        st.yellow(&audit.grades.claimed.to_string()),
        st.red(&audit.grades.dark.to_string()),
        audit.grades.failed
    );
    println!(
        "blast radius: {} local-fs · {} local-git · {} remote-git · {} network · {} read-only",
        audit.radii.local_fs,
        audit.radii.local_git,
        audit.radii.remote_git,
        audit.radii.network,
        audit.radii.read_only
    );

    let green = audit.intervals.iter().filter(|i| i.balanced()).count();
    let total = audit.intervals.len();
    let claims_total: usize = audit.intervals.iter().map(|i| i.ledger.len()).sum();
    let claims_landed: usize = audit
        .intervals
        .iter()
        .map(|i| {
            i.ledger
                .iter()
                .filter(|l| l.landing != crate::reconcile::Landing::Never)
                .count()
        })
        .sum();
    let residue_total: usize = audit.intervals.iter().map(|i| i.residue.len()).sum();
    let pct = |n: usize, d: usize| {
        if d == 0 {
            100.0
        } else {
            100.0 * n as f64 / d as f64
        }
    };

    // ---- intent → outcome + side effects ---------------------------------
    let diag_paths = |prefix: &str| -> usize {
        let mut paths: Vec<&str> = audit
            .intervals
            .iter()
            .flat_map(|i| i.ledger.iter())
            .filter(|l| l.diagnosis.is_some_and(|d| d.starts_with(prefix)))
            .map(|l| l.path.as_str())
            .collect();
        paths.sort_unstable();
        paths.dedup();
        paths.len()
    };
    let gitignored = diag_paths("gitignored");
    let thrown_away = diag_paths("deleted before any commit");
    let mut oor_paths: Vec<&str> = audit.out_of_repo.iter().map(|(p, _)| p.as_str()).collect();
    oor_paths.sort_unstable();
    oor_paths.dedup();

    println!();
    println!("{}", st.bold("intent → outcome"));
    println!(
        "  {} prompts drove {} commits; {}/{} claimed files landed ({:.0}%), {} intervals fully balanced",
        audit.prompts,
        total,
        claims_landed,
        claims_total,
        pct(claims_landed, claims_total),
        green,
    );
    println!("  side effects beyond the repo's history:");
    let side = |n: usize, text: String| {
        if n > 0 {
            println!("    · {text}");
        }
    };
    side(
        audit.out_of_repo.len(),
        format!(
            "writes outside this repo: {} across {} paths (scratch dirs, other repos)",
            audit.out_of_repo.len(),
            oor_paths.len()
        ),
    );
    side(
        audit.radii.network,
        format!("network commands: {}", audit.radii.network),
    );
    side(
        audit.radii.remote_git,
        format!(
            "remote-git commands: {} (push/pull/fetch reached beyond this machine)",
            audit.radii.remote_git
        ),
    );
    side(
        gitignored,
        format!("gitignored write targets: {gitignored} (real writes git never saw)"),
    );
    side(
        thrown_away,
        format!("written, used, deleted before any commit: {thrown_away}"),
    );
    side(
        audit.tail_claims.len(),
        format!(
            "claims after the last commit, still uncommitted: {}",
            audit.tail_claims.len()
        ),
    );
    side(
        audit.grades.failed,
        format!("failed commands or edits: {}", audit.grades.failed),
    );

    let agent_commits = audit.intervals.iter().filter(|i| i.agent_committed).count();
    let keyframes = audit.intervals.len() - agent_commits;
    println!();
    println!(
        "{}",
        st.bold(&format!(
            "interval spine: {} commits ({} agent-committed, {} unclaimed keyframes)",
            audit.intervals.len(),
            agent_commits,
            keyframes
        ))
    );

    for interval in &audit.intervals {
        let never: Vec<_> = interval.never_landed().collect();
        let late: Vec<_> = interval.landed_late().collect();
        let landed = interval.ledger.len() - never.len() - late.len();
        let mark = if interval.balanced() {
            st.green("✔")
        } else {
            st.red("✘")
        };
        let who = if interval.agent_committed {
            ""
        } else {
            " [keyframe: not committed by agent]"
        };
        let ghost = if interval.commit.reachable {
            ""
        } else {
            " [gone from branches — reflog only]"
        };
        let mut subject = interval.commit.subject.clone();
        subject.truncate(56);
        println!(
            "{mark} {} {} {}{}{}",
            interval.commit.short,
            interval.commit.ts.format("%m-%d %H:%M"),
            st.dim(&subject),
            st.yellow(who),
            st.yellow(ghost),
        );
        if show_intent && let Some(first) = interval.intents.first() {
            let mut shown: String = first.chars().take(76).collect();
            if first.chars().count() > 76 {
                shown.push('\u{2026}');
            }
            let more = if interval.intents.len() > 1 {
                format!("  (+{} more)", interval.intents.len() - 1)
            } else {
                String::new()
            };
            println!(
                "    {} {}{}",
                st.cyan("\u{bb} intent:"),
                shown,
                st.dim(&more)
            );
        }
        if interval.spine_jump {
            println!(
                "    {}",
                st.yellow("⑂ spine jump: parent is not the previous commit (rebase/reset/branch)")
            );
        }
        for s in &interval.superseded {
            println!(
                "    {} draft {} committed {} file{}, amended {}s later",
                st.yellow("↻ amend:"),
                s.short,
                s.files,
                if s.files == 1 { "" } else { "s" },
                s.seconds_before_amend
            );
        }
        let late_note = if late.is_empty() {
            String::new()
        } else {
            format!(" / {} landed next commit", late.len())
        };
        println!(
            "    {} claimed / {} landed{} / {} residue   ({} commands)",
            interval.ledger.len(),
            landed,
            late_note,
            interval.residue.len(),
            interval.commands
        );
        for line in &never {
            println!(
                "    {} {} ({} edit{}, f#{})",
                st.red("✘ never landed:"),
                line.path,
                line.edits,
                if line.edits == 1 { "" } else { "s" },
                line.frames.first().copied().unwrap_or(0)
            );
            if let Some(d) = line.diagnosis {
                println!("      {}", st.dim(d));
            }
        }
        for res in &interval.residue {
            println!(
                "    {} {} {}",
                st.yellow("● residue:"),
                res.path,
                st.dim(&format!("[{}] changed, never claimed", res.status))
            );
        }
        if !interval.residue.is_empty() {
            let hint = if interval.effectful_commands > 0 {
                format!(
                    "likely command fallout — {} effectful commands ran in this interval",
                    interval.effectful_commands
                )
            } else {
                "no effectful commands ran — human edit?".to_string()
            };
            println!("      {}", st.dim(&hint));
        }
    }

    if !audit.tail_claims.is_empty() {
        println!();
        println!(
            "{}",
            st.yellow(&format!(
                "uncommitted tail: {} file claims after the last commit (unverifiable now)",
                audit.tail_claims.len()
            ))
        );
        let mut paths: Vec<&str> = audit.tail_claims.iter().map(|(p, _)| p.as_str()).collect();
        paths.sort_unstable();
        paths.dedup();
        for p in paths.iter().take(10) {
            println!("    · {p}");
        }
        if paths.len() > 10 {
            println!("    · … {} more", paths.len() - 10);
        }
    }

    if !audit.out_of_repo.is_empty() {
        let mut paths: Vec<&str> = audit.out_of_repo.iter().map(|(p, _)| p.as_str()).collect();
        paths.sort_unstable();
        paths.dedup();
        println!();
        println!(
            "out-of-repo writes: {} claims across {} paths {}",
            audit.out_of_repo.len(),
            paths.len(),
            st.dim("(outside the audited repo — not in the equation)")
        );
    }

    println!();
    println!(
        "{}",
        st.bold(&format!(
            "balance: {green}/{total} intervals green ({:.0}%) · claims landed {claims_landed}/{claims_total} ({:.0}%) · residue files {residue_total}",
            pct(green, total),
            pct(claims_landed, claims_total),
        ))
    );
}
