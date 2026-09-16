#!/usr/bin/env bash
# Ground truth about this repository, measured rather than remembered.
#
# This exists because the project accumulated ~100 working documents in .tmp/
# (1.6 MB) describing its own state, and they disagreed with each other and with
# the code. README.md claimed 576 golden tests while the compiler passed 713. A
# corpus README carried a CRITICAL bug marker for five months after the bug was
# fixed. Written state rots; measured state cannot.
#
# Fast by default (reads artifacts and logs). A health check nobody runs because
# it takes ten minutes is not a health check.
#
#   ./tools/health.sh            measure, print
#   ./tools/health.sh --write    also regenerate STATE.md
#   ./tools/health.sh --full     additionally run the corpus (slow, minutes)
#
# Exit: 0 healthy | 1 one or more checks failed

set -u
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT" || exit 2
GREP="$(type -P grep)"
SELFHOST="$REPO_ROOT/src/selfhost"
WRITE=0; FULL=0
for a in "$@"; do
    case "$a" in --write) WRITE=1;; --full) FULL=1;; esac
done

fails=0
out=""
say()  { out+="$1"$'\n'; }
check(){ # check <label> <ok|warn|fail> <detail>
    local mark
    case "$2" in ok) mark="  ok  ";; warn) mark=" warn ";; *) mark=" FAIL "; fails=$((fails+1));; esac
    out+="$(printf '[%s] %-26s %s' "$mark" "$1" "$3")"$'\n'
}

say "Blood — measured state  ($(date -u '+%Y-%m-%d %H:%M UTC'))"
say "commit $(git rev-parse --short HEAD 2>/dev/null || echo '?')  on $(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo '?')"
say ""

# --- compiler artifacts -----------------------------------------------------
FG="$SELFHOST/build/first_gen"
if [ -x "$FG" ]; then
    check "first_gen" ok "$(stat -c %y "$FG" | cut -d. -f1), $(( $(stat -c %s "$FG") / 1048576 )) MB"
else
    check "first_gen" fail "absent — run: cd src/selfhost && ./build_selfhost.sh build first_gen"
fi

SEED="$REPO_ROOT/bootstrap/seed"
if [ -x "$SEED" ]; then
    seed_commit=$($GREP -oE 'commit=[0-9a-f]+' bootstrap/seed.meta 2>/dev/null | head -1 | cut -d= -f2)
    if [ -n "$seed_commit" ] && git cat-file -e "$seed_commit" 2>/dev/null; then
        behind=$(git rev-list --count "$seed_commit"..HEAD 2>/dev/null || echo '?')
        if [ "$behind" != "?" ] && [ "$behind" -gt 15 ] 2>/dev/null; then
            check "seed freshness" warn "$behind commits behind HEAD (>15: re-gate)"
        else
            check "seed freshness" ok "$behind commits behind HEAD"
        fi
    else
        check "seed freshness" warn "seed.meta commit not in history"
    fi
else
    check "seed" fail "bootstrap/seed absent — the project cannot bootstrap"
fi

# --- runtime consistency (the single most expensive documented mistake) -----
A1="$REPO_ROOT/bootstrap/libblood_runtime_blood.a"
A2="$SELFHOST/build/libblood_runtime.a"
if [ -f "$A1" ] && [ -f "$A2" ]; then
    if [ "$(md5sum < "$A1")" = "$(md5sum < "$A2")" ]; then
        check "runtime consistency" ok "$(md5sum < "$A1" | cut -c1-8)"
    else
        check "runtime consistency" fail "bootstrap/ and build/ archives differ — see CLAUDE.md runtime-evolution gotcha"
    fi
else
    check "runtime consistency" warn "one or both archives absent"
fi

# --- golden tests (from the last build log; .logs is gitignored) ------------
# The newest log is often a build or gate log with no golden result, so search
# newest-first for the most recent log that actually contains one.
last_log=""
for lg in $(ls -t "$SELFHOST"/.logs/build_*.log 2>/dev/null); do
    if $GREP -qE 'Passed: [0-9]+  Compile fail: [0-9]+  Run fail: [0-9]+' "$lg"; then
        last_log="$lg"; break
    fi
done
GOLDEN_N="unknown"
if [ -n "$last_log" ]; then
    line=$($GREP -oE 'Passed: [0-9]+  Compile fail: [0-9]+  Run fail: [0-9]+' "$last_log" | tail -1)
    if [ -n "$line" ]; then
        GOLDEN_N=$(echo "$line" | awk '{print $2}')
        gfail=$(( $(echo "$line" | awk '{print $5}') + $(echo "$line" | awk '{print $8}') ))
        age=$(( ( $(date +%s) - $(stat -c %Y "$last_log") ) / 86400 ))
        if [ "$gfail" -gt 0 ]; then check "golden tests" fail "$GOLDEN_N pass, $gfail fail (log ${age}d old)"
        elif [ "$age" -gt 30 ]; then check "golden tests" warn "$GOLDEN_N pass, but log is ${age}d old — unverified"
        else check "golden tests" ok "$GOLDEN_N pass (log ${age}d old)"; fi
    else
        check "golden tests" warn "no result line in newest log"
    fi
else
    check "golden tests" warn "no build log — run: ./build_selfhost.sh test golden -q"
fi

# --- corpus -----------------------------------------------------------------
CORPUS_N=$(ls -d "$REPO_ROOT"/corpus/*/ 2>/dev/null | wc -l)
if [ "$FULL" = "1" ] && [ -x "$REPO_ROOT/corpus/run_corpus.sh" ]; then
    cres=$("$REPO_ROOT/corpus/run_corpus.sh" 2>&1)
    cpass=$(echo "$cres" | $GREP -oE 'Passed: [0-9]+' | awk '{print $2}')
    cknown=$(echo "$cres" | $GREP -oE 'Known-failing: [0-9]+' | awk '{print $2}')
    creg=$(echo "$cres" | $GREP -oE 'Regressed: [0-9]+' | awk '{print $2}')
    if [ -z "$cpass" ] || [ -z "$creg" ]; then
        check "corpus (ran)" fail "could not parse run_corpus.sh output"
    elif [ "$creg" -gt 0 ]; then
        check "corpus (ran)" fail "$creg REGRESSED -- $cpass/$CORPUS_N compile"
    elif [ "${cknown:-0}" -gt 0 ]; then
        check "corpus (ran)" warn "$cpass/$CORPUS_N compile; $cknown known failure(s) are the next work"
    else
        check "corpus (ran)" ok "$cpass/$CORPUS_N compile"
    fi
else
    check "corpus" ok "$CORPUS_N projects present (use --full to compile)"
fi

# --- builtin parity ---------------------------------------------------------
if [ -x "$REPO_ROOT/tools/builtin-parity.sh" ]; then
    bp=$("$REPO_ROOT/tools/builtin-parity.sh" 2>&1); bprc=$?
    nph=$(echo "$bp" | $GREP -oE '[0-9]+ known PHANTOM' | awk '{print $1}')
    if [ "$bprc" -ne 0 ]; then check "builtin parity" fail "new unclassified builtin — run tools/builtin-parity.sh"
    elif [ "${nph:-0}" -gt 0 ]; then check "builtin parity" warn "${nph} builtins typecheck but fail at link"
    else check "builtin parity" ok "no phantom builtins"; fi
else
    check "builtin parity" warn "tools/builtin-parity.sh absent"
fi

# --- documentation claims vs measurement ------------------------------------
# Every document that states a golden count. README said 576 and KNOWN_LIMITATIONS
# said 586 while the compiler passed 713 -- the same stale-number bug, twice.
claim_check() { # claim_check <label> <file> <ERE whose first number is the claim>
    local claimed
    claimed=$($GREP -oE "$3" "$2" 2>/dev/null | head -1 | $GREP -oE '[0-9]+' | head -1)
    if [ "$GOLDEN_N" = "unknown" ]; then
        # Never skip silently: an unverifiable claim is a finding, not an absence.
        check "$1" warn "cannot verify — no measured golden count to compare"
    elif [ -z "$claimed" ]; then
        check "$1" warn "no golden-test claim found to verify"
    elif [ "$claimed" != "$GOLDEN_N" ]; then
        check "$1" fail "says $claimed golden tests, measured $GOLDEN_N"
    else
        check "$1" ok "golden count matches ($claimed)"
    fi
}
claim_check "README claim"            README.md                 '[0-9]+/[0-9]+ golden tests'
claim_check "KNOWN_LIMITATIONS claim" docs/KNOWN_LIMITATIONS.md '\*\*Golden tests:\*\* [0-9]+ pass'

# --- CI ---------------------------------------------------------------------
unattended=""
for wf in .github/workflows/*.yml .github/workflows/*.yaml; do
    [ -f "$wf" ] || continue
    # A trigger is only unattended if it is not commented out.
    if $GREP -qE '^\s*(push|schedule):' "$wf"; then
        unattended+="$(basename "$wf") "
    fi
done
if [ -n "$unattended" ]; then
    check "CI automation" ok "unattended: ${unattended% }"
else
    check "CI automation" fail "no workflow runs without a human triggering it"
fi

# --- working tree -----------------------------------------------------------
dirty=$(git status --porcelain | wc -l)
if [ "$dirty" -gt 0 ]; then check "working tree" warn "$dirty uncommitted path(s)"
else check "working tree" ok "clean"; fi

say ""
if [ "$fails" -gt 0 ]; then say "$fails check(s) FAILED"; else say "all checks pass"; fi

printf '%s' "$out"

if [ "$WRITE" = "1" ]; then
    {
        echo "# Measured state"
        echo
        echo '```'
        printf '%s' "$out"
        echo '```'
        echo
        echo "Regenerate with \`./tools/health.sh --write\`. Do not edit by hand —"
        echo "this file exists because hand-maintained status documents in this repo"
        echo "disagreed with the code for months."
    } > "$REPO_ROOT/STATE.md"
    echo "wrote STATE.md"
fi

[ "$fails" -gt 0 ] && exit 1
exit 0
