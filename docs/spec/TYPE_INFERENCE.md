# Blood Type Inference Specification

**Version**: 0.1.0
**Status**: Specified
**Implementation**: `bloodc/src/typeck/unify.rs`, `bloodc/src/typeck/infer.rs`
**Last Updated**: 2026-01-14

This document specifies the type inference algorithm used in Blood, based on Hindley-Milner with extensions for effects, row polymorphism, and linear types.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Algorithm W](#2-algorithm-w)
3. [Unification Algorithm](#3-unification-algorithm)
4. [Generalization and Instantiation](#4-generalization-and-instantiation)
5. [Row Polymorphism Unification](#5-row-polymorphism-unification)
6. [Effect Row Inference](#6-effect-row-inference)
7. [Linear Type Inference](#7-linear-type-inference)
8. [Implementation Details](#8-implementation-details)
9. [References](#9-references)

---

## 1. Overview

### 1.1 Design Goals

Blood's type inference provides:

1. **Principal Types** - Infers the most general type without annotations
2. **Bidirectional Checking** - Combines synthesis and checking modes
3. **Effect Inference** - Automatically infers effect signatures
4. **Row Polymorphism** - Supports extensible records

### 1.2 Related Specifications

- [FORMAL_SEMANTICS.md](./FORMAL_SEMANTICS.md) - Typing rules
- [DISPATCH.md](./DISPATCH.md) - Multiple dispatch type resolution
- [SPECIFICATION.md](./SPECIFICATION.md) - Core language types

### 1.3 Core Concepts

| Concept | Description |
|---------|-------------|
| **Type Variable** | Unknown type `?T` to be determined |
| **Substitution** | Mapping from type variables to types |
| **Unification** | Finding substitution making two types equal |
| **Generalization** | Converting monotype to polytype |
| **Instantiation** | Converting polytype to monotype with fresh vars |

---

## 2. Algorithm W

### 2.1 Overview

Algorithm W is the standard Hindley-Milner inference algorithm. It processes expressions and produces both a type and a substitution.

```
W : (TypeEnv, Expr) → (Substitution, Type)
```

### 2.2 Pseudocode

```
ALGORITHM W(Γ, e):
    Input:
        Γ : TypeEnv    -- Type environment mapping variables to type schemes
        e : Expr       -- Expression to type
    Output:
        (S, τ)         -- Substitution and inferred type

    CASE e OF:

        -- Variable
        x:
            IF x ∉ dom(Γ) THEN
                ERROR "Unbound variable: " + x
            σ = Γ(x)
            τ = instantiate(σ)
            RETURN (∅, τ)

        -- Literal
        c:
            τ = typeOfLiteral(c)
            RETURN (∅, τ)

        -- Lambda abstraction: λx. e₁
        λx. e₁:
            α = freshTypeVar()
            Γ' = Γ, x : α
            (S₁, τ₁) = W(Γ', e₁)
            RETURN (S₁, S₁(α) → τ₁)

        -- Application: e₁ e₂
        e₁ e₂:
            (S₁, τ₁) = W(Γ, e₁)
            (S₂, τ₂) = W(S₁(Γ), e₂)
            α = freshTypeVar()
            S₃ = unify(S₂(τ₁), τ₂ → α)
            RETURN (S₃ ∘ S₂ ∘ S₁, S₃(α))

        -- Let binding: let x = e₁ in e₂
        let x = e₁ in e₂:
            (S₁, τ₁) = W(Γ, e₁)
            σ = generalize(S₁(Γ), τ₁)
            Γ' = S₁(Γ), x : σ
            (S₂, τ₂) = W(Γ', e₂)
            RETURN (S₂ ∘ S₁, τ₂)

        -- Type annotation: (e : τ)
        (e₁ : τ_ann):
            (S₁, τ₁) = W(Γ, e₁)
            S₂ = unify(τ₁, τ_ann)
            RETURN (S₂ ∘ S₁, S₂(τ_ann))

        -- If expression: if e₁ then e₂ else e₃
        if e₁ then e₂ else e₃:
            (S₁, τ₁) = W(Γ, e₁)
            S₂ = unify(τ₁, Bool)
            (S₃, τ₂) = W(S₂∘S₁(Γ), e₂)
            (S₄, τ₃) = W(S₃∘S₂∘S₁(Γ), e₃)
            S₅ = unify(S₄(τ₂), τ₃)
            RETURN (S₅ ∘ S₄ ∘ S₃ ∘ S₂ ∘ S₁, S₅(τ₃))

        -- Record literal: {l₁ = e₁, ..., lₙ = eₙ}
        {l₁ = e₁, ..., lₙ = eₙ}:
            S = ∅
            fields = []
            FOR i = 1 TO n:
                (Sᵢ, τᵢ) = W(S(Γ), eᵢ)
                S = Sᵢ ∘ S
                fields.append((lᵢ, τᵢ))
            RETURN (S, Record(fields, None))

        -- Field access: e.l
        e₁.l:
            (S₁, τ₁) = W(Γ, e₁)
            α = freshTypeVar()
            ρ = freshRowVar()
            S₂ = unify(τ₁, Record([(l, α)], Some(ρ)))
            RETURN (S₂ ∘ S₁, S₂(α))

        -- Effect operation: perform E.op(e)
        perform E.op(e₁):
            (S₁, τ₁) = W(Γ, e₁)
            (τ_param, τ_ret, ε_op) = lookupEffectOp(E, op)
            S₂ = unify(τ₁, τ_param)
            ε = addEffect(E, currentEffectRow())
            RETURN (S₂ ∘ S₁, τ_ret) with effect ε

        -- Match expression
        match e₁ { p₁ => e₂, ..., pₙ => eₙ₊₁ }:
            (S₁, τ_scrutinee) = W(Γ, e₁)
            S = S₁
            τ_result = freshTypeVar()
            FOR i = 1 TO n:
                (S', τ_pat, bindings) = inferPattern(pᵢ)
                S = S' ∘ S
                S_unify = unify(S(τ_scrutinee), S(τ_pat))
                S = S_unify ∘ S
                Γ' = S(Γ) ∪ bindings
                (Sᵢ, τᵢ) = W(Γ', eᵢ₊₁)
                S = Sᵢ ∘ S
                S_branch = unify(S(τ_result), τᵢ)
                S = S_branch ∘ S
            RETURN (S, S(τ_result))
```

### 2.3 Correctness Properties

**Theorem (Soundness)**: If `W(Γ, e) = (S, τ)` succeeds, then `S(Γ) ⊢ e : τ`.

**Theorem (Completeness)**: If `Γ ⊢ e : τ`, then `W(Γ, e)` succeeds with `(S, τ')` where `τ` is an instance of `τ'`.

**Theorem (Principal Types)**: The type returned by Algorithm W is principal (most general).

---

## 3. Unification Algorithm

### 3.1 Overview

Unification finds a substitution `S` such that `S(τ₁) = S(τ₂)`.

```
unify : (Type, Type) → Substitution
```

### 3.2 Pseudocode

```
ALGORITHM UNIFY(τ₁, τ₂):
    Input:
        τ₁, τ₂ : Type  -- Types to unify
    Output:
        S : Substitution  -- Unifying substitution
    Raises:
        TypeError if types cannot be unified

    -- First, resolve any existing substitutions
    τ₁ = resolve(τ₁)
    τ₂ = resolve(τ₂)

    CASE (τ₁, τ₂) OF:

        -- Same type variable
        (Var(α), Var(β)) WHERE α = β:
            RETURN ∅

        -- Type variable on left
        (Var(α), τ):
            IF occursIn(α, τ) THEN
                ERROR "Infinite type: " + α + " ~ " + τ
            RETURN {α ↦ τ}

        -- Type variable on right
        (τ, Var(α)):
            IF occursIn(α, τ) THEN
                ERROR "Infinite type: " + α + " ~ " + τ
            RETURN {α ↦ τ}

        -- Same primitive type
        (Primitive(p₁), Primitive(p₂)) WHERE p₁ = p₂:
            RETURN ∅

        -- Function types
        (Fn(params₁, ret₁), Fn(params₂, ret₂)):
            IF length(params₁) ≠ length(params₂) THEN
                ERROR "Arity mismatch"
            S = ∅
            FOR i = 1 TO length(params₁):
                S' = unify(S(params₁[i]), S(params₂[i]))
                S = S' ∘ S
            S' = unify(S(ret₁), S(ret₂))
            RETURN S' ∘ S

        -- Tuple types
        (Tuple(ts₁), Tuple(ts₂)):
            IF length(ts₁) ≠ length(ts₂) THEN
                ERROR "Tuple length mismatch"
            S = ∅
            FOR i = 1 TO length(ts₁):
                S' = unify(S(ts₁[i]), S(ts₂[i]))
                S = S' ∘ S
            RETURN S

        -- Array types
        (Array(elem₁, size₁), Array(elem₂, size₂)):
            IF size₁ ≠ size₂ THEN
                ERROR "Array size mismatch"
            RETURN unify(elem₁, elem₂)

        -- Reference types
        (Ref(inner₁, mut₁), Ref(inner₂, mut₂)):
            IF mut₁ ≠ mut₂ THEN
                ERROR "Mutability mismatch"
            RETURN unify(inner₁, inner₂)

        -- ADT (struct/enum) types
        (Adt(def₁, args₁), Adt(def₂, args₂)):
            IF def₁ ≠ def₂ THEN
                ERROR "Type constructor mismatch: " + def₁ + " vs " + def₂
            IF length(args₁) ≠ length(args₂) THEN
                ERROR "Type argument count mismatch"
            S = ∅
            FOR i = 1 TO length(args₁):
                S' = unify(S(args₁[i]), S(args₂[i]))
                S = S' ∘ S
            RETURN S

        -- Record types (row polymorphism)
        (Record(fields₁, row₁), Record(fields₂, row₂)):
            RETURN unifyRecords(fields₁, row₁, fields₂, row₂)

        -- Forall types (higher-rank)
        (Forall(params₁, body₁), Forall(params₂, body₂)):
            IF length(params₁) ≠ length(params₂) THEN
                ERROR "Quantifier count mismatch"
            -- Alpha-rename: instantiate both with same fresh vars
            fresh = [freshTypeVar() FOR _ IN params₁]
            body₁' = substitute(body₁, zip(params₁, fresh))
            body₂' = substitute(body₂, zip(params₂, fresh))
            RETURN unify(body₁', body₂')

        -- Forall on left only: instantiate
        (Forall(params, body), τ):
            fresh = [freshTypeVar() FOR _ IN params]
            body' = substitute(body, zip(params, fresh))
            RETURN unify(body', τ)

        -- Forall on right only: instantiate
        (τ, Forall(params, body)):
            fresh = [freshTypeVar() FOR _ IN params]
            body' = substitute(body, zip(params, fresh))
            RETURN unify(τ, body')

        -- Never type unifies with anything
        (Never, _) OR (_, Never):
            RETURN ∅

        -- Error type unifies with anything (error recovery)
        (Error, _) OR (_, Error):
            RETURN ∅

        -- Ownership types
        (Ownership(q₁, inner₁), Ownership(q₂, inner₂)) WHERE q₁ = q₂:
            RETURN unify(inner₁, inner₂)

        -- Linear → Affine coercion
        (Ownership(Affine, inner₁), Ownership(Linear, inner₂)):
            RETURN unify(inner₁, inner₂)

        -- No match
        _:
            ERROR "Type mismatch: " + τ₁ + " vs " + τ₂
```

### 3.3 Occurs Check

The occurs check prevents infinite types:

```
ALGORITHM OCCURS_IN(α, τ):
    τ = resolve(τ)

    CASE τ OF:
        Var(β):          RETURN α = β
        Primitive(_):    RETURN false
        Tuple(ts):       RETURN any(occursIn(α, t) FOR t IN ts)
        Array(elem, _):  RETURN occursIn(α, elem)
        Fn(params, ret): RETURN any(occursIn(α, p) FOR p IN params)
                                OR occursIn(α, ret)
        Adt(_, args):    RETURN any(occursIn(α, a) FOR a IN args)
        Record(fs, _):   RETURN any(occursIn(α, f.ty) FOR f IN fs)
        Forall(ps, body):
            IF α IN ps THEN RETURN false  -- Bound variable
            RETURN occursIn(α, body)
        _:               RETURN false
```

### 3.4 Resolution

Resolution follows substitution chains:

```
ALGORITHM RESOLVE(τ):
    CASE τ OF:
        Var(α):
            IF α IN substitutions THEN
                RETURN resolve(substitutions[α])
            RETURN τ
        _:
            RETURN τ
```

---

## 4. Generalization and Instantiation

### 4.1 Generalization

Generalization converts a monotype to a polytype by quantifying over free type variables:

```
ALGORITHM GENERALIZE(Γ, τ):
    Input:
        Γ : TypeEnv
        τ : Type (monotype)
    Output:
        σ : TypeScheme (polytype)

    freeInType = freeTypeVars(τ)
    freeInEnv = ⋃{freeTypeVars(σ) | (x, σ) ∈ Γ}
    varsToQuantify = freeInType - freeInEnv

    IF varsToQuantify = ∅ THEN
        RETURN τ
    ELSE
        RETURN ∀varsToQuantify. τ
```

### 4.2 Instantiation

Instantiation converts a polytype to a monotype with fresh type variables:

```
ALGORITHM INSTANTIATE(σ):
    Input:
        σ : TypeScheme
    Output:
        τ : Type (monotype with fresh vars)

    CASE σ OF:
        Forall(αs, τ):
            fresh = [freshTypeVar() FOR α IN αs]
            RETURN substitute(τ, zip(αs, fresh))
        τ:
            RETURN τ
```

### 4.3 Free Type Variables

```
ALGORITHM FREE_TYPE_VARS(τ):
    CASE τ OF:
        Var(α):          RETURN {α}
        Primitive(_):    RETURN ∅
        Tuple(ts):       RETURN ⋃{freeTypeVars(t) | t ∈ ts}
        Fn(params, ret): RETURN ⋃{freeTypeVars(p) | p ∈ params}
                                ∪ freeTypeVars(ret)
        Adt(_, args):    RETURN ⋃{freeTypeVars(a) | a ∈ args}
        Forall(αs, body): RETURN freeTypeVars(body) - set(αs)
        _:               RETURN ∅
```

---

## 5. Row Polymorphism Unification

### 5.1 Overview

Row polymorphism allows records with extra fields to unify:

```blood
fn get_x<R>(r: {x: i32 | R}) -> i32 { r.x }

get_x({x: 1, y: 2})  // OK: R = {y: i32}
```

### 5.2 Record Unification Algorithm

```
ALGORITHM UNIFY_RECORDS(fields₁, row₁, fields₂, row₂):
    Input:
        fields₁, fields₂ : List<(Label, Type)>
        row₁, row₂ : Option<RowVar>
    Output:
        S : Substitution

    -- Build label → type maps
    map₁ = {f.label: f.type FOR f IN fields₁}
    map₂ = {f.label: f.type FOR f IN fields₂}

    -- Unify common fields
    S = ∅
    commonLabels = keys(map₁) ∩ keys(map₂)
    FOR l IN commonLabels:
        S' = unify(S(map₁[l]), S(map₂[l]))
        S = S' ∘ S

    -- Find fields unique to each side
    onlyIn1 = [(l, map₁[l]) FOR l IN keys(map₁) - commonLabels]
    onlyIn2 = [(l, map₂[l]) FOR l IN keys(map₂) - commonLabels]

    CASE (row₁, row₂, isEmpty(onlyIn1), isEmpty(onlyIn2)) OF:

        -- Both closed, no extra fields: OK
        (None, None, true, true):
            RETURN S

        -- Both closed, extra fields: ERROR
        (None, None, _, _):
            ERROR "Record field mismatch"

        -- Record 1 is open: bind row var to record 2's extras
        (Some(ρ₁), None, true, _):
            rowSubst[ρ₁] = (onlyIn2, None)
            RETURN S

        (Some(ρ₁), None, false, _):
            ERROR "Closed record missing fields: " + onlyIn1

        -- Record 2 is open: bind row var to record 1's extras
        (None, Some(ρ₂), _, true):
            rowSubst[ρ₂] = (onlyIn1, None)
            RETURN S

        (None, Some(ρ₂), _, false):
            ERROR "Closed record missing fields: " + onlyIn2

        -- Both open: create fresh row var for remainder
        (Some(ρ₁), Some(ρ₂), _, _):
            combined = onlyIn1 ++ onlyIn2
            IF isEmpty(combined) AND ρ₁ = ρ₂ THEN
                RETURN S
            ELSE IF isEmpty(combined) THEN
                rowSubst[ρ₁] = ([], Some(ρ₂))
                RETURN S
            ELSE:
                ρ_fresh = freshRowVar()
                rowSubst[ρ₁] = (combined, Some(ρ_fresh))
                rowSubst[ρ₂] = (combined, Some(ρ_fresh))
                RETURN S
```

---

## 6. Effect Row Inference

### 6.1 Overview

Effect inference determines which effects a function may perform:

```blood
fn example() / {State<i32>, IO} {
    let x = get();      // State<i32>
    println(x);         // IO
}
```

### 6.2 Effect Inference Rules

```
ALGORITHM INFER_EFFECTS(Γ, e):
    Input:
        Γ : TypeEnv
        e : Expr
    Output:
        ε : EffectRow

    CASE e OF:
        -- Literal: pure
        c:
            RETURN pure

        -- Variable: pure
        x:
            RETURN pure

        -- Lambda: effects from body
        λx. e₁:
            RETURN pure  -- Lambda itself is pure; body effects are in signature

        -- Application: union of effects
        e₁ e₂:
            ε₁ = inferEffects(Γ, e₁)
            ε₂ = inferEffects(Γ, e₂)
            ε_fn = effectsOfFunctionType(typeOf(e₁))
            RETURN ε₁ ∪ ε₂ ∪ ε_fn

        -- Perform: adds the effect
        perform E.op(e₁):
            ε₁ = inferEffects(Γ, e₁)
            RETURN ε₁ ∪ {E}

        -- Handle: masks the handled effect
        with h handle e₁:
            ε₁ = inferEffects(Γ, e₁)
            E = effectHandledBy(h)
            RETURN ε₁ - {E}

        -- Let: union of effects
        let x = e₁ in e₂:
            ε₁ = inferEffects(Γ, e₁)
            ε₂ = inferEffects(Γ, e₂)
            RETURN ε₁ ∪ ε₂

        -- If: union of all branches
        if e₁ then e₂ else e₃:
            ε₁ = inferEffects(Γ, e₁)
            ε₂ = inferEffects(Γ, e₂)
            ε₃ = inferEffects(Γ, e₃)
            RETURN ε₁ ∪ ε₂ ∪ ε₃

        -- Match: union of scrutinee and all arms
        match e₁ { arms }:
            ε₁ = inferEffects(Γ, e₁)
            ε_arms = ⋃{inferEffects(Γ, arm.body) FOR arm IN arms}
            RETURN ε₁ ∪ ε_arms
```

### 6.3 Effect Row Unification

```
ALGORITHM UNIFY_EFFECT_ROWS(ε₁, ε₂):
    Input:
        ε₁, ε₂ : EffectRow  -- Each is {E₁, ..., Eₙ | ρ}
    Output:
        S : Substitution

    effects₁ = concreteEffects(ε₁)
    effects₂ = concreteEffects(ε₂)
    row₁ = rowVar(ε₁)
    row₂ = rowVar(ε₂)

    common = effects₁ ∩ effects₂
    onlyIn1 = effects₁ - common
    onlyIn2 = effects₂ - common

    -- Similar logic to record row unification
    CASE (row₁, row₂) OF:
        (None, None):
            IF onlyIn1 ≠ ∅ OR onlyIn2 ≠ ∅ THEN
                ERROR "Effect mismatch"
            RETURN ∅

        (Some(ρ₁), None):
            effectRowSubst[ρ₁] = (onlyIn2, None)
            RETURN ∅

        (None, Some(ρ₂)):
            effectRowSubst[ρ₂] = (onlyIn1, None)
            RETURN ∅

        (Some(ρ₁), Some(ρ₂)):
            ρ_fresh = freshEffectRowVar()
            effectRowSubst[ρ₁] = (onlyIn2, Some(ρ_fresh))
            effectRowSubst[ρ₂] = (onlyIn1, Some(ρ_fresh))
            RETURN ∅
```

---

## 7. Linear Type Inference

### 7.1 Overview

Linear types require exactly one use; affine types allow at most one use.

### 7.2 Linearity Context

```
Δ : Var → Usage

Usage = {
    Unrestricted,  -- Can use any number of times
    Linear,        -- Must use exactly once
    Affine,        -- Must use at most once
}
```

### 7.3 Linearity Checking

```
ALGORITHM CHECK_LINEARITY(Δ, e):
    Input:
        Δ : LinearityContext
        e : Expr
    Output:
        Δ' : LinearityContext (remaining uses)

    CASE e OF:
        x:
            IF Δ(x) = Linear THEN
                IF x already used THEN
                    ERROR "Linear variable used twice: " + x
                Δ' = Δ[x ↦ Used]
            RETURN Δ'

        λx. e₁:
            Δ' = checkLinearity(Δ ∪ {x : Unrestricted}, e₁)
            RETURN Δ' - {x}

        e₁ e₂:
            Δ₁ = checkLinearity(Δ, e₁)
            Δ₂ = checkLinearity(Δ₁, e₂)
            RETURN Δ₂

        let x = e₁ in e₂:
            Δ₁ = checkLinearity(Δ, e₁)
            usage = IF isLinear(typeOf(e₁)) THEN Linear
                    ELSE IF isAffine(typeOf(e₁)) THEN Affine
                    ELSE Unrestricted
            Δ₂ = checkLinearity(Δ₁ ∪ {x : usage}, e₂)
            IF usage = Linear AND x NOT used in e₂ THEN
                ERROR "Linear binding not used: " + x
            RETURN Δ₂ - {x}
```

---

## 8. Implementation Details

### 8.1 Data Structures

```rust
// From bloodc/src/typeck/unify.rs

pub struct Unifier {
    /// Type variable substitutions: TyVarId → Type
    substitutions: HashMap<TyVarId, Type>,

    /// Next type variable ID
    next_var: u32,

    /// Row variable substitutions for records
    row_substitutions: HashMap<RecordRowVarId, (Vec<RecordField>, Option<RecordRowVarId>)>,

    /// Next row variable ID
    next_row_var: u32,
}
```

### 8.2 Key Functions

| Function | Location | Purpose |
|----------|----------|---------|
| `Unifier::unify` | `unify.rs:101` | Main unification entry point |
| `Unifier::bind` | `unify.rs:444` | Bind type variable with occurs check |
| `Unifier::resolve` | `unify.rs:494` | Follow substitution chains |
| `Unifier::occurs_in` | `unify.rs:456` | Occurs check for infinite types |
| `unify_records` | `unify.rs:337` | Row polymorphism unification |

### 8.3 Complexity

| Operation | Time Complexity | Notes |
|-----------|-----------------|-------|
| `unify` | O(n) amortized | n = type size, with path compression |
| `resolve` | O(α(n)) | α = inverse Ackermann (nearly constant) |
| `occurs_in` | O(n) | n = type size |
| `generalize` | O(n + m) | n = type size, m = env size |

---

## 9. References

### 9.1 Primary Sources

1. **Hindley-Milner Type System**
   - [Wikipedia: Hindley-Milner](https://en.wikipedia.org/wiki/Hindley–Milner_type_system)
   - Original papers: Hindley (1969), Milner (1978), Damas & Milner (1982)

2. **Algorithm W**
   - Damas, L., & Milner, R. (1982). "Principal type-schemes for functional programs"
   - [PDF](https://steshaw.org/hm/hindley-milner.pdf)

3. **Row Polymorphism**
   - Wand, M. (1987). "Complete Type Inference for Simple Objects"
   - Rémy, D. (1989). "Type Inference for Records in a Natural Extension of ML"

4. **Effect Systems**
   - [Generalized Evidence Passing for Effect Handlers](https://dl.acm.org/doi/10.1145/3473576) (ICFP'21)
   - [Koka Language](https://koka-lang.github.io/koka/doc/book.html)

### 9.2 Implementation References

- [Write You a Haskell](http://dev.stephendiehl.com/fun/006_hindley_milner.html) - Stephen Diehl
- [prakhar1989/type-inference](https://github.com/prakhar1989/type-inference) - OCaml implementation
- [Affine Types with HM Inference](https://arxiv.org/abs/2203.17125) - arXiv:2203.17125

---

*Last updated: 2026-01-14*
