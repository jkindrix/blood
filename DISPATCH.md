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
5. [Ambiguity Detection](#5-ambiguity-detection)
6. [Compile-Time vs Runtime Dispatch](#6-compile-time-vs-runtime-dispatch)
7. [Dispatch and Effects](#7-dispatch-and-effects)
8. [Performance Considerations](#8-performance-considerations)

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

### 4.4 Stability Annotations

```blood
// Explicit stability assertion (checked by compiler)
#[stable]
fn definitely_stable<T>(x: T) -> T { x }

// Opt-out for dynamic scenarios (requires justification)
#[unstable(reason = "Returns heterogeneous collection")]
fn parse_json(input: String) -> Any / {Error<ParseError>} { ... }
```

### 4.5 Union Types for Controlled Instability

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

## 9. Related Work

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
