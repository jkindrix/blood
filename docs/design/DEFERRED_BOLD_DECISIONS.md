# Deferred Bold Decisions

**Status:** Decisions made, implementation deferred
**Date:** 2026-03-07
**Context:** Audit-derived workload design decisions (WORKLOAD.md)

These are the *best* design decisions for Blood — the ones that leverage its unique feature combination (algebraic effects, regions, linear types, multiple dispatch, content-addressing). Each was deferred to a right-sized stepping stone because of prerequisites or scope, NOT because the design is wrong. The stepping stones are intermediate milestones, not permanent compromises.

---

## 1. MEM-03: Full 7-State Escape Lattice

### The Best Decision
Blood's escape analysis should use all 7 states, especially EffectLocal and EffectCapture. No other language has escape analysis that understands algebraic effect boundaries — this is a direct consequence of Blood's unique combination of regions + effects.

### Why It Matters
The 3-state lattice conflates HeapEscape (needs Tier 1 region, ~65ns) with GlobalEscape (needs Tier 2 persistent RC, ~34ns + cycle risk). Values that could live in a region get promoted to RC. Worse: without EffectLocal/EffectCapture, the compiler can't reason about whether a region-allocated value is captured by an effect handler (needs snapshot protection) or only used synchronously (safe without snapshot).

This is Blood-specific: no other language has this problem because no other language combines regions with algebraic effects.

### Stepping Stone (Implemented)
5-state lattice: add HeapEscape to distinguish Tier 1 from Tier 2. EffectEscape sub-states deferred.

### Prerequisites for Full 7-State
1. **Region-effect suspension protocol (MEM-05)** must be complete — suspend_count tracking, PendingDeallocation state, deferred deallocation, continuation-carried region state
2. **Continuation-carried snapshots (EFF-02)** — snapshots must travel with continuations, not be local variables at the perform site
3. **Effect capture analysis** — escape analysis must understand which locals are captured by handler closures vs. which are only used before/after suspension

### Estimated Scope
~200+ lines across both compilers: new EscapeState variants, MIR escape analysis pass extension, integration with effect handler lowering.

### When to Revisit
When MEM-05 (region-effect suspension) reaches full implementation, the EffectLocal/EffectCapture distinction becomes implementable and immediately valuable.

---

## 2. SYN-06: Full Continuation-Rule Semicolon Optionality

### The Best Decision
Semicolons should be fully optional per GRAMMAR.md §5.2.1. Blood is expression-oriented — when everything is an expression, semicolons are value-discard markers, not statement terminators. The continuation rules (line continues when it ends with an operator, `|>`, `.`, etc.) are well-defined and handle Blood's pipe operator naturally.

### Why It Matters
Blood's heavy use of effect handler blocks, region blocks, match arms, and pipe chains creates deeply nested code. Required semicolons add visual noise without semantic value. The spec's continuation rules are designed to work with Blood's `|>` operator:

```blood
let result = data
    |> transform       // continues (|> is continuation token)
    |> filter           // continues
    |> collect          // implicit semicolon after this line
```

### Stepping Stone (Implemented)
Block-only optional: semicolons optional after `}`. Handles ~70% of visual noise with ~10% of parsing complexity.

### Prerequisites for Full Continuation Rules
1. **Block-only optional must be battle-tested** — confirm no edge cases with existing code
2. **Continuation token interaction with `-`** — `-` is both subtraction and negation. Rule: line ending with expression + next line starting with `-` → continuation (subtraction). Line ending with `;`/`}`/keyword + `-` → new statement (negation). Need validation with real Blood code.
3. **`(` ambiguity** — is `foo\n(bar)` a call or a new parenthesized expression? Rule: `(` on new line after expression → continuation (call). After statement → new expression. Need to verify this matches user expectations.
4. **`{` ambiguity** — is `foo\n{ ... }` a block expression or struct literal? Blood requires type name before `{` for struct literals (`Point { x: 1 }`), so bare `{` is always a block. This should be fine.

### Estimated Scope
Parser changes in both compilers: track line boundaries, implement continuation token lookahead, implicit semicolon insertion. ~150 lines per compiler + comprehensive test suite for edge cases.

### When to Revisit
After block-only optional semicolons are stable in both compilers and have been used in real Blood code for at least one development cycle. Edge cases should be catalogued from actual usage, not hypothesized.

---

## 3. DIS-16: Full Constraint Solver

### The Best Decision
Blood's dispatch should use a general constraint solver supporting the full language: TypeEq, TypeSub, EffectSub, HasField, HasMethod, Implements, conjunction, and disjunction. This enables dispatch decisions based on structural properties (does this type have a `.length` field?) and effect compatibility, not just nominal type matching.

### Why It Matters
Blood's multiple dispatch is its primary extensibility mechanism. The constraint solver is what makes dispatch composable with effects and structural typing:

- **EffectSub** enables effect-aware dispatch (pure functions preferred over effectful ones) — this is Blood's unique dispatch+effects composition
- **HasField** enables record-polymorphic dispatch (process any type with a `.name` field) — leverages Blood's anonymous records
- **HasMethod** enables structural method dispatch (accept any type with a `.serialize()` method) — enables duck-typing patterns without trait boilerplate
- **Implements** enables trait-bounded dispatch with proper constraint solving, not heuristic matching

### Stepping Stone (Implemented)
Implements + TypeEq + EffectSub. Covers ~90% of practical dispatch. EffectSub is the Blood-specific constraint that makes dispatch+effects compose.

### Prerequisites for Full Constraint Solver
1. **EffectSub must work correctly** — this requires TYP-09 (pure <: ε / row-polymorphic function types) to be implemented
2. **Anonymous record type infrastructure** — HasField requires the compiler to reason about record types structurally, which requires anonymous records to be fully wired in type inference
3. **Worklist algorithm from §9.3** — the spec defines a specific solving strategy; implementing it requires understanding the interaction between constraints and type inference
4. **Disjunction from match analysis** — requires the type checker to feed match arm type information back to the constraint solver

### Estimated Scope
~400 lines for the constraint solver core. ~200 lines for integration with dispatch resolution. ~100 lines for HasField/HasMethod analysis. Test suite: ~20 tests covering each constraint type and their interactions.

### When to Revisit
When EffectSub is working (after TYP-09), implement HasField alongside anonymous record improvements. HasMethod can follow when structural method patterns emerge in user code.

---

## 4. MEM-08 + MEM-11: Full Tier 2 API (persist + freeze + Frozen\<T\> + Cycle Collector)

### The Best Decision
Blood's tiered memory model should expose `persist()` for Tier 2 promotion and `freeze()` for deep immutability with `Frozen<T>` as a type-level guarantee. A deferred reference counting cycle collector should handle Tier 2 reference cycles.

### Why It Matters
This is Blood's answer to garbage collection: deterministic, tiered, with explicit control. No other language gives programmers a `freeze()` that provides both a type-level immutability guarantee AND enables lock-free cross-fiber sharing:

```blood
let config = freeze(Config::load())
// config: Frozen<Config> — deeply immutable, safe to share across fibers
// No synchronization needed because Frozen<T> guarantees no mutation
spawn(|| read_config(&config))  // Safe: Frozen<T> is Send
```

The cycle collector is essential for long-running programs — without it, any Tier 2 reference cycle leaks permanently. The spec's backup mark-sweep (§8.5) is designed to work with Blood's effect system by treating suspended continuation snapshot refs as roots.

### Stepping Stone (Implemented)
`persist()` with actual Tier 2 promotion. `freeze()` deferred. Cycles documented as leaking (bounded to cycle lifetime).

### Prerequisites for Full Implementation
1. **`persist()` must work end-to-end** — deep copy to Tier 2, RC initialization, proper generation counter setup
2. **Deep immutability analysis for `freeze()`** — every reachable reference from a frozen value must also be Tier 2 and immutable. Requires a type-system-level `Frozen<T>` wrapper that the borrow checker (when it exists) understands.
3. **`Frozen<T>` type constructor** — type system must track frozen-ness. `Frozen<T>` is `Send` (safe for cross-fiber). `&Frozen<T>` is freely copyable (no mutation possible).
4. **Cycle collector runtime** — backup mark-sweep over Tier 2 objects only. Must treat suspended continuation snapshot refs as GC roots (MEMORY_MODEL.md §8.5). Triggers: memory pressure, explicit `collect_cycles()`, periodic timer.
5. **Snapshot roots extraction** — cycle collector must enumerate all references held by suspended effect continuations to avoid collecting live Tier 2 objects

### Estimated Scope
- `persist()`: ~100 lines compiler (both) + ~50 lines runtime
- `freeze()` + `Frozen<T>`: ~200 lines compiler (type system + codegen) + ~30 lines runtime
- Cycle collector: ~500 lines runtime (mark-sweep, root enumeration, snapshot integration)
- Total: ~900 lines across compiler and runtime

### When to Revisit
- `freeze()`: When cross-fiber data sharing becomes a priority (fiber/concurrency work)
- Cycle collector: When long-running Blood programs exist and leak detection shows cycles as a practical problem

---

## 5. MEM-04: StaleReference as Full Algebraic Effect (IMPLEMENTED)

### The Best Decision (Implemented)
`StaleReference` is an algebraic effect with `op stale(expected_gen, actual_gen) -> never`. This IS Blood's headline feature — "handle memory errors as composable effects." The `-> never` return type bounds complexity (handler can't resume, so no state corruption risk). Default handler panics; user handlers can log, circuit-break, or shut down gracefully.

### Implementation
- **Runtime**: Default `blood_default_stale_handler` registered at `blood_runtime_init()` with effect ID `0x1004`
- **Bootstrap codegen**: All 4 stale reference sites (terminator.rs, place.rs, memory.rs) now call `blood_perform(0x1004, 0, [expected, actual], 2)` through the evidence vector instead of `blood_stale_reference_panic` directly
- **Selfhost codegen**: `codegen_expr.blood:emit_generation_check` emits `blood_perform` call on stale path
- **Effect dispatch**: Uses existing evidence vector infrastructure — hot path (generation check) unchanged, only failure path goes through effect system
- **Default behavior**: Identical to previous — the default handler calls `blood_stale_reference_panic` which aborts with diagnostic message
- **User override**: Users can install custom handlers using standard `with handler handle { ... }` syntax; handler must diverge (-> never)

---

## Cross-Cutting Theme

Every deferred bold decision shares a pattern: **Blood's unique feature interactions create design spaces no other language has explored.** The stepping stones are sound (conservative-safe, spec-compatible), but the bold versions are where Blood's value proposition lives:

| Feature Interaction | Bold Decision | Value |
|---|---|---|
| Regions + Effects | 7-state escape (MEM-03) | Allocation precision at effect boundaries |
| Effects + Memory Safety | StaleReference effect (MEM-04) | Composable memory error handling |
| Dispatch + Effects | Full constraint solver (DIS-16) | Effect-aware method selection |
| Tiered Memory + Fibers | freeze() + Frozen\<T\> (MEM-08) | Type-safe cross-fiber sharing |
| Expression-Oriented + Pipe | Full semicolon optionality (SYN-06) | Clean syntax for effect-heavy code |

These aren't independent — they're facets of the same design philosophy: **safety and composability emerge from the interaction of Blood's pillars, not from any single feature.**
