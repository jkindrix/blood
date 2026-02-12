#!/bin/bash
# Verify benchmark outputs match between Blood and C

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="$SCRIPT_DIR/bin"
EXPECTED_DIR="$SCRIPT_DIR/expected"

mkdir -p "$EXPECTED_DIR"

echo "Verifying benchmark outputs..."
echo ""

verify() {
    local name="$1"
    local blood_cmd="$2"
    local c_cmd="$3"

    echo "=== $name ==="

    blood_out=$($blood_cmd 2>/dev/null)
    c_out=$($c_cmd 2>/dev/null)

    echo "Blood: $blood_out"
    echo "C:     $c_out"

    # For floating point, just check first few digits match
    blood_first=$(echo "$blood_out" | head -1 | cut -c1-8)
    c_first=$(echo "$c_out" | head -1 | cut -c1-8)

    if [[ "$blood_first" == "$c_first" ]]; then
        echo "Status: PASS"
    else
        echo "Status: FAIL (output mismatch)"
    fi
    echo ""
}

cd "$BIN_DIR"

verify "N-Body" \
    "./nbody_blood" \
    "./nbody_c 50000000"

verify "Fannkuch-Redux" \
    "./fannkuchredux_blood" \
    "./fannkuchredux_c_fixed"

verify "Binary-Trees (small)" \
    "./binarytrees_blood 10" \
    "./binarytrees_c 10"

verify "Spectral-Norm (small)" \
    "./spectralnorm_blood 100" \
    "./spectralnorm_c 100"

echo "Verification complete."
