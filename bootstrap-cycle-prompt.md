# Task: Execute Full Compiler Bootstrap Cycle for the Blood Programming Language

## Objective

Perform a three-stage bootstrap validation of the Blood compiler to achieve a self-hosted, bootstrap-free compiler (the "Golden Image"). A successful bootstrap proves the compiler can reproduce itself without any dependency on the original bootstrap toolchain.

## Definitions

| Symbol | Meaning |
|--------|---------|
| `C_B` | The bootstrap compiler (the current Rust-based Blood compiler) |
| `S` | The complete Blood compiler source tree, written in Blood — including the Blood standard library, runtime, and all Blood-language dependencies required to build the compiler. Pin the exact commit hash after completing Phase 0 Steps 1–3 (remediation). The determinism gate (Step 4) and all subsequent phases use the pinned source. |
| `V_n` | The compiler binary produced at stage `n` |

> **Command template convention:** In command templates, `$SOURCE_DIR` is the root of the compiler source tree (e.g., `blood-std/std/compiler`) and `main.blood` is the compiler's entry point. `S` in prose (e.g., "compile `S`") refers to the full source tree; `$SOURCE_DIR` + `main.blood` is its concrete realization in shell commands.

### Compilation Flow Summary

The three-stage bootstrap with determinism gates requires **7 compilations** (not 3). The stage reuse optimization absorbs stage compilations into their preceding determinism gates, saving 3 redundant builds:

```
Phase 0:  C_B(S) → det_cb_a     ─── determinism gate ───  C_B(S) → det_cb_b
                    │                                                │
                    ╰─ if det_cb_a = det_cb_b: reuse as blood_v1    ╯

Phase 1:  V1(S)  → det_v1a      ─── determinism gate ───  V1(S)  → det_v1b
                    │                                                │
                    ╰─ if det_v1a = det_v1b: reuse as blood_v2      ╯

          V2(S)  → det_v2a      ─── determinism gate ───  V2(S)  → det_v2b
                    │                                                │
                    ╰─ if det_v2a = det_v2b: reuse as blood_v3      ╯

Phase 2:  Compare blood_v2 = blood_v3?  (fixed-point test)

Phase 3:  (no compilations — test suite, audits, and performance checks only)

Phase 4:  V3(S)  → v3_verify    ─── post-promotion round-trip
```

| Compilation | Compiler | Output | Purpose |
|-------------|----------|--------|---------|
| 1 | `C_B` | `blood_det_cb_a` | Det. gate A (reused as `blood_v1`) |
| 2 | `C_B` | `blood_det_cb_b` | Det. gate B |
| 3 | `V1` | `blood_det_v1a` | Det. gate A (reused as `blood_v2`) |
| 4 | `V1` | `blood_det_v1b` | Det. gate B |
| 5 | `V2` | `blood_det_v2a` | Det. gate A (reused as `blood_v3`) |
| 6 | `V2` | `blood_det_v2b` | Det. gate B |
| 7 | `V3` | `blood_v3_verify` | Post-promotion round-trip |

## System Requirements

- **Bootstrap Compiler (`C_B`):** The current stable Rust-based Blood compiler, verified as passing the existing test suite.
- **Source Code (`S`):** The complete source tree for the Blood compiler. This includes all Blood-language transitive dependencies (standard library, runtime, codegen support libraries, etc.) — anything written in Blood that is required to produce a working compiler binary. Phase 0 may modify `S` to remediate non-determinism. After Phase 0 is complete, pin the exact commit hash and freeze the source. No source modifications are permitted during Phases 1–4. If a later phase requires a source fix, apply it, re-pin the commit hash, and restart from Phase 0.
- **Test Harness:** An external test runner that accepts a compiler path as input (e.g., `bash compiler-rust/tests/ground-truth/run_tests.sh <compiler_path>`). The harness must be independent of both `C_B` and the Blood compiler under test, ensuring test results are not influenced by which compiler is being evaluated. Document the harness version and commit hash.
- **Platform Tooling:**
  - `sha256sum` (Linux) or `Get-FileHash -Algorithm SHA256` (PowerShell)
  - `diff` for text comparison
  - `ldd` (Linux) or `otool -L` (macOS) for dependency auditing
  - `objdump`, `llvm-readobj`, or equivalent for binary inspection
  - `strip` for producing stripped binaries for comparison (if needed)
- **Environment:** A clean, reproducible build environment. Document the OS, kernel version, toolchain versions, linker, and all relevant environment variables. Use a container or VM snapshot if available. Set `LC_ALL=C` to prevent locale-dependent string ordering from introducing non-determinism.

### Resource Bounds

- **Per-stage timeout:** Set a wall-clock timeout for each compilation stage (recommended: 10× the expected build time). If a stage exceeds this limit, treat it as a build failure and investigate.
- **Timing measurement:** Both `C_B` and the self-hosted compiler support `--timings` (per-phase durations to stderr), though they report different phase granularity — `C_B` reports ~13 phases while the self-hosted reports 4 high-level phases. Additionally, wrap all compilation commands with `/usr/bin/time -v` (Linux) or `time -l` (macOS) to capture wall-clock time and peak RSS uniformly. Record both `/usr/bin/time` output (for the Build Manifest) and `--timings` output (for Phase 3E per-phase analysis).
- **Memory limit:** Document available RAM. The Blood compiler currently uses ~29 GB RSS during self-compilation; ensure at least 48 GB available (the OS, kernel caches, linker, and LLVM backend consume additional memory beyond the compiler's RSS). If running on a machine with exactly 32 GB, expect swap pressure — disable swap or provision more RAM to avoid non-deterministic OOM behavior. If any stage OOMs, record the failure and investigate before retrying with increased resources.
- **Disk space:** Ensure sufficient space for all intermediate artifacts (`V1`, `V2`, `V3`, IR dumps, test outputs). Estimate 5× the size of a single compiler binary.

### Frozen Build Invocation

> **Rationale:** If compiler flags differ between stages, bitwise identity is impossible even with a perfectly deterministic compiler. All stages must use an identical invocation.

Before beginning, define and freeze the exact compilation command template. All stages must use this template verbatim, substituting only the compiler path and output path:

```bash
cd <SOURCE_DIR> && <COMPILER> build main.blood -o <OUTPUT> [frozen flags]
```

Document the frozen flags (optimization level, cache behavior, etc.) in the Environment Record. No flag may vary between stages unless explicitly justified and documented.

**Example:**
```bash
# Frozen invocation template — do not modify during bootstrap
REPO_ROOT="$(cd "$(dirname "$0")" && pwd)"   # or set to absolute path of blood/ repo root
C_B="$REPO_ROOT/compiler-rust/target/release/blood"
SOURCE_DIR="$REPO_ROOT/blood-std/std/compiler"
BLOOD_FLAGS="--release --no-cache"

# Frozen environment — runtime artifacts must be identical across all stages.
# Paths must be absolute; relative paths would resolve against $SOURCE_DIR (the cwd
# during builds) where these files do not exist.
export BLOOD_RUNTIME="$REPO_ROOT/compiler-rust/runtime/runtime.o"
export BLOOD_RUST_RUNTIME="$REPO_ROOT/compiler-rust/target/release/libblood_runtime.a"

cd $SOURCE_DIR && <COMPILER> build main.blood -o <OUTPUT> $BLOOD_FLAGS
```

> **Note:** `--release` enables LLVM optimizations and strips symbols during linking (`-s` to the linker). `--no-cache` forces a full recompilation at every stage, eliminating cache-dependent non-determinism. The Blood compiler is single-threaded, so no parallelism flag is needed. The frozen flags must be supported by **both** `C_B` and the self-hosted compiler. **Verify** that both compilers actually support every frozen flag (`--release`, `--no-cache`) before freezing — if a flag is silently ignored or rejected by one compiler, builds will diverge for reasons unrelated to codegen. Similarly, verify `--emit llvm-ir-unopt` support on `C_B` before relying on it in the Phase 2 Failure Protocol. **Verify** that both compilers actually strip binaries under `--release` before relying on the "already stripped" assumption in Phase 0, Step 3 — if only one compiler strips, the comparison strategy must account for the difference (e.g., apply `strip -s` uniformly).
>
> **Runtime environment:** `BLOOD_RUNTIME` and `BLOOD_RUST_RUNTIME` specify the runtime object files linked into every compiled binary. These must be frozen alongside the compiler flags — if a different runtime `.o` is used between stages, bitwise identity will fail for reasons unrelated to the compiler itself. Export them before the first build and do not modify them.
>
> **Optimization level:** Both `C_B` and the self-hosted compiler interpret `--release` as `-O2` (LLVM default optimization). Since both use the same optimization level, optimization-level differences are **not** a source of `V1 ≠ V2` divergence. Any `V1 ≠ V2` divergence is attributable to codegen differences between the two compilers (different IR generation, different lowering decisions), not optimization settings.
>
> **`--emit llvm-ir` caveat:** `C_B`'s `--emit llvm-ir` applies `-O3` (LLVM aggressive) to the IR before printing, which does **not** match the `-O2` used during actual `--release` builds. When capturing IR in the Phase 2 failure protocol, be aware that `C_B`'s emitted IR is more aggressively optimized than what produced the binary. The self-hosted compiler's IR output is unoptimized (optimization happens later in `llc-18`), so the same caveat does not apply to `V1`/`V2`/`V3` IR. For precise IR-to-binary correspondence from `C_B`, use `--emit llvm-ir-unopt` instead — this emits unoptimized IR, though neither exactly matches the O2-optimized form used in the actual binary.

### Artifact Scope

If the compiler build process produces multiple distinct artifacts — for example, a compiler binary *and* a separately compiled standard library (`.a`, `.so`, `.dylib`) — then **all** shipping artifacts are subject to the same determinism gates, bitwise identity checks, and hash tracking described in this document. The Build Manifest must include a row for each artifact at each stage.

### IR Output Conventions

The two compiler families expose LLVM IR differently. This convention applies to all IR capture commands throughout this document (Phase 2 Failure Protocol, Phase 3E performance measurement, etc.).

- **`C_B` (Rust bootstrap):** Use `--emit llvm-ir` to capture IR without linking. With `-o`, the IR is written to the specified path; without `-o`, it is printed to stdout. **Caveat:** `--emit llvm-ir` applies O3 optimization to the IR, which does not match the O2 used during `--release` builds (see "Optimization level" note above). Use `--emit llvm-ir-unopt` for unoptimized IR.
- **`V1`/`V2`/`V3` (self-hosted):** Two options for capturing IR:
  - **`--emit=llvm-ir`** (preferred for IR-only capture): Writes unoptimized LLVM IR to the `-o` path and stops — no object file or binary is produced. Also accepts space-separated syntax: `--emit llvm-ir`.
  - **Normal `build` with `-o file.ll`**: The compiler writes LLVM IR to the `.ll` file as part of its normal build pipeline, then derives object and binary paths by stripping the `.ll` extension (`ir_from_v1.ll` → object `ir_from_v1.o`, binary `ir_from_v1`). The `.ll` file persists on disk — `llc-18` reads it but does not overwrite it.
  - **Important:** When using normal `build`, always include the `.ll` extension in the `-o` value; without it, the final binary overwrites the IR file (both share the same path). `/dev/null` is not suitable as the `-o` path. The self-hosted compiler also supports `--emit=obj` to stop after object file generation (no linking).

### Artifact Hygiene

All output artifacts (`blood_v*`, `blood_det_*`, `ir_from_*`, `asm_from_*`) are written to `$SOURCE_DIR` via relative `-o` paths. Before starting Phase 0, clean any stale artifacts from prior runs to prevent accidentally hashing an old file if a build crashes:

```bash
cd $SOURCE_DIR && rm -f blood_v* blood_det_* ir_from_* asm_from_* results_*.txt timings_*.txt
```

Re-run this cleanup before each restart if a phase fails and requires restarting from Phase 0.

### Diagnostic Toolkit

The following tools in `tools/` are referenced throughout this document for failure investigation. All accept `--help` for full usage. Environment variables `BLOOD_REF`, `BLOOD_TEST`, `BLOOD_RUNTIME`, and `BLOOD_RUST_RUNTIME` are shared across tools and default to the standard repository paths.

| Tool | Purpose | Primary Bootstrap Use |
|------|---------|----------------------|
| `tools/difftest.sh` | Per-function IR diff and behavioral comparison between two compilers | Phase 2 failure triage, Phase 3A attribution |
| `tools/minimize.sh` | Delta-debug reduction of failing `.blood` files (crash, wrong-output, compile-fail, compile-crash) | Isolating self-compile failures (Phase 1), minimal reproductions |
| `tools/phase-compare.sh` | 4-phase divergence localization (compilation, MIR, LLVM IR, behavior) | Phase 3A divergence attribution, narrowing root cause |
| `tools/memprofile.sh` | Memory profiling (summary, RSS sampling, valgrind massif, side-by-side comparison) | Phase 1 OOM triage, Phase 3E performance regression |
| `tools/asan-selfcompile.sh` | AddressSanitizer-instrumented build pipeline for memory safety debugging | Phase 1 crash triage, UB detection in any stage |
| `tools/validate-all-mir.sh` | Pre-codegen MIR structural validation gate | Phase 3B correctness verification |
| `tools/track-regression.sh` | Ground-truth regression tracking with saved baselines | Phase 3A test comparison |
| `tools/filecheck-audit.sh` | FileCheck test coverage audit and gap identification | Phase 3B coverage gap analysis |
| `tools/FAILURE_LOG.md` | Historical bug database — search before debugging any new issue | All phases |

**External tools** expected on `$PATH` for failure investigation:

| Tool | Purpose |
|------|---------|
| `gdb` or `lldb` | Crash forensics (stack traces), infinite loop detection (attach to hung process) |
| `strace` | Syscall-level hang and I/O diagnosis |
| `perf` | CPU profiling and flame graph generation for performance regression analysis |
| `valgrind --tool=massif` | Heap profiling (invoked via `tools/memprofile.sh --massif`) |
| `objdump` / `readelf` / `nm` | Binary inspection, ABI verification, symbol audit |

---

## Phase 0: Deterministic Build Verification

> **Rationale:** If the compiler embeds timestamps, absolute paths, random seeds, pointer-derived ordering, or build-environment metadata into its output, Stage 2 and Stage 3 binaries will never be bitwise identical — producing false negatives that block the entire bootstrap.

### Steps

1. **Audit the compiler source (`S`) for non-deterministic artifacts:**
   - Search for timestamp injection (e.g., `__DATE__`, `__TIME__`, `chrono::now()`, `SystemTime`).
   - Search for absolute path embedding (e.g., build directory paths baked into debug info or error messages).
   - Search for iteration over unordered collections (e.g., `HashMap` in Rust) that could produce non-deterministic output ordering in IR or codegen.
   - Search for random seed usage without fixed initialization.
   - Search for thread-count-sensitive parallelism in compilation passes that could produce ordering differences.
   - Search for ASLR/PIE-sensitive codegen paths — any code that derives values from runtime load addresses (function pointers used as sort keys, pointer-to-integer casts influencing output ordering) can produce non-determinism that manifests intermittently depending on address space layout.
   - Search for directory listing/iteration that could produce non-deterministic file processing order (e.g., `readdir` results used without sorting for module resolution or multi-file compilation).
   - Search for filesystem `mtime`-sensitive code paths — any logic that reads file modification times for purposes other than caching (e.g., stale-object detection, conditional recompilation guards) can produce non-determinism if filesystem timestamps differ between runs. Note that `--no-cache` mitigates cache-related `mtime` sensitivity, but non-cache uses of `mtime` must be audited separately.

   **Audit procedure — concrete search patterns for `S` (Blood source):**
   ```bash
   # Timestamp / time-dependent code
   grep -rn 'SystemTime\|time\.\|chrono\|epoch\|__DATE__\|__TIME__\|timestamp' $SOURCE_DIR/*.blood

   # Absolute path embedding
   grep -rn 'env\.\|home\|/usr\|/tmp\|cwd\|current_dir\|absolute' $SOURCE_DIR/*.blood

   # HashMap iteration (primary non-determinism vector — see FAILURE_LOG.md BUG-003)
   grep -rn 'HashMap' $SOURCE_DIR/*.blood
   # For each hit: verify iteration order does not flow into IR output.
   # Safe: lookup-only (get, contains_key, insert).
   # Unsafe: iter, keys, values flowing into emit_*, write_*, or any output-ordered context.

   # Pointer-derived ordering
   grep -rn 'as_ptr\|ptr_to_int\|addr\|as_usize.*sort\|as_usize.*cmp' $SOURCE_DIR/*.blood

   # Random seeds
   grep -rn 'rand\|seed\|random\|shuffle' $SOURCE_DIR/*.blood

   # Directory iteration (module resolution)
   grep -rn 'readdir\|read_dir\|list_dir\|glob' $SOURCE_DIR/*.blood
   ```

   **Audit procedure — `C_B` (Rust bootstrap):**
   ```bash
   # HashMap iteration flowing into codegen output
   grep -rn 'HashMap.*iter\|\.keys()\|\.values()' compiler-rust/bloodc/src/**/*.rs
   # Cross-reference hits with any write!/format!/emit that produces IR or binary content.

   # Verify BUG-003 fix (deterministic HashMap iteration) is present:
   grep -rn 'BTreeMap\|sort\|sorted' compiler-rust/bloodc/src/codegen/ | head -20
   ```

   **Validation — confirm audit via IR diff:**
   ```bash
   cd $SOURCE_DIR && $C_B build main.blood --emit llvm-ir-unopt -o /tmp/det_ir_a.ll $BLOOD_FLAGS
   cd $SOURCE_DIR && $C_B build main.blood --emit llvm-ir-unopt -o /tmp/det_ir_b.ll $BLOOD_FLAGS
   diff /tmp/det_ir_a.ll /tmp/det_ir_b.ll
   ```
   If `diff` reports differences, the IR shows which functions have non-deterministic output. Use `tools/difftest.sh --ir` with function-level splitting to isolate the divergent function(s).

2. **Remediate any findings** before proceeding. Acceptable fixes include:
   - Replacing timestamps with a fixed epoch or omitting them entirely.
   - Using relative paths or stripping path info from release builds.
   - Replacing unordered iteration with deterministic alternatives (`BTreeMap`, sorted output).
   - Fixing parallel passes to produce deterministic output regardless of scheduling.
   - Replacing pointer-derived ordering with explicit sequence numbers or stable identifiers.

3. **Decide on binary comparison strategy:**
   - If `--release` is among the frozen flags, binaries are **already fully stripped** (`-s` is passed to the linker), and no additional stripping is needed. Compare binaries directly.
   - If building without `--release`, choose one of:
     - **Preferred:** Compare fully stripped binaries (`strip -s`) to eliminate debug symbol layout differences that do not affect correctness.
     - **Alternative:** Compare unstripped binaries if debug info determinism is also a project goal.
   - Document the chosen strategy and apply it consistently across all phases.
   - **Verify strip behavior** before relying on the "already stripped" assumption:
     ```bash
     # Build one test binary with each compiler under --release
     cd $SOURCE_DIR && $C_B build main.blood -o strip_test_cb $BLOOD_FLAGS
     cd $SOURCE_DIR && ./blood_v1 build main.blood -o strip_test_v1 $BLOOD_FLAGS  # requires V1 from a prior run or skip until Stage 1
     file strip_test_cb strip_test_v1   # both should say "stripped"
     rm -f strip_test_cb strip_test_v1
     ```
     If only one compiler strips, apply `strip -s` uniformly to all artifacts before comparison.

4. **Determinism gate — bootstrap compiler (`C_B`):** Compile `S` with `C_B` twice in succession under identical conditions (same working directory, same environment variables, same frozen flags, same filesystem state). Compare the two outputs using the chosen comparison strategy. If they are not identical, determinism has not been achieved in `C_B`'s compilation of Blood. Do not proceed until this gate passes.

   > **Note:** If this gate fails, the non-determinism could originate in `C_B`'s own code generation (Rust-side) or in the Blood source (`S`). To distinguish:
   >
   > 1. **Diff the IR, not just the binary:** Capture unoptimized IR from both builds and diff:
   >    ```bash
   >    cd $SOURCE_DIR && $C_B build main.blood --emit llvm-ir-unopt -o /tmp/det_cb_ir_a.ll $BLOOD_FLAGS
   >    cd $SOURCE_DIR && $C_B build main.blood --emit llvm-ir-unopt -o /tmp/det_cb_ir_b.ll $BLOOD_FLAGS
   >    diff /tmp/det_cb_ir_a.ll /tmp/det_cb_ir_b.ll | head -80
   >    ```
   > 2. **If IR differs:** The non-determinism is in `C_B`'s codegen — it emits different IR from the same source on successive runs. Use `tools/difftest.sh --ir` on a smaller Blood file with `BLOOD_REF=$C_B BLOOD_TEST=$C_B` to narrow which function(s) diverge. Check `C_B`'s Rust source for HashMap iteration in codegen output paths (see Step 1 audit procedure). Fix in `compiler-rust/`, rebuild `C_B`, re-verify `C_B` passes its own test suite, and re-run this gate.
   > 3. **If IR is identical but binary differs:** The non-determinism is below the IR layer — in `llc-18`, the linker, or the object file layout. Lower both IR files to assembly with `llc-18` and diff. If assembly also matches, the linker is the source; investigate linker flags (e.g., `--hash-style=sysv`, `--build-id=none`) for deterministic output.

   ```bash
   cd $SOURCE_DIR && $C_B build main.blood -o blood_det_cb_a $BLOOD_FLAGS
   cd $SOURCE_DIR && $C_B build main.blood -o blood_det_cb_b $BLOOD_FLAGS
   sha256sum blood_det_cb_a blood_det_cb_b
   ```

### Exit Criteria

- Two consecutive builds of `S` using `C_B` produce identical artifacts under the chosen comparison strategy.

> **Optimization (Stage Reuse):** If the determinism gate passes, copy `blood_det_cb_a` to `blood_v1` (`cp blood_det_cb_a blood_v1`). This avoids a redundant recompilation — the determinism gate already produced `C_B(S)` under frozen conditions. Use `cp` (not `mv` or `ln -s`) so both the determinism gate artifact and the stage artifact are preserved for the Build Manifest and archival. Apply this optimization consistently: when any determinism gate passes, copy the first of its two builds as the next stage's output.

---

## Phase 1: Three-Stage Build

### Stage 1 — Cross-Compiled

```
C_B compiles S → produces V1 (blood_v1)
```

- `V1` is a Blood compiler, but its binary is shaped by `C_B`'s code generation. It is **not** expected to be bitwise identical to later stages.
- If the Phase 0 stage reuse optimization was applied, `blood_v1` already exists (copied from `blood_det_cb_a`). Skip recompilation and proceed to the V1 determinism gate.

**Immediately after Stage 1, record:**
```bash
sha256sum blood_v1
```

### Determinism Gate — `V1`

Before proceeding to Stage 2, verify that `V1` itself produces deterministic output. Both builds must use identical conditions (same working directory, same environment variables, same frozen flags):

```bash
cd $SOURCE_DIR && ./blood_v1 build main.blood -o blood_det_v1a $BLOOD_FLAGS
cd $SOURCE_DIR && ./blood_v1 build main.blood -o blood_det_v1b $BLOOD_FLAGS
sha256sum blood_det_v1a blood_det_v1b
```

If the hashes differ, `V1` has a non-determinism bug. This could be a bug in `S` (the Blood source) or a miscompilation by `C_B` that causes `V1` to behave non-deterministically. To distinguish:

   1. **Diff the IR from both V1 builds:**
      ```bash
      cd $SOURCE_DIR && ./blood_v1 build main.blood --emit=llvm-ir -o /tmp/det_v1_ir_a.ll $BLOOD_FLAGS
      cd $SOURCE_DIR && ./blood_v1 build main.blood --emit=llvm-ir -o /tmp/det_v1_ir_b.ll $BLOOD_FLAGS
      diff /tmp/det_v1_ir_a.ll /tmp/det_v1_ir_b.ll | head -80
      ```
   2. **If IR differs:** The non-determinism is in `S`'s codegen logic (since both builds use the same V1 binary, the binary is not the variable — the source logic is). Re-run the Phase 0, Step 1 audit against `S` with focus on HashMap iteration flowing into IR emission (`codegen*.blood` files). Use `tools/difftest.sh --ir` with `BLOOD_TEST=./blood_v1` on individual ground-truth tests to find which functions produce non-deterministic IR.
   3. **If IR is identical but binaries differ:** The non-determinism is in the `llc-18`/linker layer. Test with deterministic linker flags. This is unlikely to be a `C_B` miscompilation.
   4. **If you suspect C_B miscompilation:** Build V1 with ASan instrumentation (`tools/asan-selfcompile.sh`) and run the determinism gate under ASan. Memory corruption in V1 (caused by C_B miscompilation) can manifest as intermittent non-determinism.

Fix the root cause, then restart from Phase 0. Note that if the root cause is a `C_B` miscompilation (not a bug in `S`), the fix must be applied to the Rust bootstrap compiler — a different workflow than patching `S`. Rebuild `C_B` after fixing, re-verify `C_B` passes its own test suite, then restart.

**Stage Reuse Optimization:** If the V1 determinism gate passes, copy `blood_det_v1a` to `blood_v2` (`cp blood_det_v1a blood_v2`) and skip the Stage 2 compilation. The determinism gate already produced `V1(S)` under frozen conditions — recompiling would yield a bitwise-identical result.

### Stage 2 — First Self-Compiled

```
V1 compiles S → produces V2 (blood_v2)
```

- `V2` is the first binary produced entirely by Blood's own code generation.
- If `V1` crashes, OOMs, or produces a non-functional binary when compiling `S`, this indicates a fundamental self-hosting gap — either a bug in `S` or a miscompilation by `C_B` that renders `V1` unable to self-compile. Triage as a blocking defect. Do not proceed; investigate using the following procedure:

   **If V1 crashes (SIGSEGV, SIGABRT) during self-compilation:**
   ```bash
   # 1. Get a stack trace
   gdb -batch -ex run -ex bt -ex quit --args ./blood_v1 build main.blood -o /tmp/v1_crash_test.ll $BLOOD_FLAGS

   # 2. Build an ASan-instrumented version to detect memory corruption
   tools/asan-selfcompile.sh --compiler ./blood_v1 --test "./blood_v1 check main.blood"

   # 3. Verify V1 works on smaller programs to localize the failure
   BLOOD_TEST=./blood_v1 tools/track-regression.sh  # full ground-truth suite under V1

   # 4. Search tools/FAILURE_LOG.md for similar crash signatures
   ```

   **If V1 runs out of memory (OOM):**
   ```bash
   # 1. Profile memory to identify which phase exhausts RAM
   BLOOD_TEST=./blood_v1 tools/memprofile.sh $SOURCE_DIR/main.blood --test-only --sample

   # 2. Compare against reference compiler baseline
   BLOOD_TEST=./blood_v1 tools/memprofile.sh $SOURCE_DIR/main.blood --compare

   # 3. For detailed heap analysis if the above is insufficient
   BLOOD_TEST=./blood_v1 tools/memprofile.sh $SOURCE_DIR/main.blood --test-only --massif
   ```

   **If V1 produces a non-functional binary (`blood_v2` crashes or misbehaves):**
   ```bash
   # 1. Verify the binary exists and is well-formed
   file blood_v2
   ldd blood_v2

   # 2. Test the binary on trivial input
   ./blood_v2 --version 2>/dev/null || echo "version command failed"
   ./blood_v2 check $REPO_ROOT/compiler-rust/tests/ground-truth/hello.blood

   # 3. Use difftest to compare V1's IR output against C_B's on the full source
   BLOOD_TEST=./blood_v1 tools/difftest.sh $SOURCE_DIR/main.blood --ir --summary-only

   # 4. Use phase-compare on a small test to narrow the divergent phase
   BLOOD_TEST=./blood_v1 tools/phase-compare.sh $REPO_ROOT/compiler-rust/tests/ground-truth/hello.blood
   ```

**Immediately after Stage 2, record:**
```bash
sha256sum blood_v2
```

### Determinism Gate — `V2`

Before proceeding to Stage 3, verify that `V2` also produces deterministic output under identical conditions:

```bash
cd $SOURCE_DIR && ./blood_v2 build main.blood -o blood_det_v2a $BLOOD_FLAGS
cd $SOURCE_DIR && ./blood_v2 build main.blood -o blood_det_v2b $BLOOD_FLAGS
sha256sum blood_det_v2a blood_det_v2b
```

If the hashes differ, `V2` has a non-determinism bug that `V1` may not have had. Since `V2` was produced by `V1` (not `C_B`), this is likely a codegen bug in `S` that only manifests when `S` is compiled by itself — or a miscompilation by V1 that causes V2 to behave non-deterministically. Investigate:

   1. **Confirm V1 passed its determinism gate** (it must have, or you wouldn't be here). This means `S` produces deterministic output when compiled by `C_B`, but non-deterministic output when compiled by `V1` — pointing to a miscompilation that V1 introduced.
   2. **Diff V2's IR from both builds** (same procedure as the V1 gate above):
      ```bash
      cd $SOURCE_DIR && ./blood_v2 build main.blood --emit=llvm-ir -o /tmp/det_v2_ir_a.ll $BLOOD_FLAGS
      cd $SOURCE_DIR && ./blood_v2 build main.blood --emit=llvm-ir -o /tmp/det_v2_ir_b.ll $BLOOD_FLAGS
      diff /tmp/det_v2_ir_a.ll /tmp/det_v2_ir_b.ll | head -80
      ```
   3. **Compare V1 vs V2 IR on a small test** to find the miscompilation:
      ```bash
      BLOOD_TEST=./blood_v1 tools/difftest.sh $REPO_ROOT/compiler-rust/tests/ground-truth/<suspect>.blood --ir --verbose
      BLOOD_TEST=./blood_v2 tools/difftest.sh $REPO_ROOT/compiler-rust/tests/ground-truth/<suspect>.blood --ir --verbose
      ```
   4. **Use `tools/phase-compare.sh`** on the divergent function's source file to localize which compilation phase introduces the difference.

   Fix the root cause in `S`, then restart from Phase 0.

**Stage Reuse Optimization:** If the V2 determinism gate passes, copy `blood_det_v2a` to `blood_v3` (`cp blood_det_v2a blood_v3`) and skip the Stage 3 compilation. Same reasoning as the V1 optimization above.

### Stage 3 — Fixed-Point Validation

```
V2 compiles S → produces V3 (blood_v3)
```

- `V3` should be bitwise identical to `V2`. This proves the compiler has reached a **fixed point** — it reproduces itself perfectly.

**Immediately after Stage 3, record:**
```bash
sha256sum blood_v3
```

### Build Timeout Protocol

If any stage exceeds the configured timeout:

1. Record the stage, elapsed time, and system resource state (CPU, memory, disk).
2. Kill the process.
3. **Diagnose the hang** using the following triage procedure:

   **Step 1 — Classify the hang (before killing the process):**
   ```bash
   # Check CPU vs I/O state
   ps -p $PID -o pid,state,pcpu,rss,wchan
   # State 'R' + high CPU → infinite loop
   # State 'D' → I/O wait (blocked on disk/network)
   # State 'S' + 0% CPU → deadlock or blocked on input
   ```

   **Step 2a — If CPU-bound (suspected infinite loop):**
   ```bash
   # Attach debugger and get stack trace without killing the process
   gdb -batch -p $PID -ex 'thread apply all bt' -ex detach -ex quit 2>/dev/null
   # If the trace shows the same function repeatedly, that function contains the loop.
   # For deeper analysis, collect a CPU profile:
   perf record -p $PID -g -- sleep 10 && perf report --stdio | head -60
   ```

   **Step 2b — If memory-bound (RSS growing toward limit):**
   ```bash
   # Monitor RSS growth
   while kill -0 $PID 2>/dev/null; do
       grep -E 'VmRSS|VmSwap' /proc/$PID/status; sleep 5
   done
   # If RSS grows unboundedly, use memprofile to identify the phase:
   tools/memprofile.sh $SOURCE_DIR/main.blood --test-only --sample
   ```

   **Step 2c — If I/O-bound or idle:**
   ```bash
   # Trace syscalls to see what the process is waiting on
   strace -p $PID -e trace=read,write,open,close -c   # summary mode
   strace -p $PID -e trace=read,write -f 2>&1 | head -40   # live trace
   ```

4. Do not proceed until the root cause is identified and resolved.

---

## Phase 2: Bitwise Identity Verification

### Primary Gate

1. Compare the SHA-256 hashes of `V2` and `V3` (using stripped or unstripped binaries per the strategy chosen in Phase 0, Step 3).
2. **If identical:** The fixed-point property holds. Proceed to Phase 3.
3. **If not identical:** The bootstrap has **failed**. Execute the failure protocol below.

### Failure Protocol

1. **Do not proceed** to Phase 3.
2. Emit the LLVM IR produced by `C_B`, `V1`, and `V2` when each compiles `S`. Since `V2` is the output of "`V1` compiling `S`" and `V3` is the output of "`V2` compiling `S`," diffing the IR from `V1` and `V2` reveals why they — as compilers — produce different binaries from the same source. The IR from `C_B` serves as a reference baseline for triangulation: if `ir_from_v1.ll` diverges from both `ir_from_cb.ll` and `ir_from_v2.ll`, the issue likely originates in how `C_B` compiled `V1`. (See "IR Output Conventions" in System Requirements for how each compiler exposes IR.)

   ```bash
   # C_B: --emit llvm-ir-unopt captures unoptimized IR without linking.
   # (--emit llvm-ir applies O3 optimization, which would not match the O2 used in actual builds.)
   cd $SOURCE_DIR && $C_B build main.blood --emit llvm-ir-unopt -o ir_from_cb.ll $BLOOD_FLAGS
   # V1/V2: --emit=llvm-ir captures unoptimized IR without linking (preferred over -o .ll build).
   cd $SOURCE_DIR && ./blood_v1 build main.blood --emit=llvm-ir -o ir_from_v1.ll $BLOOD_FLAGS
   cd $SOURCE_DIR && ./blood_v2 build main.blood --emit=llvm-ir -o ir_from_v2.ll $BLOOD_FLAGS
   ```
3. Diff the IR outputs:
   ```bash
   diff ir_from_v1.ll ir_from_v2.ll > ir_diff.txt
   ```
4. If the IR is identical, the divergence is in the lowering/linking layer. Lower the LLVM IR to assembly using `llc` and diff:
   ```bash
   llc-18 ir_from_v1.ll -o asm_from_v1.s
   llc-18 ir_from_v2.ll -o asm_from_v2.s
   diff asm_from_v1.s asm_from_v2.s > asm_diff.txt
   ```
5. Categorize the divergence:
   - **Ordering divergence:** Non-deterministic iteration (e.g., hash map key order).
   - **Metadata divergence:** Timestamps, paths, or build IDs leaking into output.
   - **Codegen bug:** Genuine miscompilation where the same IR lowers to different machine code.
   - **Lowering instability:** Optimization passes that are sensitive to input binary layout.
   - **ABI divergence:** Cross-stage ABI mismatch (see Phase 3D below).

   **Diagnostic criteria for categorization:**

   | Category | Signature in `ir_diff.txt` / `asm_diff.txt` | Confirmation Method |
   |----------|----------------------------------------------|---------------------|
   | **Ordering** | Function definitions appear in different order; same content, different sequence. Local variable numbering (`%42` vs `%43`) shifted throughout. | `tools/difftest.sh <test_file> --ir` — canonicalized comparison eliminates ordering noise. If divergence disappears under canonicalization, it is ordering-only. |
   | **Metadata** | Diff lines contain paths (`/home/...`), timestamps, or `!dbg`/`!DIFile` metadata differences while instruction bodies match. | Count metadata-only diff lines: `grep -c '^\(<\|>\).*!dbg\|!DIFile\|source_filename' ir_diff.txt` — if this accounts for all diff lines, it is metadata-only. |
   | **Codegen bug** | Structurally different instructions for the same function (different opcodes, different types, missing/extra basic blocks). | `tools/minimize.sh <test_file>` to reduce, then `tools/difftest.sh <minimized> --ir --verbose` for per-function instruction-level diff. |
   | **Lowering instability** | `ir_diff.txt` is empty but `asm_diff.txt` shows different register allocation or instruction scheduling. | Confirms LLVM backend sensitivity. Test with `llc-18 --rng-seed=0` to rule out LLVM-internal non-determinism. |
   | **ABI divergence** | Calling convention markers differ (`x86_64_sysvcc` vs default), `byval` vs direct parameter passing, or struct return conventions differ. | `objdump -d blood_v2 | grep -A5 'call.*<function>'` — compare calling sequences between V2 and V3 binaries. Cross-reference with Phase 3D findings. |

6. Fix the root cause in `S`, re-pin the commit hash, clean stale artifacts (see "Artifact Hygiene"), then restart:
   - **If the fix touches ordering-sensitive code** — defined as any change to: HashMap/BTreeMap usage, iteration over collections that feed into IR emission or binary output, sort key definitions, output serialization order, or symbol interning order — restart from **Phase 0, Step 1** to re-audit for new non-determinism vectors introduced by the fix.
     > **Quick check:** If `git diff` of the fix shows changes to files matching `codegen*.blood`, `interner*.blood`, or any file containing `emit_*`/`write_*` functions that serialize to IR or binary output, treat it as ordering-sensitive.
   - **Otherwise** (pure logic/codegen fix that does not alter iteration or output ordering): restart from **Phase 0, Step 4** (the determinism gate). Manually review whether Steps 1–3 audit findings are still valid given the code change, but formal re-execution begins at Step 4.

---

## Phase 3: Feature Parity and Correctness Audit

### 3A — Dual Test Suite Execution

> **Important:** The test harness must be external to and independent of the compiler under test. Both compilers are evaluated using the same harness and the same test inputs.

> **Note on interpreting divergence:** `C_B` is treated as the reference compiler, but it is not infallible. If `V3` diverges from `C_B` on specific tests, the divergence may indicate a regression in `V3` *or* a bug fix — cases where `S` corrects behavior that `C_B` implemented incorrectly. Every divergence must be investigated and classified as a **regression**, a **fix**, or an **expected behavioral difference**, with justification documented.

1. Run the **full** Blood test suite using the **bootstrap compiler** (`C_B`):
   > **Note:** `$BLOOD_RUNTIME` and `$BLOOD_RUST_RUNTIME` must already be exported from the Frozen Build Invocation setup. Do not override them inline — all stages and tests must use the same frozen runtime artifacts.
   ```bash
   cd $SOURCE_DIR && bash $REPO_ROOT/compiler-rust/tests/ground-truth/run_tests.sh $C_B > results_bootstrap.txt 2>&1
   ```
2. Run the **full** Blood test suite using `V3`:
   ```bash
   cd $SOURCE_DIR && bash $REPO_ROOT/compiler-rust/tests/ground-truth/run_tests.sh ./blood_v3 > results_v3.txt 2>&1
   ```
3. Diff the results:
   ```bash
   diff $SOURCE_DIR/results_bootstrap.txt $SOURCE_DIR/results_v3.txt
   ```
4. **(Optional but recommended)** Run the test suite against `V1` as well:
   ```bash
   cd $SOURCE_DIR && bash $REPO_ROOT/compiler-rust/tests/ground-truth/run_tests.sh ./blood_v1 > results_v1.txt 2>&1
   diff results_bootstrap.txt results_v1.txt
   diff results_v1.txt results_v3.txt
   ```
   If `V1` and `V3` produce different test results, the divergence narrows the root cause — since `V1` and `V3` execute the same source logic (`S`), the only difference is which compiler produced the binary (`C_B` for `V1`, `V2` for `V3`). Classify the divergence:
   - **`C_B` miscompiled `S`:** `V1` fails tests that `V3` passes — `C_B`'s code generation introduced a bug when compiling `S`.
   - **`V2` miscompiled `S`:** `V3` fails tests that `V1` passes — the self-hosted compiler's code generation introduced a bug when compiling `S`.
   - **Undefined behavior in `S`:** Both pass but with different outputs, or failures appear non-deterministically — `S` contains undefined behavior that manifests differently depending on the compiling compiler's code generation choices (e.g., struct layout, register allocation, optimization decisions).

   This three-way comparison (`C_B` vs `V1` vs `V3`) is the primary tool for attributing bugs to the correct compiler.

   **Attribution procedure for each divergent test:**
   ```bash
   TEST=$REPO_ROOT/compiler-rust/tests/ground-truth/<divergent_test>.blood

   # Step 1: Localize the divergence phase (compilation, MIR, LLVM IR, behavior)
   BLOOD_TEST=./blood_v1 tools/phase-compare.sh $TEST
   BLOOD_TEST=./blood_v3 tools/phase-compare.sh $TEST

   # Step 2: If behavior diverges, compare IR to find the miscompiled function
   BLOOD_TEST=./blood_v1 tools/difftest.sh $TEST --ir --verbose
   BLOOD_TEST=./blood_v3 tools/difftest.sh $TEST --ir --verbose

   # Step 3: If the test crashes under one compiler, minimize the reproduction
   BLOOD_TEST=./blood_v3 tools/minimize.sh $TEST --mode crash

   # Step 4: For UB suspicion, build ASan-instrumented versions and run the test
   # under each — UB manifests as different ASan reports or non-deterministic behavior
   tools/asan-selfcompile.sh --compiler ./blood_v1 --test "./blood_v1 build $TEST -o /tmp/ub_test"
   ```

5. **Any divergence must be investigated and classified.** Passing under `V3` alone is insufficient — it only proves self-consistency, not parity with the known-good bootstrap compiler.

### 3B — Critical Feature Verification

Verify the following Blood-specific subsystems behave identically under `V3`. Mark items as N/A if the subsystem does not yet exist in the current compiler, with justification.

- [ ] Target architecture correctness — all *currently implemented* targets emit correct code (list targets: e.g., x86-64 via LLVM)
- [ ] Match arms — pattern matching exhaustiveness and correctness
- [ ] Memory interning — string/constant deduplication produces identical layouts
- [ ] Error reporting — diagnostic messages, source locations, span accuracy
- [ ] Optimization passes — output equivalence in both default and `--release` modes
- [ ] Standard library integration — Blood stdlib compiles and links correctly under `V3`

**Verification methodology for each item:**

- **Target architecture correctness:** Compile and run the full ground-truth suite under `V3` via behavioral comparison:
  ```bash
  BLOOD_TEST=./blood_v3 tools/difftest.sh $REPO_ROOT/compiler-rust/tests/ground-truth/ --behavioral --summary-only
  ```
  Any DIVERGE result indicates a target codegen bug. Use `tools/phase-compare.sh` on divergent tests to narrow the phase.

- **Match arms:** Validate MIR structural correctness for pattern matching, then spot-check match-heavy tests:
  ```bash
  BLOOD_TEST=./blood_v3 tools/validate-all-mir.sh --self
  grep -rl 'match ' $REPO_ROOT/compiler-rust/tests/ground-truth/*.blood | head -10 | while read f; do
      BLOOD_TEST=./blood_v3 tools/phase-compare.sh "$f"
  done
  ```

- **Memory interning:** Compare string constant sections in IR between `C_B` and `V3` (requires IR captured from Phase 2 or re-emitted):
  ```bash
  grep '^@.*constant.*c"' ir_from_cb.ll | sort > /tmp/strings_cb.txt
  grep '^@.*constant.*c"' ir_from_v2.ll | sort > /tmp/strings_v3.txt
  diff /tmp/strings_cb.txt /tmp/strings_v3.txt
  ```

- **Error reporting:** Compile a set of intentionally-invalid programs with both `C_B` and `V3` and diff the diagnostic output (error messages, line numbers, span ranges).

- **Optimization passes:** Build the same test program with and without `--release` under `V3` and verify both produce identical behavior (same output, same exit code).

- **Standard library integration:** Use `tools/track-regression.sh` (exercises stdlib integration via ground-truth tests) and `tools/filecheck-audit.sh --gaps-only` to identify untested codegen patterns. Verify zero FAIL results under V3.

### 3C — Dependency Audit

1. Inspect the runtime linking dependencies of `V3`:
   ```bash
   # Linux
   ldd blood_v3

   # macOS
   otool -L blood_v3
   ```
2. **Verification rule:** `V3` must not dynamically link against any component of the bootstrap toolchain (e.g., the Rust standard library, Rust-specific allocators, or any crate artifacts from `C_B`'s build tree).
3. Document all shared library dependencies. Only system-level libraries (libc, libm, libpthread, ld-linux) and explicitly expected dependencies should appear.
4. If `V3` is statically linked, verify with `file blood_v3` and confirm no unexpected static archives were pulled from the bootstrap toolchain's build tree.

### 3D — ABI Conformance Check

> **Rationale:** `V1` was compiled by `C_B` (a Rust compiler), which may make different ABI decisions — struct layout, calling conventions, stack alignment — than Blood's own specification dictates. If `V1` silently produces `V2` with incorrect ABI assumptions, the fixed-point check (Phase 2) will pass (since both `V2` and `V3` share the same bug) but the compiler will be subtly broken.

1. If Blood defines its own calling convention or ABI:
   - Compile a set of ABI test cases (struct layout probes, calling convention exercisers, alignment checks) using `C_B` and `V3` separately.
   - Compare the results. Any divergence indicates that `V1`'s ABI interpretation differs from `V3`'s, which may mean the self-hosted compiler has "locked in" incorrect ABI behavior.
2. If Blood targets a standard ABI (e.g., System V AMD64):
   - Verify that `V3`'s output conforms to the platform ABI specification using `objdump` or `llvm-readobj` to inspect calling convention usage, struct padding, and alignment in a representative sample of compiled functions.
3. Document the ABI verification approach and results.

### 3E — Performance Sanity Check

Compare self-compilation wall-clock times across stages. Stage 2 uses `V1` (compiled by `C_B`) as the compiler; Stage 3 uses `V2` (compiled by `V1`). Both `V1` and `V2` were compiled at the same optimization level (`-O2` via `--release`), so any performance difference reflects codegen quality alone — not optimization level. A significant slowdown in Stage 3 relative to Stage 2 may indicate that Blood's own code generation produces worse code for performance-critical paths than `C_B` does.

1. Record the build time for each stage (already captured in the Build Manifest via `/usr/bin/time -v`).
2. Compare: Stage 2 build time (`V1` compiling `S`) vs. Stage 3 build time (`V2` compiling `S`). Since both compilers use the same optimization level, any difference is attributable to codegen quality. Investigate if Stage 3 is more than 2× slower than Stage 2.
3. If Stage 3 is significantly slower, use `--timings` to identify which compilation phase diverges. Both `C_B` and the self-hosted compiler support `--timings` (per-phase durations printed to stderr), though they report different phase granularity — `C_B` reports ~13 phases while the self-hosted reports 4 high-level phases. Since `V1` and `V2` are both self-hosted binaries, their `--timings` output uses the same format and is directly comparable:
   ```bash
   cd $SOURCE_DIR && ./blood_v1 build main.blood -o /tmp/timings_v1_out.ll --timings $BLOOD_FLAGS 2> timings_v1.txt
   cd $SOURCE_DIR && ./blood_v2 build main.blood -o /tmp/timings_v2_out.ll --timings $BLOOD_FLAGS 2> timings_v2.txt
   diff timings_v1.txt timings_v2.txt
   ```
   > **Note:** The `-o` path uses a `.ll` extension per the IR Output Conventions in System Requirements. The IR and binary artifacts at `/tmp/` are discarded after timing; only the stderr output matters here. Since Phase 2 proved `V2` = `V3` (bitwise identical), comparing `V2` vs. `V3` timings is meaningless — they are the same binary. The meaningful comparison is `V1` vs. `V2`, which are different binaries produced by different compilers.

   If `--timings` identifies the slow phase, investigate further:

   ```bash
   # Memory-driven regression: compare peak RSS per-phase
   BLOOD_TEST=./blood_v1 tools/memprofile.sh $SOURCE_DIR/main.blood --compare
   # (Re-run with BLOOD_TEST=./blood_v2 for the V2 side)

   # CPU-driven regression: profile the slow phase with perf
   perf record -g -o /tmp/perf_v1.data -- ./blood_v1 build main.blood -o /tmp/perf_v1_out.ll $BLOOD_FLAGS
   perf record -g -o /tmp/perf_v2.data -- ./blood_v2 build main.blood -o /tmp/perf_v2_out.ll $BLOOD_FLAGS
   perf report -i /tmp/perf_v1.data --stdio | head -40
   perf report -i /tmp/perf_v2.data --stdio | head -40
   # Compare: if the same function dominates but takes longer under V2,
   # V1's codegen produced worse machine code for that hot function.

   # Codegen quality comparison for the hot function:
   BLOOD_TEST=./blood_v1 tools/difftest.sh $SOURCE_DIR/main.blood --ir --first-divergence
   ```

   > **Interpretation:** If memory usage is similar but CPU time is higher, `V2` produced worse machine code (instruction selection, register allocation). If memory is significantly higher, the regression is likely an allocation or data structure issue rather than codegen quality.

4. A performance regression does not block promotion but must be documented and triaged.

---

## Phase 4: Golden Image Promotion

### Promotion Criteria

All of the following must be verified:

- [ ] Phase 0 determinism gate passed (both `C_B` and per-stage gates)
- [ ] Phase 2 bitwise identity confirmed (`V2` = `V3`)
- [ ] Phase 3A dual test suite shows zero unexplained divergence (all divergences classified)
- [ ] Phase 3B critical features verified (or marked N/A with justification)
- [ ] Phase 3C dependency audit shows no bootstrap toolchain contamination
- [ ] Phase 3D ABI conformance check passed (or documented as N/A with rationale)
- [ ] Phase 3E performance sanity check passed (or regressions documented and triaged)

### Promotion Steps

1. Designate `V3` as the **Golden Image** — the canonical, self-hosted Blood compiler. (Since Phase 2 proved `V2` and `V3` are bitwise identical, either could serve as the Golden Image; `V3` is chosen by convention as the latest-stage artifact.)
2. Archive the following for recovery and provenance:
   - `C_B` (the bootstrap compiler binary, with its own SHA-256 hash)
   - `V1` (the cross-compiled intermediate)
   - `V2` and `V3` (for future audit reference)
   - The frozen source commit hash
   - All SHA-256 hashes (full manifest)
   - The build environment specification (OS, kernel, linker, toolchain versions)
   - The frozen build flags (`$BLOOD_FLAGS`)
   - The test harness version/commit hash
3. Update the project `Justfile` (or equivalent build scripts) to use the Golden Image as the default compiler for all future development.
4. Tag the source repository with a bootstrap milestone (e.g., `bootstrap-v1.0`).

### Post-Promotion Verification

Perform one final round-trip: use the Golden Image to compile `S` one more time and confirm the output is still bitwise identical. This guards against environmental fluke during the bootstrap session.

```bash
cd $SOURCE_DIR && ./blood_v3 build main.blood -o blood_v3_verify $BLOOD_FLAGS
sha256sum blood_v3 blood_v3_verify
```

If the hashes differ, the environment is not stable. **Do not trust the Golden Image.** Investigate:

1. **Re-run the V3 determinism gate** to confirm V3 itself is still deterministic:
   ```bash
   cd $SOURCE_DIR && ./blood_v3 build main.blood -o blood_v3_recheck_a $BLOOD_FLAGS
   cd $SOURCE_DIR && ./blood_v3 build main.blood -o blood_v3_recheck_b $BLOOD_FLAGS
   sha256sum blood_v3_recheck_a blood_v3_recheck_b
   ```
2. **If the recheck hashes match each other but differ from `blood_v3`:** The environment changed between the original Phase 1 build and this post-promotion step. Check for: OS updates applied between phases, different `llc-18`/`clang-18` versions loaded, modified runtime artifacts (`sha256sum $BLOOD_RUNTIME $BLOOD_RUST_RUNTIME`), filesystem remount, or different `LC_ALL` setting.
3. **If the recheck hashes differ from each other:** V3 has a non-determinism bug that the V2 determinism gate should have caught. Suspect environmental interference (memory pressure causing swap, thermal throttling affecting floating-point reproducibility). Re-run the entire bootstrap in a clean environment (container or fresh VM snapshot).
4. **Diff the IR** to localize the divergence:
   ```bash
   cd $SOURCE_DIR && ./blood_v3 build main.blood --emit=llvm-ir -o /tmp/verify_ir.ll $BLOOD_FLAGS
   # Compare against the Phase 2 IR capture if available
   diff ir_from_v2.ll /tmp/verify_ir.ll | head -40
   ```

---

## Phase 5 (Optional): Diverse Double Compilation (DDC)

> **Rationale:** A standard three-stage bootstrap proves self-consistency but cannot detect a Ken Thompson–style "trusting trust" attack where a malicious compiler reproduces the backdoor in its own compilation. DDC mitigates this by introducing an independent trust root.

### Steps

1. Obtain a **second, independent compiler** capable of compiling Blood (e.g., a different version of `C_B`, a Blood interpreter, or a compiler from a different author/codebase).
2. Compile `S` using the second compiler to produce `V_alt`.
3. Compile `S` using `V_alt` to produce `V_alt2`.
4. Compare `V_alt2` against `V3`.
5. **If identical:** High confidence that no trust attack is present.
6. **If not identical:** Non-identical results do not necessarily indicate an attack — they may reflect legitimate codegen differences in the alternate compiler that wash out after additional self-compilation stages.

   **Convergence procedure:**
   ```bash
   # Stage 1: V_alt2 already exists. Compile one more round.
   cd $SOURCE_DIR && ./blood_v_alt2 build main.blood -o blood_v_alt3 $BLOOD_FLAGS
   sha256sum blood_v_alt3 blood_v3
   ```

   **If `V_alt3` = `V3`:** Convergence achieved after one extra stage. The `V_alt` vs `V3` difference was benign codegen divergence from the alternate compiler, which washed out. DDC passes.

   **If `V_alt3` != `V3`:** Compile one final round:
   ```bash
   cd $SOURCE_DIR && ./blood_v_alt3 build main.blood -o blood_v_alt4 $BLOOD_FLAGS
   sha256sum blood_v_alt4 blood_v3
   ```

   **Convergence limit:** If `V_alt4` != `V3`, DDC has failed to converge within 4 stages from the alternate compiler. This requires manual investigation:
   - **Diff the IR** from `V_alt3` and `V3` (same procedure as Phase 2 Failure Protocol):
     ```bash
     cd $SOURCE_DIR && ./blood_v_alt3 build main.blood --emit=llvm-ir -o /tmp/ir_alt3.ll $BLOOD_FLAGS
     diff ir_from_v2.ll /tmp/ir_alt3.ll | head -80
     ```
   - **If the diff shows only naming/ordering differences** (no semantic divergence): The alternate compiler has a systematic codegen difference that does not converge. This is **not** evidence of a trust attack — it is a limitation of the alternate compiler's fidelity. Document and accept.
   - **If the diff shows semantic differences** (different opcodes, missing functions, incorrect logic): Suspect either a bug in the alternate compiler or a genuine trust attack. Run the full ground-truth test suite under `V_alt3` vs `V3` to determine whether the semantic difference causes behavioral divergence:
     ```bash
     cd $SOURCE_DIR && bash $REPO_ROOT/compiler-rust/tests/ground-truth/run_tests.sh ./blood_v_alt3 > results_alt3.txt 2>&1
     diff results_v3.txt results_alt3.txt
     ```

> **Prerequisite:** The alternate compiler must itself pass a basic correctness validation (e.g., the ground-truth test suite) before being used as a DDC trust root. If the alternate compiler has its own bugs, `V_alt` may be non-functional or subtly incorrect, and the convergence property (`V_alt3` = `V3`) will not hold regardless of whether a trust attack is present. DDC only provides meaningful signal when the alternate compiler is known-good.

> **Note:** DDC is unnecessary for most projects. Include it when the Blood compiler's threat model requires defense against supply-chain or compiler-level attacks.

---

## Output Requirements

Deliver a structured status report containing:

### 1. Build Manifest

#### 1a. Build Info

| Stage | Binary | Compiler Used | Build Time | Peak RSS |
|-------|--------|---------------|------------|----------|
| 0 (det. gate `C_B` A) | `blood_det_cb_a` | `C_B` | `<duration>` | `<MB>` |
| 0 (det. gate `C_B` B) | `blood_det_cb_b` | `C_B` | `<duration>` | `<MB>` |
| 1 (= det. gate A if optimized) | `blood_v1` | `C_B` | `<duration>` | `<MB>` |
| 1 (det. gate `V1` A) | `blood_det_v1a` | `V1` | `<duration>` | `<MB>` |
| 1 (det. gate `V1` B) | `blood_det_v1b` | `V1` | `<duration>` | `<MB>` |
| 2 (= det. gate A if optimized) | `blood_v2` | `V1` | `<duration>` | `<MB>` |
| 2 (det. gate `V2` A) | `blood_det_v2a` | `V2` | `<duration>` | `<MB>` |
| 2 (det. gate `V2` B) | `blood_det_v2b` | `V2` | `<duration>` | `<MB>` |
| 3 (= det. gate A if optimized) | `blood_v3` | `V2` | `<duration>` | `<MB>` |
| Post-promotion | `blood_v3_verify` | `V3` | `<duration>` | `<MB>` |

#### 1b. Verification Info

| Binary | SHA-256 Hash | Stripped? | Det. Gate |
|--------|--------------|-----------|-----------|
| `blood_det_cb_a` | `<hash>` | `<yes/no>` | `C_B`: PASS/FAIL |
| `blood_det_cb_b` | `<hash>` | `<yes/no>` | ↑ (pair) |
| `blood_v1` | `<hash>` | `<yes/no>` | — |
| `blood_det_v1a` | `<hash>` | `<yes/no>` | `V1`: PASS/FAIL |
| `blood_det_v1b` | `<hash>` | `<yes/no>` | ↑ (pair) |
| `blood_v2` | `<hash>` | `<yes/no>` | — |
| `blood_det_v2a` | `<hash>` | `<yes/no>` | `V2`: PASS/FAIL |
| `blood_det_v2b` | `<hash>` | `<yes/no>` | ↑ (pair) |
| `blood_v3` | `<hash>` | `<yes/no>` | — |
| `blood_v3_verify` | `<hash>` | `<yes/no>` | — |

> **Note:** If the build produces additional artifacts (e.g., a separately compiled standard library), add rows for each artifact at each stage.

### 2. Verification Results

- **Determinism gate (`C_B`):** PASS / FAIL
- **Determinism gate (`V1`):** PASS / FAIL
- **Determinism gate (`V2`):** PASS / FAIL
- **Bitwise identity (`V2` = `V3`):** PASS / FAIL
- **Test suite parity (bootstrap vs. `V3`):** PASS / FAIL (with divergence count and classification if applicable)
- **Critical feature verification:** PASS / PARTIAL (list N/A items with justification) / FAIL
- **Dependency audit:** CLEAN / CONTAMINATED (list offending libraries)
- **ABI conformance:** PASS / N/A (with rationale) / FAIL
- **Performance sanity check:** PASS / REGRESSION (with details) / N/A
- **Post-promotion round-trip:** PASS / FAIL

### 3. Verdict

- **Can the bootstrap compiler be safely retired?** YES / NO
- **If NO:** Root cause summary and recommended remediation steps.
- **If YES:** Confirm Golden Image designation and provide the canonical hash for `V3`.

### 4. Environment Record

- OS, kernel version, and architecture
- Linker name and version
- Source commit hash for `S`
- Source commit hash for the test harness
- Bootstrap compiler (`C_B`) version/commit hash and SHA-256
- Frozen build flags (`$BLOOD_FLAGS`)
- Frozen runtime artifacts (`$BLOOD_RUNTIME`, `$BLOOD_RUST_RUNTIME`) with SHA-256 hashes
- Binary comparison strategy used (stripped / unstripped)
- Build timestamp (wall clock, for human reference only — not embedded in binaries)
- Locale setting (`LC_ALL=C`)
- Any other non-default environment variables
- Container/VM snapshot ID (if applicable)

