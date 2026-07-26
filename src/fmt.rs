//! Small display helpers shared by the console (`report`) and HTML
//! renderers — path privacy and number abbreviation.
//!
//! Home-directory redaction is matched **component-aware** (a home of
//! `/Users/ada` never matches inside `/Users/adabot`) and **ASCII
//! case-insensitively** — macOS's default filesystem is case-insensitive, so
//! a differently-cased path must still be caught. Targeted and vouched for on
//! macOS; the ASCII/`/`-separator assumptions do not claim Windows coverage.
//!
//! The home(s) to redact are **derived from the session log's own `cwd`s**
//! (see [`set_redaction`]), not just the running machine's `$HOME` — so the OS
//! username is masked even when the audit runs on a different machine than
//! recorded the session (a mounted `--store`, a teammate's export). Callers may
//! add further literal words (`--redact`) for names/hosts we can't infer.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The current home directory as a string, if non-empty. Uses
/// `std::env::home_dir()` (un-deprecated and platform-correct since Rust
/// 1.85) rather than a raw `$HOME` read, which is wrong on Windows.
fn home() -> Option<String> {
    std::env::home_dir()
        .map(|h| h.display().to_string())
        .filter(|h| !h.is_empty())
}

/// The redaction target for this run: the home directories to collapse/mask
/// and any extra literal words the user asked to redact. Set once, at load.
struct Redaction {
    /// Full home paths (e.g. `/Users/name`), derived from the session's cwds
    /// plus `$HOME`. Each is collapsed to `~` and its username masked to ****.
    homes: Vec<String>,
    /// Extra literal words to mask (`--redact`) — names, hosts, client ids.
    extra: Vec<String>,
    /// Run the secret/PII scanner ([`crate::scan`]) as a final pass. Default-on
    /// (disabled by `--no-scan`).
    scan: bool,
}

static REDACTION: OnceLock<Redaction> = OnceLock::new();

/// Running tallies of what the secret scanner masked this process, so a report
/// can print an honest "N redacted" line. Approximate (a secret rendered in two
/// places counts twice) — an indicator, not an audit.
static SCAN_SECRETS: AtomicUsize = AtomicUsize::new(0);
static SCAN_PII: AtomicUsize = AtomicUsize::new(0);

/// Configure redaction for this process. `cwds` are the working directories the
/// session recorded; we derive the home(s) it ran under from them — so masking
/// follows the LOG, correct no matter where the audit executes — and add the
/// running machine's `$HOME`. `extra` are additional literal words to mask.
/// `scan` enables the secret/PII scanner pass. First call wins (the CLI calls it
/// once, at load).
pub fn set_redaction(cwds: &[String], extra: &[String], scan: bool) {
    let mut homes: Vec<String> = Vec::new();
    let mut push = |h: String| {
        if !homes.iter().any(|x| x.eq_ignore_ascii_case(&h)) {
            homes.push(h);
        }
    };
    for c in cwds {
        if let Some(h) = home_of(c) {
            push(h);
        }
    }
    if let Some(h) = home() {
        push(h);
    }
    let extra = extra.iter().filter(|w| !w.is_empty()).cloned().collect();
    let _ = REDACTION.set(Redaction { homes, extra, scan });
}

/// How many secrets / PII the scanner has masked so far — for the report's
/// "N redacted" line. Read after rendering (the tallies accrue as strings pass
/// through [`redact_home`]).
pub fn scan_counts() -> (usize, usize) {
    (
        SCAN_SECRETS.load(Ordering::Relaxed),
        SCAN_PII.load(Ordering::Relaxed),
    )
}

/// The `/Users/<name>` or `/home/<name>` home prefix of an absolute path, if it
/// has one. This is the platform shape we redact (macOS vouched, Linux
/// best-effort); other roots fall back to `$HOME`.
fn home_of(path: &str) -> Option<String> {
    let mut it = path.split('/');
    if !it.next()?.is_empty() {
        return None; // must be absolute — the segment before the leading '/' is ""
    }
    let root = it.next()?;
    let user = it.next().filter(|u| !u.is_empty())?;
    matches!(root, "Users" | "home").then(|| format!("/{root}/{user}"))
}

/// A character that can continue a path component. A home-dir match flanked
/// by one is part of a *longer* name (a different user, a `.bak` sibling, or
/// a deeper `/foo/Users/...`), so it is not the home and must be left alone.
fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')
}

/// Does `home` sit at `[start, end)` in `s` on real component boundaries?
/// `s_lower`/`home` are ASCII-lowercased so the match is case-insensitive;
/// byte indices align because ASCII lowercasing is length-preserving.
fn boundary_ok(s: &str, start: usize, end: usize) -> bool {
    let before_ok = start == 0 || !s[..start].chars().next_back().is_some_and(is_name_char);
    let after_ok = !s[end..].chars().next().is_some_and(is_name_char);
    before_ok && after_ok
}

/// Replace every home-dir occurrence in `s` with `~`. Pure core of
/// [`redact_home`], taking `home` explicitly so it can be tested without a
/// real `$HOME`.
fn collapse(s: &str, home: &str) -> String {
    if home.is_empty() {
        return s.to_string();
    }
    let s_lower = s.to_ascii_lowercase();
    let h_lower = home.to_ascii_lowercase();
    let hlen = h_lower.len();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s_lower[i..].starts_with(&h_lower) && boundary_ok(s, i, i + hlen) {
            out.push('~');
            i += hlen;
        } else {
            let ch = s[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// Collapse `home` only when it is a prefix of `path`. Pure core of [`tilde`].
fn collapse_prefix(path: &str, home: &str) -> String {
    if home.is_empty() {
        return path.to_string();
    }
    let p_lower = path.to_ascii_lowercase();
    let h_lower = home.to_ascii_lowercase();
    let hlen = h_lower.len();
    if p_lower.starts_with(&h_lower) && boundary_ok(path, 0, hlen) {
        format!("~{}", &path[hlen..])
    } else {
        path.to_string()
    }
}

/// Collapse the home directory to `~` at the START of a path. Reports get
/// screenshotted and shared; the OS username does not need to ship with
/// them.
pub fn tilde(path: &str) -> String {
    match home() {
        Some(h) => collapse_prefix(path, &h),
        None => path.to_string(),
    }
}

/// Redact private strings ANYWHERE in a line — for command text, output, and
/// MCP receipts, where absolute paths, the username, and dash-encoded
/// project-dir names appear mid-line. For each home: slash paths collapse to
/// `~` (component-aware); then the bare username is masked as a WHOLE WORD
/// wherever it survives — the `ls -l` owner column, `whoami`, `cd`, and the
/// dash-encoded form (`-Users-name-Developer-…` → `-Users-****-Developer-…`).
/// Finally any user-supplied `--redact` words are masked. Uses the redaction
/// target from [`set_redaction`]; before that's called (unit tests), falls back
/// to the running `$HOME`.
pub fn redact_home(s: &str) -> String {
    match REDACTION.get() {
        Some(r) => {
            let mut out = s.to_string();
            for h in &r.homes {
                out = redact_one_home(&out, h);
            }
            for w in &r.extra {
                out = mask_word(&out, w, "****");
            }
            // Final pass: mask machine secrets and validated PII (default-on).
            if r.scan {
                out = scan_pass(&out);
            }
            out
        }
        None => match home() {
            Some(h) => redact_one_home(s, &h),
            None => s.to_string(),
        },
    }
}

/// Run the secret/PII scanner over `s`, tallying what it masks.
fn scan_pass(s: &str) -> String {
    let (redacted, found) = crate::scan::scan_redact(s);
    if !found.is_empty() {
        let (secrets, pii) = crate::scan::counts(&found);
        SCAN_SECRETS.fetch_add(secrets, Ordering::Relaxed);
        SCAN_PII.fetch_add(pii, Ordering::Relaxed);
    }
    redacted
}

/// Collapse one home to `~`, then mask its bare username (`ls` owner, `whoami`,
/// dash-encoded project dirs) to ****.
fn redact_one_home(s: &str, home: &str) -> String {
    let out = collapse(s, home);
    let user = home.rsplit('/').next().unwrap_or(home);
    mask_word(&out, user, "****")
}

/// Replace whole-word occurrences of `word` with `mask`. A match is whole-word
/// when flanked by non-alphanumeric chars (so `-`, `/`, `.`, space all bound it,
/// but `name2`/`namex` don't match). Length-preserving indices aren't needed —
/// we rebuild the string.
fn mask_word(s: &str, word: &str, mask: &str) -> String {
    if word.is_empty() {
        return s.to_string();
    }
    let boundary = |c: Option<char>| c.is_none_or(|c| !c.is_ascii_alphanumeric());
    let wl = word.len();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i..].starts_with(word)
            && boundary(s[..i].chars().next_back())
            && boundary(s[i + wl..].chars().next())
        {
            out.push_str(mask);
            i += wl;
        } else {
            let ch = s[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// A one-line display summary of a shell command: the first line that
/// actually does something — skipping `cd`, comments, and the blank lines a
/// scripted Bash call opens with — truncated to fit a report row. The full
/// command is preserved in the data model (and the JSON receipt); this is
/// only for the console/HTML views.
pub fn command_summary(command: &str) -> String {
    command
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#') && *l != "cd" && !l.starts_with("cd "))
        .or_else(|| command.lines().map(str::trim).find(|l| !l.is_empty()))
        .unwrap_or("")
        .chars()
        .take(140)
        .collect()
}

/// Abbreviate a large count: 4_997_300 → "5.0M", 125_368 → "125K".
pub fn abbrev(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.0}K", n as f64 / 1_000.0),
        1_000_000..=999_999_999 => format!("{:.1}M", n as f64 / 1_000_000.0),
        _ => format!("{:.1}B", n as f64 / 1_000_000_000.0),
    }
}

#[cfg(test)]
mod tests {
    use super::{collapse, collapse_prefix, home_of, mask_word};

    // A realistic macOS home; tests pass it explicitly so they don't depend
    // on the machine's actual $HOME.
    const H: &str = "/Users/jagmeetchawla";

    #[test]
    fn redacts_home_anywhere_in_a_command() {
        assert_eq!(
            collapse("git -C /Users/jagmeetchawla/repo rev-parse", H),
            "git -C ~/repo rev-parse"
        );
    }

    #[test]
    fn exact_home_and_home_with_subpath_collapse() {
        assert_eq!(collapse(H, H), "~");
        assert_eq!(collapse("/Users/jagmeetchawla/x", H), "~/x");
    }

    #[test]
    fn a_longer_username_is_not_a_match() {
        // the component-boundary bug: a naive replace would rewrite these.
        assert_eq!(
            collapse("/Users/jagmeetchawla2/x", H),
            "/Users/jagmeetchawla2/x"
        );
        assert_eq!(
            collapse("/Users/jagmeetchawla.bak/x", H),
            "/Users/jagmeetchawla.bak/x"
        );
    }

    #[test]
    fn a_deeper_path_that_merely_contains_the_name_is_not_home() {
        assert_eq!(
            collapse("/mnt/backup/Users/jagmeetchawla/x", H),
            "/mnt/backup/Users/jagmeetchawla/x"
        );
    }

    #[test]
    fn matching_is_case_insensitive_for_apfs() {
        assert_eq!(collapse("/users/JagMeetChawla/notes.md", H), "~/notes.md");
    }

    #[test]
    fn redacts_inside_quotes_and_at_string_boundaries() {
        assert_eq!(
            collapse("@\"/Users/jagmeetchawla/Downloads/x.md\"", H),
            "@\"~/Downloads/x.md\""
        );
    }

    #[test]
    fn tilde_only_collapses_a_prefix() {
        assert_eq!(collapse_prefix("/Users/jagmeetchawla/p", H), "~/p");
        // home mid-string is not a prefix — left untouched.
        assert_eq!(
            collapse_prefix("see /Users/jagmeetchawla/p", H),
            "see /Users/jagmeetchawla/p"
        );
    }

    #[test]
    fn home_is_derived_from_a_recorded_cwd() {
        // The signal that makes redaction correct off-machine: the log's cwd.
        assert_eq!(
            home_of("/Users/otheruser/Developer/Projects/app"),
            Some("/Users/otheruser".to_string())
        );
        assert_eq!(home_of("/home/ada/src"), Some("/home/ada".to_string()));
        // The home path itself, and non-home roots.
        assert_eq!(home_of("/Users/ada"), Some("/Users/ada".to_string()));
        assert_eq!(home_of("/tmp/scratch"), None);
        assert_eq!(home_of("/Users"), None);
        assert_eq!(home_of("relative/path"), None);
    }

    #[test]
    fn mask_word_hits_bare_username_forms_but_not_substrings() {
        // ls -l owner column, dash-encoded project dir, cd, whoami — all bare.
        assert_eq!(
            mask_word("drwxr-xr-x 5 ada staff 160", "ada", "****"),
            "drwxr-xr-x 5 **** staff 160"
        );
        assert_eq!(
            mask_word("-Users-ada-Developer-app", "ada", "****"),
            "-Users-****-Developer-app"
        );
        assert_eq!(
            mask_word("cd /Users/ada && ls", "ada", "****"),
            "cd /Users/**** && ls"
        );
        // Not a substring of a longer token.
        assert_eq!(
            mask_word("adabot and adams", "ada", "****"),
            "adabot and adams"
        );
    }

    #[test]
    fn mask_word_masks_user_supplied_words() {
        assert_eq!(
            mask_word(
                "deploying to staging.internal now",
                "staging.internal",
                "****"
            ),
            "deploying to **** now"
        );
    }

    #[test]
    fn empty_home_is_a_no_op() {
        assert_eq!(
            collapse("/Users/jagmeetchawla/x", ""),
            "/Users/jagmeetchawla/x"
        );
        assert_eq!(
            collapse_prefix("/Users/jagmeetchawla/x", ""),
            "/Users/jagmeetchawla/x"
        );
    }
}
