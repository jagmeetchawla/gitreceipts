---
description: Produce a COMPACT HTML report from a Claude Code session — small enough to render inline in Claude's preview. Use when the user asks for a compact report, a small report, a report they can read here, or a preview of the audit.
---

# Compact HTML report — small enough to read right here

Same receipt as `/gitreceipts:audit`, rendered as a self-contained page and
kept small on purpose: commits with unexplained findings keep their full
file and command lists, everything else collapses to its headline, and long
prose and captured output are clipped. A full report runs several hundred
KB and preview panes choke; this one is a fraction of that.

**This is the one that renders inline.** Write the file and hand back the
path — let the preview show it. Don't open a browser unless the user asks;
that's `/gitreceipts:HTMLReport-Full`'s job.

## Run it

Requires the `git-receipts` binary (0.1.1+) — `/gitreceipts:recap` owns the
install and upgrade offers if it's missing or old.

```bash
F="${TMPDIR:-/tmp}/gitreceipts-report.html"
git receipts audit --format html --compact > "$F" && echo "$F"
```

Match the scope to whatever the user is asking about — the same flags every
other skill takes: `--commit <hash>` (much smaller, ~20KB — the best case
for inline preview), `--project`, `--latest`, `--this-session <marker>`,
`--full-history`. For the story rather than the verdict, swap `audit` for
`recap`; the page framing follows the command.

State the path. It lands OUTSIDE the repo deliberately — a report must not
leave files its own next run would flag as residue. If the user wants it
kept somewhere, regenerate to the path they name.

## What to tell them

Say what the page contains and what it hides: every cap in a compact page
announces how much it omitted, and the JSON receipt
(`/gitreceipts:JSONExport`) is always complete. If they need the full
inventory for every commit, that's `/gitreceipts:HTMLReport-Full`.

## Privacy

The page is built from the user's prompts and command output — private by
default. Whenever sharing comes up, name the flags in the same breath:
`--no-intent` (drops the conversation, keeps every count), `--no-identity`
(drops names and emails), `--redact <word>`. Home paths are masked
automatically. **Never pass `--no-scan`** — the secret/PII scanner stays on.
