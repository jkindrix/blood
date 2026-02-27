# Blood: Features That Could Make It Extraordinary — Part II

**Status:** Research & Ideation
**Date:** 2026-02-26
**Prerequisite:** Read `EXTRAORDINARY_FEATURES.md` (Proposals 1–7) and `SAFETY_LEVELS.md` first.

---

## Context

Part I (`EXTRAORDINARY_FEATURES.md`) proposed seven features leveraging Blood's unique combination of algebraic effects, content-addressed code, generational memory safety, and multiple dispatch. Those proposals focused on verification, security, and performance.

This document extends the set with **Proposals 8–15**, discovered through systematic analysis of the formal verification landscape, unsolved systems programming problems, and cutting-edge PL research (POPL 2025, ICFP 2025, PLDI 2025, OOPSLA 2025). These proposals share a unifying insight:

> **An algebraic effect handler is a universal interception mechanism.** Debugging, tracing, fault injection, sandboxing, caching, mocking, logging, and resource management are all instances of the same concept: intercepting and transforming effects.

Content-addressing adds **identity** (sound caching, version tracking, distributed deployment). Generational memory adds **temporal ordering** (deterministic allocation, lifecycle tracking). Multiple dispatch adds **type specialization** (different strategies for different resource types).

Every proposal below exploits the *intersection* of these features — a design space that no research community is exploring because each community treats these innovations independently.

### Combined Proposal Map (Parts I, II, and III)

| # | Proposal | Document | Pillars | Priority |
|---|----------|----------|---------|----------|
| 1 | Compile-Time WCET Analysis | Part I | Effects | 4 |
| 2 | Session Types for Protocol Safety | Part I | Effects + CAS | 3 |
| 3 | Automatic Memoization via Content-Addressing | Part I | CAS + Effects | 6 |
| 4 | Capability-Based Security via Effects | Part I | Effects | 2 |
| 5 | Effect-Guided Automatic Parallelization | Part I | Effects + Gen | 5 |
| 6 | Provenance Tracking for Compliance | Part I | CAS + Effects | 7 |
| 7 | Gradual Verification (Lightweight Proofs) | Part I | Effects + CAS + Gen | 1 |
| 8 | Deterministic Simulation Testing | **Part II** | Effects + CAS + Gen + MD | **A** |
| 9 | Taint Tracking via Effects | **Part II** | Effects + CAS | **B** |
| 10 | Proof-Carrying Code | **Part II** | CAS + Verification | **C** |
| 11 | Automatic Semantic Versioning | **Part II** | Effects + CAS | **D** |
| 12 | Deterministic Replay Debugging | **Part II** | Effects + CAS + Gen | **E** |
| 13 | Zero-Code Observability | **Part II** | Effects + CAS | **F** |
| 14 | Choreographic Programming | **Part II** | Effects + CAS | **G** |
| 15 | Compile-Time Complexity Bounds | **Part II** | Effects + CAS | **H** |
| 16 | Type-and-Effect Constrained Decoding Oracle | **Part III** | Effects + Types | **A** |
| 17 | Machine-Readable Structured Diagnostics | **Part III** | Toolchain | **B** |
| 18 | Content-Addressed Verification Cache | **Part III** | CAS + Verification | **A** |
| 19 | Compact Module Signatures as AI Context | **Part III** | Effects + CAS | **B** |
| 20 | First-Class Specification Annotations | **Part III** | Effects + CAS + Verification | **A** |
| 21 | AI-Optimized Syntax Decisions | **Part III** | Syntax | **C** |
| 22 | Toolchain-Integrated Dependency Graph API | **Part III** | CAS + Toolchain | **B** |
| 23 | Effect Handlers as AI Agent Middleware | **Part III** | Effects + CAS + Gen + MD | **C** |

Legend: CAS = Content-Addressed Storage/Code, Gen = Generational Memory, MD = Multiple Dispatch

**Part III** (`EXTRAORDINARY_FEATURES_III.md`) focuses on making Blood AI-native — features that make Blood the best language for AI to write, verify, and reason about.

---

## Proposal 8: Deterministic Simulation Testing as a Language Feature

### The Problem Nobody Has Solved at the Language Level

FoundationDB found every bug in their distributed database by running trillions of simulated hours via Deterministic Simulation Testing (DST). TigerBeetle uses the same technique. Antithesis built an entire company ($50M+ funded) around providing DST as a service because it requires such heroic engineering effort.

DST requires total control over all sources of nondeterminism: I/O, scheduling, time, randomness, network ordering. In every existing implementation, this means building custom I/O layers, custom schedulers, custom time sources — thousands of lines of infrastructure code per project. The technique is proven to be transformatively effective, and it is inaccessible to nearly everyone.

### Why Only Blood Can Do This

A function typed `/ {IO, Time, Random, Network}` declares *exactly* what needs to be simulated. The effect system is a complete manifest of nondeterminism boundaries. A simulation handler intercepts all effects and replaces them with deterministic simulators. The compiler *guarantees* completeness: if a nondeterministic effect is not handled, the program does not compile.

### Proposed Design

```blood
// Normal code — developer writes this, no simulation awareness needed
fn handle_request(req: Request) -> Response / {Network, Time, Storage, Random} {
    let now = perform Time.now();
    let data = perform Storage.read(req.key);
    let jitter = perform Random.range(0, 100);
    perform Time.sleep(Duration::ms(jitter));
    perform Network.send(peer, data);
    Response::ok(data)
}

// Simulation handler — provided by Blood's test framework
fn simulate<T>(seed: u64, f: fn() -> T / {Network, Time, Storage, Random}) -> T {
    let sim = Simulator::new(seed);
    with sim.handle_network,
         sim.handle_time,
         sim.handle_storage,
         sim.handle_random
    handle {
        f()
    }
    // Every effect intercepted. Every "random" value derived from seed.
    // Every "network delay" controlled by simulator.
    // Entire execution is deterministic and reproducible.
}

// Test: run 10 million simulated scenarios
#[test]
fn fuzz_distributed_protocol() {
    for seed in 0..10_000_000 {
        simulate(seed, || {
            run_three_node_cluster()
        });
    }
}
```

### The Simulation Harness

```blood
// The simulator controls all nondeterminism sources
struct Simulator {
    rng: DeterministicRng,
    clock: SimulatedClock,
    network: SimulatedNetwork,    // Controllable latency, reordering, partitions
    storage: SimulatedStorage,    // Controllable latency, corruption, failures
}

impl Simulator {
    fn new(seed: u64) -> Self {
        // All randomness derived from single seed
        // Entire execution is a pure function of the seed
        Simulator {
            rng: DeterministicRng::from_seed(seed),
            clock: SimulatedClock::new(),
            network: SimulatedNetwork::new(seed),
            storage: SimulatedStorage::new(seed),
        }
    }
}

// Fault injection is just a handler policy
impl Simulator {
    fn with_network_partition(
        &mut self,
        partition: Partition,  // Which nodes are isolated
        duration: Duration,    // How long the partition lasts
    ) -> &mut Self {
        self.network.schedule_partition(partition, duration);
        self
    }

    fn with_disk_corruption(
        &mut self,
        probability: f64,      // Probability per write
    ) -> &mut Self {
        self.storage.set_corruption_rate(probability);
        self
    }
}
```

### What Each Pillar Contributes

| Pillar | Contribution |
|---|---|
| **Effects** | Complete manifest of nondeterminism boundaries; compiler guarantees no unhandled nondeterminism |
| **Content-addressing** | Exact code version identified by hash; test results cacheable across CI runs; if `#abc123` passed seeds `0..10M`, cache that result |
| **Generational memory** | Deterministic allocation ordering; no GC pauses corrupting timing simulation; address-dependent behavior eliminated |
| **Multiple dispatch** | Different simulation strategies for different effect types (network simulation differs from storage simulation) |

### Comparison to Existing Approaches

| Approach | Completeness | Overhead | Developer Effort | Cross-Network |
|---|---|---|---|---|
| **FoundationDB** (custom framework) | Manual | Low | Thousands of lines per project | Yes (custom) |
| **Antithesis** (VM-level determinism) | Complete | High (full VM) | Low (external service) | Yes |
| **rr** (Linux record/replay) | Complete | 15-40% | Low | No |
| **Blood DST** (effect-level simulation) | Complete (by construction) | ~0% (handlers are normal code) | Zero (write normal effects) | Yes (effects cross boundaries) |

### Impact

Blood would be the **first language where deterministic simulation testing is a zero-effort language feature**. Developers write normal effectful code; the test framework provides simulation handlers; the compiler guarantees that no nondeterminism escapes. The entire distributed systems testing methodology pioneered by FoundationDB becomes accessible to every Blood developer.

**References:**
- [FoundationDB Simulation Testing](https://apple.github.io/foundationdb/testing.html)
- [Antithesis: Deterministic Simulation Testing](https://antithesis.com/resources/deterministic_simulation_testing/)
- [DST Primer for Unit Test Maxxers](https://www.amplifypartners.com/blog-posts/a-dst-primer-for-unit-test-maxxers)
- [TigerBeetle: Simulation Testing](https://tigerbeetle.com/blog/2023-07-11-we-put-a-distributed-database-in-the-browser/)

---

## Proposal 9: Taint Tracking via Effects (Information Flow Control)

### The Problem

Taint analysis tracks how untrusted data propagates through programs to prevent SQL injection, XSS, data exfiltration, and supply chain attacks. Existing approaches are either:

- **Static** (CodeQL, Snyk, Semgrep): Imprecise, many false positives, no runtime guarantees
- **Dynamic** (TaintDroid, libdft): Runtime overhead (5-30%), no compile-time guarantees

No language makes taint propagation a type-level concern. This is becoming critical in the AI agent era: LLM agents that read untrusted web content and then execute code need taint tracking to prevent prompt injection from reaching privileged operations.

### Why Only Blood Can Do This

Effects are already an information flow tracking mechanism. A function that declares `/ {Network}` has data that may have come from the network. This is taint information hiding in plain sight.

### Proposed Design

```blood
// Tainted data sources are effects
effect UntrustedInput {
    op read_user_input() -> String;
    op read_http_body() -> Bytes;
    op read_environment_var(name: &str) -> String;
}

// Sanitization is an effect handler that converts tainted to clean
handler HtmlSanitize for UntrustedInput {
    read_user_input() => {
        let raw = resume();
        html_escape(raw)  // Returns clean data
    },
    read_http_body() => {
        let raw = resume();
        validate_json_schema(raw)?  // Returns validated data
    },
    read_environment_var(name) => {
        let raw = resume();
        validate_config_value(name, raw)
    }
}

handler SqlSanitize for UntrustedInput {
    read_user_input() => {
        let raw = resume();
        parameterize(raw)  // Returns parameterized query fragment
    },
    // ... other operations
}
```

### The Structural Guarantee

```blood
// Functions that accept only clean data have no UntrustedInput effect
fn execute_sql(query: Query) / {Database} {
    // This function's effect signature does NOT include UntrustedInput.
    // It is STRUCTURALLY IMPOSSIBLE to pass tainted data here
    // without going through a sanitization handler.
    perform Database.execute(query);
}

// The compiler PREVENTS this:
fn vulnerable() / {UntrustedInput, Database} {
    let raw = perform UntrustedInput.read_user_input();
    execute_sql(Query::from(raw));
    // COMPILE ERROR: Query::from(raw) carries UntrustedInput taint
    // but execute_sql requires clean input (no UntrustedInput in signature)
}

// The compiler ALLOWS this:
fn safe() / {UntrustedInput, Database} {
    let clean = with SqlSanitize handle {
        perform UntrustedInput.read_user_input()
    };
    // clean has been through the sanitization handler
    // UntrustedInput effect has been discharged
    execute_sql(Query::from(clean));  // OK
}
```

### Advanced: Multi-Level Security Labels

```blood
// Define security levels as effect hierarchies
effect Secret {
    op read_secret() -> Bytes;
}

effect Classified {
    op read_classified() -> Bytes;
}

// Downgrade requires explicit declassification handler
handler Declassify for Secret {
    read_secret() => {
        let data = resume();
        audit_log("Declassification", content_hash(), caller());
        redact(data)  // Returns redacted version
    }
}

// Content-addressing adds: every declassification is recorded with
// the exact function hash that performed it, creating an immutable
// audit trail for security reviews.
```

### What Each Pillar Contributes

| Pillar | Contribution |
|---|---|
| **Effects** | Taint is an effect; sanitization is a handler; the type system prevents tainted data from reaching sinks |
| **Content-addressing** | Taint provenance is hashable — trace exactly which version of which sanitizer processed which input |
| **Generational memory** | Tainted objects can carry generation metadata that survives across function boundaries |

### Impact

Blood would offer **compile-time taint tracking** that is:

- **Complete**: Every data source that could be tainted is declared as an effect; the compiler ensures no tainted data reaches a sensitive sink without sanitization
- **Sound**: No false negatives — if the compiler accepts the code, every tainted path goes through a handler
- **Zero runtime overhead**: Taint is tracked in the type system, not at runtime
- **Auditable**: Content-addressed sanitization functions create a verifiable chain of custody

This is stronger than any existing static or dynamic taint analysis. The structural guarantee — that tainted data *cannot reach* a sink without passing through a handler — is a property of the type system, not an approximation.

**References:**
- [Information Flow Control (Chalmers)](https://www.cse.chalmers.se/~andrei/mod11.pdf)
- [Addressing Agentic Risks with Taint Analysis](https://www.pillar.security/blog/addressing-vertical-agentic-risks-with-taint-analysis)
- [Taint Analysis Guide for Developers](https://can-ozkan.medium.com/what-is-taint-analysis-a-guide-for-developers-and-security-researchers-11f2ad876ea3)

---

## Proposal 10: Proof-Carrying Code via Content-Addressed Proofs

### The Problem

When code is deployed — shipped to a customer, deployed to a cluster node, loaded as a plugin, generated by an AI — the receiving system has no way to verify correctness without re-verifying from scratch. This was an [ICFP 2025 keynote topic](https://icfp25.sigplan.org/details/icfp-2025-icfp-keynotes/2/Proof-Carrying-Neuro-Symbolic-Code) specifically in the context of LLM-generated code.

George Necula proposed proof-carrying code in 1997. It has never been made practical in a general-purpose language because code identity is unstable — recompilation, optimization, or even whitespace changes produce different binaries with no way to link them back to their proofs.

### Why Only Blood Can Do This

Content-addressed functions have stable, unique identities independent of compilation. A function's hash is derived from its semantic content, not its binary representation. If a verification proof is itself content-addressed and linked to the function hash, the proof *travels with the code*.

### Proposed Design

```blood
// Function verified at Level 3 (#[verify])
#[verify]
fn transfer(from: &mut Account, to: &mut Account, amount: Money) -> Result<(), Error>
    requires from.balance >= amount
    ensures from.balance == old(from.balance) - amount
    ensures to.balance == old(to.balance) + amount
{
    if from.balance < amount {
        return Err(Error::InsufficientFunds);
    }
    from.balance = from.balance - amount;
    to.balance = to.balance + amount;
    Ok(())
}
```

### The Proof Artifact

```
Compiled artifact for transfer():
  ┌─────────────────────────────────────────┐
  │ Function code hash:  #abc123            │
  │ Contract hash:       #contract789       │
  │ Proof artifact hash: #proof456          │
  │ Proof type:          SMT (Z3 + CVC5)   │
  │ Verification level:  3 (#[verify])      │
  │ Compiler version:    blood 0.9.0        │
  │ Binding signature:   sign(#abc123,      │
  │                           #proof456)    │
  └─────────────────────────────────────────┘
```

### Deployment Scenarios

```blood
// Scenario 1: Distributed deployment (Unison-style)
// Node A sends function transfer() to Node B
// Node B receives:
//   - Function code (hash #abc123)
//   - Proof artifact (hash #proof456)
//   - Binding (compiler-signed)
//
// Node B verifies:
//   1. Does proof #proof456 verify function #abc123? → Check
//   2. Is the binding signature valid? → Check
//   3. Accept function without re-verification

// Scenario 2: Plugin loading
fn load_plugin(path: Path) -> Plugin / {FileSystem} {
    let plugin = perform FileSystem.read_plugin(path);

    // Check verification level
    match plugin.verification_level() {
        Level::Proven => {
            // Proof travels with code; verify binding, skip re-verification
            assert!(plugin.verify_proof_binding());
            plugin
        },
        Level::Verified => {
            // SMT proof included; verify or trust based on policy
            plugin
        },
        Level::Contracted => {
            // Runtime contracts will be checked during execution
            plugin
        },
        Level::Unsafe => {
            // No proof, no contracts — sandbox via capability attenuation
            plugin.with_restricted_capabilities()
        },
    }
}

// Scenario 3: AI-generated code
// LLM generates function + contract
// Blood compiler verifies contract → attaches proof
// Proof artifact proves the AI-generated code is correct
// Human reviews the contract (readable), trusts the proof (machine-checked)
```

### Integration with the Verification Continuum

| Level | What Travels with the Code |
|---|---|
| Level 0 (`#[unsafe]`) | Nothing — receiver must sandbox or trust blindly |
| Level 1 (default) | Runtime check metadata — receiver knows which checks are active |
| Level 2 (`requires`/`ensures`) | Contract definitions — receiver can check at runtime |
| Level 3 (`#[verify]`) | SMT proof artifact — receiver can verify binding without re-solving |
| Level 4 (`#[prove]`) | Full proof term — receiver can type-check proof independently |

### Impact

Blood would be the **first language where code carries its own machine-checkable correctness proof as a standard artifact**, identified by content hash and verifiable by any receiving system. This makes verified code *deployable* — not just verified on one machine, but provably correct everywhere it runs.

**References:**
- [Proof-Carrying Code (Necula, 1997)](https://www.cs.cmu.edu/~necula/Papers/pcc97.pdf)
- [Proof-Carrying Neuro-Symbolic Code (ICFP 2025 Keynote)](https://icfp25.sigplan.org/details/icfp-2025-icfp-keynotes/2/Proof-Carrying-Neuro-Symbolic-Code)
- [Proof-Carrying Code Completions (PC3)](https://web.cs.ucdavis.edu/~cdstanford/doc/2024/ASEW24b.pdf)
- [Verified Compilation from Lean to C](https://www.researchgate.net/publication/397883522)

---

## Proposal 11: Automatic Semantic Versioning via Effect Signatures

### The Problem

When a library evolves, determining whether the change is breaking (major), additive (minor), or internal (patch) requires human judgment. Semantic versioning is widely adopted but manually maintained and frequently wrong. Cargo, npm, and pip all depend on authors correctly classifying their changes — and authors regularly get it wrong, causing cascading dependency failures across ecosystems.

Tools like `cargo-semver-checks` and `elm-package` do partial API diff checking, but none can detect behavioral changes or provide sound "nothing changed" guarantees.

### Why Only Blood Can Do This

Every function has a content hash. Every function has an effect signature. The compiler can automatically compute the semver classification with mathematical precision.

### Proposed Design

```bash
$ blood semver --compare v1.0.0..HEAD

Semantic Version Analysis
=========================

MAJOR changes (breaking):
  fetch(url: Url) -> Data / {Network, Error<HttpError>}
  fetch(url: Url, opts: Options) -> Data / {Network, Error<HttpError>}
    → Parameter added (callers will not compile)

  parse(input: &str) -> Ast / {Error<ParseError>, Error<ParseWarning>}
  parse(input: &str) -> Ast / {Error<ParseError>}
    → Effect NARROWED: Error<ParseWarning> removed
    → Callers handling ParseWarning have dead code (may indicate logic change)

MINOR changes (additive):
  + validate(input: &str) -> bool / pure             [NEW FUNCTION]
  + retry(f: fn() -> T / {Network}) -> T / {Network} [NEW FUNCTION]

  connect(addr: Addr) -> Conn / {Network, Error<NetError>}
  connect(addr: Addr) -> Conn / {Network, Error<NetError>, Error<TimeoutError>}
    → Effect WIDENED: Error<TimeoutError> added
    → Callers may need new error handler

PATCH changes (internal only):
  process: hash changed (#a1b2 → #c3d4), signature identical
    → Internal optimization, no behavioral change visible to callers
  sort: hash unchanged (#e5f6)
    → No change whatsoever

Recommended version: 2.0.0 (2 breaking changes detected)
```

### The Classification Rules

| Change Type | Content Hash | Effect Signature | Classification |
|---|---|---|---|
| No change | Same | Same | No version bump needed |
| Internal optimization | Changed | Same | **Patch** |
| New effect added | Changed | Widened | **Minor** (new failure mode) |
| Effect removed | Changed | Narrowed | **Major** (caller error handlers become dead code) |
| Parameter added/removed | Changed | Changed | **Major** (callers won't compile) |
| Return type changed | Changed | Changed | **Major** (callers won't compile) |
| New function added | N/A | N/A | **Minor** |
| Function removed | N/A | N/A | **Major** |

### The Soundness Guarantee

Content-addressing provides something no other semver tool can offer: a **provably correct "patch" classification**. If a function's content hash didn't change, its behavior didn't change — period. This is not an approximation; it is a mathematical identity. No existing semver checker can make this guarantee.

### Impact

Blood would be the **first language with provably sound automatic semantic versioning**. Library authors never manually classify versions again. Consumers trust that "patch" means "identical behavior" because content hashes don't lie.

**References:**
- [Semantic Versioning (semver.org)](https://semver.org/)
- [Static Detection of SemVer Violations (arXiv)](https://arxiv.org/abs/2209.00393)
- [Putting Semantics into Semantic Versioning (ACM)](https://dl.acm.org/doi/10.1145/3426428.3426922)
- [cargo-semver-checks](https://github.com/obi1kenobi/cargo-semver-checks)

---

## Proposal 12: Deterministic Replay Debugging (Time-Travel)

### The Problem

Debugging concurrent and distributed systems requires reproducing exact execution sequences. Current time-travel debuggers operate at the OS/binary level:

| Tool | Level | Overhead | Cross-Network | Platform |
|---|---|---|---|---|
| rr (Mozilla) | syscall | 15-40% | No | Linux only |
| UndoDB | instruction | 2-5x | No | Linux only |
| Microsoft TTD | instruction | 2-10x | No | Windows only |

All require kernel cooperation, impose significant overhead, and cannot cross process or network boundaries. None work at the language level.

### Why Only Blood Can Do This

This is the debugging counterpart to Proposal 8 (DST). The same effect interception mechanism that enables simulation also enables recording and replay — but with orders-of-magnitude less overhead because only effect boundaries are recorded, not every instruction.

### Proposed Design

```blood
// Record mode: capture every effect invocation and result
fn record<T>(f: fn() -> T / {IO, Time, Network}) -> (T, EffectTrace) {
    let trace = EffectTrace::new();
    let result = with trace.recording_handler handle { f() };
    (result, trace)
}

// Replay mode: substitute recorded results for real effects
fn replay<T>(trace: EffectTrace, f: fn() -> T / {IO, Time, Network}) -> T {
    with trace.replay_handler handle { f() }
}
```

### The Developer Experience

```bash
# Record a failing run
$ blood run --record=trace.bin my_server
# Server runs normally. Effects recorded at boundaries only.
# Overhead: ~2-5% (only effect invocations, not every instruction)

# Replay deterministically
$ blood debug --replay=trace.bin my_server
Replaying from trace (487,293 effect invocations recorded)
Code version: #abc123def (verified match ✓)

> break handle_request
> continue
Breakpoint hit: handle_request() at src/server.blood:42
> step-backward        # ← This works because effects are checkpoints
Rewound to: perform Storage.read(key) at src/server.blood:38
> inspect key
key = "user:1234"
> inspect @effect_result
StorageResult::Ok({name: "Alice", balance: 150})
```

### What Makes This Different from rr

| Dimension | rr / UndoDB | Blood Replay |
|---|---|---|
| Recording granularity | Every syscall / instruction | Effect boundaries only |
| Overhead | 15-40% / 2-5x | ~2-5% |
| Cross-process | No | Yes (effects cross boundaries) |
| Cross-network | No | Yes (network effects are recorded on both sides) |
| Code version verification | No | Content hash verification |
| Platform | Linux only | Any platform Blood targets |
| Deterministic replay guarantee | Probabilistic (depends on recording completeness) | Structural (all nondeterminism is effects; effects are recorded) |

### Distributed Time-Travel Debugging

```blood
// Record across multiple nodes
$ blood run --record=trace_node_a.bin node_a &
$ blood run --record=trace_node_b.bin node_b &
$ blood run --record=trace_node_c.bin node_c &

// Replay with synchronized traces
$ blood debug --replay=trace_node_a.bin,trace_node_b.bin,trace_node_c.bin
# All three nodes replay in lockstep
# Network effects on node A match receive effects on node B
# Causal ordering is reconstructed from effect traces
```

### Impact

Blood would offer **language-level time-travel debugging** with:
- Orders of magnitude less overhead than instruction-level tools
- Cross-process and cross-network replay
- Content-hash verification that replayed code matches recorded code
- Structural completeness guarantee (all nondeterminism captured)

**References:**
- [Deterministic Record-and-Replay (CACM)](https://cacm.acm.org/practice/deterministic-record-and-replay/)
- [Time Travel Debugging (Wikipedia)](https://en.wikipedia.org/wiki/Time_travel_debugging)
- [rr: Record and Replay](https://rr-project.org/)
- [Debugging Distributed Systems (ACM Queue)](https://queue.acm.org/detail.cfm?id=2940294)

---

## Proposal 13: Zero-Code Observability

### The Problem

Observability (tracing, metrics, logging) requires manual instrumentation. OpenTelemetry adoption means adding spans to every function, every library, every service. The instrumentation code pollutes business logic, is inconsistent and incomplete, and there is no guarantee that all effectful operations are traced.

### Why Only Blood Can Do This

Every effect invocation is a natural tracing span. An observability handler wraps all effects with timing and metadata without modifying application code.

### Proposed Design

```blood
// Application code — ZERO observability instrumentation
fn process_order(order: Order) -> Receipt / {Database, Payment, Email} {
    let inventory = perform Database.check_stock(order.items);
    let charge = perform Payment.charge(order.customer, order.total);
    let receipt = Receipt::new(order, charge);
    perform Email.send(order.customer.email, receipt.to_html());
    receipt
}

// Observability handler — provided by infrastructure, not application
handler Traced<E: Effect> for E {
    op(args...) => {
        let span = Span::start(
            name: effect_name::<E>(),
            function_hash: content_hash(),  // Exact code version in every span
            args: args.debug_repr(),
        );
        let result = resume(args...);
        span.finish(
            status: if result.is_ok() { "ok" } else { "error" },
            duration: span.elapsed(),
        );
        result
    }
}

// Metrics handler — count effect invocations by type
handler Metered<E: Effect> for E {
    op(args...) => {
        counter(effect_name::<E>()).increment();
        histogram(effect_name::<E>() + ".duration").start();
        let result = resume(args...);
        histogram(effect_name::<E>() + ".duration").record();
        result
    }
}

// Wire it up at the entry point — infrastructure concern, not application
fn main() / {Database, Payment, Email, IO} {
    with Traced<Database>,
         Traced<Payment>,
         Traced<Email>,
         Metered<Database>,
         Metered<Payment>
    handle {
        serve_requests()
    }
}
```

### Content-Addressed Trace Correlation

Every span includes the exact function hash. This provides something no existing observability tool offers:

```
Trace: process_order (function #7f3a2b)
  ├── Database.check_stock (function #7f3a2b, effect #db001)  12ms
  ├── Payment.charge (function #7f3a2b, effect #pay002)       340ms
  └── Email.send (function #7f3a2b, effect #email003)         89ms

// Six months later, investigating a regression:
// "Which version of process_order produced this trace?"
// Answer: #7f3a2b — look it up in the content-addressed code store.
// Exact code, not "the version that was deployed around that time."
```

### Guaranteed Completeness

The effect type system guarantees that every effectful operation is observed. If `process_order` declares `/ {Database, Payment, Email}`, and observability handlers are installed for all three, then every database query, every payment, and every email is traced. There is no "we forgot to instrument this code path."

### Switching Backends

```blood
// Switch from Jaeger to Zipkin: change the handler, not the application
handler JaegerTraced<E: Effect> for E { /* Jaeger-specific spans */ }
handler ZipkinTraced<E: Effect> for E { /* Zipkin-specific spans */ }

// Zero application code changes. Zero.
```

### Impact

Blood would offer **zero-code, guaranteed-complete observability** where:
- Application code contains no instrumentation
- Every effectful operation is automatically traced and metered
- Traces include exact code version via content hash
- Backend switching is a handler swap, not a code rewrite

**References:**
- [Domain-Oriented Observability (Fowler)](https://martinfowler.com/articles/domain-oriented-observability.html)
- [Effect Library: Tracing](https://effect.website/docs/observability/tracing/)
- [wasmCloud: Capabilities as Managed Algebraic Effects](https://wasmcloud.com/blog/wasmcloud-capabilities-are-managed-algebraic-effects-for-webassembly-functions/)

---

## Proposal 14: Choreographic Programming via Effects

### The Problem

Distributed systems require multiple participants to follow a shared protocol. Today, each participant's code is written separately and correctness depends on the developer manually ensuring they match. Protocol violations — messages in wrong order, missing handlers, type mismatches, deadlocks — are caught at runtime or not at all.

Choreographic programming solves this: write ONE global program describing ALL participants' interactions, then automatically compile it into per-participant code. Deadlock freedom is guaranteed by construction. It exists only in research languages (Choral, Chorex, HasChor). Recent PLDI 2025 work on "census polymorphism" allows abstracting over the number of participants.

### Why Blood Has Unique Affinity

Session types (Proposal 2, Part I) are the binary case (two participants). Choreographic programming is the general case (N participants). Effects model communication naturally, and content-addressing means the choreography definition serves as a shared protocol contract.

### Proposed Design

```blood
// Global choreography — describes ALL participants
choreography TwoPhaseCommit {
    participants: Coordinator, Worker[N];

    // Phase 1: Prepare
    Coordinator -> Worker[*]: Prepare(transaction);
    Worker[i] -> Coordinator: Vote(prepared | aborted);

    // Decision
    let decision = if Worker[*].all_prepared() {
        Commit
    } else {
        Abort
    };

    // Phase 2: Commit/Abort
    Coordinator -> Worker[*]: Decision(decision);
    Worker[i] -> Coordinator: Ack;
}

// The compiler generates per-participant implementations:
//
// fn coordinator_impl(workers: &[Channel]) / {Network, Time}
//   - Sends Prepare to all workers
//   - Collects Votes
//   - Sends Decision
//   - Collects Acks
//
// fn worker_impl(coordinator: Channel) / {Network, Storage}
//   - Receives Prepare
//   - Sends Vote (based on local state)
//   - Receives Decision
//   - Sends Ack
//
// BOTH are guaranteed deadlock-free by construction.
// The protocol state machine is derived from the choreography.
```

### Content-Addressed Protocol Contracts

```blood
// The choreography definition has a content hash: #choreo_abc123
// All participants can verify they're running the same protocol:
//
// Node A: "My protocol hash is #choreo_abc123"
// Node B: "My protocol hash is #choreo_abc123"
// → Match: protocol-compatible
//
// After a protocol update:
// Node A: "My protocol hash is #choreo_def456"
// Node B: "My protocol hash is #choreo_abc123"
// → Mismatch: incompatible versions, refuse connection
```

### Relationship to Session Types (Proposal 2)

| Feature | Session Types (Proposal 2) | Choreographic Programming (Proposal 14) |
|---|---|---|
| Participants | 2 (binary) | N (multiparty) |
| Specification | Per-participant protocol | Global choreography |
| Deadlock freedom | Via protocol well-formedness | By construction |
| Implementation | Developer writes both sides | Compiler generates per-participant code |
| Complexity | Low | High |
| Recommended phase | Phase 1 | Phase 2 (builds on session types) |

### Impact

Blood would offer **compile-time verified distributed protocols for N participants**, where a single choreography specification generates all participant implementations with deadlock freedom guaranteed by construction.

**References:**
- [Choreographic Programming (PLDI 2025)](https://pldi25.sigplan.org/details/pldi-2025-papers/47/Efficient-Portable-Census-Polymorphic-Choreographic-Programming)
- [Chorex: Restartable Choreographies](https://programming-journal.org/2025/10/20/)
- [Multiparty Session Types](http://mrg.doc.ic.ac.uk/publications/a-very-gentle-introduction-to-multiparty-session-types/main.pdf)
- [HasChor: Functional Choreographic Programming](https://dl.acm.org/doi/10.1145/3607849)

---

## Proposal 15: Compile-Time Complexity Bounds

### The Problem

Algorithmic complexity bugs ("accidentally quadratic") cause production outages. The Rust standard library had a quadratic `Display` impl for `IpAddr` that went undetected for years. Cloudflare had a global outage from a regex with catastrophic backtracking. No language prevents a function annotated as O(n) from accidentally implementing O(n²).

Current detection is entirely post-hoc: profiling, load testing, or AI-based analysis tools that guess complexity from runtime behavior.

### Why Blood Can Do This

Effects with resource annotations can track computational complexity as a type-level concern. The key insight: for *pure functions* with bounded loops and structurally decreasing recursion, complexity analysis is decidable. Blood's effect system identifies exactly which functions are pure, making the analysis tractable.

### Proposed Design

```blood
// Declare complexity bounds alongside the function
fn merge_sort(data: &mut [T]) / pure
    @ complexity(time: O(n * log(n)), space: O(n))
    where n = data.len()
{
    if data.len() <= 1 { return; }
    let mid = data.len() / 2;
    merge_sort(&mut data[..mid]);     // T(n/2)
    merge_sort(&mut data[mid..]);     // T(n/2)
    merge(&mut data, mid);            // O(n)
    // Compiler verifies: T(n) = 2*T(n/2) + O(n) = O(n log n) ✓
}

fn naive_contains(haystack: &[T], needles: &[T]) -> Vec<T> / pure
    @ complexity(time: O(n * m))
    where n = haystack.len(), m = needles.len()
{
    let mut result = Vec::new();
    for needle in needles {           // O(m) iterations
        for item in haystack {        // O(n) iterations per
            if item == needle {
                result.push(item);    // This push is O(1) amortized
            }
        }
    }
    result
    // Compiler verifies: O(m) * O(n) * O(1) = O(n * m) ✓
}

// What happens when the declared bound is wrong:
fn buggy_sort(data: &mut [T]) / pure
    @ complexity(time: O(n * log(n)))
    where n = data.len()
{
    // Bubble sort implementation...
    for i in 0..data.len() {
        for j in 0..data.len() {
            if data[j] > data[j+1] { swap(&mut data[j], &mut data[j+1]); }
        }
    }
    // COMPILE ERROR: Analyzed complexity O(n²) exceeds declared bound O(n log n)
}
```

### The Analysis

```bash
$ blood build --complexity-report

Complexity Analysis Report
==========================
merge_sort:
  Declared: O(n * log(n)) time, O(n) space
  Analyzed: O(n * log(n)) time, O(n) space  ✓

naive_contains:
  Declared: O(n * m) time
  Analyzed: O(n * m) time  ✓

process_requests:
  Declared: none
  Analyzed: O(n) time (linear in request count)
  NOTE: No declared bound — consider adding @ complexity annotation

buggy_handler:
  Declared: none
  Analyzed: COULD NOT DETERMINE (contains effectful loop with dynamic bound)
  NOTE: Only pure functions with bounded loops can be analyzed
```

### Honest Limitations

This does not require solving the halting problem. The analysis is restricted to:

- **Pure functions** (`/ pure`) — no effects means no hidden costs
- **Bounded loops** — loop bounds must be derivable from input sizes
- **Structurally decreasing recursion** — recursive calls must demonstrably reduce toward a base case
- **Known-cost operations** — standard library functions have declared complexity

Functions that don't fit these constraints get `@ complexity(unknown)` — the analysis is honest about what it can and cannot determine. This is the same philosophy as WCET analysis (Proposal 1): start with what's tractable, be honest about limits.

Content-addressing adds: complexity analysis results are cached by function hash. If function `#abc123` was analyzed as O(n log n), that result persists until the function changes.

### Impact

Blood would be the **first language with compile-time complexity verification** for the subset of code where analysis is tractable (pure functions with bounded iteration). This catches accidentally-quadratic bugs, regex catastrophic backtracking, and other algorithmic performance regressions before they reach production.

**References:**
- [Big O Analysis Tools](https://www.bigocalc.com/)
- [Accidentally Quadratic](https://accidentallyquadratic.tumblr.com/)
- [Cloudflare Outage (Regex Backtracking)](https://blog.cloudflare.com/details-of-the-cloudflare-outage-on-july-2-2019/)
- [WCET Analysis (related — Proposal 1, Part I)](https://www.cs.fsu.edu/~whalley/papers/tecs07.pdf)

---

## Priority Ranking (Part II Proposals)

| Priority | Proposal | Rationale |
|---|---|---|
| **A** | **Deterministic Simulation Testing** | Highest novelty × impact product. Leverages all four pillars. Proven transformative value (FoundationDB). Zero-effort for developers. Would make Blood the language of choice for distributed systems. |
| **B** | **Taint Tracking via Effects** | Low implementation effort (design pattern on existing effect system). Universally relevant security feature. Becoming critical in the AI agent era. Structural guarantee stronger than any existing taint analysis. |
| **C** | **Proof-Carrying Code** | Natural extension of gradual verification (Proposal 7) and content-addressing. Makes verified code *deployable*. Becomes more valuable as AI code generation grows. Requires Proposal 7 first. |
| **D** | **Automatic Semantic Versioning** | Extremely low effort (compiler tool, not language feature). Immediately useful to every library author. Content-addressed "patch" classification is provably sound — no other tool can claim this. |
| **E** | **Deterministic Replay Debugging** | Shares infrastructure with Proposal 8 (DST). Orders of magnitude cheaper than instruction-level tools. Cross-network debugging is genuinely novel. Build after DST. |
| **F** | **Zero-Code Observability** | Low effort (standard library handlers). Universally useful. Guaranteed completeness via effect types. Content-addressed trace correlation is novel. |
| **G** | **Choreographic Programming** | High value for distributed systems. Builds on session types (Proposal 2). High implementation effort. Phase 2 feature. |
| **H** | **Compile-Time Complexity Bounds** | High novelty but high effort. Restricted to pure functions with bounded loops. Start with simple cases; expand as analysis improves. |

### Suggested Implementation Order

**Phase 1** (build on existing infrastructure):
- **D: Automatic Semantic Versioning** — compiler tool, ships immediately
- **F: Zero-Code Observability** — standard library handlers
- **B: Taint Tracking** — design patterns on effect system

**Phase 2** (requires test framework):
- **A: Deterministic Simulation Testing** — simulation handlers + test framework
- **E: Deterministic Replay Debugging** — shares DST infrastructure

**Phase 3** (requires verification infrastructure from Proposal 7):
- **C: Proof-Carrying Code** — extends gradual verification
- **H: Complexity Bounds** — static analysis over pure functions

**Phase 4** (major language extension):
- **G: Choreographic Programming** — extends session types (Proposal 2)

---

## The Unified Architecture

All 15 proposals (Parts I and II) are applications of the same underlying insight:

```
Blood's Effect System
    ├── What code DOES          → Verification (7), Security (4, 9), Observability (13)
    ├── What code NEEDS         → Capabilities (4), Resources (1, 15)
    ├── What code COMMUNICATES  → Session Types (2), Choreography (14)
    └── What COULD GO WRONG     → DST (8), Fault Injection, Replay (12)

Blood's Content-Addressing
    ├── Code IDENTITY           → Proof-Carrying (10), Semver (11), Memoization (3)
    ├── Code PROVENANCE         → Taint Tracking (9), Provenance (6)
    └── Code VERSIONING         → Live Migration, Trace Correlation (13)

Blood's Generational Memory
    ├── Object ORDERING         → Deterministic Allocation (8, 12)
    ├── Object VALIDITY         → Resource Lifecycle, Stale Detection
    └── Object GENERATION       → Version Coexistence, Cache Eviction

Blood's Multiple Dispatch
    ├── Type SPECIALIZATION     → Simulation Strategies (8), Gradient Kernels
    ├── Backend SELECTION       → Solver Portfolio, Hardware Targeting
    └── Protocol VERSIONING     → Migration Handlers, ABI Evolution
```

These are not 15 independent features. They are 15 facets of a language architecture that was designed, perhaps unknowingly, to be a universal substrate for safe, verifiable, observable systems programming.

---

## References (Collected)

### Deterministic Simulation Testing
- [FoundationDB Simulation Testing](https://apple.github.io/foundationdb/testing.html)
- [Antithesis: DST](https://antithesis.com/resources/deterministic_simulation_testing/)
- [DST Primer](https://www.amplifypartners.com/blog-posts/a-dst-primer-for-unit-test-maxxers)

### Information Flow Control
- [Taint Analysis Guide](https://can-ozkan.medium.com/what-is-taint-analysis-a-guide-for-developers-and-security-researchers-11f2ad876ea3)
- [Information Flow Control (Chalmers)](https://www.cse.chalmers.se/~andrei/mod11.pdf)

### Proof-Carrying Code
- [Proof-Carrying Code (Necula, 1997)](https://www.cs.cmu.edu/~necula/Papers/pcc97.pdf)
- [ICFP 2025 Keynote: Proof-Carrying Neuro-Symbolic Code](https://icfp25.sigplan.org/details/icfp-2025-icfp-keynotes/2/Proof-Carrying-Neuro-Symbolic-Code)
- [PC3: Proof-Carrying Code Completions](https://web.cs.ucdavis.edu/~cdstanford/doc/2024/ASEW24b.pdf)

### Semantic Versioning
- [semver.org](https://semver.org/)
- [Static Detection of SemVer Violations](https://arxiv.org/abs/2209.00393)
- [Putting Semantics into Semantic Versioning](https://dl.acm.org/doi/10.1145/3426428.3426922)

### Time-Travel Debugging
- [Deterministic Record-and-Replay (CACM)](https://cacm.acm.org/practice/deterministic-record-and-replay/)
- [rr: Record and Replay](https://rr-project.org/)

### Observability
- [Domain-Oriented Observability (Fowler)](https://martinfowler.com/articles/domain-oriented-observability.html)
- [Effect Library: Tracing](https://effect.website/docs/observability/tracing/)

### Choreographic Programming
- [PLDI 2025: Census-Polymorphic Choreographic Programming](https://pldi25.sigplan.org/details/pldi-2025-papers/47/Efficient-Portable-Census-Polymorphic-Choreographic-Programming)
- [Chorex: Restartable Choreographies](https://programming-journal.org/2025/10/20/)
- [HasChor](https://dl.acm.org/doi/10.1145/3607849)

### Complexity Analysis
- [Accidentally Quadratic](https://accidentallyquadratic.tumblr.com/)
- [Cloudflare Regex Outage](https://blog.cloudflare.com/details-of-the-cloudflare-outage-on-july-2-2019/)

### PL Research (General)
- [POPL 2025 Research Papers](https://popl25.sigplan.org/track/POPL-2025-popl-research-papers)
- [ICFP 2025 Papers](https://icfp25.sigplan.org/track/icfp-2025-papers)
- [PLDI 2025](https://pldi25.sigplan.org/)
- [OOPSLA 2025](https://2025.splashcon.org/)
- [Kleppmann: AI Will Make Formal Verification Mainstream](https://martin.kleppmann.com/2025/12/08/ai-formal-verification.html)
