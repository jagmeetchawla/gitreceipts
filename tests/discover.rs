//! Repo inference for sessions launched outside the repo: container
//! directories holding several repos, claims pointing the way.

mod common;

use common::{SessionBuilder, TempRepo};
use gitreceipts::reconcile;
use gitreceipts::{causal, discover, extract, ingest};

/// A container directory holding two child repos; the session was
/// launched in the container.
fn container_with_two_repos(name: &str) -> (TempRepo, String, String) {
    let container = TempRepo::new(name); // .git at the top is fine; children matter
    std::fs::remove_dir_all(container.root.join(".git")).unwrap();
    let a = container.root.join("app");
    let b = container.root.join("notes");
    for r in [&a, &b] {
        std::fs::create_dir_all(r).unwrap();
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(r)
                .args(["init", "-q"])
                .status()
                .unwrap()
                .success()
        );
    }
    (container, a.display().to_string(), b.display().to_string())
}

fn session_from(builder: &SessionBuilder, dir: &std::path::Path) -> gitreceipts::extract::Session {
    let path = dir.join("session.jsonl");
    builder.save(&path);
    let (records, _) = ingest::ingest(&path).unwrap();
    extract::extract(&causal::order(records))
}

#[test]
fn claims_pick_the_repo_out_of_a_container() {
    let (container, app, _notes) = container_with_two_repos("container");
    let root = container.root.display().to_string();

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "go")
        .write_claim_abs(
            "2026-01-01T10:00:05Z",
            "2026-01-01T10:00:06Z",
            &format!("{app}/src/main.rs"),
        )
        .write_claim_abs(
            "2026-01-01T10:00:07Z",
            "2026-01-01T10:00:08Z",
            &format!("{app}/Cargo.toml"),
        );
    let session = session_from(&s, &container.root);

    let inferred = discover::infer_repo(&session).unwrap();
    assert_eq!(
        inferred,
        std::path::PathBuf::from(&app).canonicalize().unwrap()
    );
}

#[test]
fn ambiguous_containers_refuse_to_guess() {
    let (container, app, notes) = container_with_two_repos("ambiguous");
    let root = container.root.display().to_string();

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "go")
        .write_claim_abs(
            "2026-01-01T10:00:05Z",
            "2026-01-01T10:00:06Z",
            &format!("{app}/a.txt"),
        )
        .write_claim_abs(
            "2026-01-01T10:00:07Z",
            "2026-01-01T10:00:08Z",
            &format!("{notes}/b.txt"),
        );
    let session = session_from(&s, &container.root);

    let err = discover::infer_repo(&session).unwrap_err().to_string();
    assert!(err.contains("pass --repo"), "error was: {err}");
    assert!(err.contains("candidates"), "error names them: {err}");
}

#[test]
fn a_single_candidate_needs_no_claims() {
    let repo = TempRepo::new("solo");
    let root = repo.root.display().to_string();
    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "hello");
    let session = session_from(&s, &repo.root);

    let inferred = discover::infer_repo(&session).unwrap();
    assert_eq!(inferred, repo.root);
}

#[test]
fn forked_sessions_do_not_double_count_claims() {
    // Two files carrying the same events (a fork's shared prefix) merge
    // to one copy of everything.
    let repo = TempRepo::new("fork");
    let root = repo.root.display().to_string();
    repo.write("a.txt", "x");
    repo.git(&["add", "a.txt"]);
    repo.git_at(&["commit", "-q", "-m", "one"], Some("2026-01-01T10:00:20Z"));

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "go")
        .write_claim("2026-01-01T10:00:05Z", "2026-01-01T10:00:06Z", "a.txt")
        .bash_claim(
            "2026-01-01T10:00:19Z",
            "2026-01-01T10:00:21Z",
            "git add a.txt && git commit -m one",
        );
    let f1 = repo.root.join("s1.jsonl");
    let f2 = repo.root.join("s2.jsonl");
    s.save(&f1);
    s.save(&f2); // identical fork

    let (records, stats) = discover::merge_sessions(&[f1, f2]).unwrap();
    assert_eq!(
        stats.duplicates, 5,
        "every event of the fork is a duplicate"
    );
    let session = extract::extract(&records);
    assert_eq!(session.claims.len(), 2, "one write + one command, not four");

    let audit = reconcile::reconcile(&repo.root, &session).unwrap();
    assert_eq!(audit.intervals[0].ledger.len(), 1);
    assert_eq!(audit.intervals[0].ledger[0].edits, 1, "not double-counted");
}

#[test]
fn distinct_sessions_merge_into_one_ledger() {
    // Session A makes commit one, session B makes commit two. Audited
    // separately, each sees the other's commit as an unclaimed keyframe;
    // merged, both intervals are agent-committed and both claims land.
    let repo = TempRepo::new("union");
    let root = repo.root.display().to_string();
    repo.write("a.txt", "x");
    repo.git(&["add", "a.txt"]);
    repo.git_at(&["commit", "-q", "-m", "one"], Some("2026-01-01T10:00:20Z"));
    repo.write("b.txt", "x");
    repo.git(&["add", "b.txt"]);
    repo.git_at(&["commit", "-q", "-m", "two"], Some("2026-01-01T10:00:40Z"));

    let mut sa = SessionBuilder::with_id(&root, "A-");
    sa.user_text("2026-01-01T10:00:00Z", "go")
        .write_claim("2026-01-01T10:00:05Z", "2026-01-01T10:00:06Z", "a.txt")
        .bash_claim(
            "2026-01-01T10:00:19Z",
            "2026-01-01T10:00:21Z",
            "git add a.txt && git commit -m one",
        );
    let mut sb = SessionBuilder::with_id(&root, "B-");
    sb.user_text("2026-01-01T10:00:25Z", "more")
        .write_claim("2026-01-01T10:00:30Z", "2026-01-01T10:00:31Z", "b.txt")
        .bash_claim(
            "2026-01-01T10:00:39Z",
            "2026-01-01T10:00:41Z",
            "git add b.txt && git commit -m two",
        );
    let fa = repo.root.join("a.jsonl");
    let fb = repo.root.join("b.jsonl");
    sa.save(&fa);
    sb.save(&fb);

    let (records, stats) = discover::merge_sessions(&[fb.clone(), fa.clone()]).unwrap();
    assert_eq!(stats.duplicates, 0);
    let session = extract::extract(&records);
    let audit = reconcile::reconcile(&repo.root, &session).unwrap();

    assert_eq!(audit.intervals.len(), 2);
    assert!(audit.intervals[0].agent_committed, "session A's commit");
    assert!(audit.intervals[1].agent_committed, "session B's commit");
    assert!(audit.intervals.iter().all(|i| i.balanced()));
}

/// A fake session store — the Claude Code projects directory itself (session
/// project dirs go directly under it; there is no extra `projects/` layer).
fn fake_store(name: &str) -> std::path::PathBuf {
    let root =
        std::env::temp_dir().join(format!("gitreceipts-store-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn store_lookup_matches_exact_ancestor_encodings() {
    let store = fake_store("exact");
    // a repo at <tmp>/…/work/myapp, session recorded at the parent "work"
    let work = std::env::temp_dir()
        .join(format!("gitreceipts-anchor-{}", std::process::id()))
        .join("work");
    let repo = work.join("myapp");
    std::fs::create_dir_all(&repo).unwrap();
    let canon = work.canonicalize().unwrap();
    let encoded: String = canon
        .display()
        .to_string()
        .chars()
        .map(|c| if c == '/' || c == '.' { '-' } else { c })
        .collect();
    let project_dir = store.join(encoded);
    std::fs::create_dir_all(&project_dir).unwrap();
    std::fs::write(project_dir.join("s1.jsonl"), "{}").unwrap();

    let found = discover::latest_session(&store, &repo).unwrap();
    assert_eq!(found.file_name().unwrap(), "s1.jsonl");
    let _ = std::fs::remove_dir_all(&store);
}

#[test]
fn mounted_store_falls_back_to_name_suffix_match() {
    // The store was recorded on another machine: its encoded dir carries
    // that machine's absolute path. Our local mount path shares only the
    // trailing directory name.
    let store = fake_store("mounted");
    let foreign = store.join("-Users-someone-else-Developer-myapp");
    std::fs::create_dir_all(&foreign).unwrap();
    std::fs::write(foreign.join("remote.jsonl"), "{}").unwrap();

    let local_mount = std::env::temp_dir()
        .join(format!("gitreceipts-mount-{}", std::process::id()))
        .join("myapp");
    std::fs::create_dir_all(&local_mount).unwrap();

    let found = discover::latest_session(&store, &local_mount).unwrap();
    assert_eq!(found.file_name().unwrap(), "remote.jsonl");
    let _ = std::fs::remove_dir_all(&store);
}
