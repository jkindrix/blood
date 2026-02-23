#!/usr/bin/env bash
#
# memprofile.sh — Memory Budget Tracker for Blood Compilers
#
# Profiles memory usage of compilation runs with three modes:
#
#   --summary (default): Peak RSS via /usr/bin/time, with --timings phase breakdown
#   --sample:            Poll /proc/PID/status during compilation, report RSS timeline
#   --massif:            Full heap profile via valgrind massif (slow but detailed)
#
# Usage:
#   ./tools/memprofile.sh <file.blood>                    # summary of both compilers
#   ./tools/memprofile.sh <file.blood> --ref-only         # reference compiler only
#   ./tools/memprofile.sh <file.blood> --test-only        # test compiler only
#   ./tools/memprofile.sh <file.blood> --sample           # RSS sampling mode
#   ./tools/memprofile.sh <file.blood> --massif           # valgrind massif mode
#   ./tools/memprofile.sh <file.blood> --compare          # side-by-side comparison
#
# Environment variables (same as difftest.sh):
#   BLOOD_REF, BLOOD_TEST, BLOOD_RUNTIME, BLOOD_RUST_RUNTIME

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

BLOOD_REF="${BLOOD_REF:-$REPO_ROOT/src/bootstrap/target/release/blood}"
BLOOD_TEST="${BLOOD_TEST:-$REPO_ROOT/src/selfhost/build/first_gen}"
export BLOOD_RUNTIME="${BLOOD_RUNTIME:-$REPO_ROOT/runtime/runtime.o}"
export BLOOD_RUST_RUNTIME="${BLOOD_RUST_RUNTIME:-$REPO_ROOT/src/bootstrap/target/release/libblood_runtime.a}"

MODE="summary"
WHICH="both"
TARGET=""

for arg in "$@"; do
    case "$arg" in
        --summary)  MODE="summary" ;;
        --sample)   MODE="sample" ;;
        --massif)   MODE="massif" ;;
        --compare)  MODE="compare" ;;
        --ref-only) WHICH="ref" ;;
        --test-only) WHICH="test" ;;
        --help|-h)
            echo "Usage: $0 <file.blood> [--summary|--sample|--massif|--compare] [--ref-only|--test-only]"
            exit 0 ;;
        -*) echo "Unknown option: $arg" >&2; exit 3 ;;
        *)  TARGET="$arg" ;;
    esac
done

if [[ -z "$TARGET" || ! -f "$TARGET" ]]; then
    echo "Usage: $0 <file.blood> [--summary|--sample|--massif|--compare]" >&2
    exit 3
fi

# ── Colors ───────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
DIM='\033[2m'
RESET='\033[0m'

BASENAME="$(basename "$TARGET" .blood)"
WORK="$(mktemp -d "/tmp/memprofile.${BASENAME}.XXXXXX")"
trap "rm -rf '$WORK'" EXIT

# ── Utility Functions ────────────────────────────────────────────────────────

format_kb() {
    local kb="$1"
    if [[ $kb -ge 1048576 ]]; then
        printf "%.1f GB" "$(echo "scale=1; $kb / 1048576" | bc)"
    elif [[ $kb -ge 1024 ]]; then
        printf "%.1f MB" "$(echo "scale=1; $kb / 1024" | bc)"
    else
        printf "%d KB" "$kb"
    fi
}

# Extract timings from compiler output
extract_timings() {
    local file="$1"
    perl -ne '
        if (/^\s+(\S.*?)\s{2,}(\d+)ms/) {
            printf "  %-24s %s ms\n", $1, $2;
        }
    ' "$file"
}

# ═══════════════════════════════════════════════════════════════════════════════
# Mode: summary — peak RSS + phase timings
# ═══════════════════════════════════════════════════════════════════════════════

run_summary() {
    local compiler="$1"
    local label="$2"
    local outfile="$WORK/${label}_output.txt"
    local timefile="$WORK/${label}_time.txt"

    echo -e "${BOLD}${CYAN}$label${RESET}"

    local cmd
    if [[ "$label" == "ref" ]]; then
        cmd=("$compiler" build "$TARGET" -o "$WORK/${label}_exe" --timings --quiet --color never)
    else
        cmd=("$compiler" build "$TARGET" -o "$WORK/${label}.ll" --timings --no-cache)
    fi

    /usr/bin/time -v "${cmd[@]}" >"$outfile" 2>"$timefile"
    local exit_code=$?

    if [[ $exit_code -ne 0 ]]; then
        echo -e "  ${RED}Compilation failed (exit $exit_code)${RESET}"
        # Still extract memory info from /usr/bin/time
    fi

    # Extract peak RSS from /usr/bin/time output
    local peak_kb
    peak_kb=$(grep "Maximum resident set size" "$timefile" | grep -o '[0-9]*')
    if [[ -n "$peak_kb" ]]; then
        echo -e "  Peak RSS:    ${BOLD}$(format_kb "$peak_kb")${RESET}"
    fi

    # Extract wall clock time
    local wall_time
    wall_time=$(grep "Elapsed (wall clock)" "$timefile" | sed 's/.*: //')
    if [[ -n "$wall_time" ]]; then
        echo -e "  Wall time:   $wall_time"
    fi

    # Extract phase timings from compiler output
    local timing_lines
    timing_lines=$(extract_timings "$outfile")
    if [[ -z "$timing_lines" ]]; then
        # Try from stderr (first_gen sends timings to stderr via status messages)
        timing_lines=$(extract_timings "$timefile")
    fi
    if [[ -n "$timing_lines" ]]; then
        echo -e "  ${DIM}Phase timings:${RESET}"
        echo "$timing_lines" | sed 's/^/    /'
    fi

    echo ""

    # Clean up executables
    rm -f "$WORK/${label}_exe" "$WORK/${label}.ll" "$WORK/${label}.o" "$WORK/${label}" 2>/dev/null
    # Clean up first_gen exe in source dir
    rm -f "$(dirname "$TARGET")/$BASENAME" 2>/dev/null
}

# ═══════════════════════════════════════════════════════════════════════════════
# Mode: sample — poll /proc/PID/status for RSS timeline
# ═══════════════════════════════════════════════════════════════════════════════

run_sample() {
    local compiler="$1"
    local label="$2"
    local outfile="$WORK/${label}_output.txt"
    local rss_log="$WORK/${label}_rss.log"

    echo -e "${BOLD}${CYAN}$label${RESET} (sampling RSS every 50ms)"

    local cmd
    if [[ "$label" == "ref" ]]; then
        cmd=("$compiler" build "$TARGET" -o "$WORK/${label}_exe" --timings --quiet --color never)
    else
        cmd=("$compiler" build "$TARGET" -o "$WORK/${label}.ll" --timings --no-cache)
    fi

    # Start the compiler in background
    "${cmd[@]}" >"$outfile" 2>&1 &
    local pid=$!

    # Sample RSS every 50ms
    local start_time
    start_time=$(date +%s%N)
    local max_rss=0

    while kill -0 "$pid" 2>/dev/null; do
        local now
        now=$(date +%s%N)
        local elapsed_ms=$(( (now - start_time) / 1000000 ))

        local rss_kb=0
        if [[ -f "/proc/$pid/status" ]]; then
            rss_kb=$(grep "^VmRSS:" "/proc/$pid/status" 2>/dev/null | awk '{print $2}')
            rss_kb="${rss_kb:-0}"
        fi

        echo "$elapsed_ms $rss_kb" >> "$rss_log"
        if [[ $rss_kb -gt $max_rss ]]; then
            max_rss=$rss_kb
        fi

        sleep 0.05
    done

    wait "$pid" 2>/dev/null
    local exit_code=$?

    if [[ $exit_code -ne 0 ]]; then
        echo -e "  ${RED}Compilation failed (exit $exit_code)${RESET}"
    fi

    echo -e "  Peak RSS:    ${BOLD}$(format_kb "$max_rss")${RESET}"

    # Show RSS timeline (first, peak, final samples)
    if [[ -f "$rss_log" && -s "$rss_log" ]]; then
        local total_samples
        total_samples=$(wc -l < "$rss_log")
        echo -e "  Samples:     $total_samples"

        # Show condensed timeline: 10 evenly-spaced samples
        echo -e "  ${DIM}RSS timeline (ms → KB):${RESET}"
        perl -e '
            my @lines;
            while (<>) {
                chomp;
                my ($ms, $kb) = split /\s+/;
                push @lines, [$ms, $kb];
            }
            my $n = scalar @lines;
            my $step = $n > 10 ? int($n / 10) : 1;
            for (my $i = 0; $i < $n; $i += $step) {
                my ($ms, $kb) = @{$lines[$i]};
                my $bar_len = int($kb / 1024);  # 1 char per MB
                $bar_len = 1 if $bar_len < 1 && $kb > 0;
                $bar_len = 60 if $bar_len > 60;
                my $bar = "#" x $bar_len;
                printf "    %6d ms  %8d KB  %s\n", $ms, $kb, $bar;
            }
            # Always show last
            if ($n > 0) {
                my ($ms, $kb) = @{$lines[-1]};
                my $bar_len = int($kb / 1024);
                $bar_len = 1 if $bar_len < 1 && $kb > 0;
                $bar_len = 60 if $bar_len > 60;
                my $bar = "#" x $bar_len;
                printf "    %6d ms  %8d KB  %s  (final)\n", $ms, $kb, $bar;
            }
        ' "$rss_log"
    fi

    # Show phase timings
    local timing_lines
    timing_lines=$(extract_timings "$outfile")
    if [[ -n "$timing_lines" ]]; then
        echo -e "  ${DIM}Phase timings:${RESET}"
        echo "$timing_lines" | sed 's/^/    /'
    fi

    echo ""

    # Clean up
    rm -f "$WORK/${label}_exe" "$WORK/${label}.ll" "$WORK/${label}.o" "$WORK/${label}" 2>/dev/null
    rm -f "$(dirname "$TARGET")/$BASENAME" 2>/dev/null
}

# ═══════════════════════════════════════════════════════════════════════════════
# Mode: massif — valgrind heap profiling
# ═══════════════════════════════════════════════════════════════════════════════

run_massif() {
    local compiler="$1"
    local label="$2"
    local outfile="$WORK/${label}_massif.out"
    local msfile="$WORK/${label}_ms_print.txt"

    echo -e "${BOLD}${CYAN}$label${RESET} (valgrind massif — this will be slow)"

    if ! command -v valgrind &>/dev/null; then
        echo -e "  ${RED}valgrind not found${RESET}"
        return 1
    fi

    local cmd
    if [[ "$label" == "ref" ]]; then
        cmd=("$compiler" build "$TARGET" -o "$WORK/${label}_exe" --quiet --color never)
    else
        cmd=("$compiler" build "$TARGET" -o "$WORK/${label}.ll" --no-cache)
    fi

    valgrind --tool=massif --massif-out-file="$outfile" \
        --pages-as-heap=yes --stacks=yes \
        "${cmd[@]}" >/dev/null 2>"$WORK/${label}_valgrind_stderr.txt"
    local exit_code=$?

    if [[ $exit_code -ne 0 ]]; then
        echo -e "  ${YELLOW}Exit code: $exit_code (valgrind overhead may cause issues)${RESET}"
    fi

    if [[ -f "$outfile" ]]; then
        ms_print "$outfile" > "$msfile" 2>/dev/null

        # Extract peak from ms_print header
        local peak_bytes
        peak_bytes=$(grep "peak" "$msfile" | head -1 | grep -o '[0-9,]*' | head -1 | tr -d ',')
        if [[ -n "$peak_bytes" ]]; then
            local peak_kb=$(( peak_bytes / 1024 ))
            echo -e "  Peak heap:   ${BOLD}$(format_kb "$peak_kb")${RESET}"
        fi

        # Show the ASCII graph
        echo -e "  ${DIM}Heap profile (see $outfile for full data):${RESET}"
        # Extract the graph lines from ms_print
        sed -n '/^    [GM]B/,/^$/p' "$msfile" | head -25 | sed 's/^/    /'
        # If no GB/MB header, try KB
        if [[ $(sed -n '/^    [GM]B/p' "$msfile" | wc -l) -eq 0 ]]; then
            sed -n '/^    KB/,/^$/p' "$msfile" | head -25 | sed 's/^/    /'
        fi

        echo -e "  ${DIM}Full massif output: $outfile${RESET}"
        echo -e "  ${DIM}View with: ms_print $outfile${RESET}"
    else
        echo -e "  ${RED}No massif output produced${RESET}"
    fi

    echo ""

    # Clean up executables but keep massif output
    rm -f "$WORK/${label}_exe" "$WORK/${label}.ll" "$WORK/${label}.o" "$WORK/${label}" 2>/dev/null
    rm -f "$(dirname "$TARGET")/$BASENAME" 2>/dev/null
}

# ═══════════════════════════════════════════════════════════════════════════════
# Mode: compare — side-by-side summary of both compilers
# ═══════════════════════════════════════════════════════════════════════════════

run_compare() {
    echo -e "${BOLD}Memory Comparison: ${CYAN}$BASENAME${RESET}"
    echo -e "${DIM}  ref:  $BLOOD_REF${RESET}"
    echo -e "${DIM}  test: $BLOOD_TEST${RESET}"
    echo ""

    # Run both with /usr/bin/time
    local ref_time="$WORK/ref_time.txt"
    local test_time="$WORK/test_time.txt"
    local ref_out="$WORK/ref_out.txt"
    local test_out="$WORK/test_out.txt"

    /usr/bin/time -v "$BLOOD_REF" build "$TARGET" -o "$WORK/ref_exe" \
        --timings --quiet --color never >"$ref_out" 2>"$ref_time" || true

    /usr/bin/time -v "$BLOOD_TEST" build "$TARGET" -o "$WORK/test.ll" \
        --timings --no-cache >"$test_out" 2>"$test_time" || true

    local ref_peak test_peak
    ref_peak=$(grep "Maximum resident set size" "$ref_time" | grep -o '[0-9]*')
    test_peak=$(grep "Maximum resident set size" "$test_time" | grep -o '[0-9]*')
    ref_peak="${ref_peak:-0}"
    test_peak="${test_peak:-0}"

    local ref_wall test_wall
    ref_wall=$(grep "Elapsed (wall clock)" "$ref_time" | sed 's/.*: //')
    test_wall=$(grep "Elapsed (wall clock)" "$test_time" | sed 's/.*: //')

    printf "  %-20s  %-18s  %-18s\n" "" "Reference" "Test"
    printf "  %-20s  %-18s  %-18s\n" "────────────────────" "──────────────────" "──────────────────"
    printf "  %-20s  %-18s  %-18s\n" "Peak RSS" "$(format_kb "$ref_peak")" "$(format_kb "$test_peak")"
    printf "  %-20s  %-18s  %-18s\n" "Wall time" "$ref_wall" "$test_wall"

    # Compute ratio
    if [[ $ref_peak -gt 0 && $test_peak -gt 0 ]]; then
        local ratio
        ratio=$(echo "scale=1; $test_peak * 100 / $ref_peak" | bc)
        echo ""
        echo -e "  Test uses ${BOLD}${ratio}%${RESET} of reference memory"
    fi

    echo ""

    # Show both timings side by side
    echo -e "  ${DIM}Phase timings:${RESET}"
    echo -e "  ${BOLD}Reference:${RESET}"
    local ref_timings
    ref_timings=$(extract_timings "$ref_out")
    if [[ -z "$ref_timings" ]]; then
        ref_timings=$(extract_timings "$ref_time")
    fi
    if [[ -n "$ref_timings" ]]; then
        echo "$ref_timings" | sed 's/^/    /'
    fi

    echo -e "  ${BOLD}Test:${RESET}"
    local test_timings
    test_timings=$(extract_timings "$test_out")
    if [[ -z "$test_timings" ]]; then
        test_timings=$(extract_timings "$test_time")
    fi
    if [[ -n "$test_timings" ]]; then
        echo "$test_timings" | sed 's/^/    /'
    fi

    # Clean up
    rm -f "$WORK/ref_exe" "$WORK/test.ll" "$WORK/test.o" "$WORK/test" 2>/dev/null
    rm -f "$(dirname "$TARGET")/$BASENAME" 2>/dev/null
}

# ═══════════════════════════════════════════════════════════════════════════════
# Main dispatch
# ═══════════════════════════════════════════════════════════════════════════════

echo -e "${BOLD}Memory Profile: ${CYAN}$BASENAME${RESET}  (mode: $MODE)"
echo ""

case "$MODE" in
    summary)
        [[ "$WHICH" != "test" ]] && run_summary "$BLOOD_REF" "ref"
        [[ "$WHICH" != "ref" ]] && run_summary "$BLOOD_TEST" "test"
        ;;
    sample)
        [[ "$WHICH" != "test" ]] && run_sample "$BLOOD_REF" "ref"
        [[ "$WHICH" != "ref" ]] && run_sample "$BLOOD_TEST" "test"
        ;;
    massif)
        [[ "$WHICH" != "test" ]] && run_massif "$BLOOD_REF" "ref"
        [[ "$WHICH" != "ref" ]] && run_massif "$BLOOD_TEST" "test"
        ;;
    compare)
        run_compare
        ;;
esac
