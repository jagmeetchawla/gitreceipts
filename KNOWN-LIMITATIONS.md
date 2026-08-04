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

## 7. One harness today

v0.1 reads Claude Code session logs. The event model is deliberately
harness-neutral; adapters for other agent CLIs are on the roadmap — the
reconciliation itself never depended on who wrote the log.
