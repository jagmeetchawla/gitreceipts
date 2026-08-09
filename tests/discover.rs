//! Repo inference for sessions launched outside the repo: container
//! directories holding several repos, claims pointing the way.

mod common;

use common::{SessionBuilder, TempRepo};
use gitreceipts::reconcile;
use gitreceipts::{discover, extract};

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

#[test]
fn a_folder_holding_repos_refuses_to_pick_one() {
    // The 0.1.1 rule: the tool never decides which repo you meant. A
    // container of repos is an error naming both ways forward — it used to
    // silently audit whichever repo had the most file claims, which reads
    // as a complete report about a quarter of the project.
    let (container, _app, _notes) = container_with_two_repos("container");

    let err = discover::resolve_repo(&container.root)
        .unwrap_err()
        .to_string();
    assert!(err.contains("holds 2 repos"), "error was: {err}");
    assert!(
        err.contains("--repo"),
        "names the single-repo switch: {err}"
    );
    assert!(
        err.contains("--project"),
        "names the all-repos switch: {err}"
    );
}

#[test]
fn a_folder_with_one_repo_below_still_asks() {
    // Not even a lone candidate is chosen for you: `.git` is here or the
    // tool asks. A folder with one repo today can hold two tomorrow, and
    // the answer should not change shape when it does.
    let container = TempRepo::new("one-below");
    std::fs::remove_dir_all(container.root.join(".git")).unwrap();
    let inner = container.root.join("app");
    std::fs::create_dir_all(&inner).unwrap();
    assert!(
        std::process::Command::new("git")
            .arg("-C")
            .arg(&inner)
            .args(["init", "-q"])
            .status()
            .unwrap()
            .success()
    );

    let err = discover::resolve_repo(&container.root)
        .unwrap_err()
        .to_string();
    assert!(err.contains("is not a git repo"), "error was: {err}");
    assert!(err.contains("1 repo"), "counts what it found: {err}");
    assert!(err.contains("app"), "names it: {err}");
}

#[test]
fn a_repo_resolves_to_itself_and_never_upward() {
    // No ancestor walk: a subdirectory of a repo is not the repo. The rule
    // is "this folder, or one below" — one direction only.
    let repo = TempRepo::new("selfres");
    assert_eq!(discover::resolve_repo(&repo.root).unwrap(), repo.root);

    let sub = repo.root.join("src/deep");
    std::fs::create_dir_all(&sub).unwrap();
    let err = discover::resolve_repo(&sub).unwrap_err().to_string();
    assert!(err.contains("is not a git repo"), "error was: {err}");
    assert!(
        err.contains("Run this from a git repo"),
        "says the rule: {err}"
    );
}

#[test]
fn a_missing_directory_says_so() {
    let err = discover::resolve_repo(std::path::Path::new("/tmp/gitreceipts-does-not-exist-xyz"))
        .unwrap_err()
        .to_string();
    assert!(err.contains("no such directory"), "error was: {err}");
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

/// Dash-encode a path the way the store does, and create that project dir with
/// one session file so `session_dirs_for` finds it.
fn seed_session(store: &std::path::Path, repo: &std::path::Path) {
    let canon = repo.canonicalize().unwrap();
    let encoded: String = canon
        .display()
        .to_string()
        .chars()
        .map(|c| if c == '/' || c == '.' { '-' } else { c })
        .collect();
    let dir = store.join(encoded);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("s.jsonl"), "{}").unwrap();
}

fn git_init(dir: &std::path::Path) {
    std::fs::create_dir_all(dir).unwrap();
    assert!(
        std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["init", "-q"])
            .status()
            .unwrap()
            .success()
    );
}

#[test]
fn project_repos_finds_only_repos_with_sessions_and_skips_nested() {
    let store = fake_store("proj");
    let project = std::env::temp_dir().join(format!("gitreceipts-proj-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&project);
    let app = project.join("app");
    let notes = project.join("notes");
    let plain = project.join("docs"); // a dir, not a git repo
    git_init(&app);
    git_init(&app.join("vendor")); // nested repo — must NOT be descended into
    git_init(&notes);
    std::fs::create_dir_all(&plain).unwrap();

    // Only `app` and `notes` have sessions in the store; `vendor` does not.
    seed_session(&store, &app);
    seed_session(&store, &notes);

    let repos = discover::project_repos(&project, &store);
    let names: Vec<String> = repos
        .iter()
        .map(|r| r.file_name().unwrap().to_str().unwrap().to_string())
        .collect();
    assert!(names.contains(&"app".to_string()), "app has sessions");
    assert!(names.contains(&"notes".to_string()), "notes has sessions");
    assert!(
        !names.contains(&"vendor".to_string()),
        "a nested repo is not descended into"
    );
    assert!(
        !names.contains(&"docs".to_string()),
        "docs is not a git repo"
    );
    assert_eq!(repos.len(), 2, "exactly the two repos with sessions");
    let _ = std::fs::remove_dir_all(&store);
    let _ = std::fs::remove_dir_all(&project);
}

#[test]
fn project_that_is_itself_a_repo_yields_just_itself() {
    // Monorepo case: the project folder has a .git at its root. It is the one
    // repo — project ≡ repo — and discovery does not descend past it.
    let store = fake_store("mono");
    let project = std::env::temp_dir().join(format!("gitreceipts-mono-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&project);
    git_init(&project);
    git_init(&project.join("packages").join("inner")); // must be ignored
    seed_session(&store, &project);

    let repos = discover::project_repos(&project, &store);
    assert_eq!(repos.len(), 1, "just the project root");
    assert_eq!(
        repos[0].canonicalize().unwrap(),
        project.canonicalize().unwrap()
    );
    let _ = std::fs::remove_dir_all(&store);
    let _ = std::fs::remove_dir_all(&project);
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
