//! Self-contained HTML report, rendered directly from the [`Audit`].
//!
//! One file, no external assets — inline CSS + a few lines of JS for the
//! filter. Theme-aware (light/dark). The structure mirrors the console
//! report in `report.rs`: header metrics, the intent→outcome summary, then
//! the interval spine with every finding class.
//!
//! The trailing newlines in the `write!` templates are deliberate document
//! whitespace, so the write-with-newline lint is silenced module-wide.
#![allow(clippy::write_with_newline)]

use std::fmt::Write as _;

use crate::extract::Session;
use crate::fmt::{abbrev, command_summary, redact_home, tilde};
use crate::ingest::IngestStats;
use crate::reconcile::{Audit, Status};
use crate::report::{Expand, Show};

/// Redact, then HTML-escape. This is the single sanitizer every dynamic string
/// in the HTML report flows through, so routing redaction through it makes the
/// rule "everything the report shows is scanned/redacted" STRUCTURAL: home/
/// username masking + the secret/PII scanner run on every rendered value, with
/// no per-site discipline to forget. Redaction is idempotent, so callers that
/// still redact first are harmless (and don't double-count — the second pass
/// finds nothing).
fn esc(s: &str) -> String {
    let s = redact_home(s);
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

const STYLE: &str = include_str!("html/report.css");
const SCRIPT: &str = include_str!("html/report.js");

/// One repo's report body: the sub-header through the balance line — everything
/// except the document shell (doctype/head/`<h1>`) and footer, which the page
/// owns. `show_notice` prints the privacy notice (once per page, so a project
/// page passes false on sections); `filter_controls` prints the color filter
/// (a project page has ONE global filter instead of one per section).
#[allow(clippy::too_many_arguments)]
fn render_body(
    session_name: &str,
    repo: &str,
    session: &Session,
    stats: &IngestStats,
    audit: &Audit,
    show: Show,
    show_identity: bool,
    expand: Expand,
    with_output: bool,
    commit: Option<&str>,
    full: bool,
    show_notice: bool,
    filter_controls: bool,
) -> String {
    // Headline counts over the EQUATION set (agent's own commits by default,
    // everything under --full-history) — same source as console/JSON, so a
    // teammate's keyframe never shapes the numbers.
    let c = audit.counts();
    let total = c.total;
    let green = c.green;
    let red = c.red;
    let amber = c.amber;
    let claims_total = c.claims_total;
    let claims_landed = c.claims_landed;
    let broken = c.broken;
    let keyframes_excluded = c.keyframes_excluded;
    // Your unexplained residue — unique files in your own commits (deduped).
    let residue_files = audit.residue_files();
    let full_history = audit.full_history;
    let in_eq = |iv: &crate::reconcile::Interval| full_history || iv.agent_committed;
    let pct = |n: usize, d: usize| {
        if d == 0 {
            100.0
        } else {
            100.0 * n as f64 / d as f64
        }
    };
    let enriched = audit.intervals.iter().any(|i| !i.commit.from_history);

    let mut b = String::with_capacity(64 * 1024);

    // ---- section sub-header (session + repo + window) ------------------
    let _ = write!(
        b,
        "<div class=\"sub\">{name}<br>repo {repo}",
        name = esc(session_name),
        repo = esc(&tilde(repo)),
    );
    if let (Some(a), Some(z)) = (session.first_ts, session.last_ts) {
        let dur = z - a;
        let _ = write!(
            b,
            " · {} → {} ({}d {}h)",
            a.format("%Y-%m-%d %H:%MZ"),
            z.format("%Y-%m-%d %H:%MZ"),
            dur.num_days(),
            dur.num_hours() % 24
        );
    }
    b.push_str("</div>\n");

    // ---- privacy notice (once per page) --------------------------------
    // These reports bundle chat/agent logs, git contents, and command output
    // — private by default. Warn before anyone shares one. Unconditional:
    // --no-intent/--no-identity/--redact reduce exposure but don't remove it.
    if show_notice {
        b.push_str(NOTICE_HTML);
    }

    // ---- meta line -----------------------------------------------------
    let dup = if stats.duplicates > 0 {
        format!(", {} fork-duplicates removed", stats.duplicates)
    } else {
        String::new()
    };
    let _ = write!(
        b,
        "<div class=\"meta\">\
         <div>{} events kept / {} lines ({} bookkeeping, {} unparseable{})</div>\
         <div>{} file mutations · {} commands · {} MCP calls · {} observations</div>\
         <div>OS/FS: {} commands · {} failed · {} aborted by you \u{00b7} \
         MCP: {} calls · {} errored · {} aborted by you \
         <span class=\"dim\">(amber signals — a look, never red)</span></div>\
         <div>blast radius: {} local-fs · {} local-git · {} remote-git · {} network · {} read-only</div>\
         </div>\n",
        stats.kept - stats.duplicates,
        stats.lines,
        stats.skipped_types,
        stats.unparseable,
        dup,
        audit.file_claims,
        audit.commands,
        audit.mcp_calls,
        audit.observations,
        audit.commands,
        audit.cmd_failed,
        audit.cmd_aborted,
        audit.mcp_calls,
        audit.mcp_errored,
        audit.mcp_aborted,
        audit.radii.local_fs,
        audit.radii.local_git,
        audit.radii.remote_git,
        audit.radii.network,
        audit.radii.read_only,
    );

    // ---- stat tiles ----------------------------------------------------
    let _ = write!(
        b,
        "<div class=\"stats\">\
         <div class=\"stat good\"><b>{:.0}%</b><span>intervals green · {green}/{total}</span></div>\
         <div class=\"stat good\"><b>{:.0}%</b><span>claims landed · {claims_landed}/{claims_total}</span></div>\
         <div class=\"stat {}\"><b>{broken}</b><span>broken promises</span></div>\
         <div class=\"stat {}\"><b>{}</b><span>commands with errors</span></div>\
         <div class=\"stat {}\"><b>{}</b><span>MCP with errors</span></div>\
         <div class=\"stat {}\"><b>{residue_files}</b><span>residue</span></div>\
         </div>\n",
        pct(green, total),
        pct(claims_landed, claims_total),
        if broken == 0 { "good" } else { "bad" },
        if audit.cmd_failed == 0 {
            "good"
        } else {
            "warn"
        },
        audit.cmd_failed,
        if audit.mcp_errored == 0 {
            "good"
        } else {
            "warn"
        },
        audit.mcp_errored,
        if residue_files == 0 { "good" } else { "warn" },
    );

    // ---- intent → outcome + exceptions --------------------------------
    render_summary(
        &mut b,
        session,
        audit,
        green,
        total,
        claims_landed,
        claims_total,
        broken,
        show_identity,
    );

    // ---- controls ------------------------------------------------------
    let spine_h = if full_history {
        format!("interval spine — {total} commits")
    } else if keyframes_excluded > 0 {
        format!(
            "interval spine — {total} agent commits <span class=\"dim\">({keyframes_excluded} by others held out; --full-history to include)</span>"
        )
    } else {
        format!("interval spine — {total} agent commits")
    };
    let _ = write!(b, "<div class=\"controls\"><h2>{spine_h}</h2>");
    if filter_controls {
        b.push_str(FILTER_HTML);
    }
    b.push_str("</div>\n<section class=\"ledger\">\n");

    // A per-commit model line only earns its place when the session spanned
    // more than one model; otherwise the header roll-up already covers it.
    let multi_model = session.models_used(None, None).len() > 1;
    for (i, iv) in audit.intervals.iter().enumerate() {
        if let Some(h) = commit
            && iv.commit.hash != h
        {
            continue;
        }
        // Default: hold out non-agent keyframes — they aren't the agent's account.
        if !in_eq(iv) {
            continue;
        }
        let after = (i > 0).then(|| audit.intervals[i - 1].commit.ts);
        // --full: the interval's conversation, in its span (after, commit ts].
        // Suppression applies per-turn: --no-prompt drops your turns,
        // --no-summary the agent's.
        let convo = if full && (show.prompt || show.summary) {
            session
                .conversation(after, Some(iv.commit.ts))
                .into_iter()
                .filter(|t| if t.user { show.prompt } else { show.summary })
                .collect()
        } else {
            Vec::new()
        };
        let prov = commit_provenance(session, after, iv.commit.ts, multi_model);
        let cost = session.cost_in(after, Some(iv.commit.ts));
        render_interval(
            &mut b,
            iv,
            show,
            show_identity,
            enriched,
            expand,
            with_output,
            &convo,
            prov,
            cost,
        );
    }
    b.push_str("</section>\n");

    // ---- balance + footer ---------------------------------------------
    let residue_total: usize = audit.intervals.iter().map(|i| i.residue.len()).sum();
    let _ = write!(
        b,
        "<div class=\"balance\">balance: {green} green · {amber} amber · {red} red \
         of {total} intervals ({:.0}% green) · claims landed {claims_landed}/{claims_total} · \
         residue {residue_total}</div>\n",
        pct(green, total),
    );

    b
}

/// The privacy notice — emitted once per page (a project page carries it in the
/// masthead, not per section).
const NOTICE_HTML: &str = "<div class=\"notice\"><b>\u{26a0} private audit report</b> \u{2014} \
     built from your chat &amp; agent logs, code and history from git, and the \
     output of the commands that ran. It is meant for developers to review and \
     audit their own work, and can contain sensitive detail. Handle it with \
     extreme caution and treat it like any private record before sharing.</div>\n";

/// The color filter — one per page. A single-repo report puts it in its controls
/// row; a project page puts one global filter above all the sections.
const FILTER_HTML: &str = "<fieldset class=\"filter\">show \
     <label><input type=\"checkbox\" id=\"f-all\" checked> all</label>\
     <label class=\"c-green\"><input type=\"checkbox\" class=\"fc\" value=\"green\" checked> green</label>\
     <label class=\"c-amber\"><input type=\"checkbox\" class=\"fc\" value=\"amber\" checked> amber</label>\
     <label class=\"c-red\"><input type=\"checkbox\" class=\"fc\" value=\"red\" checked> red</label>\
     </fieldset>";

/// Open the document: doctype, head (title + inlined CSS), and the page `<h1>`.
fn doc_head(b: &mut String, title: &str) {
    let _ = write!(
        b,
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>git receipts — {title}</title>\n<style>{STYLE}</style>\n</head><body>\n\
         <div class=\"wrap\">\n\
         <h1><span class=\"cmd\">git receipts</span> — what your agent actually did</h1>\n",
    );
}

/// Close the document: the secret-scanner tally (complete once every body has
/// rendered), the footer, the inlined script, and the closing tags.
fn doc_tail(b: &mut String) {
    let (secrets, pii) = crate::fmt::scan_counts();
    if secrets + pii > 0 {
        let _ = write!(
            b,
            "<div class=\"scan-note\">🔒 redacted {secrets} secret{} and {pii} PII value{} the \
             scanner recognized — masked in place; values never shown.</div>\n",
            if secrets == 1 { "" } else { "s" },
            if pii == 1 { "" } else { "s" },
        );
    }
    b.push_str(
        "<footer>Generated by <code>git receipts</code>. Every line is derived from a receipt — \
         the log's own tool calls, or git's own diffs. Broken promises are content-verified, not assumed.</footer>\n",
    );
    let _ = write!(b, "<script>{SCRIPT}</script>\n</div>\n</body></html>\n");
}

/// A single-repo HTML report: the document shell wrapped around one report body.
#[allow(clippy::too_many_arguments)]
pub fn render(
    session_name: &str,
    repo: &str,
    session: &Session,
    stats: &IngestStats,
    audit: &Audit,
    show: Show,
    show_identity: bool,
    expand: Expand,
    with_output: bool,
    commit: Option<&str>,
    full: bool,
) -> String {
    let mut b = String::with_capacity(64 * 1024);
    doc_head(&mut b, &esc(session_name));
    b.push_str(&render_body(
        session_name,
        repo,
        session,
        stats,
        audit,
        show,
        show_identity,
        expand,
        with_output,
        commit,
        full,
        true,
        true,
    ));
    doc_tail(&mut b);
    b
}

/// One repo's slice of a `--project` HTML page: the section header context plus
/// the reconciled audit its body renders from. Uses only library types (the
/// binary's `Loaded` can't cross into this crate).
pub struct HtmlSection<'a> {
    /// The repo's directory name — the section heading and landing-table row.
    pub name: &'a str,
    pub session_name: &'a str,
    pub repo: &'a str,
    pub session: &'a Session,
    pub stats: &'a IngestStats,
    pub audit: &'a Audit,
}

/// A `--project` HTML page: one masthead (project path + notice + where-it-landed
/// roll-up + a single global color filter), then one section per repo — the HTML
/// twin of `audit --project`. Self-contained, like the single-repo report.
pub fn render_project(
    project_display: &str,
    sections: &[HtmlSection],
    show: Show,
    show_identity: bool,
    expand: Expand,
    with_output: bool,
    full: bool,
) -> String {
    let mut b = String::with_capacity(128 * 1024);
    // tilde the path first: it must never ship the home dir in the <title>.
    doc_head(&mut b, &esc(&tilde(project_display)));
    let _ = write!(
        b,
        "<div class=\"sub\">project {}<br>{} git repos with sessions</div>\n",
        esc(&tilde(project_display)),
        sections.len(),
    );
    b.push_str(NOTICE_HTML);

    // ---- where-it-landed roll-up --------------------------------------
    b.push_str(
        "<table class=\"landing\"><caption>where it landed</caption><thead><tr>\
         <th>repo</th><th>commits</th><th>landed</th><th>broken</th><th>residue</th><th>verdict</th>\
         </tr></thead><tbody>\n",
    );
    for s in sections {
        let l = s.audit.landing_summary();
        let (cls, word) = match l.verdict {
            Status::Green => ("good", "green"),
            Status::Amber => ("warn", "amber"),
            Status::Red => ("bad", "red"),
        };
        let _ = write!(
            b,
            "<tr><td><a href=\"#repo-{anchor}\">{name}</a></td><td>{commits}</td>\
             <td>{landed}/{claims}</td><td class=\"{bk}\">{broken}</td>\
             <td class=\"{rs}\">{residue}</td><td class=\"verdict {cls}\">\u{25cf} {word}</td></tr>\n",
            anchor = esc(s.name),
            name = esc(s.name),
            commits = l.commits,
            landed = l.landed,
            claims = l.claims,
            bk = if l.broken == 0 { "dim" } else { "bad" },
            broken = l.broken,
            rs = if l.residue_files == 0 { "dim" } else { "warn" },
            residue = l.residue_files,
        );
    }
    b.push_str("</tbody></table>\n");

    // ---- one global color filter for the whole page -------------------
    let _ = write!(b, "<div class=\"controls\">{FILTER_HTML}</div>\n");

    // ---- one section per repo -----------------------------------------
    for s in sections {
        let _ = write!(
            b,
            "<section class=\"repo-section\" id=\"repo-{anchor}\"><h2 class=\"repo-h\">{name}</h2>\n",
            anchor = esc(s.name),
            name = esc(s.name),
        );
        b.push_str(&render_body(
            s.session_name,
            s.repo,
            s.session,
            s.stats,
            s.audit,
            show,
            show_identity,
            expand,
            with_output,
            None,
            full,
            false,
            false,
        ));
        b.push_str("</section>\n");
    }

    doc_tail(&mut b);
    b
}

#[allow(clippy::too_many_arguments)]
fn render_summary(
    b: &mut String,
    session: &Session,
    audit: &Audit,
    green: usize,
    total: usize,
    claims_landed: usize,
    claims_total: usize,
    broken: usize,
    show_identity: bool,
) {
    // Exception + attribution aggregates — the SAME computation the console and
    // JSON receipt use (`Audit::exceptions`), so the three surfaces never drift.
    // (This also fixes a latent bug: the old inline `deliberate` needle here was
    // "removes or moves", not the console's "deliberately".)
    let ex = audit.exceptions();
    let late = ex.landed_late;
    let superseded = ex.resolved_superseded;
    let persisted = ex.resolved_persisted;
    let deliberate = ex.resolved_deliberate;
    let relocated = ex.resolved_relocated;
    let scratch = ex.resolved_scratch;
    let attributed = ex.unclaimed_by_command;
    let dismissed = ex.dismissed;
    let residue = ex.residue;
    let not_session = ex.unclaimed_other_contributor;
    let unexplained = ex.unclaimed_unexplained;

    b.push_str("<div class=\"outcome\"><h3>intent → outcome</h3>");
    let _ = write!(
        b,
        "<div class=\"line\">{} prompts drove {total} commits; {claims_landed}/{claims_total} claimed files landed, {green} intervals fully balanced</div>",
        audit.prompts,
    );
    let tk = &session.tokens;
    if tk.requests > 0 {
        let _ = write!(
            b,
            "<div class=\"line\">agent effort: {} commands · ~{} output tokens across {} requests \
             <span class=\"dim\">(est. from the log — approximate, not billing; ~{} input, ~{} cache read, ~{} cache write)</span></div>",
            audit.commands,
            abbrev(tk.output),
            tk.requests,
            abbrev(tk.input),
            abbrev(tk.cache_read),
            abbrev(tk.cache_creation),
        );
    }
    // Provenance: model(s) authoritative, effort labelled with its coverage.
    let models = session.models_used(None, None);
    if !models.is_empty() {
        let list = models
            .iter()
            .map(|(m, c)| format!("{} ({} req)", esc(short_model(m)), c))
            .collect::<Vec<_>>()
            .join(", ");
        let (efforts, coverage) = session.effort_seen(None, None);
        let etail = if efforts.is_empty() {
            String::new()
        } else {
            format!(
                " <span class=\"dim\">· reasoning effort: {} ({} coverage in the log)</span>",
                esc(&efforts.join(", ")),
                coverage.as_str()
            )
        };
        let _ = write!(b, "<div class=\"line\">model(s): {list}{etail}</div>");
    }
    b.push_str("<div class=\"heading\">what happened to every exception</div>");
    if late > 0 {
        let _ = write!(
            b,
            "<div class=\"line dim\">· claims that landed late: {late}, content-verified against the commit they landed in</div>"
        );
    }
    if superseded + persisted + deliberate + relocated + scratch > 0 {
        let _ = write!(
            b,
            "<div class=\"line dim\">· never landed, resolved: {} — superseded by later landed edits: {superseded} · removed deliberately: {deliberate} · persisted on disk outside git: {persisted} · relocated before first commit, landed at a different path (content-verified): {relocated} · written and discarded before any commit (scratch churn; ambers): {scratch}</div>",
            superseded + persisted + deliberate + relocated + scratch
        );
    }
    let _ = write!(
        b,
        "<div class=\"line dim\">· unclaimed changes (git recorded it, no matching edit claim): {} — this agent via a command: {attributed} · not this session's commit (another contributor): {not_session} · unexplained, inside an agent commit: {unexplained} · dismissed as now ignored/untracked: {dismissed}</div>",
        residue + attributed + dismissed
    );
    let cls = if broken == 0 { "ok" } else { "bad" };
    let _ = write!(
        b,
        "<div class=\"line bottomline {cls}\">· broken promises (claimed, never landed, nothing explains it): {broken}</div>"
    );

    // who touched this repo — attribution for free, straight from git
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
    b.push_str("<div class=\"heading\">who touched this repo <span class=\"dim\">(git identity — not how they authored)</span></div>");
    if show_identity {
        let _ = write!(
            b,
            "<div class=\"line dim\">· committed by: {}</div>",
            esc(&authors.join(" \u{00b7} "))
        );
        if !coauthors.is_empty() {
            let _ = write!(
                b,
                "<div class=\"line dim\">· co-authored-by (declared): {}</div>",
                esc(&coauthors.join(" \u{00b7} "))
            );
        }
    } else {
        let plural = |n: usize| if n == 1 { "identity" } else { "identities" };
        let _ = write!(
            b,
            "<div class=\"line dim\">· {} committer {} · {} co-author {} (names hidden — --no-identity)</div>",
            authors.len(),
            plural(authors.len()),
            coauthors.len(),
            plural(coauthors.len()),
        );
    }
    if keyframes > 0 {
        let _ = write!(
            b,
            "<div class=\"line dim\">· {keyframes} commit(s) not made by this session \u{2014} another contributor</div>"
        );
    }
    b.push_str("</div>\n");
}

/// `claude-opus-4-8` → `opus-4-8` for display; full id stays in the JSON.
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

/// Per-commit provenance text (plain — the caller escapes it). `None` when a
/// single-model session left no effort here: the header roll-up already says
/// it. See the console `commit_provenance` for the rationale.
fn commit_provenance(
    session: &Session,
    after: Option<chrono::DateTime<chrono::Utc>>,
    until: chrono::DateTime<chrono::Utc>,
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
    Some(format!("model: {m}{etail}"))
}

#[allow(clippy::too_many_arguments)]
fn render_interval(
    b: &mut String,
    iv: &crate::reconcile::Interval,
    show: Show,
    show_identity: bool,
    enriched: bool,
    expand: Expand,
    with_output: bool,
    convo: &[crate::extract::Turn],
    prov: Option<String>,
    cost: (usize, u64),
) {
    let (cls, mark) = match iv.status() {
        Status::Green => ("green", "\u{2714}"),
        Status::Amber => ("amber", "!"),
        Status::Red => ("red", "\u{2718}"),
    };
    let mut subject = iv.commit.subject.clone();
    if subject.chars().count() > 72 {
        subject = subject.chars().take(72).collect::<String>() + "\u{2026}";
    }

    let never: Vec<_> = iv.never_landed().collect();
    let resolved: Vec<_> = iv.resolved_never().collect();
    let late: Vec<_> = iv.landed_late().collect();
    let landed = iv.ledger.len() - never.len() - resolved.len() - late.len();

    let open = match expand {
        Expand::All => true,
        Expand::None => false,
        Expand::Auto => iv.status() != Status::Green,
    };
    let _ = write!(
        b,
        "<details class=\"interval {cls}\"{}><summary>\
        <span class=\"mark\">{mark}</span><code class=\"hash\">{}</code>\
        <span class=\"date\">{}</span><span class=\"subject\">{}</span>",
        if open { " open" } else { "" },
        esc(&iv.commit.short),
        iv.commit.ts.format("%m-%d %H:%M"),
        esc(&subject)
    );
    if !iv.agent_committed {
        b.push_str("<span class=\"tag\">keyframe</span>");
    }
    if enriched && iv.commit.from_history {
        b.push_str("<span class=\"tag\">created elsewhere</span>");
    }
    if !iv.commit.reachable {
        b.push_str("<span class=\"tag\">reflog only</span>");
    }
    if iv.commit.clock_anomaly {
        b.push_str("<span class=\"tag\">clock anomaly</span>");
    }
    let _ = write!(
        b,
        "<span class=\"scount\">{} claimed / {landed} landed / {} residue</span></summary>\n<div class=\"drill\">",
        iv.ledger.len(),
        iv.residue.len(),
    );

    // ---- status badges -------------------------------------------------
    b.push_str("<div class=\"badges\">");
    if iv.agent_committed {
        b.push_str("<span class=\"badge ok\">committed by agent</span>");
    } else if show_identity {
        let _ = write!(
            b,
            "<span class=\"badge\">keyframe \u{2014} committed by {}</span>",
            esc(&iv.commit.author)
        );
    } else {
        b.push_str("<span class=\"badge\">keyframe \u{2014} another contributor</span>");
    }
    if show_identity {
        for co in &iv.commit.co_authors {
            let _ = write!(b, "<span class=\"badge\">co-authored-by {}</span>", esc(co));
        }
    }
    if iv.pushed {
        b.push_str("<span class=\"badge ok\">pushed (as of last fetch)</span>");
    } else {
        b.push_str("<span class=\"badge\">local only \u{2014} not pushed</span>");
    }
    if let Some(p) = &prov {
        let _ = write!(b, "<span class=\"badge\">\u{2699} {}</span>", esc(p));
    }
    let parent = if iv.commit.parent.is_empty() {
        "(root)"
    } else {
        &iv.commit.parent[..iv.commit.parent.len().min(9)]
    };
    let _ = write!(
        b,
        "<span class=\"badge\">parent {}</span></div>",
        esc(parent)
    );

    // The work behind this commit — commands, MCP calls, and the agent token
    // cost of its conversation window. Each part shows only when non-trivial.
    let (creq, cout) = cost;
    let mcp = iv.mcp_runs.len();
    let mut work: Vec<String> = Vec::new();
    if iv.commands > 0 {
        work.push(format!(
            "{} {}",
            iv.commands,
            plural(iv.commands, "command")
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
        let _ = write!(
            b,
            "<div class=\"line work dim\">{}</div>",
            esc(&work.join(" · "))
        );
    }

    if show.prompt
        && let Some(first) = iv.intents.first()
    {
        let more = if iv.intents.len() > 1 {
            format!(" (+{} more)", iv.intents.len() - 1)
        } else {
            String::new()
        };
        let _ = write!(
            b,
            "<div class=\"line intent\">\u{00bb} intent: {}{}</div>",
            esc(first),
            esc(&more)
        );
    }
    if iv.spine_jump {
        b.push_str(
            "<div class=\"line dim\">\u{2442} spine jump: parent is not the previous commit</div>",
        );
    }
    for s in &iv.superseded {
        let _ = write!(
            b,
            "<div class=\"line dim\">\u{21bb} amend: draft {} committed {} files, amended {}s later</div>",
            esc(&s.short),
            s.files,
            s.seconds_before_amend
        );
    }

    // The bulky detail sections (files, commands, MCP) collapse for a clean
    // green commit — nothing to inspect — but stay open for a red or residue
    // commit, so the problem is visible without a click. `--expand all` forces
    // everything open, honoring its "expand everything" contract.
    let open_sections = iv.status() != Status::Green || expand == Expand::All;
    render_statement(b, iv, open_sections);
    render_commands(b, iv, with_output, open_sections);
    render_mcp(b, iv, with_output, open_sections);

    // ---- reconciliation findings ---------------------------------------
    for l in &late {
        let (at, dist) = l
            .landed_at
            .as_ref()
            .map(|(s, d)| (s.as_str(), *d))
            .unwrap_or(("?", 1));
        let _ = write!(
            b,
            "<div class=\"line ok\">\u{2713} landed late: {} <span class=\"dim\">(content verified in {}, {} commit(s) later)</span></div>",
            esc(&l.path),
            esc(at),
            dist
        );
    }
    for l in &resolved {
        let _ = write!(
            b,
            "<div class=\"line warn\">\u{25cc} never landed, resolved: {}</div>",
            esc(&l.path)
        );
        if let Some(r) = &l.resolution {
            let _ = write!(b, "<div class=\"line dim indent\">{}</div>", esc(r));
        }
    }
    for l in &never {
        let _ = write!(
            b,
            "<div class=\"line bad\">\u{2718} never landed: {} <span class=\"dim\">({} edit(s), f#{})</span></div>",
            esc(&l.path),
            l.edits,
            l.frames.first().copied().unwrap_or(0)
        );
        if let Some(d) = l.diagnosis {
            let _ = write!(b, "<div class=\"line dim indent\">{}</div>", esc(d));
        }
    }
    for res in &iv.residue {
        let _ = write!(
            b,
            "<div class=\"line warn\">\u{25cf} residue: {} <span class=\"dim\">[{}] changed, never claimed</span></div>",
            esc(&res.path),
            res.status
        );
    }
    for (res, why) in &iv.attributed_residue {
        let moved = res
            .old_path
            .as_ref()
            .map(|o| format!(" (was {})", esc(o)))
            .unwrap_or_default();
        let _ = write!(
            b,
            "<div class=\"line dim\">\u{25cf} attributed: {}{} <span class=\"dim\">[{}] \u{2014} {}</span></div>",
            esc(&res.path),
            moved,
            res.status,
            esc(why)
        );
    }
    for (res, why) in &iv.dismissed_residue {
        let _ = write!(
            b,
            "<div class=\"line dim\">\u{25cb} dismissed: {} <span class=\"dim\">[{}] \u{2014} {}</span></div>",
            esc(&res.path),
            res.status,
            esc(why)
        );
    }
    // The agent's own account of this commit — a claim, not proof; the
    // findings above are what's verified.
    if show.summary
        && let Some(summary) = &iv.summary
    {
        let _ = write!(
            b,
            "<div class=\"section\"><div class=\"section-h\">agent summary — the agent's own words, not verified</div>\
             <div class=\"agent-summary\">{}</div></div>",
            esc(summary)
        );
    }
    // --full: the whole conversation that produced this commit.
    if !convo.is_empty() {
        let _ = write!(
            b,
            "<div class=\"section\"><div class=\"section-h\">conversation ({} turns)</div>",
            convo.len()
        );
        for t in convo {
            let who = if t.user { "you" } else { "agent" };
            let _ = write!(
                b,
                "<div class=\"turn {who}\"><span class=\"who\">{who}</span><div class=\"msg\">{}</div></div>",
                esc(t.text)
            );
        }
        b.push_str("</div>");
    }
    b.push_str("</div></details>\n");
}

/// The commit's own diff, grouped by status — what git actually recorded.
fn render_statement(b: &mut String, iv: &crate::reconcile::Interval, open: bool) {
    if iv.statement.is_empty() {
        return;
    }
    let count = |st: char| iv.statement.iter().filter(|c| c.status == st).count();
    let (a, m, d, r) = (count('A'), count('M'), count('D'), count('R') + count('C'));
    let _ = write!(
        b,
        "<details class=\"section\"{}><summary class=\"section-h\">files git recorded ({}): \
        <b class=\"add\">{a} added</b> \u{00b7} <b class=\"mod\">{m} modified</b> \u{00b7} \
        <b class=\"del\">{d} deleted</b> \u{00b7} <b class=\"ren\">{r} renamed</b></summary>",
        if open { " open" } else { "" },
        iv.statement.len()
    );
    for c in &iv.statement {
        let (klass, label) = match c.status {
            'A' => ("add", "A"),
            'M' => ("mod", "M"),
            'D' => ("del", "D"),
            'R' => ("ren", "R"),
            'C' => ("ren", "C"),
            _ => ("mod", "?"),
        };
        let moved = c
            .old_path
            .as_ref()
            .map(|o| format!(" <span class=\"dim\">\u{2190} {}</span>", esc(o)))
            .unwrap_or_default();
        let _ = write!(
            b,
            "<div class=\"frow\"><span class=\"st {klass}\">{label}</span>{}{moved}</div>",
            esc(&c.path)
        );
    }
    b.push_str("</details>");
}

/// The effectful commands the agent ran in this interval. With `with_output`,
/// each command is shown in full with its captured output — the same depth
/// the JSON receipt carries under `--with-output`.
fn render_commands(b: &mut String, iv: &crate::reconcile::Interval, with_output: bool, open: bool) {
    if iv.commands_run.is_empty() {
        return;
    }
    let _ = write!(
        b,
        "<details class=\"section\"{}><summary class=\"section-h\">commands ({} effectful of {} total)</summary>",
        if open { " open" } else { "" },
        iv.commands_run.len(),
        iv.commands
    );
    for c in &iv.commands_run {
        // A failed command keeps full ink (see .crow.fail); a succeeded one dulls.
        b.push_str(if c.failed {
            "<div class=\"crow fail\">"
        } else {
            "<div class=\"crow\">"
        });
        match c.radius {
            Some(r) => {
                let _ = write!(b, "<span class=\"radtag {}\">{}</span>", r, r);
            }
            None => b.push_str("<span class=\"radtag ro\">read-only</span>"),
        }
        if c.committed {
            b.push_str("<span class=\"radtag commit\">commit</span>");
        }
        if c.pushed {
            b.push_str("<span class=\"radtag push\">push</span>");
        }
        if c.failed {
            b.push_str("<span class=\"radtag fail\">failed</span>");
        }
        // Expand a command to full text + output when asked (--with-output)
        // or when it failed — the failure's output is always worth showing.
        if with_output || c.failed {
            // Full command, then its captured output in a scrollable block.
            let _ = write!(b, "<code class=\"cmdfull\">{}</code>", esc(&c.command));
            if let Some(receipt) = &c.output {
                let cls = if receipt.is_error {
                    "cmdout err"
                } else {
                    "cmdout"
                };
                let _ = write!(b, "<pre class=\"{cls}\">{}</pre>", esc(&receipt.text));
            }
            b.push_str("</div>");
        } else {
            let _ = write!(
                b,
                "<code>{}</code></div>",
                esc(&command_summary(&c.command))
            );
        }
    }
    b.push_str("</details>");
}

/// The MCP tool calls in this interval (S3, execution axis). The server's
/// tool_result is the oracle: errored is tagged; output shows on error or
/// under `--with-output`.
fn render_mcp(b: &mut String, iv: &crate::reconcile::Interval, with_output: bool, open: bool) {
    if iv.mcp_runs.is_empty() {
        return;
    }
    let errored = iv.mcp_runs.iter().filter(|m| m.errored).count();
    let _ = write!(
        b,
        "<details class=\"section\"{}><summary class=\"section-h\">MCP calls ({} total, {errored} errored)</summary>",
        if open { " open" } else { "" },
        iv.mcp_runs.len(),
    );
    for m in &iv.mcp_runs {
        // An errored MCP call keeps full ink; a clean one dulls like commands.
        b.push_str(if m.errored {
            "<div class=\"crow fail\">"
        } else {
            "<div class=\"crow\">"
        });
        let _ = write!(b, "<span class=\"radtag mcp\">{}</span>", esc(&m.server));
        if m.errored {
            b.push_str("<span class=\"radtag fail\">errored</span>");
        }
        let _ = write!(b, "<code class=\"cmdfull\">{}</code>", esc(&m.tool));
        if with_output || m.errored {
            let _ = write!(b, "<pre class=\"cmdout\">{}</pre>", esc(&m.input));
            if let Some(receipt) = &m.output {
                let cls = if receipt.is_error {
                    "cmdout err"
                } else {
                    "cmdout"
                };
                let _ = write!(b, "<pre class=\"{cls}\">{}</pre>", esc(&receipt.text));
            }
        }
        b.push_str("</div>");
    }
    b.push_str("</details>");
}
