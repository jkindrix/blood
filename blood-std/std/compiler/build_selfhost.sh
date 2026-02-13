#!/bin/bash
# build_selfhost.sh - Automates the self-hosting pipeline
#
# Usage:
#   ./build_selfhost.sh              # Full pipeline: blood-rust → first_gen → second_gen
#   ./build_selfhost.sh rebuild      # Skip blood-rust, reuse existing first_gen
#   ./build_selfhost.sh test [bin]   # Smoke test a binary (default: second_gen)
#   ./build_selfhost.sh ground-truth # Run ground-truth tests through first_gen
#   ./build_selfhost.sh emit [stage] # Emit intermediate IR (ast|hir|mir|llvm-ir|llvm-ir-unopt)
#   ./build_selfhost.sh verify [ir]  # Run all verification checks on IR
#   ./build_selfhost.sh ir-check [ir]# Run FileCheck tests against compiler output
#   ./build_selfhost.sh asan         # Build second_gen with AddressSanitizer
#   ./build_selfhost.sh bisect       # Binary search for miscompiled function
#   ./build_selfhost.sh timings      # Build first_gen with per-phase timing
#   ./build_selfhost.sh release      # Build first_gen with --release optimizations
#   ./build_selfhost.sh clean        # Remove build artifacts
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$DIR"

# Paths (configurable via environment)
BLOOD_RUST="${BLOOD_RUST:-$HOME/blood/compiler-rust/target/release/blood}"
RUNTIME_O="${RUNTIME_O:-$HOME/blood/compiler-rust/runtime/runtime.o}"
RUNTIME_A="${RUNTIME_A:-$HOME/blood/compiler-rust/target/release/libblood_runtime.a}"
GROUND_TRUTH="${GROUND_TRUTH:-$HOME/blood/compiler-rust/tests/ground-truth}"

# Export for first_gen/second_gen runtime discovery
export BLOOD_RUNTIME="${RUNTIME_O}"
export BLOOD_RUST_RUNTIME="${RUNTIME_A}"

step()  { printf "\n\033[1;34m==> [%s] %s\033[0m\n" "$(date +%H:%M:%S)" "$1"; }
ok()    { printf "  \033[1;32m✓\033[0m %s\n" "$1"; }
fail()  { printf "  \033[1;31m✗\033[0m %s\n" "$1"; }
warn()  { printf "  \033[1;33m!\033[0m %s\n" "$1"; }
die()   { printf "\033[1;31mERROR:\033[0m %s\n" "$1" >&2; exit 1; }

# Wall-clock elapsed time helper: call elapsed_since $SECONDS_VAR
elapsed_since() {
    local start="$1"
    local now
    now=$(date +%s)
    local diff=$((now - start))
    local mins=$((diff / 60))
    local secs=$((diff % 60))
    if [ "$mins" -gt 0 ]; then
        printf "%dm%02ds" "$mins" "$secs"
    else
        printf "%ds" "$secs"
    fi
}

# Decode process exit code into human-readable signal name
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
            8)  echo "SIGFPE (arithmetic error, e.g. division by zero)" ;;
            9)  echo "SIGKILL (killed)" ;;
            11) echo "SIGSEGV (segmentation fault)" ;;
            13) echo "SIGPIPE (broken pipe)" ;;
            15) echo "SIGTERM (terminated)" ;;
            *)  echo "signal $sig (exit $code)" ;;
        esac
    fi
}

# Log file setup — tee output to timestamped log
LOG_DIR="$DIR/.logs"
mkdir -p "$LOG_DIR"
LOG_FILE="$LOG_DIR/build_$(date +%Y%m%d_%H%M%S).log"

# Start logging (append to log file, also show on terminal)
exec > >(tee -a "$LOG_FILE") 2>&1
printf "=== Build started: %s ===\n" "$(date '+%Y-%m-%d %H:%M:%S')"
printf "=== Log: %s ===\n" "$LOG_FILE"

# Build first_gen from blood-rust
build_first_gen() {
    local flags="${1:-}"
    [ -f "$BLOOD_RUST" ] || die "blood-rust not found at $BLOOD_RUST"

    step "Clearing all build caches"
    rm -rf "${DIR}"/*.blood_objs "${DIR}"/tests/*.blood_objs
    rm -rf "${HOME}"/.blood*/cache/
    ok "Caches cleared"

    step "Building first_gen with blood-rust"
    local start_ts
    start_ts=$(date +%s)
    if $BLOOD_RUST build main.blood --no-cache $flags; then
        mv main first_gen
        ok "first_gen created ($(wc -c < first_gen) bytes) in $(elapsed_since "$start_ts")"
    else
        local rc=$?
        fail "blood-rust build failed: $(decode_exit $rc)"
        return 1
    fi
}

# Self-compile: first_gen → second_gen
build_second_gen() {
    [ -f first_gen ] || die "first_gen not found. Run './build_selfhost.sh' first."

    step "Self-compiling (first_gen → second_gen)"
    local start_ts
    start_ts=$(date +%s)
    local rc=0
    ./first_gen build main.blood --timings -o second_gen.ll || rc=$?
    local wall_time
    wall_time=$(elapsed_since "$start_ts")

    if [ "$rc" -ne 0 ]; then
        fail "first_gen failed: $(decode_exit $rc) (wall time: $wall_time)"
        if [ "$rc" -gt 128 ]; then
            printf "  \033[1;31mCrash detected!\033[0m Signal: %s\n" "$(decode_exit $rc)"
            printf "  Check log: %s\n" "$LOG_FILE"
        fi
        return 1
    fi

    # IR sanity checks (quick — catches obvious problems before expensive llc-18)
    if [ -f second_gen.ll ]; then
        local ll_lines ll_size ll_defines ll_declares
        ll_lines=$(wc -l < second_gen.ll)
        ll_size=$(wc -c < second_gen.ll)
        ll_defines=$(grep -c '^define ' second_gen.ll || true)
        ll_declares=$(grep -c '^declare ' second_gen.ll || true)
        printf "  IR: %s lines, %s bytes, %d defines, %d declares\n" \
            "$ll_lines" "$ll_size" "$ll_defines" "$ll_declares"

        # Sanity: should have >1000 function definitions for self-hosting
        if [ "$ll_defines" -lt 100 ]; then
            warn "Suspiciously few function definitions ($ll_defines) — possible codegen issue"
        fi
    fi

    ok "second_gen built ($(wc -c < second_gen) bytes) in $wall_time"
}

# Generate reference IR from blood-rust (used as baseline for comparisons)
# Uses --emit llvm-ir-unopt which emits all declarations (the full runtime ABI surface)
# even though function definitions may be partial due to build caching.
generate_reference_ir() {
    [ -f "$BLOOD_RUST" ] || die "blood-rust not found at $BLOOD_RUST"

    step "Generating reference IR from blood-rust"
    $BLOOD_RUST build --emit llvm-ir-unopt -o reference_ir.ll main.blood 2>/dev/null
    [ -s reference_ir.ll ] || die "blood-rust did not produce reference_ir.ll"
    ok "reference_ir.ll generated ($(wc -l < reference_ir.ll) lines)"
}

# Verify IR correctness and ABI compatibility
# Args: $1 = IR file to verify (default: second_gen.ll)
verify_ir() {
    local ir_file="${1:-second_gen.ll}"
    [ -f "$ir_file" ] || die "$ir_file not found"

    local errors=0

    # Ensure reference IR exists
    if [ ! -f reference_ir.ll ]; then
        generate_reference_ir
    fi

    # Step A: Structural IR verification with opt-18
    step "Verifying IR structure ($ir_file)"
    local verify_output
    if verify_output=$(opt-18 -passes=verify "$ir_file" -disable-output 2>&1); then
        ok "IR structure valid (SSA, types, dominance)"
    else
        fail "IR structure verification failed"
        printf "%s\n" "$verify_output" | head -20
        errors=$((errors + 1))
    fi

    # Step B: Declaration diff
    step "Comparing declarations against reference"
    if bash "$DIR/tests/check_declarations.sh" reference_ir.ll "$ir_file"; then
        ok "Declaration check passed"
    else
        fail "Declaration mismatches detected"
        errors=$((errors + 1))
    fi

    # Step C: FileCheck tests
    step "Running FileCheck tests"
    local fc_pass=0 fc_fail=0 fc_total=0
    for check_src in "$DIR"/tests/check_*.blood; do
        [ -f "$check_src" ] || continue
        local check_name
        check_name="$(basename "$check_src" .blood)"
        fc_total=$((fc_total + 1))

        # Compile the test with first_gen to produce IR
        local check_tmpdir
        check_tmpdir=$(mktemp -d)
        trap "rm -rf '$check_tmpdir'" RETURN 2>/dev/null || true

        if ! ./first_gen build "$check_src" -o "$check_tmpdir/check_out.ll" >/dev/null 2>&1; then
            fail "$check_name (compile failed)"
            fc_fail=$((fc_fail + 1))
            rm -rf "$check_tmpdir"
            continue
        fi

        # Run FileCheck against the produced IR
        if FileCheck-18 --input-file="$check_tmpdir/check_out.ll" "$check_src" 2>/dev/null; then
            ok "$check_name"
            fc_pass=$((fc_pass + 1))
        else
            fail "$check_name"
            fc_fail=$((fc_fail + 1))
        fi

        rm -rf "$check_tmpdir"
    done

    if [ "$fc_total" -eq 0 ]; then
        warn "No FileCheck tests found in tests/check_*.blood"
    else
        printf "  FileCheck: %d/%d passed\n" "$fc_pass" "$fc_total"
        if [ "$fc_fail" -gt 0 ]; then errors=$((errors + 1)); fi
    fi

    # Step D: Function count sanity check
    # Note: reference_ir.ll from --emit llvm-ir-unopt may have fewer definitions
    # due to build caching, so we compare declarations (full ABI surface) and
    # just report definition counts for awareness.
    step "Checking function counts"
    local ref_decls self_decls self_defines
    ref_decls=$(grep -c '^declare ' reference_ir.ll)
    self_decls=$(grep -c '^declare ' "$ir_file")
    self_defines=$(grep -c '^define ' "$ir_file")

    printf "  Self-compiled: %d definitions, %d declarations\n" "$self_defines" "$self_decls"
    printf "  Reference:     %d declarations\n" "$ref_decls"

    # Declaration count should be close (self may have fewer if it doesn't use all runtime fns)
    if [ "$self_decls" -gt "$ref_decls" ]; then
        local extra=$(( self_decls - ref_decls ))
        warn "Self-compiled has $extra more declarations than reference"
    else
        ok "Self-compiled declarations ($self_decls) <= reference ($ref_decls)"
    fi

    # Summary
    if [ "$errors" -gt 0 ]; then
        printf "\n\033[1;31mVerification failed: %d error(s)\033[0m\n" "$errors"
        return 1
    else
        printf "\n\033[1;32mAll verification checks passed.\033[0m\n"
        return 0
    fi
}

# Build second_gen with AddressSanitizer instrumentation
build_asan() {
    local ir_file="${1:-second_gen.ll}"
    [ -f "$ir_file" ] || die "$ir_file not found"
    [ -f "$RUNTIME_A" ] || die "Runtime library not found at $RUNTIME_A"
    [ -f "$RUNTIME_O" ] || die "Runtime object not found at $RUNTIME_O"

    step "Building ASan-instrumented binary from $ir_file"

    # Assemble IR to bitcode
    llvm-as-18 "$ir_file" -o second_gen_asan.bc
    ok "Assembled to bitcode"

    # Run ASan instrumentation pass
    opt-18 \
        -passes='module(asan-module),function(asan)' \
        second_gen_asan.bc -o second_gen_asan_inst.bc
    ok "ASan instrumentation applied"

    # Compile to object
    llc-18 second_gen_asan_inst.bc \
        -o second_gen_asan.o -filetype=obj -relocation-model=pic
    ok "Compiled to object"

    # Link with clang (handles ASan runtime linkage)
    # runtime.o already contains main() → blood_main(), no need for main_wrapper.c
    clang-18 second_gen_asan.o "$RUNTIME_O" "$RUNTIME_A" \
        -fsanitize=address -lstdc++ -lm -lpthread -ldl -no-pie \
        -o second_gen_asan
    ok "Linked second_gen_asan ($(wc -c < second_gen_asan) bytes)"

    # Clean intermediates
    rm -f second_gen_asan.bc second_gen_asan_inst.bc second_gen_asan.o

    printf "\n  Run with: ./second_gen_asan version\n"
    printf "  ASan will report memory errors with stack traces.\n"
}

# Binary search for the miscompiled function
bisect_functions() {
    local self_ir="${1:-second_gen.ll}"
    [ -f "$self_ir" ] || die "$self_ir not found"
    [ -f reference_ir.ll ] || generate_reference_ir
    [ -f "$RUNTIME_A" ] || die "Runtime library not found at $RUNTIME_A"
    [ -f "$RUNTIME_O" ] || die "Runtime object not found at $RUNTIME_O"

    step "Bisecting for miscompiled function"

    local bisect_dir
    bisect_dir=$(mktemp -d "$DIR/.bisect_XXXXXX")
    trap "rm -rf '$bisect_dir'" EXIT

    # Convert both IR files to bitcode
    llvm-as-18 reference_ir.ll -o "$bisect_dir/ref.bc"
    llvm-as-18 "$self_ir" -o "$bisect_dir/self.bc"
    ok "Assembled both IR files to bitcode"

    # Extract function names from self-compiled IR (only user functions, not declarations)
    grep '^define ' "$self_ir" | sed 's/^define [^@]*@\([^ (]*\).*/\1/' | sort -u > "$bisect_dir/all_funcs.txt"
    local total_funcs
    total_funcs=$(wc -l < "$bisect_dir/all_funcs.txt")
    ok "Found $total_funcs functions to bisect"

    if [ "$total_funcs" -eq 0 ]; then
        fail "No functions found in $self_ir"
        return 1
    fi

    # Test function: builds a hybrid binary and checks if it crashes on 'version'
    # Returns 0 if crash, 1 if no crash
    test_hybrid() {
        local func_list="$1"
        local hybrid_bc="$bisect_dir/hybrid.bc"

        # Start with reference bitcode
        cp "$bisect_dir/ref.bc" "$hybrid_bc"

        # Replace specified functions with self-compiled versions
        local extract_args=""
        while IFS= read -r fname; do
            [ -z "$fname" ] && continue
            extract_args="$extract_args --func=$fname"
        done < "$func_list"

        if [ -z "$extract_args" ]; then
            return 1  # No functions to test → no crash
        fi

        # Extract specified functions from self-compiled bitcode
        if ! llvm-extract-18 $extract_args \
                "$bisect_dir/self.bc" -o "$bisect_dir/extracted.bc" 2>/dev/null; then
            warn "Could not extract functions (some may be missing)"
            return 1
        fi

        # Remove those functions from reference and link with self-compiled versions
        local delete_args=""
        while IFS= read -r fname; do
            [ -z "$fname" ] && continue
            delete_args="$delete_args --delete=$fname"
        done < "$func_list"

        if ! llvm-extract-18 $delete_args \
                "$bisect_dir/ref.bc" -o "$bisect_dir/ref_trimmed.bc" 2>/dev/null; then
            # If deletion fails, try linking directly
            cp "$bisect_dir/ref.bc" "$bisect_dir/ref_trimmed.bc"
        fi

        if ! llvm-link-18 \
                "$bisect_dir/ref_trimmed.bc" "$bisect_dir/extracted.bc" \
                -o "$hybrid_bc" 2>/dev/null; then
            warn "Link failed for this subset"
            return 1
        fi

        # Build hybrid binary
        if ! llc-18 "$hybrid_bc" \
                -o "$bisect_dir/hybrid.o" -filetype=obj -relocation-model=pic 2>/dev/null; then
            warn "LLC failed for hybrid"
            return 1
        fi

        if ! clang-18 "$bisect_dir/hybrid.o" "$RUNTIME_O" "$RUNTIME_A" \
                -lm -ldl -lpthread -no-pie -o "$bisect_dir/hybrid" 2>/dev/null; then
            warn "Link failed for hybrid binary"
            return 1
        fi

        # Test: does it crash?
        if timeout 10 "$bisect_dir/hybrid" version >/dev/null 2>&1; then
            return 1  # No crash
        else
            return 0  # Crash!
        fi
    }

    # Binary search
    local lo=0
    local hi=$((total_funcs - 1))
    local iteration=0
    local max_iterations=20  # log2(3678) ≈ 12, with margin

    # First verify: does the full self-compiled set crash?
    cp "$bisect_dir/all_funcs.txt" "$bisect_dir/test_funcs.txt"
    if ! test_hybrid "$bisect_dir/test_funcs.txt"; then
        fail "Full self-compiled set does NOT crash — cannot bisect"
        printf "  The crash may require specific linking or execution conditions.\n"
        return 1
    fi
    ok "Confirmed: full self-compiled set crashes"

    while [ "$lo" -lt "$hi" ] && [ "$iteration" -lt "$max_iterations" ]; do
        iteration=$((iteration + 1))
        local mid=$(( (lo + hi) / 2 ))
        local count=$((mid - lo + 1))

        printf "  Bisect iteration %d: testing functions %d-%d of %d (range %d-%d)\n" \
            "$iteration" "$lo" "$mid" "$total_funcs" "$lo" "$hi"

        # Extract first half
        sed -n "$((lo + 1)),$((mid + 1))p" "$bisect_dir/all_funcs.txt" > "$bisect_dir/test_funcs.txt"

        if test_hybrid "$bisect_dir/test_funcs.txt"; then
            # Crash is in first half
            hi=$mid
            ok "Crash in first half (narrowed to $((hi - lo + 1)) functions)"
        else
            # Crash is in second half
            lo=$((mid + 1))
            ok "Crash in second half (narrowed to $((hi - lo + 1)) functions)"
        fi
    done

    # Report result
    local suspect
    suspect=$(sed -n "$((lo + 1))p" "$bisect_dir/all_funcs.txt")

    if [ "$lo" -eq "$hi" ]; then
        printf "\n\033[1;33mBisect result: likely miscompiled function:\033[0m\n"
        printf "  @%s (function #%d of %d)\n" "$suspect" "$lo" "$total_funcs"
    else
        printf "\n\033[1;33mBisect narrowed to %d functions (%d-%d):\033[0m\n" \
            "$((hi - lo + 1))" "$lo" "$hi"
        sed -n "$((lo + 1)),$((hi + 1))p" "$bisect_dir/all_funcs.txt" | while read -r f; do
            printf "  @%s\n" "$f"
        done
    fi

    printf "\n  To inspect, compare this function between reference_ir.ll and %s\n" "$self_ir"
}

# Smoke test a binary
smoke_test() {
    local bin="$1"
    [ -f "$bin" ] || die "$bin not found"
    local pass=0
    local total=0

    step "Smoke testing $bin"

    run_smoke() {
        local name="$1"
        shift
        total=$((total + 1))
        local start_ts rc=0
        start_ts=$(date +%s)
        "./$bin" "$@" >/dev/null 2>&1 || rc=$?
        local elapsed
        elapsed=$(elapsed_since "$start_ts")
        if [ "$rc" -eq 0 ]; then
            ok "$name ($elapsed)"; pass=$((pass + 1))
        else
            fail "$name: $(decode_exit $rc) ($elapsed)"
        fi
    }

    run_smoke "version"             version
    run_smoke "check common.blood"  check common.blood
    run_smoke "check token.blood"   check token.blood
    run_smoke "check lexer.blood"   check lexer.blood
    run_smoke "check main.blood"    check main.blood

    printf "\n  %d/%d passed\n" "$pass" "$total"
    [ "$pass" -eq "$total" ] && return 0 || return 1
}

# Run ground-truth tests through a compiler binary
run_ground_truth() {
    local bin="$1"
    [ -f "$bin" ] || die "$bin not found"
    [ -d "$GROUND_TRUTH" ] || die "Ground-truth tests not found at $GROUND_TRUTH"

    step "Running ground-truth tests through $bin"

    local pass=0 total=0 comp_fail=0 run_fail=0 skip=0

    for src in "$GROUND_TRUTH"/t00_*.blood; do
        local name
        name="$(basename "$src" .blood)"
        total=$((total + 1))

        # Skip compile-fail tests (they should fail)
        if head -1 "$src" | grep -q '^// COMPILE_FAIL:'; then
            skip=$((skip + 1))
            continue
        fi
        # Skip XFAIL tests
        if head -1 "$src" | grep -q '^// XFAIL:'; then
            skip=$((skip + 1))
            continue
        fi

        # Compile with our compiler
        local tmpdir
        tmpdir=$(mktemp -d)
        if ! "./$bin" build "$src" -o "$tmpdir/out.ll" >/dev/null 2>&1; then
            fail "$name (compile)"
            comp_fail=$((comp_fail + 1))
            rm -rf "$tmpdir"
            continue
        fi

        # Run the compiled binary
        local actual exit_code=0
        actual=$("$tmpdir/out" 2>/dev/null) || exit_code=$?

        # Check expected output
        local expected=""
        expected=$(grep '^// EXPECT:' "$src" | sed 's|^// EXPECT: *||' || true)

        if [ -n "$expected" ]; then
            if [ "$actual" = "$expected" ]; then
                ok "$name"
                pass=$((pass + 1))
            else
                fail "$name (output mismatch)"
                run_fail=$((run_fail + 1))
            fi
        else
            # No expected output — just check exit code
            local expect_exit=""
            expect_exit=$(grep '^// EXPECT_EXIT:' "$src" | head -1 | sed 's|^// EXPECT_EXIT: *||' || true)
            if [ -z "$expect_exit" ]; then expect_exit="0"; fi

            if [ "$expect_exit" = "nonzero" ] && [ "$exit_code" -ne 0 ]; then
                ok "$name"
                pass=$((pass + 1))
            elif [ "$exit_code" = "$expect_exit" ]; then
                ok "$name"
                pass=$((pass + 1))
            else
                fail "$name (exit $exit_code, expected $expect_exit)"
                run_fail=$((run_fail + 1))
            fi
        fi

        rm -rf "$tmpdir"
    done

    printf "\n  Passed: %d  Compile fail: %d  Run fail: %d  Skipped: %d  Total: %d\n" \
        "$pass" "$comp_fail" "$run_fail" "$skip" "$total"
    [ "$((comp_fail + run_fail))" -eq 0 ] && return 0 || return 1
}

# Run smoke tests (tests/ directory) through a compiler binary
run_smoke_tests() {
    local bin="$1"
    [ -f "$bin" ] || die "$bin not found"
    local test_dir="$DIR/tests"
    [ -d "$test_dir" ] || die "Test directory not found at $test_dir"

    step "Running smoke tests through $bin"

    local pass=0 total=0 fail_count=0

    for src in "$test_dir"/t*.blood; do
        [ -f "$src" ] || continue
        local name
        name="$(basename "$src" .blood)"
        total=$((total + 1))

        # Compile and run with the compiler
        # Use --quiet to suppress build progress; filter remaining "Build successful:" line
        local actual exit_code=0
        actual=$("$bin" run --quiet "$src" 2>/dev/null | grep -v '^Build successful:\|^Running:') || exit_code=$?

        if [ "$exit_code" -ne 0 ]; then
            fail "$name (exit code $exit_code)"
            fail_count=$((fail_count + 1))
            continue
        fi

        # Check expected output
        local expected=""
        expected=$(grep '^// EXPECT:' "$src" | sed 's|^// EXPECT: *||')

        if [ "$actual" = "$expected" ]; then
            ok "$name"
            pass=$((pass + 1))
        else
            fail "$name (output mismatch)"
            printf "      expected: %s\n" "$expected"
            printf "      actual:   %s\n" "$actual"
            fail_count=$((fail_count + 1))
        fi

        # Clean up compiled binary and cache
        local src_base="${src%.blood}"
        rm -f "$src_base"
        rm -rf "${src}.blood_objs" "${src_base}.blood_objs"
    done

    printf "\n  %d/%d passed\n" "$pass" "$total"
    [ "$fail_count" -eq 0 ] && return 0 || return 1
}

case "${1:-full}" in
    full)
        PIPELINE_START=$(date +%s)

        build_first_gen "--timings"

        build_second_gen

        step "Verification"
        if verify_ir second_gen.ll; then
            ok "Verification passed"
        else
            warn "Verification had issues (see above)"
        fi

        step "Smoke test"
        if smoke_test second_gen; then
            printf "\n\033[1;32mSelf-hosting pipeline complete.\033[0m Total: %s\n" "$(elapsed_since "$PIPELINE_START")"
        else
            printf "\n\033[1;33mSmoke test had failures.\033[0m Total: %s\n" "$(elapsed_since "$PIPELINE_START")"
            exit 1
        fi
        printf "Log: %s\n" "$LOG_FILE"
        ;;

    rebuild)
        PIPELINE_START=$(date +%s)

        build_second_gen

        step "Verification"
        if verify_ir second_gen.ll; then
            ok "Verification passed"
        else
            warn "Verification had issues (see above)"
        fi

        step "Smoke test"
        if smoke_test second_gen; then
            printf "\n\033[1;32mRebuild complete.\033[0m Total: %s\n" "$(elapsed_since "$PIPELINE_START")"
        else
            printf "\n\033[1;33mSmoke test had failures.\033[0m Total: %s\n" "$(elapsed_since "$PIPELINE_START")"
            exit 1
        fi
        printf "Log: %s\n" "$LOG_FILE"
        ;;

    test)
        smoke_test "${2:-second_gen}"
        ;;

    ground-truth)
        run_ground_truth "${2:-first_gen}"
        ;;

    smoke-tests)
        run_smoke_tests "${2:-$BLOOD_RUST}"
        ;;

    verify)
        verify_ir "${2:-second_gen.ll}"
        ;;

    ir-check)
        # Run just the FileCheck tests (subset of verify)
        local_ir="${2:-second_gen.ll}"
        [ -f first_gen ] || die "first_gen not found. Build it first."

        step "Running FileCheck tests"
        local_pass=0
        local_fail=0
        local_total=0

        for check_src in "$DIR"/tests/check_*.blood; do
            [ -f "$check_src" ] || continue
            local_name="$(basename "$check_src" .blood)"
            local_total=$((local_total + 1))

            local_tmpdir=$(mktemp -d)

            if ! ./first_gen build "$check_src" -o "$local_tmpdir/check_out.ll" >/dev/null 2>&1; then
                fail "$local_name (compile failed)"
                local_fail=$((local_fail + 1))
                rm -rf "$local_tmpdir"
                continue
            fi

            if FileCheck-18 --input-file="$local_tmpdir/check_out.ll" "$check_src" 2>/dev/null; then
                ok "$local_name"
                local_pass=$((local_pass + 1))
            else
                fail "$local_name"
                # Show FileCheck output for debugging
                FileCheck-18 --input-file="$local_tmpdir/check_out.ll" "$check_src" 2>&1 | head -10 || true
                local_fail=$((local_fail + 1))
            fi

            rm -rf "$local_tmpdir"
        done

        if [ "$local_total" -eq 0 ]; then
            warn "No FileCheck tests found in tests/check_*.blood"
        else
            printf "\n  %d/%d FileCheck tests passed\n" "$local_pass" "$local_total"
            if [ "$local_fail" -gt 0 ]; then
                exit 1
            fi
        fi
        ;;

    asan)
        build_asan "${2:-second_gen.ll}"
        ;;

    bisect)
        bisect_functions "${2:-second_gen.ll}"
        ;;

    emit)
        # Emit intermediate representation using blood-rust
        local_stage="${2:-llvm-ir}"
        [ -f "$BLOOD_RUST" ] || die "blood-rust not found at $BLOOD_RUST"

        step "Emitting $local_stage for main.blood"
        $BLOOD_RUST build --emit "$local_stage" -o "${local_stage}.ll" main.blood
        ;;

    timings)
        build_first_gen "--timings"
        ;;

    release)
        build_first_gen "--release --timings"
        ;;

    clean)
        step "Cleaning build artifacts"
        # Binaries and intermediate files
        rm -f first_gen second_gen second_gen_asan
        rm -f *.ll *.o *.bc core
        rm -rf .bisect_* .blood-cache .logs
        find "${DIR}" -name ".blood-cache" -type d -exec rm -rf {} + 2>/dev/null || true
        # Per-file incremental compilation caches (next to source files)
        rm -rf "${DIR}"/*.blood_objs
        rm -rf "${DIR}"/tests/*.blood_objs
        # Global per-definition object cache
        rm -rf "${HOME}"/.blood*/cache/
        ok "Build artifacts and all caches removed"
        ;;

    *)
        cat <<'USAGE'
Usage: ./build_selfhost.sh <command> [args]

Commands:
  full              Build from scratch (blood-rust → first_gen → second_gen)
  rebuild           Reuse existing first_gen to rebuild second_gen
  test [binary]     Smoke test a binary (default: second_gen)
  ground-truth [b]  Run ground-truth tests through binary (default: first_gen)
  smoke-tests [b]   Run smoke tests (tests/) through binary (default: blood-rust)
  verify [ir]       Run all verification checks (default: second_gen.ll)
  ir-check [ir]     Run FileCheck tests against compiler output
  asan [ir]         Build with AddressSanitizer (default: second_gen.ll)
  bisect [ir]       Binary search for miscompiled function
  emit [stage]      Emit intermediate IR (ast|hir|mir|llvm-ir|llvm-ir-unopt)
  timings           Build first_gen with per-phase compilation timing
  release           Build first_gen with --release optimizations
  clean             Remove build artifacts

Verification commands:
  verify            Runs: opt verify + declaration diff + FileCheck + function counts
  ir-check          Runs: FileCheck tests only (quick check)
  asan              Produces: second_gen_asan (run to get memory error reports)
  bisect            Identifies: which function causes second_gen to crash

Environment:
  BLOOD_RUST        Path to blood-rust compiler (default: ~/blood/compiler-rust/target/release/blood)
  RUNTIME_O         Path to runtime.o (default: ~/blood/compiler-rust/runtime/runtime.o)
  RUNTIME_A         Path to libblood_runtime.a (default: ~/blood/compiler-rust/target/release/libblood_runtime.a)
  GROUND_TRUTH      Path to ground-truth test dir (default: ~/blood/compiler-rust/tests/ground-truth)
USAGE
        exit 1
        ;;
esac
