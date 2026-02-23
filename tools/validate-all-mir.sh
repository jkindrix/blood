#!/usr/bin/env bash
#
# validate-all-mir.sh — MIR Validation Gate
#
# Runs --validate-mir on a set of .blood files and reports structural errors.
# Intended as a pre-codegen quality gate to catch MIR issues early.
#
# Usage:
#   ./tools/validate-all-mir.sh                              # Validate ground-truth tests
#   ./tools/validate-all-mir.sh path/to/file.blood           # Validate single file
#   ./tools/validate-all-mir.sh path/to/dir/                 # Validate all .blood in directory
#   ./tools/validate-all-mir.sh --self                       # Validate compiler source files
#   ./tools/validate-all-mir.sh --compiler REF               # Use reference compiler
#   ./tools/validate-all-mir.sh --compiler TEST              # Use test compiler (default)
#
# Environment variables:
#   BLOOD_REF, BLOOD_TEST, BLOOD_RUNTIME, BLOOD_RUST_RUNTIME

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

BLOOD_REF="${BLOOD_REF:-$REPO_ROOT/src/bootstrap/target/release/blood}"
BLOOD_TEST="${BLOOD_TEST:-$REPO_ROOT/blood-std/std/compiler/build/first_gen}"
export BLOOD_RUNTIME="${BLOOD_RUNTIME:-$REPO_ROOT/runtime/runtime.o}"
export BLOOD_RUST_RUNTIME="${BLOOD_RUST_RUNTIME:-$REPO_ROOT/src/bootstrap/target/release/libblood_runtime.a}"
GROUND_TRUTH="${GROUND_TRUTH:-$REPO_ROOT/tests/ground-truth}"
COMPILER_DIR="${COMPILER_DIR:-$REPO_ROOT/blood-std/std/compiler}"

COMPILER="$BLOOD_TEST"
COMPILER_LABEL="test"
TARGET=""
SELF_MODE=0

for arg in "$@"; do
    case "$arg" in
        --self) SELF_MODE=1 ;;
        --compiler)  shift_next=compiler ;;
        --help|-h)
            echo "Usage: $0 [path/to/file.blood | path/to/dir/ | --self] [--compiler REF|TEST]"
            exit 0 ;;
        -*)
            if [[ "${shift_next:-}" == "compiler" ]]; then
                case "$arg" in
                    REF|ref)   COMPILER="$BLOOD_REF"; COMPILER_LABEL="ref" ;;
                    TEST|test) COMPILER="$BLOOD_TEST"; COMPILER_LABEL="test" ;;
                    *)         COMPILER="$arg"; COMPILER_LABEL="custom" ;;
                esac
                shift_next=""
            else
                echo "Unknown option: $arg" >&2; exit 3
            fi ;;
        *)
            if [[ "${shift_next:-}" == "compiler" ]]; then
                case "$arg" in
                    REF|ref)   COMPILER="$BLOOD_REF"; COMPILER_LABEL="ref" ;;
                    TEST|test) COMPILER="$BLOOD_TEST"; COMPILER_LABEL="test" ;;
                    *)         COMPILER="$arg"; COMPILER_LABEL="custom" ;;
                esac
                shift_next=""
            else
                TARGET="$arg"
            fi ;;
    esac
done

# ── Colors ───────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
DIM='\033[2m'
RESET='\033[0m'

WORK="$(mktemp -d "/tmp/validate-mir.XXXXXX")"
trap "rm -rf '$WORK'" EXIT

echo -e "${BOLD}MIR Validation Gate${RESET}"
echo -e "${DIM}  compiler: $COMPILER ($COMPILER_LABEL)${RESET}"

# Build file list
FILES=()

if [[ $SELF_MODE -eq 1 ]]; then
    echo -e "${DIM}  source:   $COMPILER_DIR (self-hosted compiler)${RESET}"
    for f in "$COMPILER_DIR"/*.blood; do
        [[ -f "$f" ]] && FILES+=("$f")
    done
elif [[ -n "$TARGET" ]]; then
    if [[ -f "$TARGET" ]]; then
        FILES+=("$TARGET")
        echo -e "${DIM}  target:   $TARGET${RESET}"
    elif [[ -d "$TARGET" ]]; then
        echo -e "${DIM}  directory: $TARGET${RESET}"
        for f in "$TARGET"/*.blood; do
            [[ -f "$f" ]] && FILES+=("$f")
        done
    else
        echo -e "${RED}Not found: $TARGET${RESET}" >&2
        exit 1
    fi
else
    # Default: ground-truth tests
    echo -e "${DIM}  source:   $GROUND_TRUTH (ground-truth tests)${RESET}"
    for f in "$GROUND_TRUTH"/*.blood; do
        [[ -f "$f" ]] && FILES+=("$f")
    done
fi

echo ""

total=${#FILES[@]}
if [[ $total -eq 0 ]]; then
    echo -e "${RED}No .blood files found${RESET}"
    exit 1
fi

pass=0
fail=0
skip=0
mir_errors=0

for src in "${FILES[@]}"; do
    name="$(basename "$src" .blood)"

    # Skip compile-fail tests (they're expected to fail compilation)
    if head -1 "$src" 2>/dev/null | grep -q '^// COMPILE_FAIL:'; then
        skip=$((skip + 1))
        continue
    fi

    # Determine flags based on compiler type
    local_out="$WORK/${name}.ll"
    local_err="$WORK/${name}_err.txt"

    rc=0
    if [[ "$COMPILER_LABEL" == "test" ]]; then
        "$COMPILER" build "$src" -o "$local_out" --no-cache --validate-mir \
            >"$WORK/${name}_stdout.txt" 2>"$local_err" || rc=$?
    else
        # blood-rust doesn't have --validate-mir, just compile and check
        "$COMPILER" build "$src" -o "$local_out" --quiet --color never \
            2>"$local_err" || rc=$?
    fi

    # Check for MIR validation errors in stderr
    local_mir_errs=0
    if [[ -f "$local_err" ]]; then
        local_mir_errs=$(grep -c "MIR validation\|mir_validate\|invalid MIR\|MIR error" "$local_err" 2>/dev/null) || local_mir_errs=0
    fi

    if [[ $rc -eq 0 && $local_mir_errs -eq 0 ]]; then
        pass=$((pass + 1))
        # Only print on verbose or failure
    elif [[ $rc -gt 128 ]]; then
        # Crash (signal)
        local sig=$((rc - 128))
        printf "  ${RED}CRASH${RESET}  %-40s signal %d\n" "$name" "$sig"
        fail=$((fail + 1))
    elif [[ $local_mir_errs -gt 0 ]]; then
        printf "  ${RED}MIR${RESET}    %-40s %d MIR errors\n" "$name" "$local_mir_errs"
        grep -i "MIR validation\|mir_validate\|invalid MIR\|MIR error" "$local_err" | head -3 | sed 's/^/           /'
        mir_errors=$((mir_errors + local_mir_errs))
        fail=$((fail + 1))
    else
        printf "  ${YELLOW}FAIL${RESET}   %-40s exit %d\n" "$name" "$rc"
        # Show first few lines of error
        head -3 "$local_err" | sed 's/^/           /'
        fail=$((fail + 1))
    fi

    # Clean up per-file outputs
    rm -f "$local_out" "$WORK/${name}.o" "$WORK/${name}" "$WORK/${name}_stdout.txt" 2>/dev/null
    # Clean up exe that first_gen creates next to source
    rm -f "$(dirname "$src")/$name" 2>/dev/null
done

echo ""
echo -e "${BOLD}Results:${RESET}"
printf "  Pass:       %d\n" "$pass"
printf "  Fail:       %d\n" "$fail"
printf "  Skip:       %d\n" "$skip"
printf "  Total:      %d\n" "$total"
if [[ $mir_errors -gt 0 ]]; then
    printf "  MIR errors: %d\n" "$mir_errors"
fi

echo ""
if [[ $fail -eq 0 ]]; then
    echo -e "  ${GREEN}${BOLD}All MIR validation passed.${RESET}"
else
    echo -e "  ${RED}${BOLD}$fail file(s) failed MIR validation.${RESET}"
    exit 1
fi
