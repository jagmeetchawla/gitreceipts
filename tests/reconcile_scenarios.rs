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
    // b.txt rides in the next commit carrying the body the claim asserts
    repo.write("a.txt", "x");
    repo.write("b.txt", &SessionBuilder::default_body("b.txt"));
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
    assert_eq!(sneaky.status(), Status::Amber);
}

#[test]
fn later_same_path_change_counts_as_a_late_landing() {
    // Path-level landing (landing is landing, on time or late): the agent
    // claims a.txt, which does not land in its own interval; a later commit
    // touches a.txt, so that counts as a late landing and strikes the residue
    // there — the SAME bar the on-time check uses. We deliberately do not
    // content-verify, so an unrelated later change to the path launders the
    // claim; that is the accepted trade for a consistent, blob-free audit.
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
        Landing::Late,
        "a later commit touching a.txt is a late landing (path-level)"
    );
    assert!(
        !audit.intervals[2].residue.iter().any(|c| c.path == "a.txt"),
        "the late landing strikes commit three's a.txt residue"
    );
}

#[test]
fn claim_swept_into_a_much_later_commit_is_found_and_verified() {
    // The claimed file skips the next commit entirely and lands two
    // commits later — the forward sweep finds it, verifies content, and
    // strikes the residue where it landed.
    let repo = TempRepo::new("latesweep");
    let root = repo.root.display().to_string();

    // b.txt lands in commit three carrying the SAME body the claim will
    // assert, so the content sweep can verify it.
    let b_body = SessionBuilder::default_body("b.txt");
    repo.write("a.txt", "x");
    repo.git(&["add", "a.txt"]);
    repo.git_at(&["commit", "-q", "-m", "one"], Some("2026-01-01T10:00:20Z"));
    repo.write("c.txt", "x");
    repo.git(&["add", "c.txt"]);
    repo.git_at(&["commit", "-q", "-m", "two"], Some("2026-01-01T10:00:40Z"));
    repo.write("b.txt", &b_body);
    repo.git(&["add", "b.txt"]);
    repo.git_at(
        &["commit", "-q", "-m", "three"],
        Some("2026-01-01T10:01:00Z"),
    );

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "go")
        .write_claim("2026-01-01T10:00:10Z", "2026-01-01T10:00:11Z", "a.txt")
        // claimed before commit one, but only committed in commit three
        .write_claim("2026-01-01T10:00:12Z", "2026-01-01T10:00:13Z", "b.txt")
        .write_claim("2026-01-01T10:00:25Z", "2026-01-01T10:00:26Z", "c.txt")
        .bash_claim(
            "2026-01-01T10:00:19Z",
            "2026-01-01T10:01:01Z",
            "git commit -m one && git commit -m two && git commit -m three",
        );
    let audit = run(&repo, &s);

    assert_eq!(audit.intervals.len(), 3);
    let line = audit.intervals[0]
        .ledger
        .iter()
        .find(|l| l.path == "b.txt")
        .unwrap();
    assert_eq!(line.landing, Landing::Late);
    assert_eq!(line.landed_at.as_ref().unwrap().1, 2, "two commits later");
    assert!(
        audit.intervals[2].residue.iter().all(|c| c.path != "b.txt"),
        "the landing explains the residue at commit three"
    );
    assert!(audit.intervals[0].balanced());
}

#[test]
fn residue_later_gitignored_is_dismissed_not_yellow() {
    // A file committed unclaimed (residue) that the user has SINCE
    // gitignored and untracked is yesterday's noise: still listed, but
    // dismissed — the interval goes green, not yellow.
    let repo = TempRepo::new("dismiss");
    let root = repo.root.display().to_string();

    repo.write("a.txt", "x");
    repo.write("generated.plist", "junk");
    repo.git(&["add", "-A"]);
    repo.git_at(&["commit", "-q", "-m", "one"], Some("2026-01-01T10:00:20Z"));
    // later: user decides generated.plist should never have been tracked
    repo.git(&["rm", "-q", "--cached", "generated.plist"]);
    repo.write(".gitignore", "generated.plist\n");
    repo.git(&["add", ".gitignore"]);
    repo.git_at(
        &["commit", "-q", "-m", "ignore it"],
        Some("2026-01-01T10:00:40Z"),
    );

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "go")
        .write_claim("2026-01-01T10:00:10Z", "2026-01-01T10:00:11Z", "a.txt")
        .bash_claim(
            "2026-01-01T10:00:19Z",
            "2026-01-01T10:00:41Z",
            "git commit -m one && git commit -m two",
        );
    let audit = run(&repo, &s);

    let first = &audit.intervals[0];
    assert!(
        first.residue.iter().all(|c| c.path != "generated.plist"),
        "the since-ignored file is not live residue"
    );
    assert!(
        first
            .dismissed_residue
            .iter()
            .any(|(c, why)| c.path == "generated.plist" && why.contains("gitignored")),
        "but it is still listed, with the reason"
    );
    assert_eq!(
        first.status(),
        Status::Green,
        "dismissed residue does not color the interval"
    );
}

#[test]
fn rename_residue_is_attributed_to_the_git_mv_that_did_it() {
    // A whole-tree rename via git mv produces R-status residue with zero
    // exact claims; the command names the directories, so every child
    // attributes to it and the interval settles green.
    let repo = TempRepo::new("mvattr");
    let root = repo.root.display().to_string();

    repo.write("Old.xcodeproj/project.pbxproj", "x");
    repo.write("Sources/OldName/App.swift", "x");
    repo.git(&["add", "-A"]);
    repo.git_at(
        &["commit", "-q", "-m", "base"],
        Some("2026-01-01T10:00:20Z"),
    );
    repo.git(&["mv", "Old.xcodeproj", "New.xcodeproj"]);
    repo.git(&["mv", "Sources/OldName", "Sources/NewName"]);
    repo.git_at(
        &["commit", "-q", "-m", "rename"],
        Some("2026-01-01T10:00:40Z"),
    );

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "go")
        .write_claim(
            "2026-01-01T10:00:05Z",
            "2026-01-01T10:00:06Z",
            "Old.xcodeproj/project.pbxproj",
        )
        .write_claim(
            "2026-01-01T10:00:07Z",
            "2026-01-01T10:00:08Z",
            "Sources/OldName/App.swift",
        )
        .bash_claim(
            "2026-01-01T10:00:19Z",
            "2026-01-01T10:00:21Z",
            "git add -A && git commit -m base",
        )
        .bash_claim(
            "2026-01-01T10:00:30Z",
            "2026-01-01T10:00:41Z",
            "git mv Old.xcodeproj New.xcodeproj && git mv Sources/OldName Sources/NewName && git commit -m rename",
        );
    let audit = run(&repo, &s);

    let rename = &audit.intervals[1];
    assert!(rename.residue.is_empty(), "everything is accounted for");
    assert_eq!(rename.attributed_residue.len(), 2);
    let (change, why) = rename
        .attributed_residue
        .iter()
        .find(|(c, _)| c.path == "Sources/NewName/App.swift")
        .unwrap();
    assert_eq!(
        change.old_path.as_deref(),
        Some("Sources/OldName/App.swift")
    );
    assert!(why.contains("commands"));
    assert_eq!(rename.status(), Status::Green);
}

#[test]
fn a_commits_own_output_listing_residue_does_not_attribute_it() {
    // Round-2 review finding #1: `git commit`/`git status` output lists
    // the very files in the statement. Pooling that output would attribute
    // ANY co-committed unclaimed change to "a command named it" — laundering
    // real residue to green. Attribution is command TEXT only, so a
    // human-edited file committed alongside stays honest yellow residue.
    let repo = TempRepo::new("commitout");
    let root = repo.root.display().to_string();

    repo.write("claimed.txt", "x");
    repo.write("human_edit.txt", "not the agent");
    repo.git(&["add", "-A"]);
    repo.git_at(&["commit", "-q", "-m", "one"], Some("2026-01-01T10:00:20Z"));

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "go")
        .write_claim(
            "2026-01-01T10:00:05Z",
            "2026-01-01T10:00:06Z",
            "claimed.txt",
        )
        // the commit's captured output happens to list human_edit.txt
        .bash_claim_with_output(
            "2026-01-01T10:00:19Z",
            "2026-01-01T10:00:21Z",
            "git add -A && git commit -m one",
            " create mode 100644 human_edit.txt\n create mode 100644 claimed.txt",
        );
    let audit = run(&repo, &s);

    let interval = &audit.intervals[0];
    assert!(
        interval.residue.iter().any(|c| c.path == "human_edit.txt"),
        "the human edit stays real residue, not attributed away"
    );
    assert!(interval.attributed_residue.is_empty());
    assert_eq!(
        interval.status(),
        Status::Amber,
        "yellow, not laundered to green"
    );
}

#[test]
fn intermediate_edit_that_lands_in_a_later_commit_is_not_red() {
    // The agent edits a file, doesn't commit that state, edits it again later
    // and commits. Path-level: the first (intermediate) claim's path lands in
    // the later commit, so it is a late landing — green, not a broken promise.
    let repo = TempRepo::new("supersede");
    let root = repo.root.display().to_string();

    repo.write("a.txt", "x");
    repo.git(&["add", "a.txt"]);
    repo.git_at(&["commit", "-q", "-m", "one"], Some("2026-01-01T10:00:20Z"));
    repo.write("app.swift", "x");
    repo.git(&["add", "app.swift"]);
    repo.git_at(&["commit", "-q", "-m", "two"], Some("2026-01-01T10:00:40Z"));

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "go")
        .write_claim("2026-01-01T10:00:10Z", "2026-01-01T10:00:11Z", "a.txt")
        // intermediate app.swift state, never committed as such — its
        // content differs from what finally landed
        .write_claim_content(
            "2026-01-01T10:00:12Z",
            "2026-01-01T10:00:13Z",
            "app.swift",
            "draft that never shipped",
        )
        .bash_claim(
            "2026-01-01T10:00:19Z",
            "2026-01-01T10:00:21Z",
            "git add a.txt && git commit -m one",
        )
        // the later edit that actually lands in commit two
        .write_claim("2026-01-01T10:00:25Z", "2026-01-01T10:00:26Z", "app.swift")
        .bash_claim(
            "2026-01-01T10:00:39Z",
            "2026-01-01T10:00:41Z",
            "git add app.swift && git commit -m two",
        );
    let audit = run(&repo, &s);

    let first = &audit.intervals[0];
    let line = first.ledger.iter().find(|l| l.path == "app.swift").unwrap();
    assert_eq!(
        line.landing,
        Landing::Late,
        "the intermediate claim's path lands in the later commit (path-level)"
    );
    assert_eq!(
        first.status(),
        Status::Green,
        "an edit that lands later is not a broken promise"
    );
}

#[test]
fn file_removed_by_a_named_command_is_resolved_not_red() {
    let repo = TempRepo::new("deliberate");
    let root = repo.root.display().to_string();

    repo.write("a.txt", "x");
    repo.git(&["add", "a.txt"]);
    repo.git_at(&["commit", "-q", "-m", "one"], Some("2026-01-01T10:00:20Z"));

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "go")
        .write_claim("2026-01-01T10:00:05Z", "2026-01-01T10:00:06Z", "a.txt")
        .write_claim("2026-01-01T10:00:08Z", "2026-01-01T10:00:09Z", "probe.txt")
        .bash_claim(
            "2026-01-01T10:00:12Z",
            "2026-01-01T10:00:13Z",
            "cd /somewhere\nrm -f probe.txt",
        )
        .bash_claim(
            "2026-01-01T10:00:19Z",
            "2026-01-01T10:00:21Z",
            "git add a.txt && git commit -m one",
        );
    let audit = run(&repo, &s);

    let first = &audit.intervals[0];
    let line = first.ledger.iter().find(|l| l.path == "probe.txt").unwrap();
    assert_eq!(line.landing, Landing::Never);
    assert!(
        line.resolution
            .as_deref()
            .unwrap_or("")
            .contains("deliberately"),
        "resolution: {:?}",
        line.resolution
    );
    assert_eq!(first.status(), Status::Green);
}

#[test]
fn gitignored_claim_whose_content_persists_on_disk_is_resolved() {
    let repo = TempRepo::new("persisted");
    let root = repo.root.display().to_string();

    repo.write(".gitignore", "out.txt\n");
    repo.write("a.txt", "x");
    repo.git(&["add", "-A"]);
    repo.git_at(&["commit", "-q", "-m", "one"], Some("2026-01-01T10:00:20Z"));
    // the claimed write persisted on disk with the claimed body; git never
    // saw it (gitignored)
    repo.write("out.txt", &SessionBuilder::default_body("out.txt"));

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "go")
        .write_claim("2026-01-01T10:00:03Z", "2026-01-01T10:00:04Z", ".gitignore")
        .write_claim("2026-01-01T10:00:05Z", "2026-01-01T10:00:06Z", "a.txt")
        .write_claim("2026-01-01T10:00:08Z", "2026-01-01T10:00:09Z", "out.txt")
        .bash_claim(
            "2026-01-01T10:00:19Z",
            "2026-01-01T10:00:21Z",
            "git add .gitignore a.txt && git commit -m one",
        );
    let audit = run(&repo, &s);

    let first = &audit.intervals[0];
    let line = first.ledger.iter().find(|l| l.path == "out.txt").unwrap();
    assert!(
        line.resolution
            .as_deref()
            .unwrap_or("")
            .contains("persisted outside git"),
        "resolution: {:?}",
        line.resolution
    );
    assert_eq!(
        first.status(),
        Status::Green,
        "a kept promise outside git's view is not red or yellow"
    );
}

#[test]
fn a_silently_vanished_file_stays_red() {
    // No later landed edit, no command names it, nothing on disk — the
    // one story that must remain a broken promise.
    let repo = TempRepo::new("vanish");
    let root = repo.root.display().to_string();

    repo.write("a.txt", "x");
    repo.git(&["add", "a.txt"]);
    repo.git_at(&["commit", "-q", "-m", "one"], Some("2026-01-01T10:00:20Z"));

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "go")
        .write_claim("2026-01-01T10:00:05Z", "2026-01-01T10:00:06Z", "a.txt")
        .write_claim("2026-01-01T10:00:08Z", "2026-01-01T10:00:09Z", "ghost.txt")
        .bash_claim(
            "2026-01-01T10:00:19Z",
            "2026-01-01T10:00:21Z",
            "git add a.txt && git commit -m one",
        );
    let audit = run(&repo, &s);

    let first = &audit.intervals[0];
    let line = first.ledger.iter().find(|l| l.path == "ghost.txt").unwrap();
    assert_eq!(line.landing, Landing::Never);
    assert!(line.resolution.is_none(), "nothing explains this one");
    assert_eq!(first.status(), Status::Red);
}

#[test]
fn a_clone_still_audits_via_the_history_spine() {
    // No session-era reflog in a clone — the spine falls back to commit
    // history and the equation still runs.
    let origin = TempRepo::new("cloneorigin");
    let root = origin.root.display().to_string();
    origin.write("a.txt", "x");
    origin.git(&["add", "a.txt"]);
    origin.git_at(&["commit", "-q", "-m", "one"], Some("2026-01-01T10:00:20Z"));

    let clone_path = std::env::temp_dir().join(format!("gitreceipts-clone-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&clone_path);
    assert!(
        std::process::Command::new("git")
            .arg("clone")
            .arg("-q")
            .arg(&origin.root)
            .arg(&clone_path)
            .status()
            .unwrap()
            .success()
    );

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "go")
        .write_claim("2026-01-01T10:00:05Z", "2026-01-01T10:00:06Z", "a.txt")
        .bash_claim(
            "2026-01-01T10:00:19Z",
            "2026-01-01T10:00:21Z",
            "git add a.txt && git commit -m one",
        );
    let path = clone_path.join("session.jsonl");
    s.save(&path);
    let (records, _) = ingest::ingest(&path).unwrap();
    let session = extract::extract(&causal::order(records));
    let audit = reconcile::reconcile(&clone_path, &session).unwrap();

    assert_eq!(audit.intervals.len(), 1, "history spine found the commit");
    assert!(audit.intervals[0].commit.from_history);
    // claims recorded under the ORIGIN path resolve via historical-alias
    // validation against the clone
    assert!(audit.intervals[0].balanced());
    let _ = std::fs::remove_dir_all(&clone_path);
}

#[test]
fn a_commit_created_outside_the_reflog_joins_the_spine_from_history() {
    // Simulates a pulled commit: created via plumbing so HEAD's reflog
    // never sees it, reachable from a branch.
    let repo = TempRepo::new("pulled");
    let root = repo.root.display().to_string();
    repo.write("a.txt", "x");
    repo.git(&["add", "a.txt"]);
    repo.git_at(&["commit", "-q", "-m", "one"], Some("2026-01-01T10:00:20Z"));

    // teammate's commit: build it with commit-tree (no reflog entry)
    repo.write("b.txt", "x");
    repo.git(&["add", "b.txt"]);
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(&repo.root)
        .args(["write-tree"])
        .output()
        .unwrap();
    let tree = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let head = std::process::Command::new("git")
        .arg("-C")
        .arg(&repo.root)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    let head = String::from_utf8_lossy(&head.stdout).trim().to_string();
    let commit = std::process::Command::new("git")
        .arg("-C")
        .arg(&repo.root)
        .args(["commit-tree", &tree, "-p", &head, "-m", "teammate work"])
        .env("GIT_AUTHOR_DATE", "2026-01-01T10:00:40Z")
        .env("GIT_COMMITTER_DATE", "2026-01-01T10:00:40Z")
        .env("GIT_AUTHOR_NAME", "mate")
        .env("GIT_AUTHOR_EMAIL", "m@x")
        .env("GIT_COMMITTER_NAME", "mate")
        .env("GIT_COMMITTER_EMAIL", "m@x")
        .output()
        .unwrap();
    let commit = String::from_utf8_lossy(&commit.stdout).trim().to_string();
    repo.git(&["branch", "teammate", &commit]);

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "go")
        .write_claim("2026-01-01T10:00:05Z", "2026-01-01T10:00:06Z", "a.txt")
        .bash_claim(
            "2026-01-01T10:00:19Z",
            "2026-01-01T10:00:21Z",
            "git add a.txt && git commit -m one",
        );
    let audit = run(&repo, &s);

    assert_eq!(audit.intervals.len(), 2);
    let mate = &audit.intervals[1];
    assert_eq!(mate.commit.subject, "teammate work");
    assert!(
        mate.commit.from_history,
        "known from history, not the reflog"
    );
    assert!(
        !mate.agent_committed,
        "someone else's work shows as an unclaimed keyframe, not this agent's"
    );
}

#[test]
fn a_broken_promise_is_not_resolved_by_a_substring_mention() {
    // Round-2 review finding #2: `config.rs` never lands, and a later
    // command `echo done > config.rs.log` merely CONTAINS the substring
    // "config.rs". Whole-token matching + a real removal verb keep this red.
    let repo = TempRepo::new("substr");
    let root = repo.root.display().to_string();

    repo.write("a.txt", "x");
    repo.git(&["add", "a.txt"]);
    repo.git_at(&["commit", "-q", "-m", "one"], Some("2026-01-01T10:00:20Z"));

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "go")
        .write_claim("2026-01-01T10:00:05Z", "2026-01-01T10:00:06Z", "a.txt")
        // this edit's content never lands anywhere
        .write_claim_content(
            "2026-01-01T10:00:08Z",
            "2026-01-01T10:00:09Z",
            "config.rs",
            "fn never_shipped() { unreachable!() }",
        )
        .bash_claim(
            "2026-01-01T10:00:15Z",
            "2026-01-01T10:00:16Z",
            "echo done > config.rs.log",
        )
        .bash_claim(
            "2026-01-01T10:00:19Z",
            "2026-01-01T10:00:21Z",
            "git add a.txt && git commit -m one",
        );
    let audit = run(&repo, &s);

    let first = &audit.intervals[0];
    let line = first.ledger.iter().find(|l| l.path == "config.rs").unwrap();
    assert_eq!(line.landing, Landing::Never);
    assert!(
        line.resolution.is_none(),
        "a substring mention must not resolve a broken promise: {:?}",
        line.resolution
    );
    assert_eq!(first.status(), Status::Red);
}

#[test]
fn committer_identity_and_co_authors_are_surfaced() {
    // git records who committed and any Co-Authored-By trailer; the audit
    // carries both onto the interval. Identity only — never inferred as
    // agent-vs-hand-coded.
    let repo = TempRepo::new("coauthor");
    let root = repo.root.display().to_string();
    repo.write("a.txt", "x");
    repo.git(&["add", "a.txt"]);
    repo.git_at(
        &[
            "commit",
            "-q",
            "-m",
            "do the thing\n\nCo-Authored-By: Robo Agent <robo@example.invalid>",
        ],
        Some("2026-01-01T10:00:20Z"),
    );

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "go")
        .write_claim("2026-01-01T10:00:05Z", "2026-01-01T10:00:06Z", "a.txt")
        .bash_claim(
            "2026-01-01T10:00:19Z",
            "2026-01-01T10:00:21Z",
            "git add -A && git commit -m 'do the thing'",
        );
    let audit = run(&repo, &s);

    let c = &audit.intervals[0].commit;
    assert!(
        c.author.contains("test"),
        "author identity captured: {}",
        c.author
    );
    assert_eq!(
        c.co_authors,
        vec!["Robo Agent <robo@example.invalid>".to_string()],
        "Co-Authored-By trailer parsed"
    );
}

#[test]
fn multi_author_keyframe_residue_attributes_to_the_committer() {
    // A teammate's commit (a keyframe this session never made) shows its
    // changes as unclaimed residue attributed to the committer git records
    // — not "unexplained", and not this agent.
    let repo = TempRepo::new("multiauthor");
    let root = repo.root.display().to_string();

    // Ada's commit — the audited session made this one
    repo.write("a.txt", "x");
    repo.git(&["add", "-A"]);
    repo.commit_as(
        "Ada Lovelace",
        "ada@studio.dev",
        "2026-01-01T10:00:20+00:00",
        "scaffold",
        Some("Claude <noreply@anthropic.com>"),
    );
    // Bjarne's commit — a teammate on another agent; this session never
    // touched it
    repo.write("b.txt", "y");
    repo.git(&["add", "-A"]);
    repo.commit_as(
        "Bjarne Stroustrup",
        "bjarne@studio.dev",
        "2026-01-01T10:00:40+00:00",
        "add auth",
        Some("Codex <codex@openai.com>"),
    );

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "go")
        .write_claim("2026-01-01T10:00:05Z", "2026-01-01T10:00:06Z", "a.txt")
        .bash_claim(
            "2026-01-01T10:00:19Z",
            "2026-01-01T10:00:21Z",
            "git add -A && git commit -m scaffold",
        );
    let audit = run(&repo, &s);

    assert_eq!(audit.intervals.len(), 2);
    // Ada's commit: this session's work
    assert!(audit.intervals[0].agent_committed);
    assert!(audit.intervals[0].commit.author.contains("Ada"));
    // Bjarne's commit: a keyframe, residue attributed to him by identity
    let bj = &audit.intervals[1];
    assert!(!bj.agent_committed, "teammate commit is a keyframe");
    assert!(bj.commit.author.contains("Bjarne Stroustrup"));
    assert_eq!(
        bj.commit.co_authors,
        vec!["Codex <codex@openai.com>".to_string()]
    );
    assert!(
        bj.residue.iter().any(|c| c.path == "b.txt"),
        "the teammate's file is unclaimed residue"
    );

    // distinct committers + co-authors are both surfaced
    let authors: std::collections::HashSet<&str> = audit
        .intervals
        .iter()
        .map(|i| i.commit.author.as_str())
        .collect();
    assert!(authors.iter().any(|a| a.contains("Ada")));
    assert!(authors.iter().any(|a| a.contains("Bjarne")));
    let coauthors: std::collections::HashSet<&str> = audit
        .intervals
        .iter()
        .flat_map(|i| i.commit.co_authors.iter().map(String::as_str))
        .collect();
    assert!(coauthors.iter().any(|c| c.contains("Claude")));
    assert!(coauthors.iter().any(|c| c.contains("Codex")));
}

#[test]
fn commit_summary_is_the_agent_narration_right_after_the_commit() {
    let repo = TempRepo::new("summary");
    let root = repo.root.display().to_string();

    repo.write("a.txt", &SessionBuilder::default_body("a.txt"));
    repo.git(&["add", "-A"]);
    repo.git_at(
        &["commit", "-q", "-m", "land a"],
        Some("2026-05-01T10:05:00Z"),
    );

    let summary = "Done — committed a.txt with the new module and its tests.";
    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-05-01T10:00:00Z", "make a")
        .write_claim("2026-05-01T10:01:00Z", "2026-05-01T10:01:01Z", "a.txt")
        .bash_claim(
            "2026-05-01T10:04:00Z",
            "2026-05-01T10:05:01Z",
            "git add -A && git commit -m 'land a'",
        )
        // a trivial one-liner (skipped) then the real summary, both post-commit
        .assistant_text("2026-05-01T10:05:05Z", "ok")
        .assistant_text("2026-05-01T10:05:10Z", summary);

    let audit = run(&repo, &s);
    assert_eq!(audit.intervals.len(), 1);
    assert_eq!(
        audit.intervals[0].summary.as_deref(),
        Some(summary),
        "the first substantial narration after the commit is its summary"
    );
}

#[test]
fn a_commit_with_no_narration_after_it_has_no_summary() {
    let repo = TempRepo::new("summary-none");
    let root = repo.root.display().to_string();

    repo.write("a.txt", &SessionBuilder::default_body("a.txt"));
    repo.git(&["add", "-A"]);
    repo.git_at(
        &["commit", "-q", "-m", "land a"],
        Some("2026-05-01T10:05:00Z"),
    );

    // narration is BEFORE the commit — it is not a post-commit summary
    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-05-01T10:00:00Z", "make a")
        .assistant_text(
            "2026-05-01T10:00:30Z",
            "Let me create a.txt and wire it up now.",
        )
        .write_claim("2026-05-01T10:01:00Z", "2026-05-01T10:01:01Z", "a.txt")
        .bash_claim(
            "2026-05-01T10:04:00Z",
            "2026-05-01T10:05:01Z",
            "git add -A && git commit -m 'land a'",
        );

    let audit = run(&repo, &s);
    assert_eq!(audit.intervals[0].summary, None);
}

#[test]
fn mcp_calls_are_counted_as_first_class_actions() {
    let repo = TempRepo::new("mcp-count");
    let root = repo.root.display().to_string();
    repo.write("a.txt", &SessionBuilder::default_body("a.txt"));
    repo.git(&["add", "-A"]);
    repo.git_at(
        &["commit", "-q", "-m", "land a"],
        Some("2026-06-01T10:05:00Z"),
    );

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-06-01T10:00:00Z", "go")
        .mcp_claim(
            "2026-06-01T10:01:00Z",
            "2026-06-01T10:01:01Z",
            "mcp__postgres__query",
            r#"{"sql":"SELECT 1"}"#,
        )
        .write_claim("2026-06-01T10:02:00Z", "2026-06-01T10:02:01Z", "a.txt")
        .bash_claim(
            "2026-06-01T10:04:00Z",
            "2026-06-01T10:05:01Z",
            "git add -A && git commit -m 'land a'",
        );

    let audit = run(&repo, &s);
    assert_eq!(
        audit.mcp_calls, 1,
        "the MCP call is counted, not dropped into observations"
    );
    // retained on its interval with the server + receipt (the execution axis)
    let runs = &audit.intervals[0].mcp_runs;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].server, "postgres");
    assert_eq!(runs[0].tool, "query");
    assert!(!runs[0].errored, "a success result is not errored");
}
