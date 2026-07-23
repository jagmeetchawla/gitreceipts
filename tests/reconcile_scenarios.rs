//! Focused reconcile scenarios — each one pins a behavior that a real
//! session actually exercised: amend collapse, git's second-granular
//! timestamps, multi-commit Bash calls, partial staging, and cwds that
//! must not count as repo roots.

mod common;

use common::{SessionBuilder, TempRepo};
use gitreceipts::reconcile::{Landing, Status};
use gitreceipts::{causal, extract, ingest, reconcile};

fn run(repo: &TempRepo, session: &SessionBuilder) -> gitreceipts::reconcile::Audit {
    let path = repo.root.join("session.jsonl");
    session.save(&path);
    let (records, _) = ingest::ingest(&path).unwrap();
    let ordered = causal::order(records);
    let extracted = extract::extract(&ordered);
    reconcile::reconcile(&repo.root, &extracted).unwrap()
}

#[test]
fn amend_collapses_the_draft_into_its_survivor() {
    let repo = TempRepo::new("amend");
    let root = repo.root.display().to_string();

    repo.write("a.txt", "x");
    repo.git(&["add", "-A"]);
    repo.git_at(
        &["commit", "-q", "-m", "draft"],
        Some("2026-01-01T10:00:20Z"),
    );
    repo.write("b.txt", "x");
    repo.git(&["add", "-A"]);
    repo.git_at(
        &["commit", "-q", "--amend", "-m", "final"],
        Some("2026-01-01T10:00:32Z"),
    );

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "go")
        .write_claim("2026-01-01T10:00:05Z", "2026-01-01T10:00:06Z", "a.txt")
        .write_claim("2026-01-01T10:00:25Z", "2026-01-01T10:00:26Z", "b.txt")
        .bash_claim(
            "2026-01-01T10:00:30Z",
            "2026-01-01T10:00:33Z",
            "git add -A && git commit --amend -m final",
        );
    let audit = run(&repo, &s);

    assert_eq!(audit.intervals.len(), 1, "draft and amend are one interval");
    let interval = &audit.intervals[0];
    assert_eq!(interval.superseded.len(), 1);
    assert_eq!(interval.superseded[0].seconds_before_amend, 12);
    assert!(
        interval.balanced(),
        "both claims land in the amended statement"
    );
}

#[test]
fn commit_event_in_the_same_second_still_claims_its_interval() {
    // git truncates committer dates to whole seconds; the Bash event that
    // created the commit can carry a timestamp a few hundred ms past it.
    let repo = TempRepo::new("slack");
    let root = repo.root.display().to_string();

    repo.write("a.txt", "x");
    repo.git(&["add", "-A"]);
    repo.git_at(
        &["commit", "-q", "-m", "first"],
        Some("2026-01-01T10:00:31Z"),
    );

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "go")
        .write_claim("2026-01-01T10:00:10Z", "2026-01-01T10:00:11Z", "a.txt")
        .bash_claim(
            "2026-01-01T10:00:31.400Z",
            "2026-01-01T10:00:32Z",
            "git add -A && git commit -m first",
        );
    let audit = run(&repo, &s);

    assert_eq!(audit.intervals.len(), 1);
    assert!(
        audit.intervals[0].agent_committed,
        "the 400ms-late event must not fall past its own commit"
    );
}

#[test]
fn one_bash_call_with_two_commits_claims_two_intervals() {
    let repo = TempRepo::new("twocommits");
    let root = repo.root.display().to_string();

    repo.write("a.txt", "x");
    repo.git(&["add", "a.txt"]);
    repo.git_at(&["commit", "-q", "-m", "one"], Some("2026-01-01T10:00:30Z"));
    repo.write("b.txt", "x");
    repo.git(&["add", "b.txt"]);
    repo.git_at(&["commit", "-q", "-m", "two"], Some("2026-01-01T10:00:31Z"));

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "go")
        .write_claim("2026-01-01T10:00:10Z", "2026-01-01T10:00:11Z", "a.txt")
        .write_claim("2026-01-01T10:00:12Z", "2026-01-01T10:00:13Z", "b.txt")
        .bash_claim(
            "2026-01-01T10:00:29Z",
            "2026-01-01T10:00:32Z",
            "git add a.txt && git commit -m one && git add b.txt && git commit -m two",
        );
    let audit = run(&repo, &s);

    assert_eq!(audit.intervals.len(), 2);
    assert!(audit.intervals[0].agent_committed);
    assert!(
        audit.intervals[1].agent_committed,
        "the second commit of the same Bash call is not a keyframe"
    );
}

#[test]
fn partially_staged_claim_clears_one_statement_late() {
    let repo = TempRepo::new("carry");
    let root = repo.root.display().to_string();

    // both files edited before the first commit; only a.txt staged there,
    // b.txt rides in the next commit
    repo.write("a.txt", "x");
    repo.write("b.txt", "x");
    repo.git(&["add", "a.txt"]);
    repo.git_at(&["commit", "-q", "-m", "one"], Some("2026-01-01T10:00:20Z"));
    repo.git(&["add", "b.txt"]);
    repo.git_at(&["commit", "-q", "-m", "two"], Some("2026-01-01T10:00:40Z"));

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "go")
        .write_claim("2026-01-01T10:00:10Z", "2026-01-01T10:00:11Z", "a.txt")
        .write_claim("2026-01-01T10:00:12Z", "2026-01-01T10:00:13Z", "b.txt")
        .bash_claim(
            "2026-01-01T10:00:19Z",
            "2026-01-01T10:00:21Z",
            "git add a.txt && git commit -m one",
        )
        .bash_claim(
            "2026-01-01T10:00:39Z",
            "2026-01-01T10:00:41Z",
            "git add b.txt && git commit -m two",
        );
    let audit = run(&repo, &s);

    assert_eq!(audit.intervals.len(), 2);
    let first = &audit.intervals[0];
    let b_line = first.ledger.iter().find(|l| l.path == "b.txt").unwrap();
    assert_eq!(
        b_line.landing,
        Landing::Late,
        "b.txt cleared next statement"
    );
    assert!(first.balanced(), "late is not red");
    assert!(
        audit.intervals[1].residue.is_empty(),
        "the late landing is struck from the next interval's residue"
    );
}

#[test]
fn scratch_dir_cwd_never_counts_as_a_repo_root() {
    let repo = TempRepo::new("scratch");
    let root = repo.root.display().to_string();
    let scratch = std::env::temp_dir()
        .join(format!("gitreceipts-scratchpad-{}", std::process::id()))
        .display()
        .to_string();

    repo.write("a.txt", "x");
    repo.git(&["add", "-A"]);
    repo.git_at(&["commit", "-q", "-m", "one"], Some("2026-01-01T10:00:30Z"));

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "go")
        .write_claim("2026-01-01T10:00:10Z", "2026-01-01T10:00:11Z", "a.txt")
        .set_cwd(&scratch)
        .write_claim("2026-01-01T10:00:15Z", "2026-01-01T10:00:16Z", "probe.txt")
        .set_cwd(&root)
        .bash_claim(
            "2026-01-01T10:00:29Z",
            "2026-01-01T10:00:31Z",
            "git add -A && git commit -m one",
        );
    let audit = run(&repo, &s);

    assert!(
        audit
            .out_of_repo
            .iter()
            .any(|(p, _)| p.ends_with("probe.txt")),
        "the scratch write is quarantined, not a red line"
    );
    assert!(
        !audit.intervals[0]
            .ledger
            .iter()
            .any(|l| l.path == "probe.txt"),
        "probe.txt must not enter the ledger"
    );
    assert!(audit.intervals[0].balanced());
}

#[test]
fn hostile_dashed_filename_cannot_inject_git_options() {
    // A session claiming a file named `--stdin` that never lands drives the
    // diagnosis path through `git check-ignore` — without the `--` guard
    // this hangs forever on the inherited stdin.
    let repo = TempRepo::new("dashfile");
    let root = repo.root.display().to_string();

    repo.write("a.txt", "x");
    repo.git(&["add", "-A"]);
    repo.git_at(&["commit", "-q", "-m", "one"], Some("2026-01-01T10:00:30Z"));

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "go")
        .write_claim("2026-01-01T10:00:10Z", "2026-01-01T10:00:11Z", "a.txt")
        .write_claim("2026-01-01T10:00:12Z", "2026-01-01T10:00:13Z", "--stdin")
        .bash_claim(
            "2026-01-01T10:00:29Z",
            "2026-01-01T10:00:31Z",
            "git add -A && git commit -m one",
        );
    let audit = run(&repo, &s);

    let line = audit.intervals[0]
        .ledger
        .iter()
        .find(|l| l.path == "--stdin")
        .expect("the claim is in the ledger");
    assert_eq!(line.landing, Landing::Never);
    assert!(line.diagnosis.is_some(), "diagnosis ran and returned");
}

#[test]
fn backdated_commit_cannot_hide_from_the_spine() {
    // GIT_COMMITTER_DATE forges both the committer date AND the reflog
    // timestamp — but not the reflog's order. A commit created between two
    // in-window commits stays in the audit, flagged as a clock anomaly.
    let repo = TempRepo::new("backdate");
    let root = repo.root.display().to_string();

    repo.write("a.txt", "x");
    repo.git(&["add", "a.txt"]);
    repo.git_at(&["commit", "-q", "-m", "one"], Some("2026-01-01T10:00:20Z"));
    repo.write("hidden.txt", "x");
    repo.git(&["add", "hidden.txt"]);
    repo.git_at(
        &["commit", "-q", "-m", "sneaky"],
        Some("2020-06-06T06:06:06Z"), // far outside the session window
    );
    repo.write("b.txt", "x");
    repo.git(&["add", "b.txt"]);
    repo.git_at(&["commit", "-q", "-m", "two"], Some("2026-01-01T10:00:40Z"));

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "go")
        .write_claim("2026-01-01T10:00:10Z", "2026-01-01T10:00:11Z", "a.txt")
        .write_claim("2026-01-01T10:00:12Z", "2026-01-01T10:00:13Z", "b.txt")
        .bash_claim(
            "2026-01-01T10:00:19Z",
            "2026-01-01T10:00:41Z",
            "git commit -m one && git commit -m sneaky && git commit -m two",
        );
    let audit = run(&repo, &s);

    assert_eq!(audit.intervals.len(), 3, "the backdated commit is audited");
    let sneaky = &audit.intervals[1];
    assert_eq!(sneaky.commit.subject, "sneaky");
    assert!(sneaky.commit.clock_anomaly, "and flagged as untrustworthy");
    assert!(!audit.intervals[0].commit.clock_anomaly && !audit.intervals[2].commit.clock_anomaly);
    // its statement is still checked: hidden.txt shows as residue,
    // which alone is a warning (yellow), not a broken promise (red)
    assert!(sneaky.residue.iter().any(|c| c.path == "hidden.txt"));
    assert_eq!(sneaky.status(), Status::ResidueOnly);
}

#[test]
fn coincidental_same_path_change_does_not_launder_a_lost_claim() {
    // The agent claims content for a.txt that never lands. The NEXT commit
    // happens to touch a.txt too — but with unrelated content. Path-only
    // carry-forward would call the claim "landed late" and strike the
    // residue; the content check keeps both red.
    let repo = TempRepo::new("launder");
    let root = repo.root.display().to_string();

    repo.write("a.txt", "original");
    repo.git(&["add", "a.txt"]);
    repo.git_at(&["commit", "-q", "-m", "one"], Some("2026-01-01T10:00:20Z"));
    repo.write("b.txt", "x");
    repo.git(&["add", "b.txt"]);
    repo.git_at(&["commit", "-q", "-m", "two"], Some("2026-01-01T10:00:40Z"));
    repo.write("a.txt", "completely unrelated human rewrite");
    repo.git(&["add", "a.txt"]);
    repo.git_at(
        &["commit", "-q", "-m", "three"],
        Some("2026-01-01T10:01:00Z"),
    );

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "go")
        .write_claim("2026-01-01T10:00:10Z", "2026-01-01T10:00:11Z", "a.txt")
        .write_claim("2026-01-01T10:00:12Z", "2026-01-01T10:00:13Z", "b.txt")
        // claimed between commits one and two; its content ("x" from the
        // builder) never reaches any commit — commit three's a.txt is a
        // different rewrite
        .write_claim("2026-01-01T10:00:25Z", "2026-01-01T10:00:26Z", "a.txt")
        .bash_claim(
            "2026-01-01T10:00:19Z",
            "2026-01-01T10:01:01Z",
            "git commit -m one && git commit -m two && git commit -m three",
        );
    let audit = run(&repo, &s);

    assert_eq!(audit.intervals.len(), 3);
    let second = &audit.intervals[1]; // closes at commit "two"
    let line = second
        .ledger
        .iter()
        .find(|l| l.path == "a.txt")
        .expect("the mid-session a.txt claim maps to interval two");
    assert_eq!(
        line.landing,
        Landing::Never,
        "unrelated content in commit three must not launder this claim"
    );
    assert!(
        audit.intervals[2].residue.iter().any(|c| c.path == "a.txt"),
        "and commit three's real residue stays visible"
    );
}
