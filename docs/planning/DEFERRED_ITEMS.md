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

### DEF-001: Expression paths — `lowercase.lowercase` patterns remain `::` only

| Field | Value |
|-------|-------|
| **Delta** | D1 (partial — 93% resolved) |
| **Deferred from** | A.2.1 |
| **Partially resolved** | 2026-02-28 |
| **Severity** | Medium — covers 93% of patterns, remaining 7% can keep `::` |

**What**: Expression paths now accept `.` via a case-based heuristic in `parse_expr_path`:
- **After TypeIdent**: `.` always accepted (TypeIdent is always a type, never a variable)
- **After lowercase ident**: `.` accepted only when followed by TypeIdent

This covers 93% of expression-path `::` occurrences:
- `lowercase.TypeIdent` (11,734): `ast.Attribute`, `common.Span` — WORKS
- `TypeIdent.TypeIdent` (9,546): `Option.Some`, `TokenKind.Hash` — WORKS
- `TypeIdent.lowercase` (3,284): `Vec.new()`, `Point.origin()` — WORKS

**Remaining gap**: `lowercase.lowercase` (1,971 occurrences, 7%) — patterns like `parser_base.parse_string_from_span()`. These are ambiguous with `value.field` and are NOT consumed by the heuristic. They must continue using `::`.

**What unblocks the remaining 7%**: Name resolution fallback — when `infer_path` fails on a multi-segment path, try treating the first segment as a local variable and remaining segments as field access. Or accept the 7% gap and leave `lowercase.lowercase` qualified calls using `::`.

---

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

*None yet.*

---

## Summary

| ID | Delta | Severity | Target Phase | Blocks |
|----|-------|----------|-------------|--------|
| DEF-001 | Expression path `.` (7% gap) | Medium | Partial — 93% resolved | `lowercase.lowercase` calls |
| DEF-002 | UncheckedBlock | Medium | Phase C | Granular safety |
| DEF-003 | #[unchecked(...)] | Medium | Phase C | Per-fn safety |
| DEF-004 | @heap/@stack | Medium | Phase C/E | Allocation placement |
| DEF-005 | dyn Trait | Medium | Phase C | Dynamic dispatch |
| DEF-006 | `in` containment | Low | Phase C | Boolean containment |
| DEF-007 | Async → Fiber | High | Sub-phase | Concurrency naming |
| DEF-008 | Continuation tokens | Low | Phase C | Spec strictness |

**High severity (2)**: DEF-001, DEF-007 — these create visible spec/compiler mismatches.
**Medium severity (4)**: DEF-002 through DEF-005 — new features, no existing code affected.
**Low severity (2)**: DEF-006, DEF-008 — nice-to-have, non-blocking.
