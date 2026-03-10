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

For the full argument, examples, and the TYP-05 reversal history, see `.tmp/prompt.md`.

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

## Development Rules

**Compile before commit:** `src/bootstrap/target/release/blood check <file.blood>`. If blood-rust rejects it, the code is wrong.

**Incremental development:** Write 10-50 lines, compile, fix, repeat. Never write hundreds of lines without compiling.

**Blood-rust bugs: report, do NOT work around.** Write the correct code. If blood-rust miscompiles it, that's a blood-rust bug. STOP, isolate, document, report, wait. Signs: DefId errors, works in one context but not another, mutations lost through references, runtime mismatch. See `tools/FAILURE_LOG.md` for history.

**Canary-Cluster-Verify (CCV) method:** All batch changes to the self-hosted compiler follow the CCV protocol. Canary-test new patterns before mass conversion. Cluster changes by compiler phase. Verify (build + golden tests + bootstrap) after each cluster. See `DEVELOPMENT.md` for the full protocol, cluster definitions, and bootstrap gate rules.

**Document discoveries:** Test in isolation, document in this file, comment in code. Distinguish bugs (report and wait) from documented limitations (work around).

**Maintain consistency:** Check `common.blood` for canonical types before modifying shared types. Update ALL files that use the type.

## Shared Types (common.blood)

Canonical shared types: `Span`, `Symbol`, `SpannedSymbol`, `SpannedString`, `OrderedFloat`. Check `src/selfhost/common.blood` for current field definitions before modifying any shared type. Import via `mod common;`, reference as `common::Span`, etc. Update ALL files that use the type when making changes.

## Active Limitations

- **Module resolution limit:** Avoid adding new `mod` imports to files near the resolution limit (e.g., driver.blood)
- **Keyword field names:** `module` cannot be used as a field name; use `mod_decl`
- **Memory reuse:** Requires active region at startup for region-aware allocation

All fixed bugs (BUG-001 through BUG-013) are documented in `tools/FAILURE_LOG.md`.

## Build & Test Commands

```bash
# Check syntax/types (blood-rust directly)
src/bootstrap/target/release/blood check file.blood

# Build/run a file (blood-rust directly)
src/bootstrap/target/release/blood build file.blood
src/bootstrap/target/release/blood run file.blood

# Build the Rust bootstrap compiler
cd src/bootstrap && cargo build --release
```

### Build Script (`src/selfhost/build_selfhost.sh`)

All selfhost building and testing goes through the build script. No arguments shows status.

```bash
cd src/selfhost

# Build stages
./build_selfhost.sh build cargo         # Rebuild blood-rust (cargo build --release)
./build_selfhost.sh build first_gen     # Build first_gen from blood-rust
./build_selfhost.sh build second_gen    # Self-compile first_gen → second_gen
./build_selfhost.sh build third_gen     # Bootstrap second_gen → third_gen + byte-compare
./build_selfhost.sh build runtime       # Recompile runtime.o from runtime.c
./build_selfhost.sh build all           # Full chain: cargo → first_gen → GT → second_gen → GT → third_gen

# Test suites (compiler arg accepts names: bootstrap, first_gen, second_gen, third_gen, or a path)
./build_selfhost.sh test golden              # Default: first_gen
./build_selfhost.sh test golden bootstrap     # Test against blood-rust
./build_selfhost.sh test golden second_gen    # Verify first_gen codegen
./build_selfhost.sh test dispatch                   # Compare bootstrap vs first_gen output
./build_selfhost.sh test blood                      # Run tests/blood-test/ through bootstrap

# Diagnostics
./build_selfhost.sh verify              # IR verification + declaration diff + FileCheck
./build_selfhost.sh ir-check            # FileCheck tests only
./build_selfhost.sh asan                # Build ASan-instrumented binary
./build_selfhost.sh bisect              # Binary search for miscompiled function
./build_selfhost.sh emit llvm-ir        # Emit intermediate IR

# Workflow
./build_selfhost.sh run file.blood              # Compile and run (default: first_gen)
./build_selfhost.sh run file.blood bootstrap    # Run through bootstrap
./build_selfhost.sh diff file.blood             # Compare blood-rust vs first_gen output
./build_selfhost.sh status                      # Show compiler status, ages, processes
./build_selfhost.sh clean                       # Remove build artifacts (preserves .logs)
./build_selfhost.sh clean-all                   # Remove everything including logs

# Flags
-q, --quiet         # Suppress per-test output (only failures + summary)
```

**Golden test behavior:** COMPILE_FAIL tests pass when the compiler correctly rejects the code. `// EXPECT:` diagnostic pattern mismatches are reported separately as warnings, not counted as failures. Test binaries have a 30s timeout to prevent infinite-loop hangs.

### Build Directory Layout

All build artifacts go to `build/` by default (relative to source file parent):

```
build/
├── debug/          # Default profile: binary, .ll, .o
├── release/        # --release profile
└── obj/<stem>/     # Per-definition object files (blood-rust incremental)
```

Override hierarchy (highest priority first):
1. `--build-dir <path>` CLI flag
2. `[build] build-dir` in Blood.toml (project mode)
3. `BLOOD_BUILD_DIR` environment variable
4. Default: `build/` relative to source file parent

## Development Tools

| Tool | Purpose |
|------|---------|
| `tools/difftest.sh` | Compare blood-rust vs first_gen output (behavioral or IR) |
| `tools/parse-parity.sh` | Detect accept/reject drift between blood-rust and first_gen |
| `tools/minimize.sh` | Reduce failing test to minimal reproduction |
| `tools/phase-compare.sh` | Identify which compilation phase first diverges |
| `tools/memprofile.sh` | Profile memory usage with per-phase breakdown |
| `tools/filecheck-audit.sh` | Audit FileCheck test coverage and recommend gaps |
| `tools/validate-all-mir.sh` | Pre-codegen MIR validation gate |
| `tools/track-regression.sh` | Detect golden test regressions vs baseline |
| `tools/FAILURE_LOG.md` | Structured log of past bugs, root causes, resolutions |
| `tools/AGENT_PROTOCOL.md` | Session protocol: investigation workflow, stop conditions |

**Environment variables:** `BLOOD_RUST` (bootstrap compiler path), `RUNTIME_O` (runtime.o), `RUNTIME_A` (libblood_runtime.a), `BLOOD_RUNTIME` (runtime.o, exported), `BLOOD_RUST_RUNTIME` (libblood_runtime.a, exported), `BLOOD_BUILD_DIR` (build output directory), `BLOOD_CACHE` (compilation cache directory).

Run each tool with `--help` or see its header comments for detailed usage.

## Key Patterns (Mistake Prevention)

- **Interner mismatch:** AST parser and HIR lowering use different string interners. Re-intern via `ctx.span_to_string(span)` + `ctx.intern()` when creating HIR items from AST data.
- **Unresolved types in MIR:** Expression types during MIR lowering are `Infer(TyVarId)`. Use `ctx.resolve_type()` to get concrete types before type-based decisions.
- **Field resolution keys:** Use `name.span.start` (field NAME position), not `expr.span.start`. Composite key: `(body_def_id, name_span_start)`.
- **Three `type_to_llvm_with_ctx` functions:** `codegen_ctx` (method, no generics), `codegen_stmt` (standalone, full generics), `codegen_size` (standalone, same as stmt; use from `codegen_expr`).
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

Active work is tracked in `.tmp/WORKLOAD.md`, derived from the spec/implementation divergence audit in `.tmp/AUDIT.md`.

**To pick up work:**
1. Read `.tmp/WORKLOAD.md` — items are organized by tier (Tier 1 = active correctness bugs, down to Tier 5 = open design questions)
2. Work tiers in order: clear all actionable items in Tier N before moving to Tier N+1
3. Within a tier, prioritize by danger score (higher = more urgent)
4. For each item: read the AUDIT.md section for context, the relevant spec, and both compilers
5. Follow the decision procedure:
   - If the spec is right and the implementation is wrong → fix the code (`close-impl-to-spec`)
   - If the spec has a genuine omission or Rust-ism → fix the spec with documented justification (`close-spec-to-impl`)
   - If the answer is unclear → write a design evaluation, research the problem space (`open-question`)
6. After completing work, update WORKLOAD.md with results and verify (build + golden tests + bootstrap)

**Tier-skip rules:** You may skip an item within a tier only if it is genuinely blocked (marked `open-question` with no resolution path, or has unmet prerequisites). When skipping, state the specific reason for each skipped item — do not dismiss a tier as a group. If all remaining items in a tier are blocked or deferred, you may proceed to the next tier after documenting why each was skipped.

Items marked `[-]` are **deferred with re-evaluation triggers** — do not pick them up unless the trigger condition described in WORKLOAD.md is met.

**Score vs tier:** Tiers group by *type* (bugs → spec gaps → missing features → deferred → design questions). Scores indicate *urgency within and across types*. A high-score Tier 3 item does NOT automatically override a low-score Tier 2 item — clear the tier first, skip only with per-item justification.
