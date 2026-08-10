---
description: Verify a Claude Code session against git history with the git-receipts CLI — whether every claimed edit really landed, commit by commit, and what the broken promises are. Use when the user asks whether work really landed, to audit or verify a session, to check for broken promises or lost edits, or to gate on the result in CI. ALSO use when a delegated agent, sub-agent, worktree, or parallel lane REPORTS THAT IT IS DONE — "the engine lane says it's finished", "my sub-agent claims it shipped", "verify what that session actually did" — the moment a claim is relayed is the moment this is worth more than the claim.
---

# git receipts — the verdict

The audit answers one question: **did every claimed edit actually land?**
It reconciles the session log's file claims against the real commit blobs
and reports what balanced, what didn't, and what nothing explains.

For "what happened here", use `/gitreceipts:recap` — same receipt, read as
a story. This skill is for when the number matters.

## Prerequisite: the binary

```bash
command -v git-receipts >/dev/null && git-receipts --version || echo MISSING
```

If MISSING, offer the install (never unasked, never a refusal) — the
routes and the checksum-verified fallback are in `/gitreceipts:recap`,
which owns that flow. Version floor for the views below: **0.1.1**.

## Running it

```bash
git receipts audit --summary --emoji --no-pager --color never
```

That's the condensed verdict: the headline, then one row per commit —
`commit · subject · claims · findings`. Emoji because chat strips terminal
color, and the dots ARE the color layer here:

- 🟢 **green** — nothing to report
- ⚪ **grey** — explained findings: a file written and discarded before any
  commit, a failed command, an errored MCP call. Each carries its reason.
- 🟡 **amber** — **un**explained residue: git recorded a change no claim
  and no command accounts for. A loose end with no answer on record.
- 🔴 **red** — a broken promise: a claimed edit git never got, and nothing
  explains it.

A claim on a **gitignored** path that never landed is grey, not red: git was
configured never to take it, so nothing git held was lost.

Present the table as a code block (pre-aligned), header included. The rule
that makes it readable: zero counts never print, every nonzero cause is
named, a clean row shows `—`.

**Whose commits.** A commit counts as the user's when git records them as
author or committer, by name or email (`.mailmap` honoured). The header
states it: `you: Ada <ada@example.com> (191 of 204 commits are yours)`.
Read that line — a wrong `user.email` shows up as a shrinking ratio, not an
error. If it refuses to run because nothing matched, the fix is
`--me <name|email>` or `--all-authors`, never ignoring it.

**Verifying a lane you did not run.** When the user is coordinating other
agents, the valuable audit is the one on a lane they cannot see — the lane
writes its own summary, it does not write the git history. Note two things
honestly: isolating one lane out of several is manual today (the default
merges every session for the repo), and a red found this way is still a
question, not a conviction — the lane may know something the log doesn't.

Scopes compose exactly as recap's do: `--commit <hash>`, `--project`,
`--latest`, `--this-session <marker>`, `--full-history`. Narrower views:
`--filter red-amber` (the unanswered ones), `--filter grey` (the answered
ones), `--oneline` (every commit, no caps), `--verbose` (full anatomy).
`--full-history` adds the user's own commits this session didn't make;
`--all-authors` adds other people's. Different axes — neither implies the
other.

## Interpreting for the user

- **Lead with the headline** — commits, the four-way balance, claims
  landed, broken promises. Then drill only into what's flagged.
- **`broken promises: 0` is earned, not assumed** — it means zero among
  the claims audited. Absence of a claim is not absence of action.
- **A red is a question, not a conviction.** It means nothing on record
  explains the claim; the user may know something the log doesn't. Quote
  the tool's own diagnosis — those are evidence-backed ("content-verified",
  "relocated before its first commit", "reformatted before landing") —
  and never soften one into a pass.
- **Grey is not a problem to fix.** It's a finding that already carries
  its explanation. Say what it is; don't dress it as a risk.
- **Amber is the one to look at**: something changed that nothing accounts
  for.
- **Held out** — commits the agent didn't make (teammates, pulls) sit
  outside the verdict and are shown as a count; `--full-history` includes
  them.

## Failures are findings, not noise

Never wave off a failed command as "noise", "sandbox stuff", or "benign"
without looking. The binary classifies each one with its evidence —
expected-nonzero (grep's documented no-match), guarded by the command's
own `||`, retried-and-passed, aborted by you, sandbox-denied, or genuine —
and the export carries `failure_class` and `failure_evidence` per command.
Read those and say which it is. An unexamined failure called harmless is
exactly the kind of claim this tool exists to catch.

```bash
git receipts export | python3 -c "
import json,sys
d=json.load(sys.stdin)
for i in d['intervals']:
    for r in i.get('commands',{}).get('runs',[]):
        if r.get('failed'):
            print(i['commit']['short'], r.get('failure_class'), '—', r.get('failure_evidence'))
"
```

## CI

`--exit-code` encodes the verdict: **0** green or grey (the equation
balances), **1** amber, **2** red. A red-only gate tests `>= 2`. Grey never
raises it — explained findings are not failures.

## Artifacts

```bash
git receipts audit --format html --compact > audit.html   # the page
git receipts export > receipt.json                        # the data
```

Open HTML in the browser (`open` / `xdg-open`); `--compact` keeps it small
enough for a preview pane. Write it outside the repo so the audit doesn't
create residue its own next run would flag.

## Privacy and guardrails

Reports contain the user's prompts and command output — private by
default. Whenever you suggest sharing, name `--no-intent`, `--no-identity`
and `--redact <word>` in the same breath. **Never pass `--no-scan`.** Never
install the binary unasked. Never load a full report into the conversation.
