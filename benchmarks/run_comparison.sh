#!/bin/bash
# Blood vs C vs Rust Comparative Benchmark
#
# This script runs benchmarks at sizes matching Blood's current implementations.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="$SCRIPT_DIR/clbg/bin"
RESULTS_DIR="$SCRIPT_DIR/clbg/results"
ITERATIONS=5

mkdir -p "$RESULTS_DIR"

echo "============================================================"
echo "  Blood Comparative Benchmark Suite"
echo "============================================================"
echo ""

TIMESTAMP=$(date +%Y%m%d_%H%M%S)
RESULTS_FILE="$RESULTS_DIR/comparison_$TIMESTAMP.md"

# Start the report
cat > "$RESULTS_FILE" << EOF
# Blood vs C vs Rust Benchmark Comparison

**Generated**: $(date)
**System**: $(uname -srm)
**Iterations**: $ITERATIONS per benchmark

## Methodology

- Each benchmark run $ITERATIONS times, median reported
- Wall-clock time measured in milliseconds
- All compilers use maximum optimization
- Blood benchmarks have hardcoded sizes (smaller than CLBG standard)

## Results

EOF

# Function to time a command and return milliseconds
time_cmd() {
    local start end elapsed
    start=$(date +%s%N)
    "$@" > /dev/null 2>&1
    end=$(date +%s%N)
    elapsed=$(( (end - start) / 1000000 ))
    echo "$elapsed"
}

# Function to run benchmark and get median
run_bench() {
    local name="$1"
    shift
    local times=()

    for ((i=0; i<ITERATIONS; i++)); do
        t=$(time_cmd "$@")
        times+=($t)
    done

    # Sort and get median
    IFS=$'\n' sorted=($(sort -n <<<"${times[*]}")); unset IFS
    local mid=$((ITERATIONS / 2))
    local median=${sorted[$mid]}
    local min=${sorted[0]}
    local max=${sorted[$((ITERATIONS-1))]}

    echo "$median $min $max"
}

run_suite() {
    local bench_name="$1"
    local c_cmd="$2"
    local rust_cmd="$3"
    local blood_cmd="$4"

    echo "=== $bench_name ==="
    echo "" >> "$RESULTS_FILE"
    echo "### $bench_name" >> "$RESULTS_FILE"
    echo "" >> "$RESULTS_FILE"
    echo "| Language | Median (ms) | Min | Max | Ratio vs C |" >> "$RESULTS_FILE"
    echo "|----------|-------------|-----|-----|------------|" >> "$RESULTS_FILE"

    # C
    echo -n "  C: "
    c_result=$(run_bench "c" $c_cmd)
    c_median=$(echo $c_result | awk '{print $1}')
    c_min=$(echo $c_result | awk '{print $2}')
    c_max=$(echo $c_result | awk '{print $3}')
    echo "${c_median}ms (${c_min}-${c_max})"
    echo "| C | $c_median | $c_min | $c_max | 1.00x |" >> "$RESULTS_FILE"

    # Rust
    echo -n "  Rust: "
    rust_result=$(run_bench "rust" $rust_cmd)
    rust_median=$(echo $rust_result | awk '{print $1}')
    rust_min=$(echo $rust_result | awk '{print $2}')
    rust_max=$(echo $rust_result | awk '{print $3}')
    if [ "$c_median" -gt 0 ]; then
        rust_ratio=$(awk "BEGIN {printf \"%.2f\", $rust_median / $c_median}")
    else
        rust_ratio="N/A"
    fi
    echo "${rust_median}ms (${rust_min}-${rust_max}) = ${rust_ratio}x"
    echo "| Rust | $rust_median | $rust_min | $rust_max | ${rust_ratio}x |" >> "$RESULTS_FILE"

    # Blood
    echo -n "  Blood: "
    blood_result=$(run_bench "blood" $blood_cmd)
    blood_median=$(echo $blood_result | awk '{print $1}')
    blood_min=$(echo $blood_result | awk '{print $2}')
    blood_max=$(echo $blood_result | awk '{print $3}')
    if [ "$c_median" -gt 0 ]; then
        blood_ratio=$(awk "BEGIN {printf \"%.2f\", $blood_median / $c_median}")
    else
        blood_ratio="N/A"
    fi
    echo "${blood_median}ms (${blood_min}-${blood_max}) = ${blood_ratio}x"
    echo "| Blood | $blood_median | $blood_min | $blood_max | ${blood_ratio}x |" >> "$RESULTS_FILE"

    echo ""
}

echo ""

# N-Body (Blood runs 10000 steps internally)
run_suite "N-Body (10000 steps)" \
    "$BIN_DIR/nbody_c 10000" \
    "$BIN_DIR/nbody_rust 10000" \
    "$BIN_DIR/nbody_blood"

# Spectral-Norm (Blood runs N=10)
run_suite "Spectral-Norm (N=10)" \
    "$BIN_DIR/spectralnorm_c 10" \
    "$BIN_DIR/spectralnorm_rust 10" \
    "$BIN_DIR/spectralnorm_blood"

# Binary-Trees (Blood runs depth=5 and depth=10)
run_suite "Binary-Trees (depth=10)" \
    "$BIN_DIR/binarytrees_c 10" \
    "$BIN_DIR/binarytrees_rust 10" \
    "$BIN_DIR/binarytrees_blood"

# Fannkuch-Redux (Blood runs N=5)
run_suite "Fannkuch-Redux (N=5)" \
    "$BIN_DIR/fannkuchredux_c 5" \
    "$BIN_DIR/fannkuchredux_rust 5" \
    "$BIN_DIR/fannkuchredux_blood"

# Fasta (Blood runs smaller)
run_suite "Fasta (N=1000)" \
    "$BIN_DIR/fasta_c 1000" \
    "$BIN_DIR/fasta_rust 1000" \
    "$BIN_DIR/fasta_blood"

# Add summary to report
cat >> "$RESULTS_FILE" << 'EOF'

## Analysis

### Key Observations

1. **Startup overhead dominates at small sizes**: Blood programs include runtime initialization that's amortized over longer runs.

2. **128-bit pointer overhead**: Binary-trees shows the impact of Blood's fat pointers on allocation-heavy workloads.

3. **Compute-bound performance**: N-body and spectral-norm are compute-heavy and should show minimal overhead.

### Limitations

1. **Non-standard sizes**: Blood benchmarks use hardcoded sizes smaller than CLBG standard
2. **No CLI argument support**: Blood benchmarks can't be configured at runtime
3. **Startup overhead**: Significant at small problem sizes
4. **Missing: Large-scale benchmarks**: True performance characteristics require CLBG-standard sizes

### Recommendations

1. Add CLI argument parsing to Blood benchmarks
2. Run at CLBG-standard sizes for accurate comparison
3. Profile to understand overhead sources
EOF

echo "Results saved to: $RESULTS_FILE"
echo ""
echo "=== Summary ==="
cat "$RESULTS_FILE"
