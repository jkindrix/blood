#!/bin/bash
# Compare LLVM IR type signatures between reference compiler output and first_gen output.
# Identifies enum/struct type mismatches that cause layout bugs in second_gen.
#
# Usage: ./tools/compare_ir_types.sh [reference_ir] [second_gen_ir]

set -euo pipefail

REF_IR="${1:-src/selfhost/build/reference_ir.ll}"
SEC_IR="${2:-src/selfhost/build/second_gen.ll}"

if [[ ! -f "$REF_IR" ]]; then
    echo "ERROR: Reference IR not found: $REF_IR" >&2
    exit 1
fi
if [[ ! -f "$SEC_IR" ]]; then
    echo "ERROR: Second-gen IR not found: $SEC_IR" >&2
    exit 1
fi

echo "=== Comparing IR types ==="
echo "Reference: $REF_IR"
echo "Second-gen: $SEC_IR"
echo ""

# Extract define signatures: "define <rettype> @<name>(<params>)"
# We'll extract function name + first param type for comparison
extract_defines() {
    local ir_file="$1"
    # Extract lines starting with "define", get function name and full signature
    grep -oP 'define [^@]*@(def\d+_\w+)\([^)]*\)' "$ir_file" | \
        sed 's/define //' | \
        sort
}

echo "--- Extracting function signatures ---"

# Extract just the function name → return type + param types mapping
# Format: defN_name → { return_type | param_types }
extract_func_types() {
    local ir_file="$1"
    grep '^define ' "$ir_file" | \
        sed -E 's/define (internal |dso_local )?//' | \
        grep -oP '[^@]*@(def\d+_\w+)\([^)]*\)' | \
        while IFS= read -r line; do
            # Extract function name
            local fname
            fname=$(echo "$line" | grep -oP '@\Kdef\d+_\w+')
            # Extract everything before @name as return type
            local rettype
            rettype=$(echo "$line" | sed -E "s/@${fname}.*//" | sed 's/[[:space:]]*$//')
            # Extract params
            local params
            params=$(echo "$line" | grep -oP '\(\K[^)]*')
            echo "${fname}|${rettype}|${params}"
        done | sort
}

echo "Extracting reference IR signatures..."
extract_func_types "$REF_IR" > /tmp/ref_sigs.txt
echo "  Found $(wc -l < /tmp/ref_sigs.txt) functions"

echo "Extracting second-gen IR signatures..."
extract_func_types "$SEC_IR" > /tmp/sec_sigs.txt
echo "  Found $(wc -l < /tmp/sec_sigs.txt) functions"

echo ""
echo "--- Enum type pattern comparison ---"
echo ""

# Extract all enum-shaped types { i32, [N x i64] } and count occurrences
echo "Reference IR enum patterns:"
grep -oP '\{ i32, \[\d+ x i\d+\] \}' "$REF_IR" | sort | uniq -c | sort -rn | head -20

echo ""
echo "Second-gen IR enum patterns:"
grep -oP '\{ i32, \[\d+ x i\d+\] \}' "$SEC_IR" | sort | uniq -c | sort -rn | head -20

echo ""
echo "--- Function signature mismatches ---"
echo ""

# Join on function name and compare
join -t'|' -j1 /tmp/ref_sigs.txt /tmp/sec_sigs.txt | \
    while IFS='|' read -r fname ref_ret ref_params sec_ret sec_params; do
        if [[ "$ref_ret" != "$sec_ret" ]] || [[ "$ref_params" != "$sec_params" ]]; then
            echo "MISMATCH: $fname"
            if [[ "$ref_ret" != "$sec_ret" ]]; then
                echo "  Return: REF=$ref_ret"
                echo "          SEC=$sec_ret"
            fi
            if [[ "$ref_params" != "$sec_params" ]]; then
                echo "  Params: REF=$ref_params"
                echo "          SEC=$sec_params"
            fi
            echo ""
        fi
    done | head -200

echo ""
echo "--- Alloca type comparison for key functions ---"
echo ""

# Compare alloca types in specific functions known to use AST types
for funcname in lower_expr lower_block lower_fn_body lower_block_to_expr; do
    echo "=== $funcname ==="

    # Find the defN for this function in reference
    ref_def=$(grep -oP "def\d+_${funcname}" "$REF_IR" | head -1 || true)
    sec_def=$(grep -oP "def\d+_${funcname}" "$SEC_IR" | head -1 || true)

    if [[ -n "$ref_def" ]] && [[ -n "$sec_def" ]]; then
        echo "  REF: $ref_def, SEC: $sec_def"

        # Extract alloca types from each
        ref_allocas=$(grep -A1 "^define.*@${ref_def}(" "$REF_IR" -A 50000 | \
            grep 'alloca' | head -5 | \
            grep -oP 'alloca \K[^,]+' || true)
        sec_allocas=$(grep -A1 "^define.*@${sec_def}(" "$SEC_IR" -A 50000 | \
            grep 'alloca' | head -5 | \
            grep -oP 'alloca \K[^,]+' || true)

        echo "  REF allocas: $ref_allocas"
        echo "  SEC allocas: $sec_allocas"
    else
        echo "  Not found (ref=$ref_def, sec=$sec_def)"
    fi
    echo ""
done

echo "--- Summary ---"
echo "Total ref functions: $(wc -l < /tmp/ref_sigs.txt)"
echo "Total sec functions: $(wc -l < /tmp/sec_sigs.txt)"
echo "Mismatched functions: $(join -t'|' -j1 /tmp/ref_sigs.txt /tmp/sec_sigs.txt | \
    while IFS='|' read -r fname ref_ret ref_params sec_ret sec_params; do
        if [[ "$ref_ret" != "$sec_ret" ]] || [[ "$ref_params" != "$sec_params" ]]; then
            echo "x"
        fi
    done | wc -l)"
