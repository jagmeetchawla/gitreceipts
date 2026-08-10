# What gets read

One rule decides what `git receipts` reports on — recap, audit and export
alike:

> **`.git` in this exact folder, or the tool asks. Never upward, never a
> guess.**

If the tool can't name the target with certainty, it stops and tells you
what to pass. It will not pick a repo for you.

## Two definitions

Everything follows from these:

- **Repo** — a folder containing `.git`. A folder holds at most one, so a
  repo is always exactly one folder.
- **Project** — a folder that holds **one or more repos** as subfolders,
  each with its own `.git`. That is the only way several repos can sit
  together, and it is what `--project` audits. A project folder has no
  `.git` of its own and is never audited itself.

**Sessions are keyed by the folder they ran in.** The store holds one
directory per working directory, named after its full path. Looking up a
repo's sessions encodes that folder **and each of its ancestors**, keeping
every store directory that exists:

```
~/work/acme/api      → …-work-acme-api    ✅  sessions run inside the repo
~/work/acme          → …-work-acme        ✅  sessions run from the project
~/work               → …-work             ✗   no such directory
```

Both sets are merged, which is why work you drove from the project folder
still reconciles against the repo it touched. Matching is exact per
ancestor — not a string prefix. (A loose suffix match exists only as a
cross-machine fallback, when no exact ancestor matched at all; see
[KNOWN-LIMITATIONS.md](../KNOWN-LIMITATIONS.md) §8.)

## Assumptions

1. **A repo is the unit of an audit** — one repo, one reconciliation
   against one git history.
2. **You work in one repo at a time.** That is the default and the common
   case, so a bare `git receipts audit` means "this repo".
3. **`--project` is for a project folder** as defined above. If your work
   lives in a single repo, you never need the switch.
4. **We do not model how you organize your code.** Beyond those two
   shapes, you name the target explicitly and the tool audits exactly
   that. It never infers a scheme.
5. **Sessions are keyed by the path they were recorded at.** A repo that
   has moved, been renamed, or was audited on another machine may have no
   sessions in your store.
6. **Git decides who you are and what to ignore.** We add no configuration
   of our own; see below.

## Git's own settings are inputs to the verdict

gitreceipts is a git tool, so it reads git's configuration rather than
inventing its own. Two settings you already maintain change what the
report says.

**`user.name` / `user.email` — whose commits count.** A commit is yours
when git records you as its **author or committer**, matched by **name or
email**, with `.mailmap` honoured. Either side, either field: a squash-merge
that lists the forge as committer, a rebase of someone else's patch, or an
old address under the same name all still resolve to you.

Loose on purpose. Matching too loosely costs you a stray commit inside your
own audit; matching too tightly *silently drops your own work*. One repo in
our dogfood carries three emails under one name — email-only matching would
have claimed 59 of 242 commits and discarded the rest as somebody else's.

Every report prints the identity it used and how much of the window it
covered (`191 of 204 commits are yours`), names identities it skipped, and
**refuses to print at all** if nothing matches — an audit that filtered
everything out is indistinguishable from one that found nothing wrong.
Override with `--me <name|email>` (repeatable) or `--all-authors`.

**No identity at all** — `user.name` and `user.email` both unset, common on
a machine you only pull to — makes the run **equivalent to
`--all-authors`**: nothing can be matched, so everything is treated as
yours, and the report says so rather than implying a filter ran. The
multi-contributor protection is off in that state; setting the config, or
passing `--me`, restores it.

What git *cannot* tell you: every coding agent commits under your identity,
so an agent's commit and one you typed are identical to git. Only a session
log separates them, and only for the sessions a run loaded.

**`.gitignore` — what git was never going to take.** A claimed edit to an
ignored path that never lands is not a broken promise; it is git doing what
you configured it to do. Those are reported as **explained findings** with
the reason attached. `.git/info/exclude` and your global excludes count too,
because `git check-ignore` consults them all.

`.gitignore` is the right home for this because it **costs something to
abuse**: you cannot quietly add `src/` to it without git actually ignoring
your source. A private config only this tool reads would carry no such
cost, and could be tuned into a permanently green report.

## The two layouts

The tool doesn't try to understand how you organize your code. It knows
one fact — a repo is a directory with `.git` — and supports two shapes:

**A repo.** Stand in it.

```
~/code/myapp/          ← .git here
└── src/               ← a subdirectory is not the repo

cd ~/code/myapp && git receipts
```

**A container of repos.** Not a repo itself; holds several. Use
`--project`.

```
~/work/acme/           ← no .git of its own
├── api/   .git
├── web/   .git
└── ops/   .git

cd ~/work/acme && git receipts --project
```

**Anything else — name it.** Deeper nesting, odd groupings, repos scattered
across unrelated folders: pass `--repo <dir>` or `--project <dir>` and the
tool audits exactly what you pointed at. It will not go looking, and it
will not infer a layout you didn't state.

(One detail worth knowing: discovery stops at the first `.git` it finds, so
submodules and vendored trees stay part of their parent repo rather than
becoming separate project members.)

## The default: a repo

```bash
cd ~/code/myapp
git receipts                # reads myapp
```

The folder you're in must **be** a git repo. Unlike `git status`, running
from a subdirectory does not walk up:

```bash
cd ~/code/myapp/src
git receipts
# Error: …/myapp/src is not a git repo.
#        Run this from a git repo, or name one with --repo <dir>.
#        To audit every repo under a folder: --project <dir>
```

That's deliberate: the target is always something you can see from where
you stand.

A folder that isn't a repo is always an error, even when exactly one repo
sits inside it. The error names what it found, so the next command is
obvious — but it never picks for you:

```
Error: ~/work/acme is not a git repo. It holds 4 repos (api, docs, ops, web)
       — name one with --repo <dir>, or audit them all with --project …
```

## Naming a repo: `--repo`

```bash
git receipts audit --repo ~/code/myapp
git receipts audit --repo            # value optional: means "this folder"
```

Same rule, applied to the folder you named: it must contain `.git` itself.

## Several repos at once: `--project`

The deliberate switch for a folder that holds repos — a public repo beside
private ops repos, a workspace, a monorepo-of-repos:

```bash
git receipts audit --project              # this folder (the value is optional)
git receipts audit --project ~/work/site  # or name one
```

It discovers every repo underneath (5 levels deep, never descending into a
repo it has already found), audits each against its own sessions, and
prints a roll-up plus a section per repo.

Repos that exist under the folder but have **no sessions** in your store
still appear in the roll-up, marked `no sessions in the store`. They are
part of the project; a table that quietly omitted them would look complete
without being complete.

## Which sessions

Target selection (above) and session selection are independent. Once the
target is known:

| | |
|---|---|
| *(default)* | **all** sessions recorded for that repo — the complete picture |
| `--latest` | just the most recent one |
| `--this-session <marker>` | the live session, found by identity |
| `<file.jsonl> …` | exactly the session files you name |

Sessions are matched by **the path they were recorded at**, so a repo that
has moved — or was audited on another machine — may have no sessions here.
See [KNOWN-LIMITATIONS.md](../KNOWN-LIMITATIONS.md) §8.

## Why no guessing

Earlier versions inferred a repo when the working directory wasn't one, by
counting which repo the session's file claims pointed into most. It was
convenient and it was wrong in the case that mattered: standing in a
workspace of four repos, it produced a confident report about one of them
and never mentioned the rest.

A tool whose whole claim is "don't trust the agent's word, check the
record" cannot itself quietly decide what you meant.
