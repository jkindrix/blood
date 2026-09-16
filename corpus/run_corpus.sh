#!/usr/bin/env bash
# Compile every corpus program with the current selfhost compiler.
#
# The corpus is a regression signal the golden tests cannot provide: goldens
# test the compiler against what it already does, these test it against what
# someone actually tried to write. The two link failures that seeded this
# script (print_char, print_u64) were green across all 713 goldens.
#
# Usage: ./run_corpus.sh [--keep] [compiler-path]
#   --keep   leave build artifacts in place for debugging

set -u
KEEP=0
[ "${1:-}" = "--keep" ] && { KEEP=1; shift; }

CORPUS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$CORPUS_DIR/.." && pwd)"
BLOODC="${1:-$REPO_ROOT/src/selfhost/build/first_gen}"
export BLOOD_RUST_RUNTIME="${BLOOD_RUST_RUNTIME:-$REPO_ROOT/src/selfhost/build/libblood_runtime.a}"

[ -x "$BLOODC" ] || { echo "corpus: compiler not found: $BLOODC" >&2; exit 2; }

cleanup() {
    [ "$KEEP" = "1" ] && return
    find "$CORPUS_DIR" -type d \( -name build -o -name '.blood-cache' \) -prune -exec rm -rf {} + 2>/dev/null
}
trap cleanup EXIT

# Projects whose entry point is not <name>/<name>.blood
declare -A ENTRY=( [identity]="main.blood" [brainfuck]="bf.blood" )

pass=0; fail=0
declare -a FAILED=()

printf "%-16s %6s  %-8s %s\n" "PROJECT" "LINES" "STATUS" "FIRST ERROR"
printf "%-16s %6s  %-8s %s\n" "----------------" "-----" "--------" "-----------"

for dir in "$CORPUS_DIR"/*/; do
    proj="$(basename "$dir")"
    entry="${ENTRY[$proj]:-$proj.blood}"
    src="$dir$entry"
    if [ ! -f "$src" ]; then
        printf "%-16s %6s  %-8s %s\n" "$proj" "-" "NOENTRY" "$entry not found"
        FAILED+=("$proj"); fail=$((fail+1)); continue
    fi
    lines=$(wc -l < "$src")
    out=$(cd "$dir" && timeout 180 "$BLOODC" build "$entry" 2>&1)
    rc=$?
    if [ $rc -eq 0 ]; then
        printf "%-16s %6s  %-8s\n" "$proj" "$lines" "OK"
        pass=$((pass+1))
    else
        first=$(printf '%s\n' "$out" | grep -E '^(error|warning)' | head -1 | cut -c1-90)
        undef=$(printf '%s\n' "$out" | grep -oE "undefined reference to \`[a-z0-9_]+'" | head -1)
        [ -n "$undef" ] && first="$undef"
        [ $rc -eq 124 ] && first="TIMEOUT after 180s"
        printf "%-16s %6s  %-8s %s\n" "$proj" "$lines" "FAIL" "$first"
        FAILED+=("$proj"); fail=$((fail+1))
    fi
done

echo
echo "Passed: $pass  Failed: $fail  Total: $((pass+fail))"
[ $fail -gt 0 ] && { echo "Failing: ${FAILED[*]}"; exit 1; }
exit 0
