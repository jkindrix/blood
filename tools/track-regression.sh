#!/usr/bin/env bash
#
# track-regression.sh — Ground-Truth Regression Tracker
#
# Runs ground-truth tests, compares results against a stored baseline,
# and reports new passes, new failures, and flips.
#
# Usage:
#   ./tools/track-regression.sh                # Run tests and compare to baseline
#   ./tools/track-regression.sh --save         # Run tests and save as new baseline
#   ./tools/track-regression.sh --compare FILE # Compare saved baseline to another
#   ./tools/track-regression.sh --show         # Show current baseline
#   ./tools/track-regression.sh --ref          # Run with reference compiler
#
# Baseline is stored at: tools/.baseline_results.txt
#
# Environment variables:
#   BLOOD_REF, BLOOD_TEST, BLOOD_RUNTIME, BLOOD_RUST_RUNTIME

set -uo pipefail

BLOOD_REF="${BLOOD_REF:-$HOME/blood/compiler-rust/target/release/blood}"
BLOOD_TEST="${BLOOD_TEST:-$HOME/blood/blood-std/std/compiler/first_gen}"
export BLOOD_RUNTIME="${BLOOD_RUNTIME:-$HOME/blood/runtime.o}"
export BLOOD_RUST_RUNTIME="${BLOOD_RUST_RUNTIME:-$HOME/blood/libblood_runtime.a}"
GROUND_TRUTH="${GROUND_TRUTH:-$HOME/blood/compiler-rust/tests/ground-truth}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BASELINE_FILE="$SCRIPT_DIR/.baseline_results.txt"

MODE="compare"
COMPILER="$BLOOD_TEST"
COMPILER_LABEL="test"

for arg in "$@"; do
    case "$arg" in
        --save)    MODE="save" ;;
        --show)    MODE="show" ;;
        --compare) MODE="compare-file" ;;
        --ref)     COMPILER="$BLOOD_REF"; COMPILER_LABEL="ref" ;;
        --help|-h)
            echo "Usage: $0 [--save|--show|--ref]"
            exit 0 ;;
        -*) echo "Unknown option: $arg" >&2; exit 3 ;;
        *)
            if [[ "$MODE" == "compare-file" ]]; then
                BASELINE_FILE="$arg"
                MODE="compare"
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

# ── Show baseline ────────────────────────────────────────────────────────────

if [[ "$MODE" == "show" ]]; then
    if [[ ! -f "$BASELINE_FILE" ]]; then
        echo "No baseline found at $BASELINE_FILE"
        echo "Run with --save to create one."
        exit 1
    fi

    echo -e "${BOLD}Current Baseline${RESET}"
    echo -e "${DIM}File: $BASELINE_FILE${RESET}"
    echo -e "${DIM}Date: $(head -1 "$BASELINE_FILE" | sed 's/^# //')${RESET}"
    echo ""

    local_pass=$(grep -c "^PASS " "$BASELINE_FILE" 2>/dev/null) || local_pass=0
    local_fail=$(grep -c "^FAIL " "$BASELINE_FILE" 2>/dev/null) || local_fail=0
    local_cf=$(grep -c "^COMPILE_FAIL " "$BASELINE_FILE" 2>/dev/null) || local_cf=0
    local_crash=$(grep -c "^CRASH " "$BASELINE_FILE" 2>/dev/null) || local_crash=0
    local_total=$((local_pass + local_fail + local_cf + local_crash))

    printf "  Pass:         %d\n" "$local_pass"
    printf "  Fail:         %d\n" "$local_fail"
    printf "  Compile Fail: %d\n" "$local_cf"
    printf "  Crash:        %d\n" "$local_crash"
    printf "  Total:        %d\n" "$local_total"
    exit 0
fi

# ── Run tests ────────────────────────────────────────────────────────────────

WORK="$(mktemp -d "/tmp/track-regression.XXXXXX")"
trap "rm -rf '$WORK'" EXIT

RESULTS_FILE="$WORK/results.txt"
echo "# $(date '+%Y-%m-%d %H:%M:%S') compiler=$COMPILER_LABEL" > "$RESULTS_FILE"

echo -e "${BOLD}Ground-Truth Regression Test${RESET}"
echo -e "${DIM}  compiler:     $COMPILER ($COMPILER_LABEL)${RESET}"
echo -e "${DIM}  ground-truth: $GROUND_TRUTH${RESET}"
echo ""

total=0
pass=0
fail=0
compile_fail=0
crash=0

for src in "$GROUND_TRUTH"/*.blood; do
    [[ -f "$src" ]] || continue
    name="$(basename "$src" .blood)"
    total=$((total + 1))

    # Parse test markers
    is_compile_fail=0
    head -1 "$src" | grep -q '^// COMPILE_FAIL:' && is_compile_fail=1

    expect_exit=""
    expect_exit_line=$(grep '^// EXPECT_EXIT:' "$src" | head -1 2>/dev/null) || true
    if [[ -n "$expect_exit_line" ]]; then
        expect_exit=$(echo "$expect_exit_line" | sed 's|^// EXPECT_EXIT: *||')
    fi

    expected_output=""
    expected_output=$(grep '^// EXPECT:' "$src" | sed 's|^// EXPECT: *||' 2>/dev/null) || true

    # Compile
    local_exe="$WORK/$name"
    rc=0
    if [[ "$COMPILER_LABEL" == "ref" ]]; then
        "$COMPILER" build "$src" -o "$local_exe" --quiet --color never 2>"$WORK/err.txt" || rc=$?
    else
        "$COMPILER" build "$src" -o "$WORK/${name}.ll" --no-cache 2>"$WORK/err.txt" 1>/dev/null || rc=$?
        # first_gen creates exe based on -o stem
        if [[ -x "$WORK/$name" ]]; then
            local_exe="$WORK/$name"
        elif [[ -x "$(dirname "$src")/$name" ]]; then
            mv "$(dirname "$src")/$name" "$WORK/$name"
            local_exe="$WORK/$name"
        fi
    fi

    # Handle compile-fail tests
    if [[ $is_compile_fail -eq 1 ]]; then
        if [[ ! -x "$local_exe" ]]; then
            echo "PASS $name" >> "$RESULTS_FILE"
            pass=$((pass + 1))
        else
            echo "FAIL $name" >> "$RESULTS_FILE"
            fail=$((fail + 1))
        fi
        rm -f "$local_exe" "$WORK/${name}.ll" "$WORK/${name}.o" 2>/dev/null
        continue
    fi

    # Check compilation result
    if [[ ! -x "$local_exe" ]]; then
        echo "COMPILE_FAIL $name" >> "$RESULTS_FILE"
        compile_fail=$((compile_fail + 1))
        rm -f "$WORK/${name}.ll" "$WORK/${name}.o" 2>/dev/null
        continue
    fi

    # Run executable
    run_exit=0
    actual=$("$local_exe" 2>/dev/null) || run_exit=$?

    # Check for crash (signal)
    if [[ $run_exit -gt 128 ]]; then
        echo "CRASH $name" >> "$RESULTS_FILE"
        crash=$((crash + 1))
        rm -f "$local_exe" "$WORK/${name}.ll" "$WORK/${name}.o" 2>/dev/null
        continue
    fi

    # Check expected output
    test_passed=1

    if [[ -n "$expected_output" ]]; then
        if [[ "$actual" != "$expected_output" ]]; then
            test_passed=0
        fi
    fi

    # Check expected exit code
    if [[ -n "$expect_exit" ]]; then
        if [[ "$expect_exit" == "nonzero" ]]; then
            [[ $run_exit -ne 0 ]] || test_passed=0
        else
            [[ "$run_exit" == "$expect_exit" ]] || test_passed=0
        fi
    elif [[ $run_exit -ne 0 ]]; then
        test_passed=0
    fi

    if [[ $test_passed -eq 1 ]]; then
        echo "PASS $name" >> "$RESULTS_FILE"
        pass=$((pass + 1))
    else
        echo "FAIL $name" >> "$RESULTS_FILE"
        fail=$((fail + 1))
    fi

    rm -f "$local_exe" "$WORK/${name}.ll" "$WORK/${name}.o" 2>/dev/null
done

# Print summary
echo -e "${BOLD}Current Run:${RESET}"
printf "  Pass:         ${GREEN}%d${RESET}\n" "$pass"
printf "  Fail:         ${RED}%d${RESET}\n" "$fail"
printf "  Compile Fail: ${YELLOW}%d${RESET}\n" "$compile_fail"
printf "  Crash:        ${RED}%d${RESET}\n" "$crash"
printf "  Total:        %d\n" "$total"
printf "  Score:        ${BOLD}%d/%d${RESET} (%.1f%%)\n" "$pass" "$total" "$(echo "scale=1; $pass * 100 / $total" | bc)"

# ── Save or Compare ─────────────────────────────────────────────────────────

if [[ "$MODE" == "save" ]]; then
    cp "$RESULTS_FILE" "$BASELINE_FILE"
    echo ""
    echo -e "${GREEN}Baseline saved to: $BASELINE_FILE${RESET}"
    exit 0
fi

# Compare against baseline
if [[ ! -f "$BASELINE_FILE" ]]; then
    echo ""
    echo -e "${YELLOW}No baseline found.${RESET} Run with --save to create one."
    echo "  (saving current results as baseline)"
    cp "$RESULTS_FILE" "$BASELINE_FILE"
    exit 0
fi

echo ""
echo -e "${BOLD}Comparison Against Baseline${RESET}"
echo -e "${DIM}Baseline: $BASELINE_FILE${RESET}"
echo -e "${DIM}Date: $(head -1 "$BASELINE_FILE" | sed 's/^# //')${RESET}"
echo ""

# Parse baseline
baseline_pass=$(grep -c "^PASS " "$BASELINE_FILE" 2>/dev/null) || baseline_pass=0
baseline_fail=$(grep -c "^FAIL " "$BASELINE_FILE" 2>/dev/null) || baseline_fail=0
baseline_cf=$(grep -c "^COMPILE_FAIL " "$BASELINE_FILE" 2>/dev/null) || baseline_cf=0
baseline_crash=$(grep -c "^CRASH " "$BASELINE_FILE" 2>/dev/null) || baseline_crash=0

printf "  %-20s  %-12s  %-12s  %-10s\n" "" "Baseline" "Current" "Delta"
printf "  %-20s  %-12s  %-12s  %-10s\n" "────────────────────" "────────────" "────────────" "──────────"
printf "  %-20s  %-12d  %-12d  " "Pass" "$baseline_pass" "$pass"
delta=$((pass - baseline_pass))
if [[ $delta -gt 0 ]]; then
    printf "${GREEN}+%d${RESET}\n" "$delta"
elif [[ $delta -lt 0 ]]; then
    printf "${RED}%d${RESET}\n" "$delta"
else
    printf "0\n"
fi

printf "  %-20s  %-12d  %-12d  " "Fail" "$baseline_fail" "$fail"
delta=$((fail - baseline_fail))
if [[ $delta -gt 0 ]]; then
    printf "${RED}+%d${RESET}\n" "$delta"
elif [[ $delta -lt 0 ]]; then
    printf "${GREEN}%d${RESET}\n" "$delta"
else
    printf "0\n"
fi

printf "  %-20s  %-12d  %-12d  " "Compile Fail" "$baseline_cf" "$compile_fail"
delta=$((compile_fail - baseline_cf))
if [[ $delta -gt 0 ]]; then
    printf "${RED}+%d${RESET}\n" "$delta"
elif [[ $delta -lt 0 ]]; then
    printf "${GREEN}%d${RESET}\n" "$delta"
else
    printf "0\n"
fi

printf "  %-20s  %-12d  %-12d  " "Crash" "$baseline_crash" "$crash"
delta=$((crash - baseline_crash))
if [[ $delta -gt 0 ]]; then
    printf "${RED}+%d${RESET}\n" "$delta"
elif [[ $delta -lt 0 ]]; then
    printf "${GREEN}%d${RESET}\n" "$delta"
else
    printf "0\n"
fi

# Find specific changes
echo ""

# New passes (were not PASS in baseline, now PASS)
new_passes=()
while IFS= read -r line; do
    test_name="${line#PASS }"
    if ! grep -q "^PASS $test_name$" "$BASELINE_FILE" 2>/dev/null; then
        old_status=$(grep " $test_name$" "$BASELINE_FILE" | head -1 | cut -d' ' -f1)
        new_passes+=("$test_name (was: ${old_status:-new})")
    fi
done < <(grep "^PASS " "$RESULTS_FILE")

if [[ ${#new_passes[@]} -gt 0 ]]; then
    echo -e "${GREEN}${BOLD}New Passes (${#new_passes[@]}):${RESET}"
    for p in "${new_passes[@]}"; do
        echo -e "  ${GREEN}+${RESET} $p"
    done
    echo ""
fi

# New failures (were PASS in baseline, now not PASS)
new_failures=()
while IFS= read -r line; do
    test_name="${line#PASS }"
    current_status=$(grep " $test_name$" "$RESULTS_FILE" | head -1 | cut -d' ' -f1)
    if [[ "$current_status" != "PASS" ]]; then
        new_failures+=("$test_name (now: $current_status)")
    fi
done < <(grep "^PASS " "$BASELINE_FILE")

if [[ ${#new_failures[@]} -gt 0 ]]; then
    echo -e "${RED}${BOLD}New Failures / Regressions (${#new_failures[@]}):${RESET}"
    for f in "${new_failures[@]}"; do
        echo -e "  ${RED}-${RESET} $f"
    done
    echo ""
fi

# New crashes
new_crashes=()
while IFS= read -r line; do
    test_name="${line#CRASH }"
    if ! grep -q "^CRASH $test_name$" "$BASELINE_FILE" 2>/dev/null; then
        old_status=$(grep " $test_name$" "$BASELINE_FILE" | head -1 | cut -d' ' -f1)
        new_crashes+=("$test_name (was: ${old_status:-new})")
    fi
done < <(grep "^CRASH " "$RESULTS_FILE")

if [[ ${#new_crashes[@]} -gt 0 ]]; then
    echo -e "${RED}${BOLD}New Crashes (${#new_crashes[@]}):${RESET}"
    for c in "${new_crashes[@]}"; do
        echo -e "  ${RED}!${RESET} $c"
    done
    echo ""
fi

# Overall verdict
if [[ ${#new_failures[@]} -eq 0 && ${#new_crashes[@]} -eq 0 ]]; then
    if [[ ${#new_passes[@]} -gt 0 ]]; then
        echo -e "${GREEN}${BOLD}No regressions. ${#new_passes[@]} new pass(es).${RESET}"
    else
        echo -e "${GREEN}${BOLD}No regressions. No changes from baseline.${RESET}"
    fi
else
    echo -e "${RED}${BOLD}REGRESSIONS DETECTED: ${#new_failures[@]} new failure(s), ${#new_crashes[@]} new crash(es).${RESET}"
    exit 1
fi
