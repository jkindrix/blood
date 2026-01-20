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
