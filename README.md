<p align="center">
  <br>
  <strong style="font-size: 1.5em;">git receipts</strong>
  <br><br>
  See what your agent <b>actually</b> did — every prompt → commands → MCP → files → commit,
  <br>reconciled against the one record the agent can't rewrite: <b>git</b>.
  <br><br>
  <a href="#install">Install</a> ·
  <a href="#usage">Usage</a> ·
  <a href="https://github.com/cloudcraft-ai/gitreceipts/releases">Releases</a> ·
  <a href="RECEIPTS.md">RECEIPTS.md</a> ·
  <a href="KNOWN-LIMITATIONS.md">Known limitations</a> ·
  <a href="qa/">QA harness</a>
  <br><br>
</p>

**Developer velocity is no longer the bottleneck. Comprehension velocity is.**
An agent can produce a day's worth of commits in an hour. Understanding what
happened is now the slow part. The raw session log is tens of megabytes of
JSON nobody reads.

gitreceipts turns that log into an account you can read at reading speed:
each prompt you typed → the commands and MCP calls it made → the files it
touched → the commit that resulted. An 11,000-event session becomes a spine of commits you
scan in a minute and expand only where something looks off.

And you can trust the account, because it's audited, not narrated. The
session log is the agent's own story. git kept an independent record of what
actually persisted — a log the agent can't fabricate after the fact.
gitreceipts reads the two side by side and reconciles them, claim by claim,
content-level against the actual commit blobs. You're not reading the
agent's memoir. You're reading an audited one. Not the agent's word. Git's.

The account is also **yours**. Everything runs locally. It reads exactly one
directory (your session logs) plus your repo, and sends nothing anywhere. A
mirror for your own work, not a badge to show anyone. Details in
[what the report contains — and what it doesn't](#what-the-report-contains--and-what-it-doesnt).

Here's what a session looks like after the audit:

```
claims: 219 file mutations · 412 commands · 63 MCP calls · 187 observations
  OS/FS: 412 commands · 3 failed · 1 aborted by you
  MCP:    63 calls · 2 errored · 0 aborted by you

intent → outcome
  173 prompts drove 52 commits; 214/219 claimed files landed (98%), 49 intervals fully balanced
  agent effort: 412 commands · ~5.0M output tokens across 3,413 requests (est., not billing)
  model(s): opus-4-8 (2,605 req), fable-5 (808 req)
  reasoning effort: high, max (partial coverage in the log)
  what happened to every exception:
    · claims that landed late: 3, content-verified against the commit they landed in
    · claims that never landed, resolved: 4 — superseded by later landed edits: 2 ·
      removed deliberately by the session's own commands: 2
    · unclaimed changes (git recorded it, no matching edit claim): 31 —
      this agent via a command: 24 · not this session's commit: 6 · unexplained: 1
    · broken promises (claimed, never landed, nothing explains it): 0
  who touched this repo (git identity — not how they authored):
    · committed by: Ada Lovelace <ada@example.com>
    · co-authored-by (declared in commits): Claude <noreply@anthropic.com>
```

That `broken promises: 0` is the headline number: a claim that never landed
and nothing explains. Zero is not assumed — it is earned, claim by claim,
against the repo's own receipts.

And when the number isn't zero, a red line is a **question, not a
conviction**. It means nothing on record explains the claim. You may know
something the log doesn't — each explainable red is a candidate for a new
resolution category. The current honest gaps are cataloged in
[KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md).

**When you'd reach for it** — two moments especially, both where you can't
just reconstruct what happened by hand:

- **Unattended runs.** Overnight autonomy, long agentic sessions, background
  or scheduled agents. You weren't watching, and you can't replay what you
  didn't see. You come back to a stack of commits and need a trustworthy
  account of what actually happened.
- **Looking back.** Work from a while ago, where your own memory has faded
  and the raw session — tens of megabytes, if it even survived — is
  impractical to trawl. git still holds the truth; gitreceipts distills what
  you *asked* and what the agent *did* into a receipt you can actually read.

The value isn't only in catching problems. A 100%-green session still comes
out as something you can *read* — intent → outcome, ask → what landed —
instead of a log to trawl. Verification is what makes the account
trustworthy; comprehension is why you run it on every session, not just the
suspicious ones.

## Install

```bash
# crates.io (Rust 1.90+)
cargo install gitreceipts

# Homebrew
brew install cloudcraft-ai/tap/gitreceipts

# prebuilt binaries (checksums + provenance attestations attached)
# → https://github.com/cloudcraft-ai/gitreceipts/releases

# from source
git clone https://github.com/cloudcraft-ai/gitreceipts
cd gitreceipts
cargo install --path .
```

The binary is `git-receipts`, which git picks up as a subcommand:
`git receipts …`.

**Verified on macOS (Apple Silicon) and Linux (x86_64).** The Intel macOS
binary passes the full test suite as x86_64 code (under Rosetta 2 translation
in CI), but hasn't been hand-verified on Intel silicon — reports welcome.

## Usage

```bash
# audit ALL of this repo's sessions — the default, the complete picture
git receipts audit

# just my last run (one session). CAVEAT: a single-session audit still
# reconciles against the WHOLE repo, so commits from your other sessions
# match nothing and show up as residue / unclaimed keyframes. Prefer the
# default (all sessions) unless you specifically want one run.
git receipts audit --latest

# the commit spine, one line per commit (git log --oneline style)
git receipts audit --oneline

# scope to a single commit (short hash from --oneline) — lspci -s style
git receipts audit --commit 6d6cdc4

# read a commit's whole conversation (every prompt + assistant message)
git receipts audit --commit 6d6cdc4 --full

# audit a specific session file (exactly that one)
git receipts audit ~/.claude/projects/<project>/<session>.jsonl

# audit a specific session log against a specific repo
git receipts audit <session>.jsonl --repo ~/code/myapp

# just the broken promises
git receipts audit --filter red

# filter the spine purely by color (red = broken promises; amber = residue,
# failed command, or errored MCP; green = clean)
git receipts audit --filter red-amber

# drop conversational content before sharing — counts always stay
git receipts audit --no-prompt     # your prompts
git receipts audit --no-summary    # the agent's prose
git receipts audit --no-intent     # both

# show every command in full with its captured output
git receipts audit --with-output

# a self-contained HTML report (theme-aware, no external assets).
# green commits collapse to a line; red or amber commits open with their
# files and commands, and succeeded detail is dulled so failures stand out
git receipts audit --format html > audit.html

# export the reconciled audit as a versioned JSON receipt
git receipts export > receipt.json

# audit a whole PROJECT folder holding several git repos — a "where it
# landed" roll-up (verdict · commits · landed · broken · residue per repo),
# then a section per repo. Works for all three formats.
git receipts audit --project ~/Developer/Projects/myproject
git receipts export --project ~/Developer/Projects/myproject > project.json
git receipts audit --project ~/Developer/Projects/myproject --format html > project.html

# include commits the agent DIDN'T make in the verdict (see below)
git receipts audit --full-history

# audit another machine's sessions from a mounted drive
git receipts audit --store /Volumes/studio/Users/me/.claude/projects --repo /Volumes/studio/Users/me/code/myapp
```

On a terminal the report auto-pages through `$PAGER` (like git), colors
intact. `--no-pager` opts out; `--color always` forces colors through your
own pipe.

## What it checks

Reconciliation runs both directions, and both get billing:

- **Claimed → didn't land.** The agent said it, git never got it. If nothing
  explains it, that's a **broken promise**.
- **Landed → not claimed.** git recorded a change no edit claim covers. The
  interesting question is *who*, and git answers it: the **author** and any
  **`Co-Authored-By:`** trailer on the commit. The tool tells you, by
  difference, what another contributor (or another agent) changed versus
  what this session did — attribution for free.

What it **verifies** — because git can witness it:

- **File writes, edits, deletes** — claimed content checked against the
  actual commit blobs. Content-level, not filename-level, so a coincidental
  change can't be mistaken for the claim landing.
- **Commits** — matched to the real commit graph and reflog.
- **Pushes** — checked against the remote-tracking refs.
- **Who** — every commit's author and declared co-authors, straight from
  git. Identity only: a name tells you *who committed*, never *how* (agent
  or hand). A `Co-Authored-By` trailer is present-only evidence — never
  inferred from its absence.

What it **surfaces** — the rest of the story, honestly labeled:

- **Intent** — the prompts *you* typed, attached to the commit each one
  drove. Every commit reads *ask → what landed*.
- **Agent effort** — the work behind each commit: commands, MCP calls, API
  requests, and an estimated token count (deduplicated per request; marked
  *estimated, not billing*).
- **MCP calls** — first-class actions, not observations. Every call is
  classified like a command: the server's own response is its receipt
  (the `tool_result` is the oracle — receipted vs errored), and an
  errored call tints the commit **amber**: worth a look, never
  manufactured into red. Your own aborts count as your stop, not the
  agent's failure.
- **Model provenance** — which model(s) produced the session, per commit. A
  mid-session switch (say Opus → Fable) shows exactly what wrote what.
  Reasoning effort rides along where the log records it, labeled with its
  coverage.
- **Blast radius** — how far each command reached: `local-fs → local-git →
  remote-git → network`. A network call or a deploy leaves nothing in git,
  so its captured output is shown as the agent's *own receipt* — never
  dressed up as proof.

## How it works

Think of it as bank reconciliation for agent work:

- **The statement** — the repo's real commits form the spine. Each commit's
  diff is the statement for the interval that produced it. Commit history is
  the primary source — the one thing every repo has, clone or original. When
  the local reflog exists it adds what history can't carry: amended drafts,
  reset-away commits, true creation order, and backdating detection (a
  commit created *between* two in-window commits stays in the audit no
  matter what its dates claim — dates are forgeable, creation order is not).
- **The ledger** — the session log's own tool calls are the claims. File
  edits carry their exact content. Shell commands carry a blast radius
  and, at best, captured output as a receipt. MCP calls carry the
  server's response as theirs — a second oracle next to the OS,
  reported per-oracle in the header (`OS/FS: … failed · MCP: … errored`).
- **The equation** — per interval, claims are matched against the statement.
  Green when it balances. Everything else is itemized, investigated, and
  labeled with its evidence:

| finding | meaning |
|---|---|
| `✓ landed late` | the claim's content was found in a later commit's blob — cites which, and how many commits later |
| `◌ never landed, resolved` | superseded by a later landed edit · removed deliberately by an on-record command · relocated before its first commit (content-verified) · or persisted on disk outside git's view |
| `● residue (attributed)` | changed without a file-edit claim, but named by a command that ran (`git mv`, a `sed` loop) — with rename provenance |
| `○ residue dismissed` | the path is gitignored or untracked *today* — retroactively declared noise by you |
| `✘ never landed` | a claim that never landed **and nothing explains it** — the only thing that turns an interval red |
| `↻ amend` | a draft commit superseded seconds later ("committed 2,603 files, amended 12s later" is a story worth telling) |
| `⚠ clock anomaly` | a commit whose dates disagree with its place in the reflog — kept in the audit, flagged as untrustworthy |

The operating principle throughout: **when the evidence is ambiguous, the
report says "ambiguous"** — it never picks the flattering interpretation.

## Your commits, not the whole repo

By default the verdict, balance, and spine cover **only the commits this
agent made**. Commits in the same window that the agent *didn't* make — a
teammate's push, a `git pull`/merge, your own hand-commits, an existing
codebase's history swept into the window — are **held out** as context,
never counted as the agent's residue or broken promises.

This is what makes gitreceipts usable when you adopt agentic development
**on top of a mature codebase**: you get an account of *your agent's* work,
not a wall of red for years of history it never touched.

Pass `--full-history` to put every in-window commit back in the equation. A
count of what was held out is always shown, so nothing is hidden — e.g.
*"99 agent commits · 3 commits by others held out."*

## Projects — several repos, one session

Most of the time the working directory **is** the repo, and a bare
`git receipts` audits it. But a single session sometimes drives **several
git repos** under one folder — commonly a **public repo beside private
ops/config repos**, kept separate precisely so the private ones never ship
in the public history.

`--project <folder>` is the unit for that layout:

- Discovers every git repo under the folder that has sessions.
- Opens with a **where-it-landed** roll-up — one row per repo (verdict ·
  commits · landed/claims · broken · residue).
- Then a full section per repo. Console, JSON, and HTML all reconcile.
- A folder that is itself a single repo (a monorepo) collapses to the
  ordinary report: project ≡ repo.

**Sibling protection.** Each repo's section shows only *that* repo. Writes
its session made into a **sibling** repo collapse to a per-repo **count** —
*"writes into sibling project repo: ops (14 changes) — audit each directly;
paths withheld here"* — with no paths. A single repo's section (and its
receipt in the JSON wrapper) is safe to hand out without exposing another
repo's file tree. Truly-external writes (scratch dirs, memory files) still
show their paths. `--project` and `--repo` are mutually exclusive.

## Receipts (JSON export)

`git receipts export` runs the same pipeline as `audit` and emits the result
as machine-readable JSON — the interchange artifact you can commit beside
the code, feed to another program, or hand to a model to interpret. Far
smaller than the raw session log it distills.

- Same facts as the verbose report: per-commit statement, ledger, residue,
  commands, MCP calls, intents, token estimate, provenance, exceptions,
  blast radii, identity roll-up. **Every number matches the console and
  HTML** — three renderings of one receipt.
- Versioned schema (`schema_version`); pretty by default, `--compact` for a
  single line.
- Same scoping switches as `audit`: `--filter`, `--commit`, `--no-prompt` /
  `--no-summary` / `--no-intent` / `--no-identity` for a receipt you can
  share — counts always stay.
- `--full` is the maximal export: the whole chat transcript plus every
  command's output. Output is included for **failed** commands by default
  (a failure's output is the one you always want); `--with-output` includes
  all of it, capped at 64 KB each.

v0.1 reads Claude Code session logs (the JSONL under `~/.claude/projects/`).
Sessions are found for the repo *and its parent directories* — launching the
agent in a monorepo root or a container directory works — and with no
`--repo`, the target is inferred from where the session's claims point
(ambiguity refuses to guess and names the candidates). There is no local
session archive: logs live in the store until its retention cleanup removes
them, so commits older than your oldest surviving session show as unclaimed
keyframes. The event model is deliberately harness-neutral; adapters for
other agent CLIs are on the roadmap.

## Why git — and where it stops

For a coding agent, **the deliverable is code, and code lives in git.**
Everything else — reading, searching, building, testing, `curl`-ing a doc,
installing a dep — is scaffolding toward that code. git isn't a corner of
the work; it's the part that survives the session. Most side-effects leave
their durable form in git too: a deploy is ephemeral, but the script that
ran it is a commit.

Where it stops, honestly:

- **Runs of committed code.** CI/CD workflows, Terraform/Pulumi, test
  suites, scripted deploys. The *source* is committed and audited. The
  *run* — the pipeline, the `apply`, the test execution — happens
  off-process, and is shown as blast radius plus captured output. Not proof.
  - Declarative IaC is the best case: the git source *is* the intended
    state.
  - A committed test suite is source *and* a re-runnable verifier. But git
    can witness only that a test **landed** — never that it **passed**
    (self-reported output), never that it's **meaningful** (a committed
    `assert!(true)` lands fine). Landing is git's to prove; green-ness and
    quality are the reviewer's.
- **The irreducible tail.** One-shot side-effecting commands: a manual
  `curl -X POST`, a `psql UPDATE`, an `aws s3 cp`. Never scripted, nothing
  in git. No post-hoc audit of git can *ever* verify these. The tool flags
  them by blast radius — a review signal, honestly not a receipt.
- **Data as the deliverable.** git doesn't hold the dataset (it lives in a
  store or a `.gitignored` dir; git-lfs keeps a pointer). But real data
  work is *scripted* — a Python job, a dbt model, a notebook — and that
  pipeline **is** code in git, fully audited. The tool verifies the process,
  not the dataset. The one true blind spot: un-versioned, ad-hoc munging —
  the habit good engineering avoids anyway.

And the deepest limit sits on the *claims* side, not git's:

- The repo is shared, durable, complete. **Session logs are local,
  ephemeral, partial.** They expire, rotate, vanish with a wiped machine —
  and you never have teammates' logs at all.
- So gitreceipts **verifies the claims it has, period.** *Absence of a
  claim is not absence of action — it's absence of a log.*
- "broken promises: 0" means zero *among the claims you gave it*, not
  across the repo's whole history. A commit with no session log is never
  guessed at — it's named by its git author and marked *not audited*.
- That asymmetry is why git is the right anchor: check the fragile side
  (claims) against the durable, shared side (git) — never the reverse.

The operational gaps — things the tool *could* verify but doesn't yet — are
cataloged in [KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md).

## Tests, QA, and our own dogfood

A tool that audits agents should show its receipts. Three layers:

**Tests.** 100+ unit and end-to-end tests, heavy on the adversarial cases
the reconciliation has to survive: amend collapse, git's second-granular
timestamps, forged/anomalous clocks, multi-commit Bash calls, partial
staging, files relocated before their first commit, a teammate's commit
landing mid-window, hostile HTML in prompts, home-path redaction.

**Cross-format reconciliation QA.** A harness runs the shipped binary
across every CLI switch on real repos and checks *invariants*, not golden
files: exit codes, JSON parses, HTML is self-contained, no home-path
leaks — and the core promise, that **console, HTML, and JSON render
identical numbers**, verified to the digit. It has caught real bugs before
every release; nothing ships while any number disagrees. It ships in
[`qa/`](qa/) — point it at your own repos.

**Dogfood.** gitreceipts was built by a coding agent and audited with
itself, alongside every other agent-driven project on this machine: **14
sessions · 11 repos · 5 projects · ~221k LOC across Swift, Rust, Svelte,
Astro, and Tauri · 106 days · 3 models · 836 agent commits · 1,719 file
claims — 96% landed in git, every exception diagnosed.** The full corpus
table, including the per-project numbers and what the 50 broken promises
turned out to be, is in [RECEIPTS.md](RECEIPTS.md).

**Release provenance.** Every release tarball is built by a public GitHub
Actions run (tests execute per-target) and carries a **build-provenance
attestation** cryptographically linking it to the exact commit and workflow
run that produced it:

```bash
gh attestation verify git-receipts-*.tar.gz -R cloudcraft-ai/gitreceipts
```

Checksums ship beside the binaries (`SHA256SUMS`). The same standard we
hold agents to — don't trust the claim, verify the receipt — applies to us.

## Teams

The audit is deliberately per-developer: your sessions, reconciled against
the repo you work in. Teammates' commits (anything that arrived by pull
rather than being created locally) enter the spine from history and show as
*unclaimed keyframes* — correctly attributed as "not this agent's work,"
never blamed or ignored. Each developer runs their own audit; aggregating
them into a team-wide ledger is a later layer, not a v0.1 concern.

## What the report contains — and what it doesn't

Everything runs locally; nothing leaves your machine.

- gitreceipts reads exactly one directory — your **Claude Code projects
  directory** (`~/.claude/projects` by default; override with
  `--store <dir>`). It never opens anything else under `~/.claude` — not
  your settings, your prompt history, or your MCP auth caches.
- The report prints repo-relative paths (your home directory collapses to
  `~`), branch names, commit subjects, and — because intent matters — the
  prompts you typed, attached to the commits they drove.
- Each commit block is bookended: the first prompt as **intent** at the
  top, the agent's own **summary** at the bottom — the readable claim,
  closing the loop the ledger verifies. The summary is the agent's word,
  not proof.
- Suppression is granular and keeps every count: `--no-prompt` drops your
  prompts, `--no-summary` the agent's prose, `--no-intent` both — e.g.
  before sharing a report. A built-in secret/PII scanner masks API keys,
  tokens, and validated PII by default; `--redact <word>` masks anything
  else you name.
- Session files are treated as untrusted input: paths from the log never
  reach git as options, traversal is rejected, and a multi-gigabyte line
  won't take down the process.

## Development

```bash
cargo test                                  # unit + scenario + end-to-end suites
cargo clippy --all-targets -- -D warnings   # warnings are errors here
cargo fmt
```

The pipeline is five stages, one module each: `ingest` (tolerant JSONL) →
`causal` (parent-chain ordering) → `extract` (claims and receipts) →
`reconcile` (the interval equation) → a renderer. `audit` and `export`
share the pipeline and differ only at the end, which is why every headline
number matches across console, HTML, and JSON. Scenario tests build real
throwaway git repos and synthetic session logs, so every honesty rule above
is pinned by a test that would catch its regression.

## Status

Early and moving fast. The engine is exercised daily against large real
sessions (100+ MB, 200+ commit spines, ~1s wall time), but the report
format and CLI surface may still change before 0.2.

## License

MIT.
