# Blood Formal Semantics

**Version**: 0.1.0-draft
**Status**: In Development

This document provides the formal operational semantics for Blood, suitable for mechanized proof and compiler verification.

### Related Specifications

- [SPECIFICATION.md](./SPECIFICATION.md) — Core language specification
- [MEMORY_MODEL.md](./MEMORY_MODEL.md) — Generation snapshot semantics
- [DISPATCH.md](./DISPATCH.md) — Multiple dispatch typing rules
- [GRAMMAR.md](./GRAMMAR.md) — Surface syntax grammar
- [STDLIB.md](./STDLIB.md) — Standard effect and type definitions

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
```

### 1.2 Values

```
v ::= c                           -- Constants
    | λx:T. e                     -- Functions
    | {l₁ = v₁, ..., lₙ = vₙ}     -- Record values
    | (κ, Γ_gen, L)               -- Continuation (with snapshot)
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

---

## 9. Metatheory Summary

| Property | Status | Proof Sketch |
|----------|--------|--------------|
| Progress | Conjectured | Standard, with effect case analysis |
| Preservation | Conjectured | Type-preserving substitution + effect subsumption |
| Effect Safety | Conjectured | Effect row containment ensures handler exists |
| Linear Safety | Conjectured | Linearity context splitting prevents duplication |
| Generation Safety | Conjectured | Snapshot validation on resume |

---

## Appendix: Notation Reference

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

---

*This document is a work in progress. Mechanized proofs in Coq/Agda are planned.*

---

## Appendix B: Related Work and Citations

### Foundational Research

Blood's formal semantics draws on established research in programming language theory:

#### Linear Types and Effect Handlers

1. **Tang, Hillerström, Lindley, Morris. "Soundly Handling Linearity." POPL 2024.**
   - Introduces "control-flow linearity" ensuring continuations respect resource linearity
   - Addresses the interaction between linear types and multi-shot effect handlers
   - Blood's Theorem 8.2 (Linear Safety) follows this approach
   - Fixed a "long-standing type-soundness bug" in the Links language

2. **van Rooij, Krebbers. "Affect: An Affine Type and Effect System." POPL 2025.**
   - Demonstrates that multi-shot effects break reasoning rules with mutable references
   - Proposes affine types to track continuation usage
   - Blood's restriction on multi-shot handlers capturing linear values aligns with this work
   - Addresses: nested continuations, references storing continuations, generic polymorphic effectful functions

#### Algebraic Effects

3. **Leijen. "Type Directed Compilation of Row-Typed Algebraic Effects." POPL 2017.**
   - Foundation for row-polymorphic effect types
   - Evidence-passing compilation strategy
   - Blood's effect row polymorphism follows this design

4. **Hillerström, Lindley. "Shallow Effect Handlers." APLAS 2018.**
   - Distinguishes deep vs. shallow handlers
   - Blood supports both with deep as default

#### Generational References

5. **Verdi et al. "Vale: Memory Safety Without Borrow Checking or Garbage Collection."**
   - Source of generational reference technique
   - Every object has "current generation" integer incremented on free
   - Pointers store "remembered generation" for comparison

#### Content-Addressed Code

6. **Chiusano, Bjarnason. "Unison: A New Approach to Programming."**
   - Content-addressed code identification via hash
   - Eliminates dependency versioning conflicts
   - Blood extends with BLAKE3-256 and hot-swap runtime

#### Mutable Value Semantics

7. **Racordon et al. "Implementation Strategies for Mutable Value Semantics." Journal of Object Technology, 2022.**
   - Foundation for Hylo's (Val's) approach
   - Blood adapts MVS with explicit borrowing option

### Novel Contributions

Blood's **Generation Snapshots** mechanism (Section 4) represents novel work not found in prior literature. The interaction between:
- Algebraic effect handlers (continuation capture/resume)
- Generational memory safety (stale reference detection)

...has not been previously addressed in published research. This mechanism ensures use-after-free is detected even when continuations are resumed after the referenced memory has been reallocated.

### Proof Obligations

The following theorems require formal mechanized proofs:

| Theorem | Status | Recommended Approach |
|---------|--------|---------------------|
| Progress | Conjectured | Coq formalization following POPL 2024 |
| Preservation | Conjectured | Type-preserving substitution lemmas |
| Effect Safety | Conjectured | Effect row containment proof |
| Linear Safety | Conjectured | Follow "Soundly Handling Linearity" |
| Generation Safety | Novel | New proof required for snapshot mechanism |

### Citation Format

When referencing Blood's formal semantics:

```
Blood Programming Language Formal Semantics, v0.1.0-draft.
Available at: [repository URL]
```
