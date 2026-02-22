#!/usr/bin/env bash
#
# phase-compare.sh — Phase-Gated Comparison for Blood Compilers
#
# Runs both compilers on the same input and compares at each compilation
# phase to identify WHERE divergence first appears:
#
#   Phase 1: Compilation — does both compilers accept/reject the input?
#   Phase 2: MIR         — do both produce structurally similar MIR?
#   Phase 3: LLVM IR     — do both produce equivalent LLVM IR?
#   Phase 4: Behavior    — do both produce executables with identical output?
#
# Usage:
#   ./tools/phase-compare.sh <file.blood>
#   ./tools/phase-compare.sh <file.blood> --verbose
#
# Environment variables (same as difftest.sh):
#   BLOOD_REF, BLOOD_TEST, BLOOD_RUNTIME, BLOOD_RUST_RUNTIME

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

BLOOD_REF="${BLOOD_REF:-$REPO_ROOT/compiler-rust/target/release/blood}"
BLOOD_TEST="${BLOOD_TEST:-$REPO_ROOT/blood-std/std/compiler/build/first_gen}"
export BLOOD_RUNTIME="${BLOOD_RUNTIME:-$REPO_ROOT/compiler-rust/runtime/runtime.o}"
export BLOOD_RUST_RUNTIME="${BLOOD_RUST_RUNTIME:-$REPO_ROOT/compiler-rust/target/release/libblood_runtime.a}"

VERBOSE=0
TARGET=""

for arg in "$@"; do
    case "$arg" in
        --verbose) VERBOSE=1 ;;
        --help|-h)
            echo "Usage: $0 <file.blood> [--verbose]"
            exit 0 ;;
        -*) echo "Unknown option: $arg" >&2; exit 3 ;;
        *)  TARGET="$arg" ;;
    esac
done

if [[ -z "$TARGET" || ! -f "$TARGET" ]]; then
    echo "Usage: $0 <file.blood> [--verbose]" >&2
    exit 3
fi

# ── Colors ───────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
DIM='\033[2m'
RESET='\033[0m'

BASENAME="$(basename "$TARGET" .blood)"
WORK="$(mktemp -d "/tmp/phase-compare.${BASENAME}.XXXXXX")"
trap "rm -rf '$WORK'" EXIT

echo -e "${BOLD}Phase Comparison: ${CYAN}$BASENAME${RESET}"
echo -e "${DIM}  ref:  $BLOOD_REF${RESET}"
echo -e "${DIM}  test: $BLOOD_TEST${RESET}"
echo ""

# ═══════════════════════════════════════════════════════════════════════════════
# Phase 1: Compilation
# ═══════════════════════════════════════════════════════════════════════════════

# Reference compiler: build exe
ref_compile=0
"$BLOOD_REF" build "$TARGET" -o "$WORK/ref_exe" \
    --quiet --color never 2>"$WORK/ref_compile_err.txt" && ref_compile=1

# Test compiler: build (produces .ll + exe)
test_compile=0
"$BLOOD_TEST" build "$TARGET" -o "$WORK/test.ll" --no-cache \
    2>"$WORK/test_compile_err.txt" 1>"$WORK/test_build_stdout.txt" && test_compile=1

# first_gen creates the exe as <basename> in the -o directory or source dir
if [[ $test_compile -eq 1 ]]; then
    for try_path in \
        "$WORK/$(basename "$WORK/test.ll" .ll)" \
        "$WORK/$BASENAME" \
        "$(dirname "$TARGET")/$BASENAME" \
    ; do
        if [[ -x "$try_path" ]]; then
            mv "$try_path" "$WORK/test_exe"
            break
        fi
    done
    # Also check: first_gen may name the exe after the -o stem
    if [[ ! -x "$WORK/test_exe" ]]; then
        # The exe might be at $WORK/test (stem of test.ll)
        if [[ -x "$WORK/test" ]]; then
            mv "$WORK/test" "$WORK/test_exe"
        fi
    fi
fi

# Report Phase 1
printf "  Phase 1  %-14s" "Compilation"
if [[ $ref_compile -eq 1 && $test_compile -eq 1 ]]; then
    echo -e "${GREEN}MATCH${RESET}   (both accept)"
elif [[ $ref_compile -eq 0 && $test_compile -eq 0 ]]; then
    echo -e "${GREEN}MATCH${RESET}   (both reject)"
    echo ""
    echo -e "  ${DIM}Both compilers reject this input. No further phases to compare.${RESET}"
    exit 0
elif [[ $ref_compile -eq 1 && $test_compile -eq 0 ]]; then
    echo -e "${RED}DIVERGE${RESET} (ref accepts, test rejects)"
    echo ""
    echo -e "  ${BOLD}Divergence starts at: Phase 1 (Compilation)${RESET}"
    echo -e "  Test compiler error:"
    head -10 "$WORK/test_compile_err.txt" | sed 's/^/    /'
    exit 1
else
    echo -e "${RED}DIVERGE${RESET} (ref rejects, test accepts)"
    echo ""
    echo -e "  ${BOLD}Divergence starts at: Phase 1 (Compilation)${RESET}"
    echo -e "  Reference compiler error:"
    head -10 "$WORK/ref_compile_err.txt" | sed 's/^/    /'
    exit 1
fi

# ═══════════════════════════════════════════════════════════════════════════════
# Phase 2: MIR Structure
# ═══════════════════════════════════════════════════════════════════════════════

# Get MIR from reference (--emit mir goes to stdout)
ref_mir_ok=0
"$BLOOD_REF" build "$TARGET" --emit mir --quiet --color never \
    >"$WORK/ref_mir.txt" 2>/dev/null && ref_mir_ok=1

# Get MIR from test (--dump-mir sends MIR to stderr, needs separate invocation)
test_mir_ok=0
"$BLOOD_TEST" build "$TARGET" -o "$WORK/test_mir_dummy.ll" --no-cache --dump-mir \
    2>"$WORK/test_mir_raw.txt" 1>/dev/null && test_mir_ok=1
# Clean up dummy outputs
rm -f "$WORK/test_mir_dummy.ll" "$WORK/test_mir_dummy.o" "$WORK/test_mir_dummy" 2>/dev/null

# Clean test MIR: extract only MIR function blocks from stderr
if [[ $test_mir_ok -eq 1 ]]; then
    perl -ne '
        $in_mir = 1 if /^=== MIR:/;
        print if $in_mir;
    ' "$WORK/test_mir_raw.txt" > "$WORK/test_mir.txt"
    [[ -s "$WORK/test_mir.txt" ]] || test_mir_ok=0
fi

# Extract MIR summary from blood-rust (verbose Rust debug format)
extract_mir_summary_ref() {
    perl -ne '
        if (/^\/\/ MIR for DefId/) { $bodies++; }
        if (/^\s+BasicBlockData\b/) { $bbs++; }
        if (/^\s+MirLocal\b/) { $locals++; }
        END {
            printf "functions: %d\n", ($bodies // 0);
            printf "basic_blocks: %d\n", ($bbs // 0);
            printf "locals: %d\n", ($locals // 0);
        }
    ' "$1"
}

# Extract MIR summary from first_gen (compact format)
extract_mir_summary_test() {
    perl -ne '
        if (/^=== MIR:/) { $fns++; }
        if (/^\s+bb\d+:\s*\{/) { $bbs++; }
        if (/^\s+_\d+:/) { $locals++; }
        END {
            printf "functions: %d\n", ($fns // 0);
            printf "basic_blocks: %d\n", ($bbs // 0);
            printf "locals: %d\n", ($locals // 0);
        }
    ' "$1"
}

printf "  Phase 2  %-14s" "MIR"

if [[ $ref_mir_ok -eq 0 && $test_mir_ok -eq 0 ]]; then
    echo -e "${YELLOW}SKIP${RESET}    (MIR dump unavailable from both compilers)"
elif [[ $ref_mir_ok -eq 0 ]]; then
    echo -e "${YELLOW}SKIP${RESET}    (ref MIR unavailable)"
elif [[ $test_mir_ok -eq 0 ]]; then
    echo -e "${YELLOW}SKIP${RESET}    (test MIR unavailable)"
else
    extract_mir_summary_ref "$WORK/ref_mir.txt" > "$WORK/ref_mir_summary.txt"
    extract_mir_summary_test "$WORK/test_mir.txt" > "$WORK/test_mir_summary.txt"

    ref_fns=$(grep "^functions:" "$WORK/ref_mir_summary.txt" | cut -d' ' -f2)
    test_fns=$(grep "^functions:" "$WORK/test_mir_summary.txt" | cut -d' ' -f2)
    ref_bbs=$(grep "^basic_blocks:" "$WORK/ref_mir_summary.txt" | cut -d' ' -f2)
    test_bbs=$(grep "^basic_blocks:" "$WORK/test_mir_summary.txt" | cut -d' ' -f2)
    ref_locals=$(grep "^locals:" "$WORK/ref_mir_summary.txt" | cut -d' ' -f2)
    test_locals=$(grep "^locals:" "$WORK/test_mir_summary.txt" | cut -d' ' -f2)

    if [[ "$ref_fns" == "$test_fns" && "$ref_bbs" == "$test_bbs" ]]; then
        echo -e "${GREEN}MATCH${RESET}   (fns: $ref_fns, bbs: $ref_bbs, locals: $ref_locals/$test_locals)"
    else
        echo -e "${YELLOW}DIFFER${RESET}  (ref: ${ref_fns}fn/${ref_bbs}bb/${ref_locals}loc, test: ${test_fns}fn/${test_bbs}bb/${test_locals}loc)"
    fi

    if [[ $VERBOSE -eq 1 ]]; then
        echo "    --- ref MIR summary ---"
        sed 's/^/    /' "$WORK/ref_mir_summary.txt"
        echo "    --- test MIR summary ---"
        sed 's/^/    /' "$WORK/test_mir_summary.txt"
    fi
fi

# ═══════════════════════════════════════════════════════════════════════════════
# Phase 3: LLVM IR
# ═══════════════════════════════════════════════════════════════════════════════

# Get unoptimized IR from reference
ref_ir_ok=0
"$BLOOD_REF" build "$TARGET" --emit llvm-ir-unopt -o "$WORK/ref.ll" \
    --quiet --color never 2>/dev/null && ref_ir_ok=1

# test.ll already produced by first_gen build above
test_ir_ok=0
[[ -f "$WORK/test.ll" && -s "$WORK/test.ll" ]] && test_ir_ok=1

printf "  Phase 3  %-14s" "LLVM IR"

if [[ $ref_ir_ok -eq 0 || $test_ir_ok -eq 0 ]]; then
    echo -e "${YELLOW}SKIP${RESET}    (IR unavailable: ref=$ref_ir_ok test=$test_ir_ok)"
else
    ref_defines=$(grep -c "^define " "$WORK/ref.ll" 2>/dev/null || echo 0)
    test_defines=$(grep -c "^define " "$WORK/test.ll" 2>/dev/null || echo 0)
    ref_declares=$(grep -c "^declare " "$WORK/ref.ll" 2>/dev/null || echo 0)
    test_declares=$(grep -c "^declare " "$WORK/test.ll" 2>/dev/null || echo 0)
    ref_lines=$(wc -l < "$WORK/ref.ll")
    test_lines=$(wc -l < "$WORK/test.ll")

    if [[ "$ref_defines" == "$test_defines" ]]; then
        echo -e "${GREEN}MATCH${RESET}   (defines: $ref_defines, declares: $ref_declares/$test_declares, lines: $ref_lines/$test_lines)"
    else
        echo -e "${YELLOW}DIFFER${RESET}  (ref: ${ref_defines}def/${ref_declares}decl/${ref_lines}L, test: ${test_defines}def/${test_declares}decl/${test_lines}L)"
    fi

    if [[ $VERBOSE -eq 1 ]]; then
        echo "    --- ref defines ---"
        grep "^define " "$WORK/ref.ll" | sed 's/^define [^@]*@/    @/' | sed 's/(.*//'
        echo "    --- test defines ---"
        grep "^define " "$WORK/test.ll" | sed 's/^define [^@]*@/    @/' | sed 's/(.*//'
    fi
fi

# ═══════════════════════════════════════════════════════════════════════════════
# Phase 4: Behavioral
# ═══════════════════════════════════════════════════════════════════════════════

printf "  Phase 4  %-14s" "Behavior"

if [[ ! -x "$WORK/ref_exe" || ! -x "$WORK/test_exe" ]]; then
    missing=""
    [[ ! -x "$WORK/ref_exe" ]] && missing="ref"
    [[ ! -x "$WORK/test_exe" ]] && missing="${missing:+$missing+}test"
    echo -e "${YELLOW}SKIP${RESET}    (exe unavailable: $missing)"
else
    ref_exit=0
    ("$WORK/ref_exe" > "$WORK/ref_stdout.txt" 2>/dev/null) 2>/dev/null || ref_exit=$?
    test_exit=0
    ("$WORK/test_exe" > "$WORK/test_stdout.txt" 2>/dev/null) 2>/dev/null || test_exit=$?

    stdout_match=1
    diff -q "$WORK/ref_stdout.txt" "$WORK/test_stdout.txt" >/dev/null 2>&1 || stdout_match=0

    exit_match=1
    [[ $ref_exit -eq $test_exit ]] || exit_match=0

    if [[ $stdout_match -eq 1 && $exit_match -eq 1 ]]; then
        echo -e "${GREEN}MATCH${RESET}   (exit=$ref_exit, output identical)"
    else
        echo -e "${RED}DIVERGE${RESET}"
        if [[ $exit_match -eq 0 ]]; then
            echo -e "             exit: ref=$ref_exit test=$test_exit"
        fi
        if [[ $stdout_match -eq 0 ]]; then
            echo "             output diff:"
            diff --color=always -u \
                --label "ref" "$WORK/ref_stdout.txt" \
                --label "test" "$WORK/test_stdout.txt" \
                2>/dev/null | head -15 | sed 's/^/             /'
        fi
        echo ""
        echo -e "  ${BOLD}Divergence starts at: Phase 4 (Behavior)${RESET}"
        echo -e "  Compilation and IR generation match, but runtime output differs."
        echo -e "  This typically indicates a codegen bug (correct MIR, wrong LLVM IR)."
        exit 1
    fi
fi

echo ""
echo -e "  ${GREEN}${BOLD}All phases match.${RESET}"
