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

Blood syntax that blood-rust (1148f02) accepts:

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

### Rule 4: Zero Shortcuts

- Every match arm explicitly handled (no `_ =>` catch-alls)
- Every error case reported
- Every feature complete or explicitly errors with "not yet implemented"
- No silent failures, no placeholder returns

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
