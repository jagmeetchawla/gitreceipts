<p align="center">
  <br>
  <strong style="font-size: 1.5em;">git receipts</strong>
  <br><br>
  Read what your agent <b>actually</b> did — every prompt → commands → MCP → files → commit,
  <br>checked against the one record the agent can't rewrite: <b>git</b>.
  <br><br>
  A <b>Claude Code plugin</b> with a deterministic Rust CLI at its core —
  <br>ask <i>"what happened here?"</i> and read the session as a story you can trust.
  <br><br>
  <a href="#install">Install</a> ·
  <a href="#1-in-claude-code--the-plugin">Plugin</a> ·
  <a href="#2-read-it--git-receipts">Recap</a> ·
  <a href="#3-check-it--git-receipts-audit">Audit</a> ·
  <a href="#4-keep-it--pages-and-data">Export</a> ·
  <a href="https://github.com/jagmeetchawla/gitreceipts/releases">Releases</a> ·
  <a href="RECEIPTS.md">RECEIPTS.md</a> ·
  <a href="KNOWN-LIMITATIONS.md">Known limitations</a> ·
  <a href="docs/WHAT-GETS-AUDITED.md">What gets read</a> ·
  <a href="docs/COMPAT.md">Compatibility</a> ·
  <a href="qa/">QA harness</a>
  <br><br>
</p>

**Developer velocity is no longer the bottleneck. Comprehension velocity is.**
An agent can produce a day's worth of commits in an hour. Understanding what
happened is now the slow part. The raw session log is tens of megabytes of
JSON nobody reads.

`git receipts` turns that log into an account you can read at reading
speed: each prompt you typed → the commands and MCP calls it made → the
files it touched → the commit that resulted. An 11,000-event session
becomes a spine of commits you scan in a minute and expand only where
something looks off.

And you can trust the account, because it's checked, not narrated. The
session log is the agent's own story. git kept an independent record of what
actually persisted — a log the agent can't fabricate after the fact.
gitreceipts reads the two side by side and reconciles them, claim by claim,
content-level against the actual commit blobs. You're not reading the
agent's memoir. You're reading a checked one. Not the agent's word. Git's.

That checking is always running; `git receipts audit` is where it takes
the headline — every claim accounted for, and the number that matters:
**broken promises**, claims git never got that nothing explains.

It follows **git's own conventions**, and adds no configuration of its own.
Git decides whose commits are yours — `user.name` / `user.email`, matched on
author or committer, `.mailmap` honoured — and `.gitignore` decides which
paths git was never going to take, so a claimed edit to one of those is an
explained finding rather than a broken promise. Both are settings you
already maintain, and every report states the identity it used and how much
of the window it covered. See
[your commits, not the whole repo](#your-commits-not-the-whole-repo).

The account is also **yours**. Everything runs locally. It reads exactly one
directory (your session logs) plus your repo, and sends nothing anywhere. A
mirror for your own work, not a badge to show anyone. Details in
[what the report contains — and what it doesn't](#what-the-report-contains--and-what-it-doesnt).

## Install

**Claude Code plugin** — the fastest way in
([what it does](#1-in-claude-code--the-plugin)):

```
/plugin marketplace add cloudcraft-ai/claude-plugins
/plugin install gitreceipts@cloudcraft
```

**The CLI** — the engine the plugin wraps, and a full standalone tool (no
AI required). The plugin needs it on your PATH and will point you here:

```bash
# Homebrew
brew install cloudcraft-ai/tap/gitreceipts

# crates.io (Rust 1.90+)
cargo install gitreceipts

# prebuilt binaries (checksums + provenance attestations attached)
# → https://github.com/jagmeetchawla/gitreceipts/releases

# from source
git clone https://github.com/jagmeetchawla/gitreceipts
cd gitreceipts
cargo install --path .
```

The binary is **`git-receipts`** (note the hyphen — that's what makes git
pick it up as a subcommand): run it as `git receipts …`, `git-receipts …`,
or plain `gitreceipts` (the brew install aliases the package name too;
`cargo install` gives only `git-receipts` — cargo convention, like
`ripgrep` → `rg` — symlink it yourself if you want the alias).

`git receipts --help`, `git receipts -h`, and `git-receipts --help` all
work: the release tarballs and the brew formula install a man page, so
git's own `--help` lookup finds it like any native subcommand. Installed
via `cargo install`? Cargo has no man-install step — run
`git-receipts man --install` once and git's `--help` works there too.

**Verified on macOS (Apple Silicon) and Linux (x86_64).** The Intel macOS
binary passes the full test suite as x86_64 code (under Rosetta 2 translation
in CI), but hasn't been hand-verified on Intel silicon — reports welcome.

## 1. In Claude Code — the plugin

The natural place to ask *"what did my agent actually do?"* is where the
agent is:

```
/plugin marketplace add cloudcraft-ai/claude-plugins
/plugin install gitreceipts@cloudcraft
```

Then just ask. **`/gitreceipts:recap`** reads the session back to you —
what you asked, what the agent did, what landed — and fires on its own
when you say things like *"catch me up"* or *"what happened in that
commit?"*. **`/gitreceipts:audit`** is there when the question is
*"did it all land?"*. Findings arrive **explained in the context of what
the session was trying to do**, not just flagged, and the full HTML report
is one word away (say "report").

<!-- SCREENSHOT: the ambient trigger firing — asking "what happened here?" and the recap answering -->

The division of labor is the point: the facts come from a deterministic,
local Rust CLI that Claude cannot sweet-talk, and Claude reads those facts
against what you asked for and explains what it finds. Neither alone gives
you that — an agent grading its own homework is exactly the problem this
tool exists to solve.

Six skills, each named for what you want rather than which flag it passes:

| | |
|---|---|
| `/gitreceipts:recap` | what happened here |
| `/gitreceipts:audit` | did every claimed edit land |
| `/gitreceipts:HTMLReport-Compact` | a page small enough to read inline |
| `/gitreceipts:HTMLReport-Full` | the complete record, in your browser |
| `/gitreceipts:JSONExport` | the data, written to a file |
| `/gitreceipts:help` | all of the above, with the switches |

The plugin wraps the CLI below and will guide you through installing the
binary if it's missing (it never installs anything for you). And the CLI is
a full standalone tool — all of it works with no AI in the loop.

## 2. Read it — `git receipts`

The default command answers *what happened here*. One command, one screen:

```bash
git receipts
```

```
168 commits · 479 prompts · 344/344 claimed files landed
worth a look: a3f6e63, 89dcd50, 376521c — `git receipts audit` for the verdict

  · 24ccff5 yeah commit and push
           ↳ Report intent per interval and an intent -> outcome summary · 3/3 landed
  · 566b051 let's do them as well. if we can't derive a meaning -- we say it so
           ↳ Close two honesty gaps: forged dates and laundered late landings · 1/1 landed
  ! a3f6e63 harden it against a hostile session file
           ↳ Harden against hostile session files · 3/3 landed · 1 residue
```

**Your ask on top, what it became underneath.** That inversion is the
whole idea: the commit message is the agent's summary of your request, so
leading with it buries the thing you actually remember. A long session
shows its findings of any age plus the recent tail; nothing is ever
truncated silently.

<!-- SCREENSHOT: `git receipts` on a real session — the narrative spine -->

Go deeper without changing tools:

```bash
git receipts recap --commit 6d6cdc4   # one commit's whole story
git receipts recap --verbose          # every commit, in full
git receipts recap --project          # a folder of repos
git receipts recap --this-session "$MARKER"   # exactly this live session
```

**In Claude Code** — same thing, said out loud:

```
/gitreceipts:recap            what happened here
"catch me up"                 fires the same skill on its own
"recap commit 6d6cdc4"        one commit's story
"what happened in this session"   the live one, matched by identity
```

<!-- SCREENSHOT: `recap --commit <hash>` — the ask, the conversation, what landed -->

Iteration is the story here, not noise. Drafts superseded, a file
relocated before its first commit, an approach written and thrown away, a
commit amended twelve seconds later — the audit keeps those quiet because
they're resolved. A recap is exactly where they belong.

## 3. Check it — `git receipts audit`

Same receipt, verification brought forward. The question is no longer
*what happened* but *did every claimed edit actually land*:

```bash
git receipts audit --summary --emoji
```

```
commits 168 (+1 by others held out) · 132 🟢 / 31 ⚪ / 5 🟡 / 0 🔴 · claims 344/344 · broken promises 0

    commit  subject                                       claims  findings
  ⚪ 24ccff5 Report intent per interval and an intent -> o   3/3  1 cmd failed (grep)
  🟡 a3f6e63 Harden against hostile session files            3/3  1 residue
```

**The verdict is what git can witness; everything else is a finding.**

| | | |
|---|---|---|
| 🟢 | **green** | nothing to report |
| ⚪ | **grey** | explained findings — a file written and discarded before any commit, a failed command, an errored MCP call. Each is shown *with* its explanation. |
| 🟡 | **amber** | **un**explained residue: git recorded a change no claim and no command accounts for. A loose end with no answer on record. |
| 🔴 | **red** | a broken promise: a claimed edit git never got, and nothing explains it. |

The difference between ⚪ and 🟡 is the point: grey means *we know why*,
amber means *nobody does yet*. `broken promises: 0` is earned, claim by
claim — and a red is a **question, not a conviction**. It means nothing on
record explains the claim; you may know something the log doesn't.

Here is an amber interval in full — and what makes it amber. Git recorded
eight files; four are **attributed** to a command that ran in this
interval, and one, `Cargo.lock`, is **residue**: changed, never claimed,
nothing naming it. That single unaccounted-for line is the whole
difference between 🟡 and ⚪:

![An amber commit drill-down: badges for committed-by-agent and pushed, model provenance, the 8 files git recorded, four commands tagged by blast radius, then one residue line (Cargo.lock, changed and never claimed) beside four attributed ones, and the agent's own summary labelled as its own words, not verified](docs/images/commit-drilldown.png)

*(No red to show you here — this repo has none. When there is one, it
arrives with its diagnosis attached and is presented as a question.)*

```bash
git receipts audit --filter red-amber   # just the unanswered ones
git receipts audit --exit-code          # 0 green/grey · 1 amber · 2 red — for CI
```

**In Claude Code:**

```
/gitreceipts:audit            the verdict, explained in context
"did all that actually land?"  fires it on its own
"just the problems"            the unanswered findings only
```

## 4. Keep it — pages and data

Every view renders three ways, from one receipt: the console, a
self-contained HTML page, and JSON. **The numbers are identical across all
three** — a QA harness checks that to the digit on every release, and
nothing ships while one disagrees.

**A page to keep or share.** Theme-aware, no external assets, works
offline:

```bash
git receipts recap --format html --compact > recap.html   # the story
git receipts audit --format html --compact > audit.html   # the verdict
```

**In Claude Code**, each artifact is a command, named for what it's for
rather than which flag it passes:

```
/gitreceipts:HTMLReport-Compact   a page small enough to read right here
/gitreceipts:HTMLReport-Full      the complete record, opened in your browser
/gitreceipts:JSONExport           the data, written to a file
```

`--compact` is the one to reach for: commits with unexplained findings keep
their full file and command lists, everything else collapses to its
headline, and long output is clipped — a few hundred KB becomes small
enough to open in a chat preview pane. Every cap says how much it hid and
points at the JSON, which stays complete. Without it you get the full
document; `--expand all|none` controls what starts open.

<!-- SCREENSHOT: the compact HTML report open in a browser -->

**The data.** `git receipts export` runs the same pipeline and emits the
reconciled result as JSON — the interchange artifact you can commit beside
the code, diff between runs, feed to another program, or hand to a model:

```bash
git receipts export > receipt.json
git receipts export --project > project.json
git receipts export --compact          # single line, for piping
```

(`/gitreceipts:JSONExport` does the same from Claude Code — always to a
file, never pasted into the conversation. A real receipt is megabytes.)

- Same facts as the report: per-commit statement, ledger, residue,
  commands, MCP calls, intents, token estimate, provenance, exceptions,
  blast radii, identity roll-up.
- Versioned schema (`schema_version`), **additive-only within 0.x** — new
  fields may appear, existing ones never change meaning. The full promise
  is in [docs/COMPAT.md](docs/COMPAT.md).
- Same scoping and privacy switches as the reading commands.
- `--full` is the maximal export: the whole transcript plus every
  command's output (failed commands' output is included by default).

**Before you share any of it:** these are built from your prompts and
command output. `--no-intent` drops the conversation and keeps every
count, `--no-identity` drops names and emails, `--redact <word>` masks
anything else. Home paths are masked automatically and a secret/PII
scanner runs by default.

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
  errored call marks the commit **grey**: a finding shown with its
  reason, never manufactured into a verdict. Your own aborts count as
  your stop, not the agent's failure.
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

The [amber interval above](#3-check-it--git-receipts-audit) is this
machinery in one frame: the statement (eight files git recorded), the
ledger (three claims, all landed), the commands that ran with their blast
radii, and then every leftover accounted for — four attributed to a
command, one left as genuine residue.

## Your commits, not the whole repo

gitreceipts is a git tool, so it asks **git** who you are and what git was
told to ignore. It introduces no config of its own — the two inputs below
are ones you already maintain, and `git log`, `git blame` and `git status`
read them the same way.

### Whose commits count — `user.name` / `user.email`

A commit is **yours** when git records you as its **author or committer**,
matched by **name or email**. Either side, either field, and `.mailmap` is
honoured — so an old address, a squash-merge that lists the forge as
committer, or a rebase of someone else's patch all still resolve to you.

That looseness is deliberate. Matching too loosely costs you a stray commit
inside your own audit; matching too tightly *silently drops your own work*.
One repo in our own dogfood carries three different emails under one name —
email-only matching would have claimed 59 of 242 commits and quietly
discarded three quarters of the author's history as someone else's.

Every report says which identity it used and how much of the window it
covers, so a wrong `user.email` shows up as a line you can read rather than
a smaller, cleaner-looking audit:

```
you: Ada Lovelace <ada@example.com>   (191 of 204 commits are yours)
     not counted as you: A. Lovelace <old@corp.example> — if one of those is
     also you, add it with --me, or map it in .mailmap
```

If **nothing** matches, gitreceipts refuses to print a report at all. An
audit that filtered everything out looks exactly like an audit that found
nothing wrong, and only one of those is true — so it names the identity it
resolved, tells you how to check it, and stops.

| | |
|---|---|
| `--me <name\|email>` | count another identity as you (repeatable) |
| `--all-authors` | include everyone's commits |
| `--full-history` | include your own commits this agent didn't make |

The last two are different axes and compose: `--full-history` opens **when**
(your hand-made commits, not just this session's), `--all-authors` opens
**who**. Opening one never opens the other, so a full-history run still
won't bill you for a colleague's work.

Holding other people's commits out is what makes gitreceipts usable when
you adopt agentic development **on top of a mature codebase**: you get an
account of *your agent's* work, not a wall of red for years of history it
never touched. A count of what was held out is always shown.

### What git was told to ignore — `.gitignore`

A path matched by `.gitignore` (or `.git/info/exclude`, or your global
excludes) is one git was **configured never to take**. So when a claimed
edit to such a path never lands, that is not a broken promise — it is git
doing exactly what you told it. Those are reported as **explained
findings**, with the reason attached, and never as a lie. An agent writing
into `dist/` or `target/` no longer produces a red the moment that output
is cleaned.

`.gitignore` is the right place for this precisely because it **costs
something to abuse**: you cannot quietly add `src/` to it without git
actually ignoring your source. A private config file that only this tool
reads would have no such cost, and could be tuned into a permanently green
report.

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

## Verifying a lane you can't see

The use this tool was *not* designed for, and may be the one it's best at.

Run several agents in parallel — a coordinating session relaying work
between an engine lane, an app lane, a design lane — and the coordinator's
job becomes passing along claims it cannot check. A lane reports *"done,
all green."* That gets relayed upward as fact. The only other way to
confirm it is to **ask the lane**, which is asking the party under review to
file its own report.

The lane writes its own summary. It does not write the git history.

That's the whole argument, and it's why the competition here isn't
"nothing" — it's the agent's own status report, which is always available,
far richer, and free. gitreceipts doesn't beat it on richness and never
will. It beats it on being **unfalsifiable**.

We ran this on our own multi-lane work: four parallel sessions on a stealth
macOS app, one coordinating three others across three worktrees. The
coordinator audited a lane it had been relaying for six weeks without ever
independently seeing, and the audit surfaced a **broken promise the lane
had no explanation for** — a claimed edit to a tracked file whose content
reached no later commit. That is the failure mode a status report
structurally cannot catch, because the same party writes both.

Two honest caveats from that run:

- **Auditing your own live session is the least useful case.** An agent
  with its context intact remembers more than any recap can reconstruct.
  Recap is for whoever *wasn't there* — which is you, and which is also the
  agent itself **after compaction**, when the receipts hand back a
  prompt→commit chain the session no longer holds. git can't rewrite it.
- **Picking one lane out of many is still manual.** By default gitreceipts
  merges every session for a repo, which is right for solo work and wrong
  here — it blends the lanes together. Isolating one currently means
  identifying its session file yourself. A first-class way to list and
  select sessions is the top item on the roadmap for this workflow.

## Where sessions come from

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
itself, alongside every other agent-driven project on this machine —
Swift, Rust, Svelte, Astro and Tauri: **924 agent commits · 1,703 file
claims — 98% landed in git, 5 broken promises, every one diagnosed.**

That number was 50 at launch. Most of those turned out to be the *tool's*
errors, not the agent's — claims from one repo billed against its
siblings, scratch files that no commit ever contained, a formatter
rewrapping a file between the write and the commit. Each was found by
this tool auditing its own history. The full accounting, per project and
per class, is in [RECEIPTS.md](RECEIPTS.md).

The self-audit of this very repo — every claim landed, `broken promises: 0`
earned, not asserted:

![gitreceipts auditing its own repo: 8 sessions merged over 17 days, 100% of claims landed (347/347), 0 broken promises, 41 commands with errors surfaced as facts, and the exception waterfall accounting for all 239 unclaimed changes — 232 attributed to a command the agent ran, 6 unexplained](docs/images/self-audit-section.png)

**Release provenance.** Every release tarball is built by a public GitHub
Actions run (tests execute per-target) and carries a **build-provenance
attestation** cryptographically linking it to the exact commit and workflow
run that produced it:

```bash
gh attestation verify git-receipts-*.tar.gz -R jagmeetchawla/gitreceipts
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
