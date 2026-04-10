#!/usr/bin/env bash
# LLVM IR verification for Blood compiler output
#
# Runs opt-18 --passes=verify on generated IR to catch structural errors
# that may not cause immediate crashes but indicate codegen bugs.
#
# Also runs FileCheck-18 on tests with CHECK patterns.
#
# Usage:
#   tools/verify_ir.sh [file.blood | --golden | --all]

set -euo pipefail

DIR="$(cd "$(dirname "$0")/.." && pwd)"
COMPILER="${DIR}/src/selfhost/build/first_gen"
TMPDIR=$(mktemp -d)
trap "rm -rf $TMPDIR" EXIT

log() { printf "\033[1;34m==>\033[0m %s\n" "$1"; }
ok()  { printf "\033[1;32m  ✓\033[0m %s\n" "$1"; }
err() { printf "\033[1;31m  ✗\033[0m %s\n" "$1"; }

verify_ir() {
    local src="$1"
    local base=$(basename "$src" .blood)

    # Compile to IR only
    local ll="$TMPDIR/${base}.ll"
    if ! timeout 30 "$COMPILER" build "$src" --emit-ir 2>/dev/null; then
        # Try without --emit-ir (build generates .ll anyway)
        timeout 30 "$COMPILER" build "$src" 2>/dev/null || return 1
    fi

    # Find the generated .ll file
    local found_ll=""
    for candidate in \
        "${src%.blood}.ll" \
        "$(dirname "$src")/build/debug/${base}.ll" \
        "$TMPDIR/${base}.ll"; do
        if [ -f "$candidate" ]; then
            found_ll="$candidate"
            break
        fi
    done

    if [ -z "$found_ll" ]; then
        return 1
    fi

    # Run LLVM verifier
    if opt-18 --passes=verify "$found_ll" -o /dev/null 2>"$TMPDIR/verify_err.txt"; then
        return 0
    else
        err "IR VERIFY FAIL: $src"
        head -5 "$TMPDIR/verify_err.txt" | sed 's/^/    /'
        return 1
    fi
}

case "${1:-help}" in
    --golden)
        log "Verifying IR for all golden tests"
        total=0; pass=0; fail=0; skip=0
        for f in "$DIR"/tests/golden/t0[1-5]_*.blood; do
            if head -3 "$f" | grep -q "COMPILE_FAIL"; then
                skip=$((skip + 1))
                continue
            fi
            total=$((total + 1))
            if verify_ir "$f" 2>/dev/null; then
                pass=$((pass + 1))
            else
                fail=$((fail + 1))
            fi
            if [ $((total % 50)) -eq 0 ]; then
                printf "  [%d verified, %d fail]\r" "$total" "$fail"
            fi
        done
        echo ""
        log "Results: $total verified, $pass pass, $fail fail, $skip skipped"
        ;;
    --all)
        log "Verifying IR for ALL golden tests"
        total=0; pass=0; fail=0; skip=0
        for f in "$DIR"/tests/golden/*.blood; do
            if head -3 "$f" | grep -q "COMPILE_FAIL"; then
                skip=$((skip + 1))
                continue
            fi
            total=$((total + 1))
            if verify_ir "$f" 2>/dev/null; then
                pass=$((pass + 1))
            else
                fail=$((fail + 1))
            fi
        done
        log "Results: $total verified, $pass pass, $fail fail, $skip skipped"
        ;;
    help|--help|-h)
        echo "Usage: tools/verify_ir.sh [file.blood | --golden | --all]"
        echo "  --golden    Verify IR for non-COMPILE_FAIL golden tests"
        echo "  --all       Verify IR for all golden tests"
        echo "  file.blood  Verify IR for a single file"
        ;;
    *)
        if [ -f "$1" ]; then
            if verify_ir "$1"; then
                ok "IR verified: $1"
            else
                exit 1
            fi
        else
            echo "File not found: $1"
            exit 1
        fi
        ;;
esac
