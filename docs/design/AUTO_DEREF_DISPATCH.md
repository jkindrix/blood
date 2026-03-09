# Design Evaluation: Auto-Deref in Method Resolution

**Status:** Decided — No auto-deref in dispatch
**Date:** 2026-03-09
**Spec ref:** DISPATCH.md §3.1–3.9, GRAMMAR.md §7.2
**Unblocks:** WORKLOAD.md Tier 5 item I, BLOCKERS.md B5

## Problem Statement

Rust automatically dereferences receivers during method resolution, walking the Deref chain (`Box<T>` → `T` → ...) until a matching method is found. Blood has multiple dispatch (methods overload on all argument types, not just receiver). Should Blood adopt auto-deref in method resolution?

## Decision

**No auto-deref in method resolution.** Method dispatch operates on declared types only. Smart pointer methods require explicit `(*wrapper).method()` or forwarding methods defined on the wrapper type.

Built-in reference stripping (`&T` → `T`) is retained as a **syntactic convenience**, not a Deref chain walk. This is hardcoded in both compilers and does not invoke `impl Deref`.

## Options Evaluated

### Option A: No auto-deref (SELECTED)

Method resolution uses the declared receiver type. `*expr` (explicit deref) is the only way to access inner type methods through wrapper types.

**Pros:**
- Spec-consistent — DISPATCH.md §3.1–3.9 is silent on auto-deref; no feature is implied
- Predictable — the type you see is the type dispatched on
- No conflict with multiple dispatch — `fn foo(x: Box<T>)` and `fn foo(x: T)` are unambiguously different overloads
- No conflict with effect overloading — `fn foo(x: &T) / pure` and `fn foo(x: &T) / {IO}` remain distinguishable
- Matches Julia precedent (multiple dispatch, no auto-deref)

**Cons:**
- More verbose for wrapper types: `(*box).method()` instead of `box.method()`
- Wrapper types must define forwarding methods manually

### Option B: Auto-deref before dispatch (Rust model)

Insert Deref coercions before dispatch sees arguments. Walk the Deref chain on the receiver until a matching method is found.

**Pros:**
- Ergonomic — `box.method()` works without explicit deref
- Familiar to Rust users

**Cons:**
- **Conflicts with multiple dispatch** — if `Box<Animal>` and `Animal` both define `speak()`, auto-deref makes it ambiguous which is called. Rust avoids this because single dispatch only considers the receiver, but Blood's multi-argument dispatch means auto-deref would need to apply to ALL arguments, compounding ambiguity.
- **Hides effect signatures** — `Box<T>::deref()` may carry `{StaleReference}` effects. Auto-deref silently injects these effects into method resolution, violating Blood's Composability pillar.
- **Unpredictable** — adding a method to an inner type can change which overload is selected on the outer type, violating Type Stability (DISPATCH.md §3.6).

### Option C: Auto-deref as dispatch fallback

Try dispatch on the declared type first. Only if no match is found, auto-deref and retry.

**Pros:**
- Preserves explicit overloads (declared type always wins)
- Falls back to convenience for wrapper types

**Cons:**
- **Still conflicts with type stability** — adding a method to a wrapper type can shadow a previously auto-deref'd inner method, changing resolution
- **Complex semantics** — "try first, then deref" creates a priority system that's harder to reason about than Rust's linear Deref chain
- **Blood-specific complexity** — multi-argument dispatch means: deref which arguments? All of them? Only the receiver? Each combination?
- No language precedent for this approach

### Option D: Multiple dispatch on wrapper types (bold)

Define methods directly on `Box<T>`, `Frozen<T>`, etc. via Blood's dispatch system. No auto-deref needed — the type system handles wrapper-specific behavior natively.

**Pros:**
- Leverages Blood's existing strength (multiple dispatch) instead of adding a new mechanism
- Wrapper types can define different behavior than inner types (intentional, not accidental)
- No implicit coercions, no hidden effects
- Fully predictable

**Cons:**
- Requires explicit forwarding methods on wrapper types (boilerplate)
- Could be mitigated by a `#[derive(Forward)]` macro or delegation syntax (future work)

## Rationale

Blood's design hierarchy: **Correctness > Safety > Predictability > Performance > Ergonomics**.

Auto-deref is an ergonomic feature. Options B and C sacrifice Predictability for Ergonomics — wrong priority order. Option A preserves Predictability. Option D goes further by embracing Blood's dispatch system but adds boilerplate.

**Selected: Option A** (with Option D as the natural evolution via future delegation syntax).

The decision aligns with:
1. **Spec silence** — DISPATCH.md does not prescribe auto-deref; adding it is a new feature, not a gap
2. **Multiple dispatch** — auto-deref fundamentally conflicts with multi-argument overload resolution
3. **Effect composability** — implicit Deref calls inject hidden effects
4. **Julia precedent** — the most successful multiple-dispatch language has no auto-deref

## Implementation Notes

### Current state (correct)
- Both compilers strip built-in references (`&T` → `T`) before method lookup — this is a syntactic convenience for the common case, not a Deref chain walk
- `impl Deref` resolves `*expr` as a value-level operation (explicit deref)
- Stdlib wrapper types (`Box<T>`, `Frozen<T>`, `String`) define their own methods

### Spec update needed
Add one sentence to DISPATCH.md §3.1:

> Method resolution operates on declared argument types. No implicit Deref chain traversal is performed during overload selection; smart pointer types must define forwarding methods or use explicit dereference (`*expr`).

### Future work (non-blocking)
- Delegation syntax (`delegate fn method to inner;`) to reduce forwarding boilerplate
- `#[derive(Forward)]` macro for common Deref-based forwarding patterns
