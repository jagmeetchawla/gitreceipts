//! The secret scanner is wired into the redaction choke point (`fmt::redact_home`)
//! and gated by `set_redaction(.., scan=true)`. This lives in its own test file
//! (own binary) because `set_redaction` is a set-once `OnceLock` — one setter
//! per process keeps it deterministic.

use gitreceipts::fmt;

#[test]
fn scanner_masks_secrets_through_the_redaction_choke_point() {
    // Enable redaction + scanning for this process (no cwds/extra words needed).
    fmt::set_redaction(&[], &[], true);

    // A GitHub token embedded in command text, exactly as it would arrive from a
    // session's Bash call, gets masked to a typed placeholder.
    let out = fmt::redact_home(
        "git remote set-url origin https://ghp_1234567890abcdefghijklmnopqrstuvwxyz@github.com/x/y",
    );
    assert!(out.contains("[redacted:github-token]"), "got: {out}");
    assert!(!out.contains("ghp_1234567890"), "token leaked: {out}");

    // A git SHA in the same stream is left intact — anchored, not entropy.
    let sha = fmt::redact_home("HEAD is 6d6cdc4e8f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c");
    assert!(
        sha.contains("6d6cdc4e8f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c"),
        "sha masked: {sha}"
    );

    // The tally reflects at least the one secret we pushed through.
    let (secrets, _pii) = fmt::scan_counts();
    assert!(
        secrets >= 1,
        "expected the scanner tally to count the token"
    );
}
