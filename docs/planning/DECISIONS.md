# Blood Architecture Decision Records

This document captures key architectural decisions made during the design of Blood and their rationale.

### Related Specifications

- [SPECIFICATION.md](./SPECIFICATION.md) — Core language specification
- [MEMORY_MODEL.md](./MEMORY_MODEL.md) — ADR-001, ADR-004, ADR-008, ADR-013, ADR-014 details
- [DISPATCH.md](./DISPATCH.md) — ADR-005 details
- [CONTENT_ADDRESSED.md](./CONTENT_ADDRESSED.md) — ADR-003, ADR-012 details
- [FORMAL_SEMANTICS.md](./FORMAL_SEMANTICS.md) — ADR-002, ADR-006, ADR-007, ADR-011 details
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

## Decision Status Legend

- **Proposed**: Under discussion
- **Accepted**: Decision made and documented
- **Deprecated**: No longer valid
- **Superseded**: Replaced by another decision

---

*Last updated: 2026-02-28*
