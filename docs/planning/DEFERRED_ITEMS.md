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
| **Status** | **UNBLOCKED** — DEF-014 resolved (2026-03-04), design evaluation confirms vtables needed |

**What**: GRAMMAR.md v0.4.0 specifies `dyn Trait` as a type form. Neither compiler parses it.

**Why**: Requires vtable layout implementation, object safety enforcement (4 rules from DISPATCH.md §10.7), and codegen for dynamic dispatch through vtables. Deep semantic work across all compiler phases.

**What it blocks**: Dynamic dispatch, trait objects, heterogeneous collections.

**What unblocks it**: ~~DEF-014 evaluation~~ ✓ + Object safety checking in typeck + vtable layout in codegen + runtime support.

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

### ~~DEF-007: `Async` → `Fiber` naming migration~~ ✓

| Field | Value |
|-------|-------|
| **Delta** | D9 |
| **Deferred from** | Phase A (scope decision) |
| **Resolved** | 2026-03-01 |
| **Severity** | High — naming mismatch between spec (CONCURRENCY.md v0.4.0) and compilers |

**Resolution**: Renamed `Async`→`Fiber` and `Await`→`Suspend` across both compilers and all spec/guide documents. Bootstrap compiler: lexer (token variants + keyword aliases), syntax_kind, parser, AST (`is_async`→`is_fiber`), HIR, typeck, codegen, VFT, build cache, LSP, macro expansion. Self-hosted compiler: token.blood, lexer.blood, interner.blood, common.blood, parser_item.blood, parser_expr.blood, parser_base.blood. Both compilers accept both `async`/`await` (backward compat) and `fiber`/`suspend` (new canonical). All spec documents updated: STDLIB.md (effect definition + 11 annotations + handler section), 10 guide/comparison docs. Verification: 1269/1269 lib tests, 344/344 ground-truth, second_gen/third_gen byte-identical (13,079,144 bytes).

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

**Result**: ~1,960 `lowercase::lowercase` expression paths converted to dot syntax across 47 selfhost files. Final 8 three-segment type paths (`token::common.Span` × 6, `codegen::codegen_ctx.CodegenError` × 2) resolved by extending both compilers' type-path parsers to accept `.lowercase` continuation when the previous segment is also lowercase (module chain heuristic). Zero `::` path separators remain in `.blood` files (only turbofish `::<T>` syntax).

**Verification**: 344/344 ground-truth (342 pass + 2 pre-existing first_gen COMPILE_FAIL limitations), second_gen/third_gen byte-identical (13,079,144 bytes), 1269/1269 bootstrap lib tests.

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

### ~~DEF-010: `dyn Trait` vtable `drop_fn` conflicts with Blood's memory model~~ ✓

| Field | Value |
|-------|-------|
| **Discovered** | Rust-ism audit (2026-03-01) |
| **Resolved** | 2026-03-01 |
| **Severity** | High — architectural conflict in spec |

**Resolution**: Removed `drop_fn` from vtable layout in DISPATCH.md §10.8.1. Blood has no `Drop` trait and does not call per-value destructors. Memory cleanup is handled by the tier system (region deallocation, ref-counting, `finally` clauses). Added design note explaining the rationale. Resource cleanup for `dyn Trait` values is the responsibility of enclosing effect handler `finally` clauses, consistent with Blood's explicit resource management model.

---

### ~~DEF-011: `Send` trait used in spec but never defined~~ ✓

| Field | Value |
|-------|-------|
| **Discovered** | Rust-ism audit (2026-03-01) |
| **Resolved** | 2026-03-01 |
| **Severity** | High — undefined symbol in a formal soundness proof |

**Resolution**: Restored `Send` as an auto-derived marker trait that **cannot be manually implemented**. Derivation is purely from memory tier (Tier 1 stack: yes; Tier 2 region mutable: no; Tier 2 Frozen: yes; Tier 3 persistent: yes; linear: yes via transfer; raw pointers: no). The original DEF-011 resolution (remove traits entirely, compiler checks at spawn call sites) was revised because generic code (`fn foo<T>` that transitively spawns) cannot express fiber-transferability constraints without a named trait bound. The key insight from DEF-011 was preserved: `Send` is structural and unforgeable — no `unsafe impl Send` exists in Blood. `Sync` was removed (tier system handles sharing). Updated GRAMMAR.md §3.4.1, CONCURRENCY.md §2.4/§8.1, SPECIFICATION.md, STDLIB.md §5.9, and DECISIONS.md ADR-036 Sub-5.

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

### ~~DEF-014: `dyn Trait` vs effects for heterogeneous collections~~ ✓

| Field | Value |
|-------|-------|
| **Discovered** | Rust-ism audit (2026-03-01) |
| **Resolved** | 2026-03-04 |
| **Severity** | Medium — design space question |

**Resolution**: Evaluated whether Blood's effects + handlers + fibers can replace `dyn Trait` vtables. **Verdict: effects and vtables are complementary, not competing — both are needed.**

Effects dispatch operations through the call stack (behavioral polymorphism). Vtables dispatch methods through data pointers (data polymorphism). You cannot put an effect in a `Vec` — effects flow through continuations and the call stack, not as first-class data values. Heterogeneous collections require first-class data values with uniform representation.

Options evaluated:
- **Option A (effects only)**: Rejected — cannot store heterogeneous data in collections
- **Option B (fingerprint dispatch)**: Deferred indefinitely — YAGNI, vtables simpler for single-receiver case
- **Option C (`dyn Trait` vtables)**: Recommended — already designed in DISPATCH.md §10.7-10.8, composes with Blood's effects/tiers/linearity
- **Option D (enum dispatch)**: Already works for closed sets — zero overhead, exhaustive

Blood's hybrid dispatch hierarchy (priority order):
1. Enum + match (closed sets) — zero-cost, compile-time checked
2. Static multiple dispatch (types known) — monomorphized, zero overhead
3. Effect handlers (behavioral polymorphism) — middleware, interceptors, resources
4. `dyn Trait` vtables (data polymorphism) — heterogeneous collections, plugins
5. Fingerprint dispatch — deferred indefinitely

Blood-specific advantages: no `drop_fn` in vtable (cleanup via tiers + finally), generation checks on data pointer only (vtable is static), effect rows on trait methods, content-addressed vtable deduplication.

See `docs/design/DYN_TRAIT_EVALUATION.md` for the full evaluation.

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
| DEF-005 | dyn Trait | Medium | Phase C | Active — **UNBLOCKED** (DEF-014 resolved) |
| DEF-006 | `in` containment | Low | Phase C | Active |
| ~~DEF-007~~ | ~~Async → Fiber~~ | ~~High~~ | ~~Sub-phase~~ | **RESOLVED** (2026-03-01) |
| DEF-008 | Continuation tokens | Low | Phase C | Active |
| DEF-009 | Cast + linearity/regions | Medium | Spec addendum | Active |
| ~~DEF-010~~ | ~~`dyn Trait` vtable `drop_fn`~~ | ~~High~~ | ~~Pre-DEF-005~~ | **RESOLVED** (2026-03-01) |
| ~~DEF-011~~ | ~~`Send` undefined in Blood~~ | ~~High~~ | ~~Spec addendum~~ | **RESOLVED** (2026-03-01) |
| DEF-012 | `&T`/`&mut T` misleading | Low | Long-term | Active |
| DEF-013 | `derive` Rust trait names | Low | Stdlib design | Active |
| ~~DEF-014~~ | ~~`dyn Trait` vs effects eval~~ | ~~Medium~~ | ~~Pre-DEF-005~~ | **RESOLVED** (2026-03-04) |
| DEF-015 | Result/Option × effects | Medium | Ecosystem guidelines | Active |

**Active: 9 items.** Resolved: 5 (DEF-001, DEF-007, DEF-010, DEF-011, DEF-014).
**High severity (0)**: All high-severity items resolved.
**Medium severity (5)**: DEF-002–005, DEF-009, DEF-015.
**Low severity (4)**: DEF-006, DEF-008, DEF-012, DEF-013.
