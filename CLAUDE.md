# Blood Compiler Development Guidelines

## Project Structure

```
blood/                          # Repository root
├── src/selfhost/     # Self-hosted Blood compiler (written in Blood)
├── src/bootstrap/bloodc/src/   # Rust bootstrap compiler
├── docs/spec/GRAMMAR.md        # Language grammar specification
└── tools/                      # Development & debugging tools
```

| Compiler | Location | Language | Purpose |
|----------|----------|----------|---------|
| **Reference** | `src/bootstrap/bloodc/src/` | Rust | Bootstrap compiler |
| **Self-Hosted** | `src/selfhost/` | Blood | Self-hosting target |

Pipeline: `Source → Lexer → Parser → AST → HIR → Type Check → MIR → Codegen → LLVM`

Both compilers must conform to the spec (`docs/spec/`). When a compiler diverges from the spec, the compiler is wrong — not the spec. When the two compilers disagree with each other, check the spec to determine which (if either) is correct. Mismatches are documented in `COMPILER_NOTES.md`.

## Spec-First Principle

Blood's Five Pillars are **Veracity** (generational refs), **Identity** (content-addressing), **Composability** (algebraic effects), **Extensibility** (multiple dispatch), and **Isolation** (linear types + regions). The design hierarchy is: **Correctness > Safety > Predictability > Performance > Ergonomics**.

**The spec prescribes; the implementation conforms.** When you find a divergence between a spec and a compiler, the default assumption is the compiler is wrong. Agreement between two compilers is evidence of shared shortcuts, not correctness. The correct question is always: "Given Blood's aspirational design, what is the right answer?"

**Updating a spec to match an implementation** requires one of three justifications — no exceptions:
1. **Spec omission** — the spec failed to list a universally expected behavior
2. **Rust-ism correction** — the spec inherited a Rust design that contradicts Blood's goals
3. **Non-normative clarification** — an informational note, not a normative change

"Both compilers do it" is **never sufficient** on its own. Safety-critical spec changes (memory model, type safety, effects) additionally require a design evaluation document before the change is made.

**Partial implementations** must not be presented as closing a gap. If the spec prescribes behaviors A, B, and C and you implement only A, that is a valid first step — document what remains, do not mark it complete.

Do not be afraid to declare that a compiler implementation is fundamentally wrong and shift back into investigation, research, and design before creating technical debt. The cost of getting the design right is always lower than the cost of building on a wrong foundation.

For the full argument, examples, and the TYP-05 reversal history, see `.tmp/DECISIONS.md` §Spec-First Principle.

## Blood is NOT Rust

Blood uses **dot-separated module paths**, NOT Rust's `::`:

```blood
use std.mem.allocate;       // CORRECT
// use std::mem::allocate;  // WRONG — this is Rust
```

`::` is **not part of Blood's syntax** (removed in GRAMMAR.md v0.4.0). Dots are used everywhere:
- Grouped imports: `use std.iter.{A, B}`
- Glob imports: `use std.ops.*`
- Qualified paths: `module.Type { ... }`

> **Note:** The compilers have not yet been updated to match — they still use `::` for grouped/glob imports and qualified expressions. This is a known alignment gap.

These patterns are **correct in Blood**, not shortcuts:

| Pattern | Why It's Correct |
|---------|------------------|
| `for i in 0..len { ... }` | Preferred over while-counter loops |
| `i += 1` | Compound assignment (also `-=`, `*=`, `/=`, `%=`) |
| Explicit match arms for every variant | Required by zero shortcuts principle |
| `HashMap<u32, Type>` vs newtype keys | Blood's type system differs from Rust |

**Before writing any syntax, verify it exists in `docs/spec/GRAMMAR.md`.** Do not add Rust features that don't exist in Blood — use the patterns shown in the table above instead.

## Zero Shortcuts

**This codebase must have ZERO shortcuts.** Every pattern match must be exhaustive with proper handling. Every error case must be reported. Every feature must be complete or explicitly error with "not yet implemented."

Shortcuts include: `_ => Ok(())`, `_ => continue`, `Type::error()`, `unwrap_or_default()`, catch-all `_ =>`, dead code, magic numbers, TODO/FIXME without action, silent skips, incomplete error messages.

Audit search terms: `_ =>`, `unwrap_or_default`, `unwrap_or_else`, `Type::error()`, `continue` (in match arms), `Ok(())` (suspicious early returns), `TODO`, `FIXME`, `XXX`, `HACK`, `Phase 2`, `not yet`, `later`, `unreachable!()`, `panic!()`, empty function bodies, functions returning hardcoded values.

## Development Workflow

The selfhost compiler is developed using a **self-compilation loop**: edit source, recompile the compiler using itself, test the result.

**Inner loop (seconds):** `first_gen check file.blood` — validates syntax, types, definite initialization, linearity, and dangling reference detection against the current compiler. Use this while editing to catch errors fast. Write 10-50 lines, check, fix, repeat.

**Build loop (incremental, ~3 min):** `./build_selfhost.sh build second_gen` — first_gen compiles the current source with `--split-modules`. Only changed modules are re-codegen'd. Test with `./build_selfhost.sh test golden second_gen`. This is the primary development cycle.

**Recovery (clean, ~4 min):** `./build_selfhost.sh build first_gen` — rebuilds from the bootstrap seed. Use this only when self-compilation breaks (your edit introduced a bug that prevents the compiler from compiling itself). Fix the issue, rebuild first_gen, then return to the build loop.

**Gate (full chain, ~15 min):** `./build_selfhost.sh gate` — runs the full bootstrap chain (first_gen → second_gen → third_gen byte-compare) and updates the seed. Run at end-of-session or before pushing. Required for ABI/calling-convention changes. Use `gate --quick` (~8 min) to skip first_gen/second_gen rebuilds when they're already built and tested.

## Development Rules

**IR before code (codegen bugs):** Before writing ANY codegen fix, dump the LLVM IR for a minimal repro through both bootstrap (`--emit=llvm-ir`) and first_gen (`--dump-ir --no-cache`). Find the relevant function, trace the actual instruction sequence, identify the exact divergence, THEN trace back to the codegen source. Do NOT hypothesize what the IR looks like — read it. This takes 5 minutes and prevents 30-60 minutes of wrong fixes.

**When something crashes or produces wrong results — use the debugging toolkit:**

1. **Read the build log first.** Every build writes to `.logs/`. Don't re-run the build — read `tail -50 src/selfhost/.logs/build_*.log | tail -1` to find the latest log. Crash backtraces, error messages, and timing are all captured.
2. **Backtraces show source locations.** All runtime panic functions print full backtraces with DWARF source mapping. Binaries compiled by the selfhost have per-instruction source line debug info. Use `addr2line -e binary -f ADDRESS` to resolve to `file.blood:line`. Different offsets within a function resolve to different source lines.
3. **Use `--validate-mir`** to catch MIR structural errors before codegen. Definite initialization analysis and MIR-level linearity checking run by default (no flag needed).
4. **Use `--dump-mir` or `--dump-mir=fn_name`** to inspect MIR for a specific function.
5. **Use `--trace-codegen`** for per-function codegen tracing.
6. **Use `./build_selfhost.sh asan`** for AddressSanitizer-instrumented builds when chasing memory corruption.
7. **Use `./build_selfhost.sh bisect`** to binary search for the miscompiled function when the selfhost produces wrong output.
8. **Use `./build_selfhost.sh diff file.blood`** to compare bootstrap vs first_gen output for a specific file.
9. **Don't run the same 2-minute build repeatedly** trying to filter different output. Build ONCE, read the log, use addr2line and dump tools on the existing artifacts.
10. **Use `./build_selfhost.sh debug-test file.blood`** to compile a single test with `--dump-mir --validate-mir` and preserve all artifacts (IR, MIR, binary, stderr) in `build/debug/`.
11. **Use `tools/blood-diag`** as the unified entry point for all diagnostic tools: `ir-diff`, `minimize`, `parity`, `memprofile`, `phase`, `asan`, `bisect`, `debug-test`, `metrics`.
12. **Use `./build_selfhost.sh metrics`** to check build size and time trends. Every build writes JSON metrics to `.logs/metrics.jsonl`.
13. **Use `--alloc-profile`** to see the top 10 functions by IR size (identifies codegen hotspots and binary size contributors).
14. **Use `tools/memprofile.sh --perf`** for CPU flame graphs via perf record (requires linux-tools).
15. **gdb variable inspection works.** Binaries have DILocalVariable DWARF metadata. `gdb print variable_name` shows values for named Blood variables.

**Selfhost bugs: report, do NOT work around.** Write the correct code. If the compiler miscompiles it, that's a compiler bug. STOP, isolate, document, report. Signs: DefId errors, works in one context but not another, mutations lost through references, runtime mismatch. See `tools/FAILURE_LOG.md` for history.

**Bootstrap gate protocol:** Changes that touch calling conventions, type layouts, or runtime FFI **must** pass `./build_selfhost.sh gate` before being considered complete. This runs the full build chain (first_gen → golden → second_gen → golden → third_gen byte-compare) and updates `bootstrap/seed` on success. Other codegen changes should gate at end-of-session or before pushing. The build script warns when the seed is >15 commits behind HEAD. For breaking ABI changes, use a two-stage bootstrap: Stage 1 adds new code paths without activating them (seed can compile this), gate to get a new seed, then Stage 2 activates the new behavior.

**Build caches are compiler-version-specific.** The build script has three cache layers: (1) module-level source hashes (`build/obj/.hashes/`), (2) per-function content hashes (`build/.content_hashes/`), (3) source-level build cache (`build/.blood-cache`). All caches are automatically cleared between generations during gate/build-all. When manually testing across compiler generations, clear caches: `rm -rf build/.content_hashes build/obj/.hashes build/.blood-cache`. Symptom of stale cache: `undefined value '@.str.NNNN'` errors during llc.

**Canary-Cluster-Verify (CCV) method:** All batch changes to the self-hosted compiler follow the CCV protocol. Canary-test new patterns before mass conversion. Cluster changes by compiler phase. Verify (build + golden tests + bootstrap) after each cluster. See `DEVELOPMENT.md` for the full protocol, cluster definitions, and bootstrap gate rules.

**Document discoveries:** Test in isolation, document in this file, comment in code. Distinguish bugs (report and wait) from documented limitations (work around).

**Maintain consistency:** Check `common.blood` for canonical types before modifying shared types. Update ALL files that use the type.

## Shared Types (common.blood)

Canonical shared types: `Span`, `Symbol`, `SpannedSymbol`, `SpannedString`, `OrderedFloat`. Check `src/selfhost/common.blood` for current field definitions before modifying any shared type. Import via `mod common;`, reference as `common.Span`, etc. Update ALL files that use the type when making changes.

## Active Limitations

- **Per-function type inference size:** Very large functions can hit type inference limits. Split oversized functions rather than adding more code to them.
- **Keyword field names:** `module` cannot be used as a field name; use `mod_decl`
- **Memory reuse:** Requires active region at startup for region-aware allocation

Fixed bugs are documented in `tools/FAILURE_LOG.md`.

## Build & Test Commands

```bash
# Check syntax/types
src/selfhost/build/first_gen check file.blood

# Build/run a file
src/selfhost/build/first_gen build file.blood
src/selfhost/build/first_gen run file.blood
```

### Build Script (`src/selfhost/build_selfhost.sh`)

All building and testing goes through the build script. No arguments shows status.

```bash
cd src/selfhost

# Build stages
./build_selfhost.sh build first_gen     # Build first_gen from seed compiler
./build_selfhost.sh build second_gen    # Self-compile first_gen → second_gen
./build_selfhost.sh build third_gen     # Bootstrap second_gen → third_gen + byte-compare
./build_selfhost.sh build blood_runtime # Compile Blood runtime → libblood_runtime_blood.a
./build_selfhost.sh build first_gen_blood  # Link first_gen against Blood runtime (no Rust)
./build_selfhost.sh build all           # Full chain: blood_runtime → first_gen → GT → second_gen → GT → third_gen

# Test suites (compiler arg accepts names: first_gen, second_gen, third_gen, or a path)
./build_selfhost.sh test golden              # Default: first_gen
./build_selfhost.sh test golden second_gen    # Verify first_gen codegen
./build_selfhost.sh test golden-blood             # Golden tests linked against Blood runtime
./build_selfhost.sh test dispatch                   # Compare first_gen vs second_gen output

# Bootstrap gate (full pipeline + update seed on success)
./build_selfhost.sh gate               # build all + cp second_gen → bootstrap/seed
./build_selfhost.sh gate --quick       # third_gen byte-compare + seed only (assumes fg/sg built)

# Diagnostics
./build_selfhost.sh verify              # IR verification + declaration diff + FileCheck
./build_selfhost.sh ir-check            # FileCheck tests only
./build_selfhost.sh asan                # Build ASan-instrumented binary
./build_selfhost.sh bisect              # Binary search for miscompiled function
./build_selfhost.sh emit llvm-ir        # Emit intermediate IR

# Workflow
./build_selfhost.sh run file.blood              # Compile and run (default: first_gen)
./build_selfhost.sh run file.blood bootstrap    # Run through bootstrap
./build_selfhost.sh run file.blood --dump-mir   # Run with extra compiler flags
./build_selfhost.sh diff file.blood             # Compare blood-rust vs first_gen output
./build_selfhost.sh status                      # Show compiler status, ages, processes
./build_selfhost.sh install                     # Install toolchain to ~/.blood/{bin,lib}/
./build_selfhost.sh clean                       # Remove build artifacts (preserves .logs)
./build_selfhost.sh clean-cache                 # Remove only caches (preserves binaries + .logs)
./build_selfhost.sh clean-all                   # Remove everything including logs

# Flags
-q, --quiet         # Suppress per-test output (only failures + summary)
--fresh              # Clear caches before building (use with build commands)
```

**Golden test behavior:** COMPILE_FAIL tests pass when the compiler correctly rejects the code. `// EXPECT:` diagnostic pattern mismatches are reported separately as warnings, not counted as failures. Test binaries have a 30s timeout to prevent infinite-loop hangs.

### Build Directory Layout

All build artifacts go to `build/` by default (relative to source file parent):

```
build/
├── debug/          # Default profile: binary, .ll, .o
├── release/        # --release profile
├── obj/            # Per-module .ll and .o files (--split-modules incremental)
└── .content_hashes/ # Per-function BLAKE3 hash + cached IR fragments
```

Override hierarchy (highest priority first):
1. `--build-dir <path>` CLI flag
2. `[build] build-dir` in Blood.toml (project mode)
3. `BLOOD_BUILD_DIR` environment variable
4. Default: `build/` relative to source file parent

## Development Tools

| Tool | Purpose |
|------|---------|
| `tools/difftest.sh` | Compare first_gen vs second_gen output (behavioral or IR) |
| `tools/parse-parity.sh` | Detect accept/reject drift between compilers |
| `tools/minimize.sh` | Reduce failing test to minimal reproduction |
| `tools/phase-compare.sh` | Identify which compilation phase first diverges |
| `tools/memprofile.sh` | Profile memory usage with per-phase breakdown |
| `tools/filecheck-audit.sh` | Audit FileCheck test coverage and recommend gaps |
| `tools/validate-all-mir.sh` | Pre-codegen MIR validation gate |
| `tools/track-regression.sh` | Detect golden test regressions vs baseline |
| `tools/FAILURE_LOG.md` | Structured log of past bugs, root causes, resolutions |
| `tools/AGENT_PROTOCOL.md` | Session protocol: investigation workflow, stop conditions |

**Environment variables:** `SEED_COMPILER` (bootstrap seed binary path), `BLOOD_RUST` (legacy bootstrap compiler path), `RUNTIME_A` (libblood_runtime_blood.a), `BLOOD_RUST_RUNTIME` (runtime linked into programs, exported), `BLOOD_STDLIB_PATH` (stdlib directory), `BLOOD_BUILD_DIR` (build output directory), `BLOOD_CACHE` (compilation cache directory).

**Installed toolchain:** `./build_selfhost.sh install` copies compiler, runtime, and stdlib to `~/.blood/{bin,lib}/`. The compiler falls back to `~/.blood/lib/` for runtime and stdlib when env vars and CLI flags aren't set. Resolution: CLI flag > env var > `~/.blood/lib/` > exe-relative.

Run each tool with `--help` or see its header comments for detailed usage.

## Key Patterns (Mistake Prevention)

- **Interner mismatch:** AST parser and HIR lowering use different string interners. Re-intern via `ctx.span_to_string(span)` + `ctx.intern()` when creating HIR items from AST data.
- **Unresolved types in MIR expressions:** MIR local types are resolved (`apply_substs_id` runs after `ctx.finish()`), but expression types during MIR lowering are still `Infer(TyVarId)`. Use `ctx.resolve_type()` to get concrete types before type-based decisions in MIR lowering code.
- **Field resolution keys:** Use `name.span.start` (field NAME position), not `expr.span.start`. Composite key: `(body_def_id, name_span_start)`.
- **Four `type_to_llvm_with_ctx` functions:** `codegen_ctx` (method, no generics), `codegen_stmt` (standalone, &Type, full generics), `codegen_size` (standalone, &Type, ADT registry only), `codegen_size::type_to_llvm_with_ctx_id` (TyId, full generics + fast path — preferred).
- **TypeError API:** Use `checker.error(TypeErrorKind::Variant, span)` — NOT `checker.errors.push(TypeError::new(...))`.
- **Trait default method remapping:** See `CallRemapEntry` in `typeck_driver.blood` + `extract_direct_fn_name` in `codegen_term.blood`.

## Reference

- **Grammar spec**: `docs/spec/GRAMMAR.md` — source of truth for Blood's surface syntax
- **Macro spec**: `docs/spec/MACROS.md` — macro system design, fragment kinds, hygiene roadmap
- **Design evaluations**: `docs/design/IMPL_TRAIT.md`, `docs/design/COMPARISON_CHAINING.md`
- **Compiler notes**: `src/selfhost/COMPILER_NOTES.md`
- **Aether examples**: `~/blood-test/aether/`
- **Bug history**: `tools/FAILURE_LOG.md`
- **Session protocol**: `tools/AGENT_PROTOCOL.md`

## Current Work Intake

Start with `.tmp/INDEX.md` for the file index. Working documents are organized by content type:

- **`.tmp/WORK.md`** — What to work on next. Phases, milestones, blockers, deferred items.
- **`.tmp/BUGS.md`** — All known bugs by compiler and severity.
- **`.tmp/DECISIONS.md`** — Design decisions with the spec-first principle, 223-entry design reference, and decision provenance.
- **`.tmp/GAPS.md`** — What's designed but not built, what was never designed.

For methodology see `.tmp/METHODS.md`. For long-term plans see `.tmp/PLANS.md`. For deep-dive investigation logs see `.tmp/INVESTIGATIONS.md`.

Source material preserved in `.tmp/archive/` (26 files + tracks/).

**To pick up work:**
1. Read `.tmp/WORK.md` — items are organized by remediation phase (Phase 0 = soundness, up to Phase 6 = bounds/init), then non-remediation sections (compiler bugs, performance, test expansion, deferred items)
2. Work phases in order: complete Phase N before moving to Phase N+1. Within non-remediation sections, prioritize by severity.
3. For each item: read the relevant spec and both compilers. Check `.tmp/DECISIONS.md` for design context and `.tmp/GAPS.md` for implementation status.
4. Follow the decision procedure:
   - If the spec is right and the implementation is wrong → fix the code (`close-impl-to-spec`)
   - If the spec has a genuine omission or Rust-ism → fix the spec with documented justification (`close-spec-to-impl`)
   - If the answer is unclear → write a design evaluation, research the problem space (`open-question`)
5. After completing work, update `.tmp/WORK.md` with results and verify (build + golden tests + bootstrap)

**Phase-skip rules:** You may skip an item within a phase only if it is genuinely blocked (marked `open-question` with no resolution path, or has unmet prerequisites). When skipping, state the specific reason for each skipped item — do not dismiss a phase as a group. If all remaining items in a phase are blocked or deferred, you may proceed to the next phase after documenting why each was skipped.

Items marked `[-]` are **deferred with re-evaluation triggers** — do not pick them up unless the trigger condition described in WORK.md is met.
