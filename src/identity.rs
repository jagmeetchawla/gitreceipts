//! Whose commits are yours.
//!
//! The commit spine is built from the session's TIME WINDOW, so in a repo
//! with more than one contributor it contains other people's commits. Time
//! alone cannot tell them apart, and the consequence is not noise but a
//! false accusation: a colleague's commit inside the window used to be
//! stamped as the agent's, which pulled its unexplained files in as YOUR
//! residue and its interval into YOUR verdict.
//!
//! Identity is the missing gate. A commit is yours when git's own record
//! says so — matched on the AUTHOR or the COMMITTER, by NAME or by EMAIL.
//!
//! The looseness is deliberate and the asymmetry is the reason. Matching
//! too loosely costs you a stray commit inside your own audit. Matching too
//! tightly silently drops your own work — and a rebase, cherry-pick,
//! squash-merge or a commit made through a forge's web UI all keep you as
//! author while making someone else the committer. Losing your own commits
//! is the worse failure, so either side and either field counts.

use std::path::Path;

/// The identities that count as "you" in one repo.
#[derive(Debug, Clone, Default)]
pub struct Identity {
    names: Vec<String>,
    emails: Vec<String>,
    /// What we'll show the user when nothing matches.
    pub described: String,
    /// Did git (or the user) actually give us an identity? An unset
    /// `user.email` must never be read as "everything is mine" — with no
    /// identity we cannot answer the question at all, and saying so is the
    /// only honest move.
    pub known: bool,
}

/// Split git's "Name <email>" into its two halves, both lowercased.
fn split_ident(s: &str) -> (String, String) {
    match (s.rfind('<'), s.rfind('>')) {
        (Some(a), Some(b)) if b > a => (
            s[..a].trim().to_lowercase(),
            s[a + 1..b].trim().to_lowercase(),
        ),
        _ => (s.trim().to_lowercase(), String::new()),
    }
}

impl Identity {
    /// Read `user.name` / `user.email` as git itself resolves them for this
    /// repo — local config, then global, then system. `extra` adds
    /// identities the user named explicitly; each may be a name, an email,
    /// or a full "Name <email>".
    pub fn resolve(repo: &Path, extra: &[String]) -> Self {
        let cfg = |key: &str| -> Option<String> {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(["config", "--get", key])
                .output()
                .ok()?;
            let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
            (out.status.success() && !v.is_empty()).then_some(v)
        };

        let mut names = Vec::new();
        let mut emails = Vec::new();
        let mut shown = Vec::new();

        if let Some(n) = cfg("user.name") {
            shown.push(n.clone());
            names.push(n.to_lowercase());
        }
        if let Some(e) = cfg("user.email") {
            shown.push(format!("<{e}>"));
            emails.push(e.to_lowercase());
        }
        for raw in extra {
            shown.push(raw.clone());
            let (n, e) = split_ident(raw);
            // A bare token with no angle brackets could be either; an "@"
            // is the only reliable tell, and counting it as both would let
            // a name accidentally match an address.
            if e.is_empty() {
                if n.contains('@') {
                    emails.push(n);
                } else if !n.is_empty() {
                    names.push(n);
                }
            } else {
                if !n.is_empty() {
                    names.push(n);
                }
                emails.push(e);
            }
        }

        let known = !names.is_empty() || !emails.is_empty();
        Identity {
            names,
            emails,
            described: if shown.is_empty() {
                "(git user.name and user.email are both unset)".to_string()
            } else {
                shown.join(" ")
            },
            known,
        }
    }

    /// Build from explicit values — for tests and for callers that already
    /// know who they are.
    pub fn from_parts(names: &[&str], emails: &[&str]) -> Self {
        Identity {
            names: names.iter().map(|s| s.to_lowercase()).collect(),
            emails: emails.iter().map(|s| s.to_lowercase()).collect(),
            described: names
                .iter()
                .map(|s| (*s).to_string())
                .chain(emails.iter().map(|e| format!("<{e}>")))
                .collect::<Vec<_>>()
                .join(" "),
            known: !names.is_empty() || !emails.is_empty(),
        }
    }

    /// Is this commit yours? `author` and `committer` are git's
    /// "Name <email>" strings.
    ///
    /// With no identity known this answers `true` for everything: we cannot
    /// tell whose commits these are, so filtering would be a guess dressed
    /// as a fact. The caller says so out loud instead — see [`Self::known`].
    pub fn owns(&self, author: &str, committer: &str) -> bool {
        if !self.known {
            return true;
        }
        for side in [author, committer] {
            let (n, e) = split_ident(side);
            if !e.is_empty() && self.emails.contains(&e) {
                return true;
            }
            if !n.is_empty() && self.names.contains(&n) {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me() -> Identity {
        Identity::from_parts(&["Ada Lovelace"], &["ada@example.com"])
    }

    #[test]
    fn matches_on_either_side_and_either_field() {
        let i = me();
        // plain: both sides are you
        assert!(i.owns(
            "Ada Lovelace <ada@example.com>",
            "Ada Lovelace <ada@example.com>"
        ));
        // squash-merge: you authored, the forge committed
        assert!(i.owns(
            "Ada Lovelace <ada@example.com>",
            "GitHub <noreply@github.com>"
        ));
        // rebase by you of someone else's patch: they authored, you committed
        assert!(i.owns("Bob <bob@example.com>", "Ada Lovelace <ada@example.com>"));
        // email differs (work laptop) but the name is the same
        assert!(i.owns(
            "Ada Lovelace <ada@corp.example>",
            "Ada Lovelace <ada@corp.example>"
        ));
        // name differs but the email is the same
        assert!(i.owns(
            "a.lovelace <ada@example.com>",
            "a.lovelace <ada@example.com>"
        ));
    }

    #[test]
    fn a_colleagues_commit_is_not_yours() {
        let i = me();
        assert!(!i.owns("Bob <bob@example.com>", "Bob <bob@example.com>"));
        assert!(!i.owns("GitHub <noreply@github.com>", "GitHub <noreply@github.com>"));
    }

    #[test]
    fn matching_is_case_insensitive() {
        let i = me();
        assert!(i.owns("ADA LOVELACE <ADA@EXAMPLE.COM>", "x <y@z>"));
    }

    #[test]
    fn no_identity_owns_everything_but_says_it_does_not_know() {
        let i = Identity::from_parts(&[], &[]);
        assert!(!i.known, "an unset identity must not claim to know");
        assert!(
            i.owns("Bob <bob@example.com>", "Bob <bob@example.com>"),
            "with nothing to match on, filtering would be a guess — own everything and say so"
        );
    }

    #[test]
    fn extra_identities_split_names_from_emails() {
        let i = Identity {
            ..Identity::from_parts(&[], &[])
        };
        assert!(!i.known);
        // A bare address is an email, a bare word is a name.
        let e = Identity::from_parts(&[], &["other@example.com"]);
        assert!(e.owns("Someone <other@example.com>", "x <y@z>"));
        assert!(!e.owns("other@example.com <nope@example.com>", "x <y@z>"));
    }
}
