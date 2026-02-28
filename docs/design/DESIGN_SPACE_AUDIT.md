# Blood Language Design Space Audit

**Version:** 1.1
**Date:** 2026-02-28
**Scope:** Evaluation of Blood's design decisions against the full language design space
**Method:** Each axis from a comprehensive language design reference is classified by decision status
**Sources:** Spec documents (GRAMMAR.md, FORMAL_SEMANTICS.md, MEMORY_MODEL.md, DISPATCH.md, CONTENT_ADDRESSED.md, FFI.md, MACROS.md), design evaluations (IMPL_TRAIT.md, COMPARISON_CHAINING.md), planning documents (ROADMAP.md, DECISIONS.md, IMPLEMENTATION_ROADMAP.md, IMPLEMENTATION_STATUS.md, ACTION_ITEMS.md, LEGITIMIZATION_CHECKLIST.md, SYNTAX_SUPPORT.md), proposals (EXTRAORDINARY_FEATURES.md I/II/III, PROPOSAL_ANALYSIS.md, SAFETY_LEVELS.md, SYNTAX_REDESIGN.md), and compiler notes (COMPILER_NOTES.md)

---

## Classification Key

| Status | Meaning |
|--------|---------|
| **Decided** | Explicit ADR, design document, or spec rationale exists |
| **Proposed** | Researched and designed in a proposal document, but not yet committed (implementation not started) |
| **Inherited** | Adopted from Rust without documented independent evaluation |
| **Defaulted** | No evidence of deliberate choice; position is a side effect of other decisions |
| **Deferred** | Explicitly acknowledged as open with revisit criteria |

---

## Tier 1: Foundational Decisions

### 1.1 Purpose, Domain, and Paradigm

| Axis | Blood's Position | Status | Evidence |
|------|-----------------|--------|----------|
| Target domain | Systems programming | **Decided** | ADR-010 priority hierarchy; C-competitive benchmarks (1.0x ratio) |
| Paradigm | Multi-paradigm: imperative + functional + effect-oriented | **Decided** | ADR-002 (effects), ADR-005 (dispatch), ADR-011 (five-innovation composition) |
| Compilation model | AOT-first with optional JIT | **Decided** | ADR-015 |
| Evaluation strategy | Strict/eager, call-by-value | **Inherited** | No document evaluating CBV vs. lazy evaluation for Blood's effect system; CBV is natural for effects but the interaction with lazy data structures is unexamined |

**Notes:** The paradigm composition is Blood's most distinctive foundational decision, synthesizing ideas from Unison (content addressing), Vale (generational references), Hylo (mutable value semantics), Koka (algebraic effects), and Julia (multiple dispatch). This is documented in ADR-011 as a deliberate architectural thesis, not an accidental accumulation.

### 1.2 Type System

| Axis | Blood's Position | Status | Evidence |
|------|-----------------|--------|----------|
| Static vs. dynamic | Static | **Decided** | Type stability requirement (DISPATCH.md) |
| Inference strategy | Bidirectional | **Decided** | ROADMAP.md |
| Generics implementation | Monomorphization | **Inherited** | No document evaluating monomorphization vs. erasure vs. dictionary passing; see Finding F-01 |
| Parametric polymorphism | Generics + const generics | **Decided** | GRAMMAR.md, ground-truth tests |
| Ad-hoc polymorphism | Traits + multiple dispatch | **Decided** | ADR-005 |
| Subtyping | Structural via row polymorphism (records + effects) | **Decided** | ADR-009, FORMAL_SEMANTICS.md |
| Nominal vs. structural types | Hybrid: nominal structs/enums, structural records | **Decided** | FORMAL_SEMANTICS.md |
| Higher-kinded types | Not addressed | **Defaulted** | See Finding F-02 |
| Dependent types | Not addressed | **Defaulted** | Const generics provide limited type-level computation; broader space unevaluated |
| Variance | Not addressed | **Defaulted** | See Finding F-03 |
| Existential types | Under consideration as `opaque` type aliases | **Deferred** | IMPL_TRAIT.md; revisit criteria specified |
| Type-level computation | Minimal (const eval of integer arithmetic only) | **Defaulted** | COMPILER_NOTES.md lists supported const operations; no design-level evaluation of the space |

### 1.3 Data Representation

| Axis | Blood's Position | Status | Evidence |
|------|-----------------|--------|----------|
| Numeric tower | Fixed-width integers (i8–i64, u8–u64, usize), f32/f64 | **Inherited** | Matches Rust exactly; no evaluation of arbitrary precision, rationals, decimal, or BigInt (deferred to post-1.0 in IMPLEMENTATION_ROADMAP.md) |
| String representation | UTF-8 `&str` slices + owned `String` | **Inherited** | No document on encoding tradeoffs or interaction with 128-bit pointers; see Finding F-04 |
| Array/slice model | Fixed arrays `[T; N]` + slices `&[T]` with fat pointers | **Inherited** | Matches Rust |
| Algebraic data types | Structs + enums (tagged unions) | **Inherited** | Matches Rust syntax and semantics |
| Anonymous records | Row-polymorphic structural records | **Decided** | FORMAL_SEMANTICS.md, ADR-009 |
| Tuples | Yes | **Inherited** | No independent evaluation |
| Pointer representation | 128-bit fat pointers (heap), 64-bit thin (stack) | **Decided** | ADR-001, MEMORY_MODEL.md (extensive tradeoff analysis including cache impact, break-even analysis) |
| Null handling | `Option<T>` | **Inherited** | See Finding F-05 |

### 1.4 Memory Model

| Axis | Blood's Position | Status | Evidence |
|------|-----------------|--------|----------|
| Primary safety mechanism | Generational references | **Decided** | ADR-001, with fallback strategies in ADR-018 |
| Ownership model | Hybrid mutable value semantics | **Decided** | ADR-014 |
| Memory tiers | Stack (Tier 0) / Region (Tier 1) / Persistent RC (Tier 2) | **Decided** | ADR-008, MEMORY_MODEL.md |
| Linear/affine types | Both supported; linear cannot cross multi-shot resume | **Decided** | ADR-006, ADR-026, FORMAL_SEMANTICS.md |
| Garbage collection | None (generational refs + RC for Tier 2) | **Decided** | MEMORY_MODEL.md |
| Escape analysis | Three-tier with 98.3% stack promotion rate | **Decided** | ESCAPE_ANALYSIS.md, ACTION_ITEMS.md |
| Interior mutability | Not documented | **Defaulted** | No `Cell`/`RefCell` equivalent discussed; unclear how shared mutable state interacts with generational references |
| Region semantics | Scoped allocation with generational safety | **Decided** | FORMAL_SEMANTICS.md, MEMORY_MODEL.md |

### 1.5 Error Handling and Verification

| Axis | Blood's Position | Status | Evidence |
|------|-----------------|--------|----------|
| Primary mechanism | Algebraic effects | **Decided** | ADR-002 |
| Panic semantics | `panic!` macro exists; FFI boundary behavior specified | Partially **decided** | FFI.md (`#[no_unwind]`); general panic-vs-abort strategy undocumented |
| `Result<T, E>` | Exists in the language | **Inherited** | See Finding F-05 |
| `?` operator | Present in grammar | **Inherited** | No document on interaction between `?` and effect handlers |
| Error propagation strategy | Undocumented | **Defaulted** | When to use effects vs. Result vs. panic is not specified |
| Specification annotations | `requires`/`ensures`/`invariant`/`decreases` as first-class keywords | **Proposed** | SYNTAX_REDESIGN.md (B.1), EF_III Proposal #20; definitive signature ordering specified |
| Graduated verification | Four levels: runtime contracts → SMT verification → full proofs | **Proposed** | EF_I Proposal #7, EF_III Proposal #20; depends on spec annotations |
| Proof-carrying code | Proofs indexed by `(function_hash, contract_hash, proof_hash)` | **Proposed** | EF_II Proposal #10; depends on graduated verification + content addressing |

### 1.6 Control Flow and Pattern Matching

| Axis | Blood's Position | Status | Evidence |
|------|-----------------|--------|----------|
| Pattern matching | Exhaustive `match` with destructuring | **Decided** | GRAMMAR.md, FORMAL_SEMANTICS.md |
| Exhaustiveness checking | Required (compile-time error if non-exhaustive) | **Decided** | FORMAL_SEMANTICS.md |
| Iteration | `for`/`while` loops, ranges (`0..len`) | **Decided** | Syntax modernization documented; 919 while→for conversions |
| Closures | Full closures with capture modes (ref default, `move`) | **Decided** | FORMAL_SEMANTICS.md |
| Pipeline operator | `|>` with precedence above assignment | **Decided** | GRAMMAR.md v0.4.0 |
| Comparison chaining | Explicitly rejected | **Decided** | COMPARISON_CHAINING.md (linear type conflict, effect ordering) |
| Labeled loops | Yes | **Decided** | GRAMMAR.md |
| Labeled blocks | Explicitly rejected | **Decided** | GRAMMAR.md (effects subsume the use case) |
| Containment expressions | `x in lo..hi` | **Decided** | GRAMMAR.md v0.4.0 |
| Semicolons | Optional with continuation rules | **Proposed** | SYNTAX_REDESIGN.md (C.1); `Statement ::= ... ';'?`; both styles compile identically |
| Named arguments | Gradual adoption; prefer for 3+ params | **Proposed** | SYNTAX_REDESIGN.md (C.2), EF_III Proposal #21; eliminates 6.9% "Wrong Attribute" AI bug category |
| Expression-oriented design | Every construct returns a value | **Proposed** | EF_III Proposal #21; 5-10% token reduction, more locally replaceable for AI |
| Function signature ordering | Definitive: attrs → sig → effects → specs → where → body | **Proposed** | SYNTAX_REDESIGN.md (B.1); resolves previously ad-hoc ordering |

### 1.7 Concurrency

| Axis | Blood's Position | Status | Evidence |
|------|-----------------|--------|----------|
| Concurrency model | Effects-based async + fiber runtime | Partially **decided** | ADR-002 (effects); CONCURRENCY.md exists but fiber-language integration is incomplete |
| Colored function problem | Avoided (async is an effect, not a type) | **Decided** | IMPL_TRAIT.md rationale |
| Structured concurrency | Not documented | **Defaulted** | See Finding F-06 |
| Cancellation semantics | Not decided | **Deferred** | DECISIONS.md explicitly defers cooperative vs. preemptive |
| Cancellation safety | Not addressed | **Defaulted** | No analysis of partial-operation consistency on cancellation |
| Async drop | Not addressed | **Defaulted** | Effects may solve this, but no explicit analysis exists |
| Send/Sync equivalents | Not documented | **Defaulted** | No thread-safety marker traits or effect-based equivalent discussed |
| Async iterators/streams | Not addressed | **Defaulted** | No evaluation of async sequences |
| Runtime-provided vs. library async | Not documented | **Defaulted** | Fiber scheduler exists in runtime; relationship to language semantics unclear |
| Deterministic simulation testing | Effect handlers intercept all nondeterminism sources | **Proposed** | EF_II Proposal #8; no compiler changes needed — library pattern on existing effects |
| Deterministic replay debugging | Record/replay all effect invocations at handler boundaries | **Proposed** | EF_II Proposal #12; ~2-5% overhead vs. 15-40% for OS-level (rr) |

### 1.8 Module System

| Axis | Blood's Position | Status | Evidence |
|------|-----------------|--------|----------|
| Path separator | `.` (dot-separated) | **Decided** | GRAMMAR.md v0.4.0; explicit break from Rust's `::` |
| Module hierarchy | File-based modules | **Inherited** | Matches Rust's file=module model without independent evaluation |
| Visibility | `pub` modifier | **Inherited** | No document evaluating fine-grained visibility (`pub(crate)`, `pub(super)`) or alternatives |
| Cyclic imports | Not documented | **Defaulted** | No explicit policy on whether cyclic module dependencies are allowed |
| Compilation unit | Individual definitions (by content hash) | **Decided** | ADR-003, CONTENT_ADDRESSED.md |
| Import semantics | `use` with simple/grouped/glob forms | **Decided** | Implemented and tested |
| Re-exports | `pub use` (simple and glob) | **Decided** | Implemented and tested |
| Prelude | Auto-imported stdlib prelude | **Decided** | Implemented (`inject_stdlib_prelude()`) |
| Functors/parameterized modules | Not addressed | **Defaulted** | ML-style functors unevaluated |

### 1.9 Metaprogramming

| Axis | Blood's Position | Status | Evidence |
|------|-----------------|--------|----------|
| Macro system tiers | Built-in < declarative < procedural | **Decided** | MACROS.md (least-power principle) |
| Expansion level | Text/source-level preprocessing | **Decided** | Rationale: self-hosting simplicity, content-addressing compatibility |
| Hygiene | Unhygienic now; definition-site target model specified | **Decided** | Target model in MACROS.md; implementation phases 1-3 unscheduled |
| Determinism | Required (pure function of definition + arguments) | **Decided** | Content-addressing constraint |
| Procedural macros | Constraints specified, syntax deferred | **Deferred** | MACROS.md — the only legitimate grammar deferral |
| Compile-time execution | Not addressed | **Defaulted** | No evaluation of Zig-style `comptime` |
| Reflection | Not addressed | **Defaulted** | Deferred to post-1.0 (IMPLEMENTATION_ROADMAP.md) |
| Multi-stage programming | Not addressed | **Defaulted** | No evaluation of MetaOCaml/Scala 3 staging |

### 1.10 Compiler Diagnostics

| Axis | Blood's Position | Status | Evidence |
|------|-----------------|--------|----------|
| Source span tracking | Per-node `Span` type throughout AST/HIR/MIR | **Decided** | Designed in from the start; `Span` is a canonical shared type |
| Error recovery (parser) | Parser recovers from errors | **Decided** | Implemented |
| Error recovery (type checker) | Error types prevent cascading | **Decided** | `TypeError` API with `TypeErrorKind` variants |
| Structured diagnostics | Error codes (E0300+) with spans | **Decided** | Implemented |
| Dual-consumption diagnostics | Human-readable default + `--diagnostics=json` for machines | **Proposed** | EF_III Proposal #17; constraint provenance chains, fix suggestions as structured diffs, stable error codes as public API |
| Type error quality | Not specifically designed | **Defaulted** | No design doc on inference chain explanation, expected-vs-found rendering |

---

## Tier 2: Core Implementation

### 2.1 Grammar, AST, and Parsing

| Axis | Blood's Position | Status | Evidence |
|------|-----------------|--------|----------|
| Grammar formalism | EBNF in GRAMMAR.md | **Decided** | Comprehensive spec (source of truth) |
| Parser architecture | Hand-written recursive descent | **Decided** | ROADMAP.md (for better error messages and incremental parsing) |
| AST allocation | Arena-allocated | **Decided** | Region allocator used throughout |
| CST vs. AST | Direct AST (no lossless CST) | **Defaulted** | No evaluation of CST-first approach; limits future formatter/IDE support |
| Trailing commas | Allowed | **Decided** | GRAMMAR.md |

### 2.2 Semantic Analysis

| Axis | Blood's Position | Status | Evidence |
|------|-----------------|--------|----------|
| Name resolution | Multi-phase (scope building, then resolution) | **Decided** | Implemented in resolve.blood |
| Definite initialization | Not documented | **Defaulted** | Unclear whether uninitialized variables are statically prevented |
| Reachability / dead code | Not documented | **Defaulted** | No dead code analysis mentioned |
| Const evaluation | Limited (integer literals, basic arithmetic, bitwise, unary) | **Decided** | COMPILER_NOTES.md (explicit list of supported operations) |

### 2.3 Intermediate Representations

| Axis | Blood's Position | Status | Evidence |
|------|-----------------|--------|----------|
| IR pipeline | AST → HIR (desugared) → MIR (control-flow) → LLVM IR | **Decided** | ROADMAP.md |
| MIR purpose | Uniform generation check insertion, escape analysis, borrow-like analysis | **Decided** | Designed for Blood's specific safety model |
| SSA form | Via LLVM IR (MIR is not SSA) | **Inherited** | Side effect of LLVM backend choice |
| Nanopass vs. large passes | Few large passes | **Defaulted** | Not explicitly evaluated |
| Blood-specific optimizations | Tail-resumptive effect optimization | **Decided** | ADR-028 |
| General optimization | Delegated to LLVM | **Defaulted** | No Blood-specific optimization passes beyond effect handlers |

### 2.4 Runtime Architecture

| Axis | Blood's Position | Status | Evidence |
|------|-----------------|--------|----------|
| Object layout | 128-bit fat pointers documented; general struct layout undocumented | Partially **decided** | MEMORY_MODEL.md covers pointers; struct field ordering, padding, alignment undocumented |
| Calling convention | Platform C ABI for FFI | Partially **decided** | FFI.md; Blood-internal calling convention undocumented |
| Effect handler implementation | Evidence passing (ICFP'21 approach) | **Decided** | ADR-025 (O(1) handler lookup, ~1.3 cycle tail-resumptive) |
| Fiber/stack model | Exists in runtime | Partially **decided** | Runtime implemented; language-level semantics incomplete |
| Signal handling | Not addressed | **Defaulted** | No doc on POSIX signal interaction with effect handlers |
| Compiler-as-a-library | Not addressed | **Defaulted** | See Finding F-07 |
| Hot code reloading | Three modes (immediate, barrier, epoch) | **Decided** | CONTENT_ADDRESSED.md (VFT invalidation strategy) |
| Dynamic loading / plugins | Not addressed | **Defaulted** | Plugin architecture deferred to post-1.0 |

### 2.5 Code Generation

| Axis | Blood's Position | Status | Evidence |
|------|-----------------|--------|----------|
| Backend | LLVM | **Decided** | ROADMAP.md (mature, multi-target; Cranelift noted as future option) |
| JIT compilation | Deferred | **Deferred** | ADR-015 (AOT-first) |
| Linker | System linker | **Defaulted** | No evaluation of custom linker (Zig-style) or cross-compilation implications |
| Debug information | DWARF (via LLVM) | **Defaulted** | No explicit debug info strategy |
| LTO | Not addressed | **Defaulted** | No link-time optimization strategy |

---

## Tier 3: Ecosystem and Maturation

| Axis | Blood's Position | Status | Evidence |
|------|-----------------|--------|----------|
| Build system / package manager | Specified, not implemented | **Decided** / **Deferred** | Design exists; implementation is post-bootstrap |
| Package versioning | Content-addressed (eliminates version conflicts) | **Decided** | ADR-003 |
| REPL | Not addressed | **Defaulted** | SYNTAX_REDESIGN.md mentions future REPL should default to expression-mode (semicolonless) but no REPL is planned |
| Language server (LSP) | Constrained decoding oracle proposed as LSP extension | **Proposed** / **Defaulted** | EF_III Proposal #16 proposes `ConstrainedDecodingService` as LSP extension; no general LSP strategy; see Finding F-07 |
| Formatter | Not addressed | **Defaulted** | Grammar designed with mechanical formatting potential (trailing commas, brace-delimited blocks) but no formatter planned |
| Linter | Safety audit tooling proposed | **Proposed** | SAFETY_LEVELS.md: `--warn-unchecked`, `blood audit --safety`, certification mode |
| Debugger / DAP | Not addressed | **Defaulted** | |
| Profiling | Diagnostic flags (`--dump-mir`, `--dump-types`, `--dump-adt-layouts`) | Partially **decided** | No profiling strategy (frame pointers, instrumentation hooks) |
| Observability | Zero-code via effect handler wrapping | **Proposed** | EF_II Proposal #13; generic `Traced<E>` and `Metered<E>` handlers; guaranteed-complete because effect type system tracks all operations |
| Documentation generator | Not addressed | **Defaulted** | No doc comment syntax defined in grammar |
| Std library scope | Minimal (8 modules) | **Defaulted** | No doc on batteries-included vs. minimal strategy; see Finding F-08 |
| Freestanding / no-OS support | Not addressed | **Defaulted** | See Finding F-08 |
| Testing as language feature | `assert!`/`panic!` macros; DST via effects proposed | Partially **proposed** | See Finding F-09; EF_II Proposal #8 (DST) and FFI.md `MockFFI` handler demonstrate the pattern; no first-class `#[test]` declaration |
| Module signatures for AI | `blood sig` and `blood context --for-ai` commands | **Proposed** | EF_III Proposal #19; 14x compression of module interfaces; content-addressed signatures change only when public API changes |
| Dependency graph API | `blood deps`, `blood effects`, `blood impact` commands | **Proposed** | EF_III Proposal #22; JSON output for AI consumption |
| Bootstrapping | Active (first_gen/second_gen/third_gen byte-identical) | **Decided** | Comprehensive bootstrap infrastructure with verification |

---

## Cross-Cutting Concerns

| Axis | Blood's Position | Status | Evidence |
|------|-----------------|--------|----------|
| FFI | Bridge blocks with `@unsafe`, FFI as effect | **Decided** | FFI.md (thorough: ownership annotations, calling conventions, callbacks, panic safety, mock handlers) |
| Safety controls | Granular per-check `#[unchecked(generation\|bounds\|overflow\|null\|alignment)]` | **Proposed** | SAFETY_LEVELS.md (RFC-S); diverges from Rust's binary `unsafe`; includes block scoping, conditional safety, certification mode, audit tooling |
| ABI stability | Not addressed | **Defaulted** | No stable/unstable ABI commitment; see Finding F-10 |
| Backward compatibility | Not addressed | **Defaulted** | No edition system, versioning strategy, or evolution policy |
| Semantic versioning | Automatic via content hashes + effect signature diff | **Proposed** | EF_II Proposal #11; provably sound "patch" classification (identical hash = identical behavior) |
| Ecosystem governance | Not addressed | **Defaulted** | No RFC process or governance model |
| Security model — capability security | Effects as capabilities; `attenuate` for restriction; `main()` as capability root | **Proposed** | EF_I Proposal #4; effects already track capabilities, handlers enable attenuation |
| Security model — information flow | Tainted data as effects; sanitization as handlers; compile-time guarantee | **Proposed** | EF_II Proposal #9; structural guarantee via type system, not approximation |
| Compilation speed | Not a stated design goal | **Defaulted** | Monomorphization + LLVM accepted without compile-time budget analysis |
| Incremental compilation | Content-addressing enables it; partial implementation | **Decided** | CONTENT_ADDRESSED.md (theory); IMPLEMENTATION_STATUS.md (partial practice) |
| Reproducible builds | Determinism proof for content addressing | **Decided** | CONTENT_ADDRESSED.md (determinism proof sketch with structural induction) |
| SIMD / vectorization | Not addressed | **Defaulted** | |
| Compile-time resource limits | Macro expansion limits (32 passes / 256 depth) | Partially **decided** | Macros bounded; type recursion and monomorphization unbounded |
| Compile-time timing analysis | `@ wcet(duration)` annotations verified against effect-explicit control flow | **Proposed** | EF_I Proposal #1; no other language provides WCET as first-class concept |
| Compile-time complexity bounds | `@ complexity(time: O(...), space: O(...))` for pure functions | **Proposed** | EF_II Proposal #15; decidable subset only; honest about limitations |
| Serialization | Not addressed | **Defaulted** | |
| Cross-compilation | Not addressed | **Defaulted** | |
| Conditional compilation | `#[cfg(target_os = "...")]` in FFI context | **Inherited** | No broader conditional compilation design |
| Internationalization | Not addressed | **Defaulted** | |
| WebAssembly | WASM listed as FFI target; limited support | **Defaulted** | No strategic evaluation of Component Model |
| Formal specification | GRAMMAR.md, FORMAL_SEMANTICS.md, Coq proofs (10/12 theorems) | **Decided** | Major strength; few languages at this maturity have this level of formalization |

---

## Beyond the Reference Framework: AI-Native Design

The reference design document does not address AI-native language design — an axis Blood's proposals treat as foundational. The EXTRAORDINARY_FEATURES_III.md proposals and SYNTAX_REDESIGN.md collectively define a design surface that has no precedent in the reference literature.

| Axis | Blood's Position | Status | Evidence |
|------|-----------------|--------|----------|
| AI-native identity | "AI-native, effects-first systems language" | **Proposed** | SYNTAX_REDESIGN.md identity statement |
| Constrained decoding oracle | Three-level (grammar → types → effects) LLM token constraint | **Proposed** | EF_III Proposal #16; ~50% of LLM code generation bugs eliminated before full generation; requires incremental type checker |
| Verification cache | Content-addressed `(spec_hash, impl_hash)` → proof artifact cache | **Proposed** | EF_III Proposal #18; verification cost per function approaches zero for common operations; global cache trustworthy because content-addressing is tamper-evident |
| Module signatures for AI context | `blood sig` generates compact type+effect signatures (14x compression) | **Proposed** | EF_III Proposal #19; addresses 54% vs. 90% success gap between multi-file and single-file AI tasks |
| Specification as AI prompt | `requires`/`ensures` serve as generation prompt, verification target, and documentation simultaneously | **Proposed** | EF_III Proposal #20; "triple-duty principle"; POPL 2026 vericoding benchmark shows specs eliminate 20.77% of LLM misinterpretation bugs |
| Syntax optimized for token efficiency | ~15-25% more compact than Rust/TypeScript; ~20% more codebase fits in context | **Proposed** | EF_III Proposal #21; specific decisions: optional semicolons, named args, pipeline, expression-oriented |
| Effect handlers as agent middleware | Agent capabilities as effects with composable handler stacks | **Proposed** | EF_III Proposal #23; if an effect is not handled, the program does not compile — no other agent framework provides this guarantee |
| Session types / choreographic programming | Binary (`protocol` keyword) and N-party (choreography) protocol verification | **Proposed** | EF_I Proposal #2, EF_II Proposal #14; session types are structured effects; choreography is Phase 4+ |
| Automatic memoization | `#[memoize]` via `(function_hash, input_hash)` caching | **Proposed** | EF_I Proposal #3; only pure functions; content addressing makes this natural |
| Provenance tracking | `#[provenance]` attribute with forward/backward trace via Provenance effect | **Proposed** | EF_I Proposal #6; regulated industry data lineage at language level |
| Auto-parallelization | `/ pure` functions automatically parallelizable; `#[parallel]` verification | **Proposed** | EF_I Proposal #5; effects solve the proof obligation that has historically made auto-parallelization fail |

### Proposal Maturity and Dependencies

The proposals follow a documented critical path (from PROPOSAL_ANALYSIS.md):

```
Phase 1: Spec annotations (#20) + Safety controls (RFC-S) + AI syntax (#21)
Phase 2: Runtime contracts + Structured diagnostics (#17) + DST (#8)
Phase 3: Compile-time verification (#7) + Verification cache (#18)
Phase 4: Constrained decoding (#16) + Taint tracking (#9) + Observability (#13)
Phase 5: Capability security (#4) + WCET (#1) + Session types (#2)
Phase 6: Choreography (#14) + Proof-carrying code (#10) + Complexity bounds (#15)
```

### Pillar Utilization (from PROPOSAL_ANALYSIS.md)

| Pillar | Proposals Using It | Coverage |
|--------|-------------------|----------|
| Algebraic Effects | 17/23 | 74% |
| Content-Addressed Code | 14/23 | 61% |
| Generational Memory | 6/23 | 25% — **underrepresented** |
| Multiple Dispatch | 3/23 | 13% — **underrepresented** |

The proposals disproportionately leverage effects and content addressing. Generational memory and multiple dispatch are underexploited — suggesting either these pillars have fewer novel applications or that the proposal space has not been fully explored for them.

---

## Summary

| Category | Count | Percentage |
|----------|-------|------------|
| **Consciously Decided** | 42 | 34% |
| **Proposed (researched, not committed)** | 26 | 21% |
| **Inherited from Rust** | 18 | 15% |
| **Accidentally Defaulted** | 28 | 23% |
| **Explicitly Deferred** | 8 | 7% |

With proposals included, the decided+proposed coverage rises to **55%** of the design space, up from 43%. The largest shift is in the security model (from "inherited" to "proposed: capability-based + information flow"), verification (from "not addressed" to "proposed: graduated four-level"), and diagnostics (from "basic" to "proposed: dual human/machine consumption").

---

## Findings

### F-01: Monomorphization × Content Addressing Tension

**Severity:** Architectural
**Status:** Defaulted — no document addresses this interaction

Blood uses content-addressed compilation (each definition identified by BLAKE3-256 hash of its canonicalized AST) and monomorphization (each generic instantiation produces a specialized copy). These two core innovations interact in ways that are not documented:

1. **Hash space explosion.** A generic function `fn map<T, U>(...)` instantiated with 50 type combinations produces 50 monomorphized functions. Does each get its own content hash? If yes, the hash space scales multiplicatively with the number of generic instantiations × the number of types.

2. **Incremental invalidation cascading.** If a type `Foo` changes, every monomorphized function that was instantiated with `Foo` has a different hash. Content-addressed incremental compilation must invalidate all of them. The invalidation set grows with the number of generic uses of the changed type.

3. **Caching efficiency.** Content addressing promises global caching — identical definitions produce identical hashes. But monomorphized instances are only identical if their type arguments are identical. Two crates using `Vec<i32>` share a cache entry; two crates using `Vec<MyType>` and `Vec<YourType>` do not, even if `MyType` and `YourType` are structurally identical (because Blood uses nominal typing for structs).

4. **Alternatives unconsidered.** Dictionary passing (Haskell/Koka-style) keeps one copy of each generic function and passes type information at runtime. This composes naturally with content addressing (one hash per generic definition) at the cost of runtime dispatch overhead. Erasure (Java-style) has similar properties. Neither has been evaluated for Blood.

**Recommendation:** Write an ADR documenting: (a) the hashing strategy for monomorphized instances, (b) the invalidation model when types change, (c) whether dictionary passing was evaluated and why it was rejected.

### F-02: Higher-Kinded Types

**Severity:** Design gap
**Status:** Defaulted

Blood's effect handlers are structurally higher-kinded — a handler transforms a computation parameterized by an effect row. The `Handler` concept operates on type constructors, not types. Row polymorphism provides some of the expressiveness that HKTs would provide, but the relationship is not documented.

Questions that arise in practice:
- Can a trait be generic over a type constructor? (e.g., `trait Functor<F<_>>`)
- Can effect handlers be abstracted over? (e.g., a function generic in the handler type)
- How does multiple dispatch interact with higher-kinded type arguments?

**Recommendation:** Evaluate whether row polymorphism + effects + multiple dispatch together cover the practical use cases of HKTs, and document the conclusion. If gaps exist, document them as known limitations rather than leaving them as surprises.

### F-03: Variance

**Severity:** Design gap
**Status:** Defaulted

Generic type parameters have variance properties (covariant, contravariant, invariant) that determine subtyping relationships between parameterized types. Blood has:
- Generics (`Vec<T>`, `HashMap<K, V>`)
- References (`&T`, `&mut T`)
- Linear/affine types
- Row polymorphism

The interaction of variance with these features is unspecified:
- Is `Vec<&'a Cat>` a subtype of `Vec<&'a Animal>` if `Cat: Animal`? (Blood doesn't have inheritance, but row-polymorphic subtyping raises analogous questions.)
- How does variance interact with linear types? A `Container<linear T>` has different safety requirements than `Container<T>`.
- Row polymorphism has its own notion of width subtyping — how does this compose with generic variance?

**Recommendation:** Document variance rules for generic type constructors, even if the rule is "all type parameters are invariant" (the simplest safe choice).

### F-04: String Representation and 128-bit Pointers

**Severity:** Clarification needed
**Status:** Inherited

Blood inherits Rust's string model (`&str` as a fat pointer with ptr + length, `String` as an owned heap buffer). Blood also uses 128-bit fat pointers for all heap allocations (64-bit address + 32-bit generation + 32-bit metadata).

Undocumented questions:
- Is `&str` a 128-bit fat pointer (128-bit base + 64-bit length = 192 bits total) or a Rust-style 128-bit fat pointer (64-bit ptr + 64-bit length)?
- If `&str` uses the 128-bit format, string slices are 24 bytes instead of 16 — a 50% overhead that affects every string operation.
- How do string slices interact with generational checking? A string slice pointing into a `String`'s buffer needs generation validation when the `String` might have been deallocated.

**Recommendation:** Document the concrete representation of `&str` and `&[T]` slices under Blood's memory model.

### F-05: Result/Option Alongside Effects

**Severity:** Ecosystem coherence
**Status:** Inherited

Blood has algebraic effects as its primary error handling mechanism. It also has `Result<T, E>` and `Option<T>` inherited from Rust. The coexistence is undocumented, creating ambiguity:

- When should a function return `Result<T, E>` vs. performing an error effect?
- When should a function return `Option<T>` vs. performing a "not found" effect?
- Does `?` propagate through effects, or only through `Result`?
- Is `Option` an effect (`Maybe`) in disguise?
- Should library authors use effects or `Result` for their APIs?

Without guidance, the ecosystem will split: some libraries will use effects, others will use `Result`, and composing them will require boilerplate adapters.

**Recommendation:** Write a short ADR or guideline specifying: (a) the intended role of `Result` and `Option` alongside effects, (b) when each is appropriate, (c) how they interconvert.

### F-06: Concurrency Model

**Severity:** Architectural — largest undecided area
**Status:** Partially decided, partially defaulted

Blood has the pieces of a concurrency story:
- Algebraic effects can express async operations (async is an effect, not a colored function)
- A fiber runtime exists (scheduler, stack management)
- CONCURRENCY.md exists as a spec document

But the composition is incomplete:

| Sub-decision | Status |
|-------------|--------|
| Structured concurrency (task scoping) | **Defaulted** |
| Cancellation mechanism | **Deferred** (DECISIONS.md) |
| Cancellation safety guarantees | **Defaulted** |
| Async drop / cleanup | **Defaulted** |
| Thread-safety markers (Send/Sync) | **Defaulted** |
| Async iterators / streams | **Defaulted** |
| Runtime-provided vs. library concurrency | **Defaulted** |
| Fiber ↔ OS thread interaction | **Defaulted** |

This is the area where Blood could most distinguish itself. Effects naturally express structured concurrency (a handler scope = a task scope). Cancellation can be an effect. Send/Sync can be effect-based constraints. But none of this is designed or documented.

**Risk:** If the fiber runtime calcifies before the language-level concurrency model is designed, the runtime may constrain the language design rather than serving it.

**Recommendation:** Design the concurrency model as a cohesive whole, leveraging effects as the unifying mechanism. Specifically: (a) define structured concurrency via effect handler scoping, (b) specify cancellation as an effect, (c) specify thread-safety constraints as effect-based or trait-based.

### F-07: Compiler-as-a-Library

**Severity:** Architectural — expensive to retrofit
**Status:** Defaulted

Blood's self-hosted compiler is a monolithic pipeline (source in, binary out). The reference design literature warns that retrofitting query-based architecture for LSP support is "extremely expensive."

Blood's content-addressed compilation model is naturally query-based — each definition is independently hashable and cacheable. This is an advantage that the compiler architecture does not yet exploit.

Current state:
- No language server
- No incremental re-analysis
- No partial program processing
- Compiler cannot be embedded as a library

**Recommendation:** Before the self-hosted compiler architecture solidifies further, evaluate a query-based internal architecture that aligns with the content-addressed model. This does not require immediate implementation but should constrain the compiler's internal API boundaries.

### F-08: Standard Library Scope and Freestanding Split

**Severity:** Forward-looking
**Status:** Defaulted

Blood has 8 stdlib modules (HashMap, HashSet, Arena, Sort/Search, fmt, String, Math, prelude). There is no documented strategy for:
- How large the stdlib should grow (batteries-included vs. minimal)
- Whether a `core`/`alloc`/`std` split is planned for freestanding environments
- Which APIs are available without an OS or allocator

Blood's tiered memory model maps naturally to a freestanding split:
- `core`: Tier 0 only (stack allocation, no heap) — pure computation
- `alloc`: Tier 0 + Tier 1 (stack + region allocation) — heap without OS
- `std`: All tiers (stack + region + persistent + I/O)

This split would enable Blood for embedded systems, OS kernels, and WebAssembly without an OS — contexts that align with Blood's systems programming target domain.

**Recommendation:** Document the stdlib scope strategy and evaluate the tiered freestanding split before the API surface grows further. Retrofitting a freestanding split onto APIs that assume Tier 2 availability is the pattern the reference literature warns against.

### F-09: Testing as a Language Feature

**Severity:** Ecosystem — opportunity
**Status:** Partially proposed, partially defaulted

Blood has `assert!` and `panic!` macros but no first-class test declaration mechanism (`#[test]`, `test` blocks, or equivalent). The proposals partially address the testing story:

- **EF_II Proposal #8 (DST)** demonstrates that effect handlers enable deterministic simulation testing as a library pattern — no compiler changes needed. This is the strongest testing proposal.
- **EF_II Proposal #12 (Replay Debugging)** extends the same handler infrastructure to record/replay all effect invocations for time-travel debugging.
- **FFI.md** documents the `MockFFI` handler pattern for testing FFI-dependent code.

What remains unaddressed:
- **No first-class test declaration syntax.** There is no `#[test]`, `test` block, or equivalent. Tests are not compiler-aware.
- **No test runner.** No `blood test` command or standard test discovery mechanism.
- **No property-based testing primitives.** Random generation as an effect is a natural fit but not proposed.

Blood's effect system enables a testing model no other systems language can match. The DST proposal demonstrates the pattern, but the gap between "effects enable good testing" and "Blood has a great testing story" is a first-class declaration mechanism + toolchain integration.

**Recommendation:** Add a first-class test declaration mechanism (e.g., `test "name" { ... }` blocks or `#[test]` attribute) and a `blood test` runner that leverages effect handler isolation. The DST proposal provides the conceptual foundation; what's missing is the ergonomic surface.

### F-10: ABI Stability

**Severity:** Forward-looking
**Status:** Defaulted

Blood has not committed to a stable or unstable ABI. This is common for young languages, but the decision interacts with:
- Content-addressed compilation (definitions identified by hash, not by name — hash-based ABI?)
- Hot code reloading (VFT swap requires some form of ABI contract)
- FFI (bridge blocks provide C ABI; Blood-to-Blood ABI is unspecified)
- Dynamic linking feasibility

Content addressing could enable a novel approach: ABI compatibility defined by content hash rather than by version number. Two definitions with the same hash are ABI-compatible by construction. This is worth evaluating as a distinctive design.

**Recommendation:** Document the ABI strategy, even if the commitment is "explicitly unstable until further notice." The interaction with content addressing deserves specific analysis.

---

## Inherited Decisions Warranting Independent Evaluation

The following decisions were adopted from Rust and are likely correct for Blood, but have no documented rationale in Blood's design context. Each should receive at minimum a brief ADR confirming the choice:

| Decision | Why It Warrants Evaluation | Proposal Coverage |
|----------|---------------------------|-------------------|
| Monomorphization | Interacts with content addressing (F-01) | None |
| `Option<T>` / `Result<T, E>` | Coexists with effects (F-05) | None |
| UTF-8 strings | Interacts with 128-bit pointers (F-04) | None |
| File-based module hierarchy | Content addressing decouples identity from files | None |
| `pub` visibility (Rust-style) | Row polymorphism introduces structural subtyping that may need different visibility rules | None |
| Call-by-value evaluation | Natural for effects but undocumented | None |
| No runtime type information | Multiple dispatch uses 24-bit type fingerprints — this IS runtime type info by another name | None |
| `&T` / `&mut T` reference syntax | Blood's references are semantically different (generational, not borrowed) — same syntax may mislead | None |
| Binary `unsafe` blocks | Granular safety controls proposed in SAFETY_LEVELS.md (RFC-S) but not yet committed | **Partially addressed** by RFC-S |

---

## Accidental Defaults Requiring Minimal Effort to Resolve

These items can be resolved with a one-paragraph decision record each:

1. **Cyclic imports:** Allowed or forbidden? (Recommendation: forbidden, matching content-addressed DAG structure)
2. **Variance:** Invariant by default? (Recommendation: yes, with future relaxation if needed)
3. **Interior mutability:** Supported or not? (Recommendation: defer, document as not-yet-designed)
4. **Dead code detection:** Planned or not? (Recommendation: yes, as a compiler warning)
5. **Definite initialization:** Statically enforced? (Recommendation: yes, via MIR analysis)
6. **Doc comment syntax:** `///` or other? (Recommendation: decide before stdlib grows further)
7. **Frame pointer preservation:** Default on or off? (Recommendation: on, for profiling)

---

## Overall Assessment

Blood's design is characterized by a **strong core, an ambitious proposal layer, and a thin periphery**.

### Strengths

The five central innovations (generational references, algebraic effects, content addressing, multiple dispatch, linear/affine types) are among the most thoroughly evaluated design decisions in any pre-1.0 language. The formal semantics, Coq mechanization, and ADR process demonstrate unusual rigor.

The proposal documents (EXTRAORDINARY_FEATURES I/II/III, SAFETY_LEVELS.md, SYNTAX_REDESIGN.md) demonstrate that Blood's designers have thought deeply about areas the reference framework considers — security (capability-based + information flow), verification (graduated four-level), diagnostics (dual human/machine), and testing (DST via effects) — even though these decisions are at "proposed" rather than "committed" status. The proposal layer adds 26 design axes that are researched and designed but not yet implemented.

The AI-native design surface (EF_III) extends beyond anything in the reference framework. No other language has a documented design for constrained decoding oracles, content-addressed verification caches, or specification annotations that serve simultaneously as AI prompts, verification targets, and documentation.

### Remaining Gaps

Even with proposals included, gaps remain in four areas:

1. **Rust-inherited surface** (15% of decisions): The syntax and standard library conventions were adopted from Rust without re-evaluating them in Blood's divergent context. Most are fine. Some (monomorphization, Result/Option, string representation) have non-obvious interactions with Blood's core innovations. The proposals do not address any of these inherited decisions.

2. **Ecosystem infrastructure** (23% of decisions defaulted): Normal for a pre-1.0 language, but some ecosystem decisions (compiler-as-a-library, freestanding split) exert backward pressure on the core and should be decided before implementation locks them in. The proposals partially address this (linter via safety audit, observability via effect handlers) but leave REPL, formatter, debugger, and doc generator unaddressed.

3. **Concurrency** (the largest single gap): Blood has the most promising concurrency foundation of any systems language (effects that subsume async, fibers, structured scoping via handlers) but has not yet designed the cohesive model that ties these pieces together. The DST and replay proposals (EF_II #8, #12) demonstrate the testing side of concurrency but do not address the programming model (structured concurrency, cancellation, Send/Sync, async drop).

4. **Proposal-to-commitment gap**: 21% of the design space is at "proposed" status. These proposals are well-researched but carry risk: none have been implemented, some depend on unbuilt infrastructure (incremental type checker, SMT solver integration), and the proposal layer is disproportionately focused on effects and content addressing while underexploiting generational memory (25%) and multiple dispatch (13%). Committing to proposals requires implementation evidence, not just design documents.

### Critical Questions

1. **Monomorphization × content addressing (F-01):** No proposal or document addresses this interaction despite it being at the intersection of two core innovations. This remains the highest-priority architectural question.

2. **Concurrency composition (F-06):** Effects + fibers + structured concurrency + cancellation need a cohesive design. The pieces exist but the composition doesn't.

3. **Compiler-as-a-library (F-07):** The constrained decoding oracle (Proposal #16) and verification cache (Proposal #18) both assume a query-based, incremental compiler architecture that does not yet exist. These proposals implicitly require the compiler-as-a-library architecture but do not explicitly call it out as a prerequisite.

4. **Proposal dependency depth:** The critical path `#20 → #7 → #18 → #10` (Spec Annotations → Verification → Cache → Proof-Carrying Code) is four proposals deep. Each link must work before the next can begin. The risk of a single foundational proposal proving impractical is significant.
