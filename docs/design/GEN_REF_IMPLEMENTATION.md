# DD-1: Generational Reference Implementation

**Status:** CLOSED — Header-field approach with static elision
**Date:** 2026-03-15
**Blocks:** 14 modules, entire Pillar 1 (Veracity) enforcement
**Also resolves:** DD-6 (Reference Syntax)

## Problem

Blood's Pillar 1 (Veracity) requires generational references for memory safety without a garbage collector or borrow checker. References must detect use-after-free at runtime. Two approaches were considered:

1. **128-bit fat pointers** (original spec): `{ address: u64, generation: u32, metadata: u32 }`
2. **Side-table lookup**: 64-bit pointers with generation stored in a global hash table
3. **Header-field** (Vale's approach): 64-bit pointers with generation stored at `[ptr - 8]`

## Research Summary

| Approach | Pointer size | Check cost | Cache impact | Measured overhead |
|----------|-------------|------------|--------------|-------------------|
| 128-bit fat ptr | 16 bytes | 1 inline compare | Doubles pointer footprint | 3-65% (CHERI), worst on ptr-heavy |
| Side table | 8 bytes + hash lookup | 1 hash + 1 compare | Table may miss cache | ~15-20% estimated |
| Header field | 8 bytes + header load | 1 load at `[ptr-8]` + 1 compare | Minimal — gen adjacent to data | ~11% (Vale unoptimized) |

Critical finding from CHERI/Morello research: **128-bit pointers cause 1.4-2x slowdown in pointer-heavy workloads** due to cache pressure, not instruction count. The mcf benchmark (extremely pointer-heavy, similar to compiler registries) showed 64% overhead. The Blood compiler itself — with ADT registries, type interners, and symbol tables — is exactly this kind of pointer-heavy workload.

Vale's research shows **~11% overhead unoptimized, <5% with static elision (HGM)**, and **0% for region-scoped data** where the compiler can prove safety statically.

## Decision: Header-Field with Static Elision

### Reference representation stays at 64 bits

`&T` remains a plain pointer. No ABI change. No FFI bridge blocks needed. No doubling of struct sizes containing references.

### Generation stored in allocation header

Every region-allocated or persistent-allocated object gets an 8-byte header preceding the data:

```
[generation: u32][padding: u32][user data starts here]
                               ^--- pointer points here
```

Allocation functions (`blood_region_alloc`, `blood_alloc_or_abort`) return `data_start`, not `header_start`. The generation is at `[ptr - 8]`.

### Generation check on dereference

```llvm
%gen_ptr = getelementptr i8, ptr %ref, i64 -8
%gen = load i32, ptr %gen_ptr
%expected = <compile-time or captured generation>
%valid = icmp eq i32 %gen, %expected
br i1 %valid, label %ok, label %stale
stale:
  call void @blood_stale_reference_panic(i32 %expected, i32 %gen)
  unreachable
ok:
  ; proceed with dereference
```

### Static elision (critical optimization)

Blood's escape analysis already classifies locals:
- `NoEscape` / `EffectLocal` → **Stack tier: no gen check.** Pointer validity is guaranteed by stack frame lifetime.
- `ArgEscape` / `EffectCapture` → **Region tier: gen check.** Pointer may outlive allocation.
- `HeapEscape` / `GlobalEscape` → **Persistent tier: gen check + RC.**

The compiler SKIPS generation checks for stack-tier references. Since the selfhost compiler keeps most data on the stack tier (current escape analysis shows most locals as `NoEscape`), the majority of dereferences are zero-cost.

### Reference creation

When taking `&x` where `x` is region-allocated:
1. Load generation from `[ptr - 8]`
2. Store generation alongside the pointer in a local (not in the pointer itself)
3. On dereference, compare stored generation against current generation at `[ptr - 8]`

For stack-allocated locals, `&x` produces a plain pointer with no generation tracking.

### DD-6 resolution: Reference syntax

`&T` is always a plain 64-bit pointer. The generation is tracked separately (in a local variable at the reference creation site, not in the pointer representation). No new syntax (`gen &T`) is needed. All references are uniform — the check is conditional on the allocation tier, resolved at compile time by escape analysis.

### Performance threshold

**Go/no-go criterion:** Selfhost build time regression < 15%.

Expected breakdown:
- Stack-tier references (majority): 0% overhead
- Region-tier references (minority): ~11% per dereference (one load + one compare)
- Static elision removes checks where owning scope is provably live

### Incremental implementation path

1. **Runtime:** Add 8-byte header to `blood_region_alloc`. Expose `blood_check_generation(ptr, expected_gen) -> bool`.
2. **Codegen (region alloc):** Adjust allocation to reserve header space. Return data pointer (header + 8).
3. **Codegen (&x):** For region-allocated locals, load generation from header and store alongside pointer.
4. **Codegen (*ref):** For region-tier references, emit gen check before dereference. Skip for stack-tier.
5. **Codegen (region destroy):** Increment generation in all headers within the region, invalidating all references.
6. **Benchmark:** Measure selfhost build time. If > 15% regression, investigate static elision coverage.

## Rationale

- **128-bit fat pointers rejected** because CHERI research shows they cause structural cache overhead proportional to pointer density. The compiler is pointer-heavy — this would be the worst case.
- **Side-table rejected** because hash table lookup per dereference adds unpredictable latency and cache misses for the table itself.
- **Header-field chosen** because:
  - Pointers stay 64-bit (no ABI change, no cache blowup)
  - Generation load is spatially local (adjacent to object data, likely in same cache line)
  - Vale's measured 11% overhead is acceptable and improvable with static elision
  - Blood's escape analysis already provides the tier classification needed for elision

## Sources

- Vale generational references: verdagon.dev/blog/generational-references (~11% overhead)
- Vale HGM: verdagon.dev/blog/hybrid-generational-memory (static elision)
- CHERI/Morello performance: IISWC'25 (1.4-2x on pointer-heavy workloads)
- Fat pointer overhead: arxiv.org/abs/2208.12900 (64% on mcf)
