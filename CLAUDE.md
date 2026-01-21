# Blood Self-Hosted Compiler

## Repository Purpose

This repository contains the **self-hosted Blood compiler** written in Blood. This is Blood source code that compiles to a working compiler.

**This is NOT the Rust-based compiler.** The Rust bootstrap compiler lives at:
- Repository: `~/blood-rust` (or `github.com/jkindrix/blood-rust`)
- Commit: `1148f02` - stable, tested, Aether-verified

## Architecture

```
blood-rust (Rust)          blood (this repo)
┌─────────────────┐        ┌─────────────────────────────┐
│ bloodc          │ ──────>│ blood-std/std/compiler/*.blood │
│ blood-runtime   │compiles│                             │
│ blood-tools     │        │ Self-hosted compiler source │
└─────────────────┘        └─────────────────────────────┘
```

The Rust compiler (`blood-rust`) compiles the Blood compiler source code (this repo).

## Development Rules

### Rule 1: Compile Against blood-rust

**Every file must compile with blood-rust before committing.**

```bash
/home/jkindrix/blood-rust/target/release/blood check <file.blood>
```

If blood-rust rejects the code, the code is wrong. Do NOT modify blood-rust to accept bad syntax.

### Rule 2: Incremental Development

Write one file at a time:
1. Write the file
2. Compile with blood-rust
3. Test functionality
4. Commit
5. Move to next file

Never write thousands of lines without compiling. The previous 96k-line compiler was written without testing against blood-rust - that's why it failed.

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

**Visibility specifiers** in Blood use `crate`, `super`, `self` as modifiers, NOT path prefixes:
```blood
pub(crate) fn internal_function() { }  // visible within crate
pub(super) fn parent_visible() { }      // visible to parent module
```

**Before assuming ANY syntax, check:** `/home/jkindrix/blood/docs/spec/GRAMMAR.md`

#### Blood syntax that blood-rust (1148f02) accepts:

```blood
// Match expressions - NO semicolon after arms
match value {
    Some(x) => { do_something(x) }
    None => { default() }
}

// While loops with manual increment
let mut i: usize = 0;
while i < len {
    process(items[i]);
    i = i + 1;
}

// Array literals (not vec![])
let items: [i32] = [];
items.push(1);

// Closures with effects
let f = || / {Emit<i32>} { perform Emit.emit(42); };
```

### Rule 3a: Known Syntax Constraints

**Document all constraints discovered during development here.**

| Constraint | Example That Fails | Workaround |
|------------|-------------------|------------|
| No cross-file imports | `use std.compiler.common.Span;` | Each file must be self-contained; duplicate shared types |
| Some keywords as field names | `pub module: ...` | Rename: `mod_decl` |
| Limited &str methods | No `.chars()`, `.as_bytes()`, indexing | Use unsafe pointer arithmetic |

**Fixed constraints (no longer apply):**
- Nested generics like `Option<Box<Expr>>` now work (fixed in blood-rust commit 40a4efe)
- Field name `end` now works (was incorrectly thought to be a keyword)

**When you discover a new constraint:**
1. Test it in isolation with a minimal example
2. Document it in this table
3. Only then work around it in actual code

### Rule 4: Zero Shortcuts

- Every match arm explicitly handled (no `_ =>` catch-alls)
- Every error case reported
- Every feature complete or explicitly errors with "not yet implemented"
- No silent failures, no placeholder returns

### Rule 5: No Rushing

**Slow down. Think. Test. Document.**

When writing a new file:

1. **Understand first** - Read reference implementations, understand the problem
2. **Start minimal** - Begin with 10-20 lines that compile, not 1000 lines that don't
3. **Test each addition** - Add one struct/enum, compile, verify
4. **Document discoveries** - When something fails, understand WHY before working around it
5. **Explain your reasoning** - Provide commentary on decisions, not just code
6. **Preserve context** - Don't delete comments or structure without explicit reason

**Signs you're rushing (STOP if you notice these):**
- Writing hundreds of lines without compiling
- Repeatedly fixing compile errors without understanding root cause
- Deleting/rewriting large sections of code
- Not explaining what you're doing or why
- Skipping documentation of discovered constraints

**The goal is a working compiler, not a fast one. Quality over speed.**

## Compiler Phases

Build in this order, testing each phase before moving on:

| Phase | File(s) | Purpose |
|-------|---------|---------|
| 1 | `lexer.blood` | Tokenize source |
| 2 | `parser.blood` | Build AST |
| 3 | `ast/*.blood` | AST data structures |
| 4 | `hir/*.blood` | High-level IR |
| 5 | `typeck/*.blood` | Type checking |
| 6 | `mir/*.blood` | Mid-level IR |
| 7 | `codegen/*.blood` | LLVM codegen |

## Current Status

**Starting from scratch.** The previous 96k-line implementation was written without compiling against blood-rust and used invalid syntax throughout.

## Testing

Use blood-rust to compile and run test programs:

```bash
# Check syntax/types
/home/jkindrix/blood-rust/target/release/blood check file.blood

# Build executable
/home/jkindrix/blood-rust/target/release/blood build file.blood

# Run
/home/jkindrix/blood-rust/target/release/blood run file.blood
```

## Reference

The old (broken) compiler code can be referenced for algorithms and architecture, but do NOT copy-paste. Rewrite with correct syntax.

The Aether project (`~/blood-test/aether/`) demonstrates correct Blood syntax that blood-rust accepts.
