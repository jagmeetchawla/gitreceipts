# Receipts — gitreceipts, audited by gitreceipts

gitreceipts was built by an AI coding agent. So we pointed the shipped binary at
its own construction and let it reconcile what the agent *claimed* against what
git actually *recorded*, commit by commit. Numbers as of v0.1.0, launch day
(2026-08-04); rerun the command below for the current ones.

## The receipt

```
window   2026-07-23 → 2026-08-04  (~12 days)
events   10,139 kept / 19,501 log lines

110 commits · 318 file-edit claims · 1,282 commands · 309 prompts

balance  86 green · 24 amber · 0 red  of 110 intervals   (78% green)
claims   318 / 318 landed in git                         (100%)
broken   0 broken promises
```

**Every one of the 318 file edits the agent said it made landed in the git
history — zero broken promises.** 14 of those claims landed a commit or two
late and were content-verified against the commit they landed in. The 24 amber
intervals are "worth a look," not lies: unclaimed changes git recorded (93 of
them attributed to a command the agent ran, 2 dismissed as since-gitignored)
plus commands that exited non-zero — 27 of 1,282, surfaced as *facts*, not
manufactured into red (a failed `grep` often means the test passed; the tool
never cries wolf). One in-window commit wasn't the agent's and is held out of
the equation, counted honestly in the header.

## Reproduce it

The proof isn't these numbers — it's that you can run the same audit on
**your** agent's work and see for yourself:

```sh
# from inside any repo a coding agent has worked in
git receipts audit                    # the whole spine, every session merged
git receipts audit --latest --oneline # just the most recent session
git receipts export > receipt.json    # the versioned JSON receipt
```

git is the oracle. The tool doesn't take the agent's word for anything — it
checks each claim against the commit history the agent can't fabricate after
the fact.

## The full dogfood corpus

gitreceipts wasn't only pointed at itself. Launch-morning snapshot across every
agent-driven project on the author's machine — **14 sessions · 11 repos · 5
projects · ~221k LOC · 106 days · 3 models** (two projects are pre-announcement
and appear under stealth labels; the numbers are real):

| Project | Stack | LOC | Sessions | Agent commits | Claims landed | Broken |
|---|---|---|---|---|---|---|
| a stealth macOS app | Swift/SPM · 3 worktrees | ~115k | 7 | 278 | 489/497 | 1 |
| ClipBob (menu-bar app) | Swift/SPM | ~9k | 2 | 204 | 483/490 | 1 |
| **gitreceipts** (this tool) | Rust | ~13k | 1 | 102 | 310/310 | 0 |
| a private ops repo | Markdown/Shell | ~6k | 1 | 88 | 98/98 | 0 |
| rustic-playground | Rust + Tauri + Svelte | ~39k | 2 | 70 | 98/100 | 0 |
| four Astro/Wrangler sites | Astro | ~39k | 2 | 94 | 173/224 | 48 |
| **Total** | five stacks | **~221k** | **14** | **836** | **1,651/1,719 (96%)** | **50** |

Verdicts across the corpus: **682 green · 142 amber · 12 red** of 836 agent
commits — and **every one of the 50 broken promises carries a diagnosis** (25
scratch files written-and-discarded, 24 overwritten by build tooling before
landing, 1 file still sitting uncommitted on disk). No mystery reds. The
largest red cluster (the Astro sites) is build-tool churn — exactly the
known-limitation the taxonomy refinement will move to amber
(`KNOWN-LIMITATIONS.md` §1).

The harness behind these numbers ships in [`qa/`](qa/) — run it on your own
repos.
