//! Repo inference for sessions launched outside the repo: container
//! directories holding several repos, claims pointing the way.

mod common;

use common::{SessionBuilder, TempRepo};
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
