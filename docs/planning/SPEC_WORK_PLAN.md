# Specification Work Plan

**Version**: 2.0
**Established**: 2026-02-28
**Last Updated**: 2026-02-28
**Status**: Active

This document captures the remaining work to resolve open design questions, bring Blood's specifications and compilers into full alignment, complete formal verification, and close all known gaps.

---

## Table of Contents

1. [Context](#1-context)
2. [Spec Maturity Summary](#2-spec-maturity-summary)
3. [Phase 0: Design Space Resolution](#3-phase-0-design-space-resolution)
4. [Phase A: Syntax Alignment](#4-phase-a-syntax-alignment)
5. [Phase B: Semantic Alignment Audit](#5-phase-b-semantic-alignment-audit)
6. [Phase C: Semantic Alignment Fixes](#6-phase-c-semantic-alignment-fixes)
7. [Phase D: Coq Mechanization](#7-phase-d-coq-mechanization)
8. [Phase E: Tier 3 Implementation](#8-phase-e-tier-3-implementation-pre-10)
9. [Phase F: Performance Validation](#9-phase-f-performance-validation-pre-10)
10. [Sequencing Rationale](#10-sequencing-rationale)
11. [Decisions Made](#11-decisions-made)
12. [Design Space Audit Reference](#12-design-space-audit-reference)

---

## 1. Context

As of 2026-02-28, the specification documents have reached a mature state:

- **GRAMMAR.md** (v0.4.0) is settled — the source of truth for surface syntax. Only procedural macros remain deferred (legitimate: semantic design must precede syntax).
- **FORMAL_SEMANTICS.md** (v0.4.0) now formalizes closures, regions, pattern matching, casts, and associated types. A scope statement explicitly lists what is and isn't formalized.
- **DISPATCH.md** (v0.4.0) now includes object safety rules and dyn Trait dispatch semantics.
- **SPECIFICATION.md** (v0.3.0) body is current and comprehensive. Dead links fixed, MACROS.md added to hierarchy.

All 337/337 ground-truth tests pass. Bootstrap is stable (second_gen/third_gen byte-identical). The compilers work correctly but use **old syntax** in places where GRAMMAR.md has evolved.

**However**, a comprehensive design space audit ([DESIGN_SPACE_AUDIT.md](../design/DESIGN_SPACE_AUDIT.md)) identified 30 accidental defaults and several architectural tensions that must be resolved **before** committing to compiler alignment work. Syntax and semantic alignment are premature if unresolved design questions could change the language.

---

## 2. Spec Maturity Summary

| Document | Version | Status | Gaps |
|----------|---------|--------|------|
| GRAMMAR.md | 0.4.0 | Settled (pending Phase 0 outcomes) | Procedural macros deferred; concurrency syntax TBD |
| FORMAL_SEMANTICS.md | 0.4.0 | Core features formalized | Coq mechanization incomplete (§7) |
| DISPATCH.md | 0.4.0 | Complete | None |
| MEMORY_MODEL.md | 0.3.0 | Tiers 0/1 solid | Tier 3 designed but not implemented |
| CONCURRENCY.md | 0.3.0 | Incomplete | Largest design gap (see Phase 0) |
| MACROS.md | 0.1.0 | Syntax/expansion covered | Hygiene deferred (compiler semantics, not grammar) |
| SPECIFICATION.md | 0.3.0 | Current | None |
| FFI.md | 0.4.0 | Complete | None |
| CONTENT_ADDRESSED.md | 0.4.0 | Updated with monomorphized instance hashing | ADR-030 resolved F-01 tension |
| DIAGNOSTICS.md | 0.4.0 | Complete | None |

---

## 3. Phase 0: Design Space Resolution

**Priority**: Highest — unresolved design questions could change syntax, semantics, and compiler architecture. Alignment work is premature until these are settled.

**Input**: [DESIGN_SPACE_AUDIT.md](../design/DESIGN_SPACE_AUDIT.md) — 98 design axes evaluated, 10 findings.

### 0.1 — Architectural Findings (must resolve first)

These findings could change what we're building. Each requires an ADR or design document.

#### F-01: Monomorphization × Content Addressing

**Severity**: Architectural — latent tension between two core innovations.

Blood uses content-addressed compilation (each definition identified by hash) and monomorphization (each generic instantiation produces a specialized copy). Undocumented interactions:

1. Hash space explosion (50 type instantiations × 1 generic = 50 hashes)
2. Incremental invalidation cascading (type change invalidates all monomorphized uses)
3. Caching efficiency (nominal types prevent structural sharing)
4. Dictionary passing as alternative (one hash per generic, runtime dispatch cost)

**Deliverable**: ADR documenting the hashing strategy for monomorphized instances, the invalidation model, and whether dictionary passing was evaluated.

**Impact if answer changes**: Could change codegen, caching model, CONTENT_ADDRESSED.md, and performance characteristics.

#### F-06: Concurrency Model

**Severity**: Architectural — largest undecided area.

Blood has the pieces (effects for async, fiber runtime, handler scoping) but hasn't assembled them into a cohesive concurrency model. Eight sub-decisions are defaulted:

| Sub-decision | Status |
|-------------|--------|
| Structured concurrency (task scoping) | Defaulted |
| Cancellation mechanism | Deferred (DECISIONS.md) |
| Cancellation safety guarantees | Defaulted |
| Async drop / cleanup | Defaulted |
| Thread-safety markers (Send/Sync) | Defaulted |
| Async iterators / streams | Defaulted |
| Runtime-provided vs. library concurrency | Defaulted |
| Fiber ↔ OS thread interaction | Defaulted |

**Deliverable**: Design document composing effects + fibers + handlers into a cohesive concurrency model. Define structured concurrency via effect handler scoping, cancellation as an effect, thread-safety as effect-based or trait-based constraints.

**Impact if answer changes**: Could add new syntax (GRAMMAR.md), new effects (FORMAL_SEMANTICS.md), new runtime contracts (CONCURRENCY.md), and new typing rules.

**Risk**: If the fiber runtime calcifies before the language-level model is designed, the runtime constrains the language rather than serving it.

#### F-07: Compiler-as-a-Library

**Severity**: Architectural — expensive to retrofit.

The self-hosted compiler is a monolithic pipeline. Content-addressed compilation is naturally query-based (each definition independently hashable and cacheable), but the compiler doesn't exploit this. Every CCV cluster that adds code to the monolithic pipeline makes a query-based retrofit more expensive.

**Deliverable**: Architectural note evaluating query-based internal architecture aligned with content addressing. This constrains the compiler's internal API boundaries — it does not require immediate implementation.

**Impact if answer changes**: Could restructure self-hosted compiler modules, change how CCV clusters are organized.

### 0.2 — Ecosystem Coherence (resolve before alignment)

These don't change the core language but affect how it's used. Short ADRs.

#### F-05: Result/Option × Effects

**Severity**: Ecosystem coherence.

Blood has effects as primary error handling AND `Result<T, E>` / `Option<T>` inherited from Rust. Without guidance, the ecosystem will split: some libraries use effects, others use `Result`, composing them requires boilerplate.

**Deliverable**: ADR specifying the intended role of `Result` and `Option` alongside effects, when each is appropriate, and how they interconvert.

### 0.3 — Design Gaps (resolve with short ADRs)

These require evaluation but likely confirm existing direction.

| # | Finding | Recommended Resolution |
|---|---------|----------------------|
| F-02 | Higher-kinded types | Evaluate whether row poly + effects + dispatch cover HKT use cases; document conclusion |
| F-03 | Variance | Document "all type parameters invariant by default" with future relaxation path |
| F-04 | String representation × 128-bit pointers | Document concrete `&str` and `&[T]` representation under Blood's memory model |
| F-08 | Stdlib scope / freestanding split | Document core/alloc/std tier mapping strategy |
| F-09 | Testing as language feature | Evaluate effect-based test declarations as differentiator |
| F-10 | ABI stability | Document "explicitly unstable until further notice" + content-hash-based ABI concept |

### 0.4 — Minimal-Effort Defaults (batch resolve)

One-paragraph decision records each:

1. **Cyclic imports**: Allowed or forbidden? (Likely: forbidden, matches content-addressed DAG)
2. **Interior mutability**: Supported or not? (Likely: defer, document as not-yet-designed)
3. **Dead code detection**: Planned or not? (Likely: yes, as compiler warning)
4. **Definite initialization**: Statically enforced? (Likely: yes, via MIR analysis)
5. **Doc comment syntax**: `///` or other? (Decide before stdlib grows)
6. **Frame pointer preservation**: Default on or off? (Likely: on, for profiling)
7. **Variance**: Invariant by default? (Likely: yes)

### 0.5 — Inherited Decisions to Confirm

These were adopted from Rust without documented independent evaluation. Each needs at minimum a brief ADR confirming the choice in Blood's context:

| Decision | Why It Warrants Evaluation |
|----------|---------------------------|
| Monomorphization | Interacts with content addressing (F-01) |
| `Option<T>` / `Result<T, E>` | Coexists with effects (F-05) |
| UTF-8 strings | Interacts with 128-bit pointers (F-04) |
| File-based module hierarchy | Content addressing decouples identity from files |
| `pub` visibility (Rust-style) | Row polymorphism introduces structural subtyping |
| Call-by-value evaluation | Natural for effects but undocumented |
| No runtime type information | Multiple dispatch uses 24-bit type fingerprints — this IS RTTI |
| `&T` / `&mut T` reference syntax | Blood's references are generational, not borrowed |

### Phase 0 Exit Criteria

Phase 0 is complete when:
- [x] F-01 ADR written and accepted (ADR-030, 2026-02-28)
- [ ] F-06 design document written and accepted
- [ ] F-07 architectural note written
- [ ] F-05 ADR written
- [ ] F-02, F-03, F-04, F-08, F-09, F-10 resolved (ADRs or design notes)
- [ ] All 7 minimal-effort defaults documented
- [ ] All 8 inherited decisions confirmed or revised
- [ ] GRAMMAR.md updated if any Phase 0 outcome changes syntax
- [ ] FORMAL_SEMANTICS.md updated if any Phase 0 outcome changes typing rules
- [ ] CONCURRENCY.md updated with cohesive concurrency model

---

## 4. Phase A: Syntax Alignment

**Priority**: High — blocks compiler alignment.
**Prerequisite**: Phase 0 complete (design is stable).
**Method**: CCV (Canary-Cluster-Verify) per DEVELOPMENT.md.

The compilers currently accept old syntax in several places where GRAMMAR.md has evolved. Every `.blood` file in the repository must be audited and updated.

### A.1 — Syntax Delta Analysis

Comprehensive diff between GRAMMAR.md productions and what each parser actually accepts. Covers:

- Imports (grouped, glob, simple) — known `::` vs `.` gap
- Qualified expressions and paths
- Type syntax
- Expression syntax
- Pattern syntax
- Bridge/FFI syntax
- Effect/handler syntax
- Macro syntax
- Any new syntax from Phase 0 outcomes

**Inputs**: GRAMMAR.md, `src/bootstrap/bloodc/src/parser/`, `src/bootstrap/bloodc/src/lexer.rs`, `src/selfhost/parser_*.blood`, `src/selfhost/lexer.blood`, `src/selfhost/token.blood`

**Output**: Complete list of deltas with severity (breaking vs cosmetic).

### A.2 — Update Bootstrap Compiler (Rust)

The bootstrap compiler defines language semantics. It must accept the spec syntax **first**.

- Update lexer/parser in `src/bootstrap/bloodc/src/` to match GRAMMAR.md
- Rebuild: `cd src/bootstrap && cargo build --release`
- Verify: `cargo test --workspace` (unit tests must pass)

**This is a Bootstrap Gate prerequisite** — nothing else moves until this is done.

### A.3 — CCV Migration of Self-Hosted Compiler

Update all `.blood` files in `src/selfhost/` following CCV:

| Cluster | Files | Subsystem |
|---------|-------|-----------|
| A | `common`, `interner`, `source`, `error`, `reporter` | Utilities |
| B | `lexer`, `token`, `parser_*`, `macro_expand` | Frontend |
| C | `ast`, `hir`, `hir_expr`, `hir_item`, `hir_ty`, `hir_def` | AST/HIR Definitions |
| D | `hir_lower`, `hir_lower_*`, `resolve` | HIR Lowering |
| E | `typeck`, `typeck_*`, `unify`, `type_intern` | Type Checking |
| F | `mir_*`, `validate_mir` | MIR |
| G | `codegen`, `codegen_*` | Code Generation |
| H | `driver`, `main`, `project`, `package`, `build_cache` | Driver + Project |
| I | `stdlib/*.blood` | Standard Library |

After **each** cluster:
```bash
cd src/selfhost
./build_selfhost.sh timings        # Build first_gen
./build_selfhost.sh ground-truth   # All 337 must pass
./build_selfhost.sh rebuild        # second_gen/third_gen byte-identical
cd ../.. && ./tools/parse-parity.sh --quiet  # No new drift
git commit                          # Clean rollback point
```

### A.4 — Update Tests and Examples

- Update `tests/ground-truth/*.blood` to spec syntax
- Update `stdlib/*.blood` to spec syntax
- Update any example files
- Verify all tests still pass

---

## 5. Phase B: Semantic Alignment Audit

**Priority**: High — cheap analysis, high information value.
**Prerequisite**: Phase A complete (syntax aligned).

Audit whether compiler behavior matches the formal semantics we've specified:

| # | Check | Spec Section | Method |
|---|-------|-------------|--------|
| B.1 | Closure capture modes match §5.7 | FORMAL_SEMANTICS.md §5.7 | Review typeck_expr, mir_closure vs rules |
| B.2 | Region generation bumping matches §5.8 | FORMAL_SEMANTICS.md §5.8 | Review codegen + runtime vs rules |
| B.3 | Object safety enforced per §10.7 | DISPATCH.md §10.7 | Attempt to create dyn Trait from unsafe trait |
| B.4 | dyn Trait vtable layout matches §10.8 | DISPATCH.md §10.8 | Review codegen_types vs spec |
| B.5 | Pattern exhaustiveness matches §5.9 | FORMAL_SEMANTICS.md §5.9 | Test non-exhaustive patterns |
| B.6 | Cast compatibility matches §5.10 | FORMAL_SEMANTICS.md §5.10 | Test each cast category |
| B.7 | Associated type resolution matches §5.11 | FORMAL_SEMANTICS.md §5.11 | Test projection, defaults |

**Output**: List of semantic gaps (if any) with severity and recommended fixes.

---

## 6. Phase C: Semantic Alignment Fixes

**Priority**: High — depends on Phase B findings.
**Prerequisite**: Phase B complete.

Fix any semantic gaps discovered in Phase B. Scope is unknown until audit completes. Each fix follows CCV.

---

## 7. Phase D: Coq Mechanization

**Priority**: Medium-high — validates soundness claims.
**Prerequisite**: Phases A-C complete (rules are stable before proving them).

### Current State

- 10 Coq files in `proofs/theories/`, ~2,635 lines
- 12/32 theorems proved, 20 Admitted
- Build infrastructure missing (`_CoqProject`, `Makefile`)
- Blocking dependency: Substitution lemma incomplete → blocks all Preservation proofs

### D.1 — Infrastructure

| # | Work Item |
|---|-----------|
| D.1.1 | Create `proofs/_CoqProject` with file list and imports |
| D.1.2 | Create `proofs/Makefile` targeting Coq 8.18+ |
| D.1.3 | Verify all 10 .v files compile |

### D.2 — Blocking Proofs (unblock everything else)

| # | Work Item | File |
|---|-----------|------|
| D.2.1 | Complete `substitution_preserves_typing` | Substitution.v |
| D.2.2 | Complete `shift_subst_commute` (3 admits) | Substitution.v |
| D.2.3 | Complete `shift_then_subst_general` (record fields, handlers) | Substitution.v |

### D.3 — Core Theorems

| # | Work Item | File | Admits |
|---|-----------|------|--------|
| D.3.1 | Prove `progress` (full induction) | Progress.v | 1 |
| D.3.2 | Prove `preservation` (all 10 reduction cases) | Preservation.v | 7 |
| D.3.3 | Prove `context_typing` | Preservation.v | 1 |
| D.3.4 | Prove effect row algebra lemmas | Preservation.v | 2 |

### D.4 — Safety Theorems

| # | Work Item | File | Admits |
|---|-----------|------|--------|
| D.4.1 | `static_effect_containment` | EffectSafety.v | 1 |
| D.4.2 | `effect_handling_completeness` | EffectSafety.v | 1 |
| D.4.3 | `effect_union_monotone_left` + `effect_union_comm` | EffectSafety.v | 2 |
| D.4.4 | `effect_discipline` | EffectSafety.v | 1 |
| D.4.5 | `linear_safety_static` | LinearSafety.v | 1 |
| D.4.6 | `affine_safety_static` | LinearSafety.v | 1 |
| D.4.7 | `linear_single_shot_safe` | LinearSafety.v | 1 |
| D.4.8 | `multishot_no_linear_capture` | LinearSafety.v | 1 |
| D.4.9 | `effect_suspension_linear_safety` | LinearSafety.v | 1 |
| D.4.10 | `no_use_after_free` | GenerationSnapshots.v | 1 |

### D.5 — Composition

| # | Work Item | File | Admits |
|---|-----------|------|--------|
| D.5.1 | `type_soundness_full` | Soundness.v | 1 |
| D.5.2 | `effect_safety` (complete logic) | Soundness.v | 1 |
| D.5.3 | `linear_safety` | Soundness.v | 1 |
| D.5.4 | `full_composition_safety` | Soundness.v | 1 |

### D.6 — New Typing Rules (from 2026-02-28 additions)

| # | Work Item | Files |
|---|-----------|-------|
| D.6.1 | Add closure typing (T-Closure, T-Closure-Move) | Syntax.v, Typing.v |
| D.6.2 | Add region typing (T-Region) | Syntax.v, Typing.v |
| D.6.3 | Add pattern matching (T-Match, P-* rules) | Syntax.v, Typing.v |
| D.6.4 | Add cast typing (T-Cast, cast_compatible) | Typing.v |
| D.6.5 | Add associated type projection | Typing.v |
| D.6.6 | Add closure-handler linearity interaction | LinearSafety.v |
| D.6.7 | Extend Progress/Preservation for new cases | Progress.v, Preservation.v |

**Dependency chain**: D.1 → D.2 → D.3 → D.4/D.5 (parallel). D.6 can proceed in parallel with D.2-D.5 but D.6.7 depends on D.3.

---

## 8. Phase E: Tier 3 Implementation (Pre-1.0)

**Priority**: Low — designed but not yet needed by any user code.
**Prerequisite**: Phases A-C complete.

| # | Work Item | Component |
|---|-----------|-----------|
| E.1 | Implement Tier 3 allocation (reference counting) in runtime | runtime/ |
| E.2 | Implement cycle collection algorithm in runtime | runtime/ |
| E.3 | Implement Tier 3 codegen in bootstrap compiler | src/bootstrap/bloodc/ |
| E.4 | Implement Tier 3 codegen in self-hosted compiler | src/selfhost/ |
| E.5 | Implement escape analysis (Tier 1 → Tier 3 promotion) | Both compilers |
| E.6 | Write ground-truth tests for Tier 3 | tests/ground-truth/ |
| E.7 | Write ground-truth tests for cycle collection | tests/ground-truth/ |
| E.8 | Validate spec matches implementation, update MEMORY_MODEL.md | docs/spec/ |

---

## 9. Phase F: Performance Validation (Pre-1.0)

**Priority**: Lowest — explicitly marked pre-1.0 in SPECIFICATION.md §11.7.
**Prerequisite**: Tier 3 implemented (for memory benchmarks).

| # | Work Item | Type |
|---|-----------|------|
| F.1 | Design dispatch benchmark suite | Design |
| F.2 | Design memory model benchmark suite | Design |
| F.3 | Implement benchmarks as Blood programs | Implementation |
| F.4 | Run benchmarks against first_gen | Measurement |
| F.5 | Replace external citations in DISPATCH.md with Blood measurements | Documentation |
| F.6 | Replace external citations in MEMORY_MODEL.md with Blood measurements | Documentation |

---

## 10. Sequencing Rationale

```
Phase 0  ──►  Phase A  ──►  Phase B  ──►  Phase C  ──►  Phase D
(design)      (syntax)      (semantic      (semantic      (Coq proofs)
                            audit)         fixes)
                                                         ──► Phase E ──► Phase F
                                                             (Tier 3)    (benchmarks)
```

**Why Phase 0 comes first:**

The design space audit ([DESIGN_SPACE_AUDIT.md](../design/DESIGN_SPACE_AUDIT.md)) identified 30 accidental defaults, 3 architectural tensions (F-01, F-06, F-07), and 18 inherited decisions without independent evaluation. If any of these change the language — particularly F-01 (monomorphization strategy) or F-06 (concurrency model adding new syntax/effects) — then syntax alignment work done before resolution would need to be redone.

Phase 0 is design work: ADRs, design documents, architectural notes. It produces documents, not code. But those documents determine what the code should look like.

**Why Phase A after Phase 0:**

Once design questions are settled, the specs become truly stable. Only then does it make sense to spend CCV cycles (expensive, meticulous) aligning every `.blood` file with the spec. The `::` vs `.` syntax gap is known; Phase 0 may reveal others.

**Why Phase B/C after A:**

Semantic audit is meaningless until syntax is aligned — you can't test whether the compiler implements the spec if the compiler can't parse the spec's syntax.

**Why Phase D after A-C:**

Coq proofs formalize typing rules. If Phase 0 changes a typing rule, or Phase B/C reveals a rule is wrong, we'd have to redo Coq work. Align first, then prove.

**Why Phase E/F deferred:**

Tier 3 is fully designed but no code exercises it. Performance benchmarks don't affect correctness. Both are explicitly pre-1.0 work.

---

## 11. Decisions Made

The following semantic decisions were made during the 2026-02-28 specification session, derived from Blood's design philosophy documents (SPECIFICATION.md, DECISIONS.md, MEMORY_MODEL.md, DISPATCH.md, FORMAL_SEMANTICS.md):

### Closure Typing (FORMAL_SEMANTICS.md §5.7)

| Rust Concept | Blood Replacement | Derived From |
|---|---|---|
| `Fn` (shared access) | `fn(T) -> U` with no mutation effects | ADR-002: effects track mutation |
| `FnMut` (mutable access) | `fn(T) -> U / {State<S>}` | ADR-002: mutation is an effect |
| `FnOnce` (consumed) | `linear fn(T) -> U` | ADR-006: linear types = exactly-once |

- **Linearity propagation**: Closure capturing linear value by-value becomes linear itself
- **No aliasing**: ByRef/ByMut capture of linear values forbidden
- **Effects orthogonal**: Effect row describes what closure *does*, not what it *captures*
- **Inference**: `move` keyword signals by-value capture; linearity inferred from capture analysis

### Region Typing (FORMAL_SEMANTICS.md §5.8)

- **No type-level region annotations** — would re-introduce borrow checking (violates ADR-001)
- **Safety via generations**: Region exit bumps generations; stale references detected at runtime
- **Invisible to type system**: `region { e }` has same type as `e`
- **Effect interaction**: Region deallocation deferred when effect handlers hold continuations referencing region memory

### Object Safety (DISPATCH.md §10.7)

- **ABI constraints**, not arbitrary restrictions
- **Four rules**: No generic methods on Self, no Self by value, no Self return, associated types must be determinable
- **Orthogonal to multiple dispatch**: Type stability applies to static dispatch, not vtable construction

### dyn Trait Dispatch (DISPATCH.md §10.8)

- **Vtable layout**: drop_fn, size, align, then methods in declaration order
- **Fat pointer**: `{ data: *const (), vtable: *const Vtable<Trait> }`
- **Composes with multiple dispatch**: A method family can include specialization for `dyn Trait` types

### Other Decisions

- **impl Trait return-position**: Not planned (effects eliminate async returns, universal `fn` type eliminates unnamed closures)
- **Labeled blocks**: Not planned (loops only; effects subsume non-local control flow)
- **T: 'a lifetime bounds**: Not planned (ADR-001 rejected borrow checker; ADR-008 tiered regions replace)
- **Fn/FnMut/FnOnce traits**: Not planned (effects + linear types + row polymorphism replace)
- **union keyword**: Bridge FFI only; Blood uses enums (tagged unions)

---

## 12. Design Space Audit Reference

The full design space audit is at [docs/design/DESIGN_SPACE_AUDIT.md](../design/DESIGN_SPACE_AUDIT.md). Key statistics:

| Category | Count | Percentage |
|----------|-------|------------|
| Consciously Decided | 42 | 43% |
| Inherited from Rust | 18 | 18% |
| Accidentally Defaulted | 30 | 31% |
| Explicitly Deferred | 8 | 8% |

**Top 3 findings by severity:**

1. **F-01**: Monomorphization × content addressing — latent tension between two core innovations
2. **F-06**: Concurrency model — largest undecided area; effects + fibers not composed
3. **F-07**: Compiler-as-a-library — monolithic pipeline; retrofit is expensive

**Phase 0 resolves all findings before alignment work begins.**

---

## Work Item Counts

| Phase | Items | Nature |
|-------|-------|--------|
| 0: Design resolution | 3 architectural + 1 ecosystem + 6 design gaps + 7 defaults + 8 inherited = **25** | Design / ADRs |
| A: Syntax alignment | 4 major steps (A.1-A.4), 9 CCV clusters in A.3 | Implementation |
| B: Semantic audit | 7 checks | Analysis |
| C: Semantic fixes | Unknown (depends on B) | Implementation |
| D: Coq mechanization | 31 items across 7 sub-phases | Formal proofs |
| E: Tier 3 | 8 items | Implementation |
| F: Performance | 6 items | Measurement |
| **Total** | **80+ items** | |
