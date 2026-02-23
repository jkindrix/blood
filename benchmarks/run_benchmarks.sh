#!/bin/bash
#
# Blood Benchmark Runner
# ======================
#
# Runs CLBG benchmarks following the methodology in METHODOLOGY.md
#
# Usage:
#   ./run_benchmarks.sh [options]
#
# Options:
#   --quick     Run fewer iterations (3 instead of 12)
#   --no-isolation  Skip isolation checks (for development)
#   --help      Show this help
#
# Requirements:
#   - hyperfine (cargo install hyperfine)
#   - Blood compiler built (cargo build --release)
#   - GCC for C reference builds
#

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"
BLOOD="$REPO_ROOT/target/release/blood"
HYPERFINE="${HOME}/.cargo/bin/hyperfine"
FPDIFF="$SCRIPT_DIR/tools/fpdiff.py"

# Benchmark configuration
RUNS=12
WARMUP=1
QUICK_RUNS=3

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Parse arguments
QUICK=false
SKIP_ISOLATION=false
while [[ $# -gt 0 ]]; do
    case $1 in
        --quick)
            QUICK=true
            RUNS=$QUICK_RUNS
            shift
            ;;
        --no-isolation)
            SKIP_ISOLATION=true
            shift
            ;;
        --help)
            head -25 "$0" | tail -20
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# =============================================================================
# Helper Functions
# =============================================================================

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

check_tool() {
    local tool=$1
    local path=$2
    if [ ! -x "$path" ]; then
        log_error "$tool not found at $path"
        return 1
    fi
    return 0
}

# =============================================================================
# System Information
# =============================================================================

gather_system_info() {
    echo "## System Configuration"
    echo ""
    echo "### Hardware"
    echo "- CPU: $(lscpu | grep 'Model name' | sed 's/Model name:[[:space:]]*//')"
    echo "- Cores: $(nproc) ($(lscpu | grep 'Core(s) per socket' | awk '{print $NF}') physical)"
    echo "- Memory: $(free -h | awk '/^Mem:/ {print $2}')"
    echo ""
    echo "### Software"
    echo "- OS: $(uname -sr)"
    echo "- Kernel: $(uname -r)"
    echo "- Blood: $(cd "$REPO_ROOT" && git rev-parse --short HEAD 2>/dev/null || echo 'unknown')"
    echo "- GCC: $(gcc --version | head -1)"
    echo ""
}

# =============================================================================
# Isolation Checks
# =============================================================================

check_isolation() {
    if $SKIP_ISOLATION; then
        log_warn "Skipping isolation checks (--no-isolation)"
        return 0
    fi

    echo "### Isolation Status"
    echo ""

    # Check CPU governor
    local governor=$(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor 2>/dev/null || echo "unknown")
    if [ "$governor" = "performance" ]; then
        echo "- CPU governor: $governor (OK)"
    else
        echo "- CPU governor: $governor (WARN: should be 'performance')"
        log_warn "CPU governor is '$governor', not 'performance'. Results may have higher variance."
    fi

    # Check turbo boost (Intel)
    local turbo_intel=$(cat /sys/devices/system/cpu/intel_pstate/no_turbo 2>/dev/null || echo "N/A")
    if [ "$turbo_intel" != "N/A" ]; then
        if [ "$turbo_intel" = "1" ]; then
            echo "- Turbo boost: disabled (OK)"
        else
            echo "- Turbo boost: enabled (WARN: may increase variance)"
        fi
    fi

    # Check turbo boost (AMD)
    local turbo_amd=$(cat /sys/devices/system/cpu/cpufreq/boost 2>/dev/null || echo "N/A")
    if [ "$turbo_amd" != "N/A" ]; then
        if [ "$turbo_amd" = "0" ]; then
            echo "- Turbo boost: disabled (OK)"
        else
            echo "- Turbo boost: enabled (WARN: may increase variance)"
        fi
    fi

    echo ""
}

# =============================================================================
# Benchmark Compilation
# =============================================================================

compile_benchmarks() {
    local bin_dir="$SCRIPT_DIR/clbg/bin"
    mkdir -p "$bin_dir"

    log_info "Compiling benchmarks..."

    # Compile Blood n-body
    if [ -f "$SCRIPT_DIR/clbg/blood/nbody.blood" ]; then
        log_info "Compiling Blood n-body..."
        "$BLOOD" build "$SCRIPT_DIR/clbg/blood/nbody.blood" --release -q 2>/dev/null || true
        mv "$SCRIPT_DIR/clbg/blood/nbody" "$bin_dir/nbody_blood" 2>/dev/null || true
    fi

    # Compile C n-body
    if [ -f "$SCRIPT_DIR/clbg/c/nbody.c" ]; then
        log_info "Compiling C n-body (gcc -O3)..."
        gcc -O3 -march=native "$SCRIPT_DIR/clbg/c/nbody.c" -o "$bin_dir/nbody_c" -lm
    fi

    log_info "Compilation complete."
    echo ""
}

# =============================================================================
# Output Validation
# =============================================================================

validate_output() {
    local name=$1
    local blood_bin=$2
    local c_bin=$3
    local c_args=$4

    log_info "Validating $name output..."

    # Run both and capture output
    "$blood_bin" > /tmp/blood_output.txt 2>&1
    "$c_bin" $c_args > /tmp/c_output.txt 2>&1

    # Compare with tolerance (Blood uses %g format, C uses %.9f)
    if python3 "$FPDIFF" -abserr 1e-5 -relerr 1e-5 /tmp/c_output.txt /tmp/blood_output.txt >/dev/null 2>&1; then
        echo "- Output validation: PASS (matches C within tolerance)"
        return 0
    else
        echo "- Output validation: FAIL"
        echo "  Expected (C):"
        cat /tmp/c_output.txt | sed 's/^/    /'
        echo "  Actual (Blood):"
        cat /tmp/blood_output.txt | sed 's/^/    /'
        return 1
    fi
}

# =============================================================================
# Run Benchmark
# =============================================================================

run_benchmark() {
    local name=$1
    local blood_bin=$2
    local c_bin=$3
    local c_args=$4

    echo "### $name"
    echo ""

    # Validate output first
    if ! validate_output "$name" "$blood_bin" "$c_bin" "$c_args"; then
        log_error "Output validation failed for $name. Skipping timing."
        echo ""
        return 1
    fi

    echo ""
    echo "#### Timing Results"
    echo ""

    # Run hyperfine
    "$HYPERFINE" \
        --warmup $WARMUP \
        --runs $RUNS \
        --export-json /tmp/bench_results.json \
        --style basic \
        "$blood_bin" \
        "$c_bin $c_args" \
        2>&1 | grep -E "^(Benchmark|Time|Range)"

    echo ""

    # Extract and format results
    python3 << EOF
import json
import sys

with open('/tmp/bench_results.json') as f:
    data = json.load(f)

results = data['results']
blood = results[0]
c = results[1]

blood_mean = blood['mean']
blood_stddev = blood['stddev']
blood_min = blood['min']
blood_max = blood['max']

c_mean = c['mean']
c_stddev = c['stddev']
c_min = c['min']
c_max = c['max']

ratio = blood_mean / c_mean
cv_blood = (blood_stddev / blood_mean) * 100
cv_c = (c_stddev / c_mean) * 100

print("| Implementation | Mean (s) | Std Dev | Min | Max | CV% |")
print("|----------------|----------|---------|-----|-----|-----|")
print(f"| Blood | {blood_mean:.3f} | {blood_stddev:.3f} | {blood_min:.3f} | {blood_max:.3f} | {cv_blood:.1f}% |")
print(f"| C (gcc -O3) | {c_mean:.3f} | {c_stddev:.3f} | {c_min:.3f} | {c_max:.3f} | {cv_c:.1f}% |")
print()
print(f"**Blood/C ratio: {ratio:.2f}x**")
print()

if cv_blood > 20 or cv_c > 20:
    print("**WARNING: CV% > 20% indicates high variance. Results may be unreliable.**")
    print()
EOF

    echo ""
}

# =============================================================================
# Main
# =============================================================================

main() {
    echo "# Blood Benchmark Report"
    echo ""
    echo "**Generated**: $(date)"
    echo "**Runs per benchmark**: $RUNS (warmup: $WARMUP)"
    echo ""

    # Check prerequisites
    check_tool "Blood compiler" "$BLOOD" || exit 1
    check_tool "hyperfine" "$HYPERFINE" || {
        log_error "hyperfine not found. Install with: cargo install hyperfine"
        exit 1
    }

    # System info
    gather_system_info

    # Isolation checks
    check_isolation

    # Compile benchmarks
    compile_benchmarks

    # Run benchmarks
    echo "## Benchmark Results"
    echo ""

    local bin_dir="$SCRIPT_DIR/clbg/bin"

    # N-Body (50M iterations)
    if [ -x "$bin_dir/nbody_blood" ] && [ -x "$bin_dir/nbody_c" ]; then
        run_benchmark "N-Body (50M iterations)" \
            "$bin_dir/nbody_blood" \
            "$bin_dir/nbody_c" \
            "50000000"
    else
        log_warn "N-Body binaries not found, skipping."
    fi

    echo "## Known Limitations"
    echo ""
    echo "1. **Output format**: Blood uses \`%g\` format (6 significant digits), C uses \`%.9f\` (9 decimal places)."
    echo "   Values are mathematically equivalent within tolerance."
    echo ""
    echo "2. **Structural difference**: Blood n-body uses unrolled function calls, C uses nested loops."
    echo "   Algorithms are equivalent but code structure differs."
    echo ""
    echo "3. **No CLI arguments**: Blood benchmarks use hardcoded iteration counts."
    echo ""

    echo "## Reproduction"
    echo ""
    echo "\`\`\`bash"
    echo "cd $REPO_ROOT"
    echo "cargo build --release"
    echo "./benchmarks/run_benchmarks.sh"
    echo "\`\`\`"
    echo ""

    log_info "Benchmark complete."
}

main "$@"
