#!/usr/bin/env bash
#
# difftest.sh — Differential Testing Harness for Blood Compilers
#
# Compiles the same .blood file with both the reference compiler (blood-rust)
# and the test compiler (any generation), extracts per-function LLVM IR,
# matches functions by name, and reports divergences.
#
# Usage:
#   ./tools/difftest.sh <file.blood>                   # single file
#   ./tools/difftest.sh <dir>                           # batch (all .blood in dir)
#   ./tools/difftest.sh <file.blood> --verbose          # show full diffs
#   ./tools/difftest.sh <file.blood> --summary-only     # counts only
#   ./tools/difftest.sh <file.blood> --first-divergence # stop at first mismatch
#
# Environment variables (override defaults):
#   BLOOD_REF       — path to reference compiler (blood-rust)
#   BLOOD_TEST      — path to test compiler (any generation)
#   BLOOD_RUNTIME   — path to runtime.o
#   BLOOD_RUST_RUNTIME — path to libblood_runtime.a
#
# Exit codes:
#   0 — all matched functions produce equivalent IR
#   1 — divergences found
#   2 — compilation failure (one or both compilers failed)
#   3 — usage error

set -euo pipefail

# ── Defaults ─────────────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

BLOOD_REF="${BLOOD_REF:-$REPO_ROOT/src/bootstrap/target/release/blood}"
BLOOD_TEST="${BLOOD_TEST:-$REPO_ROOT/blood-std/std/compiler/build/first_gen}"
export BLOOD_RUNTIME="${BLOOD_RUNTIME:-$REPO_ROOT/runtime/runtime.o}"
export BLOOD_RUST_RUNTIME="${BLOOD_RUST_RUNTIME:-$REPO_ROOT/src/bootstrap/target/release/libblood_runtime.a}"

VERBOSE=0
SUMMARY_ONLY=0
FIRST_DIVERGENCE=0
BATCH_MODE=0
MODE="behavioral"  # "behavioral" (default) or "ir"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
RESET='\033[0m'

# ── Argument parsing ─────────────────────────────────────────────────────────

usage() {
    echo "Usage: $0 <file.blood|directory> [options]"
    echo ""
    echo "Modes:"
    echo "  --behavioral       (default) Compile+run with both compilers, compare output"
    echo "  --ir               Compare LLVM IR function-by-function"
    echo ""
    echo "Options:"
    echo "  --verbose          Show full diffs for divergent items"
    echo "  --summary-only     Only print counts, no details"
    echo "  --first-divergence Stop at first divergent function (--ir mode)"
    echo "  --help             Show this help"
    echo ""
    echo "Environment:"
    echo "  BLOOD_REF          Reference compiler  (default: ~/blood/src/bootstrap/target/release/blood)"
    echo "  BLOOD_TEST         Test compiler        (default: ~/blood/blood-std/std/compiler/build/first_gen)"
    echo "  BLOOD_RUNTIME      C runtime object     (default: ~/blood/runtime.o)"
    echo "  BLOOD_RUST_RUNTIME Rust runtime archive (default: ~/blood/libblood_runtime.a)"
    exit 3
}

TARGET=""
for arg in "$@"; do
    case "$arg" in
        --behavioral)       MODE="behavioral" ;;
        --ir)               MODE="ir" ;;
        --verbose)          VERBOSE=1 ;;
        --summary-only)     SUMMARY_ONLY=1 ;;
        --first-divergence) FIRST_DIVERGENCE=1 ;;
        --help|-h)          usage ;;
        -*)                 echo "Unknown option: $arg"; usage ;;
        *)                  TARGET="$arg" ;;
    esac
done

if [[ -z "$TARGET" ]]; then
    usage
fi

# ── Validation ───────────────────────────────────────────────────────────────

if [[ ! -x "$BLOOD_REF" ]]; then
    echo -e "${RED}Error: Reference compiler not found: $BLOOD_REF${RESET}" >&2
    exit 3
fi
if [[ ! -x "$BLOOD_TEST" ]]; then
    echo -e "${RED}Error: Test compiler not found: $BLOOD_TEST${RESET}" >&2
    exit 3
fi

# ── Function extraction ──────────────────────────────────────────────────────

# extract_functions <ir_file> <output_dir>
#
# Splits an LLVM IR file into per-function files named by the function's
# canonical name. Each file contains the full `define ... { ... }` block.
# Also emits a manifest file listing all function names.
extract_functions() {
    local ir_file="$1"
    local out_dir="$2"

    mkdir -p "$out_dir"

    awk -v out_dir="$out_dir" '
    /^define / {
        # Extract function name: find @name( pattern
        fname = $0
        sub(/.*@/, "", fname)       # remove everything before @
        sub(/\(.*/, "", fname)       # remove ( and everything after
        gsub(/"/, "", fname)         # remove quotes if present
        in_func = 1
        func_body = $0 "\n"
        next
    }
    in_func {
        func_body = func_body $0 "\n"
        if (/^}/) {
            # Sanitize fname for filesystem (replace $ with _)
            safename = fname
            gsub(/\$/, "_", safename)
            # Write function body to file
            outfile = out_dir "/" safename ".ll"
            printf "%s", func_body > outfile
            close(outfile)
            # Write original name to manifest
            print fname >> (out_dir "/MANIFEST")
            in_func = 0
            func_body = ""
        }
    }
    ' "$ir_file"

    # Sort manifest for consistent ordering
    if [[ -f "$out_dir/MANIFEST" ]]; then
        sort "$out_dir/MANIFEST" -o "$out_dir/MANIFEST"
    else
        touch "$out_dir/MANIFEST"
    fi
}

# ── Name canonicalization ────────────────────────────────────────────────────

# canonicalize_name <llvm_name>
#
# Maps compiler-specific function names to a canonical form so that
# functions from different compilers can be matched by semantic name.
#
# Naming conventions handled:
#   blood-rust (bootstrap):     "blood$add$i32$i32"     → "add"
#   self-hosted (any gen):      "def18_add"             → "add"
#   entry point:                "blood_main"            → "blood_main"
#   self-hosted type stubs:     "def1_String"           → "String" (may not match)
#   self-hosted internals:      "option_none_ctor"      → "option_none_ctor"
canonicalize_name() {
    local name="$1"

    # blood_main is the same in both
    if [[ "$name" == "blood_main" ]]; then
        echo "blood_main"
        return
    fi

    # blood-rust style: blood$name$type1$type2...
    # Examples:
    #   blood$add$i32$i32       → add
    #   blood$fibonacci$i32     → fibonacci
    #   blood$Lexer$next_token  → Lexer$next_token (method)
    #   blood$Vec$push$T        → Vec$push (method)
    if [[ "$name" == blood\$* ]]; then
        local stripped="${name#blood\$}"
        # Extract function name: first segment that isn't a type
        # Types are: i8, i16, i32, i64, i128, u8, ..., f32, f64, bool, ptr, usize, T, E
        # Strategy: strip trailing $type segments
        local result="$stripped"
        while [[ "$result" =~ ^(.+)\$(i8|i16|i32|i64|i128|u8|u16|u32|u64|u128|f32|f64|bool|ptr|usize|isize|T|E|K|V|String|str)$ ]]; do
            result="${BASH_REMATCH[1]}"
        done
        echo "$result"
        return
    fi

    # Self-hosted compiler style (all generations): def{N}_{name}
    # The Blood codegen emits LLVM names as def{DefId}_{name}. Since all
    # self-hosted generations share the same codegen source, this pattern
    # applies to first_gen, second_gen, third_gen, etc.
    if [[ "$name" =~ ^def[0-9]+_(.*) ]]; then
        echo "${BASH_REMATCH[1]}"
        return
    fi

    # Everything else: pass through as-is
    echo "$name"
}

# ── Normalize IR for comparison ──────────────────────────────────────────────

# normalize_ir <ir_file>
#
# Normalizes LLVM IR to make structural comparison meaningful:
# - Strips the function signature line (names differ between compilers)
# - Renumbers %variables sequentially
# - Renumbers bb/label names sequentially
# - Removes alignment hints (may differ)
# - Removes metadata references
normalize_ir() {
    local ir_file="$1"

    perl -e '
        my %var_map;
        my %label_map;
        my $vc = 0;
        my $lc = 0;
        my $first = 1;

        while (<>) {
            # Skip the define line (function signatures differ)
            if ($first) { $first = 0; next; }

            # Strip comments, alignment, metadata
            s/;.*$//;
            s/, align \d+//g;
            s/!dbg !\d+//g;
            s/!tbaa !\d+//g;
            s/, !nonnull//g;
            s/\s+$//;

            # Renumber labels at start of line
            if (/^([a-zA-Z_]\w*):/) {
                my $lbl = $1;
                if (!exists $label_map{$lbl}) {
                    $label_map{$lbl} = "L" . $lc++;
                }
                s/^\Q$lbl\E:/$label_map{$lbl}:/;
            }

            # Renumber %variables
            s/%([a-zA-Z_]\w*)/
                my $v = $1;
                if (!exists $var_map{$v}) {
                    $var_map{$v} = "v" . $vc++;
                }
                "%" . $var_map{$v}
            /ge;

            # Replace label references in branches
            for my $lbl (keys %label_map) {
                s/label %\Q$lbl\E\b/label %$label_map{$lbl}/g;
            }

            print "$_\n";
        }
    ' "$ir_file"
}

# ── Behavioral comparison ─────────────────────────────────────────────────────

# behavioral_file <blood_file>
#
# Compiles with both compilers, runs both executables, compares stdout + exit code.
# Returns: 0 = match, 1 = divergence, 2 = compile failure
behavioral_file() {
    local src="$1"
    local basename="$(basename "$src" .blood)"
    local tmpdir
    tmpdir="$(mktemp -d "/tmp/difftest.${basename}.XXXXXX")"
    trap "rm -rf '$tmpdir'" RETURN

    # ── Compile with reference compiler ──
    local ref_compile_ok=1
    if ! "$BLOOD_REF" build "$src" -o "$tmpdir/ref_exe" \
         --quiet --color never 2>"$tmpdir/ref_cerr.txt"; then
        ref_compile_ok=0
    fi

    # ── Compile with test compiler ──
    local test_compile_ok=1
    if ! "$BLOOD_TEST" build "$src" -o "$tmpdir/test_exe" --no-cache \
         2>"$tmpdir/test_cerr.txt" 1>/dev/null; then
        test_compile_ok=0
    fi
    # Self-hosted compiler outputs to a different path — find the executable
    if [[ $test_compile_ok -eq 1 && ! -x "$tmpdir/test_exe" ]]; then
        # Self-hosted compilers create the exe next to the .ll file
        local test_exe_name="${basename}"
        if [[ -x "/tmp/${test_exe_name}" ]]; then
            mv "/tmp/${test_exe_name}" "$tmpdir/test_exe"
        elif [[ -x "$(dirname "$src")/${test_exe_name}" ]]; then
            mv "$(dirname "$src")/${test_exe_name}" "$tmpdir/test_exe"
        else
            test_compile_ok=0
        fi
    fi

    # ── Handle compilation outcomes ──
    if [[ $ref_compile_ok -eq 0 && $test_compile_ok -eq 0 ]]; then
        if [[ $SUMMARY_ONLY -eq 1 ]]; then
            printf "  ${YELLOW}BOTH_FAIL${RESET}  %s\n" "$src"
        else
            echo -e "  ${YELLOW}BOTH_FAIL${RESET}  $basename  (both compilers rejected — consistent)"
        fi
        return 0
    elif [[ $ref_compile_ok -eq 0 ]]; then
        if [[ $SUMMARY_ONLY -eq 1 ]]; then
            printf "  ${RED}REF_FAIL${RESET}   %s\n" "$src"
        else
            echo -e "  ${RED}REF_FAIL${RESET}   $basename  (reference compiler failed)"
        fi
        return 2
    elif [[ $test_compile_ok -eq 0 ]]; then
        if [[ $SUMMARY_ONLY -eq 1 ]]; then
            printf "  ${RED}TEST_FAIL${RESET}  %s\n" "$src"
        else
            echo -e "  ${RED}TEST_FAIL${RESET}  $basename  (test compiler failed, reference succeeded)"
            if [[ $VERBOSE -eq 1 ]]; then
                echo "    $(head -3 "$tmpdir/test_cerr.txt")"
            fi
        fi
        return 2
    fi

    # ── Run both executables (suppress bash signal messages) ──
    local ref_exit=0
    ("$tmpdir/ref_exe" > "$tmpdir/ref_stdout.txt" 2>"$tmpdir/ref_stderr.txt") 2>/dev/null || ref_exit=$?

    local test_exit=0
    ("$tmpdir/test_exe" > "$tmpdir/test_stdout.txt" 2>"$tmpdir/test_stderr.txt") 2>/dev/null || test_exit=$?

    # ── Compare ──
    local stdout_match=1
    if ! diff -q "$tmpdir/ref_stdout.txt" "$tmpdir/test_stdout.txt" >/dev/null 2>&1; then
        stdout_match=0
    fi

    local exit_match=1
    if [[ $ref_exit -ne $test_exit ]]; then
        exit_match=0
    fi

    if [[ $stdout_match -eq 1 && $exit_match -eq 1 ]]; then
        if [[ $SUMMARY_ONLY -eq 1 ]]; then
            printf "  ${GREEN}MATCH${RESET}      %s\n" "$basename"
        else
            echo -e "  ${GREEN}MATCH${RESET}      $basename  (exit=$ref_exit, output identical)"
        fi
        return 0
    else
        if [[ $SUMMARY_ONLY -eq 1 ]]; then
            printf "  ${RED}DIVERGE${RESET}    %s\n" "$basename"
        else
            echo -e "  ${RED}DIVERGE${RESET}    $basename"
            if [[ $exit_match -eq 0 ]]; then
                echo -e "             exit: ref=$ref_exit test=$test_exit"
            fi
            if [[ $stdout_match -eq 0 ]]; then
                echo -e "             output differs:"
                diff --color=always -u \
                    --label "ref" "$tmpdir/ref_stdout.txt" \
                    --label "test" "$tmpdir/test_stdout.txt" \
                    | head -20 | sed 's/^/             /'
            fi
        fi
        return 1
    fi
}

# ── Single file IR diff ─────────────────────────────────────────────────────

# difftest_file <blood_file>
#
# Returns: 0 = match, 1 = divergence, 2 = compile failure
difftest_file() {
    local src="$1"
    local basename="$(basename "$src" .blood)"
    local tmpdir
    tmpdir="$(mktemp -d "/tmp/difftest.${basename}.XXXXXX")"
    trap "rm -rf '$tmpdir'" RETURN

    local ref_ir="$tmpdir/ref.ll"
    local test_ir="$tmpdir/test.ll"
    local ref_funcs="$tmpdir/ref_funcs"
    local test_funcs="$tmpdir/test_funcs"

    # ── Compile with reference compiler ──
    local ref_ok=1
    if ! "$BLOOD_REF" build "$src" --emit llvm-ir-unopt -o "$ref_ir" \
         --quiet --color never 2>"$tmpdir/ref_err.txt"; then
        ref_ok=0
    fi

    # ── Compile with test compiler ──
    local test_ok=1
    if ! "$BLOOD_TEST" build "$src" -o "$test_ir" --no-cache \
         2>"$tmpdir/test_err.txt" 1>/dev/null; then
        test_ok=0
    fi

    # ── Handle compilation failures ──
    if [[ $ref_ok -eq 0 && $test_ok -eq 0 ]]; then
        echo -e "  ${YELLOW}BOTH_FAIL${RESET}  $src"
        return 0  # both fail = consistent behavior
    elif [[ $ref_ok -eq 0 ]]; then
        echo -e "  ${RED}REF_FAIL${RESET}   $src  (reference compiler failed)"
        if [[ $VERBOSE -eq 1 ]]; then
            echo "    stderr: $(head -3 "$tmpdir/ref_err.txt")"
        fi
        return 2
    elif [[ $test_ok -eq 0 ]]; then
        echo -e "  ${RED}TEST_FAIL${RESET}  $src  (test compiler failed, reference succeeded)"
        if [[ $VERBOSE -eq 1 ]]; then
            echo "    stderr: $(head -3 "$tmpdir/test_err.txt")"
        fi
        return 2
    fi

    # ── Extract functions ──
    extract_functions "$ref_ir" "$ref_funcs"
    extract_functions "$test_ir" "$test_funcs"

    # ── Build canonical name maps ──
    # ref_canon[canonical] = llvm_name (original)
    # ref_safe[canonical] = safe_filename ($ replaced with _)
    # test_canon[canonical] = llvm_name
    # test_safe[canonical] = safe_filename
    declare -A ref_canon
    declare -A ref_safe
    declare -A test_canon
    declare -A test_safe

    while IFS= read -r name; do
        local canon
        canon="$(canonicalize_name "$name")"
        ref_canon["$canon"]="$name"
        local safe="$name"
        safe="${safe//\$/_}"
        ref_safe["$canon"]="$safe"
    done < "$ref_funcs/MANIFEST"

    while IFS= read -r name; do
        local canon
        canon="$(canonicalize_name "$name")"
        test_canon["$canon"]="$name"
        local safe="$name"
        safe="${safe//\$/_}"
        test_safe["$canon"]="$safe"
    done < "$test_funcs/MANIFEST"

    # ── Compare ──
    local matched=0
    local identical=0
    local divergent=0
    local ref_only=0
    local test_only=0
    local divergent_names=()

    # Find all canonical names across both
    declare -A all_names
    for k in "${!ref_canon[@]}"; do all_names["$k"]=1; done
    for k in "${!test_canon[@]}"; do all_names["$k"]=1; done

    for canon in $(echo "${!all_names[@]}" | tr ' ' '\n' | sort); do
        local in_ref=${ref_canon[$canon]+1}
        local in_test=${test_canon[$canon]+1}

        if [[ -n "${in_ref:-}" && -z "${in_test:-}" ]]; then
            ref_only=$((ref_only + 1))
            continue
        fi
        if [[ -z "${in_ref:-}" && -n "${in_test:-}" ]]; then
            test_only=$((test_only + 1))
            continue
        fi

        # Both have this function — compare normalized IR
        matched=$((matched + 1))
        local ref_name="${ref_canon[$canon]}"
        local test_name="${test_canon[$canon]}"
        local ref_file="${ref_safe[$canon]}"
        local test_file="${test_safe[$canon]}"

        local ref_norm="$tmpdir/ref_norm.ll"
        local test_norm="$tmpdir/test_norm.ll"

        normalize_ir "$ref_funcs/$ref_file.ll" > "$ref_norm"
        normalize_ir "$test_funcs/$test_file.ll" > "$test_norm"

        if diff -q "$ref_norm" "$test_norm" >/dev/null 2>&1; then
            identical=$((identical + 1))
        else
            divergent=$((divergent + 1))
            divergent_names+=("$canon")

            if [[ $SUMMARY_ONLY -eq 0 ]]; then
                echo -e "  ${RED}DIVERGE${RESET}  ${BOLD}$canon${RESET}"
                echo -e "           ref: ${CYAN}$ref_name${RESET}  test: ${CYAN}$test_name${RESET}"

                if [[ $VERBOSE -eq 1 ]]; then
                    echo "    --- normalized diff ---"
                    diff --color=always -u "$ref_norm" "$test_norm" | head -40 | sed 's/^/    /'
                    echo "    --- end ---"
                fi
            fi

            if [[ $FIRST_DIVERGENCE -eq 1 ]]; then
                echo ""
                echo -e "  Stopped at first divergence. Full diff:"
                echo ""
                diff --color=always -u \
                    --label "ref ($ref_name)" "$ref_funcs/$ref_file.ll" \
                    --label "test ($test_name)" "$test_funcs/$test_file.ll" \
                    | head -80
                echo ""
                return 1
            fi
        fi
    done

    # ── Report ──
    if [[ $SUMMARY_ONLY -eq 1 ]]; then
        local status_icon="${GREEN}MATCH${RESET}"
        if [[ $divergent -gt 0 ]]; then
            status_icon="${RED}DIVG${RESET}"
        fi
        printf "  %b  %-50s  matched:%-3d identical:%-3d divergent:%-3d ref_only:%-3d test_only:%-3d\n" \
            "$status_icon" "$src" "$matched" "$identical" "$divergent" "$ref_only" "$test_only"
    else
        if [[ $divergent -eq 0 ]]; then
            echo -e "  ${GREEN}MATCH${RESET}    $src  ($matched functions matched, $identical identical)"
        else
            echo -e "  ${RED}RESULT${RESET}   $src  $divergent/${matched} functions diverge"
        fi
        if [[ $ref_only -gt 0 || $test_only -gt 0 ]]; then
            echo -e "           ${YELLOW}ref-only: $ref_only  test-only: $test_only${RESET}"
        fi
    fi

    if [[ $divergent -gt 0 ]]; then
        return 1
    fi
    return 0
}

# ── Main ─────────────────────────────────────────────────────────────────────

echo -e "${BOLD}Blood Differential Testing Harness${RESET}  (mode: $MODE)"
echo -e "  ref:  $BLOOD_REF"
echo -e "  test: $BLOOD_TEST"
echo ""

# Select comparison function based on mode
if [[ "$MODE" == "behavioral" ]]; then
    compare_fn="behavioral_file"
else
    compare_fn="difftest_file"
fi

exit_code=0

if [[ -d "$TARGET" ]]; then
    # Batch mode: process all .blood files in directory
    BATCH_MODE=1
    total=0
    pass=0
    diverge=0
    fail=0

    for f in "$TARGET"/*.blood; do
        [[ -f "$f" ]] || continue

        # Skip COMPILE_FAIL tests — they're supposed to fail
        if grep -q "// COMPILE_FAIL:" "$f" 2>/dev/null; then
            continue
        fi

        total=$((total + 1))
        if $compare_fn "$f"; then
            pass=$((pass + 1))
        else
            rc=$?
            if [[ $rc -eq 1 ]]; then
                diverge=$((diverge + 1))
                exit_code=1
            else
                fail=$((fail + 1))
            fi
        fi
    done

    echo ""
    echo -e "${BOLD}Summary:${RESET} $total tests, ${GREEN}$pass match${RESET}, ${RED}$diverge diverge${RESET}, ${YELLOW}$fail compile_fail${RESET}"
else
    # Single file mode
    if ! $compare_fn "$TARGET"; then
        exit_code=$?
    fi
fi

exit $exit_code
