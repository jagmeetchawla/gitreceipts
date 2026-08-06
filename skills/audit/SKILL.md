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

## Running an audit — token discipline first

**Never load a full report into the conversation.** Full console output and
full export JSON are for FILES humans open, not for context. Extract in the
shell; only the summary enters the chat. Work in tiers:

**Tier 1 — headline + spine table (default for every audit).** The JSON
schema is versioned and stable — parse it, don't screen-scrape the console.
One command yields the headline, every finding (any age), and the recent
spine; only this enters the conversation:

```bash
git-receipts export 2>/dev/null | python3 -c "
import json,sys
d=json.load(sys.stdin); s=d['summary']; b=s['balance']
print('commits', s['commits'], '(+'+str(s['keyframes_excluded'])+' by others held out) ·', b['green'],'green /',b['amber'],'amber /',b['red'],'red · claims', str(s['claims_landed'])+'/'+str(s['claims_total']), '· broken promises', s['broken_promises'])
iv=d['intervals']; CAP=15
g={'green':'.','amber':'!','red':'X'}
def row(i):
    L=i.get('ledger',[]); landed=sum(1 for l in L if l.get('landing')!='never')
    print(' ',g.get(i['status'],'?'), i['commit']['short'], (i['commit']['subject'][:46]).ljust(46), str(landed)+'/'+str(len(L)), 'landed ·', len(i.get('residue',[])), 'residue')
older=iv[:-CAP] if len(iv)>CAP else []
for i in older:
    if i['status']!='green': row(i)
if older: print('  …', len(older), 'earlier commits (findings above shown; --oneline for all)')
for i in iv[-CAP:]: row(i)
"
```

Present the table as a code block (it is pre-aligned). `.` = green,
`!` = amber, `X` = red. If the findings list itself runs very long (>25),
truncate it the same way and say how many were omitted — never silently.

For `--project`, same idea on the wrapper: `git-receipts export --project <dir>`
then read `summary.verdict` and the `summary.landing[]` rows only.

**Tier 2 — lists, when the user wants to scan.**
`git-receipts audit --oneline --no-pager --color never` (one line per commit;
add `--filter red-amber` for findings only).

**Tier 3 — one commit's full story, on demand.**
`git-receipts audit --commit <hash> --no-pager --color never` (add `--full`
for its conversation). For programmatic drill-down, filter the export:
`… | python3` selecting `intervals[]` by `status` or `commit.short` — never
print all intervals.

**Tier 4 — durable artifacts for humans.**
`--format html > audit.html` (then open it in the browser) ·
`export > receipt.json` (state the path; don't cat it).

Session-file and `--latest`/`--repo <dir>` arguments compose with all tiers.

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

## After the audit — offer the durable outputs

After presenting the FIRST audit in a conversation (not after every
follow-up), end with one compact hint line so users discover the two
outputs they can keep:

> Want this as a self-contained **HTML report** (`git-receipts audit
> --format html > audit.html`) or **machine-readable JSON**
> (`git-receipts export > receipt.json`)? If you plan to share either,
> add `--no-intent` (drops prompts; counts stay) and consider
> `--no-identity`.

If the user asks for one, generate it, tell them the file path, and restate
the privacy flags if they didn't use any. **Open HTML reports directly in the
browser** (`open <file>` on macOS, `xdg-open` on Linux) — full reports are
large single-file pages that chat preview panes often can't render inline;
the browser is the intended viewer. For a lighter file, offer
`--filter red-amber` (findings only) or `--latest` (one session).

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
