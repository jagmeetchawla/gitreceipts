//! The JSON receipt mirrors the reconciled audit: the same headline numbers
//! the console prints, per-finding detail (a broken promise carries
//! `broken: true`), honored `--no-intent` redaction, and it round-trips as
//! valid JSON under the versioned schema.

mod common;

use common::{SessionBuilder, TempRepo};
use gitreceipts::extract::Session;
use gitreceipts::ingest::IngestStats;
use gitreceipts::receipt::{Receipt, SCHEMA_VERSION};
use gitreceipts::reconcile::Audit;
use gitreceipts::{causal, extract, ingest, reconcile};

fn audit(repo: &TempRepo, session: &SessionBuilder) -> (Session, IngestStats, Audit) {
    let path = repo.root.join("session.jsonl");
    session.save(&path);
    let (records, stats) = ingest::ingest(&path).unwrap();
    let ordered = causal::order(records);
    let extracted = extract::extract(&ordered);
    let a = reconcile::reconcile(&repo.root, &extracted).unwrap();
    (extracted, stats, a)
}

/// One green interval (a claim that lands) and one red interval (a claim that
/// never lands) — enough to exercise every headline the receipt carries.
fn green_and_broken(name: &str) -> (TempRepo, SessionBuilder) {
    let repo = TempRepo::new(name);
    let root = repo.root.display().to_string();

    // Interval 1: a.txt is claimed and actually committed → green.
    repo.write("a.txt", &SessionBuilder::default_body("a.txt"));
    repo.git(&["add", "-A"]);
    repo.git_at(
        &["commit", "-q", "-m", "land a"],
        Some("2026-02-01T10:05:00Z"),
    );

    // Interval 2: c.txt lands, b.txt is claimed but never committed → red.
    repo.write("c.txt", &SessionBuilder::default_body("c.txt"));
    repo.git(&["add", "-A"]);
    repo.git_at(
        &["commit", "-q", "-m", "land c"],
        Some("2026-02-01T10:15:00Z"),
    );

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-02-01T10:00:00Z", "make a")
        .write_claim("2026-02-01T10:01:00Z", "2026-02-01T10:01:01Z", "a.txt")
        .bash_claim(
            "2026-02-01T10:04:00Z",
            "2026-02-01T10:05:01Z",
            "git add -A && git commit -m 'land a'",
        )
        .user_text("2026-02-01T10:09:00Z", "make b and c")
        .write_claim("2026-02-01T10:10:00Z", "2026-02-01T10:10:01Z", "b.txt")
        .write_claim("2026-02-01T10:11:00Z", "2026-02-01T10:11:01Z", "c.txt")
        .bash_claim(
            "2026-02-01T10:14:00Z",
            "2026-02-01T10:15:01Z",
            "git add -A && git commit -m 'land c'",
        );
    (repo, s)
}

#[test]
fn receipt_headline_numbers_mirror_the_audit() {
    let (repo, s) = green_and_broken("receipt-headline");
    let (session, stats, a) = audit(&repo, &s);
    let r = Receipt::build(
        "session",
        "/tmp/receipt",
        &session,
        &stats,
        &a,
        true,
        true,
        false,
        None,
        gitreceipts::report::Filter::All,
        false,
    );

    assert_eq!(r.schema_version, SCHEMA_VERSION);
    assert_eq!(r.tool.name, "gitreceipts");

    // The audit is the source of truth; the receipt must not drift from it.
    assert_eq!(r.summary.commits, a.intervals.len());
    assert_eq!(r.summary.commits, 2);
    assert_eq!(r.summary.prompts, a.prompts);
    let broken_in_audit = a.intervals.iter().flat_map(|i| i.never_landed()).count();
    assert_eq!(r.summary.broken_promises, broken_in_audit);
    assert_eq!(r.summary.broken_promises, 1);

    // One green, one red.
    assert_eq!(r.summary.balance.green, 1);
    assert_eq!(r.summary.balance.red, 1);
    assert_eq!(r.summary.balance.total, 2);

    // a.txt and c.txt landed; b.txt did not.
    assert_eq!(r.summary.claims_total, 3);
    assert_eq!(r.summary.claims_landed, 2);
}

#[test]
fn broken_promise_is_flagged_at_the_ledger_line() {
    let (repo, s) = green_and_broken("receipt-ledger");
    let (session, stats, a) = audit(&repo, &s);
    let r = Receipt::build(
        "session",
        "/tmp/receipt",
        &session,
        &stats,
        &a,
        true,
        true,
        false,
        None,
        gitreceipts::report::Filter::All,
        false,
    );

    let broken: Vec<&str> = r
        .intervals
        .iter()
        .flat_map(|i| i.ledger.iter())
        .filter(|l| l.broken)
        .map(|l| l.path.as_str())
        .collect();
    assert_eq!(broken, ["b.txt"], "only the never-landed claim is broken");

    // Its interval is red; the other is green.
    let statuses: Vec<&str> = r.intervals.iter().map(|i| i.status).collect();
    assert!(statuses.contains(&"green"));
    assert!(statuses.contains(&"red"));
}

#[test]
fn no_intent_redacts_prompt_text_but_keeps_counts() {
    let (repo, s) = green_and_broken("receipt-redact");
    let (session, stats, a) = audit(&repo, &s);

    let shown = Receipt::build(
        "session",
        "/tmp/receipt",
        &session,
        &stats,
        &a,
        true,
        true,
        false,
        None,
        gitreceipts::report::Filter::All,
        false,
    );
    let redacted = Receipt::build(
        "session",
        "/tmp/receipt",
        &session,
        &stats,
        &a,
        false,
        true,
        false,
        None,
        gitreceipts::report::Filter::All,
        false,
    );

    let intent_strings =
        |r: &Receipt| -> usize { r.intervals.iter().map(|i| i.intents.len()).sum() };
    assert!(intent_strings(&shown) > 0, "intents present when shown");
    assert_eq!(
        intent_strings(&redacted),
        0,
        "intents dropped when redacted"
    );
    // Counts are unaffected by redaction.
    assert_eq!(shown.summary.prompts, redacted.summary.prompts);
}

#[test]
fn receipt_round_trips_as_valid_json() {
    let (repo, s) = green_and_broken("receipt-json");
    let (session, stats, a) = audit(&repo, &s);
    let r = Receipt::build(
        "session",
        "/tmp/receipt",
        &session,
        &stats,
        &a,
        true,
        true,
        false,
        None,
        gitreceipts::report::Filter::All,
        false,
    );

    for pretty in [true, false] {
        let json = r.to_json(pretty).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["schema_version"], SCHEMA_VERSION);
        assert_eq!(parsed["summary"]["broken_promises"], 1);
        assert!(parsed["intervals"].is_array());
    }
}

#[test]
fn command_text_is_full_and_output_is_opt_in() {
    let repo = TempRepo::new("receipt-output");
    let root = repo.root.display().to_string();
    repo.write("a.txt", &SessionBuilder::default_body("a.txt"));
    repo.git(&["add", "-A"]);
    repo.git_at(
        &["commit", "-q", "-m", "land a"],
        Some("2026-03-01T10:05:00Z"),
    );

    // A multi-line command whose display summary (first meaningful line) is a
    // strict prefix of the full text — so we can tell truncation from whole.
    let full_cmd = "cd repo\n# stage and commit\ngit add -A && git commit -m 'land a'";
    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-03-01T10:00:00Z", "go")
        .write_claim("2026-03-01T10:01:00Z", "2026-03-01T10:01:01Z", "a.txt")
        .bash_claim_with_output(
            "2026-03-01T10:04:00Z",
            "2026-03-01T10:05:01Z",
            full_cmd,
            "created commit abc123",
        );
    let (session, stats, a) = audit(&repo, &s);

    // The full command survives — comment line and all — not just a summary.
    let cmd = |r: &Receipt| -> String {
        r.intervals
            .iter()
            .flat_map(|i| i.commands.runs.iter())
            .map(|c| c.command.clone())
            .find(|c| c.contains("git commit"))
            .expect("the commit command run")
    };

    let without = Receipt::build(
        "s",
        "/tmp/r",
        &session,
        &stats,
        &a,
        true,
        true,
        false,
        None,
        gitreceipts::report::Filter::All,
        false,
    );
    assert_eq!(cmd(&without), full_cmd, "full multi-line command retained");
    let no_output = without
        .intervals
        .iter()
        .flat_map(|i| i.commands.runs.iter())
        .all(|c| c.output.is_none());
    assert!(no_output, "output omitted by default");

    let with = Receipt::build(
        "s",
        "/tmp/r",
        &session,
        &stats,
        &a,
        true,
        true,
        true,
        None,
        gitreceipts::report::Filter::All,
        false,
    );
    let out = with
        .intervals
        .iter()
        .flat_map(|i| i.commands.runs.iter())
        .find_map(|c| c.output.as_ref())
        .expect("output present under --with-output");
    assert_eq!(out.text, "created commit abc123");
    assert!(!out.is_error);
}

#[test]
fn commit_scope_filters_intervals_but_keeps_the_whole_session_summary() {
    let (repo, s) = green_and_broken("receipt-scope");
    let (session, stats, a) = audit(&repo, &s);
    let target = a.intervals[0].commit.hash.clone();

    let r = Receipt::build(
        "s",
        "/tmp/r",
        &session,
        &stats,
        &a,
        true,
        true,
        false,
        Some(&target),
        gitreceipts::report::Filter::All,
        false,
    );
    // Only the scoped commit is in the intervals array…
    assert_eq!(r.intervals.len(), 1);
    assert_eq!(r.intervals[0].commit.hash, target);
    // …but the summary still describes the whole session (2 commits).
    assert_eq!(r.summary.commits, 2);
    assert_eq!(r.summary.broken_promises, 1);
}

#[test]
fn a_failed_command_carries_output_by_default() {
    let repo = TempRepo::new("receipt-failout");
    let root = repo.root.display().to_string();
    repo.write("a.txt", &SessionBuilder::default_body("a.txt"));
    repo.git(&["add", "-A"]);
    repo.git_at(
        &["commit", "-q", "-m", "land a"],
        Some("2026-04-01T10:05:00Z"),
    );

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-04-01T10:00:00Z", "go")
        .write_claim("2026-04-01T10:01:00Z", "2026-04-01T10:01:01Z", "a.txt")
        // one command that succeeds, one that fails
        .bash_claim_with_output(
            "2026-04-01T10:02:00Z",
            "2026-04-01T10:02:01Z",
            "npm run build",
            "build ok",
        )
        .bash_claim_failed(
            "2026-04-01T10:03:00Z",
            "2026-04-01T10:03:01Z",
            "npm test",
            "FAIL: 1 test failed",
        )
        .bash_claim(
            "2026-04-01T10:04:00Z",
            "2026-04-01T10:05:01Z",
            "git add -A && git commit -m 'land a'",
        );
    let (session, stats, a) = audit(&repo, &s);

    // Default (with_output = false): only the failed command carries output.
    let r = Receipt::build(
        "s",
        "/tmp/r",
        &session,
        &stats,
        &a,
        true,
        true,
        false,
        None,
        gitreceipts::report::Filter::All,
        false,
    );
    let with_out: Vec<(&str, bool)> = r
        .intervals
        .iter()
        .flat_map(|i| i.commands.runs.iter())
        .filter_map(|c| c.output.as_ref().map(|o| (c.command.as_str(), o.is_error)))
        .collect();
    assert_eq!(
        with_out.len(),
        1,
        "only the failed command carries output by default"
    );
    assert!(with_out[0].0.contains("npm test"));
    assert!(with_out[0].1, "and it is flagged is_error");
}

#[test]
fn full_adds_the_transcript_scoped_by_commit() {
    let (repo, s) = green_and_broken("receipt-full");
    let (session, stats, a) = audit(&repo, &s);
    let build = |commit: Option<&str>, full: bool| {
        Receipt::build(
            "s",
            "/tmp/r",
            &session,
            &stats,
            &a,
            true,
            true,
            false,
            commit,
            gitreceipts::report::Filter::All,
            full,
        )
    };

    // No --full: no transcript.
    assert!(build(None, false).transcript.is_none());

    // --full, whole session: both prompts present.
    let whole = build(None, true).transcript.expect("transcript");
    assert!(whole.iter().any(|m| m.text.contains("make a")));
    assert!(whole.iter().any(|m| m.text.contains("make b and c")));

    // --full scoped to the second commit: only its window's prompt.
    let second = a.intervals[1].commit.hash.clone();
    let scoped = build(Some(&second), true).transcript.expect("transcript");
    assert!(scoped.iter().any(|m| m.text.contains("make b and c")));
    assert!(
        !scoped.iter().any(|m| m.text.contains("make a")),
        "the first commit's prompt is outside the second commit's window"
    );
}
