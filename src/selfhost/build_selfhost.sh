#!/bin/bash
# build_selfhost.sh — Blood self-hosting build and test driver
#
# Usage: ./build_selfhost.sh [command] [args] [-q|--quiet]
# No arguments shows status. Run --help for full command list.
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$DIR"

REPO_ROOT="$(cd "$DIR/../.." && pwd)"
BUILD_DIR="$DIR/build"
mkdir -p "$BUILD_DIR"

# Paths (configurable via environment)
SEED_COMPILER="${SEED_COMPILER:-$REPO_ROOT/bootstrap/seed}"
BLOOD_RUST="${BLOOD_RUST:-$REPO_ROOT/src/bootstrap/target/release/blood}"
RUNTIME_A="${RUNTIME_A:-$REPO_ROOT/runtime/blood-runtime/build/debug/libblood_runtime_blood.a}"
RUNTIME_A_RUST="${RUNTIME_A_RUST:-$REPO_ROOT/src/bootstrap/target/release/libblood_runtime.a}"
GOLDEN_TESTS="${GOLDEN_TESTS:-$REPO_ROOT/tests/golden}"
BLOOD_TESTS="${BLOOD_TESTS:-$REPO_ROOT/tests/blood-test}"
STDLIB_PATH="${STDLIB_PATH:-$REPO_ROOT/stdlib}"

# LLVM toolchain detection (shared helper exports LLC/CLANG/OPT/FILECHECK/LLVM_AS/
# LLVM_EXTRACT/LLVM_LINK). Probes versioned binaries and respects env overrides.
# shellcheck source=./_llvm_tools.sh
. "$DIR/_llvm_tools.sh"

export BLOOD_RUST_RUNTIME="${RUNTIME_A}"
export BLOOD_BUILD_DIR="${BUILD_DIR}"
export BLOOD_CACHE="${BUILD_DIR}/.blood-cache"

# ── Output helpers ──────────────────────────────────────────────────────────

step()  { printf "\n\033[1;34m==> [%s] %s\033[0m\n" "$(date +%H:%M:%S)" "$1"; }
ok()    { printf "  \033[1;32m✓\033[0m %s\n" "$1"; }
fail()  { printf "  \033[1;31m✗\033[0m %s\n" "$1"; }
warn()  { printf "  \033[1;33m!\033[0m %s\n" "$1"; }
die()   { printf "\033[1;31mERROR:\033[0m %s\n" "$1" >&2; exit 1; }

elapsed_since() {
    local start="$1"
    local diff=$(( $(date +%s) - start ))
    local mins=$((diff / 60)) secs=$((diff % 60))
    if [ "$mins" -gt 0 ]; then printf "%dm%02ds" "$mins" "$secs"
    else printf "%ds" "$secs"; fi
}

log_metric() {
    local stage="$1" size="$2" wall_secs="$3"
    local ts ir_lines obj_bytes
    ts=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
    ir_lines=0
    obj_bytes=0
    [ -f "$BUILD_DIR/${stage}.ll" ] && ir_lines=$(wc -l < "$BUILD_DIR/${stage}.ll")
    [ -d "$BUILD_DIR/obj" ] && obj_bytes=$(du -sb "$BUILD_DIR/obj" 2>/dev/null | cut -f1 || echo 0)

    # Parse sub-phase timings from the build log (if available).
    # These are printed by the compiler's --timings flag.
    local parse_ms=0 hir_ms=0 typeck_ms=0 codegen_ms=0 llc_ms=0 split_ms=0
    local par_workers=0 par_wall_ms=0 fn_count=0 peak_rss_kb=0
    if [ -n "${LOG_FILE:-}" ] && [ -f "$LOG_FILE" ]; then
        parse_ms=$(grep '^ *Parse ' "$LOG_FILE" | tail -1 | sed 's/.*Parse *//; s/,//g; s/ms.*//' || true)
        hir_ms=$(grep '^ *HIR lowering' "$LOG_FILE" | tail -1 | sed 's/.*HIR lowering *//; s/,//g; s/ms.*//' || true)
        typeck_ms=$(grep '^ *Type checking' "$LOG_FILE" | tail -1 | sed 's/.*Type checking *//; s/,//g; s/ms.*//' || true)
        codegen_ms=$(grep '^ *Codegen ' "$LOG_FILE" | tail -1 | sed 's/.*Codegen *//; s/,//g; s/ms.*//' || true)
        # Match both old ("llc-18 (per-module)") and new ("llc (per-module)") compiler output formats.
        llc_ms=$(grep -E 'llc(-[0-9]+)? \(per-module\)' "$LOG_FILE" | tail -1 | sed -E 's/.*llc(-[0-9]+)? \(per-module\) *//; s/,//g; s/ms.*//' || true)
        split_ms=$(grep 'Module split:' "$LOG_FILE" | tail -1 | sed 's/.*in //; s/ms.*//' || true)
        par_workers=$(grep 'parallel codegen:' "$LOG_FILE" | tail -1 | sed 's/.*parallel codegen: //; s/ workers.*//' || true)
        par_wall_ms=$(grep 'parallel codegen done:' "$LOG_FILE" | tail -1 | sed 's/.*done: //; s/ms.*//' || true)
        fn_count=$(grep 'worklist:' "$LOG_FILE" | tail -1 | sed 's/.*worklist://; s/].*//' || true)
        peak_rss_kb=$(grep 'peak_rss_kb=' "$LOG_FILE" | tail -1 | sed 's/.*peak_rss_kb=//' || true)
    fi
    : "${parse_ms:=0}" "${hir_ms:=0}" "${typeck_ms:=0}" "${codegen_ms:=0}"
    : "${llc_ms:=0}" "${split_ms:=0}" "${par_workers:=0}" "${par_wall_ms:=0}"
    : "${fn_count:=0}" "${peak_rss_kb:=0}"

    printf '{"ts":"%s","stage":"%s","size":%s,"wall_secs":%s,"ir_lines":%s,"obj_bytes":%s,"parse_ms":%s,"hir_ms":%s,"typeck_ms":%s,"codegen_ms":%s,"llc_ms":%s,"split_ms":%s,"par_workers":%s,"par_wall_ms":%s,"fn_count":%s,"peak_rss_kb":%s}\n' \
        "$ts" "$stage" "$size" "$wall_secs" "$ir_lines" "$obj_bytes" \
        "$parse_ms" "$hir_ms" "$typeck_ms" "$codegen_ms" "$llc_ms" "$split_ms" \
        "$par_workers" "$par_wall_ms" "$fn_count" "$peak_rss_kb" \
        >> "$DIR/.logs/metrics.jsonl"
    check_build_time_regression "$stage" "$wall_secs"
}

# Build-time regression alarm.
#
# Baseline: 2026-04-08 post-perf-fixes. A clean first_gen/second_gen
# build should complete in ~360s on this hardware (20-core machine).
# Previous regressions went unnoticed for weeks because nothing
# alerted on gradual creep — the build climbed from ~192s (2026-03-20,
# per MEMORY.md) to ~660s (2026-04-06) to ~755s (2026-04-08) over 19
# days, unnoticed until the user's patience broke.
#
# Thresholds are deliberately loose (WARN at +40%, FAIL at +100%) so
# day-to-day noise doesn't cause false alarms, but any real regression
# is caught within 1-2 commits. Adjust BUILD_TIME_BASELINE as the
# build legitimately gets faster from future optimizations.
#
# Set BLOOD_NO_PERF_ALARM=1 to silence (e.g., on slower hardware).
check_build_time_regression() {
    local stage="$1" wall_secs="$2"
    [ -n "${BLOOD_NO_PERF_ALARM:-}" ] && return 0
    local baseline warn_threshold fail_threshold
    case "$stage" in
        first_gen|second_gen|third_gen)
            # 2026-04-09 perf session dropped clean first_gen from 354 s
            # ~117s via parallel codegen (4 workers) + parallel llc-18.
            # Reduced from 167s by parallel codegen pass2, parallel per-module
            # llc-18, and module splitter hardening.
            baseline=130
            warn_threshold=182   # baseline × 1.4
            fail_threshold=260   # baseline × 2.0
            ;;
        *) return 0 ;;
    esac
    if [ "$wall_secs" -gt "$fail_threshold" ]; then
        printf "\n\033[1;31m╭─ BUILD TIME REGRESSION (FAIL) ────────────────────────────────╮\033[0m\n"
        printf "\033[1;31m│\033[0m %-8s took \033[1;31m%ds\033[0m — more than 2× the %ds baseline.            \033[1;31m│\033[0m\n" \
            "$stage" "$wall_secs" "$baseline"
        printf "\033[1;31m│\033[0m This is a serious regression. Do not ship without fixing.     \033[1;31m│\033[0m\n"
        printf "\033[1;31m│\033[0m Check the [typeck phases], [codegen pass2], [mir_lower],      \033[1;31m│\033[0m\n"
        printf "\033[1;31m│\033[0m and [codegen fn] lines in this build log to identify which   \033[1;31m│\033[0m\n"
        printf "\033[1;31m│\033[0m sub-phase regressed. Compare against recent metrics.jsonl.    \033[1;31m│\033[0m\n"
        printf "\033[1;31m│\033[0m Set BLOOD_NO_PERF_ALARM=1 to silence (temporarily).           \033[1;31m│\033[0m\n"
        printf "\033[1;31m╰───────────────────────────────────────────────────────────────╯\033[0m\n\n"
    elif [ "$wall_secs" -gt "$warn_threshold" ]; then
        printf "\n\033[1;33m╭─ BUILD TIME REGRESSION WARNING ───────────────────────────────╮\033[0m\n"
        printf "\033[1;33m│\033[0m %-8s took \033[1;33m%ds\033[0m — 1.4× the %ds baseline (threshold: %ds). \033[1;33m│\033[0m\n" \
            "$stage" "$wall_secs" "$baseline" "$warn_threshold"
        printf "\033[1;33m│\033[0m Check the sub-phase instrumentation in this log to see what  \033[1;33m│\033[0m\n"
        printf "\033[1;33m│\033[0m regressed. Previous regressions went unnoticed for weeks of   \033[1;33m│\033[0m\n"
        printf "\033[1;33m│\033[0m silent creep. Investigate before it gets worse.               \033[1;33m│\033[0m\n"
        printf "\033[1;33m╰───────────────────────────────────────────────────────────────╯\033[0m\n\n"
    fi
}

# Per-phase regression alarm. Parses sub-phase timers from the build log
# and warns if any individual phase exceeds its baseline × 1.5.
# This catches regressions that are masked in the total wall time
# (e.g., codegen regresses 10s but HIR improves 10s → net zero).
check_sub_phase_regression() {
    local log_file="$1"
    [ -n "${BLOOD_NO_PERF_ALARM:-}" ] && return 0
    [ ! -f "$log_file" ] && return 0

    # Parse sub-phase millisecond values from the compiler's output.
    # Format: "  Phase Name         N,NNNms" or "  Phase Name         Nms"
    local parse_ms hir_ms typeck_ms codegen_ms llc_ms
    parse_ms=$(grep '^ *Parse ' "$log_file" | tail -1 | sed 's/.*Parse *//; s/,//g; s/ms.*//')
    hir_ms=$(grep '^ *HIR lowering' "$log_file" | tail -1 | sed 's/.*HIR lowering *//; s/,//g; s/ms.*//')
    typeck_ms=$(grep '^ *Type checking' "$log_file" | tail -1 | sed 's/.*Type checking *//; s/,//g; s/ms.*//')
    codegen_ms=$(grep '^ *Codegen ' "$log_file" | tail -1 | sed 's/.*Codegen *//; s/,//g; s/ms.*//')
    # Match both old ("llc-18 (per-module)") and new ("llc (per-module)") compiler output formats.
    llc_ms=$(grep -E 'llc(-[0-9]+)? \(per-module\)' "$log_file" | tail -1 | sed -E 's/.*llc(-[0-9]+)? \(per-module\) *//; s/,//g; s/ms.*//')

    # Per-phase baselines (milliseconds) and thresholds (baseline × 1.5).
    # These match the 2026-04-10 steady state with parallel codegen + parallel llc.
    local warned=0
    _check_phase() {
        local name="$1" actual="$2" baseline="$3"
        [ -z "$actual" ] && return 0
        local threshold=$(( baseline * 3 / 2 ))
        if [ "$actual" -gt "$threshold" ]; then
            if [ "$warned" -eq 0 ]; then
                printf "\n\033[1;33m╭─ SUB-PHASE REGRESSION ────────────────────────────────────────╮\033[0m\n"
                warned=1
            fi
            printf "\033[1;33m│\033[0m  %-16s %6dms  (baseline: %dms, threshold: %dms) \033[1;33m│\033[0m\n" \
                "$name" "$actual" "$baseline" "$threshold"
        fi
    }

    _check_phase "Parse"         "$parse_ms"   1000
    _check_phase "HIR lowering"  "$hir_ms"     22000
    _check_phase "Type checking" "$typeck_ms"  22000
    _check_phase "Codegen"       "$codegen_ms" 70000
    _check_phase "llc"           "$llc_ms"     3000

    if [ "$warned" -ne 0 ]; then
        printf "\033[1;33m│\033[0m  Check [hir phases], [codegen pass2], [check_body] in log.   \033[1;33m│\033[0m\n"
        printf "\033[1;33m╰───────────────────────────────────────────────────────────────╯\033[0m\n\n"
    fi
}

decode_exit() {
    local code="$1"
    if [ "$code" -eq 0 ]; then echo "success"
    elif [ "$code" -le 128 ]; then echo "exit $code"
    else
        local sig=$((code - 128))
        case "$sig" in
            6)  echo "SIGABRT (abort/assert)" ;;
            8)  echo "SIGFPE (arithmetic error)" ;;
            9)  echo "SIGKILL (killed)" ;;
            11) echo "SIGSEGV (segmentation fault)" ;;
            13) echo "SIGPIPE (broken pipe)" ;;
            15) echo "SIGTERM (terminated)" ;;
            *)  echo "signal $sig (exit $code)" ;;
        esac
    fi
}

# ── Parse global flags ──────────────────────────────────────────────────────

QUIET=""
FORCE=""
clean_args=()
for arg in "$@"; do
    case "$arg" in
        -q|--quiet) QUIET="--quiet" ;;
        --force) FORCE="--force" ;;
        *) clean_args+=("$arg") ;;
    esac
done
set -- "${clean_args[@]+"${clean_args[@]}"}"

# ── Log setup (skip for lightweight commands) ───────────────────────────────

case "${1:-status}" in
    status|run|diff|install|clean|clean-all|--help|-h) ;;
    *)
        LOG_DIR="$DIR/.logs"
        mkdir -p "$LOG_DIR"
        LOG_FILE="$LOG_DIR/build_$(date +%Y%m%d_%H%M%S).log"

        # Log rotation: keep last 20. `ls` errors (exit 2) when the glob
        # has no matches, which silently kills the script under
        # set -euo pipefail on a fresh runner; use `find` instead.
        log_count=$(find "$LOG_DIR" -maxdepth 1 -name 'build_*.log' -type f 2>/dev/null | wc -l)
        if [ "$log_count" -gt 20 ]; then
            ls -1t "$LOG_DIR"/build_*.log | tail -n +21 | xargs rm -f
        fi

        exec > >(tee -a "$LOG_FILE") 2>&1
        printf "=== Build started: %s ===\n" "$(date '+%Y-%m-%d %H:%M:%S')"
        printf "=== Log: %s ===\n" "$LOG_FILE"
        ;;
esac

# ── Compiler name resolution ──────────────────────────────────────────────

resolve_compiler() {
    case "${1:-first_gen}" in
        bootstrap|blood-rust) echo "$BLOOD_RUST" ;;
        first_gen)            echo "$BUILD_DIR/first_gen" ;;
        second_gen)           echo "$BUILD_DIR/second_gen" ;;
        third_gen)            echo "$BUILD_DIR/third_gen" ;;
        *)                    echo "$1" ;;  # treat as path
    esac
}

# ── Operational helpers ─────────────────────────────────────────────────────

# Workaround: first_gen has had region-related memory issues during large
# compilations (self-compile).  Wrapping in `script -qc` provides a pseudo-TTY.
# The TypeInterner region corruption (commit 0a32199) and expanded_source
# UAF (type-check error path) have been fixed.  This wrapper may no longer
# be necessary but is kept as a safety net until verified.
run_with_pty() {
    local rc=0
    script -qec "$*" /dev/null || rc=$?
    return "$rc"
}

copy_runtime() {
    local bootstrap_rt="$REPO_ROOT/bootstrap/libblood_runtime_blood.a"

    if [ -f "$RUNTIME_A" ]; then
        cp -f "$RUNTIME_A" "$BUILD_DIR/libblood_runtime.a"
    elif [ -f "$bootstrap_rt" ]; then
        warn "RUNTIME_A not found at $RUNTIME_A — using bootstrap copy"
        cp -f "$bootstrap_rt" "$BUILD_DIR/libblood_runtime.a"
    else
        die "No runtime archive found. Need either:\n  $RUNTIME_A (run: build blood_runtime)\n  $bootstrap_rt (committed with bootstrap/seed)"
    fi
}

# Verify the runtime archive matches the bootstrap version.
# Different compiler generations produce different archives; using the wrong
# one causes stale-reference crashes during self-compilation.
check_runtime_consistency() {
    local bootstrap_rt="$REPO_ROOT/bootstrap/libblood_runtime_blood.a"
    [ -f "$bootstrap_rt" ] || return 0  # No bootstrap runtime to compare against

    local build_rt="$BUILD_DIR/libblood_runtime.a"
    [ -f "$build_rt" ] || return 0  # Will be copied by copy_runtime

    local bh bld_h
    bh=$(md5sum "$bootstrap_rt" | cut -d' ' -f1)
    bld_h=$(md5sum "$build_rt" | cut -d' ' -f1)
    if [ "$bh" != "$bld_h" ]; then
        warn "Runtime archive mismatch: build/ differs from bootstrap/"
        warn "  bootstrap: $bh"
        warn "  build:     $bld_h"
        warn "Fixing: copying bootstrap runtime to build/"
        cp -f "$bootstrap_rt" "$build_rt"
        # Also update the default RUNTIME_A location if it exists and differs
        if [ -f "$RUNTIME_A" ]; then
            local ra_h
            ra_h=$(md5sum "$RUNTIME_A" | cut -d' ' -f1)
            if [ "$ra_h" != "$bh" ]; then
                cp -f "$bootstrap_rt" "$RUNTIME_A"
            fi
        fi
    fi
}

# Clear all compiler IR caches across the repo and the centralized ~/.blood/cache.
clear_all_caches() {
    rm -rf "$BUILD_DIR/.content_hashes" "$BUILD_DIR/obj/.hashes" "$BUILD_DIR/.blood-cache" 2>/dev/null
    rm -rf "${HOME}/.blood/cache" 2>/dev/null
    find "$REPO_ROOT" -name ".blood-cache" -type d -not -path '*/.claude/*' -exec rm -rf {} + 2>/dev/null || true
}

check_zombies() {
    local procs
    procs=$(pgrep -af "(first_gen|second_gen|third_gen) build" 2>/dev/null | grep -v "$$" || true)
    if [ -n "$procs" ]; then
        warn "Existing compiler processes detected:"
        printf "%s\n" "$procs" | sed 's/^/    /'
    fi
}

check_staleness() {
    local bin="$1"
    [ -f "$bin" ] || return 0
    local newer
    newer=$(find "$DIR" -maxdepth 1 -name '*.blood' -newer "$bin" -print -quit 2>/dev/null)
    if [ -n "$newer" ]; then
        warn "$(basename "$bin") may be stale (source files are newer)"
    fi
}

# Check how many commits have landed since the seed was last gated.
# Warns if the seed is significantly behind HEAD.
check_seed_staleness() {
    local meta="$REPO_ROOT/bootstrap/seed.meta"
    local threshold=15
    if [ ! -f "$meta" ]; then
        warn "No seed.meta found — seed provenance unknown. Run: ./build_selfhost.sh gate"
        return 0
    fi
    local seed_commit
    seed_commit=$(grep '^commit=' "$meta" 2>/dev/null | cut -d= -f2)
    if [ -z "$seed_commit" ]; then
        return 0
    fi
    # Count commits since seed was gated
    if git rev-parse --verify "$seed_commit" >/dev/null 2>&1; then
        local distance
        distance=$(git rev-list --count "${seed_commit}..HEAD" 2>/dev/null || echo 0)
        if [ "$distance" -gt "$threshold" ]; then
            warn "Seed is $distance commits behind HEAD. Consider: ./build_selfhost.sh gate"
        fi
    fi
}

# ── Build stages ────────────────────────────────────────────────────────────

do_build_cargo() {
    step "Building blood-rust (cargo build --release)"
    local start_ts
    start_ts=$(date +%s)
    (cd "$REPO_ROOT/src/bootstrap" && cargo build --release)
    ok "blood-rust built in $(elapsed_since "$start_ts")"
}

do_build_first_gen() {
    local flags="${1:-}"

    check_runtime_consistency

    # Find a compiler to bootstrap with: seed binary (preferred) or blood-rust (legacy fallback)
    local bootstrap_compiler=""
    if [ -f "$SEED_COMPILER" ]; then
        bootstrap_compiler="$SEED_COMPILER"
    elif [ -f "$BLOOD_RUST" ]; then
        bootstrap_compiler="$BLOOD_RUST"
        warn "Using blood-rust (legacy). Consider: ./build_selfhost.sh build second_gen && cp build/second_gen ../../bootstrap/seed"
    else
        die "No bootstrap compiler found. Need either:\n  bootstrap/seed (run: build second_gen && cp build/second_gen ../../bootstrap/seed)\n  blood-rust at $BLOOD_RUST (run: cd src/bootstrap && cargo build --release)"
    fi

    check_seed_staleness

    # Clear all caches — cached IR is compiler-version-specific.
    clear_all_caches

    # Pass --no-cache: populating the per-function content hash cache on a
    # clean build is ~232s of pure I/O waste (2551 functions × ~91ms each
    # for hash computation + file writes). The cache gets wiped above before
    # every first_gen rebuild because it's compiler-version-specific, so it
    # can never be reused across builds anyway.
    step "Building first_gen with $(basename "$bootstrap_compiler")"
    local start_ts rc=0
    start_ts=$(date +%s)
    $bootstrap_compiler build main.blood --timings --no-cache --split-modules -o "$BUILD_DIR/first_gen.ll" $flags ${BLOOD_EXTRA_COMPILER_FLAGS:-} || rc=$?
    if [ "$rc" -ne 0 ]; then
        fail "Build failed ($(basename "$bootstrap_compiler")): $(decode_exit $rc)"
        return 1
    fi
    local fg_size fg_wall
    fg_size=$(wc -c < "$BUILD_DIR/first_gen")
    fg_wall=$(($(date +%s) - start_ts))
    ok "first_gen built ($fg_size bytes) in $(elapsed_since "$start_ts")"
    log_metric "first_gen" "$fg_size" "$fg_wall"
    check_sub_phase_regression "$LOG_FILE"
    copy_runtime
}

# do_relink_first_gen: fast path that re-runs only the clang-18 link step
# using existing build/obj/*.o files and the current runtime archive.
#
# Use when only the runtime (not selfhost source) changed. Saves ~11 minutes
# per iteration (skips seed → selfhost IR → per-module llc).
#
# Does NOT detect source staleness. If you modify src/selfhost/*.blood or
# the seed compiler, use `build first_gen` (without --relink) instead.
# Safer-than-silent: this function refuses to run if build/obj is missing
# or suspiciously empty.
do_relink_first_gen() {
    local obj_dir="$BUILD_DIR/obj"
    [ -d "$obj_dir" ] || die "--relink: $obj_dir does not exist. Run full 'build first_gen' first."
    local obj_count
    obj_count=$(find "$obj_dir" -maxdepth 1 -name '*.o' | wc -l)
    [ "$obj_count" -ge 60 ] || die "--relink: only $obj_count object files in $obj_dir (expected 60+). Run full 'build first_gen' first."

    [ -f "$RUNTIME_A" ] || die "--relink: runtime archive not found at $RUNTIME_A. Run 'build blood_runtime' first."

    # Warn if any selfhost source file is newer than the newest .o file.
    # This is advisory — the user asked for --relink and we honor it, but we
    # surface the staleness so they can abort if they forgot to rebuild.
    local newest_o newest_src
    newest_o=$(find "$obj_dir" -maxdepth 1 -name '*.o' -printf '%T@\n' | sort -rn | head -1)
    newest_src=$(find "$DIR" -maxdepth 1 -name '*.blood' -printf '%T@\n' | sort -rn | head -1)
    if [ -n "$newest_o" ] && [ -n "$newest_src" ]; then
        # bash can compare float-as-string lexicographically for these timestamps
        if [ "$(echo "$newest_src > $newest_o" | bc 2>/dev/null || echo 0)" = "1" ]; then
            warn "--relink: selfhost source files are newer than object files."
            warn "           The relinked first_gen will NOT reflect those source changes."
            warn "           Run './build_selfhost.sh build first_gen' (without --relink) for a full rebuild."
        fi
    fi

    step "Relinking first_gen (fast path, runtime-only changes)"
    local start_ts rc=0
    start_ts=$(date +%s)

    # Match the exact link command from src/selfhost/main.blood:524-539
    local bin_path="$BUILD_DIR/first_gen"
    local clang_args=("$CLANG")
    # shellcheck disable=SC2206  # intentional glob expansion for .o files
    clang_args+=($obj_dir/*.o)
    clang_args+=("$RUNTIME_A" -Wl,-z,muldefs -lm -ldl -lpthread -pie -o "$bin_path")
    "${clang_args[@]}" || rc=$?
    if [ "$rc" -ne 0 ]; then
        fail "$CLANG linking failed (exit $rc)"
        return 1
    fi

    local fg_size fg_wall
    fg_size=$(wc -c < "$bin_path")
    fg_wall=$(($(date +%s) - start_ts))
    ok "first_gen relinked ($fg_size bytes) in $(elapsed_since "$start_ts")"
    log_metric "first_gen_relink" "$fg_size" "$fg_wall"
    copy_runtime
}

do_build_second_gen() {
    [ -f "$BUILD_DIR/first_gen" ] || die "first_gen not found. Run: ./build_selfhost.sh build first_gen"

    check_runtime_consistency

    # Guard: verify first_gen is linked against the same runtime we'll use for
    # second_gen. If first_gen was built by a different seed or manually copied,
    # the runtime linkage may be wrong, causing stale-reference crashes.
    # The build-dir runtime was set by copy_runtime at the end of build first_gen.
    local build_rt="$BUILD_DIR/libblood_runtime.a"
    local bootstrap_rt="$REPO_ROOT/bootstrap/libblood_runtime_blood.a"
    if [ -f "$build_rt" ] && [ -f "$bootstrap_rt" ]; then
        local bh bld_h
        bh=$(md5sum "$bootstrap_rt" | cut -d' ' -f1)
        bld_h=$(md5sum "$build_rt" | cut -d' ' -f1)
        if [ "$bh" != "$bld_h" ]; then
            warn "Runtime still mismatched after check_runtime_consistency."
            warn "This usually means first_gen was built with a different runtime."
            warn "Forcing bootstrap runtime for consistency."
            cp -f "$bootstrap_rt" "$build_rt"
        fi
    fi

    clear_all_caches

    # Warn if source files are newer than first_gen — second_gen won't include
    # those changes since first_gen is the compiler doing the work.
    check_staleness "$BUILD_DIR/first_gen"

    step "Self-compiling (first_gen → second_gen)"
    local start_ts rc=0
    start_ts=$(date +%s)
    run_with_pty "$BUILD_DIR/first_gen" build main.blood --timings --no-cache --split-modules -o "$BUILD_DIR/second_gen.ll" ${BLOOD_EXTRA_COMPILER_FLAGS:-} || rc=$?
    local wall_time
    wall_time=$(elapsed_since "$start_ts")

    if [ "$rc" -ne 0 ]; then
        fail "first_gen failed: $(decode_exit $rc) (wall time: $wall_time)"
        if [ "$rc" -gt 128 ]; then
            printf "  \033[1;31mCrash detected!\033[0m Signal: %s\n" "$(decode_exit $rc)"
            printf "  Check log: %s\n" "${LOG_FILE:-<no log>}"
        fi
        return 1
    fi

    # IR sanity checks
    if [ -f "$BUILD_DIR/second_gen.ll" ]; then
        local ll_defines ll_declares
        ll_defines=$(grep -c '^define ' "$BUILD_DIR/second_gen.ll" || true)
        ll_declares=$(grep -c '^declare ' "$BUILD_DIR/second_gen.ll" || true)
        printf "  IR: %s lines, %s bytes, %d defines, %d declares\n" \
            "$(wc -l < "$BUILD_DIR/second_gen.ll")" "$(wc -c < "$BUILD_DIR/second_gen.ll")" \
            "$ll_defines" "$ll_declares"
        if [ "$ll_defines" -lt 100 ]; then
            warn "Suspiciously few function definitions ($ll_defines)"
        fi
    fi

    local sg_size sg_wall
    sg_size=$(wc -c < "$BUILD_DIR/second_gen")
    sg_wall=$(($(date +%s) - start_ts))
    ok "second_gen built ($sg_size bytes) in $wall_time"
    log_metric "second_gen" "$sg_size" "$sg_wall"
    check_sub_phase_regression "$LOG_FILE"
}

do_build_third_gen() {
    [ -f "$BUILD_DIR/second_gen" ] || die "second_gen not found. Run: ./build_selfhost.sh build second_gen"

    check_runtime_consistency
    clear_all_caches

    # Pass --no-cache: see comment in do_build_second_gen above. Same rationale.
    step "Bootstrap (second_gen → third_gen)"
    local start_ts rc=0
    start_ts=$(date +%s)
    run_with_pty "$BUILD_DIR/second_gen" build main.blood --timings --no-cache --split-modules -o "$BUILD_DIR/third_gen.ll" ${BLOOD_EXTRA_COMPILER_FLAGS:-} || rc=$?
    local wall_time
    wall_time=$(elapsed_since "$start_ts")

    if [ "$rc" -ne 0 ]; then
        fail "second_gen build failed: $(decode_exit $rc) (wall time: $wall_time)"
        return 1
    fi

    local tg_size tg_wall
    tg_size=$(wc -c < "$BUILD_DIR/third_gen")
    tg_wall=$(($(date +%s) - start_ts))
    ok "third_gen built ($tg_size bytes) in $wall_time"
    log_metric "third_gen" "$tg_size" "$tg_wall"
    check_sub_phase_regression "$LOG_FILE"

    step "Comparing second_gen vs third_gen"
    local hash2 hash3
    hash2=$(md5sum "$BUILD_DIR/second_gen" | cut -d' ' -f1)
    hash3=$(md5sum "$BUILD_DIR/third_gen" | cut -d' ' -f1)
    printf "  second_gen: %s (%s bytes)\n" "$hash2" "$(wc -c < "$BUILD_DIR/second_gen")"
    printf "  third_gen:  %s (%s bytes)\n" "$hash3" "$(wc -c < "$BUILD_DIR/third_gen")"

    if [ "$hash2" = "$hash3" ]; then
        ok "Byte-identical — bootstrap verified"
    else
        fail "NOT byte-identical — bootstrap FAILED"
        return 1
    fi
}

do_build_runtime() {
    # runtime.o is no longer needed — the compiler emits main() in IR.
    # This target is kept for backward compat but is now a no-op.
    ok "runtime.o no longer needed (compiler emits main() in IR)"
}

do_build_libmprompt() {
    local mp_dir="$REPO_ROOT/vendor/libmprompt"
    local mp_src="$mp_dir/src/mprompt"
    local mp_build="$mp_dir/build"
    [ -d "$mp_src" ] || die "libmprompt source not found at $mp_src"
    command -v "$CLANG" >/dev/null || die "$CLANG required for libmprompt build"

    mkdir -p "$mp_build"

    step "Building libmprompt"

    # Unity build: main.c includes mprompt.c, gstack.c, util.c
    "$CLANG" -c -O2 -fPIC \
        -I"$mp_dir/include" \
        -I"$mp_src" \
        "$mp_src/main.c" -o "$mp_build/mprompt.o"

    # Platform-specific assembly (x86-64 Linux)
    "$CLANG" -c -O2 -fPIC \
        "$mp_src/asm/longjmp_amd64.S" -o "$mp_build/longjmp_amd64.o"

    ar rcs "$mp_build/libmprompt.a" "$mp_build/mprompt.o" "$mp_build/longjmp_amd64.o"
    ok "libmprompt.a ($(stat -c%s "$mp_build/libmprompt.a") bytes)"
}

do_build_blood_runtime() {
    local debug_alloc=""
    if [ "${1:-}" = "--debug-alloc" ]; then
        debug_alloc="--debug-alloc"
        shift
    fi

    local rt_dir="$REPO_ROOT/runtime/blood-runtime"
    local rt_build="$rt_dir/build/debug"
    local fg="$BUILD_DIR/first_gen"
    [ -f "$fg" ] || die "first_gen not found. Run: ./build_selfhost.sh build first_gen"
    [ -f "$rt_dir/lib.blood" ] || die "Blood runtime source not found at $rt_dir/lib.blood"
    [ -f "$rt_dir/rt_mprompt_shim.c" ] || die "rt_mprompt_shim.c not found at $rt_dir/rt_mprompt_shim.c"
    [ -f "$rt_dir/rt_hashmap.c" ] || die "rt_hashmap.c not found at $rt_dir/rt_hashmap.c"
    command -v python3 >/dev/null || die "python3 required for IR post-processing"
    command -v "$LLC" >/dev/null || die "$LLC required for object compilation"
    command -v "$CLANG" >/dev/null || die "$CLANG required for C runtime pieces"

    mkdir -p "$rt_build"

    # libmprompt is embedded in libblood_runtime_blood.a so every downstream
    # link inherits mp_* symbols via the Blood runtime archive — no separate
    # -lmprompt flag threading through main.blood's 5 emit sites.
    local mp_build="$REPO_ROOT/vendor/libmprompt/build"
    if [ ! -f "$mp_build/libmprompt.a" ] \
       || [ ! -f "$mp_build/mprompt.o" ] \
       || [ ! -f "$mp_build/longjmp_amd64.o" ]; then
        do_build_libmprompt
    fi

    step "Compiling Blood runtime to LLVM IR${debug_alloc:+ (debug-alloc mode)}"
    "$fg" build --emit llvm-ir --no-cache --build-dir "$rt_dir/build" "$rt_dir/lib.blood"
    ok "IR generated"

    step "Post-processing IR"
    python3 "$rt_dir/build_runtime.py" $debug_alloc "$rt_build/lib.ll" "$rt_build/lib_clean.ll"
    ok "IR post-processed"

    step "Compiling to archive"
    # Capture llc output to a temp file, then filter with grep separately.
    # Going through a temp file (instead of a pipe) lets us check llc's exit
    # code with set -e enabled, and makes grep's "no matches" exit code (1)
    # completely irrelevant to script termination.
    #
    # History: we previously tried `llc ... 2>&1 | grep -v noise` with a
    # `set +o pipefail` hack and a PIPESTATUS check. That failed when llc
    # produced clean output (nothing to filter) because the pipe's exit
    # code was grep's (1 = no matches), and set -e killed the script BEFORE
    # the PIPESTATUS check could override the decision. Triggered when the
    # selfhost IR for the runtime became clean enough that llc emitted no
    # noise lines. See commit history around 2026-04-11 session 7 for detail.
    local llc_out="$rt_build/lib_clean.llc.log"
    local llc_status=0
    "$LLC" -filetype=obj -relocation-model=pic "$rt_build/lib_clean.ll" \
        -o "$rt_build/lib.o" >"$llc_out" 2>&1 || llc_status=$?
    # Surface any noise lines llc emitted (filtered to hide known-benign warnings).
    # Ignore grep's exit code; `grep || true` is safe here because we only want
    # its OUTPUT, not its exit status.
    grep -v 'inlinable function\|ignoring invalid debug' "$llc_out" || true
    rm -f "$llc_out"
    if [ "$llc_status" -ne 0 ]; then
        die "$LLC failed (exit $llc_status) compiling runtime IR at $rt_build/lib_clean.ll"
    fi
    # Remove the old object file if llc didn't produce a fresh one to prevent
    # packaging a stale archive on future failures.
    [ -f "$rt_build/lib.o" ] || die "$LLC did not produce $rt_build/lib.o"

    step "Compiling C runtime pieces (rt_hashmap.c, rt_mprompt_shim.c)"
    # rt_hashmap.c contains the HashMap type-erased runtime (hashmap_new/get/
    # insert/iter/...). Was historically baked into bootstrap/ manually, but
    # freshly-built runtime archives would miss these symbols and fail hashmap
    # golden tests. Now unconditionally compiled so the archive is complete.
    "$CLANG" -c -O2 -fPIC \
        "$rt_dir/rt_hashmap.c" \
        -o "$rt_build/rt_hashmap.o"
    "$CLANG" -c -O2 -fPIC \
        -I"$REPO_ROOT/vendor/libmprompt/include" \
        "$rt_dir/rt_mprompt_shim.c" \
        -o "$rt_build/rt_mprompt_shim.o"
    ok "rt_hashmap.o + rt_mprompt_shim.o"

    # Build the archive fresh. `ar rcs` on an existing archive APPENDS members,
    # which would leave stale object files in place when the inputs change. `rm`
    # then `rcs` gives us a clean build.
    rm -f "$rt_build/libblood_runtime_blood.a"
    ar rcs "$rt_build/libblood_runtime_blood.a" \
        "$rt_build/lib.o" \
        "$rt_build/rt_hashmap.o" \
        "$rt_build/rt_mprompt_shim.o" \
        "$mp_build/mprompt.o" \
        "$mp_build/longjmp_amd64.o"
    ok "libblood_runtime_blood.a ($(stat -c%s "$rt_build/libblood_runtime_blood.a") bytes)"
}

# ── Test suites ─────────────────────────────────────────────────────────────

do_test_golden() {
    local bin="${1:-$BUILD_DIR/first_gen}"
    [ -f "$bin" ] || die "$bin not found"
    [ -d "$GOLDEN_TESTS" ] || die "Golden tests not found at $GOLDEN_TESTS"

    check_staleness "$bin"

    # Parallelism: use available cores
    local jobs
    jobs=$(nproc 2>/dev/null || echo 4)

    # Incremental cache: per-compiler, invalidated when compiler changes
    local cache_dir="$BUILD_DIR/.golden-cache/$(basename "$bin")"
    local compiler_mtime
    compiler_mtime=$(stat -c '%Y' "$bin")

    if [ "$FORCE" = "--force" ]; then
        rm -rf "$cache_dir"
    elif [ -f "$cache_dir/.mtime" ]; then
        if [ "$(cat "$cache_dir/.mtime")" != "$compiler_mtime" ]; then
            rm -rf "$cache_dir"
        fi
    fi
    mkdir -p "$cache_dir"
    echo "$compiler_mtime" > "$cache_dir/.mtime"

    # Temp dir for per-test result files
    local results_dir
    results_dir=$(mktemp -d)

    # Enumerate tests
    local test_files=("$GOLDEN_TESTS"/t[0-9][0-9]_*.blood)
    local total=${#test_files[@]}

    step "Running golden tests through $(basename "$bin") ($total tests, $jobs workers)"

    # ── Per-test worker (runs in subshell via xargs) ──────────────────────
    _gt_worker() {
        local src="$1" bin="$2" stdlib="$3" cache_dir="$4" results_dir="$5"
        local name
        name=$(basename "$src" .blood)
        local rf="$results_dir/$name.result"
        local of="$results_dir/$name.output"

        # Incremental: check source hash against cache
        local src_hash
        src_hash=$(md5sum "$src" | cut -d' ' -f1)
        if [ -f "$cache_dir/$name" ]; then
            local cached
            cached=$(cat "$cache_dir/$name")
            if [ "${cached%%:*}" = "$src_hash" ]; then
                echo "CACHED_${cached#*:}" > "$rf"
                return 0
            fi
        fi

        # ── COMPILE_FAIL ──
        if head -1 "$src" | grep -q '^// COMPILE_FAIL:'; then
            local tmpdir stderr_file
            tmpdir=$(mktemp -d)
            stderr_file="$tmpdir/stderr.txt"
            # Use 'check' (no codegen/link needed) with compile timeout
            if timeout 30 "$bin" check "$src" --stdlib-path "$stdlib" \
                    >/dev/null 2>"$stderr_file"; then
                echo "COMP_FAIL" > "$rf"
                echo "(expected compile failure, but succeeded)" > "$of"
            else
                local diag_ok=1
                if grep -q '^// EXPECT: ' "$src"; then
                    while IFS= read -r line; do
                        local pat="${line#// EXPECT: }"
                        if ! grep -qF "$pat" "$stderr_file"; then
                            diag_ok=0
                            break
                        fi
                    done < <(grep '^// EXPECT: ' "$src")
                fi
                if [ "$diag_ok" -eq 1 ]; then
                    echo "PASS" > "$rf"
                    echo "$src_hash:PASS" > "$cache_dir/$name"
                else
                    echo "PASS_DIAG" > "$rf"
                    echo "$src_hash:PASS_DIAG" > "$cache_dir/$name"
                fi
            fi
            rm -rf "$tmpdir"
            return 0
        fi

        # ── XFAIL ──
        if head -1 "$src" | grep -q '^// XFAIL:'; then
            echo "SKIP" > "$rf"
            return 0
        fi

        # ── Normal test: compile + run ──
        local tmpdir
        tmpdir=$(mktemp -d)
        # Compile with timeout (prevents compiler hangs)
        if ! timeout 30 "$bin" build "$src" --build-dir "$tmpdir" \
                -o "$tmpdir/out" --stdlib-path "$stdlib" >/dev/null 2>&1; then
            echo "COMP_FAIL" > "$rf"
            echo "(compile)" > "$of"
            rm -rf "$tmpdir"
            return 0
        fi

        local actual exit_code=0 stderr_file="$tmpdir/stderr"
        actual=$(timeout 30 "$tmpdir/out" 2>"$stderr_file") || exit_code=$?

        local expected=""
        expected=$(grep '^// EXPECT:' "$src" | sed 's|^// EXPECT: *||' || true)

        if [ -n "$expected" ]; then
            if [ "$actual" = "$expected" ]; then
                echo "PASS" > "$rf"
                echo "$src_hash:PASS" > "$cache_dir/$name"
            else
                echo "RUN_FAIL" > "$rf"
                {
                    echo "(output mismatch)"
                    printf "      expected: %s\n" "$expected"
                    printf "      actual:   %s\n" "$actual"
                    [ -s "$stderr_file" ] && printf "      stderr: %s\n" "$(head -5 "$stderr_file")"
                } > "$of"
            fi
        else
            local expect_exit=""
            expect_exit=$(grep '^// EXPECT_EXIT:' "$src" | head -1 \
                | sed 's|^// EXPECT_EXIT: *||' || true)
            [ -z "$expect_exit" ] && expect_exit="0"

            local passed=0
            if [ "$expect_exit" = "nonzero" ] && [ "$exit_code" -ne 0 ]; then
                passed=1
            elif [ "$exit_code" = "$expect_exit" ]; then
                passed=1
            fi

            if [ "$passed" -eq 1 ]; then
                echo "PASS" > "$rf"
                echo "$src_hash:PASS" > "$cache_dir/$name"
            else
                echo "RUN_FAIL" > "$rf"
                {
                    echo "(exit $exit_code, expected $expect_exit)"
                    [ -s "$stderr_file" ] && printf "      stderr: %s\n" "$(head -5 "$stderr_file")"
                } > "$of"
            fi
        fi

        rm -rf "$tmpdir"
        return 0
    }
    export -f _gt_worker

    # ── Dispatch tests in parallel ────────────────────────────────────────
    printf '%s\n' "${test_files[@]}" | \
        xargs -P "$jobs" -I{} bash -c '_gt_worker "$@"' _ {} \
            "$bin" "$STDLIB_PATH" "$cache_dir" "$results_dir" || true

    # ── Aggregate results (in test-name order) ────────────────────────────
    local pass=0 comp_fail=0 run_fail=0 skip=0 diag_miss=0 cached=0

    for src in "${test_files[@]}"; do
        local name
        name=$(basename "$src" .blood)
        local rf="$results_dir/$name.result"

        if [ ! -f "$rf" ]; then
            fail "$name (no result — worker crashed?)"
            comp_fail=$((comp_fail + 1))
            continue
        fi

        local result
        result=$(cat "$rf")
        local of="$results_dir/$name.output"

        case "$result" in
            PASS)
                [ "$QUIET" = "--quiet" ] || ok "$name"
                pass=$((pass + 1))
                ;;
            PASS_DIAG)
                [ "$QUIET" = "--quiet" ] || ok "$name (reject ok, diagnostic mismatch)"
                pass=$((pass + 1))
                diag_miss=$((diag_miss + 1))
                ;;
            CACHED_PASS)
                [ "$QUIET" = "--quiet" ] || ok "$name (cached)"
                pass=$((pass + 1))
                cached=$((cached + 1))
                ;;
            CACHED_PASS_DIAG)
                [ "$QUIET" = "--quiet" ] || ok "$name (cached, diagnostic mismatch)"
                pass=$((pass + 1))
                diag_miss=$((diag_miss + 1))
                cached=$((cached + 1))
                ;;
            COMP_FAIL)
                local detail=""
                [ -f "$of" ] && detail=$(cat "$of")
                fail "$name $detail"
                comp_fail=$((comp_fail + 1))
                ;;
            RUN_FAIL)
                if [ -f "$of" ]; then
                    fail "$name $(head -1 "$of")"
                    tail -n +2 "$of"
                else
                    fail "$name (run)"
                fi
                run_fail=$((run_fail + 1))
                ;;
            SKIP)
                skip=$((skip + 1))
                ;;
        esac
    done

    rm -rf "$results_dir"

    printf "\n  Passed: %d  Compile fail: %d  Run fail: %d  Skipped: %d  Total: %d" \
        "$pass" "$comp_fail" "$run_fail" "$skip" "$total"
    [ "$cached" -gt 0 ] && printf "  (cached: %d)" "$cached"
    printf "\n"
    if [ "$diag_miss" -gt 0 ]; then
        warn "Diagnostic mismatches: $diag_miss (compile-fail tests that reject correctly but emit wrong message)"
    fi
    [ "$((comp_fail + run_fail))" -eq 0 ] && return 0 || return 1
}

do_test_golden_blood() {
    # Run golden tests with test binaries linked against the Blood runtime
    # (instead of the Rust runtime). This validates Stage 2 runtime independence.
    # Uses separate cache to avoid false passes from Rust-runtime cached results.
    local blood_rt="$REPO_ROOT/runtime/blood-runtime/build/debug/libblood_runtime_blood.a"
    [ -f "$blood_rt" ] || die "Blood runtime not found. Run: ./build_selfhost.sh build blood_runtime"
    step "Running golden tests linked against Blood runtime"
    local saved_build_dir="$BUILD_DIR"
    BUILD_DIR="$BUILD_DIR/blood-rt"
    mkdir -p "$BUILD_DIR"
    BLOOD_RUST_RUNTIME="$blood_rt" do_test_golden "$@"
    BUILD_DIR="$saved_build_dir"
}

do_test_dispatch() {
    local bin1="${1:-$BLOOD_RUST}"
    local bin2="${2:-$BUILD_DIR/first_gen}"
    [ -f "$bin1" ] || die "Compiler 1 not found: $bin1"
    [ -f "$bin2" ] || die "Compiler 2 not found: $bin2"
    local dispatch_dir="$REPO_ROOT/tests/dispatch"
    [ -d "$dispatch_dir" ] || die "Dispatch tests not found at $dispatch_dir"

    step "Dispatch tests: $(basename "$bin1") vs $(basename "$bin2")"
    local match=0 mismatch=0 total=0

    for src in "$dispatch_dir"/t05_*.blood; do
        [ -f "$src" ] || continue
        local name
        name="$(basename "$src" .blood)"
        total=$((total + 1))

        local out1="" out2="" rc1=0 rc2=0
        out1=$("$bin1" run "$src" --stdlib-path "$STDLIB_PATH" 2>/dev/null) || rc1=$?
        out2=$("$bin2" run "$src" --stdlib-path "$STDLIB_PATH" 2>/dev/null) || rc2=$?

        if [ "$out1" = "$out2" ] && [ "$rc1" = "$rc2" ]; then
            ok "$name"
            match=$((match + 1))
        else
            fail "$name"
            [ "$out1" != "$out2" ] && printf "      output differs\n"
            [ "$rc1" != "$rc2" ] && printf "      exit: %s vs %s\n" "$rc1" "$rc2"
            mismatch=$((mismatch + 1))
        fi
    done

    printf "\n  Match: %d  Mismatch: %d  Total: %d\n" "$match" "$mismatch" "$total"
    [ "$mismatch" -eq 0 ] && return 0 || return 1
}

do_test_pillar2() {
    # Verifies Pillar 2 (Identity / Content-Addressing) end-to-end through the
    # proving-ground P5 test. This is the canonical demo that cross-module
    # hash-based linking works: mathlib.blood defines factorial/fibonacci/gcd,
    # those definitions get stored in the codebase by content hash, and
    # p5_identity.blood imports them by hash prefix (`use hash("a13d")` etc.).
    #
    # The golden test framework doesn't support the two-step setup that this
    # flow requires, so we provide it as a separate `test pillar2` target.
    local compiler="${1:-$BUILD_DIR/first_gen}"
    [ -f "$compiler" ] || die "$compiler not found"
    local mathlib="$REPO_ROOT/tests/proving/mathlib.blood"
    local p5="$REPO_ROOT/tests/proving/p5_identity.blood"
    [ -f "$mathlib" ] || die "mathlib not found at $mathlib"
    [ -f "$p5" ] || die "p5_identity not found at $p5"

    step "Pillar 2 demo: cross-module hash-based linking"

    # Step 1: Store mathlib definitions in the codebase by content hash.
    # The build command fails at llc because mathlib has no main() — that's
    # expected and harmless: the store is a side effect of emit_content_hashes
    # which runs before codegen finishes. We capture the exit code but don't
    # treat it as fatal; we verify success by checking the codebase names
    # index instead.
    local codebase_dir="${HOME}/.blood/codebases/default"
    local names_idx="$codebase_dir/names.idx"
    # Run the store step (stderr suppressed; llc error is expected).
    "$compiler" build --store-codebase "$mathlib" >/dev/null 2>&1 || true

    if [ ! -f "$names_idx" ]; then
        fail "codebase names index not produced at $names_idx"
        return 1
    fi

    # Verify the three expected functions are in the codebase.
    local missing=0
    for fn in factorial fibonacci gcd; do
        if ! grep -q "^$fn " "$names_idx"; then
            fail "codebase missing: $fn"
            missing=$((missing + 1))
        fi
    done
    if [ "$missing" -ne 0 ]; then
        return 1
    fi
    ok "codebase populated: factorial, fibonacci, gcd"

    # Step 2: Run p5_identity.blood, which imports those functions by
    # content-hash prefix (`use hash("a13d")` etc.). Capture stdout and
    # compare against expected lines.
    local expected=$(cat <<'EOF'
=== Part 1: factorial by hash ===
factorial(5)=120
factorial(10)=3628800
=== Part 2: fibonacci by hash ===
fibonacci(10)=55
=== Part 3: gcd by hash ===
gcd(48,18)=6
gcd(100,75)=25
=== All parts passed ===
EOF
)
    local actual
    actual=$(BLOOD_RUST_RUNTIME="$BUILD_DIR/libblood_runtime.a" \
        "$compiler" run "$p5" 2>/dev/null || true)

    if [ "$actual" = "$expected" ]; then
        ok "p5_identity ran through cross-module hash linking"
        printf "\n  [1;32m✓[0m Pillar 2 end-to-end demo PASSED\n"
        return 0
    else
        fail "p5_identity output mismatch"
        printf "  expected:\n%s\n" "$expected" | sed 's/^/    /'
        printf "  actual:\n%s\n" "$actual" | sed 's/^/    /'
        return 1
    fi
}

do_test_blood() {
    local compiler="${1:-$BLOOD_RUST}"
    [ -f "$compiler" ] || die "$compiler not found"
    [ -d "$BLOOD_TESTS" ] || die "blood-test directory not found at $BLOOD_TESTS"

    step "Running blood test suite through $(basename "$compiler")"

    local pass=0 fail_count=0 total=0

    for src in "$BLOOD_TESTS"/*.blood; do
        [ -f "$src" ] || continue
        local name
        name="$(basename "$src" .blood)"
        total=$((total + 1))

        local output rc=0
        local stdlib_flag=""
        if grep -qE '^(mod std;|use std\.)' "$src"; then
            stdlib_flag="--stdlib-path $STDLIB_PATH"
        fi
        output=$("$compiler" test "$src" $stdlib_flag 2>&1) || rc=$?

        if [ "$rc" -eq 0 ]; then
            local summary
            summary=$(echo "$output" | grep '^test result:' || true)
            ok "$name ${summary:+($summary)}"
            pass=$((pass + 1))
        else
            fail "$name (exit $rc)"
            echo "$output" | tail -5 | sed 's/^/      /'
            fail_count=$((fail_count + 1))
        fi
    done

    printf "\n  %d/%d test files passed\n" "$pass" "$total"
    [ "$fail_count" -eq 0 ] && return 0 || return 1
}

# ── Verification ────────────────────────────────────────────────────────────

generate_reference_ir() {
    [ -f "$BLOOD_RUST" ] || die "blood-rust not found at $BLOOD_RUST"
    step "Generating reference IR from blood-rust"
    $BLOOD_RUST build --emit llvm-ir-unopt -o "$BUILD_DIR/reference_ir.ll" main.blood 2>/dev/null
    [ -s "$BUILD_DIR/reference_ir.ll" ] || die "blood-rust did not produce reference_ir.ll"
    ok "reference_ir.ll generated ($(wc -l < "$BUILD_DIR/reference_ir.ll") lines)"
}

verify_ir() {
    local ir_file="${1:-$BUILD_DIR/second_gen.ll}"
    [ -f "$ir_file" ] || die "$ir_file not found"

    local errors=0

    [ -f "$BUILD_DIR/reference_ir.ll" ] || generate_reference_ir

    # A: Structural IR verification
    step "Verifying IR structure ($ir_file)"
    local verify_output
    if verify_output=$("$OPT" -passes=verify "$ir_file" -disable-output 2>&1); then
        ok "IR structure valid (SSA, types, dominance)"
    else
        fail "IR structure verification failed"
        printf "%s\n" "$verify_output" | head -20
        errors=$((errors + 1))
    fi

    # B: Declaration signature diff
    step "Comparing declarations against reference"
    local ref_sigs self_sigs decl_diff
    ref_sigs=$(grep -E '^(declare|define) ' "$BUILD_DIR/reference_ir.ll" | sed 's/ {$//' | sed 's/^define /declare /' | sort)
    self_sigs=$(grep -E '^(declare|define) ' "$ir_file" | sed 's/ {$//' | sed 's/^define /declare /' | sort)
    decl_diff=$(diff <(echo "$ref_sigs") <(echo "$self_sigs") || true)
    if [ -z "$decl_diff" ]; then
        ok "Declaration signatures match"
    else
        fail "Declaration mismatches detected"
        echo "$decl_diff" | head -20
        errors=$((errors + 1))
    fi

    # C: FileCheck tests
    step "Running FileCheck tests"
    local fc_pass=0 fc_fail=0 fc_total=0
    for check_src in "$DIR"/tests/check_*.blood; do
        [ -f "$check_src" ] || continue
        local check_name
        check_name="$(basename "$check_src" .blood)"
        fc_total=$((fc_total + 1))

        local check_tmpdir
        check_tmpdir=$(mktemp -d)

        if ! "$BUILD_DIR/first_gen" build "$check_src" -o "$check_tmpdir/check_out.ll" >/dev/null 2>&1; then
            fail "$check_name (compile failed)"
            fc_fail=$((fc_fail + 1))
            rm -rf "$check_tmpdir"
            continue
        fi

        if "$FILECHECK" --input-file="$check_tmpdir/check_out.ll" "$check_src" 2>/dev/null; then
            ok "$check_name"
            fc_pass=$((fc_pass + 1))
        else
            fail "$check_name"
            fc_fail=$((fc_fail + 1))
        fi

        rm -rf "$check_tmpdir"
    done

    if [ "$fc_total" -eq 0 ]; then
        warn "No FileCheck tests found in tests/check_*.blood"
    else
        printf "  FileCheck: %d/%d passed\n" "$fc_pass" "$fc_total"
        if [ "$fc_fail" -gt 0 ]; then errors=$((errors + 1)); fi
    fi

    # D: Function count sanity check
    step "Checking function counts"
    local ref_decls self_decls self_defines
    ref_decls=$(grep -c '^declare ' "$BUILD_DIR/reference_ir.ll")
    self_decls=$(grep -c '^declare ' "$ir_file")
    self_defines=$(grep -c '^define ' "$ir_file")

    printf "  Self-compiled: %d definitions, %d declarations\n" "$self_defines" "$self_decls"
    printf "  Reference:     %d declarations\n" "$ref_decls"

    if [ "$self_decls" -gt "$ref_decls" ]; then
        local extra=$(( self_decls - ref_decls ))
        warn "Self-compiled has $extra more declarations than reference"
    else
        ok "Self-compiled declarations ($self_decls) <= reference ($ref_decls)"
    fi

    if [ "$errors" -gt 0 ]; then
        printf "\n\033[1;31mVerification failed: %d error(s)\033[0m\n" "$errors"
        return 1
    else
        printf "\n\033[1;32mAll verification checks passed.\033[0m\n"
    fi
}

# ── Diagnostic tools ────────────────────────────────────────────────────────

build_asan() {
    local ir_file="${1:-$BUILD_DIR/second_gen.ll}"
    [ -f "$ir_file" ] || die "$ir_file not found"

    # Step 1: Build a debug-alloc runtime (calloc/free instead of mmap/munmap).
    # Regions use calloc (visible to ASan) and all calloc→__libc_calloc
    # (consistent pair with __libc_free, avoids ASan alloc/free mismatch).
    # Save/restore the normal runtime — do_build_blood_runtime overwrites RUNTIME_A.
    local saved_rt=""
    if [ -f "$RUNTIME_A" ]; then
        saved_rt="$(mktemp)"
        cp "$RUNTIME_A" "$saved_rt"
    fi
    step "Building debug-alloc runtime for ASan"
    do_build_blood_runtime --debug-alloc
    local debug_rt="$REPO_ROOT/runtime/blood-runtime/build/debug/libblood_runtime_blood.a"
    [ -f "$debug_rt" ] || die "Debug-alloc runtime not found at $debug_rt"
    # Copy the debug-alloc archive to a stable location so we can restore RUNTIME_A
    cp "$debug_rt" "$BUILD_DIR/libblood_runtime_debug.a"
    debug_rt="$BUILD_DIR/libblood_runtime_debug.a"
    # Restore the normal runtime so subsequent commands (test golden, etc.) work
    if [ -n "$saved_rt" ] && [ -f "$saved_rt" ]; then
        cp "$saved_rt" "$RUNTIME_A"
        rm -f "$saved_rt"
    fi
    ok "Debug-alloc runtime built ($(stat -c%s "$debug_rt") bytes)"

    step "Building ASan-instrumented binary from $ir_file"

    "$LLVM_AS" "$ir_file" -o "$BUILD_DIR/second_gen_asan.bc"
    ok "Assembled to bitcode"

    # opt-18+ accepts just `asan` — the older `module(asan-module),function(asan)`
    # pass-manager syntax is invalid in the new pass manager.
    "$OPT" -passes='asan' \
        "$BUILD_DIR/second_gen_asan.bc" -o "$BUILD_DIR/second_gen_asan_inst.bc"
    ok "ASan instrumentation applied"

    "$LLC" "$BUILD_DIR/second_gen_asan_inst.bc" \
        -o "$BUILD_DIR/second_gen_asan.o" -filetype=obj -relocation-model=pic
    ok "Compiled to object"

    # Link against the debug-alloc runtime (not the normal runtime).
    "$CLANG" "$BUILD_DIR/second_gen_asan.o" "$debug_rt" \
        -fsanitize=address -Wl,-z,muldefs \
        -lstdc++ -lm -lpthread -ldl -no-pie \
        -o "$BUILD_DIR/second_gen_asan"
    ok "Linked second_gen_asan ($(wc -c < "$BUILD_DIR/second_gen_asan") bytes)"

    rm -f "$BUILD_DIR/second_gen_asan.bc" "$BUILD_DIR/second_gen_asan_inst.bc" "$BUILD_DIR/second_gen_asan.o"

    printf "\n  Run with: ./build/second_gen_asan version\n"
    printf "  ASan will report memory errors with stack traces.\n"
    printf "  Note: debug-alloc routes regions through calloc/free (ASan-tracked),\n"
    printf "  and replaces __libc_free/__libc_realloc with free/realloc (ASan-tracked).\n"
}

bisect_functions() {
    local self_ir="${1:-$BUILD_DIR/second_gen.ll}"
    [ -f "$self_ir" ] || die "$self_ir not found"
    [ -f "$BUILD_DIR/reference_ir.ll" ] || generate_reference_ir
    [ -f "$RUNTIME_A" ] || die "Runtime library not found at $RUNTIME_A"

    step "Bisecting for miscompiled function"

    local bisect_dir
    bisect_dir=$(mktemp -d "$DIR/.bisect_XXXXXX")
    trap "rm -rf '$bisect_dir'" EXIT

    "$LLVM_AS" "$BUILD_DIR/reference_ir.ll" -o "$bisect_dir/ref.bc"
    "$LLVM_AS" "$self_ir" -o "$bisect_dir/self.bc"
    ok "Assembled both IR files to bitcode"

    grep '^define ' "$self_ir" | sed 's/^define [^@]*@\([^ (]*\).*/\1/' | sort -u > "$bisect_dir/all_funcs.txt"
    local total_funcs
    total_funcs=$(wc -l < "$bisect_dir/all_funcs.txt")
    ok "Found $total_funcs functions to bisect"

    if [ "$total_funcs" -eq 0 ]; then
        fail "No functions found in $self_ir"
        return 1
    fi

    test_hybrid() {
        local func_list="$1"
        local hybrid_bc="$bisect_dir/hybrid.bc"

        cp "$bisect_dir/ref.bc" "$hybrid_bc"

        local extract_args=""
        while IFS= read -r fname; do
            [ -z "$fname" ] && continue
            extract_args="$extract_args --func=$fname"
        done < "$func_list"

        if [ -z "$extract_args" ]; then return 1; fi

        if ! "$LLVM_EXTRACT" $extract_args \
                "$bisect_dir/self.bc" -o "$bisect_dir/extracted.bc" 2>/dev/null; then
            warn "Could not extract functions (some may be missing)"
            return 1
        fi

        local delete_args=""
        while IFS= read -r fname; do
            [ -z "$fname" ] && continue
            delete_args="$delete_args --delete=$fname"
        done < "$func_list"

        if ! "$LLVM_EXTRACT" $delete_args \
                "$bisect_dir/ref.bc" -o "$bisect_dir/ref_trimmed.bc" 2>/dev/null; then
            cp "$bisect_dir/ref.bc" "$bisect_dir/ref_trimmed.bc"
        fi

        if ! "$LLVM_LINK" \
                "$bisect_dir/ref_trimmed.bc" "$bisect_dir/extracted.bc" \
                -o "$hybrid_bc" 2>/dev/null; then
            warn "Link failed for this subset"
            return 1
        fi

        if ! "$LLC" "$hybrid_bc" \
                -o "$bisect_dir/hybrid.o" -filetype=obj -relocation-model=pic 2>/dev/null; then
            warn "LLC failed for hybrid"
            return 1
        fi

        if ! "$CLANG" "$bisect_dir/hybrid.o" "$RUNTIME_A" \
                -lm -ldl -lpthread -no-pie -o "$bisect_dir/hybrid" 2>/dev/null; then
            warn "Link failed for hybrid binary"
            return 1
        fi

        if timeout 10 "$bisect_dir/hybrid" version >/dev/null 2>&1; then
            return 1  # No crash
        else
            return 0  # Crash
        fi
    }

    local lo=0 hi=$((total_funcs - 1)) iteration=0 max_iterations=20

    cp "$bisect_dir/all_funcs.txt" "$bisect_dir/test_funcs.txt"
    if ! test_hybrid "$bisect_dir/test_funcs.txt"; then
        fail "Full self-compiled set does NOT crash — cannot bisect"
        printf "  The crash may require specific linking or execution conditions.\n"
        return 1
    fi
    ok "Confirmed: full self-compiled set crashes"

    while [ "$lo" -lt "$hi" ] && [ "$iteration" -lt "$max_iterations" ]; do
        iteration=$((iteration + 1))
        local mid=$(( (lo + hi) / 2 ))

        printf "  Bisect iteration %d: testing functions %d-%d of %d (range %d-%d)\n" \
            "$iteration" "$lo" "$mid" "$total_funcs" "$lo" "$hi"

        sed -n "$((lo + 1)),$((mid + 1))p" "$bisect_dir/all_funcs.txt" > "$bisect_dir/test_funcs.txt"

        if test_hybrid "$bisect_dir/test_funcs.txt"; then
            hi=$mid
            ok "Crash in first half (narrowed to $((hi - lo + 1)) functions)"
        else
            lo=$((mid + 1))
            ok "Crash in second half (narrowed to $((hi - lo + 1)) functions)"
        fi
    done

    local suspect
    suspect=$(sed -n "$((lo + 1))p" "$bisect_dir/all_funcs.txt")

    if [ "$lo" -eq "$hi" ]; then
        printf "\n\033[1;33mBisect result: likely miscompiled function:\033[0m\n"
        printf "  @%s (function #%d of %d)\n" "$suspect" "$lo" "$total_funcs"
    else
        printf "\n\033[1;33mBisect narrowed to %d functions (%d-%d):\033[0m\n" \
            "$((hi - lo + 1))" "$lo" "$hi"
        sed -n "$((lo + 1)),$((hi + 1))p" "$bisect_dir/all_funcs.txt" | while read -r f; do
            printf "  @%s\n" "$f"
        done
    fi

    printf "\n  To inspect, compare this function between build/reference_ir.ll and %s\n" "$self_ir"
}

# ── Status ──────────────────────────────────────────────────────────────────

show_status() {
    step "Blood compiler status"
    printf "  %-20s %s\n" "Git HEAD:" "$(git -C "$REPO_ROOT" log --oneline -1 2>/dev/null || echo 'N/A')"
    printf "  %-20s %s\n" "Branch:" "$(git -C "$REPO_ROOT" rev-parse --abbrev-ref HEAD 2>/dev/null || echo 'N/A')"
    printf "\n"

    for entry in "blood-rust:$BLOOD_RUST" "first_gen:$BUILD_DIR/first_gen" "second_gen:$BUILD_DIR/second_gen" "third_gen:$BUILD_DIR/third_gen"; do
        local label="${entry%%:*}"
        local path="${entry#*:}"
        if [ -f "$path" ]; then
            local size ago_mins
            size=$(wc -c < "$path")
            ago_mins=$(( ($(date +%s) - $(stat -c '%Y' "$path")) / 60 ))
            printf "  %-20s %s bytes, %dm ago\n" "$label:" "$size" "$ago_mins"
        else
            printf "  %-20s (not built)\n" "$label:"
        fi
    done

    printf "\n"
    local procs
    procs=$(pgrep -af "(first_gen|second_gen|third_gen) build" 2>/dev/null | grep -v "$$\|build_selfhost" || true)
    if [ -n "$procs" ]; then
        warn "Running compiler processes:"
        printf "%s\n" "$procs" | sed 's/^/    /'
    else
        printf "  No running compiler processes.\n"
    fi

    local last_gt
    last_gt=$(grep -h 'Passed:.*Compile fail:.*Run fail:' "$DIR/.logs"/build_*.log 2>/dev/null | tail -1 || true)
    if [ -n "$last_gt" ]; then
        printf "\n  Last golden:%s\n" "$last_gt"
    fi
}

# ── Usage ───────────────────────────────────────────────────────────────────

show_usage() {
    cat <<'USAGE'
Usage: ./build_selfhost.sh [command] [args] [-q|--quiet]

Build:
  build first_gen     Build first_gen from seed compiler (bootstrap/seed)
  build first_gen --relink
                      Fast path: skip selfhost compilation, just re-run
                      clang-18 link against existing build/obj/*.o and
                      the current runtime archive. Use when only the
                      runtime changed. Drops cycle from ~11 min to <1s.
                      Warns if selfhost source is newer than .o files.
  build second_gen    Self-compile first_gen → second_gen
  build third_gen     Bootstrap second_gen → third_gen + byte-compare
  build blood_runtime Compile Blood-native runtime → libblood_runtime_blood.a
  build all           Full chain: blood_runtime → first_gen → GT → second_gen → GT → third_gen
  build cargo         (legacy) Rebuild blood-rust via cargo

Test:
  test golden [compiler]    Run golden suite (default: first_gen)
  test golden-blood [compiler]    Run golden suite linked against Blood runtime
  test pillar2 [compiler]         Run Pillar 2 (content-addressing) end-to-end demo via proving/p5_identity
  test dispatch [bin1] [bin2]     Compare dispatch behavior (default: bootstrap vs first_gen)
  test blood [compiler]           Run tests/blood-test/ (default: bootstrap)

  Compiler names: bootstrap, first_gen, second_gen, third_gen (or a path)

Diagnostics:
  verify [ir]         Structural IR verification + declaration diff + FileCheck
  ir-check [ir]       FileCheck tests only
  asan [ir]           Build ASan-instrumented binary
  bisect [ir]         Binary search for miscompiled function
  emit [stage]        Emit intermediate IR (ast|hir|mir|llvm-ir|llvm-ir-unopt)
  debug-test <file> [compiler]  Compile with --dump-mir --validate-mir, run, preserve artifacts
  metrics             Show build size/time trends from .logs/metrics.jsonl

Workflow:
  gate [--quick]      Full bootstrap pipeline + update seed on success
                      --quick: skip first_gen/second_gen builds (assumes already verified)
  run <file> [compiler] [flags...]  Compile and run a file (extra flags passed to compiler)
  diff <file>         Compare blood-rust vs first_gen output
  status              Show compiler status, ages, processes (default command)
  install             Install toolchain to ~/.blood/{bin,lib}/
  clean               Remove build artifacts (preserves .logs)
  clean-cache         Remove only caches (content hashes, obj hashes, blood-cache)
  clean-all           Remove build artifacts and logs

Flags:
  -q, --quiet         Suppress per-test output (only failures + summary)
  --fresh             Clear caches before building (use with build commands)
  --force             Ignore incremental cache, re-run all tests
USAGE
}

# ── Command dispatch ────────────────────────────────────────────────────────

case "${1:-status}" in

    # ── build ───────────────────────────────────────────────────────────────

    build)
        check_zombies
        # Check for --fresh flag in any position
        _build_fresh=false
        for _ba in "$@"; do
            if [[ "$_ba" == "--fresh" ]]; then _build_fresh=true; fi
        done
        if $_build_fresh; then
            clear_all_caches
        fi
        case "${2:-}" in
            cargo)
                do_build_cargo
                ;;
            first_gen)
                # Check for --relink flag anywhere in remaining args
                _relink=false
                for _arg in "${@:3}"; do
                    if [ "$_arg" = "--relink" ]; then
                        _relink=true
                    fi
                done
                if $_relink; then
                    do_relink_first_gen
                else
                    do_build_first_gen "--timings"
                fi
                ;;
            second_gen)
                do_build_second_gen
                ;;
            third_gen)
                do_build_third_gen
                ;;
            runtime)
                do_build_runtime
                ;;
            blood_runtime)
                do_build_blood_runtime "${3:-}"
                ;;
            libmprompt)
                do_build_libmprompt
                ;;
            all)
                PIPELINE_START=$(date +%s)

                do_build_blood_runtime
                do_build_first_gen "--timings"
                do_test_golden "$BUILD_DIR/first_gen"
                do_build_second_gen
                do_test_golden "$BUILD_DIR/second_gen"
                do_build_third_gen

                printf "\n\033[1;32mFull pipeline complete.\033[0m Total: %s\n" "$(elapsed_since "$PIPELINE_START")"
                printf "Log: %s\n" "${LOG_FILE:-<none>}"
                ;;
            "")
                die "build requires a stage: first_gen, second_gen, third_gen, blood_runtime, all (legacy: cargo, runtime)"
                ;;
            *)
                die "Unknown build stage: $2. Expected: first_gen, second_gen, third_gen, blood_runtime, all (legacy: cargo, runtime)"
                ;;
        esac
        ;;

    # ── test ────────────────────────────────────────────────────────────────

    test)
        case "${2:-golden}" in
            golden)
                do_test_golden "$(resolve_compiler "${3:-first_gen}")"
                ;;
            dispatch)
                do_test_dispatch "$(resolve_compiler "${3:-bootstrap}")" "$(resolve_compiler "${4:-first_gen}")"
                ;;
            blood)
                do_test_blood "$(resolve_compiler "${3:-bootstrap}")"
                ;;
            golden-blood)
                do_test_golden_blood "$(resolve_compiler "${3:-first_gen}")"
                ;;
            pillar2)
                do_test_pillar2 "$(resolve_compiler "${3:-first_gen}")"
                ;;
            *)
                die "Unknown test suite: $2. Expected: golden, dispatch, blood, golden-blood, pillar2"
                ;;
        esac
        ;;

    # ── diagnostics ─────────────────────────────────────────────────────────

    verify)
        verify_ir "${2:-$BUILD_DIR/second_gen.ll}"
        ;;

    ir-check)
        [ -f "$BUILD_DIR/first_gen" ] || die "first_gen not found. Build it first."

        step "Running FileCheck tests"
        fc_pass=0 fc_fail=0 fc_total=0

        for check_src in "$DIR"/tests/check_*.blood; do
            [ -f "$check_src" ] || continue
            check_name="$(basename "$check_src" .blood)"
            fc_total=$((fc_total + 1))

            tmpdir=$(mktemp -d)

            if ! "$BUILD_DIR/first_gen" build "$check_src" -o "$tmpdir/check_out.ll" >/dev/null 2>&1; then
                fail "$check_name (compile failed)"
                fc_fail=$((fc_fail + 1))
                rm -rf "$tmpdir"
                continue
            fi

            if "$FILECHECK" --input-file="$tmpdir/check_out.ll" "$check_src" 2>/dev/null; then
                ok "$check_name"
                fc_pass=$((fc_pass + 1))
            else
                fail "$check_name"
                "$FILECHECK" --input-file="$tmpdir/check_out.ll" "$check_src" 2>&1 | head -10 || true
                fc_fail=$((fc_fail + 1))
            fi

            rm -rf "$tmpdir"
        done

        if [ "$fc_total" -eq 0 ]; then
            warn "No FileCheck tests found in tests/check_*.blood"
        else
            printf "\n  %d/%d FileCheck tests passed\n" "$fc_pass" "$fc_total"
            if [ "$fc_fail" -gt 0 ]; then exit 1; fi
        fi
        ;;

    asan)
        build_asan "${2:-$BUILD_DIR/second_gen.ll}"
        ;;

    bisect)
        bisect_functions "${2:-$BUILD_DIR/second_gen.ll}"
        ;;

    emit)
        stage="${2:-llvm-ir}"
        [ -f "$BLOOD_RUST" ] || die "blood-rust not found at $BLOOD_RUST"
        step "Emitting $stage for main.blood"
        $BLOOD_RUST build --emit "$stage" -o "$BUILD_DIR/${stage}.ll" main.blood
        ;;

    # ── workflow ────────────────────────────────────────────────────────────

    run)
        src="${2:?Usage: ./build_selfhost.sh run <file.blood> [compiler] [flags...]}"
        # Detect if arg 3 is a compiler name or a flag
        _run_bin="$BUILD_DIR/first_gen"
        _run_extra_start=3
        if [[ -n "${3:-}" && "${3:0:1}" != "-" ]]; then
            _run_bin="$(resolve_compiler "$3")"
            _run_extra_start=4
        fi
        [ -f "$src" ] || die "Source file not found: $src"
        [ -f "$_run_bin" ] || die "Compiler not found: $_run_bin"
        # Collect extra flags (positions $_run_extra_start onward)
        _run_extra=()
        for (( _ri=$_run_extra_start; _ri<=$#; _ri++ )); do
            _run_extra+=("${!_ri}")
        done
        exec "$_run_bin" run "$src" --build-dir "$BUILD_DIR" --stdlib-path "$STDLIB_PATH" "${_run_extra[@]}"
        ;;

    debug-test)
        src="${2:?Usage: ./build_selfhost.sh debug-test <file.blood> [compiler]}"
        bin="$(resolve_compiler "${3:-first_gen}")"
        [ -f "$src" ] || die "Source file not found: $src"
        [ -f "$bin" ] || die "Compiler not found: $bin"
        name="$(basename "$src" .blood)"
        tmpdir="$BUILD_DIR/debug/$name"
        mkdir -p "$tmpdir"
        step "Debug-compiling $name with $bin"
        echo "  MIR dump: $tmpdir/$name.mir"
        echo "  LLVM IR:  $tmpdir/$name.ll"
        echo "  Binary:   $tmpdir/$name"
        "$bin" build "$src" \
            --dump-mir --validate-mir --no-cache \
            --build-dir "$tmpdir" \
            --stdlib-path "$STDLIB_PATH" \
            2>"$tmpdir/$name.stderr" && {
            echo "  Running..."
            "$tmpdir/$name" 2>"$tmpdir/$name.run-stderr" && ok "$name" || {
                fail "$name (exit $?)"
                echo "  Runtime stderr: $tmpdir/$name.run-stderr"
                tail -20 "$tmpdir/$name.run-stderr"
            }
        } || {
            fail "$name (compile)"
            echo "  Compile stderr:"
            tail -30 "$tmpdir/$name.stderr"
        }
        ;;

    diff)
        src="${2:?Usage: ./build_selfhost.sh diff <file.blood>}"
        [ -f "$src" ] || die "Source file not found: $src"
        [ -f "$BLOOD_RUST" ] || die "blood-rust not found at $BLOOD_RUST"
        [ -f "$BUILD_DIR/first_gen" ] || die "first_gen not found"

        tmpdir=$(mktemp -d)
        step "Comparing output: blood-rust vs first_gen"
        out1="" out2="" rc1=0 rc2=0
        out1=$("$BLOOD_RUST" run "$src" --stdlib-path "$STDLIB_PATH" 2>"$tmpdir/stderr1") || rc1=$?
        out2=$("$BUILD_DIR/first_gen" run "$src" --stdlib-path "$STDLIB_PATH" 2>"$tmpdir/stderr2") || rc2=$?
        rm -rf "$tmpdir"

        if [ "$out1" = "$out2" ] && [ "$rc1" = "$rc2" ]; then
            ok "Output matches (exit $rc1)"
        else
            fail "Output differs"
            diff <(echo "$out1") <(echo "$out2") || true
            [ "$rc1" != "$rc2" ] && printf "  Exit codes: blood-rust=%s first_gen=%s\n" "$rc1" "$rc2"
        fi
        ;;

    metrics)
        _mfile="$DIR/.logs/metrics.jsonl"
        if [ ! -f "$_mfile" ]; then
            echo "No metrics yet. Run a build first."
        else
            # Parse 2nd arg as "N" (number of recent entries to show) or
            # "all" to show the full summary. Defaults to 10.
            _n_recent="${2:-10}"
            if [[ "$_n_recent" == "all" ]]; then _n_recent=9999; fi

            echo "=== Build Metrics (last $_n_recent) ==="
            tail -n "$_n_recent" "$_mfile" | while IFS= read -r line; do
                printf "  %s\n" "$line"
            done

            _stage_stats() {
                local stage="$1"
                local mfile="$2"
                local times sizes
                times=$(grep "\"$stage\"" "$mfile" | tail -20 | sed 's/.*"wall_secs":\([0-9]*\).*/\1/')
                sizes=$(grep "\"$stage\"" "$mfile" | tail -20 | sed 's/.*"size":\([0-9]*\).*/\1/')
                if [ -z "$times" ]; then
                    echo "  $stage: (no data)"
                    return
                fi
                local n t_min t_max t_last t_avg
                n=$(echo "$times" | wc -l)
                t_min=$(echo "$times" | sort -n | head -1)
                t_max=$(echo "$times" | sort -n | tail -1)
                t_last=$(echo "$times" | tail -1)
                t_avg=$(echo "$times" | awk '{sum+=$1; n++} END {if(n>0) printf "%d", sum/n; else print "0"}')
                local s_min s_max s_last
                s_min=$(echo "$sizes" | sort -n | head -1)
                s_max=$(echo "$sizes" | sort -n | tail -1)
                s_last=$(echo "$sizes" | tail -1)
                printf "  %-10s  last=%ss  min=%ss  max=%ss  avg=%ss  (n=%s)\n" \
                    "$stage" "$t_last" "$t_min" "$t_max" "$t_avg" "$n"
                printf "  %-10s  size last=%s  min=%s  max=%s\n" \
                    "" "$s_last" "$s_min" "$s_max"

                # Delta vs first recorded in this window
                local t_first
                t_first=$(echo "$times" | head -1)
                if [ "$t_first" -gt 0 ] && [ "$t_last" != "$t_first" ]; then
                    local delta pct
                    delta=$((t_last - t_first))
                    pct=$(( (delta * 100) / t_first ))
                    printf "  %-10s  delta vs oldest in window: %+ds (%+d%%)\n" \
                        "" "$delta" "$pct"
                fi
            }

            echo ""
            echo "=== Trends (last 20 per stage) ==="
            _stage_stats "first_gen" "$_mfile"
            _stage_stats "second_gen" "$_mfile"
            _stage_stats "third_gen" "$_mfile"
            _stage_stats "first_gen_blood" "$_mfile"

            # Show the regression alarm thresholds so a user can see them
            # at a glance without digging through the script.
            echo ""
            echo "=== Regression alarm ==="
            echo "  baseline=180s  warn>252s  fail>360s"
            echo "  Set BLOOD_NO_PERF_ALARM=1 to silence."
        fi
        ;;

    # ── gate (bootstrap verify + seed update) ─────────────────────────────

    gate)
        check_zombies
        PIPELINE_START=$(date +%s)

        _gate_quick=false
        if [[ "${2:-}" == "--quick" || "${2:-}" == "-q" ]]; then
            _gate_quick=true
        fi

        if $_gate_quick; then
            # Quick gate: skip first_gen/second_gen builds + golden tests.
            # Assumes both already exist and are verified.
            if [[ ! -f "$BUILD_DIR/first_gen" ]]; then
                fail "gate --quick: $BUILD_DIR/first_gen not found. Run full gate instead."
            fi
            if [[ ! -f "$BUILD_DIR/second_gen" ]]; then
                fail "gate --quick: $BUILD_DIR/second_gen not found. Run full gate instead."
            fi
            step "Bootstrap gate (quick): third_gen byte-compare + seed update"
            do_build_third_gen
        else
            step "Bootstrap gate: full pipeline + seed update"

            do_build_blood_runtime
            do_build_first_gen "--timings"
            do_test_golden "$BUILD_DIR/first_gen" || warn "first_gen golden: some tests failed (non-fatal for gate)"
            do_build_second_gen
            do_test_golden "$BUILD_DIR/second_gen" || warn "second_gen golden: some tests failed (non-fatal for gate)"
            do_build_third_gen
        fi

        # third_gen byte-identical check is inside do_build_third_gen.
        # If we get here, the bootstrap passed. Update the seed.
        _seed_path="$REPO_ROOT/bootstrap/seed"
        _meta_path="$REPO_ROOT/bootstrap/seed.meta"
        _rt_path="$REPO_ROOT/bootstrap/libblood_runtime_blood.a"
        _commit_hash=$(git rev-parse HEAD 2>/dev/null || echo "unknown")
        cp "$BUILD_DIR/second_gen" "$_seed_path"
        _size_bytes=$(wc -c < "$_seed_path")

        # Copy runtime archive alongside seed (needed for CI bootstrap). In
        # CI, RUNTIME_A is the bootstrap copy directly, so skip the cp.
        if [ -f "$RUNTIME_A" ]; then
            if [ "$(readlink -f "$RUNTIME_A")" = "$(readlink -f "$_rt_path")" ]; then
                ok "Runtime already at $(basename "$_rt_path") ($(wc -c < "$_rt_path") bytes; source is the same file)"
            else
                cp "$RUNTIME_A" "$_rt_path"
                ok "Runtime updated: $(basename "$_rt_path") ($(wc -c < "$_rt_path") bytes)"
            fi
        else
            warn "Runtime archive not found at $RUNTIME_A — bootstrap/libblood_runtime_blood.a not updated"
        fi

        # Write seed metadata
        cat > "$_meta_path" <<SEEDMETA
commit=$_commit_hash
date=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
size=$_size_bytes
hash=$(md5sum "$_seed_path" | cut -d' ' -f1)
SEEDMETA
        ok "Seed updated: $(basename "$_seed_path") ($_size_bytes bytes, $_commit_hash)"

        printf "\n\033[1;32mBootstrap gate PASSED.\033[0m Total: %s\n" "$(elapsed_since "$PIPELINE_START")"
        printf "Seed: %s\nMeta: %s\n" "$_seed_path" "$_meta_path"
        ;;

    status)
        show_status
        ;;

    install)
        step "Installing Blood toolchain to ~/.blood/"
        install_dir="${HOME}/.blood"
        bin_dir="${install_dir}/bin"
        lib_dir="${install_dir}/lib"
        mkdir -p "$bin_dir" "$lib_dir"

        # Install compiler binary (prefer second_gen, fall back to first_gen)
        install_bin=""
        if [ -f "$BUILD_DIR/second_gen" ]; then
            install_bin="$BUILD_DIR/second_gen"
        elif [ -f "$BUILD_DIR/first_gen" ]; then
            install_bin="$BUILD_DIR/first_gen"
        else
            die "No compiler binary found. Run: build first_gen"
        fi
        cp "$install_bin" "$bin_dir/blood"
        chmod +x "$bin_dir/blood"
        ok "Compiler → $bin_dir/blood ($(basename "$install_bin"))"

        # Install runtime library (Blood runtime)
        [ -f "$RUNTIME_A" ] || die "Blood runtime not found at $RUNTIME_A (run: build blood_runtime)"
        cp "$RUNTIME_A" "$lib_dir/libblood_runtime.a"
        ok "Runtime → $lib_dir/libblood_runtime.a"

        # Install stdlib
        [ -d "$STDLIB_PATH" ] || die "stdlib not found at $STDLIB_PATH"
        rm -rf "$lib_dir/stdlib"
        cp -r "$STDLIB_PATH" "$lib_dir/stdlib"
        ok "Stdlib → $lib_dir/stdlib/"

        # Install LSP binary (if built)
        lsp_bin="${DIR}/../../src/bootstrap/target/release/blood-lsp"
        if [ -f "$lsp_bin" ]; then
            cp "$lsp_bin" "$bin_dir/blood-lsp"
            chmod +x "$bin_dir/blood-lsp"
            ok "LSP    → $bin_dir/blood-lsp"
        fi

        echo ""
        if echo "$PATH" | tr ':' '\n' | grep -qx "$bin_dir"; then
            ok "$bin_dir is already in PATH"
        else
            warn "Add to your shell profile: export PATH=\"$bin_dir:\$PATH\""
        fi
        ;;

    clean)
        step "Cleaning build artifacts"
        rm -rf "$BUILD_DIR"
        rm -rf .bisect_*
        find "${DIR}" -name ".blood-cache" -type d -exec rm -rf {} + 2>/dev/null || true
        rm -rf "${DIR}"/*.blood_objs "${DIR}"/tests/*.blood_objs
        rm -f "${DIR}"/*.ll "${DIR}"/*.o
        rm -rf "${HOME}"/.blood*/cache/
        ok "Build artifacts and caches removed"
        ;;

    clean-cache)
        step "Cleaning caches only"
        clear_all_caches
        ok "Caches removed (binaries preserved)"
        ;;

    clean-all)
        step "Cleaning build artifacts and logs"
        rm -rf "$BUILD_DIR"
        rm -rf .bisect_* .logs
        find "${DIR}" -name ".blood-cache" -type d -exec rm -rf {} + 2>/dev/null || true
        rm -rf "${DIR}"/*.blood_objs "${DIR}"/tests/*.blood_objs
        rm -f "${DIR}"/*.ll "${DIR}"/*.o
        rm -rf "${HOME}"/.blood*/cache/
        ok "Build artifacts, caches, and logs removed"
        ;;

    --help|-h)
        show_usage
        ;;

    *)
        printf "Unknown command: %s\n\n" "$1"
        show_usage
        exit 1
        ;;
esac
