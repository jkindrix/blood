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

## Summary

| ID | Delta | Severity | Target Phase | Status |
|----|-------|----------|-------------|--------|
| ~~DEF-001~~ | ~~Expression path `.`~~ | ~~Medium~~ | ~~Phase A~~ | **RESOLVED** (2026-02-28) |
| DEF-002 | UncheckedBlock | Medium | Phase C | Active |
| DEF-003 | #[unchecked(...)] | Medium | Phase C | Active |
| DEF-004 | @heap/@stack | Medium | Phase C/E | Active |
| DEF-005 | dyn Trait | Medium | Phase C | Active |
| DEF-006 | `in` containment | Low | Phase C | Active |
| DEF-007 | Async → Fiber | High | Sub-phase | Active |
| DEF-008 | Continuation tokens | Low | Phase C | Active |

**Active: 7 items.** Resolved: 1 (DEF-001).
**High severity (1)**: DEF-007 — visible spec/compiler naming mismatch.
**Medium severity (4)**: DEF-002 through DEF-005 — new features, no existing code affected.
**Low severity (2)**: DEF-006, DEF-008 — nice-to-have, non-blocking.
