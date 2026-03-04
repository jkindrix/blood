# DEF-014 Design Evaluation: Dynamic Dispatch Strategy for Blood

**Date**: 2026-03-04
**Status**: Resolved
**Verdict**: Effects complement vtables — both are needed

---

## Question

Can Blood's effects + handlers + fibers serve the use cases of `dyn Trait` (heterogeneous collections, plugin interfaces) WITHOUT Rust-style vtables?

**Answer: No.** Effects and vtables solve fundamentally different problems.

---

## What's Implemented vs Not

| Component | Status |
|-----------|--------|
| Static multiple dispatch (compile-time, all args, zero overhead) | Implemented |
| Monomorphization + specialization | Implemented |
| Type stability enforcement | Implemented |
| Effect handler dispatch (runtime, ~128ns/perform) | Implemented |
| Enum match dispatch (zero-cost with Copy optimization) | Implemented |
| `dyn Trait` vtable dispatch (DISPATCH.md §10.7-10.8) | Designed only |
| Object safety enforcement (4 rules) | Designed only |
| Fingerprint-based runtime multiple dispatch (DISPATCH.md §6.2-6.5) | Designed only |

---

## Coverage Analysis

| Use Case | Covered By | Gap? |
|----------|-----------|------|
| Callbacks | Closures + effects | No |
| Middleware/interceptors | Deep effect handlers | No |
| Type erasure (API stability) | Effects + explicit generics | Minimal |
| **Heterogeneous collections** | Enums (closed sets only) | **Yes — open sets** |
| **Plugin interfaces** | Nothing | **Yes** |

---

## Why Effects Cannot Replace Vtables

Effects and vtables operate in **different dimensions**:

- **Effects dispatch operations through the call stack.** `perform E.op(x)` walks the evidence vector to find a handler. The handler lives on the stack, not in the data. Effects are *behavioral polymorphism*.
- **Vtables dispatch methods through data pointers.** A `dyn Trait` value carries its implementation with it. The vtable lives alongside the data. Vtable dispatch is *data polymorphism*.

You cannot put an effect in a `Vec`. Effects flow through continuations and the call stack — they are not first-class data values. Heterogeneous collections require first-class data values with uniform representation.

---

## Options Evaluated

### Option A: Effect-Based Polymorphism Only
**Rejected.** Cannot store heterogeneous data in collections.

### Option B: Fingerprint-Based Runtime Multiple Dispatch (DISPATCH.md §6)
**Deferred indefinitely.** 24-bit type fingerprint → bloom filter → full type ID cache. Solves runtime multi-argument dispatch, which has no demonstrated need. Vtables are simpler and faster (~3-5 cycles vs ~5-10 cycles) for the common single-receiver case.

### Option C: `dyn Trait` Vtable Dispatch (DISPATCH.md §10.7-10.8) — Recommended
Fat pointer `{ data_ptr, vtable_ptr }`, indirect function pointer call. Already designed with Blood-specific simplifications:
- **No `drop_fn` slot** (DEF-010 resolved — Blood uses regions + finally clauses)
- Composes with effects (vtable methods declare effect rows)
- Composes with memory tiers (data pointer is tier-aware, vtable pointer is static)
- Composes with linearity ([No-Self-By-Value] prevents linear move through erasure)

### Option D: Enum Dispatch (for closed polymorphism)
**Already works.** Zero overhead with Copy optimization. Default choice when all variants are known.

---

## Recommendation: Hybrid Dispatch Hierarchy

Blood's dispatch mechanisms, in priority order:

1. **Enum + match** (closed sets) — zero-cost, exhaustive, compile-time checked
2. **Static multiple dispatch** (types known at compile time) — monomorphized, zero overhead
3. **Effect handlers** (behavioral polymorphism) — middleware, interceptors, resource management
4. **`dyn Trait` vtables** (data polymorphism) — heterogeneous collections, plugin interfaces
5. Fingerprint dispatch — **deferred indefinitely** (YAGNI)

See DISPATCH.md §10.10 for the formal specification.

---

## How Formal Proofs Inform This

- **Dispatch.v** (0 Admitted, 3 theorems): Determinism, type stability, preserves typing. Parameterized over any subtype relation satisfying reflexivity, transitivity, antisymmetry, decidability. Instantiated with `@eq ty` and `record_width_subtype`.
- **CompositionSafety.v**: Proves dispatch composes with effects, regions, linearity, memory safety. Vtable dispatch as indirect calls introduces no new stuck states.
- The proofs **neither require nor prevent** vtable dispatch — they cover static multiple dispatch resolution. Vtable dispatch is orthogonal (separate mechanism at function pointer level).
- The `record_width_subtype` instantiation demonstrates the proofs hold for structural subtyping, relevant since `dyn Trait` introduces a coercion relationship.

---

## Blood-Specific Design Advantages

1. **No `drop_fn` in vtable** — Simpler than Rust. Cleanup via tiers + finally clauses.
2. **Generation checks on data pointer only** — Vtable pointer is static (immutable compile-time data).
3. **Effect rows on trait methods** — `fn draw(&self) / {IO}` checked at trait object construction.
4. **Content-addressed vtables** — Vtable identity = hash of implementations. Enables deduplication and hot reload.

---

## Deferred Items

- Fingerprint-based runtime multiple dispatch — indefinitely
- `dyn Trait1 + Trait2` (multi-trait bounds) — post-1.0
- Trait object upcasting (`dyn SubTrait` → `dyn SuperTrait`) — post-1.0
