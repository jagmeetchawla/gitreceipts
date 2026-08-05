# Known limitations

The tool's honesty is the product, so its gaps are documented with the same
care as its features. These are the places where today's verdicts still need
human judgment, and where we've deliberately deferred a fix rather than ship a
clever guess. (The deeper boundary — what git can and cannot witness at all —
is in the README's "Why git — and where it stops.")

## 1. Red conflates three different kinds of "never landed"

A **broken promise** (red) today covers: *deliberate scratch* (a file written,
used, and thrown away on purpose), *build-tool churn* (Astro, Wrangler, npm and
friends regenerating a file between the agent's edit and the commit — the write
was real; tooling rewrote it), and *genuine loss* (a claimed edit that simply
vanished). Only the last is a broken promise in the trust sense. The
`diagnosis` field already distinguishes the cases; the *color* does not yet.
Planned refinement: scratch and churn become **amber** ("worth a look"), red
stays reserved for genuine unexplained loss. Until then, read a red's diagnosis
before reading it as a lie — the errors here skew toward false alarms, never
toward hiding.

## 2. Uncommitted work in the working tree reads as red

A claimed edit whose content is on disk **but never committed** (and not
gitignored) is diagnosed "on disk right now, still uncommitted" and counted
broken. The work is real; it just lives outside committed history, and the tool
has no first-class "landed in the working tree" state yet. Common first victim:
a `.gitignore` that was written but never committed.

## 3. Relocation detection is bounded

A file written at one path and moved elsewhere *before its first commit* is
detected (content-verified, same filename). But a file that was moved **and
renamed**, or moved **and further edited** past probe recognition, still reads
as red. Same safe direction: a false alarm, not a hidden miss.

## 4. Other people's commits can be counted, never explained

By default the verdict covers only the agent's own commits; concurrent commits
by teammates (or your own hand) are **held out** with an honest count —
attributing them would require *their* session logs, which you don't have.
`--full-history` includes them as unclaimed keyframes, attributed by git
identity only.

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

## 7. No man page yet — `git receipts --help` doesn't work

git intercepts `--help` for subcommands and looks for a man page, which v0.1.0
doesn't ship — so plain `git receipts --help` fails. Use `git receipts -h` or
`git-receipts --help` (both print full help). A generated man page ships in
the next version, at which point `git receipts --help` opens proper docs like
any native git command.

## 8. Mounted-drive / other-machine audits: single repo is full-fidelity, `--project` sibling protection is not

Auditing sessions recorded on a **different machine** (a mounted backup drive,
a copied `~/.claude/projects`, a second computer) is supported and works better
than you might expect — with one sharp edge.

**What works, fully.** A **single-repo audit** cross-machine has the same
fidelity as a local one. Claims resolve through the session's *own recorded
working directories* (validated against the repo's git history), then
content-verify against your local clone — so landings, late landings, broken
promises, verdicts, and all three formats reconcile exactly. Verified on a
Linux box auditing a macOS-recorded corpus: identical claim-landing numbers.

```bash
git receipts audit --store /Volumes/backup/Users/me/.claude/projects --repo ~/anywhere/myapp
```

The repo can live at any local path; the store can be a mount or a copy.

**The sharp edge: `--project` sibling protection does NOT engage
cross-machine.** Sibling collapse works by matching out-of-repo write paths
against the *local* paths of the sibling repos. Claims from the original
machine carry *that* machine's absolute paths, which never prefix-match your
local sibling roots — so writes into a sibling repo stay listed as raw
original-machine paths instead of collapsing to a count. The audit is still
correct; the **privacy guarantee** ("sibling paths withheld") is what lapses.
**Treat a cross-machine `--project` export as private.**

**Workarounds:**

1. **Recreate the original absolute path** and sibling protection engages
   normally — a symlink is enough:

   ```bash
   # claims were recorded under /Users/alice/Developer on the original Mac
   sudo mkdir -p /Users/alice
   sudo ln -s /Volumes/backup/Users/alice/Developer /Users/alice/Developer
   git receipts audit --project /Users/alice/Developer/myproject --store /Volumes/backup/Users/alice/.claude/projects
   ```

2. Or generate shareable per-repo exports **on the original machine**, and use
   the cross-machine audit for your own (private) review only.

**One more copying gotcha:** preserve **exact directory casing**. macOS
filesystems are case-insensitive and will hide a `Sites` vs `sites` mismatch
from you; the path matching is exact, and on Linux the difference is real.
Copy with tools that preserve names as-is (rsync does; retyping paths by hand
may not).

**Two more path-dependent conveniences that degrade cross-machine:**

- **Bare-command repo inference.** With no `--repo`, and run outside a git
  repo, the tool infers the target from the session's recorded working
  directories — which are the *original machine's* paths. If they don't exist
  locally, inference fails with "pass --repo". Workaround: `cd` into the
  actual repo (nothing to infer), or name it — `--repo <dir>` /
  `--project <dir>`.
- **The name-suffix store fallback can over-match.** When no exact store
  directory matches, sessions are found by matching *every ancestor
  directory's name* — and a generic ancestor like `dev` or `src` can match an
  unrelated project (observed: a copy under `/data/dev/...` pulled in a
  `something.dev` site's sessions). The tool prints a
  `matched … by name` note for every fallback match — **read those notes**;
  an unrelated project there means foreign sessions are being merged in and
  the effort/prompt numbers are polluted. Workarounds: keep the copied layout's
  directory names distinctive, or pass the session `.jsonl` file(s)
  explicitly. v0.1.1 tightens the fallback to the repo's own name.

A first-class `--map-root OLD=NEW` (rewrite claim roots at ingest) is on the
roadmap to make all of this explicit.

## 9. One harness today

v0.1 reads Claude Code session logs. The event model is deliberately
harness-neutral; adapters for other agent CLIs are on the roadmap — the
reconciliation itself never depended on who wrote the log.
