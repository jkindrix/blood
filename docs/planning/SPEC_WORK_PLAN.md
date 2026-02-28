# Specification Work Plan

**Version**: 3.1
**Established**: 2026-02-28
**Last Updated**: 2026-02-28
**Status**: Active

This document captures the remaining work to resolve open design questions, approve early-impact proposals, bring Blood's specifications and compilers into full alignment, complete formal verification, and close all known gaps.

---

## Table of Contents

1. [Context](#1-context)
2. [Spec Maturity Summary](#2-spec-maturity-summary)
3. [Phase 0: Design Space Resolution](#3-phase-0-design-space-resolution)
   - [0.1 Architectural Findings](#01--architectural-findings)
   - [0.2 Proposal Triage and Approval](#02--proposal-triage-and-approval)
   - [0.3 Grammar Update (v0.5.0)](#03--grammar-update-v050)
   - [0.4 Remaining Design Gaps](#04--remaining-design-gaps)
   - [0.5 Minimal-Effort Defaults](#05--minimal-effort-defaults)
   - [0.6 Inherited Decision Confirmations](#06--inherited-decision-confirmations)
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

- **GRAMMAR.md** (v0.6.0) is settled — the source of truth for surface syntax. Only procedural macros remain deferred (legitimate: semantic design must precede syntax).
- **FORMAL_SEMANTICS.md** (v0.4.0) now formalizes closures, regions, pattern matching, casts, and associated types. A scope statement explicitly lists what is and isn't formalized.
- **DISPATCH.md** (v0.4.0) now includes object safety rules and dyn Trait dispatch semantics.
- **CONTENT_ADDRESSED.md** (v0.4.0) now specifies monomorphized instance hashing (three-level cache model, ADR-030).
- **SPECIFICATION.md** (v0.3.0) body is current and comprehensive. Dead links fixed, MACROS.md added to hierarchy.

All 337/337 ground-truth tests pass. Bootstrap is stable (second_gen/third_gen byte-identical). The compilers work correctly but use **old syntax** in places where GRAMMAR.md has evolved.

**However**, two factors require resolution before compiler alignment:

1. **Design space audit** ([DESIGN_SPACE_AUDIT.md](../design/DESIGN_SPACE_AUDIT.md)) identified 28 accidental defaults and several architectural tensions.

2. **Proposal layer** ([PROPOSAL_ANALYSIS.md](../proposals/PROPOSAL_ANALYSIS.md)) contains 26 researched-but-uncommitted design decisions. Several proposals **change the grammar** — if approved after alignment, every `.blood` file would need to be updated twice. Approving grammar-affecting proposals **before** alignment saves enormous rework.

---

## 2. Spec Maturity Summary

| Document | Version | Status | Gaps |
|----------|---------|--------|------|
| GRAMMAR.md | **0.6.0** | Tier 1 proposals + `FinallyClause` (ADR-031, ADR-036) | Procedural macros deferred |
| FORMAL_SEMANTICS.md | 0.4.0 | Core features formalized | Coq mechanization incomplete (§7); may need updates from approved proposals |
| DISPATCH.md | 0.4.0 | Complete | None |
| MEMORY_MODEL.md | 0.3.0 | Tiers 0/1 solid | Tier 3 designed but not implemented |
| CONCURRENCY.md | **0.4.0** | Cohesive model (ADR-036) | None — all sub-decisions resolved |
| MACROS.md | 0.1.0 | Syntax/expansion covered | Hygiene deferred (compiler semantics, not grammar) |
| SPECIFICATION.md | 0.3.0 | Current | None |
| FFI.md | 0.4.0 | Complete | None |
| CONTENT_ADDRESSED.md | 0.4.0 | Updated with monomorphized instance hashing | ADR-030 resolved F-01 tension |
| DIAGNOSTICS.md | 0.4.0 | Complete | None |

---

## 3. Phase 0: Design Space Resolution

**Priority**: Highest — unresolved design questions and unapproved proposals could change syntax, semantics, and compiler architecture. Alignment work is premature until these are settled.

### Phase 0 Methodology: Design-First Principle

> **CRITICAL: All Phase 0 decisions are made from first principles, independent of existing implementations.**
>
> Phase 0 resolves what Blood's design *should be* — not what it currently *is*. The existing compilers (`src/bootstrap/`, `src/selfhost/`), runtime (`blood-runtime/`), and stdlib (`stdlib/`) are **irrelevant** to Phase 0 reasoning. They must NOT be used to constrain, validate, or shortcut design decisions.
>
> **Correct reasoning:** "Given Blood's goals (effects-based, memory-safe, content-addressed), what is the best concurrency model? What does the research say? What do Blood's design principles require?"
>
> **Incorrect reasoning:** "The stdlib already implements nurseries with `Send` bounds, so we should codify that." ← This is implementation-constrained thinking. The existing implementation may be wrong, incomplete, or suboptimal. The design must be evaluated independently.
>
> Existing implementations become relevant only in **Phase A and later**, when we align compilers and runtime to match the settled design. If the design contradicts the implementation, the implementation changes — not the design.
>
> This principle applies to all Phase 0 work: architectural findings (F-06, F-07), proposal triage (Tier 1-3), grammar updates, design gap resolutions, defaults, and inherited decision confirmations.

**Inputs**:
- [DESIGN_SPACE_AUDIT.md](../design/DESIGN_SPACE_AUDIT.md) — 122 design axes evaluated, 10 findings
- [PROPOSAL_ANALYSIS.md](../proposals/PROPOSAL_ANALYSIS.md) — 26 proposals with critical path analysis
- [EXTRAORDINARY_FEATURES.md](../proposals/EXTRAORDINARY_FEATURES.md) (I, II, III) — proposal details
- [SAFETY_LEVELS.md](../proposals/SAFETY_LEVELS.md) — granular safety controls RFC
- [SYNTAX_REDESIGN.md](../proposals/SYNTAX_REDESIGN.md) — AI-native syntax decisions
- [Designing a Programming Language](~/references/language-design/designing-a-programming-language.md) — practitioner's reference for language design decisions

### 0.1 — Architectural Findings

These could change what we're building. Each requires an ADR or design document.

| # | Finding | Severity | Status | Deliverable |
|---|---------|----------|--------|-------------|
| F-01 | Monomorphization × Content Addressing | Architectural | **RESOLVED** (ADR-030) | Two-level content-addressed cache |
| F-06 | Concurrency Model | Architectural | **RESOLVED** (ADR-036) | Effect-based structured concurrency; CONCURRENCY.md v0.4.0 |
| F-07 | Compiler-as-a-Library | Architectural | **RESOLVED** (ADR-037) | Content-hash-gated query architecture |

#### F-06: Concurrency Model

**Design question:** What is the best language-level concurrency model for Blood, given its effect system, region-based memory, linear types, and content-addressed compilation?

This is a first-principles design problem. Eight sub-decisions must be resolved by reasoning from Blood's goals, its unique features, and the state of the art in language design — NOT from what currently exists in any compiler or runtime.

| Sub-decision | Status | Design question |
|-------------|--------|-----------------|
| Structured concurrency (task scoping) | Defaulted | How should task lifetimes relate to effect handler scopes? |
| Cancellation mechanism | Deferred (DECISIONS.md) | Should cancellation be an effect, a scope property, handler non-resumption, or something else? |
| Cancellation safety guarantees | Defaulted | What invariants does the language guarantee when a concurrent task is cancelled? |
| Async drop / cleanup | Defaulted | How do linear types, regions, and effect handlers interact with task cancellation and resource cleanup? |
| Thread-safety markers (Send/Sync) | Defaulted | Should cross-fiber safety be modeled as traits, effects, capabilities, or something else? What fits Blood's effect-first philosophy? |
| Async iterators / streams | Defaulted | What is the right abstraction for asynchronous sequences in an effect-based language? |
| Runtime-provided vs. library concurrency | Defaulted | What belongs in the language runtime vs. what can be expressed as library-level effect handlers? |
| Fiber ↔ OS thread interaction | Defaulted | How should the language expose (or hide) the mapping between fibers and OS threads? |

**Methodology**: Research effect-based concurrency (Koka, OCaml 5/Eio, Effekt), structured concurrency (Trio, Kotlin, Swift, Java Loom), and the academic literature on cancellation safety and async cleanup. Evaluate each sub-decision against Blood's design principles. Propose the best design regardless of implementation cost.

**Deliverable**: Design document (ADR + CONCURRENCY.md update) specifying the cohesive concurrency model. All 8 sub-decisions resolved with rationale.

**Impact if answer changes**: Could add new syntax (GRAMMAR.md), new effects (FORMAL_SEMANTICS.md), new runtime contracts (CONCURRENCY.md), new typing rules, and new stdlib abstractions.

#### F-07: Compiler-as-a-Library

**Design question:** What internal architecture should Blood's compiler adopt to support query-based compilation, content-addressed caching, and use as a library by external tools?

This is an architectural design question about what the compiler *should* look like, independent of its current structure. The deliverable is an architectural note that identifies query boundaries, cache invalidation strategies, and API surfaces — not an implementation plan.

| Design axis | Question |
|------------|----------|
| Query granularity | Per-file, per-definition, or per-expression? |
| Cache integration | How does BLAKE3 content addressing map to query invalidation? |
| API boundaries | What queries should external consumers (LSP, verification tools, AI oracles) be able to invoke? |
| Incremental strategy | Signature/body split? Fingerprint-based cascading prevention? |
| Phased adoption | Can query architecture be adopted incrementally without a full rewrite? |

**Methodology**: Research query-based compiler architecture (Salsa/rust-analyzer, Roslyn, rustc's query system, Sixten). Evaluate how Blood's content-addressed compilation model creates natural query boundaries.

**Deliverable**: Architectural note (ADR) specifying query boundaries, invalidation strategy, and API surface. No implementation required.

**Impact if answer changes**: Constrains compiler internal API boundaries, affects how CCV clusters are organized in future phases, and determines feasibility of Tier 3 proposals (#16, #18, #19).

### 0.2 — Proposal Triage and Approval

**Rationale**: 26 proposals are at "Proposed" status. Several change the grammar. If approved after alignment (Phase A), every `.blood` file would need updating twice. The cost of approving grammar-affecting proposals **now** vs. **after alignment** is the difference between aligning once and aligning twice.

Proposals are evaluated in three tiers based on their impact on the alignment pass.

#### Tier 1: Grammar-Affecting — **ALL APPROVED** (ADR-031)

All six Tier 1 proposals were evaluated and unanimously approved. GRAMMAR.md has been updated to v0.5.0.

| # | Proposal | Source | Status | GRAMMAR.md |
|---|----------|--------|--------|------------|
| **#20** | Spec annotations (`requires`/`ensures`/`invariant`/`decreases`) | EF_III, SYNTAX_REDESIGN | **APPROVED** | Already present (v0.4.0 `SpecClause`) |
| **—** | Optional semicolons | SYNTAX_REDESIGN C.1 | **APPROVED** | §5.2.1 — continuation rules added |
| **—** | Function signature ordering | SYNTAX_REDESIGN B.1 | **APPROVED** | §3.2 — canonical ordering formalized |
| **#21a** | Named arguments | EF_III, SYNTAX_REDESIGN C.2 | **APPROVED** | Already present (v0.4.0 `Arg` production) |
| **#21b** | Expression-oriented design | EF_III #21 | **APPROVED** | §5.2.2 — design note added |
| **RFC-S** | Granular safety controls | SAFETY_LEVELS.md | **APPROVED** | §5.4 — `UncheckedBlock` + `#[unchecked(...)]` |

#### Tier 2: Architecture-Affecting — **ALL APPROVED** (ADR-032)

All five Tier 2 proposals approved as committed design direction. No grammar changes required.

| # | Proposal | Source | Status | Implementation |
|---|----------|--------|--------|---------------|
| **#17** | Structured diagnostics (dual human/machine) | EF_III | **APPROVED** | Compiler internal (incremental) |
| **#8** | Deterministic simulation testing (DST) | EF_II | **APPROVED** | Library/stdlib pattern |
| **#12** | Deterministic replay debugging | EF_II | **APPROVED** | Tooling (recording runtime) |
| **#13** | Observability (zero-code via effect wrapping) | EF_II | **APPROVED** | Library/stdlib pattern |
| **#11** | Semantic versioning (automatic via content hashes) | EF_II | **APPROVED** | Tooling (`blood semver`) |

#### Tier 3: Deferred (deep infrastructure dependencies)

These depend on infrastructure that doesn't exist yet (incremental type checker, SMT integration, query-based compiler). Approving them prematurely creates commitment without implementation evidence.

| # | Proposal | Source | Blocked By |
|---|----------|--------|-----------|
| #7 | Graduated verification (4 levels) | EF_I, EF_III | #20 (spec annotations — approved in Tier 1 if accepted) |
| #18 | Verification cache | EF_III | #7 + content addressing |
| #16 | Constrained decoding oracle | EF_III | F-07 (compiler-as-a-library), incremental type checker |
| #10 | Proof-carrying code | EF_II | #7 + #18 (verification pipeline) |
| #9 | Taint tracking / information flow | EF_II | Effects infrastructure (exists) |
| #4 | Capability security | EF_I | Effects infrastructure (exists) |
| #1 | WCET analysis | EF_I | #20 + verification infrastructure |
| #2 | Session types | EF_I, EF_II | New `protocol` keyword (grammar change) |
| #14 | Choreographic programming | EF_II | #2 (session types) |
| #15 | Complexity bounds | EF_II | #7 (verification), purity analysis |
| #3 | Automatic memoization | EF_I | Content addressing + purity |
| #5 | Auto-parallelization | EF_I | Purity analysis + runtime |
| #6 | Provenance tracking | EF_I | Effects infrastructure |
| #19 | Module signatures for AI | EF_III | Compiler-as-a-library (F-07) |
| #22 | Dependency graph API | EF_III | Full compilation model |
| #23 | Effect handlers as agent middleware | EF_III | Effects (exists) + tooling |

**Decision**: Acknowledge as design direction. Do not commit to implementation timeline. Revisit when blocking dependencies are resolved.

### 0.3 — Grammar Update (v0.5.0) — **COMPLETE**

GRAMMAR.md has been updated to v0.5.0 incorporating all six approved Tier 1 proposals:
- §3.2: Canonical signature ordering formalized
- §5.2.1: Semicolon optionality with `ContinuationToken` rules
- §5.2.2: Expression-oriented design note
- §5.4: `UncheckedBlock` production, `@unsafe` vs `unchecked` distinction
- §1.5.1: `#[unchecked(...)]` and `#![default_unchecked(...)]` standard attributes
- §9.2: `unchecked` added to contextual keywords

**Remaining**: FORMAL_SEMANTICS.md may need typing rules for `UncheckedBlock` and named argument resolution. These are non-blocking for alignment.

**Deliverable**: GRAMMAR.md v0.5.0 — the final pre-alignment grammar. ✓

### 0.4 — Remaining Design Gaps — **COMPLETE** (ADR-033)

All 7 design gaps resolved:

| # | Finding | Decision |
|---|---------|----------|
| F-02 | Higher-kinded types | **Not planned** — row poly + effects + dispatch cover use cases |
| F-03 | Variance | **Invariant by default**, compiler-inferred where safe |
| F-04 | String representation | **`{ ptr, i64 }` (16 bytes)** — gen checks at creation, not access |
| F-05 | Result/Option × Effects | **Complementary** — Result for leaf, effects for orchestration |
| F-08 | Stdlib scope | **Three-tier** — core (freestanding) / alloc / std (OS) |
| F-09 | Testing | **First-class** — `#[test]` + `blood test` + effect-based DST |
| F-10 | ABI stability | **Explicitly unstable** — content hashes replace ABI guarantees |

### 0.5 — Minimal-Effort Defaults — **COMPLETE** (ADR-034)

All 7 defaults documented:

1. **Cyclic imports**: Forbidden (content-addressed DAG)
2. **Interior mutability**: Deferred (no concrete use case yet)
3. **Dead code detection**: Yes, compiler warning via MIR analysis
4. **Definite initialization**: Statically enforced via MIR
5. **Doc comment syntax**: `///` (already in grammar)
6. **Frame pointer preservation**: On by default
7. **Variance**: Invariant by default (= F-03)

### 0.6 — Inherited Decision Confirmations — **COMPLETE** (ADR-035)

All 9 inherited decisions evaluated:

| Decision | Verdict |
|----------|---------|
| Monomorphization | **Already resolved** (ADR-030) |
| `Option<T>` / `Result<T, E>` | **Confirmed** with guidance (F-05) |
| UTF-8 strings | **Confirmed** (F-04) |
| File-based module hierarchy | **Confirmed** — files are authoring UI, hashes are identity |
| `pub` visibility | **Confirmed** — orthogonal to row polymorphism |
| Call-by-value evaluation | **Confirmed** — natural for effects |
| No runtime type information | **Revised** — minimal RTTI (type fingerprints for dispatch), no reflection |
| `&T` / `&mut T` syntax | **Confirmed** — same syntax, generational semantics |
| Binary `unsafe` blocks | **Superseded** by ADR-031 (granular `unchecked`) |

### Phase 0 Exit Criteria

Phase 0 is complete when:

**Architectural:**
- [x] F-01 ADR written and accepted (ADR-030, 2026-02-28)
- [x] F-06 concurrency model design document written and accepted (ADR-036, 2026-02-28)
- [x] F-07 compiler-as-a-library architectural note written (ADR-037, 2026-02-28)

**Proposals:**
- [x] All Tier 1 proposals evaluated: **all 6 approved** (ADR-031, 2026-02-28)
- [x] All Tier 2 proposals evaluated: **all 5 approved** (ADR-032, 2026-02-28)
- [x] Tier 3 proposals acknowledged with dependency map (documented in §0.2, 2026-02-28)

**Grammar:**
- [x] GRAMMAR.md updated to v0.5.0 incorporating all approved Tier 1 proposals (2026-02-28)
- [x] GRAMMAR.md updated to v0.6.0: `FinallyClause` added to handler syntax (ADR-036, 2026-02-28)
- [x] FORMAL_SEMANTICS.md updated: `finally` clause typing rules added (§6.3, 2026-02-28)

**Design gaps:**
- [x] F-02, F-03, F-04, F-05, F-08, F-09, F-10 resolved (ADR-033, 2026-02-28)
- [x] All 7 minimal-effort defaults documented (ADR-034, 2026-02-28)
- [x] All inherited decisions confirmed or revised (ADR-035, 2026-02-28)

**Coordination:**
- [x] CONCURRENCY.md updated to v0.4.0 with cohesive concurrency model (ADR-036, 2026-02-28)
- [x] SPECIFICATION.md updated: `Async` → `Fiber` throughout, `Cancel` effect added (ADR-036, 2026-02-28)

---

## 4. Phase A: Syntax Alignment

**Priority**: High — blocks compiler alignment.
**Prerequisite**: Phase 0 complete (design is stable, GRAMMAR.md at v0.6.0).
**Method**: CCV (Canary-Cluster-Verify) per DEVELOPMENT.md.

The compilers currently accept old syntax in several places where GRAMMAR.md has evolved. Every `.blood` file in the repository must be audited and updated. Because Tier 1 proposals were resolved in Phase 0, this alignment happens **once** against the **final** grammar.

> **Stale Documents Warning (Phase A)**
>
> - **`docs/planning/SYNTAX_SUPPORT.md`** (v0.5.2, 2026-01-14) — **STALE. Do not use.** References GRAMMAR.md v0.4.0; predates user macros, glob imports, const generics, closures, and all Phase 0 grammar changes (v0.5.0, v0.6.0). Must be regenerated from GRAMMAR.md v0.6.0 before use.
>
> **Current design evaluations (Phase A):**
>
> - [docs/design/IMPL_TRAIT.md](../design/IMPL_TRAIT.md) — Current. Evaluates `impl Trait` against Blood's effect system; conclusion: not planned (effects + dispatch subsume).
> - [docs/design/COMPARISON_CHAINING.md](../design/COMPARISON_CHAINING.md) — Current. Evaluates comparison chaining; conclusion: not planned (use `x in lo..hi`).

### Deferred Items Tracking

**[DEFERRED_ITEMS.md](DEFERRED_ITEMS.md)** tracks every item deferred during Phase A with rationale, severity, downstream impact, and unblocking prerequisites. Must be updated whenever work is deferred. Items are removed only when fully resolved.

### A.1 — Syntax Delta Analysis

Comprehensive diff between GRAMMAR.md v0.6.0 productions and what each parser actually accepts. Covers:

- Imports (grouped, glob, simple) — known `::` vs `.` gap
- Qualified expressions and paths
- Type syntax
- Expression syntax (including any expression-oriented changes from approved proposals)
- Pattern syntax
- Bridge/FFI syntax
- Effect/handler syntax
- Macro syntax
- Spec annotation syntax (if #20 approved — already in v0.4.0)
- Named argument syntax (if #21a approved)
- Safety attribute syntax (if RFC-S approved)

**Inputs**: GRAMMAR.md v0.6.0, `src/bootstrap/bloodc/src/parser/`, `src/bootstrap/bloodc/src/lexer.rs`, `src/selfhost/parser_*.blood`, `src/selfhost/lexer.blood`, `src/selfhost/token.blood`

**Output**: Complete list of deltas with severity (breaking vs cosmetic).

### A.2 — Update Bootstrap Compiler (Rust) — **IN PROGRESS**

The bootstrap compiler defines language semantics. It must accept the spec syntax **first**.

- Update lexer/parser in `src/bootstrap/bloodc/src/` to match GRAMMAR.md v0.6.0
- Rebuild: `cd src/bootstrap && cargo build --release`
- Verify: `cargo test --workspace` (unit tests must pass)

**This is a Bootstrap Gate prerequisite** — nothing else moves until this is done.

**Status (2026-02-28): COMPLETE**
- [x] A.2.1: Import paths and type paths accept both `::` and `.`
- [x] A.2.2: `FinallyClause` parsed in handler bodies (`finally_clause: Option<Block>` on `HandlerDecl`)
- [x] A.2.3: `finally` reclassified as contextual keyword
- [x] A.2.4: Release build passes, 1269 lib tests pass
- [x] DEF-001: Expression paths resolved via name-resolution fallback (`try_extract_module_chain` + `try_resolve_qualified_chain`). Commits: `ff5ac38`, `4bc7ca1`

**Residual**: 8 three-segment type paths (`token::common.Span` × 6, `codegen::codegen_ctx.CodegenError` × 2) still use `::` for the first separator. Requires type-path parser to support 3+ dot-separated segments — a minor parser enhancement, not a blocking issue.

### A.3 — CCV Migration of Self-Hosted Compiler — **COMPLETE** (2026-02-28)

All 47 `.blood` files in `src/selfhost/` migrated. Key steps:
1. Import paths (grouped, glob) converted to `.` syntax — canary tests in `tests/fixtures/modules/`
2. DEF-001 resolved in selfhost (`0f60269`): `try_extract_module_chain` + `resolve_qualified_path` fallback
3. Batch conversion of ~1,960 `lowercase::lowercase` expression paths (`f6fbc79`)
4. `FinallyClause` parsing added to selfhost parser (`4797be9`)

**Verification**: 339/339 ground-truth, second_gen/third_gen byte-identical (13,079,128 bytes).

**Residual `::` (8 occurrences)**: Three-segment type paths — `token::common.Span` (lexer.blood × 6), `codegen::codegen_ctx.CodegenError` (driver.blood × 2). Not convertible until type-path parser supports 3+ dot-separated segments.

### A.4 — Update Tests and Examples — **COMPLETE** (2026-02-28)

- [x] `tests/ground-truth/*.blood` — all expression-path `::` converted to `.` (`c8bc323`, `788df5a`)
- [x] `stdlib/*.blood` — converted in earlier commit (`3ea4e4f`)
- [x] All 339/339 tests pass

---

## 5. Phase B: Semantic Alignment Audit

**Priority**: High — cheap analysis, high information value.
**Prerequisite**: Phase A complete (syntax aligned).

> **Stale Documents Warning (Phase B)**
>
> - **`docs/planning/IMPLEMENTATION_STATUS.md`** (v0.5.3, 2026-01-29) — **STALE. Do not use.** Missing all 337 ground-truth tests, closures, array-to-slice coercion, MIR region fixes, self-hosted compiler progress, and ADR-001 through ADR-037. Must be regenerated from current compiler state before use.

Audit whether compiler behavior matches the formal semantics we've specified:

| # | Check | Spec Section | Method |
|---|-------|-------------|--------|
| # | Check | Spec Section | Result |
|---|-------|-------------|--------|
| B.1 | Closure capture modes match §5.7 | FORMAL_SEMANTICS.md §5.7 | **4 gaps** — linear capture not enforced, binary capture mode, no move validation, no effect composition tests |
| B.2 | Region generation bumping matches §5.8 | FORMAL_SEMANTICS.md §5.8 | **3 gaps** — MIR lifecycle stub, no generation checks at dereference, no region-effect interaction |
| B.3 | Object safety enforced per §10.7 | DISPATCH.md §10.7 | **Expected gap** — not implemented (DEF-005) |
| B.4 | dyn Trait vtable layout matches §10.8 | DISPATCH.md §10.8 | **Expected gap** — not implemented (DEF-005) |
| B.5 | Pattern exhaustiveness matches §5.9 | FORMAL_SEMANTICS.md §5.9 | **1 gap** — or-pattern binding consistency not enforced |
| B.6 | Cast compatibility matches §5.10 | FORMAL_SEMANTICS.md §5.10 | **1 critical gap** — typeck has zero cast validation; any cast accepted |
| B.7 | Associated type resolution matches §5.11 | FORMAL_SEMANTICS.md §5.11 | **Minor gaps** — defaults untested, qualified projection unimplemented |

### Phase B Findings (2026-02-28)

**Critical gaps requiring Phase C fixes (ordered by safety impact):**

1. **Cast validation absent** (B.6) — `infer_cast()` accepts any type→type cast without checking `cast_compatible(S, T)`. Pointer↔integer casts not gated by `@unsafe`. Files: `typeck/context/expr.rs:9074`, `mir_lower_expr.blood:1421`.

2. **Linear closure capture not enforced** (B.1) — Linear values can be captured by ref/mut (spec says error). Closures with linear captures don't become linear. Bootstrap uses binary `by_move: bool` instead of ternary capture modes. Files: `typeck/context/closure.rs`, `mir_closure.blood`.

3. **Region MIR lifecycle stub** (B.2) — `mir_lower_expr.blood:263` just lowers body statements; no `region_create/activate/deactivate/destroy` calls emitted. No generation checks at dereference. Stale reference detection specified but unimplemented. Files: `mir_lower_expr.blood`, codegen.

4. **Or-pattern binding consistency** (B.5) — Both branches of `p1 | p2` must bind identical variables (spec rule [P-Or]). Not enforced. Files: `hir_lower_expr.blood:1619`, `typeck.blood`.

**Working correctly:** Exhaustiveness checking, unreachable patterns, associated type declaration/error, region type transparency, all cast codegen, closure basic functionality.

---

## 6. Phase C: Semantic Alignment Fixes

**Priority**: High — addresses Phase B findings.
**Prerequisite**: Phase B complete.

### C.1 — Cast Validation (B.6)

Add `cast_compatible(source, target)` check in `infer_cast()` for both compilers:
- Validate against spec cast table (numeric widening/narrowing, int↔float, sign reinterpret, bool↔int, ptr↔usize)
- Gate ptr↔int and ptr→ptr casts behind `@unsafe` / bridge context
- Reject incompatible casts (struct→array, etc.)
- Add ground-truth tests for all cast categories + COMPILE_FAIL tests for invalid casts

### C.2 — Linear Closure Captures (B.1)

Enforce spec rules [Linear-No-Ref], [Linear-No-Mut], [Linear-Closure]:
- After capture analysis, check each capture's type for linearity
- If linear + ref/mut capture → type error
- If linear + val capture → mark closure as linear
- Upgrade bootstrap from binary to ternary capture mode
- Add COMPILE_FAIL tests for linear ref/mut captures

### C.3 — Region Lifecycle (B.2)

Complete region MIR lowering:
- Emit `region_create/activate/deactivate/destroy` in MIR
- Add generation checks at region pointer dereference
- Implement stale reference detection via `blood_validate_generation()`
- Add tests for stale reference after region exit
- (Advanced) Region-effect interaction / deferred deallocation

### C.4 — Or-Pattern Binding Consistency (B.5)

Enforce [P-Or] rule:
- After lowering both branches of an or-pattern, verify binding environments are identical
- Add `TypeErrorKind::OrPatternBindingMismatch` error
- Add COMPILE_FAIL test for inconsistent bindings

### C.5 — Associated Type Defaults & Projection (B.7)

- Add ground-truth test for default associated types
- Add test for qualified projection `<T as Trait>::Item` (if implemented)
- Verify default fallback works when impl omits associated type

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
Phase 0.1  ──►  Phase 0.2  ──►  Phase 0.3  ──►  Phase 0.4-0.6  ──►  Phase A
(arch           (proposal        (grammar         (gaps, defaults,     (syntax
 findings)       triage)          v0.5.0)          inherited)           alignment
                                                                        — ONCE)

Phase A  ──►  Phase B  ──►  Phase C  ──►  Phase D
(syntax)      (semantic      (semantic      (Coq proofs)
               audit)         fixes)
                                            ──► Phase E ──► Phase F
                                                (Tier 3)    (benchmarks)
```

**Why proposals (0.2) come before grammar update (0.3):**

Proposals #20 (spec annotations), #21a (named arguments), #21b (expression-oriented), and RFC-S (safety controls) all change the grammar. If approved after Phase A alignment, every `.blood` file would need updating twice — once for v0.4.0 alignment, once for v0.5.0 additions. Evaluating these proposals now and incorporating approved ones into v0.5.0 means alignment happens **once** against the **final** grammar.

**Why Phase 0 is split into sub-phases:**

- **0.1 (architectural)** resolves tensions that could invalidate everything downstream (F-06 concurrency could add syntax; F-07 compiler-as-library constrains implementation).
- **0.2 (proposals)** evaluates what goes into the grammar. This depends on 0.1 because F-06 concurrency decisions may affect proposal feasibility.
- **0.3 (grammar update)** is mechanical: write the productions for whatever was approved.
- **0.4-0.6 (gaps/defaults/inherited)** are independent of grammar and can be resolved in parallel or after 0.3.

**Why Phase 0 uses design-first methodology:**

Phase 0 decides what Blood *should be*. Phases A-F make it *actually be that*. If Phase 0 reasoning is constrained by existing implementations, it degenerates into documenting the status quo rather than designing the best language. The compilers and runtime are prototypes that informed the design space — they are not the design itself. Design decisions that contradict existing implementations are not bugs; they are signals that the implementation needs to change in Phase A or later.

**Why alignment (Phase A) happens only once:**

Previous plan versions had alignment against v0.4.0 with a risk of grammar re-revision. The v3.0 plan eliminates this risk by settling the grammar completely before alignment begins. The CCV cost of a full alignment pass is high (~65 files, 9 clusters, 337 tests × 3 verification steps per cluster). Doing it twice would be prohibitive.

**Why Phases B-F are unchanged:**

Semantic audit, Coq proofs, Tier 3, and benchmarks are not affected by the proposal triage. They depend on stable specs and aligned compilers, which Phases 0 and A provide.

---

## 11. Decisions Made

The following semantic decisions were made during the 2026-02-28 specification sessions, derived from Blood's design philosophy documents (SPECIFICATION.md, DECISIONS.md, MEMORY_MODEL.md, DISPATCH.md, FORMAL_SEMANTICS.md):

### ADR-030: Monomorphization × Content Addressing (F-01)

Resolved via two-level content-addressed cache. Monomorphization retained as primary strategy (zero-cost abstraction for generics). Instance hashes use `BLAKE3(def_hash ‖ type_arg_hashes)` — no DefId in hash, enabling cross-project artifact sharing. Dictionary passing evaluated and rejected (5-15% runtime overhead conflicts with ADR-010 priority hierarchy). See DECISIONS.md ADR-030 and CONTENT_ADDRESSED.md §4.6.

### Closure Typing (FORMAL_SEMANTICS.md §5.7)

| Rust Concept | Blood Replacement | Derived From |
|---|---|---|
| `Fn` (shared access) | `fn(T) -> U` with no mutation effects | ADR-002: effects track mutation |
| `FnMut` (mutable access) | `fn(T) -> U / {State<S>}` | ADR-002: mutation is an effect |
| `FnOnce` (consumed) | `linear fn(T) -> U` | ADR-006: linear types = exactly-once |

### Region Typing (FORMAL_SEMANTICS.md §5.8)

- **No type-level region annotations** — would re-introduce borrow checking (violates ADR-001)
- **Safety via generations**: Region exit bumps generations; stale references detected at runtime
- **Invisible to type system**: `region { e }` has same type as `e`

### Object Safety (DISPATCH.md §10.7)

- **ABI constraints**, not arbitrary restrictions
- **Four rules**: No generic methods on Self, no Self by value, no Self return, associated types must be determinable

### Other Settled Decisions

- **impl Trait return-position**: Not planned (effects + universal `fn` type replace)
- **Labeled blocks**: Not planned (effects subsume non-local control flow)
- **T: 'a lifetime bounds**: Not planned (ADR-001 rejected borrow checker)
- **Fn/FnMut/FnOnce traits**: Not planned (effects + linear types + row polymorphism replace)
- **union keyword**: Bridge FFI only; Blood uses enums (tagged unions)

---

## 12. Design Space Audit Reference

The full design space audit is at [docs/design/DESIGN_SPACE_AUDIT.md](../design/DESIGN_SPACE_AUDIT.md). Updated statistics (v1.1, including proposals):

| Category | Count | Percentage |
|----------|-------|------------|
| Consciously Decided | 42 | 34% |
| Proposed (researched, not committed) | 26 | 21% |
| Inherited from Rust | 18 | 15% |
| Accidentally Defaulted | 28 | 23% |
| Explicitly Deferred | 8 | 7% |

**Decided + Proposed = 55%** of the design space is covered. The proposal triage in Phase 0.2 will move approved proposals from "Proposed" to "Decided", further increasing coverage.

**Top findings by severity:**

1. **F-01**: Monomorphization × content addressing — **RESOLVED** (ADR-030)
2. **F-06**: Concurrency model — **RESOLVED** (ADR-036); effect-based structured concurrency
3. **F-07**: Compiler-as-a-library — **RESOLVED** (ADR-037); content-hash-gated query architecture

**Proposal critical path** (from PROPOSAL_ANALYSIS.md):

```
#20 (Spec Annotations) → #7 (Verification) → #18 (Cache) → #10 (Proof-Carrying Code)
```

Tier 1 triage (Phase 0.2) evaluates #20 and the other grammar-affecting proposals. The rest of the critical path (#7→#18→#10) is in Tier 3 (deferred until infrastructure exists).

---

## Work Item Counts

| Phase | Items | Nature |
|-------|-------|--------|
| 0.1: Architectural findings | 3 resolved (F-01, F-06, F-07) | Design documents |
| 0.2: Proposal triage | 6 Tier 1 + 5 Tier 2 + 16 Tier 3 = **27** evaluations | Decision records |
| 0.3: Grammar update | 1 (GRAMMAR.md v0.5.0) | Specification |
| 0.4: Design gaps | 7 findings (F-02–F-10, minus F-01) | ADRs / design notes |
| 0.5: Minimal defaults | 7 | One-paragraph decisions |
| 0.6: Inherited confirmations | 8 (minus monomorphization, already done) | Brief ADRs |
| A: Syntax alignment | 4 major steps, 9 CCV clusters | Implementation |
| B: Semantic audit | 7 checks | Analysis |
| C: Semantic fixes | Unknown (depends on B) | Implementation |
| D: Coq mechanization | 31 items across 7 sub-phases | Formal proofs |
| E: Tier 3 | 8 items | Implementation |
| F: Performance | 6 items | Measurement |
| **Total** | **~108 items** | |

---

## Version History

| Version | Date | Changes |
|---------|------|---------|
| 1.0 | 2026-02-28 | Initial plan: 6 phases (A-F), 56+ items |
| 2.0 | 2026-02-28 | Added Phase 0 (design space resolution) from DESIGN_SPACE_AUDIT.md |
| 3.0 | 2026-02-28 | Restructured Phase 0: added proposal triage (0.2), grammar pre-update (0.3); 26 proposals evaluated in 3 tiers; align-once strategy |
| 3.1 | 2026-02-28 | Added Phase 0 Methodology (design-first principle); reframed F-06/F-07 as design questions; added sequencing rationale for design-first methodology |
