//! Home-directory redaction in the rendered HTML. Its own test binary because
//! it sets the process-global redaction target (`fmt::set_redaction` is a
//! set-once `OnceLock`, like `scan_wiring`), and it uses a SYNTHETIC home so the
//! test is hermetic — same result on any machine, no dependency on the runner's
//! real `$HOME`.

mod common;

use common::{SessionBuilder, TempRepo};
use gitreceipts::report::Expand;
use gitreceipts::{causal, extract, fmt, html, ingest, reconcile};

#[test]
fn command_home_paths_render_as_tilde_in_html() {
    // A command that touches an absolute home path must render with ~, not the
    // raw username, in the shareable HTML.
    let repo = TempRepo::new("redact");
    let root = repo.root.display().to_string();
    repo.write("a.txt", "x");
    repo.git(&["add", "-A"]);
    repo.git_at(&["commit", "-q", "-m", "one"], Some("2026-01-01T10:00:31Z"));

    // A synthetic home, derived from a cwd we control — the redaction target is
    // `/Users/testhome`, independent of the machine this test runs on.
    let fake_home = "/Users/testhome";
    fmt::set_redaction(&[format!("{fake_home}/project")], &[], false);

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "go")
        .write_claim("2026-01-01T10:00:05Z", "2026-01-01T10:00:06Z", "a.txt")
        .bash_claim(
            "2026-01-01T10:00:29Z",
            "2026-01-01T10:00:31Z",
            &format!("touch {fake_home}/marker && git add -A && git commit -m one"),
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
        true,
        true,
        Expand::All,
        false,
        None,
        false,
    );

    assert!(
        !out.contains(fake_home),
        "raw home path must not appear in the report"
    );
    assert!(out.contains('~'), "the home path should collapse to ~");
}
