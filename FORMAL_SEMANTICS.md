# Blood Formal Semantics

**Version**: 0.2.0-draft
**Status**: Active Development
**Last Updated**: 2026-01-09

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

### 10.9 Known Limitations

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

## Appendix A: Notation Reference

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
Blood Programming Language Formal Semantics, v0.2.0-draft.
Available at: [repository URL]
```
