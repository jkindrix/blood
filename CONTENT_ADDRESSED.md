# Blood Content-Addressed Storage Specification

**Version**: 0.1.0-draft
**Status**: Active Development
**Last Updated**: 2026-01-09

This document specifies Blood's content-addressed code identity system, including AST canonicalization, hash computation, storage format, and the Virtual Function Table (VFT) specification.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Hash Function](#2-hash-function)
3. [AST Canonicalization](#3-ast-canonicalization)
4. [Hash Computation Algorithm](#4-hash-computation-algorithm)
5. [Storage Format](#5-storage-format)
6. [Virtual Function Table (VFT)](#6-virtual-function-table-vft)
7. [Codebase Operations](#7-codebase-operations)
8. [Hot-Swapping](#8-hot-swapping)
9. [Collision Handling](#9-collision-handling)
10. [Tooling Integration](#10-tooling-integration)

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

### 1.3 Benefits

| Benefit | Description |
|---------|-------------|
| **No Dependency Hell** | Multiple versions of the same library coexist by hash |
| **Perfect Incremental Compilation** | Only recompile definitions whose hashes changed |
| **Safe Refactoring** | Renames never break code (names are metadata) |
| **Zero-Downtime Upgrades** | Hot-swap code by hash without restart |
| **Distributed Caching** | Share compiled artifacts by hash globally |
| **Reproducible Builds** | Same code always produces same hash |

### 1.3 How It Works

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
| **Speed** | ~5x faster | Baseline | ~3x slower |
| **Parallelism** | Native SIMD | Limited | None |
| **Security** | 128-bit (256/2) | 256-bit | 128-bit |
| **Output Size** | 256 bits | 512 bits | 256 bits |

BLAKE3 provides sufficient collision resistance for code identification while being significantly faster than alternatives.

### 2.3 Collision Probability

With 256-bit hashes and the birthday paradox:

```
Expected first collision at ~2^128 unique hashes
= 340,282,366,920,938,463,463,374,607,431,768,211,456

At 1 million definitions/second:
First collision expected after ~10^31 years
```

This is effectively zero probability for any practical codebase.

### 2.4 Display Format

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

*This document is part of the Blood Language Specification.*
