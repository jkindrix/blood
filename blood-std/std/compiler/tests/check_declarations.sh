#!/bin/bash
# check_declarations.sh - Compare runtime declarations between two IR files
#
# Usage:
#   ./tests/check_declarations.sh <reference.ll> <self_compiled.ll>
#
# Compares declarations with opaque pointer normalization (i8* → ptr).
# The self-hosted compiler may use fewer runtime functions than blood-rust,
# so we check:
#   1. Functions declared in BOTH files must have matching signatures
#   2. Functions only in self-compiled (new) are flagged as warnings
#   3. Functions only in reference (unused by self-hosted) are expected
#
# Known intentional signature differences are annotated.
set -euo pipefail

if [ $# -lt 2 ]; then
    echo "Usage: $0 <reference.ll> <self_compiled.ll>"
    exit 1
fi

REF="$1"
SELF="$2"

[ -f "$REF" ]  || { echo "ERROR: $REF not found"; exit 1; }
[ -f "$SELF" ] || { echo "ERROR: $SELF not found"; exit 1; }

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

# Normalize opaque pointers: replace i8* with ptr, i32* with ptr, etc.
normalize_ptr() {
    sed -E 's/\bi[0-9]+\*/ptr/g; s/\{ ptr, i64 \}\*/ptr/g'
}

# Extract and normalize declarations, keyed by function name
grep '^declare ' "$REF"  | normalize_ptr | sort > "$TMPDIR/decls_ref.txt"
grep '^declare ' "$SELF" | normalize_ptr | sort > "$TMPDIR/decls_self.txt"

REF_COUNT=$(wc -l < "$TMPDIR/decls_ref.txt")
SELF_COUNT=$(wc -l < "$TMPDIR/decls_self.txt")

echo "Reference declarations:     $REF_COUNT (after normalization)"
echo "Self-compiled declarations: $SELF_COUNT (after normalization)"

# Extract function names from declarations
extract_names() {
    sed 's/^declare [^@]*@\([^ (]*\).*/\1/' | sort -u
}

cat "$TMPDIR/decls_ref.txt"  | extract_names > "$TMPDIR/names_ref.txt"
cat "$TMPDIR/decls_self.txt" | extract_names > "$TMPDIR/names_self.txt"

# Find common functions
comm -12 "$TMPDIR/names_ref.txt" "$TMPDIR/names_self.txt" > "$TMPDIR/names_common.txt"
COMMON_COUNT=$(wc -l < "$TMPDIR/names_common.txt")

# Find functions only in self-compiled (unexpected new declarations)
comm -13 "$TMPDIR/names_ref.txt" "$TMPDIR/names_self.txt" > "$TMPDIR/names_self_only.txt"
SELF_ONLY=$(wc -l < "$TMPDIR/names_self_only.txt")

# Find functions only in reference (unused by self-hosted — expected)
comm -23 "$TMPDIR/names_ref.txt" "$TMPDIR/names_self.txt" > "$TMPDIR/names_ref_only.txt"
REF_ONLY=$(wc -l < "$TMPDIR/names_ref_only.txt")

echo "Common functions: $COMMON_COUNT"
echo "Self-hosted only: $SELF_ONLY"
echo "Reference only:   $REF_ONLY (expected — not used by self-hosted)"

# Known intentional signature differences between blood-rust and self-hosted.
# blood_perform: self-hosted uses simplified (i64, i64, i64) signature
KNOWN_SIG_DIFFS=(
    "blood_perform"
)

is_known_diff() {
    local fname="$1"
    for pattern in "${KNOWN_SIG_DIFFS[@]}"; do
        if [ "$fname" = "$pattern" ]; then
            return 0
        fi
    done
    return 1
}

# Check: do common functions have matching signatures?
MISMATCH=0
KNOWN_MISMATCH=0
while IFS= read -r fname; do
    ref_sig=$(grep "@${fname}(" "$TMPDIR/decls_ref.txt" | head -1)
    self_sig=$(grep "@${fname}(" "$TMPDIR/decls_self.txt" | head -1)

    if [ "$ref_sig" != "$self_sig" ]; then
        if is_known_diff "$fname"; then
            echo "  KNOWN: @$fname"
            echo "    ref:  $ref_sig"
            echo "    self: $self_sig"
            KNOWN_MISMATCH=$((KNOWN_MISMATCH + 1))
        else
            echo "  MISMATCH: @$fname"
            echo "    ref:  $ref_sig"
            echo "    self: $self_sig"
            MISMATCH=$((MISMATCH + 1))
        fi
    fi
done < "$TMPDIR/names_common.txt"

# Report new declarations (only in self-compiled)
if [ "$SELF_ONLY" -gt 0 ]; then
    echo ""
    echo "New declarations in self-compiled (not in reference):"
    while IFS= read -r fname; do
        # llvm.* intrinsics are expected to differ
        if [[ "$fname" == llvm.* ]]; then
            continue
        fi
        sig=$(grep "@${fname}(" "$TMPDIR/decls_self.txt" | head -1)
        echo "  NEW: $sig"
    done < "$TMPDIR/names_self_only.txt"
fi

# Summary
echo ""
if [ "$MISMATCH" -gt 0 ]; then
    echo "FAIL: $MISMATCH unexpected signature mismatch(es) ($KNOWN_MISMATCH known)."
    exit 1
elif [ "$KNOWN_MISMATCH" -gt 0 ]; then
    echo "PASS: $KNOWN_MISMATCH known/intentional difference(s), no unexpected mismatches."
    exit 0
else
    echo "PASS: All common declarations match exactly."
    exit 0
fi
