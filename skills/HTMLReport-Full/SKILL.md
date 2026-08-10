---
description: Produce the FULL HTML report from a Claude Code session — every commit with its complete file and command lists, opened in the browser. Use when the user asks for the full report, the complete report, the whole audit as a page, or wants to open the report in a browser.
---

# Full HTML report — the complete record, in a browser

Same receipt as `/gitreceipts:audit`, rendered as a self-contained page with
**nothing collapsed away**: every commit keeps its file list, its commands
with blast radii, its MCP calls, its intent and the agent's own summary.
Theme-aware, no external assets, works offline.

**This one is for the browser, not the preview.** It runs several hundred
KB on a real session — chat preview panes render it as raw markup or not at
all. Generate and open it in one step. If the user wants something readable
in the conversation, that's `/gitreceipts:HTMLReport-Compact`.

## Run it

Requires the `git-receipts` binary (0.1.1+) — `/gitreceipts:recap` owns the
install and upgrade offers if it's missing or old.

```bash
F="${TMPDIR:-/tmp}/gitreceipts-report-full.html"
git receipts audit --format html > "$F" && open "$F" && echo "$F"
```

(`xdg-open` on Linux.) No browser — SSH, headless, a container? Still write
it, give the path, and say why it didn't open. Never skip silently.

Match the scope to what the user is asking about: `--commit <hash>`,
`--project`, `--latest`, `--this-session <marker>`, `--full-history`. Swap
`audit` for `recap` if they want the story framing rather than the verdict.
`--expand all|none` controls what starts open (default: findings open,
clean commits collapsed). `--with-output` includes every command's captured
output, not just the failures'.

State the path afterward. It lands OUTSIDE the repo deliberately — a report
must not leave files its own next run would flag as residue. Regenerate to
a path the user names if they want to keep it.

## Privacy

This is the most complete artifact the tool produces: every prompt, every
command, every captured output. Treat it as private, and whenever sharing
comes up name the flags in the same breath — `--no-intent` (drops the
conversation, keeps every count), `--no-identity`, `--redact <word>`. Home
paths are masked automatically. **Never pass `--no-scan`** — the secret/PII
scanner stays on, always.
