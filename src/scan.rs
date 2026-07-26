//! Secret & PII detection for redaction — **anchored regex only, no entropy**.
//!
//! Self-contained: depends only on `regex` + std, so this file can be lifted
//! into another project wholesale. The public surface is small:
//!
//! - [`scan_redact`] — the workhorse: returns the redacted text *and* what was
//!   found (spans + class), so a caller can both mask and report a count.
//! - [`redact`] / [`scan`] — convenience wrappers when you want only one half.
//!
//! Design, distilled from five open-source scanners (gitleaks, Kingfisher,
//! Nosey Parker, ripsecrets, Velka): every rule anchors on a real token
//! *grammar* (a fixed prefix / structure), never on entropy — so it fires on
//! `ghp_…` and stays silent on a git SHA.
//! Secrets pass a cheap distinct-character guard (kills `AKIAXXXX…`
//! placeholders); PII passes a deterministic **checksum** (Luhn / IBAN mod-97 /
//! SSN structural) — the checksum is load-bearing, the loose regex alone is not.
//! Redaction is tuned for an *audit report*: over-masking is the safe error, but
//! typed placeholders (`[redacted:aws-key]`) keep the report auditable.
//!
//! Attribution: patterns are published token grammars (facts); validators are
//! reimplemented from description. Credit gitleaks (MIT), Kingfisher (Apache-2.0),
//! Nosey Parker (Apache-2.0), ripsecrets (MIT), Velka (MIT/Apache-2.0).

use std::sync::OnceLock;

use regex::{Regex, RegexSet};

/// One redacted span: the class of secret and its byte range in the input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// A stable class label, e.g. `aws-key`, `github-token`, `pii-ssn`. PII
    /// labels are prefixed `pii-` so callers can split the two (see
    /// [`Finding::is_pii`]).
    pub kind: &'static str,
    pub start: usize,
    pub end: usize,
}

impl Finding {
    /// Is this a PII finding (vs. a machine secret)?
    pub fn is_pii(&self) -> bool {
        self.kind.starts_with("pii-")
    }
}

/// Scan `text`, replace every finding with `[redacted:<kind>]`, and return the
/// redacted string alongside the (non-overlapping) findings. This is the
/// portable entry point.
pub fn scan_redact(text: &str) -> (String, Vec<Finding>) {
    let found = scan(text);
    if found.is_empty() {
        return (text.to_string(), found);
    }
    let mut out = String::with_capacity(text.len());
    let mut idx = 0;
    for f in &found {
        out.push_str(&text[idx..f.start]);
        out.push_str("[redacted:");
        out.push_str(f.kind);
        out.push(']');
        idx = f.end;
    }
    out.push_str(&text[idx..]);
    (out, found)
}

/// Redact `text`, discarding the findings (convenience over [`scan_redact`]).
pub fn redact(text: &str) -> String {
    scan_redact(text).0
}

/// Detect secrets/PII in `text` without modifying it. Findings are sorted by
/// position and guaranteed non-overlapping (when two rules hit the same region,
/// the leftmost-longest wins).
pub fn scan(text: &str) -> Vec<Finding> {
    let rs = ruleset();
    let mut hits: Vec<Finding> = Vec::new();
    // RegexSet is a cheap single-pass prefilter: only run the capturing scan
    // for rules that could match at all.
    for i in rs.set.matches(text).iter() {
        let rule = &rs.rules[i];
        for caps in rule.re.captures_iter(text) {
            // Every rule puts the secret itself in capture group 1; surrounding
            // context (keyword, assignment op, boundary) sits outside it.
            if let Some(m) = caps.get(1)
                && rule.guard.accept(m.as_str())
            {
                hits.push(Finding {
                    kind: rule.kind,
                    start: m.start(),
                    end: m.end(),
                });
            }
        }
    }
    // Resolve overlaps: sort by start, longer first, then greedily keep
    // non-overlapping spans (interval scheduling, leftmost-longest).
    hits.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
    let mut chosen: Vec<Finding> = Vec::new();
    let mut last_end = 0;
    for h in hits {
        if h.start >= last_end {
            last_end = h.end;
            chosen.push(h);
        }
    }
    chosen
}

/// Count findings as `(secrets, pii)`.
pub fn counts(findings: &[Finding]) -> (usize, usize) {
    let pii = findings.iter().filter(|f| f.is_pii()).count();
    (findings.len() - pii, pii)
}

// ---------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------

/// How a candidate is confirmed after its pattern matches.
#[derive(Clone, Copy)]
enum Guard {
    /// The format/anchor alone is trusted (strong prefix, structural token).
    Format,
    /// The captured token must contain at least N distinct bytes — the cheap,
    /// deterministic placeholder filter (ripsecrets' idea, sans statistics).
    Distinct(usize),
    /// PII: Luhn mod-10 (credit cards).
    Luhn,
    /// PII: ISO-13616 IBAN mod-97.
    Iban,
    /// PII: US SSN structural rules.
    Ssn,
    /// Generic `secret = <value>` assignment value — deterministic sanity gates.
    Generic,
}

impl Guard {
    fn accept(&self, tok: &str) -> bool {
        match self {
            Guard::Format => true,
            Guard::Distinct(n) => distinct_at_least(tok, *n),
            Guard::Luhn => luhn_ok(tok),
            Guard::Iban => iban_ok(tok),
            Guard::Ssn => ssn_ok(tok),
            Guard::Generic => generic_value_ok(tok),
        }
    }
}

struct Rule {
    kind: &'static str,
    re: Regex,
    guard: Guard,
}

struct Ruleset {
    set: RegexSet,
    rules: Vec<Rule>,
}

fn ruleset() -> &'static Ruleset {
    static RS: OnceLock<Ruleset> = OnceLock::new();
    RS.get_or_init(build)
}

fn build() -> Ruleset {
    use Guard::*;
    // (kind, pattern, guard). The secret is ALWAYS capture group 1.
    let defs: &[(&str, &str, Guard)] = &[
        // ---- cloud / infra ----
        (
            "aws-key",
            r"((?:A3T[A-Z0-9]|AKIA|ASIA|AGPA|AIDA|AROA|AIPA|ANPA|ANVA|ACCA)[A-Z0-9]{16})",
            Format,
        ),
        (
            "aws-bedrock-key",
            r"(ABSK[A-Za-z0-9+/]{100,269}={0,2})",
            Format,
        ),
        ("google-api-key", r"(AIza[0-9A-Za-z_-]{35})", Format),
        ("google-oauth-secret", r"(GOCSPX-[0-9A-Za-z_-]{28})", Format),
        (
            "google-oauth-token",
            r"(ya29\.[0-9A-Za-z_-]{20,400})",
            Format,
        ),
        ("digitalocean-token", r"(do[oprt]_v1_[a-f0-9]{64})", Format),
        ("vault-token", r"(hv[sbr]\.[0-9A-Za-z_-]{24,130})", Format),
        ("cloudflare-token", r"(cfut_[0-9A-Za-z_-]{41,64})", Format),
        (
            "azure-storage-key",
            r"AccountKey=([0-9A-Za-z+/]{86,88}={0,2})",
            Format,
        ),
        // ---- source hosting / CI / registries ----
        (
            "github-token",
            r"((?:gh[oprsu]|github_pat)_[0-9A-Za-z_]{36,251})",
            Format,
        ),
        (
            "gitlab-token",
            r"(gl(?:pat|rt|dt|ft|oas|cbt|imt|agent|soat|ptt)-[0-9A-Za-z_-]{20,50})",
            Format,
        ),
        (
            "gitlab-runner-token",
            r"(GR1348941[0-9A-Za-z_-]{20})",
            Format,
        ),
        ("npm-token", r"(npm_[0-9A-Za-z]{36})", Format),
        (
            "pypi-token",
            r"(pypi-AgEIcHlwaS5vcmc[0-9A-Za-z_-]{50,})",
            Format,
        ),
        (
            "dockerhub-token",
            r"(dckr_(?:pat|oat)_[0-9A-Za-z_-]{27,32})",
            Format,
        ),
        (
            "postman-key",
            r"(PMAK-[0-9A-Za-z]{24}-[0-9A-Za-z]{34})",
            Format,
        ),
        // ---- AI / LLM ----
        (
            "openai-key",
            r"(sk-(?:proj|svcacct|admin)-[0-9A-Za-z_-]{40,})",
            Format,
        ),
        ("openai-key-classic", r"(sk-[A-Za-z0-9]{48})", Distinct(10)),
        (
            "anthropic-key",
            r"(sk-ant-(?:api|admin)[0-9]{2}-[0-9A-Za-z_-]{80,120})",
            Format,
        ),
        (
            "anthropic-oauth-token",
            r"(sk-ant-o[ar]t01-[0-9A-Za-z_-]{40,})",
            Format,
        ),
        ("groq-key", r"(gsk_[0-9A-Za-z]{50,54})", Format),
        ("huggingface-token", r"(hf_[a-zA-Z]{34})", Format),
        // ---- payments / commerce ----
        (
            "stripe-key",
            r"((?:sk|rk|pk)_(?:live|test)_[0-9A-Za-z]{24,99})",
            Format,
        ),
        ("square-access", r"(sq0atp-[0-9A-Za-z_-]{22})", Format),
        ("square-secret", r"(sq0csp-[0-9A-Za-z_-]{43})", Format),
        ("square-paypal", r"(EAAA[0-9A-Za-z+/=-]{60})", Distinct(12)),
        (
            "shopify-token",
            r"(shp(?:at|ca|pa|ss)_[a-fA-F0-9]{32})",
            Format,
        ),
        // ---- comms / collaboration ----
        (
            "sendgrid-key",
            r"(SG\.[0-9A-Za-z_-]{22}\.[0-9A-Za-z_-]{43})",
            Format,
        ),
        (
            "slack-token",
            r"(xox[baprse]-[0-9A-Za-z-]{10,72})",
            Distinct(8),
        ),
        (
            "slack-webhook",
            r"(https://hooks\.slack\.com/services/T[0-9A-Za-z_]+/B[0-9A-Za-z_]+/[0-9A-Za-z_]{20,30})",
            Format,
        ),
        (
            "telegram-bot",
            r"([0-9]{8,10}:AA[0-9A-Za-z_-]{32,33})",
            Format,
        ),
        (
            "discord-bot",
            r"([MNO][0-9A-Za-z_-]{23,25}\.[0-9A-Za-z_-]{6}\.[0-9A-Za-z_-]{27,38})",
            Distinct(12),
        ),
        // ---- observability / dev tooling ----
        (
            "grafana-sa-token",
            r"(glsa_[A-Za-z0-9]{32}_[a-fA-F0-9]{8})",
            Format,
        ),
        (
            "grafana-cloud-token",
            r"((?:glc_)?eyJrIjoi[A-Za-z0-9]{60,120})",
            Format,
        ),
        (
            "doppler-token",
            r"(dp\.(?:pt|ct|st|sa|scim|audit)\.[0-9A-Za-z]{40,44})",
            Format,
        ),
        (
            "dynatrace-token",
            r"(dt0[a-zA-Z][0-9]{2}\.[A-Z0-9]{24}\.[A-Z0-9]{64})",
            Format,
        ),
        (
            "databricks-token",
            r"(dapi[a-f0-9]{32}(?:-[0-9]+)?)",
            Distinct(10),
        ),
        ("linear-key", r"(lin_api_[0-9A-Za-z]{40})", Format),
        ("figma-token", r"(figd_[0-9A-Za-z_-]{38,44})", Format),
        // ---- keys / identity / generic-structural ----
        ("age-secret-key", r"(AGE-SECRET-KEY-1[0-9A-Z]{58})", Format),
        ("twilio-key", r"((?:AC|SK)[0-9a-f]{32})", Distinct(10)),
        (
            "jwt",
            r"(eyJ[0-9A-Za-z_-]{10,}\.eyJ[0-9A-Za-z_-]{10,}\.[0-9A-Za-z_-]{10,})",
            Format,
        ),
        (
            "private-key",
            r"(-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP |ENCRYPTED )?PRIVATE KEY-----[\s\S]{0,4000}?-----END (?:RSA |EC |DSA |OPENSSH |PGP |ENCRYPTED )?PRIVATE KEY-----)",
            Format,
        ),
        // URL / DSN embedded credentials — capture the password span only.
        (
            "url-password",
            r"[a-z][a-z0-9+.-]*://[^:/\s@]{1,60}:([^@\s/]{6,80})@",
            Distinct(4),
        ),
        // AWS *secret* access key — no prefix, so keyword-gated (the one
        // prefix-less secret worth the exception).
        (
            "aws-secret-key",
            r#"(?i)aws_?secret_?(?:access_?)?key["'\s:=]{1,6}([A-Za-z0-9/+]{40})"#,
            Distinct(8),
        ),
        // Generic `password/secret/token = <value>` — deterministic value gates.
        (
            "secret",
            r#"(?i)(?:pass(?:word|wd)?|secret|api[_-]?key|access[_-]?key|auth[_-]?token|client[_-]?secret|private[_-]?key|token)["' ]{0,3}[:=]{1,2}>?\s*["']?([^\s"']{6,120})"#,
            Generic,
        ),
        // ---- PII (validated) ----
        ("pii-ssn", r"\b([0-9]{3}-[0-9]{2}-[0-9]{4})\b", Ssn),
        ("pii-credit-card", r"\b((?:[0-9][ -]?){13,19})\b", Luhn),
        ("pii-iban", r"\b([A-Z]{2}[0-9]{2}[A-Z0-9]{11,30})\b", Iban),
    ];

    let set = RegexSet::new(defs.iter().map(|d| d.1)).expect("scan: valid RegexSet");
    let rules = defs
        .iter()
        .map(|&(kind, pat, guard)| Rule {
            kind,
            re: Regex::new(pat).expect("scan: valid Regex"),
            guard,
        })
        .collect();
    Ruleset { set, rules }
}

// ---------------------------------------------------------------------------
// Deterministic validators (no entropy)
// ---------------------------------------------------------------------------

/// At least `n` distinct byte values in `s` — the cheap placeholder filter.
fn distinct_at_least(s: &str, n: usize) -> bool {
    let mut seen = [false; 256];
    let mut count = 0usize;
    for b in s.bytes() {
        if !seen[b as usize] {
            seen[b as usize] = true;
            count += 1;
            if count >= n {
                return true;
            }
        }
    }
    false
}

/// Luhn mod-10 over the digits in `s` (separators ignored). Credit cards.
fn luhn_ok(s: &str) -> bool {
    let digits: Vec<u32> = s
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| (b - b'0') as u32)
        .collect();
    if !(13..=19).contains(&digits.len()) {
        return false;
    }
    let mut sum = 0;
    for (i, &d) in digits.iter().rev().enumerate() {
        let mut x = d;
        if i % 2 == 1 {
            x *= 2;
            if x > 9 {
                x -= 9;
            }
        }
        sum += x;
    }
    sum % 10 == 0
}

/// ISO 13616 IBAN mod-97: move first four chars to the end, map letters
/// A–Z → 10–35, reduce mod 97 in a streaming fashion; valid iff the remainder
/// is 1.
fn iban_ok(s: &str) -> bool {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if !(15..=34).contains(&s.len()) {
        return false;
    }
    let b = s.as_bytes();
    let mut rem: u32 = 0;
    // Rearranged order: chars [4..], then [0..4].
    for &c in b[4..].iter().chain(b[..4].iter()) {
        if c.is_ascii_digit() {
            rem = (rem * 10 + (c - b'0') as u32) % 97;
        } else if c.is_ascii_alphabetic() {
            let v = (c.to_ascii_uppercase() - b'A' + 10) as u32; // 10..=35
            rem = (rem * 100 + v) % 97;
        } else {
            return false;
        }
    }
    rem == 1
}

/// US SSN structural validity: area ∉ {000, 666, 900–999}, group ≠ 00,
/// serial ≠ 0000, and not a single repeated digit.
fn ssn_ok(s: &str) -> bool {
    let d: Vec<u8> = s
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .collect();
    if d.len() != 9 {
        return false;
    }
    if d.iter().all(|&x| x == d[0]) {
        return false;
    }
    let area = d[0] as u16 * 100 + d[1] as u16 * 10 + d[2] as u16;
    let group = d[3] * 10 + d[4];
    let serial = d[5] as u16 * 1000 + d[6] as u16 * 100 + d[7] as u16 * 10 + d[8] as u16;
    area != 0 && area != 666 && area < 900 && group != 0 && serial != 0
}

/// Deterministic sanity for a generic `secret = <value>` value. Tuned for an
/// audit report (over-masking is safe), but skips the obvious non-secrets so
/// the report stays readable: too-short, single-char-ish, known placeholders,
/// and *references* (env lookups / interpolation) that point at a secret rather
/// than being one.
fn generic_value_ok(v: &str) -> bool {
    if v.len() < 6 || !distinct_at_least(v, 4) {
        return false;
    }
    let lower = v.to_ascii_lowercase();
    const DENY: &[&str] = &[
        "null",
        "nil",
        "none",
        "true",
        "false",
        "undefined",
        "changeme",
        "change-me",
        "example",
        "placeholder",
        "redacted",
        "your_key",
        "test",
        "password",
        "secret",
        "123456",
        "000000",
    ];
    if DENY.contains(&lower.as_str()) {
        return false;
    }
    // A reference to a secret, not the secret itself — leave it (it's signal,
    // not a leak).
    if v.starts_with('$')
        || v.starts_with('<')
        || v.starts_with("{{")
        || v.contains("${")
        || lower.contains("process.env")
        || lower.contains("os.environ")
        || lower.contains("getenv")
        || lower.ends_with("_here")
    {
        return false;
    }
    // Credential shape (ripsecrets' "needs a digit" insight, deterministic): a
    // real secret value carries a digit, a real symbol (+/=@!…, not the `_-.`
    // of identifiers), or unusual length. A plain dictionary/status word —
    // `failed`, `Success`, `enabled` — carries none, so `password: failed`
    // isn't masked.
    let has_digit = v.bytes().any(|b| b.is_ascii_digit());
    let has_symbol = v
        .bytes()
        .any(|b| !b.is_ascii_alphanumeric() && !matches!(b, b'_' | b'-' | b'.'));
    has_digit || has_symbol || v.len() >= 16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<&'static str> {
        scan(text).into_iter().map(|f| f.kind).collect()
    }

    #[test]
    fn catches_strong_anchor_secrets() {
        assert_eq!(kinds("key AKIAIOSFODNN7REALKEYX here"), vec!["aws-key"]);
        assert_eq!(
            kinds("token=ghp_1234567890abcdefghijklmnopqrstuvwxyz"),
            vec!["github-token"]
        );
        assert_eq!(
            kinds("GOOGLE=AIzaSyA1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r"),
            vec!["google-api-key"]
        );
    }

    #[test]
    fn catches_anthropic_claude_code_oauth() {
        // The token class most likely to appear in a Claude Code session log.
        let t = "sk-ant-oat01-abcDEF123456ghiJKL789012mnoPQR345678stuVWX";
        assert_eq!(kinds(t), vec!["anthropic-oauth-token"]);
    }

    #[test]
    fn leaves_git_sha_and_hex_alone() {
        // The whole reason for anchored-not-entropy: hashes must survive.
        assert!(scan("commit 6d6cdc4e8f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c").is_empty());
        assert!(scan("blob a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0").is_empty());
    }

    #[test]
    fn redacts_to_typed_placeholder() {
        let (out, f) = scan_redact("export STRIPE=sk_live_abcdefghijklmnopqrstuvwx");
        assert_eq!(out, "export STRIPE=[redacted:stripe-key]");
        assert_eq!(f.len(), 1);
    }

    #[test]
    fn private_key_block_redacted_whole() {
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIBmiddlestuff\n-----END RSA PRIVATE KEY-----";
        let (out, _) = scan_redact(pem);
        assert_eq!(out, "[redacted:private-key]");
    }

    #[test]
    fn pii_requires_valid_checksum() {
        // Valid IBAN (checksum passes) → redacted; wrong checksum → left.
        assert_eq!(kinds("acct GB82WEST12345698765432"), vec!["pii-iban"]);
        assert!(scan("acct GB00WEST12345698765432").is_empty());
        // Valid test Visa (Luhn passes) → redacted; not-Luhn digits → left.
        assert_eq!(kinds("card 4111111111111111"), vec!["pii-credit-card"]);
        assert!(scan("card 4111111111111112").is_empty());
        // SSN structural: 000 area is invalid.
        assert_eq!(kinds("ssn 123-45-6789"), vec!["pii-ssn"]);
        assert!(scan("ssn 000-45-6789").is_empty());
    }

    #[test]
    fn generic_masks_literals_but_skips_references() {
        assert_eq!(kinds(r#"password = "hunter2mydog""#), vec!["secret"]);
        assert_eq!(kinds("token = a1b2c3d4e5f6g7"), vec!["secret"]);
        // References / placeholders are signal, not leaks — left alone.
        assert!(scan("api_key = process.env.API_KEY").is_empty());
        assert!(scan("password = null").is_empty());
        assert!(scan("token = your_key").is_empty());
    }

    #[test]
    fn generic_skips_plain_status_words() {
        // Real found FP: `Password: failed` masked the word "failed". A value
        // with no digit/symbol and ordinary length isn't a credential.
        assert!(scan("Password: failed").is_empty());
        assert!(scan("auth token: Success").is_empty());
        assert!(scan("secret = enabled").is_empty());
    }

    #[test]
    fn aws_secret_key_is_keyword_gated() {
        let real = "aws_secret_access_key = wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
        assert!(kinds(real).contains(&"aws-secret-key"));
        // A bare 40-char base64 with no keyword nearby is NOT flagged as an AWS
        // secret (that would nuke every hash-like blob).
        assert!(
            !kinds("blob wJalrXUtnFEMIzK7MDENGabPxRfiCYzzzzzzzzzz").contains(&"aws-secret-key")
        );
    }

    #[test]
    fn overlaps_resolve_to_one_span() {
        // `token=ghp_…` matches both the generic and the github rule; only one
        // span survives, and the redaction is clean.
        let (out, f) = scan_redact("token=ghp_1234567890abcdefghijklmnopqrstuvwxyz rest");
        assert_eq!(f.len(), 1);
        assert!(out.starts_with("token=[redacted:"));
        assert!(out.ends_with(" rest"));
    }
}
