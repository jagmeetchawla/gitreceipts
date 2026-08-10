# Compatibility promise

gitreceipts is pre-1.0 and moving, but the parts other software depends on
are governed. This page says exactly which parts, and what we promise about
each — so a script, a CI gate, or an agent skill can be written against this
tool without waiting for 1.0.

## The JSON receipt is an API

`git receipts export` is the interchange surface. Everything else — console
layout, HTML, wording — is presentation and may change in any release.

**Within 0.x, schema changes are additive.** New fields may appear; existing
fields keep their names, types, and meaning. A field is never repurposed to
mean something new. Removals and renames wait for a major version.

**Every shape change bumps `schema_version`'s minor — including additive
ones** — so you can feature-detect instead of probing for fields. Schema
`0.8` (shipped with 0.1.1) added `balance.grey`, the ledger's `scratch`
flag, and per-command `failure_class` / `failure_evidence`.

`0.9` (shipped with 0.1.3) added `summary.commits_total`,
`summary.commits_mine`, `summary.identity` (`described` / `known` /
`all_authors` / `matched_nothing`), and split unclaimed changes into
`unclaimed_other_contributor` and `unclaimed_yours_outside_session`. It
REMOVED `exceptions.created_elsewhere`, which measured how far back the
local reflog reached rather than where a commit came from — the one
removal so far, made because keeping a field that cannot mean what its
name says is worse than the break.

Every receipt carries `schema_version`. Consumers should:

- **ignore unknown fields** — new ones will show up;
- **treat optional fields as optional** — several are omitted rather than
  emitted null (`resolution`, `diagnosis`, `landed_at`, `failure_class`, …),
  so read them defensively;
- **not depend on field order or on JSON formatting** (`--compact` exists
  for machines).

Numbers are the strongest guarantee: **console, HTML, and JSON always carry
identical values for every headline and exception count.** A QA harness
checks this to the digit on every release, and nothing ships while a number
disagrees — the three surfaces are three renderings of one receipt.

## Verdict semantics

The four marks — green, grey, amber, red — mean what
[the README's legend](../README.md) says they mean:

> The verdict is what git can witness; everything else is a finding.

Changes to what a mark *means* are treated as breaking, announced in the
release notes with a before/after on a real corpus, and never made quietly.
This happened once already and is the standard for how it happens again:
**0.1.1 split v0.1.0's amber** into amber (unexplained residue — a verdict
matter) and grey (explained findings — a failed command, an errored MCP
call, a file written and discarded before any commit). Audits that were
amber under 0.1.0 for process reasons alone are grey under 0.1.1. Broken
promises — the headline number — kept their meaning exactly.

## Exit codes

The default exit status reports whether the **program ran**, not what it
found: 0 on success, non-zero when the audit itself failed. This will not
change; scripts written against it stay correct.

`--exit-code` opts in to verdict-carrying status:

| exit | meaning |
|---|---|
| 0 | green or grey — the equation balances |
| 1 | amber — unexplained residue |
| 2 | red — a broken promise |

A red-only gate tests `>= 2`. Grey never raises the exit code: explained
findings are findings, not failures. New verdict levels, if any ever exist,
extend upward rather than renumbering these.

## CLI surface

Flags are additive within 0.x. If a flag must change, the old spelling keeps
working as a deprecated alias for at least one minor release, and the
deprecation is in the release notes. Output *layout* is not covered — parse
the JSON, not the console. (`--summary` exists precisely so a machine or an
agent gets a stable condensed view without screen-scraping the full report.)

## Plugin ↔ binary versions

The Claude Code plugin wraps this CLI and is versioned separately. Each
plugin release documents its **binary floor** — the oldest `git-receipts`
it supports — in its marketplace entry. The plugin probes
`git-receipts --version` and degrades gracefully when the installed binary
predates a feature it would prefer to use, rather than failing.

| plugin | binary floor | notes |
|---|---|---|
| 0.1.x | 0.1.0 | extracts the condensed view from `export` JSON (schema 0.7) |
| 0.2.0–0.2.1 | 0.1.1 | uses `recap`, `--summary`, `--emoji`, `--this-session`, `--compact`; on an older binary it says so, offers the upgrade for the route you installed by, and falls back to the 0.1.x path |
| 0.2.2+ | 0.1.1 (identity guidance wants 0.1.3) | reads the `you:` identity header and can suggest `--me` / `--all-authors`; still runs against 0.1.1, simply without that surface |

The plugin ships no binary and never could — it is markdown that invokes
whatever `git-receipts` is on your PATH. So the two version independently
and the coupling is this floor, not a lockstep: plugin 0.1.1 through
0.1.13 all ran against binary 0.1.0. A plugin never fails because the
binary is old; it degrades, says so once, and offers the upgrade.

## What is explicitly *not* promised

- Console and HTML layout, wording, colors, and column widths.
- Performance characteristics.
- The exact text of a diagnosis or resolution line (the *categories* are
  stable; the prose may be sharpened).
- Anything documented in [KNOWN-LIMITATIONS.md](../KNOWN-LIMITATIONS.md) as
  a current gap — those are expected to change, that's the point.
