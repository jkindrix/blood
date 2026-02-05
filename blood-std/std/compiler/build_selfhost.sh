#!/bin/bash
# build_selfhost.sh - Automates the self-hosting pipeline
#
# Usage:
#   ./build_selfhost.sh              # Full pipeline: blood-rust → first_gen → second_gen
#   ./build_selfhost.sh rebuild      # Skip blood-rust, reuse existing first_gen
#   ./build_selfhost.sh test [bin]   # Smoke test a binary (default: second_gen)
#   ./build_selfhost.sh ground-truth # Run ground-truth tests through first_gen
#   ./build_selfhost.sh emit [stage] # Emit intermediate IR (ast|hir|mir|llvm-ir|llvm-ir-unopt)
#   ./build_selfhost.sh timings      # Build first_gen with per-phase timing
#   ./build_selfhost.sh release      # Build first_gen with --release optimizations
#   ./build_selfhost.sh clean        # Remove build artifacts
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$DIR"

# Paths (configurable via environment)
BLOOD_RUST="${BLOOD_RUST:-$HOME/blood-rust/target/release/blood}"
RUNTIME_O="${RUNTIME_O:-$HOME/blood-rust/runtime/runtime.o}"
RUNTIME_A="${RUNTIME_A:-$HOME/blood-rust/target/release/libblood_runtime.a}"
GROUND_TRUTH="${GROUND_TRUTH:-$HOME/blood-rust/tests/ground-truth}"

step()  { printf "\n\033[1;34m==> %s\033[0m\n" "$1"; }
ok()    { printf "  \033[1;32m✓\033[0m %s\n" "$1"; }
fail()  { printf "  \033[1;31m✗\033[0m %s\n" "$1"; }
warn()  { printf "  \033[1;33m!\033[0m %s\n" "$1"; }
die()   { printf "\033[1;31mERROR:\033[0m %s\n" "$1" >&2; exit 1; }

# Build first_gen from blood-rust
build_first_gen() {
    local flags="${1:-}"
    [ -f "$BLOOD_RUST" ] || die "blood-rust not found at $BLOOD_RUST"

    step "Building first_gen with blood-rust"
    $BLOOD_RUST build main.blood $flags
    mv main first_gen
    ok "first_gen created ($(wc -c < first_gen) bytes)"
}

# Self-compile: first_gen → second_gen
build_second_gen() {
    [ -f first_gen ] || die "first_gen not found. Run './build_selfhost.sh' first."

    step "Self-compiling (first_gen → second_gen)"
    ./first_gen build main.blood -o second_gen.ll
    ok "second_gen created ($(wc -c < second_gen) bytes)"
}

# Smoke test a binary
smoke_test() {
    local bin="$1"
    [ -f "$bin" ] || die "$bin not found"
    local pass=0
    local total=0

    step "Smoke testing $bin"

    # Test 1: version command
    total=$((total + 1))
    if "./$bin" version >/dev/null 2>&1; then
        ok "version"; pass=$((pass + 1))
    else
        fail "version (exit $?)"
    fi

    # Test 2: check common.blood
    total=$((total + 1))
    if "./$bin" check common.blood >/dev/null 2>&1; then
        ok "check common.blood"; pass=$((pass + 1))
    else
        fail "check common.blood (exit $?)"
    fi

    # Test 3: check token.blood (cross-module import)
    total=$((total + 1))
    if "./$bin" check token.blood >/dev/null 2>&1; then
        ok "check token.blood"; pass=$((pass + 1))
    else
        fail "check token.blood (exit $?)"
    fi

    # Test 4: check lexer.blood (chained imports)
    total=$((total + 1))
    if "./$bin" check lexer.blood >/dev/null 2>&1; then
        ok "check lexer.blood"; pass=$((pass + 1))
    else
        fail "check lexer.blood (exit $?)"
    fi

    # Test 5: check main.blood (full compiler)
    total=$((total + 1))
    if "./$bin" check main.blood >/dev/null 2>&1; then
        ok "check main.blood"; pass=$((pass + 1))
    else
        fail "check main.blood (exit $?)"
    fi

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
        build_first_gen "--timings"

        build_second_gen

        step "Smoke test"
        if smoke_test second_gen; then
            printf "\n\033[1;32mSelf-hosting pipeline complete.\033[0m\n"
        else
            printf "\n\033[1;33mSmoke test had failures (may be expected with BUG-008).\033[0m\n"
            exit 1
        fi
        ;;

    rebuild)
        build_second_gen

        step "Smoke test"
        if smoke_test second_gen; then
            printf "\n\033[1;32mRebuild complete.\033[0m\n"
        else
            printf "\n\033[1;33mSmoke test had failures.\033[0m\n"
            exit 1
        fi
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

    emit)
        # Emit intermediate representation using blood-rust
        local_stage="${2:-llvm-ir}"
        [ -f "$BLOOD_RUST" ] || die "blood-rust not found at $BLOOD_RUST"

        step "Emitting $local_stage for main.blood"
        $BLOOD_RUST build --emit "$local_stage" main.blood
        ;;

    timings)
        build_first_gen "--timings"
        ;;

    release)
        build_first_gen "--release --timings"
        ;;

    clean)
        step "Cleaning build artifacts"
        rm -f first_gen second_gen
        rm -f *.ll *.o *.bc core
        ok "Build artifacts removed"
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
  emit [stage]      Emit intermediate IR (ast|hir|mir|llvm-ir|llvm-ir-unopt)
  timings           Build first_gen with per-phase compilation timing
  release           Build first_gen with --release optimizations
  clean             Remove build artifacts

Environment:
  BLOOD_RUST        Path to blood-rust compiler (default: ~/blood-rust/target/release/blood)
  GROUND_TRUTH      Path to ground-truth test dir (default: ~/blood-rust/tests/ground-truth)
USAGE
        exit 1
        ;;
esac
