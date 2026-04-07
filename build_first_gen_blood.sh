#!/bin/bash
# Build first_gen_blood — the selfhost compiler linked against the Blood-native runtime.
# This replaces the Rust runtime (libblood_runtime.a) with the Blood-native runtime.
#
# Prerequisites:
#   - src/bootstrap/target/release/blood (bootstrap compiler, symlink to ~/blood)
#   - src/selfhost/build/debug/main (selfhost first_gen, built by bootstrap)
#   - runtime/runtime.o (C runtime stub providing main())
#
# Pipeline:
#   1. Compile Blood runtime source → LLVM IR (using selfhost compiler)
#   2. Post-process IR: strip conflicting declares, inject builtins
#   3. Compile IR → object file → static archive
#   4. Build first_gen linking against Blood runtime instead of Rust runtime

set -euo pipefail
cd "$(dirname "$0")"

SELFHOST="${SELFHOST:-src/selfhost/build/debug/main}"
BOOTSTRAP="${BOOTSTRAP:-src/bootstrap/target/release/blood}"
RUNTIME_DIR="runtime/blood-runtime"
BUILD_DIR="$RUNTIME_DIR/build/debug"
OUTPUT="build/debug/first_gen_blood"

mkdir -p "$BUILD_DIR" "$(dirname "$OUTPUT")"

echo "=== Step 1: Compile Blood runtime to LLVM IR ==="
"$SELFHOST" build --emit llvm-ir --no-cache "$RUNTIME_DIR/lib.blood"

echo "=== Step 2: Post-process IR ==="
python3 "$RUNTIME_DIR/build_runtime.py" "$BUILD_DIR/lib.ll" "$BUILD_DIR/lib_clean.ll"

echo "=== Step 3: Compile to archive ==="
# Use PIPESTATUS to check llc's exit code specifically. grep returning 1
# (no noise matched) must not mask llc failures, and llc failures must not
# be hidden by a blanket `|| true`.
set +o pipefail
llc-18 -filetype=obj -relocation-model=pic "$BUILD_DIR/lib_clean.ll" -o "$BUILD_DIR/lib.o" 2>&1 \
    | grep -v 'inlinable function\|ignoring invalid debug'
llc_status="${PIPESTATUS[0]}"
set -o pipefail
if [ "$llc_status" -ne 0 ]; then
    echo "ERROR: llc-18 failed (exit $llc_status) compiling runtime IR at $BUILD_DIR/lib_clean.ll" >&2
    exit 1
fi
[ -f "$BUILD_DIR/lib.o" ] || { echo "ERROR: llc-18 did not produce $BUILD_DIR/lib.o" >&2; exit 1; }
ar rcs "$BUILD_DIR/libblood_runtime_blood.a" "$BUILD_DIR/lib.o"
echo "  Archive: $BUILD_DIR/libblood_runtime_blood.a ($(stat -c%s "$BUILD_DIR/libblood_runtime_blood.a") bytes)"

echo "=== Step 4: Link first_gen against Blood runtime ==="
BLOOD_RUST_RUNTIME="$BUILD_DIR/libblood_runtime_blood.a" \
    "$BOOTSTRAP" build src/selfhost/main.blood -o "$OUTPUT" 2>&1 | tail -3

echo ""
echo "=== Result ==="
echo "  Binary: $OUTPUT ($(stat -c%s "$OUTPUT") bytes)"
echo "  Rust symbols: $(nm "$OUTPUT" | grep -c '_ZN\|_RN')"
echo ""
echo "To test: cd src/selfhost && bash -c 'source ./build_selfhost.sh; do_test_golden $PWD/$OUTPUT'"
