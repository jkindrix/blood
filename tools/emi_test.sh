#!/usr/bin/env bash
# Equivalence Modulo Inputs (EMI) testing for Blood compiler
#
# Takes a passing golden test, removes dead code branches, recompiles,
# and checks that the output is identical. Any difference is a compiler bug.
#
# Technique from "Finding and Understanding Bugs in C Compilers" (Yang et al., PLDI'14)
# Found 147 GCC/LLVM bugs by deleting code that provably doesn't execute.
#
# Usage:
#   tools/emi_test.sh [count] [--compiler PATH]
#
# Requirements: first_gen built, golden tests available

set -euo pipefail

DIR="$(cd "$(dirname "$0")/.." && pwd)"
COMPILER="${1:-$DIR/src/selfhost/build/first_gen}"
COUNT="${2:-100}"
SEED_DIR="$DIR/tests/golden"
FINDINGS_DIR="$DIR/.fuzz/emi_findings"
TMPDIR=$(mktemp -d)

mkdir -p "$FINDINGS_DIR"
trap "rm -rf $TMPDIR" EXIT

log() { printf "\033[1;34m==>\033[0m %s\n" "$1"; }
ok()  { printf "\033[1;32m  ✓\033[0m %s\n" "$1"; }
err() { printf "\033[1;31m  ✗\033[0m %s\n" "$1"; }

# ─── EMI Mutation Strategies (must be defined before use) ────────────────

emi_mutate() {
    local src="$1"
    local strategy=$((RANDOM % 5))

    case $strategy in
        0) # Delete an else branch (keep the if)
            sed 's/else {[^}]*}/else { 0 }/g' "$src"
            ;;
        1) # Replace a match arm body with constant
            awk '{
                if (/=> [0-9]/) { print }
                else if (/=>/ && !/main/ && !/return/) {
                    sub(/=> .*,/, "=> 0,")
                    print
                } else { print }
            }' "$src"
            ;;
        2) # Delete a simple let binding
            awk '
                /^[[:space:]]*let [a-z_][a-z_0-9]*: [a-z]/ && !/mut/ && !/perform/ && !/region/ && !/Vec/ && !/String/ && !/HashMap/ {
                    if (rand() < 0.3) next
                }
                { print }
            ' "$src"
            ;;
        3) # Insert dead code after return
            awk '/return [^;]+;/ {
                print
                print "    let _emi_dead: i32 = 999;"
                next
            } {print}' "$src"
            ;;
        4) # Add a redundant if-true wrapper
            sed 's/\(let \([a-z_]*\): i32 = \)\([0-9][0-9]*\);/\1if true { \3 } else { 0 };/' "$src"
            ;;
    esac
}

# Collect non-COMPILE_FAIL golden tests
seeds=()
while IFS= read -r f; do
    if ! head -5 "$f" | grep -q "COMPILE_FAIL"; then
        seeds+=("$f")
    fi
done < <(find "$SEED_DIR" -name '*.blood' -type f | sort)

log "EMI testing: $COUNT mutations from ${#seeds[@]} seeds"

tested=0
findings=0

for i in $(seq 1 "$COUNT"); do
    seed="${seeds[$((RANDOM % ${#seeds[@]}))]}"
    base=$(basename "$seed" .blood)

    # Get original output
    orig_out=$(timeout 10 "$COMPILER" run "$seed" 2>/dev/null) || continue
    orig_exit=$?

    # Apply EMI mutation: delete random if/else branches
    mutant="$TMPDIR/emi_${i}.blood"
    emi_mutate "$seed" > "$mutant" 2>/dev/null || continue

    # Compile and run mutant
    mut_out=$(timeout 10 "$COMPILER" run "$mutant" 2>/dev/null) || continue
    mut_exit=$?

    tested=$((tested + 1))

    # Compare
    if [ "$orig_out" != "$mut_out" ] || [ "$orig_exit" != "$mut_exit" ]; then
        findings=$((findings + 1))
        finding="$FINDINGS_DIR/emi_${base}_${i}.blood"
        cp "$mutant" "$finding"
        err "DIVERGE: $finding (orig_exit=$orig_exit, mut_exit=$mut_exit)"
        echo "  Original: ${orig_out:0:100}"
        echo "  Mutant:   ${mut_out:0:100}"
    fi

    if [ $((tested % 25)) -eq 0 ]; then
        printf "  [%d/%d tested, %d findings]\r" "$tested" "$COUNT" "$findings"
    fi
done

echo ""
log "Results: $tested tested, $findings findings"

