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
use crate::ingest::IngestStats;
use crate::reconcile::{Audit, Landing, Status};

/// HTML-escape a string for text/attribute content.
fn esc(s: &str) -> String {
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

fn tilde(path: &str) -> String {
    std::env::home_dir()
        .map(|h| h.display().to_string())
        .and_then(|h| path.strip_prefix(&h).map(|rest| format!("~{rest}")))
        .unwrap_or_else(|| path.to_string())
}

const STYLE: &str = include_str!("html/report.css");
const SCRIPT: &str = include_str!("html/report.js");

pub fn render(
    session_name: &str,
    repo: &str,
    session: &Session,
    stats: &IngestStats,
    audit: &Audit,
    show_intent: bool,
) -> String {
    let total = audit.intervals.len();
    let green = audit.intervals.iter().filter(|i| i.balanced()).count();
    let red = audit
        .intervals
        .iter()
        .filter(|i| i.status() == Status::Red)
        .count();
    let residue_only = audit
        .intervals
        .iter()
        .filter(|i| i.status() == Status::ResidueOnly)
        .count();
    let claims_total: usize = audit.intervals.iter().map(|i| i.ledger.len()).sum();
    let claims_landed: usize = audit
        .intervals
        .iter()
        .flat_map(|i| i.ledger.iter())
        .filter(|l| l.landing != Landing::Never)
        .count();
    let pct = |n: usize, d: usize| {
        if d == 0 {
            100.0
        } else {
            100.0 * n as f64 / d as f64
        }
    };
    let enriched = audit.intervals.iter().any(|i| !i.commit.from_history);

    let mut b = String::with_capacity(64 * 1024);

    // ---- document head -------------------------------------------------
    let _ = write!(
        b,
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>git receipts — {name}</title>\n<style>{STYLE}</style>\n</head><body>\n\
         <div class=\"wrap\">\n\
         <h1><span class=\"cmd\">git receipts</span> — what your agent actually did</h1>\n\
         <div class=\"sub\">{name}<br>repo {repo}",
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
         <div>{} file mutations · {} commands · {} observations</div>\
         <div>blast radius: {} local-fs · {} local-git · {} remote-git · {} network · {} read-only</div>\
         </div>\n",
        stats.kept - stats.duplicates,
        stats.lines,
        stats.skipped_types,
        stats.unparseable,
        dup,
        audit.file_claims,
        audit.commands,
        audit.observations,
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
         <div class=\"stat {}\"><b>{red}</b><span>red · broken promises</span></div>\
         <div class=\"stat warn\"><b>{residue_only}</b><span>residue-only intervals</span></div>\
         </div>\n",
        pct(green, total),
        pct(claims_landed, claims_total),
        if red == 0 { "good" } else { "bad" },
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
        red,
    );

    // ---- controls ------------------------------------------------------
    let _ = write!(
        b,
        "<div class=\"controls\"><h2>interval spine — {total} commits ({} agent-committed)</h2>\
         <label>show <select id=\"filter\">\
         <option value=\"all\" selected>all</option>\
         <option value=\"red\">red only</option>\
         <option value=\"red-residue\">red + residue</option>\
         </select></label></div>\n<section class=\"ledger\">\n",
        audit.intervals.iter().filter(|i| i.agent_committed).count(),
    );

    for iv in &audit.intervals {
        render_interval(&mut b, iv, show_intent, enriched);
    }
    b.push_str("</section>\n");

    // ---- balance + footer ---------------------------------------------
    let residue_total: usize = audit.intervals.iter().map(|i| i.residue.len()).sum();
    let _ = write!(
        b,
        "<div class=\"balance\">balance: {green} green · {residue_only} residue-only · {red} red \
         of {total} intervals ({:.0}% green) · claims landed {claims_landed}/{claims_total} · \
         residue {residue_total}</div>\n\
         <footer>Generated by <code>git receipts</code>. Every line is derived from a receipt — \
         the log's own tool calls, or git's own diffs. Broken promises are content-verified, not assumed.</footer>\n\
         <script>{SCRIPT}</script>\n</div>\n</body></html>\n",
        pct(green, total),
    );
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
) {
    let all = || audit.intervals.iter().flat_map(|i| i.ledger.iter());
    let late = all().filter(|l| l.landing == Landing::Late).count();
    let resolved = |needle: &str| {
        all()
            .filter(|l| l.resolution.as_deref().is_some_and(|r| r.contains(needle)))
            .count()
    };
    let superseded = resolved("superseded");
    let persisted = resolved("persisted outside git");
    let deliberate = resolved("removes or moves");
    let attributed: usize = audit
        .intervals
        .iter()
        .map(|i| i.attributed_residue.len())
        .sum();
    let dismissed: usize = audit
        .intervals
        .iter()
        .map(|i| i.dismissed_residue.len())
        .sum();
    let residue: usize = audit.intervals.iter().map(|i| i.residue.len()).sum();

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
             <span class=\"dim\">(est. from the log — approximate, not billing; ~{} input, ~{} cached)</span></div>",
            audit.commands,
            crate::report::abbrev(tk.output),
            tk.requests,
            crate::report::abbrev(tk.input),
            crate::report::abbrev(tk.cache_read),
        );
    }
    b.push_str("<div class=\"heading\">what happened to every exception</div>");
    if late > 0 {
        let _ = write!(
            b,
            "<div class=\"line dim\">· claims that landed late: {late}, content-verified against the commit they landed in</div>"
        );
    }
    if superseded + persisted + deliberate > 0 {
        let _ = write!(
            b,
            "<div class=\"line dim\">· never landed, resolved: {} — superseded by later landed edits: {superseded} · removed deliberately: {deliberate} · persisted on disk outside git: {persisted}</div>",
            superseded + persisted + deliberate
        );
    }
    let _ = write!(
        b,
        "<div class=\"line dim\">· unclaimed changes (residue): {} — attributed to commands: {attributed} · dismissed as now ignored/untracked: {dismissed} · unexplained: {residue}</div>",
        residue + attributed + dismissed
    );
    let cls = if broken == 0 { "ok" } else { "bad" };
    let _ = write!(
        b,
        "<div class=\"line bottomline {cls}\">· broken promises (never landed, nothing explains it): {broken}</div>"
    );
    b.push_str("</div>\n");
}

fn render_interval(
    b: &mut String,
    iv: &crate::reconcile::Interval,
    show_intent: bool,
    enriched: bool,
) {
    let (cls, mark) = match iv.status() {
        Status::Green => ("green", "✔"),
        Status::ResidueOnly => ("residue", "!"),
        Status::Red => ("red", "✘"),
    };
    let mut subject = iv.commit.subject.clone();
    if subject.chars().count() > 72 {
        subject = subject.chars().take(72).collect::<String>() + "…";
    }
    let _ = write!(
        b,
        "<article class=\"interval {cls}\"><header>\
         <span class=\"mark\">{mark}</span><code class=\"hash\">{}</code>\
         <span class=\"date\">{}</span><span class=\"subject\">{}</span>",
        esc(&iv.commit.short),
        iv.commit.ts.format("%m-%d %H:%M"),
        esc(&subject),
    );
    if !iv.agent_committed {
        b.push_str("<span class=\"tag\">keyframe: not committed by agent</span>");
    }
    if enriched && iv.commit.from_history {
        b.push_str("<span class=\"tag\">created elsewhere</span>");
    }
    if !iv.commit.reachable {
        b.push_str("<span class=\"tag\">reflog only</span>");
    }
    if iv.commit.clock_anomaly {
        b.push_str("<span class=\"tag warn\">clock anomaly</span>");
    }
    b.push_str("</header>");

    if show_intent && let Some(first) = iv.intents.first() {
        let more = if iv.intents.len() > 1 {
            format!(" (+{} more)", iv.intents.len() - 1)
        } else {
            String::new()
        };
        let _ = write!(
            b,
            "<div class=\"line intent\">» intent: {}{}</div>",
            esc(first),
            esc(&more)
        );
    }
    if iv.spine_jump {
        b.push_str(
            "<div class=\"line warn\">⑂ spine jump: parent is not the previous commit</div>",
        );
    }
    for s in &iv.superseded {
        let _ = write!(
            b,
            "<div class=\"line warn\">↻ amend: draft {} committed {} files, amended {}s later</div>",
            esc(&s.short),
            s.files,
            s.seconds_before_amend
        );
    }

    let never: Vec<_> = iv.never_landed().collect();
    let resolved: Vec<_> = iv.resolved_never().collect();
    let late: Vec<_> = iv.landed_late().collect();
    let landed = iv.ledger.len() - never.len() - resolved.len() - late.len();
    let mut notes = String::new();
    if !late.is_empty() {
        let _ = write!(notes, " / {} landed late", late.len());
    }
    if !resolved.is_empty() {
        let _ = write!(notes, " / {} resolved", resolved.len());
    }
    if !iv.attributed_residue.is_empty() {
        let _ = write!(
            notes,
            " / +{} command-attributed",
            iv.attributed_residue.len()
        );
    }
    if !iv.dismissed_residue.is_empty() {
        let _ = write!(notes, " / +{} dismissed", iv.dismissed_residue.len());
    }
    let _ = write!(
        b,
        "<div class=\"counts\">{} claimed / {landed} landed / {} residue{notes} ({} commands)</div>",
        iv.ledger.len(),
        iv.residue.len(),
        iv.commands,
    );

    for l in &late {
        let (at, dist) = l
            .landed_at
            .as_ref()
            .map(|(s, d)| (s.as_str(), *d))
            .unwrap_or(("?", 1));
        let _ = write!(
            b,
            "<div class=\"line ok\">✓ landed late: {} <span class=\"dim\">(content verified in {}, {} commit(s) later)</span></div>",
            esc(&l.path),
            esc(at),
            dist
        );
    }
    for l in &resolved {
        let _ = write!(
            b,
            "<div class=\"line warn\">◌ never landed, resolved: {}</div>",
            esc(&l.path)
        );
        if let Some(r) = &l.resolution {
            let _ = write!(b, "<div class=\"line dim indent\">{}</div>", esc(r));
        }
    }
    for l in &never {
        let _ = write!(
            b,
            "<div class=\"line bad\">✘ never landed: {} <span class=\"dim\">({} edit(s), f#{})</span></div>",
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
            "<div class=\"line warn\">● residue: {} <span class=\"dim\">[{}] changed, never claimed</span></div>",
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
            "<div class=\"line dim\">● attributed: {}{} <span class=\"dim\">[{}] — {}</span></div>",
            esc(&res.path),
            moved,
            res.status,
            esc(why)
        );
    }
    for (res, why) in &iv.dismissed_residue {
        let _ = write!(
            b,
            "<div class=\"line dim\">○ dismissed: {} <span class=\"dim\">[{}] — {}</span></div>",
            esc(&res.path),
            res.status,
            esc(why)
        );
    }
    b.push_str("</article>\n");
}
