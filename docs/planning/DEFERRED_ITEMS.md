# Phase A Deferred Items Log

**Created**: 2026-02-28
**Purpose**: Track every item deferred during Phase A so nothing falls through the cracks.

---

## How to Use This Log

Every time work is deferred — whether by explicit scope decision or by hitting a technical wall — it gets an entry here with:
- **What** was deferred
- **Why** it was deferred
- **What it blocks** (downstream impact)
- **What unblocks it** (prerequisites to resolve)
- **Where it lives now** (which phase owns it)

Items are removed only when fully resolved, with a completion date and commit hash.

---

## Active Deferrals

### DEF-002: `UncheckedBlock` expression

| Field | Value |
|-------|-------|
| **Delta** | D4 |
| **Deferred from** | Phase A (scope decision) |
| **Deferred to** | Phase C |
| **Date** | 2026-02-28 |
| **Severity** | Medium — new feature, no existing code uses it |

**What**: GRAMMAR.md v0.5.0 added `UncheckedBlock ::= 'unchecked' Block` (§5.4). Neither compiler parses this.

**Why**: Requires semantic implementation — the compiler must know *which checks* to disable inside the block (bounds checks, overflow checks, etc.) and how to propagate that through HIR/MIR/codegen. Parser stub alone has no value without semantics.

**What it blocks**: Granular safety controls (RFC-S / ADR-031).

**What unblocks it**: Design decision on check categories + implementation in typeck/MIR/codegen.

---

### DEF-003: `#[unchecked(...)]` attributes

| Field | Value |
|-------|-------|
| **Delta** | D5 |
| **Deferred from** | Phase A (scope decision) |
| **Deferred to** | Phase C |
| **Date** | 2026-02-28 |
| **Severity** | Medium — new feature, no existing code uses it |

**What**: GRAMMAR.md v0.5.0 added `#[unchecked(bounds)]`, `#[unchecked(overflow)]`, and `#![default_unchecked(...)]` (§1.5.1). Neither compiler recognizes these attributes.

**Why**: Same as DEF-002. Attributes without semantic backing are dead syntax.

**What it blocks**: Per-function and per-module safety control granularity.

**What unblocks it**: DEF-002 (block-level unchecked) should come first; attribute form extends it.

---

### DEF-004: `@heap` / `@stack` allocation expressions

| Field | Value |
|-------|-------|
| **Delta** | D6 |
| **Deferred from** | Phase A (scope decision) |
| **Deferred to** | Phase C or Phase E |
| **Date** | 2026-02-28 |
| **Severity** | Medium — tokens exist in lexer but no parse rules |

**What**: GRAMMAR.md v0.4.0 specifies `@heap expr` and `@stack expr` as allocation placement expressions. Both compilers lex `@heap` and `@stack` as tokens but have no parser rules to handle them.

**Why**: Requires allocation strategy implementation — where does `@heap` allocate? How does `@stack` interact with regions? What's the runtime API? These are Tier 3 memory model questions.

**What it blocks**: Explicit allocation placement (users currently have no way to control heap vs stack).

**What unblocks it**: Memory model Tier 3 design decisions (Phase E) + runtime API for explicit allocation.

---

### DEF-005: `dyn Trait` type syntax

| Field | Value |
|-------|-------|
| **Delta** | D7 |
| **Deferred from** | Phase A (scope decision) |
| **Deferred to** | Phase C |
| **Date** | 2026-02-28 |
| **Severity** | Medium — specified in DISPATCH.md §10.7-10.8, not implemented |

**What**: GRAMMAR.md v0.4.0 specifies `dyn Trait` as a type form. Neither compiler parses it.

**Why**: Requires vtable layout implementation, object safety enforcement (4 rules from DISPATCH.md §10.7), and codegen for dynamic dispatch through vtables. Deep semantic work across all compiler phases.

**What it blocks**: Dynamic dispatch, trait objects, heterogeneous collections.

**What unblocks it**: Object safety checking in typeck + vtable layout in codegen + runtime support.

---

### DEF-006: `in` containment operator (general)

| Field | Value |
|-------|-------|
| **Delta** | D8 |
| **Deferred from** | Phase A (scope decision) |
| **Deferred to** | Phase C |
| **Date** | 2026-02-28 |
| **Severity** | Low — `in` works in for-loops; general containment is new |

**What**: GRAMMAR.md v0.4.0 specifies `ContainmentExpr ::= Expr 'in' Expr` as a general boolean expression (e.g., `x in 0..10`). Currently `in` is only recognized in for-loop headers.

**Why**: General containment requires operator overloading or trait infrastructure so that `in` can work with ranges, collections, etc. For-loop `in` is hard-coded.

**What it blocks**: `x in lo..hi` as a boolean expression (currently must write `x >= lo && x < hi`).

**What unblocks it**: Trait/operator infrastructure for `Contains` or similar protocol.

---

### DEF-007: `Async` → `Fiber` naming migration

| Field | Value |
|-------|-------|
| **Delta** | D9 |
| **Deferred from** | Phase A (scope decision) |
| **Deferred to** | Own sub-phase (A.5 or B/C) |
| **Date** | 2026-02-28 |
| **Severity** | High — naming mismatch between spec (CONCURRENCY.md v0.4.0) and compilers |

**What**: ADR-036 renamed `Async`/`Await` to `Fiber`/`Suspend` throughout the spec. Both compilers still use the `Async`/`Await` naming in lexer, parser, AST, HIR, typeck, MIR, and codegen.

**Why**: Touches every compiler phase in both compilers. Too broad for Phase A which is scoped to parser-level syntax. Needs its own CCV pass.

**What it blocks**: Spec compliance for concurrency syntax. Any new fiber-related features would use wrong naming.

**What unblocks it**: Dedicated rename sub-phase with CCV across all 9 clusters in both compilers. Mechanical but high-volume.

**Scale**: Unknown — needs grep audit. Likely hundreds of occurrences across both compilers.

---

### DEF-008: Continuation token enforcement

| Field | Value |
|-------|-------|
| **Delta** | D10 |
| **Deferred from** | Phase A (scope decision) |
| **Deferred to** | Phase C |
| **Date** | 2026-02-28 |
| **Severity** | Low — existing behavior works; enforcement is polish |

**What**: GRAMMAR.md v0.5.0 §5.2.1 specifies rules for when semicolons can be omitted (continuation tokens: `{`, `.`, `|>`, binary operators, etc.). Neither compiler enforces these rules — they use a practical "accept most things" approach.

**Why**: Enforcement-only change. The current permissive behavior is a superset of the spec's rules. Tightening would reject currently-accepted code without enabling new functionality.

**What it blocks**: Nothing functional. Spec-strict parsing.

**What unblocks it**: Decision on whether to enforce at all, or leave permissive as a pragmatic choice.

---

## Resolved Deferrals

### DEF-001: Expression paths — `lowercase.lowercase` dot syntax ✓

| Field | Value |
|-------|-------|
| **Delta** | D1 |
| **Deferred from** | A.2.1 |
| **Resolved** | 2026-02-28 |
| **Commits** | `0f60269` (selfhost), `ff5ac38` + `4bc7ca1` (bootstrap), `f6fbc79` (migration) |

**Resolution**: Both compilers now resolve `module.function(args)` as qualified calls via name resolution fallback (DEF-001 fix). The approach: `try_extract_module_chain` extracts path segments from nested Field/Path AST expressions, guards against local variables, then `resolve_qualified_path` (selfhost) or `try_resolve_qualified_chain` (bootstrap) walks the module hierarchy. Return types are properly extracted from function signatures and unified with arguments.

**Result**: ~1,960 `lowercase::lowercase` expression paths converted to dot syntax across 47 selfhost files. 8 three-segment type paths remain using `::` for the first separator (`token::common.Span` × 6, `codegen::codegen_ctx.CodegenError` × 2) — these require the type-path parser to support 3+ dot-separated segments, a separate issue.

**Verification**: 339/339 ground-truth, second_gen/third_gen byte-identical (13,079,128 bytes), 1269/1269 bootstrap lib tests.

---

### DEF-009: Cast + linearity/regions interaction unspecified

| Field | Value |
|-------|-------|
| **Discovered** | Phase C audit (2026-03-01) |
| **Deferred to** | Spec addendum (FORMAL_SEMANTICS.md) |
| **Date** | 2026-03-01 |
| **Severity** | Medium — correctness gap in spec, not yet exploitable by user code |

**What**: The [T-Cast] rule in FORMAL_SEMANTICS.md §5.10 propagates effect rows but does not specify how ownership qualifiers (`linear`, `affine`) or region validity interact with casts. Can you cast `linear i32` to `i32` (dropping linearity)? Can you cast a region-derived pointer to `usize` and escape the region?

**Why deferred**: C.1 implemented `is_cast_compatible()` for numeric/pointer categories, which is correct for those categories. The ownership/region interaction is a spec-level design question requiring a formal decision, not just an implementation fix.

**What it blocks**: Sound cast semantics for linear/affine types and region pointers.

**What unblocks it**: Design decision on whether casts preserve, strip, or transform ownership qualifiers. Addendum to [T-Cast] rule.

---

### DEF-010: `dyn Trait` vtable `drop_fn` conflicts with Blood's memory model

| Field | Value |
|-------|-------|
| **Discovered** | Rust-ism audit (2026-03-01) |
| **Deferred to** | Pre-DEF-005 (must resolve before implementing dyn Trait) |
| **Date** | 2026-03-01 |
| **Severity** | High — architectural conflict in spec |

**What**: DISPATCH.md §10.8.1 specifies a `drop_fn` slot in the `dyn Trait` vtable layout. Blood has no `Drop` trait — memory cleanup uses region deallocation, Tier 2 reference counting, and `finally` clauses in handlers. The `drop_fn` vtable slot is undefined in Blood's memory model.

**Why deferred**: `dyn Trait` is not yet implemented (DEF-005). But the vtable spec must be corrected before implementation begins.

**What it blocks**: DEF-005 (dyn Trait implementation).

**What unblocks it**: Decide what replaces `drop_fn` in Blood's vtable: region-aware cleanup? `finally` handler? Remove the slot? Update DISPATCH.md §10.8.

---

### DEF-011: `Send` trait used in spec but never defined

| Field | Value |
|-------|-------|
| **Discovered** | Rust-ism audit (2026-03-01) |
| **Deferred to** | Spec addendum |
| **Date** | 2026-03-01 |
| **Severity** | High — undefined symbol in a formal soundness proof |

**What**: `Send` appears in GRAMMAR.md §3.4.1 (fiber spawn signature) and MEMORY_MODEL.md §7.8 (soundness proof) but has no Blood-native definition. In Rust, `Send` is an auto-trait for thread-safe transfer. Blood's region isolation, linear types, and effect system provide orthogonal mechanisms for cross-fiber safety. DESIGN_SPACE_AUDIT.md flags this as "Defaulted" with no resolution.

**Why deferred**: Requires a design decision on whether `Send` is a trait, an effect, a type-level property, or unnecessary in Blood's model.

**What it blocks**: Fiber spawn type safety, soundness proof completeness.

**What unblocks it**: Design decision on Blood-native cross-fiber safety mechanism. ADR needed.

---

### DEF-012: `&T`/`&mut T` syntax semantically misleading for generational references

| Field | Value |
|-------|-------|
| **Discovered** | Rust-ism audit (2026-03-01) |
| **Deferred to** | Long-term evaluation |
| **Date** | 2026-03-01 |
| **Severity** | Low — syntax works, semantics are documented, but misleading to Rust developers |

**What**: Blood uses `&T`/`&mut T` syntax identical to Rust, but the semantics are fundamentally different (generational references, not borrow-checked). MEMORY_MODEL.md §1.5 uses the word "borrowing" despite Blood not having borrows. DESIGN_SPACE_AUDIT.md flags this under "Inherited Decisions Warranting Independent Evaluation." ADR-035 confirmed the syntax but noted the semantic difference.

**Why deferred**: Changing reference syntax would be enormously disruptive for minimal benefit. The syntax is familiar and functional. The semantic difference is well-documented in the spec.

**What it blocks**: Nothing functional. Developer mental model accuracy.

**What unblocks it**: If Blood ever revisits reference syntax, this should be reconsidered. For now, improve documentation to avoid "borrowing" language.

---

### DEF-013: `#[derive(Clone, Debug, Eq)]` — Rust trait names without Blood definitions

| Field | Value |
|-------|-------|
| **Discovered** | Rust-ism audit (2026-03-01) |
| **Deferred to** | Stdlib design phase |
| **Date** | 2026-03-01 |
| **Severity** | Low — cosmetic; `derive` is not yet implemented |

**What**: GRAMMAR.md §1.5.1 lists `#[derive(Clone, Debug, Eq)]` with Rust trait names. Blood has mutable value semantics (MVS) where copying is the default, making `Clone` semantically different from Rust's. `Debug` and `Eq` have no Blood-specific definitions.

**Why deferred**: `derive` macro expansion is not implemented. These names are placeholders.

**What it blocks**: Nothing currently. Will need resolution when derive macros are implemented.

**What unblocks it**: Stdlib trait design phase — define what `Clone`, `Debug`, `Eq` mean in Blood's MVS context.

---

### DEF-014: `dyn Trait` vs effects for heterogeneous collections (unevaluated)

| Field | Value |
|-------|-------|
| **Discovered** | Rust-ism audit (2026-03-01) |
| **Deferred to** | Pre-DEF-005 |
| **Date** | 2026-03-01 |
| **Severity** | Medium — design space question |

**What**: `dyn Trait` is in the spec with ABI rationale (heterogeneous collections, plugin interfaces). However, no evaluation exists of whether Blood's effects + handlers + fibers could serve these use cases without Rust-style vtables. The justification is an ABI constraint argument, not a "this is the best Blood-native solution" argument.

**Why deferred**: `dyn Trait` is not yet implemented. The evaluation should happen before implementation begins.

**What it blocks**: Confidence that DEF-005 implementation is the right approach.

**What unblocks it**: Design evaluation: can effects replace vtables for Blood's use cases? If not, document why vtables are necessary alongside effects.

---

### DEF-015: `Result<T,E>`/`Option<T>`/`?` coexistence with effects (F-05)

| Field | Value |
|-------|-------|
| **Discovered** | DESIGN_SPACE_AUDIT.md F-05, confirmed in Rust-ism audit (2026-03-01) |
| **Deferred to** | Ecosystem guidelines |
| **Date** | 2026-03-01 |
| **Severity** | Medium — ecosystem coherence risk |

**What**: ADR-033 resolved F-05 as "Complementary — Result for leaf, effects for orchestration." However, the `?` operator's interaction with algebraic effects is unspecified. Libraries may split between Result-style and effect-style error handling without clear guidance.

**Why deferred**: The complementary model works in principle. Concrete guidance needs more implementation experience.

**What it blocks**: Ecosystem consistency, stdlib API design.

**What unblocks it**: Usage patterns from real Blood programs. Write ecosystem guidelines when sufficient experience exists.

---

## Summary

| ID | Item | Severity | Target Phase | Status |
|----|------|----------|-------------|--------|
| ~~DEF-001~~ | ~~Expression path `.`~~ | ~~Medium~~ | ~~Phase A~~ | **RESOLVED** (2026-02-28) |
| DEF-002 | UncheckedBlock | Medium | Phase C | Active |
| DEF-003 | #[unchecked(...)] | Medium | Phase C | Active |
| DEF-004 | @heap/@stack | Medium | Phase C/E | Active |
| DEF-005 | dyn Trait | Medium | Phase C | Active — **blocked by DEF-010, DEF-014** |
| DEF-006 | `in` containment | Low | Phase C | Active |
| DEF-007 | Async → Fiber | High | Sub-phase | Active |
| DEF-008 | Continuation tokens | Low | Phase C | Active |
| DEF-009 | Cast + linearity/regions | Medium | Spec addendum | Active |
| DEF-010 | `dyn Trait` vtable `drop_fn` | High | Pre-DEF-005 | Active |
| DEF-011 | `Send` undefined in Blood | High | Spec addendum | Active |
| DEF-012 | `&T`/`&mut T` misleading | Low | Long-term | Active |
| DEF-013 | `derive` Rust trait names | Low | Stdlib design | Active |
| DEF-014 | `dyn Trait` vs effects eval | Medium | Pre-DEF-005 | Active |
| DEF-015 | Result/Option × effects | Medium | Ecosystem guidelines | Active |

**Active: 14 items.** Resolved: 1 (DEF-001).
**High severity (3)**: DEF-007 (naming mismatch), DEF-010 (`drop_fn` conflict), DEF-011 (`Send` undefined).
**Medium severity (7)**: DEF-002–005, DEF-009, DEF-014, DEF-015.
**Low severity (4)**: DEF-006, DEF-008, DEF-012, DEF-013.
