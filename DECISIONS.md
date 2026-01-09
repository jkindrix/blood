# Blood Architecture Decision Records

This document captures key architectural decisions made during the design of Blood and their rationale.

### Related Specifications

- [SPECIFICATION.md](./SPECIFICATION.md) — Core language specification
- [MEMORY_MODEL.md](./MEMORY_MODEL.md) — ADR-001, ADR-004, ADR-008, ADR-013, ADR-014 details
- [DISPATCH.md](./DISPATCH.md) — ADR-005 details
- [CONTENT_ADDRESSED.md](./CONTENT_ADDRESSED.md) — ADR-003, ADR-012 details
- [FORMAL_SEMANTICS.md](./FORMAL_SEMANTICS.md) — ADR-002, ADR-006, ADR-007, ADR-011 details
- [ROADMAP.md](./ROADMAP.md) — Implementation timeline

---

## ADR-001: Use Generational References Instead of Borrow Checking

**Status**: Accepted

**Context**: Blood needs memory safety without garbage collection. The two main approaches are:
1. Borrow checking (Rust) — compile-time ownership tracking
2. Generational references (Vale) — runtime generation tag checking

**Decision**: Blood uses generational references with 128-bit fat pointers.

**Rationale**:
- Borrow checking has a steep learning curve and adversarial feel
- Generational references are simpler to understand and use
- Runtime overhead is minimal (~1-2 cycles per dereference, theoretical—see MEMORY_MODEL.md §1.1)
- Escape analysis can eliminate checks for provably-safe references
- Mutable value semantics further reduce the need for references

**Consequences**:
- Slightly larger pointer size (128-bit vs 64-bit)
- Small runtime overhead for non-optimized paths
- Simpler mental model for developers
- Easier to achieve memory safety correctness

---

## ADR-002: Algebraic Effects for All Side Effects

**Status**: Accepted

**Context**: Languages handle side effects in various ways:
- Untracked (C, Go)
- Monads (Haskell)
- Keywords (async/await)
- Algebraic effects (Koka)

**Decision**: Blood uses algebraic effects as the universal effect mechanism.

**Rationale**:
- Unifies IO, state, exceptions, async, non-determinism
- Effects are explicit in function signatures
- Handlers enable dependency injection and testing
- Composable without "wrapper hell"
- Resumable exceptions enable powerful control flow

**Consequences**:
- All side effects visible in types
- Some learning curve for effect handlers
- Enables mock handlers for testing
- Requires careful design of standard effect library

---

## ADR-003: Content-Addressed Code via BLAKE3-256

**Status**: Accepted

**Context**: Traditional languages use file paths and symbol names for code identity. Unison pioneered content-addressed code using hashes.

**Decision**: Blood identifies all definitions by BLAKE3-256 hash of canonicalized AST.

**Rationale**:
- Eliminates dependency hell (multiple versions coexist by hash)
- Enables perfect incremental compilation
- Makes refactoring safe (renames don't change identity)
- Enables zero-downtime hot-swapping
- BLAKE3 provides sufficient collision resistance with high performance

**Consequences**:
- Requires new tooling paradigm (codebase manager vs files)
- FFI requires bridge dialect for C symbol mapping
- Learning curve for content-addressed workflow
- Perfect reproducibility and caching

---

## ADR-004: Generation Snapshots for Effect Safety

**Status**: Accepted

**Context**: When algebraic effects suspend computation, captured continuations may hold generational references that become stale before resume.

**Decision**: Blood captures a "generation snapshot" with each continuation and validates on resume.

**Rationale**:
- No existing language addresses this interaction
- Stale references could cause use-after-free on resume
- Validation cost is proportional to captured references
- Lazy validation amortizes cost to actual dereferences
- StaleReference effect enables graceful recovery

**Consequences**:
- Novel contribution (no prior art)
- Small overhead on continuation capture
- Validation on resume adds safety guarantee
- Handlers can choose panic or graceful degradation

---

## ADR-005: Multiple Dispatch with Type Stability Enforcement

**Status**: Accepted

**Context**: Julia demonstrates multiple dispatch can enable high performance, but type instability causes performance cliffs.

**Decision**: Blood uses multiple dispatch with compile-time type stability checking.

**Rationale**:
- Solves the Expression Problem (add types and operations independently)
- Enables retroactive protocol conformance
- Type stability ensures predictable performance
- Compiler warnings prevent performance cliffs

**Consequences**:
- More flexible than single dispatch
- Requires clear dispatch resolution rules
- Ambiguity is a compile error
- Type-unstable code rejected

---

## ADR-006: Linear Types for Resource Management

**Status**: Accepted

**Context**: Some resources (file handles, network connections) must be used exactly once and cannot be forgotten.

**Decision**: Blood supports linear types (must use exactly once) and affine types (at most once).

**Rationale**:
- Prevents resource leaks at compile time
- Ensures cleanup code always runs
- Interacts with effect system (linear values can't cross multi-shot resume)
- More precise than Rust's affine-only approach

**Consequences**:
- Additional type annotations for resources
- Compiler enforces use-exactly-once
- Multi-shot handlers cannot capture linear values
- Strong resource safety guarantees

---

## ADR-007: Deep and Shallow Handlers

**Status**: Accepted

**Context**: Effect handlers can be "deep" (persistent) or "shallow" (one-shot). Different use cases benefit from each.

**Decision**: Blood supports both, with deep as default.

**Rationale**:
- Deep handlers handle all operations in a computation (most common)
- Shallow handlers handle one operation then disappear (generators, streams)
- Explicit choice prevents confusion about handler semantics
- Both are needed for full expressiveness

**Consequences**:
- Handler kind must be specified (or defaulted to deep)
- Different operational semantics for each
- Enables both state-like and stream-like patterns

---

## ADR-008: Tiered Memory Model

**Status**: Accepted

**Context**: Different allocations have different lifecycles and safety requirements.

**Decision**: Blood uses three memory tiers:
1. Stack (lexical, zero cost)
2. Region (scoped, generational checks)
3. Persistent (global, reference counted)

**Rationale**:
- Stack allocation is fastest
- Most allocations can be proven to be stack-safe
- Generational checks for heap allocations
- Reference counting fallback for long-lived objects
- Escape analysis promotes to optimal tier

**Consequences**:
- Compiler complexity for tier selection
- Most code gets zero-cost safety
- Performance predictable by tier
- Generation overflow handled by tier promotion

---

## ADR-009: Row Polymorphism for Records and Effects

**Status**: Accepted

**Context**: Structural typing and effect polymorphism both benefit from row variables.

**Decision**: Blood uses row polymorphism for both record types and effect rows.

**Rationale**:
- Functions can accept any record with required fields
- Functions can be generic over additional effects
- Enables "extensible records" pattern
- Unified approach for data and effects

**Consequences**:
- More flexible than nominal typing
- Slightly more complex type inference
- Enables powerful generic programming
- Well-established theory (Rémy's rows, Koka's effects)

---

## ADR-010: Hierarchy of Concerns

**Status**: Accepted

**Context**: Design decisions sometimes conflict (e.g., safety vs ergonomics). A priority ordering is needed.

**Decision**: Blood prioritizes: Correctness > Safety > Predictability > Performance > Ergonomics

**Rationale**:
- Incorrect code is worthless regardless of speed
- Memory safety is non-negotiable for target domains
- Developers must understand performance characteristics
- Performance matters after correctness/safety
- Ergonomics is last but not unimportant

**Consequences**:
- Sometimes verbose syntax when safety requires it
- No "escape hatches" that compromise safety
- Poor ergonomics indicates design problem
- Clear decision framework for tradeoffs

---

## ADR-011: Five Innovation Composition

**Status**: Accepted

**Context**: Blood combines five specific innovations from different research languages:
1. Content-addressed code (Unison)
2. Generational references (Vale)
3. Mutable value semantics (Hylo)
4. Algebraic effects (Koka)
5. Multiple dispatch (Julia)

This combination is unprecedented and required formal analysis of interaction safety.

**Decision**: Blood adopts all five innovations with formal composition safety proofs.

**Rationale**:
- Each innovation solves real problems independently
- Composition benefits exceed sum of parts (synergies documented in FORMAL_SEMANTICS.md §10)
- Formal analysis proves innovations compose safely (no emergent unsoundness)
- Addresses gaps in existing languages (safety-performance tradeoff, expression problem, effect management)

**Consequences**:
- Unprecedented language design requiring novel research
- Complexity in implementation and tooling
- Rich feature set enabling new programming patterns
- Formal proofs provide confidence in soundness (see FORMAL_SEMANTICS.md §10)

**Novel Contribution**: The composition and its formal analysis constitute Blood's primary research contribution.

---

## ADR-012: VFT Hot-Swap with Effect Coordination

**Status**: Accepted

**Context**: Content-addressed code enables hot-swapping by redirecting hash references. However, in-flight operations (active function calls, suspended effect handlers) complicate safe replacement.

**Decision**: Blood supports three swap strategies with effect handler coordination:
1. **Immediate** — New version takes effect at next call (may mix versions)
2. **Barrier** — Wait for quiescent point before swap
3. **Epoch** — Requests entering after swap use new version; in-flight complete with old

**Rationale**:
- Different applications need different consistency guarantees
- Effect handlers can span VFT boundaries (suspended continuations)
- Version mixing is sometimes acceptable (stateless functions)
- Full consistency sometimes required (stateful operations)
- See CONTENT_ADDRESSED.md §8.5 for full specification

**Consequences**:
- Runtime must track version epochs per handler
- Rollback possible if new version fails validation
- Observability metrics for swap progress
- Clear semantics for each consistency level

**Novel Contribution**: Integrating hot-swap with algebraic effect handlers is novel to Blood.

---

## ADR-013: Effect-Aware Escape Analysis

**Status**: Accepted

**Context**: Traditional escape analysis determines whether values can be stack-allocated based on whether references outlive their scope. Algebraic effects add complexity: effect suspension points can capture references in continuations.

**Decision**: Blood extends escape analysis with effect boundary tracking:
- Values that may be captured in continuations at effect suspension points are classified differently
- Deep handlers preserve captured references across multiple resumes
- Shallow handlers consume values (single resume)
- Multi-shot handlers require special handling (values may be used multiple times)

**Rationale**:
- Effect suspension creates implicit reference capture (in continuation)
- Captured references must survive to resume point
- Optimization requires understanding effect handler semantics
- Shallow handlers enable optimizations impossible with deep handlers
- See MEMORY_MODEL.md §5.8 for full specification

**Consequences**:
- More conservative stack promotion near effect boundaries
- Optimization based on handler kind (deep vs shallow)
- Multi-shot handlers require stricter escape classification
- Effect inference provides information for escape analysis

**Novel Contribution**: This interaction between escape analysis and algebraic effects is novel to Blood.

---

## ADR-014: Hybrid Mutable Value Semantics

**Status**: Accepted

**Context**: Hylo demonstrates pure mutable value semantics (MVS) can eliminate many reference-related bugs. However, some patterns genuinely require references (graph structures, shared state).

**Decision**: Blood uses a hybrid model:
- Default to value semantics (like Hylo)
- Explicit borrowing syntax (`&T`, `&mut T`) when references are genuinely needed
- Clear distinction between value operations and reference operations

**Rationale**:
- Pure MVS is too restrictive for systems programming
- Explicit references make aliasing visible
- Value semantics simplify reasoning for most code
- Hybrid approach provides best of both worlds
- See MEMORY_MODEL.md §1.3 for clarification

**Consequences**:
- Most code uses simple value semantics
- Reference patterns require explicit annotation
- Clear mental model: values copy, references alias
- Gradual adoption path from reference-heavy code

---

## ADR-015: AOT-First with Optional JIT Compilation

**Status**: Accepted

**Context**: Blood must choose between Ahead-of-Time (AOT) and Just-in-Time (JIT) compilation strategies. This affects startup time, peak performance, memory usage, and development experience.

| Factor | AOT | JIT |
|--------|-----|-----|
| **Startup time** | Fast (pre-compiled) | Slow (compile at runtime) |
| **Peak performance** | Good | Excellent (runtime profiling) |
| **Memory usage** | Lower | Higher (compiler in memory) |
| **Predictability** | High | Variable (warmup phase) |
| **Debugging** | Straightforward | Complex (multiple code versions) |
| **Deployment** | Simple binary | Requires runtime |

**Decision**: Blood uses **AOT compilation as the primary strategy** with optional JIT for development and specific use cases.

### Compilation Modes

| Mode | Use Case | Implementation |
|------|----------|----------------|
| `blood build` | Production deployment | Full AOT via LLVM |
| `blood run` (default) | Development | AOT with fast compilation |
| `blood run --jit` | Performance exploration | Optional JIT mode (Phase 5+) |
| `blood repl` | Interactive development | Interpreter + incremental AOT |

**Rationale**:

1. **Systems programming alignment**: Blood targets safety-critical systems where predictable performance and minimal runtime are essential. AOT provides deterministic startup and no warmup variance.

2. **Effect system compatibility**: Evidence passing compilation (see ROADMAP.md §13) works naturally with AOT. Effect handlers compile to direct calls in monomorphic code, avoiding JIT complexity.

3. **Content-addressed synergy**: Blood's content-addressed design enables perfect incremental AOT compilation. Function hashes enable aggressive caching—a JIT benefit achieved at compile time.

4. **Embedded/resource-constrained targets**: AOT produces standalone binaries without runtime overhead, suitable for embedded systems and containers.

5. **Development experience preserved**: Fast incremental AOT compilation (~100ms for typical changes) provides JIT-like iteration speed. The REPL uses interpretation for immediate feedback.

6. **Optional JIT for specific needs**: Some workloads benefit from runtime profiling (e.g., generic algorithms with unpredictable type distributions). JIT mode available when explicitly requested.

### AOT Optimization Strategy

```
Source → Parse → Type Check → Effect Inference → Monomorphize
       → LLVM IR → Optimize → Native Code

Optimization levels:
  -O0: Debug (no optimization, fast compile)
  -O1: Basic (local optimizations)
  -O2: Standard (full optimization, default for release)
  -O3: Aggressive (LTO, PGO-guided if profile available)
```

### Profile-Guided AOT

Blood supports AOT with profiling data, achieving JIT-like optimization without runtime overhead:

```bash
# Generate profile
$ blood build --profile
$ ./my_program < typical_input.txt
$ blood build --use-profile=my_program.profdata -O3
```

**Consequences**:

- Predictable, fast startup for all Blood programs
- No runtime compiler overhead in production
- Incremental compilation provides fast development iteration
- JIT available as opt-in for performance exploration
- Profile-guided optimization bridges the AOT/JIT performance gap
- Simpler debugging (one code version at a time)

**Implementation Notes**:

- Phase 1-4: AOT only via LLVM backend
- Phase 5+: Optional JIT mode using Cranelift or LLVM MCJIT
- REPL: Tree-walking interpreter for immediate feedback, AOT for defined functions

**References**:
- [JIT vs AOT Trade-offs](https://www.infoq.com/presentations/jit-aot-tradeoffs/) (InfoQ)
- [GraalVM Native Image](https://www.graalvm.org/native-image/) — AOT for JVM languages
- [Koka Compilation](https://koka-lang.github.io/koka/doc/book.html) — AOT with evidence passing

---

## ADR-016: Prototype-First Validation Strategy

**Status**: Accepted

**Context**: Blood combines five cutting-edge innovations, several of which have novel interactions (e.g., generation snapshots + effect resume, linear types + multi-shot handlers). Implementing the full compiler without validating these novel mechanisms risks significant rework if design flaws are discovered late.

**Decision**: Before proceeding past Phase 0, validate all novel mechanisms through isolated Rust prototypes (see ROADMAP.md §16).

**Rationale**:
- Novel mechanisms (generation snapshots, region suspension) have no prior art to reference
- Early validation reduces costly rework later
- Prototypes provide performance data for design validation
- Safety properties can be tested with property-based testing before formal proofs

**Validation Spikes**:

| Spike | Priority | Validates |
|-------|----------|-----------|
| Generation Snapshots + Resume | P0 (Critical) | MEMORY_MODEL.md §4, FORMAL_SEMANTICS.md §4 |
| Effects + Linear Types | P1 (High) | FORMAL_SEMANTICS.md §8 |
| Region Suspension | P1 (High) | MEMORY_MODEL.md §6 |
| Reserved Generation Values | P2 (Medium) | MEMORY_MODEL.md §3.4 |

**Success Criteria**:
- 100% detection of stale references in prototype
- No linear value escapes in multi-shot scenarios
- Performance overhead within design targets
- Property-based tests pass (100+ random scenarios per spike)

**Consequences**:
- Phase 1 blocked until P0 spikes validate successfully
- May require specification amendments based on prototype findings
- Provides empirical data for MEMORY_MODEL.md performance claims
- Creates reference implementations for full compiler

---

## ADR-017: Minimal Viable Language Subset (MVL)

**Status**: Accepted

**Context**: Blood's full specification is ambitious. Attempting to implement everything simultaneously risks never reaching a working compiler. Julia, Rust, and other successful languages started with minimal subsets and grew.

**Decision**: Define and implement a Minimal Viable Language (MVL) subset that can compile and run useful programs before implementing advanced features.

**MVL Subset Definition**:

| Feature | MVL Status | Full Blood |
|---------|------------|------------|
| Primitive types (i32, f64, bool, String) | ✓ Included | ✓ |
| Functions with explicit types | ✓ Included | ✓ |
| Let bindings | ✓ Included | ✓ |
| If/else expressions | ✓ Included | ✓ |
| Match expressions (basic) | ✓ Included | ✓ |
| Struct types | ✓ Included | ✓ |
| Enum types | ✓ Included | ✓ |
| Basic generics (no constraints) | ✓ Included | ✓ |
| IO effect (hardcoded) | ✓ Included | Algebraic |
| Error effect (Result type) | ✓ Included | Algebraic |
| **Type inference** | Deferred | ✓ |
| **Algebraic effects** | Deferred | ✓ |
| **Effect handlers** | Deferred | ✓ |
| **Generational references** | Deferred | ✓ |
| **Multiple dispatch** | Deferred | ✓ |
| **Content addressing** | Deferred | ✓ |
| **Linear/affine types** | Deferred | ✓ |

**MVL Milestone**: Compile and run FizzBuzz with file I/O

```blood
// MVL-compatible FizzBuzz
fn fizzbuzz(n: i32) -> String {
    if n % 15 == 0 { "FizzBuzz" }
    else if n % 3 == 0 { "Fizz" }
    else if n % 5 == 0 { "Buzz" }
    else { n.to_string() }
}

fn main() -> Result<(), IOError> {
    for i in 1..=100 {
        println(fizzbuzz(i))?;
    }
    Ok(())
}
```

**Rationale**:
- Provides working compiler faster than full implementation
- Enables real-world testing and feedback
- Reduces risk of fundamental architecture issues
- Creates foundation for incremental feature addition
- Attracts early adopters and contributors

**Feature Addition Order (Post-MVL)**:
1. Algebraic effects (core differentiator)
2. Type inference
3. Generational references
4. Multiple dispatch
5. Content addressing
6. Linear types

**Consequences**:
- Earlier usable compiler
- Clearer implementation roadmap
- Some features deferred
- MVL programs remain valid in full Blood

---

## ADR-018: Vale Memory Model Fallback Strategy

**Status**: Accepted

**Context**: Blood's memory model is based on Vale's generational references, which as of January 2026 lacks production benchmarks. While theoretically sound, the approach is unproven at scale. If generational references prove impractical, Blood needs fallback options.

**Decision**: Design memory model with fallback strategies while proceeding optimistically with generational references.

**Primary Strategy**: Generational references (as specified in MEMORY_MODEL.md)
- 128-bit fat pointers
- Generation validation on dereference
- Escape analysis optimization

**Fallback Strategy A**: Compile-Time Restriction Mode
If runtime overhead proves unacceptable:
- Restrict reference patterns to compile-time verifiable subset
- Similar to Rust's borrow checker but simpler (no lifetimes)
- All references must be provably stack-valid or region-bound
- Heap references only via Rc<T>/Arc<T>

**Fallback Strategy B**: Hybrid Mode
If pure generational proves too slow in hot paths:
- Allow opt-in `#[unchecked]` blocks for performance-critical code
- Require formal verification or extensive testing for unchecked regions
- Maintain checked mode as default with clear escape hatch

**Fallback Strategy C**: Alternative Generation Encoding
If 128-bit pointers cause cache pressure:
- Use 64-bit pointers with side table for generations
- Trade pointer dereference speed for smaller pointers
- Configurable via compiler flag

**Validation Metrics** (from prototype spikes):
| Metric | Target | Fallback Trigger |
|--------|--------|------------------|
| Gen check overhead | <3 cycles | >10 cycles |
| Pointer size impact | <5% slowdown | >15% slowdown |
| Escape analysis success | >80% elimination | <50% elimination |

**Rationale**:
- Optimistic about Vale approach but prepared for alternatives
- Fallback options maintain safety guarantees
- Early detection of issues via prototype spikes
- Community can help validate/optimize

**Consequences**:
- Generational references remain primary design
- Fallback code paths add implementation complexity
- Performance benchmarks guide final decision
- Transparent communication about validation status

---

## ADR-019: Early Benchmarking Strategy

**Status**: Accepted

**Context**: Blood's specification contains many performance claims (e.g., "~1-2 cycles per generation check") that are explicitly disclaimed as "unvalidated design targets." Without early empirical validation, these claims risk becoming misleading promises.

**Decision**: Establish benchmarking infrastructure before Phase 1, not after.

**Benchmark Categories**:

| Category | Measures | Established When |
|----------|----------|------------------|
| **Lexer/Parser** | Tokens/sec, AST nodes/sec | Phase 0 (exists) |
| **Generation Check** | Cycles per check | Prototype spike |
| **Effect Handler** | Handler call overhead | Phase 2 |
| **Dispatch** | Static vs dynamic overhead | Phase 3 |
| **End-to-end** | Real program performance | Phase 4+ |

**Benchmark Infrastructure**:

```rust
// bloodc/benches/ structure
benches/
├── lexer_bench.rs       // ✓ Exists
├── parser_bench.rs      // ✓ Exists
├── codegen_bench.rs     // Add in Phase 1
├── runtime/
│   ├── generation.rs    // Add with prototype spike
│   ├── effects.rs       // Add in Phase 2
│   └── dispatch.rs      // Add in Phase 3
└── integration/
    ├── microbench.rs    // Small program benchmarks
    └── realworld.rs     // Larger program benchmarks
```

**Comparison Baselines**:
- **Memory safety**: Compare to Rust (borrow checking) and Vale (generational)
- **Effects**: Compare to Koka (algebraic effects) and OCaml (exceptions)
- **Dispatch**: Compare to Julia (multiple dispatch) and C++ (virtual)

**Reporting**:
- Benchmark results published with each milestone
- Clear indication of "design target vs measured"
- Regression detection in CI

**Rationale**:
- Prevents misleading performance claims
- Guides optimization efforts
- Provides data for design decisions
- Demonstrates intellectual honesty

**Consequences**:
- Some upfront infrastructure investment
- Performance claims become verifiable
- May reveal need for design changes
- Attracts performance-conscious contributors

---

## ADR-020: Research Publication Strategy

**Status**: Accepted

**Context**: Blood contains genuine novel contributions (generation snapshots, effects+generational composition, effect-aware escape analysis) that merit academic publication. Publication increases credibility, attracts collaborators, and provides external peer review.

**Decision**: Pursue academic publication for Blood's novel contributions.

**Publication Targets**:

| Contribution | Venue | Target Date |
|--------------|-------|-------------|
| Five Innovation Composition | OOPSLA/ECOOP workshop | After prototype validation |
| Generation Snapshots | PLDI/ICFP | After implementation |
| Effect-Aware Escape Analysis | CC (Compiler Construction) | After Phase 3 |
| Full Language Design | JFP/TOPLAS | After self-hosting |

**Publication Preparation**:
1. **Formalization**: Mechanize proofs in Coq/Agda before submission
2. **Evaluation**: Include empirical benchmarks
3. **Comparison**: Position against related work (Vale, Koka, Unison, etc.)
4. **Reproducibility**: Provide artifact for paper evaluation

**Collaboration Opportunities**:
- Programming language research groups (PLDI/POPL community)
- Systems research groups (for safety-critical applications)
- Industry partners interested in memory safety

**Pre-Publication Actions**:
- Write technical reports documenting novel mechanisms
- Present at regional PL seminars/workshops
- Engage with Koka, Vale, Hylo communities for feedback

**Rationale**:
- External validation of novel contributions
- Attracts academic collaborators
- Increases project credibility
- Provides rigorous feedback on formal semantics

**Consequences**:
- Requires investment in mechanized proofs
- Publication timeline adds pressure
- May receive feedback requiring design changes
- Increases visibility and contributor pool

---

## ADR-021: Community Development Strategy

**Status**: Accepted

**Context**: Blood is currently developed by a small team. Long-term sustainability requires community growth. However, the project's complexity (five innovations, novel mechanisms) creates a high barrier to entry.

**Decision**: Implement a multi-tier contribution model with clear entry points.

**Contribution Tiers**:

| Tier | Barrier | Examples | Onboarding |
|------|---------|----------|------------|
| **Explorer** | Low | Bug reports, documentation, examples | CONTRIBUTING.md |
| **Contributor** | Medium | Parser tests, error messages, tooling | Good First Issues |
| **Core** | High | Type checker, effects, memory model | Mentorship required |
| **Architect** | Very High | Novel mechanism design, formal proofs | Direct collaboration |

**Community Infrastructure**:
- **CONTRIBUTING.md**: Clear contribution guide (see separate file)
- **Good First Issues**: Tagged issues suitable for newcomers
- **Architecture Docs**: ROADMAP.md, DECISIONS.md (this file)
- **Discussion Forum**: GitHub Discussions for design conversations
- **Office Hours**: Regular video calls for contributor questions

**Onboarding Path**:
1. Read SPECIFICATION.md overview
2. Build and run bloodc on examples
3. Pick a Good First Issue
4. Submit PR, receive feedback
5. Graduate to larger contributions

**Mentorship Model**:
- Core contributors mentor new contributors
- Code review includes teaching, not just critique
- Design discussions welcome from all levels

**Rationale**:
- Sustainable development requires community
- Clear tiers reduce overwhelming newcomers
- Mentorship builds capable contributors
- Good First Issues provide entry point

**Consequences**:
- Requires maintaining contributor infrastructure
- Core team time allocated to mentorship
- May slow short-term velocity for long-term sustainability
- Creates path from user to contributor to maintainer

---

## Decision Status Legend

- **Proposed**: Under discussion
- **Accepted**: Decision made and documented
- **Deprecated**: No longer valid
- **Superseded**: Replaced by another decision

---

*Last updated: 2026-01-09*
