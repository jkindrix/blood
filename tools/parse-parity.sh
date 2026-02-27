#!/usr/bin/env bash
#
# parse-parity.sh — Parser Drift Detection for Blood Compilers
#
# Runs both the reference compiler (blood-rust) and the test compiler (any
# generation) on a corpus of .blood files, comparing accept/reject outcomes.
# Reports any "drift" where one compiler accepts a file the other rejects.
#
# Usage:
#   ./tools/parse-parity.sh                        # default: ground-truth + selfhost
#   ./tools/parse-parity.sh <file.blood>           # single file
#   ./tools/parse-parity.sh <directory>            # all .blood files in directory
#   ./tools/parse-parity.sh --verbose              # show all per-file results
#   ./tools/parse-parity.sh --quiet                # summary only
#
# Environment variables (override defaults):
#   BLOOD_REF          — path to reference compiler (blood-rust)
#   BLOOD_TEST         — path to test compiler (any generation)
#   BLOOD_RUNTIME      — path to runtime.o
#   BLOOD_RUST_RUNTIME — path to libblood_runtime.a
#
# Exit codes:
#   0 — no drift (all files agree)
#   1 — drift detected (compilers disagree on at least one file)
#   2 — usage error

set -euo pipefail

# ── Defaults ─────────────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

BLOOD_REF="${BLOOD_REF:-$REPO_ROOT/src/bootstrap/target/release/blood}"
BLOOD_TEST="${BLOOD_TEST:-$REPO_ROOT/src/selfhost/build/first_gen}"
export BLOOD_RUNTIME="${BLOOD_RUNTIME:-$REPO_ROOT/runtime/runtime.o}"
export BLOOD_RUST_RUNTIME="${BLOOD_RUST_RUNTIME:-$REPO_ROOT/src/bootstrap/target/release/libblood_runtime.a}"

VERBOSE=0
QUIET=0
TIMEOUT=5
TARGET=""

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BOLD='\033[1m'
RESET='\033[0m'

# ── Helpers ──────────────────────────────────────────────────────────────────

step()  { printf "\n${BOLD}==> %s${RESET}\n" "$1"; }
ok()    { printf "  ${GREEN}✓${RESET} %s\n" "$1"; }
fail()  { printf "  ${RED}✗${RESET} %s\n" "$1"; }
warn()  { printf "  ${YELLOW}!${RESET} %s\n" "$1"; }

usage() {
    echo "Usage: $0 [file.blood|directory] [options]"
    echo ""
    echo "Options:"
    echo "  --verbose       Show all per-file results (agree + drift)"
    echo "  --quiet         Summary line only"
    echo "  --timeout N     Per-file timeout in seconds (default: 5)"
    echo "  --help, -h      Show this help"
    echo ""
    echo "Defaults:"
    echo "  No arguments: tests ground-truth + selfhost corpora"
    echo ""
    echo "Environment:"
    echo "  BLOOD_REF          Reference compiler (default: blood-rust)"
    echo "  BLOOD_TEST         Test compiler (default: first_gen)"
    echo "  BLOOD_RUNTIME      Path to runtime.o"
    echo "  BLOOD_RUST_RUNTIME Path to libblood_runtime.a"
    exit 2
}

# ── Argument parsing ─────────────────────────────────────────────────────────

for arg in "$@"; do
    case "$arg" in
        --verbose)    VERBOSE=1 ;;
        --quiet)      QUIET=1 ;;
        --timeout)    shift; TIMEOUT="${1:-5}" ;;
        --timeout=*)  TIMEOUT="${arg#--timeout=}" ;;
        --help|-h)    usage ;;
        -*)           echo "Unknown option: $arg"; usage ;;
        *)            TARGET="$arg" ;;
    esac
done

# ── Validation ───────────────────────────────────────────────────────────────

if [[ ! -x "$BLOOD_REF" ]]; then
    printf "${RED}Error: Reference compiler not found: %s${RESET}\n" "$BLOOD_REF" >&2
    echo "Build with: cd src/bootstrap && cargo build --release" >&2
    exit 2
fi

if [[ ! -x "$BLOOD_TEST" ]]; then
    printf "${RED}Error: Test compiler not found: %s${RESET}\n" "$BLOOD_TEST" >&2
    echo "Build with: cd src/selfhost && ./build_selfhost.sh timings" >&2
    exit 2
fi

# ── Core: check one file with both compilers ─────────────────────────────────

# Globals for accumulation
TOTAL=0
AGREE=0
DRIFT=0

# Check if a failure is just "main function not found" — not a parser/type error.
# blood-rust check passes library files; first_gen check requires main(). This
# difference is a known behavioral gap in `check` semantics, not parser drift.
is_no_main_error() {
    local stderr="$1"
    [[ "$stderr" == *"main function not found"* ]]
}

check_file() {
    local src="$1"
    local name
    name="$(basename "$src")"

    TOTAL=$((TOTAL + 1))

    # Run reference compiler, capture output
    local ref_ok=0 ref_stderr=""
    if ref_stderr=$(timeout "$TIMEOUT" "$BLOOD_REF" check "$src" 2>&1); then
        ref_ok=0
    else
        ref_ok=1
    fi

    # Run test compiler, capture output
    local test_ok=0 test_stderr=""
    if test_stderr=$(timeout "$TIMEOUT" "$BLOOD_TEST" check "$src" 2>&1); then
        test_ok=0
    else
        test_ok=1
    fi

    # Normalize: treat "main function not found" as pass (not a parser/type error)
    if [[ "$ref_ok" -ne 0 ]] && is_no_main_error "$ref_stderr"; then
        ref_ok=0
    fi
    if [[ "$test_ok" -ne 0 ]] && is_no_main_error "$test_stderr"; then
        test_ok=0
    fi

    if [[ "$ref_ok" -eq "$test_ok" ]]; then
        # Agreement
        AGREE=$((AGREE + 1))
        if [[ "$VERBOSE" -eq 1 && "$QUIET" -eq 0 ]]; then
            if [[ "$ref_ok" -eq 0 ]]; then
                ok "AGREE-PASS  $name"
            else
                ok "AGREE-FAIL  $name"
            fi
        fi
    else
        # Drift detected
        DRIFT=$((DRIFT + 1))
        if [[ "$QUIET" -eq 0 ]]; then
            local ref_str="PASS" test_str="PASS"
            [[ "$ref_ok" -ne 0 ]] && ref_str="FAIL"
            [[ "$test_ok" -ne 0 ]] && test_str="FAIL"
            fail "DRIFT  $name  ref=$ref_str  test=$test_str"

            if [[ "$VERBOSE" -eq 1 ]]; then
                # Show error from the compiler that rejected
                if [[ "$ref_ok" -ne 0 ]]; then
                    printf "      ref error:\n"
                    echo "$ref_stderr" | tail -5 | sed 's/^/        /'
                fi
                if [[ "$test_ok" -ne 0 ]]; then
                    printf "      test error:\n"
                    echo "$test_stderr" | tail -5 | sed 's/^/        /'
                fi
            fi
        fi
    fi
}

# ── Run corpus ───────────────────────────────────────────────────────────────

check_corpus() {
    local label="$1"
    shift
    local files=("$@")

    if [[ ${#files[@]} -eq 0 ]]; then
        return
    fi

    if [[ "$QUIET" -eq 0 ]]; then
        step "Parse parity: ${#files[@]} $label files"
    fi

    local corpus_drift_before=$DRIFT

    for src in "${files[@]}"; do
        check_file "$src"
    done

    if [[ "$QUIET" -eq 0 && "$DRIFT" -eq "$corpus_drift_before" ]]; then
        printf "  ${GREEN}(no drift)${RESET}\n"
    fi
}

START_TIME=$(date +%s)

if [[ -n "$TARGET" ]]; then
    # Single file or directory
    if [[ -f "$TARGET" ]]; then
        check_corpus "target" "$TARGET"
    elif [[ -d "$TARGET" ]]; then
        mapfile -t files < <(find "$TARGET" -maxdepth 1 -name '*.blood' -type f | sort)
        check_corpus "$(basename "$TARGET")" "${files[@]}"
    else
        printf "${RED}Error: Not a file or directory: %s${RESET}\n" "$TARGET" >&2
        exit 2
    fi
else
    # Default: ground-truth + selfhost
    mapfile -t gt_files < <(find "$REPO_ROOT/tests/ground-truth" -maxdepth 1 -name 't*.blood' -type f | sort)
    check_corpus "ground-truth" "${gt_files[@]}"

    mapfile -t sh_files < <(find "$REPO_ROOT/src/selfhost" -maxdepth 1 -name '*.blood' -type f | sort)
    check_corpus "selfhost" "${sh_files[@]}"
fi

# ── Summary ──────────────────────────────────────────────────────────────────

END_TIME=$(date +%s)
ELAPSED=$((END_TIME - START_TIME))

if [[ "$DRIFT" -eq 0 ]]; then
    printf "\n${BOLD}Summary:${RESET} %d files, ${GREEN}%d agree${RESET}, ${GREEN}0 drift${RESET}  (%ds)\n" \
        "$TOTAL" "$AGREE" "$ELAPSED"
    exit 0
else
    printf "\n${BOLD}Summary:${RESET} %d files, %d agree, ${RED}%d drift${RESET}  (%ds)\n" \
        "$TOTAL" "$AGREE" "$DRIFT" "$ELAPSED"
    exit 1
fi
