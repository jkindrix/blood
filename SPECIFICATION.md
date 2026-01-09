# Blood Programming Language Specification

**Version**: 0.1.0-draft
**Status**: Active Development
**Last Updated**: 2026-01-09

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
| Mutable Value Semantics | Hylo | Default value semantics with escape analysis |
| Algebraic Effects | Koka | Row-polymorphic effect system |
| Multiple Dispatch | Julia | Type-stable open dispatch |

### 1.2 Design Goals

1. **Memory safety without garbage collection** — Deterministic resource management
2. **Effect tracking in types** — All side effects visible in function signatures
3. **Zero-cost abstractions when provable** — Compile-time proofs eliminate runtime checks
4. **Predictable performance** — No hidden allocations or implicit dispatch
5. **Seamless C interoperability** — First-class FFI with content-addressed bridges

> **Performance Disclaimer**: Performance claims in this specification (e.g., "~1-2 cycles per dereference", "zero-cost when provable") are **design goals** based on theoretical analysis and source language experience, not empirically validated benchmarks. Vale, the source of generational references, has not published production benchmarks as of January 2026. Actual performance will be validated during implementation. See [§11. Performance Characteristics](#11-performance-characteristics) for detailed analysis.

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
| Semantics | [FORMAL_SEMANTICS.md](./FORMAL_SEMANTICS.md) | Operational semantics, typing rules |
| Library | [STDLIB.md](./STDLIB.md) | Standard library effects and types |
| Diagnostics | [DIAGNOSTICS.md](./DIAGNOSTICS.md) | Error messages, warning system |
| Roadmap | [ROADMAP.md](./ROADMAP.md) | Implementation plan, milestones |

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

Blood should be simpler than Rust to learn, but not by hiding complexity — by **eliminating unnecessary complexity**. Mutable value semantics are genuinely simpler than borrow checking.

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
| **Affine** | `affine T` | 0..1 | Auto | Forbidden | Move-only resources |
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
- **Resume later** — Async, coroutines

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

effect Async<T> {
    op await(future: Future<T>) -> T
    op spawn(task: fn() -> T / Async<T>) -> Handle<T>
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

#### Async

```blood
effect Async {
    op await<T>(future: Future<T>) -> T
    op spawn<T>(task: fn() -> T / Async) -> TaskHandle<T>
    op yield_now() -> unit
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
| **Default** | Mutable value semantics (copies, not references) | Hylo (Val) |
| **Explicit** | Borrowing via `&T` and `&mut T` when needed | Rust |
| **Heap** | Generational references for `Box<T>`, collections | Vale |
| **Resources** | Linear/affine types for must-use handles | Linear logic |

> **Terminology Note**: Blood uses a **hybrid model**, not pure Hylo-style MVS. While Hylo deliberately avoids explicit borrowing, Blood includes `&T` and `&mut T` for cases where value semantics are insufficient. This provides the simplicity benefits of MVS while retaining reference capability when genuinely needed. See [MEMORY_MODEL.md §1.3](./MEMORY_MODEL.md#13-hybrid-model-clarification) for details.

### 5.2 Tiered Memory Model

Blood uses a three-tier memory model based on escape analysis:

| Tier | Lifecycle | Mechanism | Cost |
|------|-----------|-----------|------|
| **Tier 1: Stack** | Lexical | CPU SP manipulation | Zero |
| **Tier 2: Region** | Scoped | Generational Check | ~1-2 cycles/deref |
| **Tier 3: Persistent** | Global | Deferred Ref Counting | Higher |

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

### 5.4 Mutable Value Semantics

Blood defaults to value semantics. The compiler performs inter-procedural escape analysis:

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
fn sender(linear data: Buffer) / {Async} {
    let handle = spawn(|| receiver(data))  // Ownership transferred
    // data no longer accessible here
}

fn receiver(linear data: Buffer) / {Async} {
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
fn parallel_work() / {Async} {
    // Both tasks complete before function returns
    let a = spawn(|| compute_a())
    let b = spawn(|| compute_b())
    combine(await(a), await(b))
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

*To be specified in future revision.*

### 9.1 Core Types

- `Option<T>`, `Result<T, E>`
- `Vec<T>`, `HashMap<K, V>`, `String`
- `Box<T>`, `Rc<T>`, `Arc<T>`

### 9.2 Core Effects

- `IO`, `Error<E>`, `State<S>`
- `Async`, `Yield<T>`, `NonDet`

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
| Snapshot capture | O(n) refs | Theoretical | Novel mechanism |
| Snapshot validation | O(n) refs | Theoretical | Novel mechanism |
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

Based on Java HotSpot research (validated) and theoretical extension:

| Code Pattern | Stack Allocation Rate | Status |
|--------------|----------------------|--------|
| Local objects, no escape | >95% | Validated (Java) |
| Objects passed to callees | 60-80% | Validated (Java) |
| Objects crossing effect boundaries | Lower | Theoretical |
| Objects in closures | Varies | Requires analysis |

### 11.6 Compile-Time Considerations

| Analysis | Complexity | Impact |
|----------|------------|--------|
| Escape analysis (local) | O(n) | Fast |
| Escape analysis (inter-procedural) | O(n²) worst case | May require optimization levels |
| Type stability checking | O(methods × types) | Potentially expensive |
| Effect inference | O(n) | Linear in program size |
| Generation snapshot analysis | O(refs × effects) | Novel, complexity TBD |

**Recommendation**: Blood may offer optimization levels trading compile time for runtime performance:
- `-O0`: No escape analysis, all heap allocations use generation checks
- `-O1`: Local escape analysis only
- `-O2`: Inter-procedural escape analysis (default)
- `-O3`: Aggressive analysis with higher compile time

### 11.7 Benchmarking Plan

Before Blood 1.0, the following benchmarks must validate performance claims:

1. **Micro-benchmarks**
   - Generation check overhead (cycles per dereference)
   - Snapshot capture/validation cost
   - Effect handler installation/invocation

2. **Macro-benchmarks**
   - Memory-intensive algorithms (sorting, graph traversal)
   - Allocation-heavy workloads
   - Effect-heavy concurrent code

3. **Comparative benchmarks**
   - vs. Rust (borrow checking baseline)
   - vs. Go (GC baseline)
   - vs. C (unsafe baseline)

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

## Revision History

| Version | Date | Changes |
|---------|------|---------|
| 0.1.0-draft | 2026-01-09 | Initial specification |

---

*This specification is a living document. Sections marked "To be specified" are under active development.*
