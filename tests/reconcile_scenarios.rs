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

#[test]
fn claim_swept_into_a_much_later_commit_is_found_and_verified() {
    // The claimed file skips the next commit entirely and lands two
    // commits later — the forward sweep finds it, verifies content, and
    // strikes the residue where it landed.
    let repo = TempRepo::new("latesweep");
    let root = repo.root.display().to_string();

    repo.write("a.txt", "x");
    repo.git(&["add", "a.txt"]);
    repo.git_at(&["commit", "-q", "-m", "one"], Some("2026-01-01T10:00:20Z"));
    repo.write("c.txt", "x");
    repo.git(&["add", "c.txt"]);
    repo.git_at(&["commit", "-q", "-m", "two"], Some("2026-01-01T10:00:40Z"));
    repo.write("b.txt", "x");
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
    assert_eq!(line.late_verified, Some(true));
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
fn residue_named_only_in_command_output_attributes_with_the_weaker_reason() {
    // A sed loop's command text never names its targets — but its captured
    // output lists them. That still explains the change, with the reason
    // saying the evidence came from output, not the command itself.
    let repo = TempRepo::new("outattr");
    let root = repo.root.display().to_string();

    repo.write("docs/notes.md", "old token here");
    repo.git(&["add", "-A"]);
    repo.git_at(
        &["commit", "-q", "-m", "base"],
        Some("2026-01-01T10:00:20Z"),
    );
    repo.write("docs/notes.md", "new token here");
    repo.git(&["add", "-A"]);
    repo.git_at(
        &["commit", "-q", "-m", "sweep"],
        Some("2026-01-01T10:00:40Z"),
    );

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "go")
        .write_claim("2026-01-01T10:00:05Z", "2026-01-01T10:00:06Z", "docs/notes.md")
        .bash_claim(
            "2026-01-01T10:00:19Z",
            "2026-01-01T10:00:21Z",
            "git add -A && git commit -m base",
        )
        .bash_claim_with_output(
            "2026-01-01T10:00:30Z",
            "2026-01-01T10:00:41Z",
            "for f in $(grep -rl token docs); do sed -i '' s/old/new/ $f; done && git commit -am sweep",
            "rewrote: docs/notes.md",
        );
    let audit = run(&repo, &s);

    let sweep = &audit.intervals[1];
    assert!(sweep.residue.is_empty());
    let (_, why) = sweep
        .attributed_residue
        .iter()
        .find(|(c, _)| c.path == "docs/notes.md")
        .unwrap();
    assert!(why.contains("output"), "reason was: {why}");
    assert_eq!(sweep.status(), Status::Green);
}

#[test]
fn intermediate_edit_superseded_by_later_landed_edit_is_not_red() {
    // The agent edits a file, doesn't commit that state, edits it again
    // later and commits. The first edit is an intermediate state in a
    // kept promise chain — resolved, not red.
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
    assert_eq!(line.landing, Landing::Never);
    assert!(
        line.resolution
            .as_deref()
            .unwrap_or("")
            .contains("superseded"),
        "resolution: {:?}",
        line.resolution
    );
    assert_eq!(
        first.status(),
        Status::Green,
        "an intermediate state is not a broken promise"
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
    // the claimed write persisted on disk; git never saw it
    repo.write("out.txt", "x");

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
