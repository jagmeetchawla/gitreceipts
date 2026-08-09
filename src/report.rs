//! Terminal report: the honest console statement of what balanced and what
//! didn't. Red lines are itemized — a shrug is not a report.

use std::io::IsTerminal;

use chrono::{DateTime, Utc};

use crate::extract::Session;
use crate::fmt::{abbrev, command_summary, redact_home, scan_counts};
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

/// Which intervals to list in the spine section, filtered purely by COLOR.
/// Header, summary, and balance always cover the whole session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Filter {
    /// Every interval.
    #[default]
    All,
    /// Red only: broken promises (a claimed edit git never got).
    Red,
    /// Amber only: worth a look — residue, a failed command, or an errored MCP.
    Amber,
    /// Green only: clean intervals.
    Green,
    /// Grey only: explained findings (scratch churn, failed commands, MCP
    /// errors) — each carries its explanation.
    Grey,
    /// Red + amber: the UNANSWERED findings. Grey (answered) and green are
    /// both excluded.
    RedAmber,
}

impl Filter {
    pub fn keeps(self, status: Status) -> bool {
        match self {
            Filter::All => true,
            Filter::Red => status == Status::Red,
            Filter::Amber => status == Status::Amber,
            Filter::Green => status == Status::Green,
            Filter::Grey => status == Status::Grey,
            Filter::RedAmber => matches!(status, Status::Red | Status::Amber),
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
    /// Findings open (red + amber), green commits collapsed.
    #[default]
    Auto,
    /// Every commit expanded.
    All,
    /// Every commit collapsed.
    None,
}

/// Which coding agent produced the session being audited. The reconciliation
/// against git is agent-neutral; everything that reads the *log* — its JSONL
/// shape, the store location, how claims/commands/prompts are extracted — is
/// agent-specific. Only Claude Code is supported in v0.1; this reserves the
/// switch so other agents are an additive change, never a breaking one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Agent {
    /// Claude Code — the `~/.claude/projects` session JSONL.
    #[default]
    Claude,
}

impl Agent {
    /// The stable identifier recorded in the receipt's `source.agent`.
    pub fn source(self) -> &'static str {
        match self {
            Agent::Claude => "claude-code",
        }
    }
}

/// Which conversational content to render. Suppression is orthogonal to
/// redaction: `--no-prompt`/`--no-summary` DROP a whole category; `--redact`/
/// `--no-identity`/`--no-scan` MASK spans in place. `--no-intent` drops both.
#[derive(Debug, Clone, Copy)]
pub struct Show {
    /// The user's prompt text — intent lines, and user turns in `--full`.
    pub prompt: bool,
    /// The agent's prose — post-commit summaries, and assistant turns in `--full`.
    pub summary: bool,
}

pub struct Options {
    pub color: ColorMode,
    pub show: Show,
    /// Show git-identity names/emails (the "who touched this repo" roll-up and
    /// per-commit committer/co-author lines). False (`--no-identity`) keeps the
    /// counts and attribution but drops the names — for sharing a report
    /// without exposing contributors.
    pub show_identity: bool,
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
    /// The canonical condensed view: headline + spine table (commit ·
    /// subject · claims · findings) — every finding any age, recent tail,
    /// zeros never printed, every cause named. The agent-native default.
    pub summary: bool,
    /// Status dots as emoji (🟢⚪🟡🔴) instead of ANSI-colored marks — for
    /// chat surfaces where terminal color is stripped.
    pub emoji: bool,
    /// HTML: keep the page small enough to open anywhere — cap the full
    /// text and captured output of failed commands, and the per-commit
    /// file/command lists. Nothing is dropped silently; every cap says how
    /// much it hid.
    pub compact: bool,
    /// Recap framing: lead with what was ASKED and what happened, mute the
    /// verdict marks, and close with what to run next. The numbers are the
    /// same receipt the audit renders — only the emphasis differs.
    pub narrative: bool,
    /// Print each commit's full conversation (every prompt and assistant
    /// message) — the whole chat, not just intent + summary. Most useful
    /// scoped with `--commit`; the whole session gets long.
    pub full: bool,
    /// This report is one repo's section inside a `--project` view: skip the
    /// redundant session-name line and privacy notice (the project header
    /// carries them once).
    pub project_section: bool,
    /// The OTHER project repos' roots (empty outside `--project`). Their
    /// out-of-repo writes are named and counted, never pathed — so a per-repo
    /// section is safe to share without exposing a sibling's file tree.
    pub siblings: Vec<std::path::PathBuf>,
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

/// A `--project` view's shared masthead: the project path, the repo count, and
/// the one privacy notice the per-repo sections then omit.
pub fn project_header(project_display: &str, repo_count: usize, color: ColorMode) {
    let st = Style::new(color);
    println!(
        "{}",
        st.bold(&format!(
            "git receipts — project {}",
            redact_home(project_display)
        ))
    );
    println!(
        "{} git repo{} with sessions under this project",
        repo_count,
        if repo_count == 1 { "" } else { "s" }
    );
    println!(
        "{}",
        st.dim(
            "⚠ private audit report — from your chat/agent logs, git, and command output; handle with caution before sharing."
        )
    );
}

/// The `--project` roll-up: one row per repo — verdict, commits, landed/claims,
/// broken, residue — so you see where the work landed before reading the
/// per-repo sections. Numbers come from [`Audit::landing_summary`], the same
/// source the JSON wrapper uses.
pub fn landing_table(
    rows: &[(String, crate::reconcile::LandingSummary)],
    no_sessions: &[String],
    color: ColorMode,
) {
    let st = Style::new(color);
    let name_w = rows
        .iter()
        .map(|(n, _)| n.chars().count())
        .chain(no_sessions.iter().map(|n| n.chars().count()))
        .max()
        .unwrap_or(4)
        .max(4);
    println!();
    println!("{}", st.bold("where it landed"));
    println!(
        "  {}",
        st.dim(&format!(
            "{:<name_w$}  {:>7}  {:>11}  {:>6}  {:>7}  verdict",
            "repo", "commits", "landed", "broken", "residue",
        ))
    );
    for (name, s) in rows {
        let (dot, word) = match s.verdict {
            Status::Green => (st.green("●"), st.green("green")),
            Status::Grey => (st.dim("●"), st.dim("grey")),
            Status::Amber => (st.yellow("●"), st.yellow("amber")),
            Status::Red => (st.red("●"), st.red("red")),
        };
        // Pad to width FIRST, then colorize — ANSI escapes have no display
        // width, so `{:>N}` on an already-colored string would misalign.
        // broken (>0) and residue (>0) are the amber/red facts — dim the zeros
        // so the rows that warrant a look are the ones that stand out.
        let broken = format!("{:>6}", s.broken);
        let broken = if s.broken == 0 {
            st.dim(&broken)
        } else {
            st.red(&broken)
        };
        let residue = format!("{:>7}", s.residue_files);
        let residue = if s.residue_files == 0 {
            st.dim(&residue)
        } else {
            st.yellow(&residue)
        };
        println!(
            "  {:<name_w$}  {:>7}  {:>11}  {}  {}  {} {}",
            name,
            s.commits,
            format!("{}/{}", s.landed, s.claims),
            broken,
            residue,
            dot,
            word,
        );
    }
    // Repos that are part of the project but have no sessions in the store.
    // Listing them keeps the table's row count equal to what is on disk —
    // a roll-up that quietly omits repos looks complete and isn't.
    for name in no_sessions {
        println!(
            "  {:<name_w$}  {}",
            name,
            st.dim("—        —        —        —  no sessions in the store")
        );
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
    if opts.summary {
        print_summary(audit, opts);
        return;
    }
    let st = Style::new(opts.color);
    // A per-commit model line is only worth printing when the session spanned
    // more than one model (a mid-session switch); otherwise the header roll-up
    // says it once and per-commit would just repeat.
    let multi_model = session.models_used(None, None).len() > 1;

    // In a --project view the project header carries the session-name line and
    // the privacy notice once, up top; a repo section repeats neither.
    if !opts.project_section {
        println!(
            "{}",
            st.bold(&format!("git receipts — {}", redact_home(session_name)))
        );
    }
    println!(
        "repo: {}   branches seen: {}",
        redact_home(repo),
        redact_home(&session.branches.join(", "))
    );
    if !opts.project_section {
        // Private by default: built from chat/agent logs, git contents, and
        // command output. Warn before anyone shares it (redaction reduces,
        // never removes).
        println!(
            "{}",
            st.dim(
                "⚠ private audit report — from your chat/agent logs, git, and command output; handle with caution before sharing."
            )
        );
    }

    // Scoped to one commit (--commit): show only that commit's block — none of
    // the session-wide summary above it. lspci -s: just the addressed device.
    if let Some(h) = &opts.commit {
        let enriched = audit.intervals.iter().any(|i| !i.commit.from_history);
        for (i, interval) in audit.intervals.iter().enumerate() {
            if &interval.commit.hash == h {
                println!();
                let after = (i > 0).then(|| audit.intervals[i - 1].commit.ts);
                let prov = commit_provenance(session, after, interval.commit.ts, multi_model);
                let cost = session.cost_in(after, Some(interval.commit.ts));
                render_interval(&st, interval, opts, enriched, prov, cost);
                if opts.full && (opts.show.prompt || opts.show.summary) {
                    render_conversation(&st, session, after, Some(interval.commit.ts), opts.show);
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
    // Execution-axis FACTS, per oracle — surfaced, not scored (a non-zero exit
    // is often benign; the auditor judges). MCP is first-class here.
    println!(
        "  {} {} commands · {} failed · {} aborted by you",
        st.dim("OS/FS:"),
        audit.commands,
        audit.cmd_failed,
        audit.cmd_aborted,
    );
    println!(
        "  {}   {} calls · {} errored · {} aborted by you",
        st.dim("MCP:  "),
        audit.mcp_calls,
        audit.mcp_errored,
        audit.mcp_aborted,
    );
    // Provenance — who attested each claim (a fact, not a grade): git durably
    // verified it (landed) · an executor returned a receipt (receipted) · only
    // the agent's word, nothing corroborated (claimed). Ladder: claimed <
    // receipted < landed. See "report facts, not grades" — only git lands.
    let landed: usize = audit
        .intervals
        .iter()
        .flat_map(|i| i.ledger.iter())
        .filter(|l| l.landing != crate::reconcile::Landing::Never)
        .count();
    let claims_tot: usize = audit.intervals.iter().map(|i| i.ledger.len()).sum();
    let receipted = audit.grades.receipted + audit.grades.claimed + audit.mcp_calls;
    let claimed_only = claims_tot.saturating_sub(landed) + audit.grades.dark;
    println!(
        "provenance {}: {landed} landed in git · {receipted} receipted by an executor · {claimed_only} claimed only",
        st.dim("(who attested)")
    );
    println!(
        "blast radius: {} local-fs · {} local-git · {} remote-git · {} network · {} read-only",
        audit.radii.local_fs,
        audit.radii.local_git,
        audit.radii.remote_git,
        audit.radii.network,
        audit.radii.read_only
    );

    // Headline counts over the EQUATION set (the agent's own commits by default,
    // everything under --full-history) — one source, so console/HTML/JSON agree
    // and a teammate's keyframe never shapes your numbers.
    let c = audit.counts();
    let green = c.green;
    let total = c.total;
    let red_n = c.red;
    let residue_n = c.amber;
    let grey_n = c.grey;
    let claims_total = c.claims_total;
    let claims_landed = c.claims_landed;
    let keyframes_excluded = c.keyframes_excluded;
    // Exception + attribution aggregates — computed once on the Audit and shared
    // with the HTML and JSON receipt, so all three surfaces show the same numbers.
    let ex = audit.exceptions();
    let residue_total = ex.residue;
    let dismissed_total = ex.dismissed;
    let attributed_total = ex.unclaimed_by_command;
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
    // Sibling protection: in a --project section, writes into a SIBLING project
    // repo are named and counted here, never pathed. Outside --project the
    // sibling list is empty, so everything stays "external" as before.
    let (external_oor, sibling_writes) = audit.partition_out_of_repo(&opts.siblings);
    let mut oor_paths: Vec<&str> = external_oor.iter().map(|(p, _)| p.as_str()).collect();
    oor_paths.sort_unstable();
    oor_paths.dedup();
    let sibling_changes: usize = sibling_writes.iter().map(|s| s.changes).sum();

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
            "  agent effort: {} commands · ~{} output tokens across {} requests (est. from the log; ~{} input, ~{} cache read, ~{} cache write) {}",
            audit.commands,
            abbrev(tk.output),
            tk.requests,
            abbrev(tk.input),
            abbrev(tk.cache_read),
            abbrev(tk.cache_creation),
            st.dim("— usage records are approximate, not billing")
        );
    }
    // Provenance: which model(s) produced the session, and — where the log
    // carries it — the reasoning effort. Model is authoritative (every turn);
    // effort is sparse, so it is labelled with its coverage, never as complete.
    let models = session.models_used(None, None);
    if !models.is_empty() {
        let list = models
            .iter()
            .map(|(m, c)| format!("{} ({} req)", short_model(m), c))
            .collect::<Vec<_>>()
            .join(", ");
        println!("  {} {}", st.dim("model(s):"), list);
    }
    let (efforts, coverage) = session.effort_seen(None, None);
    if !efforts.is_empty() {
        println!(
            "  {} {} {}",
            st.dim("reasoning effort:"),
            efforts.join(", "),
            st.dim(&format!("({} coverage in the log)", coverage.as_str()))
        );
    }
    // what happened to every exception — the findings, not just counts
    let late_verified = ex.landed_late;
    let superseded = ex.resolved_superseded;
    let persisted = ex.resolved_persisted;
    let deliberate = ex.resolved_deliberate;
    let relocated = ex.resolved_relocated;
    let scratch = ex.resolved_scratch;
    let resolved_total = superseded + persisted + deliberate + relocated + scratch;
    // Equation-scoped, same as the balance/tiles — a keyframe's claims are not
    // this agent's broken promises.
    let broken = c.broken;
    let residue_all = ex.unclaimed_total;

    println!("  what happened to every exception:");
    if late_verified > 0 {
        println!(
            "    · claims that landed late: {late_verified}, content-verified against the commit they landed in (a formatter may have rewrapped the text first — those say so)"
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
        if relocated > 0 {
            parts.push(format!(
                "relocated before first commit, landed at a different path (content-verified): {relocated}"
            ));
        }
        if scratch > 0 {
            parts.push(format!(
                "written and discarded before any commit (scratch churn; ambers): {scratch}"
            ));
        }
        println!(
            "    · claims that never landed, resolved: {resolved_total} — {}",
            parts.join(" · ")
        );
    }
    // Split the genuine residue by WHO: a change in a keyframe (a commit
    // this session did not make) is another contributor's, attributed by
    // git identity; residue inside an agent commit is unexplained.
    let not_session = ex.unclaimed_other_contributor;
    let unexplained = ex.unclaimed_unexplained;
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
    // Your unexplained residue — unique files, your own commits (deduped). The
    // honest "residue" number, distinct from the per-event unclaimed total above.
    let residue_files = audit.residue_files();
    println!(
        "    · your unexplained residue: {residue_files} file{} (unique, in your own commits)",
        if residue_files == 1 { "" } else { "s" }
    );

    // Who touched this repo — attribution for free, straight from git.
    // Identity only: a name never says agent-vs-hand-coded; a Co-Authored-By
    // trailer is present-only evidence of an agent/pair, never inferred from
    // absence.
    let keyframes = ex.keyframes;
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
    if opts.show_identity {
        println!("    · committed by: {}", authors.join(" · "));
        if !coauthors.is_empty() {
            println!(
                "    · co-authored-by (declared in commits): {}",
                coauthors.join(" · ")
            );
        }
    } else {
        let plural = |n: usize, s| if n == 1 { s } else { "identities" };
        println!(
            "    · {} committer {} · {} co-author {} (names hidden — --no-identity)",
            authors.len(),
            plural(authors.len(), "identity"),
            coauthors.len(),
            plural(coauthors.len(), "identity"),
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
        external_oor.len(),
        format!(
            "writes outside this repo: {} across {} paths (scratch dirs, other repos)",
            external_oor.len(),
            oor_paths.len()
        ),
    );
    if !sibling_writes.is_empty() {
        let named = sibling_writes
            .iter()
            .map(|s| format!("{} ({} change{})", s.name, s.changes, plural(s.changes, "")))
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "    · writes into sibling project repo{}: {} {}",
            plural(sibling_writes.len(), ""),
            named,
            st.dim("— audit each directly; paths withheld here")
        );
    }
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
        ex.failed_commands_or_edits,
        format!("failed commands or edits: {}", ex.failed_commands_or_edits),
    );

    let agent_commits = ex.agent_committed;
    let keyframes = ex.keyframes;
    println!();
    let hist_n = ex.created_elsewhere;
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
            Filter::Amber => format!(" — showing only amber ({residue_n} of {total})"),
            Filter::Green => format!(" — showing only green ({green} of {total})"),
            Filter::Grey => format!(" — showing only grey ({grey_n} of {total})"),
            Filter::RedAmber => {
                format!(" — showing red + amber ({} of {total})", red_n + residue_n)
            }
        }
    };
    let spine_header = if audit.full_history {
        format!(
            "interval spine: {} commits ({} agent-committed, {} unclaimed keyframes){}{}",
            total, agent_commits, keyframes, source_note, filter_note
        )
    } else {
        // Default: the spine IS the agent's own commits; commits by others in the
        // window are held out (a teammate's work is not the agent's account).
        let held = if keyframes_excluded > 0 {
            format!(
                " · {keyframes_excluded} commit{} by others held out ({})",
                if keyframes_excluded == 1 { "" } else { "s" },
                st.dim("--full-history to include")
            )
        } else {
            String::new()
        };
        // source_note ("N created elsewhere") stays in both modes — it's a
        // whole-spine provenance fact the JSON also carries; dropping it here
        // would desync the formats.
        format!(
            "interval spine: {} agent commit{}{}{}{}",
            total,
            if total == 1 { "" } else { "s" },
            held,
            source_note,
            filter_note
        )
    };
    println!("{}", st.bold(&spine_header));

    let full_history = audit.full_history;
    let in_eq = |iv: &Interval| full_history || iv.agent_committed;
    // --commit scoping returned early above, so `opts.commit` is None here.
    if opts.oneline {
        // Terse spine: a column header, then one line per commit — the
        // session summary above already gives the totals (git log --oneline).
        println!(
            "{}",
            st.dim(&format!(
                "{:<9}   {:<48} {:>7}  {}",
                "commit", "subject", "claims", "findings"
            ))
        );
        for interval in &audit.intervals {
            if in_eq(interval) && opts.filter.keeps(interval.status()) {
                render_oneline_row(&st, interval, opts.emoji);
            }
        }
    } else {
        for (i, interval) in audit.intervals.iter().enumerate() {
            if in_eq(interval) && opts.filter.keeps(interval.status()) {
                let after = (i > 0).then(|| audit.intervals[i - 1].commit.ts);
                let prov = commit_provenance(session, after, interval.commit.ts, multi_model);
                let cost = session.cost_in(after, Some(interval.commit.ts));
                render_interval(&st, interval, opts, enriched, prov, cost);
                if opts.full && (opts.show.prompt || opts.show.summary) {
                    render_conversation(&st, session, after, Some(interval.commit.ts), opts.show);
                }
            }
        }
    }

    if !external_oor.is_empty() {
        let mut paths: Vec<&str> = external_oor.iter().map(|(p, _)| p.as_str()).collect();
        paths.sort_unstable();
        paths.dedup();
        println!();
        println!(
            "out-of-repo writes: {} claims across {} paths {}",
            external_oor.len(),
            paths.len(),
            st.dim("(outside the audited repo — not in the equation)")
        );
    }
    if sibling_changes > 0 {
        println!(
            "sibling-repo writes: {} change{} across {} repo{} {}",
            sibling_changes,
            plural(sibling_changes, ""),
            sibling_writes.len(),
            plural(sibling_writes.len(), ""),
            st.dim("(other project repos — counted, paths withheld)")
        );
    }

    println!();
    println!(
        "{}",
        st.bold(&format!(
            "balance: {green} green · {grey_n} grey · {residue_n} amber · {red_n} red of {total} intervals ({:.0}% green) · claims landed {claims_landed}/{claims_total} ({:.0}%) · residue {residue_total} (+{attributed_total} command-attributed, +{dismissed_total} dismissed)",
            pct(green, total),
            pct(claims_landed, claims_total),
        ))
    );

    // Secret scanner tally — accrues as commands/output pass through redaction
    // above, so it is complete by here.
    let (secrets, pii) = scan_counts();
    if secrets + pii > 0 {
        println!(
            "{}",
            st.yellow(&format!(
                "🔒 redacted {secrets} secret{} and {pii} PII value{} the scanner recognized (masked in place; values never shown)",
                if secrets == 1 { "" } else { "s" },
                if pii == 1 { "" } else { "s" },
            ))
        );
    }
}

/// `--full`: the commit's whole conversation — every prompt and assistant
/// message in the interval's span `(after, until]`, in order. The verbose
/// counterpart to the intent/summary bookends.
fn render_conversation(
    st: &Style,
    session: &Session,
    after: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
    show: Show,
) {
    // Suppression applies inside --full too: --no-prompt drops your turns,
    // --no-summary drops the agent's.
    let turns: Vec<_> = session
        .conversation(after, until)
        .into_iter()
        .filter(|t| if t.user { show.prompt } else { show.summary })
        .collect();
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
/// `--commit`), status dot, subject, claims (landed/claimed) and the
/// findings cell — one grammar with `--summary`: zeros never print, every
/// cause named, `—` means clean.
fn render_oneline_row(st: &Style, iv: &Interval, emoji: bool) {
    let claimed = iv.ledger.len();
    let landed = iv
        .ledger
        .iter()
        .filter(|l| l.landing != crate::reconcile::Landing::Never)
        .count();
    // char-safe truncate + pad so the columns align (byte ops panic
    // mid-multibyte — a real crash found dogfooding).
    let subject: String = redact_home(&iv.commit.subject).chars().take(48).collect();
    let pad = 48usize.saturating_sub(subject.chars().count());

    println!(
        "{} {} {}{} {:>7}  {}",
        st.cyan(&iv.commit.short),
        status_dot(st, iv.status(), emoji),
        subject,
        " ".repeat(pad),
        format!("{landed}/{claimed}"),
        findings_cell(iv),
    );
}

/// Drop the `claude-` prefix for a tidier console label; the full id stays in
/// the JSON receipt. `codex-x` and other non-Claude ids pass through untouched.
fn short_model(id: &str) -> &str {
    id.strip_prefix("claude-").unwrap_or(id)
}

/// `word` for one, `words` for any other count.
fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        word.to_string()
    } else {
        format!("{word}s")
    }
}

/// The per-commit provenance line — the model(s) that drove this interval and,
/// where logged, its reasoning effort. Returns `None` (no line) when the
/// session used a single model AND no effort was captured here: the header
/// roll-up already says everything, so a per-commit echo would be pure noise.
/// It earns a line only when the model varies across the session or effort adds
/// something. `until` is the commit ts; `after` the previous spine commit's.
fn commit_provenance(
    session: &Session,
    after: Option<DateTime<Utc>>,
    until: DateTime<Utc>,
    multi_model: bool,
) -> Option<String> {
    let models = session.models_used(after, Some(until));
    let (efforts, coverage) = session.effort_seen(after, Some(until));
    if models.is_empty() || (!multi_model && efforts.is_empty()) {
        return None;
    }
    let m = models
        .iter()
        .map(|(id, _)| short_model(id))
        .collect::<Vec<_>>()
        .join(", ");
    let etail = if efforts.is_empty() {
        String::new()
    } else {
        format!(" · effort: {} ({})", efforts.join(","), coverage.as_str())
    };
    Some(format!("⚙ model: {m}{etail}"))
}

/// Render one interval's block: the commit line, its tags, intent, the
/// claimed/landed/residue counts, the optional verbose anatomy, and every
/// reconciliation finding (late, resolved, never-landed, residue).
fn render_interval(
    st: &Style,
    interval: &Interval,
    opts: &Options,
    enriched: bool,
    prov: Option<String>,
    cost: (usize, u64),
) {
    let never: Vec<_> = interval.never_landed().collect();
    let resolved: Vec<_> = interval.resolved_never().collect();
    let late: Vec<_> = interval.landed_late().collect();
    let landed = interval.ledger.len() - never.len() - resolved.len() - late.len();
    let mark = match interval.status() {
        Status::Green => st.green("✔"),
        Status::Grey => st.dim("○"),
        Status::Amber => st.yellow("!"),
        Status::Red => st.red("✘"),
    };
    // A keyframe is not this session's commit — name who git says made it.
    let who = if interval.agent_committed {
        String::new()
    } else if opts.show_identity {
        format!(
            " [keyframe: not this session — committed by {}]",
            interval.commit.author
        )
    } else {
        " [keyframe: not this session — another contributor]".to_string()
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
    let subject: String = redact_home(&interval.commit.subject)
        .chars()
        .take(56)
        .collect();
    println!(
        "{mark} {} {} {}{}{}{}",
        interval.commit.short,
        interval.commit.ts.format("%m-%d %H:%M"),
        st.dim(&subject),
        st.dim(&who),
        st.dim(ghost),
        st.dim(hist),
    );
    if let Some(p) = &prov {
        println!("    {}", st.dim(p));
    }
    // Co-Authored-By is declared evidence of co-authorship (an agent, a
    // pair). Present-only — never inferred from absence. Per-commit detail
    // goes in --verbose; the summary carries the session-wide picture.
    if opts.verbose && opts.show_identity && !interval.commit.co_authors.is_empty() {
        println!(
            "    {} {}",
            st.dim("co-authored-by:"),
            st.dim(&interval.commit.co_authors.join(", "))
        );
    }
    if interval.commit.clock_anomaly {
        println!(
                "    {}",
                st.dim(&format!(
                    "⚠ clock anomaly: dated {}, but the reflog places it between in-window commits — dates on this commit cannot be trusted (backdated?)",
                    interval.commit.committer_ts.format("%Y-%m-%d %H:%M")
                ))
            );
    }
    if opts.show.prompt
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
            st.dim("⑂ spine jump: parent is not the previous commit (rebase/reset/branch)")
        );
    }
    for s in &interval.superseded {
        println!(
            "    {} draft {} committed {} file{}, amended {}s later",
            st.dim("↻ amend:"),
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
        "    {} claimed / {} landed{} / {} residue{}",
        interval.ledger.len(),
        landed,
        late_note,
        interval.residue.len(),
        residue_notes,
    );
    // Line 2 — the work behind this commit: commands, MCP calls, and the agent
    // token cost of its conversation window. Each part appears only when it has
    // something to say, and the whole line is skipped for an empty keyframe.
    let (creq, cout) = cost;
    let mcp = interval.mcp_runs.len();
    let mut work: Vec<String> = Vec::new();
    if interval.commands > 0 {
        work.push(format!(
            "{} {}",
            interval.commands,
            plural(interval.commands, "command")
        ));
    }
    if mcp > 0 {
        work.push(format!("{mcp} MCP {}", plural(mcp, "call")));
    }
    if creq > 0 {
        work.push(format!(
            "{creq} {} · ~{} output tokens",
            plural(creq, "request"),
            abbrev(cout)
        ));
    }
    if !work.is_empty() {
        println!("    {}", st.dim(&work.join(" · ")));
    }

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
                    redact_home(&c.path),
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
        // MCP calls — the execution axis. The tool_result is the oracle:
        // errored is shown amber (a fact worth a look, not a red verdict);
        // output is surfaced on error or --with-output.
        for m in &interval.mcp_runs {
            let head = format!("⚡ mcp {}/{}", m.server, m.tool);
            let flag = if m.errored {
                st.yellow("  ✗ errored")
            } else {
                String::new()
            };
            if opts.with_output || m.errored {
                println!("      {}{}", st.dim(&head), flag);
                for line in redact_home(&m.input).lines() {
                    println!("        {line}");
                }
                if let Some(receipt) = &m.output {
                    let marker = if receipt.is_error {
                        "⤷ error"
                    } else {
                        "⤷ result"
                    };
                    println!("        {}", st.dim(marker));
                    for line in redact_home(&receipt.text).lines() {
                        println!("        {}", st.dim(&format!("  {line}")));
                    }
                }
            } else {
                let input1: String = redact_home(&m.input).chars().take(100).collect();
                println!("      {}  {}", st.dim(&head), st.dim(&input1));
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
            redact_home(&line.path),
            st.dim(&format!(
                "(content verified in {at}, {dist} {commits} later{})",
                if line.reformatted {
                    " — reformatted before landing"
                } else {
                    ""
                }
            ))
        );
    }
    for line in &resolved {
        println!(
            "    {} {} ({} edit{}, f#{})",
            st.yellow("◌ never landed, resolved:"),
            redact_home(&line.path),
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
            redact_home(&line.path),
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
            redact_home(&res.path),
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
            redact_home(&res.path),
            st.dim(&moved),
            st.dim(&format!("[{}] — {why}", res.status))
        );
    }
    for (res, why) in &interval.dismissed_residue {
        println!(
            "    {}",
            st.dim(&format!(
                "○ residue dismissed: {} [{}] — {why}",
                redact_home(&res.path),
                res.status
            ))
        );
    }
    if !interval.residue.is_empty() {
        let hint = if !interval.agent_committed {
            // a whole commit this session didn't make — attribute it to
            // the committer git records, not "human edit".
            if opts.show_identity {
                format!(
                    "not this session's commit — committed by {}",
                    interval.commit.author
                )
            } else {
                "not this session's commit — another contributor".to_string()
            }
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
    if opts.show.summary
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

// ---------------------------------------------------------------------------
// The condensed summary view (`--summary`) — the canonical agent-native
// table. One grammar everywhere: header row, zeros never printed, every
// cause named, `—` means clean. Shared by `--oneline`'s findings column.
// ---------------------------------------------------------------------------

/// The status dot for a summary/oneline row. Emoji for chat surfaces
/// (terminal colors get stripped there); ANSI-painted marks otherwise.
fn status_dot_muted(st: &Style, s: Status) -> String {
    // Recap names findings without shouting them: one dim character, and
    // nothing at all where there is nothing to say.
    st.dim(match s {
        Status::Green => " ",
        Status::Grey => "·",
        Status::Amber => "!",
        Status::Red => "✘",
    })
}

fn status_dot(st: &Style, s: Status, emoji: bool) -> String {
    if emoji {
        match s {
            Status::Green => "🟢".into(),
            Status::Grey => "⚪".into(),
            Status::Amber => "🟡".into(),
            Status::Red => "🔴".into(),
        }
    } else {
        match s {
            Status::Green => st.green("✔"),
            Status::Grey => st.dim("○"),
            Status::Amber => st.yellow("!"),
            Status::Red => st.red("✘"),
        }
    }
}

/// The findings column: one consistent rule — zero counts never print,
/// every nonzero cause is named, a clean row shows `—`. Failed commands
/// name their program (skipping VAR= prefixes) and only GENUINE failures
/// read as "failed"; triaged-benign classes carry their own labels.
pub fn findings_cell(iv: &Interval) -> String {
    let mut fs: Vec<String> = Vec::new();
    if !iv.residue.is_empty() {
        fs.push(format!("{} residue", iv.residue.len()));
    }
    let scratch = iv.ledger.iter().filter(|l| l.scratch).count();
    if scratch > 0 {
        fs.push(format!("{scratch} scratch (discarded pre-commit)"));
    }
    let prog = |c: &str| -> String {
        let t = c
            .split_whitespace()
            .find(|t| !t.contains('='))
            .unwrap_or("?");
        t.rsplit('/').next().unwrap_or(t).to_string()
    };
    let mut genuine: Vec<String> = Vec::new();
    let mut by_class: std::collections::BTreeMap<&'static str, usize> = Default::default();
    for r in iv.commands_run.iter().filter(|r| r.failed) {
        match r.triage.as_ref().map(|t| t.class) {
            Some("genuine") | None => {
                let p = prog(&r.command);
                if !genuine.contains(&p) {
                    genuine.push(p);
                }
                *by_class.entry("genuine").or_default() += 1;
            }
            Some(c) => *by_class.entry(c).or_default() += 1,
        }
    }
    if let Some(&n) = by_class.get("genuine") {
        let mut progs = genuine;
        let more = progs.len() > 2;
        progs.truncate(2);
        fs.push(format!(
            "{n} cmd{} failed ({}{})",
            if n > 1 { "s" } else { "" },
            progs.join(", "),
            if more { ", …" } else { "" }
        ));
    }
    for (class, label) in [
        ("expected-nonzero", "expected-nonzero"),
        ("guarded", "guarded"),
        ("retried-and-passed", "retried"),
        ("user-abort", "aborted by you"),
        ("sandbox-denial", "sandbox-denied"),
    ] {
        if let Some(&n) = by_class.get(class) {
            fs.push(format!("{n} {label}"));
        }
    }
    let mcp: Vec<&str> = iv
        .mcp_runs
        .iter()
        .filter(|m| m.errored)
        .map(|m| m.server.as_str())
        .collect();
    if !mcp.is_empty() {
        let mut servers: Vec<&str> = Vec::new();
        for s in &mcp {
            if !servers.contains(s) {
                servers.push(s);
            }
        }
        servers.truncate(2);
        fs.push(format!("{} mcp err ({})", mcp.len(), servers.join(", ")));
    }
    if fs.is_empty() {
        "—".into()
    } else {
        fs.join(" · ")
    }
}

/// A recap entry: what you asked, then what it became. The ask is the
/// story's subject — the commit message is the agent's summary of it, and
/// leading with that was how recap ended up saying LESS than the audit.
fn recap_entry(st: &Style, iv: &Interval, show_intent: bool) {
    let landed = iv
        .ledger
        .iter()
        .filter(|l| l.landing != crate::reconcile::Landing::Never)
        .count();
    let found = findings_cell(iv);
    let mark = status_dot_muted(st, iv.status());

    let ask = iv.intents.first().filter(|_| show_intent).map(|t| {
        let one: String = t.split_whitespace().collect::<Vec<_>>().join(" ");
        let clipped: String = one.chars().take(96).collect();
        if one.chars().count() > 96 {
            format!("{clipped}…")
        } else {
            clipped
        }
    });
    match ask {
        Some(ask) => {
            println!("  {} {} {}", mark, st.cyan(&iv.commit.short), ask);
            println!(
                "           {} {}",
                st.dim("↳"),
                st.dim(&format!(
                    "{} · {}/{} landed{}",
                    redact_home(&iv.commit.subject.chars().take(58).collect::<String>()),
                    landed,
                    iv.ledger.len(),
                    if found == "—" {
                        String::new()
                    } else {
                        format!(" · {found}")
                    }
                ))
            );
        }
        None => println!(
            "  {} {} {} {}",
            mark,
            st.cyan(&iv.commit.short),
            redact_home(&iv.commit.subject.chars().take(58).collect::<String>()),
            st.dim(&format!(
                "· {}/{} landed{}",
                landed,
                iv.ledger.len(),
                if found == "—" {
                    String::new()
                } else {
                    format!(" · {found}")
                }
            ))
        ),
    }
}

fn summary_row(st: &Style, iv: &Interval, emoji: bool, narrative: bool) {
    let landed = iv
        .ledger
        .iter()
        .filter(|l| l.landing != crate::reconcile::Landing::Never)
        .count();
    let subject: String = iv.commit.subject.chars().take(45).collect();
    println!(
        "  {} {} {:<45} {:>5}  {}",
        if narrative {
            status_dot_muted(st, iv.status())
        } else {
            status_dot(st, iv.status(), emoji)
        },
        iv.commit.short,
        redact_home(&subject),
        format!("{landed}/{}", iv.ledger.len()),
        findings_cell(iv)
    );
}

/// `--summary`: the headline and ONE table — findings of any age, then the
/// recent tail. CAP recent rows; older findings capped with an announced
/// omission (never silent).
pub fn print_summary(audit: &Audit, opts: &Options) {
    let st = Style::new(opts.color);
    let c = audit.counts();
    const CAP: usize = 15;
    if opts.narrative {
        return print_recap(audit, opts, &st, c, CAP);
    }
    println!(
        "commits {} (+{} by others held out) · {} · claims {}/{} · broken promises {}",
        c.total,
        c.keyframes_excluded,
        if opts.emoji {
            format!(
                "{} 🟢 / {} ⚪ / {} 🟡 / {} 🔴",
                c.green, c.grey, c.amber, c.red
            )
        } else {
            format!(
                "{} · {} · {} · {}",
                st.green(&format!("{} green", c.green)),
                st.dim(&format!("{} grey", c.grey)),
                st.yellow(&format!("{} amber", c.amber)),
                st.red(&format!("{} red", c.red)),
            )
        },
        c.claims_landed,
        c.claims_total,
        c.broken,
    );
    println!();
    println!(
        "{}",
        st.dim(&format!("    commit  {:<45} claims  findings", "subject"))
    );
    let eq: Vec<&Interval> = audit.equation().collect();
    let older = eq.len().saturating_sub(CAP);
    let findings: Vec<&&Interval> = eq[..older]
        .iter()
        .filter(|iv| iv.status() != Status::Green)
        .collect();
    let omitted = findings.len().saturating_sub(CAP);
    for iv in findings.iter().take(CAP) {
        summary_row(&st, iv, opts.emoji, opts.narrative);
    }
    if omitted > 0 {
        println!(
            "    {}",
            st.dim(&format!("… {omitted} more findings omitted"))
        );
    }
    if older > 0 {
        println!(
            "    {}",
            st.dim(&format!(
                "… {older} earlier commits (findings above shown; --oneline for all)"
            ))
        );
    }
    for iv in &eq[older..] {
        summary_row(&st, iv, opts.emoji, opts.narrative);
    }
}

/// `--summary --project`: the roll-up (repo · commits · claims · findings),
/// then a condensed table per repo that has commits. Zero-commit repos stay
/// as roll-up rows. Same grammar as the single-repo summary.
pub fn print_project_summary(sections: &[(String, &Audit)], opts: &Options) {
    let st = Style::new(opts.color);
    let verdict = sections
        .iter()
        .map(|(_, a)| a.verdict())
        .max()
        .unwrap_or(Status::Green);
    if opts.narrative {
        let commits: usize = sections.iter().map(|(_, a)| a.counts().total).sum();
        println!("{} repos · {commits} commits", sections.len());
    } else {
        let word = match verdict {
            Status::Green => "green",
            Status::Grey => "grey",
            Status::Amber => "amber",
            Status::Red => "red",
        };
        println!(
            "project verdict: {} {word} · {} repos",
            status_dot(&st, verdict, opts.emoji),
            sections.len()
        );
    }
    println!();
    println!(
        "{}",
        st.dim(if opts.narrative {
            "    repo                   commits    claims  what happened"
        } else {
            "    repo                   commits    claims  findings"
        })
    );
    for (name, a) in sections {
        let s = a.landing_summary();
        let mut fs: Vec<String> = Vec::new();
        if s.broken > 0 {
            fs.push(format!("{} broken", s.broken));
        }
        if s.residue_files > 0 {
            fs.push(format!("{} residue", s.residue_files));
        }
        println!(
            "  {} {:<22} {:>5}  {:>8}  {}",
            if opts.narrative {
                status_dot_muted(&st, s.verdict)
            } else {
                status_dot(&st, s.verdict, opts.emoji)
            },
            name,
            s.commits,
            format!("{}/{}", s.landed, s.claims),
            if fs.is_empty() {
                "—".into()
            } else {
                fs.join(" · ")
            }
        );
    }
    for (name, a) in sections {
        if a.counts().total == 0 {
            continue;
        }
        println!();
        println!("{}", st.bold(&format!("=== {name} ===")));
        print_summary(a, opts);
    }
}

/// The recap view — the default command's output.
///
/// Same receipt as the audit, different first sentence. The audit leads
/// with what LANDED (balance, broken promises, colored dots); the recap
/// leads with what was ASKED and what happened next, mutes the marks, and
/// closes by naming what to run to go deeper. Findings are never hidden —
/// comprehension without the trust layer is a prettier memoir — they just
/// don't shout.
fn print_recap(
    audit: &Audit,
    _opts: &Options,
    st: &Style,
    c: crate::reconcile::Counts,
    cap: usize,
) {
    let commits = if c.total == 1 { "commit" } else { "commits" };
    println!(
        "{} {commits} · {} prompts · {}/{} claimed files landed",
        c.total, audit.prompts, c.claims_landed, c.claims_total,
    );
    let eq: Vec<&Interval> = audit.equation().collect();
    // Name them. "5 of them are worth a look" without saying WHICH five
    // makes the reader hunt for something the tool already knows.
    let flagged: Vec<&&Interval> = eq
        .iter()
        .filter(|iv| matches!(iv.status(), Status::Amber | Status::Red))
        .collect();
    if !flagged.is_empty() {
        let names: Vec<String> = flagged
            .iter()
            .take(6)
            .map(|iv| iv.commit.short.clone())
            .collect();
        println!(
            "{}",
            st.dim(&format!(
                "worth a look: {}{} — `git receipts audit` for the verdict",
                names.join(", "),
                if flagged.len() > 6 {
                    format!(" and {} more", flagged.len() - 6)
                } else {
                    String::new()
                }
            ))
        );
    }
    println!();
    let older = eq.len().saturating_sub(cap);
    let earlier_findings: Vec<&&Interval> = eq[..older]
        .iter()
        .filter(|iv| iv.status() != Status::Green)
        .collect();
    let omitted = earlier_findings.len().saturating_sub(cap);
    for iv in earlier_findings.iter().take(cap) {
        recap_entry(st, iv, true);
    }
    if omitted > 0 {
        println!("    {}", st.dim(&format!("… {omitted} more omitted")));
    }
    if older > 0 {
        println!(
            "    {}",
            st.dim(&format!(
                "… {older} earlier commits (anything notable is above)"
            ))
        );
    }
    for iv in &eq[older..] {
        recap_entry(st, iv, true);
    }

    // What to run next — the invitation that turns a table into a thread
    // you can pull. Always the drill-down; the audit only when it has
    // something to say.
    println!();
    let example = eq
        .last()
        .map(|iv| iv.commit.short.as_str())
        .unwrap_or("<hash>");
    println!(
        "{}",
        st.dim(&format!(
            "one commit's story:  git receipts recap --commit {example}\n\
             keep or share it:    git receipts recap --format html > recap.html\n\
             the data:            git receipts export > receipt.json"
        ))
    );
}
