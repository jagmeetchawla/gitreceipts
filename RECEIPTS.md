# Receipts — gitreceipts, audited by gitreceipts

gitreceipts was built by an AI coding agent. So we pointed the tool at its own
construction and let it reconcile what the agent *claimed* against what git
actually *recorded*, commit by commit.

## The receipt

```
window   2026-07-23 → 2026-07-27  (~4 days)
events   5,990 kept / 11,431 log lines

75 commits · 206 file-edit claims · 772 commands · 202 prompts

balance  69 green · 6 residue-only · 0 red  of 75 intervals   (92% green)
claims   206 / 206 landed in git                              (100%)
broken   0 broken promises
```

**Every one of the 206 file edits the agent said it made landed in the git
history — zero broken promises.** The 6 residue-only intervals are unclaimed
changes git recorded (command fallout and human edits), attributed automatically
rather than flagged: 85 of the residue files were accounted for by a command that
ran, 1 was dismissed as gitignored, leaving a handful of honest "changed, no edit
claim" notes. Of 772 commands, 22 exited non-zero — surfaced as *facts*, not
manufactured into red (a failed `grep` means the test passed; the tool never
cries wolf).

## Reproduce it

The proof isn't these numbers — it's that you can run the same audit on **your**
agent's work and see for yourself:

```sh
# from inside any repo a coding agent has worked in
git receipts audit --all              # the whole spine, every session merged
git receipts audit --latest --oneline # just the most recent session
git receipts export --all > receipt.json   # the versioned JSON receipt
```

git is the oracle. The tool doesn't take the agent's word for anything — it
checks each claim against the commit history the agent can't fabricate after the
fact.
