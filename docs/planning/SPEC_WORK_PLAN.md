# Specification Work Plan

**Version**: 3.0
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

- **GRAMMAR.md** (v0.4.0) is settled — the source of truth for surface syntax. Only procedural macros remain deferred (legitimate: semantic design must precede syntax).
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
| GRAMMAR.md | 0.4.0 → **0.5.0** (after Phase 0.3) | Pending proposal incorporation | Procedural macros deferred; concurrency syntax TBD |
| FORMAL_SEMANTICS.md | 0.4.0 | Core features formalized | Coq mechanization incomplete (§7); may need updates from approved proposals |
| DISPATCH.md | 0.4.0 | Complete | None |
| MEMORY_MODEL.md | 0.3.0 | Tiers 0/1 solid | Tier 3 designed but not implemented |
| CONCURRENCY.md | 0.3.0 | Incomplete | Largest design gap (F-06) |
| MACROS.md | 0.1.0 | Syntax/expansion covered | Hygiene deferred (compiler semantics, not grammar) |
| SPECIFICATION.md | 0.3.0 | Current | None |
| FFI.md | 0.4.0 | Complete | None |
| CONTENT_ADDRESSED.md | 0.4.0 | Updated with monomorphized instance hashing | ADR-030 resolved F-01 tension |
| DIAGNOSTICS.md | 0.4.0 | Complete | None |

---

## 3. Phase 0: Design Space Resolution

**Priority**: Highest — unresolved design questions and unapproved proposals could change syntax, semantics, and compiler architecture. Alignment work is premature until these are settled.

**Inputs**:
- [DESIGN_SPACE_AUDIT.md](../design/DESIGN_SPACE_AUDIT.md) — 122 design axes evaluated, 10 findings
- [PROPOSAL_ANALYSIS.md](../proposals/PROPOSAL_ANALYSIS.md) — 26 proposals with critical path analysis
- [EXTRAORDINARY_FEATURES.md](../proposals/EXTRAORDINARY_FEATURES.md) (I, II, III) — proposal details
- [SAFETY_LEVELS.md](../proposals/SAFETY_LEVELS.md) — granular safety controls RFC
- [SYNTAX_REDESIGN.md](../proposals/SYNTAX_REDESIGN.md) — AI-native syntax decisions

### 0.1 — Architectural Findings

These could change what we're building. Each requires an ADR or design document.

| # | Finding | Severity | Status | Deliverable |
|---|---------|----------|--------|-------------|
| F-01 | Monomorphization × Content Addressing | Architectural | **RESOLVED** (ADR-030) | Two-level content-addressed cache |
| F-06 | Concurrency Model | Architectural | Open | Design document: effects + fibers + structured concurrency |
| F-07 | Compiler-as-a-Library | Architectural | Open | Architectural note: query-based API boundaries |

#### F-06: Concurrency Model

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

**Deliverable**: Design document composing effects + fibers + handlers into a cohesive concurrency model.

**Impact if answer changes**: Could add new syntax (GRAMMAR.md), new effects (FORMAL_SEMANTICS.md), new runtime contracts (CONCURRENCY.md), and new typing rules.

#### F-07: Compiler-as-a-Library

The self-hosted compiler is a monolithic pipeline. Content-addressed compilation is naturally query-based, but the compiler doesn't exploit this. Proposals #16 (constrained decoding oracle) and #18 (verification cache) both implicitly assume query-based architecture.

**Deliverable**: Architectural note evaluating query-based internal architecture. This constrains the compiler's internal API boundaries — it does not require immediate implementation.

**Impact if answer changes**: Could restructure self-hosted compiler modules, change how CCV clusters are organized.

### 0.2 — Proposal Triage and Approval

**Rationale**: 26 proposals are at "Proposed" status. Several change the grammar. If approved after alignment (Phase A), every `.blood` file would need updating twice. The cost of approving grammar-affecting proposals **now** vs. **after alignment** is the difference between aligning once and aligning twice.

Proposals are evaluated in three tiers based on their impact on the alignment pass.

#### Tier 1: Grammar-Affecting (evaluate before alignment)

These proposals change surface syntax. If approved, they must be incorporated into GRAMMAR.md before Phase A begins. If rejected, their syntax is excluded and alignment proceeds against v0.4.0.

| # | Proposal | Source | Grammar Change | Already in Grammar? | Risk |
|---|----------|--------|---------------|---------------------|------|
| **#20** | Spec annotations (`requires`/`ensures`/`invariant`/`decreases`) | EF_III, SYNTAX_REDESIGN | New clause syntax on function signatures | **Yes** (SpecClause production, v0.4.0) | Low — additive keywords |
| **—** | Optional semicolons | SYNTAX_REDESIGN C.1 | `Statement ::= ... ';'?` | **Yes** (v0.4.0) | Low — already specified |
| **—** | Function signature ordering | SYNTAX_REDESIGN B.1 | attrs → sig → effects → specs → where → body | Partially | Low — ordering convention |
| **#21a** | Named arguments | EF_III, SYNTAX_REDESIGN C.2 | `f(name: value)` call-site syntax | **No** — new production | Medium — parser change |
| **#21b** | Expression-oriented design | EF_III #21 | Every construct returns a value; blocks are expressions | **No** — semantic change to blocks/if/match | Medium — pervasive change |
| **RFC-S** | Granular safety controls | SAFETY_LEVELS.md | `#[unchecked(check)]` attribute, `unchecked(checks) { }` block | **No** — replaces binary `unsafe` | Low — more expressive replacement |

**Decision required for each**: Approve (→ add to GRAMMAR.md v0.5.0) or Reject (→ not in scope for alignment).

**Evaluation criteria**:
- Does it align with Blood's design philosophy (Five Pillars, Priority Hierarchy)?
- Is it well-researched with clear semantics?
- Does deferring it create technical debt that outweighs the cost of adoption?
- Does it have unresolved dependencies that prevent commitment?

#### Tier 2: Architecture-Affecting (evaluate before alignment)

These don't change the grammar but affect compiler internals, diagnostic output, or tooling contracts. Approving them constrains implementation decisions during Phase A.

| # | Proposal | Source | Impact | Dependencies |
|---|----------|--------|--------|-------------|
| **#17** | Structured diagnostics (dual human/machine) | EF_III | Error codes as public API, JSON output, fix suggestions as structured diffs | None — compiler internal |
| **#8** | Deterministic simulation testing (DST) | EF_II | Library pattern on existing effects — no compiler changes | None — ready now |
| **#12** | Deterministic replay debugging | EF_II | Library pattern on existing effects — no compiler changes | None — ready now |
| **#13** | Observability (zero-code via effect wrapping) | EF_II | Library pattern on existing effects — no compiler changes | None — ready now |
| **#11** | Semantic versioning (automatic via content hashes) | EF_II | Tooling — `blood semver` command | Content addressing (exists) |

**Decision**: Approve as committed design direction or defer. These can be approved at any time without rework cost, but approving now signals design intent and unlocks downstream proposals.

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

### 0.3 — Grammar Update (v0.5.0)

**Prerequisite**: Tier 1 proposal decisions complete.

For each Tier 1 proposal approved in §0.2:
1. Write or update the GRAMMAR.md production rules
2. Add formal typing rules to FORMAL_SEMANTICS.md where applicable
3. Update SPECIFICATION.md hierarchy if new companion documents are created
4. Increment GRAMMAR.md to v0.5.0

This is the **last grammar change before alignment**. Phase A aligns to v0.5.0 and does not revisit.

**Deliverable**: GRAMMAR.md v0.5.0 — the final pre-alignment grammar.

### 0.4 — Remaining Design Gaps

Short ADRs or design notes. These don't change grammar but resolve ambiguity.

| # | Finding | Deliverable |
|---|---------|-------------|
| F-02 | Higher-kinded types | ADR: row poly + effects + dispatch cover HKT use cases (or document gaps) |
| F-03 | Variance | ADR: all type parameters invariant by default, future relaxation path |
| F-04 | String representation × 128-bit pointers | ADR: concrete `&str` and `&[T]` representation |
| F-05 | Result/Option × Effects | ADR: role of `Result`/`Option` alongside effects, when each is appropriate, interconversion |
| F-08 | Stdlib scope / freestanding split | Design note: core/alloc/std tier mapping |
| F-09 | Testing as language feature | Design note: effect-based test declarations, `blood test` runner |
| F-10 | ABI stability | ADR: "explicitly unstable" + content-hash-based ABI concept |

### 0.5 — Minimal-Effort Defaults

One-paragraph decision records each:

1. **Cyclic imports**: Allowed or forbidden? (Likely: forbidden, matches content-addressed DAG)
2. **Interior mutability**: Supported or not? (Likely: defer, document as not-yet-designed)
3. **Dead code detection**: Planned or not? (Likely: yes, as compiler warning)
4. **Definite initialization**: Statically enforced? (Likely: yes, via MIR analysis)
5. **Doc comment syntax**: `///` or other? (Decide before stdlib grows)
6. **Frame pointer preservation**: Default on or off? (Likely: on, for profiling)
7. **Variance**: Invariant by default? (Likely: yes — see also F-03)

### 0.6 — Inherited Decision Confirmations

These were adopted from Rust without documented independent evaluation. Each needs a brief ADR confirming or revising the choice in Blood's context.

| Decision | Why It Warrants Evaluation | Overlaps With |
|----------|---------------------------|---------------|
| Monomorphization | Interacts with content addressing | **Resolved** (ADR-030 / F-01) |
| `Option<T>` / `Result<T, E>` | Coexists with effects | F-05 |
| UTF-8 strings | Interacts with 128-bit pointers | F-04 |
| File-based module hierarchy | Content addressing decouples identity from files | — |
| `pub` visibility (Rust-style) | Row polymorphism introduces structural subtyping | — |
| Call-by-value evaluation | Natural for effects but undocumented | — |
| No runtime type information | Multiple dispatch uses 24-bit type fingerprints — this IS RTTI | — |
| `&T` / `&mut T` reference syntax | Blood's references are generational, not borrowed | — |
| Binary `unsafe` blocks | Granular safety controls proposed (RFC-S) | Tier 1 proposals |

### Phase 0 Exit Criteria

Phase 0 is complete when:

**Architectural:**
- [x] F-01 ADR written and accepted (ADR-030, 2026-02-28)
- [ ] F-06 concurrency model design document written and accepted
- [ ] F-07 compiler-as-a-library architectural note written

**Proposals:**
- [ ] All Tier 1 proposals evaluated: approved or rejected with rationale
- [ ] All Tier 2 proposals evaluated: approved as direction or deferred
- [ ] Tier 3 proposals acknowledged with dependency map

**Grammar:**
- [ ] GRAMMAR.md updated to v0.5.0 incorporating all approved Tier 1 proposals
- [ ] FORMAL_SEMANTICS.md updated if approved proposals add typing rules

**Design gaps:**
- [ ] F-02, F-03, F-04, F-05, F-08, F-09, F-10 resolved (ADRs or design notes)
- [ ] All 7 minimal-effort defaults documented
- [ ] All inherited decisions confirmed or revised (monomorphization already done)

**Coordination:**
- [ ] CONCURRENCY.md updated with cohesive concurrency model (from F-06)
- [ ] SPECIFICATION.md updated if hierarchy changes

---

## 4. Phase A: Syntax Alignment

**Priority**: High — blocks compiler alignment.
**Prerequisite**: Phase 0 complete (design is stable, GRAMMAR.md at v0.5.0).
**Method**: CCV (Canary-Cluster-Verify) per DEVELOPMENT.md.

The compilers currently accept old syntax in several places where GRAMMAR.md has evolved. Every `.blood` file in the repository must be audited and updated. Because Tier 1 proposals were resolved in Phase 0, this alignment happens **once** against the **final** grammar.

### A.1 — Syntax Delta Analysis

Comprehensive diff between GRAMMAR.md v0.5.0 productions and what each parser actually accepts. Covers:

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

**Inputs**: GRAMMAR.md v0.5.0, `src/bootstrap/bloodc/src/parser/`, `src/bootstrap/bloodc/src/lexer.rs`, `src/selfhost/parser_*.blood`, `src/selfhost/lexer.blood`, `src/selfhost/token.blood`

**Output**: Complete list of deltas with severity (breaking vs cosmetic).

### A.2 — Update Bootstrap Compiler (Rust)

The bootstrap compiler defines language semantics. It must accept the spec syntax **first**.

- Update lexer/parser in `src/bootstrap/bloodc/src/` to match GRAMMAR.md v0.5.0
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
2. **F-06**: Concurrency model — largest undecided area; effects + fibers not composed
3. **F-07**: Compiler-as-a-library — monolithic pipeline; retrofit is expensive

**Proposal critical path** (from PROPOSAL_ANALYSIS.md):

```
#20 (Spec Annotations) → #7 (Verification) → #18 (Cache) → #10 (Proof-Carrying Code)
```

Tier 1 triage (Phase 0.2) evaluates #20 and the other grammar-affecting proposals. The rest of the critical path (#7→#18→#10) is in Tier 3 (deferred until infrastructure exists).

---

## Work Item Counts

| Phase | Items | Nature |
|-------|-------|--------|
| 0.1: Architectural findings | 2 remaining (F-06, F-07) | Design documents |
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
