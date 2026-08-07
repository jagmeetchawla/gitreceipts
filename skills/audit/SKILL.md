---
description: Audit a Claude Code session against git history with the git-receipts CLI — verify what the agent claimed vs. what actually landed, commit by commit. Use when the user asks what an agent actually did, whether a session's work really landed, to audit/verify/review an agentic session, to check for broken promises, or after a long unattended run.
---

# git receipts — audit the session against git

gitreceipts reconciles a Claude Code session log against the repo's git
history. The session log is the agent's own story; git is the independent
record it can't rewrite. The audit checks every file-edit claim against the
actual commit blobs and reports what landed, what landed late
(content-verified), what was benignly resolved, and what is a **broken
promise** — claimed, never landed, nothing explains it.

## Prerequisite: the binary

Check it exists before anything else:

```bash
command -v git-receipts || echo MISSING
```

If MISSING, don't install silently — but don't refuse either. Detect the
routes and **offer**; never answer MISSING with a bare "I won't install
it" or a bare URL:

```bash
command -v brew; command -v cargo
```

Preference order: **brew → cargo → prebuilt attested binary.**

- **brew available** → offer it: "The binary isn't installed — want me to
  run `brew install cloudcraft-ai/tap/gitreceipts`?" On yes, run it.
- **no brew, cargo available** → offer `cargo install gitreceipts`
  (needs Rust 1.90+; compiles from source, takes a minute).
- **both available** → ask which they prefer: brew (prebuilt, updates via
  `brew upgrade`) or cargo (builds from source). Run their choice.
- **neither** → offer the prebuilt, attested binary: downloaded, checksum-
  verified, installed to `~/.local/bin` (no sudo). On yes:

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

  **The checksum line must print `OK` — never proceed past a failed
  checksum.** When `gh` is available, also run
  `gh attestation verify "$A" -R jagmeetchawla/gitreceipts` (Sigstore
  provenance; passes for v0.1.1+ — v0.1.0 predates a repo transfer and its
  attestation lookup 404s, so for that release the checksum is the gate;
  say so rather than skipping silently). If the platform matched no
  prebuilt (`T` empty), say so — cargo is the remaining route. If
  `~/.local/bin` isn't on PATH, show the export line for their shell.

Every install runs only AFTER the user says yes — their choice of route,
through the normal Bash permission prompt. The audit itself never
installs anything as a side effect.

## Running an audit — token discipline first

**Never load a full report into the conversation.** Full console output and
full export JSON are for FILES humans open, not for context. Extract in the
shell; only the summary enters the chat. Work in tiers:

**Tier 1 — headline + spine table (default for every audit).** The JSON
schema is versioned and stable — parse it, don't screen-scrape the console.
One command yields the headline, every finding (any age), and the recent
spine; only this enters the conversation:

```bash
git-receipts export 2>/dev/null | python3 -c "
import json,sys
d=json.load(sys.stdin); s=d['summary']; b=s['balance']
print('commits', s['commits'], '(+'+str(s['keyframes_excluded'])+' by others held out) ·', b['green'],'🟢 /',b['amber'],'🟡 /',b['red'],'🔴 · claims', str(s['claims_landed'])+'/'+str(s['claims_total']), '· broken promises', s['broken_promises'])
iv=d['intervals']; CAP=15
g={'green':'🟢','amber':'🟡','red':'🔴'}
def row(i):
    L=i.get('ledger',[]); landed=sum(1 for l in L if l.get('landing')!='never')
    fs=[]
    res=len(i.get('residue',[]))
    if res: fs.append(str(res)+' residue')
    fr=[r for r in i.get('commands',{}).get('runs',[]) if r.get('failed')]
    if fr:
        names=[]
        for r in fr:
            t=(r.get('command') or '?').split(); n=t[0] if t else '?'
            if n not in names: names.append(n)
        fs.append(str(len(fr))+' cmd'+('s' if len(fr)>1 else '')+' failed ('+', '.join(names[:2])+('…' if len(names)>2 else '')+')')
    me=[c for c in i.get('mcp',[]) if c.get('errored')]
    if me:
        srv=[]
        for c in me:
            n=c.get('server','?')
            if n not in srv: srv.append(n)
        fs.append(str(len(me))+' mcp err ('+', '.join(srv[:2])+')')
    print(' ',g.get(i['status'],'⚪'), i['commit']['short'], (i['commit']['subject'][:45]).ljust(45), (str(landed)+'/'+str(len(L))).rjust(5), '', ' · '.join(fs) if fs else '—')
print()
print('   commit  '+'subject'.ljust(47)+'claims  findings')
older=iv[:-CAP] if len(iv)>CAP else []
for i in older:
    if i['status']!='green': row(i)
if older: print('   …', len(older), 'earlier commits (findings above shown; --oneline for all)')
for i in iv[-CAP:]: row(i)
"
```

Present the table as a code block, INCLUDING the header line the snippet
prints (`commit · subject · claims · findings`) — never drop it. Status
dots carry the color: 🟢 green, 🟡 amber, 🔴 red — chat markdown strips
terminal ANSI colors, so the dots ARE the color layer; never swap them
for plain ASCII. The findings column follows one consistent rule: **zero
counts are never printed, every nonzero cause is named, a clean row shows
`—`** — `N residue`, `N cmds failed (program, …)`, `N mcp err (server)`.
A colored row is never unexplained. If the findings list itself runs very
long (>25), truncate it the same way and say how many were omitted —
never silently.

For `--project`, one wrapper parse yields the roll-up AND a spine table per
repo that has commits (zero-commit repos stay as roll-up rows):

```bash
git-receipts export --project <dir> 2>/dev/null | python3 -c "
import json,sys
d=json.load(sys.stdin); s=d['summary']
g={'green':'🟢','amber':'🟡','red':'🔴'}
print('project verdict:', g.get(s['verdict'],''), s['verdict'], '·', s['repos'], 'repos')
print()
print('   repo        commits    claims  findings')
for r in s['landing']:
    fs=[]
    if r['broken']: fs.append(str(r['broken'])+' broken')
    if r['residue_files']: fs.append(str(r['residue_files'])+' residue')
    print(' ', g.get(r['verdict'],'⚪'), r['name'].ljust(10), str(r['commits']).rjust(5), '', (str(r['landed'])+'/'+str(r['claims'])).rjust(8), '', ' · '.join(fs) if fs else '—')
CAP=10
def row(i):
    L=i.get('ledger',[]); landed=sum(1 for l in L if l.get('landing')!='never')
    fs=[]
    res=len(i.get('residue',[]))
    if res: fs.append(str(res)+' residue')
    fr=[r for r in i.get('commands',{}).get('runs',[]) if r.get('failed')]
    if fr:
        names=[]
        for r in fr:
            t=(r.get('command') or '?').split(); n=t[0] if t else '?'
            if n not in names: names.append(n)
        fs.append(str(len(fr))+' cmd'+('s' if len(fr)>1 else '')+' failed ('+', '.join(names[:2])+('…' if len(names)>2 else '')+')')
    me=[c for c in i.get('mcp',[]) if c.get('errored')]
    if me:
        srv=[]
        for c in me:
            n=c.get('server','?')
            if n not in srv: srv.append(n)
        fs.append(str(len(me))+' mcp err ('+', '.join(srv[:2])+')')
    print(' ',g.get(i['status'],'⚪'), i['commit']['short'], (i['commit']['subject'][:45]).ljust(45), (str(landed)+'/'+str(len(L))).rjust(5), '', ' · '.join(fs) if fs else '—')
for rep in d['repos']:
    iv=rep['receipt']['intervals']
    if not iv: continue
    print(); print('===', rep['name'], '===')
    print('   commit  '+'subject'.ljust(47)+'claims  findings')
    older=iv[:-CAP] if len(iv)>CAP else []
    fnd=[i for i in older if i['status']!='green']
    for i in fnd[:15]: row(i)
    if len(fnd)>15: print('   …', len(fnd)-15, 'more findings omitted')
    if older: print('   …', len(older), 'earlier commits (findings above shown)')
    for i in iv[-CAP:]: row(i)
"
```

**Tier 2 — lists, when the user wants to scan.**
`git-receipts audit --oneline --no-pager --color never` (one line per commit;
add `--filter red-amber` for findings only).

**Tier 3 — one commit's full story, on demand.**
`git-receipts audit --commit <hash> --no-pager --color never` (add `--full`
for its conversation). For programmatic drill-down, filter the export:
`… | python3` selecting `intervals[]` by `status` or `commit.short` — never
print all intervals.

**Tier 4 — durable artifacts for humans.**
`--format html > audit.html` (then open it in the browser) ·
`export > receipt.json` (state the path; don't cat it).

Session-file and `--latest`/`--repo <dir>` arguments compose with all tiers.

## Choosing scope (defaults)

- Bare invocation or "audit this repo" → the CLI default (**all sessions** —
  the complete picture). Always state the scope you ran.
- "this session" / "what did you just do" → target THIS session precisely,
  not `--latest` (which guesses by mtime and can race a parallel session).
  The live log lets you find it by identity:

  ```bash
  N="receipts-$$-$(date +%s)"; echo "$N"; sleep 1
  S=$(grep -rl "$N" ~/.claude/projects/*/ 2>/dev/null | head -1)
  git-receipts audit "$S" --no-pager --color never
  ```

  If the grep finds nothing (write lag), wait a second and retry once; if it
  still misses, fall back to `--latest` and say so.
- "just the problems" → `--filter red-amber` · one commit → `--commit <hash>`
  (add `--full` for its conversation) · a folder of repos → `--project <dir>`
  · "include commits I made by hand" → `--full-history`.
- cwd not a git repo but contains repos? Use `--project .` instead of letting
  inference fail.

If the user supplies `$ARGUMENTS`, pass them through to `git-receipts audit`
verbatim — but only if they look like flags, paths, or a commit hash, never
shell syntax. **Never pass `--no-scan`**: the built-in secret/PII scanner
stays on, always.

## Interpreting for the user

- **Green** — the interval balances: every claim landed, no residue, no
  failed commands. Nothing to look at.
- **Amber** — worth a look, never a lie: unexplained residue, a failed
  command, or an errored MCP call.
- **Red / broken promises** — a claimed edit git never got, with no verified
  explanation. This is the headline number. Present a red as **a question,
  not a conviction** — it means nothing on record explains the claim, and
  the user may know something the log doesn't.
- **Held out** — commits the agent didn't make (teammates, pulls) are
  excluded from the verdict by default and shown as a count;
  `--full-history` includes them.
- `broken promises: 0` means zero among the claims audited — absence of a
  claim is not absence of action.

Summarize the headline first (commits, % green, claims landed, broken
promises), then drill into ambers and reds only if present or asked. Quote
the tool's own diagnosis lines — they are evidence-backed
("content-verified", "relocated before its first commit", "deleted before
any commit") — and never soften a red into a pass.

### Characterizing failures — evidence, not reflex

**Never dismiss failed commands as "noise", "sandbox artifacts", or
"benign" without inspecting them.** "Worth a look" means YOU look. The
export carries every failed command with its captured output — one
extraction shows all of them (heads only, token-thin):

```bash
git-receipts export 2>/dev/null | python3 -c "
import json,sys
d=json.load(sys.stdin)
for i in d['intervals']:
    for r in i.get('commands',{}).get('runs',[]):
        if r.get('failed'):
            o=r.get('output') or {}
            t=(o.get('text') or '').strip().splitlines()
            print(i['commit']['short'], '$', ' '.join((r.get('command') or '').split())[:70])
            print('    ', ' / '.join(t[:2])[:160])
"
```

(For `--project` exports, iterate `d['repos'][*]['receipt']['intervals']`
the same way.) Then characterize each failure BY its evidence:

- the same/similar command **succeeded later** in the interval → transient,
  say "retried and passed"
- an expected nonzero exit (`grep`/`rg` with no match, `diff` with
  differences) → expected-nonzero, cite the convention
- **aborted by the user** → the user's stop, not the agent's failure
- anything else → a real failure: give it one sentence WITH its error text,
  and let the user judge

If you present an audit without having inspected the failures, write
"N failed commands (uninspected)" — never "benign", never "noise". An
unexamined failure characterized as harmless is exactly the kind of claim
this tool exists to catch.

## After the audit — the full report is one word away

The spine table is the condensed view; the full HTML report — every commit,
its claim ledger, residue, effort, and the conversation behind each verdict —
is the complete one. After presenting the FIRST audit in a conversation,
end with a direct offer plus a one-line orientation, so new users learn the
moves without reading any docs:

> This is the condensed view. Say **report** and I'll open the full HTML
> report in your browser — every commit, drill-downs, the conversation
> behind each verdict.
>
> You can also ask: "what happened in commit \<hash\>" · "just the
> problems" · "audit this session" · "export the JSON".

After later audits, drop the orientation; when findings are present, one
short reminder line is enough: "**report** opens the full view."

When the user accepts — "report", "yes", "open it", any phrasing — generate
and open in ONE step, matching the scope of the audit just run (same
`--project` / `--latest` / session-file / `--commit` arguments):

```bash
F="${TMPDIR:-/tmp}/gitreceipts-audit.html"
git-receipts audit <scope args> --format html > "$F" && open "$F"
```

(`xdg-open` on Linux.) Then state the path so they can keep or move the
file. It lands OUTSIDE the repo deliberately — an audit tool must not leave
residue its own next run would flag; if the user wants a copy in the repo
or elsewhere, regenerate to the path they name. If there is no browser
(SSH, headless), still write the file, give the path, and say why it
didn't open — never silently skip.

The browser is the intended viewer: full reports are large single-file
pages that chat preview panes often can't render inline. For a lighter
file, offer `--filter red-amber` (findings only) or `--latest` (one
session).

## Privacy

Reports contain the user's prompts and command output — treat every report
as private. Whenever you suggest sharing or exporting one, mention the
privacy flags in the same breath: `--no-intent` (drops prompts and agent
prose; every count stays), `--no-identity` (drops names/emails), and
`--redact <word>` (masks any extra term). Self-contained HTML report:
`git-receipts audit --format html > audit.html`.

## Caveats (v0.1)

- Sessions recorded on a **different machine** are not supported — see
  KNOWN-LIMITATIONS.md §8 in the repo.
- Plain `git receipts --help` wants a man page that ships in 0.1.1; use
  `git-receipts --help` or `-h`.
