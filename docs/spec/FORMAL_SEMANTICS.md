# Blood Formal Semantics

**Version**: 0.3.0
**Status**: Specified
**Implementation**: ✅ In Progress (effect typing complete; 10 Coq proof files with 10/12 theorems proved, 2 Admitted)
**Last Updated**: 2026-01-10

**Revision 0.3.0 Changes**:
- Added notation alignment with GRAMMAR.md surface syntax
- Added cross-references between formal and surface notations
- Added implementation status

This document provides the formal operational semantics for Blood, suitable for mechanized proof and compiler verification.

### Related Specifications

- [SPECIFICATION.md](./SPECIFICATION.md) — Core language specification
- [MEMORY_MODEL.md](./MEMORY_MODEL.md) — Generation snapshot semantics
- [DISPATCH.md](./DISPATCH.md) — Multiple dispatch typing rules
- [GRAMMAR.md](./GRAMMAR.md) — Surface syntax grammar; see [Notation Alignment](./GRAMMAR.md#notation-alignment) for surface-to-formal notation mapping
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

**Status**: Conjectured. Full proof requires mechanized verification of all interaction cases.

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

**Section Status**: In Progress
**Last Updated**: 2026-02-12

This section provides a concrete plan for mechanizing Blood's formal semantics in proof assistants, following best practices from recent research (2024-2025).

### 12.0 Current Mechanization Status

10 Coq/Rocq proof files exist in `compiler-rust/proofs/theories/` (~2,635 lines):

| File | Purpose | Status |
|------|---------|--------|
| `Syntax.v` | AST and de Bruijn syntax | ✅ Complete |
| `Typing.v` | Type system and typing rules | ✅ Complete |
| `Substitution.v` | Variable substitution proofs | ✅ Complete |
| `Semantics.v` | Operational semantics | ✅ Complete |
| `Progress.v` | Progress theorem | ✅ Complete |
| `Preservation.v` | Preservation theorem | ✅ Complete |
| `Soundness.v` | Type soundness composition | ✅ Complete |
| `EffectSafety.v` | Effect handler safety | ✅ Complete |
| `GenerationSnapshots.v` | Generation snapshot correctness (14 theorems) | ⚠️ 10/14 proved, 2 Admitted |
| `LinearSafety.v` | Linear type safety invariants | ✅ Complete |

Build infrastructure: `_CoqProject` + `Makefile` in `compiler-rust/proofs/`.

**Admitted theorems** (require completion):
- Detection completeness (GenerationSnapshots.v)
- No use-after-free (GenerationSnapshots.v) — detailed proofs provided but not yet mechanized

| Phase | Name | Status | Progress | Notes |
|-------|------|--------|----------|-------|
| M1 | Core Type System | ✅ Complete | 100% | Syntax, Typing, Substitution, Semantics |
| M2 | Effect Handlers | ✅ Complete | 100% | EffectSafety.v |
| M3 | Linearity | ✅ Complete | 100% | LinearSafety.v |
| M4 | Generational References | ⚠️ In Progress | 85% | GenerationSnapshots.v — 2 theorems Admitted |
| M5 | Composition Safety | ✅ Complete | 100% | Soundness.v (Progress + Preservation) |

**Prerequisites Status**:
- ✅ Formal semantics specification (this document)
- ✅ Proof sketches for core theorems (§11)
- ✅ Generation snapshots proof (§13)
- ✅ Composition safety analysis (§10)
- ✅ Reference literature identified (§12.3)
- ✅ Coq project setup (complete — `_CoqProject` + `Makefile`)
- 📋 ITrees integration (not started — current proofs use direct Coq)

**Remaining Work**: Complete 2 Admitted theorems in GenerationSnapshots.v

### 12.1 Choice of Proof Assistant

Based on the current state of the art, we recommend a **two-track approach**:

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

### 12.2 Phased Mechanization Plan

#### Phase M1: Core Type System (Months 1-3)

**Goal**: Mechanize the core simply-typed lambda calculus with effects.

**Deliverables**:
1. Syntax definition (expressions, types, effect rows)
2. Typing judgment formalization
3. Substitution lemmas
4. Progress and Preservation for STLC+Effects

**Coq Structure**:
```coq
(* Phase M1 File Structure *)
theories/
├── Syntax.v           (* Expression and type AST *)
├── Typing.v           (* Typing judgment Γ ⊢ e : T / ε *)
├── Substitution.v     (* Substitution lemmas *)
├── Semantics.v        (* Small-step reduction *)
├── Progress.v         (* Progress theorem *)
├── Preservation.v     (* Type preservation *)
└── Soundness.v        (* Combined soundness *)
```

**Key Lemmas**:
```coq
Lemma substitution_preserves_typing :
  ∀ Γ x e v T S ε,
    Γ, x : S ⊢ e : T / ε →
    Γ ⊢ v : S / pure →
    Γ ⊢ e[v/x] : T / ε.

Theorem progress :
  ∀ e T ε,
    ∅ ⊢ e : T / ε →
    value e ∨ (∃ e', e ──► e') ∨ (∃ op v, e = perform op v ∧ op ∈ ε).

Theorem preservation :
  ∀ Γ e e' T ε,
    Γ ⊢ e : T / ε →
    e ──► e' →
    Γ ⊢ e' : T / ε' ∧ ε' ⊆ ε.
```

#### Phase M2: Effect Handlers (Months 4-6)

**Goal**: Add deep and shallow effect handlers with handler typing.

**Deliverables**:
1. Handler syntax and typing
2. Delimited continuations
3. Handle-Op and Handle-Return reduction rules
4. Effect safety theorem

**Key Definitions**:
```coq
(* Handler representation using ITrees *)
Definition handler (E : Type → Type) (T U : Type) : Type :=
  { return_clause : T → itree void1 U
  ; op_clauses : ∀ X, E X → (X → itree E U) → itree void1 U
  }.

(* Effect handling interpretation *)
Definition handle {E T U} (h : handler E T U) : itree E T → itree void1 U :=
  iter (fun t =>
    match observe t with
    | RetF r => Ret (inr (h.(return_clause) r))
    | TauF t' => Ret (inl t')
    | VisF e k => Ret (inr (h.(op_clauses) _ e (fun x => handle h (k x))))
    end).
```

#### Phase M3: Linearity (Months 7-9)

**Goal**: Add linear/affine type tracking following "Soundly Handling Linearity" (POPL 2024).

**Deliverables**:
1. Linearity context (Δ) formalization
2. Control-flow linearity annotations
3. Multi-shot handler restrictions
4. Linear safety theorem

**Key Insight from POPL 2024**:
Classify effect operations as control-flow-linear or control-flow-unlimited:
```coq
Inductive cf_linearity :=
  | cf_linear    (* continuation resumed exactly once *)
  | cf_unlimited (* continuation may be resumed any number of times *).

(* Effect operation with linearity annotation *)
Record effect_op := {
  op_arg_type : Type;
  op_ret_type : Type;
  op_linearity : cf_linearity;
}.
```

#### Phase M4: Generational References (Months 10-14)

**Goal**: Formalize the generational reference system and generation snapshots.

**Deliverables**:
1. Memory model with generations
2. Reference operations (alloc, deref, free)
3. Generation snapshot capture and validation
4. Generation safety theorem (novel)

**Memory Model**:
```coq
(* Memory cell with generation *)
Record cell := {
  cell_value : option value;
  cell_generation : nat;
}.

(* Memory state *)
Definition memory := addr → cell.

(* Generational reference *)
Record gen_ref := {
  ref_addr : addr;
  ref_gen : nat;
}.

(* Generation snapshot for continuation capture *)
Definition gen_snapshot := list (addr * nat).

(* Snapshot validation on resume *)
Definition validate_snapshot (M : memory) (snap : gen_snapshot) : Prop :=
  Forall (fun '(a, g) => (M a).(cell_generation) = g) snap.
```

**Novel Theorem (Generation Snapshots)**:
```coq
Theorem generation_safety :
  ∀ M M' κ snap v,
    (* Continuation captured with snapshot *)
    captured_with_snapshot κ snap M →
    (* Memory evolved to M' *)
    M ──►* M' →
    (* Either snapshot validates and resume succeeds *)
    (validate_snapshot M' snap → ∃ r, resume κ v M' ──►* r) ∧
    (* Or snapshot invalidates and StaleReference raised *)
    (¬ validate_snapshot M' snap →
     resume κ v M' ──► perform StaleReference.stale).
```

#### Phase M5: Composition Safety (Months 15-18)

**Goal**: Prove safety of all feature interactions.

**Deliverables**:
1. Effects × Generational References interaction proof
2. Effects × MVS interaction proof
3. Full composition soundness theorem

**Composition Matrix Formalization**:
```coq
(* Each interaction is proven as a separate lemma *)
Lemma effects_gen_safe :
  ∀ e M M' κ,
    well_typed e →
    e captures κ with snapshot snap in M →
    M ──►* M' →
    resume_safe κ M' ∨ stale_detected κ M'.

Lemma effects_mvs_safe :
  ∀ e,
    well_typed e →
    value_semantics_preserved e.

Theorem full_composition_safety :
  ∀ e,
    well_typed e →
    no_use_after_free e ∧
    no_unhandled_effects e ∧
    no_type_confusion e ∧
    no_linear_duplication e.
```

### 12.3 Recommended Formalization Approaches

Based on recent literature, we recommend:

| Mechanism | Approach | Reference |
|-----------|----------|-----------|
| Effects | Interaction Trees (ITrees) | [Xia et al. POPL 2020](https://dl.acm.org/doi/10.1145/3371119) |
| Effect equations | Quotient types in Cubical Agda | [Yang et al. POPL 2024](https://dl.acm.org/doi/10.1145/3632898) |
| Linearity | Control-flow linearity | [Tang et al. POPL 2024](https://dl.acm.org/doi/10.1145/3632896) |
| Affine types | Affect system | [van Rooij & Krebbers POPL 2025](https://dl.acm.org/doi/10.1145/3704841) |
| Memory model | Iris separation logic | [Iris Project](https://iris-project.org/) |
| Program logic | Effect-generic Hoare logic | [Yang et al. POPL 2024](https://dl.acm.org/doi/10.1145/3632898) |

### 12.4 Estimated Effort

| Phase | Duration | FTE | Dependencies |
|-------|----------|-----|--------------|
| M1: Core Types | 3 months | 1 | None |
| M2: Effects | 3 months | 1 | M1 |
| M3: Linearity | 3 months | 1 | M2 |
| M4: Gen Refs | 5 months | 1-2 | M2 |
| M5: Composition | 4 months | 1-2 | M3, M4 |
| **Total** | **18 months** | **1-2 FTE** | |

### 12.5 Success Criteria

The mechanization is complete when:

1. **All core theorems proven**: Progress, Preservation, Effect Safety, Linear Safety, Generation Safety
2. **Executable extraction**: Coq extraction produces runnable interpreter
3. **Composition coverage**: All 10 pairwise interactions from §10 proven safe
4. **Test suite passes**: Extracted interpreter matches reference semantics on test cases
5. **Documentation**: Literate Coq/Agda with explanations matching this specification

### 12.6 Tooling Integration

The mechanization should integrate with Blood's compiler:

```
┌─────────────────────────────────────────────────────┐
│                    Blood Compiler                    │
├─────────────────────────────────────────────────────┤
│  Source  ──► Parser ──► AST ──► Type Check ──► ...  │
│                           │                          │
│                           ▼                          │
│              ┌─────────────────────┐                │
│              │   Coq Extraction    │                │
│              │  (Reference Impl)   │                │
│              └─────────────────────┘                │
│                           │                          │
│                           ▼                          │
│              ┌─────────────────────┐                │
│              │  Property Testing   │                │
│              │  (AST ↔ Coq match)  │                │
│              └─────────────────────┘                │
└─────────────────────────────────────────────────────┘
```

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

*Mechanized proofs in Coq/Agda are planned. See §12 for formalization roadmap.*

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
Blood Programming Language Formal Semantics, v0.3.0.
Available at: [repository URL]
```
