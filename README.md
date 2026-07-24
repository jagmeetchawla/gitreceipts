# `git receipts` — see what your agent actually did.

**The name is the thesis: git is the oracle.** A coding agent tells you
what it did; git kept its own record of what actually persisted — a log
the agent can't fabricate after the fact. **gitreceipts** reads your
Claude Code session alongside the repo and reconciles the two, claim by
claim, against that record. Not the agent's word. Git's.

**When you'd reach for it** — two moments especially, both where you
can't just reconstruct what happened by hand:
- **Unattended runs.** Overnight autonomy, long agentic sessions,
  background or scheduled agents — you weren't watching, and you can't
  replay what you didn't see. You come back to a stack of commits and
  need a trustworthy account of what actually happened.
- **Looking back.** Work from a while ago, where your own memory has
  faded and the raw session — tens of megabytes, if it even survived —
  is impractical to trawl. git still holds the truth; gitreceipts
  distills what you *asked* and what the agent *did* into a receipt you
  can actually read.

Reconciliation runs both directions, and both get billing:
- **Claimed → didn't land** — the agent said it, git never got it. If
  nothing explains it, that's a **broken promise**.
- **Landed → not claimed** — git recorded a change no edit claim covers.
  The interesting question is *who*, and git answers it: the **author**
  and any **`Co-Authored-By:`** trailer on the commit. So the tool tells
  you, by difference, what *another contributor* (or another agent)
  changed versus what this session did — attribution for free.

What it **verifies** — because git can witness it:
- **File writes, edits, deletes** — the claimed content is checked
  against the actual commit blobs (content-level, not filename-level, so
  a coincidental change can't be mistaken for the claim landing).
- **Commits** — matched to the real commit graph and reflog.
- **Pushes** — checked against the remote-tracking refs.
- **Who** — every commit's author and declared co-authors, straight from
  git. Identity only: a name tells you *who committed*, never *how* (an
  agent or a human hand). A `Co-Authored-By` trailer is present-only
  evidence of an agent or a pair — never inferred from its absence.

What it **surfaces** — the rest of the story, honestly labeled:
- **Intent** — the prompts *you* typed, attached to the commit each one
  drove, so every commit reads *ask → what landed*.
- **Agent effort** — commands issued and an estimated token count
  (deduplicated per request; marked *estimated, not billing*).
- **Blast radius** — how far each command reached: `local-fs →
  local-git → remote-git → network`. This is the honest boundary:
  a network call or a deploy leaves nothing in git, so its captured
  output is shown as the agent's *own receipt* — never dressed up as
  proof. git receipts audits what git can prove, and flags what it can't.

The headline number is **broken promises**: a claim that never landed
and nothing explains. It is *earned*, claim by claim, not assumed —
verified against later commits, reconciled with the session's own
commands, checked against the working tree.

```
intent → outcome
  173 prompts drove 52 commits; 214/219 claimed files landed (98%), 49 intervals fully balanced
  agent effort: 412 commands · ~5.0M output tokens across 3,413 requests (est., not billing)
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

That last line above the identity block is the point. Zero is not
assumed — it is earned against the repo's own receipts.

## Why git — and where it stops

For a coding agent the **deliverable is code, and code lives in git.**
Everything else the agent does — reading, searching, building, testing,
`curl`-ing a doc, installing a dep — is scaffolding *toward* that code,
not the product. So git isn't a narrow corner of the work; it's the
output, the thing that survives the session. Even most side-effects
leave their durable form in git: a deploy is ephemeral, but the
pipeline or script that ran it is a commit. And the trend runs the
tool's way — GitOps, infra-as-code, config-as-code keep moving
deliverables *into* git.

Where it stops, honestly:
- **Side-effects whose source is still code** — CI/CD (the workflow
  YAML), infrastructure (the Terraform/Pulumi), an automated test
  suite, a scripted deploy or API client. The *source of truth* is
  committed and audited; only the *run* — the pipeline, the `apply`,
  the test execution, the request — happens off-process, shown as blast
  radius plus captured output, not proof. For declarative IaC the git
  source literally *is* the intended state. A committed test suite is
  the best case — it's not just source, it's a re-runnable verifier —
  but git can only witness that a test **landed**, never that it
  **passed** (that's self-reported output) or that it's **meaningful**
  (a committed `assert!(true)` lands fine): landing is git's to prove,
  green-ness and quality are the reviewer's.
- **The irreducible tail** — a one-shot side-effecting command: a
  manual `curl -X POST`, a `psql UPDATE`, an `aws s3 cp`, never
  scripted, leaving nothing in git. No post-hoc audit of git can *ever*
  verify these. The tool flags them by blast radius so you know
  *something reached outside* — a review signal, honestly not a receipt.
- **Data as the deliverable** — when the product is processed or
  synthesized *data*, git doesn't hold the data itself (it lives in a
  store, a database, or a `.gitignored` dir; git-lfs keeps only a
  pointer). But real data work is *scripted* — a Python job, a SQL/dbt
  model, a notebook — and that pipeline **is** code in git, fully
  audited. The data is a reproducible build-output of it, the way a
  binary is of source. So the tool verifies the process that produces
  the data; it doesn't verify the dataset — the same way it can't
  confirm a build's binary, only that you committed the build. The one
  true blind spot is *un-versioned, ad-hoc* data munging — the
  non-reproducible habit good engineering avoids anyway.

And the deepest limit of all — the one on the *claims* side, not git's.
The repo is shared, durable, complete; **session logs are local,
ephemeral, and partial.** You have only your own agent's logs, and even
those expire, rotate, or vanish with a wiped machine — teammates' and
other harnesses' logs you don't have at all. So gitreceipts **verifies
the claims it has, period.** *Absence of a claim is not absence of
action — it's absence of a log.* "broken promises: 0" means zero *among
the claims you gave it*, not across the repo's whole history. A commit
with no session log is never guessed at — it's named by its git author
and marked *not audited*. That asymmetry is also why git is the right
anchor: you check the fragile, suspect side (claims) against the durable,
shared side (git) — never the reverse.

## How it works

Think of it as bank reconciliation for agent work:

- **The statement** — the repo's real commits form the spine. Each
  commit's diff is the statement for the interval that produced it.
  Commit history is the primary source — the one thing every repo
  has, clone or original. When the local reflog exists it enriches
  the spine with evidence history cannot carry: amended drafts and
  reset-away commits (objects that never reached a ref), true
  creation order, and backdating detection — a commit created
  *between* two in-window commits stays in the audit no matter what
  its dates claim, because dates are forgeable and creation order is
  not. On a repo with no useful reflog you simply get the history
  spine: the full equation, verifications, and resolutions all run;
  only the reflog-borne stories are absent.
- **The ledger** — the session log's own tool calls are the claims.
  File edits carry their exact content (the diff is in the log); shell
  commands carry a blast radius (local-fs → local-git → remote-git →
  network) and, at best, captured output as a receipt.
- **The equation** — per interval, claims are matched against the
  statement. Green when it balances. Everything else is itemized,
  investigated, and labeled with its evidence:

| finding | meaning |
|---|---|
| `✓ landed late` | the claim's content was found in a later commit's blob — cites which, and how many commits later |
| `◌ never landed, resolved` | superseded by a later landed edit · removed deliberately by an on-record command · or persisted on disk outside git's view |
| `● residue (attributed)` | changed without a file-edit claim, but named by a command that ran (`git mv`, a `sed` loop) — with rename provenance |
| `○ residue dismissed` | the path is gitignored or untracked *today* — retroactively declared noise by you |
| `✘ never landed` | a claim that never landed **and nothing explains it** — the only thing that turns an interval red |
| `↻ amend` | a draft commit superseded seconds later ("committed 2,603 files, amended 12s later" is a story worth telling) |
| `⚠ clock anomaly` | a commit whose dates disagree with its place in the reflog — kept in the audit, flagged as untrustworthy |

The operating principle throughout: **when the evidence is ambiguous,
the report says "ambiguous" — it never picks the flattering
interpretation.** Verification is content-level (claimed bytes checked
against later blobs), not name-level, so a coincidental change to the
same file cannot launder a lost claim.

## Install

```sh
# from source (Rust 1.90+)
git clone https://github.com/cloudcraft-ai/gitreceipts
cd gitreceipts
cargo install --path .
```

The binary is `git-receipts`, which git picks up as a subcommand:
`git receipts …`. Publication to crates.io and a Homebrew tap are
planned.

## Usage

```sh
# audit the most recent session for the repo you're in
git receipts audit --latest

# audit every session the store still has for this repo, merged into
# one ledger (forked sessions deduplicate; concurrent sessions union)
git receipts audit --all

# audit a specific session log against a specific repo
git receipts audit ~/.claude/projects/<project>/<session>.jsonl --repo ~/code/myapp

# just the broken promises
git receipts audit --latest --filter red

# broken promises + unexplained residue
git receipts audit --latest --filter red-residue

# on a terminal the report auto-pages through $PAGER (like git),
# colors intact — no flags needed. To opt out:
git receipts audit --latest --no-pager

# force colors through your own pipe (bat, less -R, a saved transcript)
git receipts audit --latest --color=always | less -R

# suppress quoted prompt text before sharing a screenshot
git receipts audit --latest --no-intent

# write a self-contained HTML report (theme-aware, no external assets)
git receipts audit --latest --format html > audit.html

# audit another machine's sessions from a mounted drive
git receipts audit --all --store /Volumes/studio/Users/me/.claude --repo /Volumes/studio/Users/me/code/myapp

# export the reconciled audit as a versioned JSON receipt
git receipts export --latest > receipt.json
```

## Receipts (JSON export)

`git receipts export` runs the same pipeline as `audit` and emits the
result as machine-readable JSON — the same facts the verbose report
shows (per-commit statement, ledger, residue, commands, intents) plus
the header context, token estimate, evidence grades, blast radii, and
the git-identity roll-up. Every headline number matches the report.

This is the interchange artifact: a compact, verified receipt you can
commit beside the code, feed to another program, or hand to a model to
interpret — far smaller than the raw session log it distills. The
schema is versioned (`schema_version`), pretty-printed by default
(`--compact` for a single line), and `--no-intent` drops the quoted
prompt text while keeping every count, for a receipt you can share.

v0.1 reads Claude Code session logs (the JSONL under
`~/.claude/projects/`). Sessions are found for the repo *and its parent
directories* — launching the agent in a monorepo root or a container
directory above the repo works — and with no `--repo`, the target is
inferred from where the session's claims point (ambiguity refuses to
guess and names the candidates). Note there is no local session
archive: logs live in the store until its retention cleanup removes
them, so commits older than your oldest surviving session will show as
unclaimed keyframes. The event model is deliberately harness-neutral;
adapters for other agent CLIs are on the roadmap.

## Teams

The audit is deliberately per-developer: your sessions, reconciled
against the repo you work in. Teammates' commits (anything that
arrived by pull rather than being created locally) enter the spine
from history and show as *unclaimed keyframes* — correctly attributed
as "not this agent's work" rather than blamed or ignored. Each
developer runs their own audit; aggregating them into a team-wide
ledger is a later layer, not a v0.1 concern.

## What the report contains — and what it doesn't

Everything runs locally; nothing leaves your machine. The report
prints repo-relative paths (your home directory collapses to `~`),
branch names, commit subjects, and — because intent matters — the
prompts you typed, attached to the commits they drove. Use
`--no-intent` to drop the quoted text while keeping the counts, e.g.
before sharing a report. Session files are treated as untrusted
input: paths from the log never reach git as options, traversal is
rejected, and a multi-gigabyte line won't take down the process.

## Development

```sh
cargo test                                  # unit + scenario + end-to-end suites
cargo clippy --all-targets -- -D warnings   # warnings are errors here
cargo fmt
```

The pipeline is five stages, one module each: `ingest` (tolerant
JSONL) → `causal` (parent-chain ordering) → `extract` (claims and
receipts) → `reconcile` (the interval equation) → `report`. Scenario
tests build real throwaway git repos and synthetic session logs, so
every honesty rule above is pinned by a test that would catch its
regression — including the adversarial ones (backdated commits,
option-injection filenames, laundered path matches).

## Status

Early and moving fast. The engine is exercised daily against large
real sessions (100+ MB, 200+ commit spines, ~1s wall time), but the
report format and CLI surface may still change before 0.2.

## License

MIT.
