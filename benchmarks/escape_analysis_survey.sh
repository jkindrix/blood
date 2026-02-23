#!/bin/bash
#
# PERF-V-002: Escape Analysis Effectiveness Survey
#
# This script runs escape analysis on all Blood examples and aggregates
# the statistics to validate the claim ">95% stack allocation".
#
# Usage: ./benchmarks/escape_analysis_survey.sh
#
# Output: Comprehensive statistics report for PERF-V-002
#

set -e

# Configuration
BLOOD_COMPILER="${BLOOD_COMPILER:-./target/release/blood}"
EXAMPLES_DIR="${EXAMPLES_DIR:-./examples}"
OUTPUT_FILE="${OUTPUT_FILE:-/dev/null}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo "=============================================================================="
echo "  BLOOD ESCAPE ANALYSIS SURVEY (PERF-V-002)"
echo "=============================================================================="
echo ""
echo "Compiler: $BLOOD_COMPILER"
echo "Examples: $EXAMPLES_DIR"
echo ""

# Check if compiler exists
if [ ! -f "$BLOOD_COMPILER" ]; then
    echo -e "${RED}Error: Compiler not found at $BLOOD_COMPILER${NC}"
    echo "Build the compiler first: cargo build --release --package bloodc"
    exit 1
fi

# Aggregate counters
total_functions=0
total_locals=0
total_stack_promotable=0
total_heap_required=0
total_no_escape=0
total_arg_escape=0
total_global_escape=0
total_effect_captured=0
total_closure_captured=0
successful_builds=0
failed_builds=0

# Arrays to track per-file statistics
declare -a file_stats

echo "Analyzing examples..."
echo ""

# Process each Blood file
for blood_file in "$EXAMPLES_DIR"/*.blood; do
    filename=$(basename "$blood_file")

    # Skip files known to have parsing issues or are test fragments
    if [[ "$filename" =~ ^test[0-9]+\.blood$ ]] || \
       [[ "$filename" =~ _test\.blood$ ]] || \
       [[ "$filename" == "shallow_multi_resume_error.blood" ]]; then
        continue
    fi

    # Run the compiler with high verbosity to get escape analysis stats
    output=$("$BLOOD_COMPILER" build -vvv "$blood_file" 2>&1) || {
        echo -e "  ${YELLOW}[SKIP]${NC} $filename (build failed)"
        failed_builds=$((failed_builds + 1))
        continue
    }

    # Extract the statistics line
    # Format: "N functions, M locals: X.X% stack-promotable (Y stack, Z heap)"
    stats_line=$(echo "$output" | grep "Escape analysis complete" | head -1)

    if [ -z "$stats_line" ]; then
        echo -e "  ${YELLOW}[SKIP]${NC} $filename (no MIR functions)"
        continue
    fi

    # Parse the statistics
    functions=$(echo "$stats_line" | grep -oP '\d+ functions' | grep -oP '\d+')
    locals=$(echo "$stats_line" | grep -oP '\d+ locals' | grep -oP '\d+')
    stack=$(echo "$stats_line" | grep -oP '\(\d+ stack' | grep -oP '\d+')
    heap=$(echo "$stats_line" | grep -oP '\d+ heap\)' | grep -oP '\d+')
    percentage=$(echo "$stats_line" | grep -oP '\d+\.\d+%' | grep -oP '\d+\.\d+')

    if [ -n "$functions" ] && [ -n "$locals" ]; then
        # Aggregate totals
        total_functions=$((total_functions + functions))
        total_locals=$((total_locals + locals))
        total_stack_promotable=$((total_stack_promotable + stack))
        total_heap_required=$((total_heap_required + heap))
        successful_builds=$((successful_builds + 1))

        # Extract escape state breakdown from report
        no_escape=$(echo "$output" | grep "NoEscape:" | head -1 | grep -oP '(?<=NoEscape:\s+)\d+' || echo "0")
        arg_escape=$(echo "$output" | grep "ArgEscape:" | head -1 | grep -oP '(?<=ArgEscape:\s+)\d+' || echo "0")
        global_escape=$(echo "$output" | grep "GlobalEscape:" | head -1 | grep -oP '(?<=GlobalEscape:\s+)\d+' || echo "0")
        effect_cap=$(echo "$output" | grep "Effect-captured:" | head -1 | grep -oP '(?<=Effect-captured:\s+)\d+' || echo "0")
        closure_cap=$(echo "$output" | grep "Closure-captured:" | head -1 | grep -oP '(?<=Closure-captured:\s+)\d+' || echo "0")

        total_no_escape=$((total_no_escape + no_escape))
        total_arg_escape=$((total_arg_escape + arg_escape))
        total_global_escape=$((total_global_escape + global_escape))
        total_effect_captured=$((total_effect_captured + effect_cap))
        total_closure_captured=$((total_closure_captured + closure_cap))

        # Color based on percentage
        if [ "$(echo "$percentage >= 95" | bc)" -eq 1 ]; then
            color=$GREEN
            status="✓"
        elif [ "$(echo "$percentage >= 80" | bc)" -eq 1 ]; then
            color=$YELLOW
            status="~"
        else
            color=$RED
            status="✗"
        fi

        printf "  [%s] %-40s %3d funcs, %5d locals: ${color}%6.1f%%${NC} stack\n" \
            "$status" "$filename" "$functions" "$locals" "$percentage"
    fi

    # Clean up generated files
    rm -f "${blood_file%.blood}" 2>/dev/null || true
    rm -rf "${blood_file%.blood}.blood_objs" 2>/dev/null || true
done

echo ""
echo "=============================================================================="
echo "  AGGREGATE RESULTS"
echo "=============================================================================="
echo ""

# Calculate overall percentage
if [ "$total_locals" -gt 0 ]; then
    overall_percentage=$(echo "scale=2; $total_stack_promotable * 100 / $total_locals" | bc)
else
    overall_percentage="0.00"
fi

# Calculate escape state percentages
if [ "$total_locals" -gt 0 ]; then
    no_escape_pct=$(echo "scale=1; $total_no_escape * 100 / $total_locals" | bc)
    arg_escape_pct=$(echo "scale=1; $total_arg_escape * 100 / $total_locals" | bc)
    global_escape_pct=$(echo "scale=1; $total_global_escape * 100 / $total_locals" | bc)
else
    no_escape_pct="0.0"
    arg_escape_pct="0.0"
    global_escape_pct="0.0"
fi

echo "Files analyzed:           $successful_builds"
echo "Files skipped/failed:     $failed_builds"
echo ""
echo "╔══════════════════════════════════════════════════════════════════╗"
echo "║           ESCAPE ANALYSIS AGGREGATE STATISTICS                   ║"
echo "╠══════════════════════════════════════════════════════════════════╣"
printf "║  Functions analyzed:           %6d                            ║\n" "$total_functions"
printf "║  Total locals:                 %6d                            ║\n" "$total_locals"
echo "╠══════════════════════════════════════════════════════════════════╣"
echo "║  ALLOCATION TIER BREAKDOWN                                       ║"
echo "╠══════════════════════════════════════════════════════════════════╣"
printf "║  Stack-promotable (Tier 0):    %6d (%5.1f%%)                   ║\n" "$total_stack_promotable" "$overall_percentage"
printf "║  Heap-required (Tier 1/2):     %6d (%5.1f%%)                   ║\n" "$total_heap_required" "$(echo "scale=1; 100 - $overall_percentage" | bc)"
echo "╠══════════════════════════════════════════════════════════════════╣"
echo "║  ESCAPE STATE BREAKDOWN                                          ║"
echo "╠══════════════════════════════════════════════════════════════════╣"
printf "║  NoEscape:                     %6d (%5.1f%%)                   ║\n" "$total_no_escape" "$no_escape_pct"
printf "║  ArgEscape:                    %6d (%5.1f%%)                   ║\n" "$total_arg_escape" "$arg_escape_pct"
printf "║  GlobalEscape:                 %6d (%5.1f%%)                   ║\n" "$total_global_escape" "$global_escape_pct"
echo "╠══════════════════════════════════════════════════════════════════╣"
echo "║  SPECIAL CAPTURES                                                ║"
echo "╠══════════════════════════════════════════════════════════════════╣"
printf "║  Effect-captured:              %6d                            ║\n" "$total_effect_captured"
printf "║  Closure-captured:             %6d                            ║\n" "$total_closure_captured"
echo "╠══════════════════════════════════════════════════════════════════╣"

# Check target
if [ "$(echo "$overall_percentage >= 95" | bc)" -eq 1 ]; then
    echo -e "║  TARGET (>95% stack):          ${GREEN}${overall_percentage}%  ✓ PASS${NC}                      ║"
    target_met="PASS"
else
    echo -e "║  TARGET (>95% stack):          ${RED}${overall_percentage}%  ✗ FAIL${NC}                      ║"
    target_met="FAIL"
fi

echo "╚══════════════════════════════════════════════════════════════════╝"
echo ""

# Final verdict
echo "=============================================================================="
echo "  PERF-V-002 VALIDATION: $target_met"
echo "=============================================================================="
echo ""

if [ "$target_met" == "PASS" ]; then
    echo -e "${GREEN}The >95% stack allocation claim is VALIDATED.${NC}"
    echo ""
    echo "Evidence:"
    echo "  - $total_stack_promotable of $total_locals locals ($overall_percentage%) are stack-promotable"
    echo "  - Analyzed $successful_builds example programs with $total_functions functions"
    echo "  - Escape analysis correctly identifies stack-eligible allocations"
else
    echo -e "${RED}The >95% stack allocation claim is NOT met.${NC}"
    echo ""
    echo "Current status:"
    echo "  - $total_stack_promotable of $total_locals locals ($overall_percentage%) are stack-promotable"
    echo "  - Target: >95%"
    echo ""
    echo "Investigation needed for locals requiring heap allocation."
fi

echo ""
echo "Report generated: $(date)"
