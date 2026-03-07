# Design Evaluation: Numeric Subtyping in Blood

**Status:** Resolved — no numeric subtyping
**Date:** 2026-03-07
**Linked:** DIS-08, DISPATCH.md §C.4, WORKLOAD.md Tier 2 #6 / Tier 5 E

---

## Question

Should Blood's type system include integer promotion subtyping (`i32 <: i64`, `u8 <: u32`, etc.)?

## Decision

**No.** Blood's type system has no numeric subtyping. All integer types are distinct, unrelated types. Conversions require explicit casts or (in the future) a `Widen` trait.

## Rationale

### 1. Multiple Dispatch Ambiguity

Blood uses multiple dispatch (DISPATCH.md). Numeric subtyping creates dispatch ambiguity that is difficult to resolve predictably:

```blood
fn process(x: i32, y: i32) -> i32 { ... }    // M1
fn process<T>(x: T, y: T) -> T { ... }        // M2

// With numeric subtyping: process(1u8, 2u8) — is u8 <: i32?
// M1 becomes applicable via promotion. Both M1 and M2 match.
// Specificity ordering becomes complex and surprising.
```

**Julia** — the most mature multiple-dispatch language — explicitly chose `Int32 <: Int64 == false`. Their experience shows numeric subtyping creates dispatch ambiguity that violates user expectations.

### 2. Design Hierarchy: Correctness > Safety > Predictability

Blood's design hierarchy places **Predictability** above **Performance** and **Ergonomics**. Implicit numeric conversions trade predictability for ergonomics:

- Silent precision loss (`i64` → `i32` via contravariance in function args)
- Surprising dispatch selection when methods differ by numeric width
- Type inference ambiguity when literals could promote to multiple targets

### 3. Cross-Language Evidence

| Language | Numeric subtyping? | Rationale |
|----------|-------------------|-----------|
| **Julia** | No | Dispatch ambiguity |
| **Scala 3** | Removing implicit widening | Source of bugs, inference complexity |
| **Swift** | No | Safety, predictability |
| **Rust** | No | Explicitness |
| **OCaml** | No | Type safety |
| **Zig** | No | Explicitness |
| **Haskell** | No (typeclasses) | Num typeclass instead |
| **C/C++/Java** | Yes | Legacy; widely recognized as error-prone |

The trend across modern languages is toward explicit conversions. Languages with subtyping (C, Java) are the ones whose implicit promotion rules are most frequently cited as sources of bugs.

### 4. Spec Consistency

The formal subtyping rules in SPECIFICATION.md §3.7 define no numeric promotion. The only mentions of `i32 <: i64` were in informal DISPATCH.md examples (test vector C.4), which have been corrected.

---

## Bold Alternative: `Widen` Trait (Deferred)

While Blood correctly rejects implicit numeric subtyping, the ergonomic cost of explicit casts everywhere is real. A future alternative leverages Blood's trait and dispatch system:

### Widen Trait Design

```blood
trait Widen<Target> {
    fn widen(self) -> Target;
}

// Compiler-provided impls for provably lossless conversions
impl Widen<i16> for i8 { fn widen(self) -> i16 { /* intrinsic */ } }
impl Widen<i32> for i8 { fn widen(self) -> i32 { /* intrinsic */ } }
impl Widen<i32> for i16 { fn widen(self) -> i32 { /* intrinsic */ } }
impl Widen<i64> for i32 { fn widen(self) -> i64 { /* intrinsic */ } }
// ... etc. Only lossless conversions get Widen impls.
```

### Usage

```blood
fn sum_large(a: i64, b: i64) -> i64 { a + b }

let x: i32 = 42;
let y: i32 = 17;
sum_large(x.widen(), y.widen());  // Explicit, discoverable, type-safe
```

### Why This Fits Blood

- **Composable with dispatch:** `Widen` is a normal trait, no special subtyping rules needed
- **Composable with effects:** Widening is pure, no effect interaction
- **Composable with linear types:** `Widen` consumes `self` (move), works with linear values
- **Content-addressing friendly:** No implicit coercions means deterministic type signatures for content hashing
- **Explicit but ergonomic:** `.widen()` is short, communicates intent, IDE-discoverable

### Polymorphic Numeric Literals (Further Future)

```blood
// Literal `42` has type `forall T: Numeric. T` until constrained
let x: i64 = 42;  // 42 instantiated as i64
let y: i32 = 42;  // 42 instantiated as i32
```

This follows Haskell's approach. It interacts well with dispatch (the literal's type is determined at the call site, not via promotion) and avoids the ambiguity problems of subtyping.

### Infrastructure Gaps

Before implementing the `Widen` trait approach:
1. **Trait method dispatch must be complete** — currently partial in selfhost (DIS-01/02/07)
2. **Compiler-provided trait impls** — mechanism for the compiler to inject `Widen` impls for builtin types
3. **Blanket impls / coherence** — needed to ensure `Widen` impls don't conflict with user-defined conversions

---

## Spec Changes

- **DISPATCH.md** test vector C.4: Corrected to show `is_subtype(u8, i32) = false`
- **SPECIFICATION.md §3.7**: No change needed (formal rules already correct)
- **No new subtyping rules added**
