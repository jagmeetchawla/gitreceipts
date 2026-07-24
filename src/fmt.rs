//! Small display helpers shared by the console (`report`) and HTML
//! renderers — path privacy and number abbreviation.
//!
//! Home-directory redaction is matched **component-aware** (a home of
//! `/Users/ada` never matches inside `/Users/adabot`) and **ASCII
//! case-insensitively** — macOS's default filesystem is case-insensitive, so
//! a differently-cased path must still be caught. Targeted and vouched for on
//! macOS; the ASCII/`/`-separator assumptions do not claim Windows coverage.

/// The current home directory as a string, if non-empty. Uses
/// `std::env::home_dir()` (un-deprecated and platform-correct since Rust
/// 1.85) rather than a raw `$HOME` read, which is wrong on Windows.
fn home() -> Option<String> {
    std::env::home_dir()
        .map(|h| h.display().to_string())
        .filter(|h| !h.is_empty())
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

/// Replace the home directory ANYWHERE in a string with `~` — for command
/// text, where absolute paths (and the username) appear mid-line.
pub fn redact_home(s: &str) -> String {
    match home() {
        Some(h) => collapse(s, &h),
        None => s.to_string(),
    }
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
    use super::{collapse, collapse_prefix};

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
