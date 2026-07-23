//! End-to-end: a synthetic session against a real (temporary) git repo,
//! through the full pipeline — ingest → causal order → extract → reconcile.
//!
//! The scenario: the agent writes two files, deletes one before committing,
//! and commits the survivor plus a file it never claimed. The equation
//! must show one landed claim, one never-landed claim with a diagnosis,
//! and one residue file.

mod common;

use common::{SessionBuilder, TempRepo};
use gitreceipts::reconcile::Landing;
use gitreceipts::{causal, extract, ingest, reconcile};

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
    assert!(
        !interval.balanced(),
        "a red interval: one lost claim + residue"
    );

    let landed: Vec<&str> = interval
        .ledger
        .iter()
        .filter(|l| l.landing == Landing::OnTime)
        .map(|l| l.path.as_str())
        .collect();
    assert_eq!(landed, vec!["src.txt"]);

    let never: Vec<&gitreceipts::reconcile::LedgerLine> = interval.never_landed().collect();
    assert_eq!(never.len(), 1);
    assert_eq!(never[0].path, "scratch.txt");
    assert!(
        never[0]
            .diagnosis
            .unwrap()
            .starts_with("deleted before any commit"),
        "diagnosis was: {:?}",
        never[0].diagnosis
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
