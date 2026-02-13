#!/bin/bash
# debug_test.sh - Debugging infrastructure for self-hosted compiler tests
#
# Usage:
#   ./debug_test.sh ir-diff <test>    Compare IR between blood-rust and first_gen
#   ./debug_test.sh run <test>        Build with first_gen, run, check output
#   ./debug_test.sh ir <test>         Quick IR inspection (first_gen)
#   ./debug_test.sh status            Show binary provenance and environment info
#   ./debug_test.sh sweep [filter]    Run all ground-truth tests, categorize results
set -uo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$DIR"

# Paths (configurable via environment, same defaults as build_selfhost.sh)
BLOOD_RUST="${BLOOD_RUST:-$HOME/blood/compiler-rust/target/release/blood}"
RUNTIME_O="${RUNTIME_O:-$HOME/blood/compiler-rust/runtime/runtime.o}"
RUNTIME_A="${RUNTIME_A:-$HOME/blood/compiler-rust/target/release/libblood_runtime.a}"
GROUND_TRUTH="${GROUND_TRUTH:-$HOME/blood/compiler-rust/tests/ground-truth}"
FIRST_GEN="${FIRST_GEN:-$DIR/first_gen}"

export BLOOD_RUNTIME="${RUNTIME_O}"
export BLOOD_RUST_RUNTIME="${RUNTIME_A}"

# Artifact directory
DEBUG_DIR="$DIR/.debug"
LAST_DIR="$DEBUG_DIR/last"
SWEEP_DIR="$DEBUG_DIR/sweep"

# Colors
ok()    { printf "  \033[1;32m✓\033[0m %s\n" "$1"; }
fail()  { printf "  \033[1;31m✗\033[0m %s\n" "$1"; }
warn()  { printf "  \033[1;33m!\033[0m %s\n" "$1"; }
info()  { printf "  \033[0;36m·\033[0m %s\n" "$1"; }
hdr()   { printf "\n\033[1;34m==> %s\033[0m\n" "$1"; }
die()   { printf "\033[1;31mERROR:\033[0m %s\n" "$1" >&2; exit 1; }
dim()   { printf "  \033[0;37m%s\033[0m\n" "$1"; }

decode_exit() {
    local code="$1"
    if [ "$code" -eq 0 ]; then
        echo "success"
    elif [ "$code" -le 128 ]; then
        echo "exit $code"
    else
        local sig=$((code - 128))
        case "$sig" in
            6)  echo "SIGABRT (abort/assert)" ;;
            8)  echo "SIGFPE (arithmetic error)" ;;
            9)  echo "SIGKILL (killed)" ;;
            11) echo "SIGSEGV (segfault)" ;;
            13) echo "SIGPIPE (broken pipe)" ;;
            15) echo "SIGTERM (terminated)" ;;
            *)  echo "signal $sig (exit $code)" ;;
        esac
    fi
}

# Resolve test path: accepts bare name, name.blood, or full path
resolve_test() {
    local input="$1"

    # Full path
    if [ -f "$input" ]; then
        echo "$input"
        return 0
    fi

    # Strip .blood extension if present
    local bare="${input%.blood}"

    # Search ground-truth directory
    local match="$GROUND_TRUTH/${bare}.blood"
    if [ -f "$match" ]; then
        echo "$match"
        return 0
    fi

    # Try glob match in ground-truth
    local found
    found=$(find "$GROUND_TRUTH" -name "${bare}*.blood" -type f 2>/dev/null | head -1)
    if [ -n "$found" ]; then
        echo "$found"
        return 0
    fi

    # Search local tests directory
    match="$DIR/tests/${bare}.blood"
    if [ -f "$match" ]; then
        echo "$match"
        return 0
    fi

    die "Test not found: $input (searched $GROUND_TRUTH and $DIR/tests)"
}

ensure_first_gen() {
    [ -f "$FIRST_GEN" ] || die "first_gen not found at $FIRST_GEN"
}

ensure_blood_rust() {
    [ -f "$BLOOD_RUST" ] || die "blood-rust not found at $BLOOD_RUST"
}

ensure_debug_dir() {
    mkdir -p "$LAST_DIR"
}

# Normalize LLVM IR for diffing — strips noise that differs between compilers
# but doesn't affect semantics
normalize_ir() {
    sed -E \
        -e '/^;/d' \
        -e '/^source_filename/d' \
        -e '/^target datalayout/d' \
        -e '/^target triple/d' \
        -e '/^!/d' \
        -e 's/;[^"]*$//g' \
        -e 's/, !dbg ![0-9]+//g' \
        -e 's/, !tbaa ![0-9]+//g' \
        -e 's/, !range ![0-9]+//g' \
        -e 's/, !noalias ![0-9]+//g' \
        -e 's/, !alias.scope ![0-9]+//g' \
        -e 's/\bi[0-9]+\*/ptr/g' \
        -e 's/\{ ptr, i64 \}\*/ptr/g' \
        -e 's/, align [0-9]+//g' \
        -e 's/ nuw / /g' \
        -e 's/ nsw / /g' \
        -e 's/ nuw$//g' \
        -e 's/ nsw$//g' \
        -e '/^$/d' \
        | sed -E 's/[[:space:]]+$//'
}

# Canonicalize SSA values and labels within a single function body.
# Renames all %identifier → %v0, %v1, ... and labels → L0, L1, ...
# in order of first appearance. This makes structurally identical code
# from different compilers produce identical output regardless of naming.
canonicalize_ssa() {
    python3 << 'PYEOF'
import re, sys

def canonicalize(text):
    lines = text.rstrip("\n").split("\n")
    if not lines:
        return text

    # Pass 1: identify label definitions (lines starting with identifier:)
    labels = set()
    for line in lines:
        stripped = line.strip()
        m = re.match(r"^([a-zA-Z_][a-zA-Z0-9_.]*)\s*:", stripped)
        if m:
            labels.add(m.group(1))

    # Maps and counters
    var_map = {}
    vc = [0]  # variable counter (list for closure mutation)
    lc = [0]  # label counter

    def get_canonical(name):
        if name in var_map:
            return var_map[name]
        if name in labels:
            canonical = "L" + str(lc[0])
            lc[0] += 1
        else:
            canonical = "v" + str(vc[0])
            vc[0] += 1
        var_map[name] = canonical
        return canonical

    # Pass 2: replace all %identifier and label definitions
    result = []
    for line in lines:
        # Replace label definitions at start of line: "name:" → "L0:"
        # Capture only the name, replace name+colon together
        def replace_label_def(m):
            lbl = m.group(1)
            return get_canonical(lbl) + ":"
        line = re.sub(r"^([a-zA-Z_][a-zA-Z0-9_.]*):", replace_label_def, line)

        # Replace %identifier and %number references
        def replace_var_ref(m):
            name = m.group(1)
            return "%" + get_canonical(name)
        line = re.sub(r"%([a-zA-Z_][a-zA-Z0-9_.]*)", replace_var_ref, line)
        line = re.sub(r"%(\d+)", replace_var_ref, line)

        result.append(line)

    return "\n".join(result)

sys.stdout.write(canonicalize(sys.stdin.read()))
PYEOF
}

# Extract bare function name: strip @defN_ prefix
bare_fn_name() {
    local name="$1"
    # Strip leading @
    name="${name#@}"
    # Strip defN_ prefix (e.g., def123_)
    echo "$name" | sed -E 's/^def[0-9]+_//'
}

# Validate LLVM IR with llvm-as-18
validate_ir() {
    local ll_file="$1"
    local errors
    if ! errors=$(llvm-as-18 "$ll_file" -o /dev/null 2>&1); then
        fail "Invalid LLVM IR"
        printf "%s\n" "$errors" | head -10 | while IFS= read -r line; do
            dim "  $line"
        done
        return 1
    fi
    ok "LLVM IR validates"
    return 0
}

# ============================================================
# Mode: status
# ============================================================
mode_status() {
    hdr "Binary Provenance"

    # first_gen
    if [ -f "$FIRST_GEN" ]; then
        local sz ts md5
        sz=$(wc -c < "$FIRST_GEN")
        ts=$(stat -c '%Y' "$FIRST_GEN" 2>/dev/null)
        md5=$(md5sum "$FIRST_GEN" 2>/dev/null | cut -d' ' -f1)
        ok "first_gen: $(numfmt --to=iec "$sz") | $(date -d "@$ts" '+%Y-%m-%d %H:%M:%S') | $md5"

        # Check for copies with different hashes
        local other_fg="$HOME/blood/first_gen"
        if [ -f "$other_fg" ] && [ "$other_fg" != "$FIRST_GEN" ]; then
            local other_md5
            other_md5=$(md5sum "$other_fg" 2>/dev/null | cut -d' ' -f1)
            if [ "$md5" != "$other_md5" ]; then
                warn "Different first_gen at $other_fg (hash: $other_md5)"
            else
                dim "Copy at $other_fg matches"
            fi
        fi
    else
        fail "first_gen not found at $FIRST_GEN"
    fi

    # blood-rust
    if [ -f "$BLOOD_RUST" ]; then
        local sz ts
        sz=$(wc -c < "$BLOOD_RUST")
        ts=$(stat -c '%Y' "$BLOOD_RUST" 2>/dev/null)
        ok "blood-rust: $(numfmt --to=iec "$sz") | $(date -d "@$ts" '+%Y-%m-%d %H:%M:%S')"
    else
        fail "blood-rust not found at $BLOOD_RUST"
    fi

    # Runtime objects
    hdr "Runtime"
    if [ -f "$RUNTIME_O" ]; then
        local rt_ts
        rt_ts=$(stat -c '%Y' "$RUNTIME_O" 2>/dev/null)
        ok "runtime.o: $(wc -c < "$RUNTIME_O") bytes | $(date -d "@$rt_ts" '+%Y-%m-%d %H:%M:%S')"
    else
        fail "runtime.o not found at $RUNTIME_O"
    fi
    if [ -f "$RUNTIME_A" ]; then
        local rta_ts
        rta_ts=$(stat -c '%Y' "$RUNTIME_A" 2>/dev/null)
        ok "libblood_runtime.a: $(wc -c < "$RUNTIME_A") bytes | $(date -d "@$rta_ts" '+%Y-%m-%d %H:%M:%S')"
    else
        fail "libblood_runtime.a not found at $RUNTIME_A"
    fi

    # Staleness check
    hdr "Staleness"
    if [ -f "$FIRST_GEN" ] && [ -f "$BLOOD_RUST" ]; then
        local fg_ts br_ts
        fg_ts=$(stat -c '%Y' "$FIRST_GEN")
        br_ts=$(stat -c '%Y' "$BLOOD_RUST")
        if [ "$fg_ts" -lt "$br_ts" ]; then
            warn "first_gen is OLDER than blood-rust (may need rebuild)"
        else
            ok "first_gen is newer than blood-rust"
        fi
    fi

    # Check if blood source is newer than first_gen
    if [ -f "$FIRST_GEN" ]; then
        local fg_ts newer_count
        fg_ts=$(stat -c '%Y' "$FIRST_GEN")
        newer_count=$(find "$DIR" -name "*.blood" -newer "$FIRST_GEN" 2>/dev/null | wc -l)
        if [ "$newer_count" -gt 0 ]; then
            warn "$newer_count .blood files are newer than first_gen"
        else
            ok "No .blood files newer than first_gen"
        fi
    fi

    # Cache directories
    hdr "Caches"
    if [ -d "$DIR/.blood-cache" ]; then
        local cache_size cache_count
        cache_size=$(du -sh "$DIR/.blood-cache" 2>/dev/null | cut -f1)
        cache_count=$(find "$DIR/.blood-cache" -name "*.ll" 2>/dev/null | wc -l)
        info ".blood-cache/: $cache_size ($cache_count .ll files)"
    else
        dim "No .blood-cache/ directory"
    fi
    if [ -d "$DIR/.blood_objs" ]; then
        local objs_size objs_count
        objs_size=$(du -sh "$DIR/.blood_objs" 2>/dev/null | cut -f1)
        objs_count=$(find "$DIR/.blood_objs" -type f 2>/dev/null | wc -l)
        info ".blood_objs/: $objs_size ($objs_count files)"
    else
        dim "No .blood_objs/ directory"
    fi

    # Ground-truth test count
    hdr "Ground-truth tests"
    if [ -d "$GROUND_TRUTH" ]; then
        local total
        total=$(find "$GROUND_TRUTH" -name "*.blood" | wc -l)
        info "$total tests in $GROUND_TRUTH"
        for tier in t00 t01 t02 t03 t04 t05 t06; do
            local count
            count=$(find "$GROUND_TRUTH" -name "${tier}_*.blood" | wc -l)
            [ "$count" -gt 0 ] && dim "  $tier: $count tests"
        done
    else
        fail "Ground-truth directory not found"
    fi

    # Last debug run
    hdr "Debug artifacts"
    if [ -d "$LAST_DIR" ]; then
        local artifact_count
        artifact_count=$(find "$LAST_DIR" -type f 2>/dev/null | wc -l)
        info ".debug/last/: $artifact_count files"
        for f in "$LAST_DIR"/*; do
            [ -f "$f" ] && dim "  $(basename "$f"): $(wc -c < "$f") bytes"
        done
    else
        dim "No .debug/last/ directory"
    fi
    if [ -d "$SWEEP_DIR" ]; then
        local sweep_count
        sweep_count=$(find "$SWEEP_DIR" -name "results_*.txt" 2>/dev/null | wc -l)
        info ".debug/sweep/: $sweep_count result files"
    fi
}

# ============================================================
# Mode: ir
# ============================================================
mode_ir() {
    local test_path
    test_path=$(resolve_test "$1")
    local test_name
    test_name=$(basename "$test_path" .blood)

    ensure_first_gen
    ensure_debug_dir

    hdr "IR inspection: $test_name"

    local tmpdir
    tmpdir=$(mktemp -d)
    trap "rm -rf '$tmpdir'" EXIT

    # Compile with first_gen
    info "Compiling with first_gen..."
    local compile_out compile_exit=0
    compile_out=$("$FIRST_GEN" build "$test_path" -o "$tmpdir/out.ll" 2>&1) || compile_exit=$?

    if [ "$compile_exit" -ne 0 ]; then
        fail "first_gen compile failed ($(decode_exit "$compile_exit"))"
        if [ -n "$compile_out" ]; then
            printf "%s\n" "$compile_out" | head -20
        fi
        return 1
    fi

    # Find the .ll file
    local ll_file="$tmpdir/out.ll"
    if [ ! -f "$ll_file" ]; then
        ll_file=$(find "$tmpdir" -name "*.ll" -type f 2>/dev/null | head -1)
        if [ -z "$ll_file" ]; then
            local cwd_ll="${test_name}.ll"
            if [ -f "$cwd_ll" ]; then
                ll_file="$cwd_ll"
            else
                fail "No .ll file produced"
                return 1
            fi
        fi
    fi

    # Copy to debug dir
    cp "$ll_file" "$LAST_DIR/out.ll"

    local lines defs decls
    lines=$(wc -l < "$ll_file")
    defs=$(grep -c '^define ' "$ll_file" || true)
    decls=$(grep -c '^declare ' "$ll_file" || true)

    ok "IR: $lines lines, $defs definitions, $decls declarations"
    info "Saved to .debug/last/out.ll"

    # Validate IR
    validate_ir "$ll_file"

    # List function names
    printf "\n"
    hdr "Defined functions ($defs)"
    grep '^define ' "$ll_file" | sed -E 's/^define [^@]*@([^ (]+).*/  \1/' | sort
}

# ============================================================
# Mode: run
# ============================================================
mode_run() {
    local test_path
    test_path=$(resolve_test "$1")
    local test_name
    test_name=$(basename "$test_path" .blood)

    ensure_first_gen
    ensure_debug_dir

    hdr "Run: $test_name"

    local tmpdir
    tmpdir=$(mktemp -d)
    trap "rm -rf '$tmpdir'" EXIT

    # Compile with first_gen
    info "Compiling with first_gen..."
    local compile_out compile_exit=0
    compile_out=$("$FIRST_GEN" build "$test_path" -o "$tmpdir/out.ll" 2>&1) || compile_exit=$?

    if [ "$compile_exit" -ne 0 ]; then
        fail "Compile failed ($(decode_exit "$compile_exit"))"
        if [ -n "$compile_out" ]; then
            printf "%s\n" "$compile_out" | head -20
        fi
        return 1
    fi

    # Find the ll file and binary
    local ll_file="$tmpdir/out.ll"
    local bin_file="$tmpdir/out"

    if [ ! -f "$ll_file" ]; then
        ll_file=$(find "$tmpdir" -name "*.ll" -type f 2>/dev/null | head -1)
        [ -z "$ll_file" ] && ll_file="${test_name}.ll"
    fi
    if [ ! -f "$bin_file" ]; then
        bin_file=$(find "$tmpdir" -type f -executable 2>/dev/null | head -1)
        [ -z "$bin_file" ] && bin_file="${test_name}"
    fi

    # Save artifacts
    [ -f "$ll_file" ] && cp "$ll_file" "$LAST_DIR/out.ll"
    [ -f "$bin_file" ] && cp "$bin_file" "$LAST_DIR/out"

    if [ ! -f "$bin_file" ] || [ ! -x "$bin_file" ]; then
        fail "No executable produced"
        info "Check .debug/last/out.ll for IR"
        return 1
    fi

    ok "Compiled successfully"

    # Validate IR
    if [ -f "$ll_file" ]; then
        validate_ir "$ll_file"
    fi

    # Run — capture stdout and stderr separately
    info "Running..."
    local actual exit_code=0
    actual=$("$bin_file" 2>"$LAST_DIR/stderr.txt") || exit_code=$?

    # Save actual output
    printf "%s\n" "$actual" > "$LAST_DIR/stdout.txt"

    # Show stderr if non-empty
    if [ -s "$LAST_DIR/stderr.txt" ]; then
        warn "stderr output captured:"
        head -5 "$LAST_DIR/stderr.txt" | while IFS= read -r line; do
            dim "  $line"
        done
        local stderr_lines
        stderr_lines=$(wc -l < "$LAST_DIR/stderr.txt")
        if [ "$stderr_lines" -gt 5 ]; then
            dim "  ... ($stderr_lines lines total, see .debug/last/stderr.txt)"
        fi
    fi

    # Parse expected output
    local expected=""
    expected=$(grep '^// EXPECT:' "$test_path" | sed 's|^// EXPECT: *||' || true)

    local expect_exit=""
    expect_exit=$(grep '^// EXPECT_EXIT:' "$test_path" | head -1 | sed 's|^// EXPECT_EXIT: *||' || true)
    [ -z "$expect_exit" ] && expect_exit="0"

    # Report exit code
    if [ "$exit_code" -ne 0 ]; then
        if [ "$exit_code" -gt 128 ]; then
            fail "CRASHED: $(decode_exit "$exit_code")"
        else
            if [ "$expect_exit" = "nonzero" ] || [ "$exit_code" = "$expect_exit" ]; then
                ok "Exit code: $exit_code (expected)"
            else
                fail "Exit code: $exit_code (expected $expect_exit)"
            fi
        fi
    else
        if [ "$expect_exit" = "0" ]; then
            ok "Exit code: 0"
        elif [ "$expect_exit" = "nonzero" ]; then
            fail "Exit code: 0 (expected nonzero)"
        else
            fail "Exit code: 0 (expected $expect_exit)"
        fi
    fi

    # Compare output line-by-line
    if [ -n "$expected" ]; then
        printf "\n"
        hdr "Output comparison"
        local exp_lines act_lines
        exp_lines=$(printf "%s\n" "$expected" | wc -l)
        if [ -n "$actual" ]; then
            act_lines=$(printf "%s\n" "$actual" | wc -l)
        else
            act_lines=0
        fi

        local i=1 pass=0 total=0
        while IFS= read -r exp_line; do
            total=$((total + 1))
            local act_line
            act_line=$(printf "%s\n" "$actual" | sed -n "${i}p")
            if [ "$act_line" = "$exp_line" ]; then
                ok "line $i: $exp_line"
                pass=$((pass + 1))
            else
                fail "line $i: expected '$exp_line' got '$act_line'"
            fi
            i=$((i + 1))
        done <<< "$expected"

        # Extra output lines
        if [ "$act_lines" -gt "$exp_lines" ]; then
            local j=$((exp_lines + 1))
            while [ "$j" -le "$act_lines" ]; do
                local extra_line
                extra_line=$(printf "%s\n" "$actual" | sed -n "${j}p")
                warn "line $j: unexpected extra output: '$extra_line'"
                j=$((j + 1))
            done
        fi

        # Check if exit code matches expectation
        local exit_ok=0
        if [ "$expect_exit" = "nonzero" ]; then
            [ "$exit_code" -ne 0 ] && exit_ok=1
        elif [ "$exit_code" = "$expect_exit" ]; then
            exit_ok=1
        fi

        if [ "$pass" -eq "$total" ] && [ "$act_lines" -eq "$exp_lines" ] && [ "$exit_ok" -eq 1 ]; then
            printf "\n"
            ok "PASS ($pass/$total lines match)"
        else
            printf "\n"
            fail "FAIL ($pass/$total lines match, exit=$exit_code expected=$expect_exit)"
        fi
    else
        # No expected output — just report what we got
        if [ -n "$actual" ]; then
            printf "\n"
            hdr "Output (no EXPECT lines in test)"
            printf "%s\n" "$actual"
        fi
    fi

    # Artifact paths
    printf "\n"
    hdr "Artifacts"
    [ -f "$LAST_DIR/out.ll" ]     && info "IR:     .debug/last/out.ll"
    [ -f "$LAST_DIR/out" ]        && info "Binary: .debug/last/out"
    [ -f "$LAST_DIR/stdout.txt" ] && info "Stdout: .debug/last/stdout.txt"
    [ -s "$LAST_DIR/stderr.txt" ] && info "Stderr: .debug/last/stderr.txt"
    return 0
}

# ============================================================
# Mode: ir-diff
# ============================================================
mode_ir_diff() {
    local test_path
    test_path=$(resolve_test "$1")
    local test_name
    test_name=$(basename "$test_path" .blood)

    ensure_first_gen
    ensure_blood_rust
    ensure_debug_dir

    hdr "IR diff: $test_name"

    local tmpdir
    tmpdir=$(mktemp -d)
    trap "rm -rf '$tmpdir'" EXIT

    # Compile with blood-rust
    info "Compiling with blood-rust (--emit llvm-ir-unopt)..."
    local ref_exit=0
    $BLOOD_RUST build --emit llvm-ir-unopt -o "$tmpdir/ref.ll" "$test_path" 2>/dev/null || ref_exit=$?
    if [ "$ref_exit" -ne 0 ] || [ ! -f "$tmpdir/ref.ll" ]; then
        local cwd_ll="${test_name}.ll"
        if [ -f "$cwd_ll" ]; then
            mv "$cwd_ll" "$tmpdir/ref.ll"
        else
            fail "blood-rust compile failed (exit $ref_exit)"
            return 1
        fi
    fi
    ok "blood-rust: $(wc -l < "$tmpdir/ref.ll") lines"

    # Compile with first_gen
    info "Compiling with first_gen..."
    local self_exit=0
    "$FIRST_GEN" build "$test_path" -o "$tmpdir/self.ll" >/dev/null 2>&1 || self_exit=$?
    if [ "$self_exit" -ne 0 ] || [ ! -f "$tmpdir/self.ll" ]; then
        local cwd_ll="${test_name}.ll"
        if [ -f "$cwd_ll" ]; then
            mv "$cwd_ll" "$tmpdir/self.ll"
        else
            fail "first_gen compile failed ($(decode_exit "$self_exit"))"
            cp "$tmpdir/ref.ll" "$LAST_DIR/ref.ll" 2>/dev/null
            info "Reference IR saved to .debug/last/ref.ll"
            return 1
        fi
    fi
    ok "first_gen:  $(wc -l < "$tmpdir/self.ll") lines"

    # Validate both
    local ref_valid=0 self_valid=0
    if llvm-as-18 "$tmpdir/ref.ll" -o /dev/null 2>/dev/null; then ref_valid=1; fi
    if llvm-as-18 "$tmpdir/self.ll" -o /dev/null 2>/dev/null; then self_valid=1; fi
    if [ "$ref_valid" -eq 1 ] && [ "$self_valid" -eq 1 ]; then
        ok "Both IR files validate"
    else
        [ "$ref_valid" -eq 0 ] && warn "blood-rust IR is invalid (unexpected)"
        [ "$self_valid" -eq 0 ] && fail "first_gen IR is invalid"
    fi

    # Normalize both
    normalize_ir < "$tmpdir/ref.ll"  > "$tmpdir/ref_norm.ll"
    normalize_ir < "$tmpdir/self.ll" > "$tmpdir/self_norm.ll"

    # Save to debug dir
    cp "$tmpdir/ref.ll"  "$LAST_DIR/ref.ll"
    cp "$tmpdir/self.ll" "$LAST_DIR/self.ll"

    # Extract per-function blocks
    mkdir -p "$tmpdir/ref_fns" "$tmpdir/self_fns" "$tmpdir/ref_canon" "$tmpdir/self_canon"

    extract_functions() {
        local infile="$1" outdir="$2"
        local current_fn="" current_file=""
        local in_fn=0

        while IFS= read -r line; do
            if [[ "$line" =~ ^define[[:space:]] ]]; then
                current_fn=$(echo "$line" | sed -E 's/^define [^@]*@([^ (]+).*/\1/')
                local bare
                bare=$(bare_fn_name "$current_fn")
                current_file="$outdir/$bare.ll"
                echo "$line" > "$current_file"
                in_fn=1
            elif [ "$in_fn" -eq 1 ]; then
                echo "$line" >> "$current_file"
                if [ "$line" = "}" ]; then
                    in_fn=0
                fi
            fi
        done < "$infile"
    }

    extract_functions "$tmpdir/ref_norm.ll"  "$tmpdir/ref_fns"
    extract_functions "$tmpdir/self_norm.ll" "$tmpdir/self_fns"

    # Canonicalize each extracted function
    for f in "$tmpdir/ref_fns/"*.ll; do
        [ -f "$f" ] || continue
        canonicalize_ssa < "$f" > "$tmpdir/ref_canon/$(basename "$f")"
    done
    for f in "$tmpdir/self_fns/"*.ll; do
        [ -f "$f" ] || continue
        canonicalize_ssa < "$f" > "$tmpdir/self_canon/$(basename "$f")"
    done

    # Collect all bare function names
    local all_fns
    all_fns=$( (ls "$tmpdir/ref_canon/"*.ll 2>/dev/null | xargs -I{} basename {} .ll; \
                ls "$tmpdir/self_canon/"*.ll 2>/dev/null | xargs -I{} basename {} .ll) | sort -u )

    # Categorize functions
    local matched_fns="" differing_fns="" ref_only_fns="" self_only_fns=""
    local ref_only=0 self_only=0 differ=0 match=0 total=0

    local diff_output=""

    while IFS= read -r fn_name; do
        [ -z "$fn_name" ] && continue
        total=$((total + 1))

        local ref_f="$tmpdir/ref_canon/${fn_name}.ll"
        local self_f="$tmpdir/self_canon/${fn_name}.ll"

        if [ ! -f "$ref_f" ]; then
            self_only=$((self_only + 1))
            self_only_fns+="$fn_name "
        elif [ ! -f "$self_f" ]; then
            ref_only=$((ref_only + 1))
            ref_only_fns+="$fn_name "
        else
            local fn_diff
            fn_diff=$(diff -u "$ref_f" "$self_f" 2>/dev/null || true)
            if [ -z "$fn_diff" ]; then
                match=$((match + 1))
                matched_fns+="$fn_name "
            else
                differ=$((differ + 1))
                differing_fns+="$fn_name "
                diff_output+="=== $fn_name ===
$fn_diff

"
            fi
        fi
    done <<< "$all_fns"

    # Report: important stuff first
    printf "\n"

    # Differing functions — most important, show first
    if [ "$differ" -gt 0 ]; then
        hdr "Differing functions ($differ)"
        for fn in $differing_fns; do
            fail "$fn"
        done
    fi

    # Matching functions
    if [ "$match" -gt 0 ]; then
        hdr "Matching functions ($match)"
        for fn in $matched_fns; do
            ok "$fn"
        done
    fi

    # Ref-only functions (potentially missing from first_gen)
    if [ "$ref_only" -gt 0 ]; then
        hdr "blood-rust only ($ref_only)"
        for fn in $ref_only_fns; do
            warn "$fn"
        done
    fi

    # Self-only functions (typically builtins) — collapsed
    if [ "$self_only" -gt 0 ]; then
        hdr "first_gen only ($self_only) — typically builtins"
        local self_list=""
        for fn in $self_only_fns; do
            if [ -n "$self_list" ]; then
                self_list+=", $fn"
            else
                self_list="$fn"
            fi
        done
        dim "  $self_list"
    fi

    # Summary line
    printf "\n"
    hdr "Summary"
    info "Total: $total | Match: $match | Differ: $differ | Ref-only: $ref_only | Self-only: $self_only"

    # Save diff
    if [ -n "$diff_output" ]; then
        printf "%s" "$diff_output" > "$LAST_DIR/diff.txt"
        info "Full diff: .debug/last/diff.txt ($(wc -l < "$LAST_DIR/diff.txt") lines)"

        # Show inline preview of first differing function
        printf "\n"
        local first_diff_fn
        first_diff_fn=$(echo "$differing_fns" | awk '{print $1}')
        hdr "Diff preview: $first_diff_fn"
        diff -u "$tmpdir/ref_canon/${first_diff_fn}.ll" "$tmpdir/self_canon/${first_diff_fn}.ll" 2>/dev/null \
            | head -40 \
            | while IFS= read -r line; do
                case "$line" in
                    ---*) printf "  \033[1;31m%s\033[0m\n" "$line" ;;
                    +++*) printf "  \033[1;32m%s\033[0m\n" "$line" ;;
                    @@*)  printf "  \033[1;36m%s\033[0m\n" "$line" ;;
                    -*)   printf "  \033[0;31m%s\033[0m\n" "$line" ;;
                    +*)   printf "  \033[0;32m%s\033[0m\n" "$line" ;;
                    *)    printf "  %s\n" "$line" ;;
                esac
            done
        local diff_lines
        diff_lines=$(diff -u "$tmpdir/ref_canon/${first_diff_fn}.ll" "$tmpdir/self_canon/${first_diff_fn}.ll" 2>/dev/null | wc -l)
        if [ "$diff_lines" -gt 40 ]; then
            dim "  ... ($diff_lines lines total)"
        fi
    else
        echo "No differences." > "$LAST_DIR/diff.txt"
        ok "IR is identical after canonicalization!"
    fi
}

# ============================================================
# Mode: sweep
# ============================================================
mode_sweep() {
    local filter="${1:-}"

    ensure_first_gen

    hdr "Sweep: ground-truth tests${filter:+ (filter: $filter)}"

    [ -d "$GROUND_TRUTH" ] || die "Ground-truth directory not found at $GROUND_TRUTH"

    mkdir -p "$SWEEP_DIR"

    local pass=0 compile_fail=0 compiler_crash=0 output_mismatch=0
    local crash_segv=0 crash_sigfpe=0 crash_other=0
    local expected_fail=0 diagnostic_miss=0
    local total=0

    local results_file="$SWEEP_DIR/results_$(date +%Y%m%d_%H%M%S).txt"
    local start_time
    start_time=$(date +%s)

    # Collect test files
    local test_files=()
    if [ -n "$filter" ]; then
        while IFS= read -r -d '' f; do
            test_files+=("$f")
        done < <(find "$GROUND_TRUTH" -name "${filter}*.blood" -type f -print0 | sort -z)
    else
        while IFS= read -r -d '' f; do
            test_files+=("$f")
        done < <(find "$GROUND_TRUTH" -name "*.blood" -type f -print0 | sort -z)
    fi

    local test_count=${#test_files[@]}
    [ "$test_count" -eq 0 ] && die "No tests match filter '${filter:-*}'"

    info "$test_count tests to run"
    printf "\n"

    {
        echo "# debug_test.sh sweep results"
        echo "# Date: $(date)"
        echo "# Filter: ${filter:-<all>}"
        echo "# first_gen: $FIRST_GEN"
        echo "# Tests: $test_count"
        echo ""
    } > "$results_file"

    for src in "${test_files[@]}"; do
        local name
        name=$(basename "$src" .blood)
        total=$((total + 1))

        # Progress counter
        printf "\r  [%d/%d] %-40s" "$total" "$test_count" "$name" >&2

        local is_compile_fail=0
        if head -1 "$src" | grep -q '^// COMPILE_FAIL:'; then
            is_compile_fail=1
        fi

        # Skip XFAIL
        if head -1 "$src" | grep -q '^// XFAIL:'; then
            echo "SKIP $name" >> "$results_file"
            continue
        fi

        # Compile
        local tmpdir
        tmpdir=$(mktemp -d)
        local compile_out compile_exit=0
        compile_out=$("$FIRST_GEN" build "$src" -o "$tmpdir/out.ll" 2>&1) || compile_exit=$?

        if [ "$compile_exit" -ne 0 ] || [ ! -f "$tmpdir/out" ]; then
            if [ "$is_compile_fail" -eq 1 ]; then
                expected_fail=$((expected_fail + 1))
                echo "EXPECTED_FAIL $name" >> "$results_file"
            elif [ "$compile_exit" -gt 128 ]; then
                compiler_crash=$((compiler_crash + 1))
                {
                    echo "COMPILER_CRASH $name ($(decode_exit "$compile_exit"))"
                    printf "%s\n" "$compile_out" | head -3 | sed 's/^/  # /'
                } >> "$results_file"
            else
                compile_fail=$((compile_fail + 1))
                {
                    echo "COMPILE_FAIL $name ($(decode_exit "$compile_exit"))"
                    printf "%s\n" "$compile_out" | head -3 | sed 's/^/  # /'
                } >> "$results_file"
            fi
            rm -rf "$tmpdir"
            continue
        fi

        # If it's a compile-fail test but compilation succeeded
        if [ "$is_compile_fail" -eq 1 ]; then
            diagnostic_miss=$((diagnostic_miss + 1))
            echo "DIAGNOSTIC_MISS $name" >> "$results_file"
            rm -rf "$tmpdir"
            continue
        fi

        # Run
        local actual exit_code=0
        actual=$("$tmpdir/out" 2>/dev/null) || exit_code=$?

        if [ "$exit_code" -gt 128 ]; then
            local sig=$((exit_code - 128))
            case "$sig" in
                11)
                    crash_segv=$((crash_segv + 1))
                    echo "CRASH_SEGV $name" >> "$results_file"
                    ;;
                8)
                    crash_sigfpe=$((crash_sigfpe + 1))
                    echo "CRASH_SIGFPE $name" >> "$results_file"
                    ;;
                *)
                    crash_other=$((crash_other + 1))
                    echo "CRASH_OTHER $name ($(decode_exit "$exit_code"))" >> "$results_file"
                    ;;
            esac
            rm -rf "$tmpdir"
            continue
        fi

        # Check output
        local expected=""
        expected=$(grep '^// EXPECT:' "$src" | sed 's|^// EXPECT: *||' || true)

        local expect_exit=""
        expect_exit=$(grep '^// EXPECT_EXIT:' "$src" | head -1 | sed 's|^// EXPECT_EXIT: *||' || true)
        [ -z "$expect_exit" ] && expect_exit="0"

        local test_pass=1

        # Check exit code
        if [ "$expect_exit" = "nonzero" ]; then
            [ "$exit_code" -eq 0 ] && test_pass=0
        else
            [ "$exit_code" != "$expect_exit" ] && test_pass=0
        fi

        # Check output
        if [ -n "$expected" ] && [ "$actual" != "$expected" ]; then
            test_pass=0
        fi

        if [ "$test_pass" -eq 1 ]; then
            pass=$((pass + 1))
            echo "PASS $name" >> "$results_file"
        else
            output_mismatch=$((output_mismatch + 1))
            echo "OUTPUT_MISMATCH $name (exit=$exit_code, expected_exit=$expect_exit)" >> "$results_file"
        fi

        rm -rf "$tmpdir"
    done

    # Clear progress line
    printf "\r%80s\r" "" >&2

    # Summary
    local elapsed
    elapsed=$(( $(date +%s) - start_time ))

    printf "\n"
    hdr "Sweep Results (${elapsed}s)"

    local fail_total=$((compile_fail + compiler_crash + output_mismatch + crash_segv + crash_sigfpe + crash_other + diagnostic_miss))

    [ "$pass" -gt 0 ]             && ok   "PASS:             $pass"
    [ "$expected_fail" -gt 0 ]    && ok   "EXPECTED_FAIL:    $expected_fail"
    [ "$compile_fail" -gt 0 ]     && fail "COMPILE_FAIL:     $compile_fail"
    [ "$compiler_crash" -gt 0 ]   && fail "COMPILER_CRASH:   $compiler_crash"
    [ "$output_mismatch" -gt 0 ]  && fail "OUTPUT_MISMATCH:  $output_mismatch"
    [ "$crash_segv" -gt 0 ]       && fail "CRASH_SEGV:       $crash_segv"
    [ "$crash_sigfpe" -gt 0 ]     && fail "CRASH_SIGFPE:     $crash_sigfpe"
    [ "$crash_other" -gt 0 ]      && fail "CRASH_OTHER:      $crash_other"
    [ "$diagnostic_miss" -gt 0 ]  && warn "DIAGNOSTIC_MISS:  $diagnostic_miss"

    printf "\n"
    info "Total: $total | Pass: $pass | Fail: $fail_total | Results: $results_file"

    # Show failing tests by category
    if [ "$fail_total" -gt 0 ]; then
        printf "\n"
        for cat in COMPILER_CRASH COMPILE_FAIL OUTPUT_MISMATCH CRASH_SEGV CRASH_SIGFPE CRASH_OTHER DIAGNOSTIC_MISS; do
            local cat_lines
            cat_lines=$(grep "^$cat " "$results_file" 2>/dev/null || true)
            if [ -n "$cat_lines" ]; then
                hdr "$cat"
                printf "%s\n" "$cat_lines" | while IFS= read -r line; do
                    dim "  $line"
                done
            fi
        done
    fi
}

# ============================================================
# Dispatch
# ============================================================
usage() {
    echo "Usage: $0 <mode> [args]"
    echo ""
    echo "Modes:"
    echo "  ir-diff <test>    Compare IR between blood-rust and first_gen"
    echo "  run <test>        Build with first_gen, run, check output"
    echo "  ir <test>         Quick IR inspection (first_gen)"
    echo "  status            Show binary provenance and environment info"
    echo "  sweep [filter]    Run all ground-truth tests, categorize results"
    echo ""
    echo "Test names: bare name (t00_arithmetic), .blood extension, or full path"
    echo ""
    echo "Environment variables:"
    echo "  BLOOD_RUST     blood-rust binary (default: ~/blood/compiler-rust/target/release/blood)"
    echo "  FIRST_GEN      first_gen binary  (default: ./first_gen)"
    echo "  GROUND_TRUTH   test directory    (default: ~/blood/compiler-rust/tests/ground-truth)"
    exit 1
}

case "${1:-}" in
    status)
        mode_status
        ;;
    ir)
        [ -z "${2:-}" ] && die "Usage: $0 ir <test>"
        mode_ir "$2"
        ;;
    run)
        [ -z "${2:-}" ] && die "Usage: $0 run <test>"
        mode_run "$2"
        ;;
    ir-diff)
        [ -z "${2:-}" ] && die "Usage: $0 ir-diff <test>"
        mode_ir_diff "$2"
        ;;
    sweep)
        mode_sweep "${2:-}"
        ;;
    -h|--help|help)
        usage
        ;;
    *)
        usage
        ;;
esac
