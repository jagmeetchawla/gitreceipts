# Receipts — gitreceipts, audited by gitreceipts

gitreceipts was built by an AI coding agent. So we pointed the shipped binary at
its own construction and let it reconcile what the agent *claimed* against what
git actually *recorded*, commit by commit. Numbers as of **v0.1.1**
(2026-08-09); rerun the command below for the current ones.

## The receipt

```
window   2026-07-23 12:25Z → 2026-08-09 22:35Z   (17d 10h, 8 sessions)
events   13,573 kept / 26,755 log lines

169 commits · 347 file-edit claims · 2,061 commands · 479 prompts

balance  133 of 169 intervals fully balanced             (79%)
claims   347 / 347 landed in git                        (100%)
broken   0 broken promises
```

**Every one of the 347 file edits the agent said it made landed in the git
history — zero broken promises.** 14 of those landed a commit or two late and
were content-verified against the commit they landed in.

Everything else is a finding, and each one says what it is. Git recorded 239
changes no edit claim covered: **232 are accounted for** by a command the
agent ran, 1 was dismissed as since-gitignored, and **6 are unexplained** —
the only ones that colour an interval amber. 41 of 2,061 commands exited
non-zero, 7 of them the author stopping the agent rather than the agent
failing; those are grey, never red. One in-window commit wasn't the agent's
and is held out of the equation, counted in the header.

479 prompts drove those 169 commits — which is the other half of the point:
the receipt is not only a check, it is the fastest way to read back what
seventeen days of agent work actually was.

## Reproduce it

The proof isn't these numbers — it's that you can run the same audit on
**your** agent's work and see for yourself:

```sh
# from inside any repo a coding agent has worked in
git receipts                          # what happened — every session merged
git receipts audit                    # …and did every claimed edit land?
git receipts export > receipt.json    # the versioned JSON receipt
```

git is the oracle. The tool doesn't take the agent's word for anything — it
checks each claim against the commit history the agent can't fabricate after
the fact.

## The full dogfood corpus

gitreceipts wasn't only pointed at itself. Every agent-driven project on the
author's machine, re-audited under v0.1.1 (two are pre-announcement and appear
under stealth labels; the numbers are real):

| Project | Stack | Agent commits | Claims landed | Broken |
|---|---|---|---|---|
| a stealth macOS app | Swift/SPM · 3 worktrees | 273 | 477/493 | 3 |
| ClipBob (menu-bar app) | Swift/SPM | 204 | 481/490 | 1 |
| **gitreceipts** (this tool) | Rust | 173 | 347/347 | 0 |
| a private ops repo | Markdown/Shell | 108 | 105/105 | 0 |
| four Astro/Wrangler sites | Astro | 96 | 166/168 | 1 |
| rustic-playground | Rust + Tauri + Svelte | 70 | 97/100 | 0 |
| **Total** | five stacks | **924** | **1,673/1,703 (98%)** | **5** |

Verdicts across the corpus: **752 green · 128 grey · 39 amber · 5 red** of 924
agent commits.

### The number that moved: 50 broken promises → 5

At launch this table said **50**. The corpus has grown since, and the count
fell by 90% — so it is worth saying exactly why, because "our bad number got
better" is the one claim a tool like this cannot ask you to take on faith.

Four of every five of those 50 were **the tool's fault, not the agent's**:

- **~40 were cross-repo attribution.** A session driving several sibling
  repos in one turn had its claims counted against every one of them, so
  each repo was billed for edits that landed in its neighbour. Claims now
  attribute by absolute-path containment; the four Astro sites went from 48
  broken to 1.
- **A cluster were scratch churn** — files written, used, and discarded
  before any commit ever contained them. Nothing that was ever in git was
  lost, so they resolve as grey findings with their reason attached rather
  than counting as promises broken.
- **One was a formatter.** A test file the agent wrote, which `cargo fmt`
  rewrapped and gave a trailing comma before it was committed one commit
  later. Verification looked for the exact bytes, found none, and called a
  kept promise broken.

The remaining **5 are real** and each carries its own diagnosis — a tracked
file whose claimed content reached no later commit, which is exactly what red
is for. What changed is not the standard; it is that red now means only that.
The tool that found these four false-positive classes was this tool, auditing
its own history — every one surfaced as an unexplained red that turned out to
have an explanation the engine could see and wasn't using.


### Dogfooded across four parallel agent lanes

The corpus above is one agent at a time. The stealth macOS app is also run
the other way: **four concurrent sessions across three worktrees**, one
coordinating the other three.

The coordinating session audited a lane it had been relaying for six weeks
and had never independently seen — a transcript far too large to read. The
audit returned a **broken promise the lane had no explanation for**: a
claimed edit to a tracked file whose content reached no later commit. Not a
false positive of the kind this page catalogues above; a real one, of the
class red exists for.

Its own read of the exercise, worth recording because it cuts both ways:
auditing the session it was *living in* told it almost nothing it didn't
already know, and auditing the lane it *couldn't see* was the first
independent view it had ever had of that work. An agent holding its own
context does not need a recap; the human does, and so does the agent once
it has been compacted.

Two limits that run exposed, both open: isolating one lane out of several
is still manual (the default merges every session for a repo — right alone,
wrong in parallel), and there is no "what changed since I last looked".

The harness behind these numbers ships in [`qa/`](qa/) — run it on your own
repos.
