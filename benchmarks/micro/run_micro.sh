#!/bin/bash
# Blood Micro-Benchmark Runner
# Compiles and runs all Blood micro-benchmarks, compares against spec targets.
# Usage: ./run_micro.sh [--release] [--bench <name>]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
BLOOD="${BLOOD_REF:-$REPO_ROOT/src/bootstrap/target/release/blood}"

MODE="debug"
FILTER=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --release) MODE="release"; shift ;;
        --bench) FILTER="$2"; shift 2 ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

BUILD_FLAGS=""
if [[ "$MODE" == "release" ]]; then
    BUILD_FLAGS="--release"
fi

# Spec targets (nanoseconds)
declare -A TARGETS=(
    [baseline]="0"
    [region_alloc]="50"
    [persistent_alloc]="200"
    [effect_handler_install]="100"
    [effect_dispatch]="50"
    [static_dispatch]="0"
    [trait_dispatch]="0"
)

PASS_COUNT=0
WARN_COUNT=0
FAIL_COUNT=0
SKIP_COUNT=0
RESULTS=""

compile_bench() {
    local name="$1"
    local src="$SCRIPT_DIR/${name}.blood"
    if [[ ! -f "$src" ]]; then
        echo "  SKIP: $src not found"
        return 1
    fi
    "$BLOOD" build "$src" $BUILD_FLAGS 2>&1 | tail -1
}

run_bench() {
    local name="$1"
    local bin="$SCRIPT_DIR/build/$MODE/${name}"
    if [[ ! -x "$bin" ]]; then
        echo "  SKIP: $bin not found"
        return 1
    fi
    # CPU pin if taskset is available
    if command -v taskset &>/dev/null; then
        taskset -c 0 "$bin"
    else
        "$bin"
    fi
}

verdict() {
    local name="$1"
    local measured="$2"
    local target="${TARGETS[$name]:-}"

    if [[ -z "$target" ]]; then
        echo "INFO"
        return
    fi
    if [[ "$target" == "0" ]]; then
        # Zero-cost target: anything <=2ns is PASS
        if [[ "$measured" -le 2 ]]; then
            echo "PASS"
        elif [[ "$measured" -le 10 ]]; then
            echo "WARN"
        else
            echo "FAIL"
        fi
    else
        if [[ "$measured" -le "$target" ]]; then
            echo "PASS"
        elif [[ "$measured" -le $((target * 2)) ]]; then
            echo "WARN"
        else
            echo "FAIL"
        fi
    fi
}

echo "============================================"
echo "Blood Micro-Benchmark Suite"
echo "Mode: $MODE"
echo "Compiler: $BLOOD"
echo "============================================"
echo ""

BENCHMARKS=(
    bench_baseline
    bench_region_alloc
    bench_region_dealloc
    bench_persistent_alloc
    bench_pointer_overhead
    bench_effect_handler_install
    bench_effect_dispatch
    bench_effect_state_loop
    bench_generator_sum
    bench_static_dispatch
    bench_trait_dispatch
    bench_enum_dispatch
)

for bench in "${BENCHMARKS[@]}"; do
    # Apply filter if specified
    if [[ -n "$FILTER" && "$bench" != *"$FILTER"* ]]; then
        continue
    fi

    short="${bench#bench_}"
    echo "--- $short ---"

    # Compile
    echo -n "  Compiling... "
    if ! compile_bench "$bench"; then
        SKIP_COUNT=$((SKIP_COUNT + 1))
        echo ""
        continue
    fi

    # Run
    echo -n "  Running...   "
    output=$(run_bench "$bench" 2>&1) || {
        echo "CRASH"
        FAIL_COUNT=$((FAIL_COUNT + 1))
        RESULTS+="| $short | CRASH | - | - |\n"
        continue
    }
    echo "done"

    # Parse ns_per_op (or ns_per_yield, ns_per_iter, ns_per_dispatch)
    ns_per_op=$(echo "$output" | grep -oP '(?<=ns_per_op=)\d+' | head -1 || true)
    if [[ -z "$ns_per_op" ]]; then
        ns_per_op=$(echo "$output" | grep -oP '(?<=ns_per_yield=)\d+' | head -1 || true)
    fi
    if [[ -z "$ns_per_op" ]]; then
        ns_per_op=$(echo "$output" | grep -oP '(?<=ns_per_iter=)\d+' | head -1 || true)
    fi
    if [[ -z "$ns_per_op" ]]; then
        ns_per_op=$(echo "$output" | grep -oP '(?<=ns_per_dispatch=)\d+' | head -1 || true)
    fi
    if [[ -z "$ns_per_op" ]]; then
        ns_per_op=$(echo "$output" | grep -oP '(?<=overhead_pct=)\d+' | head -1 || true)
        if [[ -n "$ns_per_op" ]]; then
            ns_per_op="${ns_per_op}%"
        fi
    fi

    median=$(echo "$output" | grep -oP '(?<=median_total_ns=)\d+' | head -1 || true)
    target="${TARGETS[$short]:-}"

    if [[ -z "$ns_per_op" || "$ns_per_op" == *"%" ]]; then
        if [[ "$ns_per_op" == *"%" ]]; then
            pct_val="${ns_per_op%\%}"
            v="INFO"
            echo "  Result: ${ns_per_op} overhead"
            RESULTS+="| $short | $ns_per_op | - | $v |\n"
        else
            echo "  Result: (no per-op metric)"
            RESULTS+="| $short | - | - | INFO |\n"
        fi
    else
        v=$(verdict "$short" "$ns_per_op")
        target_str="${target:-n/a}"
        echo "  Result: ${ns_per_op}ns/op (target: ${target_str}ns) [$v]"
        RESULTS+="| $short | ${ns_per_op}ns | ${target_str}ns | $v |\n"

        case "$v" in
            PASS) PASS_COUNT=$((PASS_COUNT + 1)) ;;
            WARN) WARN_COUNT=$((WARN_COUNT + 1)) ;;
            FAIL) FAIL_COUNT=$((FAIL_COUNT + 1)) ;;
        esac
    fi
    echo ""
done

echo "============================================"
echo "Summary: $PASS_COUNT PASS, $WARN_COUNT WARN, $FAIL_COUNT FAIL, $SKIP_COUNT SKIP"
echo "============================================"
echo ""
echo "| Benchmark | Measured | Target | Verdict |"
echo "|-----------|----------|--------|---------|"
echo -e "$RESULTS"
