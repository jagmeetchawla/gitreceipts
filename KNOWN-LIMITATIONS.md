# Known limitations

The tool's honesty is the product, so its gaps are documented with the same
care as its features. These are the places where today's verdicts still need
human judgment, and where we've deliberately deferred a fix rather than ship a
clever guess. (The deeper boundary — what git can and cannot witness at all —
is in the README's "Why git — and where it stops.")

## 1. ~~Red conflates three kinds of "never landed"~~ — LARGELY FIXED in 0.1.1

v0.1.0's red covered *deliberate scratch* (a file written, used, and thrown
away on purpose), *build-tool churn* (Astro, Wrangler, npm regenerating a
file between the agent's edit and the commit), and *genuine loss*. Only the
last is a broken promise in the trust sense.

0.1.1 splits them. A claim whose path **never entered any commit** and is
gone from disk resolves as *scratch churn* — shown, with its reason, as a
grey finding; never red. Genuine loss (the file IS in history, the claimed
content reached no commit) stays red. Two related false-red classes went
with it: **cross-repo attribution** (one session driving sibling repos used
to bill each repo for the others' edits — 48 false reds on a real four-repo
corpus, now 1 genuine one) and a **path-level late-landing check** that had
regressed into treating any later touch of the same path as "landed".

0.1.1 also stopped **formatters** from breaking a landing. `cargo fmt`
rewrapping a call and adding a trailing comma between the write and the
commit used to leave the claimed bytes unfindable, so a kept promise read
as broken. Verification now tries the exact bytes, then the bytes with
whitespace removed, then a single small local difference — identical
except in one place.

What remains: **a generator that substantially rewrites the file is still
red** when the path exists in history and the agent's content was replaced
before the commit. The diagnosis says so ("a tracked file, but this edit's
content reached no later commit"), but telling "a build step regenerated
it" from "the edit was lost" needs evidence we don't collect — and
loosening the match far enough to cover it would start laundering real
losses. Read the diagnosis before reading the red as a lie; the errors
here skew toward false alarms, never toward hiding.

## 2. Uncommitted work in the working tree reads as red

A claimed edit whose content is on disk **but never committed** (and not
gitignored) is diagnosed "on disk right now, still uncommitted" and counted
broken. The work is real; it just lives outside committed history, and the tool
has no first-class "landed in the working tree" state yet. Common first victim:
a `.gitignore` that was written but never committed.

## 2b. A repo is the folder you are in — never the one above it

`git receipts` looks for `.git` in the folder you name (or are standing in)
and nowhere else. Unlike `git status`, running from a subdirectory does not
walk up: `cd src && git receipts` is an error, not an audit of the repo two
levels above. This is deliberate — the target is always something you can
see from where you stand, and the tool never picks a repo for you — but it
will catch people used to git's behaviour. The error says what to do. See
[docs/WHAT-GETS-AUDITED.md](docs/WHAT-GETS-AUDITED.md).

## 3. Relocation detection is bounded

A file written at one path and moved elsewhere *before its first commit* is
detected (content-verified, same filename). But a file that was moved **and
renamed**, or moved **and further edited** past probe recognition, still reads
as red. Same safe direction: a false alarm, not a hidden miss.

## 4. ~~Other people's commits can be counted, never explained~~ — FIXED in 0.1.3

Until 0.1.3 the spine was built from the session's time window alone, and
`agent_committed` was stamped **positionally** — the next N commits after a
`git commit` command. A colleague's commit, or a merge from a pull, landing
inside that range was marked as the agent's: its unexplained files became
*your* residue and its interval entered *your* verdict.

A commit is now yours only when git records you as **author or committer**,
by **name or email**, honouring `.mailmap`. Other people's commits are held
out; `--all-authors` includes them.

What remains true: attributing *what someone else did* would need **their**
session logs, which you don't have. Their commits are shown and counted,
never explained.

**With no git identity configured, there is no filter.** If `user.name` and
`user.email` are both unset, nothing can be matched and the run behaves like
`--all-authors` — every commit counts as yours, stated plainly in the report
header. On such a machine a colleague's commits would be counted as the
agent's, which is what identity exists to prevent.

**And one thing git genuinely cannot tell you.** Every coding agent commits
under *your* git identity — Claude runs `git commit` with your
`user.name`/`user.email`, and so does every other tool. So git cannot
separate an agent's commit from one you typed yourself. Only a session log
can, and only for the sessions a given run loaded. That's why unclaimed
changes are reported as *"your identity, outside this session"* rather than
"you" — that bucket holds your hand-written commits, other agent sessions,
and other tools alike, and nothing on record distinguishes them.

## 5. The claims side is only as complete as your logs

git's record is durable and shared; session logs are local, partial, and
subject to store retention. Absence of a claim is not absence of work — commits
older than your oldest surviving session show as unclaimed keyframes, honestly
labeled, not blamed.

## 6. Honest for reasonable use, not adversary-proof

Timestamps and reflog order can be forged by someone determined to fool their
own audit. The tool guards against accidents (clock anomalies, amends, rebases)
— not against deliberate self-deception. Point a session at an unrelated repo
and you get a wall of red: the tool correctly saying "these claims have nothing
to do with this repo."

## 7. ~~No man page~~ — FIXED in 0.1.1

git intercepts `--help` for subcommands and looks for a man page, which
v0.1.0 didn't ship — so plain `git receipts --help` failed. 0.1.1 generates
the pages from the CLI definition itself (they can't drift from `--help`)
and installs them from the tarballs and the brew formula. Installing by
`cargo install` ships no man page — cargo has no man-install step — so on
that path run `git-receipts man --install` once: it writes the pages to
the first writable directory on your MANPATH (and tells you what to add
to MANPATH if there isn't one).

## 8. Cross-machine / mounted-drive audits are NOT supported in v0.1

Auditing sessions recorded on a **different machine** — a mounted backup
drive, a copied `~/.claude/projects`, a second computer — is **not a
supported flow in v0.1**. First-class cross-machine support is **on the
roadmap** (a `--map-root OLD=NEW` to remap the original machine's paths
explicitly); until it ships, treat everything below as unsupported
behavior, not a contract. 0.1.1 did tighten one sharp edge: the name-based
store fallback now matches the **repo's own directory name only**, so a
path component like `/data/dev` no longer "matches" an unrelated repo whose
name merely ends in `-dev`.

What breaks today, concretely:

- **Repo inference fails.** With no `--repo`, the tool infers the target from
  the session's recorded working directories — the *original machine's*
  paths, which don't exist locally.
- **The store-lookup fallback can silently merge the wrong sessions.** With
  no exact store match, sessions are found by directory-*name* matching, and
  a generic path component (`dev`, `src`) can pull in an unrelated project's
  sessions — polluting prompt/effort numbers. Every fallback match prints a
  `matched … by name` note; if you see one naming an unrelated project, the
  merge is wrong.
- **`--project` sibling protection does not engage** (original-machine claim
  paths never match local sibling roots), so the "sibling paths withheld"
  privacy guarantee lapses. Treat any cross-machine `--project` export as
  private.

In our own testing, *single-repo* audits with an explicit `--repo` produced
full-fidelity results cross-machine (claims resolve through the session's own
recorded roots and content-verify against the local clone) — which is why the
roadmap item is confidence-backed. But "worked in our testing" is not
"supported": if you try it anyway, name the repo explicitly, read every
`matched … by name` note, replicate the original absolute paths when you can
(a symlink to the mount is enough), preserve exact directory casing, and keep
the outputs private.

## 9. One harness today

v0.1 reads Claude Code session logs. The event model is deliberately
harness-neutral; adapters for other agent CLIs are on the roadmap — the
reconciliation itself never depended on who wrote the log.

## 10. No way to list or select one session — the multi-lane gap

By default gitreceipts merges **every** session for a repo into one stream.
That is right for solo work: it avoids the trap where your *other* sessions'
commits look like another contributor's.

Run several agents in parallel and the same default blends the lanes
together. There is no `git receipts sessions` to list what's available with
each session's own summary, and no way to select one by name — isolating a
lane today means identifying its session file yourself. `--this-session`
only finds the session you are *in*; `--latest` only finds the newest.

This is the top roadmap item for the multi-agent workflow described in the
README.

## 11. No time scope — you cannot ask "what changed since I last looked"

Scope is per-repo, per-session, or per-commit; there is no `--since`. The
oversight question after a day away is *what moved since yesterday*, and
today the repo-wide default answers it by showing everything, oldest
findings included, with recent work no more prominent than months-old
history. `--latest` narrows to one session, which is not the same question.

## 12. `genuine` is a residual failure class, not a diagnosis

Every failed command is triaged — expected-nonzero, guarded, retried-and-passed,
user-abort, sandbox-denial, or **`genuine`**. That last one is what's left when
no known-benign pattern matched, and its evidence string says so
(*"unclassified failure — its captured output is attached"*). The **name**
reads like a positive finding of a real problem, which is not what it means.

In one real audit, 132 of 143 failed commands landed there. Reading their
captured output, the actual mix was: a command run from the wrong working
directory, commands the harness blocked, a genuine scripting bug, real network
failures during DNS propagation, cleanup of paths that didn't exist — and at
least one **harness cancellation that should have been `user-abort`**.

Treat `genuine` as *"we could not classify this — read the output"*, which is
what the export's `failure_evidence` is for. Renaming it, and adding the
classes that are cheap to detect, is the top item on the roadmap.

## 13. Files written by shell commands aren't always attributed

A file the agent creates through the Write/Edit tools is a claim. A file it
creates by running `cp`, `sips`, `convert`, or a script is not — the ledger
never sees it, so it surfaces as unexplained residue.

Command attribution catches many of these already: if the path appears as a
whole token in a command that ran in the same interval, it's attributed. What
it cannot catch is a path that **never appears in the command at all** — a
Python script that writes `favicon-16.png`, a build tool that fills a
directory.

This matters because agents shell out constantly for anything binary or bulk,
so the noise scales with how much real work the agent did — the worst shape a
false positive can have. Note the flip side, which is the feature working: a
file a *human* dropped into the repo and the agent committed is also residue,
and should stay visible.

## 14. Project-mode exports repeat session-level events per repo

When one session drives several repos under `--project`, each repo's section
of the JSON export carries that session's command runs. The same command can
therefore appear in several repos' sections, because it genuinely falls
between two commits in each of them.

The rendered output is unaffected — per-repo and per-commit figures are
correct. But **a consumer summing a field across repos will over-count**: in
one real audit, one repo showed 34 failed commands while naively summing all
four gave 143.

Two rules for anyone parsing the export:

- **Don't sum across repos** in project mode without deduplicating.
- **Don't derive broken-promise counts from `ledger[].landing`.** Use
  `summary.broken_promises`. A never-landed claim may carry a `resolution`
  (superseded, deliberately removed, gitignored, scratch churn), and resolved
  claims are not broken promises.
