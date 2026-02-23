#!/bin/bash
# CLBG Comparative Benchmark Runner for Blood
# This script runs the Computer Language Benchmarks Game suite
# comparing Blood against C, Rust, and Go implementations.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BLOOD_ROOT="$(dirname "$SCRIPT_DIR")"
RESULTS_DIR="$SCRIPT_DIR/clbg/results"
ITERATIONS=5  # Number of runs per benchmark for statistical validity

# Benchmark parameters (matching CLBG standard sizes)
declare -A PARAMS
PARAMS[nbody]="50000000"
PARAMS[spectralnorm]="5500"
PARAMS[binarytrees]="21"
PARAMS[fannkuchredux]="12"
PARAMS[fasta]="25000000"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo "============================================================"
echo "  Blood CLBG Benchmark Suite"
echo "  Comparing Blood vs C vs Rust"
echo "============================================================"
echo ""
echo "Configuration:"
echo "  Iterations per benchmark: $ITERATIONS"
echo "  Results directory: $RESULTS_DIR"
echo ""

# Create results directory
mkdir -p "$RESULTS_DIR"

# Function to measure time (returns milliseconds)
measure_time() {
    local cmd="$1"
    local start end elapsed
    start=$(date +%s%N)
    eval "$cmd" > /dev/null 2>&1
    end=$(date +%s%N)
    elapsed=$(( (end - start) / 1000000 ))
    echo "$elapsed"
}

# Function to run benchmark multiple times and compute statistics
run_benchmark() {
    local name="$1"
    local cmd="$2"
    local times=()

    echo -n "    Running $ITERATIONS iterations..."

    for ((i=1; i<=ITERATIONS; i++)); do
        time_ms=$(measure_time "$cmd")
        times+=($time_ms)
    done

    # Compute statistics
    local sum=0
    local min=${times[0]}
    local max=${times[0]}

    for t in "${times[@]}"; do
        sum=$((sum + t))
        if (( t < min )); then min=$t; fi
        if (( t > max )); then max=$t; fi
    done

    local mean=$((sum / ITERATIONS))

    # Compute std dev
    local sumsq=0
    for t in "${times[@]}"; do
        local diff=$((t - mean))
        sumsq=$((sumsq + diff * diff))
    done
    local variance=$((sumsq / ITERATIONS))
    local stddev=$(echo "scale=2; sqrt($variance)" | bc 2>/dev/null || echo "0")

    echo " done"
    echo "      Mean: ${mean}ms, Min: ${min}ms, Max: ${max}ms, StdDev: ${stddev}ms"

    # Return mean time
    echo "$mean"
}

# Function to compile C benchmarks
compile_c() {
    echo -e "${BLUE}Compiling C benchmarks...${NC}"
    local c_dir="$SCRIPT_DIR/clbg/c"
    local bin_dir="$SCRIPT_DIR/clbg/bin"
    mkdir -p "$bin_dir"

    for src in "$c_dir"/*.c; do
        name=$(basename "$src" .c)
        echo "  Compiling $name..."
        gcc -O3 -march=native -fomit-frame-pointer "$src" -o "$bin_dir/${name}_c" -lm
    done
    echo -e "${GREEN}C benchmarks compiled.${NC}"
    echo ""
}

# Function to compile Rust benchmarks
compile_rust() {
    echo -e "${BLUE}Compiling Rust benchmarks...${NC}"
    local rust_dir="$SCRIPT_DIR/clbg/rust"
    local bin_dir="$SCRIPT_DIR/clbg/bin"
    mkdir -p "$bin_dir"

    for src in "$rust_dir"/*.rs; do
        name=$(basename "$src" .rs)
        echo "  Compiling $name..."
        rustc -C opt-level=3 -C target-cpu=native "$src" -o "$bin_dir/${name}_rust"
    done
    echo -e "${GREEN}Rust benchmarks compiled.${NC}"
    echo ""
}

# Function to compile Blood benchmarks
compile_blood() {
    echo -e "${BLUE}Compiling Blood benchmarks...${NC}"
    local bin_dir="$SCRIPT_DIR/clbg/bin"
    mkdir -p "$bin_dir"

    local blood_compiler="$BLOOD_ROOT/target/release/blood"

    if [[ ! -f "$blood_compiler" ]]; then
        echo "  Building Blood compiler first..."
        (cd "$BLOOD_ROOT" && cargo build --release -p bloodc)
    fi

    echo "  Compiling binary_tree_benchmark..."
    "$blood_compiler" build "$BLOOD_ROOT/examples/binary_tree_benchmark.blood" 2>/dev/null
    cp "$BLOOD_ROOT/examples/binary_tree_benchmark" "$bin_dir/binarytrees_blood" 2>/dev/null || true

    echo "  Compiling nbody_benchmark..."
    "$blood_compiler" build "$BLOOD_ROOT/examples/nbody_benchmark.blood" 2>/dev/null
    cp "$BLOOD_ROOT/examples/nbody_benchmark" "$bin_dir/nbody_blood" 2>/dev/null || true

    echo "  Compiling spectral_norm_benchmark..."
    "$blood_compiler" build "$BLOOD_ROOT/examples/spectral_norm_benchmark.blood" 2>/dev/null
    cp "$BLOOD_ROOT/examples/spectral_norm_benchmark" "$bin_dir/spectralnorm_blood" 2>/dev/null || true

    echo "  Compiling fannkuch_benchmark..."
    "$blood_compiler" build "$BLOOD_ROOT/examples/fannkuch_benchmark.blood" 2>/dev/null
    cp "$BLOOD_ROOT/examples/fannkuch_benchmark" "$bin_dir/fannkuchredux_blood" 2>/dev/null || true

    echo "  Compiling fasta_benchmark..."
    "$blood_compiler" build "$BLOOD_ROOT/examples/fasta_benchmark.blood" 2>/dev/null
    cp "$BLOOD_ROOT/examples/fasta_benchmark" "$bin_dir/fasta_blood" 2>/dev/null || true

    echo -e "${GREEN}Blood benchmarks compiled.${NC}"
    echo ""
}

# Run benchmarks
run_all_benchmarks() {
    local bin_dir="$SCRIPT_DIR/clbg/bin"
    local timestamp=$(date +%Y%m%d_%H%M%S)
    local results_file="$RESULTS_DIR/benchmark_results_$timestamp.csv"

    echo "benchmark,language,param,mean_ms,min_ms,max_ms,iterations" > "$results_file"

    for bench in nbody spectralnorm binarytrees fannkuchredux fasta; do
        param="${PARAMS[$bench]}"
        echo ""
        echo -e "${YELLOW}=== $bench (param=$param) ===${NC}"

        # C
        if [[ -f "$bin_dir/${bench}_c" ]]; then
            echo -e "  ${BLUE}C:${NC}"
            c_time=$(run_benchmark "$bench-c" "$bin_dir/${bench}_c $param")
        else
            echo -e "  ${RED}C: not found${NC}"
            c_time="N/A"
        fi

        # Rust
        if [[ -f "$bin_dir/${bench}_rust" ]]; then
            echo -e "  ${BLUE}Rust:${NC}"
            rust_time=$(run_benchmark "$bench-rust" "$bin_dir/${bench}_rust $param")
        else
            echo -e "  ${RED}Rust: not found${NC}"
            rust_time="N/A"
        fi

        # Blood
        if [[ -f "$bin_dir/${bench}_blood" ]]; then
            echo -e "  ${BLUE}Blood:${NC}"
            blood_time=$(run_benchmark "$bench-blood" "$bin_dir/${bench}_blood $param" 2>/dev/null || echo "N/A")
        else
            echo -e "  ${RED}Blood: not found${NC}"
            blood_time="N/A"
        fi

        # Log results
        if [[ "$c_time" != "N/A" ]]; then
            echo "$bench,C,$param,$c_time,,,$ITERATIONS" >> "$results_file"
        fi
        if [[ "$rust_time" != "N/A" ]]; then
            echo "$bench,Rust,$param,$rust_time,,,$ITERATIONS" >> "$results_file"
        fi
        if [[ "$blood_time" != "N/A" ]]; then
            echo "$bench,Blood,$param,$blood_time,,,$ITERATIONS" >> "$results_file"
        fi
    done

    echo ""
    echo -e "${GREEN}Results saved to: $results_file${NC}"
}

# Generate summary report
generate_report() {
    local results_file="$1"
    local report_file="$RESULTS_DIR/BENCHMARK_SUMMARY.md"

    echo "# CLBG Benchmark Results" > "$report_file"
    echo "" >> "$report_file"
    echo "**Generated**: $(date)" >> "$report_file"
    echo "**Iterations per benchmark**: $ITERATIONS" >> "$report_file"
    echo "" >> "$report_file"
    echo "## Results" >> "$report_file"
    echo "" >> "$report_file"
    echo "| Benchmark | C (ms) | Rust (ms) | Blood (ms) | Blood/C | Blood/Rust |" >> "$report_file"
    echo "|-----------|--------|-----------|------------|---------|------------|" >> "$report_file"

    echo "" >> "$report_file"
    echo "See $results_file for raw data." >> "$report_file"

    echo -e "${GREEN}Summary report generated: $report_file${NC}"
}

# Main execution
main() {
    case "${1:-all}" in
        compile-c)
            compile_c
            ;;
        compile-rust)
            compile_rust
            ;;
        compile-blood)
            compile_blood
            ;;
        compile)
            compile_c
            compile_rust
            compile_blood
            ;;
        run)
            run_all_benchmarks
            ;;
        all)
            compile_c
            compile_rust
            compile_blood
            run_all_benchmarks
            ;;
        *)
            echo "Usage: $0 [compile|compile-c|compile-rust|compile-blood|run|all]"
            exit 1
            ;;
    esac
}

main "$@"
