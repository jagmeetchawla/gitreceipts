---
description: Export the reconciled Claude Code session receipt as JSON — the machine-readable artifact to keep, diff, or feed to another tool. Use when the user asks for the data, the JSON, an export, a receipt file, or something to download or process programmatically.
---

# JSON export — the receipt as data

`git receipts export` runs the same pipeline as the audit and emits the
reconciled result as JSON: per-commit statement, claim ledger, residue,
commands with their failure classes, MCP calls, intents, token estimates,
provenance, exceptions, blast radii, identity roll-up. **Every number
matches the console and the HTML** — three renderings of one receipt.

This is the artifact to hand over: commit it beside the code, diff two runs
against each other, feed it to another program, or give it to a model to
interpret. It is far smaller than the raw session log it distills.

## Run it

Requires the `git-receipts` binary (0.1.1+) — `/gitreceipts:recap` owns the
install and upgrade offers if it's missing or old.

```bash
F="${TMPDIR:-/tmp}/gitreceipts-receipt.json"
git receipts export > "$F" && echo "$F"
```

**Write it to a file and give the path — never paste the JSON into the
conversation.** A real receipt is megabytes; it belongs in a file the user
downloads or opens, not in context.

Scopes are the same as everywhere: `--commit <hash>`, `--project` (a
`{project, repos: […]}` wrapper), `--latest`, `--this-session <marker>`,
`--full-history`. Shape flags: `--compact` (single line, for piping),
`--full` (the whole transcript plus every command's output — the maximal
export).

## What to tell them

- The schema is versioned (`schema_version`) and **additive-only within
  0.x** — new fields may appear, existing ones never change meaning. The
  full promise is in `docs/COMPAT.md` in the repo.
- If they're building on it: read defensively, ignore unknown fields, and
  treat optional fields as optional (`resolution`, `diagnosis`,
  `landed_at`, `failure_class` are omitted rather than null).

## Privacy — read this before sharing one

The export is the most complete artifact of the three, and unlike a report
nobody skims it before passing it on. It carries prompts, command text, and
captured output verbatim. Whenever the user mentions sharing, committing,
or uploading it, name the flags in the same breath: `--no-intent` (drops
prompts and agent prose; every count stays), `--no-identity` (drops names
and emails), `--redact <word>`. Home paths are masked automatically.
**Never pass `--no-scan`** — the secret/PII scanner stays on, always.
