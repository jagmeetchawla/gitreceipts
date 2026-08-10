---
description: Read a Claude Code session as a story with the git-receipts CLI — what was asked, what the agent did, what landed, commit by commit. Use when the user asks what happened, what an agent did, to catch up on an unattended or overnight run, to recap a session or a commit, to understand work from a while ago, or after any long run. ALSO use to catch up on a lane or sub-agent the user did not watch, and after this conversation has been compacted — the receipt holds the prompt-to-commit chain the session no longer does.
---

# git receipts — read the session as a story

`git receipts` reconciles a Claude Code session log against the repo's git
history, then reads it back: each prompt you typed → the work it drove →
the commit it became. The checking always runs (git is the record the
agent can't rewrite); recap just doesn't lead with the verdict.

## Prerequisite: the binary

```bash
command -v git-receipts >/dev/null && git-receipts --version || echo MISSING
```

If MISSING, **offer** — never install unasked, never refuse either.
Detect what's available (`command -v brew; command -v cargo`) and ask:

- **brew** → `brew install cloudcraft-ai/tap/gitreceipts` (preferred)
- **cargo** → `cargo install gitreceipts` (then `git-receipts man --install`
  once, so `git receipts --help` works — cargo installs no man page)
- **both** → ask which they prefer
- **neither** → the prebuilt, checksum-verified binary:

  ```bash
  cd "$(mktemp -d)"
  TAG=$(curl -fsSL https://api.github.com/repos/jagmeetchawla/gitreceipts/releases/latest | python3 -c "import json,sys; print(json.load(sys.stdin)['tag_name'])")
  case "$(uname -s)-$(uname -m)" in
    Darwin-arm64) T=aarch64-apple-darwin;;
    Darwin-x86_64) T=x86_64-apple-darwin;;
    Linux-x86_64) T=x86_64-unknown-linux-musl;;
    *) T="";;
  esac
  A="git-receipts-$TAG-$T.tar.gz"
  curl -fsSLO "https://github.com/jagmeetchawla/gitreceipts/releases/download/$TAG/$A"
  curl -fsSLO "https://github.com/jagmeetchawla/gitreceipts/releases/download/$TAG/SHA256SUMS"
  (shasum -a 256 -c --ignore-missing SHA256SUMS 2>/dev/null || sha256sum -c --ignore-missing SHA256SUMS) | grep "$A"
  tar xzf "$A" && mkdir -p ~/.local/bin && mv "git-receipts-$TAG-$T/git-receipts" ~/.local/bin/
  ```

  **The checksum line must print `OK`** — never proceed past a failed one.

If `which -a git-receipts` shows more than one path, say which one wins and
that the others are shadowed — an older copy earlier on PATH silently
decides everything below.

## Version floor — and offering the upgrade

This skill drives views that shipped in **binary 0.1.1**, and the identity
guidance below (the `you:` header, `--me`, `--all-authors`) needs **0.1.3**. If
`git-receipts --version` reports older, say so ONCE, offer the upgrade for
the route they actually installed by, and carry on with the fallback in
"Older binaries" — never block the answer they asked for.

Detect the route rather than guessing (a cargo copy can sit behind a brew
one on PATH):

```bash
which -a git-receipts
brew list gitreceipts >/dev/null 2>&1 && echo "route: brew"
[ -x "$HOME/.cargo/bin/git-receipts" ] && echo "route: cargo"
```

| Route | Offer |
|---|---|
| brew | `brew update && brew upgrade gitreceipts` |
| cargo | `cargo install gitreceipts --force` (then `git-receipts man --install`) |
| prebuilt binary | re-run the download block above — it always fetches the latest release |

Phrase it as an offer, once per conversation, and run it only on an
explicit yes — the same rule as the install. Something like: *"this is
binary 0.1.0; recap arrived in 0.1.1, and identity in 0.1.3. Want me to run `brew upgrade
gitreceipts`? I can read the session either way."*

## Reading a session

The default command is the whole point — one command, one screen:

```bash
git receipts recap --no-pager --color never
```

That prints the headline (commits · prompts · claimed files landed), names
any commits worth a look, then one entry per commit: **the ask on top,
what it became underneath**. Present it as a code block — it is
pre-aligned — and let it speak. Don't re-narrate what the table already
says; add only what it can't: which of these the user probably cares about
right now, and why.

Scopes, all composable:

| The user says | Run |
|---|---|
| nothing / "what happened" | `git receipts recap` (all sessions for this repo) |
| "this session" | `git receipts recap --this-session "$MARKER"` (below) |
| "yesterday's run", "the last one" | `git receipts recap --latest` |
| "what happened in commit X" | `git receipts recap --commit <hash>` |
| "all my repos here" | `git receipts recap --project` |
| "more detail" | add `--verbose` (every commit, in full) |
| "include my own commits" | add `--full-history` |

**"This session"** needs identity, not a guess: echo a unique marker into
the conversation first, then let the binary find the file that contains it.

```bash
M="receipts-$$-$(date +%s)"; echo "$M"; sleep 1
git receipts recap --this-session "$M" --no-pager --color never
```

If it reports no session contains the marker, the log hasn't flushed —
wait a second, retry once, then fall back to `--latest` and say so.

**Whose commits.** The header says which git identity the report covers and
how much of the window it matched — `you: Ada <ada@example.com> (191 of 204
commits are yours)`. A shrinking ratio means the repo's `user.email` isn't
the one that made the commits; `--me <name|email>` adds another.

**Who this is for.** An agent with its context intact remembers more than
any recap can reconstruct — recap is for whoever wasn't there. That's the
user, and it is also this session **after compaction**, when the receipt
hands back a prompt-to-commit chain the conversation no longer holds.

## Interpreting for the user

- **Lead with the story, not the score.** What was asked, what happened,
  what landed. The numbers support the sentence; they aren't the sentence.
- **Never hide a finding.** If the headline names commits worth a look,
  say what they are and offer the audit. A recap that omits problems is a
  prettier memoir, which is the failure this tool exists to catch.
- The marks are muted on purpose: `·` means something is noted (a failed
  command, a file written and discarded), `!` means unexplained residue,
  `✘` means a claimed edit git never got. Findings are *named* in the
  entry — quote the tool's own words rather than inventing a diagnosis.
- **Iteration is the story, not noise.** Drafts superseded, files
  relocated before their first commit, approaches written and thrown
  away, a commit amended seconds later — the audit keeps these quiet
  because they're resolved; a recap is exactly where they belong.
- Never soften a red. It means nothing on record explains the claim — and
  the user may know something the log doesn't, so present it as a
  question.

## Going deeper

Always end the first recap with the ways in, one line, no menu:

> **`recap <hash>`** for one commit's whole story · **`report`** for a page
> you can keep · **`audit`** for the verdict.

When they take one:

```bash
git receipts recap --commit <hash> --no-pager --color never   # the story
git receipts audit --summary --emoji --no-pager               # the verdict
```

For a page — generate and open in one step, scope-matched to what you just
read, and use `--compact` so it opens anywhere (a full report is several
hundred KB and chat preview panes choke on it):

```bash
F="${TMPDIR:-/tmp}/gitreceipts-recap.html"
git receipts recap <scope args> --format html --compact > "$F" && open "$F"
```

(`xdg-open` on Linux.) State the path. It lands OUTSIDE the repo on
purpose — a reading tool must not leave files its own next run would flag.
No browser (SSH, headless)? Still write it, give the path, say why it
didn't open.

Machine-readable twin, unchanged and complete: `git receipts export >
receipt.json`.

## Privacy

Reports are built from the user's own prompts and command output — treat
every one as private. Whenever you suggest sharing or exporting, name the
flags in the same breath: `--no-intent` (drops prompts and agent prose;
every count stays), `--no-identity` (drops names and emails), `--redact
<word>` (masks anything else). Home paths are masked automatically.
**Never pass `--no-scan`** — the secret/PII scanner stays on, always.

## Guardrails

- Never install the binary unasked; offer, and let the user choose.
- Never `--no-scan`.
- Never load a full report into the conversation — the console views are
  already condensed; HTML and JSON are files humans open.
- Pass through explicit flags the user gives verbatim, but only if they
  look like flags, paths, or a commit hash — never shell syntax.

## Older binaries (before 0.1.1)

`recap`, `--summary`, `--this-session` and `--compact` don't exist there.
Say an upgrade improves this, then fall back:

```bash
git-receipts audit --oneline --no-pager --color never   # the closest view
git-receipts audit --latest --no-pager                  # one session
```

For "this session", find the file by marker yourself and pass it as an
argument: `grep -rl "$M" ~/.claude/projects/*/ | head -1`.
