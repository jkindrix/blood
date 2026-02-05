# Generation-Aware Slab Allocator Design

**Version**: 1.0.0
**Status**: Design Complete
**Target**: `blood-rust/blood-runtime/src/memory.rs`
**Author**: Design session 2026-02-03

---

## Executive Summary

This document specifies a **Generation-Aware Slab Allocator** for Blood's runtime that:

1. **Enables memory reuse within regions** - freed memory goes to size-class free lists
2. **Maintains generation semantics** - every free increments generation (SSM compliance)
3. **Preserves region bulk-free** - region_destroy still O(n) and frees everything
4. **Integrates with existing slot registry** - unified tracking of all allocations

This replaces the current bump-only allocator that cannot reclaim memory until region destruction.

---

## Table of Contents

1. [Problem Statement](#1-problem-statement)
2. [Design Goals](#2-design-goals)
3. [Architecture Overview](#3-architecture-overview)
4. [Data Structures](#4-data-structures)
5. [Size Class Design](#5-size-class-design)
6. [Core Algorithms](#6-core-algorithms)
7. [Region Integration](#7-region-integration)
8. [FFI Interface](#8-ffi-interface)
9. [Implementation Guide](#9-implementation-guide)
10. [Migration Path](#10-migration-path)
11. [Testing Strategy](#11-testing-strategy)

---

## 1. Problem Statement

### Current Behavior

```
blood_region_alloc(size):
    offset = region.offset
    region.offset += size
    return region.base + offset    // Offset ONLY goes up

blood_unregister_allocation(addr):
    slot.is_allocated = false
    slot.generation += 1
    // Memory is NOT returned to any free list
    // Cannot be reused until region_destroy
```

### Consequence

```
Vec<Token> growth cycle:
  1. Allocate 64 bytes  → offset = 64
  2. Allocate 128 bytes → offset = 192, "free" 64 (no-op)
  3. Allocate 256 bytes → offset = 448, "free" 128 (no-op)

Result: 448 bytes consumed, 256 bytes live, 192 bytes leaked until region_destroy
```

### Impact

- Self-hosted compiler OOMs on large files
- 45x memory usage vs reference compiler
- Cannot compile itself (self-hosting blocked)

---

## 2. Design Goals

| Goal | Metric | Current | Target |
|------|--------|---------|--------|
| Memory reuse | Freed bytes reusable | 0% | 100% |
| Allocation speed | Cycles per alloc | ~20 | ~25 (acceptable overhead) |
| Deallocation speed | Cycles per free | ~10 | ~15 (acceptable overhead) |
| Generation compliance | SSM spec compliance | ✅ | ✅ (must maintain) |
| Region bulk-free | O(n) destruction | ✅ | ✅ (must maintain) |
| Thread safety | Lock-free hot path | ✅ | ✅ (must maintain) |

### Non-Goals

- Perfect fit allocation (some internal fragmentation acceptable)
- Compacting/defragmenting (too complex, not needed)
- Cross-region memory sharing (violates region semantics)

---

## 3. Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                    Generation-Aware Slab Allocator                   │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │                      SlotRegistry (existing)                   │  │
│  │  HashMap<u64, SlotEntry>                                       │  │
│  │  - address → (generation, size, is_allocated, size_class)     │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                              │                                       │
│                              ▼                                       │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │                    RegionAllocator (per region)                │  │
│  │                                                                │  │
│  │  ┌─────────────────────────────────────────────────────────┐  │  │
│  │  │              Size Class Free Lists                       │  │  │
│  │  │                                                          │  │  │
│  │  │  [8B]   → [addr1] → [addr2] → [addr3] → nil             │  │  │
│  │  │  [16B]  → [addr4] → nil                                  │  │  │
│  │  │  [32B]  → nil                                            │  │  │
│  │  │  [64B]  → [addr5] → [addr6] → nil                        │  │  │
│  │  │  [128B] → [addr7] → nil                                  │  │  │
│  │  │  ...                                                     │  │  │
│  │  │  [4KB]  → nil                                            │  │  │
│  │  └─────────────────────────────────────────────────────────┘  │  │
│  │                              │                                 │  │
│  │                              ▼                                 │  │
│  │  ┌─────────────────────────────────────────────────────────┐  │  │
│  │  │              Bump Allocator (fallback)                   │  │  │
│  │  │  base: *mut u8                                           │  │  │
│  │  │  offset: AtomicUsize  ─── only used when free list empty │  │  │
│  │  │  committed: AtomicUsize                                  │  │  │
│  │  └─────────────────────────────────────────────────────────┘  │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                                                                     │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │                    Large Allocation Tracker                    │  │
│  │  HashMap<u64, LargeAlloc>  (sizes > MAX_SIZE_CLASS)           │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

### Key Insight

The free list stores **addresses of freed slots**. When we need to allocate:
1. Check free list for matching size class
2. If found: reuse that address (generation already incremented on previous free)
3. If not found: bump allocate from region

---

## 4. Data Structures

### 4.1 Extended SlotEntry

```rust
/// Entry in the slot registry tracking allocation state.
#[derive(Debug, Clone, Copy)]
pub struct SlotEntry {
    /// Current generation of this slot.
    pub generation: Generation,
    /// Size of the allocation.
    pub size: usize,
    /// Whether the slot is currently allocated.
    pub is_allocated: bool,
    /// Size class index (0-11 for slab, 255 for large).
    pub size_class: u8,
    /// Region ID this allocation belongs to.
    pub region_id: u64,
}

impl SlotEntry {
    /// Create a new allocated slot entry.
    pub fn new(generation: Generation, size: usize, size_class: u8, region_id: u64) -> Self {
        Self {
            generation,
            size,
            is_allocated: true,
            size_class,
            region_id,
        }
    }

    /// Mark as deallocated, increment generation, return address to free list.
    pub fn deallocate(&mut self) -> bool {
        if !self.is_allocated {
            return false; // Already freed
        }
        self.is_allocated = false;
        if self.generation < generation::OVERFLOW_GUARD {
            self.generation += 1;
        }
        true // Caller should add to free list
    }
}
```

### 4.2 Size Class Definition

```rust
/// Size class configuration.
#[derive(Debug, Clone, Copy)]
pub struct SizeClass {
    /// Maximum allocation size for this class.
    pub max_size: usize,
    /// Actual slot size (with alignment padding).
    pub slot_size: usize,
    /// Index in the size class array.
    pub index: u8,
}

/// All size classes. Power-of-2 sizes for efficient lookup.
pub const SIZE_CLASSES: [SizeClass; 12] = [
    SizeClass { max_size: 8,    slot_size: 8,    index: 0 },
    SizeClass { max_size: 16,   slot_size: 16,   index: 1 },
    SizeClass { max_size: 32,   slot_size: 32,   index: 2 },
    SizeClass { max_size: 64,   slot_size: 64,   index: 3 },
    SizeClass { max_size: 128,  slot_size: 128,  index: 4 },
    SizeClass { max_size: 256,  slot_size: 256,  index: 5 },
    SizeClass { max_size: 512,  slot_size: 512,  index: 6 },
    SizeClass { max_size: 1024, slot_size: 1024, index: 7 },
    SizeClass { max_size: 2048, slot_size: 2048, index: 8 },
    SizeClass { max_size: 4096, slot_size: 4096, index: 9 },
    SizeClass { max_size: 8192, slot_size: 8192, index: 10 },
    SizeClass { max_size: 16384, slot_size: 16384, index: 11 },
];

/// Size class for large allocations (>16KB).
pub const SIZE_CLASS_LARGE: u8 = 255;

/// Maximum size handled by slab allocator.
pub const MAX_SLAB_SIZE: usize = 16384;
```

### 4.3 Free List Structure

```rust
/// Per-size-class free list for a region.
///
/// Uses a simple Vec<u64> of freed addresses. This is sufficient because:
/// - Pop/push are O(1)
/// - No ordering requirements
/// - Region destruction clears all lists anyway
#[derive(Debug)]
pub struct SizeClassFreeList {
    /// Freed slot addresses available for reuse.
    slots: Vec<u64>,
    /// Statistics: total reuses from this list.
    reuse_count: u64,
}

impl SizeClassFreeList {
    pub fn new() -> Self {
        Self {
            slots: Vec::with_capacity(64), // Pre-allocate for common case
            reuse_count: 0,
        }
    }

    /// Try to get a free slot. Returns None if list is empty.
    #[inline]
    pub fn pop(&mut self) -> Option<u64> {
        let addr = self.slots.pop()?;
        self.reuse_count += 1;
        Some(addr)
    }

    /// Return a slot to the free list.
    #[inline]
    pub fn push(&mut self, addr: u64) {
        self.slots.push(addr);
    }

    /// Clear all entries (called on region destroy).
    pub fn clear(&mut self) {
        self.slots.clear();
    }

    /// Number of available slots.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Total reuses from this list.
    pub fn reuse_count(&self) -> u64 {
        self.reuse_count
    }
}
```

### 4.4 Region Allocator (Extended)

```rust
/// Extended region with slab allocation support.
pub struct RegionAllocator {
    /// Region ID.
    id: RegionId,

    /// Base pointer (stable for lifetime on Unix).
    #[cfg(unix)]
    base: *mut u8,

    /// Reserved address space.
    #[cfg(unix)]
    reserved: usize,

    /// Committed bytes.
    #[cfg(unix)]
    committed: AtomicUsize,

    /// Bump pointer offset (used when free lists empty).
    offset: AtomicUsize,

    /// Maximum size.
    max_size: usize,

    /// Per-size-class free lists.
    /// Protected by mutex for thread safety.
    free_lists: Mutex<[SizeClassFreeList; 12]>,

    /// Large allocation tracking (>16KB).
    large_allocs: Mutex<HashMap<u64, LargeAlloc>>,

    /// Statistics.
    stats: RegionStats,

    /// Status flags (existing).
    closed: AtomicU32,
    suspend_count: AtomicU32,
    status: AtomicU32,
}

/// Statistics for region allocation.
#[derive(Debug, Default)]
pub struct RegionStats {
    /// Total allocations.
    pub allocations: AtomicU64,
    /// Allocations satisfied from free list.
    pub reused: AtomicU64,
    /// Allocations requiring bump.
    pub bumped: AtomicU64,
    /// Total deallocations.
    pub deallocations: AtomicU64,
    /// Large allocations.
    pub large_allocs: AtomicU64,
}

/// Tracking for large allocations (>16KB).
#[derive(Debug)]
pub struct LargeAlloc {
    pub size: usize,
    pub generation: Generation,
}
```

---

## 5. Size Class Design

### 5.1 Size Class Selection

```rust
/// Get the size class index for a given size.
/// Returns SIZE_CLASS_LARGE (255) if size exceeds MAX_SLAB_SIZE.
#[inline]
pub fn size_class_for(size: usize) -> u8 {
    // Fast path: use leading zeros to find power-of-2 bucket
    if size == 0 {
        return 0;
    }
    if size > MAX_SLAB_SIZE {
        return SIZE_CLASS_LARGE;
    }

    // Round up to next power of 2, then find index
    let rounded = size.next_power_of_two();
    let index = rounded.trailing_zeros();

    // Map to our size classes (starting at 8 = 2^3)
    if index < 3 {
        0 // Sizes 1-8 go to class 0 (8 bytes)
    } else {
        (index - 3).min(11) as u8
    }
}

/// Get the slot size for a size class.
#[inline]
pub fn slot_size_for_class(class: u8) -> usize {
    if class == SIZE_CLASS_LARGE || class as usize >= SIZE_CLASSES.len() {
        0 // Large allocs tracked separately
    } else {
        SIZE_CLASSES[class as usize].slot_size
    }
}
```

### 5.2 Internal Fragmentation Analysis

| Requested | Class | Slot Size | Wasted | Waste % |
|-----------|-------|-----------|--------|---------|
| 1-8       | 0     | 8         | 0-7    | 0-87%   |
| 9-16      | 1     | 16        | 0-7    | 0-43%   |
| 17-32     | 2     | 32        | 0-15   | 0-46%   |
| 33-64     | 3     | 64        | 0-31   | 0-48%   |
| 65-128    | 4     | 128       | 0-63   | 0-49%   |
| ...       | ...   | ...       | ...    | ...     |

**Worst case**: ~50% internal fragmentation for sizes just above a power of 2.

**Mitigation**: Most compiler allocations cluster around specific sizes (AST nodes, tokens, types). Profiling can add intermediate size classes if needed.

---

## 6. Core Algorithms

### 6.1 Allocation

```rust
impl RegionAllocator {
    /// Allocate memory from the region.
    ///
    /// Strategy:
    /// 1. Determine size class
    /// 2. Try free list for that class
    /// 3. Fall back to bump allocation
    /// 4. Register in slot registry
    pub fn allocate(&self, size: usize, align: usize) -> Option<u64> {
        if self.is_closed() {
            return None;
        }

        let class = size_class_for(size);

        if class == SIZE_CLASS_LARGE {
            return self.allocate_large(size, align);
        }

        let slot_size = slot_size_for_class(class);

        // Try free list first (fast path)
        {
            let mut lists = self.free_lists.lock();
            if let Some(addr) = lists[class as usize].pop() {
                // Reusing freed slot - generation was already incremented on free
                // Just mark as allocated in registry
                self.mark_allocated(addr, size, class);
                self.stats.reused.fetch_add(1, Ordering::Relaxed);
                self.stats.allocations.fetch_add(1, Ordering::Relaxed);
                return Some(addr);
            }
        }

        // Free list empty - bump allocate
        let addr = self.bump_allocate(slot_size, align)?;

        // Register new allocation
        let generation = self.register_new_allocation(addr, size, class);

        self.stats.bumped.fetch_add(1, Ordering::Relaxed);
        self.stats.allocations.fetch_add(1, Ordering::Relaxed);

        Some(addr)
    }

    /// Bump allocate from the region's contiguous memory.
    fn bump_allocate(&self, size: usize, align: usize) -> Option<u64> {
        loop {
            let offset = self.offset.load(Ordering::Acquire);
            let aligned_offset = round_up(offset, align);
            let new_offset = aligned_offset + size;

            if new_offset > self.reserved {
                return None; // Out of reserved space
            }

            // Commit more pages if needed
            self.ensure_committed(new_offset)?;

            // CAS to claim this range
            if self.offset.compare_exchange_weak(
                offset,
                new_offset,
                Ordering::AcqRel,
                Ordering::Relaxed
            ).is_ok() {
                return Some(unsafe { self.base.add(aligned_offset) as u64 });
            }
            // CAS failed, retry
        }
    }

    /// Register a new allocation in the slot registry.
    fn register_new_allocation(&self, addr: u64, size: usize, class: u8) -> Generation {
        let mut slots = slot_registry().slots.write();

        if let Some(entry) = slots.get_mut(&addr) {
            // Slot existed before (was freed) - generation already correct
            entry.is_allocated = true;
            entry.size = size;
            entry.size_class = class;
            entry.region_id = self.id.as_u64();
            entry.generation
        } else {
            // Brand new slot
            let entry = SlotEntry::new(
                generation::FIRST,
                size,
                class,
                self.id.as_u64()
            );
            let gen = entry.generation;
            slots.insert(addr, entry);
            gen
        }
    }

    /// Mark an existing slot as allocated (for free list reuse).
    fn mark_allocated(&self, addr: u64, size: usize, class: u8) {
        let mut slots = slot_registry().slots.write();
        if let Some(entry) = slots.get_mut(&addr) {
            entry.is_allocated = true;
            entry.size = size;
            entry.size_class = class;
        }
    }
}
```

### 6.2 Deallocation

```rust
impl RegionAllocator {
    /// Deallocate memory, returning it to the appropriate free list.
    ///
    /// Steps:
    /// 1. Look up slot in registry
    /// 2. Increment generation (invalidates all pointers)
    /// 3. Mark as not allocated
    /// 4. Add address to free list for its size class
    pub fn deallocate(&self, addr: u64) -> bool {
        let (class, should_free) = {
            let mut slots = slot_registry().slots.write();

            if let Some(entry) = slots.get_mut(&addr) {
                if !entry.is_allocated {
                    return false; // Double free
                }
                if entry.region_id != self.id.as_u64() {
                    return false; // Wrong region
                }

                let class = entry.size_class;
                let should_free = entry.deallocate(); // Increments generation
                (class, should_free)
            } else {
                return false; // Unknown address
            }
        };

        if !should_free {
            return false;
        }

        // Add to appropriate free list
        if class == SIZE_CLASS_LARGE {
            // Large allocations don't go to slab free list
            // They're handled separately
            let mut large = self.large_allocs.lock();
            large.remove(&addr);
        } else {
            let mut lists = self.free_lists.lock();
            lists[class as usize].push(addr);
        }

        self.stats.deallocations.fetch_add(1, Ordering::Relaxed);
        true
    }
}
```

### 6.3 Large Allocation Handling

```rust
impl RegionAllocator {
    /// Allocate a large object (>16KB).
    ///
    /// Large objects are bump-allocated but tracked separately
    /// so they can be individually freed back to the OS if desired.
    fn allocate_large(&self, size: usize, align: usize) -> Option<u64> {
        let addr = self.bump_allocate(size, align)?;

        let generation = self.register_new_allocation(addr, size, SIZE_CLASS_LARGE);

        let mut large = self.large_allocs.lock();
        large.insert(addr, LargeAlloc { size, generation });

        self.stats.large_allocs.fetch_add(1, Ordering::Relaxed);
        self.stats.allocations.fetch_add(1, Ordering::Relaxed);

        Some(addr)
    }
}
```

---

## 7. Region Integration

### 7.1 Region Destruction

```rust
impl RegionAllocator {
    /// Destroy the region, invalidating all allocations.
    ///
    /// Steps:
    /// 1. Increment generation for ALL slots in this region
    /// 2. Clear all free lists
    /// 3. Clear large allocation tracking
    /// 4. Unmap/release memory
    pub fn destroy(&mut self) {
        // Step 1: Invalidate all allocations by incrementing generations
        {
            let mut slots = slot_registry().slots.write();
            let region_id = self.id.as_u64();

            for (_, entry) in slots.iter_mut() {
                if entry.region_id == region_id && entry.is_allocated {
                    entry.deallocate(); // Increments generation
                }
            }

            // Remove all entries for this region
            slots.retain(|_, entry| entry.region_id != region_id);
        }

        // Step 2: Clear free lists
        {
            let mut lists = self.free_lists.lock();
            for list in lists.iter_mut() {
                list.clear();
            }
        }

        // Step 3: Clear large allocation tracking
        {
            let mut large = self.large_allocs.lock();
            large.clear();
        }

        // Step 4: Release memory back to OS
        #[cfg(unix)]
        unsafe {
            libc::munmap(self.base as *mut libc::c_void, self.reserved);
        }
    }

    /// Reset the region for reuse (keeps memory mapped).
    pub fn reset(&mut self) {
        // Invalidate all current allocations
        {
            let mut slots = slot_registry().slots.write();
            let region_id = self.id.as_u64();

            for (_, entry) in slots.iter_mut() {
                if entry.region_id == region_id {
                    if entry.is_allocated {
                        entry.deallocate();
                    }
                }
            }
        }

        // Clear free lists (don't want stale addresses)
        {
            let mut lists = self.free_lists.lock();
            for list in lists.iter_mut() {
                list.clear();
            }
        }

        // Clear large allocations
        {
            let mut large = self.large_allocs.lock();
            large.clear();
        }

        // Reset bump pointer
        self.offset.store(0, Ordering::Release);

        // Reset status
        self.closed.store(0, Ordering::Release);
        self.suspend_count.store(0, Ordering::Release);
        self.status.store(RegionStatus::Active as u32, Ordering::Release);
    }
}
```

### 7.2 Effect Suspension Integration

No changes needed - the existing suspension mechanism works with the new allocator:

- `suspend_count` tracking unchanged
- `PendingDeallocation` status unchanged
- Generation snapshots work the same (generations still increment on free)

---

## 8. FFI Interface

### 8.1 Updated FFI Functions

```rust
/// Allocate memory from a region with slab support.
///
/// Returns the address of the allocated memory, or 0 on failure.
#[no_mangle]
pub extern "C" fn blood_region_alloc(region_id: u64, size: usize, align: usize) -> u64 {
    let registry = get_region_registry();
    let mut reg = registry.lock();

    if let Some(region) = get_region_by_id(&mut reg, region_id) {
        region.allocate(size, align).unwrap_or(0)
    } else {
        0
    }
}

/// Deallocate memory, returning it to the region's free list.
///
/// This now actually enables memory reuse within the region.
#[no_mangle]
pub extern "C" fn blood_region_dealloc(region_id: u64, addr: u64) -> u32 {
    let registry = get_region_registry();
    let mut reg = registry.lock();

    if let Some(region) = get_region_by_id(&mut reg, region_id) {
        if region.deallocate(addr) { 1 } else { 0 }
    } else {
        0
    }
}

/// Unregister an allocation (legacy interface - calls region_dealloc internally).
#[no_mangle]
pub extern "C" fn blood_unregister_allocation(address: u64) {
    // Look up which region this allocation belongs to
    let region_id = {
        let slots = slot_registry().slots.read();
        slots.get(&address).map(|e| e.region_id)
    };

    if let Some(rid) = region_id {
        if rid != 0 {
            // Belongs to a region - use region dealloc
            let registry = get_region_registry();
            let mut reg = registry.lock();
            if let Some(region) = get_region_by_id(&mut reg, rid) {
                region.deallocate(address);
                return;
            }
        }
    }

    // Fall back to just updating registry (non-region allocation)
    unregister_allocation(address);
}

/// Get region allocation statistics.
#[no_mangle]
pub extern "C" fn blood_region_get_stats(
    region_id: u64,
    out_allocations: *mut u64,
    out_reused: *mut u64,
    out_bumped: *mut u64,
    out_deallocations: *mut u64,
) {
    let registry = get_region_registry();
    let reg = registry.lock();

    if let Some(region) = reg.get(&region_id) {
        unsafe {
            *out_allocations = region.stats.allocations.load(Ordering::Relaxed);
            *out_reused = region.stats.reused.load(Ordering::Relaxed);
            *out_bumped = region.stats.bumped.load(Ordering::Relaxed);
            *out_deallocations = region.stats.deallocations.load(Ordering::Relaxed);
        }
    }
}
```

### 8.2 New FFI Functions

```rust
/// Get the size class for a given size.
#[no_mangle]
pub extern "C" fn blood_size_class_for(size: usize) -> u8 {
    size_class_for(size)
}

/// Get the slot size for a size class.
#[no_mangle]
pub extern "C" fn blood_slot_size_for_class(class: u8) -> usize {
    slot_size_for_class(class)
}

/// Get the number of free slots in a size class.
#[no_mangle]
pub extern "C" fn blood_region_free_list_len(region_id: u64, class: u8) -> usize {
    let registry = get_region_registry();
    let reg = registry.lock();

    if let Some(region) = reg.get(&region_id) {
        if class < 12 {
            let lists = region.free_lists.lock();
            lists[class as usize].len()
        } else {
            0
        }
    } else {
        0
    }
}
```

---

## 9. Implementation Guide

### 9.1 Files to Modify

| File | Changes |
|------|---------|
| `memory.rs` | Add SlotEntry.size_class, SlotEntry.region_id |
| `memory.rs` | Add SizeClass, SIZE_CLASSES, size_class_for() |
| `memory.rs` | Add SizeClassFreeList struct |
| `memory.rs` | Extend Region → RegionAllocator with free_lists |
| `memory.rs` | Modify Region::allocate() to check free list |
| `memory.rs` | Add Region::deallocate() method |
| `memory.rs` | Update Region::reset/destroy for free lists |
| `ffi_exports.rs` | Update blood_region_alloc to use new allocator |
| `ffi_exports.rs` | Update blood_unregister_allocation |
| `ffi_exports.rs` | Add blood_region_dealloc |
| `ffi_exports.rs` | Add blood_region_get_stats |

### 9.2 Implementation Order

1. **Phase A: Data Structures** (no behavior change)
   - Add `size_class` and `region_id` to `SlotEntry`
   - Add `SizeClass` constants
   - Add `SizeClassFreeList` struct
   - Add `free_lists` field to `Region` (empty, unused)

2. **Phase B: Allocation Path** (additive)
   - Add `size_class_for()` function
   - Modify `register_allocation()` to set size_class
   - Modify `Region::allocate()` to check free list first
   - If free list empty, fall back to existing bump

3. **Phase C: Deallocation Path** (behavior change)
   - Add `Region::deallocate()` method
   - Modify `blood_unregister_allocation` to call it
   - Actually add freed addresses to free lists

4. **Phase D: Cleanup**
   - Update `Region::destroy()` to clear free lists
   - Update `Region::reset()` to clear free lists
   - Add statistics tracking
   - Add new FFI functions

### 9.3 Testing Checkpoints

After each phase:
```bash
cd blood-rust
cargo test -p blood-runtime
cargo build --release
cd ../blood
/home/jkindrix/blood-rust/target/release/blood check blood-std/std/compiler/main.blood
```

---

## 10. Migration Path

### 10.1 Backward Compatibility

The new allocator is **fully backward compatible**:

- `blood_region_alloc` signature unchanged
- `blood_region_create` signature unchanged
- `blood_region_destroy` signature unchanged
- `blood_unregister_allocation` now actually frees (but existing code works)

### 10.2 Codegen Changes (Optional)

To maximize benefit, update codegen to call `blood_region_dealloc` when emitting `StorageDead`:

```blood
// codegen_stmt.blood - emit_storage_dead

fn emit_storage_dead(self: &mut Self, local: LocalId) {
    if self.is_region_allocated(local) {
        let ptr_slot = self.local_slots.get(&local).unwrap();

        // Load the pointer value
        let ptr_val = self.emit_load(ptr_slot);
        let ptr_i64 = self.emit_ptrtoint(ptr_val);

        // Get region ID (currently hardcoded to 0)
        let region_id = self.emit_const_i64(0);

        // Call blood_region_dealloc instead of blood_unregister_allocation
        self.emit_call("@blood_region_dealloc", &[region_id, ptr_i64]);
    }
}
```

---

## 11. Testing Strategy

### 11.1 Unit Tests

```rust
#[test]
fn test_size_class_selection() {
    assert_eq!(size_class_for(0), 0);
    assert_eq!(size_class_for(1), 0);
    assert_eq!(size_class_for(8), 0);
    assert_eq!(size_class_for(9), 1);
    assert_eq!(size_class_for(16), 1);
    assert_eq!(size_class_for(17), 2);
    assert_eq!(size_class_for(16384), 11);
    assert_eq!(size_class_for(16385), SIZE_CLASS_LARGE);
}

#[test]
fn test_free_list_reuse() {
    let region_id = blood_region_create(1024 * 1024, 10 * 1024 * 1024);

    // Allocate
    let addr1 = blood_region_alloc(region_id, 64, 8);
    assert_ne!(addr1, 0);

    // Free
    blood_region_dealloc(region_id, addr1);

    // Allocate same size - should reuse
    let addr2 = blood_region_alloc(region_id, 64, 8);
    assert_eq!(addr1, addr2, "Should reuse freed slot");

    blood_region_destroy(region_id);
}

#[test]
fn test_generation_increments_on_free() {
    let region_id = blood_region_create(1024 * 1024, 10 * 1024 * 1024);

    let addr = blood_region_alloc(region_id, 64, 8);
    let gen1 = blood_get_generation(addr);

    blood_region_dealloc(region_id, addr);
    let gen2 = blood_get_generation(addr);

    assert_eq!(gen2, gen1 + 1, "Generation should increment on free");

    // Reuse
    let addr2 = blood_region_alloc(region_id, 64, 8);
    assert_eq!(addr, addr2);
    let gen3 = blood_get_generation(addr);

    assert_eq!(gen3, gen2, "Generation unchanged on reuse");

    blood_region_destroy(region_id);
}

#[test]
fn test_vec_growth_memory_reuse() {
    let region_id = blood_region_create(1024 * 1024, 10 * 1024 * 1024);

    // Simulate Vec growth pattern
    let sizes = [64, 128, 256, 512, 1024];
    let mut addrs = Vec::new();

    for size in &sizes {
        let addr = blood_region_alloc(region_id, *size, 8);
        addrs.push(addr);
    }

    // Free in reverse (like Vec dropping old buffers)
    for i in (0..addrs.len()-1).rev() {
        blood_region_dealloc(region_id, addrs[i]);
    }

    // Check stats - should show reuse potential
    let mut allocs = 0u64;
    let mut reused = 0u64;
    let mut bumped = 0u64;
    let mut deallocs = 0u64;

    blood_region_get_stats(region_id, &mut allocs, &mut reused, &mut bumped, &mut deallocs);

    assert_eq!(deallocs, 4, "Should have 4 deallocations");

    // Now allocate those sizes again - should reuse
    for size in &sizes[..4] {
        blood_region_alloc(region_id, *size, 8);
    }

    blood_region_get_stats(region_id, &mut allocs, &mut reused, &mut bumped, &mut deallocs);
    assert_eq!(reused, 4, "Should reuse all 4 freed slots");

    blood_region_destroy(region_id);
}
```

### 11.2 Integration Test

```bash
# Compile the self-hosted compiler with the new runtime
cd blood-rust
cargo build --release

# Test: Compile a large file
cd ../blood
/home/jkindrix/blood-rust/target/release/blood build \
    blood-std/std/compiler/main.blood \
    -o /tmp/bloodc_self

# Compare memory usage
/usr/bin/time -v /home/jkindrix/blood-rust/target/release/blood build \
    blood-std/std/compiler/main.blood \
    -o /tmp/bloodc_self 2>&1 | grep "Maximum resident"
```

### 11.3 Stress Test

```blood
// test_slab_stress.blood
fn main() {
    let region = region_create(1024 * 1024, 100 * 1024 * 1024);

    // Simulate heavy allocation/deallocation
    let mut i = 0;
    while i < 100000 {
        // Allocate
        let p1 = region_alloc(region, 64, 8);
        let p2 = region_alloc(region, 128, 8);
        let p3 = region_alloc(region, 256, 8);

        // Free
        region_dealloc(region, p1);
        region_dealloc(region, p2);

        // Reallocate - should reuse p1, p2
        let p4 = region_alloc(region, 64, 8);
        let p5 = region_alloc(region, 128, 8);

        // Free all
        region_dealloc(region, p3);
        region_dealloc(region, p4);
        region_dealloc(region, p5);

        i = i + 1;
    }

    // Print stats
    let stats = region_get_stats(region);
    println("Allocations: " + stats.allocations.to_string());
    println("Reused: " + stats.reused.to_string());
    println("Reuse rate: " + (stats.reused * 100 / stats.allocations).to_string() + "%");

    region_destroy(region);
}
```

---

## 12. Expected Results

### Memory Usage Improvement

| Metric | Before | After | Improvement |
|--------|--------|-------|-------------|
| Self-hosting memory | OOM | ~50-100 MB | ✅ Works |
| Per-let memory growth | 237 KB | ~5-10 KB | ~25x |
| Vec growth waste | 100% | ~0% | ✅ Reused |
| Reuse rate | 0% | >80% | Major |

### Performance Impact

| Operation | Before | After | Change |
|-----------|--------|-------|--------|
| Allocation | ~20 cycles | ~25 cycles | +25% |
| Deallocation | ~10 cycles | ~15 cycles | +50% |
| Overall compile | Baseline | +5-10% | Acceptable |

The slight performance overhead is acceptable because:
1. We avoid OOM (compilation completes)
2. Better cache locality from memory reuse
3. Less pressure on OS memory management

---

## Summary

This design provides a **complete solution** for Blood's memory management problems:

1. ✅ **Memory reuse** - freed slots go to size-class free lists
2. ✅ **Generation semantics** - every free increments generation (SSM compliant)
3. ✅ **Region bulk-free** - destroy still O(n) and releases all memory
4. ✅ **Backward compatible** - existing code works unchanged
5. ✅ **Minimal overhead** - simple free lists, no complex data structures

The implementation can be done incrementally with testing at each phase, and the result will unblock self-hosting of the Blood compiler.
