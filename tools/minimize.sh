#!/usr/bin/env bash
#
# minimize.sh — Test Case Minimizer for Blood Compiler Bugs
#
# Given a .blood file that exhibits a bug under the test compiler (first_gen),
# automatically reduces it to the smallest program that still triggers the bug.
#
# Failure modes detected:
#   crash        — test compiler produces an executable that crashes (nonzero exit)
#   wrong-output — test compiler produces an executable with different output than reference
#   compile-fail — test compiler rejects the file but reference compiler accepts it
#   compile-crash — test compiler crashes/aborts during compilation
#
# Usage:
#   ./tools/minimize.sh <file.blood>                    # auto-detect failure mode
#   ./tools/minimize.sh <file.blood> --mode crash       # explicit mode
#   ./tools/minimize.sh <file.blood> --mode wrong-output
#   ./tools/minimize.sh <file.blood> --mode compile-fail
#   ./tools/minimize.sh <file.blood> --mode compile-crash
#   ./tools/minimize.sh <file.blood> --keep-temps       # don't clean up work dir
#
# Output:
#   Writes the minimized .blood file to stdout.
#   Progress is printed to stderr.
#   The minimized file is also saved next to the original as <name>.min.blood.
#
# Environment variables (same as difftest.sh):
#   BLOOD_REF, BLOOD_TEST, BLOOD_RUNTIME, BLOOD_RUST_RUNTIME

set -euo pipefail

# ── Defaults ─────────────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

BLOOD_REF="${BLOOD_REF:-$REPO_ROOT/src/bootstrap/target/release/blood}"
BLOOD_TEST="${BLOOD_TEST:-$REPO_ROOT/src/selfhost/build/first_gen}"
export BLOOD_RUNTIME="${BLOOD_RUNTIME:-$REPO_ROOT/runtime/runtime.o}"
export BLOOD_RUST_RUNTIME="${BLOOD_RUST_RUNTIME:-$REPO_ROOT/src/bootstrap/target/release/libblood_runtime.a}"

MODE=""
KEEP_TEMPS=0
TARGET=""

# ── Argument parsing ─────────────────────────────────────────────────────────

usage() {
    echo "Usage: $0 <file.blood> [--mode crash|wrong-output|compile-fail|compile-crash] [--keep-temps]" >&2
    exit 3
}

for arg in "$@"; do
    case "$arg" in
        --mode)       shift_next=1 ;;
        crash|wrong-output|compile-fail|compile-crash)
            if [[ "${shift_next:-0}" -eq 1 ]]; then
                MODE="$arg"; shift_next=0
            else
                TARGET="$arg"
            fi ;;
        --keep-temps) KEEP_TEMPS=1 ;;
        --help|-h)    usage ;;
        -*)           echo "Unknown option: $arg" >&2; usage ;;
        *)            TARGET="$arg" ;;
    esac
done

# Handle --mode with next positional
if [[ -z "$TARGET" ]]; then usage; fi
if [[ ! -f "$TARGET" ]]; then
    echo "Error: file not found: $TARGET" >&2
    exit 3
fi

# ── Work directory ───────────────────────────────────────────────────────────

BASENAME="$(basename "$TARGET" .blood)"
WORKDIR="$(mktemp -d "/tmp/minimize.${BASENAME}.XXXXXX")"
if [[ $KEEP_TEMPS -eq 0 ]]; then
    trap "rm -rf '$WORKDIR'" EXIT
fi

log() { echo "  [minimize] $*" >&2; }

# ── Oracle: does a given .blood file still trigger the bug? ──────────────────

# Returns 0 if the bug is still present (we want to KEEP reducing).
# Returns 1 if the bug disappeared (this reduction went too far).
oracle() {
    local candidate="$1"
    local ref_ok=1
    local test_ok=1

    # Compile with reference
    if ! "$BLOOD_REF" build "$candidate" -o "$WORKDIR/ref_exe" \
         --quiet --color never 2>"$WORKDIR/ref_err.txt"; then
        ref_ok=0
    fi

    # Compile with test
    if ! "$BLOOD_TEST" build "$candidate" -o "$WORKDIR/test_exe" --no-cache \
         2>"$WORKDIR/test_err.txt" 1>/dev/null; then
        test_ok=0
    fi
    # first_gen may put exe elsewhere
    if [[ $test_ok -eq 1 && ! -x "$WORKDIR/test_exe" ]]; then
        local cand_base="$(basename "$candidate" .blood)"
        local cand_dir="$(dirname "$candidate")"
        for try_path in "$cand_dir/$cand_base" "$WORKDIR/$cand_base"; do
            if [[ -x "$try_path" ]]; then
                mv "$try_path" "$WORKDIR/test_exe"
                break
            fi
        done
        if [[ ! -x "$WORKDIR/test_exe" ]]; then
            test_ok=0
        fi
    fi

    case "$MODE" in
        compile-fail)
            # Bug: test compiler rejects, reference accepts
            if [[ $ref_ok -eq 1 && $test_ok -eq 0 ]]; then
                return 0  # bug still present
            fi
            return 1
            ;;
        compile-crash)
            # Bug: test compiler crashes (segfault/abort) during compilation
            if [[ $test_ok -eq 0 ]]; then
                # Check if it was a signal death vs normal error
                local err_text
                err_text="$(cat "$WORKDIR/test_err.txt" 2>/dev/null || true)"
                if echo "$err_text" | grep -qi "abort\|segfault\|fatal\|assertion\|signal"; then
                    return 0  # bug still present
                fi
                # Also check exit code > 128 (signal)
                return 0  # treat any test failure as bug present for this mode
            fi
            return 1
            ;;
        crash)
            # Bug: both compile, but test exe crashes
            if [[ $ref_ok -eq 0 || $test_ok -eq 0 ]]; then
                return 1  # can't test if won't compile
            fi
            local test_exit=0
            ("$WORKDIR/test_exe" > "$WORKDIR/test_out.txt" 2>/dev/null) 2>/dev/null || test_exit=$?
            if [[ $test_exit -ne 0 ]]; then
                return 0  # bug still present
            fi
            return 1
            ;;
        wrong-output)
            # Bug: both compile, both run, but output differs
            if [[ $ref_ok -eq 0 || $test_ok -eq 0 ]]; then
                return 1
            fi
            local ref_exit=0
            ("$WORKDIR/ref_exe" > "$WORKDIR/ref_out.txt" 2>/dev/null) 2>/dev/null || ref_exit=$?
            local test_exit=0
            ("$WORKDIR/test_exe" > "$WORKDIR/test_out.txt" 2>/dev/null) 2>/dev/null || test_exit=$?

            if ! diff -q "$WORKDIR/ref_out.txt" "$WORKDIR/test_out.txt" >/dev/null 2>&1; then
                return 0  # bug still present — output differs
            fi
            if [[ $ref_exit -ne $test_exit ]]; then
                return 0  # bug still present — exit code differs
            fi
            return 1
            ;;
    esac
    return 1
}

# ── Auto-detect failure mode ─────────────────────────────────────────────────

if [[ -z "$MODE" ]]; then
    log "Auto-detecting failure mode..."
    cp "$TARGET" "$WORKDIR/detect.blood"

    # Try compiling with both
    local_ref_ok=1
    "$BLOOD_REF" build "$WORKDIR/detect.blood" -o "$WORKDIR/detect_ref" \
        --quiet --color never 2>/dev/null || local_ref_ok=0

    local_test_ok=1
    "$BLOOD_TEST" build "$WORKDIR/detect.blood" -o "$WORKDIR/detect_test" --no-cache \
        2>"$WORKDIR/detect_test_err.txt" 1>/dev/null || local_test_ok=0
    # Find test exe
    if [[ $local_test_ok -eq 1 && ! -x "$WORKDIR/detect_test" ]]; then
        for try_path in "$WORKDIR/$BASENAME" "$(dirname "$TARGET")/$BASENAME"; do
            if [[ -x "$try_path" ]]; then
                mv "$try_path" "$WORKDIR/detect_test"
                break
            fi
        done
        [[ -x "$WORKDIR/detect_test" ]] || local_test_ok=0
    fi

    if [[ $local_ref_ok -eq 1 && $local_test_ok -eq 0 ]]; then
        # Check for crash signals in stderr
        if grep -qi "abort\|fatal\|assertion\|signal" "$WORKDIR/detect_test_err.txt" 2>/dev/null; then
            MODE="compile-crash"
        else
            MODE="compile-fail"
        fi
    elif [[ $local_ref_ok -eq 1 && $local_test_ok -eq 1 ]]; then
        # Both compile — run both
        ref_exit=0
        ("$WORKDIR/detect_ref" > "$WORKDIR/detect_ref_out.txt" 2>/dev/null) 2>/dev/null || ref_exit=$?
        test_exit=0
        ("$WORKDIR/detect_test" > "$WORKDIR/detect_test_out.txt" 2>/dev/null) 2>/dev/null || test_exit=$?

        if [[ $test_exit -gt 128 ]]; then
            MODE="crash"
        elif [[ $test_exit -ne 0 && $ref_exit -eq 0 ]]; then
            MODE="crash"
        elif ! diff -q "$WORKDIR/detect_ref_out.txt" "$WORKDIR/detect_test_out.txt" >/dev/null 2>&1; then
            MODE="wrong-output"
        elif [[ $ref_exit -ne $test_exit ]]; then
            MODE="wrong-output"
        else
            echo "Error: no bug detected — both compilers produce identical results" >&2
            exit 1
        fi
    else
        echo "Error: reference compiler also fails on this input" >&2
        exit 1
    fi
    log "Detected mode: $MODE"
fi

# ── Verify the original file triggers the bug ────────────────────────────────

cp "$TARGET" "$WORKDIR/current.blood"
if ! oracle "$WORKDIR/current.blood"; then
    echo "Error: original file does not trigger the bug in mode '$MODE'" >&2
    exit 1
fi

original_lines=$(wc -l < "$WORKDIR/current.blood")
log "Original: $original_lines lines, mode=$MODE"

# ── Reduction passes ─────────────────────────────────────────────────────────

pass_count=0
reduced=1

while [[ $reduced -eq 1 ]]; do
    reduced=0
    pass_count=$((pass_count + 1))
    current_lines=$(wc -l < "$WORKDIR/current.blood")
    log "Pass $pass_count ($current_lines lines)"

    # ── Pass A: Remove top-level items ──────────────────────────────────────
    # Identify top-level item boundaries (lines starting with fn/struct/enum/impl/mod/pub/trait/effect/use)
    # Try removing each item block. Keep main fn.

    # Extract item start lines
    item_starts=()
    while IFS= read -r line_info; do
        item_starts+=("$line_info")
    done < <(grep -n '^\(pub \)\?\(fn \|struct \|enum \|impl \|mod \|trait \|effect \|use \|type \)' \
        "$WORKDIR/current.blood" 2>/dev/null | cut -d: -f1)

    if [[ ${#item_starts[@]} -gt 1 ]]; then
        total_lines=$(wc -l < "$WORKDIR/current.blood")

        # Try removing each item (in reverse order to preserve line numbers)
        for ((idx=${#item_starts[@]}-1; idx>=0; idx--)); do
            start="${item_starts[$idx]}"

            # Find end of this item: next item start - 1, or EOF
            if [[ $((idx + 1)) -lt ${#item_starts[@]} ]]; then
                end=$((${item_starts[$((idx + 1))]} - 1))
            else
                end=$total_lines
            fi

            # Don't remove main function
            item_line=$(sed -n "${start}p" "$WORKDIR/current.blood")
            if echo "$item_line" | grep -q 'fn main\b'; then
                continue
            fi

            # Try removing lines start..end
            sed "${start},${end}d" "$WORKDIR/current.blood" > "$WORKDIR/candidate.blood"

            if oracle "$WORKDIR/candidate.blood"; then
                cp "$WORKDIR/candidate.blood" "$WORKDIR/current.blood"
                reduced=1
                new_lines=$(wc -l < "$WORKDIR/current.blood")
                log "  Removed item at line $start ($((end - start + 1)) lines) → $new_lines lines"
                break  # restart pass since line numbers changed
            fi
        done
    fi

    # ── Pass B: Remove individual lines/statements ──────────────────────────
    # Try removing each non-brace, non-fn-signature line one at a time
    total_lines=$(wc -l < "$WORKDIR/current.blood")
    for ((lineno=total_lines; lineno>=1; lineno--)); do
        line_text=$(sed -n "${lineno}p" "$WORKDIR/current.blood")

        # Skip essential structural lines
        case "$line_text" in
            ""|"}"*|"{"*|"// "*|"fn main"*|"pub fn main"*) continue ;;
        esac

        sed "${lineno}d" "$WORKDIR/current.blood" > "$WORKDIR/candidate.blood"

        if oracle "$WORKDIR/candidate.blood"; then
            cp "$WORKDIR/candidate.blood" "$WORKDIR/current.blood"
            reduced=1
            new_lines=$(wc -l < "$WORKDIR/current.blood")
            log "  Removed line $lineno → $new_lines lines"
            break  # restart pass
        fi
    done

    # ── Pass C: Blank line cleanup ──────────────────────────────────────────
    # Remove consecutive blank lines (cosmetic, doesn't affect oracle)
    sed '/^$/{ N; /^\n$/d; }' "$WORKDIR/current.blood" > "$WORKDIR/cleaned.blood"
    if oracle "$WORKDIR/cleaned.blood"; then
        cp "$WORKDIR/cleaned.blood" "$WORKDIR/current.blood"
    fi
done

# ── Output ───────────────────────────────────────────────────────────────────

final_lines=$(wc -l < "$WORKDIR/current.blood")
log "Done: $original_lines → $final_lines lines ($pass_count passes)"

# Save next to original
output_path="$(dirname "$TARGET")/${BASENAME}.min.blood"
cp "$WORKDIR/current.blood" "$output_path"
log "Saved to: $output_path"

# Print to stdout
cat "$WORKDIR/current.blood"

if [[ $KEEP_TEMPS -eq 1 ]]; then
    log "Work directory: $WORKDIR"
fi
