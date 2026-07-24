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
    let r = Receipt::build("session", "/tmp/receipt", &session, &stats, &a, true);

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
    let r = Receipt::build("session", "/tmp/receipt", &session, &stats, &a, true);

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

    let shown = Receipt::build("session", "/tmp/receipt", &session, &stats, &a, true);
    let redacted = Receipt::build("session", "/tmp/receipt", &session, &stats, &a, false);

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
    let r = Receipt::build("session", "/tmp/receipt", &session, &stats, &a, true);

    for pretty in [true, false] {
        let json = r.to_json(pretty).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["schema_version"], SCHEMA_VERSION);
        assert_eq!(parsed["summary"]["broken_promises"], 1);
        assert!(parsed["intervals"].is_array());
    }
}
