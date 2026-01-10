# Blood Content-Addressed Storage Specification

**Version**: 0.3.0-draft
**Status**: Active Development
**Last Updated**: 2026-01-10

This document specifies Blood's content-addressed code identity system, including AST canonicalization, hash computation, storage format, and the Virtual Function Table (VFT) specification.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Hash Function](#2-hash-function)
3. [AST Canonicalization](#3-ast-canonicalization)
   - 3.2.1 [De Bruijn Indexing Algorithm](#321-de-bruijn-indexing-algorithm)
   - 3.5.1 [Type Canonicalization Algorithm](#351-type-canonicalization-algorithm)
   - 3.6.1 [Effect Row Sorting Algorithm](#361-effect-row-sorting-algorithm)
4. [Hash Computation Algorithm](#4-hash-computation-algorithm)
   - 4.5 [Determinism Proof Sketch](#45-determinism-proof-sketch)
5. [Storage Format](#5-storage-format)
   - 5.5 [Format Version Migration](#55-format-version-migration)
6. [Virtual Function Table (VFT)](#6-virtual-function-table-vft)
7. [Codebase Operations](#7-codebase-operations)
8. [Hot-Swapping](#8-hot-swapping)
9. [Collision Handling](#9-collision-handling)
10. [Tooling Integration](#10-tooling-integration)
    - 10.5 [Marrow Codebase Manager](#105-marrow-codebase-manager)
11. [Appendices](#11-appendices)
    - A. [Canonical AST Node Types](#appendix-a-canonical-ast-node-types)
    - B. [References](#appendix-b-references)
    - C. [Worked Canonicalization Example](#appendix-c-worked-canonicalization-example)
    - D. [Determinism Proofs](#appendix-d-determinism-proofs)

---

## 1. Overview

### 1.1 The Big Idea

Blood identifies all definitions by a cryptographic hash of their canonicalized AST, not by name. This is called **content-addressed code**.

```
┌──────────────────────────────────────────────────────────────┐
│  Traditional: Name-Based Identity                             │
│                                                               │
│  "add" in module A ────────────────────┐                      │
│  "add" in module B ────────────────────┼─── Name collision!   │
│  "add" in library v1.0 ────────────────┤                      │
│  "add" in library v2.0 ────────────────┘                      │
└──────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────┐
│  Blood: Content-Addressed Identity                            │
│                                                               │
│  fn add(x, y) = x + y  ──► #a7f2...  (unique hash)            │
│  fn add(x, y) = x + y  ──► #a7f2...  (same hash - same code!) │
│  fn add(x, y, z) = ...  ──► #b3c1...  (different hash)        │
│                                                               │
│  No collisions. Different versions coexist.                   │
└──────────────────────────────────────────────────────────────┘
```

### 1.2 Related Specifications

- [SPECIFICATION.md](./SPECIFICATION.md) — Core language specification
- [DISPATCH.md](./DISPATCH.md) — VFT dispatch integration
- [FORMAL_SEMANTICS.md](./FORMAL_SEMANTICS.md) — De Bruijn indexing
- [ROADMAP.md](./ROADMAP.md) — Codebase manager implementation
- [FFI.md](./FFI.md) — Hash-based symbol resolution

### 1.3 Implementation Status

The following table tracks implementation status of content addressing subsystems:

| Component | Status | Location | Notes |
|-----------|--------|----------|-------|
| ContentHash (BLAKE3-256) | ✅ Implemented | `bloodc/src/content/hash.rs` | 32-byte hash type |
| ContentHasher | ✅ Implemented | `bloodc/src/content/hash.rs` | Streaming BLAKE3 |
| CanonicalAST | ✅ Implemented | `bloodc/src/content/canonical.rs` | De Bruijn indexed |
| Canonicalizer | ✅ Implemented | `bloodc/src/content/canonical.rs` | AST→canonical transform |
| Codebase storage | ✅ Implemented | `bloodc/src/content/storage.rs` | DefinitionRecord store |
| Namespace | ✅ Implemented | `bloodc/src/content/namespace.rs` | Name→hash mappings |
| NameRegistry | ✅ Implemented | `bloodc/src/content/namespace.rs` | Hierarchical lookup |
| VFT structure | ✅ Implemented | `bloodc/src/content/vft.rs` | Virtual function table |
| VFTEntry | ✅ Implemented | `bloodc/src/content/vft.rs` | Per-method dispatch |
| DispatchTable | ✅ Implemented | `bloodc/src/content/vft.rs` | Type→methods mapping |
| BuildCache | ✅ Implemented | `bloodc/src/content/build_cache.rs` | Compiled object cache |
| hash_hir_item | ✅ Implemented | `bloodc/src/content/build_cache.rs` | HIR→hash computation |
| Cache lookup in build | ✅ Integrated | `bloodc/src/main.rs` | Per-definition hashing |
| Incremental compilation | 🔶 Partial | `bloodc/src/main.rs` | Hashes computed, caching basic |
| Hot-swap support | 📋 Designed | — | VFT update mechanism |
| Marrow codebase manager | 📋 Designed | — | UCM-style tooling |
| Distributed cache | 📋 Designed | — | Future enhancement |

**Legend**: ✅ Implemented | 🔶 Partial | 📋 Designed | ❌ Not Started

### 1.4 Benefits

| Benefit | Description |
|---------|-------------|
| **No Dependency Hell** | Multiple versions of the same library coexist by hash |
| **Perfect Incremental Compilation** | Only recompile definitions whose hashes changed |
| **Safe Refactoring** | Renames never break code (names are metadata) |
| **Zero-Downtime Upgrades** | Hot-swap code by hash without restart |
| **Distributed Caching** | Share compiled artifacts by hash globally |
| **Reproducible Builds** | Same code always produces same hash |

### 1.5 How It Works

```
┌─────────────┐    ┌──────────────────┐    ┌───────────────┐
│ Source Code │ ─► │ Canonicalized    │ ─► │ BLAKE3-256    │ ─► Hash
│             │    │ AST              │    │ Hash          │
└─────────────┘    └──────────────────┘    └───────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────┐
│                      CODEBASE DATABASE                       │
│                                                              │
│  Hash #a7f2...  ──►  { ast: ..., metadata: { name: "add" } } │
│  Hash #b3c1...  ──►  { ast: ..., metadata: { name: "sum" } } │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

---

## 2. Hash Function

### 2.1 BLAKE3-256

Blood uses BLAKE3-256 for content addressing:

| Property | Value |
|----------|-------|
| **Algorithm** | BLAKE3 |
| **Output Size** | 256 bits (32 bytes) |
| **Display Format** | Base32 (first 10 chars typical) |
| **Collision Resistance** | 128 bits (birthday attack) |

### 2.2 Why BLAKE3?

| Factor | BLAKE3 | SHA3-512 (Unison) | SHA-256 |
|--------|--------|-------------------|---------|
| **Speed** | ~5x faster¹ | Baseline | ~3x slower |
| **Parallelism** | Native SIMD/AVX2/AVX-512 | Limited | None |
| **Security** | 128-bit (256/2) | 256-bit | 128-bit |
| **Output Size** | 256 bits | 512 bits | 256 bits |
| **Hardware Accel** | SSE4.1, AVX2, AVX-512, NEON | Limited | SHA-NI only |

¹ Speed comparison based on [published BLAKE3 benchmarks](https://github.com/BLAKE3-team/BLAKE3) comparing against SHA3-256; relative performance vs SHA3-512 may vary by platform.

BLAKE3 provides sufficient collision resistance for code identification while being significantly faster than alternatives.

### 2.3 Divergence from Unison

Blood deliberately diverges from [Unison's](https://www.unison-lang.org/) choice of SHA3-512:

| Aspect | Unison | Blood | Rationale |
|--------|--------|-------|-----------|
| **Hash Algorithm** | SHA3-512 | BLAKE3-256 | Performance (~5x faster per published benchmarks) |
| **Hash Size** | 512 bits | 256 bits | Sufficient security, smaller pointers |
| **Security Level** | ~256 bits | ~128 bits | 128 bits adequate for code identity |

**Why this divergence is sound:**

1. **128-bit security is sufficient**: Code identity doesn't require post-quantum resistance. The threat model is accidental collision, not adversarial attack. At 128-bit security, collision probability is ~10⁻³⁸ at one million definitions.

2. **Performance matters for systems programming**: Blood targets safety-critical systems where compilation speed affects developer productivity. BLAKE3's ~5x speedup over SHA3 (per published benchmarks) is significant for large codebases.

3. **Smaller hash = smaller pointers**: 256-bit hashes fit better in cache and reduce memory overhead compared to 512-bit hashes. Given Blood's 128-bit fat pointers already impose overhead, minimizing hash size helps.

4. **BLAKE3 is mature and validated**: BLAKE3 was published in 2020, has an [IETF Internet-Draft (January 2025)](https://www.ietf.org/archive/id/draft-aumasson-blake3-00.html), and is widely deployed in production systems including blockchain networks.

**Migration path**: If future cryptanalysis weakens BLAKE3, Blood can extend to BLAKE3-512 using the collision-resistant storage mechanism (see §9.2). The format version field (§4.1) enables this upgrade.

### 2.4 Collision Probability

With 256-bit hashes and the birthday paradox:

```
Expected first collision at ~2^128 unique hashes
= 340,282,366,920,938,463,463,374,607,431,768,211,456

At 1 million definitions/second:
First collision expected after ~10^31 years
```

This is effectively zero probability for any practical codebase.

### 2.5 Display Format

Hashes are displayed in base32 for human readability:

```blood
// Full hash (52 characters in base32)
#a7f2k9m3xp5jht2ngqw4bc8rv6ys7dz1ef0il

// Short hash (10 characters, typical display)
#a7f2k9m3xp

// Disambiguation (when short hash collides, extend)
#a7f2k9m3xp5j
```

Base32 alphabet (RFC 4648, lowercase):
```
a b c d e f g h i j k l m n o p q r s t u v w x y z 2 3 4 5 6 7
```

---

## 3. AST Canonicalization

### 3.1 Canonicalization Goals

For the same semantic code to produce the same hash:

1. **Eliminate naming** — Local variable names replaced with de Bruijn indices
2. **Normalize structure** — Consistent AST representation
3. **Resolve dependencies** — Replace names with dependency hashes
4. **Strip metadata** — Comments, formatting, source locations removed

### 3.2 De Bruijn Indexing

Variable names are replaced with position-based indices:

```blood
// Source code
fn add(x, y) {
    x + y
}

// Canonicalized (de Bruijn indices)
fn #0(#0, #1) {
    #0 + #1
}
// #0 = first parameter, #1 = second parameter
```

This makes `add(x, y) = x + y` and `add(a, b) = a + b` identical after canonicalization.

#### 3.2.1 De Bruijn Indexing Algorithm

The de Bruijn indexing algorithm converts named variables to position-based indices. The index represents how many binders (lambdas, let-bindings, function parameters) are between the variable use and its binding site.

```
COMPUTE_DE_BRUIJN_INDICES(ast, depth_map) → CanonicalAST:
    // depth_map: Map<Name, u32> - maps variable names to their de Bruijn depth

    MATCH ast:
        // Variables: look up in depth map
        Var(name) →
            IF name IN depth_map:
                // Local variable - use de Bruijn index
                depth ← depth_map[name]
                RETURN LocalVar(depth)
            ELSE IF is_global_name(name):
                // Global reference - resolve to hash
                hash ← RESOLVE_TO_HASH(name)
                RETURN DefRef(hash)
            ELSE:
                ERROR "Unbound variable: {name}"

        // Lambda: introduce new binding at depth 0, shift others up
        Lambda(param_name, body) →
            new_map ← shift_depths(depth_map, 1)
            new_map[param_name] ← 0
            canonical_body ← COMPUTE_DE_BRUIJN_INDICES(body, new_map)
            RETURN Lambda(canonical_body)

        // Let binding: similar to lambda
        Let(name, value, body) →
            canonical_value ← COMPUTE_DE_BRUIJN_INDICES(value, depth_map)
            new_map ← shift_depths(depth_map, 1)
            new_map[name] ← 0
            canonical_body ← COMPUTE_DE_BRUIJN_INDICES(body, new_map)
            RETURN Let(canonical_value, canonical_body)

        // Function definition: parameters become sequential indices
        FnDef(params, body) →
            new_map ← {}
            FOR (i, param_name) IN params.enumerate().reversed():
                new_map[param_name] ← i
            // Shift existing bindings by param count
            FOR (name, depth) IN depth_map:
                new_map[name] ← depth + params.len()
            canonical_body ← COMPUTE_DE_BRUIJN_INDICES(body, new_map)
            RETURN FnDef(params.len(), canonical_body)

        // Pattern match: each arm introduces bindings
        Match(scrutinee, arms) →
            canonical_scrutinee ← COMPUTE_DE_BRUIJN_INDICES(scrutinee, depth_map)
            canonical_arms ← []
            FOR (pattern, guard, expr) IN arms:
                bindings ← EXTRACT_PATTERN_BINDINGS(pattern)
                new_map ← shift_depths(depth_map, bindings.len())
                FOR (i, binding_name) IN bindings.enumerate().reversed():
                    new_map[binding_name] ← i
                canonical_pattern ← CANONICALIZE_PATTERN(pattern)
                canonical_guard ← IF guard.is_some():
                    COMPUTE_DE_BRUIJN_INDICES(guard, new_map)
                canonical_expr ← COMPUTE_DE_BRUIJN_INDICES(expr, new_map)
                canonical_arms.push((canonical_pattern, canonical_guard, canonical_expr))
            RETURN Match(canonical_scrutinee, canonical_arms)

        // Application: recurse on function and arguments
        Apply(func, args) →
            canonical_func ← COMPUTE_DE_BRUIJN_INDICES(func, depth_map)
            canonical_args ← [COMPUTE_DE_BRUIJN_INDICES(arg, depth_map) FOR arg IN args]
            RETURN Apply(canonical_func, canonical_args)

        // Binary operation: recurse on operands
        BinOp(op, left, right) →
            canonical_left ← COMPUTE_DE_BRUIJN_INDICES(left, depth_map)
            canonical_right ← COMPUTE_DE_BRUIJN_INDICES(right, depth_map)
            RETURN BinOp(op, canonical_left, canonical_right)

        // Literals: return as-is
        IntLit(n) → IntLit(n)
        FloatLit(f) → FloatLit(f)
        StringLit(s) → StringLit(s)
        BoolLit(b) → BoolLit(b)

        // If-then-else: recurse on all branches
        IfThenElse(cond, then_branch, else_branch) →
            RETURN IfThenElse(
                COMPUTE_DE_BRUIJN_INDICES(cond, depth_map),
                COMPUTE_DE_BRUIJN_INDICES(then_branch, depth_map),
                COMPUTE_DE_BRUIJN_INDICES(else_branch, depth_map)
            )

        // Effect operations: recurse on arguments
        Perform(effect, op, args) →
            canonical_args ← [COMPUTE_DE_BRUIJN_INDICES(arg, depth_map) FOR arg IN args]
            effect_hash ← RESOLVE_TO_HASH(effect)
            RETURN Perform(effect_hash, op, canonical_args)

        // Handle: body and handler have different scopes
        Handle(body, handlers, return_clause) →
            canonical_body ← COMPUTE_DE_BRUIJN_INDICES(body, depth_map)
            canonical_handlers ← []
            FOR handler IN handlers:
                // Handler introduces resume and operation parameters
                handler_map ← shift_depths(depth_map, handler.params.len() + 1)
                FOR (i, param) IN handler.params.enumerate().reversed():
                    handler_map[param] ← i
                handler_map["resume"] ← handler.params.len()  // resume is special
                canonical_handler ← COMPUTE_DE_BRUIJN_INDICES(handler.body, handler_map)
                canonical_handlers.push(canonical_handler)
            // Return clause has single parameter
            return_map ← shift_depths(depth_map, 1)
            return_map[return_clause.param] ← 0
            canonical_return ← COMPUTE_DE_BRUIJN_INDICES(return_clause.body, return_map)
            RETURN Handle(canonical_body, canonical_handlers, canonical_return)


SHIFT_DEPTHS(depth_map, amount) → Map<Name, u32>:
    new_map ← {}
    FOR (name, depth) IN depth_map:
        new_map[name] ← depth + amount
    RETURN new_map


EXTRACT_PATTERN_BINDINGS(pattern) → List<Name>:
    // Returns bindings in left-to-right order
    MATCH pattern:
        VarPattern(name) → [name]
        WildcardPattern → []
        LiteralPattern(_) → []
        TuplePattern(patterns) →
            bindings ← []
            FOR p IN patterns:
                bindings.extend(EXTRACT_PATTERN_BINDINGS(p))
            RETURN bindings
        ConstructorPattern(_, patterns) →
            bindings ← []
            FOR p IN patterns:
                bindings.extend(EXTRACT_PATTERN_BINDINGS(p))
            RETURN bindings
        AsPattern(inner, name) →
            bindings ← EXTRACT_PATTERN_BINDINGS(inner)
            bindings.push(name)
            RETURN bindings
```

**Example Trace**:

```
Source: fn foo(x, y) { let z = x + y; z * 2 }

Step 1: FnDef(["x", "y"], Let("z", BinOp(+, Var("x"), Var("y")), BinOp(*, Var("z"), IntLit(2))))

Step 2: Process FnDef
  - Create depth_map: {"x": 1, "y": 0}  // rightmost param is index 0

Step 3: Process Let
  - Process value (x + y) with depth_map {"x": 1, "y": 0}
    - Var("x") → LocalVar(1)
    - Var("y") → LocalVar(0)
    - Result: BinOp(+, LocalVar(1), LocalVar(0))
  - Shift depths for body: {"x": 2, "y": 1, "z": 0}
  - Process body (z * 2) with new depth_map
    - Var("z") → LocalVar(0)
    - IntLit(2) → IntLit(2)
    - Result: BinOp(*, LocalVar(0), IntLit(2))

Step 4: Final canonical form:
  FnDef(2, Let(BinOp(+, LocalVar(1), LocalVar(0)), BinOp(*, LocalVar(0), IntLit(2))))
```

### 3.3 Dependency Resolution

All references to other definitions are replaced by their hashes:

```blood
// Source
fn double(x) = add(x, x)

// Canonicalized
fn #0(#0) = #a7f2k9...(#0, #0)
//          ↑ hash of 'add'
```

### 3.4 Canonicalization Algorithm

```
CANONICALIZE(definition) → CanonicalAST:
    1. Parse to raw AST
    2. Resolve all name references to hashes
    3. Convert local names to de Bruijn indices
    4. Normalize type expressions
    5. Strip comments, whitespace, source locations
    6. Sort unordered elements (struct fields in some contexts)
    7. Apply effect normalization
    8. Return canonical AST

RESOLVE_REFERENCE(name, scope) → Hash:
    IF name is local variable:
        RETURN DeBruijnIndex(name, scope)
    ELSE IF name is imported definition:
        RETURN lookup_hash(name)
    ELSE IF name is builtin:
        RETURN BUILTIN_HASH(name)
    ELSE:
        ERROR "Unresolved reference: {name}"
```

### 3.5 Type Canonicalization

Type expressions are also canonicalized:

```blood
// These are identical after canonicalization:
type Pair<A, B> = { first: A, second: B }
type Pair<X, Y> = { first: X, second: Y }

// Canonicalized form:
type #struct(#0: $0, #1: $1)
// $0, $1 = type parameters by position
```

#### 3.5.1 Type Canonicalization Algorithm

Types are canonicalized to ensure identical semantic types produce identical representations.

```
CANONICALIZE_TYPE(type_ast, type_param_map) → CanonicalTypeAST:
    // type_param_map: Map<TypeParamName, u32> - maps type parameter names to indices

    MATCH type_ast:
        // Primitive types: use fixed identifiers
        PrimitiveType(name) →
            MATCH name:
                "i8"    → PrimitiveRef(0x01)
                "i16"   → PrimitiveRef(0x02)
                "i32"   → PrimitiveRef(0x03)
                "i64"   → PrimitiveRef(0x04)
                "i128"  → PrimitiveRef(0x05)
                "u8"    → PrimitiveRef(0x11)
                "u16"   → PrimitiveRef(0x12)
                "u32"   → PrimitiveRef(0x13)
                "u64"   → PrimitiveRef(0x14)
                "u128"  → PrimitiveRef(0x15)
                "f32"   → PrimitiveRef(0x21)
                "f64"   → PrimitiveRef(0x22)
                "bool"  → PrimitiveRef(0x31)
                "char"  → PrimitiveRef(0x32)
                "str"   → PrimitiveRef(0x33)
                "never" → PrimitiveRef(0x00)
                _       → ERROR "Unknown primitive: {name}"

        // Type variables: convert to positional indices
        TypeVar(name) →
            IF name IN type_param_map:
                RETURN TypeVarRef(type_param_map[name])
            ELSE:
                ERROR "Unbound type variable: {name}"

        // Named types: resolve to hash
        NamedType(name) →
            hash ← RESOLVE_TYPE_TO_HASH(name)
            RETURN TypeDefRef(hash)

        // Function types: canonicalize params, return, and effects
        FunctionType(params, return_type, effect_row) →
            canonical_params ← []
            FOR param_type IN params:
                canonical_params.push(CANONICALIZE_TYPE(param_type, type_param_map))
            canonical_return ← CANONICALIZE_TYPE(return_type, type_param_map)
            canonical_effects ← CANONICALIZE_EFFECT_ROW(effect_row, type_param_map)
            RETURN FunctionTypeCanon(canonical_params, canonical_return, canonical_effects)

        // Generic type application: List<Int>, Option<T>
        TypeApp(constructor, type_args) →
            canonical_constructor ← CANONICALIZE_TYPE(constructor, type_param_map)
            canonical_args ← []
            FOR arg IN type_args:
                canonical_args.push(CANONICALIZE_TYPE(arg, type_param_map))
            RETURN TypeAppCanon(canonical_constructor, canonical_args)

        // Tuple types: (A, B, C)
        TupleType(element_types) →
            canonical_elements ← []
            FOR elem IN element_types:
                canonical_elements.push(CANONICALIZE_TYPE(elem, type_param_map))
            RETURN TupleTypeCanon(canonical_elements)

        // Record types: { field1: T1, field2: T2 }
        RecordType(fields) →
            // Sort fields by canonical field ID (not by name)
            canonical_fields ← []
            FOR (field_name, field_type) IN SORT_BY_FIELD_HASH(fields):
                field_id ← COMPUTE_FIELD_ID(field_name)
                canonical_type ← CANONICALIZE_TYPE(field_type, type_param_map)
                canonical_fields.push((field_id, canonical_type))
            RETURN RecordTypeCanon(canonical_fields)

        // Reference types: &T, &mut T
        RefType(inner, mutability) →
            canonical_inner ← CANONICALIZE_TYPE(inner, type_param_map)
            RETURN RefTypeCanon(canonical_inner, mutability)

        // Pointer types: *const T, *mut T
        PtrType(inner, mutability) →
            canonical_inner ← CANONICALIZE_TYPE(inner, type_param_map)
            RETURN PtrTypeCanon(canonical_inner, mutability)

        // Array types: [T; N]
        ArrayType(element_type, size) →
            canonical_element ← CANONICALIZE_TYPE(element_type, type_param_map)
            RETURN ArrayTypeCanon(canonical_element, size)

        // Slice types: [T]
        SliceType(element_type) →
            canonical_element ← CANONICALIZE_TYPE(element_type, type_param_map)
            RETURN SliceTypeCanon(canonical_element)

        // Forall (polymorphic) types: forall<T, U>. T -> U
        ForallType(type_params, inner_type) →
            // Create new type_param_map with indices for these params
            new_map ← type_param_map.clone()
            base_index ← type_param_map.max_index() + 1
            FOR (i, param_name) IN type_params.enumerate():
                new_map[param_name] ← base_index + i
            canonical_inner ← CANONICALIZE_TYPE(inner_type, new_map)
            RETURN ForallTypeCanon(type_params.len(), canonical_inner)

        // Constrained types: T where T: Display
        ConstrainedType(inner, constraints) →
            canonical_inner ← CANONICALIZE_TYPE(inner, type_param_map)
            canonical_constraints ← []
            FOR constraint IN SORT_BY_HASH(constraints):
                trait_hash ← RESOLVE_TRAIT_TO_HASH(constraint.trait_name)
                canonical_constraints.push((constraint.type_var_index, trait_hash))
            RETURN ConstrainedTypeCanon(canonical_inner, canonical_constraints)


COMPUTE_FIELD_ID(field_name) → u32:
    // Field IDs are deterministic hashes of field names
    // This ensures field order is consistent across different source files
    hash ← BLAKE3_256(field_name.as_bytes())
    RETURN u32_from_bytes(hash[0..4])


SORT_BY_FIELD_HASH(fields) → List<(String, TypeAST)>:
    // Sort fields by their hash-based ID for deterministic ordering
    RETURN fields.sort_by(|(name, _)| COMPUTE_FIELD_ID(name))
```

### 3.6 Effect Row Canonicalization

Effect rows are normalized to a canonical order:

```blood
// These are identical:
fn f() / {IO, State<Int>}
fn f() / {State<Int>, IO}

// Canonicalized (sorted by effect hash):
fn f() / {#abc123..., #def456...}
```

Sorting rule: Effects sorted lexicographically by their hash.

#### 3.6.1 Effect Row Sorting Algorithm

Effect rows must be sorted to ensure the same set of effects always produces the same canonical representation, regardless of declaration order.

```
CANONICALIZE_EFFECT_ROW(effect_row, type_param_map) → CanonicalEffectRow:
    MATCH effect_row:
        // Pure: no effects
        PureEffect →
            RETURN CanonicalPure

        // Concrete effect set: {IO, State<T>}
        EffectSet(effects) →
            canonical_effects ← []
            FOR effect IN effects:
                canonical_effects.push(CANONICALIZE_SINGLE_EFFECT(effect, type_param_map))
            // Sort by hash for deterministic ordering
            sorted_effects ← SORT_EFFECTS_BY_HASH(canonical_effects)
            RETURN CanonicalEffectSet(sorted_effects)

        // Effect variable: ε
        EffectVar(name) →
            // Effect variables are also indexed like type variables
            IF name IN type_param_map:
                RETURN CanonicalEffectVar(type_param_map[name])
            ELSE:
                ERROR "Unbound effect variable: {name}"

        // Open effect row: {IO | ε} (concrete effects plus variable)
        OpenEffectRow(effects, tail_var) →
            canonical_effects ← []
            FOR effect IN effects:
                canonical_effects.push(CANONICALIZE_SINGLE_EFFECT(effect, type_param_map))
            sorted_effects ← SORT_EFFECTS_BY_HASH(canonical_effects)
            canonical_tail ← type_param_map[tail_var]
            RETURN CanonicalOpenRow(sorted_effects, canonical_tail)


CANONICALIZE_SINGLE_EFFECT(effect, type_param_map) → CanonicalEffect:
    MATCH effect:
        // Simple effect: IO, Async
        SimpleEffect(name) →
            hash ← RESOLVE_EFFECT_TO_HASH(name)
            RETURN CanonicalSimpleEffect(hash)

        // Parameterized effect: State<Int>, Error<MyError>
        ParameterizedEffect(name, type_args) →
            effect_hash ← RESOLVE_EFFECT_TO_HASH(name)
            canonical_args ← []
            FOR arg IN type_args:
                canonical_args.push(CANONICALIZE_TYPE(arg, type_param_map))
            RETURN CanonicalParamEffect(effect_hash, canonical_args)


SORT_EFFECTS_BY_HASH(effects) → List<CanonicalEffect>:
    // Sort effects by their complete serialized representation's hash
    // This ensures deterministic ordering even for parameterized effects

    RETURN effects.sort_by(|effect| {
        buffer ← serialize_effect(effect)
        BLAKE3_256(buffer)
    })


COMPUTE_EFFECT_HASH(effect) → Hash:
    // The hash of an effect includes its definition and type arguments
    MATCH effect:
        CanonicalSimpleEffect(hash) →
            RETURN hash

        CanonicalParamEffect(effect_hash, args) →
            buffer ← []
            buffer.append(effect_hash.as_bytes())
            FOR arg IN args:
                buffer.append(SERIALIZE_TYPE(arg))
            RETURN BLAKE3_256(buffer)
```

**Example: Effect Row Canonicalization**

```blood
// Source 1:
fn process() / {State<Int>, IO, Error<String>}

// Source 2:
fn process() / {Error<String>, IO, State<Int>}

// Both canonicalize to (assuming these hash values):
// IO           → #io_hash_abc
// State<Int>   → #state_int_def  (compound: State hash + Int type)
// Error<String>→ #error_str_ghi  (compound: Error hash + String type)

// Sorted by hash (lexicographic on hash bytes):
// If #error_str_ghi < #io_hash_abc < #state_int_def, then:
// Canonical: {#error_str_ghi, #io_hash_abc, #state_int_def}

// Both source files produce identical canonical form!
```

**Effect Row Equivalence**

Two effect rows are equivalent if and only if their canonical forms are byte-identical:

```
EFFECT_ROW_EQUIVALENT(row1, row2) → bool:
    canon1 ← CANONICALIZE_EFFECT_ROW(row1, {})
    canon2 ← CANONICALIZE_EFFECT_ROW(row2, {})
    RETURN SERIALIZE(canon1) == SERIALIZE(canon2)
```

### 3.7 Canonical AST Format

```rust
enum CanonicalAST {
    // Literals
    IntLit(i128),
    FloatLit(f64),
    StringLit(Vec<u8>),
    BoolLit(bool),

    // Variables (de Bruijn indexed)
    LocalVar(u32),              // #n
    TypeVar(u32),               // $n

    // References (by hash)
    DefRef(Hash),               // Reference to definition
    BuiltinRef(BuiltinId),      // Reference to builtin

    // Expressions
    Lambda(Box<CanonicalAST>),
    Apply(Box<CanonicalAST>, Box<CanonicalAST>),
    Let(Box<CanonicalAST>, Box<CanonicalAST>),
    IfThenElse(Box<CanonicalAST>, Box<CanonicalAST>, Box<CanonicalAST>),

    // Types
    TypeArrow(Box<CanonicalAST>, Box<CanonicalAST>, EffectRow),
    TypeApp(Box<CanonicalAST>, Vec<CanonicalAST>),
    TypeRecord(Vec<(FieldId, CanonicalAST)>),

    // Effects
    Perform(Hash, Vec<CanonicalAST>),
    Handle(Box<CanonicalAST>, Handler),

    // Patterns
    Pattern(PatternAST),

    // Structs/Enums
    Struct(Vec<(FieldId, CanonicalAST)>),
    Enum(VariantId, Box<CanonicalAST>),
}
```

### 3.8 Builtin Identities

Primitive operations have fixed hashes:

| Builtin | Hash (Base32 prefix) |
|---------|---------------------|
| `Int.add` | `#int_add_0001` |
| `Int.sub` | `#int_sub_0001` |
| `Int.mul` | `#int_mul_0001` |
| `Bool.and` | `#bool_and_001` |
| `Bool.or` | `#bool_or_0001` |
| ... | ... |

Builtin hashes are versioned. Changing builtin semantics requires a new version.

---

## 4. Hash Computation Algorithm

### 4.1 Serialization Format

The canonical AST is serialized to bytes before hashing:

```
SERIALIZE(ast) → Bytes:
    buffer ← empty

    // Write format version (for future compatibility)
    buffer.write_u8(FORMAT_VERSION)  // Currently: 1

    // Write AST recursively
    SERIALIZE_NODE(ast, buffer)

    RETURN buffer

SERIALIZE_NODE(node, buffer):
    // Write node type tag
    buffer.write_u8(node.tag())

    MATCH node:
        IntLit(n) →
            buffer.write_i128_le(n)

        FloatLit(f) →
            buffer.write_f64_le(f)

        StringLit(s) →
            buffer.write_u32_le(s.len())
            buffer.write_bytes(s)

        LocalVar(idx) →
            buffer.write_u32_le(idx)

        DefRef(hash) →
            buffer.write_bytes(hash.bytes())  // 32 bytes

        Lambda(body) →
            SERIALIZE_NODE(body, buffer)

        Apply(func, arg) →
            SERIALIZE_NODE(func, buffer)
            SERIALIZE_NODE(arg, buffer)

        // ... etc for all node types
```

### 4.2 Hash Computation

```
COMPUTE_HASH(definition) → Hash:
    canonical ← CANONICALIZE(definition)
    bytes ← SERIALIZE(canonical)
    hash ← BLAKE3_256(bytes)
    RETURN hash
```

### 4.3 Incremental Hashing

For large definitions, use BLAKE3's streaming interface:

```blood
fn compute_hash_streaming(ast: &CanonicalAST) -> Hash {
    let mut hasher = Blake3::new();

    // Version prefix
    hasher.update(&[FORMAT_VERSION]);

    // Stream AST nodes
    serialize_streaming(ast, &mut hasher);

    hasher.finalize()
}
```

### 4.4 Determinism Guarantees

The hash computation is deterministic if:

1. ✅ Same source code → same canonical AST
2. ✅ Same canonical AST → same serialization
3. ✅ Same serialization → same hash (BLAKE3 is deterministic)

**Non-determinism sources (avoided by design)**:
- ❌ HashMap iteration order → Use sorted structures
- ❌ Pointer addresses → Use indices
- ❌ Floating-point → Use IEEE 754 canonical form
- ❌ Source file encoding → Normalize to UTF-8

### 4.5 Determinism Proof Sketch

This section provides a proof sketch that the canonicalization and hashing pipeline is deterministic.

#### 4.5.1 Theorem: Canonicalization Determinism

**Theorem**: For any syntactically valid Blood definition `D`, the canonicalization function `CANONICALIZE(D)` produces a unique canonical AST `C` such that:

```
∀ D₁, D₂: SEMANTICALLY_EQUAL(D₁, D₂) ⟹ CANONICALIZE(D₁) = CANONICALIZE(D₂)
```

**Proof Sketch**:

We prove by structural induction on the AST that each canonicalization step is deterministic.

**Base Cases (Leaves)**:

1. **Literals** (IntLit, FloatLit, StringLit, BoolLit):
   - Identity transformation: `CANONICALIZE(Lit(v)) = Lit(v)`
   - Deterministic: Same literal value → same canonical form

2. **Variables** (Var):
   - De Bruijn indexing: `CANONICALIZE(Var(name)) = LocalVar(depth_map[name])`
   - Deterministic if `depth_map` is deterministic (see Lemma 1)

**Inductive Cases (Internal Nodes)**:

3. **Lambda**: `CANONICALIZE(Lambda(p, b)) = Lambda(CANONICALIZE(b, updated_depth_map))`
   - By IH, `CANONICALIZE(b, ...)` is deterministic
   - `updated_depth_map` is deterministically computed from `depth_map` and `p`
   - ∴ Deterministic

4. **Application**: `CANONICALIZE(App(f, a)) = App(CANONICALIZE(f), CANONICALIZE(a))`
   - By IH, both sub-canonicalizations are deterministic
   - ∴ Deterministic

5. **Let**: Similar to Lambda

6. **Match**: Arms are processed in source order; bindings within each arm are processed left-to-right
   - Source order is preserved by parser
   - Left-to-right binding order is specified
   - ∴ Deterministic

7. **Record/Struct Fields**:
   - Fields sorted by `COMPUTE_FIELD_ID(field_name)`
   - `COMPUTE_FIELD_ID` is deterministic (BLAKE3 hash of name bytes)
   - ∴ Field order is deterministic regardless of source order

8. **Effect Rows**:
   - Effects sorted by `SORT_EFFECTS_BY_HASH`
   - Hash comparison is deterministic
   - ∴ Effect order is deterministic regardless of source order

**Lemma 1: Depth Map Determinism**

The depth map at any point in canonicalization depends only on:
1. The set of enclosing binders (lambdas, let-bindings, match arms)
2. The nesting depth of each binder

Both are determined by the AST structure, which is fixed for a given definition.
∴ The depth map is deterministic. ∎

#### 4.5.2 Theorem: Hash Determinism

**Theorem**: For any canonical AST `C`, the hash function `COMPUTE_HASH(C)` produces a unique hash `H`:

```
∀ C₁, C₂: C₁ = C₂ ⟹ COMPUTE_HASH(C₁) = COMPUTE_HASH(C₂)
```

**Proof Sketch**:

1. **Serialization Determinism**:
   - `SERIALIZE(C)` traverses `C` in a fixed order (pre-order DFS)
   - Each node type has a unique tag
   - Each node's fields are serialized in a fixed order
   - ∴ Same canonical AST → same byte sequence

2. **Hash Function Determinism**:
   - BLAKE3 is a deterministic function
   - Same byte sequence → same hash
   - ∴ Deterministic

3. **Composition**:
   - `COMPUTE_HASH(C) = BLAKE3(SERIALIZE(C))`
   - Both components are deterministic
   - ∴ Composition is deterministic ∎

#### 4.5.3 Corollary: Alpha-Equivalence

**Corollary**: Alpha-equivalent definitions (differing only in variable names) produce identical hashes.

```
fn add(x, y) = x + y   ──► Hash: #abc123
fn add(a, b) = a + b   ──► Hash: #abc123  (same!)
```

**Proof**: De Bruijn indexing eliminates variable names. Both definitions canonicalize to:
```
FnDef(2, BinOp(+, LocalVar(1), LocalVar(0)))
```
Same canonical form → same serialization → same hash. ∎

#### 4.5.4 Potential Non-Determinism Sources and Mitigations

| Potential Source | Mitigation | Status |
|------------------|------------|--------|
| HashMap iteration | Use `BTreeMap` or sorted vectors | ✅ Addressed |
| HashSet iteration | Convert to sorted vector before serialization | ✅ Addressed |
| Floating-point NaN | Canonicalize to single NaN representation | ✅ Addressed |
| Unicode normalization | Normalize to NFC before processing | ✅ Addressed |
| Thread scheduling | Canonicalization is single-threaded | ✅ Addressed |
| Global state | Canonicalization is pure (no side effects) | ✅ Addressed |
| Time/randomness | Not used in canonicalization | ✅ Addressed |

---

## 5. Storage Format

### 5.1 Codebase Database

Blood stores code in a content-addressed database, not files:

```
~/.blood/codebases/
├── default/
│   ├── terms/           # Definition storage
│   │   ├── a7/
│   │   │   └── f2k9m3...  # Hash-addressed
│   │   ├── b3/
│   │   │   └── c1xp5j...
│   │   └── ...
│   ├── types/           # Type definitions
│   ├── effects/         # Effect definitions
│   ├── names/           # Name → Hash mappings
│   │   ├── main.idx
│   │   └── lib.idx
│   ├── metadata/        # Comments, docs, locations
│   └── compiled/        # Compiled artifacts (by hash)
└── projects/
    └── myproject/
        └── namespace.idx  # Project namespace
```

### 5.2 Definition Record Format

Each stored definition contains:

```rust
struct DefinitionRecord {
    // Core identity
    hash: Hash,                     // 32 bytes

    // Canonical representation
    canonical_ast: CanonicalAST,    // Variable size

    // Type information
    type_signature: TypeAST,        // Variable size
    effect_signature: EffectRow,    // Variable size

    // Dependencies
    references: Vec<Hash>,          // Other definitions used

    // Optional metadata (not part of hash)
    metadata: Option<Metadata>,
}

struct Metadata {
    names: Vec<Name>,               // Human-readable names
    documentation: Option<String>,  // Doc comments
    source_location: Option<SourceLoc>,
    created_at: Timestamp,
    author: Option<String>,
}
```

### 5.3 Index Files

Name-to-hash mappings are stored in index files:

```
# names/main.idx (human-readable format)
# Namespace: main

add : #a7f2k9m3xp
subtract : #b3c1xp5jht
multiply : #c4d2yr6kiu

# Type aliases
type Point : #e5f3zs7lmn
type Vector : #f6g4at8mop

# Effects
effect IO : #g7h5bu9npq
effect State : #h8i6cv0oqr
```

### 5.4 Binary Storage Format

For efficiency, definitions are stored in a binary format:

```
┌────────────────────────────────────────────────────────────┐
│ DEFINITION BINARY FORMAT                                   │
├────────────────────────────────────────────────────────────┤
│ Offset │ Size │ Field                                      │
├────────┼──────┼────────────────────────────────────────────┤
│ 0      │ 4    │ Magic: "BLOD" (0x424C4F44)                 │
│ 4      │ 1    │ Format version                             │
│ 5      │ 32   │ Hash (BLAKE3-256)                          │
│ 37     │ 4    │ AST size (bytes)                           │
│ 41     │ N    │ Canonical AST (serialized)                 │
│ 41+N   │ 4    │ Type size (bytes)                          │
│ 45+N   │ M    │ Type signature (serialized)                │
│ 45+N+M │ 4    │ References count                           │
│ 49+N+M │ 32*R │ Reference hashes                           │
│ ...    │ ...  │ Optional metadata (if present)             │
└────────────────────────────────────────────────────────────┘
```

### 5.5 Format Version Migration

The storage format may evolve over time. This section specifies the migration strategy.

#### 5.5.1 Format Version History

| Version | Introduced | Changes |
|---------|------------|---------|
| 1 | Initial | BLAKE3-256, basic canonicalization |
| 2 | (Planned) | Extended effect row encoding |
| 3 | (Reserved) | Potential BLAKE3-512 upgrade |

#### 5.5.2 Version Compatibility Matrix

| Reader Version | Writer Version 1 | Writer Version 2 | Writer Version 3 |
|----------------|------------------|------------------|------------------|
| 1 | ✅ Read | ❌ Fail | ❌ Fail |
| 2 | ✅ Read | ✅ Read | ❌ Fail |
| 3 | ✅ Read | ✅ Read | ✅ Read |

**Forward compatibility**: Readers can always read older versions.
**Backward compatibility**: Readers cannot read newer versions (must upgrade).

#### 5.5.3 Migration Algorithm

```
MIGRATE_DEFINITION(old_bytes, target_version) → NewBytes | Error:
    // Read header
    magic ← old_bytes[0..4]
    IF magic ≠ "BLOD":
        RETURN Error::InvalidFormat

    source_version ← old_bytes[4]

    IF source_version == target_version:
        // No migration needed
        RETURN old_bytes

    IF source_version > target_version:
        RETURN Error::CannotDowngrade

    // Parse with source version
    definition ← PARSE_DEFINITION(old_bytes, source_version)

    // Apply incremental migrations
    current ← definition
    FOR v IN (source_version + 1)..=target_version:
        current ← MIGRATE_ONE_VERSION(current, v - 1, v)

    // Serialize with target version
    RETURN SERIALIZE_DEFINITION(current, target_version)


MIGRATE_ONE_VERSION(definition, from_version, to_version) → Definition:
    MATCH (from_version, to_version):
        (1, 2) →
            // Version 1 → 2: Extended effect row encoding
            FOR effect_row IN definition.all_effect_rows():
                // Old: effect rows stored as flat list
                // New: effect rows have explicit open/closed marker
                new_row ← CONVERT_TO_MARKED_EFFECT_ROW(effect_row)
                REPLACE(effect_row, new_row)
            RETURN definition

        (2, 3) →
            // Version 2 → 3: Hash algorithm upgrade (if needed)
            // This migration would require rehashing all content
            // Only triggered if BLAKE3-256 is compromised
            new_hash ← BLAKE3_512(definition.canonical_bytes)
            definition.extended_hash ← new_hash
            RETURN definition

        _ →
            ERROR "Unknown migration path: {from_version} → {to_version}"
```

#### 5.5.4 Codebase-Wide Migration

When upgrading a codebase to a new format version:

```
MIGRATE_CODEBASE(codebase_path, target_version) → Result:
    // 1. Create backup
    backup_path ← codebase_path + ".backup." + timestamp()
    COPY_DIRECTORY(codebase_path, backup_path)

    // 2. Discover all definitions
    definitions ← LIST_ALL_DEFINITIONS(codebase_path)

    // 3. Migrate in dependency order (leaves first)
    sorted ← TOPOLOGICAL_SORT(definitions, by_dependencies)

    FOR definition IN sorted:
        old_bytes ← READ(definition.path)
        new_bytes ← MIGRATE_DEFINITION(old_bytes, target_version)?

        // Hashes may change during migration (version 3)
        old_hash ← definition.hash
        new_hash ← EXTRACT_HASH(new_bytes)

        IF old_hash ≠ new_hash:
            // Update all references to this definition
            UPDATE_REFERENCES(codebase_path, old_hash, new_hash)

        WRITE(definition.path, new_bytes)

    // 4. Update codebase metadata
    SET_CODEBASE_VERSION(codebase_path, target_version)

    // 5. Verify integrity
    IF NOT VERIFY_CODEBASE_INTEGRITY(codebase_path):
        // Rollback
        REMOVE_DIRECTORY(codebase_path)
        RENAME(backup_path, codebase_path)
        RETURN Error::MigrationFailed

    RETURN Ok(())
```

#### 5.5.5 Hash Algorithm Migration (Hypothetical)

If BLAKE3-256 is ever compromised, Blood can upgrade to BLAKE3-512:

```
HASH_UPGRADE_STRATEGY:

Phase 1: Dual-Hash Period
  - New definitions stored with both BLAKE3-256 and BLAKE3-512
  - Lookups try BLAKE3-256 first, fall back to BLAKE3-512
  - Index maps old_hash → new_hash

Phase 2: Migration Window
  - Users prompted to migrate codebases
  - New codebases created with BLAKE3-512 only
  - Legacy codebases continue working with dual hashes

Phase 3: Deprecation
  - BLAKE3-256-only definitions no longer accepted
  - All codebases must be migrated
  - Old hashes stored in compatibility index only

Phase 4: Removal
  - BLAKE3-256 support removed
  - Compatibility index archived
  - Only BLAKE3-512 supported
```

**Timeline**: Phases would be measured in years, not months, to allow ecosystem migration.

#### 5.5.6 Migration CLI

```bash
# Check current format version
$ marrow format-version
Format version: 1
Compatible with: Blood 0.1.0 - 0.3.x

# Preview migration
$ marrow migrate --preview --target=2
Would migrate 1,234 definitions
Estimated time: 12 seconds
Hash changes: 0 (format-only migration)

# Perform migration
$ marrow migrate --target=2
Creating backup at ~/.blood/codebases/myproject.backup.20260110
Migrating definitions... [████████████████████] 1234/1234
Verifying integrity... OK
Migration complete. Format version: 2

# Verify codebase
$ marrow verify
Checking hashes... OK
Checking references... OK
Checking indices... OK
Codebase verified: 1,234 definitions, 0 errors
```

---

## 6. Virtual Function Table (VFT)

### 6.1 VFT Overview

The Virtual Function Table maps content hashes to runtime entry points:

```
┌─────────────────────────────────────────────────────────────┐
│                    VIRTUAL FUNCTION TABLE                    │
├─────────────────────────────────────────────────────────────┤
│ Hash                    │ Entry Point    │ Metadata          │
├─────────────────────────┼────────────────┼───────────────────┤
│ #a7f2k9m3xp...          │ 0x00401000     │ { arity: 2 }      │
│ #b3c1xp5jht...          │ 0x00401050     │ { arity: 2 }      │
│ #c4d2yr6kiu...          │ 0x004010A0     │ { arity: 2 }      │
└─────────────────────────────────────────────────────────────┘
```

### 6.2 VFT Structure

```rust
struct VFT {
    // Hash-indexed lookup
    entries: HashMap<Hash, VFTEntry>,

    // For iteration and debugging
    all_entries: Vec<VFTEntry>,

    // Hot-swap support
    version: u64,
    pending_updates: Vec<VFTUpdate>,
}

struct VFTEntry {
    hash: Hash,
    entry_point: *const fn(),   // Native code pointer
    arity: u8,                  // Number of arguments
    calling_convention: CallingConvention,
    effects: EffectMask,        // Bit mask of effect categories
    compiled_size: u32,         // Size of compiled code
}

enum CallingConvention {
    Blood,      // Standard Blood calling convention
    Tail,       // Tail-call optimized
    Effect,     // Effect-aware (captures continuation)
    Foreign,    // FFI callback
}
```

### 6.3 VFT Operations

```
VFT_LOOKUP(hash) → Option<VFTEntry>:
    RETURN vft.entries.get(hash)

VFT_CALL(hash, args) → Result:
    entry ← VFT_LOOKUP(hash)
    IF entry.is_none():
        RETURN Err(UndefinedFunction(hash))

    // Check arity
    IF args.len() ≠ entry.arity:
        RETURN Err(ArityMismatch)

    // Call through entry point
    RETURN entry.entry_point(args)

VFT_REGISTER(hash, entry_point, metadata):
    entry ← VFTEntry {
        hash,
        entry_point,
        arity: metadata.arity,
        ...
    }
    vft.entries.insert(hash, entry)
    vft.all_entries.push(entry)
```

### 6.4 Lazy Loading

Definitions are loaded on demand:

```
VFT_LAZY_LOOKUP(hash) → VFTEntry:
    IF hash NOT IN vft.entries:
        // Load and compile on demand
        definition ← LOAD_DEFINITION(hash)
        code ← COMPILE(definition)
        entry ← CREATE_ENTRY(hash, code)
        vft.entries.insert(hash, entry)

    RETURN vft.entries.get(hash)
```

### 6.5 Dispatch Optimization

For multiple dispatch, the VFT supports type-indexed lookup:

```rust
struct DispatchTable {
    // Method hash → dispatch entries
    methods: HashMap<Hash, Vec<DispatchEntry>>,
}

struct DispatchEntry {
    // Type signature for matching
    type_pattern: TypePattern,

    // Specialization specificity (for ordering)
    specificity: u32,

    // Implementation hash
    impl_hash: Hash,
}

fn dispatch(method_hash: Hash, arg_types: &[Type]) -> Hash {
    let entries = dispatch_table.methods.get(method_hash)?;

    // Find most specific matching implementation
    entries
        .iter()
        .filter(|e| e.type_pattern.matches(arg_types))
        .max_by_key(|e| e.specificity)
        .map(|e| e.impl_hash)
}
```

---

## 7. Codebase Operations

### 7.1 Adding Definitions

```blood
// User writes:
fn factorial(n: Int) -> Int / pure {
    if n <= 1 { 1 }
    else { n * factorial(n - 1) }
}

// System performs:
// 1. Parse to AST
// 2. Type check
// 3. Canonicalize
// 4. Compute hash: #qr7st8uvwx
// 5. Store in codebase
// 6. Add name mapping: factorial → #qr7st8uvwx
```

### 7.2 Renaming Definitions

Renaming only changes metadata, not the hash:

```blood
// Before
factorial : #qr7st8uvwx

// Rename to 'fact'
fact : #qr7st8uvwx  // Same hash!

// Old name can be kept as alias
factorial = fact  // Alias mapping
```

### 7.3 Updating Definitions

Changing a definition creates a new hash:

```blood
// Original
fn add(x: Int, y: Int) = x + y  // Hash: #a7f2k9

// Updated (added logging)
fn add(x: Int, y: Int) / {Log} = {
    log("Adding {} + {}", x, y)
    x + y
}  // Hash: #b3c1xp (different!)

// Both versions exist in codebase
// Name 'add' now points to #b3c1xp
// Dependents using #a7f2k9 still work (use old version)
```

### 7.4 Propagating Updates

When a definition changes, dependents can be updated:

```
UPDATE_PROPAGATION(old_hash, new_hash):
    dependents ← FIND_DEPENDENTS(old_hash)

    FOR EACH dependent IN dependents:
        // Offer to update
        new_dependent ← REPLACE_REFERENCE(dependent, old_hash, new_hash)
        new_dependent_hash ← COMPUTE_HASH(new_dependent)

        // Recursively propagate
        UPDATE_PROPAGATION(dependent.hash, new_dependent_hash)
```

### 7.5 Garbage Collection

Unreferenced definitions can be garbage collected:

```
COLLECT_GARBAGE(codebase):
    roots ← codebase.named_definitions()  // All definitions with names
    reachable ← TRANSITIVE_CLOSURE(roots)

    FOR EACH definition IN codebase.all_definitions():
        IF definition.hash NOT IN reachable:
            IF definition.age > RETENTION_PERIOD:
                codebase.delete(definition.hash)
```

---

## 8. Hot-Swapping

### 8.1 Zero-Downtime Updates

Blood supports updating running code without restart:

```
┌─────────────────────────────────────────────────────────────┐
│                    HOT-SWAP SEQUENCE                         │
│                                                              │
│  1. Load new definition (#new_hash)                          │
│  2. Compile new code                                         │
│  3. Atomic VFT update:                                       │
│     - Old: method → #old_hash                                │
│     - New: method → #new_hash                                │
│  4. In-flight calls complete with old code                   │
│  5. New calls use new code                                   │
│  6. Old code garbage collected when unreferenced             │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

### 8.2 Swap Semantics

```rust
enum SwapMode {
    // Immediate: new calls use new code, in-flight continue with old
    Immediate,

    // Barrier: wait for in-flight calls to complete, then swap
    Barrier,

    // Epoch: swap at next epoch boundary (for consistency)
    Epoch,
}

fn hot_swap(old_hash: Hash, new_hash: Hash, mode: SwapMode) -> Result<()> {
    // Validate new definition is type-compatible
    validate_compatibility(old_hash, new_hash)?;

    match mode {
        SwapMode::Immediate => {
            vft.atomic_update(old_hash, new_hash);
        }
        SwapMode::Barrier => {
            vft.prepare_update(old_hash, new_hash);
            wait_for_in_flight();
            vft.commit_update();
        }
        SwapMode::Epoch => {
            vft.schedule_update(old_hash, new_hash, next_epoch());
        }
    }

    Ok(())
}
```

### 8.3 Compatibility Checking

Updates must maintain type compatibility:

```
VALIDATE_COMPATIBILITY(old_hash, new_hash) → Result:
    old_type ← GET_TYPE(old_hash)
    new_type ← GET_TYPE(new_hash)

    // Check parameter types (contravariant)
    FOR EACH (old_param, new_param) IN zip(old_type.params, new_type.params):
        IF NOT new_param <: old_param:
            RETURN Err(IncompatibleParameterType)

    // Check return type (covariant)
    IF NOT old_type.return <: new_type.return:
        RETURN Err(IncompatibleReturnType)

    // Check effects (must be subset)
    IF NOT new_type.effects ⊆ old_type.effects:
        RETURN Err(IncompatibleEffects)

    RETURN Ok(())
```

### 8.4 State Migration

For stateful updates, Blood provides migration hooks:

```blood
/// State migration for hot-swap
trait Migratable<Old, New> {
    fn migrate(old: Old) -> New / {Migrate};
}

/// Effect for migration operations
effect Migrate {
    op log_migration(from: Hash, to: Hash);
    op migration_failed(reason: String) -> never;
}
```

### 8.5 In-Flight Request Handling

A critical challenge in hot-swapping is handling requests that are in progress when code changes.

#### 8.5.1 Request Lifecycle

```
┌─────────────────────────────────────────────────────────────────────┐
│                    REQUEST LIFECYCLE VS HOT-SWAP                     │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  Request 1:  ┌───────────────────────────────────────┐               │
│              │ Start │ Processing... │ Complete     │               │
│              └───────────────────────────────────────┘               │
│                              ↑                                       │
│  Hot-Swap:   ─────────────── │ ────────────────────────────────     │
│                              │                                       │
│  Request 2:                  │  ┌────────────────────────────┐      │
│                              │  │ Start │ Processing │ Done │      │
│                              │  └────────────────────────────┘      │
│                                                                      │
│  Question: What code version does Request 2 use?                     │
└─────────────────────────────────────────────────────────────────────┘
```

#### 8.5.2 Swap Strategies

**Strategy 1: Immediate (Default)**

```rust
fn swap_immediate(old_hash: Hash, new_hash: Hash) {
    // Step 1: Register new code in VFT
    vft.register(new_hash, compile(new_hash));

    // Step 2: Atomic pointer update for dispatch
    vft.redirect(old_hash, new_hash);  // Atomic CAS

    // Step 3: In-flight calls continue with old code
    // (they hold direct pointers, not going through VFT)

    // Step 4: Old code GC'd when all references drop
    vft.mark_for_gc(old_hash);
}
```

**Semantics**:
- Requests started before swap → complete with old code
- Requests started after swap → use new code
- No request sees mixed old/new code

**Strategy 2: Barrier (Consistent)**

```rust
fn swap_barrier(old_hash: Hash, new_hash: Hash) {
    // Step 1: Enter swap mode
    swap_lock.write_lock();

    // Step 2: Wait for in-flight requests to complete
    while in_flight_count.load() > 0 {
        wait();
    }

    // Step 3: Perform swap
    vft.atomic_update(old_hash, new_hash);

    // Step 4: Resume accepting requests
    swap_lock.write_unlock();
}
```

**Semantics**:
- Brief pause in request acceptance
- All requests after barrier use new code
- Guaranteed consistency point

**Strategy 3: Epoch (Scheduled)**

```rust
fn swap_epoch(old_hash: Hash, new_hash: Hash, epoch: EpochId) {
    // Schedule swap for future epoch boundary
    epoch_scheduler.schedule(epoch, move || {
        // At epoch boundary, all requests complete
        // Then swap occurs
        vft.atomic_update(old_hash, new_hash);
    });
}
```

**Semantics**:
- No disruption to current requests
- Swap occurs at natural boundary (e.g., end of batch)
- Useful for batch processing systems

#### 8.5.3 Effect Handler Interaction

When hot-swapping code with active effect handlers:

```blood
// Original handler installed
with Logger handle {
    process_request()  // Uses old 'process_request'
    // ← Hot-swap occurs here
    continue_processing()  // Which version?
}
```

**Resolution**: Handler captures function references by hash at installation time.

```rust
struct EffectHandler {
    // Function references captured at handler installation
    captured_functions: HashMap<String, Hash>,

    // These hashes don't change during hot-swap
    // Handler continues using original code
}
```

**Rule**: Effect handlers see a **consistent snapshot** of code at installation time.

#### 8.5.4 Concurrent Fiber Handling

When multiple fibers are active:

```
Fiber 1: [old_code] ────────────────────────────────► [complete]
                              ↑
Hot-Swap: ──────────────────── ────────────────────────────────
                              ↓
Fiber 2:                    [start] ───► [new_code] ──► [complete]
```

**Implementation**:

```rust
fn fiber_aware_swap(old_hash: Hash, new_hash: Hash) {
    // Each fiber maintains its own VFT view
    // Only fibers started AFTER swap see new VFT

    let swap_generation = generation_counter.fetch_add(1);

    // New VFT entry with generation
    vft.insert_versioned(new_hash, swap_generation);

    // Fibers check generation on function lookup
    // generation < swap_generation → old code
    // generation >= swap_generation → new code
}
```

#### 8.5.5 Rollback on Failure

If new code fails during swap:

```rust
fn swap_with_rollback(old_hash: Hash, new_hash: Hash) -> Result<()> {
    // Step 1: Keep old code available
    let old_entry = vft.get(old_hash).clone();

    // Step 2: Attempt swap
    vft.atomic_update(old_hash, new_hash);

    // Step 3: Run health check
    if let Err(e) = health_check() {
        // Rollback
        log::error!("Swap failed, rolling back: {}", e);
        vft.atomic_update(new_hash, old_hash);
        return Err(e);
    }

    Ok(())
}
```

#### 8.5.6 Metrics and Observability

Hot-swap events are tracked:

```blood
effect HotSwapMetrics {
    op record_swap(old: Hash, new: Hash, duration: Duration);
    op record_in_flight_wait(count: u32, duration: Duration);
    op record_rollback(reason: String);
}
```

**Exported Metrics**:
- `blood_hotswap_total` — Total swaps
- `blood_hotswap_duration_seconds` — Swap duration histogram
- `blood_hotswap_in_flight_requests` — Requests during swap
- `blood_hotswap_rollbacks` — Failed swap count

---

## 9. Collision Handling

### 9.1 Hash Collision Detection

While astronomically unlikely, collisions are handled:

```
ON_COLLISION_DETECTED(hash, def1, def2):
    // Log for investigation
    log_critical("Hash collision detected: {hash}")
    log_critical("Definition 1: {def1.canonical_form}")
    log_critical("Definition 2: {def2.canonical_form}")

    // Use extended hash for disambiguation
    extended_hash1 ← BLAKE3_512(SERIALIZE(def1))
    extended_hash2 ← BLAKE3_512(SERIALIZE(def2))

    // Store both with extended hashes
    store(extended_hash1, def1)
    store(extended_hash2, def2)

    // Update index to use extended hashes
    update_index(hash, [extended_hash1, extended_hash2])
```

### 9.2 Collision-Resistant Storage

```rust
struct CollisionResistantStore {
    // Primary index: 256-bit hash → definition
    primary: HashMap<Hash256, Definition>,

    // Collision index: 256-bit hash → [512-bit hashes]
    collisions: HashMap<Hash256, Vec<Hash512>>,

    // Extended storage: 512-bit hash → definition
    extended: HashMap<Hash512, Definition>,
}

impl CollisionResistantStore {
    fn get(&self, hash: Hash256) -> Option<&Definition> {
        if let Some(collision_hashes) = self.collisions.get(&hash) {
            // Collision detected - need more info to disambiguate
            return None;  // Caller must use extended hash
        }
        self.primary.get(&hash)
    }

    fn insert(&mut self, def: Definition) -> Hash256 {
        let hash256 = blake3_256(&def);

        if self.primary.contains_key(&hash256) {
            // Collision! Upgrade to extended hashing
            self.handle_collision(hash256, def);
        } else {
            self.primary.insert(hash256, def);
        }

        hash256
    }
}
```

### 9.3 Practical Implications

Given the collision probability (~2^-128), in practice:

- **No collision will ever occur** in normal usage
- Collision handling exists for theoretical completeness
- Implementation can defer collision handling until needed

---

## 10. Tooling Integration

### 10.1 UCM (Blood Codebase Manager)

Blood provides a CLI for codebase management:

```bash
# Initialize a new codebase
$ blood init

# Add definitions from file (names become metadata)
$ blood add src/math.blood
  Added: add #a7f2k9m3xp
  Added: multiply #b3c1xp5jht

# Find definition by name
$ blood find add
  add : #a7f2k9m3xp (src/math.blood:3)
  add : #c4d2yr6kiu (src/vector.blood:15)

# View definition by hash
$ blood view #a7f2k9m3xp
  fn add(x: Int, y: Int) -> Int / pure = x + y

# Show dependents
$ blood dependents #a7f2k9m3xp
  #qr7st8... (sum)
  #xy9ab0... (average)

# Rename (changes metadata only)
$ blood rename add addition
  Renamed: add → addition (hash unchanged)

# Garbage collect unreferenced definitions
$ blood gc
  Collected 42 unreferenced definitions

# Export to traditional file (for sharing)
$ blood export mylib > mylib.blood
```

### 10.2 Editor Integration

```json
// blood-lsp protocol extensions
{
    "method": "blood/resolveHash",
    "params": {
        "name": "add",
        "position": { "line": 10, "character": 5 }
    },
    "result": {
        "hash": "#a7f2k9m3xp",
        "definition": "fn add(x: Int, y: Int) -> Int",
        "location": { "uri": "codebase://default/terms/a7f2k9..." }
    }
}
```

### 10.3 Distributed Caching

Share compiled artifacts globally:

```
┌─────────────────────────────────────────────────────────────┐
│                   DISTRIBUTED CACHE                          │
│                                                              │
│  Local Codebase                                              │
│       ↓                                                      │
│  Need #a7f2k9m3xp (compiled for x86_64-linux)                │
│       ↓                                                      │
│  Check local cache: miss                                     │
│       ↓                                                      │
│  Check global cache: hit!                                    │
│       ↓                                                      │
│  Download pre-compiled artifact                              │
│       ↓                                                      │
│  Verify hash matches                                         │
│       ↓                                                      │
│  Use immediately                                             │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

### 10.4 CI/CD Integration

```yaml
# .blood-ci.yaml
build:
  steps:
    - blood build
    - blood test

    # Only recompile changed definitions
    - blood diff --since=$PREVIOUS_COMMIT
      | blood compile --incremental

    # Publish to shared cache
    - blood publish --to=cache.blood-lang.org

deploy:
  steps:
    # Hot-swap in production
    - blood swap --old=$RUNNING_VERSION --new=$NEW_VERSION --mode=barrier
```

### 10.5 Marrow Codebase Manager

Blood's codebase manager is called **marrow** (bone marrow = source of blood cells = source of code definitions). This section documents how marrow integrates with the content-addressed storage system.

For complete naming conventions and toolchain design, see [naming.md](./naming.md).

#### 10.5.1 Marrow Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        MARROW CODEBASE MANAGER                   │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌─────────────┐    ┌──────────────┐    ┌──────────────────┐    │
│  │ CLI Parser  │ ─► │ Command      │ ─► │ Storage Backend  │    │
│  │             │    │ Dispatcher   │    │ (SQLite)         │    │
│  └─────────────┘    └──────────────┘    └──────────────────┘    │
│         │                  │                     │               │
│         │                  ▼                     ▼               │
│         │          ┌──────────────┐    ┌──────────────────┐     │
│         │          │ Canonicalizer│    │ Hash Index       │     │
│         │          │ (§3)         │    │ Name → Hash      │     │
│         │          └──────────────┘    └──────────────────┘     │
│         │                  │                     │               │
│         │                  ▼                     ▼               │
│         │          ┌──────────────┐    ┌──────────────────┐     │
│         │          │ Hasher       │    │ Dependency Graph │     │
│         │          │ (BLAKE3-256) │    │ (DAG)            │     │
│         │          └──────────────┘    └──────────────────┘     │
│         │                                                        │
│         ▼                                                        │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                    CODEBASE DATABASE                      │    │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────────┐  │    │
│  │  │ definitions │  │ names       │  │ dependencies    │  │    │
│  │  │ (hash→ast)  │  │ (name→hash) │  │ (from→to)       │  │    │
│  │  └─────────────┘  └─────────────┘  └─────────────────┘  │    │
│  └─────────────────────────────────────────────────────────┘    │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

#### 10.5.2 Marrow Commands

| Command | Description | Relation to Content-Addressing |
|---------|-------------|-------------------------------|
| `marrow init` | Initialize codebase | Creates storage schema |
| `marrow add <file>` | Add definitions | Canonicalizes, hashes, stores |
| `marrow find <name>` | Find by name | Looks up name→hash mapping |
| `marrow view <hash>` | View definition | Retrieves by content hash |
| `marrow deps <hash>` | Show dependencies | Traverses dependency DAG |
| `marrow dependents <hash>` | Show dependents | Reverse dependency lookup |
| `marrow rename <old> <new>` | Rename definition | Updates name index only (hash unchanged) |
| `marrow gc` | Garbage collect | Removes unreferenced hashes |
| `marrow history <name>` | Show history | Lists all hashes for this name over time |
| `marrow push <remote>` | Push to remote | Syncs definitions by hash |
| `marrow pull <remote>` | Pull from remote | Fetches missing hashes |
| `marrow diff <hash1> <hash2>` | Compare definitions | Structural diff of canonical ASTs |

#### 10.5.3 Integration with Hybrid CLI

Blood provides both conventional and thematic interfaces (see [naming.md §5](./naming.md#5-hybrid-cli-design)):

```bash
# Conventional (discoverable)
$ blood add mylib.blood       # Invokes marrow internally
$ blood find myfunction
$ blood deps myfunction

# Thematic (expressive)
$ marrow add mylib.blood      # Direct marrow invocation
$ marrow find myfunction
$ marrow deps myfunction
```

Both interfaces use the same underlying storage and canonicalization.

#### 10.5.4 Bloodbank Integration

Marrow connects to **Bloodbank**, the distributed code registry:

```bash
# Publish to Bloodbank
$ marrow publish --to=bloodbank.blood-lang.org
Publishing 42 definitions...
  #a7f2k9m3... (add) - already exists
  #b3c1xp5j... (multiply) - uploaded
  #c4d2yr6k... (factorial) - uploaded

# Pull from Bloodbank
$ marrow pull std
Fetching std library from bloodbank.blood-lang.org...
  Downloaded 1,234 definitions
  Verified all hashes

# Search Bloodbank
$ marrow search "json parser"
Results from bloodbank.blood-lang.org:
  json.parse : #qr7st8uv (★ 234)
  json.stringify : #xy9ab0cd (★ 234)
  json.Value : #mn3op4rs (★ 234)
```

#### 10.5.5 Cross-References

- [naming.md](./naming.md) — Toolchain naming conventions, including marrow
- [naming.md §2](./naming.md#2-content-addressed-architecture) — Content-addressing philosophy
- [naming.md §5](./naming.md#5-hybrid-cli-design) — Hybrid CLI design
- [blood-tools/ucm/](./blood-tools/ucm/) — Marrow implementation

---

## Appendix A: Canonical AST Node Types

| Tag | Node Type | Serialized Fields |
|-----|-----------|-------------------|
| 0x01 | IntLit | i128 value |
| 0x02 | FloatLit | f64 value |
| 0x03 | StringLit | u32 length, bytes |
| 0x04 | BoolLit | u8 (0 or 1) |
| 0x05 | LocalVar | u32 de Bruijn index |
| 0x06 | TypeVar | u32 index |
| 0x07 | DefRef | 32-byte hash |
| 0x08 | BuiltinRef | u16 builtin ID |
| 0x10 | Lambda | body node |
| 0x11 | Apply | func node, arg node |
| 0x12 | Let | value node, body node |
| 0x13 | IfThenElse | cond, then, else nodes |
| 0x20 | TypeArrow | param type, return type, effect row |
| 0x21 | TypeApp | constructor, args |
| 0x22 | TypeRecord | fields (sorted by ID) |
| 0x30 | Perform | effect hash, op index, args |
| 0x31 | Handle | body, handler |
| 0x32 | Resume | value |
| 0x40 | Struct | fields (sorted by ID) |
| 0x41 | Enum | variant ID, payload |
| 0x42 | Match | scrutinee, arms |

---

## Appendix B: References

Content-addressed code design draws from:

- [Unison: The Big Idea](https://www.unison-lang.org/docs/the-big-idea/)
- [IPFS Content Addressing](https://docs.ipfs.tech/concepts/content-addressing/)
- [BLAKE3 Specification](https://github.com/BLAKE3-team/BLAKE3-specs)
- [De Bruijn Indices](https://en.wikipedia.org/wiki/De_Bruijn_index)
- [RFC 4648: Base32 Encoding](https://tools.ietf.org/html/rfc4648)

---

## Appendix C: Worked Canonicalization Example

This appendix provides a complete, step-by-step example of canonicalizing a Blood function from source code to final hash.

### C.1 Source Code

```blood
/// Computes the sum of squared differences from mean
fn variance(data: List<f64>) -> f64 / pure {
    let n = data.length()
    let mean = data.sum() / n
    let squared_diffs = data.map(|x| {
        let diff = x - mean
        diff * diff
    })
    squared_diffs.sum() / n
}
```

### C.2 Step 1: Parse to Raw AST

```
FnDef {
    name: "variance",
    params: [("data", TypeApp(Named("List"), [Named("f64")]))],
    return_type: Named("f64"),
    effects: EffectRow::Pure,
    body: Let {
        name: "n",
        value: MethodCall(Var("data"), "length", []),
        body: Let {
            name: "mean",
            value: BinOp(Div,
                MethodCall(Var("data"), "sum", []),
                Var("n")
            ),
            body: Let {
                name: "squared_diffs",
                value: MethodCall(
                    Var("data"),
                    "map",
                    [Lambda {
                        param: "x",
                        body: Let {
                            name: "diff",
                            value: BinOp(Sub, Var("x"), Var("mean")),
                            body: BinOp(Mul, Var("diff"), Var("diff"))
                        }
                    }]
                ),
                body: BinOp(Div,
                    MethodCall(Var("squared_diffs"), "sum", []),
                    Var("n")
                )
            }
        }
    }
}
```

### C.3 Step 2: Resolve External References to Hashes

Assuming these external definitions:
- `List` type: `#list_type_abc123`
- `f64` type: `#f64_prim_def456`
- `List.length`: `#list_len_ghi789`
- `List.sum`: `#list_sum_jkl012`
- `List.map`: `#list_map_mno345`

```
FnDef {
    params: [("data", TypeApp(DefRef(#list_type_abc123), [DefRef(#f64_prim_def456)]))],
    return_type: DefRef(#f64_prim_def456),
    effects: CanonicalPure,
    body: Let {
        name: "n",
        value: Apply(DefRef(#list_len_ghi789), [Var("data")]),
        body: Let {
            name: "mean",
            value: BinOp(Div,
                Apply(DefRef(#list_sum_jkl012), [Var("data")]),
                Var("n")
            ),
            body: Let {
                name: "squared_diffs",
                value: Apply(
                    DefRef(#list_map_mno345),
                    [Var("data"), Lambda { param: "x", body: ... }]
                ),
                body: BinOp(Div,
                    Apply(DefRef(#list_sum_jkl012), [Var("squared_diffs")]),
                    Var("n")
                )
            }
        }
    }
}
```

### C.4 Step 3: Convert to De Bruijn Indices

Build depth map as we traverse:

```
Initial: depth_map = { "data": 0 }

After "let n": depth_map = { "data": 1, "n": 0 }

After "let mean": depth_map = { "data": 2, "n": 1, "mean": 0 }

After "let squared_diffs": depth_map = { "data": 3, "n": 2, "mean": 1, "squared_diffs": 0 }

Inside lambda |x|: depth_map = { "data": 4, "n": 3, "mean": 2, "squared_diffs": 1, "x": 0 }

After "let diff" in lambda: depth_map = { "data": 5, "n": 4, "mean": 3, "squared_diffs": 2, "x": 1, "diff": 0 }
```

Canonical AST with de Bruijn indices:

```
FnDef(1,  // 1 parameter
    Let(
        // n = data.length()
        Apply(DefRef(#list_len_ghi789), [LocalVar(0)]),  // data is at index 0
        Let(
            // mean = data.sum() / n
            BinOp(Div,
                Apply(DefRef(#list_sum_jkl012), [LocalVar(1)]),  // data shifted to 1
                LocalVar(0)  // n at index 0
            ),
            Let(
                // squared_diffs = data.map(|x| ...)
                Apply(
                    DefRef(#list_map_mno345),
                    [
                        LocalVar(2),  // data shifted to 2
                        Lambda(
                            Let(
                                // diff = x - mean
                                BinOp(Sub, LocalVar(0), LocalVar(2)),  // x=0, mean=2 (after shift)
                                // diff * diff
                                BinOp(Mul, LocalVar(0), LocalVar(0))  // diff=0
                            )
                        )
                    ]
                ),
                // squared_diffs.sum() / n
                BinOp(Div,
                    Apply(DefRef(#list_sum_jkl012), [LocalVar(0)]),  // squared_diffs at 0
                    LocalVar(2)  // n shifted to 2
                )
            )
        )
    )
)
```

### C.5 Step 4: Canonicalize Types

```
Type: (List<f64>) -> f64 / pure

Canonical:
FunctionTypeCanon(
    params: [TypeAppCanon(DefRef(#list_type_abc123), [DefRef(#f64_prim_def456)])],
    return_type: DefRef(#f64_prim_def456),
    effects: CanonicalPure
)
```

### C.6 Step 5: Serialize to Bytes

```
Serialized bytes (hex, annotated):

01                          // FORMAT_VERSION = 1
10                          // Tag: FnDef
01 00 00 00                 // param_count = 1 (little-endian u32)
12                          // Tag: Let
11                          // Tag: Apply
07                          // Tag: DefRef
[32 bytes: #list_len_ghi789]  // hash of List.length
05                          // Tag: LocalVar
00 00 00 00                 // index = 0
12                          // Tag: Let
20                          // Tag: BinOp(Div)
11                          // Tag: Apply
07                          // Tag: DefRef
[32 bytes: #list_sum_jkl012]  // hash of List.sum
05                          // Tag: LocalVar
01 00 00 00                 // index = 1
05                          // Tag: LocalVar
00 00 00 00                 // index = 0
... // (continues for rest of AST)
```

### C.7 Step 6: Compute Hash

```
Full serialized bytes: 284 bytes (example)

BLAKE3-256 hash: e7a3f9c8b2d5146890ab7c3e5f1d2048
                 a9b0c7e8f3d2a1b0c9e8d7f6a5b4c3d2

Base32: e7a3f9c8b2d5146890ab7c3e5f1d2048...
        ↓
Short display: #e7a3f9c8b2
```

### C.8 Final Result

```
variance : #e7a3f9c8b2

Stored in codebase:
  ~/.blood/codebases/default/terms/e7/a3f9c8b2...
```

### C.9 Alpha-Equivalence Verification

This code produces the SAME hash:

```blood
fn variance(xs: List<f64>) -> f64 / pure {
    let count = xs.length()
    let avg = xs.sum() / count
    let sq = xs.map(|val| {
        let d = val - avg
        d * d
    })
    sq.sum() / count
}
```

Both canonicalize to identical de Bruijn indexed AST → same hash.

---

## Appendix D: Determinism Proofs

This appendix provides more rigorous proofs for the determinism claims in §4.5.

### D.1 Proof: De Bruijn Indexing is Deterministic

**Claim**: `COMPUTE_DE_BRUIJN_INDICES(ast, ∅) = COMPUTE_DE_BRUIJN_INDICES(ast, ∅)` for all valid ASTs.

**Proof by Strong Induction on AST Structure**:

Let `P(n)` = "For all ASTs of depth ≤ n, `COMPUTE_DE_BRUIJN_INDICES` is deterministic"

**Base Case (n=0)**: Leaf nodes
- `IntLit(v)`: Returns `IntLit(v)` — deterministic
- `StringLit(s)`: Returns `StringLit(s)` — deterministic
- `Var(name)`: Returns `LocalVar(depth_map[name])` — deterministic if `depth_map` is deterministic (Lemma below)

**Inductive Case (n=k+1)**: Assume `P(k)` holds.

For an AST of depth k+1:

*Case Lambda*:
```
COMPUTE_DE_BRUIJN_INDICES(Lambda(p, body), depth_map)
  = Lambda(COMPUTE_DE_BRUIJN_INDICES(body, shift_and_add(depth_map, p)))
```

- `shift_and_add` is deterministic (pure function on depth_map and p)
- By IH, `COMPUTE_DE_BRUIJN_INDICES(body, ...)` is deterministic
- Composition of deterministic functions is deterministic ✓

*Case Let*:
```
COMPUTE_DE_BRUIJN_INDICES(Let(name, value, body), depth_map)
  = Let(COMPUTE_DE_BRUIJN_INDICES(value, depth_map),
        COMPUTE_DE_BRUIJN_INDICES(body, shift_and_add(depth_map, name)))
```

- Both recursive calls have depth ≤ k
- By IH, both are deterministic
- ✓

*Case Apply, BinOp, IfThenElse*: Similar structure-preserving recursion.

*Case Match*:
```
Arms processed in source order (parser deterministic)
Bindings extracted left-to-right (EXTRACT_PATTERN_BINDINGS deterministic)
```
- ✓

**Lemma (Depth Map Determinism)**:

The depth map at any program point depends only on the lexical structure of enclosing binders. This structure is fixed by the AST, which is the output of a deterministic parser.

∴ Depth map is deterministic at every program point. ∎

### D.2 Proof: Serialization is Deterministic

**Claim**: `SERIALIZE(c1) = SERIALIZE(c2)` iff `c1 = c2` (for canonical ASTs).

**Proof**:

(⇐) Trivial: same input → same output.

(⇒) By induction on AST structure:

For each node type, serialization writes:
1. A unique tag byte (injective: different types → different tags)
2. Child nodes in a fixed order (defined by serialization spec)

Deserialization can therefore uniquely reconstruct the original AST.

∴ `SERIALIZE` is injective, hence deterministic. ∎

### D.3 Proof: BLAKE3 is Deterministic

**Claim**: `BLAKE3(bytes)` always produces the same hash for the same input.

**Proof**: By construction.

BLAKE3 is defined as a mathematical function based on:
1. Fixed constants (derived from nothing-up-my-sleeve numbers)
2. Deterministic operations (XOR, addition mod 2³², rotations)
3. No internal state or randomness

The specification at [github.com/BLAKE3-team/BLAKE3-specs](https://github.com/BLAKE3-team/BLAKE3-specs) defines BLAKE3 purely in terms of its inputs.

∴ BLAKE3 is deterministic. ∎

### D.4 Combined Proof: End-to-End Determinism

**Theorem**: For any valid Blood definition `D`:

```
COMPUTE_HASH(CANONICALIZE(D)) produces a unique, deterministic hash.
```

**Proof**:

```
D ─────────► CANONICALIZE ─────────► SERIALIZE ─────────► BLAKE3 ─────────► Hash
     │               │                    │                   │
     │               │                    │                   │
     ▼               ▼                    ▼                   ▼
  (input)    (deterministic,        (deterministic,     (deterministic,
             D.1-D.3)               D.2)                D.3)
```

Composition of deterministic functions is deterministic.

∴ `COMPUTE_HASH(CANONICALIZE(D))` is deterministic. ∎

---

*Last updated: 2026-01-10*

---

*This document is part of the Blood Language Specification.*
