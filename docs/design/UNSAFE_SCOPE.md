# Design Evaluation: Unsafe and Bridge Scope

**Status:** Decided — `@unsafe` is the safety gate; `bridge` is for FFI only
**Date:** 2026-03-09
**Spec ref:** GRAMMAR.md §5.4, §3.8, §9.5; FORMAL_SEMANTICS.md §5.10.1; FFI.md §1.4
**Unblocks:** WORKLOAD.md TYP-12 (Ptr↔usize restriction), BLOCKERS.md B6

## Problem Statement

The spec requires `@unsafe` for pointer-integer casts (FORMAL_SEMANTICS.md §5.10.1), raw pointer dereference, and type punning. Neither compiler enforces this — all casts are accepted unconditionally. The selfhost uses ~237 Ptr↔usize instances across ~30 files, most already inside `@unsafe` blocks by convention. Blood also has `bridge "C"` blocks for FFI. The question: what is the relationship between `@unsafe` and `bridge`, and how should enforcement work?

## Decision

1. **`@unsafe` and `bridge` are distinct scopes with distinct purposes.**
2. **`bridge` blocks grant implicit `@unsafe` context** (FFI inherently requires unsafe operations).
3. **Enforcement via `in_unsafe_context` flag on TypeChecker**, checked at cast validation time.
4. **Enforcement is additive** — add the check, fix the ~dozen selfhost instances that lack `@unsafe` wrapping.

## Options Evaluated

### Option A: Enforce `@unsafe` strictly, `bridge` grants implicit unsafe (SELECTED)

Add `in_unsafe_context: bool` to TypeChecker. Set it when entering `@unsafe {}` or `bridge "C" {}` blocks. Check it in `is_valid_cast()` for Ptr↔usize, Ptr→Ref, `*const T → *mut T`, and fn→integer casts. Reject with a diagnostic suggesting `@unsafe {}` wrapping.

**Pros:**
- Spec-compliant — implements FORMAL_SEMANTICS.md §5.10.1 as written
- Most selfhost code already uses `@unsafe` — minimal churn
- Auditable — `grep @unsafe` finds all safety-escape points (GRAMMAR.md §9.5 design intent)
- `bridge` implicitly granting `@unsafe` is intuitive — FFI is inherently unsafe

**Cons:**
- ~10-15 selfhost instances may need `@unsafe` wrapping (minor)
- Adds a field to TypeChecker and a check to cast validation (minimal complexity)

### Option B: Keep casts unrestricted, remove spec restriction

Allow Ptr↔usize anywhere. Update FORMAL_SEMANTICS.md to remove the context restriction.

**Pros:**
- Zero implementation work
- Pragmatic — the selfhost is a real program that uses these pervasively

**Cons:**
- **Violates design hierarchy** — Safety is #2 after Correctness. Weakening safety for convenience contradicts Blood's goals.
- **Undermines `@unsafe` purpose** — if the most dangerous casts don't require it, what does?
- **Loses auditability** — can't `grep @unsafe` to find all safety escapes
- **`close-spec-to-impl` without justification** — neither spec omission, Rust-ism correction, nor non-normative clarification

### Option C: `unsafe mod` for whole-module opt-out (bold)

Mark entire modules as `unsafe mod` to skip all `@unsafe` requirements within. The selfhost's low-level modules (interner, codegen_ctx, mir_lower) would be marked `unsafe mod`.

**Pros:**
- Coarser — less syntactic noise in low-level modules
- Acknowledges that some modules are inherently unsafe throughout

**Cons:**
- **Loses granularity** — `@unsafe` blocks mark exactly which expressions are unsafe. `unsafe mod` makes the whole module a blind spot.
- **Not in the spec** — would require GRAMMAR.md changes
- **Violates auditability** — can't distinguish safe from unsafe code within the module
- **Not needed** — the selfhost already wraps most casts in `@unsafe`; the remaining ~10-15 are small fixes

## Scope Relationships

```
bridge "C" { ... }     — FFI declarations. Grants implicit @unsafe context.
@unsafe { ... }         — Safety escape hatch. Required for:
                          - Ptr ↔ usize/isize casts
                          - Ptr → Ref conversion
                          - *const T → *mut T cast
                          - fn → usize/u64/i64 cast
                          - Raw pointer dereference
                          - Type punning (future)
                          - Inline assembly (future)
unchecked(checks) { }   — Elides runtime checks (bounds, overflow, generation,
                          null, alignment). NOT a safety escape — operations
                          remain type-safe, only validation is skipped.
```

`bridge` ⊃ `@unsafe` (bridge grants unsafe context). `unchecked` is orthogonal to both — it controls runtime checks, not type-system safety.

## Implementation Plan

### Phase 1: TypeChecker context tracking (both compilers)

**Selfhost (`typeck.blood`, `typeck_expr.blood`):**
1. Add `in_unsafe_context: bool` field to `TypeChecker` struct (default `false`)
2. In `infer_expr` for `ExprKind.Unsafe`: set `in_unsafe_context = true`, infer body, restore
3. In `infer_expr` for bridge blocks (if they reach typeck): set `in_unsafe_context = true`
4. In `is_valid_cast()`: check `checker.in_unsafe_context` for the four restricted cast categories. Emit new diagnostic (e.g., E0950 "cast requires @unsafe context") when rejected.

**Bootstrap (`typeck/context/expr.rs`):**
1. Add `in_unsafe_context: bool` to type checker context
2. Same pattern: set on entering `@unsafe`/`bridge`, check on cast validation

### Phase 2: Fix selfhost instances

Audit the ~237 Ptr↔usize instances. Most already have `@unsafe`. For the ~10-15 that don't, wrap in `@unsafe { ... }`. This is a mechanical fix.

### Phase 3: Golden tests

1. **Compile-fail test:** Ptr↔usize cast outside `@unsafe` → error
2. **Run-pass test:** Same cast inside `@unsafe` → accepted
3. **Run-pass test:** Cast inside `bridge "C"` block → accepted (implicit unsafe)

## Interaction with `unchecked`

`unchecked` does NOT grant `@unsafe` context. They are orthogonal:

```blood
unchecked(bounds) {
    let x = ptr as usize;  // ERROR: requires @unsafe
}

@unsafe {
    unchecked(bounds) {
        let x = ptr as usize;  // OK: in @unsafe context
        arr[idx]               // OK: bounds check elided
    }
}
```

This is correct: `unchecked` controls runtime check insertion (a codegen concern), while `@unsafe` gates type-system safety escapes (a typeck concern).

## Rationale

Blood's design hierarchy: **Correctness > Safety > Predictability > Performance > Ergonomics**.

Option A implements the spec as written, preserves Safety (#2), and maintains the `@unsafe` auditability contract. The implementation cost is low (one bool field, one check in cast validation, ~15 mechanical fixes). Option B sacrifices Safety for zero effort — wrong tradeoff. Option C adds unnecessary grammar complexity when the simple approach suffices.

The `bridge` → implicit `@unsafe` relationship is intuitive and reduces boilerplate in FFI code, which is inherently unsafe. This doesn't weaken the safety model — it acknowledges the reality that FFI operations can't be type-checked.
