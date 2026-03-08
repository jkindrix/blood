# Blood Formal Semantics

**Version**: 0.4.0
**Status**: Specified
**Implementation**: ✅ Complete (22 Coq proof files, 10,507 lines, 43 named theorems, 227 Qed total, 0 Admitted)
**Last Updated**: 2026-03-07

**Revision 0.4.0 Changes**:
- Added closure typing rules (§5.7) — capture modes, linearity propagation, effect composition
- Added region typing (§5.8) — scoped allocation with generational safety
- Added pattern matching typing (§5.9) — exhaustiveness, pattern binding
- Added cast typing rules (§5.10) — numeric, pointer, and sign reinterpretation casts
- Added associated type typing (§5.11) — projection, defaults, generic resolution
- Added closure-handler linearity interaction (§8.3)
- Added scope statement listing formalized vs. companion-document features
- Added cross-references to MACROS.md and FFI.md
- Extended surface syntax (§1.1) with closures, regions, match, cast
- Extended reduction rules (§3.1.1) with region evaluation semantics

**Revision 0.3.0 Changes**:
- Added notation alignment with GRAMMAR.md surface syntax
- Added cross-references between formal and surface notations
- Added implementation status

This document provides the formal operational semantics for Blood, suitable for mechanized proof and compiler verification.

### Scope

This document formalizes the **core type system and evaluation semantics** of Blood. The following features are formalized here:

| Feature | Section | Status |
|---------|---------|--------|
| Lambda calculus + let bindings | §1-§3, §5.1-§5.2 | Complete |
| Row-polymorphic records | §5.5 | Complete |
| Algebraic effects + handlers | §3.2-§3.4, §5.3, §6 | Complete |
| Generational references | §4 | Complete |
| Linear and affine types | §5.4, §8 | Complete |
| Closures + capture semantics | §5.7 | Complete |
| Region scoped allocation | §5.8 | Complete |
| Pattern matching | §5.9 | Complete |
| Type casts | §5.10 | Complete |
| Associated types | §5.11 | Complete |

The following features are **specified in companion documents** and are NOT duplicated here:

| Feature | Document | Rationale |
|---------|----------|-----------|
| Macro expansion | [MACROS.md](./MACROS.md) | Pre-type-checking source transformation; no interaction with typing rules |
| Bridge FFI safety | [FFI.md](./FFI.md) | FFI operates at the ABI boundary, outside the type-safe core |
| Multiple dispatch resolution | [DISPATCH.md §3](./DISPATCH.md#3-dispatch-resolution-algorithm) | Extends [T-App] with dispatch; cross-referenced in §5.2 |
| Object safety + dyn Trait | [DISPATCH.md §10.7-10.8](./DISPATCH.md#107-object-safety) | ABI constraints for vtable construction |
| Memory model (tiers, GC) | [MEMORY_MODEL.md](./MEMORY_MODEL.md) | Runtime memory management details |
| Concurrency (fibers) | [CONCURRENCY.md](./CONCURRENCY.md) | Fiber scheduling and isolation |

### Related Specifications

- [SPECIFICATION.md](./SPECIFICATION.md) — Core language specification
- [MEMORY_MODEL.md](./MEMORY_MODEL.md) — Generation snapshot semantics
- [DISPATCH.md](./DISPATCH.md) — Multiple dispatch typing rules and object safety
- [GRAMMAR.md](./GRAMMAR.md) — Surface syntax grammar; see [Notation Alignment](./GRAMMAR.md#notation-alignment) for surface-to-formal notation mapping
- [MACROS.md](./MACROS.md) — Macro system design, fragment kinds, expansion semantics
- [FFI.md](./FFI.md) — Bridge FFI safety model and ABI specification
- [STDLIB.md](./STDLIB.md) — Standard effect and type definitions

---

## Table of Contents

1. [Syntax](#1-syntax)
2. [Evaluation Contexts](#2-evaluation-contexts)
3. [Reduction Rules](#3-reduction-rules)
4. [Generation Snapshots](#4-generation-snapshots)
5. [Typing Rules](#5-typing-rules)
6. [Handler Typing](#6-handler-typing)
7. [Progress and Preservation](#7-progress-and-preservation)
8. [Linear Types and Effects Interaction](#8-linear-types-and-effects-interaction)
9. [Metatheory Summary](#9-metatheory-summary)
10. [Composition Safety Analysis](#10-composition-safety-analysis)
11. [Proof Sketches for Core Theorems](#11-proof-sketches-for-core-theorems)
12. [Mechanization Roadmap](#12-mechanization-roadmap)
13. [Complete Generation Snapshots Proof](#13-complete-generation-snapshots-proof)
- [Appendix A: Notation Reference](#appendix-a-notation-reference)
- [Appendix B: Related Work and Citations](#appendix-b-related-work-and-citations)

---

## 1. Syntax

### 1.1 Surface Syntax (Expressions)

```
e ::= x                           -- Variable
    | c                           -- Constant
    | λx:T. e                     -- Lambda abstraction
    | e e                         -- Application
    | let x = e in e              -- Let binding
    | (e : T)                     -- Type annotation
    | {l₁ = e₁, ..., lₙ = eₙ}     -- Record
    | e.l                         -- Field access
    | {l = e | e}                 -- Record extension
    | perform E.op(e)             -- Effect operation
    | with h handle e             -- Effect handling
    | resume(e)                   -- Continuation resume (in handlers)
    | |x₁:T₁,...,xₙ:Tₙ| e       -- Closure (captures by reference)
    | move |x₁:T₁,...,xₙ:Tₙ| e  -- Move closure (captures by value)
    | match e { p₁ => e₁, ..., pₙ => eₙ }  -- Pattern match
    | e as T                      -- Type cast
    | region e                    -- Anonymous region scope
    | region 'r e                 -- Named region scope
```

### 1.2 Values

```
v ::= c                           -- Constants
    | λx:T. e                     -- Functions
    | {l₁ = v₁, ..., lₙ = vₙ}     -- Record values
    | (κ, Γ_gen, L)               -- Continuation (with snapshot)
    | ⟨Env, λx:T. e⟩             -- Closure value (environment + code)
```

### 1.3 Types

```
T ::= B                           -- Base types (i32, bool, etc.)
    | T → T / ε                   -- Function types with effects
    | {l₁: T₁, ..., lₙ: Tₙ | ρ}   -- Row-polymorphic records
    | ∀α. T                       -- Universal quantification
    | linear T                    -- Linear types
    | affine T                    -- Affine types
    | !T                          -- Generational reference

ε ::= {E₁, ..., Eₙ | ρ}           -- Effect rows
    | pure                        -- Empty effect row (sugar for {})
```

---

## 2. Evaluation Contexts

### 2.1 Standard Contexts

```
E ::= □
    | E e                         -- Function position
    | v E                         -- Argument position
    | let x = E in e              -- Let binding (scrutinee)
    | {l₁=v₁,...,lᵢ=E,...,lₙ=eₙ}  -- Record (left-to-right)
    | E.l                         -- Field access
    | {l = E | e}                 -- Record extension (value)
    | {l = v | E}                 -- Record extension (base)
    | perform E.op(e)             -- Effect operation (effect expr)
    | perform v.op(E)             -- Effect operation (argument)
    | with E handle e             -- Handler expression
    | with v handle E             -- Handled computation
```

### 2.2 Delimited Contexts (for Effect Handling)

A delimited evaluation context `D` is an evaluation context that does not cross a handler boundary:

```
D ::= □
    | D e
    | v D
    | let x = D in e
    | {l₁=v₁,...,lᵢ=D,...,lₙ=eₙ}
    | D.l
    | {l = D | e}
    | {l = v | D}
    | perform D.op(e)
    | perform v.op(D)
    -- Note: NO 'with h handle D' here
```

---

## 3. Reduction Rules

### 3.1 Standard Reduction

```
(λx:T. e) v  ──►  e[v/x]                                    [β-App]

let x = v in e  ──►  e[v/x]                                 [β-Let]

{l₁=v₁,...,lₙ=vₙ}.lᵢ  ──►  vᵢ                               [Record-Select]

{l = v | {l₁=v₁,...,lₙ=vₙ}}  ──►  {l=v,l₁=v₁,...,lₙ=vₙ}    [Record-Extend]
```

### 3.1.1 Region Reduction

```
region e  ──►  let r = fresh_region() in
               activate(r); let result = e in
               deactivate(r); destroy(r); result        [Region-Eval]

-- Region destruction bumps generations of all allocations in r:
destroy(r):
    ∀a ∈ allocations(r). M(a).generation += 1           [Region-Destroy]
```

> **Design note**: Blood uses generational references (ADR-001), not borrow checking. References to region-allocated objects are NOT prevented from escaping at the type level. Instead, generation bumps on region exit invalidate all pointers. Any subsequent dereference triggers `StaleReference` via [Resume-Stale] or the standard dereference check (§4.5). Compile-time escape analysis may warn or optimize, but the safety guarantee is the generation system. See [MEMORY_MODEL.md §7](./MEMORY_MODEL.md#7-region-based-allocation) for the full runtime specification.

### 3.2 Effect Handling (Deep Handlers)

Let `h` be a deep handler for effect `E` with:
- Return clause: `return(x) { e_ret }`
- Operation clause for `op`: `op(x) { e_op }` (where `resume` may appear in `e_op`)

```
with h handle v  ──►  e_ret[v/x]                            [Handle-Return]

with h handle D[perform E.op(v)]
    ──►  e_op[v/x, (λy. with h handle D[y], Γ_gen, L)/resume]
    where Γ_gen = GenSnapshot(D)                            [Handle-Op-Deep]
          L = LinearVars(D) (must be ∅ or explicitly transferred)
```

### 3.3 Effect Handling (Shallow Handlers)

Let `h` be a shallow handler:

```
with h handle v  ──►  e_ret[v/x]                            [Handle-Return-Shallow]

with h handle D[perform E.op(v)]
    ──►  e_op[v/x, (λy. D[y], Γ_gen, L)/resume]             [Handle-Op-Shallow]
    -- Note: handler NOT re-wrapped around continuation
```

### 3.4 Continuation Resume

```
resume((κ, Γ_gen, ∅), v)  ──►  κ(v)
    if ∀(a,g) ∈ Γ_gen. currentGen(a) = g                   [Resume-Valid]

resume((κ, Γ_gen, ∅), v)  ──►  perform StaleReference.stale(a, g, g')
    if ∃(a,g) ∈ Γ_gen. currentGen(a) = g' ≠ g              [Resume-Stale]
```

> **Note**: The formal notation `stale(a, g, g')` corresponds to the concrete Blood API `op stale(info: StaleInfo) -> !` where `StaleInfo` packages address, expected/actual generation, and optional debug fields. See SPECIFICATION.md §4.5 and MEMORY_MODEL.md Appendix B for the concrete definition.

---

## 4. Generation Snapshots

### 4.1 Definitions

```
Address     = ℕ                   -- Memory addresses
Generation  = ℕ                   -- Generation counters
GenRef      = (Address, Generation)
GenSnapshot = P(GenRef)           -- Finite set of gen-refs
```

### 4.2 Extraction Function

`GenRefs : Context → GenSnapshot` extracts all generational references from an evaluation context:

```
GenRefs(□) = ∅

GenRefs(E e) = GenRefs(E) ∪ GenRefsExpr(e)

GenRefs(v E) = GenRefsVal(v) ∪ GenRefs(E)

GenRefs(let x = E in e) = GenRefs(E) ∪ GenRefsExpr(e)

GenRefs(with h handle E) = GenRefs(E)
    -- Handler boundary: we only capture refs IN the continuation

GenRefsExpr(e) = { (a, g) | !a^g appears in e }

GenRefsVal(v) = { (a, g) | !a^g appears in v }
```

### 4.3 Current Generation Query

`currentGen : Address → Generation` queries the memory to get current generation:

```
currentGen(a) = M(a).generation
    where M is the memory state
```

### 4.4 Memory State

```
Memory M : Address → (Value × Generation × Metadata)

M(a) = (v, g, m)   -- Address a holds value v, generation g, metadata m
```

### 4.5 Allocation and Deallocation

```
alloc(v) :
    let a = fresh_address()
    let g = 0
    M := M[a ↦ (v, g, default_metadata)]
    return !a^g

free(!a^g) :
    let (v, g', m) = M(a)
    if g ≠ g' then TRAP  -- Generation mismatch
    M := M[a ↦ (⊥, g' + 1, m)]  -- Increment generation, clear value

deref(!a^g) :
    let (v, g', m) = M(a)
    if g ≠ g' then TRAP  -- Generation mismatch (use-after-free)
    return v
```

---

## 5. Typing Rules

### 5.1 Typing Judgment

```
Γ; Δ ⊢ e : T / ε

where:
  Γ = Type context (x : T)
  Δ = Linearity context (linear/affine tracking)
  T = Result type
  ε = Effect row
```

### 5.2 Core Rules

```
x : T ∈ Γ
─────────────────                                           [T-Var]
Γ; Δ ⊢ x : T / pure


─────────────────                                           [T-Const]
Γ; Δ ⊢ c : typeof(c) / pure


Γ, x:A; Δ,x:○ ⊢ e : B / ε
─────────────────────────────────                           [T-Lam]
Γ; Δ ⊢ λx:A. e : A → B / ε / pure


Γ; Δ₁ ⊢ e₁ : A → B / ε / ε₁    Γ; Δ₂ ⊢ e₂ : A / ε₂
Δ = Δ₁ ⊗ Δ₂                                                 [T-App]
───────────────────────────────────────────────────
Γ; Δ ⊢ e₁ e₂ : B / ε ∪ ε₁ ∪ ε₂
```

**Note on Multiple Dispatch**: For method calls `f(e₁, ..., eₙ)` where `f` is a method family (multiple dispatch), see [DISPATCH.md §10](./DISPATCH.md#10-cross-reference-formal-typing-rules) for the extended typing rule that includes dispatch resolution. The rule above applies to direct function application.

```
Γ; Δ₁ ⊢ e₁ : A / ε₁    Γ, x:A; Δ₂, x:○ ⊢ e₂ : B / ε₂
Δ = Δ₁ ⊗ Δ₂
───────────────────────────────────────────────────         [T-Let]
Γ; Δ ⊢ let x = e₁ in e₂ : B / ε₁ ∪ ε₂
```

### 5.3 Effect Rules

```
effect E { op : A → B } ∈ Σ    E ∈ ε
Γ; Δ ⊢ e : A / ε'
───────────────────────────────────────                     [T-Perform]
Γ; Δ ⊢ perform E.op(e) : B / {E} ∪ ε'


Γ; Δ₁ ⊢ e : T / {E | ε}
Γ; Δ₂ ⊢ h : Handler(E, T, U, ε')
Δ = Δ₁ ⊗ Δ₂
───────────────────────────────────────                     [T-Handle]
Γ; Δ ⊢ with h handle e : U / ε ∪ ε'
```

### 5.4 Linearity Rules

```
Γ; Δ, x:1 ⊢ e : T / ε    x ∈ FV(e)
───────────────────────────────────                         [T-Linear-Use]
Γ; Δ ⊢ let x: linear A = v in e : T / ε


Γ; Δ, x:1 ⊢ e : T / ε    x ∉ FV(e)
───────────────────────────────────                         [T-Linear-Unused: ERROR]
⊥


Γ; Δ, x:? ⊢ e : T / ε
───────────────────────────────────                         [T-Affine-Use]
Γ; Δ ⊢ let x: affine A = v in e : T / ε
    -- x may or may not appear in e
```

### 5.5 Row Polymorphism Rules

```
Γ; Δ ⊢ e : {l₁:T₁,...,lₙ:Tₙ | ρ}    l = lᵢ
──────────────────────────────────────────                  [T-Select]
Γ; Δ ⊢ e.l : Tᵢ / pure


Γ; Δ₁ ⊢ e₁ : T    Γ; Δ₂ ⊢ e₂ : {l₁:T₁,...,lₙ:Tₙ | ρ}
l ∉ {l₁,...,lₙ}    Δ = Δ₁ ⊗ Δ₂
──────────────────────────────────────────────────          [T-Extend]
Γ; Δ ⊢ {l = e₁ | e₂} : {l:T, l₁:T₁,...,lₙ:Tₙ | ρ} / pure
```

### 5.6 Subtyping

```
─────────                                                   [S-Refl]
T <: T


S <: T    T <: U
────────────────                                            [S-Trans]
S <: U


A' <: A    B <: B'    ε ⊆ ε'
────────────────────────────                                [S-Fun]
(A → B / ε) <: (A' → B' / ε')


ε₁ ⊆ ε₂
────────────────────                                        [S-Effect]
ε₁ <: ε₂


─────────────────                                           [S-Pure]
pure <: ε
```

### 5.7 Closure Typing

Closures differ from lambda abstractions in that they capture variables from an enclosing scope. The capture environment determines the closure's linearity and effect constraints.

> **Design note**: Blood uses a single callable type `fn(T) -> U / ε` rather than Rust's `Fn`/`FnMut`/`FnOnce` trait hierarchy. The three concerns those traits encode are handled by orthogonal systems: mutation tracking via effects (ADR-002), one-shot semantics via linear types (ADR-006), and effect polymorphism via row polymorphism (ADR-009). See [GRAMMAR.md §4.4](./GRAMMAR.md#44-callable-types-design-note).

#### 5.7.1 Capture Environment

A capture environment `Env` maps captured variables to their capture modes:

```
CaptureMode ::= ref    -- Shared reference to enclosing variable
              | mut    -- Mutable reference to enclosing variable
              | val    -- By-value (move) of enclosing variable

Env = { (x₁, m₁, T₁), ..., (xₙ, mₙ, Tₙ) }
    where xᵢ ∈ dom(Γ), mᵢ ∈ CaptureMode, Tᵢ = Γ(xᵢ)
```

For a closure expression `|params| body`:
- Variables in `FV(body) \ {params}` that are bound in the enclosing scope are captured
- Without `move`, capture mode is inferred: `ref` for read-only access, `mut` for mutation
- With `move`, all captures use `val` mode

#### 5.7.2 Typing Rules

```
Env = captures(Γ, FV(body) \ {x₁,...,xₘ})
Γ, x₁:A₁,...,xₘ:Aₘ; Δ_body ⊢ body : U / ε
∀(xᵢ, ref, Tᵢ) ∈ Env. xᵢ remains available in Γ
∀(xᵢ, mut, Tᵢ) ∈ Env. xᵢ remains available in Γ
∀(xᵢ, val, Tᵢ) ∈ Env. xᵢ consumed from Δ
──────────────────────────────────────────────  [T-Closure]
Γ; Δ ⊢ |x₁:A₁,...,xₘ:Aₘ| body
       : fn(A₁,...,Aₘ) -> U / ε / pure

Env = { (xᵢ, val, Tᵢ) | xᵢ ∈ FV(body) \ {params} }
Γ, x₁:A₁,...,xₘ:Aₘ; Δ_body ⊢ body : U / ε
∀(xᵢ, val, Tᵢ) ∈ Env. xᵢ consumed from Δ
──────────────────────────────────────────────  [T-Closure-Move]
Γ; Δ ⊢ move |x₁:A₁,...,xₘ:Aₘ| body
       : fn(A₁,...,Aₘ) -> U / ε / pure
```

**Note**: The closure expression itself is pure — it constructs a value. The effect row `ε` describes what happens when the closure is *called*, not when it is *created*.

#### 5.7.3 Closure Application

Closure application follows the standard function application rule [T-App]:

```
Γ; Δ₁ ⊢ e_clo : fn(A) -> U / ε / ε₁    Γ; Δ₂ ⊢ e_arg : A / ε₂
Δ = Δ₁ ⊗ Δ₂
───────────────────────────────────────────────────                [T-Closure-App]
Γ; Δ ⊢ e_clo(e_arg) : U / ε ∪ ε₁ ∪ ε₂
```

At runtime, closure application evaluates the body in the captured environment extended with the arguments:

```
⟨Env, λx:T. body⟩ v  ──►  body[Env, v/x]                       [β-Closure]
```

#### 5.7.4 Capture Linearity

Closures interact with linearity through capture modes:

```
-- Linear values CANNOT be captured by reference (no aliasing)
∀(xᵢ, ref, Tᵢ) ∈ Env. Tᵢ ≠ linear S                           [Linear-No-Ref]

-- Linear values CANNOT be captured by mutable reference
∀(xᵢ, mut, Tᵢ) ∈ Env. Tᵢ ≠ linear S                           [Linear-No-Mut]

-- Linear values CAN be captured by value (move)
-- The closure itself becomes linear
If ∃(xᵢ, val, Tᵢ) ∈ Env. Tᵢ = linear S
then the closure has type: linear fn(A) -> U / ε                 [Linear-Closure]
```

**Consequence**: A closure capturing a linear value by-value is itself linear — it must be called exactly once. This replaces Rust's `FnOnce` without a separate trait.

#### 5.7.5 Closure Effect Composition

The effect row on a closure type describes the effects performed when the closure is called:

```
-- Pure closure (no effects when called)
|x| x * 2                        : fn(i32) -> i32 / pure

-- IO closure
|x| perform IO.print(x)          : fn(i32) -> () / {IO}

-- Closure with row-polymorphic effects
-- (when used as argument to a generic function)
f : ∀ε. fn(fn() -> i32 / {IO | ε}) -> i32 / {IO | ε}
```

Effect subtyping through closures follows [S-Fun]: a closure with fewer effects can be passed where more effects are expected.

### 5.8 Region Typing

Regions provide scoped bulk allocation with generational safety. Unlike systems with borrow checking (which prevent reference escape at the type level), Blood detects escaped references at runtime via generation checks (ADR-001).

> **Design note**: Region typing does NOT introduce lifetime annotations on types. The typing rule is simple: a region block produces a value, and references allocated within the region become stale when the region exits. Safety is guaranteed by the generation system (§4), not by type-level region tracking. See [MEMORY_MODEL.md §7](./MEMORY_MODEL.md#7-region-based-allocation) for the runtime specification and [GRAMMAR.md §5.3](./GRAMMAR.md#53-region-expressions) for surface syntax.

#### 5.8.1 Region Expression Typing

```
Γ; Δ ⊢ e : T / ε
─────────────────────────────────────                              [T-Region]
Γ; Δ ⊢ region e : T / ε

Γ; Δ ⊢ e : T / ε
─────────────────────────────────────                              [T-Region-Named]
Γ; Δ ⊢ region 'r e : T / ε
```

The region expression has the same type and effects as its body. No type-level distinction is introduced — region boundaries are invisible to the type system.

#### 5.8.2 Region Safety via Generations

Region safety is NOT a typing property — it is a runtime property guaranteed by the generation system:

```
-- After region exit, all allocations in region r have bumped generations
∀a ∈ allocations(r). currentGen(a) = g_old + 1                   [Region-Invalidation]

-- Any reference !a^g to region-allocated memory satisfies:
-- g = g_old (from before region exit), currentGen(a) = g_old + 1
-- Therefore g ≠ currentGen(a), so deref triggers StaleReference  [Region-Stale-Detect]
```

This is a consequence of the existing generation safety theorem (§7.3, §11.5). No new proof obligations arise — region deallocation is just a specific case of `free` that bumps generations.

#### 5.8.3 Region-Effect Interaction

When an effect handler captures a continuation that references region-allocated memory:

```
-- At effect operation inside a region:
with h handle region { ... D[perform E.op(v)] ... }

-- The continuation κ captures references to region-allocated memory
-- The generation snapshot Γ_gen includes these references
-- Region deallocation is DEFERRED until all continuations complete
```

This deferred deallocation is specified operationally in [MEMORY_MODEL.md §7.7](./MEMORY_MODEL.md#77-region-effect-interaction). The formal guarantee follows from [Handle-Op-Deep] and [Resume-Valid]/[Resume-Stale]: if the region is deallocated before resume, the generation snapshot detects it.

#### 5.8.4 Nested Regions

Regions may nest. Inner region deallocation does not affect outer region allocations:

```
region 'outer {
    let x = alloc_in('outer, v₁)     -- allocated in outer
    region 'inner {
        let y = alloc_in('inner, v₂) -- allocated in inner
        use(x, y)                     -- both valid
    }
    -- y's generation bumped (stale), x still valid
    use(x)                            -- OK: outer still live
}
-- x's generation bumped (stale)
```

The nesting invariant follows from generation independence: each region maintains its own generation counters, and `destroy(r_inner)` only bumps generations for allocations in `r_inner`.

### 5.9 Pattern Matching

Pattern matching is exhaustive: every `match` expression must cover all possible values of the scrutinee type. The typing rules ensure that bindings introduced by patterns are correctly typed and that the overall match expression has a consistent type.

See [GRAMMAR.md §5.2](./GRAMMAR.md#52-block-and-control-flow) for surface syntax.

#### 5.9.1 Pattern Syntax

```
p ::= x                           -- Variable binding
    | _                           -- Wildcard
    | c                           -- Literal constant
    | C(p₁, ..., pₙ)             -- Constructor pattern (enum variant)
    | (p₁, ..., pₙ)              -- Tuple pattern
    | { l₁: p₁, ..., lₙ: pₙ }   -- Struct pattern
    | p₁ | p₂                    -- Or-pattern
    | p if e                      -- Guard
```

#### 5.9.2 Pattern Typing

```
Γ; Δ ⊢ e : T / ε₀
∀i. Γ ⊢ pᵢ : T ⊣ Γᵢ                     -- Pattern pᵢ matches type T, binding Γᵢ
∀i. Γ, Γᵢ; Δᵢ ⊢ eᵢ : U / εᵢ            -- Each arm body has type U
exhaustive(T, [p₁, ..., pₙ])             -- Patterns cover all of T
────────────────────────────────────────────────────────── [T-Match]
Γ; Δ ⊢ match e { p₁ => e₁, ..., pₙ => eₙ } : U / ε₀ ∪ ε₁ ∪ ... ∪ εₙ
```

All arms must produce the same type `U`. The effect row is the union of the scrutinee's effects and all arm body effects.

#### 5.9.3 Pattern Binding

```
───────────────────────────                [P-Var]
Γ ⊢ x : T ⊣ {x : T}

───────────────────────────                [P-Wildcard]
Γ ⊢ _ : T ⊣ ∅

typeof(c) = T
───────────────────────────                [P-Literal]
Γ ⊢ c : T ⊣ ∅

C : (T₁, ..., Tₙ) → T_enum
∀i. Γ ⊢ pᵢ : Tᵢ ⊣ Γᵢ
───────────────────────────────────        [P-Constructor]
Γ ⊢ C(p₁, ..., pₙ) : T_enum ⊣ Γ₁ ∪ ... ∪ Γₙ

∀i. Γ ⊢ pᵢ : Tᵢ ⊣ Γᵢ
───────────────────────────────────        [P-Tuple]
Γ ⊢ (p₁, ..., pₙ) : (T₁, ..., Tₙ) ⊣ Γ₁ ∪ ... ∪ Γₙ

Γ ⊢ p₁ : T ⊣ Γ₁    Γ ⊢ p₂ : T ⊣ Γ₂
Γ₁ = Γ₂   (same bindings in both alternatives)
───────────────────────────────────        [P-Or]
Γ ⊢ p₁ | p₂ : T ⊣ Γ₁
```

#### 5.9.4 Exhaustiveness

A pattern set `{p₁, ..., pₙ}` is exhaustive for type `T` if every value of type `T` is matched by at least one pattern. For algebraic data types (enums), this requires that every constructor is covered:

```
exhaustive(T, P) ⟺
    -- For enum types: every variant has a matching pattern
    ∀ C ∈ constructors(T). ∃ pᵢ ∈ P. matches(pᵢ, C)
    -- Recursively: sub-patterns must be exhaustive for their types
    ∧ ∀ C(p₁,...,pₖ) ∈ P. exhaustive(Tⱼ, sub_patterns(P, C, j)) for each field j

    -- For integer/string types: wildcard or variable pattern required
    -- (finite enumeration is not practical)
```

Non-exhaustive matches are compile-time errors.

### 5.10 Cast Typing

Type casts (`e as T`) perform explicit type conversions. Allowed casts are defined by the cast compatibility relation. See [GRAMMAR.md §5.6 CastExpr](./GRAMMAR.md#56-cast-expressions) for the surface syntax and cast semantics table.

```
Γ; Δ ⊢ e : S / ε    cast_compatible(S, T)
──────────────────────────────────────────     [T-Cast]
Γ; Δ ⊢ e as T : T / ε
```

#### 5.10.1 Cast Compatibility

The `cast_compatible` relation defines which casts are allowed:

```
cast_compatible(S, T) ⟺ one of:
    1. Numeric widening:  S and T are numeric, sizeof(S) ≤ sizeof(T)
                          e.g., i32 as i64, u8 as u32, f32 as f64
    2. Numeric narrowing: S and T are numeric, sizeof(S) > sizeof(T)
                          Truncates. e.g., i64 as i32, f64 as f32
    3. Int ↔ Float:       S is integer, T is float (or vice versa)
                          e.g., i32 as f64, f64 as i32 (truncates toward zero)
    4. Sign reinterpret:  S and T are integers of same width, different signedness
                          Bit-preserving. e.g., i32 as u32, u64 as i64
    5. Bool ↔ Int:        bool as integer (false=0, true=1)
                          Integer as bool (0=false, nonzero=true)
    6. Ptr ↔ usize:       Raw pointer to usize or usize to raw pointer
                          Bit-preserving. Bridge/unsafe contexts only.
    7. Ptr coercion:      &T as *const T, &mut T as *mut T
                          Bridge contexts only.
    8. Char ↔ Numeric:    Char as integer (Unicode code point, e.g., 'A' as i32 = 65)
                          Integer as Char (code point to character, e.g., 65 as Char = 'A')
                          Out-of-range values produce replacement character U+FFFD.
    9. Fn → Integer:      fn as usize (function pointer to integer)
                          Bit-preserving. Bridge/unsafe contexts only.
```

Casts that are not in this relation are compile-time errors.

> **Design note — no `&T ↔ integer` cast**: Blood references carry 128 bits of safety information (64-bit address + 32-bit generation + 32-bit metadata). Casting `&T as usize` would discard generation and metadata, and the roundtrip `&T → usize → &T` would produce a reference that bypasses stale reference detection. Use the two-step path instead: `&T as *const T` (rule 7) then `*const T as usize` (rule 6). This makes the safety boundary explicit — you must first leave the generational reference system before entering the integer domain. See `docs/design/REF_INTEGER_CASTS.md` for the full evaluation.

### 5.11 Associated Type Typing

Traits may declare associated types. Implementations provide concrete types for these declarations. Resolution normalizes associated type projections to concrete types.

#### 5.11.1 Declaration and Implementation

```
-- In trait declaration:
trait T {
    type Assoc;                    -- Associated type (no default)
    type Assoc = DefaultType;      -- Associated type with default
}

-- In implementation:
impl T for C {
    type Assoc = ConcreteType;     -- Required unless default exists
}
```

#### 5.11.2 Projection Typing

```
C : T    impl T for C { type Assoc = U }
──────────────────────────────────────────     [T-Assoc-Resolve]
<C as T>::Assoc ≡ U

-- With default:
C : T    no explicit Assoc in impl T for C    trait T { type Assoc = D }
──────────────────────────────────────────     [T-Assoc-Default]
<C as T>::Assoc ≡ D
```

#### 5.11.3 Associated Types in Generic Contexts

In generic contexts, associated type projections remain symbolic until monomorphization:

```
∀ G : T.
    <G as T>::Assoc                            -- Symbolic projection
    -- Resolves to concrete type when G is instantiated
```

Constraints may bound associated types:

```
fn f<G: T>(x: G) -> G::Assoc where G::Assoc: Display
    -- G::Assoc must implement Display
```

---

## 6. Handler Typing

### 6.1 Handler Type

```
Handler(E, T, U, ε') where:
  E   = Effect being handled
  T   = Type of handled computation result
  U   = Type of handler result
  ε'  = Effects introduced by handler implementation
```

### 6.2 Handler Well-Formedness

A handler `h` for effect `E` with operations `{op₁: A₁ → B₁, ..., opₙ: Aₙ → Bₙ}` is well-formed if:

```
-- Return clause types correctly
Γ, x:T ⊢ e_ret : U / ε'

-- Each operation clause types correctly
∀i. Γ, xᵢ:Aᵢ, resume:(Bᵢ → U / ε') ⊢ e_opᵢ : U / ε'

-- For multi-shot handlers, no linear captures
If handler is multi-shot:
  ∀i. LinearVars(FV(e_opᵢ) ∩ Γ) = ∅
```

### 6.3 Finally Clause Typing (ADR-036)

If a handler includes a `finally` clause, it must satisfy:

```
-- Finally clause types correctly in the ENCLOSING handler context
-- It may perform effects from (ε' \ {E}) where ε' is the outer effect row
-- and E is the effect being handled by this handler
Γ ⊢ e_finally : unit / (ε' \ {E})

-- Finally clause is non-cancellable:
-- The Cancel effect is NOT available within e_finally
-- check_cancelled() within finally is an unhandled effect (compile error)
Cancel ∉ available_effects(e_finally)
```

**Execution semantics**:
- Normal exit: `e_ret` evaluates, then `e_finally` evaluates
- Abnormal exit (cancellation, error): `e_finally` evaluates only
- Nested handlers: `e_finally` clauses evaluate in reverse nesting order (innermost first)

---

## 7. Progress and Preservation

### 7.1 Progress

**Theorem (Progress)**: If `∅; ∅ ⊢ e : T / ε` then either:
1. `e` is a value, or
2. `e ──► e'` for some `e'`, or
3. `e = E[perform E.op(v)]` and `E ∉ ε` (unhandled effect — ruled out by typing)

### 7.2 Preservation

**Theorem (Preservation)**: If `Γ; Δ ⊢ e : T / ε` and `e ──► e'`, then `Γ; Δ' ⊢ e' : T / ε'` where `ε' ⊆ ε` and `Δ' ⊑ Δ`.

### 7.3 Generation Safety

**Theorem (Generation Safety)**: If `Γ; Δ ⊢ e : T / ε` and `e ──►* e'` without `StaleReference` effects, then all memory accesses in the reduction used valid generations.

**Theorem (Stale Detection)**: If a continuation is resumed and any captured generational reference has an outdated generation, the `StaleReference.stale` operation is performed.

---

## 8. Linear Types and Effects Interaction

### 8.1 Linear Capture Restriction

**Theorem (Linear Capture)**: If a handler operation clause uses `resume` more than once (multi-shot), then no linear values from the captured context may be accessed.

**Formal Statement**:
Let `h` be a handler where operation `op` has clause `e_op`.
If `resume` appears in `e_op` under a `map`, `fold`, or other iteration, then:
```
∀x ∈ FV(resume) ∩ CapturedContext. Γ(x) ≠ linear T
```

### 8.2 Effect Suspension and Linearity

**Rule**: At an effect operation `perform E.op(v)`, all linear values in scope must be:
1. Consumed before the `perform`, or
2. Passed as part of `v`, or
3. Explicitly `suspend`ed (transferred to continuation)

```
Γ; Δ ⊢ perform E.op(v) : T / ε
Δ must have no linear bindings unless transferred
```

### 8.3 Closure Capture in Handlers

When a closure is created inside a handler operation clause and captures `resume` or variables from the handler scope, the linearity rules from §5.7.4 interact with the handler multi-shot rules from §6.2:

```
-- Single-shot handler: closures may capture linear values by-value
If handler is single-shot (resume used exactly once):
    Closures in handler scope may capture linear values via [T-Closure-Move]
    The closure itself becomes linear (§5.7.4 [Linear-Closure])
    Since resume is single-shot, the closure is called at most once ✓

-- Multi-shot handler: closures CANNOT capture linear values
If handler is multi-shot (resume used more than once):
    No linear captures in handler scope (§6.2 multi-shot rule)
    This extends to closures: a closure in a multi-shot handler
    cannot capture linear values, because the closure itself
    could be duplicated through multiple resumes               [Linear-Handler-Closure]
```

**Composition theorem**: The closure linearity rules (§5.7.4) and the handler multi-shot rules (§6.2) compose safely. A linear closure in a single-shot handler is consumed exactly once. A closure in a multi-shot handler cannot capture linear values.

---

## 9. Metatheory Summary

| Property | Status | Mechanized Proof |
|----------|--------|------------------|
| Progress | ✅ Mechanized | `Progress.v` — all 13 expression cases |
| Preservation | ✅ Mechanized | `Preservation.v` — type-preserving substitution + effect subsumption |
| Effect Safety | ✅ Mechanized | `EffectSafety.v` — effect row containment (9 theorems) |
| Linear Safety | ✅ Mechanized | `LinearSafety.v` — linearity context splitting (4 main theorems) |
| Generation Safety | ✅ Mechanized | `GenerationSnapshots.v` — snapshot validation on resume (14 theorems) |

---

## 10. Composition Safety Analysis

Blood combines five major language innovations. This section analyzes their pairwise interactions and provides safety guarantees for their composition.

### 10.1 Innovation Interaction Matrix

| | Effects | Gen Refs | MVS | Content Addr | Multi-Dispatch |
|---|---|---|---|---|---|
| **Effects** | — | §10.2 | §10.3 | §10.4 | §10.5 |
| **Gen Refs** | | — | §10.6 | Orthogonal | Orthogonal |
| **MVS** | | | — | Orthogonal | Orthogonal |
| **Content Addr** | | | | — | §10.7 |
| **Multi-Dispatch** | | | | | — |

Cells marked "Orthogonal" indicate features that operate independently without semantic interaction.

### 10.2 Effects × Generational References

**Interaction**: Continuations captured by effect handlers may hold generational references that become stale before resume.

**Safety Mechanism**: Generation Snapshots (see §4)

**Theorem (Effects-Gen Safety)**:
If `Γ; Δ ⊢ e : T / ε` and `e` is well-typed with generation snapshot semantics, then:
1. Any `resume(κ, v)` either succeeds with valid references, or
2. Raises `StaleReference` effect before any use-after-free occurs

**Proof Sketch**:
1. At continuation capture, record all generational references in scope as `GenSnapshot = {(addr₁, gen₁), ..., (addrₙ, genₙ)}`
2. At resume, for each `(addr, gen)` in snapshot:
   - Query current generation: `currentGen(addr)`
   - If `gen ≠ currentGen(addr)`, raise `StaleReference`
3. Only if all checks pass does execution continue
4. Therefore, no dereference can occur with stale generation

**Key Invariant**: `∀(a,g) ∈ snapshot. (g = currentGen(a)) ∨ StaleReference raised`

### 10.3 Effects × Mutable Value Semantics

**Interaction**: MVS performs implicit copies. Effect handlers may capture values that are semantically copied vs. referenced.

**Safety Mechanism**: Copy semantics are unambiguous for values; only `Box<T>` uses generational references.

**Theorem (Effects-MVS Safety)**:
Value types in effect handler captures behave correctly because they are copied, not aliased.

**Proof Sketch**:
1. MVS types have no identity—they are pure values
2. Copying a value creates an independent copy with no aliasing
3. Effect handlers capturing values get independent copies
4. No aliasing hazard exists for value types
5. Reference types (`Box<T>`) fall under Effects-Gen safety (§10.2)

### 10.4 Effects × Content-Addressed Code

**Interaction**: Hot-swap updates code while handlers may be active. Handlers reference code by hash.

**Safety Mechanism**: VFT atomic updates + hash stability

**Theorem (Effects-Content Safety)**:
Active effect handlers continue to reference the code version present at handler installation.

**Proof Sketch**:
1. Handler installation captures function references by hash
2. Hash is immutable identifier—never changes for same code
3. VFT update installs new hash → new entry point
4. Old hash → old entry point remains valid until GC
5. In-flight handlers complete with original code version
6. New handlers get new code version

**Corollary**: Hot-swap is safe during effect handling—no code version inconsistency.

### 10.5 Effects × Multiple Dispatch

**Interaction**: Effect operations may dispatch to different implementations based on argument types.

**Safety Mechanism**: Type stability enforcement

**Theorem (Effects-Dispatch Safety)**:
Effect operations with multiple dispatch are type-stable and deterministic.

**Proof Sketch**:
1. Type stability is checked at compile time (see DISPATCH.md §5)
2. Effect operations declare fixed type signatures in effect declarations
3. Handlers implement these fixed signatures
4. Dispatch resolution is deterministic for given types
5. Effect handling order is determined by handler nesting, not dispatch

### 10.6 Generational References × MVS

**Interaction**: MVS may perform copies that include generational references.

**Safety Mechanism**: Generation is copied with value; both copies share same generation expectation.

**Theorem (Gen-MVS Safety)**:
Copying a value containing generational references is safe.

**Proof Sketch**:
1. Copying value `v` containing `GenRef(addr, gen)` produces `v'` with same `GenRef(addr, gen)`
2. Both `v` and `v'` have valid references if `gen = currentGen(addr)`
3. If `gen ≠ currentGen(addr)`, both fail consistently
4. No use-after-free possible: either both work or both fail

### 10.7 Content-Addressed × Multiple Dispatch

**Interaction**: Multiple dispatch methods are stored by hash. Method families must be consistent across hot-swaps.

**Safety Mechanism**: Method family hashes include all specializations.

**Theorem (Content-Dispatch Safety)**:
Method dispatch remains consistent across hot-swaps.

**Proof Sketch**:
1. Method family is identified by family hash (hash of generic signature)
2. Specializations are registered under family hash
3. Hot-swap of one specialization updates that entry only
4. Type compatibility check ensures new specialization matches signature
5. Dispatch table remains consistent after atomic update

### 10.8 Composition Soundness Conjecture

**Conjecture (Full Composition Safety)**:
A Blood program that is:
- Well-typed with effect tracking
- Uses generational references correctly
- Follows MVS copy semantics
- Uses content-addressed code identity
- Has type-stable multiple dispatch

...cannot exhibit:
- Use-after-free (guaranteed by generational references)
- Unhandled effects (guaranteed by effect typing)
- Type confusion (guaranteed by type system)
- Code version inconsistency (guaranteed by content addressing)
- Dispatch ambiguity (guaranteed by type stability)

**Status**: ✅ Mechanized. `full_blood_safety` in CompositionSafety.v proves the conjunction of type soundness, effect safety, linear safety, generation safety, and composition guarantee. See `proofs/PROOF_ROADMAP.md` for full theorem inventory.

### 10.9 Formalized Composition Proofs

This section provides more rigorous proofs for the critical composition theorems.

#### 10.9.1 Effects × Generational References: Complete Proof

**Theorem 10.2.1 (Effects-Gen Composition Safety)**:
Let `e` be a well-typed Blood program with `∅; ∅ ⊢ e : T / ε`.
If during evaluation of `e`:
- A continuation `κ` is captured with snapshot `Γ_gen` in memory state `M₀`
- Evaluation continues, transforming memory to state `M₁`
- `resume(κ, v)` is invoked in memory state `M₁`

Then one of the following holds:
1. `∀(a,g) ∈ Γ_gen. M₁(a).gen = g` and evaluation of `κ(v)` proceeds safely, OR
2. `∃(a,g) ∈ Γ_gen. M₁(a).gen ≠ g` and `StaleReference.stale` effect is raised

**Proof**:

*Lemma A (Well-typed references are valid)*:
If `Γ; Δ ⊢ e : T / ε` and `e` contains generational reference `!a^g`, then in any memory state `M` reachable during evaluation of `e`, either:
- `M(a).gen = g` (reference still valid), or
- Evaluation has trapped or raised `StaleReference`

*Proof of Lemma A*:
By induction on the derivation of `Γ; Δ ⊢ e : T / ε` and the reduction sequence.

Base case: Initially, all references in the program are valid by the allocation invariant (alloc returns `(a, 0)` where `M(a).gen = 0`).

Inductive case: The only operation that changes `M(a).gen` is `free`. When `free(!a^g)` is called:
- If `M(a).gen = g`: gen is incremented to `g+1`, invalidating references with gen `g`
- If `M(a).gen ≠ g`: free traps (double-free detection)

Any subsequent deref of `!a^g` after free will find `M(a).gen = g+1 ≠ g` and trap. ∎

*Lemma B (Snapshot captures all reachable references)*:
When continuation `κ` is captured at `perform E.op(v)`, the snapshot `Γ_gen` contains all generational references that may be dereferenced during execution of `κ(w)` for any `w`.

*Proof of Lemma B*:
By construction of `CAPTURE_SNAPSHOT` (§13.2), we collect all generational references syntactically present in the delimited context. Since Blood has no hidden state (all references must be syntactically present), this set is complete. ∎

*Main Proof*:

Case 1: `∀(a,g) ∈ Γ_gen. M₁(a).gen = g`
  By the Resume Rule (Safe), `resume((κ, Γ_gen), v, M₁) ──► κ(v), M₁`.
  By Lemma B, all references that will be dereferenced are in `Γ_gen`.
  By the case assumption, all these references are valid in `M₁`.
  By Lemma A, execution of `κ(v)` proceeds without use-after-free. ∎

Case 2: `∃(a,g) ∈ Γ_gen. M₁(a).gen ≠ g`
  By the Resume Rule (Stale), before any code in `κ` executes:
  `resume((κ, Γ_gen), v, M₁) ──► perform StaleReference.stale(a, g, M₁(a).gen), M₁`
  No dereference of any reference in `κ` occurs. ∎

#### 10.9.2 Effects × Linear Types: Complete Proof

**Theorem 10.9.2 (Effects-Linear Composition Safety)**:
In a well-typed Blood program, linear values captured by effect handlers are never duplicated.

**Proof**:
Following the approach of "Soundly Handling Linearity" (Tang et al., POPL 2024):

*Step 1: Control-flow linearity classification*
Each effect operation is classified as either:
- `cf_linear`: continuation must be resumed exactly once
- `cf_unlimited`: continuation may be resumed any number of times

*Step 2: Typing rule enforcement*
The typing rule for handler clauses (§6.2) requires:
```
If handler is multi-shot (cf_unlimited):
  ∀i. LinearVars(FV(e_opᵢ) ∩ Γ) = ∅
```

This prevents capturing linear values in multi-shot handler clauses.

*Step 3: Single-shot preservation*
For `cf_linear` operations, the continuation is consumed linearly:
- It appears exactly once in the handler clause
- The typing context treats it as a linear binding
- Standard linear type rules prevent duplication

*Step 4: Conclusion*
No execution path can duplicate a linear value:
- Multi-shot handlers cannot capture linear values (Step 2)
- Single-shot handlers use continuation exactly once (Step 3)
- Therefore, linear values maintain uniqueness invariant. ∎

#### 10.9.3 Full Composition Theorem

**Theorem 10.9.3 (Full Composition Safety)**:
Let `e` be a Blood program. If `∅; ∅ ⊢ e : T / ε` (closed, well-typed), then during any finite reduction sequence `e ──►* e'`:

1. **No use-after-free**: Every `deref(!a^g)` either succeeds with valid data or raises `StaleReference`
2. **No unhandled effects**: Every `perform E.op(v)` is eventually handled or propagates to a declared effect row
3. **No type confusion**: Every subexpression maintains its declared type
4. **No linear duplication**: Linear values are used exactly once
5. **No dispatch ambiguity**: Every method call resolves to a unique method

**Proof Sketch**:
By simultaneous induction on the reduction sequence, using:
- Property 1: Theorems 10.2.1 and 11.5 (Generation Safety)
- Property 2: Theorem 11.3 (Effect Safety)
- Property 3: Theorem 11.2 (Preservation)
- Property 4: Theorem 10.9.2 and §8.1 (Linear Capture Restriction)
- Property 5: DISPATCH.md §5 (Ambiguity is compile-time error)

Each property is preserved by every reduction step. Properties 1 and 4 have explicit runtime checks (snapshot validation, linear tracking). Properties 2, 3, and 5 are enforced statically with no runtime overhead. ∎

### 10.10 Known Limitations

1. **Cycle collection + effects**: Concurrent cycle collection during effect handling requires careful synchronization. See MEMORY_MODEL.md §8.5.3.

2. **Hot-swap + in-flight state**: If handler state references data that changes during hot-swap, application-level migration is required.

3. **Cross-fiber generational references**: References crossing fiber boundaries require region isolation checks. See CONCURRENCY.md §4.

---

## 11. Proof Sketches for Core Theorems

### 11.1 Progress Theorem

**Statement**: If `∅; ∅ ⊢ e : T / ε` and `e` is not a value, then either:
1. `e ──► e'` for some `e'`, or
2. `e = E[perform op(v)]` for some `E`, `op`, `v` and `op` is in effect row `ε`

**Proof Sketch**:
By structural induction on the derivation of `∅; ∅ ⊢ e : T / ε`.

*Case* `e = x`:
- Empty context, so `x` cannot be typed. Contradiction.

*Case* `e = v` (value):
- Excluded by hypothesis.

*Case* `e = e₁ e₂`:
- By IH on `e₁`: either `e₁` steps, or `e₁ = v₁` (function value), or `e₁` performs.
- If `e₁` steps: `e₁ e₂ ──► e₁' e₂` by context rule.
- If `e₁ = v₁` and `e₂` steps: `v₁ e₂ ──► v₁ e₂'`.
- If `e₁ = v₁ = λx.e'` and `e₂ = v₂`: `(λx.e') v₂ ──► e'[v₂/x]` by β-App.
- If either performs: effect propagates through context.

*Case* `e = with h handle e'`:
- By IH on `e'`: either steps, is value, or performs.
- If `e'` steps: `with h handle e' ──► with h handle e''`.
- If `e' = v`: `with h handle v ──► h.return(v)` by Handle-Return.
- If `e' = D[perform op(v)]`:
  - If `op` handled by `h`: steps by Handle-Op-Deep/Shallow.
  - Otherwise: propagates by delimited context.

*Case* `e = perform op(v)`:
- Effect `op` must be in `ε` by T-Perform.
- Case 2 applies: `e = □[perform op(v)]` where `□` is empty context.

Remaining cases follow similar structure. ∎

### 11.2 Preservation Theorem

**Statement**: If `Γ; Δ ⊢ e : T / ε` and `e ──► e'`, then `Γ; Δ ⊢ e' : T / ε'` where `ε' ⊆ ε`.

**Proof Sketch**:
By induction on the derivation of `e ──► e'`.

*Case* β-App: `(λx:S. e) v ──► e[v/x]`
- By T-Lam: `Γ, x:S ⊢ e : T / ε`
- By T-App: `Γ ⊢ v : S`
- By Substitution Lemma: `Γ ⊢ e[v/x] : T / ε` ∎

*Case* Handle-Return: `with h handle v ──► h.return(v)`
- By T-Handle: `Γ ⊢ v : T` and `h.return : T → U`
- Result has type `U` with effects from handler implementation.

*Case* Handle-Op-Deep: `with h handle D[perform op(v)] ──► e_op[v/x, κ/resume]`
- Continuation `κ = λy. with h handle D[y]`
- By T-Handle: continuation has appropriate type
- Handler clause `e_op` typed correctly by handler typing rules.

Effect subsumption maintained because handling removes effect from row. ∎

### 11.3 Effect Safety Theorem

**Statement**: If `∅; ∅ ⊢ e : T / ∅` (pure program), then `e` cannot perform any unhandled effect.

**Proof Sketch**:
1. By T-Perform, `perform op(v)` requires `op ∈ ε` in the typing context.
2. For `ε = ∅` (pure), no effects are in scope.
3. Therefore, `perform op(v)` cannot type-check.
4. A well-typed pure program contains no `perform` expressions.
5. By Progress, the program either steps or is a value—no effects. ∎

### 11.4 Linear Safety Theorem

**Statement**: In a well-typed program, no linear value is used more than once.

**Proof Sketch**:
1. Linearity context `Δ` tracks linear bindings with multiplicity.
2. T-Linear-Use requires `x ∈ FV(e)` for linear `x`.
3. Context splitting `Δ = Δ₁ ⊗ Δ₂` partitions linear bindings.
4. Each linear binding appears in exactly one sub-derivation.
5. Multi-shot handlers are checked at compile time (Theorem 8.1).
6. Linear values cannot appear in multi-shot continuation captures.
7. Therefore, linear values are used exactly once. ∎

### 11.5 Generation Safety Theorem

**Statement**: No generational reference dereference accesses freed memory.

**Proof Sketch**:
1. Every allocation assigns generation `g` to address `a`.
2. Pointer stores `(a, g)` pair.
3. Deallocation increments generation to `g+1`.
4. Dereference compares pointer's `g` with current `currentGen(a)`.
5. If `g ≠ currentGen(a)`, `StaleReference` raised before access.
6. If `g = currentGen(a)`, memory has not been freed.
7. With Generation Snapshots, continuation resume validates all captured refs.
8. Therefore, no freed memory is accessed. ∎

---

## 12. Mechanization Roadmap

**Section Status**: Complete
**Last Updated**: 2026-03-04

Blood's formal semantics have been mechanized in Coq (Rocq). The proof suite validates
the safety architecture of Blood's feature composition. See `proofs/PROOF_ROADMAP.md`
for the authoritative theorem inventory, dependency graph, and model fidelity analysis.

### 12.0 Current Mechanization Status

22 Coq/Rocq proof files exist in `proofs/theories/` (10,507 lines, 0 Admitted):

| File | Lines | Purpose | Status |
|------|-------|---------|--------|
| `Syntax.v` | 486 | AST definitions | ✅ Complete |
| `Typing.v` | 398 | Typing rules (13 constructors incl. T_Extend, T_Resume) | ✅ Complete |
| `Substitution.v` | 1,053 | Substitution lemmas (23 Qed) | ✅ Complete |
| `ShiftSubst.v` | 335 | Shift-substitution commutation | ✅ Complete |
| `Semantics.v` | 413 | Operational semantics (0 Parameters) | ✅ Complete |
| `EffectAlgebra.v` | 148 | Effect row algebra | ✅ Complete |
| `ContextTyping.v` | 876 | Evaluation context typing | ✅ Complete |
| `Inversion.v` | 673 | Typing inversion (25 lemmas) | ✅ Complete |
| `Progress.v` | 536 | Progress theorem (all 13 cases) | ✅ Complete |
| `Preservation.v` | 371 | Preservation theorem (all 13 cases) | ✅ Complete |
| `Soundness.v` | 203 | Type soundness + composition | ✅ Complete |
| `EffectSafety.v` | 261 | Effect handler safety (9 theorems) | ✅ Complete |
| `GenerationSnapshots.v` | 508 | Generation snapshot safety (14 theorems) | ✅ Complete |
| `LinearTyping.v` | 496 | Strengthened typing with linearity | ✅ Complete |
| `LinearSafety.v` | 843 | Linear/affine safety (4 main theorems) | ✅ Complete |
| `Dispatch.v` | 289 | Multiple dispatch + type stability | ✅ Complete |
| `Regions.v` | 316 | Region safety via generations | ✅ Complete |
| `FiberSafety.v` | 412 | Tier-based concurrency safety | ✅ Complete |
| `ValueSemantics.v` | 410 | Mutable value semantics | ✅ Complete |
| `EffectSubsumption.v` | 432 | Effects subsume control flow patterns | ✅ Complete |
| `MemorySafety.v` | 372 | Memory safety without GC | ✅ Complete |
| `CompositionSafety.v` | 676 | Full composition safety + instantiations | ✅ Complete |

Build infrastructure: `_CoqProject` + `Makefile.coq` in `proofs/`.

| Phase | Name | Status | Notes |
|-------|------|--------|-------|
| M1 | Core Type System | ✅ Complete | Syntax, Typing, Substitution, Semantics |
| M2 | Effect Handlers | ✅ Complete | EffectSafety.v (9 theorems) |
| M3 | Linearity | ✅ Complete | LinearTyping.v + LinearSafety.v (two-judgment design) |
| M4 | Generational References | ✅ Complete | GenerationSnapshots.v (all 14 theorems, 0 Admitted) |
| M5 | Regions × Generations | ✅ Complete | Regions.v (4 theorems) |
| M6 | Dispatch × Type Stability | ✅ Complete | Dispatch.v (4 theorems) |
| M7 | MVS × Linearity | ✅ Complete | ValueSemantics.v (8 theorems) |
| M8 | Effects Subsume Patterns | ✅ Complete | EffectSubsumption.v (7 theorems) |
| M9 | Memory Safety Without GC | ✅ Complete | MemorySafety.v (8 theorems) |
| M10 | Tier Concurrency Safety | ✅ Complete | FiberSafety.v (8 theorems) |
| M11 | Full Composition Safety | ✅ Complete | CompositionSafety.v (master theorem + instantiations) |

**Totals**: 22 files, 10,507 lines, 227 Qed, 7 Defined, 0 Admitted, 0 Parameters, 0 Axioms.

**Approach**: Direct Coq without ITrees or Iris. The mechanization validates Blood's safety
architecture (generation + tier + effect + linearity composition) for the monomorphic
fragment. ITrees/Iris remain viable for future extensions requiring executable extraction
or concurrent separation logic (e.g., cycle collection proofs).

### 12.1 Choice of Proof Assistant

*Historical note: The original plan recommended a two-track approach. The actual mechanization used direct Coq without ITrees or Iris (see §12.2). The table below records the original evaluation.*

The original evaluation considered a **two-track approach**:

| Track | Tool | Purpose | Rationale |
|-------|------|---------|-----------|
| **Primary** | Coq + ITrees | Executable semantics | ITrees support recursive/impure programs; strong ecosystem |
| **Secondary** | Cubical Agda | Equational reasoning | Quotient types for effect laws; computational proofs |

#### Recommended Libraries

**Coq Ecosystem:**
- [Interaction Trees (ITrees)](https://github.com/DeepSpec/InteractionTrees) — Core effect representation
- [Iris](https://iris-project.org/) — Separation logic for concurrent reasoning
- [coq-hazel](https://github.com/ovanr/affect) — Effect handler formalization (from Affect paper)
- [Monae](https://github.com/affeldt-aist/monae) — Monadic equational reasoning

**Agda Ecosystem:**
- [Cubical Agda](https://agda.readthedocs.io/en/latest/language/cubical.html) — Quotient types, function extensionality
- [agda-stdlib](https://github.com/agda/agda-stdlib) — Standard library
- [Free Algebras Library](https://yangzhixuan.github.io/pdf/free.pdf) — From POPL 2024 paper

### 12.2 Mechanization Summary

All 11 phases are complete. The mechanization uses direct Coq (not ITrees or Iris), which
provides clean structural proofs but does not support executable extraction. Key design
decisions:

- **Two-judgment linearity** (LinearTyping.v): Separate `has_type_lin` judgment avoids
  modifying existing `has_type` rules, preventing cascading breakage across proof files.
  Bridge lemma `has_type_lin_to_has_type` connects the two systems.
- **Section-based parameterization** (Dispatch.v, FiberSafety.v): Subtype relation and
  fiber ownership model parameterized via Coq Sections, instantiated concretely in
  CompositionSafety.v.
- **Monomorphic fragment**: The formalization covers Blood's type system without generics
  or row polymorphism. Polymorphic safety is argued by standard parametricity, not
  mechanized.

**Future extensions** where ITrees/Iris would add value:
- Executable extraction (runnable reference interpreter from Coq)
- Concurrent separation logic for cycle collection and RC concurrency proofs
- Effect equation verification via quotient types (Cubical Agda)

See `proofs/PROOF_ROADMAP.md` for the complete theorem inventory, dependency graph,
model fidelity analysis, and file inventory.

### 12.3 Future Formalization Approaches

For extending the mechanization beyond the current monomorphic fragment, these approaches from recent literature are relevant:

| Mechanism | Approach | Reference |
|-----------|----------|-----------|
| Effects | Interaction Trees (ITrees) | [Xia et al. POPL 2020](https://dl.acm.org/doi/10.1145/3371119) |
| Effect equations | Quotient types in Cubical Agda | [Yang et al. POPL 2024](https://dl.acm.org/doi/10.1145/3632898) |
| Linearity | Control-flow linearity | [Tang et al. POPL 2024](https://dl.acm.org/doi/10.1145/3632896) |
| Affine types | Affect system | [van Rooij & Krebbers POPL 2025](https://dl.acm.org/doi/10.1145/3704841) |
| Memory model | Iris separation logic | [Iris Project](https://iris-project.org/) |
| Program logic | Effect-generic Hoare logic | [Yang et al. POPL 2024](https://dl.acm.org/doi/10.1145/3632898) |

### 12.4 Mechanization Scope

The current mechanization covers Blood's safety architecture for the monomorphic fragment.
The following are **not** formalized and remain as potential future extensions:

| Feature | Why Not Formalized | Future Approach |
|---------|-------------------|-----------------|
| Polymorphism (generics, row variables) | Would require rewriting entire proof suite | Standard parametricity argument; or ITrees with polymorphic effects |
| Executable extraction | Current approach uses Prop-based proofs | ITrees would enable extraction to OCaml/Haskell interpreter |
| Concurrent cycle collection | Requires concurrent separation logic | Iris/Coq |
| Content-addressing determinism | Implementation property, not type safety | Property-based testing or Verus |
| Fiber scheduler correctness | Runtime implementation detail | Model checking or Iris |

---

## 13. Complete Generation Snapshots Proof

This section provides the complete formal proof for the Generation Snapshots mechanism, which ensures safe interaction between algebraic effects and generational references.

### 13.1 Formal Setup

**Definitions**:

```
Address      = ℕ
Generation   = ℕ
Value        = ... (standard value domain)

Cell         = { value: Option<Value>, gen: Generation }
Memory       = Address → Cell
GenRef       = (Address, Generation)
Snapshot     = ℘(GenRef)  -- finite set of gen-refs
Continuation = (Expr → Expr, Snapshot)  -- continuation with snapshot
```

**Memory Operations**:

```
alloc(M, v) :
  a ← fresh(M)
  M' ← M[a ↦ { value: Some(v), gen: 0 }]
  return (M', (a, 0))

deref(M, (a, g)) :
  let { value: ov, gen: g' } = M(a)
  if g ≠ g' then TRAP("use-after-free")
  if ov = None then TRAP("uninitialized")
  return ov.unwrap()

free(M, (a, g)) :
  let { gen: g' } = M(a)
  if g ≠ g' then TRAP("double-free")
  M' ← M[a ↦ { value: None, gen: g' + 1 }]
  return M'
```

### 13.2 Snapshot Capture

**Definition (Snapshot Capture)**:
When an effect operation `perform E.op(v)` is evaluated within a handler, the continuation `κ` is captured along with a snapshot `Γ_gen`:

```
Γ_gen = { (a, g) | (a, g) appears in κ or in values reachable from κ }
```

**Capture Algorithm**:

```
CAPTURE_SNAPSHOT(ctx: DelimitedContext, M: Memory) → Snapshot:
  refs ← ∅

  // Collect all gen-refs in the context
  for each subterm t in ctx:
    for each gen_ref (a, g) in t:
      refs ← refs ∪ {(a, g)}

  // Validate current generations match
  for (a, g) in refs:
    assert M(a).gen = g  // Invariant: refs are valid at capture time

  return refs
```

### 13.3 Snapshot Validation

**Definition (Snapshot Validity)**:
A snapshot `Γ_gen` is valid with respect to memory `M` iff:

```
Valid(Γ_gen, M) ≡ ∀(a, g) ∈ Γ_gen. M(a).gen = g
```

**Validation Algorithm**:

```
VALIDATE_SNAPSHOT(snap: Snapshot, M: Memory) → Result<(), StaleRef>:
  for (a, g) in snap:
    let { gen: g' } = M(a)
    if g ≠ g':
      return Err(StaleRef { addr: a, expected: g, actual: g' })
  return Ok(())
```

### 13.4 Resume Semantics

**Resume Rule (Safe)**:
```
          Valid(Γ_gen, M)
  ─────────────────────────────────────
  resume((κ, Γ_gen), v, M) ──► κ(v), M
```

**Resume Rule (Stale)**:
```
      ¬Valid(Γ_gen, M)    (a, g) ∈ Γ_gen    M(a).gen = g' ≠ g
  ────────────────────────────────────────────────────────────────
  resume((κ, Γ_gen), v, M) ──► perform StaleReference.stale(a, g, g'), M
```

### 13.5 Safety Theorem

**Theorem 13.1 (Generation Snapshot Safety)**:
For any well-typed program `e` with continuation capture and resume:

1. **Capture Validity**: At the moment of capture, `Valid(Γ_gen, M)` holds
2. **Detection Completeness**: If any reference becomes stale, `StaleReference` is raised
3. **No Use-After-Free**: If resume succeeds, all derefs in `κ` are safe

**Proof**:

*Part 1 (Capture Validity)*:
By construction of `CAPTURE_SNAPSHOT`, we only include references `(a, g)` that appear in the continuation. By the typing invariant, any reference `(a, g)` in a well-typed term satisfies `M(a).gen = g` (otherwise it would have already trapped). Therefore `Valid(Γ_gen, M)` at capture time. ∎

*Part 2 (Detection Completeness)*:
Assume continuation `(κ, Γ_gen)` was captured in memory state `M₀`, and we attempt `resume((κ, Γ_gen), v)` in memory state `M₁`.

Case 1: `Valid(Γ_gen, M₁)` holds.
  All references in `Γ_gen` still match current generations, so resume proceeds.

Case 2: `¬Valid(Γ_gen, M₁)`.
  There exists `(a, g) ∈ Γ_gen` with `M₁(a).gen = g' ≠ g`.
  By the Resume Rule (Stale), we immediately raise `StaleReference`.
  The continuation body `κ` is never executed, so no deref of stale reference occurs. ∎

*Part 3 (No Use-After-Free)*:
Assume `resume((κ, Γ_gen), v, M₁)` succeeds, i.e., `Valid(Γ_gen, M₁)` holds.
Consider any `deref(M₁, (a, g))` that occurs during evaluation of `κ(v)`.

Sub-case A: `(a, g) ∈ Γ_gen`.
  By `Valid(Γ_gen, M₁)`, we have `M₁(a).gen = g`, so deref succeeds.

Sub-case B: `(a, g) ∉ Γ_gen`.
  If `(a, g)` is not in the snapshot, it must be a new reference created after
  resume (by freshness of allocation). Such references have `M(a).gen = g`
  by construction of `alloc`.

In both cases, deref succeeds without use-after-free. ∎

### 13.6 Liveness Optimization

**Observation**: Not all references in the continuation will necessarily be dereferenced. We can optimize by tracking liveness:

```
CAPTURE_SNAPSHOT_OPTIMIZED(ctx: DelimitedContext, M: Memory) → Snapshot:
  // Only capture references that are definitely dereferenced
  live_refs ← LIVENESS_ANALYSIS(ctx)
  return { (a, g) | (a, g) ∈ live_refs }
```

**Trade-off**: Precise liveness reduces snapshot size but requires more sophisticated analysis. Conservative approach (capture all) is always sound.

### 13.7 Interaction with Multi-shot Handlers

For multi-shot handlers (where continuation may be called multiple times):

**Invariant**: Each invocation of the continuation must validate the snapshot independently.

```
// Multi-shot: continuation used twice
with handler {
  op get() {
    resume(state) + resume(state)  // Two invocations
  }
} handle { perform State.get() }
```

**Semantics**:
- First `resume` validates snapshot against current memory
- Memory may change between first and second resume
- Second `resume` re-validates snapshot against (potentially different) memory
- Either may succeed or raise `StaleReference` independently

This preserves safety: each resume path is independently validated.

---

## Appendix A: Notation Reference

### A.1 Formal Notation Symbols

| Symbol | Meaning |
|--------|---------|
| `Γ` | Type context |
| `Δ` | Linearity context |
| `ε` | Effect row |
| `ρ` | Row variable |
| `□` | Hole in evaluation context |
| `──►` | Small-step reduction |
| `──►*` | Multi-step reduction (reflexive-transitive closure) |
| `⊢` | Typing judgment |
| `<:` | Subtyping relation |
| `⊗` | Linearity context combination |
| `FV(e)` | Free variables in e |
| `!a^g` | Generational reference to address a, generation g |

### A.2 Surface Syntax to Formal Notation Mapping

> **See Also**: [GRAMMAR.md Notation Alignment](./GRAMMAR.md#notation-alignment) for the complete mapping table.

| Surface Syntax (Blood code) | Formal Notation | Example |
|-----------------------------|-----------------|---------|
| `/ {IO, Error<E>}` | `ε = {IO, Error<E>}` | Effect row annotation |
| `/ {IO \| ε}` | `ε = {IO \| ρ}` | Open effect row with row variable |
| `/ pure` | `ε = {}` | Empty effect row |
| `fn(T) -> U / ε` | `T → U / ε` | Function type with effect |
| `perform E.op(v)` | `perform op(v)` | Effect operation |
| `with H handle { e }` | `handle e with H` | Handler expression |

---

*Mechanized proofs complete — see §12 for details (22 Coq files, 10,507 lines, 227 Qed, 0 Admitted).*

---

## Appendix B: Related Work and Citations

### Foundational Research

Blood's formal semantics draws on established research in programming language theory:

#### Linear Types and Effect Handlers

1. **Tang, Hillerström, Lindley, Morris. "[Soundly Handling Linearity](https://dl.acm.org/doi/10.1145/3632896)." POPL 2024.**
   - Introduces "control-flow linearity" ensuring continuations respect resource linearity
   - Addresses the interaction between linear types and multi-shot effect handlers
   - Blood's Theorem 8.2 (Linear Safety) follows this approach
   - Fixed a "long-standing type-soundness bug" in the Links language
   - **Validation**: Blood adopts the cf_linear/cf_unlimited classification (§12.3)

2. **van Rooij, Krebbers. "[Affect: An Affine Type and Effect System](https://dl.acm.org/doi/10.1145/3704841)." POPL 2025.**
   - Demonstrates that multi-shot effects break reasoning rules with mutable references
   - Proposes affine types to track continuation usage
   - Blood's restriction on multi-shot handlers capturing linear values aligns with this work
   - Addresses: nested continuations, references storing continuations, generic polymorphic effectful functions
   - **Validation**: Coq formalization available at [github.com/ovanr/affect](https://github.com/ovanr/affect)

3. **Muhcu, Schuster, Steuwer, Brachthäuser. "Multiple Resumptions and Local Mutable State, Directly." ICFP 2025.**
   - Addresses direct-style effect handlers with mutable state
   - Relevant to Blood's interaction between effects and generational references
   - **Validation**: Confirms that careful handling of state + multi-shot is an active research area

#### Algebraic Effects

4. **Leijen. "[Type Directed Compilation of Row-Typed Algebraic Effects](https://dl.acm.org/doi/10.1145/3009837)." POPL 2017.**
   - Foundation for row-polymorphic effect types
   - Evidence-passing compilation strategy
   - Blood's effect row polymorphism follows this design
   - **Validation**: Koka v3.1.3 (2025) demonstrates production viability

5. **Hillerström, Lindley. "Shallow Effect Handlers." APLAS 2018.**
   - Distinguishes deep vs. shallow handlers
   - Blood supports both with deep as default

6. **Yang, Kidney, Wu. "[Algebraic Effects Meet Hoare Logic in Cubical Agda](https://dl.acm.org/doi/10.1145/3632898)." POPL 2024.**
   - Effect-generic Hoare logic for reasoning about effectful programs
   - Uses quotient types for algebraic effect laws
   - **Validation**: Blood's mechanization plan (§12) should adopt this approach

7. **Xia et al. "[Interaction Trees: Representing Recursive and Impure Programs in Coq](https://dl.acm.org/doi/10.1145/3371119)." POPL 2020.**
   - Coinductive representation of effectful programs
   - Foundation for Coq-based effect formalization
   - **Validation**: ITrees library actively maintained; used in §12 mechanization plan

8. **Stepanenko et al. "Context-Dependent Effects in Guarded Interaction Trees." ESOP 2025.**
   - Extends GITrees for context-dependent effects (call/cc, shift/reset)
   - Addresses compositionality challenges
   - **Validation**: Latest work on effect formalization in Coq/Iris

#### Generational References

9. **Verdi et al. "[Vale: Memory Safety Without Borrow Checking or Garbage Collection](https://vale.dev/memory-safe)."**
   - Source of generational reference technique
   - Every object has "current generation" integer incremented on free
   - Pointers store "remembered generation" for comparison
   - **Validation (2025)**: Vale v0.2 released; generational references fully implemented since 2021
   - **Note**: Vale roadmap shows v0.6.1 (Early 2025) focused on optimization/benchmarking

#### Content-Addressed Code

10. **Chiusano, Bjarnason. "[Unison: A New Approach to Programming](https://www.unison-lang.org/docs/the-big-idea/)."**
    - Content-addressed code identification via hash
    - Eliminates dependency versioning conflicts
    - Blood extends with BLAKE3-256 and hot-swap runtime
    - **Validation (2025)**: Unison 1.0 released; approach proven in production

#### Mutable Value Semantics

11. **Racordon et al. "[Implementation Strategies for Mutable Value Semantics](https://www.jot.fm/issues/issue_2022_02/article1.pdf)." Journal of Object Technology, 2022.**
    - Foundation for Hylo's (Val's) approach
    - Blood adapts MVS with explicit borrowing option
    - **Validation (2025)**: Hylo presented at ECOOP 2025 PLSS track

### Key Contributions

Blood's **Generation Snapshots** mechanism (Section 4) addresses a unique challenge: the interaction between:
- Algebraic effect handlers (continuation capture/resume)
- Generational memory safety (stale reference detection)

...has not been previously addressed in published research. This mechanism ensures use-after-free is detected even when continuations are resumed after the referenced memory has been reallocated.

### Proof Obligations

The following theorems have been mechanized in Coq:

| Theorem | Status | File |
|---------|--------|------|
| Progress | ✅ Mechanized | `Progress.v` (all 13 expression cases) |
| Preservation | ✅ Mechanized | `Preservation.v` (all 13 expression cases) |
| Effect Safety | ✅ Mechanized | `EffectSafety.v` (9 theorems) |
| Linear Safety | ✅ Mechanized | `LinearSafety.v` (4 main theorems, two-judgment design) |
| Generation Safety | ✅ Mechanized | `GenerationSnapshots.v` (14 theorems, 0 Admitted) |

### Citation Format

When referencing Blood's formal semantics:

```
Blood Programming Language Formal Semantics, v0.4.0.
Available at: [repository URL]
```
