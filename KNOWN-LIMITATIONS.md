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

## 7. ~~No man page~~ — FIXED in 0.1.1

git intercepts `--help` for subcommands and looks for a man page, which
v0.1.0 didn't ship — so plain `git receipts --help` failed. 0.1.1 generates
the pages from the CLI definition itself (they can't drift from `--help`)
and installs them from the tarballs and the brew formula. Installing by
`cargo install` still gets no man page — cargo has no man-install step —
so on that path use `git receipts -h` or `git-receipts --help`.

## 8. Cross-machine / mounted-drive audits are NOT supported in v0.1

Auditing sessions recorded on a **different machine** — a mounted backup
drive, a copied `~/.claude/projects`, a second computer — is **not a
supported flow in v0.1**. First-class cross-machine support is **on the
roadmap** (a `--map-root OLD=NEW` to remap the original machine's paths
explicitly, plus stricter store matching); until it ships, treat everything
below as unsupported behavior, not a contract.

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
