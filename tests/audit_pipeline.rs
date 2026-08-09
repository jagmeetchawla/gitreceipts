//! End-to-end: a synthetic session against a real (temporary) git repo,
//! through the full pipeline — ingest → causal order → extract → reconcile.
//!
//! The scenario: the agent writes two files, deletes one before committing,
//! and commits the survivor plus a file it never claimed. The equation
//! must show one landed claim, one never-landed claim with a diagnosis,
//! and one residue file.

mod common;

use common::{SessionBuilder, TempRepo};
use gitreceipts::reconcile::{Landing, Status};
use gitreceipts::{causal, extract, html, ingest, reconcile, report};

fn build_session(root: &str) -> SessionBuilder {
    let mut s = SessionBuilder::new(root);
    s.user_text("2026-01-01T10:00:00Z", "build it")
        // claim 1: src.txt — will land in the commit
        .write_claim("2026-01-01T10:00:10Z", "2026-01-01T10:00:11Z", "src.txt")
        // claim 2: scratch.txt — deleted before the commit, never lands
        .write_claim(
            "2026-01-01T10:00:20Z",
            "2026-01-01T10:00:21Z",
            "scratch.txt",
        )
        // the commit command
        .bash_claim(
            "2026-01-01T10:00:30Z",
            "2026-01-01T10:00:32Z",
            "git add -A && git commit -m first",
        )
        // bookkeeping noise the ingester must skip, plus one garbage line
        .raw_line(r#"{"type":"queue-operation","uuid":"q1"}"#)
        .raw_line("{not json at all");
    s
}

#[test]
fn full_pipeline_balances_the_interval_equation() {
    let repo = TempRepo::new("pipeline");
    let root = repo.root.display().to_string();

    // what actually happened on disk: src.txt landed, scratch.txt was
    // deleted before the commit, extra.txt was created by "a command"
    // and committed without ever being claimed
    repo.write("src.txt", "hello");
    repo.write("extra.txt", "fallout");
    repo.git(&["add", "-A"]);
    repo.git_at(
        &["commit", "-q", "-m", "first"],
        Some("2026-01-01T10:00:31Z"),
    );

    let session_path = repo.root.join("session.jsonl");
    build_session(&root).save(&session_path);

    let (records, stats) = ingest::ingest(&session_path).unwrap();
    assert_eq!(stats.kept, 7);
    assert_eq!(stats.skipped_types, 1);
    assert_eq!(stats.unparseable, 1);

    let ordered = causal::order(records);
    let session = extract::extract(&ordered);
    assert_eq!(session.claims.len(), 3); // two writes + one command
    assert_eq!(session.prompts.len(), 1, "tool results are not prompts");

    let audit = reconcile::reconcile(&repo.root, &session).unwrap();

    assert_eq!(audit.intervals.len(), 1, "one commit, one interval");
    let interval = &audit.intervals[0];
    assert!(interval.agent_committed, "the Bash claim covers the commit");
    assert_eq!(
        interval.status(),
        Status::Amber,
        "unexplained residue (extra.txt) outranks the grey scratch finding — amber wins"
    );

    let landed: Vec<&str> = interval
        .ledger
        .iter()
        .filter(|l| l.landing == Landing::OnTime)
        .map(|l| l.path.as_str())
        .collect();
    assert_eq!(landed, vec!["src.txt"]);

    assert_eq!(
        interval.never_landed().count(),
        0,
        "scratch churn is resolved, not a broken promise"
    );
    let scratch: Vec<&gitreceipts::reconcile::LedgerLine> =
        interval.ledger.iter().filter(|l| l.scratch).collect();
    assert_eq!(scratch.len(), 1);
    assert_eq!(scratch[0].path, "scratch.txt");
    assert!(
        scratch[0]
            .diagnosis
            .unwrap()
            .starts_with("deleted before any commit"),
        "diagnosis was: {:?}",
        scratch[0].diagnosis
    );

    let residue: Vec<&str> = interval.residue.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(residue, vec!["extra.txt"]);
    assert_eq!(interval.effectful_commands, 1);

    assert_eq!(audit.grades.exact, 2);
    assert_eq!(
        audit.grades.receipted, 1,
        "the commit command is corroborated"
    );

    assert_eq!(audit.prompts, 1);
    assert_eq!(
        interval.intents,
        vec!["build it".to_string()],
        "the prompt that drove the interval is attached to it"
    );
}

#[test]
fn session_jsonl_itself_stays_out_of_the_equation() {
    // The session file lives inside the temp repo in these tests but is
    // never claimed and never committed — it must not appear anywhere.
    let repo = TempRepo::new("noleak");
    let root = repo.root.display().to_string();
    repo.write("src.txt", "hello");
    repo.git(&["add", "src.txt"]);
    repo.git_at(
        &["commit", "-q", "-m", "only"],
        Some("2026-01-01T10:00:31Z"),
    );

    let session_path = repo.root.join("session.jsonl");
    build_session(&root).save(&session_path);

    let (records, _) = ingest::ingest(&session_path).unwrap();
    let session = extract::extract(&causal::order(records));
    let audit = reconcile::reconcile(&repo.root, &session).unwrap();
    let interval = &audit.intervals[0];
    assert!(
        !interval.residue.iter().any(|c| c.path == "session.jsonl"),
        "uncommitted files are not residue"
    );
}

#[test]
fn html_report_is_self_contained_and_well_formed() {
    let repo = TempRepo::new("html");
    let root = repo.root.display().to_string();
    repo.write("src.txt", "hello");
    repo.write("extra.txt", "fallout");
    repo.git(&["add", "-A"]);
    repo.git_at(
        &["commit", "-q", "-m", "first"],
        Some("2026-01-01T10:00:31Z"),
    );
    let session_path = repo.root.join("session.jsonl");
    build_session(&root).save(&session_path);

    let (records, stats) = ingest::ingest(&session_path).unwrap();
    let session = extract::extract(&causal::order(records));
    let audit = reconcile::reconcile(&repo.root, &session).unwrap();
    let out = html::render(
        "sess",
        &root,
        &session,
        &stats,
        &audit,
        report::Show {
            prompt: true,
            summary: true,
        },
        true,
        gitreceipts::report::Expand::Auto,
        false,
        None,
        false,
    );

    assert!(out.starts_with("<!doctype html>"));
    assert!(out.trim_end().ends_with("</html>"));
    // self-contained: no external asset references
    for needle in ["src=", "href=", "@import", "http://", "https://"] {
        assert!(!out.contains(needle), "external ref found: {needle}");
    }
    // the report's headline is present
    assert!(out.contains("broken promises"));
    assert!(out.contains("interval spine"));
    // content is HTML-escaped, not raw
    assert!(!out.contains("<script>alert"));
}

#[test]
fn html_report_escapes_hostile_content() {
    // A session path / prompt containing HTML must not break out of text.
    let repo = TempRepo::new("htmlesc");
    let root = repo.root.display().to_string();
    repo.write("a.txt", "x");
    repo.git(&["add", "-A"]);
    repo.git_at(&["commit", "-q", "-m", "one"], Some("2026-01-01T10:00:31Z"));

    let mut s = SessionBuilder::new(&root);
    s.user_text(
        "2026-01-01T10:00:00Z",
        "look <script>alert(1)</script> here",
    )
    .write_claim("2026-01-01T10:00:05Z", "2026-01-01T10:00:06Z", "a.txt")
    .bash_claim(
        "2026-01-01T10:00:29Z",
        "2026-01-01T10:00:31Z",
        "git add -A && git commit -m one",
    );
    let path = repo.root.join("s.jsonl");
    s.save(&path);
    let (records, stats) = ingest::ingest(&path).unwrap();
    let session = extract::extract(&causal::order(records));
    let audit = reconcile::reconcile(&repo.root, &session).unwrap();
    let out = html::render(
        "sess",
        &root,
        &session,
        &stats,
        &audit,
        report::Show {
            prompt: true,
            summary: true,
        },
        true,
        gitreceipts::report::Expand::Auto,
        false,
        None,
        false,
    );

    assert!(
        out.contains("&lt;script&gt;"),
        "prompt HTML must be escaped"
    );
    assert!(
        !out.contains("<script>alert(1)"),
        "raw script must not appear"
    );
}

#[test]
fn html_drilldown_shows_statement_commands_and_push_status() {
    let repo = TempRepo::new("drill");
    let root = repo.root.display().to_string();
    repo.write("kept.txt", "x");
    repo.write("extra.txt", "fallout");
    repo.git(&["add", "-A"]);
    repo.git_at(
        &["commit", "-q", "-m", "first"],
        Some("2026-01-01T10:00:31Z"),
    );

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "go")
        .write_claim("2026-01-01T10:00:05Z", "2026-01-01T10:00:06Z", "kept.txt")
        .bash_claim(
            "2026-01-01T10:00:29Z",
            "2026-01-01T10:00:31Z",
            "git add -A && git commit -m first",
        );
    let path = repo.root.join("s.jsonl");
    s.save(&path);
    let (records, stats) = ingest::ingest(&path).unwrap();
    let session = extract::extract(&causal::order(records));
    let audit = reconcile::reconcile(&repo.root, &session).unwrap();
    let out = html::render(
        "sess",
        &root,
        &session,
        &stats,
        &audit,
        report::Show {
            prompt: true,
            summary: true,
        },
        true,
        gitreceipts::report::Expand::All,
        false,
        None,
        false,
    );

    assert!(out.contains("<details"), "intervals are collapsible");
    assert!(
        out.contains("files git recorded"),
        "statement breakdown present"
    );
    assert!(out.contains("class=\"frow\""), "per-file rows present");
    assert!(out.contains("class=\"crow\""), "per-command rows present");
    // no remote in a bare temp repo → local only
    assert!(out.contains("local only"), "push status shown");
    // commit command tagged
    assert!(out.contains("class=\"radtag commit\""));
}

// (home-path redaction moved to tests/home_redaction.rs — it needs the
// set-once redaction global and a synthetic home, so it lives in its own binary.)

#[test]
fn multibyte_commit_subject_does_not_panic_the_console_report() {
    // A commit subject with a multibyte char (em-dash, 3 bytes) straddling
    // the 56-byte truncation boundary crashed byte-based `truncate()` —
    // found by dogfooding a real repo. The console render must be char-safe.
    let repo = TempRepo::new("multibyte");
    let root = repo.root.display().to_string();
    repo.write("a.txt", "x");
    repo.git(&["add", "-A"]);
    let subject = format!("{}\u{2014}tail past the truncation window", "x".repeat(55));
    repo.commit_as("t", "t@t", "2026-01-01T10:00:20+00:00", &subject, None);

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "go")
        .write_claim("2026-01-01T10:00:05Z", "2026-01-01T10:00:06Z", "a.txt")
        .bash_claim(
            "2026-01-01T10:00:19Z",
            "2026-01-01T10:00:21Z",
            "git add -A && git commit -m x",
        );
    let path = repo.root.join("s.jsonl");
    s.save(&path);
    let (records, stats) = ingest::ingest(&path).unwrap();
    let session = extract::extract(&causal::order(records));
    let audit = reconcile::reconcile(&repo.root, &session).unwrap();

    report::print(
        "sess",
        &root,
        &session,
        &stats,
        &audit,
        &report::Options {
            color: report::ColorMode::Never,
            show: report::Show {
                prompt: true,
                summary: true,
            },
            show_identity: true,
            filter: report::Filter::All,
            format: report::Format::Text,
            expand: report::Expand::Auto,
            verbose: true,
            with_output: false,
            commit: None,
            oneline: false,
            summary: false,
            emoji: false,
            narrative: false,
            full: false,
            project_section: false,
            siblings: Vec::new(),
        },
    );
    let _ = html::render(
        "sess",
        &root,
        &session,
        &stats,
        &audit,
        report::Show {
            prompt: true,
            summary: true,
        },
        true,
        report::Expand::All,
        false,
        None,
        false,
    );
}

#[test]
fn project_html_is_one_self_contained_page_with_a_section_per_repo() {
    // A --project HTML page must be ONE self-contained document: a single
    // doctype/notice/filter/closing, the landing table, and one section per
    // repo. The single-repo shell parts (notice, color filter) appear exactly
    // once, not once per section.
    let build = |name: &str| {
        let repo = TempRepo::new(name);
        let root = repo.root.display().to_string();
        repo.write("a.txt", "x");
        repo.git(&["add", "-A"]);
        repo.git_at(&["commit", "-q", "-m", "one"], Some("2026-01-01T10:00:20Z"));
        let mut s = SessionBuilder::new(&root);
        s.user_text("2026-01-01T10:00:00Z", "go")
            .write_claim("2026-01-01T10:00:05Z", "2026-01-01T10:00:06Z", "a.txt")
            .bash_claim(
                "2026-01-01T10:00:19Z",
                "2026-01-01T10:00:21Z",
                "git add -A && git commit -m one",
            );
        let path = repo.root.join("s.jsonl");
        s.save(&path);
        let (records, stats) = ingest::ingest(&path).unwrap();
        let session = extract::extract(&causal::order(records));
        let audit = reconcile::reconcile(&repo.root, &session).unwrap();
        (repo, root, session, stats, audit)
    };
    let (_r1, root1, sess1, stats1, audit1) = build("alpha");
    let (_r2, root2, sess2, stats2, audit2) = build("beta");

    let sections = vec![
        html::HtmlSection {
            name: "alpha",
            session_name: "s",
            repo: &root1,
            session: &sess1,
            stats: &stats1,
            audit: &audit1,
        },
        html::HtmlSection {
            name: "beta",
            session_name: "s",
            repo: &root2,
            session: &sess2,
            stats: &stats2,
            audit: &audit2,
        },
    ];
    let page = html::render_project(
        "/tmp/myproject",
        &sections,
        report::Show {
            prompt: true,
            summary: true,
        },
        true,
        report::Expand::Auto,
        false,
        false,
    );

    let count = |needle: &str| page.matches(needle).count();
    assert_eq!(count("<!doctype html>"), 1, "one document");
    assert_eq!(count("</body></html>"), 1, "one closing");
    assert_eq!(count("private audit report"), 1, "notice appears once");
    assert_eq!(count("id=\"f-all\""), 1, "one global color filter");
    assert_eq!(count("class=\"repo-section\""), 2, "one section per repo");
    assert!(
        page.contains("class=\"landing\""),
        "the roll-up table is present"
    );
    // self-contained: no external stylesheet/script/image
    assert!(
        !page.contains("<link") && !page.contains("src=\"http") && !page.contains("href=\"http"),
        "must not reference external assets"
    );
    // both repos appear in the landing table
    assert!(page.contains(">alpha</a>") && page.contains(">beta</a>"));
}
