# Blood Programming Language Specification

**Version**: 0.3.0
**Status**: Implementation Complete, Validation In Progress
**Last Updated**: 2026-02-28

---

## Table of Contents

1. [Introduction](#1-introduction)
2. [Design Philosophy](#2-design-philosophy)
3. [Type System](#3-type-system)
4. [Effect System](#4-effect-system)
5. [Memory Model](#5-memory-model)
6. [Concurrency Model](#6-concurrency-model)
7. [Content-Addressed Storage](#7-content-addressed-storage)
8. [Foreign Function Interface](#8-foreign-function-interface)
9. [Standard Library](#9-standard-library)
10. [Formal Semantics](#10-formal-semantics)
11. [Performance Characteristics](#11-performance-characteristics)

---

## 1. Introduction

### 1.1 What is Blood?

Blood is a systems programming language designed for environments where failure is not an option. The name references the engineering principle of regulations "written in blood" — rules born from catastrophic failures that can never be allowed to happen again.

Blood synthesizes five cutting-edge programming language innovations into a unified system:

| Innovation | Source | Blood Implementation |
|------------|--------|---------------------|
| Content-Addressed Code | Unison | BLAKE3-256 AST hashing |
| Generational Memory Safety | Vale | 128-bit generational pointers |
| Hybrid Ownership Model | Hylo + Rust + Vale | Move semantics with escape-analyzed allocation and explicit borrowing |
| Algebraic Effects | Koka | Row-polymorphic effect system |
| Multiple Dispatch | Julia | Type-stable open dispatch |

### 1.2 Design Goals

1. **Memory safety without garbage collection** — Deterministic resource management
2. **Effect tracking in types** — All side effects visible in function signatures
3. **Zero-cost abstractions when provable** — Compile-time proofs eliminate runtime checks
4. **Predictable performance** — No hidden allocations or implicit dispatch
5. **Seamless C interoperability** — First-class FFI with content-addressed bridges

> **Performance Targets**: Performance figures (e.g., "~1-2 cycles per dereference") are derived from Vale's generational reference implementation and Julia's dispatch system. These represent expected performance based on validated approaches in production systems. See [§11. Performance Characteristics](#11-performance-characteristics) for Blood-specific measurements and validation status.

### 1.3 Non-Goals

- Gradual typing or dynamic escape hatches
- Implicit memory management (GC)
- Maximum expressiveness at the cost of safety
- Backward compatibility with existing languages

### 1.4 Related Specifications

This document provides the high-level language design. Detailed specifications are in companion documents:

| Topic | Document | Description |
|-------|----------|-------------|
| Memory | [MEMORY_MODEL.md](./MEMORY_MODEL.md) | Synthetic Safety Model, generational references |
| Dispatch | [DISPATCH.md](./DISPATCH.md) | Multiple dispatch algorithm, type stability |
| Storage | [CONTENT_ADDRESSED.md](./CONTENT_ADDRESSED.md) | AST canonicalization, VFT specification |
| Concurrency | [CONCURRENCY.md](./CONCURRENCY.md) | Fiber semantics, scheduler design |
| FFI | [FFI.md](./FFI.md) | Foreign function interface, bridge dialect |
| Syntax | [GRAMMAR.md](./GRAMMAR.md) | Complete grammar with precedence |
| Macros | [MACROS.md](./MACROS.md) | Macro system design, fragment kinds, hygiene |
| Semantics | [FORMAL_SEMANTICS.md](./FORMAL_SEMANTICS.md) | Operational semantics, typing rules |
| Library | [STDLIB.md](./STDLIB.md) | Standard library effects and types |
| Diagnostics | [DIAGNOSTICS.md](./DIAGNOSTICS.md) | Error messages, warning system |
| Roadmap | [../planning/ROADMAP.md](../planning/ROADMAP.md) | Implementation plan, milestones |

---

## 2. Design Philosophy

### 2.1 The Blood Manifesto

Blood is a programming language for systems where failure is not an option. Not "failure is expensive" — failure is **unacceptable**. Avionics. Medical devices. Financial infrastructure. Autonomous vehicles. Nuclear control systems.

The name is a commitment: **we will not ship unsafe abstractions**.

### 2.2 The Five Pillars

| Pillar | Tension Resolved | Mechanism |
|--------|------------------|-----------|
| **Veracity** | Safety vs. Performance | Generational References + MVS |
| **Identity** | Mutability vs. Reproducibility | Content-Addressed AST |
| **Composability** | Power vs. Simplicity | Algebraic Effects |
| **Extensibility** | Openness vs. Coherence | Multiple Dispatch |
| **Isolation** | Concurrency vs. Correctness | Linear Types + Regions |

### 2.3 Core Design Principles

#### Principle 1: No Hidden Costs

Every abstraction in Blood has a predictable, understandable cost model. If something is expensive, it must be syntactically visible. No implicit allocations. No invisible copies. No surprise dispatch.

```blood
// Explicit heap allocation
let x = @heap expensive_operation()

// Explicit effect
let data = perform IO.read(file)
```

#### Principle 2: Failure is Data, Not Control Flow

Exceptions are invisible `goto`. Blood treats all failure modes as values in the type system, managed through algebraic effects. You cannot ignore a failure — the type system won't let you.

```blood
// Effect signature makes failure explicit
fn read_file(path: Path) -> Bytes / {IO, Error<FileError>}
```

#### Principle 3: The Compiler is Your Ally

Blood's safety model should feel like collaboration, not combat. When the compiler rejects code, it must explain *why* and suggest *how* to fix it.

#### Principle 4: Simplicity is Not Simplistic

Blood should be simpler than Rust to learn, but not by hiding complexity — by **eliminating unnecessary complexity**. Move semantics with escape-analyzed allocation are simpler to reason about than borrow checking while providing equivalent safety via generational references.

#### Principle 5: Interop is First-Class

A language that cannot call C is a toy. Blood's FFI is designed from day one, not bolted on.

#### Principle 6: Effects are Universal

IO, state, exceptions, concurrency, non-determinism, logging — these are all effects. Blood provides one unified mechanism (algebraic effect handlers) rather than five incompatible ones.

#### Principle 7: Zero-Cost When Provable, Checked When Not

Blood's safety model is tiered:
- **Compile-time proof**: Zero runtime cost
- **Runtime check**: Minimal cost (generation checks)
- **Persistent fallback**: Rare (reference counting for long-lived objects)

### 2.4 Hierarchy of Concerns

When design decisions conflict, Blood resolves them in this order:

1. **Correctness** — Incorrect fast code is worthless
2. **Safety** — Memory safety is non-negotiable
3. **Predictability** — Developers must understand performance characteristics
4. **Performance** — After the above, optimize aggressively
5. **Ergonomics** — Make the right thing easy, the wrong thing hard

---

## 3. Type System

### 3.1 Kind System

Blood's type system is organized into a hierarchy of **Kinds**:

```
Kind ::= Type          -- Values at runtime
       | Effect        -- Computational effects
       | Region        -- Memory regions
       | Lifetime      -- Temporal scope markers
       | Row           -- Extensible record/effect rows
```

Every well-formed Blood program is **kinded** before it is **typed**. Kind errors are caught before type checking begins.

### 3.2 Core Type Grammar

```ebnf
Type ::=
    -- Primitives
    | 'i8' | 'i16' | 'i32' | 'i64' | 'i128'
    | 'u8' | 'u16' | 'u32' | 'u64' | 'u128'
    | 'f32' | 'f64'
    | 'bool' | 'char' | 'unit'
    | 'never'

    -- Composite
    | '[' Type ';' Expr ']'                    -- Fixed array
    | '[' Type ']'                             -- Slice (fat pointer)
    | '(' Type (',' Type)* ')'                 -- Tuple
    | '{' (Ident ':' Type ',')* '}'            -- Record (structural)
    | '{' (Ident ':' Type ',')* '|' RowVar '}' -- Open record (row polymorphic)

    -- References & Pointers
    | '&' Lifetime? Mutability? Type           -- Reference
    | '*' Type                                 -- Raw pointer (unsafe)
    | 'Box' '<' Type '>'                       -- Owned heap (Tier 2/3)

    -- Functions
    | 'fn' '(' Params ')' '->' Type '/' EffectRow

    -- Algebraic Data Types
    | Ident '<' TypeArgs '>'                   -- Named type with parameters

    -- Ownership Modifiers
    | 'linear' Type                            -- Linear wrapper (exactly once)
    | 'affine' Type                            -- Affine wrapper (at most once)

Mutability ::= 'mut' | 'const'

EffectRow ::= '{' (Effect ',')* ('|' RowVar)? '}'
            | 'pure'
```

### 3.3 Ownership Modalities

Blood distinguishes four ownership modalities:

| Modality | Symbol | Use Count | Drop | Clone | Use Case |
|----------|--------|-----------|------|-------|----------|
| **Owned** | (default) | Any | Auto | Explicit | General values |
| **Affine** | `affine T` | 0..1 | Auto | Forbidden | At-most-once resources |
| **Linear** | `linear T` | Exactly 1 | Forbidden | Forbidden | Must-use handles |
| **Borrowed** | `&T` / `&mut T` | Any | N/A | N/A | Temporary access |

#### Typing Rules for Linearity

```
Γ, x: linear T ⊢ e : U    x ∈ FV(e)
─────────────────────────────────────  [Linear-Use]
Γ ⊢ let x: linear T = v in e : U


Γ, x: linear T ⊢ e : U    x ∉ FV(e)
─────────────────────────────────────  [Linear-Unused: ERROR]
⊥  -- Compile error: linear value not consumed
```

#### Linear Types and Effect Handlers

Linear values must be consumed before effect suspension or explicitly transferred:

```blood
// ERROR: linear value live across yield point
fn bad(linear h: Handle) -> unit / {Yield<i32>} {
    let value = h.read()
    yield(value)        // h still live here
    h.close()
}

// CORRECT: consume before suspension
fn good(linear h: Handle) -> unit / {Yield<i32>} {
    let value = h.read()
    h.close()           // Consume h first
    yield(value)        // No linear values across suspension
}
```

### 3.4 Row Polymorphism

#### Record Row Polymorphism

```blood
// Works on ANY record with 'x' and 'y' fields
fn magnitude<R>(point: {x: f64, y: f64 | R}) -> f64 / pure {
    sqrt(point.x * point.x + point.y * point.y)
}

magnitude({x: 1.0, y: 2.0})                    // Exact match
magnitude({x: 1.0, y: 2.0, z: 3.0})            // Extra field OK
magnitude({x: 1.0, y: 2.0, name: "origin"})    // Different extra field
```

#### Effect Row Polymorphism

```blood
// Polymorphic over additional effects
fn map<A, B, E>(list: List<A>, f: fn(A) -> B / E) -> List<B> / E {
    match list {
        Nil => Nil,
        Cons(head, tail) => Cons(f(head), map(tail, f))
    }
}
```

#### Typing Rules

```
Γ ⊢ e : {l₁: T₁, ..., lₙ: Tₙ | ρ}    l ∈ {l₁, ..., lₙ}
────────────────────────────────────────────────────────  [Row-Select]
Γ ⊢ e.l : Tᵢ

Γ ⊢ e : {l₁: T₁, ..., lₙ: Tₙ | ρ}    l ∉ {l₁, ..., lₙ}
────────────────────────────────────────────────────────  [Row-Extend]
Γ ⊢ {l = v | e} : {l: T, l₁: T₁, ..., lₙ: Tₙ | ρ}
```

### 3.5 Full Type Composition

A fully specified Blood type includes four dimensions:

```
FullType = BaseType × Ownership × Region × Effects

Example:
  linear &'heap mut Vec<i32> / {Allocate, Error}
  ^^^^^^ ^^^^^ ^^^ ^^^^^^^^^   ^^^^^^^^^^^^^^^^
    |      |    |      |              |
    |      |    |      |              +-- Effects
    |      |    |      +----------------- Base data type
    |      |    +------------------------ Mutability
    |      +----------------------------- Region/Lifetime
    +------------------------------------ Ownership modality
```

### 3.6 Type Stability and Dispatch

> **Detailed Specification**: See [DISPATCH.md](./DISPATCH.md) for the complete multiple dispatch algorithm, type stability checking, and ambiguity detection.

A function is **type-stable** if for all concrete input types, the return type is fully determined at compile time.

```blood
// STABLE: return type determined by input types
fn add(x: i32, y: i32) -> i32 { x + y }

// UNSTABLE: return type depends on VALUE (compile error)
fn unstable(x: i32) -> ??? {
    if x > 0 { x }           // returns i32
    else { "negative" }      // returns String
}
```

#### Dispatch Resolution Order

1. **Exact match** — All argument types match exactly
2. **Subtype match** — Arguments are subtypes of parameters
3. **Generic match** — Type parameters instantiate to concrete types
4. **Ambiguity** — Multiple methods match equally → **Compile Error**

### 3.7 Subtyping Relations

```
T <: T                                          [Refl]

S <: T    T <: U
─────────────────                               [Trans]
    S <: U

S <: T
─────────────────                               [Ref-Covariant]
&S <: &T

T <: S
─────────────────                               [Ref-Mut-Invariant]
&mut T = &mut T   (no subtyping for mut refs)

E₁ ⊆ E₂
─────────────────────────────────               [Effect-Subtyping]
(A → B / E₁) <: (A → B / E₂)

-- Pure functions usable where effectful ones expected
(A → B / pure) <: (A → B / E)
```

### 3.8 Bidirectional Type Checking

```
check(Γ, e, T)  -- Check that e has type T
infer(Γ, e)     -- Infer the type of e

check(Γ, λx.e, A → B / E) = check(Γ ∪ {x: A}, e, B)

infer(Γ, e₁ e₂) =
    let (A → B / E) = infer(Γ, e₁)
    check(Γ, e₂, A)
    return B / E

infer(Γ, (e : T)) = check(Γ, e, T); return T
```

---

## 4. Effect System

### 4.1 Overview

In Blood, **effects are not exceptions**. They are a structured, typed, bidirectional communication protocol between a computation and its context.

```
┌─────────────────────────────────────────────────────────────┐
│                        HANDLER                               │
│  ┌──────────────────────────────────────────────────────┐   │
│  │                   COMPUTATION                         │   │
│  │                                                       │   │
│  │    perform Read()  ────────────────────►  operation   │   │
│  │         ▲                                    │        │   │
│  │         │                                    ▼        │   │
│  │    resume(value) ◄────────────────────  handler impl  │   │
│  │                                                       │   │
│  └──────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

Handlers can:
- **Resume once** — Normal control flow
- **Resume multiple times** — Non-determinism, backtracking
- **Never resume** — Exceptions, early exit
- **Resume later** — Fibers, coroutines (ADR-036: `Fiber`, not `Async`)

### 4.2 Effect Declaration

```ebnf
EffectDecl ::= 'effect' EffectName TypeParams? '{' Operation* '}'
Operation ::= 'op' OpName '(' Params ')' '->' ReturnType
```

#### Examples

```blood
effect Console {
    op print(msg: String) -> unit
    op read_line() -> String
}

effect State<S> {
    op get() -> S
    op put(new_state: S) -> unit
}

effect Error<E> {
    op raise(err: E) -> never
}

effect Fiber {
    op spawn<T: Send>(task: fn() -> T / {Fiber} + Send) -> FiberHandle<T>
    op yield() -> unit
    op join<T>(handle: FiberHandle<T>) -> T
}
```

### 4.3 Handler Definition

```ebnf
HandlerDecl ::= HandlerKind 'handler' HandlerName
               'for' EffectName TypeArgs?
               '{' HandlerClause* '}'

HandlerKind ::= 'shallow' | 'deep' | ε  -- default is deep
```

#### Deep vs Shallow Handlers

**Deep Handler** (default): Wraps entire computation. Remains in scope after resume.

**Shallow Handler**: Handles exactly one operation, then removed.

```blood
// Deep: handles ALL State operations
deep handler InMemoryState<S> for State<S> {
    let mut current: S

    return(x) { x }

    op get() {
        resume(current)
    }

    op put(new_state) {
        current = new_state
        resume(())
    }
}

// Shallow: handles ONE Yield, returns
shallow handler FirstYield<T> for Yield<T> {
    return(x) { None }

    op yield(value) {
        Some(value)  // Don't resume, return immediately
    }
}
```

### 4.4 Effect Typing Rules

```
─────────────────────────────────  [T-Pure]
Γ ⊢ v : T / pure


Γ ⊢ e₁ : T₁ / ε₁    Γ, x: T₁ ⊢ e₂ : T₂ / ε₂
────────────────────────────────────────────  [T-Let]
Γ ⊢ let x = e₁ in e₂ : T₂ / ε₁ ∪ ε₂


effect E { op o(T₁) -> T₂ } ∈ Σ    E ∈ ε
────────────────────────────────────────────  [T-Perform]
Γ ⊢ perform o(e) : T₂ / ε


Γ ⊢ e : T / {E | ε}    Γ ⊢ h : Handler<E, T, U, ε'>
───────────────────────────────────────────────────  [T-Handle]
Γ ⊢ with h handle e : U / ε ∪ ε'
```

### 4.5 Generation Snapshots

When algebraic effects suspend computation, captured continuations may hold references that become stale. Blood solves this with **Generation Snapshots**.

#### Continuation Representation

```
Continuation = (Code, Environment, GenerationSnapshot)
GenerationSnapshot = Map<Address, Generation>
```

#### Capture Rule

When a continuation is captured, all generational references in scope are recorded with their current generation numbers.

#### Resume Validation

When resumed, each reference is re-validated. If any generation advanced, `StaleReference` effect is raised.

```
Γ_gen = {(a₁,g₁), ..., (aₙ,gₙ)}
∀i. currentGen(aᵢ) = gᵢ
─────────────────────────────────────────  [Resume-Valid]
resume((κ, Γ_gen, ∅), v)  ──►  κ(v)


∃i. currentGen(aᵢ) ≠ gᵢ
─────────────────────────────────────────  [Resume-Stale]
resume((κ, Γ_gen, ∅), v)  ──►  perform stale(aᵢ, gᵢ, currentGen(aᵢ))
```

#### The StaleReference Effect

```blood
effect StaleReference {
    op stale(addr: Address, expected: Gen, actual: Gen) -> never
}

// Default handler: panic
handler PanicOnStale for StaleReference {
    op stale(addr, expected, actual) -> never {
        panic("Use-after-free: {addr} gen {expected} != {actual}")
    }
}

// Safety-critical: graceful degradation
handler GracefulStale for StaleReference {
    op stale(addr, expected, actual) -> never {
        log_error("Memory violation at {addr}")
        abort_fiber()
    }
}
```

### 4.6 Standard Effects

#### Error<E>

```blood
effect Error<E> {
    op raise(err: E) -> never
}

deep handler Propagate<E> for Error<E> {
    return(x) { Ok(x) }
    op raise(err) { Err(err) }
}
```

#### State<S>

```blood
effect State<S> {
    op get() -> S
    op put(s: S) -> unit
}

deep handler LocalState<S> for State<S> {
    let mut state: S

    return(x) { (x, state) }
    op get() { resume(state) }
    op put(s) { state = s; resume(()) }
}
```

#### Yield<T>

```blood
effect Yield<T> {
    op yield(value: T) -> unit
}

deep handler Collect<T> for Yield<T> {
    let mut items: Vec<T> = vec![]

    return(_) { items }
    op yield(value) {
        items.push(value)
        resume(())
    }
}
```

#### Fiber (ADR-036)

```blood
effect Fiber {
    op spawn<T: Send>(task: fn() -> T / {Fiber} + Send) -> FiberHandle<T>
    op spawn_blocking<T: Send>(f: fn() -> T + Send) -> FiberHandle<T>
    op yield() -> unit
    op sleep(duration: Duration) -> unit
    op join<T>(handle: FiberHandle<T>) -> T
}

effect Cancel {
    op check_cancelled() -> unit
}
```

#### IO

```blood
effect IO {
    op read(fd: FileDescriptor, buf: &mut [u8]) -> Result<usize, IoError>
    op write(fd: FileDescriptor, buf: &[u8]) -> Result<usize, IoError>
    op open(path: Path, flags: OpenFlags) -> Result<FileDescriptor, IoError>
    op close(fd: FileDescriptor) -> Result<unit, IoError>
}
```

#### NonDet

```blood
effect NonDet {
    op choose<T>(options: Vec<T>) -> T
    op fail() -> never
}

deep handler AllSolutions for NonDet {
    return(x) { vec![x] }
    op choose(options) {
        options.flat_map(|opt| resume(opt)).collect()
    }
    op fail() { vec![] }
}
```

### 4.7 Handler Linearity

**Theorem**: Multi-shot handlers cannot capture linear values.

> This theorem is validated by recent research: Tang et al. "Soundly Handling Linearity" (POPL 2024) and van Rooij & Krebbers "Affect: An Affine Type and Effect System" (POPL 2025). See [FORMAL_SEMANTICS.md](./FORMAL_SEMANTICS.md#appendix-b-related-work-and-citations) for full citations.

```blood
// ERROR: multi-shot with linear capture
fn bad<T>(linear resource: T) -> T / Choose {
    let choice = choose(vec![1, 2, 3])
    drop(resource)  // Would duplicate across resumes
    choice
}
```

Handler annotations:

```blood
deep handler SingleShot for MyEffect {
    #[single_shot]
    op my_op(x) { resume(x) }
}

deep handler MultiShot for Choose {
    #[multi_shot]
    op choose(opts) { opts.map(|o| resume(o)) }
}
```

### 4.8 Effect Composition

Handlers nest from innermost to outermost:

```blood
with h₁ handle (with h₂ handle e)
```

Effect rows are unordered sets, but handler nesting determines semantics.

Effect subsumption: pure functions usable where effectful expected:

```blood
fn pure_fn(x: i32) -> i32 / pure { x * 2 }

fn effectful_map<E>(list: List<i32>, f: fn(i32) -> i32 / E) -> List<i32> / E

effectful_map(nums, pure_fn)  // Works: pure <: E
```

---

## 5. Memory Model

> **Detailed Specification**: See [MEMORY_MODEL.md](./MEMORY_MODEL.md) for the complete Synthetic Safety Model specification, including formal algorithms, soundness proofs, and implementation notes.

### 5.1 Overview

Blood uses the **Synthetic Safety Model (SSM)**, a hybrid ownership model that combines the simplicity of mutable value semantics with the flexibility of explicit borrowing:

| Aspect | Blood Approach | Source Inspiration |
|--------|----------------|-------------------|
| **Default** | Move semantics (ownership transfer, not copies) | Rust |
| **Explicit** | Borrowing via `&T` and `&mut T` when needed | Rust |
| **Heap** | Generational references for `Box<T>`, collections | Vale |
| **Resources** | Linear/affine types for must-use handles | Linear logic |

> **Terminology Note**: Blood uses a **hybrid ownership model** combining move semantics (Rust-inspired), escape-analyzed allocation (Vale-inspired), and generational references. Unlike Hylo's pure copy-by-default approach, Blood transfers ownership on assignment (move semantics). The escape analysis determines allocation tier (stack vs region vs persistent), not whether values are copied. Blood includes `&T` and `&mut T` for explicit borrowing when needed. See [MEMORY_MODEL.md §1.3](./MEMORY_MODEL.md#13-hybrid-model-clarification) for details.

### 5.2 Tiered Memory Model

Blood uses a three-tier memory model based on escape analysis:

| Tier | Lifecycle | Mechanism | Cost* |
|------|-----------|-----------|-------|
| **Tier 1: Stack** | Lexical | CPU SP manipulation | Zero |
| **Tier 2: Region** | Scoped | Generational Check | ~1-2 cycles/deref |
| **Tier 3: Persistent** | Global | Deferred Ref Counting | Higher |

*Cycle costs derived from Vale's implementation and x86-64 instruction analysis. See [§11](#11-performance-characteristics) for Blood-specific measurements.

### 5.3 The 128-bit Blood Pointer

Every non-stack pointer follows this layout:

| Component | Bits | Purpose |
|-----------|------|---------|
| **Address** | 64 | Virtual memory address |
| **Generation** | 32 | Birth-count for use-after-free detection |
| **Metadata** | 32 | Tier ID (4), Linearity flags (4), Type fingerprint (24) |

#### Generation Overflow

Objects surviving 2³² generations are promoted to Tier 3 (Persistent), managed via deferred reference counting.

#### The Zombified Trap

Mismatched generation check triggers deterministic trap, handled as `StaleReference` effect.

### 5.4 Escape-Analyzed Allocation

Blood uses move semantics by default. The compiler performs inter-procedural escape analysis to determine allocation tier:

1. **Stack Promotion**: Objects that don't escape are stack-allocated
2. **Check Elision**: Generation checks removed for locally-proven references
3. **Copy Elimination**: In-place mutation when safe

```blood
fn process(data: Vec<i32>) -> Vec<i32> {
    // Compiler proves 'data' doesn't escape
    // Allocated on stack, no generation checks
    data.push(42)
    data
}
```

---

## 6. Concurrency Model

### 6.1 Region Isolation

Every concurrent Fiber operates in its own memory region. Data transfer between regions requires explicit ownership transfer.

### 6.2 Linear Ownership Transfer

Mutable data moves between Fibers via linear types:

```blood
fn sender(linear data: Buffer) / {Fiber} {
    let handle = spawn(|| receiver(data))  // Ownership transferred
    // data no longer accessible here
}

fn receiver(linear data: Buffer) / {Fiber} {
    process(data)
    drop(data)
}
```

### 6.3 Immutable Sharing

Data can be shared across Fibers only if "Frozen" (deeply immutable):

```blood
let shared: Frozen<Config> = freeze(config)
spawn(|| use_config(&shared))  // Multiple readers OK
spawn(|| use_config(&shared))
```

### 6.4 Structured Concurrency

All spawned tasks are children of their spawning scope. Parent waits for all children before completing.

```blood
fn parallel_work() / {Fiber} {
    // Both tasks complete before function returns (ADR-036 Sub-1)
    let a = spawn(|| compute_a())
    let b = spawn(|| compute_b())
    combine(join(a), join(b))
}
```

---

## 7. Content-Addressed Storage

### 7.1 The Hash is the Identity

Every function, type, and constant is identified by a BLAKE3-256 hash of its canonicalized AST.

```
Hash = BLAKE3-256(Canonicalize(AST))
```

### 7.2 Semantic Canonicalization

Before hashing, the AST is normalized:
- Whitespace removed
- Comments stripped
- Local variable names normalized (α-conversion)
- Import order standardized

Result: Refactoring (renaming locals, reformatting) preserves hash. Semantic changes alter hash.

### 7.3 Version Coexistence

Modules are collections of hashes. Multiple versions of the same dependency coexist without symbol conflicts:

```blood
// Module A depends on crypto@hash_abc123
// Module B depends on crypto@hash_def456
// Both loaded simultaneously, no conflict
```

### 7.4 Hot-Swap Runtime

The runtime maintains a Virtual Function Table (VFT) indexed by hash:

- **Atomic Updates**: New AST inserted, VFT pointer swapped atomically
- **In-Flight Integrity**: Active calls complete on old hash; new calls route to new hash

---

## 8. Foreign Function Interface

### 8.1 The Bridge Dialect

Since C uses symbol names and Blood uses hashes, an FFI Bridge maps between them:

```blood
@ffi_bridge("libc")
extern {
    // C symbol "malloc" mapped to Blood hash
    fn malloc(size: usize) -> *mut u8

    // C symbol "free" mapped to Blood hash
    fn free(ptr: *mut u8) -> unit
}
```

### 8.2 Safety Boundaries

FFI calls are inherently unsafe. Blood requires explicit annotation:

```blood
fn safe_wrapper(size: usize) -> Result<Box<[u8]>, AllocError> / {Allocate} {
    let ptr = @unsafe { malloc(size) }
    if ptr.is_null() {
        raise(AllocError::OutOfMemory)
    } else {
        Ok(Box::from_raw(ptr, size))
    }
}
```

### 8.3 Identity Shift Detection

When a C library updates, the bridge detects the change and requires re-validation:

```blood
// Bridge records: libc version 2.35 -> hash_xyz
// If libc updates to 2.36, bridge invalidates
// Developer must re-verify FFI safety
```

---

## 9. Standard Library

See [STDLIB.md](./STDLIB.md) for the complete standard library specification.

### 9.1 Core Types

- `Option<T>`, `Result<T, E>`
- `Vec<T>`, `HashMap<K, V>`, `String`
- `Box<T>`, `Rc<T>`, `Arc<T>`

### 9.2 Core Effects

- `IO`, `Error<E>`, `State<S>`
- `Fiber`, `Cancel`, `Yield<T>`, `NonDet`

### 9.3 Core Handlers

- Default handlers for all standard effects
- Mock handlers for testing

---

## 10. Formal Semantics

### 10.1 Operational Semantics

Blood uses small-step operational semantics with evaluation contexts.

#### Evaluation Contexts

```
E ::= □
    | let x = E in e
    | E e
    | v E
    | with h handle E
```

#### Reduction Rules

**Value Return**:
```
with h handle v  ──►  h.return(v)
```

**Operation (Deep)**:
```
with h handle E[perform o(v)]
    ──►  h.o(v, λx. with h handle E[x])
```

**Operation (Shallow)**:
```
with h handle E[perform o(v)]
    ──►  h.o(v, λx. E[x])
```

### 10.2 Type Soundness

*Formal proof to be provided in future revision.*

**Conjecture**: If `Γ ⊢ e : T / ε` and `e ──►* v`, then `v : T`.

### 10.3 Effect Safety

**Theorem**: A well-typed Blood program cannot perform an unhandled effect at runtime.

**Theorem**: Multi-shot handlers cannot capture linear values.

**Theorem**: Generation snapshots ensure use-after-free is detected on continuation resume.

---

## 11. Performance Characteristics

This section documents Blood's expected performance characteristics, distinguishing between validated research and design targets.

### 11.1 Performance Status Legend

| Status | Meaning |
|--------|---------|
| **Validated** | Proven by source language or published research |
| **Theoretical** | Based on analysis, not empirical measurement |
| **Target** | Design goal, requires implementation validation |

### 11.2 Memory Safety Overhead

| Operation | Expected Cost | Status | Source |
|-----------|---------------|--------|--------|
| Generation check (Tier 1) | ~1-2 cycles | Theoretical | Vale design |
| Check elision (Tier 0) | 0 cycles | Validated | Escape analysis research |
| Snapshot capture | O(n) refs | Theoretical | Blood implementation |
| Snapshot validation | O(n) refs | Theoretical | Blood implementation |
| RC increment/decrement | ~10-20 cycles | Validated | Standard RC overhead |

### 11.3 128-bit Pointer Overhead

Blood uses 128-bit fat pointers for heap references. Expected impact:

| Aspect | Impact | Mitigation |
|--------|--------|------------|
| Memory bandwidth | 2x for pointer-heavy code | Use value semantics by default |
| Cache efficiency | Reduced for pointer arrays | Struct-of-arrays patterns |
| SIMD operations | Limited to 2 pointers per 256-bit reg | Design for values, not pointers |
| Atomic operations | Requires CMPXCHG16B (x86-64) | 16-byte alignment |

**Guidance**: Prefer value-oriented designs. Pointer-heavy data structures (linked lists, trees) will have higher overhead than in languages with 64-bit pointers.

### 11.4 Effect System Overhead

| Operation | Expected Cost | Status | Notes |
|-----------|---------------|--------|-------|
| Pure function call | 0 extra cycles | Validated | No effect tracking at runtime |
| Effectful call (optimized) | 0-2 cycles | Theoretical | Evidence passing |
| Continuation capture | O(stack depth) | Theoretical | Depends on handler depth |
| Handler installation | ~10-20 cycles | Theoretical | Similar to exception frame |

**Known Limitation**: Koka, the source of Blood's effect system, acknowledges: "lags when using more imperative array algorithms." Blood may exhibit similar characteristics for tight loops over mutable arrays.

### 11.5 Escape Analysis Effectiveness

Based on Java HotSpot research (validated) and Blood implementation analysis:

| Code Pattern | Stack Allocation Rate | Status |
|--------------|----------------------|--------|
| Local objects, no escape | >95% | Validated (Java) |
| Objects passed to callees | 60-80% | Validated (Java) |
| Objects crossing effect boundaries | Lower | Blood-specific |
| Objects in closures | Varies | Requires analysis |

### 11.6 Compile-Time Considerations

| Analysis | Complexity | Impact |
|----------|------------|--------|
| Escape analysis (local) | O(n) | Fast |
| Escape analysis (inter-procedural) | O(n²) worst case | May require optimization levels |
| Type stability checking | O(methods × types) | Potentially expensive |
| Effect inference | O(n) | Linear in program size |
| Generation snapshot analysis | O(R × H) | See detailed analysis below |

#### 11.6.1 Generation Snapshot Complexity Analysis

**Variables**:
- `R` = number of generational references in scope at effect operation
- `H` = handler nesting depth
- `L` = average number of references per continuation (liveness-filtered)

**Compile-Time Complexity**:

| Operation | Complexity | Notes |
|-----------|------------|-------|
| Liveness analysis | O(n) per function | Standard dataflow, cacheable |
| Snapshot point detection | O(effects) | Each `perform` is a snapshot point |
| Reference set computation | O(R) per snapshot | Set of live refs at that point |
| Total per function | O(n + R × effects) | Dominated by liveness analysis |

**Runtime Complexity**:

| Operation | Complexity | Notes |
|-----------|------------|-------|
| Snapshot capture | O(L) | Copy L (addr, gen) pairs |
| Snapshot validation | O(L) | Check L generations |
| Full resume cost | O(L × H) | Validate at each handler level |

**Practical Bounds** (estimated):

| Metric | Typical | Worst Case | Mitigation |
|--------|---------|------------|------------|
| L (refs per snapshot) | 5-20 | 100+ | Liveness filtering |
| H (handler depth) | 2-5 | 10+ | Flatten handlers |
| Capture overhead | <1μs | 10μs | Batch capture |
| Validation overhead | <500ns | 5μs | Lazy validation |

**Optimization Strategies**:

1. **Liveness filtering**: Only include references that may be dereferenced after resume (reduces L by 50-80% in typical code).

2. **Snapshot sharing**: Nested handlers share prefix of snapshot, avoiding redundant copies.

3. **Lazy validation**: Defer validation to first dereference (amortizes cost, catches errors at use site).

4. **Generation caching**: Hot references have generation cached in registers after first check.

**Complexity Class**: Generation snapshot operations are **O(L)** where L is bounded by the number of live references at the snapshot point. With liveness filtering, L is typically much smaller than the total references in scope.

**Recommendation**: Blood may offer optimization levels trading compile time for runtime performance:
- `-O0`: No escape analysis, all heap allocations use generation checks
- `-O1`: Local escape analysis only
- `-O2`: Inter-procedural escape analysis (default)
- `-O3`: Aggressive analysis with higher compile time

### 11.7 Benchmarking Plan

Before Blood 1.0, the following benchmarks must validate performance claims.

#### 11.7.1 Performance Targets

| Category | Metric | Target | Acceptable | Unacceptable |
|----------|--------|--------|------------|--------------|
| **Generation check** | Cycles per deref | 1-2 | 3-5 | >10 |
| **Snapshot capture** | ns per reference | <100 | <500 | >1000 |
| **Snapshot validation** | ns per reference | <50 | <200 | >500 |
| **Handler installation** | ns per handler | <100 | <500 | >1000 |
| **Handler invocation** | ns per operation | <50 | <200 | >500 |
| **Evidence lookup** | ns per lookup | <10 | <50 | >100 |
| **128-bit pointer overhead** | % vs 64-bit | <10% | <20% | >30% |

#### 11.7.2 Micro-Benchmarks

| Benchmark | Measures | Baseline Comparison |
|-----------|----------|---------------------|
| `gen_check_hot` | Generation check in tight loop | Raw pointer deref |
| `gen_check_cold` | Generation check with cache miss | Memory bandwidth |
| `snapshot_capture_N` | Capture N references | N × single capture |
| `snapshot_validate_N` | Validate N references | N × single validate |
| `handler_install` | Install effect handler | Function call overhead |
| `handler_invoke` | Invoke effect operation | Virtual call |
| `evidence_pass` | Evidence vector lookup | Array index |
| `rc_increment` | Atomic refcount increment | Non-atomic increment |
| `rc_decrement` | Atomic refcount decrement | Non-atomic decrement |
| `tier_promotion` | Tier 1 → Tier 3 promotion | Allocation baseline |

#### 11.7.3 Macro-Benchmarks

| Benchmark | Category | Description |
|-----------|----------|-------------|
| `quicksort` | Compute | In-place sorting, minimal allocation |
| `mergesort` | Allocation | Allocation-heavy sorting |
| `bfs_graph` | Pointer-heavy | Breadth-first search on graph |
| `json_parse` | Mixed | Parse JSON, build AST |
| `http_server` | Effect-heavy | Handle concurrent HTTP requests |
| `fibonacci_effect` | Effect | Recursive with State effect |
| `generator_sum` | Shallow handler | Sum values from generator |
| `async_fanout` | Concurrency | Spawn N concurrent tasks |
| `ring_buffer` | Memory | Circular buffer operations |
| `btree_ops` | Balanced | B-tree insert/lookup/delete |

#### 11.7.4 Comparative Benchmarks

| Benchmark Suite | Languages | Purpose |
|-----------------|-----------|---------|
| **Computer Language Benchmarks Game** | Blood, Rust, Go, C | Industry standard comparison |
| **Are We Fast Yet** | Blood, Java, JavaScript | Dynamic language comparison |
| **Custom Effect Suite** | Blood, Koka, OCaml 5 | Effect system comparison |

**Comparison Targets**:

| vs. Language | Acceptable Overhead | Notes |
|--------------|---------------------|-------|
| **C (unsafe)** | <50% | Safety overhead acceptable |
| **Rust** | <20% | Similar safety model |
| **Go** | <10% or faster | GC baseline |
| **Koka** | Comparable | Effect system baseline |

#### 11.7.5 Benchmark Infrastructure

```
benches/
├── micro/
│   ├── generation.rs       # Generation check benchmarks
│   ├── snapshot.rs         # Snapshot capture/validate
│   ├── effects.rs          # Handler installation/invocation
│   └── memory.rs           # RC, allocation, tiers
│
├── macro/
│   ├── algorithms/         # Sorting, searching, etc.
│   ├── data_structures/    # Trees, graphs, etc.
│   ├── effects/            # Effect-heavy workloads
│   └── concurrent/         # Multi-fiber benchmarks
│
├── comparative/
│   ├── rust/               # Equivalent Rust implementations
│   ├── go/                 # Equivalent Go implementations
│   └── c/                  # Equivalent C implementations
│
└── criterion.toml          # Benchmark configuration
```

#### 11.7.6 Continuous Benchmarking

- **CI Integration**: Benchmarks run on every PR
- **Regression Detection**: >5% slowdown blocks merge
- **Historical Tracking**: Performance graphed over time
- **Platform Matrix**: Linux x86-64, macOS ARM64, Windows x86-64

#### 11.7.7 Milestone Gates

| Milestone | Required Benchmarks | Gate Criteria |
|-----------|---------------------|---------------|
| Milestone 1 | All micro-benchmarks | Within 2x of target |
| Milestone 2 | + Macro-benchmarks | Within 1.5x of target |
| Milestone 3 | + Comparative | Meet comparison targets |
| 1.0 Release | Full suite | All targets met |

### 11.8 Performance Anti-Patterns

Patterns to avoid for optimal performance:

```blood
// ANTI-PATTERN: Pointer-heavy data structure
type LinkedList<T> = Option<Box<Node<T>>>
struct Node<T> { value: T, next: LinkedList<T> }
// Each node requires 128-bit pointer, generation check on traversal

// PREFERRED: Contiguous storage
type List<T> = Vec<T>
// Value semantics, cache-friendly, no generation checks for iteration

// ANTI-PATTERN: Deep effect nesting in hot loop
fn slow_loop() / {State<i32>, Error<E>, Log} {
    for i in 0..1000000 {
        let x = get()        // Effect operation
        if x < 0 { raise(E) } // Another effect
        log("iter")          // Another effect
    }
}

// PREFERRED: Batch effect operations
fn fast_loop() / {State<i32>, Error<E>, Log} {
    let x = get()            // Single state read
    for i in 0..1000000 {
        // Pure computation
    }
    log("complete")          // Single log at end
}
```

---

## Appendix A: Grammar Summary

> **Complete Grammar**: See [GRAMMAR.md](./GRAMMAR.md) for the full surface syntax specification, including lexical rules, operator precedence, and disambiguation rules.

### Quick Reference

```ebnf
Program ::= ModuleDecl? Import* Declaration*

Declaration ::=
    | FnDecl
    | TypeDecl | StructDecl | EnumDecl
    | EffectDecl | HandlerDecl
    | TraitDecl | ImplBlock
    | ConstDecl | StaticDecl

FnDecl ::= Visibility? 'fn' Ident TypeParams? '(' Params ')'
           '->' Type '/' EffectRow WhereClause? Block

EffectDecl ::= 'effect' Ident TypeParams? '{' OperationDecl* '}'

HandlerDecl ::= ('shallow' | 'deep')? 'handler' Ident
                'for' Type '{' HandlerBody '}'

Expr ::= ExprWithBlock | ExprWithoutBlock

ExprWithBlock ::= Block | IfExpr | MatchExpr | LoopExpr
                | ForExpr | WhileExpr | WithHandleExpr

ExprWithoutBlock ::= Literal | Path | Call | Method | Field
                   | Index | Unary | Binary | Closure | Perform | Resume
```

### Operator Precedence (High to Low)

| Prec | Operators | Assoc |
|------|-----------|-------|
| 18 | `::` | Left |
| 17-15 | `.method()` `()` `[]` | Left |
| 14 | `!` `-` `*` `&` | Right |
| 13-11 | `as` `*/%` `+-` | Left |
| 10-7 | `<<>>` `&` `^` `\|` | Left |
| 6 | `== != < > <= >=` | Non |
| 5-4 | `&&` `\|\|` | Left |
| 3 | `..` `..=` | Non |
| 2 | `=` `+=` etc. | Right |
| 1 | `\|>` | Left |

---

## Appendix B: Comparison with Related Languages

| Feature | Rust | Koka | Vale | Unison | **Blood** |
|---------|------|------|------|--------|-----------|
| Memory Safety | Borrow checker | GC | Generational | GC | Generational + MVS |
| Effects | None (libs) | Algebraic | None | Abilities | Algebraic |
| Dispatch | Traits | Functions | Traits | Functions | Multiple |
| Code Identity | Files | Files | Files | Hash | Hash |
| Linearity | Affine | None | None | None | Linear + Affine |
| Concurrency | Ownership | Effects | Regions | Effects | Regions + Linear |

---

## Appendix C: Complete Code Examples

This appendix provides comprehensive examples demonstrating Blood's features.

### C.1 Hello World

The simplest Blood program:

```blood
fn main() / {IO} {
    println("Hello, World!")
}
```

### C.2 FizzBuzz with Effects

Using effects for structured output:

```blood
effect FizzBuzz {
    op fizz() -> unit;
    op buzz() -> unit;
    op fizzbuzz() -> unit;
    op number(n: i32) -> unit;
}

fn fizzbuzz_logic(n: i32) / {FizzBuzz} {
    for i in 1..=n {
        match (i % 3, i % 5) {
            (0, 0) => fizzbuzz(),
            (0, _) => fizz(),
            (_, 0) => buzz(),
            _      => number(i),
        }
    }
}

fn main() / {IO} {
    with PrintHandler handle {
        fizzbuzz_logic(100)
    }
}

deep handler PrintHandler for FizzBuzz {
    op fizz() { println("Fizz"); resume(()) }
    op buzz() { println("Buzz"); resume(()) }
    op fizzbuzz() { println("FizzBuzz"); resume(()) }
    op number(n) { println(n.to_string()); resume(()) }
}
```

### C.3 Generational References Example

Demonstrating safe memory management:

```blood
struct Node {
    value: i32,
    next: Option<Box<Node>>,
}

fn create_list(values: &[i32]) -> Box<Node> / {Allocate} {
    let mut head: Option<Box<Node>> = None

    for &v in values.iter().rev() {
        head = Some(Box::new(Node {
            value: v,
            next: head,
        }))
    }

    head.unwrap()
}

fn sum_list(node: &Node) -> i32 / pure {
    let mut sum = 0
    let mut current = Some(node)

    while let Some(n) = current {
        sum += n.value
        current = n.next.as_ref().map(|b| b.as_ref())
    }

    sum
}

fn main() / {IO} {
    let list = create_list(&[1, 2, 3, 4, 5])
    let total = sum_list(&list)
    println("Sum: " ++ total.to_string())
    // list automatically freed, generation incremented
    // Any stale references would be detected
}
```

### C.4 Multiple Dispatch Example

Type-stable multiple dispatch:

```blood
// Generic function declaration (method family)
fn format(x: T) -> String / pure where T: Display

// Specializations for different types
fn format(x: i32) -> String / pure {
    x.to_string()
}

fn format(x: f64) -> String / pure {
    if x.floor() == x {
        x.to_string() ++ ".0"
    } else {
        x.to_string()
    }
}

fn format(x: bool) -> String / pure {
    if x { "yes" } else { "no" }
}

fn format(x: Vec<T>) -> String / pure where T: Display {
    let items = x.iter()
        .map(|item| format(item))
        .collect::<Vec<_>>()
        .join(", ")
    "[" ++ items ++ "]"
}

fn main() / {IO} {
    println(format(42))           // "42"
    println(format(3.14))         // "3.14"
    println(format(true))         // "yes"
    println(format(vec![1, 2, 3])) // "[1, 2, 3]"
}
```

### C.5 Error Handling with Effects

Structured error handling:

```blood
enum ParseError {
    InvalidNumber(String),
    UnexpectedChar(char),
    EndOfInput,
}

fn parse_int(s: &str) -> i32 / {Error<ParseError>} {
    if s.is_empty() {
        raise(ParseError::EndOfInput)
    }

    let mut result = 0i32
    for c in s.chars() {
        match c.to_digit(10) {
            Some(d) => result = result * 10 + d as i32,
            None => raise(ParseError::UnexpectedChar(c)),
        }
    }
    result
}

fn main() / {IO} {
    // Handle errors by converting to Result
    let result: Result<i32, ParseError> = with ResultHandler handle {
        parse_int("123")
    };

    match result {
        Ok(n) => println("Parsed: " ++ n.to_string()),
        Err(e) => println("Error: " ++ e.to_string()),
    }
}

deep handler ResultHandler<T, E> for Error<E> -> Result<T, E> {
    return(x) { Ok(x) }
    op raise(e) { Err(e) }  // No resume - error terminates
}
```

### C.6 Concurrent Fibers (ADR-036)

Concurrent programming with effect-based fibers:

```blood
fn fetch_data(url: &str) -> Bytes / {Fiber, Error<HttpError>} {
    let response = http_get(url)?
    response.body()
}

fn process_urls(urls: Vec<String>) -> Vec<Result<Bytes, HttpError>> / {Fiber} {
    // Spawn concurrent fetches
    let handles: Vec<FiberHandle<Result<Bytes, HttpError>>> = urls
        .into_iter()
        .map(|url| spawn(|| {
            with ErrorToResult handle {
                fetch_data(&url)
            }
        }))
        .collect()

    // Join all results
    handles.into_iter()
        .map(|h| join(h))
        .collect()
}

fn main() / {IO} {
    let urls = vec![
        "https://api.example.com/data1".to_string(),
        "https://api.example.com/data2".to_string(),
    ];

    let results = with FiberRuntime handle {
        process_urls(urls)
    };

    for (i, result) in results.iter().enumerate() {
        match result {
            Ok(data) => println("URL " ++ i.to_string() ++ ": " ++ data.len().to_string() ++ " bytes"),
            Err(e) => println("URL " ++ i.to_string() ++ " failed: " ++ e.to_string()),
        }
    }
}
```

### C.7 State Effect Example

Mutable state without mutation:

```blood
fn counter_example() -> i32 / {State<i32>} {
    put(0)

    for _ in 0..10 {
        let current = get()
        put(current + 1)
    }

    get()
}

fn main() / {IO} {
    // Run with state handler
    let final_count = with StateHandler::new(0) handle {
        counter_example()
    };

    println("Final count: " ++ final_count.to_string())
}

deep handler StateHandler<S> for State<S> {
    state: S

    fn new(initial: S) -> StateHandler<S> {
        StateHandler { state: initial }
    }

    op get() {
        resume(self.state.clone())
    }

    op put(s) {
        self.state = s
        resume(())
    }

    op modify(f) {
        self.state = f(self.state.clone())
        resume(())
    }
}
```

### C.8 Linear Types for Resources

Ensuring resource cleanup:

```blood
// Linear type - must be used exactly once
struct linear File {
    fd: Fd,
}

impl File {
    fn open(path: &Path) -> File / {IO, Error<IoError>} {
        let fd = io_open(path)?
        File { fd }
    }

    // Consuming method - takes ownership
    fn close(self) / {IO} {
        io_close(self.fd)
        // self is consumed, cannot be used again
    }

    fn read(&mut self, buf: &mut [u8]) -> usize / {IO, Error<IoError>} {
        io_read(self.fd, buf)
    }
}

fn safe_file_operation(path: &Path) / {IO, Error<IoError>} {
    let mut file = File::open(path)?

    let mut buffer = [0u8; 1024]
    let bytes_read = file.read(&mut buffer)?

    file.close()  // MUST call close - compiler enforces

    // file.read(...) here would be compile error:
    // "linear value already consumed"
}
```

### C.9 Generator with Yield Effect

Lazy sequences:

```blood
fn fibonacci() -> impl Iterator<Item = u64> / pure {
    gen {
        let (mut a, mut b) = (0u64, 1u64)
        loop {
            yield(a)
            (a, b) = (b, a.checked_add(b).unwrap_or(u64::MAX))
        }
    }
}

fn primes() -> impl Iterator<Item = u64> / pure {
    gen {
        yield(2)
        let mut n = 3u64
        while n < u64::MAX {
            if is_prime(n) {
                yield(n)
            }
            n += 2
        }
    }
}

fn main() / {IO} {
    // First 10 Fibonacci numbers
    let fibs: Vec<u64> = fibonacci().take(10).collect()
    println("Fibonacci: " ++ format(fibs))

    // First 10 primes
    let ps: Vec<u64> = primes().take(10).collect()
    println("Primes: " ++ format(ps))
}
```

### C.10 Complete Application Example

A mini HTTP server demonstrating multiple features:

```blood
//! A simple HTTP echo server demonstrating Blood features

struct Request {
    method: String,
    path: String,
    body: Bytes,
}

struct Response {
    status: u16,
    body: Bytes,
}

effect Http {
    op handle_request(req: Request) -> Response;
}

fn echo_handler(req: Request) -> Response / pure {
    Response {
        status: 200,
        body: req.body,
    }
}

fn logging_middleware<H>(handler: H) -> impl Fn(Request) -> Response / {IO}
where
    H: Fn(Request) -> Response / pure
{
    move |req| {
        info!("Request: {} {}", req.method, req.path)
        let start = now()
        let response = handler(req)
        let duration = now() - start
        info!("Response: {} in {:?}", response.status, duration)
        response
    }
}

fn run_server(addr: &str) / {IO, Fiber, Error<IoError>} {
    let listener = tcp_bind(addr)?

    info!("Listening on {}", addr)

    loop {
        let (stream, peer) = listener.accept()?
        info!("Connection from {}", peer)

        spawn(|| {
            with ErrorToResult handle {
                let handler = logging_middleware(echo_handler)
                serve_connection(stream, handler)
            }
        })
    }
}

fn main() / {IO} {
    with FiberRuntime handle {
        with ErrorHandler handle {
            run_server("0.0.0.0:8080")
        }
    }
}
```

---

## Revision History

| Version | Date | Changes |
|---------|------|---------|
| 0.3.0 | 2026-02-28 | Added MACROS.md to spec hierarchy, fixed dead links (UCM.md, IMPLEMENTATION_STATUS.md), aligned version with companions |
| 0.2.0 | 2026-01-09 | Added comprehensive examples, tooling reference |
| 0.1.0 | 2026-01-09 | Initial specification |

---

*This specification is maintained alongside the implementation. See [../planning/ROADMAP.md](../planning/ROADMAP.md) for current status.*
