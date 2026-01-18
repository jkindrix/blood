# Blood: Features That Could Make It Extraordinary

**Status:** Research & Ideation
**Date:** 2026-01-17

---

## Executive Summary

This document proposes features that would make Blood not just "another systems language" but something that **solves problems nobody else has solved well**. Each proposal leverages Blood's unique combination of:

- Algebraic effects (typed side effects)
- Content-addressed code (Unison-style hashing)
- Generational memory safety (Vale-style)
- Multiple dispatch (Julia-style)

The key insight: **Blood's existing features are building blocks for capabilities no other language offers.**

---

## Proposal 1: Compile-Time WCET Analysis

### The Problem Nobody Has Solved

Safety-critical systems (avionics, medical devices, automotive) require **Worst-Case Execution Time (WCET)** guarantees. Currently:

- Engineers use external tools (aiT, RapiTime) after writing code
- No language provides WCET as a first-class concept
- Analysis is divorced from development, discovered late

### Blood's Opportunity

**Blood's effect system makes control flow explicit.** If effects track all sources of timing variability (loops, branches, I/O), the compiler can compute WCET bounds.

### Proposed Design

```blood
// Declare timing budget as an effect constraint
fn control_loop(sensors: &[f64]) -> ControlOutput
    / {Emit<Command>}
    @ wcet(100µs)  // Must complete in 100 microseconds
{
    // Compiler analyzes all paths, verifies WCET
    let reading = process_sensors(sensors);
    let output = compute_pid(reading);
    perform Emit.emit(Command::Set(output));
    output
}

// Loops must have bounded iterations for WCET
fn process_sensors(data: &[f64]) -> f64
    @ wcet_per_element(50ns)  // 50ns per array element
{
    let mut sum: f64 = 0.0;
    // Compiler knows array length, computes total WCET
    for x in data {
        sum = sum + x;
    }
    sum / (data.len() as f64)
}
```

### Compiler Analysis

```blood
// WCET report generated at compile time
$ blood build --wcet-report

WCET Analysis Report
====================
control_loop:
  Base: 45µs
  Loop (process_sensors, max 64 elements): 3.2µs
  compute_pid: 12µs
  Effect dispatch: 2µs
  Total WCET: 62.2µs ✓ (under 100µs budget)

process_data_unbounded:
  ERROR: Loop has no upper bound, WCET cannot be computed
  Suggestion: Add #[max_iterations(N)] or use bounded iterator
```

### Why Blood Is Uniquely Positioned

1. **Effects make I/O explicit** — No hidden syscalls or allocations
2. **No GC** — No unpredictable pause times
3. **Generational checks are O(1)** — Predictable overhead
4. **Content-addressed code** — Function bodies are immutable, timing is stable

### Impact

Blood would be the **first language with built-in WCET analysis**, directly addressing DO-178C, ISO 26262, and IEC 62304 certification requirements.

**References:**
- [WCET Problem Overview](https://www.cs.fsu.edu/~whalley/papers/tecs07.pdf)
- [AbsInt aiT](https://www.absint.com/aiT_WCET.pdf)
- [DO-178C Requirements](https://ldra.com/capabilities/wcet/)

---

## Proposal 2: Session Types for Protocol Safety

### The Problem

Distributed systems have **communication protocols** that can fail in subtle ways:
- Messages sent in wrong order
- Deadlocks from circular waits
- Type mismatches between sender and receiver

[Session types](https://simonjf.com/2016/05/28/session-type-implementations.html) solve this, but existing implementations are:
- Academic (Scribble, Links)
- Bolted onto languages (Rust session-types crate)
- Not integrated with the type system

### Blood's Opportunity

**Effects ARE session types.** An effect like `Emit<T>` is already "I will send T." We can extend this to full protocols.

### Proposed Design

```blood
// Define a protocol as a session type
protocol LoginSession {
    Client -> Server: Credentials,
    Server -> Client: AuthResult,
    match AuthResult {
        Ok(token) => {
            Client -> Server: Request(token),
            Server -> Client: Response,
            // ... continue session
        },
        Err(e) => {
            // Session ends
        }
    }
}

// Server implementation - compiler verifies protocol compliance
fn handle_login(conn: Channel<LoginSession, Server>) / {IO} {
    // Type system ensures we receive Credentials first
    let creds: Credentials = conn.receive();

    let result = authenticate(creds);
    conn.send(result);  // Must send AuthResult

    match result {
        Ok(token) => {
            // Now we can receive Request
            let req: Request = conn.receive();
            let resp = process_request(req, token);
            conn.send(resp);
        },
        Err(_) => {
            // Session ends - trying to receive here is a compile error
        }
    }
}

// Client implementation - must follow same protocol
fn login_client(conn: Channel<LoginSession, Client>) / {IO} {
    conn.send(Credentials { user: "alice", pass: "secret" });

    match conn.receive() {
        Ok(token) => {
            conn.send(Request::GetProfile(token));
            let response: Response = conn.receive();
            // ...
        },
        Err(e) => {
            // Handle auth failure
        }
    }
}
```

### Compile-Time Guarantees

```
$ blood check protocol_example.blood

Protocol Analysis: LoginSession
===============================
✓ Server implementation follows protocol
✓ Client implementation follows protocol
✓ No deadlock possible (protocol is well-formed)
✓ All message types match between parties
```

### Why Blood Is Uniquely Positioned

1. **Effects already model "what this function does"** — Sessions are just structured effects
2. **Algebraic handlers** — Protocol handlers can transform sessions
3. **Content-addressed code** — Protocol definitions can be shared/versioned by hash

### Impact

Blood would offer **compile-time verified distributed protocols**, eliminating entire classes of distributed systems bugs.

**References:**
- [Multiparty Session Types](http://mrg.doc.ic.ac.uk/publications/a-very-gentle-introduction-to-multiparty-session-types/main.pdf)
- [Session Types in Rust](https://link.springer.com/chapter/10.1007/978-3-030-50029-0_8)
- [Scribble Protocol Language](http://mrg.doc.ic.ac.uk/talks/2014/02/SMC/slides.pdf)

---

## Proposal 3: Automatic Memoization via Content-Addressing

### The Problem

[Incremental computation](https://en.wikipedia.org/wiki/Incremental_computing) (reusing previous results when inputs haven't changed) is powerful but hard:
- Manually tracking dependencies is error-prone
- Existing systems (Adapton, Salsa) require explicit annotations
- No language makes this automatic

### Blood's Opportunity

**Blood already hashes code.** Extend this to hash **computation results** keyed by (function_hash, input_hash).

### Proposed Design

```blood
// Mark function as memoizable - results cached by input hash
#[memoize]
fn expensive_computation(data: &[f64]) -> f64 {
    // Complex analysis...
    data.iter().map(|x| x.sin().cos().tan()).sum()
}

// Automatic invalidation when dependencies change
#[memoize]
fn derived_value(config: Config, data: &[f64]) -> Report {
    let base = expensive_computation(data);  // Cached if data unchanged
    let adjusted = apply_config(config, base);
    generate_report(adjusted)
}

// Usage - second call is instant if inputs unchanged
fn main() {
    let data = load_data();

    let report1 = derived_value(config_a, &data);  // Computed
    let report2 = derived_value(config_a, &data);  // Cache hit!
    let report3 = derived_value(config_b, &data);  // Recomputes only apply_config + generate_report
}
```

### The Magic: Automatic Dependency Tracking

```blood
// Blood's content-addressing enables this:
//
// expensive_computation has hash: #a1b2c3d4
// data has hash: #e5f6g7h8
// Result cached at key: (#a1b2c3d4, #e5f6g7h8) -> result
//
// If function code changes, hash changes, cache invalidates
// If input changes, hash changes, cache invalidates
// No manual invalidation needed!
```

### Distributed Memoization

```blood
// Because hashes are deterministic, caching can be distributed
#[memoize(distributed)]
fn train_model(dataset: Dataset, params: HyperParams) -> Model {
    // If any node has computed this exact (function, inputs) combo,
    // fetch result from distributed cache
}
```

### Why Blood Is Uniquely Positioned

1. **Content-addressed code** — Function identity is its hash
2. **Immutable by default** — Inputs don't change under you
3. **Effects are explicit** — Only pure functions can be memoized
4. **Deterministic hashing** — Same inputs = same hash everywhere

### Impact

Blood would offer **automatic, correct, distributed memoization** — what [Adapton](http://adapton.org/) and [Salsa](https://github.com/salsa-rs/salsa) do, but built into the language.

**References:**
- [Adapton: Incremental Computation](https://dl.acm.org/doi/10.1145/2594291.2594324)
- [Self-Adjusting Computation](https://www.cs.cmu.edu/~rwh/students/acar.pdf)

---

## Proposal 4: Capability-Based Security via Effects

### The Problem

Traditional security models (ACLs, permissions) are:
- Checked at boundaries, not throughout code
- Easy to accidentally leak capabilities
- Not verified at compile time

[Object-capability security](https://en.wikipedia.org/wiki/Object-capability_model) solves this, but requires discipline.

### Blood's Opportunity

**Effects ARE capabilities.** `/ {FileSystem}` means "this function has filesystem access." We can make this a security model.

### Proposed Design

```blood
// Define capabilities as effects
effect FileSystem {
    op read(path: Path) -> Result<Bytes, IoError>;
    op write(path: Path, data: Bytes) -> Result<(), IoError>;
}

effect Network {
    op connect(addr: Address) -> Result<Socket, NetError>;
    op send(socket: Socket, data: Bytes) -> Result<(), NetError>;
}

// Function declares exactly what capabilities it needs
fn process_config(path: Path) -> Config / {FileSystem} {
    let data = perform FileSystem.read(path)?;
    parse_config(data)
}

// This function can't access network - it's not in the effect signature
fn pure_computation(data: &[u8]) -> u64 / pure {
    // Any attempt to perform Network.connect here is a compile error
    hash(data)
}

// Capability attenuation - restrict what's passed to untrusted code
fn run_plugin(plugin: fn() / {FileSystem}) / {FileSystem} {
    // Create restricted filesystem capability
    let restricted_fs = attenuate FileSystem {
        read(path) => {
            if path.starts_with("/plugin_data/") {
                perform FileSystem.read(path)
            } else {
                Err(IoError::PermissionDenied)
            }
        },
        write(path) => Err(IoError::PermissionDenied),  // No writes allowed
    };

    with restricted_fs handle {
        plugin()  // Plugin can only read from /plugin_data/
    }
}
```

### Least Privilege by Default

```blood
// main() is the only place capabilities are granted
fn main() / {FileSystem, Network, Time} {
    // All effects available here

    // But we can restrict what we pass down
    let config = process_config(Path::new("/etc/app.conf"));  // Only FileSystem

    let result = pure_computation(&config.data);  // No effects needed

    send_result(result);  // FileSystem + Network
}

// Compiler enforces: you can't use capabilities you weren't given
fn sneaky_function() / pure {
    perform Network.connect(...);  // COMPILE ERROR: Network not in scope
}
```

### Why Blood Is Uniquely Positioned

1. **Effects already track capabilities** — Just needs to be formalized
2. **Handlers enable attenuation** — Restrict capabilities when passing to untrusted code
3. **Compile-time verification** — No capability can be used without being declared
4. **No ambient authority** — Unlike most languages, Blood has no global I/O

### Impact

Blood would offer **compile-time capability security** — what [Pony](https://tutorial.ponylang.io/object-capabilities/object-capabilities.html) and [E](https://en.wikipedia.org/wiki/E_(programming_language)) do, but with Blood's effect system making it natural.

**References:**
- [Object-Capability Model](https://en.wikipedia.org/wiki/Object-capability_model)
- [Pony Capabilities](https://tutorial.ponylang.io/object-capabilities/object-capabilities.html)
- [Principle of Least Authority](https://en.wikipedia.org/wiki/Principle_of_least_privilege)

---

## Proposal 5: Effect-Guided Automatic Parallelization

### The Problem

[Automatic parallelization](https://www.worldscientific.com/doi/abs/10.1142/S0129626412500107) has been researched for decades but rarely works in practice because:
- Compilers can't prove code is side-effect free
- Aliasing makes analysis hard
- Manual annotations are tedious

### Blood's Opportunity

**Blood's effects declare all side effects.** A function marked `/ pure` has no side effects. The compiler can parallelize it automatically.

### Proposed Design

```blood
// Pure function - compiler knows it's safe to parallelize
fn process_element(x: f64) -> f64 / pure {
    x.sin() * x.cos() + x.tan()
}

// Compiler automatically parallelizes this map
fn process_all(data: &[f64]) -> Vec<f64> / pure {
    data.map(process_element)  // Auto-parallelized!
}

// Effect annotation enables safe parallel regions
#[parallel]  // Compiler verifies all operations in block are parallelizable
fn matrix_multiply(a: &Matrix, b: &Matrix) -> Matrix / pure {
    let mut result = Matrix::zeros(a.rows, b.cols);

    // Compiler can parallelize because:
    // 1. Each cell computation is independent
    // 2. No shared mutable state
    // 3. All functions called are pure
    for i in 0..a.rows {
        for j in 0..b.cols {
            result[i][j] = dot_product(a.row(i), b.col(j));
        }
    }

    result
}

// Explicit parallel constructs for when you want control
fn parallel_map<T, U>(data: &[T], f: fn(T) -> U / pure) -> Vec<U> / {Parallel} {
    // Uses work-stealing scheduler
    perform Parallel.map(data, f)
}
```

### The Compiler's Analysis

```
$ blood build --parallel-report

Parallelization Report
======================
process_all: Auto-parallelized (pure function over array)
  Strategy: Work-stealing with 8 chunks
  Speedup estimate: 6.2x on 8 cores

matrix_multiply: Auto-parallelized (independent iterations)
  Strategy: Loop tiling, 32x32 blocks
  Speedup estimate: 7.1x on 8 cores

process_with_io: Cannot parallelize
  Reason: Effect {IO} in loop body
  Suggestion: Extract pure computation, parallelize that
```

### Why Blood Is Uniquely Positioned

1. **Effects prove purity** — No hidden side effects to worry about
2. **Generational refs prevent aliasing issues** — Compiler knows what mutates what
3. **Multiple dispatch** — Parallel algorithms can specialize by type
4. **LLVM backend** — Can leverage polyhedral optimization (Polly)

### Impact

Blood would offer **automatic parallelization that actually works** because the effect system provides the information compilers have always needed.

**References:**
- [Polygeist: Polyhedral MLIR](https://dl.acm.org/doi/10.1109/PACT52795.2021.00011)
- [Polly: Polyhedral Optimization](https://www.worldscientific.com/doi/abs/10.1142/S0129626412500107)
- [MLIR Affine Dialect](https://www.stephendiehl.com/posts/mlir_affine/)

---

## Proposal 6: Provenance Tracking for Compliance

### The Problem

Regulated industries (healthcare, finance, government) require **data provenance** — knowing where data came from and how it was transformed. Currently:
- Tracked manually or with external tools
- Easy to lose provenance through transformations
- No language-level support

### Blood's Opportunity

**Content-addressed code + effects = automatic provenance.** Every transformation is tracked by its code hash and input hashes.

### Proposed Design

```blood
// Enable provenance tracking for a type
#[provenance]
struct PatientRecord {
    id: PatientId,
    data: HealthData,
}

// Provenance is automatically tracked through transformations
fn anonymize(record: PatientRecord) -> AnonymizedRecord / {Provenance} {
    // Provenance automatically records:
    // - Input: PatientRecord with hash #abc123
    // - Transform: anonymize function with hash #def456
    // - Output: AnonymizedRecord with hash #ghi789
    AnonymizedRecord {
        id: hash(record.id),  // Pseudonymized
        data: record.data,
    }
}

fn aggregate(records: Vec<AnonymizedRecord>) -> Statistics / {Provenance} {
    // Provenance records all inputs that contributed to output
    compute_statistics(records)
}

// Query provenance at runtime
fn audit_report(stats: &Statistics) -> ProvenanceReport / {Provenance} {
    perform Provenance.trace(stats)
    // Returns:
    // - All source PatientRecords (by hash, not content)
    // - All transformations applied
    // - Timestamps and locations
}
```

### Compliance Integration

```blood
// GDPR: Right to be forgotten
fn delete_patient(patient_id: PatientId) / {Provenance, Storage} {
    // Find all derived data
    let derived = perform Provenance.forward_trace(patient_id);

    // Delete or re-derive without this patient
    for item in derived {
        perform Storage.delete(item);
    }
}

// HIPAA: Audit trail
fn generate_hipaa_audit() -> AuditLog / {Provenance} {
    perform Provenance.full_log()
    // Every access, transformation, and derivation logged
}
```

### Why Blood Is Uniquely Positioned

1. **Content-addressed code** — Transformations identified by hash
2. **Effects make data flow explicit** — No hidden data movement
3. **Immutable by default** — Provenance chain is append-only
4. **Safety-critical focus** — Target users need compliance

### Impact

Blood would offer **automatic, language-level provenance tracking** — critical for GDPR, HIPAA, and financial regulations.

**References:**
- [Data Lineage](https://en.wikipedia.org/wiki/Data_lineage)
- [GDPR Compliance](https://gdpr.eu/)

---

## Proposal 7: Gradual Verification (Lightweight Proofs)

### The Problem

[Formal verification](https://dafny.org/) tools (Dafny, Lean, F*) are powerful but:
- Require learning a new language
- All-or-nothing approach
- Separate from development workflow

### Blood's Opportunity

**Add lightweight contracts that scale to full proofs.** Start with runtime checks, graduate to compile-time verification.

### Proposed Design

```blood
// Level 1: Runtime contracts (always works)
fn binary_search(arr: &[i32], target: i32) -> Option<usize>
    requires arr.is_sorted()           // Checked at runtime
    ensures result.is_none() || arr[result.unwrap()] == target
{
    // Implementation
}

// Level 2: Compile-time verification (when provable)
#[verify]
fn binary_search_verified(arr: &[i32], target: i32) -> Option<usize>
    requires arr.is_sorted()
    ensures result.is_none() || arr[result.unwrap()] == target
    // Compiler proves these statically
{
    // Implementation
    // Compiler: "I can prove this meets the contract"
}

// Level 3: Full proof (for critical code)
#[prove]
fn binary_search_proven(arr: &[i32], target: i32) -> Option<usize>
    requires arr.is_sorted()
    ensures result.is_none() || arr[result.unwrap()] == target
    decreases arr.len()  // Termination proof
{
    if arr.is_empty() {
        return None;
    }
    let mid = arr.len() / 2;
    if arr[mid] == target {
        Some(mid)
    } else if arr[mid] < target {
        // Compiler verifies: arr[mid+1..].len() < arr.len()
        binary_search_proven(&arr[mid+1..], target).map(|i| i + mid + 1)
    } else {
        // Compiler verifies: arr[..mid].len() < arr.len()
        binary_search_proven(&arr[..mid], target)
    }
}
```

### Gradual Migration Path

```
Runtime contracts (easy)
         ↓
Compile-time verification (some proofs automatic)
         ↓
Full proofs (all properties verified)
```

### Integration with Effects

```blood
// Effects can have contracts too
effect Storage<T> {
    op read(key: Key) -> Option<T>
        ensures result.is_some() implies was_written(key);

    op write(key: Key, value: T)
        ensures will_read(key) == Some(value);
}
```

### Why Blood Is Uniquely Positioned

1. **Effects make state explicit** — Easier to reason about
2. **Content-addressed code** — Proofs are cached by code hash
3. **Safety-critical focus** — Users need formal verification
4. **LLVM backend** — Can integrate with SMT solvers

### Impact

Blood would offer **gradual verification** — the first language where you can start with tests, add contracts, and graduate to proofs without changing languages.

**References:**
- [Dafny](https://dafny.org/)
- [Lean](https://lean-lang.org/)
- [AWS Lean Verification](https://aws.amazon.com/blogs/opensource/lean-into-verified-software-development/)

---

## Summary: The Extraordinary Blood

| Feature | Problem Solved | Unique Enabler |
|---------|---------------|----------------|
| **WCET Analysis** | Real-time timing guarantees | Effects make control flow explicit |
| **Session Types** | Protocol correctness | Effects model communication |
| **Auto-Memoization** | Incremental computation | Content-addressed code |
| **Capability Security** | Least privilege | Effects ARE capabilities |
| **Auto-Parallelization** | Safe concurrency | Effects prove purity |
| **Provenance Tracking** | Compliance (GDPR, HIPAA) | Content-addressing + effects |
| **Gradual Verification** | Lightweight formal proofs | Effects simplify reasoning |

### The Unified Vision

All these features share a common insight:

> **Blood's effect system + content-addressing enables capabilities that require external tools in other languages.**

Other languages need:
- External WCET analyzers
- Session type libraries
- Memoization frameworks
- Capability wrappers
- Parallelization hints
- Provenance databases
- Separate proof assistants

Blood can have all of this **built in** because the foundations already exist.

### Recommended Priority

1. **WCET Analysis** — Directly serves safety-critical market, no competition
2. **Gradual Verification** — Dafny/Lean interest is growing, Blood can capture it
3. **Auto-Parallelization** — Effects make this tractable, huge value
4. **Capability Security** — Natural extension of effect system
5. **Session Types** — Important for distributed systems
6. **Auto-Memoization** — Unique to content-addressed languages
7. **Provenance Tracking** — Growing regulatory pressure

### The Tagline

> **Blood: The language where safety features are performance features.**
>
> - WCET analysis because effects make timing predictable
> - Parallelization because effects prove purity
> - Verification because effects simplify proofs
> - Security because effects are capabilities

---

## Next Steps

1. **Prototype WCET analysis** — Extend effect system with timing annotations
2. **Design session type syntax** — Build on existing effect handlers
3. **Implement `#[memoize]`** — Leverage content-addressing infrastructure
4. **Document capability model** — Show how effects provide security
5. **Evaluate Polly integration** — Test automatic parallelization with effect info

---

## References

### WCET & Real-Time
- [WCET Survey](https://www.cs.fsu.edu/~whalley/papers/tecs07.pdf)
- [LDRA WCET](https://ldra.com/capabilities/wcet/)

### Session Types
- [MPST Introduction](http://mrg.doc.ic.ac.uk/publications/a-very-gentle-introduction-to-multiparty-session-types/main.pdf)
- [Session Types in Rust](https://link.springer.com/chapter/10.1007/978-3-030-50029-0_8)

### Incremental Computation
- [Adapton](http://adapton.org/)
- [Self-Adjusting Computation](https://www.cs.cmu.edu/~rwh/students/acar.pdf)

### Capability Security
- [Pony Capabilities](https://tutorial.ponylang.io/object-capabilities/object-capabilities.html)
- [Object-Capability Model](https://en.wikipedia.org/wiki/Object-capability_model)

### Automatic Parallelization
- [Polygeist](https://dl.acm.org/doi/10.1109/PACT52795.2021.00011)
- [MLIR Polyhedral](https://mlir.llvm.org/docs/Rationale/RationaleSimplifiedPolyhedralForm/)

### Formal Verification
- [Dafny](https://dafny.org/)
- [Lean](https://lean-lang.org/)
- [AWS + Lean](https://aws.amazon.com/blogs/opensource/lean-into-verified-software-development/)

### Provenance
- [Data Lineage](https://en.wikipedia.org/wiki/Data_lineage)
