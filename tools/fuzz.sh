#!/usr/bin/env bash
# Blood compiler fuzzing toolkit
#
# Usage:
#   tools/fuzz.sh crash     [duration]  — Crash fuzzing via random mutation (no AFL needed)
#   tools/fuzz.sh diff      [count]     — Differential testing: mutate golden tests, compare compilers
#   tools/fuzz.sh afl       [duration]  — AFL++ fuzzing (requires afl-fuzz in PATH)
#   tools/fuzz.sh report                — Show findings summary
#
# Crash fuzzing requires no external tools. It mutates golden test files and feeds
# them to the compiler, looking for crashes, hangs, and assertion failures.
#
# Differential testing runs the same program through both blood-rust and first_gen,
# comparing outputs. Any divergence is a bug in one of the compilers.
#
# AFL++ fuzzing provides coverage-guided mutation. Install: apt install afl++

set -euo pipefail

DIR="$(cd "$(dirname "$0")/.." && pwd)"
SELFHOST_DIR="$DIR/src/selfhost"
COMPILER="${SELFHOST_DIR}/build/first_gen"
BOOTSTRAP="${BLOOD_RUST:-}"
SEED_DIR="$DIR/tests/golden"
FUZZ_DIR="$DIR/.fuzz"
FINDINGS_DIR="$FUZZ_DIR/findings"
CORPUS_DIR="$FUZZ_DIR/corpus"

mkdir -p "$FINDINGS_DIR" "$CORPUS_DIR"

log() { printf "\033[1;34m==>\033[0m %s\n" "$1"; }
ok()  { printf "\033[1;32m  ✓\033[0m %s\n" "$1"; }
err() { printf "\033[1;31m  ✗\033[0m %s\n" "$1"; }

# ─── Crash fuzzing via random mutation ───────────────────────────────────

do_crash_fuzz() {
    local duration="${1:-300}"  # default 5 minutes
    local end_time=$(( $(date +%s) + duration ))
    local total=0 crashes=0 hangs=0

    [ -x "$COMPILER" ] || { err "Compiler not found: $COMPILER"; exit 1; }

    log "Crash fuzzing for ${duration}s (Ctrl+C to stop)"
    log "Seed corpus: $SEED_DIR ($(find "$SEED_DIR" -name '*.blood' | wc -l) files)"

    # Build seed corpus (copy golden tests, skip COMPILE_FAIL)
    local seeds=()
    while IFS= read -r f; do
        if ! head -3 "$f" | grep -q "COMPILE_FAIL"; then
            seeds+=("$f")
        fi
    done < <(find "$SEED_DIR" -name '*.blood' -type f)

    log "Using ${#seeds[@]} non-COMPILE_FAIL seeds"

    while [ "$(date +%s)" -lt "$end_time" ]; do
        # Pick a random seed
        local seed="${seeds[$((RANDOM % ${#seeds[@]}))]}"

        # Apply random mutations
        local mutant="$CORPUS_DIR/mutant_$$.blood"
        mutate_file "$seed" > "$mutant"

        # Run the compiler with timeout
        total=$((total + 1))
        local result
        if timeout 10 "$COMPILER" check "$mutant" >/dev/null 2>&1; then
            : # clean exit
        else
            local code=$?
            if [ "$code" -eq 124 ]; then
                hangs=$((hangs + 1))
                local finding="$FINDINGS_DIR/hang_$(date +%s)_$$.blood"
                cp "$mutant" "$finding"
                err "HANG: $finding (from $(basename "$seed"))"
            elif [ "$code" -ge 128 ]; then
                crashes=$((crashes + 1))
                local finding="$FINDINGS_DIR/crash_$(date +%s)_$$.blood"
                cp "$mutant" "$finding"
                local sig=$((code - 128))
                err "CRASH (signal $sig): $finding (from $(basename "$seed"))"
            fi
            # Exit codes 1-127 are normal compiler errors (expected for mutated input)
        fi

        rm -f "$mutant"

        # Progress every 100 iterations
        if [ $((total % 100)) -eq 0 ]; then
            printf "  [%d tested, %d crashes, %d hangs]\r" "$total" "$crashes" "$hangs"
        fi
    done

    echo ""
    log "Results: $total tested, $crashes crashes, $hangs hangs"
    if [ "$crashes" -gt 0 ] || [ "$hangs" -gt 0 ]; then
        log "Findings in: $FINDINGS_DIR/"
        ls -la "$FINDINGS_DIR"/
    fi
}

# Random file mutation strategies
mutate_file() {
    local src="$1"
    local strategy=$((RANDOM % 8))

    case $strategy in
        0) # Delete a random line
            local lines=$(wc -l < "$src")
            local del=$((RANDOM % lines + 1))
            sed "${del}d" "$src"
            ;;
        1) # Duplicate a random line
            local lines=$(wc -l < "$src")
            local dup=$((RANDOM % lines + 1))
            sed "${dup}p" "$src"
            ;;
        2) # Swap two random lines
            local lines=$(wc -l < "$src")
            local a=$((RANDOM % lines + 1))
            local b=$((RANDOM % lines + 1))
            awk -v a="$a" -v b="$b" 'NR==a{l=$0; getline; print; print l; next}1' "$src" 2>/dev/null || cat "$src"
            ;;
        3) # Insert random token
            local lines=$(wc -l < "$src")
            local at=$((RANDOM % lines + 1))
            local tokens=("fn" "struct" "enum" "let" "mut" "if" "else" "match" "for" "while"
                         "return" "break" "continue" "true" "false" "0" "1" "-1" "\"\"" "()"
                         "{" "}" "(" ")" "[" "]" ";" "," "." "&" "*" "+" "-" "!" "==" "!="
                         "effect" "handler" "perform" "resume" "linear" "affine" "region"
                         "pub" "mod" "use" "impl" "trait" "where" "dyn" "as" "in")
            local tok="${tokens[$((RANDOM % ${#tokens[@]}))]}"
            sed "${at}i\\${tok}" "$src"
            ;;
        4) # Replace a random character with another
            local size=$(wc -c < "$src")
            local pos=$((RANDOM % size))
            local chars=('a' 'z' '0' '9' '{' '}' '(' ')' ';' '.' ',' ' ' '\n' '"' "'" '&' '*' '+' '-')
            local ch="${chars[$((RANDOM % ${#chars[@]}))]}"
            head -c "$pos" "$src"
            printf '%s' "$ch"
            tail -c "+$((pos + 2))" "$src"
            ;;
        5) # Truncate at random point
            local size=$(wc -c < "$src")
            local at=$((RANDOM % size))
            head -c "$at" "$src"
            ;;
        6) # Concatenate two random seeds
            local other="${seeds[$((RANDOM % ${#seeds[@]}))]}"
            cat "$src" "$other"
            ;;
        7) # Remove all of one character type
            local removes=(';' ',' '.' '(' ')' '{' '}' '&' '*')
            local rm="${removes[$((RANDOM % ${#removes[@]}))]}"
            tr -d "$rm" < "$src"
            ;;
    esac
}

# ─── Differential testing ────────────────────────────────────────────────

do_diff_fuzz() {
    local count="${1:-100}"
    local divergences=0

    [ -x "$COMPILER" ] || { err "Compiler not found: $COMPILER"; exit 1; }
    [ -n "$BOOTSTRAP" ] && [ -x "$BOOTSTRAP" ] || { err "Set BLOOD_RUST to bootstrap compiler path"; exit 1; }

    log "Differential testing: $count mutations, comparing first_gen vs blood-rust"

    local seeds=()
    while IFS= read -r f; do
        if ! head -3 "$f" | grep -q "COMPILE_FAIL"; then
            seeds+=("$f")
        fi
    done < <(find "$SEED_DIR" -name '*.blood' -type f)

    for i in $(seq 1 "$count"); do
        local seed="${seeds[$((RANDOM % ${#seeds[@]}))]}"
        local mutant="$CORPUS_DIR/diff_mutant_$$.blood"
        mutate_file "$seed" > "$mutant"

        local out_fg out_bs exit_fg exit_bs
        out_fg=$(timeout 10 "$COMPILER" run "$mutant" 2>/dev/null) || exit_fg=$?
        out_bs=$(timeout 10 "$BOOTSTRAP" run "$mutant" 2>/dev/null) || exit_bs=$?

        if [ "${out_fg:-}" != "${out_bs:-}" ] && [ "${exit_fg:-0}" -eq 0 ] && [ "${exit_bs:-0}" -eq 0 ]; then
            divergences=$((divergences + 1))
            local finding="$FINDINGS_DIR/diverge_$(date +%s)_$$.blood"
            cp "$mutant" "$finding"
            err "DIVERGE: $finding"
            echo "  first_gen: ${out_fg:-(empty)}"
            echo "  bootstrap: ${out_bs:-(empty)}"
        fi

        rm -f "$mutant"

        if [ $((i % 50)) -eq 0 ]; then
            printf "  [%d/%d tested, %d divergences]\r" "$i" "$count" "$divergences"
        fi
    done

    echo ""
    log "Results: $count tested, $divergences divergences"
}

# ─── AFL++ fuzzing ───────────────────────────────────────────────────────

do_afl_fuzz() {
    local duration="${1:-3600}"

    which afl-fuzz >/dev/null 2>&1 || { err "afl-fuzz not found. Install: apt install afl++"; exit 1; }
    [ -x "$COMPILER" ] || { err "Compiler not found: $COMPILER"; exit 1; }

    local afl_in="$FUZZ_DIR/afl_in"
    local afl_out="$FUZZ_DIR/afl_out"
    mkdir -p "$afl_in"

    # Seed corpus: first 100 golden tests (diverse, small)
    log "Preparing seed corpus..."
    find "$SEED_DIR" -name '*.blood' -type f | head -100 | while read -r f; do
        cp "$f" "$afl_in/"
    done
    ok "$(ls "$afl_in/" | wc -l) seeds in $afl_in/"

    log "Starting AFL++ (duration: ${duration}s)"
    log "Findings will be in: $afl_out/"

    timeout "$duration" afl-fuzz \
        -i "$afl_in" \
        -o "$afl_out" \
        -t 10000 \
        -- "$COMPILER" check @@ \
        || true

    # Collect findings
    if [ -d "$afl_out/default/crashes" ]; then
        local crash_count=$(ls "$afl_out/default/crashes/" 2>/dev/null | wc -l)
        log "AFL++ found $crash_count crashes"
        if [ "$crash_count" -gt 0 ]; then
            cp "$afl_out/default/crashes/"* "$FINDINGS_DIR/" 2>/dev/null || true
        fi
    fi
}

# ─── Report ──────────────────────────────────────────────────────────────

do_report() {
    log "Fuzzing findings report"
    echo ""

    local crashes=$(find "$FINDINGS_DIR" -name 'crash_*' 2>/dev/null | wc -l)
    local hangs=$(find "$FINDINGS_DIR" -name 'hang_*' 2>/dev/null | wc -l)
    local diverges=$(find "$FINDINGS_DIR" -name 'diverge_*' 2>/dev/null | wc -l)

    echo "  Crashes:     $crashes"
    echo "  Hangs:       $hangs"
    echo "  Divergences: $diverges"
    echo ""

    if [ -d "$FINDINGS_DIR" ] && [ "$(ls -A "$FINDINGS_DIR" 2>/dev/null)" ]; then
        echo "  Findings:"
        ls -lhS "$FINDINGS_DIR/" | tail -n +2 | head -20
        echo ""
        echo "  To minimize a finding:"
        echo "    tools/minimize.sh <finding.blood>"
    else
        echo "  No findings yet. Run: tools/fuzz.sh crash"
    fi
}

# ─── Main ────────────────────────────────────────────────────────────────

case "${1:-help}" in
    crash) do_crash_fuzz "${2:-300}" ;;
    diff)  do_diff_fuzz "${2:-100}" ;;
    afl)   do_afl_fuzz "${2:-3600}" ;;
    report) do_report ;;
    *)
        echo "Blood compiler fuzzing toolkit"
        echo ""
        echo "Usage:"
        echo "  tools/fuzz.sh crash [seconds]  — Crash fuzz via mutation (default: 300s)"
        echo "  tools/fuzz.sh diff  [count]    — Differential test (default: 100 mutations)"
        echo "  tools/fuzz.sh afl   [seconds]  — AFL++ fuzzing (default: 3600s)"
        echo "  tools/fuzz.sh report           — Show findings summary"
        echo ""
        echo "Crash fuzzing requires no external tools."
        echo "Differential testing requires BLOOD_RUST env var."
        echo "AFL++ fuzzing requires afl-fuzz in PATH."
        ;;
esac
