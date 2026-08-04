# QA — audit gitreceipts across *your* repos

`run-qa.sh` is a zero-dependency harness that runs the built `git-receipts`
binary across every repo you have sessions for and every CLI switch — `audit`
(text + HTML) and `export` (JSON) — and checks that it holds its promises.

> ### ⚠ Privacy
> Every run writes **real session content** (your prompts, file paths, command
> output) into the output folder. **Run it in a private location and never
> commit the output.** The *script* is generic and safe to keep in the repo; its
> *output* is not. (The output folder is git-ignored here as a backstop.)

## Quick start — test your own repos

```bash
cargo build --release                       # build the binary under test
qa/run-qa.sh                                 # scans ~/Developer/Projects, uses ~/.claude/projects
qa/run-qa.sh --repos-root ~/code             # a different repos folder
qa/run-qa.sh --repos-root ~/code --store ~/.claude/projects mylabel
qa/run-qa.sh --mode repos                    # per-repo matrix only
qa/run-qa.sh --mode projects --project-dir ~/code/myproject   # --project mode only
```

It discovers every git repo under `--repos-root` that has Claude Code sessions
in `--store`, then for each one runs the full switch matrix and a cross-format
reconciliation. Results land in `<out>/<label>-<timestamp>/`:

- `summary.md` — pass/fail tally and the full results table
- `RECONCILIATION.md` — every headline number, compared across text/HTML/JSON
- `MANIFEST.md` — every output file and the exact command that produced it
- `<label>.txt` (command on line 1) · `<label>.html` (opens in a browser) · `<label>.json`

Run `qa/run-qa.sh --help` for all options.

## What it checks

Not golden snapshots — your session logs are private and change over time, so
there's nothing reproducible to diff against. Instead it asserts **invariants**:

- **exit codes** — `0` on the happy path; non-zero on bad repo / session / store
- **JSON** parses; **HTML** is self-contained (no external asset loads) and well-formed
- **leak-safety** — your home path never appears (it collapses to `~`)
- **suppression** — `--no-prompt` / `--no-summary` / `--no-intent` / `--no-identity`
  empty the right fields
- **reconciliation** — the console, HTML, and JSON are three renderings of one
  receipt, so every headline **and** exception number must be identical across
  them (commits, broken promises, claims landed, late landings, the unclaimed-
  change split, keyframes, failures)

## How this was validated

The harness runs against a real working set before every release. The launch
run: **11 repositories** across **5 projects** — **14 sessions** spanning
**~106 days**, **~221k LOC** over five stacks, produced by **3 different
models** (with mid-session switches) — **209 checks**, all passing, with the
console/HTML/JSON numbers reconciling 1-to-1 on every repo and the
`--project` roll-up reconciling across all three formats.

The reconciliation check has caught a real cross-format bug **before every
release** it has run for. Two examples: the HTML summary once showed the count
of *red intervals* where it should show *broken-promise claims* (identical
only when each red interval holds exactly one broken claim, so it hid until a
repo had several in one); and the console once dropped a provenance note
("N created elsewhere") that the JSON still carried. Both were caught by the
numbers-must-match rule, not by a human reading reports.

## Edge cases it exercises

- **Renamed repo** — sessions recorded under the old path, repo now at a new one
- **Nested repo / non-git working dir** — sessions recorded at a container that
  isn't itself a git repo; `--all` pulls them from a parent directory
- **git worktrees** — a `.git` file, shared object store
- **Multi-session merge** (`--all`) — several sessions into one ledger; commits
  from expired sessions shown as unclaimed keyframes, not broken promises
- **Mid-session model switch** — provenance attributed per commit
- **Error paths** — missing repo, missing session file, missing store, non-git
  directory: a clear non-zero exit, never a panic or a half-written report

## What redaction does — and doesn't

The tool redacts your **home directory path** (`/Users/you/...` → `~`), which is
the real privacy leak: it exposes your username and directory layout. It does
**not** rewrite your name where it legitimately appears in *your own project's
file and directory names* — e.g. a repo literally named `yourname.com` keeps its
paths intact, because mangling them would corrupt the report. If you want to
mask a name, domain, or client word even there before sharing, pass
`--redact <word>` (repeatable). The built-in secret/PII scanner additionally
masks API keys, tokens, and validated PII wherever they appear.
