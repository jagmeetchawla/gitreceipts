# What gets audited

One rule decides what `git receipts` reports on:

> **`.git` in this exact folder, or the tool asks. Never upward, never a
> guess.**

If the tool can't name the target with certainty, it stops and tells you
what to pass. It will not pick a repo for you.

## Assumptions

Stated plainly, because everything below follows from them:

1. **A repo is a directory containing `.git`.** That is the unit of an
   audit — one repo, one reconciliation against one git history.
2. **You work in one repo at a time.** That is the default and the common
   case, so a bare `git receipts audit` means "this repo".
3. **`--project` is only for a multi-repo container** — a folder that is
   not itself a repo, holding several. If you don't have that layout, you
   never need the switch. The container itself is never audited; it has no
   history.
4. **We do not model how you organize your code.** Beyond those two
   shapes, you name the target explicitly and the tool audits exactly
   that. It never infers a scheme.
5. **Sessions are keyed by the path they were recorded at.** A repo that
   has moved, been renamed, or was audited on another machine may have no
   sessions in your store.

## The two layouts

The tool doesn't try to understand how you organize your code. It knows
one fact — a repo is a directory with `.git` — and supports two shapes:

**A repo.** Stand in it.

```
~/code/myapp/          ← .git here
└── src/               ← a subdirectory is not the repo

cd ~/code/myapp && git receipts audit
```

**A container of repos.** Not a repo itself; holds several. Use
`--project`.

```
~/work/acme/           ← no .git of its own
├── api/   .git
├── web/   .git
└── ops/   .git

cd ~/work/acme && git receipts audit --project
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
git receipts audit          # audits myapp
```

The folder you're in must **be** a git repo. Unlike `git status`, running
from a subdirectory does not walk up:

```bash
cd ~/code/myapp/src
git receipts audit
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
