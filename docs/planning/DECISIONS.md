# Blood Architecture Decision Records

This document captures key architectural decisions made during the design of Blood and their rationale.

### Related Specifications

- [SPECIFICATION.md](./SPECIFICATION.md) — Core language specification
- [MEMORY_MODEL.md](./MEMORY_MODEL.md) — ADR-001, ADR-004, ADR-008, ADR-013, ADR-014 details
- [DISPATCH.md](./DISPATCH.md) — ADR-005 details
- [CONTENT_ADDRESSED.md](./CONTENT_ADDRESSED.md) — ADR-003, ADR-012 details
- [FORMAL_SEMANTICS.md](./FORMAL_SEMANTICS.md) — ADR-002, ADR-006, ADR-007, ADR-011 details
- [CONCURRENCY.md](./CONCURRENCY.md) — ADR-036 details
- [ROADMAP.md](./ROADMAP.md) — Implementation timeline
- [DESIGN_SPACE_AUDIT.md](../design/DESIGN_SPACE_AUDIT.md) — Design space evaluation (ADR-030 context)

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
- Runtime overhead is minimal (~1-2 cycles per dereference based on Vale's design—see MEMORY_MODEL.md §1.1)
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

**Key Innovation**: The composition of these five features enables new programming patterns not available in any single existing language.

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

**Key Feature**: Blood integrates hot-swap with algebraic effect handlers, enabling zero-downtime updates.

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

**Key Feature**: Blood's escape analysis understands effect boundaries for optimal memory allocation.

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

## ADR-016: Incremental Validation Strategy

**Status**: Accepted

**Context**: Blood combines features from multiple systems (Vale, Koka, Unison, Hylo, Julia). Some feature interactions (e.g., generation snapshots + effect resume, linear types + multi-shot handlers) require validation to ensure correct behavior.

**Decision**: Validate feature interactions through isolated tests and incremental integration.

**Rationale**:
- Feature interactions require empirical validation
- Early validation reduces costly rework later
- Tests provide performance data for design validation
- Safety properties can be tested with property-based testing

**Validation Approach**:

| Feature Interaction | Priority | Specification |
|---------------------|----------|---------------|
| Generation Snapshots + Resume | P0 (Critical) | MEMORY_MODEL.md §4, FORMAL_SEMANTICS.md §4 |
| Effects + Linear Types | P1 (High) | FORMAL_SEMANTICS.md §8 |
| Region Suspension | P1 (High) | MEMORY_MODEL.md §6 |
| Reserved Generation Values | P2 (Medium) | MEMORY_MODEL.md §3.4 |

**Success Criteria**:
- 100% detection of stale references
- No linear value escapes in multi-shot scenarios
- Performance overhead within design targets
- Property-based tests pass (100+ random scenarios)

**Consequences**:
- Integration proceeds after validation passes
- May require specification amendments based on findings
- Provides empirical data for performance claims
- Creates regression tests for ongoing development

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

**Context**: Blood's memory model is based on Vale's generational references. Production benchmarks for this approach are still being gathered. The design is sound based on formal analysis, but large-scale validation is ongoing. Fallback options are documented for risk mitigation.

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

**Context**: Blood's specification contains performance claims (e.g., "~1-2 cycles per generation check") that require empirical validation. Early benchmarking ensures these targets are achievable.

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

## ADR-020: External Validation Strategy

**Status**: Accepted

**Context**: Blood combines features from multiple established systems (Vale, Koka, Unison, Hylo, Julia). External validation ensures the implementation is correct and performant.

**Decision**: Pursue external validation through benchmarks, community engagement, and formal analysis.

**Validation Approach**:

| Validation Type | Method | Status |
|-----------------|--------|--------|
| Correctness | Test suite, property-based testing | Ongoing |
| Performance | Benchmark suite vs. comparable systems | Planned |
| Formal properties | Proof mechanization | Planned |
| Community feedback | Open source, community engagement | Active |

**Validation Preparation**:
1. **Benchmarking**: Compare against Rust, Go, and Koka on equivalent programs
2. **Formalization**: Mechanize proofs in Coq/Agda for critical properties
3. **Comparison**: Document differences from Vale, Koka, Unison approaches
4. **Reproducibility**: Provide benchmark artifacts for independent verification

**Collaboration Opportunities**:
- Systems programming community
- Safety-critical systems developers
- Language implementers interested in effect systems

**Rationale**:
- External validation builds confidence in correctness
- Benchmarks guide optimization priorities
- Community feedback identifies real-world requirements
- Formal proofs provide soundness guarantees

**Consequences**:
- Requires investment in benchmark infrastructure
- Formal proofs require specialized expertise
- Community feedback may drive design changes
- Increases adoption confidence

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

## ADR-022: Slot Registry for Generation Tracking

**Status**: Accepted

**Context**: Generational references need to track the current generation for each heap allocation. Two approaches:
1. **Inline storage**: Store generation adjacent to the allocation (like malloc metadata)
2. **Global registry**: Hash table mapping addresses to generations

**Decision**: Blood uses a global slot registry hash table.

**Rationale**:
- Separates generation metadata from user data (no header overhead per allocation)
- Enables generation tracking for externally-allocated memory (FFI)
- Hash table provides O(1) amortized lookup
- Simpler memory layout (allocations don't need special alignment for metadata)
- Registry can be shared across allocators

**Consequences**:
- Generation check requires hash lookup (~4 cycles measured) instead of adjacent load (~1-2 cycles)
- Global mutable state requires synchronization in multi-threaded contexts
- Fixed registry size limits maximum concurrent allocations
- Trade-off accepted for flexibility and FFI compatibility

---

## ADR-023: MIR as Intermediate Representation

**Status**: Accepted

**Context**: Compiler needs to transform high-level Blood code to LLVM IR. Options:
1. **Direct HIR→LLVM**: Single lowering pass
2. **MIR intermediate**: HIR → MIR → LLVM (like Rust)

**Decision**: Blood introduces MIR (Mid-level IR) between HIR and LLVM.

**Rationale**:
- MIR provides explicit control flow (basic blocks, terminators)
- Generation checks can be inserted uniformly at MIR level
- Escape analysis operates naturally on MIR's explicit temporaries
- Pattern matching compiles to decision trees in MIR
- Separates high-level transformations from low-level codegen
- Enables MIR-level optimizations (dead code elimination, inlining)

**Consequences**:
- Additional compilation phase and data structures
- MIR must faithfully represent Blood semantics
- Two codegen paths exist (legacy HIR→LLVM and new HIR→MIR→LLVM)
- Better debugging (MIR is inspectable)

---

## ADR-024: Closure Capture by Local ID Comparison

**Status**: Accepted

**Context**: Closures capture variables from their enclosing scope. Need to determine which variables are captures vs. local parameters.

**Decision**: A variable is a capture if its LocalId is numerically less than the closure's first local parameter ID.

**Rationale**:
- LocalIds are assigned sequentially during HIR lowering
- Outer scope variables get lower IDs than closure parameters
- Simple numeric comparison is fast and deterministic
- Works correctly even when IDs aren't contiguous (closures share outer ID space)

**Consequences**:
- Relies on ID assignment order (implementation detail)
- Must maintain ID ordering invariant across HIR transformations
- Simple heuristic may need refinement for nested closures
- Efficient O(1) capture detection

---

## ADR-025: Evidence Passing for Effect Handlers

**Status**: Accepted

**Context**: Algebraic effects require finding the appropriate handler at runtime when an operation is performed. Options:
1. **Dynamic lookup**: Walk the handler stack at each operation
2. **Evidence passing**: Pass handler references as implicit parameters
3. **Static compilation**: Monomorphize handlers into direct calls

**Decision**: Blood uses evidence passing based on the ICFP'21 approach.

**Rationale**:
- Evidence vectors enable O(1) handler lookup
- Compatible with deep and shallow handlers
- Supports polymorphic effect operations
- Enables tail-resumptive optimization (resume in tail position compiles to direct call)
- Handler installation is O(1) push onto evidence vector

**Consequences**:
- Functions with effects receive implicit evidence parameter
- Evidence vector threaded through all effectful computations
- Tail-resumptive handlers achieve ~1.3 cycles overhead (measured)
- Non-tail-resumptive handlers require continuation allocation (~65 cycles)

---

## ADR-026: Affine Value Checking for Multi-Shot Handlers

**Status**: Accepted

**Context**: Multi-shot handlers (like Choice) can resume continuations multiple times. Linear values (must use exactly once) cannot be duplicated. What about affine values (at most once)?

**Decision**: Affine values are allowed in multi-shot handlers; linear values are rejected at compile time.

**Rationale**:
- Affine values can be dropped, so duplication doesn't violate their contract
- Linear values cannot be dropped, so duplication would violate exactly-once semantics
- Compile-time check prevents runtime errors
- Type system tracks linearity annotations on values
- Only values captured across perform points are checked (not all values in scope)

**Implementation**:
```
multi-shot perform → check captured values → reject if any are linear
                                           → allow if all are affine/unrestricted
```

**Consequences**:
- Linear values require explicit consumption before multi-shot effect operations
- Affine resources (file handles) work naturally with Choice effect
- Error messages guide users to restructure code or change value linearity

---

## ADR-027: Generation Bypass for Persistent Tier

**Status**: Accepted

**Context**: Persistent tier (Tier 2) uses reference counting instead of generational references. Should generation checks still apply?

**Decision**: Persistent pointers bypass generation checks entirely, using a reserved "persistent" generation value.

**Rationale**:
- Persistent allocations are never freed (only decremented when count reaches zero)
- Reference counting guarantees liveness
- Generation check overhead is unnecessary for refcounted memory
- Reserved generation value (0xFFFF_FFFE) enables O(1) bypass detection
- Tier 2 allocator returns pointers with persistent generation

**Implementation**:
```
dereference(ptr):
  if ptr.generation == PERSISTENT_MARKER:
    return direct_access(ptr.address)  // No check needed
  else:
    return generation_checked_access(ptr)
```

**Consequences**:
- Persistent tier has lower access overhead than generational tier
- Tier promotion (generational → persistent) requires pointer rewriting
- Mixed allocations work correctly (some checked, some bypassed)
- ~425ps measured for persistent dereference vs ~1.27ns for generational

---

## ADR-028: Tail-Resumptive Handler Optimization

**Status**: Accepted

**Context**: Effect handlers that resume in tail position have a special structure: they don't need to capture a continuation.

**Decision**: Blood detects tail-resumptive handlers and compiles them to direct calls.

**Definition**: A handler is tail-resumptive if every operation clause ends with `resume(value)` in tail position.

**Example**:
```blood
// Tail-resumptive (optimized)
deep handler FastState for State<T> {
    op get() { resume(state) }      // resume in tail position
    op put(s) { state = s; resume(()) }  // resume in tail position
}

// Non-tail-resumptive (needs continuation)
deep handler SlowState for State<T> {
    op get() {
        let x = resume(state);  // resume NOT in tail position
        log("got");
        x
    }
}
```

**Optimization**:
```
tail-resumptive handler:
  perform op(args) → call handler_op(args) → return result → continue

non-tail-resumptive handler:
  perform op(args) → allocate continuation → call handler_op(args)
                   → resume → restore continuation → continue
```

**Consequences**:
- State, Reader, Writer effects typically tail-resumptive (near-zero overhead)
- Exception, Choice effects typically non-tail-resumptive (continuation overhead)
- Compiler detects and optimizes automatically
- No annotation required from user

---

## ADR-029: Hash Table Implementation for HashMap

**Status**: Accepted

**Context**: HashMap needs an efficient collision resolution strategy. Options:
1. Separate chaining (linked lists per bucket)
2. Open addressing (linear/quadratic probing)
3. Robin Hood hashing (displacement-based)
4. Swiss table (SIMD-accelerated)

**Decision**: Blood's HashMap uses quadratic probing with Robin Hood optimization.

**Rationale**:
- Quadratic probing avoids primary clustering
- Robin Hood balances probe distances for consistent performance
- Tombstone markers enable O(1) deletion
- 75% load factor provides good space/time tradeoff
- First tombstone optimization reduces average probe length

**Implementation Details**:
```
insert(key, value):
  idx = hash(key) & mask
  probe = 0
  first_tombstone = None
  while buckets[idx] not empty:
    if buckets[idx].key == key:
      return replace(idx, value)
    if buckets[idx] is tombstone and first_tombstone is None:
      first_tombstone = idx
    probe += 1
    idx = (idx + probe) & mask
  insert at first_tombstone or idx
```

**Consequences**:
- O(1) average case for insert/lookup/delete
- Worst case O(n) if hash function degrades
- Requires power-of-2 capacity (mask optimization)
- Automatic resizing at 75% load factor

---

## ADR-030: Two-Level Content-Addressed Compilation for Generics

**Status**: Accepted

**Context**: Blood's two core innovations — content-addressed compilation (ADR-003) and monomorphization of generics — are in architectural tension. This interaction was identified as Finding F-01 in the Design Space Audit (2026-02-28).

The tension manifests in three ways:

1. **Hash space explosion.** A generic function `fn map<T, U>(...)` instantiated with 50 type combinations produces 50 monomorphized copies. Each gets a different DefId and therefore a different content hash. The cache grows O(definitions × type combinations).

2. **Incremental invalidation cascading.** If type `Foo` changes, every monomorphized function instantiated with `Foo` must be invalidated. The invalidation set grows with the number of generic uses of the changed type.

3. **Cross-project cache failure.** Content addressing promises global cache sharing ("identical definitions produce identical hashes"). But two projects independently compiling `Vec<i32>` currently produce different DefIds, different hashes, and therefore different cache entries — breaking the global sharing promise.

**Alternatives evaluated:**

| Strategy | Runtime Cost | Cache Fit | Binary Size | Implementation Cost |
|----------|-------------|-----------|-------------|---------------------|
| **A. Full monomorphization (Rust)** | Zero dispatch | Poor — O(defs × types) artifacts | Large | Already implemented |
| **B. Witness-table dispatch (Swift)** | ~5-15% method overhead | Excellent — one artifact per definition | Small | Major refactor |
| **C. Dictionary passing (Haskell/Koka)** | ~5-15% dispatch overhead | Excellent — one artifact per definition | Small | Major refactor |
| **D. Hybrid: mono with two-level cache** | Zero dispatch | Good — structured cache | Large | Moderate extension |
| **E. Hybrid: witness default + opt-in mono** | Near-zero for hot paths | Very good — additive specializations | Controlled | Major refactor |

Key evidence:

- **OOPSLA 2022 ("Generic Go to Go")**: Quantitative comparison of dictionary passing, monomorphization, and hybrid. Conclusion: hybrids get neither the best compile time nor the best runtime performance. Choose one primary strategy and use the other selectively.

- **Swift's proven model**: Witness tables by default, `@inlinable`/`@_specialize` for opt-in monomorphization. The base (unspecialized) artifact has a stable content hash. Specializations are additive.

- **Blood's ADR-025**: Evidence passing for effects already threads implicit witness parameters. The infrastructure pattern exists.

- **LLVM CAS RFC (2022)**: Content-addressed compilation at the LLVM IR level uses per-function global reference arrays to make function bodies self-contained and hashable. Template/generic deduplication is a stated key use case.

- **Blood's priority hierarchy (ADR-010)**: Correctness > Safety > Predictability > Performance > Ergonomics. Content addressing serves correctness and predictability. Monomorphization serves performance.

**Decision**: Blood uses a **two-level content-addressed cache** with monomorphization as the primary compilation strategy, structured to preserve content-addressing guarantees.

**Level 1 — Generic definition hash (stable):**
- Content hash computed from canonicalized AST of the polymorphic definition (as today)
- Invariant: a generic definition's hash changes only when its source changes
- This level is what CONTENT_ADDRESSED.md §3-4 describes
- Enables: separate compilation, incremental recompilation, global cache sharing of the polymorphic definition

**Level 2 — Monomorphized instance hash (derived):**
- Content hash: `BLAKE3(generic_def_hash ‖ type_arg_hash₁ ‖ ... ‖ type_arg_hashₙ)`
- Invariant: an instance's hash changes only when the generic definition or any type argument changes
- DefId is NOT included in the instance hash (breaking current practice in `build_cache.rs:490-493`)
- Symbol names use the instance hash, not the DefId, enabling cross-project artifact sharing
- Enables: global cache sharing of monomorphized instances, bounded invalidation

**Level 3 — Native artifact hash (platform-specific):**
- Content hash: `BLAKE3(instance_hash ‖ target_triple ‖ opt_level)`
- Keyed by optimization level and target to support multi-target builds
- Enables: distributed build caching, reproducible artifacts

**Invalidation model:**
- Type `Foo` changes → `Foo`'s hash changes → all Level 2 entries containing `Foo`'s hash are invalidated (by dependency tracking, not by rehashing all instances)
- Generic definition changes → its Level 1 hash changes → all Level 2 entries derived from it are invalidated
- Dependency graph: Level 1 entries record which type hashes they were instantiated with. On change, the reverse index identifies affected Level 2 entries.

**Why monomorphization over witness tables:**

1. **Systems-language performance target (ADR-010, ADR-015).** Blood targets C-competitive performance. Monomorphization enables the LLVM optimizer to inline, devirtualize, and specialize — critical for tight loops over generic containers. The 5-15% overhead from witness-table dispatch is significant in Blood's target domain.

2. **Multiple dispatch already provides dynamic paths (ADR-005).** Where runtime polymorphism is desired, Blood offers trait objects (`dyn Trait`) and multiple dispatch. Adding witness tables as a second dynamic dispatch mechanism creates complexity without clear benefit.

3. **Manageable implementation cost.** Extending the existing cache from one level to two levels is less invasive than replacing the compilation model. The current `MonoRequest` infrastructure in `mir_lower_ctx.blood` and `main.blood` already handles specialization; the change is in hashing and caching, not code generation.

4. **LLVM CAS validates the approach.** The LLVM CAS design (Apple/Google/Sony/Nintendo/Meta collaboration) addresses exactly this problem for C++ templates. Their solution is content-addressed per-function artifacts with self-contained references — the same principle as Blood's two-level model.

**Why dictionary passing was rejected:**

- Blood is a systems language. The 5-15% method dispatch overhead (measured in Haskell and Swift studies) conflicts with ADR-010's priority hierarchy.
- Blood already has evidence passing for effects (ADR-025). Using dictionary passing for value-type generics would create two overlapping but distinct dictionary mechanisms.
- Dictionary passing prevents LLVM from specializing code paths based on concrete types, losing autovectorization, constant folding, and layout-specific optimizations.
- The OOPSLA 2022 "Generic Go to Go" paper found dictionary passing produces worse runtime performance in every benchmark compared to monomorphization, with compilation speed as the only advantage. Blood's two-level cache recovers the compilation speed benefit.

**Future optimization path:**

If compile times or binary size become problematic, Blood may adopt **polymorphization** (Rust's `-Zpolymorphize`): detecting generic functions where type parameters don't affect code generation and compiling them once. This is additive to the two-level model and does not require architectural change.

**Rationale**:
- Preserves zero-cost abstraction principle for generics (no runtime dispatch overhead)
- Recovers content-addressing benefits (global cache, incremental compilation, reproducible builds) through structured two-level hashing
- Uses deterministic, DefId-free instance hashing to enable cross-project artifact sharing
- Bounded invalidation via reverse dependency index prevents cascading recompilation
- Consistent with LLVM CAS direction for the broader ecosystem
- Minimal disruption to existing compiler architecture

**Consequences**:
- `build_cache.rs` must be updated: remove DefId from hash computation for monomorphized instances, add Level 2 cache keyed by `(generic_def_hash, [type_arg_hashes])`
- Symbol names must transition from DefId-based to instance-hash-based for monomorphized code
- CONTENT_ADDRESSED.md must be updated with §4.6 "Monomorphized Instance Hashing" specifying the two-level model
- Reverse dependency index needed for bounded invalidation
- Generic definitions remain hashable as today (no change to Level 1)
- Binary size is unchanged (still fully monomorphized) — future polymorphization is a separate optimization
- Cross-project sharing of common instantiations (e.g., `Vec<i32>`, `Option<String>`) becomes possible

**References**:
- OOPSLA 2022: "Generic Go to Go: Dictionary-Passing, Monomorphisation, and Hybrid" (Griesemer et al.)
- LLVM CAS RFC: "Fine-Grained Caching for Builds" (Discourse, 2022)
- ADR-003: Content-addressed code via BLAKE3-256
- ADR-010: Priority hierarchy (Correctness > Safety > Predictability > Performance > Ergonomics)
- ADR-015: AOT-first compilation model
- ADR-025: Evidence passing for effect handlers
- Design Space Audit Finding F-01 (2026-02-28)

---

## ADR-031: Tier 1 Proposal Approvals (Grammar v0.5.0)

**Status**: Accepted

**Context**: The Design Space Audit (v1.1) identified 26 proposals at "Proposed" status. Six of these affect the grammar. The Specification Work Plan (v3.0) established that grammar-affecting proposals must be evaluated before syntax alignment (Phase A) to avoid aligning every `.blood` file twice. These six proposals were triaged, evaluated against Blood's design philosophy (Five Pillars, Priority Hierarchy), and unanimously approved.

**Decision**: Approve all six Tier 1 (grammar-affecting) proposals and incorporate them into GRAMMAR.md v0.5.0:

| # | Proposal | Source | Decision | Grammar Impact |
|---|----------|--------|----------|----------------|
| #20 | Spec annotations (`requires`/`ensures`/`invariant`/`decreases`) | EF_III, SYNTAX_REDESIGN | **Approved** | Already present in v0.4.0 (`SpecClause` production) |
| — | Optional semicolons | SYNTAX_REDESIGN C.1 | **Approved** | `Statement ::= ... ';'?` with continuation rules |
| — | Canonical function signature ordering | SYNTAX_REDESIGN B.1 | **Approved** | Convention formalized: attrs → sig → effects → specs → where → body |
| #21a | Named arguments | EF_III, SYNTAX_REDESIGN C.2 | **Approved** | Already present in v0.4.0 (`Arg ::= (Ident ':')? Expr`) |
| #21b | Expression-oriented design | EF_III #21 | **Approved** | Semantic: all block-based constructs return their trailing expression's value |
| RFC-S | Granular safety controls | SAFETY_LEVELS.md | **Approved** | New `UncheckedBlock` expression, `#[unchecked(...)]` attribute |

**Evaluation criteria applied**:
1. Alignment with Blood's design philosophy (Five Pillars, Priority Hierarchy ADR-010)
2. Research quality and semantic clarity of each proposal
3. Technical debt cost of deferring vs. cost of adoption
4. Dependency analysis — no blocking dependencies exist for any Tier 1 proposal

**Rationale for each approval**:

1. **Spec annotations (#20)**: Blood's most strategically important syntax feature. Provides unambiguous AI generation targets (eliminates 20.77% of LLM misinterpretation bugs per Tambon et al. 2024). Participates in content-addressing (`spec_hash ‖ impl_hash → proof_key`). Already in grammar v0.4.0. Zero risk.

2. **Optional semicolons**: 2-3% token reduction. Only works cleanly with expression-oriented design (#21b). Well-specified continuation rules prevent ambiguity. Already partially specified in v0.4.0 (ExprWithBlock has `';'?`). Low risk.

3. **Signature ordering**: Convention codification, not new syntax. Establishes unambiguous reading order for both humans and AI. Low risk — formalizes what's already implied.

4. **Named arguments (#21a)**: Eliminates "Wrong Attribute" bug category (6.9% of LLM bugs). Optional at call sites — callers choose. No ambiguity with Blood's existing syntax (anonymous records use `#{}`). Already in grammar v0.4.0 (`Arg ::= (Ident ':')? Expr`). Low risk.

5. **Expression-oriented design (#21b)**: 5-10% token reduction. Every construct returns a value. Required for clean optional semicolons. Already partially implemented (`if`/`match` are expressions; `BlockExpr ::= '{' Statement* Expr? '}'` supports trailing expression). Proven in Rust, Scala, Kotlin, OCaml. Low risk.

6. **Granular safety controls (RFC-S)**: Replaces binary `@unsafe` with granular `unchecked(check, ...)`. Individual checks: `bounds`, `overflow`, `generation`, `null`, `alignment`. Auditable (`grep unchecked`), composable (module defaults + function overrides), effect-preserving. `@unsafe` retained for fundamentally unsafe operations (pointer dereference, type punning). Extensible — adding new check names is backward compatible. Low risk.

**Consequences**:
- GRAMMAR.md bumped to v0.5.0 incorporating all six proposals
- Phase A (syntax alignment) will target v0.5.0 grammar
- `unchecked` added to contextual keywords
- `@unsafe` retained alongside `unchecked(...)` — they serve different purposes
- Both compilers must eventually implement these features during alignment
- Downstream proposals (#7, #18) remain at "Proposed" — they consume spec annotations but are not gated by this ADR

**References**:
- Tambon et al. 2024: "Bugs in Large Language Models Generated Code" (LLM bug taxonomy)
- DESIGN_SPACE_AUDIT.md v1.1 (Tier 1 proposal classification)
- SPEC_WORK_PLAN.md v3.0 §0.2 (Proposal Triage and Approval)
- SYNTAX_REDESIGN.md (Categories B and C)
- EXTRAORDINARY_FEATURES_III.md (Proposals #20, #21)
- SAFETY_LEVELS.md (RFC-S)

---

## ADR-032: Tier 2 Proposal Approvals (Architecture-Affecting)

**Status**: Accepted

**Context**: Five Tier 2 proposals affect compiler internals, diagnostics, or tooling contracts but require no grammar changes. All five have their dependencies fully satisfied by existing infrastructure (algebraic effects, content-addressing, generational memory). Approving them as committed design direction costs nothing and signals clear design intent for Blood's tooling ecosystem.

**Decision**: Approve all five Tier 2 (architecture-affecting) proposals as committed design direction:

| # | Proposal | Source | Decision | Implementation Type |
|---|----------|--------|----------|-------------------|
| #17 | Structured diagnostics (dual human/machine) | EF_III | **Approved** | Compiler internal (incremental) |
| #8 | Deterministic simulation testing (DST) | EF_II | **Approved** | Library/stdlib pattern |
| #12 | Deterministic replay debugging | EF_II | **Approved** | Tooling (recording runtime) |
| #13 | Zero-code observability | EF_II | **Approved** | Library/stdlib pattern |
| #11 | Automatic semantic versioning | EF_II | **Approved** | Tooling (`blood semver` command) |

**Rationale**:

1. **Structured diagnostics (#17)**: Highest-priority Tier 2 proposal (ranked #3 overall). Foundation of Blood's AI-native developer experience. Every diagnostic natively structured JSON with stable error codes (public API), constraint provenance chains, and fix suggestions as structured diffs. Enables 60-80% token savings per AI fix cycle. Error codes become compatibility surface — design carefully upfront. Enables downstream #16 (constrained decoding oracle).

2. **Deterministic simulation testing (#8)**: FoundationDB-style simulation testing via effect handlers. All nondeterminism intercepted at effect boundaries. Single seed → deterministic execution → reproducible failures. Demonstrates that effects-as-universal-interception delivers concrete developer tools, not just type-system aesthetics. Library pattern — no compiler changes.

3. **Deterministic replay debugging (#12)**: Language-level time-travel debugging at 2-5% overhead (vs 15-40% for rr) by recording only effect invocations, not every instruction. Works on any platform Blood targets. Structural completeness guarantee: effects are the only nondeterminism source, so recording them captures everything.

4. **Zero-code observability (#13)**: Automatic tracing/metrics/logging via effect handler wrapping. Zero instrumentation in application code. Every effect invocation is a natural tracing span. Traces include exact code version via content hash. Switch backends by swapping handler, not rewriting application.

5. **Automatic semantic versioning (#11)**: Provably correct semver classification via content hashes and effect signatures. Hash unchanged → PATCH. Effect signature identical → PATCH. Effect widened → MINOR. Function/param removed → MAJOR. Eliminates version-misclassification bugs with mathematical certainty.

**Key insight**: Proposals #8, #12, and #13 form a coherent "effects-as-interception" triad. They demonstrate that algebraic effects are not just a type-system feature but a universal mechanism for testing, debugging, and observability — capabilities that require separate bespoke infrastructure in every other language.

**Consequences**:
- These proposals become committed design direction — Blood's tooling roadmap includes all five
- No grammar changes required (all approved proposals are compiler-internal or library/tooling work)
- Implementation priority is independent of approval — approval signals design intent, not timeline
- #17 (structured diagnostics) should be implemented first among these, as it enables #16 downstream
- #8, #12, #13 can be implemented in any order as stdlib/tooling work
- #11 requires `blood semver` CLI infrastructure

**References**:
- EXTRAORDINARY_FEATURES_II.md (Proposals #8, #11, #12, #13)
- EXTRAORDINARY_FEATURES_III.md (Proposal #17)
- PROPOSAL_ANALYSIS.md (Priority ranking and dependency analysis)
- ADR-002: Algebraic effects for all side effects
- ADR-003: Content-addressed code via BLAKE3-256
- ADR-031: Tier 1 proposal approvals

---

## ADR-033: Design Gap Resolutions (F-02 through F-10)

**Status**: Accepted

**Context**: The Design Space Audit identified seven design gaps (F-02 through F-10, excluding the already-resolved F-01 and the architectural F-06/F-07) that needed short ADRs or design notes. These are decisions where Blood inherited behavior without independent evaluation, or where the interaction between Blood's innovations wasn't documented.

### F-02: Higher-Kinded Types — Not Planned

**Decision**: Blood does not provide higher-kinded types (HKTs). Row polymorphism, algebraic effects, and multiple dispatch collectively cover the practical use cases.

**Coverage analysis**:
- **Functor/Monad abstraction** → algebraic effects (ADR-002). Blood uses effects instead of monads for sequencing side effects. No need for `trait Monad<M<_>>`.
- **Generic container abstraction** (`Collection<F<_>>`) → traits + multiple dispatch (ADR-005). Functions parameterized by container type use trait bounds, not type constructor parameters.
- **Iterator/Stream abstraction** → trait-based (`Iterator` trait with associated `Item` type). No HKTs needed.
- **Effect handler abstraction** → effect handlers are structurally higher-kinded (transform `Comp<E> → Comp<E'>`) but this is built into the language, not exposed as a user-level HKT mechanism.

**Known gap**: Cannot abstract over type constructors directly (e.g., `fn map<F<_>, A, B>(fa: F<A>, f: fn(A) -> B) -> F<B>`). This is intentional — such abstractions are rare outside Haskell/Scala and add significant type system complexity. If a gap is discovered in practice, `forall` types (higher-rank polymorphism) provide a partial substitute. Full HKTs can be added in a future edition if needed, as an additive change.

### F-03: Variance — Invariant by Default

**Decision**: All type parameters are invariant by default. No user-facing variance annotations.

**Rationale**: Invariance is always sound. Covariance for `&T` and contravariance for `fn(T)` can be inferred by the compiler as an optimization (following Rust's model) without user-facing syntax. Blood's generational references (not borrowed references) make variance less critical than in Rust — there's no lifetime parameter variance to reason about.

**Future path**: If ergonomic demand arises, the compiler can infer variance for read-only type parameters. This is additive and requires no language change.

### F-04: String and Slice Representation

**Decision**: `&str` and `&[T]` use **thin fat pointers**: `{ ptr, i64 }` (16 bytes). Generational checking applies to owned references (`&T` = 128-bit generational pointer), not to slice/string data pointers.

**Current implementation**: This is already how the codegen works (`codegen_size.blood`, `codegen_types.blood`). Slices store a raw data pointer + length. The generational check occurs when the slice is *created* from an owned reference, not on every element access.

**Rationale**: Slice data is contiguous memory — checking the generation of the owning allocation at slice creation time is sufficient. Per-element generation checks would impose unacceptable overhead for iteration. This matches Vale's model: generation checks at reference creation, not at every dereference.

**Consequence**: `&str` is 16 bytes (same as Rust). `&[T]` is 16 bytes (same as Rust). `&T` is 16 bytes (128-bit generational pointer — 2x Rust's 8-byte `&T`). The 2x overhead on thin references is the documented cost of generational safety (ADR-001).

### F-05: Result/Option Alongside Effects

**Decision**: `Result<T, E>` and `Option<T>` coexist with algebraic effects. They serve complementary roles:

| Mechanism | Use When | Propagation | Resumable? |
|-----------|----------|-------------|-----------|
| `Result<T, E>` + `?` | Expected failures in the function's contract. Caller must handle. | `?` propagates to immediate caller | No |
| `Option<T>` | Value may or may not exist. Not an error. | Pattern match | No |
| `perform Error.raise(e)` | Structured error handling with handler-provided recovery. | Effect propagation to enclosing handler | Yes (via `resume`) |

**Interconversion**: `Result` can be converted to an effect (`raise` on `Err`) and effects can be caught into `Result` (handler that wraps in `Ok`/`Err`). Library functions should provide both APIs where appropriate.

**Guidance for library authors**: Use `Result` for leaf functions (parsers, validators, lookups). Use effects for orchestration functions where callers benefit from handler-based recovery, dependency injection, or testing.

**`?` does not cross effect boundaries**: `?` propagates `Result` errors up the call stack. It does not interact with effect handlers. To convert between `Result` and effects, use explicit conversion functions.

### F-08: Standard Library Scope

**Decision**: Blood's standard library follows a three-tier model:

| Tier | Name | Requires | Contents |
|------|------|----------|----------|
| `core` | Core | Nothing (freestanding) | Primitives, traits, Option, Result, iterators, formatting, math |
| `alloc` | Allocation | Allocator | Vec, String, Box, HashMap, Arena, sorting |
| `std` | Standard | OS | IO, filesystem, networking, process, threading, time |

**Current state**: Blood's 8 stdlib modules (HashMap, HashSet, Arena, Sort/Search, fmt, String, Math, prelude) all fit in the `alloc` tier. No `core` or `std` split exists yet.

**Regions interaction**: Region-based allocation (`@stack`, `region`) is available at all tiers. `@heap` requires the `alloc` tier. The allocator trait is defined in `core` but implemented in `alloc`.

**Implementation**: The split is organizational, not urgent. Current stdlib is small enough that a monolithic `std` works. The split becomes necessary when targeting embedded/freestanding environments.

### F-09: Testing as a Language Feature

**Decision**: Testing is a first-class language feature using existing infrastructure:

1. **`#[test]` attribute**: Marks functions as tests. Already in GRAMMAR.md §1.5.1 standard attributes. Not yet implemented in compilers.
2. **`blood test` command**: Discovers and runs `#[test]` functions. Future CLI addition.
3. **`assert!` / `assert_eq!` macros**: Built-in assertion macros. Basic `assert!` already exists.
4. **Effect-based simulation testing**: Proposal #8 (ADR-032) provides DST via effect handlers — this is a library pattern, not a compiler feature.
5. **`#[should_panic]`**: Already in GRAMMAR.md §1.5.1.

**What testing is NOT**: Testing does not require new syntax beyond the existing `#[test]` attribute. No `test` block keyword. No special test modules. Tests are regular functions with `#[test]`, co-located with the code they test (following Rust's model).

**Implementation priority**: `#[test]` attribute recognition → `blood test` runner → `assert_eq!` macro → integration with DST handlers.

### F-10: ABI Stability

**Decision**: Blood's ABI is **explicitly unstable**. No cross-compiler-version binary compatibility is guaranteed.

**Rationale**: Content-addressed compilation (ADR-003) provides a stronger alternative to traditional ABI stability. If two compilation units share the same content hash, they are guaranteed to be compatible — not because the ABI is stable, but because they are *identical*. This gives Blood the benefits of ABI stability (artifact reuse, caching) without the costs (constraining struct layout, vtable format, calling conventions).

**Consequence**: Libraries are distributed as source (or as content-addressed artifacts for exact compiler versions). The compiler is free to change struct layout, calling conventions, and vtable format between versions. Static linking is the default. Dynamic linking requires version-locked shared libraries.

**Future**: If dynamic linking across compiler versions becomes necessary, a `#[repr(C)]` escape hatch already exists for C-compatible layout. A future `#[repr(stable)]` attribute could opt specific types into layout stability, but this is not planned.

---

## ADR-034: Minimal-Effort Defaults

**Status**: Accepted

**Context**: Seven design axes were identified as "defaulted" — the language behaves a certain way without an explicit decision having been made. Each gets a one-paragraph decision record.

1. **Cyclic imports**: **Forbidden.** Content-addressed compilation requires a DAG of definitions. Cyclic imports would create circular hash dependencies, violating the content-addressing model (ADR-003). The compiler already rejects cyclic `mod` declarations. This is a permanent constraint, not a temporary limitation.

2. **Interior mutability**: **Deferred.** No `Cell`/`RefCell` equivalent is designed. Blood's generational references validate at creation time, not at every access, which changes the interior mutability story compared to Rust. If needed, interior mutability can be provided via effect handlers (a `State` effect provides controlled mutation). Design this when a concrete use case demands it.

3. **Dead code detection**: **Yes, as compiler warning.** The compiler should warn on unreachable code, unused variables, unused imports, and unused functions. This is standard practice and requires no language design — it's a compiler quality issue implementable via MIR analysis.

4. **Definite initialization**: **Statically enforced via MIR analysis.** All variables must be initialized before use. The MIR phase already tracks local initialization status. Uninitialized reads are compile errors, not runtime errors. This matches Rust and is already partially implemented.

5. **Doc comment syntax**: **`///` (triple-slash).** Already specified in GRAMMAR.md §1.1 (`DocComment ::= '///' [^\n]* '\n'`). Consistent with Rust. Doc comments are preserved for tooling. No change needed.

6. **Frame pointer preservation**: **On by default.** Frame pointers enable profiling and debugging. The overhead is one register per function. In release builds, `#[optimize(size)]` or a future flag can disable frame pointers. Default-on follows Go's approach.

7. **Variance**: Resolved by F-03 above — invariant by default, compiler-inferred where safe.

---

## ADR-035: Inherited Decision Confirmations

**Status**: Accepted

**Context**: Eight design decisions were adopted from Rust without independent evaluation in Blood's context. Each is confirmed, revised, or noted as already resolved.

| Decision | Verdict | Notes |
|----------|---------|-------|
| Monomorphization | **Already resolved** | ADR-030 (two-level content-addressed cache) |
| `Option<T>` / `Result<T, E>` | **Confirmed with guidance** | F-05 above — coexist with effects, serve complementary roles |
| UTF-8 strings | **Confirmed** | F-04 above — `&str` = `{ ptr, i64 }` (16 bytes). UTF-8 is the universal standard. No reason to deviate. |
| File-based module hierarchy | **Confirmed** | Content addressing operates at definition level; files are a developer-facing organization convention. `mod name;` loads `name.blood` from the same directory. Files remain the authoring interface even though content hashes are the identity mechanism. |
| `pub` visibility (Rust-style) | **Confirmed** | `pub`, `pub(crate)`, `pub(super)` work as in Rust. Row polymorphism is orthogonal — it operates on record/effect types, not module visibility. Structural subtyping doesn't bypass `pub` boundaries; a module's exported interface is its public items, regardless of whether those items use structural types. |
| Call-by-value evaluation | **Confirmed** | Strict/eager evaluation is the right default for a systems language with algebraic effects. Effect handlers receive values, not thunks. Lazy evaluation would require boxing and allocation at every effect boundary. |
| No runtime type information | **Revised** | Blood DOES have lightweight RTTI: 24-bit type fingerprints stored in the metadata field of generational pointers. This enables multiple dispatch (ADR-005) at runtime. However, this is NOT reflective RTTI — programs cannot enumerate fields, invoke methods by name, or inspect type hierarchies. The correct characterization is: "minimal RTTI for dispatch, no reflection." |
| `&T` / `&mut T` reference syntax | **Confirmed with distinction** | Same syntax as Rust but different semantics. Blood's `&T` is a 128-bit generational pointer (not a borrowed reference). No borrow checker, no lifetime annotations on references (regions handle scoping). The syntax is familiar but the mental model is different — documented in ADR-001. |
| Binary `unsafe` blocks | **Superseded** | ADR-031 approved RFC-S: `@unsafe` retained for fundamentally unsafe operations; `unchecked(checks)` added for granular check disabling. Binary `unsafe` is no longer the only safety escape hatch. |

---

## ADR-036: Concurrency Model — Effect-Based Structured Concurrency (F-06)

**Status**: Accepted

**Context**: The Design Space Audit (F-06) identified that Blood has all the pieces for a concurrency model — algebraic effects, fiber runtime, handler scoping, regions, linear types — but hadn't composed them into a cohesive language-level design. Eight sub-decisions were defaulted without independent evaluation. This ADR resolves all eight from first principles, reasoning from Blood's design goals, specification documents, and external research (Koka, OCaml 5/Eio, Effekt OOPSLA 2025, Trio, Kotlin coroutines, Swift structured concurrency, Java Loom).

**Methodology**: Design-first (SPEC_WORK_PLAN.md §3). All decisions reason from Blood's goals and external research. Existing implementations are irrelevant to these design choices.

**Design Principles Governing All Sub-decisions**:
1. All side effects visible in function signatures (SPECIFICATION.md §1)
2. Effects are bidirectional — handlers resume once, multiple times, never, or later (SPECIFICATION.md §4.1)
3. Region isolation — each fiber has its own memory region (SPECIFICATION.md §6.1)
4. Linear ownership transfer — mutable data moves between fibers (SPECIFICATION.md §6.2)
5. Immutable sharing via `Frozen<T>` (SPECIFICATION.md §6.3)
6. No colored functions — concurrency is an effect, not a type bifurcation
7. Linear values cannot cross effect suspension points without explicit transfer (SPECIFICATION.md §4.5)
8. Generation snapshots validate references on handler resume (SPECIFICATION.md §4.5)

### Preliminary: Naming — `Fiber`, Not `Async`

**Decision**: The concurrency effect is named `Fiber`. SPECIFICATION.md §4.6 and §6 (which use `Async`) must be updated for consistency with CONCURRENCY.md.

**Rationale**: Blood avoids the colored-function problem by NOT having an async/sync distinction. Calling the effect `Async` imports the very framing Blood rejects. `Fiber` names the concurrency unit (what the programmer works with). `await` implies a Future/Promise model; Blood uses effect operations — a fiber *joins* another fiber, it doesn't *await* a future.

### Sub-1: Structured Concurrency — Handler Scope = Task Scope

**Decision**: Every `spawn` occurs within an effect handler scope. An unscoped `spawn` is a compile error (the `Fiber` effect must be handled). The handler scope defines the structured concurrency boundary: all spawned fibers must complete before the handler scope exits.

**Rationale**: In Blood's effect system, `with handler handle { computation }` defines a lexical scope where effect operations are intercepted. If `spawn` is a `Fiber` effect operation, then the handler that intercepts `spawn` manages the spawned fiber's lifetime. When the handler scope exits, all spawned fibers must have completed. Handler scope = task scope falls directly from effect handler semantics — it is not a library invariant but a structural consequence.

**Deep vs. shallow handler semantics**: Deep and shallow handlers have distinct concurrency roles:

| Pattern | Handler Type | Property |
|---------|-------------|----------|
| Nursery (supervise all children) | Deep | Automatically intercepts all spawns in the subtree — cannot be escaped |
| One-shot spawn-and-join | Shallow | Handles exactly one spawn, gets result |
| Spawn with inspection (middleware) | Shallow + explicit re-install | Inspects each spawn before proceeding |
| Supervisor (isolate failures) | Deep + per-child error handling | Deep catches all spawns; handler logic isolates failures |

**Key formal property**: A deep `Fiber` handler provides airtight structured concurrency. Because deep handlers automatically re-wrap the continuation, nested spawns (spawns from children, grandchildren, etc.) are all intercepted. No nested computation can spawn outside the handler's supervision. This is strictly stronger than Trio/Kotlin/Swift where structured concurrency is a library invariant.

**Convenience abstractions** (nursery, scope, par_map, supervisor) are library-level handlers with specific policies. They are not special constructs.

### Sub-2: Cancellation — Separate `Cancel` Effect

**Decision**: Cancellation is modeled as a separate `Cancel` effect, distinct from `Fiber`:

```blood
effect Cancel {
    op check_cancelled() -> unit
}
```

**Cancellation protocol**:
1. A parent scope requests cancellation of a child (sets a flag)
2. The child's `Cancel` handler checks the flag when `check_cancelled()` is performed
3. If cancelled, the handler does NOT resume the child's continuation — the child terminates
4. If not cancelled, the handler resumes normally
5. Cancellation only occurs at explicit `check_cancelled()` points — it is cooperative

**Rationale**: Separating `Cancel` from `Fiber` provides visibility in types: `fn work() / {Fiber, Cancel}` explicitly declares cancellation points; `fn work() / {Fiber}` runs to completion. This follows Blood's principle that all behaviors are visible in signatures.

**Tradeoff acknowledged**: The `Cancel` effect creates a capability distinction — cancellable (`/ {Fiber, Cancel}`) vs non-cancellable (`/ {Fiber}`) functions. This is intentional and differs from async coloring:

| Property | Async coloring | Cancel capability |
|----------|---------------|-------------------|
| Propagation | Upward (viral) | Upward (like any effect) |
| Can be eliminated? | No | Yes — handle `Cancel` at any scope |
| Callability | Async cannot call sync | Cancellable CAN call non-cancellable |
| Opt-out | Impossible | Handle `Cancel` to create non-cancellable scope |

The asymmetry: `Cancel` can be *handled* (eliminated) at any scope boundary, converting a cancellable computation to a non-cancellable one. Async coloring propagates upward and cannot be eliminated. This is why Cancel-as-effect avoids the coloring problem.

### Sub-3: Cancellation Safety — Safe by Construction

**Decision**: Blood guarantees compile-time cancellation safety through the combination of regions, linear types, and handler finalization.

**Guarantees**:
1. **Memory safety**: Regions are fiber-local (SPECIFICATION.md §6.1, MEMORY_MODEL.md Theorem 5). When a fiber is cancelled, its regions are bulk-deallocated (O(1)). No other fiber holds references into them.
2. **Resource safety**: Linear values must be consumed exactly once (SPECIFICATION.md §3.3). If a linear value is live when cancellation occurs, the compiler ensures cleanup code consumes it, or rejects the program.
3. **Handler finalization**: When cancellation discards a continuation, all nested handler scopes unwind. Each handler's `finally` clause (Sub-4) runs in reverse order.
4. **No cross-fiber corruption**: Region isolation prevents any cancelled fiber from affecting another fiber's state.

**Comparison**:

| Language | Memory cleanup | Resource cleanup | Cross-task corruption prevention |
|----------|---------------|-----------------|--------------------------------|
| Rust | Drop (sync only) | No async cleanup | Send/Sync (opt-in, unsafe overridable) |
| Kotlin | GC | `finally` (CancellationException) | Coroutine scope (runtime) |
| Swift | ARC | `defer` (sync only) | Actor isolation (compile-time) |
| Blood | Region bulk dealloc (O(1)) | Linear types (compiler-enforced) | Region isolation (structural, not overridable) |

Blood's guarantees are structural — they come from the type system, not from runtime checks or programmer discipline.

### Sub-4: Async Drop — Sidestep via Regions + Linear Types + `finally`

**Decision**: Blood sidesteps the "async drop" problem through three mechanisms:

1. **Memory doesn't need destructors** — regions provide O(1) bulk deallocation. No per-object finalizers for memory management.
2. **Resources use linear types** — a `linear Connection` cannot be implicitly dropped. The compiler rejects code that lets a linear value go out of scope unconsumed. The programmer MUST call `connection.close()`, which can be an effect operation (and therefore can suspend).
3. **Handler finalization via `finally`** — for resources managed by handlers, a `finally` clause runs when the handler scope exits regardless of exit reason.

**Grammar change** (GRAMMAR.md v0.6.0):

```ebnf
HandlerBody ::= HandlerState* ReturnClause? FinallyClause? OperationImpl*
FinallyClause ::= 'finally' Block
```

`finally` is already a reserved keyword (GRAMMAR.md §9.3). Semantics:
- `return(x) { ... }` — runs on normal completion (existing)
- `finally { ... }` — runs on ANY exit (normal + abnormal/cancellation)
- When both present: normal exit runs `return` then `finally`; abnormal exit runs `finally` only
- Nested handlers: `finally` clauses run in reverse nesting order (innermost first)

**Example**:

```blood
deep handler ManagedDB for Database {
    let conn: linear Connection

    finally {
        conn.close()  // Runs on scope exit regardless of reason
    }

    return(x) { x }

    op query(sql) {
        let result = conn.execute(sql)
        resume(result)
    }
}
```

**`finally` clause effect semantics**: A `finally` clause executes in the *enclosing* handler context — the handler stack outside the handler being finalized. The clause may perform effects handled by enclosing scopes, but NOT effects handled by the handler being torn down (that handler is being destructed).

Formally: if handler H handles effect E, then H's `finally` clause may perform effects from `(EffectRow \ {E})`.

**`finally` clauses are non-cancellable**: When a `finally` clause executes, the `Cancel` handler is not installed around it. Any `check_cancelled()` within `finally` is an unhandled effect — a compile error. Cleanup that must happen cannot be cancelled. This matches Java's `finally` (not interruptible) and Kotlin's `NonCancellable`, but is enforced structurally by the type system rather than by opt-in.

**Provenance**: The Effekt research (OOPSLA 2025, "Dynamic Wind for Effect Handlers") formally verifies that finalization clauses compose correctly with effect handlers, including nested effects and cancellation. `finally` is equivalent to Effekt's `on_suspend + on_return` (the paper proves this). More specialized lifecycle hooks (`on_suspend`, `on_resume`) can be added later if needed.

### Sub-5: Send/Sync — Auto-Derived Traits from Memory Tier

**Decision**: `Send` and `Sync` are auto-derived traits, not effects. A type is `Send` if all its fields are `Send`. A type is `Sync` if it is `Frozen` or `Synchronized`.

**Derivation rules from Blood's memory model**:

| Memory Tier | Send? | Sync? | Reason |
|-------------|-------|-------|--------|
| Tier 0 (stack) | Yes (if fields Send) | No | Stack-local, no sharing |
| Tier 1 (region), mutable | No | No | Region-local, fiber-private |
| Tier 1 (region), Frozen | Yes | Yes | Deeply immutable |
| Tier 2/3 (persistent) | Yes | Yes | Ref-counted, designed for sharing |
| Linear values | Yes (via transfer) | No | Unique ownership, no aliasing |

The `spawn` operation requires `Send` bounds:

```blood
op spawn<T: Send>(f: fn() -> T / {Fiber} + Send) -> FiberHandle<T>
```

**Rationale**: `Send`/`Sync` describe structural type properties (whether fields are sendable/sharable), not runtime behaviors. Effects describe runtime behaviors (suspension, I/O, mutation). Conflating type properties with runtime effects muddies both systems.

Blood's memory model provides a stronger foundation than Rust's `Send`/`Sync`:
- In Rust, `Send` is an `unsafe` trait — it can be incorrectly implemented
- In Blood, `Send` is derived from the memory tier: region references are structurally not `Send`. There is no way to "opt in" a region reference because the type system prevents cross-fiber region access at a fundamental level

### Sub-6: Streams — Effect Composition (`Yield<T>` + `Fiber`)

**Decision**: Streams are the natural composition of `Yield<T>` (produce values) and `Fiber` (suspend between them). No new abstraction needed.

```blood
fn sensor_readings() / {Yield<Reading>, Fiber} {
    loop {
        let reading = read_sensor()
        yield(reading)
        sleep(Duration::seconds(1))
    }
}
```

**Backpressure**: With effect-based streams, backpressure is implicit — the `Yield<T>` handler controls when to resume the producer. Delaying resumption = backpressure. With channel-based streams, backpressure is explicit via bounded channel capacity.

**Design principle**: Blood's effect system is compositional. New patterns emerge from combining existing effects, not from adding new ones. Both effect-based and channel-based streams are library patterns requiring no new syntax.

### Sub-7: Runtime vs Library — Runtime Substrate + Effect Model

**Decision**: The runtime provides the substrate; effect handlers provide the concurrency model.

**Must be runtime-provided** (cannot be expressed in Blood):
- OS thread management, fiber context switching, I/O multiplexing (epoll/kqueue/IOCP/io_uring), timer management, fiber stack allocation (mmap, guard pages)

**Must be library-defined** (effect handlers):
- Scheduling policy, structured concurrency semantics, cancellation protocol, channel semantics, select/await combinators

This enables:
- **DST** (Proposal #8): A deterministic handler replaces the real scheduler — same `Fiber` effect, different handler
- **Replay debugging** (Proposal #12): A recording handler wraps the real scheduler
- **Testing**: A blocking handler runs everything sequentially
- **Custom schedulers**: Domain-specific handlers for latency-sensitive, throughput-optimized, or real-time workloads

### Sub-8: Fiber ↔ OS Thread — Fibers as Primary Abstraction

**Decision**:
1. Fibers are the primary and only concurrency abstraction. Users spawn fibers, never OS threads directly. Raw thread creation requires FFI.
2. M:N scheduling is a handler implementation detail. The default `Fiber` handler maps M fibers to N worker threads (N = core count).
3. `spawn_blocking` for FFI interop — runs a closure on a dedicated OS thread outside the fiber scheduler:

```blood
effect Fiber {
    // ... existing operations ...
    op spawn_blocking<T: Send>(f: fn() -> T + Send) -> FiberHandle<T>
}
```

4. Thread affinity via configuration (`FiberConfig`), not a first-class concept. Optimization hint, not a correctness requirement.
5. No language-level thread pool API. The number of worker threads is a runtime configuration concern.

### Cross-Cutting: Multiple Dispatch in Concurrency

**Decision**: Multiple dispatch specializes concurrency operations by type, leveraging Blood's unique combination of dispatch + effects.

**Tier-based spawn dispatch**: The `spawn` operation dispatches on capture types to select allocation strategy:

```blood
// Region-local captures → lightweight fiber environment
fn spawn(f: fn() -> T / {Fiber}) -> FiberHandle<T>
    where T: Send, captures(f): RegionLocal { ... }

// Persistent captures → standard fiber environment
fn spawn(f: fn() -> T / {Fiber}) -> FiberHandle<T>
    where T: Send, captures(f): Persistent { ... }
```

If `captures(f)` type-level extraction is too advanced for initial implementation, `spawn` and `spawn_heavy` as explicit `Fiber` effect operations provide the same dispatch with simpler typing. Size-based dispatch (e.g., `where size_of(captures(f)) <= 64`) is noted as a future optimization enabled by richer const evaluation.

**Channel transfer dispatch**: Send/receive dispatches on message type for optimal transfer strategy:
- Region-compatible types → zero-copy transfer (move region ownership)
- Small value types → copy transfer
- Frozen types → shared reference transfer

**Observability dispatch** (Proposal #13): Tracing handlers can specialize per effect type via multiple dispatch.

### Cross-Cutting: Content Addressing in Concurrency

**Decision**: Content addressing interacts with concurrency in three specified ways:

1. **Handler composition identity**: Handler *definitions* (code) are content-addressed, not handler *instances* (code + runtime state). The composition hash `H(H_fiber || H_cancel || H_traced)` identifies the behavioral code. Different instances of the same handler definition share verification proofs (Proposal #18). Runtime state is not content-addressable.

2. **Deterministic replay format** (Proposal #12): Effect traces record `[(handler_def_hash, operation_name, args_hash, result_hash), ...]`. Content addressing makes traces structurally stable across refactors (handler renames don't change hashes). Replay reinstalls handlers by matching definitions, not instances.

3. **Pure fiber deduplication** (future optimization): If two fibers spawn identical pure computations (same function hash, same captured value hashes, `/ pure` effect row), the scheduler MAY deduplicate them (compute once, share result). This connects to Proposal #3 (automatic memoization).

### Cross-Cutting: Generation Snapshot Cost Model

**Decision**: Generation snapshots during fiber context switching use bulk region-level comparison, not per-reference comparison.

**Specification**: Each fiber maintains a `RegionSnapshot = Vec<(RegionId, Generation)>` captured at suspend, validated at resume. The snapshot tracks only mutable Tier 1 regions:

| Tier | In snapshot? | Reason |
|------|-------------|--------|
| Tier 0 (stack) | No | Stack frames are fiber-local by construction |
| Tier 1 (region), mutable access | Yes | May be invalidated during suspension |
| Tier 1 (region), Frozen access | No | Immutable — generation counter never advances |
| Tier 2/3 (persistent) | No | Uses refcounting, not generations |

**Cost**: O(R_mutable) where R_mutable is the count of mutable Tier 1 regions the fiber holds references into. For the vast majority of fibers, R_mutable = 1 (the fiber's own region). This is effectively O(1). Validation is a single integer comparison per snapshot entry.

### Cross-Cutting: Fairness, Fiber-Local Storage, Priority Inversion

**Fairness and starvation**: The default deep `Fiber` handler provides cooperative fairness — fibers yield at effect operation boundaries; the scheduler uses round-robin among ready fibers. For pure computation loops that perform no effects, the compiler inserts safepoints at loop back-edges and function prologues (matching Go 1.14's approach). Safepoint cost is one instruction per site (~1 cycle, branch predicted not-taken):

```llvm
; At each safepoint
%preempt = load i8, ptr %fiber.preempt_flag
%should_yield = icmp ne i8 %preempt, 0
br i1 %should_yield, label %yield_point, label %continue
```

The `#[unchecked(preemption)]` attribute (extending RFC-S) disables safepoint insertion in performance-critical code, with the programmer accepting starvation risk. This is chosen over signal-based preemption (SIGALRM) because safepoints are more predictable and don't interact with FFI signal handlers.

**Fiber-local storage**: Modeled as a `State` effect scoped to the fiber's handler lifetime:

```blood
deep handler FiberLocal<T> for State<T> {
    let value: T

    return(x) { x }
    op get() { resume(value) }
    op set(new_val) { value = new_val; resume(()) }
}
```

The `State<T>` operations (`get`, `set`) are tail-resumptive, so ADR-028's optimization applies — fiber-local access compiles to a direct memory read with zero effect dispatch overhead. This is both principled (visible in types as `/ {State<Config>}`) and zero-cost.

**Priority inversion**: Priority inheritance is the default policy in the standard scheduler handler — when a high-priority fiber joins a low-priority fiber's result, the low-priority fiber inherits the higher priority. Priority ceiling for linear values (the resource carries a priority ceiling as a type-level property) is available for real-time handlers. Both are handler implementation choices, not language-level changes. These connect to Proposal #1 (WCET analysis) for safety-critical domains.

### Grammar Impact

| Sub-decision | Grammar Change? | Details |
|-------------|----------------|---------|
| 1. Structured concurrency | No | Falls from effect handler semantics |
| 2. Cancellation | No | `Cancel` defined in stdlib |
| 3. Cancellation safety | No | Falls from regions + linear types |
| 4. Async drop / handler finalization | **Yes** | `FinallyClause` in handler syntax |
| 5. Send/Sync | No | Auto-derived traits |
| 6. Streams | No | Effect composition |
| 7. Runtime vs library | No | Architectural principle |
| 8. Fiber ↔ OS thread | No | `spawn_blocking` is a Fiber operation |

**One grammar change**: `FinallyClause ::= 'finally' Block` added to `HandlerBody`. Requires GRAMMAR.md v0.6.0. `finally` is already reserved (§9.3) and moves to contextual keyword status.

### Five-Pillar Leverage Summary

| Pillar | Concurrency Role |
|--------|-----------------|
| **Effects** | `Fiber`, `Cancel`, `Yield` — concurrency as effect composition |
| **Handlers** | Deep/shallow = supervision patterns; `finally` = cleanup; handler scope = task scope |
| **Regions** | Fiber-local memory, O(1) bulk dealloc on cancellation, generation snapshots O(R_mutable) |
| **Linear types** | Cancellation safety, resource cleanup enforcement, ownership transfer |
| **Multiple dispatch** | Spawn strategy, channel transfer, observability specialization |
| **Content addressing** | Handler composition hashing, deterministic replay, pure fiber deduplication |

**Consequences**:
- SPECIFICATION.md §4.6, §6 must be updated: `Async` → `Fiber`
- GRAMMAR.md v0.6.0: `FinallyClause` production added to `HandlerBody`
- CONCURRENCY.md v0.4.0: Updated with the cohesive model defined here
- FORMAL_SEMANTICS.md: `finally` clause typing rules, non-cancellability rule
- `finally` moves from reserved keyword (§9.3) to contextual keyword (§9.2)
- Compiler-inserted safepoints at loop back-edges and function prologues (codegen requirement)
- `#[unchecked(preemption)]` added to RFC-S's check list

---

## ADR-037: Compiler-as-a-Library — Content-Hash-Gated Query Architecture (F-07)

**Status**: Accepted

**Context**: The Design Space Audit (F-07) identified that Blood's self-hosted compiler is a monolithic pipeline (source in, binary out) that does not exploit the content-addressed compilation model for incremental re-analysis, partial program processing, or external tool integration (LSP, verification tools, AI oracles). The reference design literature warns that retrofitting query-based architecture for LSP support is "extremely expensive." This ADR specifies the target architecture before the compiler API boundaries solidify further.

**Methodology**: Design-first (SPEC_WORK_PLAN.md §3). Research: Salsa framework (rust-analyzer), Roslyn red-green trees, rustc query system, Sixten query driver, Unison content-addressed model, matklad's "against query-based compilers" analysis (Feb 2026). All decisions reason from Blood's unique features, not from existing compiler implementation.

### Blood's Unique Advantages

Blood has three properties that fundamentally shape its compiler architecture strategy:

1. **Content-addressed definitions (ADR-003)**: Every definition is identified by BLAKE3-256 hash of its canonicalized AST. This provides a stronger invalidation signal than rustc's post-execution fingerprints or Salsa's memoization — if the hash is unchanged, ALL downstream work can be skipped before any query executes.

2. **Explicit effect signatures**: Effect rows are part of function signatures. This makes the "body changes don't cascade" invariant structural: changing a function body without changing its signature (including effect row) cannot invalidate callers' type information. This is the critical firewall that makes per-definition incrementality effective.

3. **Three-level cache model (CONTENT_ADDRESSED.md §4.6)**: Generic definition hash → monomorphized instance hash → native artifact hash. This hierarchy maps directly to query tiers.

### Design Decision: Content-Hash Gating as Primary Incrementality Mechanism

**Decision**: Blood's compiler architecture uses BLAKE3 content hashing as its primary incrementality mechanism, with a query-based architecture as an additive optimization for IDE scenarios. This is the inverse of rustc's approach (query engine primary, fingerprints secondary) and is more aligned with Unison's model.

**Key principle**: Content hash is computed BEFORE query execution (from canonicalized AST). If unchanged, skip all downstream work entirely. In rustc, fingerprints are computed AFTER query execution as a change-detection proxy. Blood's approach is strictly cheaper — it avoids even invoking the query engine for unchanged definitions.

### Query Granularity: Per-Definition

**Decision**: Per-definition (keyed by DefId) is the natural query granularity. Per-file is too coarse (a single changed function recompiles the entire file). Per-expression is too fine (excessive overhead, low benefit). Per-definition aligns with content-addressed hashing (each definition has its own BLAKE3 hash).

Per-file serves as the coarse-grained input layer (source text, parsing). Per-definition handles all semantic analysis. Whole-program queries handle name resolution, dispatch tables, and linking.

### Natural Query Boundaries

**Tier 1: Per-File (input layer)**

| Query | Input | Output |
|-------|-------|--------|
| `source_text(FileId)` | File path | String (Salsa input) |
| `parsed_file(FileId)` | Source text | AST + diagnostics |
| `item_tree(FileId)` | Parsed AST | Condensed signature summaries (strips bodies) |

**Tier 2: Per-Definition (semantic layer, keyed by DefId)**

| Query | Input | Output | Notes |
|-------|-------|--------|-------|
| `content_hash(DefId)` | Canonicalized AST | BLAKE3Hash | **Early-exit gate** |
| `hir_of(DefId)` | AST item | HIR item | AST-to-HIR lowering |
| `fn_sig(DefId)` | HIR function | FnSig (params, return type, effect row) | Signature extraction — the critical firewall |
| `effects_of(DefId)` | HIR item | EffectRow | **Blood-specific** — effect annotations |
| `type_of(DefId)` | HIR item + resolved names | Type | Type inference for items |
| `typeck(DefId)` | HIR body + types of dependencies | TypeckResults | Full body type checking |
| `mir_built(DefId)` | Typechecked HIR body | MIR Body | MIR construction |
| `optimized_mir(DefId)` | Built MIR | Optimized MIR | MIR optimization passes |
| `codegen_of(DefId)` | Optimized MIR | LLVM IR fragment | Per-definition codegen |

**Tier 3: Whole-Program**

| Query | Input | Output |
|-------|-------|--------|
| `resolve_names(ModuleId)` | Item trees of all files in module | Resolved names/imports |
| `dispatch_table(TraitId)` | All impl blocks for trait | Dispatch table (ADR-005) |
| `link(CrateId)` | All codegen results | Final binary |

### The Signature/Body Firewall

**The critical architectural invariant**: Changing a function body NEVER invalidates other functions' type-level information, provided the signature (including effect row) is unchanged.

This is enforced by the query dependency structure:
- `fn_sig(def)` depends only on the definition's HIR signature, not its body
- `typeck(caller)` depends on `fn_sig(callee)`, not `typeck(callee)` or `mir_built(callee)`
- If `content_hash(def)` changes but `fn_sig(def)` produces the same result (backdating), no caller re-checks

Blood's explicit effect rows strengthen this firewall: effect signatures are part of `fn_sig`, so effect changes cascade correctly, but body-only changes that don't alter the effect row are isolated.

### Cache Integration with CONTENT_ADDRESSED.md

The three-level cache model (§4.6) maps to query tiers:

| Cache Level | Query Tier | Cache Key | Invalidation |
|-------------|-----------|-----------|-------------|
| L1: Generic definition hash | Tier 2 | `BLAKE3(canonicalize(generic_def))` | Source change to this definition |
| L2: Monomorphized instance hash | Tier 2 (mono) | `BLAKE3(generic_hash ‖ type_arg_hashes)` | Definition or type argument change |
| L3: Native artifact hash | Tier 2 (codegen) | `BLAKE3(instance_hash ‖ target ‖ opt_level)` | Instance, target, or optimization change |

**Content-hash gating algorithm**:
```
query(def_id):
    current_hash = content_hash(def_id)
    cached_hash = cache.load_hash(def_id)
    if current_hash == cached_hash:
        return cache.load_result(def_id)  // Skip ALL downstream work
    else:
        result = execute_query(def_id)
        cache.store(def_id, current_hash, result)
        return result
```

### API Boundaries for External Consumers

| Consumer | Queries Used | Access Pattern |
|----------|-------------|---------------|
| **LSP** (future) | `type_of`, `fn_sig`, `effects_of`, `diagnostics_of`, `completions_at` | On-demand, per-keystroke |
| **Verification tools** (Proposal #18) | `verification_result(DefId)` cached by content hash | Batch, skips verified definitions |
| **AI oracle** (Proposal #16) | `fn_sig`, `effects_of`, `item_tree` (compact signatures) | Read-only, context generation |
| **Build system** | `codegen_of`, `link` | Batch, incremental |
| **Dependency tools** (Proposal #22) | `resolve_names`, `dispatch_table`, reverse dependencies | Read-only, analysis |

### Phased Adoption Strategy

The architecture can be adopted incrementally without a full compiler rewrite:

**Stage 0 (Preparation)**: Ensure DefId is the universal key, signature extraction is separate from body processing, content hashing works per-definition, and output is deterministic. Blood's spec already requires all of these.

**Stage 1 (Projection queries)**: Add fine-grained caching on top of the existing monolithic pipeline. The pipeline still runs entirely, but projection queries extract per-definition results and compare content hashes. This creates change propagation firewalls without a query engine. *This is the minimum viable architecture.*

**Stage 2 (Content-hash gating)**: Use BLAKE3 hashes as an early-exit mechanism. Before running any query for a definition, check if its content hash is unchanged — if so, skip the entire pipeline for that definition. This gives most of the incremental compilation benefit with minimal architectural disruption.

**Stage 3 (Demand-driven type checking)**: Make `type_of` and `typeck` real queries that compute on demand. Changing a function body no longer re-type-checks all other functions. Requires refactoring the type checker from "process all items" to "process one item, calling queries for dependencies."

**Stage 4 (Demand-driven MIR/codegen)**: Make `mir_built`, `optimized_mir`, and `codegen_of` per-definition queries. Each depends only on the queries of functions it references.

**Stage 5 (Full query engine)**: Replace ad-hoc caching with a proper query engine (Salsa-like or custom). Add the red-green algorithm with backdating, durability tracking, and disk persistence.

**Stage 6 (IDE integration)**: Expose the query engine as a library. The compiler becomes a service that accepts file changes and answers queries. Cancellation support for in-progress queries on new edits.

**Recommendation**: Stages 0-2 should be the near-term target. They require no query engine and can be implemented within the existing compiler structure. Stages 3-6 should be deferred until IDE support becomes a priority. Blood's content-addressed design means Stages 0-2 provide proportionally MORE benefit than in other languages because the content hash is a complete invalidation signal.

### The matklad Counter-Argument

matklad (creator of rust-analyzer) argues against full query-based compilers (Feb 2026), recommending "grug-style" map-reduce compilation instead: parse files independently, extract signature summaries, evaluate signatures sequentially, parallelize body type-checking. This works when body changes cannot introduce type errors elsewhere.

**Blood's position**: Blood's explicit effect signatures and content-addressed definitions make the "body changes don't cascade" invariant structural. The grug-style approach is viable for Blood and could serve as an alternative to Stages 3-5. The architectural note does not commit to Salsa — it commits to the query boundaries and API surfaces, which are compatible with either approach.

**Consequences**:
- Compiler internal APIs should be organized around the query boundaries defined above
- The signature/body firewall must be maintained as an architectural invariant
- `effects_of(DefId)` is a Blood-specific query that participates in the dependency graph
- Content-hash gating (Stage 2) should be prioritized as the highest-ROI optimization
- CCV clusters (DEVELOPMENT.md) should align with query tiers where practical
- Proposals #16 (constrained decoding), #18 (verification cache), #19 (compact signatures), #22 (dependency graph API) are all enabled by this architecture

---

## Decision Status Legend

- **Proposed**: Under discussion
- **Accepted**: Decision made and documented
- **Deprecated**: No longer valid
- **Superseded**: Replaced by another decision

---

*Last updated: 2026-02-28 (ADR-036, ADR-037 added)*
