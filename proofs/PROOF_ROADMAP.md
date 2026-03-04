# Blood Formal Verification Roadmap

**Version:** 1.0
**Created:** 2026-03-04
**Status:** Authoritative — this is the single source of truth for Blood's formal verification plan

---

## Purpose

This document defines every formal proof needed to demonstrate that Blood's feature
combination is internally consistent, safe, and compositionally sound. It is organized
into three tiers reflecting increasing specificity to Blood:

- **Tier 1 — Core Calculus Soundness:** Standard PL metatheory (progress, preservation,
  effect safety, etc.). Necessary but not differentiating — any well-designed language
  should have these.

- **Tier 2 — Feature Interaction Theorems:** Proofs that require *multiple Blood features
  to even state*. These are what make Blood Blood, not "any effects+linearity language."

- **Tier 3 — Composition Theorems:** Proofs that Blood's features compose simultaneously
  to produce emergent safety guarantees no individual feature provides alone. These are
  the crown jewels that validate Blood's design thesis.

### Why Three Tiers?

No single Blood feature is unprecedented — Koka has effects, Rust has linear types, Julia
has multiple dispatch, Cyclone had regions. Blood's thesis is that the *composition* of
these features produces emergent safety guarantees that no individual feature provides
alone. Tier 1 proves each feature works. Tier 2 proves features interact safely in pairs.
Tier 3 proves the whole is greater than the sum of its parts.

### Guiding Principles

1. **Complete before extending.** Admitted theorems weaken safety claims — building new
   features on top of unproven foundations creates false confidence.
2. **Interaction theorems over individual features.** A proof about effects alone is less
   valuable than a proof about effects+linearity together.
3. **Each phase must compile independently.** `make -f Makefile.coq` must pass after each
   phase with no new warnings.
4. **The formalization should be specifically about Blood.** By Tier 3, the proofs should
   not be describable as "a generic effects calculus."

---

## Current State (2026-03-04)

18 files, 8,229 lines, **0 Admitted**, 168 Qed, 1 Axiom, 1 Parameter.
All 18 files fully proved (0 Admitted).

### Permanent Modeling Assumptions

These are deliberate abstractions, not proof gaps:

| Item | File | Kind | Rationale |
|------|------|------|-----------|
| `continuation_expr_is_value` | Inversion.v | Axiom | Continuations abstract over expression structure |
| `extract_gen_refs` | Semantics.v | Parameter | Snapshot extraction abstracted at interface level |

---

## Tier 1: Core Calculus Soundness

Standard PL metatheory. Proves the core calculus is well-behaved.

### Phase 1: Core Safety Foundation — COMPLETE

**Goal:** Fully mechanized Wright-Felleisen type soundness.

**Status:** All sub-tasks completed. 0 Admitted.

| Theorem | File | Status |
|---------|------|--------|
| Progress (all 11 cases) | Progress.v | PROVED |
| Preservation (all 11 cases) | Preservation.v | PROVED |
| `type_soundness_full` | Soundness.v | PROVED |
| `full_composition_safety` | Soundness.v | PROVED |
| Typing inversion (21 lemmas) | Inversion.v | PROVED |
| Substitution preservation (21 lemmas) | Substitution.v | PROVED |
| Shift-substitution commutation (4 lemmas) | ShiftSubst.v | PROVED |
| Effect row algebra (7 lemmas) | EffectAlgebra.v | PROVED |
| Context typing (7 lemmas) | ContextTyping.v | PROVED |

Key techniques for the hardest cases:
- HandleOpDeep/HandleOpShallow: continuation typing via `weakening_cons` +
  `delimited_context_typing_gen` + double substitution (kont at index 1, v at index 0)
- T_Record: mutual induction scheme with record field progress property
- T_Handle perform: `String.string_dec` for decidable effect name comparison,
  `DC_HandleOther` + `dc_no_match` for unhandled operations escaping through handler

### Phase 3: Effect Safety — COMPLETE

**Goal:** Prove effects are tracked, contained, and handled.

**Status:** All 9 theorems proved. 0 Admitted.

| Theorem | File | Status |
|---------|------|--------|
| `static_effect_containment` | EffectSafety.v | PROVED |
| `dynamic_effect_containment` | EffectSafety.v | PROVED |
| `deep_handler_reinstallation` | EffectSafety.v | PROVED |
| `effect_handling_completeness` | EffectSafety.v | PROVED |
| `pure_subset_all` | EffectSafety.v | PROVED |
| `effect_entries_subset_union_compat` | EffectSafety.v | PROVED |
| `effect_union_monotone_left` | EffectSafety.v | PROVED |
| `effect_union_comm` | EffectSafety.v | PROVED |
| `effect_discipline` | EffectSafety.v | PROVED |

### Phase 4: Generation Snapshot Safety — COMPLETE

**Goal:** Prove generational references prevent use-after-free across effect continuations.

**Status:** All 14 theorems proved. 0 Admitted. Blood's most novel formal contribution —
no published precedent exists for generation snapshot safety with algebraic effects.

| Theorem | File | Status |
|---------|------|--------|
| `no_use_after_free` | GenerationSnapshots.v | PROVED |
| `gen_snapshot_valid` | GenerationSnapshots.v | PROVED |
| `effects_gen_composition_safety` | Soundness.v | PROVED |
| + 11 supporting lemmas | GenerationSnapshots.v | PROVED |

---

## Tier 2: Feature Interaction Theorems

Proofs that require multiple Blood features to state. Each theorem captures a property
that emerges from the interaction of two or more features.

### Phase 2: Effects x Linearity — COMPLETE

**Goal:** Prove that algebraic effects and linear/affine types compose safely. Multi-shot
handlers cannot capture linear values; single-shot handlers can. At `perform`, linear
values are transferred (not duplicated) via context splitting.

**Why this is a Tier 2 theorem, not Tier 1:** Koka has effects but no linearity. Rust has
linearity but no effects. When you combine them, the question is: "What happens when a
handler resumes a continuation twice, and that continuation holds a linear resource?"
Blood's answer — enforced by the type system — is: you can't. This is a property that
*neither system alone needs to state*.

**Depends on:** Phase 1 (SATISFIED)

**Files:** LinearTyping.v (474 lines, 2 Qed), LinearSafety.v (833 lines, 19 Qed)

**Status:** All 4 previously-admitted theorems proved. 0 Admitted.

| Theorem | File | Status |
|---------|------|--------|
| `linear_safety_static` | LinearSafety.v | PROVED |
| `affine_safety_static` | LinearSafety.v | PROVED |
| `multishot_no_linear_capture` | LinearSafety.v | PROVED |
| `effect_suspension_linear_safety` | LinearSafety.v | PROVED |

**Architecture:** Two-judgment design. Rather than modifying existing `has_type` rules
(which would cascade through every proof file), a new `has_type_lin` judgment in
LinearTyping.v adds linearity enforcement at leaf rules:
- `TL_Var`: checks `Delta(x)` available and all other linears consumed
- `TL_Const`: checks `all_linear_consumed Delta`
- `TL_Lam`/`TL_Let`: introduce `lin_of_type A` (not always Unrestricted)
- `RFT_Cons_Lin`: splits Delta across record fields via `lin_split`
- `HWF_Lin`: requires `multishot_handler_safe_lin` and `lin_split`

Bridge lemma `has_type_lin_to_has_type` erases linearity enforcement, proving every
linearity-checked derivation is also a standard typing derivation. This avoids modifying
existing `has_type` rules and prevents cascading breakage in Progress, Preservation,
and Soundness.

### Phase 5: Regions x Generations — COMPLETE

**Goal:** Prove region deallocation is safe via generation bumps.

**Status:** All 3 main theorems + 1 nested safety corollary proved. 0 Admitted.

**Depends on:** Phase 4 (SATISFIED)

**New file:** Regions.v (316 lines, 10 Qed)

Self-contained — builds on GenerationSnapshots.v infrastructure without modifying
existing files. Region destruction is modeled as bulk generation-bump, which is the
key insight from FORMAL_SEMANTICS.md §5.8: "Region safety is NOT a typing property —
it is a runtime property guaranteed by the generation system."

| Theorem | File | Status |
|---------|------|--------|
| `region_safety` | Regions.v | PROVED |
| `region_effect_safety` | Regions.v | PROVED |
| `escape_analysis_sound` | Regions.v | PROVED |
| `region_nested_safety` | Regions.v | PROVED (bonus) |

### Phase 6: Dispatch x Type Stability — COMPLETE

**Goal:** Formalize dispatch resolution and prove type stability.

**Status:** All 3 main theorems + 1 corollary proved. 0 Admitted.

**Depends on:** Phase 1 (SATISFIED)

**New file:** Dispatch.v (289 lines, 11 Qed)

Self-contained — parameterized over a subtype relation via Section variables.
When Blood's concrete subtype relation is defined, instantiate by closing the
Section. Section hypotheses: subtype relation (5 properties), method_eq_dec (1).

| Theorem | File | Status |
|---------|------|--------|
| `dispatch_determinism` | Dispatch.v | PROVED |
| `type_stability_soundness` | Dispatch.v | PROVED |
| `dispatch_preserves_typing` | Dispatch.v | PROVED |
| `dispatch_return_type_determined` | Dispatch.v | PROVED (bonus) |

### Phase 7: MVS x Linearity — NOT STARTED

**Goal:** Formalize copy-by-default (mutable value semantics) and explicit borrowing.
Prove value types never alias.

**Why this is Tier 2:** In Rust, linearity means "move" — the original binding is
consumed. In Blood, linearity means "use exactly once" but the value was *copied in*, so
the original is independent. This is a fundamentally different resource model. The proof
shows MVS + linearity = no-aliasing guarantee without Rust's ownership complexity.

**Depends on:** Phase 2 (linear safety for borrow tracking)

**New file:** ValueSemantics.v

| Theorem | File | Proof Strategy |
|---------|------|----------------|
| `value_copy_independence` | ValueSemantics.v | Copying a value type creates an independent value |
| `borrow_linearity` | ValueSemantics.v | Mutable borrows are linear, immutable borrows unrestricted |
| `mvs_no_aliasing` | ValueSemantics.v | Value-typed bindings never alias |

Note: The core insight (values are copied by substitution) is already implicit in the
de Bruijn formalization. This phase makes it explicit.

**Estimated:** ~200-300 new lines

---

## Tier 3: Composition Theorems

Proofs that Blood's features compose *simultaneously* to produce emergent safety
guarantees. These are the crown jewels — they demonstrate that the whole is greater
than the sum of its parts.

### Phase 8: Effects Subsume Control Flow Patterns — NOT STARTED

**Goal:** Prove that Blood's algebraic effects + handlers can express exceptions,
async/await, and generators as special cases, with all safety guarantees (effect
containment, linear safety, generation safety) applying automatically.

**Why this matters:** This shows effects aren't just another feature — they're a unifying
framework. Instead of having separate mechanisms for exceptions, async, and generators
(each needing its own safety proof), Blood has one mechanism with one set of proofs
covering all patterns.

**Depends on:** Phase 2 (linear safety), Phase 3 (effect safety)

**New file:** EffectSubsumption.v

| Theorem | Proof Strategy |
|---------|----------------|
| `effects_subsume_exceptions` | Exception handling is a specialization of effect handling with a single `raise` operation and no resume |
| `effects_subsume_generators` | Generators are effect handlers that yield values and resume with unit; prove bisimulation with iterator protocol |
| `effects_subsume_async` | Async/await is a specialization of shallow effect handling with a suspend/resume protocol |
| `subsumption_safety_transfer` | Safety theorems (containment, linearity, generation) apply to all subsumed patterns without additional proof |

**Estimated:** ~200-300 new lines

### Phase 9: Memory Safety Without Garbage Collection — NOT STARTED

**Goal:** Prove that Regions + Generations + Linearity + MVS together guarantee memory
safety without garbage collection.

**Why this matters:** This is Blood's headline claim against GC-based languages. The proof
shows that every allocation is either: (a) stack-allocated and scoped, (b) region-
allocated and invalidated on region destroy, or (c) persistent and reference-counted —
and that generations + linearity prevent use-after-free in all three tiers.

**Depends on:** Phase 2 (linearity), Phase 4 (SATISFIED), Phase 5 (SATISFIED), Phase 7 (MVS)

**New file:** MemorySafety.v

| Theorem | Proof Strategy |
|---------|----------------|
| `tier_coverage` | Every allocation belongs to exactly one tier (Stack, Region, Persistent) |
| `stack_safety` | Stack-tier values are scoped; no dangling references after scope exit |
| `region_safety_composition` | Region-tier values detected stale via generation bump (combines Phase 5 `region_safety` with Phase 4 `no_use_after_free`) |
| `persistent_safety` | Persistent-tier values are reference-counted; generation checked on access |
| `memory_safety_no_gc` | **Composition theorem:** Union of tier guarantees covers all memory, no GC required |

**Estimated:** ~150-250 new lines (heavy lifting done in Phases 2, 4, 5, 7; this stitches them together)

### Phase 10: Tier-Based Concurrency Safety — COMPLETE

**Goal:** Prove that Blood's tier-based crossing rules guarantee safe concurrency without
Rust-style Send/Sync traits.

**Status:** All 5 main theorems + 3 corollaries proved. 0 Admitted.

**Depends on:** Phase 5 (SATISFIED)

**New file:** FiberSafety.v (412 lines, 13 Qed)

Self-contained — defines memory tiers (Stack, Region, Persistent), mutability (Mutable,
Frozen), typed references, and fiber crossing predicates. Builds on Regions.v for the
region-checked crossing theorem. Ownership model parameterized via Section variable
(addr_owner), same pattern as Dispatch.v.

Key insight (CONCURRENCY.md §9.2): Data race freedom follows by construction from the
tier crossing rules. Mutable references require address ownership (unique per fiber),
so two different fibers cannot both hold writable references to the same address.

| Theorem | File | Status |
|---------|------|--------|
| `stack_no_cross` | FiberSafety.v | PROVED |
| `region_checked_cross` | FiberSafety.v | PROVED |
| `persistent_free_cross` | FiberSafety.v | PROVED |
| `tier_crossing_safety` | FiberSafety.v | PROVED |
| `region_isolation` | FiberSafety.v | PROVED |
| `region_isolation_no_write` | FiberSafety.v | PROVED (bonus) |
| `region_crossing_detected` | FiberSafety.v | PROVED (bonus) |
| `crossing_region_is_frozen` | FiberSafety.v | PROVED (bonus) |

### Phase 11: Full Composition Safety — NOT STARTED

**Goal:** Prove that ALL of Blood's safety properties hold simultaneously under arbitrary
composition of features.

**Why this matters:** This is the crown jewel of the entire verification effort. Individual
proofs show each property holds in isolation. Pairwise proofs show features interact
safely. This proof shows they don't interfere with each other when all present
simultaneously — adding regions doesn't break effect safety, adding dispatch doesn't
break linear safety, etc.

**Depends on:** All previous phases (2, 5, 6, 7, 8, 9, 10)

**New file:** CompositionSafety.v

| Theorem | Proof Strategy |
|---------|----------------|
| `type_soundness_extended` | Progress + Preservation hold for the extended calculus (regions, dispatch, MVS, tiers) |
| `effect_safety_preserved` | Effect containment and handling still hold with all extensions |
| `linear_safety_preserved` | Linear/affine guarantees hold with regions, dispatch, and MVS |
| `generation_safety_preserved` | Generation snapshot validity holds with regions and tiers |
| `full_blood_safety` | **Master theorem:** conjunction of type soundness + effect safety + linear safety + generation safety + region safety + dispatch determinism + MVS no-aliasing + tier crossing safety |

**Estimated:** ~100-200 new lines (this is mostly combining existing results)

---

## What NOT to Formalize

| Feature | Reason |
|---------|--------|
| Content-addressing (BLAKE3) | Implementation property about hash determinism, not type safety |
| Full fiber scheduler (M:N) | Phase 10 covers the crossing rules; the scheduler is runtime implementation detail |
| Structured concurrency | Requires process calculus or interaction trees; massive standalone effort |
| Macro expansion | Pre-type-checking source transformation; orthogonal to safety |
| Bridge FFI | Operates at ABI boundary, outside type-safe core |
| Escape analysis algorithm | Compiler implementation detail; Phase 5 formalizes the soundness guarantee, not the algorithm |
| Concrete `extract_gen_refs` | Abstract interface sufficient for all safety proofs; concretization is optional |

---

## Dependency Graph

```
Tier 1 (Foundation)
  Phase 1 (Core Foundation)       — COMPLETE
  Phase 3 (Effect Safety)         — COMPLETE
  Phase 4 (Gen Snapshots)         — COMPLETE

Tier 2 (Interactions)
  Phase 2 (Effects x Linearity)   — COMPLETE
    depends on: Phase 1
  Phase 5 (Regions x Generations) — COMPLETE
    depends on: Phase 4
  Phase 6 (Dispatch x Stability)  — COMPLETE
    depends on: Phase 1
  Phase 7 (MVS x Linearity)       — not started (UNBLOCKED)
    depends on: Phase 2 (SATISFIED)

Tier 3 (Compositions)
  Phase 8 (Effects Subsume Patterns) — not started (UNBLOCKED)
    depends on: Phase 2 (SATISFIED), Phase 3 (SATISFIED)
  Phase 9 (Memory Safety, No GC)    — not started
    depends on: Phase 2 (SATISFIED), Phase 4 (SATISFIED), Phase 5 (SATISFIED), Phase 7
  Phase 10 (Tier Concurrency Safety) — COMPLETE
    depends on: Phase 5
  Phase 11 (Full Composition Safety) — not started
    depends on: ALL previous phases
```

Visual:

```
Phase 1 ──DONE──┬──► Phase 2 ──DONE──┬──► Phase 7 ──────────┐
                │                    │                       │
                │                    ├──► Phase 8 ───────────┤
                │                    │                       │
Phase 3 ──DONE──┤                    └──► Phase 9 ◄── P5 ◄──┤
                │                            ▲               │
Phase 4 ──DONE──┼──► Phase 5 ──DONE────────┼──► P10 ──DONE─┤
                │                            │               │
                └──► Phase 6 ──DONE─────────┼───────────────┤
                                             │               │
                                             └──► Phase 11 ◄─┘
```

### Parallelism Opportunities

These phases can proceed in parallel (all dependencies satisfied):
- Phase 7 + Phase 8 (both depend only on Phase 2, which is COMPLETE)
- Phase 10 is already COMPLETE

After Phase 7 completes, Phase 9 is fully unblocked.

### Critical Path

The longest remaining dependency chain is:

```
Phase 7 → Phase 9 → Phase 11
```

Phase 7 (MVS x Linearity) is the new bottleneck — it blocks Phase 9 and (transitively) 11.
Phase 8 can proceed independently in parallel with Phase 7.

---

## Priority Summary

| Priority | Phase | Tier | Why |
|----------|-------|------|-----|
| **Highest** | Phase 7 | T2 | Critical path bottleneck; enables Phase 9 |
| **High** | Phase 8 | T3 | Unblocked; validates effects as unifying framework |
| **Medium** | Phase 9 | T3 | Blood's headline claim (no GC); blocked on P7 only |
| **Lower** | Phase 11 | T3 | Crown jewel; depends on everything |
| DONE | Phase 1 | T1 | Core foundation — FULLY PROVED |
| DONE | Phase 2 | T2 | Effects x Linearity — FULLY PROVED (two-judgment design) |
| DONE | Phase 3 | T1 | Effect safety — FULLY PROVED |
| DONE | Phase 4 | T1 | Generation snapshots — FULLY PROVED |
| DONE | Phase 5 | T2 | Region safety via generations — FULLY PROVED |
| DONE | Phase 6 | T2 | Dispatch determinism + type stability — FULLY PROVED |
| DONE | Phase 10 | T3 | Tier crossing safety — FULLY PROVED |

---

## Effort Estimates

| Phase | Tier | New Lines | Complexity | New Files | Status |
|-------|------|-----------|------------|-----------|--------|
| Phase 1 | T1 | — | — | — | COMPLETE |
| Phase 2 | T2 | 1,307 (new) | — | LinearTyping.v, LinearSafety.v (rewritten) | COMPLETE |
| Phase 3 | T1 | — | — | — | COMPLETE |
| Phase 4 | T1 | — | — | — | COMPLETE |
| Phase 5 | T2 | 316 | — | Regions.v | COMPLETE |
| Phase 6 | T2 | 289 | — | Dispatch.v | COMPLETE |
| Phase 7 | T2 | 200-300 | Low-Medium | ValueSemantics.v | Not started |
| Phase 8 | T3 | 200-300 | Medium | EffectSubsumption.v | Not started |
| Phase 9 | T3 | 150-250 | Medium | MemorySafety.v | Not started |
| Phase 10 | T3 | 412 | — | FiberSafety.v | COMPLETE |
| Phase 11 | T3 | 100-200 | Medium | CompositionSafety.v | Not started |
| **Total remaining** | | **~1,150-2,050** | | **4 new files** | |

Current suite: 18 files, 8,229 lines.
Projected final: ~8,880-9,530 lines, 22 files, zero Admitted.

---

## Classification of Non-Qed Items

### Modeling Axioms (permanent, by design)

| Item | File | Rationale |
|------|------|-----------|
| `continuation_expr_is_value` | Inversion.v | Deliberate abstraction over continuation structure |
| `extract_gen_refs` | Semantics.v | Abstract snapshot extraction interface |

### Genuine Proof Obligations — ALL RESOLVED

All 4 previously-admitted theorems are now fully proved via the two-judgment
design in LinearTyping.v + LinearSafety.v:

| Item | File | Status |
|------|------|--------|
| `linear_safety_static` | LinearSafety.v | PROVED (mutual induction on `has_type_lin`) |
| `affine_safety_static` | LinearSafety.v | PROVED (mutual induction on `has_type_lin`) |
| `multishot_no_linear_capture` | LinearSafety.v | PROVED (inversion on `HWF_Lin`) |
| `effect_suspension_linear_safety` | LinearSafety.v | PROVED (from `linear_safety_static`) |

### Formalization Gaps

NONE REMAINING — all formalization gaps have been resolved.

---

## Verification Protocol

### After Each Phase

```bash
cd proofs/theories
eval $(opam env --switch=blood-proofs)
coq_makefile -f _CoqProject -o Makefile.coq
make -f Makefile.coq
```

Confirm:
1. Zero new warnings or errors
2. `grep -c "Admitted." *.v` shows expected count (decreasing toward 0)
3. New files added to `_CoqProject` in correct dependency order

### After All Tier 2 Phases

- Zero Admitted in LinearSafety.v
- All pairwise interaction theorems proved
- `make -f Makefile.coq` clean build

### After All Phases (Final Verification)

- Zero Admitted across entire proof suite (excluding permanent modeling axioms)
- Every safety property Blood claims in its specs has a corresponding proven Coq theorem
- The formalization is specifically about Blood (not a generic effects calculus)
- `full_blood_safety` in CompositionSafety.v is Qed — the master composition theorem

---

## Proof Suite File Inventory

### Existing Files (18)

| File | Lines | Qed | Admitted | Role |
|------|-------|-----|----------|------|
| Syntax.v | 486 | 9 | 0 | AST definitions |
| Typing.v | 372 | 1 | 0 | Typing rules |
| Substitution.v | 1,011 | 21 | 0 | Substitution lemmas |
| ShiftSubst.v | 335 | 4 | 0 | Shift-substitution commutation |
| Semantics.v | 361 | 0 | 0 | Operational semantics (1 Parameter) |
| EffectAlgebra.v | 148 | 7 | 0 | Effect row algebra |
| ContextTyping.v | 713 | 7 | 0 | Evaluation context typing |
| Inversion.v | 574 | 21 | 0 | Typing inversion (1 Axiom) |
| Progress.v | 488 | 9 | 0 | Progress theorem (all 11 cases) |
| Preservation.v | 366 | 3 | 0 | Preservation theorem (all 11 cases) |
| Soundness.v | 282 | 8 | 0 | Type soundness + composition |
| EffectSafety.v | 261 | 9 | 0 | Effect safety (9 theorems) |
| GenerationSnapshots.v | 508 | 14 | 0 | Generation snapshot safety |
| LinearTyping.v | 474 | 2 | 0 | Strengthened typing with linearity |
| LinearSafety.v | 833 | 19 | 0 | Linear/affine safety (all 4 proved) |
| Dispatch.v | 289 | 11 | 0 | Multiple dispatch + type stability |
| Regions.v | 316 | 10 | 0 | Region safety via generations |
| FiberSafety.v | 412 | 13 | 0 | Tier-based concurrency safety |

### Planned New Files (4)

| File | Phase | Tier | Role |
|------|-------|------|------|
| ValueSemantics.v | 7 | T2 | Mutable value semantics |
| EffectSubsumption.v | 8 | T3 | Effects unify control flow patterns |
| MemorySafety.v | 9 | T3 | Memory safety without GC |
| CompositionSafety.v | 11 | T3 | Full composition safety (master theorem) |

### Files Modified or Created (by Phase)

| Phase | Status | Files |
|-------|--------|-------|
| Phase 2 | COMPLETE | LinearTyping.v (new), LinearSafety.v (rewritten) — no changes to existing files |
| Phase 5 | COMPLETE | Regions.v (new, self-contained) |
| Phase 6 | COMPLETE | Dispatch.v (new, self-contained) |
| Phase 7 | Not started | ValueSemantics.v (new, self-contained) |
| Phase 8 | Not started | EffectSubsumption.v (new, self-contained) |
| Phase 9 | Not started | MemorySafety.v (new, imports Phases 2/4/5/7) |
| Phase 10 | COMPLETE | FiberSafety.v (new, imports Phase 5) |
| Phase 11 | Not started | CompositionSafety.v (new, imports all) |

---

## Master Theorem Inventory

Every theorem Blood needs, organized by what it proves.

### Type Safety (Tier 1 — COMPLETE)

| # | Theorem | Status |
|---|---------|--------|
| 1 | `progress` | PROVED |
| 2 | `preservation` | PROVED |
| 3 | `type_soundness_full` | PROVED |

### Effect Safety (Tier 1 — COMPLETE)

| # | Theorem | Status |
|---|---------|--------|
| 4 | `static_effect_containment` | PROVED |
| 5 | `dynamic_effect_containment` | PROVED |
| 6 | `deep_handler_reinstallation` | PROVED |
| 7 | `effect_handling_completeness` | PROVED |
| 8 | `effect_discipline` | PROVED |

### Generation Safety (Tier 1 — COMPLETE)

| # | Theorem | Status |
|---|---------|--------|
| 9 | `no_use_after_free` | PROVED |
| 10 | `gen_snapshot_valid` | PROVED |
| 11 | `effects_gen_composition_safety` | PROVED |

### Effects x Linearity (Tier 2 — Phase 2 — COMPLETE)

| # | Theorem | Status |
|---|---------|--------|
| 12 | `linear_safety_static` | PROVED |
| 13 | `affine_safety_static` | PROVED |
| 14 | `multishot_no_linear_capture` | PROVED |
| 15 | `effect_suspension_linear_safety` | PROVED |

### Regions x Generations (Tier 2 — Phase 5)

| # | Theorem | Status |
|---|---------|--------|
| 16 | `region_safety` | PROVED |
| 17 | `region_effect_safety` | PROVED |
| 18 | `escape_analysis_sound` | PROVED |

### Dispatch x Type Stability (Tier 2 — Phase 6)

| # | Theorem | Status |
|---|---------|--------|
| 19 | `dispatch_determinism` | PROVED |
| 20 | `type_stability_soundness` | PROVED |
| 21 | `dispatch_preserves_typing` | PROVED |

### MVS x Linearity (Tier 2 — Phase 7)

| # | Theorem | Status |
|---|---------|--------|
| 22 | `value_copy_independence` | NOT STARTED |
| 23 | `borrow_linearity` | NOT STARTED |
| 24 | `mvs_no_aliasing` | NOT STARTED |

### Effects Subsume Patterns (Tier 3 — Phase 8)

| # | Theorem | Status |
|---|---------|--------|
| 25 | `effects_subsume_exceptions` | NOT STARTED |
| 26 | `effects_subsume_generators` | NOT STARTED |
| 27 | `effects_subsume_async` | NOT STARTED |
| 28 | `subsumption_safety_transfer` | NOT STARTED |

### Memory Safety Without GC (Tier 3 — Phase 9)

| # | Theorem | Status |
|---|---------|--------|
| 29 | `tier_coverage` | NOT STARTED |
| 30 | `stack_safety` | NOT STARTED |
| 31 | `region_safety_composition` | NOT STARTED |
| 32 | `persistent_safety` | NOT STARTED |
| 33 | `memory_safety_no_gc` | NOT STARTED |

### Tier-Based Concurrency Safety (Tier 3 — Phase 10)

| # | Theorem | Status |
|---|---------|--------|
| 34 | `stack_no_cross` | PROVED |
| 35 | `region_checked_cross` | PROVED |
| 36 | `persistent_free_cross` | PROVED |
| 37 | `tier_crossing_safety` | PROVED |
| 38 | `region_isolation` | PROVED |

### Full Composition Safety (Tier 3 — Phase 11)

| # | Theorem | Status |
|---|---------|--------|
| 39 | `type_soundness_extended` | NOT STARTED |
| 40 | `effect_safety_preserved` | NOT STARTED |
| 41 | `linear_safety_preserved` | NOT STARTED |
| 42 | `generation_safety_preserved` | NOT STARTED |
| 43 | `full_blood_safety` | NOT STARTED |

**Total: 43 theorems. 26 PROVED. 0 ADMITTED. 17 NOT STARTED.**

---

## Scorecard

```
Tier 1 (Core Calculus):       11/11 theorems proved  [====================] 100%
Tier 2 (Interactions):        10/13 theorems proved  [===============     ]  77%
  Phase 2 (Effects x Linear):  4/4  PROVED           [====================] 100%
  Phase 5 (Regions x Gen):     3/3  PROVED           [====================] 100%
  Phase 6 (Dispatch):          3/3  PROVED           [====================] 100%
  Phase 7 (MVS x Linear):      0/3  not started      [                    ]   0%
Tier 3 (Compositions):         5/19 theorems proved  [=====               ]  26%
  Phase 8 (Subsumption):       0/4  not started      [                    ]   0%
  Phase 9 (No GC):             0/5  not started      [                    ]   0%
  Phase 10 (Concurrency):      5/5  PROVED           [====================] 100%
  Phase 11 (Full Composition): 0/5  not started      [                    ]   0%

Overall:                       26/43 theorems        [============        ]  60%
```

---

## Revision History

| Date | Version | Changes |
|------|---------|---------|
| 2026-03-04 | 1.0 | Initial creation. Consolidated from analysis docs 006/007. Added Tier 3 (Phases 8-11). |
| 2026-03-04 | 1.1 | Phase 5 (Regions.v), Phase 6 (Dispatch.v), Phase 10 (FiberSafety.v) completed. |
| 2026-03-04 | 1.2 | Phase 2 COMPLETE: LinearTyping.v (new) + LinearSafety.v (rewritten). 0 Admitted. Two-judgment design. 18 files, 8,229 lines, 168 Qed. All Tier 2 interactions proved except Phase 7. |
