---
description: Explain what the gitreceipts plugin can do — its commands, scopes, switches, and setup. Use when the user asks for gitreceipts help, how to use the gitreceipts plugin, or what its skills can do.
---

# gitreceipts plugin — help

Present this reference to the user, concisely and formatted:

**What it is.** gitreceipts reads a Claude Code session against the repo's
git history: each prompt you typed → the work it drove → the commit it
became. Runs locally; nothing leaves the machine.

**Setup.** Needs the `git-receipts` binary (0.1.1+, identity needs 0.1.3 — the skills offer the
upgrade if yours is older, and work either way). If it's missing, the
skills detect what you have and offer to install it — your approval, your
choice of route: `brew install cloudcraft-ai/tap/gitreceipts` ·
`cargo install gitreceipts` (then `git-receipts man --install` once) · or a
checksum-verified prebuilt binary, no package manager needed.

**Two skills, one receipt.**

| | |
|---|---|
| **`/gitreceipts:recap`** | *what happened* — the story: what you asked, what the agent did, what landed. Fires on "what happened here", "catch me up", "recap that commit". |
| **`/gitreceipts:audit`** | *did it land* — the verdict: every claimed edit checked against the real commit blobs, and the broken-promise count. Fires on "did that land", "any broken promises". |

**Say it however you like** — both skills map plain language to scopes:

| You say | It runs |
|---|---|
| nothing / "what happened" | all sessions for this repo |
| "this session" | the live one, found by identity (not newest-file guessing) |
| "the last run" | `--latest` |
| "what happened in commit X" | `--commit <hash>` |
| "all my repos here" | `--project` |
| "more detail" | `--verbose` |
| "just the problems" | `--filter red-amber` |
| "include my own commits" | `--full-history` (yours, outside this session) |
| "include everyone's commits" | `--all-authors` (other contributors too) |
| "that old email is me too" | `--me <name\|email>` |

**It follows git's conventions.** Git decides whose commits are yours —
`user.name`/`user.email`, matched on author or committer, `.mailmap`
honoured — and `.gitignore` decides which paths git was never going to take,
so a claimed edit to one of those is an explained finding, not a broken
promise. Every report states the identity it used and how much of the window
it covered; if nothing matches it refuses to run rather than print an empty
green report.

**Reading the marks.** 🟢 nothing to report · ⚪ explained findings, each
with its reason · 🟡 unexplained residue — a loose end with no answer on
record · 🔴 a broken promise: a claimed edit git never got. The verdict is
what git can witness; everything else is a finding. `broken promises: 0`
is earned, not assumed — and a red is a question, not a conviction.

**Three ways to keep it** — same receipt, different artifact:

| | |
|---|---|
| **`/gitreceipts:HTMLReport-Compact`** | a small page that renders right here in the preview. One commit is ~20KB; a whole repo a few hundred. Findings keep their full detail; everything else collapses, and every cap says what it hid. |
| **`/gitreceipts:HTMLReport-Full`** | the complete record — every commit's files, commands and conversation — opened in your browser. Too big for a preview pane by design. |
| **`/gitreceipts:JSONExport`** | the data: a versioned receipt to commit, diff between runs, or feed to another tool. Same numbers as both pages. |

**Sharing.** Reports are built from your prompts and command output —
private by default. For a shareable one: `--no-intent` (drops the
conversation, keeps every count), `--no-identity`, `--redact <word>`. Home
paths are masked automatically.

**Guardrails.** Never disables the secret/PII scanner, never installs the
binary unasked (it offers; you approve and pick the route), never loads a
full report into the conversation, and every run goes through your normal
Bash permission prompt.

**More:** README, docs/WHAT-GETS-AUDITED.md and KNOWN-LIMITATIONS.md at
https://github.com/jagmeetchawla/gitreceipts
