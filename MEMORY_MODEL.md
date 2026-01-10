# Blood Synthetic Safety Model (SSM)

**Version**: 0.3.0-draft
**Status**: Active Development
**Implementation**: 🔶 Scaffolded (see [IMPLEMENTATION_STATUS.md](./IMPLEMENTATION_STATUS.md))
**Last Updated**: 2026-01-10

> ⚠️ **Validation Status**: This specification describes design targets. Performance claims are **unvalidated** until empirical benchmarks are completed. Novel mechanisms (Generation Snapshots, Region Suspension) require prototype validation per [ROADMAP.md §16](./ROADMAP.md#16-novel-mechanisms-prototype-plan).

**Revision 0.3.0 Changes**:
- Added validation status box and implementation status link
- Clarified all performance claims as unvalidated design targets
- Added cross-references to IMPLEMENTATION_STATUS.md and ROADMAP.md

**Revision 0.2.0 Changes**:
- Added reserved generation values (4.5) to prevent overflow collision
- Added updated dereference algorithm with PERSISTENT_MARKER handling (4.6)
- Added liveness definition for snapshot optimization (6.4.1)
- Added regions and effect suspension rules (7.7)
- Added region isolation and concurrency rules (7.8)
- Added snapshot roots extraction for cycle collection (8.5.1)
- Added cycle collection and effect safety (8.5.2-8.5.3)
- Added five new safety theorems (5-9) with proof sketches
- Expanded proof obligations table with new theorems

This document specifies Blood's memory model: the Synthetic Safety Model (SSM). This is Blood's core innovation—achieving memory safety without garbage collection through a synthesis of generational references, mutable value semantics, and tiered memory management.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Pointer Representation](#2-pointer-representation)
3. [Memory Tiers](#3-memory-tiers)
4. [Generation Lifecycle](#4-generation-lifecycle)
   - 4.5 [Reserved Generation Values](#45-reserved-generation-values)
   - 4.6 [Updated Dereference Algorithm](#46-updated-dereference-algorithm)
5. [Escape Analysis](#5-escape-analysis)
6. [Generation Snapshots](#6-generation-snapshots)
   - 6.4.1 [Liveness Definition for Snapshot Optimization](#641-liveness-definition-for-snapshot-optimization)
7. [Region Management](#7-region-management)
   - 7.7 [Regions and Effect Suspension](#77-regions-and-effect-suspension)
   - 7.8 [Region Isolation and Concurrency](#78-region-isolation-and-concurrency)
8. [Reference Counting (Tier 3)](#8-reference-counting-tier-3)
   - 8.5.1 [Snapshot Roots Extraction](#851-snapshot-roots-extraction)
   - 8.5.2 [Cycle Collection and Effect Safety](#852-cycle-collection-and-effect-safety)
9. [Soundness Analysis](#9-soundness-analysis)
10. [Implementation Notes](#10-implementation-notes)

---

## 1. Overview

### 1.1 Design Goals

The Synthetic Safety Model achieves:

1. **Memory safety** — No use-after-free, double-free, or dangling pointers
2. **Deterministic cleanup** — No GC pauses, predictable resource release
3. **Zero-cost when provable** — Static analysis eliminates runtime checks
4. **Graceful degradation** — Runtime checks where static proof fails
5. **Effect integration** — Safe interaction with algebraic effect handlers

> **Performance Status**: The performance characteristics described in this document are **design targets** based on:
> - Vale's generational reference design (no published benchmarks as of January 2026)
> - Theoretical analysis of check overhead (~3-4 cycles on modern x86-64)
> - Java's escape analysis research (proven effective for stack promotion)
>
> Empirical validation will occur during implementation. Performance-critical claims are marked with "(unvalidated)" where appropriate.

### 1.2 Related Specifications

- [SPECIFICATION.md](./SPECIFICATION.md) — Core language specification
- [FORMAL_SEMANTICS.md](./FORMAL_SEMANTICS.md) — Formal operational semantics
- [CONCURRENCY.md](./CONCURRENCY.md) — Fiber-region interaction rules
- [FFI.md](./FFI.md) — Pointer representation at FFI boundaries
- [STDLIB.md](./STDLIB.md) — Box<T> and generational pointer types
- [DIAGNOSTICS.md](./DIAGNOSTICS.md) — Memory-related error messages

### 1.3 Implementation Status

The following table tracks implementation status of SSM (Synthetic Safety Model) components:

| Component | Status | Location | Notes |
|-----------|--------|----------|-------|
| 128-bit BloodPtr | ✅ Implemented | `bloodc/src/mir/ptr.rs` | Byte-identical to spec |
| PtrMetadata | ✅ Implemented | `bloodc/src/mir/ptr.rs` | Tier, flags, type_fp |
| MemoryTier enum | ✅ Implemented | `bloodc/src/mir/ptr.rs` | Stack, Region, Persistent |
| MIR types | ✅ Implemented | `bloodc/src/mir/types.rs` | BasicBlock, Statement, Terminator |
| MIR lowering | ✅ Implemented | `bloodc/src/mir/lowering.rs` | HIR→MIR complete |
| Escape analysis | ✅ Implemented | `bloodc/src/mir/escape.rs` | Inter-procedural analysis |
| Generation snapshots | ✅ Implemented | `bloodc/src/mir/snapshot.rs` | Snapshot capture/validation |
| blood_alloc/blood_free | ✅ Integrated | `bloodc/src/codegen/context.rs` | Runtime memory functions |
| MIR in codegen pipeline | 🔶 Partial | `bloodc/src/main.rs` | MIR runs, codegen uses HIR directly |
| Tier assignment from escape | 🔶 Partial | `bloodc/src/codegen/context.rs` | Analysis runs, results partially used |
| Generation check emission | 📋 Designed | — | Awaits MIR-based codegen |
| Snapshot validation at runtime | 📋 Designed | — | Algorithm specified |
| Region management | ✅ Implemented | `blood-runtime/src/memory.rs` | Slot, Region types |
| Persistent tier (Tier 3) | 📋 Designed | — | RC + cycle collection |

**Legend**: ✅ Implemented | 🔶 Partial | 📋 Designed | ❌ Not Started

### 1.4 Core Mechanism

SSM uses **generational references**: every heap allocation carries a generation counter. Pointers store both the address and the expected generation. On dereference, these must match.

```
┌─────────────────────────────────────────────────────────────┐
│                     MEMORY SLOT                              │
│  ┌─────────────────────────────────────────────────────┐    │
│  │  Value: <actual data>                                │    │
│  │  Generation: 42                                      │    │
│  │  Metadata: { tier: Region, type_id: 0x1A3F }        │    │
│  └─────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│                     POINTER TO SLOT                          │
│  ┌─────────────────────────────────────────────────────┐    │
│  │  Address: 0x7FFF_1234_5678                           │    │
│  │  Expected Generation: 42                             │    │
│  │  Metadata: { mut: true, linear: false }             │    │
│  └─────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────┘

Dereference check: pointer.generation == slot.generation
  ✓ Match → access allowed
  ✗ Mismatch → StaleReference effect raised
```

### 1.5 Hybrid Model Clarification

Blood uses a **hybrid ownership model**:

| Aspect | Blood Approach |
|--------|----------------|
| **Default** | Mutable value semantics (copies, not references) |
| **Explicit** | Borrowing via `&T` and `&mut T` when needed |
| **Heap** | Generational references for `Box<T>`, collections |
| **Resources** | Linear/affine types for must-use handles |

This differs from pure Hylo (value-only) and pure Rust (borrow-only).

---

## 2. Pointer Representation

### 2.1 The 128-bit Blood Pointer

All heap pointers in Blood are 128 bits wide:

```
┌────────────────────────────────────────────────────────────────────────────┐
│                           128-bit Blood Pointer                             │
├────────────────────────────────────────────────────────────────────────────┤
│ 127                           64 63                 32 31                 0 │
│ ├──────────────────────────────┤├───────────────────┤├───────────────────┤ │
│ │         ADDRESS (64)         ││   GENERATION (32) ││   METADATA (32)   │ │
│ └──────────────────────────────┘└───────────────────┘└───────────────────┘ │
└────────────────────────────────────────────────────────────────────────────┘
```

### 2.2 Field Specifications

#### Address Field (64 bits)

| Bits | Description |
|------|-------------|
| 63:48 | Reserved (must be canonical on x86-64) |
| 47:0 | Virtual address (48-bit addressable) |

On platforms with larger address spaces (ARM with 52-bit VA), the reserved bits shrink accordingly.

#### Generation Field (32 bits)

| Bits | Description |
|------|-------------|
| 31:0 | Unsigned 32-bit generation counter |

Range: 0 to 2³²-1 (4,294,967,295 generations before overflow).

#### Metadata Field (32 bits)

| Bits | Field | Description |
|------|-------|-------------|
| 31:28 | `tier` | Memory tier (0=Stack, 1=Region, 2=Persistent, 3-15=Reserved) |
| 27:24 | `flags` | Pointer properties (see below) |
| 23:0 | `type_fp` | Type fingerprint for dispatch optimization |

**Flag Bits (27:24)**:

| Bit | Name | Meaning |
|-----|------|---------|
| 27 | `MUT` | Mutable reference (1) or immutable (0) |
| 26 | `LINEAR` | Linear ownership (1) or unrestricted (0) |
| 25 | `FROZEN` | Deeply immutable, shareable across fibers |
| 24 | `NULLABLE` | May be null (0 address valid) |

**Type Fingerprint (23:0)**:

24-bit hash of the pointed-to type, used for:
- Fast type checks in multiple dispatch
- Debug assertions
- RTTI when needed

Collision probability: ~1/16M for type pairs. Collisions fall back to full type comparison.

### 2.3 Alignment Requirements

Blood pointers must be 16-byte aligned in memory:

```c
// C equivalent layout
typedef struct {
    uint64_t address;
    uint32_t generation;
    uint32_t metadata;
} __attribute__((aligned(16))) BloodPtr;
```

This alignment enables:
- Atomic 128-bit operations on x86-64 (`CMPXCHG16B`)
- Efficient SIMD operations on pointer arrays
- Cache line optimization

### 2.4 Null Representation

Null pointers use a canonical representation:

```
NULL = { address: 0, generation: 0, metadata: NULLABLE flag set }
```

Non-nullable pointers (the default) trap on null dereference before generation check.

### 2.5 Stack Pointers (Thin Pointers)

References to stack-allocated values use **thin pointers** (64-bit):

```
┌────────────────────────────────────────────────────────────────┐
│                    64-bit Stack Pointer                         │
├────────────────────────────────────────────────────────────────┤
│ 63                                                            0 │
│ ├────────────────────────────────────────────────────────────┤ │
│ │                      ADDRESS (64)                           │ │
│ └────────────────────────────────────────────────────────────┘ │
└────────────────────────────────────────────────────────────────┘
```

Rationale: Stack references are proven safe at compile time; no runtime generation check needed.

### 2.6 128-bit Pointer Overhead Analysis

Blood's 128-bit pointers incur overhead compared to standard 64-bit pointers. This section provides detailed analysis.

#### 2.6.1 Memory Bandwidth Overhead

| Metric | 64-bit Ptr | 128-bit Ptr | Overhead | Impact |
|--------|------------|-------------|----------|--------|
| Pointer size | 8 bytes | 16 bytes | 2x | Higher memory usage |
| Cache line fit | 8 ptrs/64B | 4 ptrs/64B | 2x | More cache misses |
| Array traversal | 8 bytes/elem | 16 bytes/elem | 2x | Slower iteration |
| Struct packing | Dense | Padding overhead | 1.3-2x | Larger objects |

**Mitigation**: Blood's tiered memory model means **only heap pointers are 128-bit**. Stack references use 64-bit thin pointers, which are the majority of references in typical programs. Escape analysis promotes values to stack when possible.

#### 2.6.2 Cache Efficiency Impact

**L1/L2 Cache Pressure** (unvalidated estimates):

```
Workload: Pointer-chasing linked structure (worst case)
64-bit pointers: 8 pointers per cache line
128-bit pointers: 4 pointers per cache line

Estimated cache miss increase: ~10-30% for pointer-heavy code
Estimated performance impact: ~5-15% slowdown
```

**Array of Pointers**:
```
64-bit:  [ptr₁|ptr₂|ptr₃|ptr₄|ptr₅|ptr₆|ptr₇|ptr₈] = 64 bytes (1 cache line)
128-bit: [ptr₁|ptr₂|ptr₃|ptr₄|             padding] = 64 bytes (1 cache line)
                                                       4 pointers vs 8
```

**Mitigation Strategies**:
1. **Value semantics**: Prefer copying small values over boxing
2. **Escape analysis**: Promotes to stack (64-bit thin pointers)
3. **Frozen types**: Immutable data uses specialized layouts
4. **Type fingerprint caching**: 24-bit fingerprint avoids full type lookup

#### 2.6.3 Atomic Operation Costs

128-bit atomic operations require special instructions:

| Platform | Instruction | Latency (approx.)* | Notes |
|----------|-------------|-------------------|-------|
| x86-64 | `CMPXCHG16B` | ~15-20 cycles | Requires 16-byte alignment |
| ARM64 | `LDXP`/`STXP` | ~10-15 cycles | Load/store exclusive pair |
| RISC-V | Emulated | ~30-50 cycles | No native 128-bit atomics |

*Latency values are approximate, based on published CPU documentation. Actual latency varies by microarchitecture and contention.

**Comparison with 64-bit atomics** (approximate):
- `CMPXCHG8B` (64-bit): ~5-10 cycles
- `CMPXCHG16B` (128-bit): ~15-20 cycles
- Overhead: ~2-3x for atomic operations

**Mitigation**: Most pointer operations are non-atomic. Atomic operations occur only for:
- Cross-fiber reference transfer
- Concurrent data structure updates
- VFT hot-swap

#### 2.6.4 Real-World Overhead Estimates (Unvalidated)

Based on analysis of similar systems (Go with GC, Vale's design documents):

| Workload Type | Estimated Overhead | Confidence |
|---------------|-------------------|------------|
| Compute-bound (minimal pointers) | <1% | High |
| Balanced (mixed) | 3-8% | Medium |
| Pointer-heavy (graphs, trees) | 10-20% | Medium |
| Linked list traversal (worst case) | 20-40% | Low |

**Note**: These estimates are unvalidated. Vale (the source of generational references) has not published production benchmarks as of January 2026. Actual performance will be measured during Blood implementation.

#### 2.6.5 Trade-off Justification

The 128-bit pointer overhead is accepted because:

1. **Memory safety without GC**: No garbage collector pauses or overhead
2. **No borrow checker complexity**: Simpler mental model than Rust's lifetimes
3. **Effect safety**: Generation snapshots enable safe continuation capture
4. **Hot-swap capability**: Metadata field enables runtime updates

**Break-even analysis**: The overhead is justified if:
- GC pause avoidance is worth 10-20% steady-state overhead
- Developer productivity gains from simpler ownership model
- Safety-critical requirements preclude GC

#### 2.6.6 Comparison with Alternatives

| Approach | Memory Overhead | CPU Overhead | Complexity | GC Pauses |
|----------|-----------------|--------------|------------|-----------|
| **Blood (128-bit gen-ref)** | 2x pointers | 1-2 cycles/check | Low | None |
| **Rust (borrow checker)** | None | None | High | None |
| **Go (GC)** | ~1.5x heap | Variable | Low | 1-10ms |
| **Java (GC)** | ~2x heap | Variable | Low | 10-100ms |
| **C (manual)** | None | None | Very High | None |

Blood's approach trades memory/CPU overhead for:
- Simpler ownership than Rust
- No GC pauses
- Stronger safety than C

---

## 3. Memory Tiers

### 3.1 Tier Overview

| Tier | Name | Lifecycle | Safety Mechanism | Cost (theoretical)* |
|------|------|-----------|------------------|---------------------|
| 0 | Stack | Lexical scope | Compile-time proof | Zero |
| 1 | Region | Explicit scope | Generational check | ~1-2 cycles |
| 2 | Persistent | Reference-counted | Deferred RC | Variable |

*Cycle costs are design targets based on analysis. See Performance Status in §1.1.

### 3.2 Tier 0: Stack

**Allocation**: Values placed on CPU stack via `SP` manipulation.

**Safety**: Compiler proves all references are outlived by the value.

**Promotion Criteria**: A value is stack-allocated if:
1. Its size is known at compile time
2. Escape analysis proves it doesn't outlive its scope
3. No reference to it is stored in a heap structure
4. No reference to it crosses an effect suspension point

**Cost**: Zero runtime overhead. No generation checks.

```blood
fn stack_example() -> i32 {
    let x = 42           // Stack allocated
    let y = &x           // Thin pointer (64-bit)
    *y + 1               // No generation check
}
```

### 3.3 Tier 1: Region

**Allocation**: Values allocated in a memory region with generational tracking.

**Safety**: Generation check on every dereference.

**Use Cases**:
- Heap allocations (`Box<T>`, `Vec<T>`)
- Values that escape their lexical scope
- Values referenced across effect operations

**Cost**: ~1-2 CPU cycles per dereference for the generation comparison.

```blood
fn region_example() -> Box<i32> {
    let x = Box::new(42)  // Region allocated
    x                      // Ownership transferred out
}

fn use_region(b: Box<i32>) {
    let val = *b          // Generation check here
}
```

### 3.4 Tier 2: Persistent

**Allocation**: Long-lived values managed by deferred reference counting.

**Promotion Triggers**:
1. Generation counter overflow (2³² generations)
2. Explicit `persist(value)` annotation
3. Cross-fiber sharing (frozen values)

**Safety**: Reference count ensures value lives while referenced.

**Cost**: RC increment/decrement overhead, potential cycle collection.

```blood
fn persistent_example() {
    let config = persist(load_config())  // Tier 2
    spawn(|| use_config(&config))        // Shared across fibers
    spawn(|| use_config(&config))
}
```

---

## 4. Generation Lifecycle

### 4.1 Allocation

When a value is allocated in Tier 1 (Region):

```
ALLOCATE(value, type) → BloodPtr:
    1. slot ← find_free_slot(sizeof(type))
    2. slot.value ← value
    3. slot.generation ← slot.generation  // Preserve existing generation
    4. slot.type_id ← type.id
    5. ptr ← BloodPtr {
           address: &slot,
           generation: slot.generation,
           metadata: make_metadata(type, flags)
       }
    6. RETURN ptr
```

Note: The generation is NOT reset on allocation. It carries forward from previous use of that memory slot.

### 4.2 Dereference

```
DEREFERENCE(ptr) → Value:
    1. IF ptr.metadata.tier == Stack:
           RETURN *ptr.address  // No check needed

    2. slot ← memory[ptr.address]

    3. IF ptr.generation ≠ slot.generation:
           RAISE StaleReference {
               address: ptr.address,
               expected: ptr.generation,
               actual: slot.generation
           }

    4. RETURN slot.value
```

### 4.3 Deallocation (Free)

```
FREE(ptr):
    1. slot ← memory[ptr.address]

    2. IF ptr.generation ≠ slot.generation:
           RAISE StaleReference { ... }  // Double-free detected

    3. slot.generation ← slot.generation + 1  // INCREMENT

    4. IF slot.generation == 0:  // Overflow!
           PROMOTE_TO_PERSISTENT(slot)

    5. slot.value ← TOMBSTONE
    6. add_to_freelist(ptr.address)
```

### 4.4 Generation Overflow

When a slot's generation counter wraps from 2³²-1 to 0:

```
PROMOTE_TO_PERSISTENT(slot):
    1. new_slot ← allocate_persistent_slot()
    2. new_slot.value ← slot.value
    3. new_slot.refcount ← 1
    4. slot.value ← REDIRECT(new_slot)
    5. slot.generation ← PERSISTENT_MARKER  // Special value

    // All existing pointers continue to work:
    // - They dereference to the old slot
    // - Old slot redirects to persistent slot
    // - Future allocations at old address get fresh generation
```

The `PERSISTENT_MARKER` generation (e.g., 0xFFFFFFFF) indicates this slot has been promoted. Dereference logic handles the redirect.

### 4.5 Reserved Generation Values

To prevent collision between valid generations and special markers, certain generation values are reserved:

| Value | Name | Purpose |
|-------|------|---------|
| `0xFFFFFFFF` | `PERSISTENT_MARKER` | Indicates slot has been promoted to Tier 3 |
| `0xFFFFFFFE` | `OVERFLOW_GUARD` | Triggers promotion before overflow |
| `0xFFFFFFFD` | `TOMBSTONE_GEN` | Indicates slot is freed but not yet reused |
| `0x00000000` - `0xFFFFFFFC` | Valid range | Normal allocation generations |

**Allocation Constraint**: When a slot's generation reaches `OVERFLOW_GUARD` (`0xFFFFFFFE`) during free, promotion to Tier 3 occurs immediately. This prevents the generation from ever reaching `PERSISTENT_MARKER` through normal increment.

**Invariant**: A slot's generation is never allocated at a reserved value. The FREE operation checks for `OVERFLOW_GUARD` before incrementing:

```
FREE(ptr):
    1. slot ← memory[ptr.address]
    2. IF ptr.generation ≠ slot.generation:
           RAISE StaleReference { ... }

    3. IF slot.generation >= OVERFLOW_GUARD:  // 0xFFFFFFFE or higher
           PROMOTE_TO_PERSISTENT(slot)
           RETURN

    4. slot.generation ← slot.generation + 1
    5. slot.value ← TOMBSTONE
    6. add_to_freelist(ptr.address)
```

### 4.6 Updated Dereference Algorithm

The dereference algorithm must handle the `PERSISTENT_MARKER` case:

```
DEREFERENCE(ptr) → Value:
    1. IF ptr.metadata.tier == Stack:
           RETURN *ptr.address  // No check needed

    2. slot ← memory[ptr.address]

    3. // Check for promoted slot (redirect case)
       IF slot.generation == PERSISTENT_MARKER:
           persistent_slot ← slot.value.as_redirect()
           RETURN DEREFERENCE_PERSISTENT(persistent_slot, ptr.generation)

    4. // Normal generation check
       IF ptr.generation ≠ slot.generation:
           RAISE StaleReference {
               address: ptr.address,
               expected: ptr.generation,
               actual: slot.generation
           }

    5. RETURN slot.value

DEREFERENCE_PERSISTENT(persistent_slot, original_gen) → Value:
    // Persistent slots use refcount, not generation
    // The original_gen is recorded for debugging but not checked
    IF persistent_slot.refcount == 0:
        RAISE StaleReference { ... }  // Shouldn't happen with proper RC
    RETURN persistent_slot.value
```

### 4.7 Generation Check Assembly

On x86-64, the generation check compiles to:

```asm
; ptr in RAX (128-bit, assume aligned load)
; Low 64 bits = address, next 32 = generation

dereference:
    movq    rdi, [rax]           ; Load address
    movl    esi, [rax + 8]       ; Load expected generation
    movl    edx, [rdi + 8]       ; Load actual generation (offset 8 in slot)
    cmpl    esi, edx             ; Compare
    jne     .stale_reference     ; Branch if mismatch
    movq    rax, [rdi]           ; Load value
    ret

.stale_reference:
    ; Set up StaleReference effect
    ; ptr.address in RDI, expected in ESI, actual in EDX
    call    blood_stale_ref_handler
```

**Typical cost (theoretical)**: ~3-4 cycles in the fast path (generations match). See Performance Status in §1.1 for validation status.

---

## 5. Escape Analysis

### 5.1 Overview

Blood's compiler performs **inter-procedural escape analysis** to determine which values can be stack-allocated and which dereferences need generation checks.

### 5.2 Escape States

Each value is assigned an escape state:

| State | Meaning | Tier Assignment |
|-------|---------|-----------------|
| `NoEscape` | Never escapes defining scope | Stack (Tier 0) |
| `ArgEscape` | Escapes via function argument | Depends on callee |
| `HeapEscape` | Stored in heap structure | Region (Tier 1) |
| `GlobalEscape` | Reachable from global/static | Persistent (Tier 2) |
| `EffectEscape` | Crosses effect suspension | Region + Snapshot |

### 5.3 Escape Analysis Algorithm

```
ANALYZE_ESCAPES(function):
    worklist ← all allocations in function

    FOR EACH alloc IN worklist:
        alloc.escape ← NoEscape  // Optimistic

    REPEAT UNTIL fixpoint:
        FOR EACH alloc IN worklist:
            FOR EACH use OF alloc:
                new_state ← CLASSIFY_USE(use, alloc)
                alloc.escape ← JOIN(alloc.escape, new_state)

    RETURN escape states

CLASSIFY_USE(use, alloc):
    MATCH use:
        | Store(heap_location, alloc) → HeapEscape
        | Call(f, alloc) → ANALYZE_CALLEE(f, arg_position)
        | Return(alloc) → ArgEscape  // Caller determines
        | EffectOp(_, alloc) → EffectEscape
        | LocalUse(_) → NoEscape
```

### 5.4 Inter-Procedural Analysis

For function calls, we analyze the callee:

```
ANALYZE_CALLEE(function, arg_position):
    summary ← get_or_compute_summary(function)
    RETURN summary.param_escapes[arg_position]

COMPUTE_SUMMARY(function):
    // Summarize how each parameter escapes
    FOR EACH param IN function.params:
        param_escape ← ANALYZE_ESCAPES(function).get(param)
    RETURN FunctionSummary { param_escapes }
```

### 5.5 Effect Escape

When a value's reference crosses an effect operation:

```blood
fn effect_escape_example(data: &Data) / {Yield<i32>} {
    let local = compute(data)
    yield(local.value)      // 'data' crosses suspension!
    use(data)               // data might be stale after resume
}
```

The analysis marks `data` as `EffectEscape`, requiring:
1. Generation check after resume
2. Inclusion in continuation's generation snapshot

### 5.6 Optimization: Check Elision

When escape analysis proves a reference doesn't escape:

```blood
fn no_escape() -> i32 {
    let v = vec![1, 2, 3]   // Heap allocated
    let sum = v.iter().sum()
    // 'v' doesn't escape, no external aliases possible
    // Compiler ELIDES generation checks on v's internal buffer
    sum
}
```

**Elision Criteria**:
1. Value is `NoEscape` or `ArgEscape` with known-safe callee
2. No references stored to heap
3. No effect operations while value is live
4. All uses are within single-owner scope

### 5.7 Escape Analysis Soundness

**Theorem (Escape Soundness)**: If escape analysis classifies a value as `NoEscape`, no runtime generation check is needed for safety.

**Proof Obligation**: Show that `NoEscape` values cannot be referenced after deallocation.

**Proof Sketch**:
1. `NoEscape` means no heap storage of references
2. Stack frame containing value outlives all references
3. Frame deallocation happens after all references die
4. Therefore, no stale references possible ∎

### 5.8 Escape Analysis × Effects Interaction

This section details how escape analysis interacts with Blood's algebraic effect system.

#### 5.8.1 Effect Suspension Points

Effect operations (`perform`) are **suspension points** where execution may yield to a handler and resume later:

```blood
fn process(data: &mut Data) / {Yield<i32>} {
    // Phase 1: data is live
    let intermediate = transform(data)

    yield(intermediate)  // ← SUSPENSION POINT

    // Phase 2: data must still be valid
    finalize(data)
}
```

At suspension points, escape analysis must answer:
1. Which references in scope might become stale?
2. Which references must be included in generation snapshot?
3. Can any references be proven safe across suspension?

#### 5.8.2 Effect-Aware Escape Classification

Extended escape states for effect interaction:

| State | Standard Meaning | Effect Implication |
|-------|------------------|-------------------|
| `NoEscape` | Never escapes scope | Safe across suspension (stack-bound) |
| `EffectEscape` | Crosses suspension | **Requires snapshot validation** |
| `EffectLocal` | Used before/after but not across | No snapshot needed |
| `EffectCapture` | Captured in handler closure | Handler owns reference |

```
CLASSIFY_EFFECT_ESCAPE(ref, suspension_point):
    IF ref.def_point AFTER suspension_point:
        RETURN EffectLocal  // Defined after, not captured

    IF ref.last_use BEFORE suspension_point:
        RETURN EffectLocal  // Used only before, not needed after

    IF ref.owner IS handler_closure:
        RETURN EffectCapture  // Handler manages lifetime

    IF ref.is_stack_bound AND ref.frame.outlives(suspension_point):
        RETURN NoEscape  // Stack frame still valid

    RETURN EffectEscape  // Must snapshot
```

#### 5.8.3 Deep vs. Shallow Handler Implications

**Deep handlers** re-install themselves on resume, affecting escape analysis:

```blood
deep handler Logger for Log {
    op log(msg) {
        // Handler state persists across multiple log calls
        let count = get_count()  // Handler-local state
        set_count(count + 1)
        resume(())  // Deep: handler reinstalled
    }
}
```

For deep handlers:
- Handler closure state is `EffectCapture`
- References in handled computation may cross multiple suspensions
- Escape analysis must be conservative

**Shallow handlers** handle once and disappear:

```blood
shallow handler OneShot for Yield<i32> {
    op yield(value) {
        // Only handles first yield
        resume(())  // Shallow: handler NOT reinstalled
    }
}
```

For shallow handlers:
- Continuation resumes without handler
- Fewer suspension points after first
- Escape analysis can be more precise

#### 5.8.4 Multi-shot Handler Analysis

When a handler may call `resume` multiple times (e.g., for backtracking):

```blood
deep handler Backtrack for NonDet {
    op choose(options) {
        for opt in options {
            resume(opt)  // Multiple resumes!
        }
    }
}
```

Multi-shot implications:
1. **Linear values forbidden** (see FORMAL_SEMANTICS.md §8)
2. **All captured references are EffectEscape**
3. **Each resume requires fresh snapshot validation**
4. **Mutable state requires isolation** (copy-on-resume)

#### 5.8.5 Optimization: Effect-Aware Check Elision

When escape analysis can prove safety across effects:

```blood
fn optimizable() / {IO} {
    let v = vec![1, 2, 3]  // NoEscape (stack-promotable)
    println(v.len().to_string())  // IO effect, but v doesn't cross
    let sum = v.iter().sum()  // Still NoEscape
    sum
}
```

Even though `println` performs IO:
- `v` is not used *across* the IO operation
- Its lifetime is within the stack frame
- No snapshot needed for `v`

**Contrast with**:

```blood
fn requires_snapshot() / {Yield<i32>} {
    let v = Box::new(vec![1, 2, 3])  // HeapEscape
    yield(v.len() as i32)  // v is live across suspension!
    v.push(4)  // v must be validated after resume
    v.len() as i32
}
```

Here `v` is `EffectEscape` because:
- `v` is heap-allocated (`Box`)
- `v` is used both before and after `yield`
- Must be included in generation snapshot

#### 5.8.6 Analysis Complexity

Effect-aware escape analysis has higher complexity than standard escape analysis:

| Analysis Level | Complexity | Precision |
|----------------|------------|-----------|
| Intra-procedural | O(n) | Low |
| Inter-procedural (no effects) | O(n²) | Medium |
| Effect-aware inter-procedural | O(n² × e) | High |
| Effect-aware + multi-shot | O(n² × e × r) | Very High |

Where:
- n = number of values
- e = number of effect operations
- r = resume count bound (infinite for multi-shot)

**Implementation Strategy**:
1. Start with conservative analysis (all EffectEscape)
2. Apply precision improvements incrementally
3. Use optimization levels to control analysis depth
4. Cache function summaries for inter-procedural reuse

---

## 6. Generation Snapshots

### 6.1 Purpose

When an algebraic effect suspends computation, the captured continuation may hold generational references. If the handler resumes later, those references might be stale (the memory freed and reallocated).

Generation snapshots capture the expected generations at suspension time and validate on resume.

### 6.2 Snapshot Structure

```
GenerationSnapshot = {
    entries: Vec<(Address, Generation)>,
    capture_time: Timestamp,  // For debugging
}
```

### 6.3 Capture Algorithm

```
CAPTURE_SNAPSHOT(continuation_env):
    snapshot ← empty GenerationSnapshot

    FOR EACH binding IN continuation_env:
        IF binding.type.contains_genref():
            refs ← EXTRACT_GENREFS(binding.value)
            FOR EACH (addr, gen) IN refs:
                snapshot.entries.push((addr, gen))

    snapshot.capture_time ← now()
    RETURN snapshot

EXTRACT_GENREFS(value):
    MATCH value:
        | BloodPtr(addr, gen, _) → [(addr, gen)]
        | Struct(fields) → CONCAT(EXTRACT_GENREFS(f) FOR f IN fields)
        | Array(elements) → CONCAT(EXTRACT_GENREFS(e) FOR e IN elements)
        | Primitive(_) → []
```

### 6.4 Capture Optimization: Static Filtering

Not all references need snapshotting. The compiler analyzes which references are actually used after resume:

```
OPTIMIZE_SNAPSHOT(continuation, full_snapshot):
    live_after_resume ← LIVENESS_ANALYSIS(continuation)

    filtered ← empty
    FOR EACH (addr, gen) IN full_snapshot.entries:
        IF addr IN live_after_resume:
            filtered.push((addr, gen))

    RETURN filtered
```

This can dramatically reduce snapshot size. A continuation that only uses primitive values after resume needs no snapshot.

#### 6.4.1 Liveness Definition for Snapshot Optimization

For snapshot optimization purposes, "live after resume" is defined precisely:

**Definition (Dereference Reachability)**: A reference `r` is **live after resume** if there exists a control flow path from the resume point to a dereference of `r`.

This is **dereference reachability**, not mere syntactic occurrence. A reference that appears in code but is never dereferenced may be excluded from the snapshot.

**Examples**:

```blood
fn example() / {Yield<i32>} {
    let a = Box::new(1)    // a is dereferenced after resume → INCLUDE
    let b = Box::new(2)    // b is only passed, never dereferenced → EXCLUDE
    let c = Box::new(3)    // c is conditionally dereferenced → INCLUDE

    yield(0)

    let x = *a             // Dereference of a
    pass_by_ref(&b)        // b passed but not dereferenced here
    if condition {
        let y = *c         // Conditional dereference of c
    }
}
```

**Analysis Algorithm**:

```
COMPUTE_LIVE_AFTER_RESUME(continuation) → Set<Address>:
    resume_point ← continuation.resume_location
    all_derefs ← {}

    // Find all dereference operations reachable from resume
    FOR EACH instruction IN reachable_from(resume_point):
        IF instruction.is_dereference():
            all_derefs.insert(instruction.target_address)

    // Filter to only captured references
    captured ← EXTRACT_GENREFS(continuation.env)
    RETURN captured ∩ all_derefs
```

**Soundness Guarantee**:

| Classification | Action | Correctness |
|----------------|--------|-------------|
| Ref is live, included in snapshot | Validate on resume | Sound ✓ |
| Ref is live, excluded from snapshot | Stale access possible | **Unsound** ✗ |
| Ref is not live, included in snapshot | Unnecessary validation | Sound but slow ✓ |
| Ref is not live, excluded from snapshot | Skip validation | Sound ✓ |

**Conservative Approximation**: When liveness cannot be precisely determined (e.g., through opaque function calls or dynamic dispatch), the reference MUST be included in the snapshot. Soundness requires no false negatives; false positives only affect performance.

```
CONSERVATIVE_LIVENESS(ref, continuation) → bool:
    // If we can prove the ref is dead, exclude it
    IF provably_dead(ref, continuation):
        RETURN false

    // If uncertain, include it (conservative)
    RETURN true
```

**Compiler Flag**: `--strict-snapshot-analysis` forces conservative inclusion of all captured references, trading performance for simpler reasoning.

### 6.5 Validation Algorithm

```
VALIDATE_SNAPSHOT(snapshot) → Result<(), StaleReference>:
    FOR EACH (addr, expected_gen) IN snapshot.entries:
        slot ← memory[addr]
        actual_gen ← slot.generation

        IF expected_gen ≠ actual_gen:
            RETURN Err(StaleReference {
                address: addr,
                expected: expected_gen,
                actual: actual_gen,
                capture_time: snapshot.capture_time
            })

    RETURN Ok(())
```

### 6.6 Validation Timing

Two strategies for when to validate:

#### Strategy A: Eager Validation (Default)

Validate immediately on resume:

```
RESUME(continuation, value):
    result ← VALIDATE_SNAPSHOT(continuation.snapshot)
    IF result.is_err():
        RAISE result.error
    RETURN continuation.code(value)
```

**Pros**: Fail-fast, clear error location
**Cons**: Pays validation cost even if references unused

#### Strategy B: Lazy Validation

Validate on first dereference of captured reference:

```
LAZY_RESUME(continuation, value):
    continuation.snapshot.validated ← false
    RETURN continuation.code(value)

LAZY_DEREF(ptr, snapshot):
    IF NOT snapshot.validated:
        result ← VALIDATE_SNAPSHOT(snapshot)
        IF result.is_err():
            RAISE result.error
        snapshot.validated ← true
    RETURN DEREFERENCE(ptr)
```

**Pros**: Skips validation if references unused
**Cons**: Error location is at dereference, not resume

### 6.7 Handler Contract for StaleReference

```blood
effect StaleReference {
    /// Called when a generational reference is stale.
    /// The handler MUST NOT resume normally - it must diverge.
    op stale(info: StaleInfo) -> never
}

struct StaleInfo {
    address: Address,
    expected_generation: u32,
    actual_generation: u32,
    capture_time: Timestamp,
}
```

**Default Handler**:

```blood
handler DefaultStaleHandler for StaleReference {
    op stale(info) -> never {
        panic("Use-after-free detected: address {info.address} \
               had generation {info.expected_generation} at capture, \
               now {info.actual_generation}")
    }
}
```

**Safety-Critical Handler**:

```blood
handler SafetyCriticalStaleHandler for StaleReference {
    op stale(info) -> never {
        // Log to flight recorder / black box
        log_critical("MEMORY VIOLATION", info)

        // Attempt graceful degradation
        IF can_recover():
            abort_current_fiber()  // Kill just this fiber
        ELSE:
            safe_shutdown()        // Orderly system halt
    }
}
```

### 6.8 Linear Values and Snapshots

Linear values require special handling:

**Rule**: Linear values MUST NOT be captured in snapshots across multi-shot handlers.

```
VALIDATE_LINEAR_CAPTURE(continuation, handler):
    IF handler.is_multi_shot:
        linears ← EXTRACT_LINEAR_VALUES(continuation.env)
        IF NOT linears.is_empty():
            COMPILE_ERROR("Linear value {linears[0]} cannot cross \
                          multi-shot resume in handler {handler.name}")
```

For single-shot handlers, linear values transfer to the continuation:

```
CAPTURE_WITH_LINEAR(env):
    snapshot ← CAPTURE_SNAPSHOT(env)
    linears ← EXTRACT_LINEAR_VALUES(env)

    // Mark linears as "in flight" - cannot be used in handler
    FOR EACH linear IN linears:
        linear.status ← InContinuation

    RETURN (snapshot, linears)
```

---

## 7. Region Management

### 7.1 Region Overview

A **region** is a collection of memory slots that can be bulk-deallocated. Regions provide:

1. Locality of related allocations
2. Efficient bulk deallocation
3. Scope-based lifetime management

### 7.2 Region Structure

```
Region = {
    id: RegionId,
    parent: Option<RegionId>,
    slots: Vec<Slot>,
    free_list: Vec<SlotIndex>,
    generation_base: u32,
    stats: AllocationStats,
}

Slot = {
    value: Value | TOMBSTONE | REDIRECT,
    generation: u32,
    metadata: SlotMetadata,
}
```

### 7.3 Region Hierarchy

Regions form a tree. Child regions are deallocated before parents:

```
       ┌─────────────┐
       │ Global (R0) │
       └──────┬──────┘
              │
    ┌─────────┴─────────┐
    │                   │
┌───┴───┐          ┌────┴────┐
│ R1    │          │ R2      │
└───┬───┘          └────┬────┘
    │                   │
┌───┴───┐          ┌────┴────┐
│ R1.1  │          │ R2.1    │
└───────┘          └─────────┘
```

### 7.4 Region Allocation

```
REGION_ALLOC(region, value, type) → BloodPtr:
    IF region.free_list.is_empty():
        GROW_REGION(region)

    slot_idx ← region.free_list.pop()
    slot ← region.slots[slot_idx]

    slot.value ← value
    // Generation already set from previous use

    ptr ← BloodPtr {
        address: &slot,
        generation: slot.generation,
        metadata: make_metadata(type, Tier::Region)
    }

    region.stats.allocations += 1
    RETURN ptr
```

### 7.5 Region Deallocation (Bulk Free)

```
DEALLOCATE_REGION(region):
    // First, deallocate all child regions
    FOR EACH child IN region.children:
        DEALLOCATE_REGION(child)

    // Increment all generations (invalidates all pointers)
    FOR EACH slot IN region.slots:
        slot.generation += 1
        IF slot.generation == 0:  // Overflow
            PROMOTE_SLOT_TO_PERSISTENT(slot)
        slot.value ← TOMBSTONE

    // Return memory to parent or system
    IF region.parent.is_some():
        RETURN_TO_PARENT(region)
    ELSE:
        RETURN_TO_SYSTEM(region)
```

### 7.6 Scoped Regions

Blood provides scoped region syntax:

```blood
fn process_batch(items: &[Item]) -> Summary {
    region batch_region {
        // All allocations here go to batch_region
        let results: Vec<Result> = items.map(|i| process(i))
        let summary = summarize(&results)

        // 'summary' must be copied out, or this is an error
        summary.clone()
    }
    // batch_region deallocated here, all internal allocations freed
}
```

**Escape Check**: Values allocated in a region cannot escape without copying:

```blood
fn bad_escape() -> &Data {
    region r {
        let data = Box::new(Data::new())
        &*data  // ERROR: reference escapes region
    }
}

fn good_escape() -> Data {
    region r {
        let data = Box::new(Data::new())
        (*data).clone()  // OK: value copied out
    }
}
```

### 7.7 Regions and Effect Suspension

When an algebraic effect suspends computation inside a scoped region, special rules apply to ensure memory safety.

#### 7.7.1 The Region-Effect Interaction Problem

```blood
fn problematic() / {Yield<i32>} {
    region r {
        let data = Box::new(42)
        yield(*data)      // Effect suspends here
        use(data)         // Is 'r' still valid after resume?
    }
}
```

If the region `r` were deallocated during suspension, `data` would become invalid before the continuation resumes.

#### 7.7.2 Region Suspension Rule

**Rule**: Scoped regions are **suspended** (deallocation deferred) during effect suspension if any allocation from that region is captured in the continuation.

```
REGION_EFFECT_CHECK(region_scope, effect_op):
    continuation ← effect_op.captured_continuation
    captured_refs ← EXTRACT_GENREFS(continuation.env)

    region_refs ← { r | r ∈ captured_refs AND r.address ∈ region_scope.slots }

    IF NOT region_refs.is_empty():
        region_scope.status ← Suspended
        region_scope.suspend_count += 1
        continuation.suspended_regions.push(region_scope.id)
```

#### 7.7.3 Region Lifetime Extension

A region's lifetime extends to the maximum of:

1. Its lexical scope end, OR
2. All continuations capturing references into it completing

```
REGION_LIFETIME(region) → Lifetime:
    lexical_end ← region.scope.end
    continuation_end ← MAX(c.completion_time FOR c IN continuations_referencing(region))
    RETURN MAX(lexical_end, continuation_end)
```

#### 7.7.4 Resume and Region Restoration

When a continuation resumes, suspended regions are checked:

```
RESUME_WITH_REGIONS(continuation, value):
    // Validate generation snapshot first
    result ← VALIDATE_SNAPSHOT(continuation.snapshot)
    IF result.is_err():
        RAISE result.error

    // Restore suspended regions to active state
    FOR EACH region_id IN continuation.suspended_regions:
        region ← get_region(region_id)
        region.suspend_count -= 1

    RETURN continuation.code(value)
```

#### 7.7.5 Deferred Region Deallocation

When a region's lexical scope ends while suspended:

```
EXIT_REGION_SCOPE(region):
    IF region.suspend_count > 0:
        // Region has active continuations - defer deallocation
        region.status ← PendingDeallocation
        region.dealloc_callback ← SCHEDULE_DEFERRED_DEALLOC(region)
        RETURN  // Don't deallocate yet

    // Normal deallocation
    DEALLOCATE_REGION(region)

// Called when last continuation completes
ON_CONTINUATION_COMPLETE(continuation):
    FOR EACH region_id IN continuation.suspended_regions:
        region ← get_region(region_id)
        region.suspend_count -= 1

        IF region.suspend_count == 0 AND region.status == PendingDeallocation:
            DEALLOCATE_REGION(region)
```

#### 7.7.6 Compile-Time Region-Effect Analysis

The compiler analyzes region-effect interactions:

```
ANALYZE_REGION_EFFECTS(function):
    FOR EACH region_scope IN function.regions:
        FOR EACH effect_op IN region_scope.effect_operations:
            captures ← ANALYZE_CAPTURES(effect_op)
            region_captures ← captures ∩ region_scope.allocations

            IF NOT region_captures.is_empty():
                MARK_REGION_SUSPENDABLE(region_scope)
                INSERT_SUSPENSION_LOGIC(region_scope, effect_op)
```

#### 7.7.7 Example: Safe Region Suspension

```blood
fn generator() -> impl Iterator<i32> / {Yield<i32>} {
    region buffer_region {
        let buffer = Vec::with_capacity(1000)

        for i in 0..100 {
            buffer.push(compute(i))

            if buffer.len() >= 10 {
                for item in buffer.drain(..) {
                    yield(item)  // Region suspended during yield
                }
                // Region still valid here after resume
            }
        }
    }
    // Region deallocated after all yields complete
}
```

### 7.8 Region Isolation and Concurrency

Regions interact with Blood's fiber-based concurrency model. This section specifies the safety rules.

#### 7.8.1 Region Ownership Rule

**Fundamental Invariant**: Each region is owned by exactly one fiber at any time.

| Ownership State | Description |
|-----------------|-------------|
| `Exclusive` | One fiber owns, no sharing |
| `Suspended` | Owner fiber suspended at effect, region preserved |
| `Transferring` | Ownership moving between fibers (brief) |

Cross-fiber region references are **prohibited** for Tier 1 regions. Use Tier 3 (Persistent) for shared data.

#### 7.8.2 Fiber-Local Regions

By default, regions are fiber-local:

```blood
fn fiber_local_example() {
    region local_data {
        let buffer = allocate_buffer()
        process(buffer)  // Only this fiber accesses local_data
    }
}
```

**Guarantee**: No other fiber can obtain a reference into a fiber-local region. Deallocation is trivially safe—no synchronization required.

#### 7.8.3 Deallocation Atomicity

Region deallocation is atomic with respect to generation checks within the owning fiber:

```
DEALLOCATE_REGION_ATOMIC(region):
    ASSERT(current_fiber() == region.owner)

    // No other fiber can access this region
    // Therefore, no synchronization needed for generation updates
    FOR EACH slot IN region.slots:
        slot.generation ← increment_or_promote(slot.generation)
        slot.value ← TOMBSTONE

    RETURN_MEMORY(region)
```

#### 7.8.4 Cross-Fiber Data Transfer

To share data between fibers, use Tier 3 mechanisms:

```blood
fn cross_fiber_sharing() {
    // WRONG: Cannot share region references
    region r {
        let data = Box::new(Data::new())
        // spawn(|| use(&data))  // COMPILE ERROR: region ref crosses fiber

        // CORRECT: Promote to Tier 3 first
        let shared = persist(data.clone())  // Now Tier 3
        spawn(|| use(&shared))              // OK: Tier 3 is ref-counted
    }
}
```

#### 7.8.5 Frozen Data Sharing

For immutable cross-fiber access:

```blood
fn frozen_sharing() {
    let config = freeze(Config::load())  // Tier 3, deeply immutable

    // Multiple fibers can read frozen data
    let h1 = spawn(|| read_config(&config))
    let h2 = spawn(|| read_config(&config))

    await(h1)
    await(h2)
}
```

**Freeze Semantics**:
1. Value is deeply copied to Tier 3
2. All internal references become Tier 3
3. Refcount initialized based on captures
4. Value is marked immutable (FROZEN flag)

#### 7.8.6 Race Condition Prevention

The following race is prevented by construction:

```
Fiber A                          Fiber B
────────                         ────────
region r { ... }
  ptr = &value_in_r
                                 // Cannot obtain ptr
                                 // No way to reference into r
  ...
}
DEALLOCATE_REGION(r)
                                 // Even if B tried to deref ptr,
                                 // it never had ptr in the first place
```

**Theorem (Region Isolation)**: If `ptr` points into region `r` owned by fiber `F`, then only fiber `F` can dereference `ptr`.

**Proof Sketch**:
1. Regions are created fiber-local (ownership = creating fiber)
2. Region references cannot cross fiber boundaries (enforced by type system)
3. Only owner fiber can access region contents
4. Therefore, only owner can dereference region pointers ∎

#### 7.8.7 Exception: Shared Mutable State

For rare cases requiring shared mutable state, Blood provides explicit synchronization:

```blood
fn shared_mutable() {
    let shared: Synchronized<Counter> = synchronized(Counter::new())

    spawn(|| {
        shared.with_lock(|counter| {
            counter.increment()  // Exclusive access guaranteed
        })
    })
}
```

`Synchronized<T>` uses Tier 3 storage with mutex protection. This is an escape hatch, not the default.

---

## 8. Reference Counting (Tier 3)

### 8.1 When Reference Counting Applies

Tier 3 (Persistent) uses deferred reference counting for:

1. Values promoted due to generation overflow
2. Values shared across fibers (`Frozen<T>`)
3. Global/static values
4. Explicitly persisted values

### 8.2 Reference Count Structure

```
PersistentSlot = {
    value: Value,
    refcount: AtomicU64,
    weak_count: AtomicU32,
    metadata: PersistentMetadata,
}
```

### 8.3 Reference Count Operations

```
RC_INCREMENT(slot):
    old ← slot.refcount.fetch_add(1, Ordering::Relaxed)
    IF old == 0:
        PANIC("Increment of zero refcount")

RC_DECREMENT(slot):
    old ← slot.refcount.fetch_sub(1, Ordering::Release)
    IF old == 1:
        // Last reference dropped
        fence(Ordering::Acquire)
        DROP_VALUE(slot.value)

        IF slot.weak_count.load() == 0:
            DEALLOCATE_PERSISTENT_SLOT(slot)
        ELSE:
            slot.value ← TOMBSTONE
```

### 8.4 Deferred Decrement

To avoid deep recursion on drop, Blood uses **deferred decrement**:

```
DEFERRED_RC_DECREMENT(slot):
    old ← slot.refcount.fetch_sub(1, Ordering::Release)
    IF old == 1:
        QUEUE_FOR_COLLECTION(slot)

// Background collector processes queue
COLLECTOR_LOOP():
    WHILE NOT terminated:
        batch ← DRAIN_QUEUE(max: 100)
        FOR EACH slot IN batch:
            fence(Ordering::Acquire)
            DROP_VALUE(slot.value)  // May queue more
            MAYBE_DEALLOCATE(slot)
```

### 8.5 Cycle Detection

Reference counting cannot collect cycles. Blood uses **backup cycle collection**:

**Detection Trigger**:
- Memory pressure (heap > threshold)
- Explicit `collect_cycles()` call
- Periodic timer (configurable)

**Algorithm**: Mark-sweep over Tier 3 objects only:

```
COLLECT_CYCLES():
    // Phase 1: Gather all roots (including suspended continuations)
    roots ← all_global_roots()
          ∪ all_fiber_stacks()
          ∪ all_suspended_snapshot_refs()  // CRITICAL: Include snapshots

    // Phase 2: Mark all reachable from roots
    worklist ← roots
    marked ← {}
    WHILE NOT worklist.is_empty():
        obj ← worklist.pop()
        IF obj NOT IN marked:
            marked.insert(obj)
            worklist.extend(obj.references())

    // Phase 3: Sweep unmarked Tier 3 objects
    FOR EACH slot IN tier3_slots:
        IF slot NOT IN marked:
            FORCE_DROP(slot)
```

#### 8.5.1 Snapshot Roots Extraction

Suspended continuations hold generation snapshots that reference Tier 3 objects. These must be treated as roots:

```
all_suspended_snapshot_refs() → Set<Address>:
    snapshot_refs ← {}

    // Iterate all active effect handlers
    FOR EACH handler IN active_effect_handlers():
        FOR EACH continuation IN handler.captured_continuations:
            // Extract refs from the snapshot
            FOR EACH (addr, gen) IN continuation.snapshot.entries:
                slot ← memory[addr]
                IF slot.tier == Persistent:
                    snapshot_refs.insert(addr)

            // Also check for Tier 3 refs in continuation environment
            FOR EACH binding IN continuation.env:
                refs ← EXTRACT_TIER3_REFS(binding.value)
                snapshot_refs.extend(refs)

    RETURN snapshot_refs
```

#### 8.5.2 Cycle Collection and Effect Safety

**Invariant**: Cycle collection must not invalidate references held in suspended continuations.

**Guarantee**: By including snapshot references as roots, any Tier 3 object referenced by a suspended continuation is protected from collection.

**Ordering Constraint**: Cycle collection is safe at any point because:
1. Tier 3 objects use refcount (not generation) for liveness
2. Snapshot refs are treated as roots → refcount implicitly >0
3. No Tier 3 object reachable from a snapshot can be collected

#### 8.5.3 Concurrent Cycle Collection

When cycle collection runs concurrently with effect handlers:

```
SAFE_CYCLE_COLLECTION():
    // Acquire read-barrier on effect handler set
    WITH effect_handlers_lock.read():
        // Snapshot the current set of suspended continuations
        suspended ← snapshot_all_continuations()

    // Collection proceeds with consistent snapshot
    roots ← compute_roots(suspended)
    mark_and_sweep(roots)
```

**Note**: This allows effect handlers to resume during collection, but newly captured continuations after the snapshot will be handled in the next collection cycle.

### 8.6 Cross-Fiber Sharing

Tier 3 enables safe sharing across fibers:

```blood
fn share_config() {
    let config: Frozen<Config> = freeze(Config::load())
    // config is now Tier 3 with refcount

    let handle1 = spawn(|| use_config(&config))  // RC++
    let handle2 = spawn(|| use_config(&config))  // RC++

    // config.refcount == 3 (main + 2 fibers)

    await(handle1)  // RC--
    await(handle2)  // RC--

    // config.refcount == 1, dropped when main exits scope
}
```

---

## 9. Soundness Analysis

### 9.1 Safety Theorems

#### Theorem 1: No Use-After-Free (Tier 1)

**Statement**: If `ptr` is a Tier 1 pointer and `DEREFERENCE(ptr)` succeeds, then `ptr.address` contains a live value.

**Proof**:
1. On allocation, `ptr.generation = slot.generation`
2. On free, `slot.generation` increments
3. Dereference checks `ptr.generation == slot.generation`
4. After free, `ptr.generation ≠ slot.generation`
5. Therefore, dereference after free raises `StaleReference` ∎

#### Theorem 2: No Use-After-Free (Tier 0)

**Statement**: If `ptr` is a Tier 0 (stack) pointer, dereference always succeeds while `ptr` is live.

**Proof**:
1. Tier 0 pointers only exist for stack values
2. Escape analysis proves `ptr` doesn't outlive value
3. Stack frames deallocate LIFO
4. While `ptr` is in scope, value's frame is on stack
5. Therefore, dereference always succeeds ∎

#### Theorem 3: Generation Snapshot Safety

**Statement**: If a continuation with snapshot `S` is resumed, and validation succeeds, then all captured references are valid.

**Proof**:
1. Snapshot `S` records `(addr, gen)` pairs at capture
2. Validation checks each `addr` still has generation `gen`
3. If `gen` matches, the slot hasn't been freed and reallocated
4. Therefore, captured references point to original values ∎

#### Theorem 4: Linear Safety Across Effects

**Statement**: If a linear value is captured in a single-shot handler's continuation, it is used exactly once.

**Proof**:
1. Compiler rejects linear capture in multi-shot handlers
2. Single-shot handler resumes continuation at most once
3. Linear value in continuation is used at most once
4. Compiler ensures linear value is used at least once in continuation
5. Therefore, exactly once ∎

#### Theorem 5: Region Isolation

**Statement**: If `ptr` points into region `r` owned by fiber `F`, then only fiber `F` can dereference `ptr`.

**Proof**:
1. Regions are created fiber-local (ownership = creating fiber)
2. The type system prevents region references from crossing fiber boundaries
3. Spawn operations require values to be `Send`, which excludes region references
4. Only the owning fiber can access region contents
5. Therefore, only the owner can dereference region pointers ∎

#### Theorem 6: Region-Effect Suspension Safety

**Statement**: If a region `r` contains allocations referenced by a suspended continuation, then `r` is not deallocated until all such continuations complete.

**Proof**:
1. `REGION_EFFECT_CHECK` detects when region refs are captured in continuations
2. Captured regions have `suspend_count` incremented
3. `EXIT_REGION_SCOPE` checks `suspend_count > 0` before deallocation
4. If suspended, region enters `PendingDeallocation` state
5. `ON_CONTINUATION_COMPLETE` decrements `suspend_count` and deallocates when zero
6. Therefore, region lives until all capturing continuations complete ∎

#### Theorem 7: Snapshot Liveness Soundness

**Statement**: If a reference `r` is dereferenced after resume and `r` was excluded from the snapshot, a stale access may occur. Conversely, if all dereferenceable references are included, no stale access occurs undetected.

**Proof**:
1. Snapshot validation checks `∀(addr, gen) ∈ S. currentGen(addr) = gen`
2. If `r` is dereferenced and `r ∉ S`, validation doesn't check `r`
3. `r` could be stale, leading to use-after-free
4. If `r` is dereferenceable and `r ∈ S`, validation ensures freshness
5. Conservative liveness analysis guarantees all possibly-dereferenced refs included
6. Therefore, including all live refs ensures safety ∎

#### Theorem 8: Cycle Collection Safety with Effects

**Statement**: Cycle collection does not collect Tier 3 objects that are reachable from suspended continuations.

**Proof**:
1. `all_suspended_snapshot_refs()` extracts all refs from suspended continuations
2. These refs are added to the root set
3. Mark phase marks all objects reachable from roots
4. Sweep phase only collects unmarked objects
5. Suspended continuation refs are marked → not collected
6. Therefore, suspended continuations retain their references ∎

#### Theorem 9: Reserved Generation Correctness

**Statement**: No valid pointer can have generation equal to `PERSISTENT_MARKER` or other reserved values.

**Proof**:
1. Allocation preserves existing slot generation (within valid range)
2. FREE checks for `generation >= OVERFLOW_GUARD` before incrementing
3. If at `OVERFLOW_GUARD`, promotion occurs instead of increment
4. `PERSISTENT_MARKER` is only set by promotion, never by increment
5. Pointers are created with `slot.generation`, never with reserved values
6. Therefore, no valid pointer has reserved generation ∎

#### Theorem 10: Region Hierarchy Correctness

**Statement**: If region `r₂` is nested within region `r₁`, then `r₂` is always deallocated before `r₁`.

**Definitions**:
- Let `regions(F)` be the set of active regions for fiber `F`
- Let `parent(r)` denote the parent region of `r` (or `⊥` if root)
- Let `depth(r)` = 0 if `parent(r) = ⊥`, else `1 + depth(parent(r))`
- Let `dealloc_time(r)` be the logical timestamp when `r` is deallocated

**Proof**:
1. Regions are allocated on a per-fiber stack with LIFO semantics
2. `ENTER_REGION` pushes a new region onto the fiber's region stack
3. `EXIT_REGION_SCOPE` pops from the region stack (after suspend_count check)
4. By LIFO property: if `depth(r₂) > depth(r₁)`, then `r₂` was pushed after `r₁`
5. LIFO deallocation ensures `r₂` is popped before `r₁`
6. Therefore, `dealloc_time(r₂) < dealloc_time(r₁)` ∎

**Corollary**: References from child to parent region are always valid while the child exists.

**Proof**: Parent outlives child by Theorem 10, so parent's contents are valid during child's lifetime ∎

#### Theorem 11: Reference Counting Correctness (Concurrent)

**Statement**: For any Tier 3 object `o`, `refcount(o) = 0` implies `o` is unreachable from all roots.

**Definitions**:
- Let `R` be the set of all roots (stack, globals, suspended continuations)
- Let `reachable(R)` be the transitive closure of objects reachable from `R`
- Let `refcount(o)` be the atomic reference count of object `o`

**Invariants** (maintained by all operations):
- **INV-RC1**: `refcount(o) ≥ |{r ∈ R : o ∈ reachable({r})}|` (count ≥ root refs)
- **INV-RC2**: `refcount(o) ≥ |{p : p.field = o}|` (count ≥ field refs)
- **INV-RC3**: All refcount modifications are atomic (fetch_add, fetch_sub)

**Proof**:

*Part A: Increment correctness*
1. `RC_INCREMENT(o)` performs `atomic_fetch_add(&o.refcount, 1)`
2. Called before creating a new reference to `o`
3. Atomicity ensures no lost increments under concurrency
4. INV-RC1 and INV-RC2 preserved ✓

*Part B: Decrement correctness*
1. `RC_DECREMENT(o)` performs `old = atomic_fetch_sub(&o.refcount, 1)`
2. Called after removing a reference to `o`
3. If `old == 1`, the reference being removed was the last
4. Atomicity ensures no double-decrements

*Part C: Zero implies unreachable*
1. Assume `refcount(o) = 0` but `o ∈ reachable(R)` (contradiction hypothesis)
2. If `o ∈ reachable(R)`, there exists a path from some root `r` to `o`
3. By INV-RC1 and INV-RC2, each edge contributes to refcount
4. At least one reference exists → `refcount(o) ≥ 1`
5. Contradiction with `refcount(o) = 0`
6. Therefore, `refcount(o) = 0` implies `o ∉ reachable(R)` ∎

**Concurrent Safety Argument** (based on [Bacon-Rajan 2001](https://pages.cs.wisc.edu/~cymen/misc/interests/Bacon01Concurrent.pdf)):
- All refcount operations use acquire-release semantics
- Deferred decrement queue ensures no premature deallocation
- Memory fences establish happens-before ordering
- Read-Copy-Update (RCU) pattern for root set modification

#### Theorem 12: Cycle Collection Completeness

**Statement**: If a set of Tier 3 objects forms a garbage cycle (unreachable from roots), the cycle collection algorithm will eventually collect all objects in the cycle.

**Definitions**:
- Let `C = {o₁, o₂, ..., oₙ}` be a cycle where `o₁ → o₂ → ... → oₙ → o₁`
- Let `external_refs(C) = |{r : r ∉ C ∧ r.field ∈ C}|` (refs into cycle from outside)
- Garbage cycle: `external_refs(C) = 0` and `C ∩ reachable(R) = ∅`

**Algorithm Properties**:
1. `COLLECT_CYCLES` is triggered by memory pressure, timer, or explicit call
2. Mark phase: traverse from all roots, mark reachable objects
3. Sweep phase: deallocate all unmarked Tier 3 objects

**Proof**:

*Part A: Garbage cycles are unmarked*
1. Let `C` be a garbage cycle with `C ∩ reachable(R) = ∅`
2. Mark phase only marks objects reachable from roots `R`
3. By definition, no object in `C` is reachable from `R`
4. Therefore, no object in `C` is marked

*Part B: All unmarked objects are swept*
1. Sweep phase iterates all Tier 3 slots
2. For each slot `s`, if `s ∉ marked`, `FORCE_DROP(s)` is called
3. All objects in `C` are in Tier 3 (cycles only exist in Tier 3)
4. All objects in `C` are unmarked (Part A)
5. Therefore, all objects in `C` are swept

*Part C: Eventual collection*
1. Detection triggers ensure `COLLECT_CYCLES` runs periodically
2. Memory pressure trigger: when heap exceeds threshold
3. Timer trigger: configurable periodic collection
4. Each run collects all current garbage cycles
5. Therefore, any garbage cycle is eventually collected ∎

**Termination Guarantee**:
- Mark phase terminates: finite object graph, each object marked at most once
- Sweep phase terminates: finite number of Tier 3 slots

#### Theorem 13: Concurrent Cycle Collection Safety

**Statement**: Concurrent cycle collection does not collect live objects, even when effect handlers are actively capturing and resuming continuations.

**Definitions**:
- Let `snap_t` be the snapshot of suspended continuations at time `t`
- Let `roots_t = globals ∪ stacks ∪ snap_t`
- Let `live_t = reachable(roots_t)`

**Invariants**:
- **INV-CC1**: Collection uses `snap_t` captured atomically under read lock
- **INV-CC2**: Objects added to roots after `snap_t` are protected until next cycle
- **INV-CC3**: Objects in `live_t` are never collected in cycle starting at `t`

**Proof**:

*Part A: Snapshot consistency*
1. `SAFE_CYCLE_COLLECTION` acquires `effect_handlers_lock.read()`
2. `snapshot_all_continuations()` captures all suspended continuations atomically
3. Released lock allows handlers to resume/capture, but `snap_t` is immutable
4. Marking uses consistent `snap_t`, not a changing view

*Part B: Live objects protected*
1. `roots_t` includes all suspended continuation refs at time `t`
2. Mark phase marks all objects in `reachable(roots_t)`
3. Any object reachable from a suspended continuation is marked
4. Sweep only collects unmarked objects
5. Live objects (in `live_t`) are marked, thus not swept

*Part C: Concurrent handler safety*
1. Handler captures new continuation `κ` after `snap_t`: `κ ∉ snap_t`
2. Objects only in `κ` may be unmarked in current cycle
3. However, `κ` holds strong references (refcount > 0) to its objects
4. RC prevents deallocation even if unmarked by cycle collector
5. Next cycle includes `κ` in snapshot, objects properly marked
6. Therefore, no premature collection occurs ∎

**Liveness**: New garbage cycles created after `snap_t` are collected in subsequent cycle.

### 9.2 Proof Obligations

The following require formal proof (e.g., in Coq/Agda). All theorems now have proof sketches; mechanized verification is recommended for high-complexity proofs.

| Theorem | Status | Complexity | Section |
|---------|--------|------------|---------|
| No UAF (Tier 1) | Sketch provided | Low | 9.1 |
| No UAF (Tier 0) | Sketch provided | Medium (escape analysis) | 9.1 |
| Snapshot Safety | Sketch provided | Medium | 9.1 |
| Linear Safety | Sketch provided | Medium | 9.1 |
| Region Isolation | Sketch provided | Low | 9.1, 7.8 |
| Region-Effect Suspension | Sketch provided | Medium | 9.1, 7.7 |
| Snapshot Liveness Soundness | Sketch provided | Medium | 9.1, 6.4.1 |
| Cycle Collection + Effects | Sketch provided | Medium | 9.1, 8.5 |
| Reserved Generation Correctness | Sketch provided | Low | 9.1, 4.5 |
| Region Hierarchy | Sketch provided | Low | 9.1 |
| RC Correctness (Concurrent) | Sketch provided | High (concurrency) | 9.1, 8.3 |
| Cycle Collection Completeness | Sketch provided | High | 9.1, 8.5 |
| Concurrent Cycle Collection | Sketch provided | High | 9.1, 8.5.3 |

**Mechanized Verification Recommendations**:

| Theorem | Recommended Tool | Priority |
|---------|------------------|----------|
| RC Correctness (Concurrent) | [Verus](https://github.com/verus-lang/verus) or Iris/Coq | High |
| Concurrent Cycle Collection | Iris/Coq with concurrent separation logic | High |
| Cycle Collection Completeness | Coq/Agda | Medium |
| Region Hierarchy | Agda (straightforward induction) | Low |

**References for Mechanized Proofs**:
- Verus: Rust verification framework ([SOSP 2024](https://dl.acm.org/doi/10.1145/3694715.3695952))
- Iris: Higher-order concurrent separation logic (MPI-SWS)
- [Bacon-Rajan 2001](https://pages.cs.wisc.edu/~cymen/misc/interests/Bacon01Concurrent.pdf): Concurrent cycle collection proofs

### 9.3 What's Established vs. Novel

| Component | Status | Notes |
|-----------|--------|-------|
| Generational refs | Established (Vale) | Core mechanism validated |
| Escape analysis | Established (Java, Go) | Well-understood algorithms |
| RC + cycles | Established (Swift, Python) | Standard approach |
| Fiber isolation | Established (Erlang, Pony) | Message-passing concurrency |
| Scoped regions | Established (Cyclone, Rust) | Region-based memory management |
| Generation snapshots | **Novel (Blood)** | No prior art for effect continuations |
| Snapshot + linear | **Novel (Blood)** | Novel interaction analysis |
| Effects + generations | **Novel (Blood)** | Novel safety model |
| Region suspension | **Novel (Blood)** | Deferred deallocation for effects |
| Snapshot liveness opt. | **Novel (Blood)** | Dereference-based filtering |
| Reserved gen values | **Novel (Blood)** | Overflow handling without collision |

---

## 10. Implementation Notes

### 10.1 Compiler Phases

```
Source → Parse → Type Check → Escape Analysis → Tier Assignment
       → Check Elision → Snapshot Insertion → Codegen
```

### 10.2 Runtime Components

| Component | Responsibility |
|-----------|----------------|
| `blood_alloc` | Region allocation, slot management |
| `blood_deref` | Generation check, StaleReference raise |
| `blood_free` | Deallocation, generation increment |
| `blood_snapshot` | Capture, validate, optimize |
| `blood_rc` | Reference counting, cycle collection |
| `blood_region` | Region create/destroy, hierarchy |

### 10.3 Tuning Parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `REGION_INITIAL_SIZE` | 64 KB | Initial region allocation |
| `REGION_GROWTH_FACTOR` | 2.0 | Region expansion multiplier |
| `RC_DEFER_QUEUE_SIZE` | 1024 | Deferred decrement queue |
| `CYCLE_COLLECT_THRESHOLD` | 10 MB | Tier 3 size trigger |
| `SNAPSHOT_LAZY_THRESHOLD` | 8 | Refs before lazy validation |

### 10.4 Debug Mode

In debug builds:
- Full generation checks even when elision is safe
- Snapshot validation always eager
- Quarantine freed memory (detect late access)
- Track allocation call sites

### 10.5 Platform Considerations

| Platform | 128-bit Atomics | Notes |
|----------|-----------------|-------|
| x86-64 | `CMPXCHG16B` | Requires alignment |
| ARM64 | `LDXP/STXP` | Native support |
| WASM | Emulated | Use mutex fallback |
| RISC-V | Extension | Requires A extension |

---

## Appendix A: Pointer Layout Reference

```
128-bit Blood Pointer (Tier 1/2):
┌────────────────────────────────────────────────────────────────────────────┐
│ Bit  │ 127:64          │ 63:32           │ 31:28 │ 27:24 │ 23:0           │
├──────┼─────────────────┼─────────────────┼───────┼───────┼────────────────┤
│ Name │ ADDRESS         │ GENERATION      │ TIER  │ FLAGS │ TYPE_FP        │
│ Size │ 64 bits         │ 32 bits         │ 4 bits│ 4 bits│ 24 bits        │
└────────────────────────────────────────────────────────────────────────────┘

64-bit Stack Pointer (Tier 0):
┌────────────────────────────────────────────────────────────────────────────┐
│ Bit  │ 63:0                                                                │
├──────┼─────────────────────────────────────────────────────────────────────┤
│ Name │ ADDRESS                                                             │
│ Size │ 64 bits                                                             │
└────────────────────────────────────────────────────────────────────────────┘

Persistent Slot Header:
┌────────────────────────────────────────────────────────────────────────────┐
│ Offset │ 0:7             │ 8:15            │ 16:19     │ 20:...           │
├────────┼─────────────────┼─────────────────┼───────────┼──────────────────┤
│ Name   │ REFCOUNT        │ WEAK_COUNT      │ METADATA  │ VALUE            │
│ Size   │ 64 bits         │ 32 bits         │ 32 bits   │ Variable         │
└────────────────────────────────────────────────────────────────────────────┘
```

---

## Appendix B: StaleReference Effect Contract

```blood
/// The StaleReference effect is raised when a generational reference
/// is dereferenced after the underlying memory has been freed and
/// potentially reallocated.
///
/// Handlers MUST diverge (return `never`). Attempting to resume
/// normally after a stale reference is undefined behavior.
effect StaleReference {
    /// Report a stale reference.
    ///
    /// # Arguments
    /// * `info` - Details about the stale access
    ///
    /// # Returns
    /// This operation never returns normally.
    op stale(info: StaleInfo) -> never
}

/// Information about a stale reference access.
struct StaleInfo {
    /// Memory address that was accessed
    address: usize,

    /// Generation expected by the pointer
    expected_generation: u32,

    /// Actual generation in the memory slot
    actual_generation: u32,

    /// When the snapshot was captured (for debugging)
    capture_timestamp: Option<Instant>,

    /// Source location of the dereference (debug builds)
    source_location: Option<SourceLoc>,
}

/// Standard handler: panic with diagnostic
handler PanicOnStale for StaleReference {
    op stale(info) {
        panic!("MEMORY SAFETY VIOLATION: Use-after-free detected\n\
                Address: {:p}\n\
                Expected generation: {}\n\
                Actual generation: {}\n\
                Generation delta: {} (freed {} times since capture)",
               info.address,
               info.expected_generation,
               info.actual_generation,
               info.actual_generation.wrapping_sub(info.expected_generation))
    }
}
```

---

*This document is part of the Blood Language Specification.*
