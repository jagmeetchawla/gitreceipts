---
description: Explain what the gitreceipts plugin can do — its commands, scope defaults, switches, and setup. Use when the user asks for gitreceipts help, how to use the gitreceipts plugin, or what the audit skill can do.
---

# gitreceipts plugin — help

Present this reference to the user, concisely and formatted:

**What it is.** gitreceipts audits a Claude Code session against git history —
what the agent *claimed* vs. what actually *landed*, commit by commit. Runs
locally; nothing leaves the machine.

**Setup.** Needs the `git-receipts` binary:
`brew install cloudcraft-ai/tap/gitreceipts` · `cargo install gitreceipts` ·
or attested binaries at https://github.com/jagmeetchawla/gitreceipts/releases

**Main skill: `/gitreceipts:audit`** (also fires automatically when you ask
things like "what did the agent actually do?" or "did that work land?")

| You say | It runs |
|---|---|
| nothing (bare) / "audit this repo" | all sessions of the current repo — the complete picture |
| "this session" | exactly the live session (identity-matched, not newest-file guessing) |
| "the latest run" | `--latest` (single-session caveats stated) |
| "just the problems" | `--filter red-amber` |
| "what happened in commit X" | `--commit <hash>` (add `--full` for its conversation) |
| "audit all my repos here" | `--project <folder>` roll-up |
| "include my hand-made commits" | `--full-history` |
| explicit flags after the command | passed through to the CLI verbatim |

**Reading the verdict.** Green = balanced. Amber = worth a look, never a lie.
Red = a claimed edit git never got and nothing explains — presented as a
question, not a conviction. `broken promises: 0` is earned, not assumed.

**Sharing.** Reports contain prompts and command output — private by default.
For a shareable report: `--no-intent` (drops conversation, keeps counts),
`--no-identity`, `--redact <word>`, or `--format html > audit.html`.

**Guardrails.** The plugin never disables the secret/PII scanner, never
installs the binary unasked, and every run goes through your normal Bash
permission prompt.

**More:** README and KNOWN-LIMITATIONS.md at
https://github.com/jagmeetchawla/gitreceipts
