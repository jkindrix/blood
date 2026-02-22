#!/usr/bin/env bash
#
# asan-selfcompile.sh — AddressSanitizer Self-Compilation Wrapper
#
# One-command wrapper that:
#   1. Builds first_gen from blood-rust (or reuses existing)
#   2. Self-compiles (first_gen → second_gen.ll)
#   3. Instruments second_gen.ll with AddressSanitizer
#   4. Runs the ASan-instrumented binary with a smoke test
#   5. Formats and reports any sanitizer findings
#
# Usage:
#   ./tools/asan-selfcompile.sh                 # Full pipeline
#   ./tools/asan-selfcompile.sh --reuse         # Skip first_gen rebuild, reuse existing
#   ./tools/asan-selfcompile.sh --ir FILE.ll    # Instrument existing IR directly
#   ./tools/asan-selfcompile.sh --run-only      # Just run existing second_gen_asan
#   ./tools/asan-selfcompile.sh --test CMD      # Custom test command (default: "version")
#
# Environment variables:
#   BLOOD_REF, BLOOD_RUNTIME, BLOOD_RUST_RUNTIME
#   BUILD_DIR — directory containing compiler sources (default: <repo>/blood-std/std/compiler)

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

BLOOD_REF="${BLOOD_REF:-$REPO_ROOT/compiler-rust/target/release/blood}"
RUNTIME_O="${RUNTIME_O:-$REPO_ROOT/compiler-rust/runtime/runtime.o}"
RUNTIME_A="${RUNTIME_A:-$REPO_ROOT/compiler-rust/target/release/libblood_runtime.a}"
BUILD_DIR="${BUILD_DIR:-$REPO_ROOT/blood-std/std/compiler}"

export BLOOD_RUNTIME="${BLOOD_RUNTIME:-$RUNTIME_O}"
export BLOOD_RUST_RUNTIME="${BLOOD_RUST_RUNTIME:-$RUNTIME_A}"

REUSE=0
IR_FILE=""
RUN_ONLY=0
TEST_CMD="version"

for arg in "$@"; do
    case "$arg" in
        --reuse)    REUSE=1 ;;
        --run-only) RUN_ONLY=1 ;;
        --help|-h)
            echo "Usage: $0 [--reuse] [--ir FILE.ll] [--run-only] [--test CMD]"
            exit 0 ;;
        --ir)       shift_next=ir ;;
        --test)     shift_next=test ;;
        -*)
            if [[ "${shift_next:-}" == "ir" ]]; then
                IR_FILE="$arg"; shift_next=""
            elif [[ "${shift_next:-}" == "test" ]]; then
                TEST_CMD="$arg"; shift_next=""
            else
                echo "Unknown option: $arg" >&2; exit 3
            fi ;;
        *)
            if [[ "${shift_next:-}" == "ir" ]]; then
                IR_FILE="$arg"; shift_next=""
            elif [[ "${shift_next:-}" == "test" ]]; then
                TEST_CMD="$arg"; shift_next=""
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

step()  { printf "\n${BOLD}${CYAN}==> %s${RESET}\n" "$1"; }
ok()    { printf "  ${GREEN}OK${RESET}  %s\n" "$1"; }
fail()  { printf "  ${RED}FAIL${RESET}  %s\n" "$1"; }
warn()  { printf "  ${YELLOW}WARN${RESET}  %s\n" "$1"; }

WORK="$(mktemp -d "/tmp/asan-selfcompile.XXXXXX")"
ASAN_LOG="$WORK/asan_output.txt"
trap "rm -rf '$WORK'" EXIT

echo -e "${BOLD}ASan Self-Compilation Pipeline${RESET}"
echo -e "${DIM}  build dir:  $BUILD_DIR${RESET}"
echo -e "${DIM}  blood-rust: $BLOOD_REF${RESET}"
echo ""

# ═══════════════════════════════════════════════════════════════════════════════
# Step 1: Build or locate first_gen
# ═══════════════════════════════════════════════════════════════════════════════

if [[ $RUN_ONLY -eq 1 ]]; then
    # Skip straight to running
    if [[ ! -x "$BUILD_DIR/second_gen_asan" ]]; then
        fail "No second_gen_asan found at $BUILD_DIR/second_gen_asan"
        echo "  Run without --run-only first to build the ASan binary."
        exit 1
    fi
elif [[ -n "$IR_FILE" ]]; then
    # Skip to ASan instrumentation with provided IR
    if [[ ! -f "$IR_FILE" ]]; then
        fail "IR file not found: $IR_FILE"
        exit 1
    fi
    echo -e "  Using provided IR: $IR_FILE"
else
    if [[ $REUSE -eq 1 && -x "$BUILD_DIR/first_gen" ]]; then
        step "Reusing existing first_gen"
        ok "first_gen found ($(wc -c < "$BUILD_DIR/first_gen") bytes)"
    else
        step "Building first_gen with blood-rust"
        if [[ ! -f "$BLOOD_REF" ]]; then
            fail "blood-rust not found at $BLOOD_REF"
            exit 1
        fi

        cd "$BUILD_DIR"
        if $BLOOD_REF build main.blood --no-cache -o "$WORK/first_gen_build.ll" 2>"$WORK/build_err.txt"; then
            # blood-rust creates the exe next to the .ll or in CWD
            if [[ -x "$BUILD_DIR/main" ]]; then
                mv "$BUILD_DIR/main" "$BUILD_DIR/first_gen"
            elif [[ -x "$WORK/first_gen_build" ]]; then
                mv "$WORK/first_gen_build" "$BUILD_DIR/first_gen"
            fi
            ok "first_gen built ($(wc -c < "$BUILD_DIR/first_gen") bytes)"
        else
            fail "blood-rust build failed"
            head -20 "$WORK/build_err.txt" | sed 's/^/    /'
            exit 1
        fi
    fi

    # ═══════════════════════════════════════════════════════════════════════════
    # Step 2: Self-compile (first_gen → second_gen.ll)
    # ═══════════════════════════════════════════════════════════════════════════

    step "Self-compiling (first_gen → second_gen.ll)"
    cd "$BUILD_DIR"

    local_start=$(date +%s)
    rc=0
    ./first_gen build main.blood --timings -o second_gen.ll 2>"$WORK/selfcompile_stderr.txt" || rc=$?
    local_end=$(date +%s)
    local_elapsed=$((local_end - local_start))

    if [[ $rc -ne 0 ]]; then
        fail "Self-compilation failed (exit $rc, ${local_elapsed}s)"
        echo "  stderr:"
        head -20 "$WORK/selfcompile_stderr.txt" | sed 's/^/    /'
        exit 1
    fi

    if [[ ! -f "$BUILD_DIR/second_gen.ll" ]]; then
        fail "second_gen.ll not produced"
        exit 1
    fi

    local_lines=$(wc -l < "$BUILD_DIR/second_gen.ll")
    local_defines=$(grep -c '^define ' "$BUILD_DIR/second_gen.ll" || echo 0)
    ok "second_gen.ll produced (${local_lines} lines, ${local_defines} defines, ${local_elapsed}s)"

    IR_FILE="$BUILD_DIR/second_gen.ll"
fi

# ═══════════════════════════════════════════════════════════════════════════════
# Step 3: ASan instrumentation
# ═══════════════════════════════════════════════════════════════════════════════

if [[ $RUN_ONLY -eq 0 ]]; then
    step "Instrumenting with AddressSanitizer"

    # Check tools
    for tool in llvm-as-18 opt-18 llc-18 clang-18; do
        if ! command -v "$tool" &>/dev/null; then
            fail "$tool not found (required for ASan instrumentation)"
            exit 1
        fi
    done

    cd "$BUILD_DIR"

    # Assemble IR to bitcode
    llvm-as-18 "$IR_FILE" -o "$WORK/asan.bc" 2>"$WORK/asm_err.txt"
    if [[ $? -ne 0 ]]; then
        fail "llvm-as failed"
        head -10 "$WORK/asm_err.txt" | sed 's/^/    /'
        exit 1
    fi
    ok "Assembled to bitcode"

    # Apply ASan instrumentation
    opt-18 -passes='module(asan-module),function(asan)' \
        "$WORK/asan.bc" -o "$WORK/asan_inst.bc" 2>"$WORK/opt_err.txt"
    if [[ $? -ne 0 ]]; then
        fail "ASan instrumentation failed"
        head -10 "$WORK/opt_err.txt" | sed 's/^/    /'
        exit 1
    fi
    ok "ASan passes applied"

    # Compile to object
    llc-18 "$WORK/asan_inst.bc" -o "$WORK/asan.o" \
        -filetype=obj -relocation-model=pic 2>"$WORK/llc_err.txt"
    if [[ $? -ne 0 ]]; then
        fail "LLC compilation failed"
        head -10 "$WORK/llc_err.txt" | sed 's/^/    /'
        exit 1
    fi
    ok "Compiled to object"

    # Link with ASan runtime
    clang-18 "$WORK/asan.o" "$RUNTIME_O" "$RUNTIME_A" \
        -fsanitize=address -lstdc++ -lm -lpthread -ldl -no-pie \
        -o "$BUILD_DIR/second_gen_asan" 2>"$WORK/link_err.txt"
    if [[ $? -ne 0 ]]; then
        fail "Linking failed"
        head -10 "$WORK/link_err.txt" | sed 's/^/    /'
        exit 1
    fi
    ok "Linked second_gen_asan ($(wc -c < "$BUILD_DIR/second_gen_asan") bytes)"
fi

# ═══════════════════════════════════════════════════════════════════════════════
# Step 4: Run ASan binary with test command
# ═══════════════════════════════════════════════════════════════════════════════

step "Running ASan binary: second_gen_asan $TEST_CMD"

ASAN_BIN="$BUILD_DIR/second_gen_asan"
if [[ ! -x "$ASAN_BIN" ]]; then
    fail "second_gen_asan not found at $ASAN_BIN"
    exit 1
fi

# Configure ASan options for maximum detail
export ASAN_OPTIONS="detect_leaks=0:print_summary=1:halt_on_error=1:symbolize=1:color=always"

rc=0
cd "$BUILD_DIR"
"$ASAN_BIN" $TEST_CMD >"$WORK/asan_stdout.txt" 2>"$ASAN_LOG" || rc=$?

echo ""
if [[ $rc -eq 0 ]]; then
    ok "No sanitizer errors detected (exit 0)"
    if [[ -s "$WORK/asan_stdout.txt" ]]; then
        echo -e "  ${DIM}stdout:${RESET}"
        head -5 "$WORK/asan_stdout.txt" | sed 's/^/    /'
    fi
else
    fail "ASan detected errors (exit $rc)"

    # Check if ASan produced output
    if [[ -s "$ASAN_LOG" ]]; then
        echo ""
        echo -e "${BOLD}${RED}=== ASan Report ===${RESET}"
        echo ""

        # Parse and format the ASan output
        # Show the error type and first stack trace
        perl -ne '
            # Error type line
            if (/^=+$/ || /^ERROR: AddressSanitizer/) {
                print "\033[1;31m$_\033[0m";
                next;
            }
            # Summary line
            if (/^SUMMARY:/) {
                print "\n\033[1;33m$_\033[0m";
                next;
            }
            # Stack frame with source location
            if (/#\d+\s+0x[0-9a-f]+\s+in\s+(\S+)/) {
                my $func = $1;
                # Highlight the function name
                s/(in\s+)(\S+)/$1\033[1;36m$2\033[0m/;
                print "  $_";
                next;
            }
            # Other ASan lines (allocation info, etc.)
            if (/^\s*(READ|WRITE|freed|allocated|previously|Shadow)/) {
                print "  \033[0;33m$_\033[0m";
                next;
            }
            # Pass through other lines
            print "  $_";
        ' "$ASAN_LOG"

        echo ""
        echo -e "${DIM}Full ASan log: $ASAN_LOG${RESET}"
        echo -e "${DIM}To symbolize: ASAN_SYMBOLIZER_PATH=\$(which llvm-symbolizer-18) $ASAN_BIN $TEST_CMD${RESET}"
    else
        warn "No ASan output captured (binary may have crashed before ASan could report)"
        echo "  Exit code: $rc"
    fi
fi

# ═══════════════════════════════════════════════════════════════════════════════
# Summary
# ═══════════════════════════════════════════════════════════════════════════════

echo ""
echo -e "${BOLD}Summary:${RESET}"
echo -e "  ASan binary:  $ASAN_BIN"
echo -e "  Test command:  $TEST_CMD"
echo -e "  Exit code:     $rc"
if [[ -s "$ASAN_LOG" ]]; then
    local_errors=$(grep -c "^ERROR: AddressSanitizer" "$ASAN_LOG" 2>/dev/null || echo 0)
    echo -e "  ASan errors:   $local_errors"
fi
echo ""
echo -e "${DIM}Re-run with different test: $0 --run-only --test \"check common.blood\"${RESET}"

exit $rc
