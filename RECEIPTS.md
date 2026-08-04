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
the fact. (The wider dogfood corpus — 14 sessions, 11 repos, ~221k LOC across
five stacks — is summarized in the README's "Tests, QA, and our own dogfood.")
