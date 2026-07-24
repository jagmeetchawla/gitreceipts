//! Small display helpers shared by the console (`report`) and HTML
//! renderers — path privacy and number abbreviation.

/// Collapse the home directory to `~` at the START of a path. Reports get
/// screenshotted and shared; the OS username does not need to ship with
/// them.
pub fn tilde(path: &str) -> String {
    std::env::home_dir()
        .map(|h| h.display().to_string())
        .and_then(|h| path.strip_prefix(&h).map(|rest| format!("~{rest}")))
        .unwrap_or_else(|| path.to_string())
}

/// Replace the home directory ANYWHERE in a string with `~` — for command
/// text, where absolute paths (and the username) appear mid-line.
pub fn redact_home(s: &str) -> String {
    match std::env::home_dir().map(|h| h.display().to_string()) {
        Some(h) if !h.is_empty() => s.replace(&h, "~"),
        _ => s.to_string(),
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
