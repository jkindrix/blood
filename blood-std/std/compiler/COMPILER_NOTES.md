# Blood Self-Hosted Compiler: Design Notes

This document explains design decisions, divergences from the Rust reference compiler, and implementation notes for the self-hosted Blood compiler.

## Overview

The Blood compiler has two implementations:
1. **Reference Implementation** (Rust): `bloodc/src/` - Uses Rust ecosystem (inkwell, ariadne)
2. **Self-Hosted Implementation** (Blood): `blood-std/std/compiler/` - Implements everything in Blood

Both compilers share the same architecture:
```
Source -> Lexer -> Parser -> AST -> HIR -> Type Check -> MIR -> Codegen -> LLVM
```

## Type Unification (`typeck/unify.blood`)

### Design Principles

1. **Explicit Pattern Matching**: Every `TypeKind` variant is explicitly handled - no catch-all `_` patterns
2. **Occurs Check**: Prevents infinite types like `T = [T]`
3. **Effect Unification**: Supports row polymorphism for effects
4. **Error Recovery**: Error types unify with anything

### Implemented Features (Aligned with Rust)

#### 1. Unit Type Equivalence
```blood
// unit keyword and () syntax are equivalent
unit == ()
```
- `Primitive(Unit)` unifies with `Tuple([])`
- Bidirectional: both syntaxes produce compatible types

#### 2. Array-to-Slice Coercion
```blood
fn process(xs: [i32]) { ... }
let arr = [1, 2, 3]  // [i32; 3]
process(arr)         // [i32; 3] coerces to [i32]
```
- Fixed-size arrays `[T; N]` coerce to slices `[T]`
- Element types must unify
- Size information is discarded during coercion

#### 3. Closure-Function Unification
```blood
fn apply(f: fn(i32) -> i32, x: i32) -> i32 { f(x) }
let closure = |x| x + 1
apply(closure, 10)  // Closure unifies with fn type
```
- Closures with compatible signatures unify with function types
- Parameter count and types must match
- Return types must unify
- Captures are ignored when unifying with function types

#### 4. Ownership Qualifier Coercion
```blood
fn consume(x: affine T) { ... }
let y: linear T = ...
consume(y)  // linear coerces to affine
```
- `linear T` coerces to `affine T` (relaxation)
- Plain types promote to ownership-qualified types
- Hierarchy: linear (strictest) -> affine -> default (least strict)

#### 5. Record Row Polymorphism
```blood
fn get_x(r: {x: i32 | R}) -> i32 { r.x }
get_x({x: 1, y: 2})     // R binds to {y: i32}
get_x({x: 1, y: 2, z: 3})  // R binds to {y: i32, z: i32}
```
- Open records `{fields | R}` match records with extra fields
- Row variable binds to the extra fields
- Closed records require exact field match
- Full algorithm:
  1. Build field name maps
  2. Unify common fields
  3. Collect extra fields
  4. Bind row variables based on openness

#### 6. Forall Alpha-Renaming
```blood
// These are equivalent:
forall<T>. T -> T
forall<U>. U -> U
```
- Forall types with same structure unify via alpha-renaming
- Fresh type variables instantiate bound parameters
- Nested foralls avoid variable capture
- Non-forall types can unify with forall by instantiation

### Implementation Notes

#### Blood vs Rust Idioms

The Blood implementation uses explicit `while` loops instead of Rust iterators:

```blood
// Blood style
let mut i: usize = 0;
while i < items.len() {
    process(&items[i]);
    i = i + 1;
};

// Rust style (not available in Blood)
// items.iter().for_each(|item| process(item));
```

This is intentional - Blood doesn't have Rust's iterator adapters.

#### Explicit Error Cases

The Blood compiler explicitly lists all non-matching type combinations:
```blood
(TypeKind::Primitive(_), TypeKind::Tuple(_)) => Err(TypeError::mismatch(...)),
(TypeKind::Primitive(_), TypeKind::Array { ... }) => Err(TypeError::mismatch(...)),
// ... hundreds of explicit cases
```

This follows the "zero shortcuts" mandate in `CLAUDE.md`:
- No silent failures with catch-all patterns
- Every type combination is explicitly considered
- Easier to audit for correctness

### Remaining Simplifications

#### Effect Type Arguments
```blood
// Current: Effect def_id match only
// Full: Should also check type arguments match
```
Line 938 in `unify.blood`:
> "For full correctness, should also check type args match but simplified for now"

## Record Representation

Records in Blood use field name strings for lookup:
```blood
struct RecordField {
    name: String,
    ty: Type,
}
```

The Rust compiler uses interned symbols for efficiency. The Blood compiler trades performance for simplicity.

## LLVM Integration

### Rust Compiler
- Uses `inkwell` crate (safe Rust wrapper around LLVM-C)
- Type-safe LLVM type construction
- Memory safety via Rust's type system

### Blood Compiler
- Direct FFI bindings via `bridge "C"`
- Manual memory management
- ~93,000 lines of LLVM type declarations

The Blood approach is more "honest" about the underlying FFI but requires careful manual management.

## Diagnostics

### Rust Compiler (~700 lines)
- Uses `ariadne` crate for pretty-printing
- Leverages Rust ecosystem

### Blood Compiler (~5,000+ lines)
- Full diagnostic system reimplemented
- Terminal, JSON, and plain text emitters
- Source snippet rendering
- Demonstrates self-hosting capability

## Performance Considerations

The Blood compiler prioritizes correctness and clarity over performance:
- O(n) field lookups in records (vs O(1) with hash maps in Rust)
- Explicit loops instead of iterator optimizations
- String-based field names vs interned symbols

These are acceptable trade-offs for a self-hosting proof-of-concept.

## Future Work

1. **Effect Type Argument Checking**: Full effect row unification with type arg matching
2. **Interned Symbols**: Replace string field names with interned symbols
3. **Iterator Support**: Add iterator abstractions to Blood stdlib
4. **Exhaustiveness Tests**: Port Rust compiler's exhaustiveness test suite

## Version History

- **2026-01-20**: Implemented 6 missing unification features to match Rust compiler
  - Unit type equivalence
  - Array-to-slice coercion
  - Closure-function unification
  - Ownership qualifier coercion
  - Full record row polymorphism
  - Forall alpha-renaming with substitution
