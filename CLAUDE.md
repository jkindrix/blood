# Blood Compiler Development Guidelines

## Dual Compiler Architecture

This repository contains two parallel compiler implementations:

| Compiler | Location | Language | Purpose |
|----------|----------|----------|---------|
| **Reference** | `bloodc/src/` | Rust | Bootstrap compiler, leverages Rust ecosystem (inkwell, ariadne) |
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
/home/jkindrix/blood-rust/target/release/blood check <file.blood>
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

**Before assuming ANY syntax, check:** `/home/jkindrix/blood/docs/spec/GRAMMAR.md`

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
/home/jkindrix/blood-rust/target/release/blood check file.blood

# Build executable
/home/jkindrix/blood-rust/target/release/blood build file.blood

# Run
/home/jkindrix/blood-rust/target/release/blood run file.blood
```

---

## Reference

- **Blood-rust compiler**: `~/blood-rust/` (commit 61c8d43)
- **Grammar spec**: `/home/jkindrix/blood/docs/spec/GRAMMAR.md`
- **Aether examples**: `~/blood-test/aether/` (demonstrates correct syntax)
