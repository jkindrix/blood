# Blood Feature Proposal Analysis

**Version:** 1.0
**Date:** 2026-02-27
**Status:** Active
**Scope:** Unified analysis of all 23 feature proposals + Safety Controls RFC

---

## Overview

This document provides a unified analysis of Blood's feature proposals across four source documents:

| Document | Proposals | Theme |
|----------|-----------|-------|
| EXTRAORDINARY_FEATURES.md | 1–7 | Pillar-enabled features (effects, CAS, generational memory, dispatch) |
| EXTRAORDINARY_FEATURES_II.md | 8–15 | Effect handlers as universal interception |
| EXTRAORDINARY_FEATURES_III.md | 16–23 | AI-native language design |
| SAFETY_LEVELS.md | RFC-S | Granular safety controls |

Blood's four pillars: **Algebraic Effects**, **Content-Addressed Code (CAS)**, **Generational Memory Safety**, **Multiple Dispatch**.

---

## 1. Unified Priority Ranking

Single ordering across all proposals, ranked by:
- **Foundation value**: Does this enable other proposals?
- **Implementation feasibility**: Can it ship incrementally?
- **Competitive moat**: How unique is this to Blood?
- **User demand**: How immediately useful?

| Rank | # | Proposal | Category | Foundation For | Effort |
|------|---|----------|----------|----------------|--------|
| 1 | 20 | First-Class Specification Annotations | Compiler | 7, 10, 16, 18 | Medium |
| 2 | RFC-S | Granular Safety Controls | Compiler | 1, 5 | Medium |
| 3 | 17 | Machine-Readable Structured Diagnostics | Toolchain | 16, 22 | Medium |
| 4 | 7 | Gradual Verification | Compiler | 10, 18 | Large |
| 5 | 8 | Deterministic Simulation Testing | Library | 12 | Medium |
| 6 | 21 | AI-Optimized Syntax Decisions | Compiler | 16 | Small |
| 7 | 19 | Compact Module Signatures | Toolchain | 22 | Small |
| 8 | 22 | Dependency Graph API | Toolchain | 19 | Medium |
| 9 | 4 | Capability-Based Security via Effects | Compiler | 9, 23 | Medium |
| 10 | 16 | Type-and-Effect Constrained Decoding | Toolchain | — | Large |
| 11 | 18 | Verification Cache | Toolchain | — | Medium |
| 12 | 9 | Taint Tracking via Effects | Library | — | Small |
| 13 | 13 | Zero-Code Observability | Library | — | Small |
| 14 | 11 | Automatic Semantic Versioning | Toolchain | — | Small |
| 15 | 5 | Effect-Guided Parallelization | Compiler | — | Large |
| 16 | 12 | Deterministic Replay Debugging | Toolchain | — | Medium |
| 17 | 1 | Compile-Time WCET Analysis | Compiler | — | Large |
| 18 | 3 | Automatic Memoization | Library | — | Medium |
| 19 | 2 | Session Types | Compiler | 14 | Large |
| 20 | 23 | Effect Handlers as Agent Middleware | Library | — | Medium |
| 21 | 14 | Choreographic Programming | Compiler | — | Very Large |
| 22 | 15 | Compile-Time Complexity Bounds | Compiler | — | Large |
| 23 | 6 | Provenance Tracking | Library | — | Medium |
| 24 | 10 | Proof-Carrying Code | Toolchain | — | Very Large |

### Ranking Rationale

**Top 3 — Foundations that everything else builds on:**

1. **Specification Annotations (#20)** — `requires`/`ensures`/`invariant`/`decreases` are syntax primitives needed by Gradual Verification (#7), Verification Cache (#18), Constrained Decoding (#16), and Proof-Carrying Code (#10). Parse them now; enforcement comes later. Zero runtime cost until activated.

2. **Safety Controls (RFC-S)** — `#[unchecked(generation)]` and `unchecked {}` blocks are required for WCET (#1), Parallelization (#5), and any performance-critical code. Syntax support is Phase 1; actual check toggling is Phase 2.

3. **Structured Diagnostics (#17)** — Machine-readable errors are the substrate for Constrained Decoding (#16), Dependency Graph (#22), and all AI toolchain features. Must be designed before any AI integration.

**Ranks 4–8 — High-value features with clear implementation paths:**

4. **Gradual Verification (#7)** builds directly on specification annotations. Runtime contracts first, compile-time verification later.

5. **DST (#8)** is uniquely enabled by effects and requires no new compiler features — it's a handler library pattern.

6. **AI Syntax (#21)** is partially already implemented (`for`, `+=`, `|>`, `else if`); just needs adoption.

7–8. **Module Signatures (#19)** and **Dependency Graph (#22)** are pure toolchain additions with no compiler changes needed.

**Ranks 9–16 — Valuable but less foundational:**

Features that provide significant value but don't unlock other features. Can be implemented independently.

**Ranks 17–24 — Research-grade or very large scope:**

Features requiring significant compiler infrastructure (WCET analysis, session types, choreographic programming) or whose value depends on having other features first (proof-carrying code requires verification).

---

## 2. Categories

### 2.1 Compiler Features (Language Changes)

Features requiring changes to lexer, parser, type checker, MIR, or codegen.

| # | Proposal | Parser | Typeck | MIR | Codegen | Runtime |
|---|----------|--------|--------|-----|---------|---------|
| 20 | Spec Annotations | New keywords + clauses | Store & validate | Pass through | No change | Optional runtime checks |
| RFC-S | Safety Controls | Attribute + block syntax | Track unchecked regions | Propagate flags | Skip checks per flag | No change |
| 7 | Gradual Verification | (via #20) | Contract checking | Contract insertion | Runtime/compile checks | Assertion runtime |
| 21 | AI Syntax | Already done (mostly) | Minor | Minor | Minor | No change |
| 4 | Capability Security | Effect attenuation syntax | Capability tracking | Effect narrowing | Handler codegen | No change |
| 5 | Parallelization | `#[parallel]` attribute | Purity verification | Parallel MIR blocks | LLVM parallel codegen | Thread pool |
| 1 | WCET Analysis | `@ wcet()` annotation | Loop bound analysis | Timing model | WCET report generation | No change |
| 2 | Session Types | `protocol` keyword | Session state machine | Protocol transitions | Channel codegen | Message runtime |
| 19 | Choreographic Programming | `choreography` keyword | Multi-party typing | Endpoint projection | Per-participant codegen | Network runtime |
| 15 | Complexity Bounds | `@ complexity()` annotation | Recurrence analysis | Loop cost model | Report generation | No change |

### 2.2 Library Patterns (No Compiler Changes)

Features implementable as Blood libraries/handlers using existing effect infrastructure.

| # | Proposal | Effect | Handler | Stdlib Module |
|---|----------|--------|---------|---------------|
| 8 | DST | `IO, Time, Random, Network` | `SimulationHandler` | `std.testing.simulation` |
| 9 | Taint Tracking | `UntrustedInput` | `HtmlSanitize`, `SqlSanitize` | `std.security.taint` |
| 12 | Replay Debugging | All effectful ops | `RecordHandler`, `ReplayHandler` | `std.debug.replay` |
| 13 | Observability | All effectful ops | `TracedHandler`, `MeteredHandler` | `std.observability` |
| 3 | Memoization | Pure functions | `MemoizeHandler` | `std.cache.memoize` |
| 23 | Agent Middleware | `FileSystem, Terminal, LLM` | `SandboxHandler`, `CostHandler` | `std.agent` |
| 6 | Provenance | `Provenance` | `ProvenanceTracker` | `std.compliance.provenance` |

### 2.3 Toolchain Features (External Tools)

Features implemented as CLI commands, LSP extensions, or build system additions.

| # | Proposal | Command / API | Dependencies |
|---|----------|---------------|--------------|
| 17 | Structured Diagnostics | `--diagnostics=json` | Compiler internals |
| 19 | Module Signatures | `blood sig`, `blood context` | Name resolution |
| 22 | Dependency Graph | `blood deps`, `blood impact` | Full compilation |
| 11 | Semantic Versioning | `blood semver --compare` | CAS hashing |
| 16 | Constrained Decoding | LSP `ConstrainedDecodingService` | Incremental typeck |
| 18 | Verification Cache | `VerificationCache` data structure | CAS + verification |
| 10 | Proof-Carrying Code | Proof artifact format | Full verification |

---

## 3. Gap Analysis Matrix

What each proposal needs across compiler pipeline stages.

### Legend
- **—** = No change needed
- **S** = Small (< 100 LOC)
- **M** = Medium (100–500 LOC)
- **L** = Large (500–2000 LOC)
- **XL** = Very Large (2000+ LOC)

| # | Proposal | Lexer | Parser | AST | HIR | Typeck | MIR | Codegen | Runtime | Tooling |
|---|----------|-------|--------|-----|-----|--------|-----|---------|---------|---------|
| 20 | Spec Annotations | S | M | S | S | M | — | — | S | — |
| RFC-S | Safety Controls | S | M | S | S | S | S | M | — | S |
| 7 | Gradual Verification | — | — | — | — | L | M | M | M | M |
| 21 | AI Syntax | — | — | — | — | — | — | — | — | — |
| 4 | Capability Security | — | S | S | S | M | S | S | — | — |
| 8 | DST | — | — | — | — | — | — | — | — | L |
| 9 | Taint Tracking | — | — | — | — | — | — | — | — | M |
| 13 | Observability | — | — | — | — | — | — | — | — | M |
| 17 | Diagnostics | — | — | — | — | — | — | — | — | L |
| 19 | Module Sigs | — | — | — | — | — | — | — | — | M |
| 22 | Dep Graph | — | — | — | — | — | — | — | — | L |
| 11 | Semver | — | — | — | — | — | — | — | — | M |
| 16 | Constrained Decoding | — | — | — | — | L | — | — | — | XL |
| 18 | Verification Cache | — | — | — | — | — | — | — | — | L |
| 5 | Parallelization | — | S | S | S | L | L | L | M | M |
| 12 | Replay | — | — | — | — | — | — | — | — | L |
| 1 | WCET | S | S | S | S | L | L | M | — | L |
| 3 | Memoization | — | S | S | S | S | — | — | M | S |
| 2 | Session Types | M | L | L | L | XL | L | L | M | M |
| 23 | Agent Middleware | — | — | — | — | — | — | — | — | L |
| 14 | Choreography | M | L | L | L | XL | XL | L | L | L |
| 15 | Complexity | S | S | S | S | L | L | — | — | M |
| 6 | Provenance | — | S | S | S | S | — | — | M | M |
| 10 | Proof-Carrying | — | — | — | — | — | — | — | — | XL |

---

## 4. Dependency Graph

```
Specification Annotations (#20)
├─► Gradual Verification (#7)
│   ├─► Verification Cache (#18)
│   │   └─► Proof-Carrying Code (#10)
│   └─► Constrained Decoding (#16) [also needs #17]
├─► WCET Analysis (#1) [also needs RFC-S]
└─► Complexity Bounds (#15)

Safety Controls (RFC-S)
├─► WCET Analysis (#1)
└─► Parallelization (#5)

Structured Diagnostics (#17)
├─► Constrained Decoding (#16)
└─► Dependency Graph (#22)
    └─► Module Signatures (#19) [bidirectional]

Capability Security (#4)
├─► Taint Tracking (#9)
└─► Agent Middleware (#23)

DST (#8)
└─► Replay Debugging (#12)

Session Types (#2)
└─► Choreographic Programming (#14)

AI Syntax (#21) ─► [no downstream deps]
Observability (#13) ─► [no downstream deps]
Memoization (#3) ─► [no downstream deps]
Semver (#11) ─► [no downstream deps]
Provenance (#6) ─► [no downstream deps]
```

### Critical Path

The longest dependency chain:

```
#20 (Spec Annotations) → #7 (Verification) → #18 (Cache) → #10 (Proof-Carrying Code)
```

This chain should be started first to unblock the most downstream features.

### Independent Tracks

These proposals have no upstream dependencies and can be implemented in parallel with the critical path:

- **Track A (Library):** #8 DST, #9 Taint, #13 Observability, #3 Memoization, #23 Agent Middleware
- **Track B (Toolchain):** #17 Diagnostics, #19 Signatures, #22 Dep Graph, #11 Semver
- **Track C (Compiler):** #21 AI Syntax, RFC-S Safety Controls

---

## 5. Implementation Roadmap

### Phase 0: Syntax Modernization (Current Sprint)

**Goal:** Update the 75,782-line self-hosted compiler to use features it already supports.

| Task | Scope | Verification |
|------|-------|-------------|
| Replace `while i < N` → `for i in 0..N` | ~1,024 loops across 65 files | 336/336 ground-truth + bootstrap |
| Replace `x = x + 1` → `x += 1` | ~1,294 instances | 336/336 ground-truth + bootstrap |
| Adopt `continue`, `|>` where natural | Opportunistic | 336/336 ground-truth + bootstrap |

### Phase 1: Foundation Syntax (v0.4.0)

**Goal:** Parse specification annotations and safety controls. No enforcement.

| Feature | Deliverable | New Tests |
|---------|-------------|-----------|
| `requires`/`ensures`/`invariant`/`decreases` | Parsed, stored in AST/HIR | t09_spec_* |
| `#[unchecked(check)]` attribute | Parsed, stored in HIR | t09_unchecked_* |
| `unchecked(checks) { }` block | Parsed, lowered to MIR | t09_unchecked_block |
| Semicolon flexibility | Parser accepts both styles | t09_semicolon_optional |

### Phase 2: Core Verification (v0.5.0)

**Goal:** Runtime contract checking and structured diagnostics.

| Feature | Deliverable | Depends On |
|---------|-------------|------------|
| Runtime `requires`/`ensures` | Assert insertion in codegen | Phase 1 |
| `--diagnostics=json` | Structured error output | — |
| `blood sig` command | Module signature extraction | — |
| `blood deps` command | Dependency graph API | — |
| DST handler library | `std.testing.simulation` | — |

### Phase 3: Static Verification (v0.6.0)

**Goal:** Compile-time contract verification for a subset of Blood.

| Feature | Deliverable | Depends On |
|---------|-------------|------------|
| `#[verify]` compile-time checking | SMT integration for pure functions | Phase 2 |
| Safety check toggling | Codegen respects `#[unchecked]` | Phase 1 |
| Verification cache | Content-addressed proof storage | Phase 2 |
| `blood semver` | Automatic versioning | CAS infrastructure |

### Phase 4: AI Toolchain (v0.7.0)

**Goal:** Blood as the best language for AI code generation.

| Feature | Deliverable | Depends On |
|---------|-------------|------------|
| `blood context --for-ai` | AI-optimized context generation | Phase 2 |
| Constrained decoding oracle | LSP extension for token filtering | Phase 3 |
| Taint tracking library | `std.security.taint` | — |
| Observability library | `std.observability` | — |

### Phase 5: Advanced Features (v0.8.0+)

**Goal:** Research-grade features for specific domains.

| Feature | Deliverable | Depends On |
|---------|-------------|------------|
| Capability attenuation | Effect narrowing in handlers | — |
| WCET analysis | Compile-time timing bounds | Phases 1, 3 |
| Parallelization | Automatic parallel map/fold | Phase 3 |
| Session types | Protocol state machines | — |
| Replay debugging | Record/replay handlers | — |

### Phase 6: Research (v1.0+)

| Feature | Deliverable | Depends On |
|---------|-------------|------------|
| Choreographic programming | Multi-party protocol compilation | Phase 5 |
| Proof-carrying code | Proof artifact format | Phases 3, 4 |
| Complexity bounds | Compile-time Big-O verification | Phase 3 |
| Agent middleware | Universal agent framework | Phase 4 |

---

## 6. Pillar Coverage Analysis

How each pillar is leveraged across proposals:

| Pillar | Primary In | Secondary In | Coverage |
|--------|-----------|-------------|----------|
| **Algebraic Effects** | 1, 2, 4, 5, 8, 9, 12, 13, 14, 16, 20, 23 | 3, 6, 7, 11, 15 | 17/23 (74%) |
| **Content-Addressed Code** | 3, 10, 11, 18 | 2, 6, 8, 9, 12, 13, 14, 19, 22, 23 | 14/23 (61%) |
| **Generational Memory** | RFC-S | 5, 7, 8, 12, 23 | 6/24 (25%) |
| **Multiple Dispatch** | — | 5, 8, 23 | 3/23 (13%) |

**Observation:** Effects dominate the proposal space (74% of proposals use them). Content-addressing is well-represented (61%). Generational memory and multiple dispatch are underrepresented — consider additional proposals that leverage these pillars specifically.

---

## 7. Risk Assessment

| Risk | Proposals Affected | Mitigation |
|------|-------------------|------------|
| SMT solver integration complexity | 7, 10, 15, 16, 18 | Start with runtime contracts; defer SMT to Phase 3 |
| Specification annotation design lock-in | 20, 7, 10 | Design syntax now; keep semantics extensible |
| Bootstrap instability from syntax changes | 21, all compiler modernization | Incremental batches with bootstrap verification |
| AI landscape changes faster than implementation | 16, 17, 19, 22, 23 | Build composable toolchain primitives, not opinionated workflows |
| Session type / choreography complexity | 2, 14 | Defer to Phase 5+; implement session types before choreography |
| Performance overhead of safety checks | RFC-S, 7 | Granular controls (RFC-S) enable selective opt-out |

---

## 8. Quick Reference: What Ships When

| Phase | Version | Proposals | Timeline Estimate |
|-------|---------|-----------|-------------------|
| 0 | v0.3.x | Syntax modernization (for, +=, \|>) | Now |
| 1 | v0.4.0 | #20, RFC-S (parse only), #21 | Near-term |
| 2 | v0.5.0 | #20 (runtime), #17, #19, #22, #8 | Short-term |
| 3 | v0.6.0 | #7, RFC-S (enforce), #18, #11 | Medium-term |
| 4 | v0.7.0 | #16, #9, #13 | Medium-term |
| 5 | v0.8.0 | #4, #1, #5, #2, #12 | Long-term |
| 6 | v1.0+ | #14, #10, #15, #23, #3, #6 | Research |

---

*Cross-references: [SYNTAX_REDESIGN.md](SYNTAX_REDESIGN.md) for grammar changes, [GRAMMAR.md](../spec/GRAMMAR.md) for current grammar spec.*
