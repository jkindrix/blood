# Blood Multiple Dispatch Specification

**Version**: 0.2.0-draft
**Status**: Active Development
**Last Updated**: 2026-01-09

This document specifies Blood's multiple dispatch system, including method resolution, type stability enforcement, and ambiguity detection.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Method Declaration](#2-method-declaration)
3. [Dispatch Resolution Algorithm](#3-dispatch-resolution-algorithm)
4. [Type Stability](#4-type-stability)
   - 4.1 [Definition](#41-definition)
   - 4.2 [Why Type Stability Matters](#42-why-type-stability-matters)
   - 4.3 [Type Stability Checking Algorithm](#43-type-stability-checking-algorithm)
   - 4.4 [Type Unification Algorithm](#44-type-unification-algorithm)
   - 4.5 [Type Stability for Generic/Polymorphic Returns](#45-type-stability-for-genericpolymorphic-returns)
   - 4.6 [Effect-Polymorphic Type Stability](#46-effect-polymorphic-type-stability)
   - 4.7 [Stability Annotations](#47-stability-annotations)
   - 4.8 [Union Types for Controlled Instability](#48-union-types-for-controlled-instability)
5. [Ambiguity Detection](#5-ambiguity-detection)
6. [Compile-Time vs Runtime Dispatch](#6-compile-time-vs-runtime-dispatch)
7. [Dispatch and Effects](#7-dispatch-and-effects)
8. [Performance Considerations](#8-performance-considerations)
9. [Constraint Solver Specification](#9-constraint-solver-specification)
10. [Cross-Reference: Formal Typing Rules](#10-cross-reference-formal-typing-rules)
11. [Related Work](#11-related-work)

---

## 1. Overview

### 1.1 What is Multiple Dispatch?

Multiple dispatch selects which method implementation to call based on the runtime types of **all** arguments, not just the receiver. This contrasts with:

| Dispatch Type | Selection Based On | Examples |
|---------------|-------------------|----------|
| Single dispatch | First argument (receiver) only | Java, C++, Python |
| Multiple dispatch | All argument types | Julia, Dylan, Blood |

### 1.2 Blood's Approach

Blood combines Julia's multiple dispatch with strict type stability enforcement:

- **Open methods**: New implementations can be added without modifying original definitions
- **Type-stable dispatch**: Return type is determined by input types at compile time
- **Ambiguity = Error**: Ambiguous dispatch is a compile-time error, not runtime
- **Effect-aware**: Methods declare their effects; dispatch considers effect compatibility

### 1.3 Design Goals

1. **Solve the Expression Problem**: Add new types and operations independently
2. **Predictable Performance**: Type stability ensures no dispatch overhead in hot paths
3. **Clear Errors**: Ambiguity caught at compile time with actionable messages
4. **Composability**: Works seamlessly with effect system and generics

### 1.4 Related Specifications

- [SPECIFICATION.md](./SPECIFICATION.md) — Core language specification
- [GRAMMAR.md](./GRAMMAR.md) — Method declaration syntax
- [FORMAL_SEMANTICS.md](./FORMAL_SEMANTICS.md) — Typing rules for dispatch
- [CONTENT_ADDRESSED.md](./CONTENT_ADDRESSED.md) — VFT and hash-based lookup
- [DIAGNOSTICS.md](./DIAGNOSTICS.md) — Dispatch error messages (E06xx)

---

## 2. Method Declaration

### 2.1 Syntax

```ebnf
MethodDecl ::= 'fn' MethodName TypeParams? '(' Params ')' '->' ReturnType '/' EffectRow Block

Params ::= (Param ',')* Param?
Param ::= Pattern ':' Type

TypeParams ::= '<' (TypeParam ',')* '>'
TypeParam ::= Ident (':' Constraint)?
```

### 2.2 Method Families

A **method family** is a set of methods sharing the same name. Each method provides a different implementation based on argument types:

```blood
// Method family: `add`
fn add(x: i32, y: i32) -> i32 / pure { x + y }
fn add(x: f64, y: f64) -> f64 / pure { x + y }
fn add(x: String, y: String) -> String / pure { x.concat(y) }
fn add<T: Numeric>(x: Vec<T>, y: Vec<T>) -> Vec<T> / pure {
    x.zip(y).map(|(a, b)| a + b)
}
```

### 2.3 Method Signatures

A method signature determines dispatch eligibility:

```
MethodSignature = (MethodName, [ParamType₁, ParamType₂, ..., ParamTypeₙ])
```

Two signatures **conflict** if they could both match the same concrete argument types without one being strictly more specific.

### 2.4 Constraints

Type parameters can have constraints:

```blood
fn sort<T: Ord>(list: Vec<T>) -> Vec<T> / pure { ... }

fn serialize<T: Serialize>(value: T) -> Bytes / {IO, Error<SerializeError>} { ... }
```

---

## 3. Dispatch Resolution Algorithm

### 3.1 Overview

When a function call `f(a₁, a₂, ..., aₙ)` is encountered:

1. **Collect candidates**: Find all methods named `f` with `n` parameters
2. **Filter applicable**: Keep methods where each parameter type matches argument type
3. **Order by specificity**: Rank candidates from most to least specific
4. **Select best**: Choose the unique most specific method, or error

### 3.2 Applicability Check

A method `m` with parameter types `[P₁, ..., Pₙ]` is **applicable** to arguments with types `[A₁, ..., Aₙ]` if:

```
∀i ∈ 1..n: Aᵢ <: Pᵢ   (each argument type is a subtype of the parameter type)
```

```
APPLICABLE(method, arg_types) → bool:
    IF len(method.params) ≠ len(arg_types):
        RETURN false

    FOR i IN 0..len(arg_types):
        param_type ← method.params[i].type
        arg_type ← arg_types[i]

        IF NOT is_subtype(arg_type, param_type):
            RETURN false

    RETURN true
```

### 3.3 Specificity Ordering

Method `m₁` is **more specific than** method `m₂` if:

```
∀i: P₁ᵢ <: P₂ᵢ   AND   ∃j: P₁ⱼ ≠ P₂ⱼ
```

(Every parameter of m₁ is at least as specific as m₂, and at least one is strictly more specific)

```
MORE_SPECIFIC(m1, m2) → bool:
    all_at_least ← true
    some_strictly ← false

    FOR i IN 0..len(m1.params):
        p1 ← m1.params[i].type
        p2 ← m2.params[i].type

        IF NOT is_subtype(p1, p2):
            all_at_least ← false

        IF is_subtype(p1, p2) AND NOT is_subtype(p2, p1):
            some_strictly ← true

    RETURN all_at_least AND some_strictly
```

### 3.4 Complete Resolution Algorithm

```
RESOLVE_DISPATCH(method_name, arg_types) → Method | Error:
    // Step 1: Collect all methods with this name
    all_methods ← get_method_family(method_name)

    // Step 2: Filter to applicable methods
    applicable ← []
    FOR method IN all_methods:
        IF APPLICABLE(method, arg_types):
            applicable.append(method)

    // Step 3: Handle no matches
    IF applicable.is_empty():
        RETURN Error::NoMethodFound {
            name: method_name,
            arg_types: arg_types,
            candidates: all_methods
        }

    // Step 4: Find maximally specific methods
    maximal ← []
    FOR m IN applicable:
        is_maximal ← true
        FOR other IN applicable:
            IF other ≠ m AND MORE_SPECIFIC(other, m):
                is_maximal ← false
                BREAK
        IF is_maximal:
            maximal.append(m)

    // Step 5: Check for unique winner
    IF len(maximal) == 1:
        RETURN maximal[0]

    // Step 6: Ambiguity error
    RETURN Error::AmbiguousDispatch {
        name: method_name,
        arg_types: arg_types,
        candidates: maximal
    }
```

### 3.5 Resolution Order Summary

| Priority | Match Type | Description |
|----------|-----------|-------------|
| 1 | Exact | All argument types match parameter types exactly |
| 2 | Subtype | Arguments are subtypes of parameters |
| 3 | Generic | Type parameters instantiate to concrete types |
| 4 | Ambiguous | Multiple methods tie → Compile Error |

### 3.6 Examples

```blood
fn process(x: i32, y: i32) -> i32 { ... }        // Method A
fn process(x: i32, y: f64) -> f64 { ... }        // Method B
fn process<T: Numeric>(x: T, y: T) -> T { ... }  // Method C
fn process(x: Any, y: Any) -> Any { ... }        // Method D

// Resolution examples:
process(1, 2)        // → Method A (exact match)
process(1, 2.0)      // → Method B (exact match)
process(1.0, 2.0)    // → Method C (T=f64, generic match)
process("a", "b")    // → Method D (Any fallback)
process(1, 2u8)      // → ERROR: Ambiguous between A and C
```

### 3.7 Parametric Polymorphism and Specificity

When methods involve type parameters, specificity ordering follows these rules:

#### 3.7.1 Constrained vs Unconstrained Type Parameters

A constrained type parameter is more specific than an unconstrained one:

```blood
fn identity<T>(x: T) -> T { ... }              // Unconstrained
fn identity<T: Clone>(x: T) -> T { x.clone() } // Constrained by Clone
fn identity<T: Copy>(x: T) -> T { x }          // Constrained by Copy (Copy <: Clone)

// Resolution:
identity(42i32)  // → Copy version (most constrained applicable)
identity(vec)    // → Clone version (Copy not satisfied)
identity(handle) // → Unconstrained version (no constraints satisfied)
```

#### 3.7.2 Specificity Ordering for Type Parameters

```
PARAM_SPECIFICITY(p1, p2) → Ordering:
    // Rule 1: Concrete types are more specific than type parameters
    IF is_concrete(p1) AND is_type_param(p2):
        RETURN MoreSpecific

    // Rule 2: Among type parameters, more constraints = more specific
    IF is_type_param(p1) AND is_type_param(p2):
        c1 ← constraints(p1)
        c2 ← constraints(p2)

        // All of p1's constraints must be at least as strong as p2's
        IF ∀c ∈ c2: ∃c' ∈ c1 where c' <: c:
            IF c1 ⊃ c2:  // p1 has strictly more constraints
                RETURN MoreSpecific
            ELSE IF c1 = c2:
                RETURN Equal
        RETURN Incomparable

    // Rule 3: Instantiated type parameters
    // Vec<i32> is more specific than Vec<T>
    IF is_instantiated(p1) AND is_parameterized(p2):
        RETURN MoreSpecific

    RETURN Incomparable
```

#### 3.7.3 Constraint Hierarchy

Constraints form a subtyping hierarchy:

```
                    Any (no constraint)
                       │
            ┌──────────┼──────────┐
            │          │          │
         Sized      Default    Display
            │          │          │
            └──────────┼──────────┘
                       │
                     Clone
                       │
                     Copy
```

When resolving dispatch with constrained type parameters:

```blood
fn format<T>(x: T) -> String { ... }              // Level 0
fn format<T: Display>(x: T) -> String { ... }     // Level 1
fn format<T: Debug + Display>(x: T) -> String { } // Level 2

// The most constrained applicable method wins
format(42)  // → Debug + Display version
```

#### 3.7.4 Ambiguity with Type Parameters

Ambiguity can arise when constraints are incomparable:

```blood
fn process<T: Serialize>(x: T) -> Bytes { ... }
fn process<T: Hash>(x: T) -> Bytes { ... }

// Neither Serialize nor Hash is a subtype of the other
process(value)  // ERROR: Ambiguous if value: Serialize + Hash

// Resolution: Add a more specific method
fn process<T: Serialize + Hash>(x: T) -> Bytes { ... }
```

### 3.8 Diamond Problem Resolution

When a type implements multiple traits that define the same method, Blood uses **explicit qualification** to resolve ambiguity:

#### 3.8.1 The Diamond Problem

```blood
trait Drawable {
    fn render(&self) -> Image / pure
}

trait Printable {
    fn render(&self) -> String / pure
}

struct Document impl Drawable, Printable {
    // Must implement both render methods
    fn render(&self) -> Image / pure { ... }   // For Drawable
    fn render(&self) -> String / pure { ... }  // For Printable
}
```

#### 3.8.2 Resolution Rules

```
RESOLVE_DIAMOND(call_site, receiver_type) → Method | Error:
    applicable_traits ← []

    FOR trait IN receiver_type.implemented_traits:
        IF trait.has_method(call_site.method_name):
            applicable_traits.append(trait)

    IF len(applicable_traits) == 0:
        RETURN Error::NoMethodFound

    IF len(applicable_traits) == 1:
        RETURN applicable_traits[0].get_method(call_site.method_name)

    // Multiple traits define this method
    IF call_site.has_trait_qualification:
        qualified_trait ← call_site.trait_qualification
        IF qualified_trait IN applicable_traits:
            RETURN qualified_trait.get_method(call_site.method_name)
        ELSE:
            RETURN Error::TraitNotImplemented

    // No qualification provided
    RETURN Error::AmbiguousTrait {
        method: call_site.method_name,
        candidates: applicable_traits,
        suggestion: "Use qualified syntax: <Type as Trait>::method()"
    }
```

#### 3.8.3 Qualified Syntax

```blood
fn use_document(doc: Document) {
    // Ambiguous - both Drawable and Printable have render()
    // doc.render()  // ERROR

    // Qualified calls resolve ambiguity
    let image = <Document as Drawable>::render(&doc)
    let text = <Document as Printable>::render(&doc)

    // Alternative syntax with type ascription
    let image: Image = doc.render()   // Infers Drawable
    let text: String = doc.render()   // Infers Printable
}
```

#### 3.8.4 Trait Inheritance Diamond

```blood
trait Base {
    fn method(&self) -> i32 / pure
}

trait Left: Base {
    fn method(&self) -> i32 / pure { 1 }  // Override
}

trait Right: Base {
    fn method(&self) -> i32 / pure { 2 }  // Override
}

struct Diamond impl Left, Right {
    // Must explicitly choose or provide own implementation
    fn method(&self) -> i32 / pure {
        // Can delegate to either
        <Self as Left>::method(self)
    }
}
```

---

## 4. Type Stability

### 4.1 Definition

A method is **type-stable** if its return type is fully determined by its parameter types at compile time.

```
TYPE_STABLE(method) ↔ ∀ concrete arg_types:
    RETURN_TYPE(method, arg_types) is statically known
```

### 4.2 Why Type Stability Matters

Type-unstable code prevents optimization:

```blood
// TYPE-UNSTABLE (rejected by Blood)
fn unstable(x: i32) -> ??? {
    if x > 0 { x }            // returns i32
    else { "negative" }        // returns String
}
// Return type depends on VALUE of x, not TYPE

// TYPE-STABLE (accepted)
fn stable(x: i32) -> i32 {
    if x > 0 { x }
    else { -x }               // Always returns i32
}
```

### 4.3 Type Stability Checking Algorithm

```
CHECK_TYPE_STABILITY(method) → Result<(), StabilityError>:
    // Analyze all return paths
    return_types ← COLLECT_RETURN_TYPES(method.body)

    // Check if all return types unify to a single type
    IF len(return_types) == 0:
        RETURN Ok(())  // No returns (diverges)

    unified ← return_types[0]
    FOR i IN 1..len(return_types):
        unified ← UNIFY(unified, return_types[i])
        IF unified.is_err():
            RETURN Err(StabilityError {
                method: method,
                conflicting_types: return_types,
                suggestion: GENERATE_SUGGESTION(return_types)
            })

    // Verify unified type matches declared return type
    IF NOT is_subtype(unified, method.return_type):
        RETURN Err(StabilityError {
            expected: method.return_type,
            actual: unified
        })

    RETURN Ok(())

COLLECT_RETURN_TYPES(expr) → Set<Type>:
    MATCH expr:
        Return(e) → { infer_type(e) }
        If(cond, then_branch, else_branch) →
            COLLECT_RETURN_TYPES(then_branch) ∪
            COLLECT_RETURN_TYPES(else_branch)
        Match(scrutinee, arms) →
            ∪ { COLLECT_RETURN_TYPES(arm.body) | arm ∈ arms }
        Block(stmts, final_expr) →
            COLLECT_RETURN_TYPES(final_expr)
        _ → {}
```

### 4.4 Type Unification Algorithm

The `UNIFY` function referenced in §4.3 is central to type stability checking. Blood uses a modified Hindley-Milner unification algorithm extended for row polymorphism and effect types.

#### 4.4.1 Unification Data Structures

```
// Type representation
Type ::=
    | TVar(id: TypeVarId)           -- Unification variable
    | TCon(name: Symbol)            -- Type constructor (i32, bool, etc.)
    | TApp(con: Type, args: [Type]) -- Type application (Vec<T>, Option<i32>)
    | TFun(params: [Type], ret: Type, effects: EffectRow)
    | TRecord(fields: [(Symbol, Type)], row: RowVar?)
    | TForall(vars: [TypeVarId], body: Type)

// Effect row representation
EffectRow ::=
    | RowEmpty                      -- pure / {}
    | RowCons(effect: Effect, tail: EffectRow)
    | RowVar(id: RowVarId)          -- Open row variable

// Substitution: maps type variables to types
Substitution = Map<TypeVarId, Type>
```

#### 4.4.2 Core Unification Algorithm

```
UNIFY(t1: Type, t2: Type, subst: Substitution) → Result<Substitution, UnifyError>:
    // Apply current substitution to resolve any already-bound variables
    t1 ← APPLY_SUBST(subst, t1)
    t2 ← APPLY_SUBST(subst, t2)

    // Case 1: Identical types
    IF t1 == t2:
        RETURN Ok(subst)

    // Case 2: Type variable on left
    IF t1 = TVar(id):
        RETURN UNIFY_VAR(id, t2, subst)

    // Case 3: Type variable on right
    IF t2 = TVar(id):
        RETURN UNIFY_VAR(id, t1, subst)

    // Case 4: Type constructors
    IF t1 = TCon(name1) AND t2 = TCon(name2):
        IF name1 == name2:
            RETURN Ok(subst)
        ELSE:
            RETURN Err(UnifyError::TypeMismatch { expected: t1, found: t2 })

    // Case 5: Type applications
    IF t1 = TApp(con1, args1) AND t2 = TApp(con2, args2):
        subst ← UNIFY(con1, con2, subst)?
        IF len(args1) ≠ len(args2):
            RETURN Err(UnifyError::ArityMismatch)
        FOR i IN 0..len(args1):
            subst ← UNIFY(args1[i], args2[i], subst)?
        RETURN Ok(subst)

    // Case 6: Function types
    IF t1 = TFun(p1, r1, e1) AND t2 = TFun(p2, r2, e2):
        IF len(p1) ≠ len(p2):
            RETURN Err(UnifyError::ArityMismatch)
        FOR i IN 0..len(p1):
            subst ← UNIFY(p1[i], p2[i], subst)?
        subst ← UNIFY(r1, r2, subst)?
        subst ← UNIFY_EFFECTS(e1, e2, subst)?
        RETURN Ok(subst)

    // Case 7: Record types (row polymorphism)
    IF t1 = TRecord(f1, r1) AND t2 = TRecord(f2, r2):
        RETURN UNIFY_RECORDS(f1, r1, f2, r2, subst)

    // Case 8: Forall types (requires instantiation)
    IF t1 = TForall(vars1, body1) AND t2 = TForall(vars2, body2):
        RETURN UNIFY_FORALL(vars1, body1, vars2, body2, subst)

    // Case 9: Incompatible types
    RETURN Err(UnifyError::TypeMismatch { expected: t1, found: t2 })


UNIFY_VAR(id: TypeVarId, t: Type, subst: Substitution) → Result<Substitution, UnifyError>:
    // Occurs check: prevent infinite types like α = List<α>
    IF OCCURS(id, t):
        RETURN Err(UnifyError::InfiniteType { var: id, type: t })

    // Extend substitution
    RETURN Ok(subst.extend(id, t))


OCCURS(id: TypeVarId, t: Type) → bool:
    MATCH t:
        TVar(id2) → id == id2
        TCon(_) → false
        TApp(con, args) → OCCURS(id, con) OR any(OCCURS(id, a) FOR a IN args)
        TFun(params, ret, _) → any(OCCURS(id, p) FOR p IN params) OR OCCURS(id, ret)
        TRecord(fields, _) → any(OCCURS(id, f.1) FOR f IN fields)
        TForall(_, body) → OCCURS(id, body)


APPLY_SUBST(subst: Substitution, t: Type) → Type:
    MATCH t:
        TVar(id) →
            IF id IN subst:
                APPLY_SUBST(subst, subst[id])  // Recursive application
            ELSE:
                t
        TCon(_) → t
        TApp(con, args) →
            TApp(APPLY_SUBST(subst, con), [APPLY_SUBST(subst, a) FOR a IN args])
        TFun(params, ret, effects) →
            TFun(
                [APPLY_SUBST(subst, p) FOR p IN params],
                APPLY_SUBST(subst, ret),
                APPLY_SUBST_EFFECTS(subst, effects)
            )
        TRecord(fields, row) →
            TRecord(
                [(name, APPLY_SUBST(subst, ty)) FOR (name, ty) IN fields],
                row  // Row variable substitution handled separately
            )
        TForall(vars, body) →
            TForall(vars, APPLY_SUBST(subst.without(vars), body))
```

#### 4.4.3 Effect Row Unification

```
UNIFY_EFFECTS(e1: EffectRow, e2: EffectRow, subst: Substitution) → Result<Substitution, UnifyError>:
    e1 ← APPLY_SUBST_EFFECTS(subst, e1)
    e2 ← APPLY_SUBST_EFFECTS(subst, e2)

    // Case 1: Both empty (pure)
    IF e1 = RowEmpty AND e2 = RowEmpty:
        RETURN Ok(subst)

    // Case 2: Row variable
    IF e1 = RowVar(id):
        RETURN UNIFY_ROW_VAR(id, e2, subst)
    IF e2 = RowVar(id):
        RETURN UNIFY_ROW_VAR(id, e1, subst)

    // Case 3: Both concrete rows - must contain same effects (order-independent)
    IF e1 = RowCons(eff1, tail1) AND e2 = RowCons(eff2, tail2):
        effects1 ← COLLECT_EFFECTS(e1)
        effects2 ← COLLECT_EFFECTS(e2)

        // Extract row variables if present
        rowvar1 ← EXTRACT_ROW_VAR(e1)
        rowvar2 ← EXTRACT_ROW_VAR(e2)

        IF rowvar1.is_none() AND rowvar2.is_none():
            // Both closed: must be identical sets
            IF effects1 == effects2:
                RETURN Ok(subst)
            ELSE:
                RETURN Err(UnifyError::EffectMismatch)

        // One or both open: unify with row constraint
        RETURN UNIFY_OPEN_ROWS(effects1, rowvar1, effects2, rowvar2, subst)

    // Case 4: Mismatch
    RETURN Err(UnifyError::EffectMismatch { expected: e1, found: e2 })


COLLECT_EFFECTS(row: EffectRow) → Set<Effect>:
    MATCH row:
        RowEmpty → {}
        RowVar(_) → {}
        RowCons(eff, tail) → {eff} ∪ COLLECT_EFFECTS(tail)
```

#### 4.4.4 Record Row Unification

```
UNIFY_RECORDS(f1: [(Symbol, Type)], r1: RowVar?,
              f2: [(Symbol, Type)], r2: RowVar?,
              subst: Substitution) → Result<Substitution, UnifyError>:

    // Convert to maps for easier manipulation
    fields1 ← Map::from(f1)
    fields2 ← Map::from(f2)

    // Find common, left-only, and right-only fields
    common ← fields1.keys() ∩ fields2.keys()
    left_only ← fields1.keys() - fields2.keys()
    right_only ← fields2.keys() - fields1.keys()

    // Unify common fields
    FOR name IN common:
        subst ← UNIFY(fields1[name], fields2[name], subst)?

    // Handle row polymorphism
    MATCH (r1, r2, left_only.is_empty(), right_only.is_empty()):
        // Both closed, no extra fields: OK
        (None, None, true, true) → RETURN Ok(subst)

        // Both closed, extra fields: Error
        (None, None, _, _) →
            RETURN Err(UnifyError::RecordFieldMismatch)

        // Left open: right_only fields absorbed by r1
        (Some(rv1), None, _, true) →
            extra_fields ← [(name, fields1[name]) FOR name IN left_only]
            RETURN Ok(subst.extend(rv1, TRecord(extra_fields, None)))

        // Right open: left_only fields absorbed by r2
        (None, Some(rv2), true, _) →
            extra_fields ← [(name, fields2[name]) FOR name IN right_only]
            RETURN Ok(subst.extend(rv2, TRecord(extra_fields, None)))

        // Both open: create fresh row variable for shared tail
        (Some(rv1), Some(rv2), _, _) →
            fresh_rv ← fresh_row_var()
            left_extra ← [(name, fields1[name]) FOR name IN left_only]
            right_extra ← [(name, fields2[name]) FOR name IN right_only]
            subst ← subst.extend(rv1, TRecord(right_extra, Some(fresh_rv)))
            subst ← subst.extend(rv2, TRecord(left_extra, Some(fresh_rv)))
            RETURN Ok(subst)
```

#### 4.4.5 Unification Error Types

```
enum UnifyError {
    TypeMismatch { expected: Type, found: Type },
    ArityMismatch { expected: usize, found: usize },
    InfiniteType { var: TypeVarId, type: Type },
    EffectMismatch { expected: EffectRow, found: EffectRow },
    RecordFieldMismatch { missing: Set<Symbol>, extra: Set<Symbol> },
    ConstraintViolation { var: TypeVarId, constraint: Constraint, actual: Type },
}
```

### 4.5 Type Stability for Generic/Polymorphic Returns

Type stability checking for generic functions requires special handling because return types may involve type parameters.

#### 4.5.1 Polymorphic Stability Rules

**Rule 1: Parametric Return Types Are Stable**

A function returning a type parameter is type-stable because the return type is determined by the instantiation at the call site:

```blood
// Type-stable: return type T determined by argument type
fn identity<T>(x: T) -> T { x }

// Type-stable: return type Vec<T> determined by argument type
fn singleton<T>(x: T) -> Vec<T> { [x] }
```

**Rule 2: Constrained Parameters Preserve Stability**

Constraints do not affect stability—they only restrict which types can instantiate the parameter:

```blood
// Type-stable: return type T determined by argument, constrained to Numeric
fn double<T: Numeric>(x: T) -> T { x + x }
```

**Rule 3: Associated Types Must Be Determinable**

When return type involves associated types, stability requires the association to be determinable from inputs:

```blood
trait Iterator {
    type Item
    fn next(&mut self) -> Option<Self::Item>
}

// Type-stable: Self::Item determined by Self type
fn first<I: Iterator>(iter: &mut I) -> Option<I::Item> {
    iter.next()
}
```

#### 4.5.2 Polymorphic Stability Checking Algorithm

```
CHECK_POLYMORPHIC_STABILITY(method: Method) → Result<(), StabilityError>:
    // Step 1: Collect all type parameters
    type_params ← method.type_params

    // Step 2: Collect return type's free type variables
    return_ftvs ← FREE_TYPE_VARS(method.return_type)

    // Step 3: Collect type variables determinable from parameters
    param_ftvs ← ∪ { FREE_TYPE_VARS(p.type) FOR p IN method.params }

    // Step 4: Every return type variable must be in parameters
    undetermined ← return_ftvs - param_ftvs

    IF undetermined.is_not_empty():
        RETURN Err(StabilityError::UndeterminedTypeVariable {
            variables: undetermined,
            hint: "Return type contains type variables not determined by parameters"
        })

    // Step 5: Check body for conditional type instability
    RETURN CHECK_TYPE_STABILITY(method)  // Standard algorithm from §4.3


FREE_TYPE_VARS(t: Type) → Set<TypeVarId>:
    MATCH t:
        TVar(id) → {id}
        TCon(_) → {}
        TApp(con, args) → FREE_TYPE_VARS(con) ∪ (∪ { FREE_TYPE_VARS(a) FOR a IN args })
        TFun(params, ret, _) → (∪ { FREE_TYPE_VARS(p) FOR p IN params }) ∪ FREE_TYPE_VARS(ret)
        TRecord(fields, _) → ∪ { FREE_TYPE_VARS(f.1) FOR f IN fields }
        TForall(vars, body) → FREE_TYPE_VARS(body) - vars
```

#### 4.5.3 Examples

```blood
// ✓ STABLE: T appears in parameter, determines return
fn wrap<T>(x: T) -> Option<T> { Some(x) }

// ✓ STABLE: T and U both appear in parameters
fn pair<T, U>(x: T, y: U) -> (T, U) { (x, y) }

// ✗ UNSTABLE: U not determined by parameters
fn phantom<T, U>(x: T) -> U { ... }  // ERROR: U undetermined

// ✓ STABLE: Result type fully determined by A and E
fn try_map<A, B, E>(x: Result<A, E>, f: fn(A) -> B) -> Result<B, E> {
    match x {
        Ok(a) => Ok(f(a)),
        Err(e) => Err(e),
    }
}
```

### 4.6 Effect-Polymorphic Type Stability

Functions with effect-polymorphic signatures require special stability rules.

#### 4.6.1 Effect Polymorphism Stability Rules

**Rule 1: Effect Parameters Don't Affect Return Type Stability**

Effect polymorphism only affects the effect signature, not the value type:

```blood
// Type-stable: effect E doesn't affect that return is List<B>
fn map<A, B, E>(xs: List<A>, f: fn(A) -> B / E) -> List<B> / E {
    // ...
}
```

**Rule 2: Effect-Dependent Return Types Are Unstable**

If the return type depends on which effects are present (beyond effect rows), this is unstable:

```blood
// ✗ UNSTABLE: Would require return type to change based on effect
// (This is a theoretical anti-pattern, not valid Blood syntax)
```

#### 4.6.2 Effect Row Stability

Effect rows in return types are stable when:

1. **Closed effect rows**: All effects statically known
2. **Open effect rows with row variable from parameters**: Propagated from inputs
3. **Inferred effect rows**: Computed from body

```blood
// ✓ STABLE: Effect row E propagated from parameter
fn apply<A, B, E>(f: fn(A) -> B / E, x: A) -> B / E {
    f(x)
}

// ✓ STABLE: Effect row inferred from body (closed)
fn pure_double(x: i32) -> i32 / pure {
    x * 2
}

// ✓ STABLE: Effect row explicitly closed
fn read_file(path: Path) -> String / {IO, Error<IOError>} {
    // ...
}
```

#### 4.6.3 Effect Inference and Stability

```
CHECK_EFFECT_STABILITY(method: Method) → Result<(), StabilityError>:
    declared_effects ← method.effect_row

    // Step 1: Collect effect row variables from parameters
    param_effect_vars ← COLLECT_EFFECT_VARS(method.params)

    // Step 2: Collect effect row variables in declared effects
    declared_effect_vars ← EFFECT_ROW_VARS(declared_effects)

    // Step 3: Undeclared variables must come from parameters
    FOR var IN declared_effect_vars:
        IF var NOT IN param_effect_vars:
            IF NOT is_inferred_from_body(var, method.body):
                RETURN Err(StabilityError::UndeterminedEffectVariable {
                    variable: var
                })

    RETURN Ok(())


EFFECT_ROW_VARS(row: EffectRow) → Set<RowVarId>:
    MATCH row:
        RowEmpty → {}
        RowVar(id) → {id}
        RowCons(_, tail) → EFFECT_ROW_VARS(tail)
```

### 4.7 Stability Annotations

```blood
// Explicit stability assertion (checked by compiler)
#[stable]
fn definitely_stable<T>(x: T) -> T { x }

// Opt-out for dynamic scenarios (requires justification)
#[unstable(reason = "Returns heterogeneous collection")]
fn parse_json(input: String) -> Any / {Error<ParseError>} { ... }
```

### 4.8 Union Types for Controlled Instability

When multiple return types are genuinely needed:

```blood
// Instead of type instability, use explicit union
enum ParseResult {
    Integer(i64),
    Float(f64),
    String(String),
    Null,
}

fn parse_value(input: &str) -> ParseResult / pure {
    // Type-stable: always returns ParseResult
    if is_integer(input) { ParseResult::Integer(parse_int(input)) }
    else if is_float(input) { ParseResult::Float(parse_float(input)) }
    else { ParseResult::String(input.to_string()) }
}
```

---

## 5. Ambiguity Detection

### 5.1 Definition

Two methods are **ambiguous** for a set of argument types if:
1. Both are applicable
2. Neither is more specific than the other

### 5.2 Detection Algorithm

```
DETECT_AMBIGUITIES(method_family) → List<Ambiguity>:
    ambiguities ← []
    methods ← method_family.methods

    FOR i IN 0..len(methods):
        FOR j IN (i+1)..len(methods):
            m1 ← methods[i]
            m2 ← methods[j]

            // Check if there exist argument types where both apply
            // but neither is more specific
            overlap ← COMPUTE_OVERLAP(m1.param_types, m2.param_types)

            IF overlap.is_some():
                // Check specificity
                m1_more ← MORE_SPECIFIC(m1, m2)
                m2_more ← MORE_SPECIFIC(m2, m1)

                IF NOT m1_more AND NOT m2_more:
                    ambiguities.append(Ambiguity {
                        method1: m1,
                        method2: m2,
                        overlapping_types: overlap
                    })

    RETURN ambiguities

COMPUTE_OVERLAP(types1, types2) → Option<List<Type>>:
    // Find concrete types that match both signatures
    // Uses type lattice intersection
    overlap ← []
    FOR i IN 0..len(types1):
        intersection ← TYPE_INTERSECT(types1[i], types2[i])
        IF intersection.is_empty():
            RETURN None  // No overlap
        overlap.append(intersection)
    RETURN Some(overlap)
```

### 5.3 Resolving Ambiguities

When ambiguity is detected, developers must add a more specific method:

```blood
// These two methods are ambiguous for process(1, 2u8):
fn process(x: i32, y: i32) -> i32 { ... }
fn process<T: Numeric>(x: T, y: T) -> T { ... }

// Resolution: Add a specific method for the overlapping case
fn process(x: i32, y: u8) -> i32 { x + y as i32 }
```

### 5.4 Ambiguity Error Messages

```
error[E0301]: ambiguous method dispatch
  --> src/lib.blood:42:5
   |
42 |     let result = process(1, 2u8)
   |                  ^^^^^^^^^^^^^^
   |
   = note: multiple methods match this call:

   candidate 1: fn process(x: i32, y: i32) -> i32
     --> src/numeric.blood:10:1

   candidate 2: fn process<T: Numeric>(x: T, y: T) -> T
     --> src/generic.blood:25:1

   = help: add a more specific method to resolve the ambiguity:

     fn process(x: i32, y: u8) -> i32 { ... }
```

---

## 6. Compile-Time vs Runtime Dispatch

### 6.1 Static Dispatch (Default)

When all argument types are known at compile time, Blood performs **static dispatch**:

```blood
fn example() {
    let x: i32 = 5
    let y: i32 = 10
    let result = add(x, y)  // Statically resolved to add(i32, i32)
}
```

Compiler inlines or direct-calls the resolved method. **Zero dispatch overhead**.

### 6.2 Dynamic Dispatch

When argument types are unknown at compile time, Blood uses **dynamic dispatch**:

```blood
fn process_any(values: Vec<Any>) {
    for v in values {
        let result = stringify(v)  // Dynamic dispatch
    }
}
```

Dynamic dispatch uses the 24-bit type fingerprint in pointer metadata:

```
DYNAMIC_DISPATCH(method_name, args) → Result:
    // Extract type fingerprints from argument metadata
    fingerprints ← [arg.metadata.type_fp FOR arg IN args]

    // Look up in dispatch table (hash map)
    key ← hash(method_name, fingerprints)
    method ← dispatch_table.get(key)

    IF method.is_none():
        // Fingerprint collision or no method: fall back to full type check
        method ← RESOLVE_DISPATCH(method_name, extract_types(args))

    RETURN method.call(args)
```

### 6.3 Dispatch Table Structure

Blood uses a **hierarchical dispatch table** with bloom filters to minimize collision impact:

```
// Primary structure: two-level dispatch table
DispatchTable = {
    method_tables: HashMap<MethodName, MethodDispatchTable>,
}

MethodDispatchTable = {
    // Fast path: fingerprint-based lookup
    fingerprint_map: HashMap<[TypeFingerprint; N], MethodEntry>,

    // Collision detection: bloom filter per method family
    collision_filter: BloomFilter,

    // Slow path: full type resolution cache
    full_type_cache: LruCache<[TypeId], Method>,
}

MethodEntry = {
    method: Method,
    // Store full type IDs for collision verification
    full_types: [TypeId],
}

// Type fingerprint: 24-bit hash of type (from pointer metadata)
TypeFingerprint = u24

// Full type ID: content-addressed hash
TypeId = Blake3Hash
```

#### 6.3.1 Collision Probability Analysis

With 24-bit fingerprints (16.7M unique values):

| Method Family Size | Expected Collisions | Collision Probability |
|--------------------|--------------------|-----------------------|
| 10 methods | ~0.000003 | 0.0003% |
| 100 methods | ~0.0003 | 0.03% |
| 1000 methods | ~0.03 | 3% |
| 10000 methods | ~3 | ~95% (at least one) |

For typical programs (< 1000 methods per family), collisions are rare.

#### 6.3.2 Hierarchical Lookup Algorithm

```
HIERARCHICAL_DISPATCH(method_name, args) → Result:
    table ← dispatch_tables.get(method_name)

    // Extract fingerprints from argument metadata
    fingerprints ← [arg.metadata.type_fp FOR arg IN args]

    // Level 1: Fast fingerprint lookup
    entry ← table.fingerprint_map.get(fingerprints)

    IF entry.is_some():
        // Verify no collision using bloom filter
        IF NOT table.collision_filter.might_contain(fingerprints):
            // No possible collision, fast path succeeds
            RETURN entry.method

        // Potential collision: verify with full type IDs
        arg_type_ids ← [get_type_id(arg) FOR arg IN args]
        IF arg_type_ids == entry.full_types:
            RETURN entry.method

        // Actual collision: fall through to slow path

    // Level 2: Full type resolution (cached)
    arg_type_ids ← [get_type_id(arg) FOR arg IN args]
    cached ← table.full_type_cache.get(arg_type_ids)

    IF cached.is_some():
        RETURN cached

    // Level 3: Full dispatch resolution
    method ← RESOLVE_DISPATCH(method_name, extract_types(args))
    table.full_type_cache.put(arg_type_ids, method)

    RETURN method
```

#### 6.3.3 Bloom Filter for Collision Detection

```
// Bloom filter with k=3 hash functions, m=2^16 bits
BloomFilter = {
    bits: BitArray<65536>,
    hash_seeds: [u64; 3],
}

BUILD_COLLISION_FILTER(method_table) → BloomFilter:
    filter ← BloomFilter::new()
    seen_fingerprints ← HashSet::new()

    FOR entry IN method_table.entries:
        fps ← entry.fingerprints
        IF seen_fingerprints.contains(fps):
            // Mark this fingerprint combo as potentially colliding
            filter.insert(fps)
        seen_fingerprints.insert(fps)

    RETURN filter
```

#### 6.3.4 Performance Characteristics (Unvalidated)

| Scenario | Cycles (est.) | Notes |
|----------|---------------|-------|
| Fingerprint hit, no collision | ~5-8 | Hash lookup + bloom check |
| Fingerprint hit, bloom positive | ~15-25 | + Full type ID comparison |
| Fingerprint hit, actual collision | ~30-50 | + LRU cache lookup |
| Cache miss, full resolution | ~100-200 | Full dispatch algorithm |

> **Note**: These cycle estimates are theoretical design targets based on similar dispatch systems in Julia and Common Lisp. Actual performance will be validated during implementation.

The bloom filter false positive rate is ~1% with the above parameters (based on standard bloom filter mathematics), meaning only ~1% of non-colliding lookups pay the verification cost.

### 6.4 Monomorphization

For generic methods, Blood uses **monomorphization** (like Rust):

```blood
fn identity<T>(x: T) -> T { x }

// Usage:
identity(42)      // Generates: identity_i32
identity("hello") // Generates: identity_String
identity(3.14)    // Generates: identity_f64
```

Monomorphization ensures type-stable code has zero dispatch overhead.

---

## 7. Dispatch and Effects

### 7.1 Effect-Aware Dispatch

Methods declare their effects. Dispatch considers effect compatibility:

```blood
fn load(path: Path) -> Data / {IO} { ... }
fn load(path: Path) -> Data / pure { load_from_cache(path) }

// In pure context:
fn process() / pure {
    let data = load("config.toml")  // Dispatches to pure version
}

// In IO context:
fn main() / {IO} {
    let data = load("config.toml")  // Could dispatch to either
    // Prefers more specific (pure) if available
}
```

### 7.2 Effect Subsumption in Dispatch

A method with **fewer effects** is more specific than one with more:

```
pure <: {IO} <: {IO, Error<E>}
```

This means:
- Pure methods are preferred when both pure and effectful versions exist
- Effectful methods can always call pure methods

### 7.3 Combined Type and Effect Specificity

When both type specificity and effect specificity vary, Blood uses **lexicographic ordering**:

```
COMBINED_SPECIFICITY(m1, m2) → Ordering:
    // Step 1: Compare type specificity
    type_cmp ← TYPE_SPECIFICITY(m1.param_types, m2.param_types)

    IF type_cmp ≠ Equal:
        RETURN type_cmp  // Type specificity takes precedence

    // Step 2: If types are equally specific, compare effects
    effect_cmp ← EFFECT_SPECIFICITY(m1.effects, m2.effects)

    RETURN effect_cmp

EFFECT_SPECIFICITY(e1, e2) → Ordering:
    // Fewer effects = more specific
    // e1 <: e2 means e1 is a subset of e2

    IF e1 ⊆ e2 AND e2 ⊆ e1:
        RETURN Equal
    IF e1 ⊂ e2:
        RETURN MoreSpecific  // m1 has fewer effects
    IF e2 ⊂ e1:
        RETURN LessSpecific
    RETURN Incomparable  // Neither is subset of other
```

#### 7.3.1 Specificity Priority: Types First, Then Effects

```blood
// Example: Type specificity wins over effect specificity
fn process(x: i32) -> i32 / {IO} { ... }        // Method A
fn process<T: Numeric>(x: T) -> T / pure { ... } // Method B

// For process(42i32):
// - Method A: type match = exact (i32), effects = {IO}
// - Method B: type match = generic (T=i32), effects = pure
// Winner: Method A (more specific type, despite more effects)
```

#### 7.3.2 Effect Specificity as Tiebreaker

```blood
// When types are equally specific, effects decide
fn load(path: Path) -> Data / {IO, Error<E>} { ... }  // Method A
fn load(path: Path) -> Data / {IO} { ... }            // Method B
fn load(path: Path) -> Data / pure { ... }            // Method C

// For load("config.toml"):
// All three have identical type signatures
// Effect specificity: pure <: {IO} <: {IO, Error<E>}
// Winner: Method C (fewest effects)
```

#### 7.3.3 Effect Context Constraints

Dispatch must also satisfy the **effect context constraint**:

```
EFFECT_CONTEXT_CHECK(method, call_context) → bool:
    // The selected method's effects must be subset of allowed effects
    RETURN method.effects ⊆ call_context.allowed_effects

DISPATCH_WITH_EFFECTS(method_name, args, context) → Method | Error:
    candidates ← RESOLVE_DISPATCH(method_name, arg_types)

    // Filter by effect compatibility
    compatible ← [m FOR m IN candidates IF EFFECT_CONTEXT_CHECK(m, context)]

    IF compatible.is_empty():
        RETURN Error::EffectNotAllowed {
            method: candidates[0],
            required: candidates[0].effects,
            available: context.allowed_effects
        }

    // Select most specific among compatible
    RETURN SELECT_MOST_SPECIFIC(compatible)
```

```blood
// Effect context constrains available methods
fn pure_context() / pure {
    load("config.toml")  // Only pure version eligible
}

fn io_context() / {IO} {
    load("config.toml")  // pure and {IO} versions eligible
                         // pure version selected (more specific)
}

fn full_context() / {IO, Error<E>} {
    load("config.toml")  // All versions eligible
                         // pure version selected (most specific)
}
```

#### 7.3.4 Ambiguity with Incomparable Effects

```blood
fn process(x: Data) -> Result / {IO} { ... }
fn process(x: Data) -> Result / {State<S>} { ... }

// In context / {IO, State<S>}:
// Neither {IO} nor {State<S>} is subset of the other
// ERROR: Ambiguous effect dispatch

// Resolution: Add disambiguating method
fn process(x: Data) -> Result / {IO, State<S>} { ... }
```

### 7.4 Effect-Polymorphic Methods

```blood
fn map<A, B, E>(list: List<A>, f: fn(A) -> B / E) -> List<B> / E {
    // Effect E is determined by the function argument
}

// Usage:
map(nums, |x| x * 2)           // E = pure
map(nums, |x| { print(x); x }) // E = {IO}
```

---

## 8. Performance Considerations

> **Validation Status**: The performance characteristics in this section are theoretical design targets based on analysis of similar systems (Julia, Common Lisp CLOS, Dylan). Empirical validation will occur during Blood implementation.

### 8.1 Dispatch Overhead Summary (Unvalidated)

| Dispatch Type | Overhead (est.) | When Used |
|---------------|-----------------|-----------|
| Static (monomorphized) | 0 cycles | Types known at compile time |
| Static (known method) | ~0 cycles | Direct call, possible inline |
| Dynamic (fingerprint hit) | ~5-10 cycles | Hash lookup + indirect call |
| Dynamic (fingerprint miss) | ~50-100 cycles | Full type resolution |

### 8.2 Optimization Strategies

1. **Inline small methods**: Eliminate call overhead entirely
2. **Devirtualize when possible**: Convert dynamic to static dispatch
3. **Type fingerprint caching**: Fast path for common dispatch patterns
4. **Profile-guided optimization**: Specialize hot paths based on runtime data

### 8.3 Performance Anti-Patterns

```blood
// ANTI-PATTERN: Heterogeneous collection requiring dynamic dispatch
fn slow(items: Vec<Any>) {
    for item in items {
        process(item)  // Dynamic dispatch every iteration
    }
}

// PREFERRED: Homogeneous collection
fn fast<T: Process>(items: Vec<T>) {
    for item in items {
        process(item)  // Static dispatch, possible vectorization
    }
}

// PREFERRED: Tagged union when heterogeneity is needed
enum Item { A(TypeA), B(TypeB), C(TypeC) }

fn medium(items: Vec<Item>) {
    for item in items {
        match item {
            Item::A(a) => process(a),  // Static dispatch
            Item::B(b) => process(b),
            Item::C(c) => process(c),
        }
    }
}
```

---

## 9. Constraint Solver Specification

This section specifies the constraint solver used during type inference and type stability checking.

### 9.1 Constraint Language

```
Constraint ::=
    | TypeEq(Type, Type)                    -- Type equality
    | TypeSub(Type, Type)                   -- Subtyping: T₁ <: T₂
    | EffectSub(EffectRow, EffectRow)       -- Effect subsumption
    | HasField(Type, Symbol, Type)          -- Record has field
    | HasMethod(Type, Symbol, MethodSig)    -- Type has method
    | Implements(Type, Trait)               -- Trait implementation
    | ConstraintConj([Constraint])          -- Conjunction
    | ConstraintDisj([Constraint])          -- Disjunction (from match)
```

### 9.2 Constraint Generation

```
GENERATE_CONSTRAINTS(expr: Expr, expected: Type, env: TypeEnv) → (Type, [Constraint]):
    MATCH expr:
        Var(x) →
            ty ← env.lookup(x)
            RETURN (ty, [TypeSub(ty, expected)])

        Literal(lit) →
            ty ← type_of_literal(lit)
            RETURN (ty, [TypeSub(ty, expected)])

        App(func, arg) →
            α ← fresh_type_var()
            β ← fresh_type_var()
            ε ← fresh_effect_var()

            (t_func, c1) ← GENERATE_CONSTRAINTS(func, TFun([α], β, ε), env)
            (t_arg, c2) ← GENERATE_CONSTRAINTS(arg, α, env)

            constraints ← c1 ++ c2 ++ [TypeSub(β, expected)]
            RETURN (β, constraints)

        Lambda(param, body) →
            α ← fresh_type_var()
            β ← fresh_type_var()
            ε ← fresh_effect_var()

            env' ← env.extend(param.name, α)
            (t_body, c) ← GENERATE_CONSTRAINTS(body, β, env')

            ty ← TFun([α], β, ε)
            RETURN (ty, c ++ [TypeSub(ty, expected)])

        Let(name, value, body) →
            α ← fresh_type_var()
            (t_val, c1) ← GENERATE_CONSTRAINTS(value, α, env)

            // Generalize if value is a syntactic value
            scheme ← IF is_value(value):
                GENERALIZE(env, α, c1)
            ELSE:
                MonoType(α)

            env' ← env.extend(name, scheme)
            (t_body, c2) ← GENERATE_CONSTRAINTS(body, expected, env')
            RETURN (t_body, c1 ++ c2)

        If(cond, then_branch, else_branch) →
            (_, c1) ← GENERATE_CONSTRAINTS(cond, TBool, env)
            (t_then, c2) ← GENERATE_CONSTRAINTS(then_branch, expected, env)
            (t_else, c3) ← GENERATE_CONSTRAINTS(else_branch, expected, env)

            RETURN (expected, c1 ++ c2 ++ c3)

        MethodCall(receiver, method, args) →
            α ← fresh_type_var()
            (t_recv, c1) ← GENERATE_CONSTRAINTS(receiver, α, env)

            arg_types ← []
            arg_constraints ← []
            FOR arg IN args:
                β ← fresh_type_var()
                (t_arg, c) ← GENERATE_CONSTRAINTS(arg, β, env)
                arg_types.push(β)
                arg_constraints.extend(c)

            method_constraint ← HasMethod(α, method, MethodSig(arg_types, expected))
            RETURN (expected, c1 ++ arg_constraints ++ [method_constraint])

        Match(scrutinee, arms) →
            α ← fresh_type_var()
            (t_scr, c_scr) ← GENERATE_CONSTRAINTS(scrutinee, α, env)

            arm_constraints ← []
            FOR arm IN arms:
                (env', c_pat) ← GENERATE_PATTERN_CONSTRAINTS(arm.pattern, α, env)
                (_, c_body) ← GENERATE_CONSTRAINTS(arm.body, expected, env')
                arm_constraints.extend(c_pat ++ c_body)

            RETURN (expected, c_scr ++ arm_constraints)
```

### 9.3 Constraint Solving

```
SOLVE_CONSTRAINTS(constraints: [Constraint]) → Result<Substitution, TypeError>:
    subst ← empty_substitution()
    worklist ← constraints

    WHILE worklist.is_not_empty():
        constraint ← worklist.pop()
        constraint ← APPLY_SUBST_CONSTRAINT(subst, constraint)

        MATCH constraint:
            TypeEq(t1, t2) →
                subst' ← UNIFY(t1, t2, subst)?
                subst ← COMPOSE(subst, subst')

            TypeSub(t1, t2) →
                // For now, treat subtyping as equality
                // Full subtyping requires more sophisticated solving
                subst' ← UNIFY(t1, t2, subst)?
                subst ← COMPOSE(subst, subst')

            EffectSub(e1, e2) →
                subst' ← UNIFY_EFFECTS(e1, e2, subst)?
                subst ← COMPOSE(subst, subst')

            HasField(t, field, field_type) →
                t ← APPLY_SUBST(subst, t)
                MATCH t:
                    TRecord(fields, row) →
                        IF (field, ty) IN fields:
                            worklist.push(TypeEq(ty, field_type))
                        ELSE IF row.is_some():
                            // Extend row with new field
                            worklist.push(TypeEq(row, TRecord([(field, field_type)], fresh_row_var())))
                        ELSE:
                            RETURN Err(TypeError::MissingField { type: t, field: field })
                    TVar(_) →
                        // Defer constraint until type is known
                        worklist.push_back(constraint)
                    _ →
                        RETURN Err(TypeError::NotARecord { type: t })

            HasMethod(t, method, sig) →
                t ← APPLY_SUBST(subst, t)
                MATCH t:
                    TVar(_) →
                        // Defer until type is resolved
                        worklist.push_back(constraint)
                    _ →
                        // Look up method in type's method table
                        method_ty ← LOOKUP_METHOD(t, method)?
                        worklist.push(TypeEq(method_ty, sig.to_fn_type()))

            Implements(t, trait) →
                t ← APPLY_SUBST(subst, t)
                IF NOT has_implementation(t, trait):
                    RETURN Err(TypeError::TraitNotImplemented { type: t, trait: trait })

            ConstraintConj(cs) →
                worklist.extend(cs)

            ConstraintDisj(cs) →
                // Try each alternative, return first success
                FOR c IN cs:
                    result ← SOLVE_CONSTRAINTS([c] ++ worklist)
                    IF result.is_ok():
                        RETURN result
                RETURN Err(TypeError::NoMatchingBranch)

    RETURN Ok(subst)
```

### 9.4 Constraint Simplification

```
SIMPLIFY(constraints: [Constraint]) → [Constraint]:
    // Remove trivially satisfied constraints
    simplified ← []

    FOR c IN constraints:
        MATCH c:
            TypeEq(t, t) → skip  // Reflexive
            TypeSub(t, t) → skip  // Reflexive
            EffectSub(e, e) → skip  // Reflexive
            TypeEq(TVar(a), TVar(b)) IF a == b → skip
            _ → simplified.push(c)

    // Merge duplicate constraints
    RETURN deduplicate(simplified)
```

### 9.5 Let-Generalization

```
GENERALIZE(env: TypeEnv, ty: Type, constraints: [Constraint]) → TypeScheme:
    // Solve constraints first
    subst ← SOLVE_CONSTRAINTS(constraints)?
    ty ← APPLY_SUBST(subst, ty)

    // Find free variables not in environment
    env_ftvs ← FREE_TYPE_VARS_ENV(env)
    ty_ftvs ← FREE_TYPE_VARS(ty)
    generalizable ← ty_ftvs - env_ftvs

    IF generalizable.is_empty():
        RETURN MonoType(ty)
    ELSE:
        RETURN ForallType(generalizable.to_list(), ty)


INSTANTIATE(scheme: TypeScheme) → Type:
    MATCH scheme:
        MonoType(ty) → ty
        ForallType(vars, body) →
            fresh ← [fresh_type_var() FOR _ IN vars]
            subst ← zip(vars, fresh).to_map()
            APPLY_SUBST(subst, body)
```

### 9.6 Error Recovery

When constraint solving fails, the solver attempts to provide actionable diagnostics:

```
SOLVE_WITH_RECOVERY(constraints: [Constraint]) → (Substitution, [TypeError]):
    subst ← empty_substitution()
    errors ← []
    worklist ← constraints

    WHILE worklist.is_not_empty():
        constraint ← worklist.pop()
        result ← TRY_SOLVE_ONE(constraint, subst)

        MATCH result:
            Ok(subst') → subst ← COMPOSE(subst, subst')
            Err(err) →
                errors.push(err)
                // Continue with partial substitution
                // This allows finding multiple errors in one pass

    RETURN (subst, errors)
```

---

## 10. Cross-Reference: Formal Typing Rules

This section provides formal typing rules for dispatch that integrate with [FORMAL_SEMANTICS.md](./FORMAL_SEMANTICS.md).

### 10.1 Method Call Typing

```
METHOD_CALL_TYPING:

Γ ⊢ e₁ : T₁ / ε₁   ...   Γ ⊢ eₙ : Tₙ / εₙ
resolve_dispatch(f, [T₁, ..., Tₙ]) = m : (S₁, ..., Sₙ) → R / ε_m
∀i. Tᵢ <: Sᵢ
────────────────────────────────────────────────────────────────
Γ ⊢ f(e₁, ..., eₙ) : R / ε₁ ∪ ... ∪ εₙ ∪ ε_m
```

### 10.2 Method Declaration Typing

```
METHOD_DECL_TYPING:

Γ, x₁:T₁, ..., xₙ:Tₙ ⊢ body : R / ε
ε ⊆ ε_declared
stable(R, [T₁, ..., Tₙ])   // Type stability check (§4.3)
────────────────────────────────────────────────────────────────
⊢ fn f(x₁: T₁, ..., xₙ: Tₙ) -> R / ε_declared { body } : Method
```

### 10.3 Dispatch Resolution in Type Checking

```
DISPATCH_TYPE_CHECK(call_site: CallSite) → Type × EffectRow:
    method_name ← call_site.name
    arg_types ← [infer_type(arg) FOR arg IN call_site.args]

    // Resolve dispatch (from §3.4)
    method ← RESOLVE_DISPATCH(method_name, arg_types)

    // Check type stability (from §4.3)
    CHECK_TYPE_STABILITY(method)?

    // Return the method's return type and effect row
    return (method.return_type, method.effect_row)
```

### 10.4 Effect-Aware Dispatch Typing

Dispatch interacts with effect typing as specified in [FORMAL_SEMANTICS.md §5.3](./FORMAL_SEMANTICS.md#53-effect-rules):

```
EFFECT_DISPATCH_TYPING:

// Method selection considers effect context
Γ; ε_context ⊢ f(e₁, ..., eₙ)

// Candidate methods must have compatible effects
candidates ← { m | m ∈ methods(f) ∧ m.effects ⊆ ε_context }

// Select most specific among compatible
selected ← most_specific(candidates)

// Final type includes selected method's effects
Γ ⊢ f(e₁, ..., eₙ) : selected.return_type / selected.effects
```

### 10.5 Integration with Handler Typing

When dispatch occurs within effect handlers ([FORMAL_SEMANTICS.md §6](./FORMAL_SEMANTICS.md#6-handler-typing)):

```
HANDLER_DISPATCH:

// In handler context, effect row is restricted
with h handle { ... f(x, y) ... }

// Dispatch for f must select method with effects ⊆ handled effects ∪ ε
// where ε is the remaining effect row after handling
```

### 10.6 Correspondence Table

| DISPATCH.md | FORMAL_SEMANTICS.md | Description |
|-------------|---------------------|-------------|
| §3 Dispatch Resolution | §5.2 Core Rules (T-App) | Method resolution during type checking |
| §4 Type Stability | §7 Progress/Preservation | Stability ensures type safety |
| §7 Dispatch and Effects | §5.3 Effect Rules | Effect-aware method selection |
| §5 Ambiguity Detection | §9 Metatheory | Ambiguity = compile error |
| §9 Constraint Solver | §5.5 Row Polymorphism | Unification for dispatch |

---

## 11. Related Work

Blood's multiple dispatch design draws from:

1. **Julia** — Dynamic multiple dispatch with JIT specialization
   - [Julia Methods Documentation](https://docs.julialang.org/en/v1/manual/methods/)
   - Blood adapts Julia's dispatch semantics with static type stability enforcement

2. **Dylan** — Early multiple dispatch language
   - Influenced Julia's design
   - Blood follows similar specificity ordering

3. **CLOS (Common Lisp Object System)** — Original multiple dispatch implementation
   - Method combination and precedence rules

4. **Fortress** — Multiple dispatch with static type checking
   - Closest to Blood's approach of compile-time ambiguity detection

---

## Appendix A: Dispatch Algorithm Pseudocode

Complete algorithm implementation for reference:

```
// Main entry point
DISPATCH(call_site) → Method:
    method_name ← call_site.name
    arg_types ← [infer_type(arg) FOR arg IN call_site.args]

    // Check if all types are concrete
    IF all_concrete(arg_types):
        // Static dispatch path
        RETURN STATIC_DISPATCH(method_name, arg_types)
    ELSE:
        // Emit dynamic dispatch code
        RETURN EMIT_DYNAMIC_DISPATCH(method_name, call_site)

STATIC_DISPATCH(method_name, arg_types) → Method:
    // Use compile-time resolution
    result ← RESOLVE_DISPATCH(method_name, arg_types)

    MATCH result:
        Ok(method) →
            CHECK_TYPE_STABILITY(method)
            RETURN method
        Err(NoMethodFound(info)) →
            EMIT_ERROR("No method found", info)
        Err(AmbiguousDispatch(info)) →
            EMIT_ERROR("Ambiguous dispatch", info)

EMIT_DYNAMIC_DISPATCH(method_name, call_site) → Code:
    // Generate runtime dispatch code
    RETURN quote {
        let types = [$(arg).type_fingerprint FOR arg IN call_site.args]
        let key = hash(method_name, types)
        let method = dispatch_table.get_or_resolve(key, $(call_site.args))
        method.call($(call_site.args))
    }
```

---

*Last updated: 2026-01-09*
