#!/usr/bin/env bash
# ===========================================================================
# gitreceipts — generic QA harness
#
#   ⚠  PRIVACY: every run writes REAL session content (your prompts, file
#      paths, command output) into the output folder. RUN THIS IN A PRIVATE
#      LOCATION AND NEVER COMMIT THE OUTPUT. This script itself is generic and
#      safe to keep in a repo; its output is not.
#
# What it does: point it at a folder of git repos and a Claude Code store, and
# it discovers every repo that has sessions, then runs `git receipts` across
# every CLI switch — audit (text + HTML) and export (JSON) — checking INVARIANTS
# rather than golden snapshots (the inputs are private and non-reproducible):
#   • exit codes • JSON parses • HTML self-contained & well-formed
#   • leak-safety (your username never appears — home→~, user→****)
#   • suppression flags empty the right fields
#   • CROSS-FORMAT reconciliation: text == HTML == JSON on every headline and
#     exception number (the tool's core promise)
#
# Zero external deps: bash + python3 + git. See README.md.
# ===========================================================================
set -uo pipefail

usage() {
  cat <<EOF
Usage: run-qa.sh [options] [LABEL]

  --repos-root DIR   scan DIR (recursively) for git repos   [default: \$HOME/Developer/Projects]
  --store DIR        Claude Code projects store             [default: \$HOME/.claude/projects]
  --bin PATH         git-receipts binary                    [default: PATH, then ../repo/target/release]
  --out DIR          where to write the run folder          [default: <script-dir>/output]
  --depth N          repo-search depth under repos-root     [default: 4]
  --exclude GLOB     skip repos whose path matches GLOB (repeatable)
  --project-dir DIR  also test \`--project\` on this multi-repo folder (repeatable;
                     containers are auto-detected from repo parents regardless)
  --mode M           repos | projects | both   [default: both]
                     repos = per-repo matrix only; projects = --project mode only
  -h, --help         this help

  LABEL              names the run folder: output/LABEL-<timestamp>/   [default: run]

⚠  Output contains private session data — run in a private dir, do not commit it.
EOF
}

# --- defaults + arg parsing -------------------------------------------------
REPOS_ROOT="$HOME/Developer/Projects"
STORE="$HOME/.claude/projects"
BIN=""
OUTROOT=""     # default resolved after HERE is known → <script>/../output
DEPTH=4
LABEL="run"
MODE="both"
declare -a EXCLUDES=()
declare -a PROJECT_DIRS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --repos-root) REPOS_ROOT="$2"; shift 2 ;;
    --store)      STORE="$2"; shift 2 ;;
    --bin)        BIN="$2"; shift 2 ;;
    --out)        OUTROOT="$2"; shift 2 ;;
    --depth)      DEPTH="$2"; shift 2 ;;
    --exclude)    EXCLUDES+=("$2"); shift 2 ;;
    --project-dir) PROJECT_DIRS+=("$2"); shift 2 ;;   # a folder holding ≥2 repos
    --mode)       MODE="$2"; shift 2 ;;                # repos | projects | both
    -h|--help)    usage; exit 0 ;;
    -*)           echo "unknown option: $1" >&2; usage; exit 2 ;;
    *)            LABEL="$1"; shift ;;
  esac
done
case "$MODE" in repos|projects|both) ;; *) echo "--mode must be repos|projects|both" >&2; exit 2 ;; esac

HERE="$(cd "$(dirname "$0")" && pwd)"
# Default output beside the script itself (qa/output when it lives in ops/qa/) —
# the runs carry REAL session data, so this dir is gitignored. Override --out.
[ -z "$OUTROOT" ] && OUTROOT="$HERE/output"
# Resolve the binary: --bin wins; else prefer a sibling release build (when this
# script ships in the repo, that build IS the thing under test and may be fresher
# than a stale `cargo install`); else fall back to git-receipts on PATH.
if [ -z "$BIN" ]; then
  for c in "$HERE/../../repo/target/release/git-receipts" "$HERE/../target/release/git-receipts" \
           "$HERE/target/release/git-receipts" "$HERE/../../target/release/git-receipts"; do
    [ -x "$c" ] && BIN="$c" && break
  done
  [ -z "$BIN" ] && command -v git-receipts >/dev/null 2>&1 && BIN="$(command -v git-receipts)"
fi
[ -n "$BIN" ] && [ -x "$BIN" ] || { echo "git-receipts binary not found — pass --bin PATH (or build it)." >&2; exit 2; }
[ -d "$REPOS_ROOT" ] || { echo "repos-root not a directory: $REPOS_ROOT" >&2; exit 2; }
[ -d "$STORE" ]      || { echo "store not a directory: $STORE" >&2; exit 2; }

TS="$(date +%Y%m%d-%H%M%S)"
OUT="$OUTROOT/$LABEL-$TS"
mkdir -p "$OUT"
RESULTS="$OUT/results.tsv"; MANIFEST="$OUT/MANIFEST.tsv"
: > "$RESULTS"; : > "$MANIFEST"
: > "$OUT/consistency.log"

# Leak check is derived at runtime — no personal data baked into the script.
LEAK_USER="$(id -un)"

echo "⚠  gitreceipts QA — output contains REAL session content."
echo "   Folder: $OUT   (private — do not commit)"
echo "   binary: $BIN ($("$BIN" --version 2>/dev/null))"
echo "   repos-root: $REPOS_ROOT    store: $STORE"
echo

# Render a command for display with the home dir collapsed to ~ (never leaks).
show_cmd() { local c="git receipts $*"; printf '%s' "${c//$HOME/\~}"; }

PASS=0; FAIL=0; declare -a FAILURES=()

# --- validators (0=pass, 1=fail; $1 = output file) --------------------------
v_nonempty() { [ -s "$1" ]; }
v_json()     { python3 -c 'import json,sys; json.load(open(sys.argv[1]))' "$1" 2>/dev/null; }
v_html() {
  # Self-contained = no external ASSET LOADS. Content is HTML-escaped, so only
  # structural tags carry a literal '<'; check literal <link>/<script src>/<img>.
  # (Read head/tail into vars: `head|grep -q` + pipefail causes SIGPIPE false-fails.
  #  Count occurrences with grep -o|wc -l: BSD `grep -oc` counts lines, not matches.)
  local h t op cl
  h=$(head -c 40 "$1"); t=$(tail -c 40 "$1")
  [[ $h == *'<!doctype html>'* ]] || return 1
  [[ $t == *'</html>'* ]] || return 1
  grep -qE '<link |<script[^>]*src=|<img ' "$1" && return 1
  op=$(grep -o '<details' "$1" | wc -l); cl=$(grep -o '</details>' "$1" | wc -l)
  [ "$op" -eq "$cl" ]
}
# The leak that matters is the HOME PATH itself (it reveals /Users/<user> and
# your directory layout). Match it as a whole path component — followed by a
# slash, a boundary, or end — so a DIFFERENT user's path (…/<user>2/…, e.g. a
# redaction test fixture) or a project directory named after you (<user>.com,
# legitimate repo content) doesn't trip a false leak. To mask your name even in
# your own filenames before sharing, use `--redact <name>`.
v_noleak() { ! grep -qE "${HOME}(/|\$|[^A-Za-z0-9])" "$1"; }
v_noemail() { ! grep -qE '<[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+>' "$1"; }  # --no-identity
v_nosecret(){ ! grep -qE 'sk-ant-o[ar]t01-[A-Za-z0-9_-]{16}|AKIA[0-9A-Z]{16}|ghp_[A-Za-z0-9]{36}|-----BEGIN [A-Z ]*PRIVATE KEY-----' "$1"; }
v_hasansi() { grep -q $'\x1b\[' "$1"; }
v_noansi()  { ! grep -q $'\x1b\[' "$1"; }
v_no_intents()   { python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); sys.exit(0 if all(not i.get("intents") for i in d["intervals"]) else 1)' "$1" 2>/dev/null; }
v_no_summaries() { python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); sys.exit(0 if all("summary" not in i for i in d["intervals"]) else 1)' "$1" 2>/dev/null; }
v_no_identity()  { python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); i=d["summary"]["identities"]; sys.exit(0 if not i.get("authors") and not i.get("co_authors") else 1)' "$1" 2>/dev/null; }
v_compact()      { [ "$(wc -l < "$1")" -le 1 ]; }

# --- run one command: run <label> <expect:0|nz> "<validators>" -- <args...> --
run() {
  local label="$1" expect="$2" vals="$3"; shift 3; [ "$1" = "--" ] && shift
  local ext=txt
  case " $* " in *" --format html "*) ext=html ;; esac
  [ "$ext" = txt ] && [ "$1" = export ] && ext=json
  local f="$OUT/$label.$ext"
  "$BIN" "$@" >"$f" 2>"$OUT/$label.err"; local ec=$?
  local cmd; cmd="$(show_cmd "$@")"
  printf '%s\t%s\t%s\n' "$label.$ext" "$ec" "$cmd" >>"$MANIFEST"
  # Command at the TOP of the human-readable text output. (JSON can't hold a
  # comment without breaking parse; HTML must stay doctype-first to render — the
  # command for those lives in MANIFEST.tsv, keyed by filename.)
  if [ "$ext" = txt ] && [ -s "$f" ]; then
    printf '# $ %s\n#\n' "$cmd" | cat - "$f" >"$f.tmp" && mv "$f.tmp" "$f"
  fi
  local ok=1 msg="" v
  case "$expect" in
    0)  [ $ec -eq 0 ] || { ok=0; msg="exit=$ec(want 0)"; } ;;
    nz) [ $ec -ne 0 ] || { ok=0; msg="exit=0(want nz)"; } ;;
  esac
  for v in $vals; do "v_$v" "$f" || { ok=0; msg="$msg !$v"; }; done
  if [ $ok -eq 1 ]; then PASS=$((PASS+1)); local st=PASS; else FAIL=$((FAIL+1)); local st=FAIL; FAILURES+=("$label —$msg"); fi
  printf '%s\t%s\t%s\t%s\n' "$st" "$label" "exit=$ec" "$msg" >>"$RESULTS"
  printf '  %-5s %-46s %s\n' "$st" "$label" "$msg"
}

# --- the full switch matrix for ONE repo ------------------------------------
matrix() {
  local p="$1" repo="$2"          # p = short label prefix, repo = path
  local name; name="$(basename "$repo")"
  local hash; hash="$("$BIN" audit --latest --repo "$repo" --store "$STORE" --oneline --color never 2>/dev/null | grep -oE '^[0-9a-f]{7}' | head -1)"
  # session selection + formats
  run "$p-audit-latest"     0 "nonempty noleak nosecret" -- audit  --latest --repo "$repo" --store "$STORE"
  run "$p-audit-all"        0 "nonempty noleak"          -- audit  --all    --repo "$repo" --store "$STORE"
  run "$p-oneline"          0 "nonempty noleak"          -- audit  --latest --repo "$repo" --store "$STORE" --oneline
  run "$p-full-history"     0 "nonempty noleak"          -- audit  --latest --repo "$repo" --store "$STORE" --full-history
  run "$p-export-fullhist"  0 "json noleak"              -- export --latest --repo "$repo" --store "$STORE" --full-history
  run "$p-verbose"          0 "nonempty noleak"          -- audit  --latest --repo "$repo" --store "$STORE" --verbose
  run "$p-with-output"      0 "nonempty noleak nosecret" -- audit  --latest --repo "$repo" --store "$STORE" --with-output
  run "$p-filter-red"       0 "nonempty noleak"          -- audit  --latest --repo "$repo" --store "$STORE" --filter red
  run "$p-filter-redres"    0 "nonempty noleak"          -- audit  --latest --repo "$repo" --store "$STORE" --filter red-amber
  run "$p-no-prompt"        0 "nonempty noleak"          -- audit  --latest --repo "$repo" --store "$STORE" --no-prompt
  run "$p-no-summary"       0 "nonempty noleak"          -- audit  --latest --repo "$repo" --store "$STORE" --no-summary
  run "$p-no-intent"        0 "nonempty noleak"          -- audit  --latest --repo "$repo" --store "$STORE" --no-intent
  run "$p-no-identity"      0 "nonempty noleak noemail"  -- audit  --latest --repo "$repo" --store "$STORE" --no-identity
  run "$p-no-scan"          0 "nonempty noleak"          -- audit  --latest --repo "$repo" --store "$STORE" --no-scan
  run "$p-redact"           0 "nonempty noleak"          -- audit  --latest --repo "$repo" --store "$STORE" --redact "$name"
  run "$p-color-always"     0 "nonempty hasansi"         -- audit  --latest --repo "$repo" --store "$STORE" --color always
  run "$p-color-never"      0 "nonempty noansi"          -- audit  --latest --repo "$repo" --store "$STORE" --color never
  run "$p-agent-claude"     0 "nonempty noleak"          -- audit  --latest --repo "$repo" --store "$STORE" --agent claude
  run "$p-html-auto"        0 "html noleak"              -- audit  --latest --repo "$repo" --store "$STORE" --format html --expand auto
  run "$p-html-all"         0 "html noleak"              -- audit  --latest --repo "$repo" --store "$STORE" --format html --expand all
  run "$p-html-none"        0 "html noleak"              -- audit  --latest --repo "$repo" --store "$STORE" --format html --expand none
  run "$p-html-noident"     0 "html noleak noemail"      -- audit  --latest --repo "$repo" --store "$STORE" --format html --no-identity
  run "$p-html-withoutput"  0 "html noleak nosecret"     -- audit  --latest --repo "$repo" --store "$STORE" --format html --with-output
  run "$p-html-all-sess"    0 "html noleak"              -- audit  --all    --repo "$repo" --store "$STORE" --format html
  run "$p-export"           0 "json nonempty noleak"     -- export --latest --repo "$repo" --store "$STORE"
  run "$p-export-compact"   0 "json compact"             -- export --latest --repo "$repo" --store "$STORE" --compact
  run "$p-export-full"      0 "json noleak"              -- export --latest --repo "$repo" --store "$STORE" --full
  run "$p-export-without"   0 "json noleak nosecret"     -- export --latest --repo "$repo" --store "$STORE" --with-output
  run "$p-export-noprompt"  0 "json no_intents"          -- export --latest --repo "$repo" --store "$STORE" --no-prompt
  run "$p-export-nosumm"    0 "json no_summaries"        -- export --latest --repo "$repo" --store "$STORE" --no-summary
  run "$p-export-noint"     0 "json no_intents no_summaries" -- export --latest --repo "$repo" --store "$STORE" --no-intent
  run "$p-export-noident"   0 "json no_identity"         -- export --latest --repo "$repo" --store "$STORE" --no-identity
  run "$p-export-all"       0 "json noleak"              -- export --all    --repo "$repo" --store "$STORE"
  if [ -n "$hash" ]; then
    run "$p-commit"         0 "nonempty noleak"          -- audit  --latest --repo "$repo" --store "$STORE" --commit "$hash"
    run "$p-full-commit"    0 "nonempty noleak"          -- audit  --latest --repo "$repo" --store "$STORE" --full --commit "$hash"
    run "$p-export-commit"  0 "json"                     -- export --latest --repo "$repo" --store "$STORE" --commit "$hash"
  fi

  # ---- 0.1.1 surfaces: recap (the default command) and the compact page --
  # Recap renders the same receipt with a different frame, so it gets the
  # same leak/secret/identity guarantees as the audit — a narrative view
  # that leaked what the audit masks would be the worst kind of gap.
  run "$p-recap"            0 "nonempty noleak nosecret" -- recap  --latest --repo "$repo" --store "$STORE"
  run "$p-recap-all"        0 "nonempty noleak"          -- recap  --all    --repo "$repo" --store "$STORE"
  run "$p-recap-verbose"    0 "nonempty noleak nosecret" -- recap  --latest --repo "$repo" --store "$STORE" --verbose
  run "$p-recap-oneline"    0 "nonempty noleak"          -- recap  --latest --repo "$repo" --store "$STORE" --oneline
  run "$p-recap-summary"    0 "nonempty noleak"          -- recap  --latest --repo "$repo" --store "$STORE" --summary
  run "$p-recap-noint"      0 "nonempty noleak"          -- recap  --latest --repo "$repo" --store "$STORE" --no-intent
  run "$p-recap-noident"    0 "nonempty noleak noemail"  -- recap  --latest --repo "$repo" --store "$STORE" --no-identity
  run "$p-recap-noansi"     0 "nonempty noansi"          -- recap  --latest --repo "$repo" --store "$STORE" --color never
  run "$p-recap-html"       0 "html noleak"              -- recap  --latest --repo "$repo" --store "$STORE" --format html
  run "$p-recap-html-comp"  0 "html noleak"              -- recap  --latest --repo "$repo" --store "$STORE" --format html --compact
  run "$p-html-compact"     0 "html noleak"              -- audit  --latest --repo "$repo" --store "$STORE" --format html --compact
  run "$p-summary-emoji"    0 "nonempty noleak"          -- audit  --latest --repo "$repo" --store "$STORE" --summary --emoji
  if [ -n "$hash" ]; then
    run "$p-recap-commit"   0 "nonempty noleak"          -- recap  --latest --repo "$repo" --store "$STORE" --commit "$hash"
  fi

  # The compact page must actually BE smaller — a --compact that quietly
  # stopped compacting would pass every other check here.
  local full comp
  full=$(wc -c < "$OUT/$p-recap-html.html" 2>/dev/null || echo 0)
  comp=$(wc -c < "$OUT/$p-recap-html-comp.html" 2>/dev/null || echo 0)
  if [ "$comp" -gt 0 ] && [ "$full" -gt 0 ] && [ "$comp" -lt "$full" ]; then
    PASS=$((PASS+1)); printf '  %-5s %-46s %s\n' PASS "$p-compact-smaller" "$comp < $full"
    printf 'PASS\t%s\t-\t-\n' "$p-compact-smaller" >>"$RESULTS"
  else
    FAIL=$((FAIL+1)); FAILURES+=("$p-compact-smaller — compact=$comp full=$full")
    printf '  %-5s %-46s %s\n' FAIL "$p-compact-smaller" "compact=$comp full=$full"
    printf 'FAIL\t%s\t-\tnot smaller\n' "$p-compact-smaller" >>"$RESULTS"
  fi
}

# --- cross-format reconciliation for one repo -------------------------------
consistency() {
  local label="$1" repo="$2"
  local t="$OUT/consist-$label-text.txt" h="$OUT/consist-$label-html.html" j="$OUT/consist-$label-json.json"
  "$BIN" audit  --latest --repo "$repo" --store "$STORE" --color never >"$t" 2>/dev/null
  "$BIN" audit  --latest --repo "$repo" --store "$STORE" --format html  >"$h" 2>/dev/null
  "$BIN" export --latest --repo "$repo" --store "$STORE"                >"$j" 2>/dev/null
  {
    printf '%s\t-\t%s\n' "consist-$label-text.txt"  "$(show_cmd audit  --latest --repo "$repo" --store "$STORE" --color never)"
    printf '%s\t-\t%s\n' "consist-$label-html.html" "$(show_cmd audit  --latest --repo "$repo" --store "$STORE" --format html)"
    printf '%s\t-\t%s\n' "consist-$label-json.json" "$(show_cmd export --latest --repo "$repo" --store "$STORE")"
  } >>"$MANIFEST"
  python3 - "$t" "$h" "$j" "$label" <<'PY' >>"$OUT/consistency.log" 2>&1
import json, re, sys
t, h, j, label = open(sys.argv[1]).read(), open(sys.argv[2]).read(), open(sys.argv[3]).read(), sys.argv[4]
d = json.load(open(sys.argv[3])); s = d["summary"]; ex = s["exceptions"]
def g(txt, pat):
    m = re.search(pat, txt); return int(m.group(1)) if m else None
bal = s["balance"]; exe = s["execution"]
want = {"commits": s["commits"], "broken": s["broken_promises"],
        "landed": s["claims_landed"], "total_claims": s["claims_total"],
        "landed_late": ex["landed_late"], "unclaimed_total": ex["unclaimed_total"],
        "keyframes": ex["keyframes"], "created_elsewhere": ex["created_elsewhere"],
        "failed": ex["failed_commands_or_edits"],
        # color balance + execution-error counts (the amber model)
        "green": bal["green"], "grey": bal.get("grey"), "amber": bal["amber"], "red": bal["red"],
        "cmd_errors": exe["os_fs_failed"], "mcp_errors": exe["mcp_errored"], "residue_files": s["residue_files"]}
got_text = {"commits": g(t, r"drove (\d+) commits"),
            "broken": g(t, r"broken promises \(claimed, never landed, nothing explains it\): (\d+)"),
            "landed": g(t, r"(\d+)/\d+ claimed files landed"),
            "total_claims": g(t, r"\d+/(\d+) claimed files landed"),
            "landed_late": g(t, r"claims that landed late: (\d+)"),
            "unclaimed_total": g(t, r"unclaimed changes \(git recorded it, no matching edit claim\): (\d+)"),
            "keyframes": g(t, r"(\d+) commits? not made by this session"),
            "created_elsewhere": g(t, r"(\d+) created elsewhere"),
            "failed": g(t, r"failed commands or edits: (\d+)"),
            # Anchor EVERY field to the balance line itself. A session's own
            # prose quotes audit output verbatim (this repo's commits are full
            # of it), so an unanchored "green · N grey" reads the conversation
            # instead of the report — a false mismatch that cost an evening.
            "green": g(t, r"(?m)^balance: (\d+) green"),
            "grey": g(t, r"(?m)^balance: \d+ green · (\d+) grey"),
            "amber": g(t, r"(?m)^balance: \d+ green · \d+ grey · (\d+) amber"),
            "red": g(t, r"(?m)^balance: \d+ green · \d+ grey · \d+ amber · (\d+) red"),
            "cmd_errors": g(t, r"OS/FS: \d+ commands · (\d+) failed"),
            "mcp_errors": g(t, r"MCP:\s+\d+ calls · (\d+) errored"),
            "residue_files": g(t, r"your unexplained residue: (\d+) file")}
got_html = {"commits": g(h, r"green · \d+/(\d+)</span>"),
            # broken PROMISES = never-landed CLAIMS (the bottomline), NOT the red
            # stat tile (which counts red INTERVALS — a different, coarser number
            # that only coincidentally equals it when each red interval has one).
            "broken": g(h, r"broken promises \(claimed, never landed, nothing explains it\): (\d+)"),
            "landed": g(h, r"claims landed · (\d+)/\d+"),
            "total_claims": g(h, r"claims landed · \d+/(\d+)"),
            "landed_late": g(h, r"claims that landed late: (\d+)"),
            "unclaimed_total": g(h, r"unclaimed changes \(git recorded it, no matching edit claim\): (\d+)"),
            "keyframes": g(h, r"(\d+) commit\(s\) not made by this session"),
            "created_elsewhere": None, "failed": None,
            "green": g(h, r"class=.balance.>balance: (\d+) green"),
            "grey": g(h, r"class=.balance.>balance: \d+ green · (\d+) grey"),
            "amber": g(h, r"class=.balance.>balance: \d+ green · \d+ grey · (\d+) amber"),
            "red": g(h, r"class=.balance.>balance: \d+ green · \d+ grey · \d+ amber · (\d+) red"),
            "cmd_errors": g(h, r"<b>(\d+)</b><span>commands with errors"),
            "mcp_errors": g(h, r"<b>(\d+)<.b><span>MCP with errors"), "residue_files": g(h, r"<b>(\d+)<.b><span>residue<")}
html_skip = {"created_elsewhere", "failed"}
def consistent(got, want): return got == want or (got is None and want == 0)
ok = True
for k in want:
    ht = got_html[k]; hstr = "skip" if k in html_skip else str(ht)
    print((("PASS " if (consistent(got_text[k], want[k]) and (k in html_skip or consistent(ht, want[k]))) else "FAIL ")
          ) + f"{label:16} {k:20} json={want[k]!s:>6} text={got_text[k]!s:>6} html={hstr:>6}")
    ok = ok and consistent(got_text[k], want[k]) and (k in html_skip or consistent(ht, want[k]))
sys.exit(0 if ok else 1)
PY
  if [ $? -eq 0 ]; then PASS=$((PASS+1)); printf '  %-5s %-46s\n' PASS "reconcile:$label (text==html==json)"; printf 'PASS\treconcile:%s\t-\t-\n' "$label" >>"$RESULTS"
  else FAIL=$((FAIL+1)); FAILURES+=("reconcile:$label — numbers differ across formats"); printf '  %-5s %-46s %s\n' FAIL "reconcile:$label" "see RECONCILIATION.md"; printf 'FAIL\treconcile:%s\t-\tmismatch\n' "$label" >>"$RESULTS"; fi
}

# --- --project switch matrix + reconciliation for one project folder --------
# audit --project (console) and export --project (JSON wrapper) must run, stay
# leak-safe, reject the unsupported HTML form, and — the core check — agree on
# the where-it-landed roll-up: the console table row and the JSON
# summary.landing entry for every repo carry the same numbers and verdict.
v_project_json() { python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); sys.exit(0 if d.get("kind")=="project" and "repos" in d and "landing" in d["summary"] else 1)' "$1" 2>/dev/null; }
# No sibling repo path may appear inside another repo section's out_of_repo.
v_no_sibling_paths() { python3 -c '
import json,sys
d=json.load(open(sys.argv[1]))
names=[r["name"] for r in d["repos"]]
bad=[]
for r in d["repos"]:
    others=[n for n in names if n!=r["name"]]
    for fc in r["receipt"].get("out_of_repo",[]):
        p=fc["path"]
        for o in others:
            # a path segment "/<sibling>/" that is a project repo → leak
            if ("/%s/"%o) in p: bad.append((r["name"],o,p))
sys.exit(1 if bad else 0)' "$1" 2>/dev/null; }

pmatrix() {
  local p="$1" dir="$2"
  run "$p-audit"          0 "nonempty noleak nosecret"       -- audit  --project "$dir" --store "$STORE"
  run "$p-audit-color"    0 "nonempty hasansi"               -- audit  --project "$dir" --store "$STORE" --color always
  run "$p-audit-noint"    0 "nonempty noleak"                -- audit  --project "$dir" --store "$STORE" --no-intent
  run "$p-export"         0 "json project_json noleak no_sibling_paths" -- export --project "$dir" --store "$STORE"
  run "$p-export-compact" 0 "json compact project_json"      -- export --project "$dir" --store "$STORE" --compact
  run "$p-export-full"    0 "json project_json noleak"       -- export --project "$dir" --store "$STORE" --full
  run "$p-html"           0 "html noleak"                    -- audit  --project "$dir" --store "$STORE" --format html
  run "$p-html-all"       0 "html noleak"                    -- audit  --project "$dir" --store "$STORE" --format html --expand all
  # ---- 0.1.1 project surfaces: recap and the compact page --------------
  run "$p-recap"          0 "nonempty noleak nosecret"       -- recap  --project "$dir" --store "$STORE"
  run "$p-recap-summary"  0 "nonempty noleak"                -- recap  --project "$dir" --store "$STORE" --summary
  run "$p-recap-verbose"  0 "nonempty noleak nosecret"       -- recap  --project "$dir" --store "$STORE" --verbose
  run "$p-recap-html"     0 "html noleak"                    -- recap  --project "$dir" --store "$STORE" --format html
  run "$p-recap-html-cmp" 0 "html noleak"                    -- recap  --project "$dir" --store "$STORE" --format html --compact
  run "$p-html-compact"   0 "html noleak"                    -- audit  --project "$dir" --store "$STORE" --format html --compact
  run "$p-summary-emoji"  0 "nonempty noleak"                -- audit  --project "$dir" --store "$STORE" --summary --emoji
  # mutually-exclusive forms must fail gracefully
  run "$p-both-repo"      nz ""                              -- audit  --project "$dir" --repo "$dir" --store "$STORE"
  # --repo on a container of repos must refuse rather than pick one
  run "$p-repo-container" nz ""                              -- audit  --repo "$dir" --store "$STORE"
}

preconcile() {
  local label="$1" dir="$2"
  local t="$OUT/pconsist-$label-text.txt" j="$OUT/pconsist-$label-json.json" h="$OUT/pconsist-$label-html.html"
  "$BIN" audit  --project "$dir" --store "$STORE" --color never  >"$t" 2>/dev/null
  "$BIN" export --project "$dir" --store "$STORE"                >"$j" 2>/dev/null
  "$BIN" audit  --project "$dir" --store "$STORE" --format html  >"$h" 2>/dev/null
  {
    printf '%s\t-\t%s\n' "pconsist-$label-text.txt" "$(show_cmd audit  --project "$dir" --store "$STORE" --color never)"
    printf '%s\t-\t%s\n' "pconsist-$label-json.json" "$(show_cmd export --project "$dir" --store "$STORE")"
    printf '%s\t-\t%s\n' "pconsist-$label-html.html" "$(show_cmd audit  --project "$dir" --store "$STORE" --format html)"
  } >>"$MANIFEST"
  # The where-it-landed roll-up must be identical across console, JSON, and HTML.
  python3 - "$t" "$j" "$h" "$label" <<'PY' >>"$OUT/project-consistency.log" 2>&1
import json, re, sys
t = open(sys.argv[1]).read(); d = json.load(open(sys.argv[2])); h = open(sys.argv[3]).read(); label = sys.argv[4]
keys = ["commits","landed","claims","broken","residue_files","verdict"]
# console "where it landed" rows: name  commits  landed/claims  broken  residue  ● verdict
trows = {}
for m in re.finditer(r"(?m)^\s{2}(\S+)\s+(\d+)\s+(\d+)/(\d+)\s+(\d+)\s+(\d+)\s+\S*\s*(green|grey|amber|red)\s*$", t):
    trows[m.group(1)] = dict(commits=int(m[2]), landed=int(m[3]), claims=int(m[4]),
                             broken=int(m[5]), residue_files=int(m[6]), verdict=m[7])
# HTML landing rows
V = {"good":"green","note":"grey","warn":"amber","bad":"red"}
hrows = {}
for m in re.finditer(r'<tr><td><a href="#repo-[^"]*">([^<]+)</a></td><td>(\d+)</td><td>(\d+)/(\d+)</td><td[^>]*>(\d+)</td><td[^>]*>(\d+)</td><td class="verdict (\w+)">', h):
    hrows[m[1]] = dict(commits=int(m[2]), landed=int(m[3]), claims=int(m[4]),
                       broken=int(m[5]), residue_files=int(m[6]), verdict=V[m[7]])
ok = True
for row in d["summary"]["landing"]:
    n = row["name"]; c = trows.get(n); hh = hrows.get(n)
    if not c or not hh:
        print(f"FAIL {label:14} {n:14} missing in {'console' if not c else 'html'}"); ok = False; continue
    match = all(c[k] == row[k] == hh[k] for k in keys)
    print(("PASS " if match else "FAIL ") + f"{label:14} {n:14} " +
          " ".join(f"{k}=j{row[k]}/t{c[k]}/h{hh[k]}" for k in keys))
    ok = ok and match
sys.exit(0 if ok else 1)
PY
  if [ $? -eq 0 ]; then PASS=$((PASS+1)); printf '  %-5s %-46s\n' PASS "reconcile:project:$label (console==json==html)"; printf 'PASS\treconcile:project:%s\t-\t-\n' "$label" >>"$RESULTS"
  else FAIL=$((FAIL+1)); FAILURES+=("reconcile:project:$label — roll-up differs across formats"); printf '  %-5s %-46s %s\n' FAIL "reconcile:project:$label" "see project-consistency.log"; printf 'FAIL\treconcile:project:%s\t-\tmismatch\n' "$label" >>"$RESULTS"; fi
}

# --- discover repos with sessions -------------------------------------------
# A repo qualifies only if the matched session did VERIFIABLE work there — at
# least one claim that landed in git, OR at least one commit made by the session.
# That drops sub-repos matched purely by parent-directory descent: when sessions
# run in a non-git CONTAINER (a workspace of sibling repos), the tool matches
# that session to every sibling, but it shows 0 landed / 0 committed for the ones
# it never actually touched — noise, not a real audit. `--exclude GLOB` skips
# anything else (e.g. a whole workspace) by path.
echo "── discovering git repos with sessions under $REPOS_ROOT ──────────"
declare -a REPOS=() LABELS=()
seen_labels=""
while IFS= read -r gitpath; do
  repo="$(dirname "$gitpath")"
  skip=""
  for glob in ${EXCLUDES+"${EXCLUDES[@]}"}; do case "$repo" in $glob) skip=excluded ;; esac; done
  [ -n "$skip" ] && { echo "  ⊘ skip $(basename "$repo") — matches --exclude"; continue; }
  out="$("$BIN" audit --latest --repo "$repo" --store "$STORE" --no-pager --color never 2>/dev/null)"
  [ -z "$out" ] && continue          # no session resolves for this repo
  landed="$(printf '%s' "$out" | grep -oE '[0-9]+ landed in git' | grep -oE '^[0-9]+' | head -1)"
  agentc="$(printf '%s' "$out" | grep -oE '\([0-9]+ agent-committed' | grep -oE '[0-9]+' | head -1)"
  if [ "${landed:-0}" = 0 ] && [ "${agentc:-0}" = 0 ]; then
    echo "  ⊘ skip $(basename "$repo")  ($repo) — session matched by container descent, no landed/committed work here"
    continue
  fi
  base="$(basename "$repo")"; lbl="$base"; n=1
  while [[ " $seen_labels " == *" $lbl "* ]]; do n=$((n+1)); lbl="$base$n"; done
  seen_labels="$seen_labels $lbl"
  REPOS+=("$repo"); LABELS+=("$lbl")
  echo "  ✓ $lbl  ($repo)  [landed ${landed:-0} · committed ${agentc:-0}]"
done < <(find "$REPOS_ROOT" -maxdepth "$DEPTH" -name .git 2>/dev/null | sort)

# In repos/both mode, no repos is a hard error. In projects mode the per-repo
# matrix is skipped anyway; repo discovery only seeds project auto-detection, so
# zero repos is fine as long as a --project-dir was given.
if [ "${#REPOS[@]}" -eq 0 ] && [ "$MODE" != projects ]; then
  echo "  no repos with sessions found under $REPOS_ROOT for store $STORE" >&2
  echo "  (is the store right? try --store ~/.claude/projects)" >&2
  exit 1
fi
echo "  → ${#REPOS[@]} repo(s) discovered"
echo

if [ "$MODE" != projects ]; then
  # --- run the matrix per repo ----------------------------------------------
  for i in "${!REPOS[@]}"; do
    echo "── ${LABELS[$i]} — full switch matrix ────────────────────────────"
    matrix "${LABELS[$i]}" "${REPOS[$i]}"
    echo
  done

  # --- error paths (generic) ------------------------------------------------
  echo "── ERROR PATHS — expect non-zero / graceful ────────────────────────"
  NOGIT="$(mktemp -d)"                       # a real dir that is NOT a git repo
  run err-bad-repo    nz "" -- audit --latest --repo "$REPOS_ROOT/__nonexistent_repo__" --store "$STORE"
  run err-notgit-repo nz "" -- audit --latest --repo "$NOGIT" --store "$STORE"
  run err-bad-session nz "" -- audit "$NOGIT/nope.jsonl" --repo "${REPOS[0]}"
  run err-empty-store nz "" -- audit --latest --repo "${REPOS[0]}" --store "$NOGIT/__no_store__"
  rmdir "$NOGIT" 2>/dev/null || true
  echo

  # --- cross-format reconciliation (each repo) ------------------------------
  echo "── CROSS-FORMAT RECONCILIATION — text == html == json ──────────────"
  for i in "${!REPOS[@]}"; do consistency "${LABELS[$i]}" "${REPOS[$i]}"; done
  echo
fi

# --- --project mode ---------------------------------------------------------
if [ "$MODE" != repos ]; then
# Discover project containers: any folder that `audit --project` reports ≥2 repos
# for. Seed candidates from the parents of discovered repos (a container holds
# sibling repos) plus any explicit --project-dir. Dedup, then keep the ones that
# actually resolve to a multi-repo project.
echo "── --project MODE — roll-up + wrapper + reconciliation ─────────────"
declare -a PCAND=()
for r in ${REPOS+"${REPOS[@]}"}; do PCAND+=("$(dirname "$r")"); done
PCAND+=(${PROJECT_DIRS+"${PROJECT_DIRS[@]}"})
if [ "${#PCAND[@]}" -eq 0 ]; then
  echo "  (nothing to test — no repos discovered and no --project-dir given)"
fi
declare -a PROJECTS=() PLABELS=() pseen=""
for c in ${PCAND+"${PCAND[@]}"}; do
  cc="$(cd "$c" 2>/dev/null && pwd)" || continue
  [[ " $pseen " == *" $cc "* ]] && continue
  n="$("$BIN" audit --project "$cc" --store "$STORE" --no-pager --color never 2>/dev/null | grep -oE '^[0-9]+ git repos? with sessions' | grep -oE '^[0-9]+' | head -1)"
  [ "${n:-0}" -ge 2 ] || continue
  pseen="$pseen $cc"
  PROJECTS+=("$cc"); PLABELS+=("proj-$(basename "$cc")")
  echo "  ✓ $(basename "$cc")  ($cc)  [$n repos]"
done
if [ "${#PROJECTS[@]}" -eq 0 ]; then
  echo "  (no multi-repo project folders found — pass --project-dir DIR to force one)"
else
  echo "  → ${#PROJECTS[@]} project(s) to test"; echo
  for i in "${!PROJECTS[@]}"; do
    echo "── ${PLABELS[$i]} — --project matrix ─────────────────────────────"
    pmatrix "${PLABELS[$i]}" "${PROJECTS[$i]}"
    preconcile "${PLABELS[$i]}" "${PROJECTS[$i]}"
    echo
  done
fi
fi   # end MODE != repos

# --- summary.md -------------------------------------------------------------
TOTAL=$((PASS+FAIL))
{
  echo "# gitreceipts QA run — $LABEL-$TS"
  echo
  echo "- binary: \`$BIN\` ($("$BIN" --version 2>/dev/null))"
  echo "- repos-root: \`${REPOS_ROOT/#$HOME/\~}\`   store: \`${STORE/#$HOME/\~}\`"
  echo "- repos tested: ${#REPOS[@]} — ${LABELS[*]}"
  echo "- total: **$TOTAL** · pass: **$PASS** · fail: **$FAIL**"
  echo
  if [ "$FAIL" -gt 0 ]; then echo "## Failures"; for x in "${FAILURES[@]}"; do echo "- $x"; done
  else echo "All checks passed."; fi
  echo; echo "## Full results"; echo '```'; column -t -s $'\t' "$RESULTS"; echo '```'
  echo; echo "See \`MANIFEST.md\` (file → command), \`RECONCILIATION.md\` (numbers across formats), and \`<label>.txt|.html|.json\`."
} > "$OUT/summary.md"

# --- MANIFEST.md ------------------------------------------------------------
{
  echo "# Output manifest — $LABEL-$TS"
  echo; echo "Each artifact and the command that produced it (home shown as \`~\`)."
  echo "\`.html\` opens in a browser · \`.json\` parses · \`.txt\` carries its command on line 1."
  echo; echo "| file | exit | command |"; echo "|------|:----:|---------|"
  sort "$MANIFEST" | while IFS=$'\t' read -r file ec cmd; do printf '| `%s` | %s | `%s` |\n' "$file" "$ec" "$cmd"; done
} > "$OUT/MANIFEST.md"

# --- RECONCILIATION.md ------------------------------------------------------
{
  echo "# Reconciliation — text == html == json — $LABEL-$TS"
  echo; echo "Every headline + exception number, extracted from each surface and compared."
  echo "\`None\` = the surface omits that line at zero (consistent); \`skip\` = not"
  echo "rendered in that surface by design. A real mismatch fails."
  echo; echo '```'; cat "$OUT/consistency.log"; echo '```'; echo
  if grep -q '^FAIL' "$OUT/consistency.log" 2>/dev/null; then echo "**RESULT: MISMATCH — see FAIL rows above.**"
  else echo "**RESULT: all numbers reconcile across console, HTML, and JSON.**"; fi
  if [ -s "$OUT/project-consistency.log" ]; then
    echo; echo "## --project roll-up — console table == JSON summary.landing"
    echo; echo "Per repo: \`key=json/console\` for commits, landed, claims, broken,"
    echo "residue_files, and verdict."
    echo; echo '```'; cat "$OUT/project-consistency.log"; echo '```'; echo
    if grep -q '^FAIL' "$OUT/project-consistency.log" 2>/dev/null; then echo "**RESULT: MISMATCH — see FAIL rows above.**"
    else echo "**RESULT: the project roll-up reconciles across console and JSON.**"; fi
  fi
} > "$OUT/RECONCILIATION.md"

echo
echo "════════════════════════════════════════════════════════════════════"
echo "  total $TOTAL · pass $PASS · fail $FAIL"
echo "  folder:  $OUT"
echo "  summary.md · MANIFEST.md · RECONCILIATION.md   (⚠ private — do not commit)"
echo "════════════════════════════════════════════════════════════════════"
[ "$FAIL" -eq 0 ]
