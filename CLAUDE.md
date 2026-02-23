# Blood Compiler Development Guidelines

## Project Structure

```
blood/                          # Repository root
├── src/selfhost/     # Self-hosted Blood compiler (written in Blood)
├── src/bootstrap/bloodc/src/   # Rust bootstrap compiler (git subtree from ~/blood-rust/)
├── docs/spec/GRAMMAR.md        # Language grammar specification
└── tools/                      # Development & debugging tools
```

| Compiler | Location | Language | Purpose |
|----------|----------|----------|---------|
| **Reference** | `src/bootstrap/bloodc/src/` | Rust | Bootstrap compiler |
| **Self-Hosted** | `src/selfhost/` | Blood | Self-hosting target |

Pipeline: `Source → Lexer → Parser → AST → HIR → Type Check → MIR → Codegen → LLVM`

The Blood compiler must match the Rust compiler's behavior. Mismatches are bugs unless documented in `COMPILER_NOTES.md`.

## Blood is NOT Rust

Blood uses **dot-separated module paths**, NOT Rust's `::`:

```blood
use std.mem.allocate;       // CORRECT
// use std::mem::allocate;  // WRONG — this is Rust
```

`::` is ONLY for: grouped imports (`use std.iter::{A, B}`), glob imports (`use std.ops::*`), and qualified expressions (`module::Type { ... }`).

These patterns are **correct in Blood**, not shortcuts:

| Pattern | Why It's Correct |
|---------|------------------|
| `while i < len { ... i = i + 1; }` | Blood lacks iterator adapters |
| `i = i + 1` | Blood lacks `+=` operator |
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
# Check syntax/types
src/bootstrap/target/release/blood check file.blood

# Build executable (output: build/debug/<stem> relative to source file)
src/bootstrap/target/release/blood build file.blood

# Build with custom output directory
src/bootstrap/target/release/blood build file.blood --build-dir /tmp/mybuild

# Build with explicit output path (-o overrides only the final binary)
src/bootstrap/target/release/blood build file.blood -o /tmp/mybin

# Run
src/bootstrap/target/release/blood run file.blood

# Build the Rust bootstrap compiler
cd src/bootstrap && cargo build --release

# Build first_gen (self-hosted, via build script)
cd src/selfhost && ./build_selfhost.sh timings
# Output: src/selfhost/build/first_gen

# Run ground-truth tests
cd src/selfhost && ./build_selfhost.sh ground-truth
```

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
| `tools/minimize.sh` | Reduce failing test to minimal reproduction |
| `tools/phase-compare.sh` | Identify which compilation phase first diverges |
| `tools/memprofile.sh` | Profile memory usage with per-phase breakdown |
| `tools/asan-selfcompile.sh` | Build ASan-instrumented second_gen for memory debugging |
| `tools/filecheck-audit.sh` | Audit FileCheck test coverage and recommend gaps |
| `tools/validate-all-mir.sh` | Pre-codegen MIR validation gate |
| `tools/track-regression.sh` | Detect ground-truth test regressions vs baseline |
| `tools/FAILURE_LOG.md` | Structured log of past bugs, root causes, resolutions |
| `tools/AGENT_PROTOCOL.md` | Session protocol: investigation workflow, stop conditions |

**Environment variables:** `BLOOD_REF` (reference compiler), `BLOOD_TEST` (test compiler), `BLOOD_RUNTIME` (runtime.o), `BLOOD_RUST_RUNTIME` (libblood_runtime.a), `BLOOD_BUILD_DIR` (build output directory), `BLOOD_CACHE` (compilation cache directory).

Run each tool with `--help` or see its header comments for detailed usage.

## Key Patterns (Mistake Prevention)

- **Interner mismatch:** AST parser and HIR lowering use different string interners. Re-intern via `ctx.span_to_string(span)` + `ctx.intern()` when creating HIR items from AST data.
- **Unresolved types in MIR:** Expression types during MIR lowering are `Infer(TyVarId)`. Use `ctx.resolve_type()` to get concrete types before type-based decisions.
- **Field resolution keys:** Use `name.span.start` (field NAME position), not `expr.span.start`. Composite key: `(body_def_id, name_span_start)`.
- **Three `type_to_llvm_with_ctx` functions:** `codegen_ctx` (method, no generics), `codegen_stmt` (standalone, full generics), `codegen_size` (standalone, same as stmt; use from `codegen_expr`).
- **TypeError API:** Use `checker.error(TypeErrorKind::Variant, span)` — NOT `checker.errors.push(TypeError::new(...))`.
- **Trait default method remapping:** See `CallRemapEntry` in `typeck_driver.blood` + `extract_direct_fn_name` in `codegen_term.blood`.

## Reference

- **Grammar spec**: `docs/spec/GRAMMAR.md`
- **Design docs**: `src/selfhost/COMPILER_NOTES.md`
- **Aether examples**: `~/blood-test/aether/`
- **Bug history**: `tools/FAILURE_LOG.md`
- **Session protocol**: `tools/AGENT_PROTOCOL.md`
