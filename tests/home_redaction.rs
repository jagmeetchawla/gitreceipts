//! Home-directory redaction in the rendered HTML. Its own test binary because
//! it sets the process-global redaction target (`fmt::set_redaction` is a
//! set-once `OnceLock`, like `scan_wiring`), and it uses a SYNTHETIC home so the
//! test is hermetic — same result on any machine, no dependency on the runner's
//! real `$HOME`.

mod common;

use common::{SessionBuilder, TempRepo};
use gitreceipts::report::{Expand, Show};
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
        Show {
            prompt: true,
            summary: true,
        },
        true,
        Expand::All,
        false,
        None,
        false,
        false,
        false,
    );

    assert!(
        !out.contains(fake_home),
        "raw home path must not appear in the report"
    );
    assert!(out.contains('~'), "the home path should collapse to ~");
}

#[test]
fn minified_css_keeps_meaningful_whitespace_and_valid_rules() {
    // The stylesheet is minified into every page. `content: "\25a0  "`
    // carries two spaces INSIDE its quotes — a naive whitespace collapse
    // would eat them and shift every marker on the page, silently, in
    // every report ever generated. Pin the string-awareness.
    let repo = TempRepo::new("cssmin");
    let root = repo.root.display().to_string();
    repo.write("a.txt", "x");
    repo.git(&["add", "-A"]);
    repo.git_at(&["commit", "-q", "-m", "one"], Some("2026-01-01T10:00:20Z"));

    let mut s = SessionBuilder::new(&root);
    s.user_text("2026-01-01T10:00:00Z", "go").write_claim(
        "2026-01-01T10:00:05Z",
        "2026-01-01T10:00:06Z",
        "a.txt",
    );
    let path = repo.root.join("session.jsonl");
    s.save(&path);
    let (records, stats) = ingest::ingest(&path).unwrap();
    let session = extract::extract(&causal::order(records));
    let audit = reconcile::reconcile(&repo.root, &session).unwrap();

    let page = html::render(
        "sess",
        &root,
        &session,
        &stats,
        &audit,
        Show {
            prompt: true,
            summary: true,
        },
        true,
        Expand::Auto,
        false,
        None,
        false,
        false,
        false,
    );

    let css = page
        .split("<style>")
        .nth(1)
        .and_then(|s| s.split("</style>").next())
        .expect("the page inlines a stylesheet");

    assert!(
        css.contains(r#"content:"\25a0  ""#),
        "the two spaces inside a content string must survive minification"
    );
    assert!(!css.contains("/*"), "comments are stripped");
    assert!(!css.contains("\n\n"), "blank lines are collapsed");
    assert_eq!(
        css.matches('{').count(),
        css.matches('}').count(),
        "braces stay balanced"
    );
    assert!(
        css.len() < 11_000,
        "minified, not merely inlined: {}",
        css.len()
    );
}
