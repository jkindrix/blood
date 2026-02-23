#!/usr/bin/env bash
#
# filecheck-audit.sh — FileCheck Test Coverage Audit
#
# Inventories existing FileCheck tests, identifies which codegen patterns
# they cover, scans the self-hosted compiler for patterns exercised during
# self-compilation, and reports coverage gaps.
#
# Usage:
#   ./tools/filecheck-audit.sh                    # Full audit
#   ./tools/filecheck-audit.sh --tests-only       # Just list existing tests
#   ./tools/filecheck-audit.sh --gaps-only        # Just show coverage gaps
#   ./tools/filecheck-audit.sh --recommend        # Show recommended new tests
#
# Environment variables:
#   COMPILER_DIR — directory with self-hosted compiler (default: <repo>/src/selfhost)
#   TEST_DIR     — directory with FileCheck tests (default: $COMPILER_DIR/tests)

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

COMPILER_DIR="${COMPILER_DIR:-$REPO_ROOT/src/selfhost}"
TEST_DIR="${TEST_DIR:-$COMPILER_DIR/tests}"
GROUND_TRUTH="${GROUND_TRUTH:-$REPO_ROOT/tests/ground-truth}"

MODE="full"
for arg in "$@"; do
    case "$arg" in
        --tests-only) MODE="tests" ;;
        --gaps-only)  MODE="gaps" ;;
        --recommend)  MODE="recommend" ;;
        --help|-h)
            echo "Usage: $0 [--tests-only|--gaps-only|--recommend]"
            exit 0 ;;
        -*) echo "Unknown option: $arg" >&2; exit 3 ;;
    esac
done

# ── Colors ───────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
DIM='\033[2m'
RESET='\033[0m'

# ═══════════════════════════════════════════════════════════════════════════════
# Part 1: Inventory Existing FileCheck Tests
# ═══════════════════════════════════════════════════════════════════════════════

inventory_tests() {
    echo -e "${BOLD}Existing FileCheck Tests${RESET}"
    echo ""

    local total=0
    local check_count=0

    if [[ ! -d "$TEST_DIR" ]]; then
        echo -e "  ${RED}Test directory not found: $TEST_DIR${RESET}"
        return
    fi

    for check_src in "$TEST_DIR"/check_*.blood; do
        [[ -f "$check_src" ]] || continue
        total=$((total + 1))

        local name
        name="$(basename "$check_src" .blood)"
        local lines
        lines=$(wc -l < "$check_src")
        local checks
        checks=$(grep -c '// CHECK' "$check_src" 2>/dev/null || echo 0)
        check_count=$((check_count + checks))

        # Extract what patterns this test covers
        local patterns=""
        patterns=$(grep '// CHECK' "$check_src" | perl -ne '
            if (/define\s/) { $p{define}++; }
            if (/declare\s/) { $p{declare}++; }
            if (/call\s/) { $p{call}++; }
            if (/alloca\s/) { $p{alloca}++; }
            if (/store\s/) { $p{store}++; }
            if (/load\s/) { $p{load}++; }
            if (/br\s/) { $p{branch}++; }
            if (/icmp\s/) { $p{icmp}++; }
            if (/phi\s/) { $p{phi}++; }
            if (/getelementptr/) { $p{gep}++; }
            if (/switch\s/) { $p{switch}++; }
            if (/ret\s/) { $p{ret}++; }
            END { print join(", ", sort keys %p) . "\n"; }
        ')

        printf "  %-35s  %3d lines  %3d checks  [%s]\n" "$name" "$lines" "$checks" "$patterns"
    done

    echo ""
    echo -e "  ${BOLD}Total: $total tests, $check_count CHECK directives${RESET}"
}

# ═══════════════════════════════════════════════════════════════════════════════
# Part 2: Scan Compiler Source for Codegen Patterns
# ═══════════════════════════════════════════════════════════════════════════════

scan_compiler_patterns() {
    echo ""
    echo -e "${BOLD}Codegen Patterns in Self-Hosted Compiler${RESET}"
    echo ""

    if [[ ! -d "$COMPILER_DIR" ]]; then
        echo -e "  ${RED}Compiler directory not found: $COMPILER_DIR${RESET}"
        return
    fi

    # Scan codegen files for LLVM IR emission patterns
    echo -e "  ${CYAN}IR Emission Patterns (codegen*.blood):${RESET}"

    # Count occurrences of LLVM instruction types emitted
    local codegen_files="$COMPILER_DIR/codegen*.blood"
    for pattern_name in \
        "alloca" "store" "load" "getelementptr" "call" "br " "ret " \
        "icmp" "fcmp" "phi" "switch" "select" "bitcast" "trunc" "zext" "sext" \
        "ptrtoint" "inttoptr" "extractvalue" "insertvalue" \
        "add " "sub " "mul " "sdiv" "udiv" "srem" "urem" \
        "and " "or " "xor " "shl " "ashr" "lshr" \
        "fadd" "fsub" "fmul" "fdiv" \
        "unreachable" "invoke" "landingpad" \
    ; do
        local count=0
        for f in $codegen_files; do
            [[ -f "$f" ]] || continue
            local c
            c=$(grep -c "\"$pattern_name" "$f" 2>/dev/null || echo 0)
            count=$((count + c))
        done
        if [[ $count -gt 0 ]]; then
            printf "    %-20s %4d occurrences\n" "$pattern_name" "$count"
        fi
    done

    # Scan for specific codegen features
    echo ""
    echo -e "  ${CYAN}Feature Patterns:${RESET}"

    local features=(
        "struct construction:Struct.*\{|aggregate"
        "enum discriminant:discriminant|variant_index|tag"
        "match/switch:switchInt|switch_targets|SwitchInt"
        "function call:emit_call|Call.*->|call_fn"
        "method dispatch:self\\.method|vtable|dispatch"
        "string operations:string_new|string_push|str_to_string|String::new"
        "vec operations:vec_new|vec_push|vec_len|Vec::new"
        "hashmap operations:hashmap|HashMap"
        "option handling:Option|Some|None|is_some|is_none"
        "result handling:Result|Ok|Err|is_ok|is_err"
        "box/heap alloc:Box::new|box_new|alloc_simple"
        "reference/deref:Ref|Deref|borrow|&mut|&self"
        "loop codegen:loop|while|for.*in|goto.*bb"
        "closure/fn ptr:closure|fn_ptr|function_pointer"
        "trait dispatch:trait|impl.*for|dyn"
        "generic instantiation:generic|monomorphize|instantiate"
        "format/print:format!|println|print_int|eprintln"
        "panic/abort:panic|abort|unreachable"
        "region/memory:region|alloc|free|realloc"
        "effect handler:effect|handler|perform|resume"
    )

    for entry in "${features[@]}"; do
        local label="${entry%%:*}"
        local pattern="${entry#*:}"
        local count=0

        for f in "$COMPILER_DIR"/*.blood; do
            [[ -f "$f" ]] || continue
            local c
            c=$(grep -cE "$pattern" "$f" 2>/dev/null || echo 0)
            count=$((count + c))
        done

        local indicator="  "
        if [[ $count -gt 50 ]]; then
            indicator="${GREEN}H${RESET}"
        elif [[ $count -gt 10 ]]; then
            indicator="${YELLOW}M${RESET}"
        elif [[ $count -gt 0 ]]; then
            indicator="${DIM}L${RESET}"
        else
            indicator="${RED}-${RESET}"
        fi

        printf "    %b %-25s %4d uses\n" "$indicator" "$label" "$count"
    done
}

# ═══════════════════════════════════════════════════════════════════════════════
# Part 3: Ground-Truth Test Coverage
# ═══════════════════════════════════════════════════════════════════════════════

scan_ground_truth() {
    echo ""
    echo -e "${BOLD}Ground-Truth Test Feature Coverage${RESET}"
    echo ""

    if [[ ! -d "$GROUND_TRUTH" ]]; then
        echo -e "  ${RED}Ground-truth directory not found: $GROUND_TRUTH${RESET}"
        return
    fi

    local total=0
    local by_tier=""

    # Count tests by tier
    for tier in t00 t01 t02 t03 t04 t05 t06; do
        local count
        count=$(ls "$GROUND_TRUTH"/${tier}_*.blood 2>/dev/null | wc -l)
        total=$((total + count))
        if [[ $count -gt 0 ]]; then
            printf "  %-6s %3d tests\n" "$tier:" "$count"
        fi
    done

    echo -e "  ${BOLD}Total: $total ground-truth tests${RESET}"

    # Scan for feature coverage in ground-truth
    echo ""
    echo -e "  ${CYAN}Feature coverage in ground-truth tests:${RESET}"

    local gt_features=(
        "struct:struct "
        "enum:enum "
        "trait:trait "
        "impl:impl "
        "generic:<[A-Z]>"
        "closure:|.*|"
        "match:match "
        "if/else:if.*{"
        "while:while "
        "for:for.*in"
        "fn ptr:fn("
        "option:Option"
        "result:Result"
        "box:Box"
        "vec:Vec"
        "hashmap:HashMap"
        "string:String"
        "effect:effect "
        "handler:handler "
        "perform:perform "
        "resume:resume"
        "region:region"
        "format:format!"
        "panic:panic!"
    )

    for entry in "${gt_features[@]}"; do
        local label="${entry%%:*}"
        local pattern="${entry#*:}"
        local count
        count=$(grep -rlE "$pattern" "$GROUND_TRUTH"/*.blood 2>/dev/null | wc -l)
        local pct=0
        if [[ $total -gt 0 ]]; then
            pct=$((count * 100 / total))
        fi

        local bar_len=$((pct / 5))
        local bar=""
        for ((i=0; i<bar_len; i++)); do bar="${bar}#"; done

        printf "    %-12s %3d tests (%2d%%) %s\n" "$label" "$count" "$pct" "$bar"
    done
}

# ═══════════════════════════════════════════════════════════════════════════════
# Part 4: Coverage Gaps and Recommendations
# ═══════════════════════════════════════════════════════════════════════════════

report_gaps() {
    echo ""
    echo -e "${BOLD}Coverage Gaps & Recommended FileCheck Tests${RESET}"
    echo ""

    # What the existing FileCheck tests cover
    local covered=""
    for check_src in "$TEST_DIR"/check_*.blood; do
        [[ -f "$check_src" ]] || continue
        local name
        name="$(basename "$check_src" .blood)"
        covered="$covered $name"
    done

    # Define recommended tests based on compiler codegen patterns
    # Format: "name|description|priority"
    local recommendations=(
        "check_codegen_struct|Struct construction, field access, GEP patterns|HIGH"
        "check_codegen_enum|Enum discriminant, variant construction, match/switch|HIGH"
        "check_codegen_call|Function calls, ABI, return values|HIGH"
        "check_codegen_loop|While loops, for loops, break/continue, basic block structure|HIGH"
        "check_codegen_match|Pattern matching, switch targets, nested patterns|HIGH"
        "check_codegen_option|Option<T> construction, is_some/is_none, unwrap|MEDIUM"
        "check_codegen_vec|Vec operations, indexing, push, len, data pointer|MEDIUM"
        "check_codegen_hashmap|HashMap construction, insert, get, contains|MEDIUM"
        "check_codegen_generic|Generic instantiation, monomorphization|MEDIUM"
        "check_codegen_trait|Trait method dispatch, default methods|MEDIUM"
        "check_codegen_ref|Reference creation, deref, borrow patterns|MEDIUM"
        "check_codegen_cast|Integer casts, pointer casts, truncation|LOW"
        "check_codegen_arithmetic|Binary ops, comparison, overflow|LOW"
        "check_codegen_string|String new, push_str, as_str, format|LOW"
        "check_codegen_region|Region alloc/free, scope management|LOW"
    )

    local existing_count=0
    local gap_count=0

    for rec in "${recommendations[@]}"; do
        local name="${rec%%|*}"
        local rest="${rec#*|}"
        local desc="${rest%%|*}"
        local priority="${rest##*|}"

        if echo "$covered" | grep -q "$name"; then
            printf "  ${GREEN}EXISTS${RESET}  %-30s %s\n" "$name" "$desc"
            existing_count=$((existing_count + 1))
        else
            local color="$YELLOW"
            [[ "$priority" == "HIGH" ]] && color="$RED"
            [[ "$priority" == "LOW" ]] && color="$DIM"
            printf "  ${color}%-6s${RESET}  %-30s %s\n" "$priority" "$name" "$desc"
            gap_count=$((gap_count + 1))
        fi
    done

    echo ""
    echo -e "  ${BOLD}Coverage: $existing_count/${#recommendations[@]} recommended tests exist${RESET}"
    echo -e "  ${BOLD}Gaps: $gap_count tests recommended${RESET}"

    # Prioritized action items
    echo ""
    echo -e "${BOLD}Priority Action Items:${RESET}"
    echo ""

    local action_num=0
    for rec in "${recommendations[@]}"; do
        local name="${rec%%|*}"
        local rest="${rec#*|}"
        local desc="${rest%%|*}"
        local priority="${rest##*|}"

        if ! echo "$covered" | grep -q "$name"; then
            if [[ "$priority" == "HIGH" ]]; then
                action_num=$((action_num + 1))
                echo -e "  ${RED}$action_num.${RESET} Create ${BOLD}$name.blood${RESET}"
                echo -e "     $desc"
                echo ""
            fi
        fi
    done

    if [[ $action_num -eq 0 ]]; then
        echo -e "  ${GREEN}No high-priority gaps!${RESET}"
    fi
}

# ═══════════════════════════════════════════════════════════════════════════════
# Main
# ═══════════════════════════════════════════════════════════════════════════════

echo -e "${BOLD}FileCheck Test Coverage Audit${RESET}"
echo -e "${DIM}  compiler:     $COMPILER_DIR${RESET}"
echo -e "${DIM}  tests:        $TEST_DIR${RESET}"
echo -e "${DIM}  ground-truth: $GROUND_TRUTH${RESET}"

case "$MODE" in
    full)
        inventory_tests
        scan_compiler_patterns
        scan_ground_truth
        report_gaps
        ;;
    tests)
        inventory_tests
        ;;
    gaps)
        inventory_tests
        report_gaps
        ;;
    recommend)
        report_gaps
        ;;
esac
