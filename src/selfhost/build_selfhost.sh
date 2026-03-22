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
    local ts ir_lines
    ts=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
    ir_lines=0
    [ -f "$BUILD_DIR/${stage}.ll" ] && ir_lines=$(wc -l < "$BUILD_DIR/${stage}.ll")
    printf '{"ts":"%s","stage":"%s","size":%s,"wall_secs":%s,"ir_lines":%s}\n' \
        "$ts" "$stage" "$size" "$wall_secs" "$ir_lines" \
        >> "$DIR/.logs/metrics.jsonl"
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

        # Log rotation: keep last 20
        log_count=$(ls -1 "$LOG_DIR"/build_*.log 2>/dev/null | wc -l)
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
    cp -f "$RUNTIME_A" "$BUILD_DIR/libblood_runtime.a" 2>/dev/null || true
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

    step "Clearing build caches"
    rm -rf "$BUILD_DIR/obj" "$BUILD_DIR/debug" "$BUILD_DIR/release"
    rm -rf "${DIR}"/*.blood_objs "${DIR}"/tests/*.blood_objs
    rm -rf "${HOME}"/.blood*/cache/
    ok "Caches cleared"

    check_seed_staleness

    step "Building first_gen with $(basename "$bootstrap_compiler")"
    local start_ts rc=0
    start_ts=$(date +%s)
    $bootstrap_compiler build main.blood --no-cache --build-dir "$BUILD_DIR" $flags || rc=$?
    if [ "$rc" -ne 0 ]; then
        fail "Build failed ($(basename "$bootstrap_compiler")): $(decode_exit $rc)"
        return 1
    fi
    mv "$BUILD_DIR/debug/main" "$BUILD_DIR/first_gen"
    local fg_size fg_wall
    fg_size=$(wc -c < "$BUILD_DIR/first_gen")
    fg_wall=$(($(date +%s) - start_ts))
    ok "first_gen built ($fg_size bytes) in $(elapsed_since "$start_ts")"
    log_metric "first_gen" "$fg_size" "$fg_wall"
    copy_runtime
}

do_build_second_gen() {
    [ -f "$BUILD_DIR/first_gen" ] || die "first_gen not found. Run: ./build_selfhost.sh build first_gen"

    step "Self-compiling (first_gen → second_gen)"
    local start_ts rc=0
    start_ts=$(date +%s)
    run_with_pty "$BUILD_DIR/first_gen" build main.blood --timings --split-modules -o "$BUILD_DIR/second_gen.ll" || rc=$?
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
}

do_build_third_gen() {
    [ -f "$BUILD_DIR/second_gen" ] || die "second_gen not found. Run: ./build_selfhost.sh build second_gen"

    step "Bootstrap (second_gen → third_gen)"
    local start_ts rc=0
    start_ts=$(date +%s)
    run_with_pty "$BUILD_DIR/second_gen" build main.blood --timings --split-modules -o "$BUILD_DIR/third_gen.ll" || rc=$?
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

do_build_blood_runtime() {
    local rt_dir="$REPO_ROOT/runtime/blood-runtime"
    local rt_build="$rt_dir/build/debug"
    local fg="$BUILD_DIR/first_gen"
    [ -f "$fg" ] || die "first_gen not found. Run: ./build_selfhost.sh build first_gen"
    [ -f "$rt_dir/lib.blood" ] || die "Blood runtime source not found at $rt_dir/lib.blood"
    command -v python3 >/dev/null || die "python3 required for IR post-processing"
    command -v llc-18 >/dev/null || die "llc-18 required for object compilation"

    mkdir -p "$rt_build"

    step "Compiling Blood runtime to LLVM IR"
    "$fg" build --emit llvm-ir --no-cache --build-dir "$rt_dir/build" "$rt_dir/lib.blood"
    ok "IR generated"

    step "Post-processing IR"
    python3 "$rt_dir/build_runtime.py" "$rt_build/lib.ll" "$rt_build/lib_clean.ll"
    ok "IR post-processed"

    step "Compiling to archive"
    llc-18 -filetype=obj -relocation-model=pic "$rt_build/lib_clean.ll" -o "$rt_build/lib.o" 2>&1 \
        | grep -v 'inlinable function\|ignoring invalid debug' || true
    ar rcs "$rt_build/libblood_runtime_blood.a" "$rt_build/lib.o"
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
    if verify_output=$(opt-18 -passes=verify "$ir_file" -disable-output 2>&1); then
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

        if FileCheck-18 --input-file="$check_tmpdir/check_out.ll" "$check_src" 2>/dev/null; then
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
    [ -f "$RUNTIME_A" ] || die "Runtime library not found at $RUNTIME_A"

    step "Building ASan-instrumented binary from $ir_file"

    llvm-as-18 "$ir_file" -o "$BUILD_DIR/second_gen_asan.bc"
    ok "Assembled to bitcode"

    opt-18 -passes='module(asan-module),function(asan)' \
        "$BUILD_DIR/second_gen_asan.bc" -o "$BUILD_DIR/second_gen_asan_inst.bc"
    ok "ASan instrumentation applied"

    llc-18 "$BUILD_DIR/second_gen_asan_inst.bc" \
        -o "$BUILD_DIR/second_gen_asan.o" -filetype=obj -relocation-model=pic
    ok "Compiled to object"

    clang-18 "$BUILD_DIR/second_gen_asan.o" "$RUNTIME_A" \
        -fsanitize=address -lstdc++ -lm -lpthread -ldl -no-pie \
        -o "$BUILD_DIR/second_gen_asan"
    ok "Linked second_gen_asan ($(wc -c < "$BUILD_DIR/second_gen_asan") bytes)"

    rm -f "$BUILD_DIR/second_gen_asan.bc" "$BUILD_DIR/second_gen_asan_inst.bc" "$BUILD_DIR/second_gen_asan.o"

    printf "\n  Run with: ./build/second_gen_asan version\n"
    printf "  ASan will report memory errors with stack traces.\n"
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

    llvm-as-18 "$BUILD_DIR/reference_ir.ll" -o "$bisect_dir/ref.bc"
    llvm-as-18 "$self_ir" -o "$bisect_dir/self.bc"
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

        if ! llvm-extract-18 $extract_args \
                "$bisect_dir/self.bc" -o "$bisect_dir/extracted.bc" 2>/dev/null; then
            warn "Could not extract functions (some may be missing)"
            return 1
        fi

        local delete_args=""
        while IFS= read -r fname; do
            [ -z "$fname" ] && continue
            delete_args="$delete_args --delete=$fname"
        done < "$func_list"

        if ! llvm-extract-18 $delete_args \
                "$bisect_dir/ref.bc" -o "$bisect_dir/ref_trimmed.bc" 2>/dev/null; then
            cp "$bisect_dir/ref.bc" "$bisect_dir/ref_trimmed.bc"
        fi

        if ! llvm-link-18 \
                "$bisect_dir/ref_trimmed.bc" "$bisect_dir/extracted.bc" \
                -o "$hybrid_bc" 2>/dev/null; then
            warn "Link failed for this subset"
            return 1
        fi

        if ! llc-18 "$hybrid_bc" \
                -o "$bisect_dir/hybrid.o" -filetype=obj -relocation-model=pic 2>/dev/null; then
            warn "LLC failed for hybrid"
            return 1
        fi

        if ! clang-18 "$bisect_dir/hybrid.o" "$RUNTIME_A" \
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
  build second_gen    Self-compile first_gen → second_gen
  build third_gen     Bootstrap second_gen → third_gen + byte-compare
  build blood_runtime Compile Blood-native runtime → libblood_runtime_blood.a
  build all           Full chain: blood_runtime → first_gen → GT → second_gen → GT → third_gen
  build cargo         (legacy) Rebuild blood-rust via cargo

Test:
  test golden [compiler]    Run golden suite (default: first_gen)
  test dispatch [bin1] [bin2]     Compare dispatch behavior (default: bootstrap vs first_gen)
  test blood [compiler]           Run tests/blood-test/ (default: bootstrap)

  Compiler names: bootstrap, first_gen, second_gen, third_gen (or a path)

Diagnostics:
  verify [ir]         Structural IR verification + declaration diff + FileCheck
  ir-check [ir]       FileCheck tests only
  asan [ir]           Build ASan-instrumented binary
  bisect [ir]         Binary search for miscompiled function
  emit [stage]        Emit intermediate IR (ast|hir|mir|llvm-ir|llvm-ir-unopt)

Workflow:
  gate                Full bootstrap pipeline + update seed on success
  run <file> [bin]    Compile and run a file (default: first_gen)
  diff <file>         Compare blood-rust vs first_gen output
  status              Show compiler status, ages, processes (default command)
  install             Install toolchain to ~/.blood/{bin,lib}/
  clean               Remove build artifacts (preserves .logs)
  clean-all           Remove build artifacts and logs

Flags:
  -q, --quiet         Suppress per-test output (only failures + summary)
  --force             Ignore incremental cache, re-run all tests
USAGE
}

# ── Command dispatch ────────────────────────────────────────────────────────

case "${1:-status}" in

    # ── build ───────────────────────────────────────────────────────────────

    build)
        check_zombies
        case "${2:-}" in
            cargo)
                do_build_cargo
                ;;
            first_gen)
                do_build_first_gen "--timings"
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
                do_build_blood_runtime
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
            *)
                die "Unknown test suite: $2. Expected: golden, dispatch, blood, golden-blood"
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

            if FileCheck-18 --input-file="$tmpdir/check_out.ll" "$check_src" 2>/dev/null; then
                ok "$check_name"
                fc_pass=$((fc_pass + 1))
            else
                fail "$check_name"
                FileCheck-18 --input-file="$tmpdir/check_out.ll" "$check_src" 2>&1 | head -10 || true
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
        src="${2:?Usage: ./build_selfhost.sh run <file.blood> [compiler]}"
        bin="$(resolve_compiler "${3:-first_gen}")"
        [ -f "$src" ] || die "Source file not found: $src"
        [ -f "$bin" ] || die "Compiler not found: $bin"
        exec "$bin" run "$src" --build-dir "$BUILD_DIR" --stdlib-path "$STDLIB_PATH"
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
        local mfile="$DIR/.logs/metrics.jsonl"
        if [ ! -f "$mfile" ]; then
            echo "No metrics yet. Run a build first."
        else
            echo "=== Build Metrics (last 10) ==="
            tail -10 "$mfile" | while IFS= read -r line; do
                printf "  %s\n" "$line"
            done
            echo ""
            echo "=== Trends ==="
            echo "  first_gen sizes:  $(grep '"first_gen"' "$mfile" | tail -5 | sed 's/.*"size":\([0-9]*\).*/\1/' | tr '\n' ' ')"
            echo "  second_gen sizes: $(grep '"second_gen"' "$mfile" | tail -5 | sed 's/.*"size":\([0-9]*\).*/\1/' | tr '\n' ' ')"
            echo "  second_gen times: $(grep '"second_gen"' "$mfile" | tail -5 | sed 's/.*"wall_secs":\([0-9]*\).*/\1s/' | tr '\n' ' ')"
        fi
        ;;

    # ── gate (bootstrap verify + seed update) ─────────────────────────────

    gate)
        check_zombies
        PIPELINE_START=$(date +%s)

        step "Bootstrap gate: full pipeline + seed update"

        do_build_blood_runtime
        do_build_first_gen "--timings"
        do_test_golden "$BUILD_DIR/first_gen" || warn "first_gen golden: some tests failed (non-fatal for gate)"
        do_build_second_gen
        do_test_golden "$BUILD_DIR/second_gen" || warn "second_gen golden: some tests failed (non-fatal for gate)"
        do_build_third_gen

        # third_gen byte-identical check is inside do_build_third_gen.
        # If we get here, the bootstrap passed. Update the seed.
        _seed_path="$REPO_ROOT/bootstrap/seed"
        _meta_path="$REPO_ROOT/bootstrap/seed.meta"
        _commit_hash=$(git rev-parse HEAD 2>/dev/null || echo "unknown")
        cp "$BUILD_DIR/second_gen" "$_seed_path"
        _size_bytes=$(wc -c < "$_seed_path")

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
        local install_bin=""
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
