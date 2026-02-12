#!/usr/bin/env bash
# Ground-truth test suite for the Blood compiler.
# Compiles and runs each .blood file, checks exit code and expected output.
#
# Usage: ./run_tests.sh [BLOOD_BINARY] [FILTER]
#   BLOOD_BINARY: path to the blood compiler (default: ../../target/release/blood)
#   FILTER: optional prefix filter, e.g. "t00" to run only tier-0 tests

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BLOOD="${1:-$SCRIPT_DIR/../../target/release/blood}"
FILTER="${2:-}"

if [[ ! -x "$BLOOD" ]]; then
    echo "ERROR: Blood compiler not found or not executable: $BLOOD"
    echo "Build it first: cargo build --release -p bloodc"
    exit 1
fi

# Counters
PASS=0
FAIL=0
CRASH=0
COMPILE_FAIL=0
SKIP=0
TOTAL=0
XFAIL=0  # expected failures

# Results accumulator
declare -a RESULTS=()

# Color codes (if terminal supports it)
if [[ -t 1 ]]; then
    GREEN='\033[0;32m'
    RED='\033[0;31m'
    YELLOW='\033[0;33m'
    CYAN='\033[0;36m'
    BOLD='\033[1m'
    RESET='\033[0m'
else
    GREEN='' RED='' YELLOW='' CYAN='' BOLD='' RESET=''
fi

# Temporary directory for build artifacts
TMPDIR=$(mktemp -d)
trap "rm -rf $TMPDIR" EXIT

run_one_test() {
    local src="$1"
    local name
    name="$(basename "$src" .blood)"

    TOTAL=$((TOTAL + 1))

    # Check for expected-failure marker (first line: // XFAIL: <reason>)
    local xfail=""
    if head -1 "$src" | grep -q '^// XFAIL:'; then
        xfail=$(head -1 "$src" | sed 's|^// XFAIL: *||')
    fi

    # Check for expected-compile-fail marker (first line: // COMPILE_FAIL: <reason>)
    local compile_fail=""
    if head -1 "$src" | grep -q '^// COMPILE_FAIL:'; then
        compile_fail=$(head -1 "$src" | sed 's|^// COMPILE_FAIL: *||')
    fi

    # Check for expected exit code marker: // EXPECT_EXIT: <code|nonzero>
    local expect_exit=""
    local expect_exit_line
    expect_exit_line=$(grep '^// EXPECT_EXIT:' "$src" | head -1) || true
    if [[ -n "$expect_exit_line" ]]; then
        expect_exit=$(echo "$expect_exit_line" | sed 's|^// EXPECT_EXIT: *||')
    fi

    # Check for expected output marker: // EXPECT: <line>
    local -a expected_lines=()
    while IFS= read -r line; do
        expected_lines+=("$(echo "$line" | sed 's|^// EXPECT: *||')")
    done < <(grep '^// EXPECT:' "$src")

    # Compile
    local compile_out
    compile_out=$("$BLOOD" build "$src" --quiet --color never 2>&1) || true
    local exe="${src%.blood}"

    # Check if compilation produced an executable
    if [[ -n "$compile_fail" ]]; then
        # We EXPECT compilation to fail
        if [[ ! -x "$exe" ]]; then
            PASS=$((PASS + 1))
            RESULTS+=("${GREEN}PASS${RESET} $name (expected compile fail: $compile_fail)")
            return
        else
            FAIL=$((FAIL + 1))
            RESULTS+=("${RED}FAIL${RESET} $name (expected compile fail but compilation succeeded)")
            rm -f "$exe"
            return
        fi
    fi

    if [[ ! -x "$exe" ]]; then
        if [[ -n "$xfail" ]]; then
            XFAIL=$((XFAIL + 1))
            RESULTS+=("${YELLOW}XFAIL${RESET} $name (compile: $xfail)")
        else
            COMPILE_FAIL=$((COMPILE_FAIL + 1))
            # Show first few lines of error
            local err_preview
            err_preview=$(echo "$compile_out" | head -5)
            RESULTS+=("${RED}COMPILE_FAIL${RESET} $name\n    ${err_preview}")
        fi
        return
    fi

    # Run
    local run_out=""
    local exit_code=0
    run_out=$("$exe" 2>&1) || exit_code=$?

    # Clean up executable
    rm -f "$exe"

    # Check exit code
    if [[ $exit_code -ne 0 ]]; then
        if [[ -n "$expect_exit" ]]; then
            if [[ "$expect_exit" == "nonzero" || "$expect_exit" == "$exit_code" ]]; then
                : # Expected exit code — fall through to output checking
            else
                FAIL=$((FAIL + 1))
                RESULTS+=("${RED}FAIL${RESET} $name (expected exit $expect_exit, got $exit_code)")
                return
            fi
        elif [[ -n "$xfail" ]]; then
            XFAIL=$((XFAIL + 1))
            RESULTS+=("${YELLOW}XFAIL${RESET} $name (runtime: $xfail)")
            return
        else
            CRASH=$((CRASH + 1))
            RESULTS+=("${RED}CRASH${RESET} $name (exit code $exit_code)\n    stdout: $(echo "$run_out" | head -3)")
            return
        fi
    fi

    # Check expected output if specified
    if [[ ${#expected_lines[@]} -gt 0 ]]; then
        local actual_lines
        mapfile -t actual_lines <<< "$run_out"
        local mismatch=0
        local i
        for i in "${!expected_lines[@]}"; do
            local expected="${expected_lines[$i]}"
            local actual="${actual_lines[$i]:-<missing>}"
            if [[ "$actual" != "$expected" ]]; then
                mismatch=1
                FAIL=$((FAIL + 1))
                RESULTS+=("${RED}FAIL${RESET} $name (output mismatch line $((i+1)))\n    expected: '$expected'\n    actual:   '$actual'")
                return
            fi
        done
    fi

    # If we get here, test passed
    if [[ -n "$xfail" ]]; then
        # Unexpected pass of an expected failure
        PASS=$((PASS + 1))
        RESULTS+=("${CYAN}XPASS${RESET} $name (expected fail but passed! Remove XFAIL marker)")
    else
        PASS=$((PASS + 1))
        RESULTS+=("${GREEN}PASS${RESET} $name")
    fi
}

# Header
echo -e "${BOLD}Blood Ground-Truth Test Suite${RESET}"
echo "Compiler: $BLOOD"
echo "$("$BLOOD" --version 2>&1 || echo "unknown version")"
echo "---"

# Collect and sort test files
mapfile -t test_files < <(find "$SCRIPT_DIR" -name '*.blood' -type f | sort)

for src in "${test_files[@]}"; do
    name="$(basename "$src" .blood)"
    # Apply filter if specified
    if [[ -n "$FILTER" && ! "$name" =~ ^$FILTER ]]; then
        SKIP=$((SKIP + 1))
        continue
    fi
    run_one_test "$src"
done

# Print results
echo ""
for r in "${RESULTS[@]}"; do
    echo -e "  $r"
done

# Summary
echo ""
echo -e "${BOLD}Summary:${RESET}"
echo -e "  Total:        $TOTAL"
echo -e "  ${GREEN}Passed:       $PASS${RESET}"
echo -e "  ${RED}Failed:       $FAIL${RESET}"
echo -e "  ${RED}Compile fail: $COMPILE_FAIL${RESET}"
echo -e "  ${RED}Crashed:      $CRASH${RESET}"
echo -e "  ${YELLOW}Expected fail:$XFAIL${RESET}"
if [[ $SKIP -gt 0 ]]; then
    echo -e "  Skipped:      $SKIP"
fi

# Exit code: 0 if all tests pass or are expected failures
if [[ $((FAIL + COMPILE_FAIL + CRASH)) -gt 0 ]]; then
    exit 1
else
    exit 0
fi
