#!/bin/bash
# CLBG Benchmark Runner
# Runs all benchmarks and captures results

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="$SCRIPT_DIR/bin"
RESULTS_DIR="$SCRIPT_DIR/results"

mkdir -p "$RESULTS_DIR"

TIMESTAMP=$(date +%Y%m%d_%H%M%S)
RESULT_FILE="$RESULTS_DIR/benchmark_$TIMESTAMP.txt"

echo "CLBG Benchmark Results" | tee "$RESULT_FILE"
echo "======================" | tee -a "$RESULT_FILE"
echo "Date: $(date)" | tee -a "$RESULT_FILE"
echo "Host: $(uname -a)" | tee -a "$RESULT_FILE"
echo "" | tee -a "$RESULT_FILE"

run_benchmark() {
    local name="$1"
    local blood_cmd="$2"
    local c_cmd="$3"
    local runs="${4:-3}"

    echo "=== $name ===" | tee -a "$RESULT_FILE"

    echo "Blood (best of $runs):" | tee -a "$RESULT_FILE"
    local blood_best=999999
    for ((i=1; i<=runs; i++)); do
        time=$( { /usr/bin/time -f "%e" $blood_cmd > /dev/null; } 2>&1 )
        echo "  Run $i: ${time}s" | tee -a "$RESULT_FILE"
        if (( $(echo "$time < $blood_best" | bc -l) )); then
            blood_best=$time
        fi
    done
    echo "  Best: ${blood_best}s" | tee -a "$RESULT_FILE"

    echo "C (best of $runs):" | tee -a "$RESULT_FILE"
    local c_best=999999
    for ((i=1; i<=runs; i++)); do
        time=$( { /usr/bin/time -f "%e" $c_cmd > /dev/null; } 2>&1 )
        echo "  Run $i: ${time}s" | tee -a "$RESULT_FILE"
        if (( $(echo "$time < $c_best" | bc -l) )); then
            c_best=$time
        fi
    done
    echo "  Best: ${c_best}s" | tee -a "$RESULT_FILE"

    # Calculate difference
    local diff=$(echo "scale=2; ($c_best - $blood_best) / $c_best * 100" | bc -l)
    echo "  Difference: Blood is ${diff}% faster (positive=faster, negative=slower)" | tee -a "$RESULT_FILE"
    echo "" | tee -a "$RESULT_FILE"
}

cd "$BIN_DIR"

run_benchmark "N-Body (N=50,000,000)" \
    "./nbody_blood" \
    "./nbody_c 50000000"

run_benchmark "Fannkuch-Redux (N=12)" \
    "./fannkuchredux_blood" \
    "./fannkuchredux_c_fixed"

run_benchmark "Binary-Trees (depth=21)" \
    "./binarytrees_blood 21" \
    "./binarytrees_c 21"

run_benchmark "Spectral-Norm (N=5500)" \
    "./spectralnorm_blood 5500" \
    "./spectralnorm_c 5500"

echo "Results saved to: $RESULT_FILE"

# Update latest symlink
ln -sf "benchmark_$TIMESTAMP.txt" "$RESULTS_DIR/latest.txt"
