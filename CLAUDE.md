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

**Current exceptions** (justified by blood-rust limitations):
- `lexer.blood` (1206 lines) - Self-contained due to module system limitations
- `ast.blood` (1161 lines) - Interconnected types, splitting not possible without cross-module types

### Consistency Requirements

**All duplicate type definitions MUST be identical.**

When types are duplicated across files (necessary due to blood-rust limitations):

1. **Canonical source**: `common.blood` is the authoritative reference
2. **Field names**: Must match exactly across all definitions
3. **Method signatures**: Must match exactly
4. **Documentation**: Keep in sync

**Consistency checklist for shared types:**
```
Span: start, end, line, column (all usize/u32)
Symbol: index (u32)
SpannedSymbol: symbol (Symbol), span (Span)  // NOT 'value'
SpannedString: value (String), span (Span)
OrderedFloat: bits (u64)
```

### Code Organization Principles

1. **Single Responsibility**: Each file should have one clear purpose
2. **Logical Grouping**: Related types stay together
3. **Dependency Direction**: Lower-level modules don't depend on higher-level
4. **Self-Containment**: Until blood-rust supports cross-module types, each file must define all types it needs

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

## Blood-Rust Module System Limitations

**CRITICAL: Understand these limitations before designing file structure.**

### What Works

| Feature | Example | Status |
|---------|---------|--------|
| External modules | `mod helper;` loads `helper.blood` | ✅ Works |
| Directory modules | `mod sub;` loads `sub/mod.blood` | ✅ Works |
| Qualified struct in expressions | `helper::Data { value: 42 }` | ✅ Works |
| Qualified function calls | `helper::add(1, 2)` | ✅ Works |

### What Does NOT Work

| Feature | Example | Status |
|---------|---------|--------|
| Cross-module types in type position | `pub field: helper::Data` | ❌ Fails |
| Cross-module enum variants | `helper::Kind::A` | ❌ Fails |
| `use` imports after declarations | `mod foo; use foo.Bar;` | ❌ Parse error |
| `use` imports finding external modules | `use std.compiler.Span;` | ❌ Module not found |

### Practical Implications

**You CANNOT:**
```blood
// This will NOT work
mod common;

pub struct Token {
    pub kind: common::TokenKind,  // ERROR: cross-module type in type position
    pub span: common::Span,       // ERROR: same issue
}
```

**You MUST:**
```blood
// Each file defines all types it needs inline
pub struct Span { pub start: usize, pub end: usize, ... }
pub enum TokenKind { ... }
pub struct Token { pub kind: TokenKind, pub span: Span }
```

### Workaround Strategy

1. **Canonical Reference**: Keep `common.blood` as the authoritative type definitions
2. **Copy, Don't Import**: Duplicate types in each file that needs them
3. **Consistency Enforcement**: Manually ensure duplicates stay synchronized
4. **Future-Proof Comments**: Mark duplicates with `// Canonical: common.blood`

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

### Rule 4: Document Discoveries

When you discover a limitation or constraint:
1. Test it in isolation with a minimal example
2. Document it in this file
3. Add a comment in the affected code
4. Only then work around it

### Rule 5: Maintain Consistency

Before modifying any shared type:
1. Check `common.blood` for the canonical definition
2. Update ALL files that duplicate the type
3. Verify all files still compile
4. Document the change

---

## Known Syntax Constraints

| Constraint | Example That Fails | Workaround |
|------------|-------------------|------------|
| Cross-module types in type position | `pub field: mod::Type` | Define types inline |
| Cross-module enum variants | `mod::Enum::Variant` | Define enums inline |
| `use` after declarations | `mod foo; use foo.Bar;` | Not supported - use qualified paths |
| Some keywords as field names | `pub module: ...` | Rename: `mod_decl` |
| Limited &str methods | No `.chars()`, `.as_bytes()` | Use unsafe pointer arithmetic |

**Fixed constraints (no longer apply):**
- Nested generics like `Option<Box<Expr>>` now work (fixed in commit 40a4efe)
- Field name `end` works (was incorrectly thought to be a keyword)
- Format strings support all integer types (fixed in commit 61c8d43)

---

## Compiler Phases

Build in this order, testing each phase before moving on:

| Phase | File(s) | Lines | Status |
|-------|---------|-------|--------|
| 1 | `common.blood` | 134 | ✅ Complete |
| 2 | `lexer.blood` | 1206 | ✅ Complete |
| 3 | `ast.blood` | 1161 | ✅ Complete |
| 4 | `parser.blood` | - | ❌ Not started |
| 5 | `hir.blood` | - | ❌ Not started |
| 6 | `typeck.blood` | - | ❌ Not started |
| 7 | `mir.blood` | - | ❌ Not started |
| 8 | `codegen.blood` | - | ❌ Not started |

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
