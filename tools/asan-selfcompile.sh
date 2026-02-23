#!/usr/bin/env bash
#
# asan-selfcompile.sh — AddressSanitizer Self-Compilation Wrapper
#
# One-command wrapper that:
#   1. Locates or builds the input compiler (any generation)
#   2. Self-compiles (input_gen → next_gen.ll)
#   3. Instruments next_gen.ll with AddressSanitizer
#   4. Runs the ASan-instrumented binary with a smoke test
#   5. Formats and reports any sanitizer findings
#
# Usage:
#   ./tools/asan-selfcompile.sh                                     # first_gen → second_gen_asan
#   ./tools/asan-selfcompile.sh --compiler ./build/second_gen       # second_gen → third_gen_asan
#   ./tools/asan-selfcompile.sh --reuse                             # Reuse existing input compiler
#   ./tools/asan-selfcompile.sh --ir FILE.ll                        # Instrument existing IR directly
#   ./tools/asan-selfcompile.sh --run-only                          # Run existing ASan binary
#   ./tools/asan-selfcompile.sh --test CMD                          # Custom test command (default: "version")
#
# Environment variables:
#   BLOOD_TEST          — input compiler for self-compilation (same as --compiler; flag takes precedence)
#   BLOOD_REF           — reference compiler for building first_gen (default: blood-rust)
#   BLOOD_RUNTIME       — path to runtime.o
#   BLOOD_RUST_RUNTIME  — path to libblood_runtime.a
#   BUILD_DIR           — compiler source directory (default: <repo>/blood-std/std/compiler)

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

BLOOD_REF="${BLOOD_REF:-$REPO_ROOT/src/bootstrap/target/release/blood}"
RUNTIME_O="${RUNTIME_O:-$REPO_ROOT/runtime/runtime.o}"
RUNTIME_A="${RUNTIME_A:-$REPO_ROOT/src/bootstrap/target/release/libblood_runtime.a}"
BUILD_DIR="${BUILD_DIR:-$REPO_ROOT/blood-std/std/compiler}"

export BLOOD_RUNTIME="${BLOOD_RUNTIME:-$RUNTIME_O}"
export BLOOD_RUST_RUNTIME="${BLOOD_RUST_RUNTIME:-$RUNTIME_A}"

REUSE=0
IR_FILE=""
RUN_ONLY=0
TEST_CMD="version"
COMPILER_ARG=""

shift_next=""
for arg in "$@"; do
    if [[ -n "$shift_next" ]]; then
        case "$shift_next" in
            ir)       IR_FILE="$arg" ;;
            test)     TEST_CMD="$arg" ;;
            compiler) COMPILER_ARG="$arg" ;;
        esac
        shift_next=""
        continue
    fi
    case "$arg" in
        --reuse)     REUSE=1 ;;
        --run-only)  RUN_ONLY=1 ;;
        --ir)        shift_next=ir ;;
        --test)      shift_next=test ;;
        --compiler)  shift_next=compiler ;;
        --help|-h)
            echo "Usage: $0 [--compiler PATH] [--reuse] [--ir FILE.ll] [--run-only] [--test CMD]"
            echo ""
            echo "Options:"
            echo "  --compiler PATH  Input compiler for self-compilation (default: first_gen)"
            echo "  --reuse          Reuse existing input compiler (skip blood-rust rebuild)"
            echo "  --ir FILE.ll     Instrument existing IR directly (skip steps 1-2)"
            echo "  --run-only       Run existing ASan binary (skip steps 1-3)"
            echo "  --test CMD       Test command for ASan binary (default: \"version\")"
            echo ""
            echo "Environment:"
            echo "  BLOOD_TEST       Input compiler (same as --compiler; flag takes precedence)"
            echo "  BLOOD_REF        Reference compiler for building first_gen"
            echo "  BUILD_DIR        Compiler source directory"
            echo ""
            echo "Examples:"
            echo "  $0                                          # first_gen -> second_gen_asan"
            echo "  $0 --compiler ./build/second_gen            # second_gen -> third_gen_asan"
            echo "  BLOOD_TEST=./build/second_gen $0 --reuse    # same, via env var"
            echo "  $0 --run-only --test 'check common.blood'   # re-run existing ASan binary"
            exit 0 ;;
        -*)  echo "Unknown option: $arg" >&2; exit 3 ;;
        *)   echo "Unexpected argument: $arg" >&2; exit 3 ;;
    esac
done

# ── Resolve input compiler ────────────────────────────────────────────────────

# --compiler flag takes precedence over BLOOD_TEST env var
if [[ -n "$COMPILER_ARG" ]]; then
    INPUT_COMPILER="$COMPILER_ARG"
elif [[ -n "${BLOOD_TEST:-}" ]]; then
    INPUT_COMPILER="$BLOOD_TEST"
else
    INPUT_COMPILER="$BUILD_DIR/first_gen"
fi

# Make absolute
if [[ "$INPUT_COMPILER" != /* ]]; then
    INPUT_COMPILER="$PWD/$INPUT_COMPILER"
fi

INPUT_NAME="$(basename "$INPUT_COMPILER")"

# ── Derive output generation name ─────────────────────────────────────────────

next_gen_name() {
    case "$1" in
        first_gen)   echo "second_gen" ;;
        second_gen)  echo "third_gen" ;;
        third_gen)   echo "fourth_gen" ;;
        fourth_gen)  echo "fifth_gen" ;;
        fifth_gen)   echo "sixth_gen" ;;
        *)           echo "${1}_next" ;;
    esac
}

OUTPUT_NAME="$(next_gen_name "$INPUT_NAME")"
ASAN_NAME="${OUTPUT_NAME}_asan"

# ── Colors ────────────────────────────────────────────────────────────────────
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
echo -e "${DIM}  input:      $INPUT_COMPILER${RESET}"
echo -e "${DIM}  pipeline:   $INPUT_NAME → $OUTPUT_NAME.ll → $ASAN_NAME${RESET}"
echo -e "${DIM}  blood-rust: $BLOOD_REF${RESET}"
echo ""

# ═══════════════════════════════════════════════════════════════════════════════
# Step 1: Locate or build the input compiler
# ═══════════════════════════════════════════════════════════════════════════════

if [[ $RUN_ONLY -eq 1 ]]; then
    # Skip straight to running
    if [[ ! -x "$BUILD_DIR/$ASAN_NAME" ]]; then
        fail "No $ASAN_NAME found at $BUILD_DIR/$ASAN_NAME"
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
    if [[ $REUSE -eq 1 && -x "$INPUT_COMPILER" ]]; then
        step "Reusing existing $INPUT_NAME"
        ok "$INPUT_NAME found ($(wc -c < "$INPUT_COMPILER") bytes)"
    elif [[ "$INPUT_NAME" == "first_gen" && $REUSE -eq 0 ]]; then
        # Auto-build first_gen from blood-rust
        step "Building first_gen with blood-rust"
        if [[ ! -f "$BLOOD_REF" ]]; then
            fail "blood-rust not found at $BLOOD_REF"
            exit 1
        fi

        cd "$BUILD_DIR"
        if $BLOOD_REF build main.blood --no-cache -o "$WORK/first_gen_build.ll" 2>"$WORK/build_err.txt"; then
            # blood-rust creates the exe next to the .ll or in CWD
            if [[ -x "$BUILD_DIR/main" ]]; then
                mv "$BUILD_DIR/main" "$INPUT_COMPILER"
            elif [[ -x "$WORK/first_gen_build" ]]; then
                mv "$WORK/first_gen_build" "$INPUT_COMPILER"
            fi
            ok "first_gen built ($(wc -c < "$INPUT_COMPILER") bytes)"
        else
            fail "blood-rust build failed"
            head -20 "$WORK/build_err.txt" | sed 's/^/    /'
            exit 1
        fi
    elif [[ -x "$INPUT_COMPILER" ]]; then
        step "Using existing $INPUT_NAME"
        ok "$INPUT_NAME found ($(wc -c < "$INPUT_COMPILER") bytes)"
    else
        fail "$INPUT_NAME not found at $INPUT_COMPILER"
        echo "  Build it first, or use --compiler to specify a different input."
        exit 1
    fi

    # ═══════════════════════════════════════════════════════════════════════════
    # Step 2: Self-compile (input_gen → next_gen.ll)
    # ═══════════════════════════════════════════════════════════════════════════

    step "Self-compiling ($INPUT_NAME → $OUTPUT_NAME.ll)"
    cd "$BUILD_DIR"

    local_start=$(date +%s)
    rc=0
    "$INPUT_COMPILER" build main.blood --timings -o "$OUTPUT_NAME.ll" 2>"$WORK/selfcompile_stderr.txt" || rc=$?
    local_end=$(date +%s)
    local_elapsed=$((local_end - local_start))

    if [[ $rc -ne 0 ]]; then
        fail "Self-compilation failed (exit $rc, ${local_elapsed}s)"
        echo "  stderr:"
        head -20 "$WORK/selfcompile_stderr.txt" | sed 's/^/    /'
        exit 1
    fi

    if [[ ! -f "$BUILD_DIR/$OUTPUT_NAME.ll" ]]; then
        fail "$OUTPUT_NAME.ll not produced"
        exit 1
    fi

    local_lines=$(wc -l < "$BUILD_DIR/$OUTPUT_NAME.ll")
    local_defines=$(grep -c '^define ' "$BUILD_DIR/$OUTPUT_NAME.ll" || echo 0)
    ok "$OUTPUT_NAME.ll produced (${local_lines} lines, ${local_defines} defines, ${local_elapsed}s)"

    IR_FILE="$BUILD_DIR/$OUTPUT_NAME.ll"
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
        -o "$BUILD_DIR/$ASAN_NAME" 2>"$WORK/link_err.txt"
    if [[ $? -ne 0 ]]; then
        fail "Linking failed"
        head -10 "$WORK/link_err.txt" | sed 's/^/    /'
        exit 1
    fi
    ok "Linked $ASAN_NAME ($(wc -c < "$BUILD_DIR/$ASAN_NAME") bytes)"
fi

# ═══════════════════════════════════════════════════════════════════════════════
# Step 4: Run ASan binary with test command
# ═══════════════════════════════════════════════════════════════════════════════

step "Running ASan binary: $ASAN_NAME $TEST_CMD"

ASAN_BIN="$BUILD_DIR/$ASAN_NAME"
if [[ ! -x "$ASAN_BIN" ]]; then
    fail "$ASAN_NAME not found at $ASAN_BIN"
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
echo -e "  Pipeline:      $INPUT_NAME → $ASAN_NAME"
echo -e "  ASan binary:   $ASAN_BIN"
echo -e "  Test command:  $TEST_CMD"
echo -e "  Exit code:     $rc"
if [[ -s "$ASAN_LOG" ]]; then
    local_errors=$(grep -c "^ERROR: AddressSanitizer" "$ASAN_LOG" 2>/dev/null || echo 0)
    echo -e "  ASan errors:   $local_errors"
fi
echo ""
echo -e "${DIM}Re-run with different test: $0 --run-only --test \"check common.blood\"${RESET}"

exit $rc
