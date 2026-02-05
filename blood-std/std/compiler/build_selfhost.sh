#!/bin/bash
# build_selfhost.sh - Automates the self-hosting pipeline
#
# Usage:
#   ./build_selfhost.sh           # Full pipeline: blood-rust → first_gen → second_gen
#   ./build_selfhost.sh rebuild   # Skip blood-rust, reuse existing first_gen
#   ./build_selfhost.sh test      # Smoke test existing second_gen
#   ./build_selfhost.sh clean     # Remove build artifacts
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$DIR"

BLOOD_RUST="$HOME/blood-rust/target/release/blood"

step() { printf "\n\033[1;34m==> %s\033[0m\n" "$1"; }
ok()   { printf "  \033[1;32m✓\033[0m %s\n" "$1"; }
fail() { printf "  \033[1;31m✗\033[0m %s\n" "$1"; }
die()  { printf "\033[1;31mERROR:\033[0m %s\n" "$1" >&2; exit 1; }

smoke_test() {
    local bin="$1"
    [ -f "$bin" ] || die "$bin not found"

    step "Smoke testing $bin"

    # Test 1: --version (should exit 0)
    if "./$bin" version >/dev/null 2>&1; then
        ok "version command works"
    else
        fail "version command failed (exit $?)"
        return 1
    fi

    # Test 2: check a source file
    if "./$bin" check common.blood >/dev/null 2>&1; then
        ok "check common.blood works"
    else
        fail "check common.blood failed (exit $?)"
        return 1
    fi

    return 0
}

case "${1:-full}" in
    full)
        [ -f "$BLOOD_RUST" ] || die "blood-rust not found at $BLOOD_RUST"

        step "Step 1/3: Building first_gen with blood-rust"
        $BLOOD_RUST build main.blood
        mv main first_gen
        ok "first_gen created ($(wc -c < first_gen) bytes)"

        step "Step 2/3: Self-compiling (first_gen → second_gen)"
        ./first_gen build main.blood -o second_gen.ll
        ok "second_gen created ($(wc -c < second_gen) bytes)"

        step "Step 3/3: Smoke test"
        if smoke_test second_gen; then
            printf "\n\033[1;32mSelf-hosting pipeline complete.\033[0m\n"
        else
            printf "\n\033[1;31mSmoke test failed.\033[0m\n"
            exit 1
        fi
        ;;

    rebuild)
        [ -f first_gen ] || die "first_gen not found. Run './build_selfhost.sh' first."

        step "Step 1/2: Self-compiling (first_gen → second_gen)"
        ./first_gen build main.blood -o second_gen.ll
        ok "second_gen created ($(wc -c < second_gen) bytes)"

        step "Step 2/2: Smoke test"
        if smoke_test second_gen; then
            printf "\n\033[1;32mRebuild complete.\033[0m\n"
        else
            printf "\n\033[1;31mSmoke test failed.\033[0m\n"
            exit 1
        fi
        ;;

    test)
        smoke_test "${2:-second_gen}"
        ;;

    clean)
        step "Cleaning build artifacts"
        rm -f first_gen second_gen
        rm -f *.ll *.o *.bc core
        ok "Build artifacts removed"
        ;;

    *)
        echo "Usage: $0 [full|rebuild|test|clean]"
        echo ""
        echo "Commands:"
        echo "  full      Build from scratch (blood-rust → first_gen → second_gen)"
        echo "  rebuild   Reuse existing first_gen to rebuild second_gen"
        echo "  test      Smoke test existing second_gen (or: test <binary>)"
        echo "  clean     Remove build artifacts"
        exit 1
        ;;
esac
