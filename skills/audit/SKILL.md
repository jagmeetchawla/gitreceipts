---
description: Audit a Claude Code session against git history with the git-receipts CLI — verify what the agent claimed vs. what actually landed, commit by commit. Use when the user asks what an agent actually did, whether a session's work really landed, to audit/verify/review an agentic session, to check for broken promises, or after a long unattended run.
---

# git receipts — audit the session against git

gitreceipts reconciles a Claude Code session log against the repo's git
history. The session log is the agent's own story; git is the independent
record it can't rewrite. The audit checks every file-edit claim against the
actual commit blobs and reports what landed, what landed late
(content-verified), what was benignly resolved, and what is a **broken
promise** — claimed, never landed, nothing explains it.

## Prerequisite: the binary

Check it exists before anything else:

```bash
command -v git-receipts || echo MISSING
```

If MISSING, tell the user how to install it — **never install it on their
behalf unless they explicitly ask**:

- `brew install cloudcraft-ai/tap/gitreceipts`
- or `cargo install gitreceipts` (Rust 1.90+)
- or prebuilt, attested binaries: https://github.com/jagmeetchawla/gitreceipts/releases

## Running an audit

Run from inside the repo being audited (or pass `--repo <dir>`):

```bash
git-receipts audit --no-pager --color never          # all sessions — the default, complete picture
git-receipts audit --no-pager --color never --latest # just the most recent session
git-receipts audit --no-pager --color never --filter red-amber   # findings only
git-receipts audit --no-pager --color never --commit <hash>      # one commit's story
git-receipts export > receipt.json                   # machine-readable, same numbers
git-receipts audit --project <folder> --no-pager --color never   # several repos under one folder
```

Always pass `--no-pager --color never` when capturing output for analysis.
For large repos prefer `export` and read the JSON: `summary.balance`
(green/amber/red), `summary.broken_promises`, `summary.exceptions`, and
`intervals[]` (the per-commit ledger with `status`, `ledger[]`, `residue[]`).

## Choosing scope (defaults)

- Bare invocation or "audit this repo" → the CLI default (**all sessions** —
  the complete picture). Always state the scope you ran.
- "this session" / "what did you just do" → target THIS session precisely,
  not `--latest` (which guesses by mtime and can race a parallel session).
  The live log lets you find it by identity:

  ```bash
  N="receipts-$$-$(date +%s)"; echo "$N"; sleep 1
  S=$(grep -rl "$N" ~/.claude/projects/*/ 2>/dev/null | head -1)
  git-receipts audit "$S" --no-pager --color never
  ```

  If the grep finds nothing (write lag), wait a second and retry once; if it
  still misses, fall back to `--latest` and say so.
- "just the problems" → `--filter red-amber` · one commit → `--commit <hash>`
  (add `--full` for its conversation) · a folder of repos → `--project <dir>`
  · "include commits I made by hand" → `--full-history`.
- cwd not a git repo but contains repos? Use `--project .` instead of letting
  inference fail.

If the user supplies `$ARGUMENTS`, pass them through to `git-receipts audit`
verbatim — but only if they look like flags, paths, or a commit hash, never
shell syntax. **Never pass `--no-scan`**: the built-in secret/PII scanner
stays on, always.

## Interpreting for the user

- **Green** — the interval balances: every claim landed, no residue, no
  failed commands. Nothing to look at.
- **Amber** — worth a look, never a lie: unexplained residue, a failed
  command, or an errored MCP call.
- **Red / broken promises** — a claimed edit git never got, with no verified
  explanation. This is the headline number. Present a red as **a question,
  not a conviction** — it means nothing on record explains the claim, and
  the user may know something the log doesn't.
- **Held out** — commits the agent didn't make (teammates, pulls) are
  excluded from the verdict by default and shown as a count;
  `--full-history` includes them.
- `broken promises: 0` means zero among the claims audited — absence of a
  claim is not absence of action.

Summarize the headline first (commits, % green, claims landed, broken
promises), then drill into ambers and reds only if present or asked. Quote
the tool's own diagnosis lines — they are evidence-backed
("content-verified", "relocated before its first commit", "deleted before
any commit") — and never soften a red into a pass.

## Privacy

Reports contain the user's prompts and command output — treat every report
as private. Whenever you suggest sharing or exporting one, mention the
privacy flags in the same breath: `--no-intent` (drops prompts and agent
prose; every count stays), `--no-identity` (drops names/emails), and
`--redact <word>` (masks any extra term). Self-contained HTML report:
`git-receipts audit --format html > audit.html`.

## Caveats (v0.1)

- Sessions recorded on a **different machine** are not supported — see
  KNOWN-LIMITATIONS.md §8 in the repo.
- Plain `git receipts --help` wants a man page that ships in 0.1.1; use
  `git-receipts --help` or `-h`.
