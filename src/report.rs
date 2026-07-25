//! Terminal report: the honest console statement of what balanced and what
//! didn't. Red lines are itemized — a shrug is not a report.

use std::io::IsTerminal;

use chrono::{DateTime, Utc};

use crate::extract::Session;
use crate::fmt::{abbrev, command_summary, redact_home, tilde};
use crate::ingest::IngestStats;
use crate::reconcile::{Audit, Interval, Status};

/// Follows the git convention: `auto` colors a terminal and strips colors
/// from pipes; `always` keeps ANSI escapes flowing into `bat`, `less -R`,
/// a file for later `cat` — anything that renders them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

/// Which intervals to list in the spine section. Header, summary, and
/// balance always cover the whole session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Filter {
    /// Every interval.
    #[default]
    All,
    /// Only broken promises: intervals with never-landed claims.
    Red,
    /// Red plus residue-only intervals (unclaimed changes).
    RedResidue,
}

impl Filter {
    pub fn keeps(self, status: Status) -> bool {
        match self {
            Filter::All => true,
            Filter::Red => status == Status::Red,
            Filter::RedResidue => status != Status::Green,
        }
    }
}

/// Output format. `text` is the console ledger; `html` is a
/// self-contained page (redirect it to a file, or pipe it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Format {
    #[default]
    Text,
    Html,
}

/// Which commit drill-downs start expanded in the HTML report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Expand {
    /// Findings open (red + residue), balanced commits collapsed.
    #[default]
    Auto,
    /// Every commit expanded.
    All,
    /// Every commit collapsed.
    None,
}

pub struct Options {
    pub color: ColorMode,
    pub show_intent: bool,
    pub filter: Filter,
    pub format: Format,
    pub expand: Expand,
    /// Console: print each commit's full anatomy — the statement's
    /// added/modified/deleted/renamed files and the commands that ran.
    pub verbose: bool,
    /// Show each command in full (not a one-line summary) with its captured
    /// output. The same depth the JSON receipt carries under `--with-output`
    /// — so console, HTML, and receipt can surface the same detail.
    pub with_output: bool,
    /// Scope the spine to a single commit (full hash), `lspci -s` style. The
    /// header, summary, and balance still cover the whole session; only the
    /// listed intervals are restricted to this one commit.
    pub commit: Option<String>,
    /// Terse spine: one line per commit after the session summary, instead of
    /// the full per-commit drill-down (like `git log --oneline`).
    pub oneline: bool,
    /// Print each commit's full conversation (every prompt and assistant
    /// message) — the whole chat, not just intent + summary. Most useful
    /// scoped with `--commit`; the whole session gets long.
    pub full: bool,
}

/// The 7-char short form of a full commit hash, for display.
fn short_hash(hash: &str) -> &str {
    &hash[..hash.len().min(7)]
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

pub fn print(
    session_name: &str,
    repo: &str,
    session: &Session,
    stats: &IngestStats,
    audit: &Audit,
    opts: &Options,
) {
    let st = Style::new(opts.color);

    println!("{}", st.bold(&format!("git receipts — {session_name}")));
    println!(
        "repo: {}   branches seen: {}",
        tilde(repo),
        session.branches.join(", ")
    );

    // Scoped to one commit (--commit): show only that commit's block — none of
    // the session-wide summary above it. lspci -s: just the addressed device.
    if let Some(h) = &opts.commit {
        let enriched = audit.intervals.iter().any(|i| !i.commit.from_history);
        for (i, interval) in audit.intervals.iter().enumerate() {
            if &interval.commit.hash == h {
                println!();
                render_interval(&st, interval, opts, enriched);
                if opts.full && opts.show_intent {
                    let after = (i > 0).then(|| audit.intervals[i - 1].commit.ts);
                    render_conversation(&st, session, after, Some(interval.commit.ts));
                }
            }
        }
        return;
    }

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
    let dup_note = if stats.duplicates > 0 {
        format!(", {} fork-duplicates removed", stats.duplicates)
    } else {
        String::new()
    };
    println!(
        "events: {} kept / {} lines ({} bookkeeping, {} unparseable{})",
        stats.kept - stats.duplicates,
        stats.lines,
        stats.skipped_types,
        stats.unparseable,
        dup_note
    );
    println!(
        "claims: {} file mutations · {} commands · {} MCP calls · {} observations",
        audit.file_claims, audit.commands, audit.mcp_calls, audit.observations
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
    let red_n = audit
        .intervals
        .iter()
        .filter(|i| i.status() == Status::Red)
        .count();
    let residue_n = audit
        .intervals
        .iter()
        .filter(|i| i.status() == Status::ResidueOnly)
        .count();
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
    let dismissed_total: usize = audit
        .intervals
        .iter()
        .map(|i| i.dismissed_residue.len())
        .sum();
    let attributed_total: usize = audit
        .intervals
        .iter()
        .map(|i| i.attributed_residue.len())
        .sum();
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
    let tk = &session.tokens;
    if tk.requests > 0 {
        println!(
            "  agent effort: {} commands · ~{} output tokens across {} requests (est. from the log; ~{} input, ~{} cached) {}",
            audit.commands,
            abbrev(tk.output),
            tk.requests,
            abbrev(tk.input),
            abbrev(tk.cache_read),
            st.dim("— usage records are approximate, not billing")
        );
    }
    // what happened to every exception — the findings, not just counts
    let all_lines = || audit.intervals.iter().flat_map(|i| i.ledger.iter());
    let late_verified = all_lines()
        .filter(|l| l.landing == crate::reconcile::Landing::Late)
        .count();
    let resolved = |needle: &str| {
        all_lines()
            .filter(|l| l.resolution.as_deref().is_some_and(|r| r.contains(needle)))
            .count()
    };
    let superseded = resolved("superseded");
    let persisted = resolved("persisted outside git");
    let deliberate = resolved("deliberately");
    let resolved_total = superseded + persisted + deliberate;
    let broken = all_lines()
        .filter(|l| l.landing == crate::reconcile::Landing::Never && l.resolution.is_none())
        .count();
    let residue_all = residue_total + attributed_total + dismissed_total;

    println!("  what happened to every exception:");
    if late_verified > 0 {
        println!(
            "    · claims that landed late: {late_verified}, content-verified against the commit they landed in"
        );
    }
    if resolved_total > 0 {
        let mut parts: Vec<String> = Vec::new();
        if superseded > 0 {
            parts.push(format!("superseded by later landed edits: {superseded}"));
        }
        if deliberate > 0 {
            parts.push(format!(
                "removed deliberately by the session's own commands: {deliberate}"
            ));
        }
        if persisted > 0 {
            parts.push(format!("persisted on disk outside git: {persisted}"));
        }
        println!(
            "    · claims that never landed, resolved: {resolved_total} — {}",
            parts.join(" · ")
        );
    }
    // Split the genuine residue by WHO: a change in a keyframe (a commit
    // this session did not make) is another contributor's, attributed by
    // git identity; residue inside an agent commit is unexplained.
    let not_session: usize = audit
        .intervals
        .iter()
        .filter(|i| !i.agent_committed)
        .map(|i| i.residue.len())
        .sum();
    let unexplained: usize = residue_total - not_session;
    if residue_all > 0 {
        println!(
            "    · unclaimed changes (git recorded it, no matching edit claim): {residue_all} — this agent via a command: {attributed_total} · not this session's commit (another contributor): {not_session} · unexplained, inside an agent commit: {unexplained} · dismissed as now ignored/untracked: {dismissed_total}",
        );
    }
    println!(
        "    · {}",
        if broken == 0 {
            st.green("broken promises (claimed, never landed, nothing explains it): 0")
                .to_string()
        } else {
            st.red(&format!(
                "broken promises (claimed, never landed, nothing explains it): {broken}"
            ))
        }
    );

    // Who touched this repo — attribution for free, straight from git.
    // Identity only: a name never says agent-vs-hand-coded; a Co-Authored-By
    // trailer is present-only evidence of an agent/pair, never inferred from
    // absence.
    let keyframes = audit
        .intervals
        .iter()
        .filter(|i| !i.agent_committed)
        .count();
    let mut authors: Vec<&str> = audit
        .intervals
        .iter()
        .map(|i| i.commit.author.as_str())
        .collect();
    authors.sort_unstable();
    authors.dedup();
    let mut coauthors: Vec<&str> = audit
        .intervals
        .iter()
        .flat_map(|i| i.commit.co_authors.iter().map(String::as_str))
        .collect();
    coauthors.sort_unstable();
    coauthors.dedup();
    println!("  who touched this repo (git identity — not how they authored):");
    println!("    · committed by: {}", authors.join(" · "));
    if !coauthors.is_empty() {
        println!(
            "    · co-authored-by (declared in commits): {}",
            coauthors.join(" · ")
        );
    }
    if keyframes > 0 {
        println!(
            "    · {keyframes} commit{} not made by this session — another contributor",
            if keyframes == 1 { "" } else { "s" }
        );
    }

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
    let hist_n = audit
        .intervals
        .iter()
        .filter(|i| i.commit.from_history)
        .count();
    let reflog_n = audit.intervals.len() - hist_n;
    // Only when a reflog exists does "absent from it" mean anything: those
    // commits were created elsewhere (pulled/fetched) — attribution, not a
    // warning. With no reflog, history is simply the spine; say nothing.
    let enriched = reflog_n > 0;
    let source_note = if enriched && hist_n > 0 {
        format!(" · {hist_n} created elsewhere (pulled/fetched)")
    } else {
        String::new()
    };
    let filter_note = if let Some(h) = &opts.commit {
        format!(" — scoped to commit {} (1 of {total})", short_hash(h))
    } else {
        match opts.filter {
            Filter::All => String::new(),
            Filter::Red => format!(" — showing only red ({red_n} of {total})"),
            Filter::RedResidue => format!(
                " — showing red + residue ({} of {total})",
                red_n + residue_n
            ),
        }
    };
    println!(
        "{}",
        st.bold(&format!(
            "interval spine: {} commits ({} agent-committed, {} unclaimed keyframes){}{}",
            audit.intervals.len(),
            agent_commits,
            keyframes,
            source_note,
            filter_note
        ))
    );

    // --commit scoping returned early above, so `opts.commit` is None here.
    if opts.oneline {
        // Terse spine: a column header, then one line per commit — the
        // session summary above already gives the totals (git log --oneline).
        println!(
            "{}",
            st.dim(&format!(
                "{:<9} {:<48} {:>7} {:>7} {:>7} {:>7}",
                "commit", "subject", "claimed", "landed", "residue", "broken"
            ))
        );
        for interval in &audit.intervals {
            if opts.filter.keeps(interval.status()) {
                render_oneline_row(&st, interval);
            }
        }
    } else {
        for (i, interval) in audit.intervals.iter().enumerate() {
            if opts.filter.keeps(interval.status()) {
                render_interval(&st, interval, opts, enriched);
                if opts.full && opts.show_intent {
                    let after = (i > 0).then(|| audit.intervals[i - 1].commit.ts);
                    render_conversation(&st, session, after, Some(interval.commit.ts));
                }
            }
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
            "balance: {green} green · {residue_n} residue-only · {red_n} red of {total} intervals ({:.0}% green) · claims landed {claims_landed}/{claims_total} ({:.0}%) · residue {residue_total} (+{attributed_total} command-attributed, +{dismissed_total} dismissed)",
            pct(green, total),
            pct(claims_landed, claims_total),
        ))
    );
}

/// `--full`: the commit's whole conversation — every prompt and assistant
/// message in the interval's span `(after, until]`, in order. The verbose
/// counterpart to the intent/summary bookends.
fn render_conversation(
    st: &Style,
    session: &Session,
    after: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
) {
    let turns = session.conversation(after, until);
    if turns.is_empty() {
        return;
    }
    println!(
        "    {}",
        st.cyan(&format!("conversation ({} turns):", turns.len()))
    );
    for t in turns {
        let who = if t.user {
            st.bold("you  ")
        } else {
            st.dim("agent")
        };
        let text = redact_home(t.text);
        let mut lines = text.lines();
        println!("      {} {}", who, lines.next().unwrap_or(""));
        for line in lines {
            println!("            {line}");
        }
    }
}

/// One `--oneline` spine row: the commit's short hash (the handle for
/// `--commit`), status mark, subject, and four aligned count columns —
/// claimed, landed, residue, broken. Column-aligned to the header above.
fn render_oneline_row(st: &Style, iv: &Interval) {
    let (mark, paint): (&str, fn(&Style, &str) -> String) = match iv.status() {
        Status::Green => ("✔", Style::green),
        Status::ResidueOnly => ("!", Style::yellow),
        Status::Red => ("✘", Style::red),
    };
    let claimed = iv.ledger.len();
    let landed = iv
        .ledger
        .iter()
        .filter(|l| l.landing != crate::reconcile::Landing::Never)
        .count();
    let n_broken = iv.never_landed().count();
    let residue = iv.residue.len();
    // char-safe truncate + pad so the count columns align (byte ops panic
    // mid-multibyte — a real crash found dogfooding).
    let subject: String = iv.commit.subject.chars().take(48).collect();
    let pad = 48usize.saturating_sub(subject.chars().count());

    // Right-aligned 7-wide count cells; a zero is dimmed so non-zero counts
    // (residue in yellow, broken in red) draw the eye.
    let neutral = |n: usize| {
        let s = format!("{n:>7}");
        if n == 0 { st.dim(&s) } else { s }
    };
    let flagged = |n: usize, paint: fn(&Style, &str) -> String| {
        let s = format!("{n:>7}");
        if n == 0 { st.dim(&s) } else { paint(st, &s) }
    };

    println!(
        "{} {} {}{} {} {} {} {}",
        st.cyan(&iv.commit.short),
        paint(st, mark),
        subject,
        " ".repeat(pad),
        neutral(claimed),
        neutral(landed),
        flagged(residue, Style::yellow),
        flagged(n_broken, Style::red),
    );
}

/// Render one interval's block: the commit line, its tags, intent, the
/// claimed/landed/residue counts, the optional verbose anatomy, and every
/// reconciliation finding (late, resolved, never-landed, residue).
fn render_interval(st: &Style, interval: &Interval, opts: &Options, enriched: bool) {
    let never: Vec<_> = interval.never_landed().collect();
    let resolved: Vec<_> = interval.resolved_never().collect();
    let late: Vec<_> = interval.landed_late().collect();
    let landed = interval.ledger.len() - never.len() - resolved.len() - late.len();
    let mark = match interval.status() {
        Status::Green => st.green("✔"),
        Status::ResidueOnly => st.yellow("!"),
        Status::Red => st.red("✘"),
    };
    // A keyframe is not this session's commit — name who git says made it.
    let who = if interval.agent_committed {
        String::new()
    } else {
        format!(
            " [keyframe: not this session — committed by {}]",
            interval.commit.author
        )
    };
    let ghost = if interval.commit.reachable {
        ""
    } else {
        " [gone from branches — reflog only]"
    };
    let hist = if enriched && interval.commit.from_history {
        " [created elsewhere]"
    } else {
        ""
    };
    // char-safe: byte truncate() panics mid-multibyte (emoji/accent in a
    // commit subject) — a real crash found dogfooding.
    let subject: String = interval.commit.subject.chars().take(56).collect();
    println!(
        "{mark} {} {} {}{}{}{}",
        interval.commit.short,
        interval.commit.ts.format("%m-%d %H:%M"),
        st.dim(&subject),
        st.yellow(&who),
        st.yellow(ghost),
        st.yellow(hist),
    );
    // Co-Authored-By is declared evidence of co-authorship (an agent, a
    // pair). Present-only — never inferred from absence. Per-commit detail
    // goes in --verbose; the summary carries the session-wide picture.
    if opts.verbose && !interval.commit.co_authors.is_empty() {
        println!(
            "    {} {}",
            st.dim("co-authored-by:"),
            st.dim(&interval.commit.co_authors.join(", "))
        );
    }
    if interval.commit.clock_anomaly {
        println!(
                "    {}",
                st.yellow(&format!(
                    "⚠ clock anomaly: dated {}, but the reflog places it between in-window commits — dates on this commit cannot be trusted (backdated?)",
                    interval.commit.committer_ts.format("%Y-%m-%d %H:%M")
                ))
            );
    }
    if opts.show_intent
        && let Some(first) = interval.intents.first()
    {
        let redacted = redact_home(first);
        let mut shown: String = redacted.chars().take(76).collect();
        if redacted.chars().count() > 76 {
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
        format!(" / {} landed late", late.len())
    };
    let mut residue_notes = String::new();
    if !resolved.is_empty() {
        residue_notes.push_str(&format!(" (+{} resolved claims)", resolved.len()));
    }
    if !interval.attributed_residue.is_empty() {
        residue_notes.push_str(&format!(
            " (+{} command-attributed)",
            interval.attributed_residue.len()
        ));
    }
    if !interval.dismissed_residue.is_empty() {
        residue_notes.push_str(&format!(
            " (+{} dismissed)",
            interval.dismissed_residue.len()
        ));
    }
    println!(
        "    {} claimed / {} landed{} / {} residue{}   ({} commands)",
        interval.ledger.len(),
        landed,
        late_note,
        interval.residue.len(),
        residue_notes,
        interval.commands
    );

    if opts.verbose {
        let commit = if interval.agent_committed {
            "committed by agent"
        } else {
            "not committed by agent"
        };
        let push = if interval.pushed {
            "pushed"
        } else {
            "local only"
        };
        println!("    {}", st.dim(&format!("{commit} · {push}")));
        if !interval.statement.is_empty() {
            let n = |s: char| interval.statement.iter().filter(|c| c.status == s).count();
            println!(
                "    {} {} added · {} modified · {} deleted · {} renamed",
                st.dim("files git recorded:"),
                n('A'),
                n('M'),
                n('D'),
                n('R') + n('C'),
            );
            for c in &interval.statement {
                let moved = c
                    .old_path
                    .as_ref()
                    .map(|o| format!("  ← {o}"))
                    .unwrap_or_default();
                println!(
                    "      {} {}{}",
                    st.dim(&format!("[{}]", c.status)),
                    c.path,
                    st.dim(&moved)
                );
            }
        }
        for cr in &interval.commands_run {
            let rad = cr
                .radius
                .map(|r| r.to_string())
                .unwrap_or_else(|| "read-only".into());
            let mut flags = String::new();
            if cr.committed {
                flags.push_str(" ✎commit");
            }
            if cr.pushed {
                flags.push_str(" ↑push");
            }
            if cr.failed {
                flags.push_str(" ✗failed");
            }
            // Show a command in full with its output when asked (--with-output)
            // or when it FAILED — a failure's output is the one you always want.
            if opts.with_output || cr.failed {
                // Full command (possibly multi-line), then its captured
                // output — the depth the JSON receipt carries.
                let full = redact_home(&cr.command);
                let mut lines = full.lines();
                let first = lines.next().unwrap_or("");
                println!(
                    "      {} {}{}",
                    st.dim(&format!("$ [{rad}]")),
                    first,
                    st.yellow(&flags)
                );
                for line in lines {
                    println!("        {line}");
                }
                if let Some(receipt) = &cr.output {
                    let marker = if receipt.is_error {
                        "⤷ error"
                    } else {
                        "⤷ output"
                    };
                    println!("        {}", st.dim(marker));
                    for line in redact_home(&receipt.text).lines() {
                        println!("        {}", st.dim(&format!("  {line}")));
                    }
                }
            } else {
                println!(
                    "      {} {}{}",
                    st.dim(&format!("$ [{rad}]")),
                    redact_home(&command_summary(&cr.command)),
                    st.yellow(&flags)
                );
            }
        }
    }
    for line in &late {
        let (at, dist) = line
            .landed_at
            .as_ref()
            .map(|(s, d)| (s.as_str(), *d))
            .unwrap_or(("?", 1));
        let commits = if dist == 1 { "commit" } else { "commits" };
        println!(
            "    {} {} {}",
            st.green("✓ landed late:"),
            line.path,
            st.dim(&format!(
                "(content verified in {at}, {dist} {commits} later)"
            ))
        );
    }
    for line in &resolved {
        println!(
            "    {} {} ({} edit{}, f#{})",
            st.yellow("◌ never landed, resolved:"),
            line.path,
            line.edits,
            if line.edits == 1 { "" } else { "s" },
            line.frames.first().copied().unwrap_or(0)
        );
        if let Some(w) = &line.resolution {
            println!("      {}", st.dim(w));
        }
    }
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
    for (res, why) in &interval.attributed_residue {
        let moved = res
            .old_path
            .as_ref()
            .map(|o| format!(" (was {o})"))
            .unwrap_or_default();
        println!(
            "    {} {}{} {}",
            st.yellow("● residue (attributed):"),
            res.path,
            st.dim(&moved),
            st.dim(&format!("[{}] — {why}", res.status))
        );
    }
    for (res, why) in &interval.dismissed_residue {
        println!(
            "    {}",
            st.dim(&format!(
                "○ residue dismissed: {} [{}] — {why}",
                res.path, res.status
            ))
        );
    }
    if !interval.residue.is_empty() {
        let hint = if !interval.agent_committed {
            // a whole commit this session didn't make — attribute it to
            // the committer git records, not "human edit".
            format!(
                "not this session's commit — committed by {}",
                interval.commit.author
            )
        } else if interval.effectful_commands > 0 {
            format!(
                "likely command fallout — {} effectful commands ran in this interval",
                interval.effectful_commands
            )
        } else {
            "no effectful commands ran in this agent commit — possibly a human edit".to_string()
        };
        println!("      {}", st.dim(&hint));
    }

    // The agent's own account of this commit, closing the block — the
    // readable claim, not proof (the ledger above is what's verified).
    if opts.show_intent
        && let Some(summary) = &interval.summary
    {
        let flat = redact_home(summary).replace('\n', " ");
        let mut shown: String = flat.chars().take(140).collect();
        if flat.chars().count() > 140 {
            shown.push('\u{2026}');
        }
        println!("    {} {}", st.cyan("\u{ab} summary:"), shown);
    }
}
