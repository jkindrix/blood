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
- `hir_lower_expr.blood` (1,709 lines) - Expression lowering, well-organized with 28 sections
- `unify.blood` (1,232 lines) - Type unification with union-find
- `parser_expr.blood` (1,179 lines) - Pratt parser for expressions
- `typeck_expr.blood` (1,113 lines) - Expression type checking
- `ast.blood` (1,070 lines) - All AST node types
- `parser_item.blood` (1,034 lines) - Top-level item parsing
- `lexer.blood` (867 lines) - Lexer state machine logic
- `hir_item.blood` (794 lines) - HIR item definitions
- `typeck.blood` (788 lines) - Main type checker

**Note:** These files should be split when practical but are well-organized internally.

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

### BUG-002: Enum payload corruption when moving structs with large enum fields into another enum

**Status:** Active - blocking self-hosted compiler codegen

**Description:**
When a struct containing an enum with a large payload (e.g., `i128`) is moved into another enum variant, the payload data gets corrupted.

**Reproduction:**
```blood
// This is simplified - actual case involves ConstantKind, Constant, Operand, Rvalue
enum Inner {
    Int(i128),  // 128-bit payload
    ZeroSized,
}

struct Container {
    kind: Inner,
}

enum Outer {
    Wrap(Container),
}

fn test() {
    let container = Container { kind: Inner::Int(42) };
    // At this point, container.kind == Inner::Int(42) ✅
    let outer = Outer::Wrap(container);
    // At this point, accessing inner.kind shows corrupted data ❌
}
```

**Actual case:**
- `mir_types::ConstantKind::Int(i128)` value (42) is correct before wrapping
- After `mir_types::Operand::Constant(constant)` where `constant: Constant { ty, kind }`, the `kind` field is corrupted
- The discriminant appears to change to a different variant (e.g., `ZeroSized` instead of `Int`)

**Impact:**
- Self-hosted compiler generates `undef` instead of actual constant values
- LLVM IR output: `store i64 undef, ptr %_0` instead of `store i32 42, ptr %_0`

**Workaround:**
None known. This requires a fix in blood-rust's codegen for enum payloads.

### BUG-005: Mutations through `&mut field_of_ref` lost when passed as function arguments

**Status:** Active — **blocking self-hosted compiler type checker**

**Description:**
When you take `&mut` of a field accessed through another reference (e.g., `&mut checker.subst_table` where `checker` is `&mut TypeChecker`) and pass it as a function argument, mutations made inside the called function are lost when the function returns. blood-rust appears to copy the field to a stack temporary and pass `&mut` to the copy instead of computing a GEP pointer to the original field.

**Reproduction:**
```blood
struct Inner {
    pub values: Vec<i32>,
}

struct Outer {
    pub inner: Inner,
}

fn add_value(inner: &mut Inner) {
    inner.values.push(42);  // This mutation is LOST
}

fn test(outer: &mut Outer) {
    add_value(&mut outer.inner);  // Bug: &mut field_of_ref
    // outer.inner.values is still empty here!

    // But this works:
    outer.inner.values.push(42);  // Direct method call chain — mutation preserved
}
```

**What works and what doesn't:**

| Pattern | Works? |
|---------|--------|
| `outer.inner.method()` (direct method call through ref chain) | ✅ |
| `helper(outer)` where `outer: &mut Outer` (pass whole ref) | ✅ |
| `helper(&mut local_var)` where `local_var` is a local | ✅ |
| `helper(&mut outer.inner)` — `&mut field_of_ref` as fn arg | ❌ |

**Impact — correctness:**
- `TypeChecker::unify()` calls `unify::unify(&mut self.subst_table, &mut self.unifier, ...)` — both `&mut field_of_ref` arguments lose all mutations on return
- This means type inference silently produces wrong results (substitutions lost)
- The type checker is written correctly in `typeck.blood` but blood-rust breaks it at codegen

**Impact — performance (if workaround were applied):**
- A previous clone-and-copy-back workaround was attempted but created O(n²) performance: cloning the entire SubstTable (O(n)) per unify call × hundreds of unify calls per function body = quadratic total work
- By function body 41 of lexer.blood, the table had 2297 entries, causing the type checker to hang (30+ seconds)
- This demonstrates why workarounds must not be applied — they create cascading problems

**Self-hosted compiler state:**
- `typeck.blood` contains the CORRECT code: `unify::unify(&mut self.subst_table, &mut self.unifier, ...)`
- This code is correct but produces wrong results until BUG-005 is fixed
- No workaround is applied. The bug must be fixed in blood-rust.

**Root cause:**
blood-rust codegen evaluates `&mut expr.field` as a function argument by: (1) loading the field value to a stack temporary, (2) taking `&mut` of the temporary. It should instead compute the address via GEP on the struct pointer.

**Workaround:** None. Write the correct code and wait for the fix.

---

**Previously fixed bugs:**
- BUG-001: Struct initialization in impl blocks when module is imported (fixed - all 25 compiler files now compile)
- BUG-003: Option<&Struct> return corruption (fixed - blood-rust devs added `by_ref` field tracking)
- BUG-004: Option::Some(Box::new(expr)) corruption (fixed - blood-rust devs added auto-deref insertion for ref bindings in method calls)

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

| Phase | File(s) | Lines | Status |
|-------|---------|-------|--------|
| 1 | `common.blood` | 273 | ✅ Complete |
| 2 | `token.blood` | 614 | ✅ Complete |
| 3 | `lexer.blood` | 867 | ✅ Complete |
| 4 | `ast.blood` | 1,070 | ✅ Complete |
| 5 | `parser*.blood` (6 files) | 3,966 | ✅ Complete |
| 6 | `hir*.blood` (5 files) | 2,376 | ✅ Complete |
| 7 | `resolve.blood` | 605 | ✅ Complete |
| 8 | `hir_lower*.blood` (6 files) | 2,704 | ✅ Complete |
| 9 | `unify.blood`, `typeck*.blood` (6 files) | 5,312 | ✅ Complete |
| 10 | `mir_*.blood` (10 files) | 5,011 | ✅ Complete |
| 11 | `codegen*.blood` (6 files) | 2,224 | ✅ Complete |
| 12 | Infrastructure (6 files) | 2,148 | ✅ Complete |

**Infrastructure files:** `interner.blood` (286), `driver.blood` (547), `reporter.blood` (364), `source.blood` (372), `main.blood` (369), `const_eval.blood` (210)

**Total: 53 files, 30,631 lines - All type-check successfully.**

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
