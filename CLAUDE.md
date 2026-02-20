# Blood Compiler Development Guidelines

## Repository Structure (Monorepo)

This is a unified monorepo containing both the Blood language project and the Rust bootstrap compiler:

```
blood/                          # Repository root
├── blood-std/std/compiler/     # Self-hosted Blood compiler (written in Blood)
├── compiler-rust/              # Rust bootstrap compiler (imported via git subtree)
│   ├── bloodc/src/             # Rust compiler source
│   ├── runtime/                # C runtime
│   ├── blood-std/              # Standard library (Rust compiler's copy)
│   ├── Cargo.toml              # Workspace manifest
│   └── Makefile                # Build & bootstrap pipeline
├── docs/                       # Language specification & documentation
├── examples/                   # Blood language examples
└── editors/                    # Editor support (VS Code, etc.)
```

### Subtree Management

The `compiler-rust/` directory is managed via `git subtree`. To sync:

```bash
# Pull updates FROM blood-rust
git subtree pull --prefix=compiler-rust blood-rust main --squash

# Push changes back TO blood-rust
git subtree push --prefix=compiler-rust blood-rust main
```

The standalone `blood-rust` repo remains at `~/blood-rust/` for independent development.

---

## Dual Compiler Architecture

This repository contains two parallel compiler implementations:

| Compiler | Location | Language | Purpose |
|----------|----------|----------|---------|
| **Reference** | `compiler-rust/bloodc/src/` | Rust | Bootstrap compiler, leverages Rust ecosystem (inkwell, ariadne) |
| **Self-Hosted** | `blood-std/std/compiler/` | Blood | Self-hosting target, implements everything in Blood |

Both compilers share identical architecture:
```
Source → Lexer → Parser → AST → HIR → Type Check → MIR → Codegen → LLVM
```

### Parity Expectations

**The Blood compiler must match the Rust compiler's behavior for all language semantics.**

When the Blood compiler lacks a feature that the Rust compiler has:
- This is generally a **bug to fix**, not a design decision
- Check `blood-std/std/compiler/COMPILER_NOTES.md` for explicitly documented limitations
- If not documented, implement the missing feature to match Rust

### Blood Language Idioms

Blood is not Rust. These patterns are **correct in Blood**, not shortcuts:

| Pattern | Why It's Correct |
|---------|------------------|
| `while i < len { ... i = i + 1; }` | Blood lacks iterator adapters |
| `i = i + 1` | Blood lacks `+=` operator |
| Explicit match arms for every variant | Required by zero shortcuts principle |
| `HashMap<u32, Type>` vs newtype keys | Blood's type system differs from Rust |

**Do not "improve" Blood code by adding Rust features that don't exist in Blood.**

### Design Documentation

For detailed design decisions, divergences, and known limitations, see:
- `blood-std/std/compiler/COMPILER_NOTES.md`

---

## Prime Directive: Zero Shortcuts

**This codebase must have ZERO shortcuts.** Every pattern match must be exhaustive with proper handling. Every error case must be reported. Every feature must be complete or explicitly error with "not yet implemented."

### What Constitutes a Shortcut

1. **Silent failures**: `_ => Ok(())`, `_ => continue`, returning success without doing work
2. **Placeholder returns**: `Type::error()`, `unwrap_or_default()` hiding real errors
3. **Catch-all patterns**: `_ =>` that should enumerate all cases explicitly
4. **Dead code**: Functions that don't work but aren't removed
5. **Magic numbers**: Hardcoded values like `0` that should be computed
6. **TODO/FIXME without action**: Comments noting problems without fixing them
7. **Silent skips**: `continue` in loops without logging/reporting
8. **Incomplete error messages**: Errors that don't help diagnose the problem

### Required Behavior

- Every match arm must either handle the case properly OR return an explicit error
- Every `unwrap()` must be justified or replaced with proper error handling
- Every `_ =>` must be replaced with explicit variant listing
- Every silent `continue` must either handle the case or report an error
- Every TODO must be addressed or converted to an error

### Audit Checklist

When auditing code, search for:
- `_ =>`
- `unwrap_or_default`
- `unwrap_or_else`
- `Type::error()`
- `continue` (in match arms)
- `Ok(())` (suspicious early returns)
- `TODO`, `FIXME`, `XXX`, `HACK`
- `Phase 2`, `not yet`, `later`
- `unreachable!()`, `panic!()`
- Empty function bodies
- Functions returning hardcoded values

## Current Status

Audit in progress. No shortcuts are acceptable.

---

## Technical Debt Prevention

**Technical debt is the enemy. Prevention is mandatory.**

### File Size Limits

| Category | Max Lines | Action if Exceeded |
|----------|-----------|-------------------|
| Single type file | 200 | Keep as-is |
| Module file | 400 | Consider splitting |
| Monolithic file | 600 | Must split or justify |
| Emergency limit | 800 | Immediate refactoring required |

**Current exceptions (files exceeding 600 lines):**
These files are well-organized internally and contain inherently large logical units:
- `hir_lower_expr.blood` - Expression lowering, 28+ match sections
- `unify.blood` - Type unification with union-find
- `parser_expr.blood` - Pratt parser for expressions
- `typeck_expr.blood` - Expression type checking
- `ast.blood` - All AST node type definitions
- `parser_item.blood` - Top-level item parsing
- `lexer.blood` - Lexer state machine logic
- `hir_item.blood` - HIR item definitions
- `typeck.blood` - Main type checker

### Consistency Requirements

**Shared types are now defined once in `common.blood` and imported.**

| Type | Defined In | Fields |
|------|------------|--------|
| `Span` | `common.blood` | `start: usize`, `end: usize`, `line: u32`, `column: u32` |
| `Symbol` | `common.blood` | `index: u32` |
| `SpannedSymbol` | `common.blood` | `symbol: Symbol`, `span: Span` |
| `SpannedString` | `common.blood` | `value: String`, `span: Span` |
| `OrderedFloat` | `common.blood` | `bits: u64` |

Files import these via `mod common;` and reference as `common::Span`, etc.

### Code Organization Principles

1. **Single Responsibility**: Each file should have one clear purpose
2. **Logical Grouping**: Related types stay together
3. **Dependency Direction**: Lower-level modules don't depend on higher-level
4. **Shared Types in Common**: Define shared types in `common.blood`, import elsewhere

### When to Refactor

**Refactor immediately when:**
- A file exceeds 600 lines without justification
- Duplicate types have inconsistent definitions
- A function exceeds 100 lines
- Nested depth exceeds 4 levels
- Copy-paste is used instead of abstraction

**Do NOT refactor when:**
- It would require features blood-rust doesn't support
- The change is purely cosmetic
- You're in the middle of implementing a feature

---

## Blood-Rust Module System

**The blood-rust module system now supports cross-module types.**

### What Works

| Feature | Example | Status |
|---------|---------|--------|
| External modules | `mod helper;` loads `helper.blood` | ✅ Works |
| Directory modules | `mod sub;` loads `sub/mod.blood` | ✅ Works |
| Qualified struct in expressions | `helper::Data { value: 42 }` | ✅ Works |
| Qualified function calls | `helper::add(1, 2)` | ✅ Works |
| Cross-module types in type position | `pub field: helper::Data` | ✅ Works |
| Chained module paths | `token::common::Span` | ✅ Works |

### What Does NOT Work

| Feature | Example | Status |
|---------|---------|--------|
| `use` imports after declarations | `mod foo; use foo.Bar;` | ❌ Parse error |
| `use` imports finding external modules | `use std.compiler.Span;` | ❌ Module not found |

### Working Module Patterns

**Simple module import:**
```blood
mod common;

pub struct Token {
    pub kind: TokenKind,
    pub span: common::Span,  // Cross-module type works!
}
```

**Chained module paths (preferred for files with @unsafe blocks):**
```blood
mod token;  // token.blood imports common

pub struct Lexer { ... }

impl Lexer {
    fn make_token(self: &Self, kind: token::TokenKind) -> token::Token {
        token::Token {
            kind,
            span: token::common::Span { ... },  // Access through chain
        }
    }
}
```

### Current Modularization

The self-hosted compiler now uses proper module imports:

| File | Imports | Shared Types From |
|------|---------|-------------------|
| `common.blood` | none | (defines canonical types) |
| `token.blood` | `mod common;` | `common::Span` |
| `lexer.blood` | `mod token;` | `token::TokenKind`, `token::Token`, `token::common::Span` |
| `ast.blood` | `mod common;` | `common::Span`, `common::Symbol`, `common::SpannedSymbol`, etc. |

---

## Development Rules

### Rule 1: Compile Before Commit

**Every file must compile with blood-rust before committing.**

```bash
compiler-rust/target/release/blood check <file.blood>
```

If blood-rust rejects the code, the code is wrong. Do NOT modify blood-rust to accept bad syntax.

### Rule 2: Incremental Development

Write in small increments:
1. Write 10-50 lines
2. Compile with blood-rust
3. Fix any errors
4. Repeat
5. Commit when a logical unit is complete

**Never write hundreds of lines without compiling.**

### Rule 3: Use Correct Blood Syntax

**CRITICAL: Blood is NOT Rust. Do not assume Rust syntax applies.**

#### Blood Module Paths (NOT Rust-style)

Blood uses **dot-separated module paths**, NOT Rust's `::` path syntax:

```blood
// CORRECT Blood syntax:
module std.collections.vec;
use std.mem.allocate;
use std.iter.Iterator;

// WRONG - this is Rust, not Blood:
// use crate::module::item;
// use super::sibling;
// use std::collections::Vec;
```

Blood's `::` is ONLY for:
1. **Grouped imports**: `use std.iter::{Iterator, IntoIterator};`
2. **Glob imports**: `use std.ops::*;`
3. **Qualified paths in expressions**: `module::Type { ... }`

**Before assuming ANY syntax, check:** `docs/spec/GRAMMAR.md`

### Rule 4: Blood-Rust Compiler Bugs Must Be Reported, NOT Worked Around

**CRITICAL: When you encounter a blood-rust compiler bug, you MUST NOT work around it. NO EXCEPTIONS.**

**This means:**
- Do NOT clone data structures to avoid mutation bugs — write the correct code
- Do NOT add "optimizations" that bypass broken code paths — fix the root cause
- Do NOT restructure correct code to avoid triggering compiler bugs — report the bug
- Do NOT add any code whose purpose is to compensate for blood-rust misbehavior

**Write the code the way it SHOULD work.** If blood-rust doesn't handle it correctly, that is a blood-rust bug. The self-hosted compiler code must be correct, not contorted to work around a broken bootstrap compiler.

A blood-rust bug is identified when:
- Code compiles in isolation but fails when imported by another module
- The error message references internal DefIds (e.g., `"def921" is not a struct`)
- Syntactically correct code is rejected
- The same pattern works in one context but not another
- Mutations through references are silently lost
- Runtime behavior doesn't match what the code should do

**When you identify a potential blood-rust bug:**

1. **STOP** - Do not attempt workarounds, band-aids, or alternative syntax
2. **Write the correct code** - The self-hosted compiler must have the RIGHT implementation
3. **Isolate** - Create a minimal reproduction case
4. **Document** - Record the bug in the "Known Blood-Rust Bugs" section below
5. **Report** - The bug must be communicated to blood-rust developers
6. **Wait** - Do not proceed with workarounds; the bug must be fixed at the source

**Why this matters:**
- Workarounds create technical debt that compounds over time
- Band-aids mask the real problem and make future debugging harder
- Workarounds on top of workarounds create exponential complexity
- The blood-rust compiler should be fixed to support valid Blood code
- Shortcuts violate the Zero Shortcuts principle
- A "working" workaround today becomes an unmaintainable mess tomorrow

**What is NOT a blood-rust bug:**
- Blood syntax that differs from Rust (documented in this file)
- Features that blood-rust explicitly doesn't support yet (documented limitations)
- Code that uses incorrect Blood syntax

### Rule 5: Document Discoveries

When you discover a limitation or constraint:
1. Test it in isolation with a minimal example
2. Document it in this file
3. Add a comment in the affected code
4. Only then work around it (if it's a documented limitation, NOT a bug)

### Rule 6: Maintain Consistency

Before modifying any shared type:
1. Check `common.blood` for the canonical definition
2. Update ALL files that duplicate the type
3. Verify all files still compile
4. Document the change

---

## Known Blood-Rust Bugs

**These are compiler bugs that need to be fixed in blood-rust. Do NOT work around them.**

### BUG-008: If-expression with inline function call condition is miscompiled (FIXED)

**Severity:** Was critical (blocked self-hosting)

**Pattern that triggered the bug:**
```blood
fn example(arg: &Type) -> String {
    if some_function(arg) {
        common::make_string("result_a")
    } else {
        common::make_string("result_b")
    }
}
```

**Symptom:** The conditional branch was eliminated entirely. Generated LLVM IR unconditionally executed the `else` branch, ignoring the function call result.

**Status:** FIXED by blood-rust developers. The branch elimination bug is resolved.

**Related issue (also fixed):** The self-hosted compiler's hardcoded runtime call handlers had calling convention mismatches (`string_push_str`, `string_as_str`, `string_as_bytes`). These passed `{ ptr, i64 }` by value where the C ABI expects `ptr` to stack, and declared return types as `ptr` instead of `{ ptr, i64 }`. Fixed in `codegen.blood` (declarations) and `codegen_term.blood` (call emission). The generated IR now has correct calling conventions.

**Remaining issue:** The second-gen binary still segfaults at startup in `vec_push` called from `intern_keywords`. This is a separate issue from BUG-008 — likely another codegen or ABI mismatch in the self-hosted compiler's output that needs investigation.

### BUG-009: Effect handler state limited to ≤ 8-byte types (ICE on String/Vec/aggregate state) (FIXED)

**Severity:** Was critical (blocked effect-abstracted codegen)

**Status:** FIXED by blood-rust developers in commits `10261fc` + `b25403e`.

**Fix 1 (10261fc):** Typed layout for handler state structs — `handlers.rs` now computes correct LLVM type per state field instead of uniform `[i64 x N]`. Direct loads/stores when types match.

**Fix 2 (b25403e):** Out-pointer builtins + pointer casting in HIR expr path — `expr.rs` now supports `string_new`/`str_to_string` via out-pointer pattern, pointer type coercion for method calls, and `Ref`/`RefMut`/`Deref` in `compile_unary`.

**All patterns now work:**
- Immutable String state (`let path: String`) ✅
- Mutable String state (`let mut buffer: String`) ✅
- `push_str` in handler op bodies ✅
- `&mut` references in handler op bodies ✅
- Delegation to regular functions from handler ops ✅

### BUG-011: `&&` and `||` operators do not implement short-circuit evaluation (FIXED)

**Severity:** Was critical (blocked self-hosting pipeline)

**Status:** FIXED by blood-rust developers.

**Fix:** Implemented short-circuit evaluation in two codegen paths:
1. MIR lowering (`util.rs`): `lower_short_circuit()` emits conditional control flow with separate basic blocks
2. HIR expr codegen (`expr.rs`): `compile_short_circuit()` emits LLVM conditional branch + phi node

Tests: `t06_short_circuit_and`, `t06_short_circuit_or`, `t06_short_circuit_guard`, `t06_short_circuit_chain`, `t06_short_circuit_while`, `t06_short_circuit_or_mutate`.

### BUG-012: Deref handler generation check uses wrong address and wrong panic argument (FIXED)

**Severity:** Critical (caused false stale reference crashes in first_gen)

**Status:** FIXED.

**Root cause:** Two bugs in the inline generation check code at `place.rs:217-297` in the Deref handler:

**Bug A — Wrong address (place.rs:230):** `ptrtoint(ptr_val)` converted the *loaded/derived* value to an address. For Region-allocated locals where the loaded value is a StructValue, lines 176-185 spill it to a stack temporary (`deref_tmp` alloca). The generation check then validated the *stack address* of this temporary instead of the *heap address* of the Region allocation. The runtime correctly determined the stack address wasn't in its slot registry and reported "stale" (since `expected_gen > FIRST` for garbage values).

Diagnostic output confirmed: `addr=0x7ffdd62ce558` (stack range), `expected_gen=3593265224` (non-deterministic garbage from uninitialized stack reads).

**Fix A:** Changed to use `ptrtoint(locals[local_id])` — the local's actual storage pointer (heap address for Region-allocated locals).

**Bug B — Wrong panic argument (place.rs:283):** `blood_stale_reference_panic(expected_gen, result)` passed `result` (the 0/1 return code from `blood_validate_generation`) as the "actual generation" argument. The error message always showed "Actual: 1" regardless of the real generation.

**Fix B:** Added `blood_get_generation(address)` call in the stale path to retrieve the actual generation before calling the panic function, matching the correct pattern in `memory.rs:emit_generation_check_impl`.

**Key files:** `compiler-rust/bloodc/src/codegen/mir_codegen/place.rs` (lines 229-306)

### BUG-013: Option<&T> variant field type resolved as i32 instead of ptr (FIXED)

**Severity:** Critical (caused segfault in ALL first_gen operations — check, build)

**Status:** FIXED.

**Root cause:** `compute_place_type()` in `place.rs` had hardcoded Option/Result field handling that ignored `variant_ctx` (set by `Downcast`). After `Downcast(1)` (entering `Some` variant), MIR `Field(0)` means "field 0 of the variant" (the `T` payload). But the hardcoded code treated it as "field 0 of the ADT struct" (the `i32` discriminant tag).

For `Option<&ItemEntry>`, this meant:
- `Downcast(1)` → `Field(0)` should resolve to `&ItemEntry` (a pointer, `ptr` in LLVM)
- Instead resolved to `i32` (the discriminant type)
- Generated: `load i32` + `inttoptr i32 %val to ptr` — truncating 64-bit pointers to 32 bits
- Result: segfault on first dereference of the truncated pointer

**Fix:** Added a check for `variant_ctx.is_some()` before the Option/Result hardcoded paths. When in a variant context, resolve variant-specific field types (Some field 0 = T, Ok field 0 = T, Err field 0 = E) instead of ADT-level struct fields.

**Key file:** `compiler-rust/bloodc/src/codegen/mir_codegen/place.rs` (`compute_place_type`)

### Known Blood-Rust Limitations (NOT Bugs)

**Memory reuse requires active region:** The runtime now includes a Generation-Aware Slab Allocator that enables memory reuse within regions. For compiled Blood programs to benefit, they must create and activate a region at startup. The `blood_alloc_simple`, `blood_realloc`, and `blood_free_simple` functions are now region-aware: when a region is active, allocations come from the region and freed memory is added to per-size-class free lists for reuse. The codegen already calls `blood_unregister_allocation` for region-allocated locals in StorageDead statements.

**Module resolution limits:** Adding `mod codegen_ctx;` to driver.blood caused `source::read_file` and `source::parent_dir` to become unresolvable in later functions. Workaround: avoid adding new module imports to files near the resolution limit.

**Previously fixed bugs:**
- BUG-001: Struct initialization in impl blocks when module is imported (fixed)
- BUG-002: Enum payload corruption when moving structs with large enum fields into another enum (fixed — verified 2026-01-31, all payload tests pass)
- BUG-003: Option<&Struct> return corruption (fixed — blood-rust devs added `by_ref` field tracking)
- BUG-004: Option::Some(Box::new(expr)) corruption (fixed — blood-rust devs added auto-deref insertion)
- BUG-005: Mutations through `&mut field_of_ref` lost when passed as function arguments (fixed)
- BUG-006: Match on enum reference (`&Enum`) always falls to last arm (fixed — verified 2026-01-31, by-ref match now dispatches correctly)
- BUG-007: Generic type params not registered in resolver scope at runtime (fixed — blood-rust devs fixed nested mutable struct field codegen)
- BUG-009: Effect handler state limited to ≤ 8-byte types (fixed — blood-rust devs added typed handler state layout + out-pointer builtins + pointer casting in HIR expr path, commits `10261fc` + `b25403e`)
- BUG-010: `push_str` with `&str` op arg in handler op body passed value instead of pointer (fixed — blood-rust devs added struct-to-pointer coercion in `compile_call`, test: `t05_handler_push_str.blood`)
- BUG-011: `&&` and `||` operators did not short-circuit (fixed — blood-rust devs added `lower_short_circuit()` in MIR lowering + `compile_short_circuit()` in HIR expr codegen, 6 ground-truth tests)

---

## Known Syntax Constraints

| Constraint | Example That Fails | Workaround |
|------------|-------------------|------------|
| Some keywords as field names | `pub module: ...` | Rename: `mod_decl` |

**Fixed constraints (no longer apply):**
- Cross-module types in type position now work (e.g., `pub field: mod::Type`)
- Cross-module enum variants now work (e.g., `mod::Enum::Variant`)
- Nested generics like `Option<Box<Expr>>` now work (fixed in commit 40a4efe)
- Field name `end` works (was incorrectly thought to be a keyword)
- Vec.push() now works with all types (was broken due to generic type inference bug)
- Format strings support all integer types (fixed in commit 61c8d43)
- `use` imports after `mod` declarations now work
- Cross-module associated functions on enums now work
- Transitive dependencies now resolved automatically
- `&str` methods (.len(), .as_bytes()) now work
- `pub use` re-exports work for structs, enums (construction, methods, AND pattern matching)

---

## Compiler Phases

Build in this order, testing each phase before moving on:

1. **Common types** - `common.blood`
2. **Tokens** - `token.blood`
3. **Lexer** - `lexer.blood`
4. **AST** - `ast.blood`
5. **Parser** - `parser*.blood` files
6. **HIR definitions** - `hir*.blood` files
7. **Name resolution** - `resolve.blood`
8. **HIR lowering** - `hir_lower*.blood` files
9. **Type checking** - `unify.blood`, `typeck*.blood` files
10. **MIR** - `mir_*.blood` files
11. **Codegen** - `codegen*.blood` files
12. **Infrastructure** - `interner.blood`, `driver.blood`, `reporter.blood`, `source.blood`, `main.blood`, `const_eval.blood`

All phases are complete and type-check successfully.

---

## Testing

```bash
# Check syntax/types
compiler-rust/target/release/blood check file.blood

# Build executable
compiler-rust/target/release/blood build file.blood

# Run
compiler-rust/target/release/blood run file.blood

# Build the Rust bootstrap compiler
cd compiler-rust && cargo build --release

# Run Rust compiler tests
cd compiler-rust && cargo test --workspace
```

---

## Development Tools (`tools/`)

### Differential Testing Harness — `tools/difftest.sh`

Compiles the same `.blood` file with both blood-rust (reference) and first_gen (test), then compares results.

**Modes:**
- `--behavioral` (default): Compile and run both executables, compare stdout and exit code
- `--ir`: Extract per-function LLVM IR, match by name, and diff normalized IR

**Usage:**
```bash
# Single file — behavioral comparison
./tools/difftest.sh path/to/test.blood

# Batch — all tests in a directory (skips COMPILE_FAIL tests)
./tools/difftest.sh compiler-rust/tests/ground-truth/ --summary-only

# IR-level comparison with details
./tools/difftest.sh path/to/test.blood --ir --verbose

# Stop at first divergent function
./tools/difftest.sh path/to/test.blood --ir --first-divergence
```

**Output categories:**
- `MATCH` — both compilers produce identical output (behavioral) or identical IR (ir mode)
- `DIVERGE` — both compile and run but output or IR differs
- `TEST_FAIL` — test compiler fails, reference succeeds
- `REF_FAIL` — reference compiler fails
- `BOTH_FAIL` — both fail (consistent behavior)

**Environment variables:**
- `BLOOD_REF` — path to reference compiler (default: `~/blood/compiler-rust/target/release/blood`)
- `BLOOD_TEST` — path to test compiler (default: `~/blood/blood-std/std/compiler/first_gen`)
- `BLOOD_RUNTIME` — path to `runtime.o`
- `BLOOD_RUST_RUNTIME` — path to `libblood_runtime.a`

**When to use:** Run after any codegen change to verify the self-hosted compiler still produces behaviorally correct binaries. The `DIVERGE` results are the highest-priority bugs — they mean both compilers accept the code but produce different runtime behavior.

### Test Case Minimizer — `tools/minimize.sh`

Automatically reduces a `.blood` file to the smallest program that still triggers a given bug. Supports four failure modes.

**Usage:**
```bash
# Auto-detect failure mode and minimize
./tools/minimize.sh path/to/failing_test.blood

# Explicit failure mode
./tools/minimize.sh path/to/test.blood --mode compile-fail
./tools/minimize.sh path/to/test.blood --mode wrong-output
./tools/minimize.sh path/to/test.blood --mode crash
./tools/minimize.sh path/to/test.blood --mode compile-crash

# Keep work directory for inspection
./tools/minimize.sh path/to/test.blood --keep-temps
```

**Failure modes:**
- `compile-fail` — test compiler rejects, reference accepts
- `compile-crash` — test compiler crashes/aborts during compilation
- `crash` — both compile, but test executable crashes
- `wrong-output` — both compile+run, but output differs

**Output:** Minimized source printed to stdout and saved as `<name>.min.blood` next to the original. Progress printed to stderr.

**Reduction strategy:** Removes top-level items (structs, fns, enums, impls), then individual statements, re-checking the oracle after each removal. Typically achieves 5-10x reduction.

**When to use:** When a test fails and the file is too large to understand at a glance. Run the minimizer first, then debug the 5-10 line output instead of the 50+ line original.

### Phase-Gated Comparison — `tools/phase-compare.sh`

Runs both compilers on a single `.blood` file and compares at each compilation phase to identify WHERE divergence first appears. Pinpoints whether the bug is in compilation, MIR, LLVM IR, or runtime behavior.

**Usage:**
```bash
# Compare a single file across all phases
./tools/phase-compare.sh path/to/test.blood

# With verbose output (shows MIR summaries, function lists)
./tools/phase-compare.sh path/to/test.blood --verbose
```

**Phases compared:**
1. **Compilation** — Do both compilers accept or reject the input?
2. **MIR** — Do both produce structurally similar MIR? (function count, basic block count, locals count)
3. **LLVM IR** — Do both produce similar LLVM IR? (define/declare counts, total lines)
4. **Behavior** — Do both executables produce identical stdout and exit code?

**Output per phase:**
- `MATCH` — both compilers agree at this phase
- `DIFFER` — structural metrics differ (informational for MIR/IR since codegen strategies differ)
- `DIVERGE` — functional divergence detected (this is a bug)
- `SKIP` — phase data unavailable from one or both compilers

**Exit codes:**
- `0` — all phases match (or only informational differences)
- `1` — functional divergence detected at some phase

**Environment variables:** Same as `difftest.sh` (`BLOOD_REF`, `BLOOD_TEST`, `BLOOD_RUNTIME`, `BLOOD_RUST_RUNTIME`).

**When to use:** When `difftest.sh` reports DIVERGE or TEST_FAIL, use this tool to narrow down which compilation phase first diverges. Phase 1 divergence = parser/typeck bug. Phase 4 divergence = codegen bug.

### Memory Budget Tracker — `tools/memprofile.sh`

Profiles memory usage of compilation runs with per-phase timing breakdowns.

**Usage:**
```bash
# Summary mode (default): peak RSS + phase timings for both compilers
./tools/memprofile.sh path/to/test.blood

# Side-by-side comparison table
./tools/memprofile.sh path/to/test.blood --compare

# RSS sampling mode (polls /proc/PID/status every 50ms, shows timeline)
./tools/memprofile.sh path/to/test.blood --sample

# Valgrind massif heap profiling (slow but detailed)
./tools/memprofile.sh path/to/test.blood --massif

# Single compiler only
./tools/memprofile.sh path/to/test.blood --ref-only
./tools/memprofile.sh path/to/test.blood --test-only
```

**Modes:**
- `--summary` (default): Peak RSS via `/usr/bin/time -v`, plus `--timings` phase breakdown
- `--compare`: Side-by-side table of both compilers' peak RSS, wall time, and phase timings
- `--sample`: Background RSS polling with ASCII timeline chart
- `--massif`: Full heap profile via `valgrind --tool=massif`

**When to use:** When self-compilation uses too much memory or when investigating memory growth across compiler phases. The `--compare` mode quickly shows if first_gen uses more memory than blood-rust for the same input.

### Failure History Log — `tools/FAILURE_LOG.md`

Structured, machine-readable log of past failures, root causes, and resolutions. Prevents future sessions from re-discovering the same issues.

**Format:** Markdown tables with columns: date, category, symptom, root cause, resolution, files.

**Sections:**
- **Active Issues** — unresolved problems with current status
- **Resolved Issues** — 20+ entries seeded from the full bug history
- **Patterns and Anti-Patterns** — common root causes to check first, debugging workflow

**When to use:**
- **Before debugging:** Search for similar symptoms to avoid rediscovering known issues
- **After resolving:** Add a new entry at the top of the resolved table
- **During onboarding:** Read the "Common Root Causes" section first

### ASan Self-Compilation Wrapper — `tools/asan-selfcompile.sh`

One-command pipeline that builds an ASan-instrumented second_gen and runs it, reporting memory errors with formatted stack traces.

**Usage:**
```bash
# Full pipeline: build first_gen → self-compile → ASan instrument → run
./tools/asan-selfcompile.sh

# Reuse existing first_gen (skip blood-rust rebuild)
./tools/asan-selfcompile.sh --reuse

# Instrument existing IR directly
./tools/asan-selfcompile.sh --ir path/to/second_gen.ll

# Just re-run existing ASan binary with different test
./tools/asan-selfcompile.sh --run-only --test "check common.blood"
```

**Pipeline steps:**
1. Build first_gen from blood-rust (or reuse existing)
2. Self-compile: first_gen → second_gen.ll
3. Instrument with ASan: llvm-as → opt (asan passes) → llc → clang -fsanitize=address
4. Run the ASan binary and format the sanitizer report

**Output:** Color-formatted ASan report with highlighted function names, error types, and stack traces. Full log saved to temp file.

**Requirements:** LLVM 18 tools (llvm-as-18, opt-18, llc-18, clang-18).

**When to use:** When self-compilation crashes (SIGSEGV, heap corruption) and you need to identify the exact memory error. The ASan report will show use-after-free, buffer overflow, etc. with precise stack traces.

### FileCheck Test Coverage Audit — `tools/filecheck-audit.sh`

Inventories existing FileCheck tests, scans compiler source for codegen patterns, and reports coverage gaps with prioritized recommendations.

**Usage:**
```bash
# Full audit (existing tests + patterns + ground-truth + gaps)
./tools/filecheck-audit.sh

# Just list existing tests
./tools/filecheck-audit.sh --tests-only

# Just show coverage gaps and recommendations
./tools/filecheck-audit.sh --gaps-only

# Just show recommended new tests
./tools/filecheck-audit.sh --recommend
```

**Output sections:**
- Existing FileCheck tests with CHECK directive counts and pattern coverage
- Codegen patterns in the compiler (IR emission counts, feature usage levels H/M/L)
- Ground-truth test feature coverage (struct, enum, trait, etc.)
- Coverage gaps with 15 recommended tests (HIGH/MEDIUM/LOW priority)

**When to use:** Before creating new FileCheck tests, run this audit to see what's already covered and where the biggest gaps are. Focus on HIGH priority recommendations first.

### MIR Validation Gate — `tools/validate-all-mir.sh`

Runs `--validate-mir` on a set of `.blood` files and reports pass/fail per file with error details. Acts as a pre-codegen quality gate.

**Usage:**
```bash
# Validate ground-truth tests (default)
./tools/validate-all-mir.sh

# Validate single file
./tools/validate-all-mir.sh path/to/test.blood

# Validate all files in a directory
./tools/validate-all-mir.sh path/to/dir/

# Validate compiler source files
./tools/validate-all-mir.sh --self

# Use reference compiler instead of test compiler
./tools/validate-all-mir.sh --compiler REF
```

**Output:** Per-file PASS (silent), FAIL (with error excerpt), CRASH (with signal), or MIR (with MIR-specific errors). Summary with pass/fail/skip counts.

**When to use:** After modifying MIR lowering code, run this to verify structural correctness before testing codegen. Catches type mismatches, malformed basic blocks, and other MIR-level issues early.

**Task tracker:** See `tools/TASKS.md` for the full infrastructure roadmap.

---

## Reference

- **Blood-rust compiler**: `compiler-rust/` (imported via git subtree from `~/blood-rust/`)
- **Grammar spec**: `docs/spec/GRAMMAR.md`
- **Aether examples**: `~/blood-test/aether/` (demonstrates correct syntax)
