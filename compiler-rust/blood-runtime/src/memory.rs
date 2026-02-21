//! # Memory Management
//!
//! Memory allocation and management for the Blood runtime.
//!
//! ## Design
//!
//! Based on the Blood memory model (MEMORY_MODEL.md):
//! - Tier 0: Stack allocation (fastest)
//! - Tier 1: Region allocation (scoped)
//! - Tier 2: Persistent heap (longest-lived)
//! - Generational references for safety
//!
//! ## Slot Registry
//!
//! The slot registry tracks all allocations and their generations, enabling
//! validation of generational references on effect handler resume. This is
//! based on the [generational arena](https://crates.io/crates/generational-arena)
//! pattern for solving the ABA problem.
//!
//! ## Technical References
//!
//! - [MEMORY_MODEL.md](../../../MEMORY_MODEL.md)
//! - [Generational References](https://floooh.github.io/2018/06/17/handles-vs-pointers.html)
//! - [Vale's Generational References](https://verdagon.dev/blog/generational-references)

use std::alloc::{self, Layout};
use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::OnceLock;
use parking_lot::RwLock;

#[cfg(unix)]
use nix::libc;

/// Generation counter for detecting stale references.
pub type Generation = u32;

/// Reserved generation values.
pub mod generation {
    use super::Generation;

    /// Uninitialized slot.
    pub const UNINITIALIZED: Generation = 0;
    /// First valid generation.
    pub const FIRST: Generation = 1;
    /// Overflow guard value (triggers tier promotion).
    pub const OVERFLOW_GUARD: Generation = u32::MAX - 1;
    /// Persistent marker (never collected).
    pub const PERSISTENT: Generation = u32::MAX;
}

/// Error when accessing a stale reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaleReferenceError {
    /// Expected generation.
    pub expected: Generation,
    /// Actual generation.
    pub actual: Generation,
}

impl fmt::Display for StaleReferenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "stale reference: expected generation {}, found {}",
            self.expected, self.actual
        )
    }
}

impl std::error::Error for StaleReferenceError {}

// ============================================================================
// Slot Registry - Global address → generation mapping
// ============================================================================

/// Entry in the slot registry tracking allocation state.
#[derive(Debug, Clone, Copy)]
pub struct SlotEntry {
    /// Current generation of this slot.
    pub generation: Generation,
    /// Size of the allocation.
    pub size: usize,
    /// Whether the slot is currently allocated.
    pub is_allocated: bool,
    /// Size class index (0-11 for slab, 255 for large allocations).
    pub size_class: u8,
    /// Region ID this allocation belongs to (0 for non-region allocations).
    pub region_id: u64,
}

impl SlotEntry {
    /// Create a new allocated slot entry.
    pub fn new(generation: Generation, size: usize) -> Self {
        Self {
            generation,
            size,
            is_allocated: true,
            size_class: size_class_for(size),
            region_id: 0,
        }
    }

    /// Create a new allocated slot entry with region tracking.
    pub fn new_in_region(generation: Generation, size: usize, region_id: u64) -> Self {
        Self {
            generation,
            size,
            is_allocated: true,
            size_class: size_class_for(size),
            region_id,
        }
    }

    /// Mark the slot as deallocated and increment generation.
    ///
    /// Returns `true` if the slot was previously allocated (successful deallocation),
    /// `false` if already deallocated (double-free attempt).
    #[must_use]
    pub fn deallocate(&mut self) -> bool {
        if !self.is_allocated {
            return false; // Already deallocated
        }
        self.is_allocated = false;
        // Increment generation for next allocation (prevents ABA problem)
        if self.generation < generation::OVERFLOW_GUARD {
            self.generation += 1;
        }
        true
    }
}

// ============================================================================
// Size Classes for Slab Allocation
// ============================================================================

/// Size class for large allocations (>16KB).
pub const SIZE_CLASS_LARGE: u8 = 255;

/// Maximum size handled by slab allocator (16KB).
pub const MAX_SLAB_SIZE: usize = 16384;

/// Number of size classes.
pub const NUM_SIZE_CLASSES: usize = 12;

/// Size class slot sizes (power of 2 from 8 to 16384).
pub const SIZE_CLASS_SLOTS: [usize; NUM_SIZE_CLASSES] = [
    8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384,
];

/// Get the size class index for a given allocation size.
/// Returns SIZE_CLASS_LARGE (255) if size exceeds MAX_SLAB_SIZE.
#[inline]
pub fn size_class_for(size: usize) -> u8 {
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
        ((index - 3) as u8).min(11)
    }
}

/// Get the slot size for a size class.
///
/// Returns the slot size in bytes for the given size class index.
/// Returns 0 for large allocations (class 255) or invalid class indices.
#[inline]
#[must_use]
pub const fn slot_size_for_class(class: u8) -> usize {
    if class == SIZE_CLASS_LARGE || class as usize >= NUM_SIZE_CLASSES {
        0 // Large allocs don't use fixed slots
    } else {
        SIZE_CLASS_SLOTS[class as usize]
    }
}

/// Global slot registry for tracking allocations and their generations.
///
/// This registry enables validation of generational references by mapping
/// addresses to their current generation. When an effect handler captures
/// a continuation, it snapshots the generations of all captured references.
/// On resume, these generations are validated against the registry to detect
/// stale references that were freed and reallocated.
///
/// Based on the generational arena pattern from:
/// - [generational-arena](https://github.com/fitzgen/generational-arena)
/// - [slotmap](https://crates.io/crates/slotmap)
pub struct SlotRegistry {
    /// Map from allocation address to slot entry.
    slots: RwLock<HashMap<u64, SlotEntry>>,
    /// Atomic count of entries for lock-free diagnostics.
    /// This tracks the number of unique addresses ever registered.
    count: AtomicUsize,
}

impl SlotRegistry {
    /// Create a new empty slot registry.
    pub fn new() -> Self {
        Self {
            slots: RwLock::new(HashMap::new()),
            count: AtomicUsize::new(0),
        }
    }

    /// Register a new allocation.
    ///
    /// Returns the generation assigned to this allocation.
    pub fn register(&self, address: u64, size: usize) -> Generation {
        let mut slots = self.slots.write();

        // Check if this address was previously used
        if let Some(entry) = slots.get_mut(&address) {
            // Reuse slot with incremented generation
            entry.generation = if entry.generation < generation::OVERFLOW_GUARD {
                entry.generation + 1
            } else {
                generation::OVERFLOW_GUARD
            };
            entry.size = size;
            entry.is_allocated = true;
            entry.generation
        } else {
            // New slot
            let entry = SlotEntry::new(generation::FIRST, size);
            let gen = entry.generation;
            slots.insert(address, entry);
            self.count.fetch_add(1, Ordering::Relaxed);
            gen
        }
    }

    /// Register a new allocation with a specific region ID.
    ///
    /// This is used when allocating from a region to associate the allocation
    /// with that region for proper cleanup.
    pub fn register_with_region(&self, address: u64, size: usize, region_id: u64) -> Generation {
        let mut slots = self.slots.write();

        // Check if this address was previously used
        if let Some(entry) = slots.get_mut(&address) {
            // Reuse slot with incremented generation
            entry.generation = if entry.generation < generation::OVERFLOW_GUARD {
                entry.generation + 1
            } else {
                generation::OVERFLOW_GUARD
            };
            entry.size = size;
            entry.is_allocated = true;
            entry.size_class = size_class_for(size);
            entry.region_id = region_id;
            entry.generation
        } else {
            // New slot
            let entry = SlotEntry::new_in_region(generation::FIRST, size, region_id);
            let gen = entry.generation;
            slots.insert(address, entry);
            self.count.fetch_add(1, Ordering::Relaxed);
            gen
        }
    }

    /// Get information about an allocation.
    ///
    /// Returns `Some((size, region_id))` if the address is registered, `None` otherwise.
    pub fn get_info(&self, address: u64) -> Option<(usize, u64)> {
        let slots = self.slots.read();
        slots.get(&address).map(|e| (e.size, e.region_id))
    }

    /// Update the size of an existing allocation (for in-place realloc).
    ///
    /// Updates the stored size and size_class for an allocation that was
    /// extended in place. The address and region_id remain unchanged.
    pub fn update_size(&self, address: u64, new_size: usize) {
        let mut slots = self.slots.write();
        if let Some(entry) = slots.get_mut(&address) {
            entry.size = new_size;
            entry.size_class = size_class_for(new_size);
        }
    }

    /// Unregister an allocation (mark as freed).
    ///
    /// The slot entry is retained with an incremented generation to detect
    /// stale references that try to access it after deallocation.
    ///
    /// Returns `Some((region_id, size_class))` if the slot was found and deallocated,
    /// `None` if the address was not registered or was already deallocated.
    #[must_use = "contains region and size class info needed for free list routing"]
    pub fn unregister(&self, address: u64) -> Option<(u64, u8)> {
        let mut slots = self.slots.write();
        if let Some(entry) = slots.get_mut(&address) {
            let region_id = entry.region_id;
            let size_class = entry.size_class;
            if entry.deallocate() {
                return Some((region_id, size_class));
            }
        }
        None
    }

    /// Unregister without returning info (for backward compatibility).
    pub fn unregister_simple(&self, address: u64) {
        let _ = self.unregister(address);
    }

    /// Get the current generation for an address.
    ///
    /// Returns `None` if the address was never registered.
    /// Returns the generation (even if deallocated) if it was registered.
    pub fn get_generation(&self, address: u64) -> Option<Generation> {
        let slots = self.slots.read();
        slots.get(&address).map(|e| e.generation)
    }

    /// Check if an address is currently allocated.
    pub fn is_allocated(&self, address: u64) -> bool {
        let slots = self.slots.read();
        slots.get(&address).map(|e| e.is_allocated).unwrap_or(false)
    }

    /// Validate an address against an expected generation.
    ///
    /// Returns `Ok(())` if the generation matches and the slot is allocated.
    /// Returns `Err` with the actual generation if validation fails.
    pub fn validate(&self, address: u64, expected_gen: Generation) -> Result<(), StaleReferenceError> {
        // Skip persistent references (they never become stale)
        if expected_gen == generation::PERSISTENT {
            return Ok(());
        }

        let slots = self.slots.read();
        match slots.get(&address) {
            Some(entry) => {
                if entry.generation != expected_gen {
                    Err(StaleReferenceError {
                        expected: expected_gen,
                        actual: entry.generation,
                    })
                } else if !entry.is_allocated {
                    // Slot was freed (generation incremented on free)
                    Err(StaleReferenceError {
                        expected: expected_gen,
                        actual: entry.generation,
                    })
                } else {
                    Ok(())
                }
            }
            None => {
                // Address was never registered - could be:
                // 1. Stack or static memory (not tracked in registry)
                // 2. Heap memory that was freed and cleaned up from registry
                //
                // If the expected generation is FIRST, assume it's stack/static
                // memory which is always valid. Otherwise, it was likely heap
                // memory that got freed.
                if expected_gen == generation::FIRST {
                    Ok(()) // Likely stack/static memory
                } else {
                    Err(StaleReferenceError {
                        expected: expected_gen,
                        actual: generation::UNINITIALIZED,
                    })
                }
            }
        }
    }

    /// Increment the generation for the given address.
    ///
    /// This invalidates all existing references to the slot. If the address
    /// is not registered, this is a no-op.
    pub fn increment_generation(&self, address: u64) {
        let mut slots = self.slots.write();
        if let Some(entry) = slots.get_mut(&address) {
            if entry.generation < generation::OVERFLOW_GUARD {
                entry.generation += 1;
            }
        }
    }

    /// Get the number of tracked slots (lock-free).
    ///
    /// This returns the count of unique addresses ever registered, using an
    /// atomic counter for lock-free access. This is suitable for diagnostics
    /// and monitoring without risking contention with allocation operations.
    pub fn len(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }

    /// Get the exact number of entries in the HashMap.
    ///
    /// **Note**: This acquires a read lock on the internal map. For most
    /// diagnostic purposes, prefer `len()` which is lock-free.
    pub fn len_exact(&self) -> usize {
        self.slots.read().len()
    }

    /// Check if the registry is empty (lock-free).
    pub fn is_empty(&self) -> bool {
        self.count.load(Ordering::Relaxed) == 0
    }

    /// Clear all slots (for testing).
    #[cfg(test)]
    pub fn clear(&self) {
        self.slots.write().clear();
        self.count.store(0, Ordering::Relaxed);
    }

    /// Get the count of unique regions that have allocations.
    ///
    /// Returns the number of distinct non-zero region IDs in the registry.
    pub fn region_count(&self) -> usize {
        let slots = self.slots.read();
        let mut regions = std::collections::HashSet::new();
        for entry in slots.values() {
            if entry.region_id != 0 && entry.is_allocated {
                regions.insert(entry.region_id);
            }
        }
        regions.len()
    }
}

impl Default for SlotRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for SlotRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let slots = self.slots.read();
        f.debug_struct("SlotRegistry")
            .field("num_slots", &slots.len())
            .finish()
    }
}

// Thread-safe: uses RwLock internally
unsafe impl Send for SlotRegistry {}
unsafe impl Sync for SlotRegistry {}

/// Global slot registry instance.
static SLOT_REGISTRY: OnceLock<SlotRegistry> = OnceLock::new();

/// Get the global slot registry.
pub fn slot_registry() -> &'static SlotRegistry {
    SLOT_REGISTRY.get_or_init(SlotRegistry::new)
}

// ============================================================================
// System Allocator Statistics (for non-region allocations)
// ============================================================================

/// Total bytes allocated via system allocator (non-region allocations).
static SYSTEM_ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);

/// Total bytes freed via system allocator (non-region allocations).
static SYSTEM_FREE_BYTES: AtomicU64 = AtomicU64::new(0);

/// Total number of system allocations (non-region).
static SYSTEM_ALLOC_COUNT: AtomicU64 = AtomicU64::new(0);

/// Total number of system frees (non-region).
static SYSTEM_FREE_COUNT: AtomicU64 = AtomicU64::new(0);

/// Record a system allocation (non-region).
#[inline]
pub fn record_system_alloc(size: usize) {
    SYSTEM_ALLOC_BYTES.fetch_add(size as u64, Ordering::Relaxed);
    SYSTEM_ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
}

/// Record a system free (non-region).
#[inline]
pub fn record_system_free(size: usize) {
    SYSTEM_FREE_BYTES.fetch_add(size as u64, Ordering::Relaxed);
    SYSTEM_FREE_COUNT.fetch_add(1, Ordering::Relaxed);
}

/// Get system allocator statistics.
///
/// Returns (allocated_bytes, freed_bytes, alloc_count, free_count).
pub fn system_alloc_stats() -> (u64, u64, u64, u64) {
    (
        SYSTEM_ALLOC_BYTES.load(Ordering::Relaxed),
        SYSTEM_FREE_BYTES.load(Ordering::Relaxed),
        SYSTEM_ALLOC_COUNT.load(Ordering::Relaxed),
        SYSTEM_FREE_COUNT.load(Ordering::Relaxed),
    )
}

/// Get current live system allocator bytes (allocated - freed).
pub fn system_alloc_live_bytes() -> u64 {
    let allocated = SYSTEM_ALLOC_BYTES.load(Ordering::Relaxed);
    let freed = SYSTEM_FREE_BYTES.load(Ordering::Relaxed);
    allocated.saturating_sub(freed)
}

// ============================================================================
// Memory Limits
// ============================================================================

/// Maximum heap size in bytes (0 = unlimited).
static MAX_HEAP_SIZE: AtomicU64 = AtomicU64::new(0);

/// Set the maximum heap size.
///
/// Allocations that would exceed this limit will fail.
/// Set to 0 for unlimited (default).
pub fn set_max_heap_size(max_bytes: u64) {
    MAX_HEAP_SIZE.store(max_bytes, Ordering::SeqCst);
}

/// Get the current maximum heap size.
///
/// Returns 0 if unlimited.
pub fn max_heap_size() -> u64 {
    MAX_HEAP_SIZE.load(Ordering::SeqCst)
}

/// Check if an allocation of the given size would exceed the memory limit.
///
/// Returns `true` if the allocation is allowed, `false` if it would exceed the limit.
pub fn check_allocation_allowed(size: usize) -> bool {
    let max = MAX_HEAP_SIZE.load(Ordering::SeqCst);
    if max == 0 {
        return true; // Unlimited
    }

    let current = system_alloc_live_bytes();
    current.saturating_add(size as u64) <= max
}

/// Try to record an allocation, checking against memory limits.
///
/// Returns `true` if the allocation was recorded successfully,
/// `false` if it would exceed the memory limit.
///
/// This is an atomic operation - if it returns `true`, the allocation
/// has been recorded. If it returns `false`, nothing has changed.
pub fn try_record_system_alloc(size: usize) -> bool {
    let max = MAX_HEAP_SIZE.load(Ordering::SeqCst);

    if max == 0 {
        // Unlimited - just record it
        record_system_alloc(size);
        return true;
    }

    // Use compare-and-swap to atomically check and update
    loop {
        let current_allocated = SYSTEM_ALLOC_BYTES.load(Ordering::Relaxed);
        let current_freed = SYSTEM_FREE_BYTES.load(Ordering::Relaxed);
        let current_live = current_allocated.saturating_sub(current_freed);

        let new_live = current_live.saturating_add(size as u64);
        if new_live > max {
            return false; // Would exceed limit
        }

        // Try to atomically increment the allocation counter
        match SYSTEM_ALLOC_BYTES.compare_exchange_weak(
            current_allocated,
            current_allocated + size as u64,
            Ordering::SeqCst,
            Ordering::Relaxed,
        ) {
            Ok(_) => {
                SYSTEM_ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
                return true;
            }
            Err(_) => {
                // Another thread modified it, retry
                continue;
            }
        }
    }
}

/// Error returned when a memory allocation fails.
#[derive(Debug, Clone, Copy)]
pub enum AllocationError {
    /// Allocation would exceed the configured memory limit.
    LimitExceeded {
        /// Requested allocation size.
        requested: usize,
        /// Current memory usage.
        current: u64,
        /// Configured memory limit.
        limit: u64,
    },
    /// Allocation failed due to system error.
    OutOfMemory,
}

impl std::fmt::Display for AllocationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AllocationError::LimitExceeded { requested, current, limit } => {
                write!(
                    f,
                    "allocation of {} bytes would exceed memory limit ({} / {} bytes used)",
                    requested, current, limit
                )
            }
            AllocationError::OutOfMemory => {
                write!(f, "out of memory")
            }
        }
    }
}

impl std::error::Error for AllocationError {}

/// Get memory usage summary.
pub fn memory_usage_summary() -> MemoryUsageSummary {
    let (allocated, freed, alloc_count, free_count) = system_alloc_stats();
    let live = allocated.saturating_sub(freed);
    let limit = max_heap_size();

    MemoryUsageSummary {
        allocated_bytes: allocated,
        freed_bytes: freed,
        live_bytes: live,
        allocation_count: alloc_count,
        free_count,
        limit_bytes: if limit == 0 { None } else { Some(limit) },
        usage_percent: if limit == 0 {
            0.0
        } else {
            (live as f64 / limit as f64) * 100.0
        },
    }
}

/// Summary of memory usage.
#[derive(Debug, Clone)]
pub struct MemoryUsageSummary {
    /// Total bytes allocated.
    pub allocated_bytes: u64,
    /// Total bytes freed.
    pub freed_bytes: u64,
    /// Currently live bytes (allocated - freed).
    pub live_bytes: u64,
    /// Total number of allocations.
    pub allocation_count: u64,
    /// Total number of frees.
    pub free_count: u64,
    /// Memory limit in bytes, if configured.
    pub limit_bytes: Option<u64>,
    /// Usage as a percentage of limit (0 if unlimited).
    pub usage_percent: f64,
}

/// Allocation statistics for observability.
#[derive(Debug, Clone)]
pub struct AllocationStats {
    /// Total bytes allocated (including freed).
    pub total_allocated: usize,
    /// Total bytes freed.
    pub total_freed: usize,
    /// Currently live bytes.
    pub current_live: usize,
    /// Number of allocations.
    pub allocation_count: usize,
    /// Number of deallocations.
    pub deallocation_count: usize,
}

/// Get total currently allocated bytes.
pub fn total_allocated() -> usize {
    let (allocated, freed, _, _) = system_alloc_stats();
    allocated.saturating_sub(freed) as usize
}

/// Get number of active regions.
///
/// Note: This currently returns the count of registered slots that have
/// non-zero region IDs, as regions are not globally tracked.
pub fn active_region_count() -> usize {
    slot_registry().region_count()
}

/// Get allocation statistics.
pub fn allocation_stats() -> AllocationStats {
    let (allocated, freed, alloc_count, free_count) = system_alloc_stats();
    AllocationStats {
        total_allocated: allocated as usize,
        total_freed: freed as usize,
        current_live: allocated.saturating_sub(freed) as usize,
        allocation_count: alloc_count as usize,
        deallocation_count: free_count as usize,
    }
}

/// Get the number of active persistent slots.
pub fn persistent_slot_count() -> usize {
    persistent_allocator().slot_count()
}

/// Register an allocation in the global slot registry.
///
/// This should be called by the allocator when a new allocation is made.
/// Returns the generation assigned to this allocation.
pub fn register_allocation(address: u64, size: usize) -> Generation {
    slot_registry().register(address, size)
}

/// Register an allocation with a specific region ID.
///
/// This is used by region-aware allocators to associate allocations with regions.
pub fn register_allocation_with_region(address: u64, size: usize, region_id: u64) -> Generation {
    slot_registry().register_with_region(address, size, region_id)
}

/// Get the size and region ID of an allocation from the global registry.
///
/// Returns `Some((size, region_id))` if the address is registered, `None` otherwise.
pub fn get_slot_info(address: u64) -> Option<(usize, u64)> {
    slot_registry().get_info(address)
}

/// Update the size of an existing allocation in the global registry.
///
/// Used by realloc-in-place to record the new size without changing the address.
pub fn update_slot_size(address: u64, new_size: usize) {
    slot_registry().update_size(address, new_size);
}

/// Unregister an allocation from the global slot registry.
///
/// This should be called by the allocator when memory is freed.
/// Returns `Some((region_id, size_class))` if found, `None` otherwise.
#[must_use = "contains region and size class info needed for free list routing"]
pub fn unregister_allocation(address: u64) -> Option<(u64, u8)> {
    slot_registry().unregister(address)
}

/// Unregister an allocation without returning info (simple version).
pub fn unregister_allocation_simple(address: u64) {
    let _ = slot_registry().unregister(address);
}

/// Get the current generation for an address from the global registry.
pub fn get_slot_generation(address: u64) -> Option<Generation> {
    slot_registry().get_generation(address)
}

/// Validate an address against an expected generation using the global registry.
pub fn validate_generation(address: u64, expected_gen: Generation) -> Result<(), StaleReferenceError> {
    slot_registry().validate(address, expected_gen)
}

/// Increment the generation for an address in the global slot registry.
///
/// This invalidates all existing references to the slot.
pub fn increment_generation(address: u64) {
    slot_registry().increment_generation(address)
}

// ============================================================================
// Pointer Metadata
// ============================================================================

/// Pointer metadata flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointerMetadata(u32);

impl PointerMetadata {
    /// No special flags.
    pub const NONE: Self = Self(0);
    /// Pointer is to stack memory.
    pub const STACK: Self = Self(1 << 0);
    /// Pointer is to region memory.
    pub const REGION: Self = Self(1 << 1);
    /// Pointer is to heap memory.
    pub const HEAP: Self = Self(1 << 2);
    /// Pointer is linear (must be used exactly once).
    pub const LINEAR: Self = Self(1 << 3);
    /// Pointer is weak (may become null).
    pub const WEAK: Self = Self(1 << 4);

    /// Create from raw bits.
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    /// Get raw bits.
    pub const fn bits(&self) -> u32 {
        self.0
    }

    /// Union of two metadata values.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Check if a flag is set.
    pub const fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

/// A 128-bit generational pointer (BloodPtr).
///
/// Layout:
/// - Bits 0-63: Address
/// - Bits 64-95: Generation (32 bits)
/// - Bits 96-127: Metadata (32 bits)
#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub struct BloodPtr {
    /// Raw address.
    address: usize,
    /// Generation counter.
    generation: Generation,
    /// Pointer metadata.
    metadata: PointerMetadata,
}

impl BloodPtr {
    /// Create a null pointer.
    pub const fn null() -> Self {
        Self {
            address: 0,
            generation: generation::UNINITIALIZED,
            metadata: PointerMetadata::NONE,
        }
    }

    /// Create a new pointer.
    pub const fn new(address: usize, generation: Generation, metadata: PointerMetadata) -> Self {
        Self {
            address,
            generation,
            metadata,
        }
    }

    /// Check if the pointer is null.
    pub const fn is_null(&self) -> bool {
        self.address == 0
    }

    /// Get the raw address.
    pub const fn address(&self) -> usize {
        self.address
    }

    /// Get the generation.
    pub const fn generation(&self) -> Generation {
        self.generation
    }

    /// Get the metadata.
    pub const fn metadata(&self) -> PointerMetadata {
        self.metadata
    }

    /// Check if this is a stack pointer.
    pub const fn is_stack(&self) -> bool {
        self.metadata.contains(PointerMetadata::STACK)
    }

    /// Check if this is a region pointer.
    pub const fn is_region(&self) -> bool {
        self.metadata.contains(PointerMetadata::REGION)
    }

    /// Check if this is a heap pointer.
    pub const fn is_heap(&self) -> bool {
        self.metadata.contains(PointerMetadata::HEAP)
    }

    /// Check if this is a linear pointer.
    pub const fn is_linear(&self) -> bool {
        self.metadata.contains(PointerMetadata::LINEAR)
    }

    /// Check if this is a weak pointer.
    pub const fn is_weak(&self) -> bool {
        self.metadata.contains(PointerMetadata::WEAK)
    }
}

impl fmt::Debug for BloodPtr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BloodPtr")
            .field("address", &format_args!("{:#x}", self.address))
            .field("generation", &self.generation)
            .field("metadata", &self.metadata)
            .finish()
    }
}

impl PartialEq for BloodPtr {
    fn eq(&self, other: &Self) -> bool {
        self.address == other.address && self.generation == other.generation
    }
}

impl Eq for BloodPtr {}

/// A memory slot with generation tracking.
pub struct Slot {
    /// Current generation (incremented on each reuse).
    generation: AtomicU32,
    /// Whether the slot is occupied.
    occupied: AtomicU32,
    /// Pointer to the actual data.
    data: UnsafeCell<*mut u8>,
    /// Size of the allocated data.
    size: AtomicUsize,
}

impl Slot {
    /// Create a new empty slot.
    pub fn new() -> Self {
        Self {
            generation: AtomicU32::new(generation::FIRST),
            occupied: AtomicU32::new(0),
            data: UnsafeCell::new(std::ptr::null_mut()),
            size: AtomicUsize::new(0),
        }
    }

    /// Get the current generation.
    pub fn generation(&self) -> Generation {
        self.generation.load(Ordering::Acquire)
    }

    /// Check if the slot is occupied.
    pub fn is_occupied(&self) -> bool {
        self.occupied.load(Ordering::Acquire) != 0
    }

    /// Allocate data in this slot.
    ///
    /// # Safety
    ///
    /// The slot must not be currently occupied.
    pub unsafe fn allocate(&self, layout: Layout) -> *mut u8 {
        debug_assert!(!self.is_occupied());

        let ptr = alloc::alloc(layout);
        if ptr.is_null() {
            return ptr;
        }

        *self.data.get() = ptr;
        self.size.store(layout.size(), Ordering::Release);
        self.occupied.store(1, Ordering::Release);

        ptr
    }

    /// Deallocate the data in this slot.
    ///
    /// # Safety
    ///
    /// The slot must be currently occupied.
    pub unsafe fn deallocate(&self) {
        debug_assert!(self.is_occupied());

        let ptr = *self.data.get();
        let size = self.size.load(Ordering::Acquire);

        if !ptr.is_null() && size > 0 {
            let layout = Layout::from_size_align_unchecked(size, 1);
            alloc::dealloc(ptr, layout);
        }

        *self.data.get() = std::ptr::null_mut();
        self.size.store(0, Ordering::Release);
        self.occupied.store(0, Ordering::Release);

        // Increment generation for next use
        let mut gen = self.generation.load(Ordering::Acquire);
        if gen >= generation::OVERFLOW_GUARD {
            // Would overflow, mark as needing tier promotion
            gen = generation::OVERFLOW_GUARD;
        } else {
            gen += 1;
        }
        self.generation.store(gen, Ordering::Release);
    }

    /// Validate a generation matches.
    pub fn validate(&self, expected_gen: Generation) -> Result<(), StaleReferenceError> {
        let actual = self.generation.load(Ordering::Acquire);
        if actual != expected_gen {
            Err(StaleReferenceError {
                expected: expected_gen,
                actual,
            })
        } else {
            Ok(())
        }
    }
}

impl Default for Slot {
    fn default() -> Self {
        Self::new()
    }
}

// Safety: Slot uses atomic operations for thread safety.
unsafe impl Send for Slot {}
unsafe impl Sync for Slot {}

/// Memory tier for allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryTier {
    /// Stack allocation (Tier 0).
    Stack,
    /// Region allocation (Tier 1).
    Region,
    /// Heap allocation (Tier 2) - basic heap without RC.
    Heap,
    /// Persistent allocation (Tier 3) - reference counted with cycle collection.
    Persistent,
}

/// Region identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RegionId(pub u64);

impl RegionId {
    /// Get the raw ID value.
    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

/// Global region ID counter.
static NEXT_REGION_ID: AtomicU64 = AtomicU64::new(1);

/// Generate a new unique region ID.
pub fn next_region_id() -> RegionId {
    RegionId(NEXT_REGION_ID.fetch_add(1, Ordering::Relaxed))
}

/// Region status for suspension tracking.
///
/// Used with atomic operations to track region lifecycle during effect suspension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum RegionStatus {
    /// Region is active and can be allocated from.
    Active = 0,
    /// Region is suspended (has active continuations).
    Suspended = 1,
    /// Region's lexical scope ended but deallocation is deferred.
    PendingDeallocation = 2,
}

impl RegionStatus {
    /// Convert from u32 value.
    pub fn from_u32(v: u32) -> Self {
        match v {
            0 => RegionStatus::Active,
            1 => RegionStatus::Suspended,
            2 => RegionStatus::PendingDeallocation,
            _ => panic!("invalid RegionStatus value {}: expected 0 (Active), 1 (Suspended), or 2 (PendingDeallocation)", v),
        }
    }
}

// ============================================================================
// Slab Allocator Free Lists
// ============================================================================

/// Initial free list capacities by size class.
///
/// Smaller allocations (classes 0-3: 8-64 bytes) are more common,
/// so they get larger initial capacities. Larger allocations are
/// less frequent and get smaller capacities.
const FREE_LIST_CAPACITIES: [usize; NUM_SIZE_CLASSES] = [
    256,  // Class 0: 8 bytes - very common (small objects, pointers)
    256,  // Class 1: 16 bytes - very common
    128,  // Class 2: 32 bytes - common
    128,  // Class 3: 64 bytes - common
    64,   // Class 4: 128 bytes - moderate
    64,   // Class 5: 256 bytes - moderate
    32,   // Class 6: 512 bytes - less common
    32,   // Class 7: 1KB - less common
    16,   // Class 8: 2KB - infrequent
    16,   // Class 9: 4KB - infrequent
    8,    // Class 10: 8KB - rare
    8,    // Class 11: 16KB - rare
];

/// Per-size-class free list for memory reuse within a region.
///
/// When memory is deallocated, the address is added to the appropriate
/// size class free list. On subsequent allocation, the free list is
/// checked first before bump-allocating new memory.
#[derive(Debug)]
pub struct SizeClassFreeList {
    /// Freed slot addresses available for reuse.
    slots: Vec<u64>,
    /// Statistics: total reuses from this list.
    reuse_count: u64,
}

impl SizeClassFreeList {
    /// Create a new empty free list with default capacity.
    pub fn new() -> Self {
        Self {
            slots: Vec::with_capacity(64), // Default capacity
            reuse_count: 0,
        }
    }

    /// Create a new empty free list with capacity appropriate for the size class.
    ///
    /// Smaller size classes get larger capacities since they're more commonly used.
    pub fn new_for_class(class: usize) -> Self {
        let capacity = if class < NUM_SIZE_CLASSES {
            FREE_LIST_CAPACITIES[class]
        } else {
            0 // Large allocations don't use free lists
        };
        Self {
            slots: Vec::with_capacity(capacity),
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

    /// Clear all entries (called on region destroy/reset).
    pub fn clear(&mut self) {
        self.slots.clear();
    }

    /// Number of available slots.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Total reuses from this list.
    pub fn reuse_count(&self) -> u64 {
        self.reuse_count
    }
}

impl Default for SizeClassFreeList {
    fn default() -> Self {
        Self::new()
    }
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
    /// Large allocations (>16KB).
    pub large_allocs: AtomicU64,
}

impl RegionStats {
    /// Create new zeroed stats.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get current allocation count.
    pub fn allocations(&self) -> u64 {
        self.allocations.load(Ordering::Relaxed)
    }

    /// Get reuse count.
    pub fn reused(&self) -> u64 {
        self.reused.load(Ordering::Relaxed)
    }

    /// Get bump allocation count.
    pub fn bumped(&self) -> u64 {
        self.bumped.load(Ordering::Relaxed)
    }

    /// Get deallocation count.
    pub fn deallocations(&self) -> u64 {
        self.deallocations.load(Ordering::Relaxed)
    }

    /// Calculate reuse percentage.
    pub fn reuse_percentage(&self) -> f64 {
        let allocs = self.allocations();
        if allocs == 0 {
            0.0
        } else {
            (self.reused() as f64 / allocs as f64) * 100.0
        }
    }
}

/// A memory region for scoped allocation with slab-based memory reuse.
///
/// On Unix, uses `mmap` to reserve a contiguous virtual address range upfront
/// (`PROT_NONE`), then commits pages on demand via `mprotect`. This ensures the
/// base address never moves, so pointers into the region remain valid even as
/// the region grows.
///
/// On non-Unix platforms, falls back to `Vec<u8>` with a documented pointer
/// invalidation risk when the region grows beyond its current capacity.
///
/// ## Slab Allocation
///
/// This region supports memory reuse through size-class free lists:
/// - When memory is deallocated, it's added to the free list for its size class
/// - On allocation, the free list is checked first before bump-allocating
/// - This enables memory reuse within a region, preventing accumulation
///
/// ## Platform Differences
///
/// - **Unix**: `allocate()` takes `&self` because the mmap-backed memory has a
///   stable base address. Page commits use `mprotect` without reallocating.
/// - **Non-Unix**: `allocate()` takes `&mut self` because `Vec::resize()` may
///   reallocate the underlying buffer when growing. For thread-safe usage on
///   non-Unix, wrap the `Region` in appropriate synchronization (e.g., `Mutex`).
pub struct Region {
    /// Region ID.
    id: RegionId,
    /// Base pointer — stable for the region's lifetime on Unix (mmap-backed).
    #[cfg(unix)]
    base: *mut u8,
    /// Total reserved address space (bytes).
    #[cfg(unix)]
    reserved: usize,
    /// Currently committed (usable) bytes.
    #[cfg(unix)]
    committed: AtomicUsize,

    /// Fallback: Vec-backed buffer for non-Unix platforms.
    /// WARNING: `Vec::resize` may reallocate, invalidating pointers.
    #[cfg(not(unix))]
    buffer: Vec<u8>,

    /// Current allocation offset (bump pointer).
    offset: AtomicUsize,
    /// Maximum size.
    max_size: usize,
    /// Whether the region is closed for new allocations.
    closed: AtomicU32,
    /// Suspension count: number of continuations holding references into this region.
    ///
    /// When an effect suspends inside a region scope, this count is incremented.
    /// When the continuation resumes or is dropped, this count is decremented.
    /// Deallocation is deferred while suspend_count > 0.
    suspend_count: AtomicU32,
    /// Current status of the region.
    ///
    /// Tracks whether the region is active, suspended, or pending deallocation.
    status: AtomicU32,

    // ========================================================================
    // Slab Allocator Fields
    // ========================================================================

    /// Per-size-class free lists for memory reuse.
    /// Protected by mutex for thread safety during alloc/dealloc.
    free_lists: parking_lot::Mutex<[SizeClassFreeList; NUM_SIZE_CLASSES]>,

    /// Allocation statistics.
    stats: RegionStats,
}

/// Round `value` up to the nearest multiple of `align` (must be a power of 2).
fn round_up(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}

#[cfg(unix)]
fn page_size() -> usize {
    // SAFETY: sysconf(_SC_PAGESIZE) is always safe and returns a positive value.
    unsafe { libc::sysconf(libc::_SC_PAGESIZE) as usize }
}

impl Region {
    /// Create a new region with the given initial and maximum sizes.
    ///
    /// On Unix, reserves `max_size` bytes of virtual address space and commits
    /// `initial_size` bytes. On other platforms, allocates `initial_size` via `Vec`.
    #[cfg(unix)]
    pub fn new(initial_size: usize, max_size: usize) -> Self {
        use std::ptr;

        let ps = page_size();
        let reserved = round_up(max_size.max(ps), ps);
        let initial_commit = round_up(initial_size.max(1), ps);

        // Reserve address space with PROT_NONE (no access, no physical pages).
        let base = unsafe {
            libc::mmap(
                ptr::null_mut(),
                reserved,
                libc::PROT_NONE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if base == libc::MAP_FAILED {
            panic!(
                "Region::new: mmap reserve failed for {} bytes: {}",
                reserved,
                std::io::Error::last_os_error()
            );
        }

        // Commit initial pages (make readable/writable).
        let commit_len = initial_commit.min(reserved);
        if commit_len > 0 {
            let ret = unsafe {
                libc::mprotect(base, commit_len, libc::PROT_READ | libc::PROT_WRITE)
            };
            if ret != 0 {
                // Clean up reservation before panicking.
                unsafe { libc::munmap(base, reserved); }
                panic!(
                    "Region::new: mprotect commit failed for {} bytes: {}",
                    commit_len,
                    std::io::Error::last_os_error()
                );
            }
        }

        Self {
            id: next_region_id(),
            base: base as *mut u8,
            reserved,
            committed: AtomicUsize::new(commit_len),
            offset: AtomicUsize::new(0),
            max_size,
            closed: AtomicU32::new(0),
            suspend_count: AtomicU32::new(0),
            status: AtomicU32::new(RegionStatus::Active as u32),
            free_lists: parking_lot::Mutex::new(std::array::from_fn(SizeClassFreeList::new_for_class)),
            stats: RegionStats::new(),
        }
    }

    /// Create a new region (non-Unix fallback).
    #[cfg(not(unix))]
    pub fn new(initial_size: usize, max_size: usize) -> Self {
        Self {
            id: next_region_id(),
            buffer: vec![0u8; initial_size],
            offset: AtomicUsize::new(0),
            max_size,
            closed: AtomicU32::new(0),
            suspend_count: AtomicU32::new(0),
            status: AtomicU32::new(RegionStatus::Active as u32),
            free_lists: parking_lot::Mutex::new(std::array::from_fn(SizeClassFreeList::new_for_class)),
            stats: RegionStats::new(),
        }
    }

    /// Get the region ID.
    pub fn id(&self) -> RegionId {
        self.id
    }

    /// Get the base address of the region's backing memory.
    #[cfg(unix)]
    pub fn base_addr(&self) -> usize {
        self.base as usize
    }

    #[cfg(not(unix))]
    pub fn base_addr(&self) -> usize {
        self.buffer.as_ptr() as usize
    }

    /// Get the current used size.
    pub fn used(&self) -> usize {
        self.offset.load(Ordering::Acquire)
    }

    /// Get the current capacity (committed bytes).
    #[cfg(unix)]
    pub fn capacity(&self) -> usize {
        self.committed.load(Ordering::Acquire)
    }

    /// Get the current capacity (Vec length).
    #[cfg(not(unix))]
    pub fn capacity(&self) -> usize {
        self.buffer.len()
    }

    /// Get the maximum size.
    pub fn max_size(&self) -> usize {
        self.max_size
    }

    /// Check if the region is closed.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire) != 0
    }

    /// Close the region for new allocations.
    pub fn close(&self) {
        self.closed.store(1, Ordering::Release);
    }

    /// Allocate memory from the region with slab-based reuse.
    ///
    /// Strategy:
    /// 1. Determine size class for the allocation
    /// 2. Check free list for that size class
    /// 3. If found: reuse the freed slot
    /// 4. If not found: bump allocate from the region
    ///
    /// On Unix, uses mmap-backed memory with on-demand page commits.
    #[cfg(unix)]
    pub fn allocate(&self, size: usize, align: usize) -> Option<*mut u8> {
        if self.is_closed() {
            return None;
        }

        let class = size_class_for(size);

        // For slab-sized allocations, try the free list first
        if class != SIZE_CLASS_LARGE {
            let slot_size = slot_size_for_class(class);
            let mut lists = self.free_lists.lock();

            if let Some(addr) = lists[class as usize].pop() {
                // Reusing freed slot - just return the address
                // Generation was already incremented when slot was freed
                self.stats.reused.fetch_add(1, Ordering::Relaxed);
                self.stats.allocations.fetch_add(1, Ordering::Relaxed);
                return Some(addr as *mut u8);
            }
            drop(lists);

            // Free list empty - bump allocate with slot size for alignment
            return self.bump_allocate_unix(slot_size.max(size), align);
        }

        // Large allocation - always bump allocate
        self.stats.large_allocs.fetch_add(1, Ordering::Relaxed);
        self.bump_allocate_unix(size, align)
    }

    /// Internal bump allocation for Unix.
    #[cfg(unix)]
    fn bump_allocate_unix(&self, size: usize, align: usize) -> Option<*mut u8> {
        let offset = self.offset.load(Ordering::Acquire);
        let aligned_offset = round_up(offset, align);
        let new_offset = aligned_offset + size;

        if new_offset > self.reserved {
            // Exceeds the total reserved address space.
            return None;
        }

        let committed = self.committed.load(Ordering::Acquire);
        if new_offset > committed {
            // Need to commit more pages.
            let ps = page_size();
            // Grow by min(committed, 256 MB) to avoid over-committing.
            // The old strategy (committed * 2) wastes up to 50% of committed memory
            // per growth step. Capping at 256 MB means a 16 GB region grows to
            // 16.25 GB instead of 32 GB.
            const MAX_GROWTH: usize = 256 * 1024 * 1024; // 256 MB
            let growth = committed.min(MAX_GROWTH);
            let desired = round_up((committed + growth).max(new_offset), ps).min(self.reserved);
            let additional = desired - committed;
            if additional > 0 {
                let ret = unsafe {
                    libc::mprotect(
                        self.base.add(committed) as *mut libc::c_void,
                        additional,
                        libc::PROT_READ | libc::PROT_WRITE,
                    )
                };
                if ret != 0 {
                    return None;
                }
                self.committed.store(desired, Ordering::Release);
            }
        }

        self.offset.store(new_offset, Ordering::Release);
        self.stats.bumped.fetch_add(1, Ordering::Relaxed);
        self.stats.allocations.fetch_add(1, Ordering::Relaxed);
        Some(unsafe { self.base.add(aligned_offset) })
    }

    /// Allocate memory from the region with slab-based reuse (non-Unix fallback).
    ///
    /// WARNING: `Vec::resize` may reallocate, invalidating previously returned
    /// pointers. This is a known limitation on non-Unix platforms.
    #[cfg(not(unix))]
    pub fn allocate(&mut self, size: usize, align: usize) -> Option<*mut u8> {
        if self.is_closed() {
            return None;
        }

        let class = size_class_for(size);

        // For slab-sized allocations, try the free list first
        if class != SIZE_CLASS_LARGE {
            let slot_size = slot_size_for_class(class);
            let mut lists = self.free_lists.lock();

            if let Some(addr) = lists[class as usize].pop() {
                self.stats.reused.fetch_add(1, Ordering::Relaxed);
                self.stats.allocations.fetch_add(1, Ordering::Relaxed);
                return Some(addr as *mut u8);
            }
            drop(lists);

            return self.bump_allocate_non_unix(slot_size.max(size), align);
        }

        self.stats.large_allocs.fetch_add(1, Ordering::Relaxed);
        self.bump_allocate_non_unix(size, align)
    }

    /// Internal bump allocation for non-Unix.
    #[cfg(not(unix))]
    fn bump_allocate_non_unix(&mut self, size: usize, align: usize) -> Option<*mut u8> {
        let offset = self.offset.load(Ordering::Acquire);
        let aligned_offset = round_up(offset, align);
        let new_offset = aligned_offset + size;

        if new_offset > self.buffer.len() {
            if new_offset > self.max_size {
                return None;
            }
            let new_capacity = (new_offset * 2).min(self.max_size);
            self.buffer.resize(new_capacity, 0);
        }

        self.offset.store(new_offset, Ordering::Release);
        self.stats.bumped.fetch_add(1, Ordering::Relaxed);
        self.stats.allocations.fetch_add(1, Ordering::Relaxed);
        Some(unsafe { self.buffer.as_mut_ptr().add(aligned_offset) })
    }

    /// Try to extend the most recent bump allocation in place.
    ///
    /// If `ptr` points to the most recent bump allocation, we can extend it to
    /// `new_size` without copying or abandoning the old buffer. This eliminates
    /// the primary source of region memory waste: abandoned Vec/String growth.
    ///
    /// The check accounts for slab rounding: the bump allocator may have advanced
    /// by `slot_size` (rounded up to power of 2) rather than `old_size`. We use
    /// `size_class_for` + `slot_size_for_class` to compute the actual bump advance.
    ///
    /// Returns `true` if the extension succeeded (caller should NOT allocate+copy).
    /// Returns `false` if the buffer is not at the top (caller must fall back).
    ///
    /// # Safety
    ///
    /// Caller must ensure `ptr` was allocated from this region with size `old_size`.
    #[cfg(unix)]
    pub fn try_realloc_in_place(&self, ptr: *mut u8, old_size: usize, new_size: usize) -> bool {
        let ptr_addr = ptr as usize;
        let base = self.base as usize;

        // Sanity: ptr must be within this region
        if ptr_addr < base {
            return false;
        }

        let offset = self.offset.load(Ordering::Acquire);

        // Compute the actual allocation size including slab rounding.
        // The bump allocator uses slot_size.max(size) for slab-sized allocs.
        let class = size_class_for(old_size);
        let actual_old_size = if class != SIZE_CLASS_LARGE {
            let slot = slot_size_for_class(class);
            slot.max(old_size)
        } else {
            old_size
        };

        // Check: is this the most recent bump allocation?
        // The old buffer occupies [ptr_addr, ptr_addr + actual_old_size).
        if ptr_addr + actual_old_size != base + offset {
            return false;
        }

        // Compute new allocation size with slab rounding for the new size
        let new_class = size_class_for(new_size);
        let actual_new_size = if new_class != SIZE_CLASS_LARGE {
            let slot = slot_size_for_class(new_class);
            slot.max(new_size)
        } else {
            new_size
        };

        // Calculate new offset
        let new_offset = (ptr_addr - base) + actual_new_size;

        if new_offset > self.reserved {
            return false;
        }

        // Ensure we have enough committed pages
        let committed = self.committed.load(Ordering::Acquire);
        if new_offset > committed {
            let ps = page_size();
            const MAX_GROWTH: usize = 256 * 1024 * 1024;
            let growth = committed.min(MAX_GROWTH);
            let desired = round_up((committed + growth).max(new_offset), ps).min(self.reserved);
            let additional = desired - committed;
            if additional > 0 {
                let ret = unsafe {
                    libc::mprotect(
                        self.base.add(committed) as *mut libc::c_void,
                        additional,
                        libc::PROT_READ | libc::PROT_WRITE,
                    )
                };
                if ret != 0 {
                    return false;
                }
                self.committed.store(desired, Ordering::Release);
            }
        }

        // Advance the bump offset — no copy needed, buffer stays in place
        self.offset.store(new_offset, Ordering::Release);
        true
    }

    /// Non-Unix variant of try_realloc_in_place.
    #[cfg(not(unix))]
    pub fn try_realloc_in_place(&mut self, ptr: *mut u8, old_size: usize, new_size: usize) -> bool {
        let ptr_addr = ptr as usize;
        let base = self.buffer.as_ptr() as usize;

        if ptr_addr < base {
            return false;
        }

        let offset = self.offset.load(Ordering::Acquire);

        let class = size_class_for(old_size);
        let actual_old_size = if class != SIZE_CLASS_LARGE {
            let slot = slot_size_for_class(class);
            slot.max(old_size)
        } else {
            old_size
        };

        if ptr_addr + actual_old_size != base + offset {
            return false;
        }

        let new_class = size_class_for(new_size);
        let actual_new_size = if new_class != SIZE_CLASS_LARGE {
            let slot = slot_size_for_class(new_class);
            slot.max(new_size)
        } else {
            new_size
        };

        let new_offset = (ptr_addr - base) + actual_new_size;
        if new_offset > self.max_size {
            return false;
        }

        if new_offset > self.buffer.len() {
            let new_capacity = (new_offset * 2).min(self.max_size);
            self.buffer.resize(new_capacity, 0);
        }

        self.offset.store(new_offset, Ordering::Release);
        true
    }

    /// Deallocate memory, returning it to the appropriate free list.
    ///
    /// This enables memory reuse within the region. The slot's generation
    /// should have been incremented by the caller (via slot registry).
    ///
    /// # Thread Safety
    ///
    /// This method is thread-safe. It acquires an internal mutex to protect
    /// the free list during modification. Multiple threads can safely call
    /// `deallocate` concurrently on the same region.
    ///
    /// # Returns
    ///
    /// `true` if the address was added to a free list for potential reuse.
    /// `false` for large allocations (>16KB) which are not added to free lists.
    #[must_use]
    pub fn deallocate(&self, addr: u64, size_class: u8) -> bool {
        if size_class == SIZE_CLASS_LARGE || size_class as usize >= NUM_SIZE_CLASSES {
            // Large allocations don't go to free lists.
            // Release physical pages via madvise so RSS drops immediately.
            #[cfg(unix)]
            {
                // We don't know the exact size, but the allocation is page-aligned
                // and at least one page. Use page-aligned madvise on the address.
                // The caller should prefer deallocate_large() with the actual size.
                self.stats.deallocations.fetch_add(1, Ordering::Relaxed);
            }
            #[cfg(not(unix))]
            {
                self.stats.deallocations.fetch_add(1, Ordering::Relaxed);
            }
            return false;
        }

        // Add to the appropriate free list
        let mut lists = self.free_lists.lock();
        lists[size_class as usize].push(addr);
        self.stats.deallocations.fetch_add(1, Ordering::Relaxed);
        true
    }

    /// Deallocate a large allocation (>16KB) and release its physical pages.
    ///
    /// Unlike `deallocate()` which doesn't know the allocation size for large
    /// allocations, this method takes the exact size and calls `madvise(MADV_DONTNEED)`
    /// to release the physical pages back to the OS while keeping the virtual
    /// address space mapped.
    #[cfg(unix)]
    pub fn deallocate_large(&self, addr: u64, size: usize) {
        let ps = page_size();
        // Align to page boundaries for madvise
        let page_start = addr as usize & !(ps - 1);
        let page_end = round_up(addr as usize + size, ps);
        let page_len = page_end - page_start;
        if page_len > 0 {
            unsafe {
                libc::madvise(
                    page_start as *mut libc::c_void,
                    page_len,
                    libc::MADV_DONTNEED,
                );
            }
        }
        self.stats.deallocations.fetch_add(1, Ordering::Relaxed);
    }

    /// Get allocation statistics.
    pub fn stats(&self) -> &RegionStats {
        &self.stats
    }

    /// Reset the region, freeing all allocations and clearing free lists.
    pub fn reset(&mut self) {
        // Clear all free lists
        {
            let mut lists = self.free_lists.lock();
            for list in lists.iter_mut() {
                list.clear();
            }
        }

        self.offset.store(0, Ordering::Release);
        self.closed.store(0, Ordering::Release);
        self.suspend_count.store(0, Ordering::Release);
        self.status.store(RegionStatus::Active as u32, Ordering::Release);
    }

    /// Trim committed pages above the current allocation offset.
    ///
    /// Calls `madvise(MADV_DONTNEED)` on committed pages beyond the current
    /// bump-pointer offset, releasing physical pages back to the OS. The virtual
    /// address space remains mapped (PROT_READ|PROT_WRITE) so future allocations
    /// will re-fault pages on demand.
    ///
    /// Also clears free lists since freed slots above the trim point are invalid.
    ///
    /// Call this at phase boundaries (after HIR lowering, after type checking)
    /// to release temporarily-needed memory.
    #[cfg(unix)]
    pub fn trim(&self) {
        let offset = self.offset.load(Ordering::Acquire);
        let committed = self.committed.load(Ordering::Acquire);
        let ps = page_size();

        // Round offset up to next page boundary — don't touch the page
        // containing the current allocation frontier.
        let trim_start = round_up(offset, ps);

        if trim_start >= committed {
            return; // Nothing to trim
        }

        let trim_len = committed - trim_start;

        unsafe {
            libc::madvise(
                self.base.add(trim_start) as *mut libc::c_void,
                trim_len,
                libc::MADV_DONTNEED,
            );
        }

        // Clear free lists — slots in the trimmed region are no longer backed
        // by physical pages, so reusing them would fault in new zeroed pages.
        // This is safe because the pages are still mapped PROT_READ|PROT_WRITE.
        let mut lists = self.free_lists.lock();
        for list in lists.iter_mut() {
            list.clear();
        }
    }

    // ========================================================================
    // Suspension Support for Effect Handlers
    // ========================================================================

    /// Get the current suspend count.
    pub fn suspend_count(&self) -> u32 {
        self.suspend_count.load(Ordering::Acquire)
    }

    /// Get the current region status.
    pub fn status(&self) -> RegionStatus {
        RegionStatus::from_u32(self.status.load(Ordering::Acquire))
    }

    /// Check if the region is suspended (has active continuations).
    pub fn is_suspended(&self) -> bool {
        self.suspend_count.load(Ordering::Acquire) > 0
    }

    /// Check if the region is pending deallocation.
    pub fn is_pending_deallocation(&self) -> bool {
        self.status() == RegionStatus::PendingDeallocation
    }

    /// Suspend the region (called when an effect captures a continuation).
    ///
    /// Increments the suspend count and sets status to Suspended.
    /// Returns the new suspend count.
    pub fn suspend(&self) -> u32 {
        let new_count = self.suspend_count.fetch_add(1, Ordering::AcqRel) + 1;
        // Set status to Suspended if not already PendingDeallocation
        let _ = self.status.compare_exchange(
            RegionStatus::Active as u32,
            RegionStatus::Suspended as u32,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        new_count
    }

    /// Resume the region (called when a continuation resumes or is dropped).
    ///
    /// Decrements the suspend count. If the count reaches 0 and status is
    /// PendingDeallocation, returns true indicating the region should be deallocated.
    /// Returns (new_count, should_deallocate).
    pub fn resume(&self) -> (u32, bool) {
        let old_count = self.suspend_count.fetch_sub(1, Ordering::AcqRel);
        let new_count = old_count.saturating_sub(1);

        if new_count == 0 {
            // Check if we should deallocate
            let status = self.status();
            if status == RegionStatus::PendingDeallocation {
                return (new_count, true);
            }
            // Otherwise, restore to Active if we were Suspended
            let _ = self.status.compare_exchange(
                RegionStatus::Suspended as u32,
                RegionStatus::Active as u32,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
        (new_count, false)
    }

    /// Exit the region scope (called at end of lexical scope).
    ///
    /// If the region is suspended (has active continuations), defers deallocation
    /// by setting status to PendingDeallocation. Returns true if deallocation
    /// should proceed immediately, false if deferred.
    pub fn exit_scope(&self) -> bool {
        // Close the region for new allocations
        self.close();

        // Check if we have suspended continuations
        if self.suspend_count.load(Ordering::Acquire) > 0 {
            // Defer deallocation
            self.status.store(RegionStatus::PendingDeallocation as u32, Ordering::Release);
            false
        } else {
            // Immediate deallocation is safe
            true
        }
    }
}

impl fmt::Debug for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Region")
            .field("id", &self.id)
            .field("used", &self.used())
            .field("capacity", &self.capacity())
            .field("max_size", &self.max_size)
            .field("closed", &self.is_closed())
            .field("suspend_count", &self.suspend_count())
            .field("status", &self.status())
            .finish()
    }
}

#[cfg(unix)]
impl Drop for Region {
    fn drop(&mut self) {
        if !self.base.is_null() {
            unsafe {
                libc::munmap(self.base as *mut libc::c_void, self.reserved);
            }
        }
    }
}

// Safety: Region uses atomic operations for all mutable state. The mmap-backed
// buffer is only freed on drop. Raw pointer access is bounded by the bump offset.
unsafe impl Send for Region {}
unsafe impl Sync for Region {}

/// Generation snapshot for continuation capture.
#[derive(Debug, Clone)]
pub struct GenerationSnapshot {
    /// Captured (address, generation) pairs.
    entries: Vec<(usize, Generation)>,
}

impl GenerationSnapshot {
    /// Create a new empty snapshot.
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    /// Create a snapshot from a list of pointers.
    pub fn capture(pointers: &[BloodPtr]) -> Self {
        let entries = pointers
            .iter()
            .filter(|p| !p.is_null())
            .map(|p| (p.address(), p.generation()))
            .collect();
        Self { entries }
    }

    /// Add a pointer to the snapshot.
    pub fn add(&mut self, ptr: &BloodPtr) {
        if !ptr.is_null() {
            self.entries.push((ptr.address(), ptr.generation()));
        }
    }

    /// Get the number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the snapshot is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Validate all references in the snapshot.
    ///
    /// Returns the first stale reference error, if any.
    pub fn validate<F>(&self, get_slot: F) -> Result<(), StaleReferenceError>
    where
        F: Fn(usize) -> Option<Generation>,
    {
        for (addr, expected_gen) in &self.entries {
            if let Some(actual_gen) = get_slot(*addr) {
                if actual_gen != *expected_gen {
                    return Err(StaleReferenceError {
                        expected: *expected_gen,
                        actual: actual_gen,
                    });
                }
            } else {
                // Slot no longer exists
                return Err(StaleReferenceError {
                    expected: *expected_gen,
                    actual: generation::UNINITIALIZED,
                });
            }
        }
        Ok(())
    }

    /// Get the entries.
    pub fn entries(&self) -> &[(usize, Generation)] {
        &self.entries
    }
}

impl Default for GenerationSnapshot {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tier 3: Persistent (Reference Counted with Cycle Collection)
// ============================================================================

/// Type layout information for cycle collection.
///
/// Describes the location of pointer fields within a type, enabling
/// the cycle collector to traverse object graphs.
#[derive(Debug, Clone)]
pub struct TypeLayout {
    /// Offsets of slot ID fields within the type.
    ///
    /// Each offset points to a `u64` field that holds a slot ID
    /// (as returned by `persistent_alloc`).
    pub slot_offsets: Vec<usize>,
    /// Size of the type in bytes.
    pub size: usize,
}

impl TypeLayout {
    /// Create a new type layout with no pointer fields.
    pub fn new(size: usize) -> Self {
        Self {
            slot_offsets: Vec::new(),
            size,
        }
    }

    /// Create a type layout with the given slot offsets.
    pub fn with_slots(size: usize, slot_offsets: Vec<usize>) -> Self {
        Self { slot_offsets, size }
    }

    /// Add a slot offset.
    pub fn add_slot_offset(&mut self, offset: usize) {
        self.slot_offsets.push(offset);
    }
}

/// Global registry mapping type fingerprints to layouts.
pub struct TypeRegistry {
    layouts: parking_lot::RwLock<std::collections::HashMap<u32, TypeLayout>>,
}

impl TypeRegistry {
    /// Create a new empty type registry.
    pub fn new() -> Self {
        Self {
            layouts: parking_lot::RwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Register a type layout.
    ///
    /// Returns the type fingerprint (computed from the layout).
    pub fn register(&self, type_fp: u32, layout: TypeLayout) {
        self.layouts.write().insert(type_fp, layout);
    }

    /// Get the layout for a type fingerprint.
    pub fn get(&self, type_fp: u32) -> Option<TypeLayout> {
        self.layouts.read().get(&type_fp).cloned()
    }

    /// Check if a type fingerprint is registered.
    pub fn contains(&self, type_fp: u32) -> bool {
        self.layouts.read().contains_key(&type_fp)
    }

    /// Get the number of registered types.
    pub fn len(&self) -> usize {
        self.layouts.read().len()
    }

    /// Check if the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.layouts.read().is_empty()
    }
}

impl Default for TypeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Global type registry for cycle collection.
static TYPE_REGISTRY: OnceLock<TypeRegistry> = OnceLock::new();

/// Get the global type registry.
pub fn type_registry() -> &'static TypeRegistry {
    TYPE_REGISTRY.get_or_init(TypeRegistry::new)
}

/// Register a type for cycle collection.
///
/// Must be called before allocating values of this type if cycle
/// collection traversal is needed.
pub fn register_type(type_fp: u32, layout: TypeLayout) {
    type_registry().register(type_fp, layout);
}

/// Metadata for persistent slots.
#[derive(Debug, Clone, Copy, Default)]
pub struct PersistentMetadata {
    /// Type fingerprint for RTTI.
    pub type_fp: u32,
    /// Flags (frozen, etc.).
    pub flags: u32,
}

impl PersistentMetadata {
    /// Flag indicating the value is deeply immutable.
    pub const FROZEN: u32 = 1 << 0;
    /// Flag indicating weak references exist.
    pub const HAS_WEAK: u32 = 1 << 1;
    /// Flag indicating pending collection.
    pub const PENDING_DROP: u32 = 1 << 2;

    /// Create new metadata.
    pub fn new(type_fp: u32) -> Self {
        Self { type_fp, flags: 0 }
    }

    /// Check if frozen.
    pub fn is_frozen(&self) -> bool {
        self.flags & Self::FROZEN != 0
    }

    /// Set frozen flag.
    pub fn set_frozen(&mut self) {
        self.flags |= Self::FROZEN;
    }
}

/// A persistent slot with reference counting.
///
/// This is used for Tier 3 allocations that are shared across fibers
/// or have lived past generation overflow.
pub struct PersistentSlot {
    /// Pointer to the actual value data.
    value: UnsafeCell<*mut u8>,
    /// Size of the value in bytes.
    size: AtomicUsize,
    /// Strong reference count.
    refcount: AtomicU64,
    /// Weak reference count.
    weak_count: AtomicU32,
    /// Slot metadata.
    metadata: UnsafeCell<PersistentMetadata>,
    /// Layout for deallocation.
    layout: UnsafeCell<Option<Layout>>,
}

impl PersistentSlot {
    /// Create a new persistent slot.
    pub fn new(value_ptr: *mut u8, size: usize, layout: Layout, type_fp: u32) -> Self {
        Self {
            value: UnsafeCell::new(value_ptr),
            size: AtomicUsize::new(size),
            refcount: AtomicU64::new(1),
            weak_count: AtomicU32::new(0),
            metadata: UnsafeCell::new(PersistentMetadata::new(type_fp)),
            layout: UnsafeCell::new(Some(layout)),
        }
    }

    /// Get the current reference count.
    pub fn refcount(&self) -> u64 {
        self.refcount.load(Ordering::Acquire)
    }

    /// Get the weak reference count.
    pub fn weak_count(&self) -> u32 {
        self.weak_count.load(Ordering::Acquire)
    }

    /// Check if the slot is still alive (refcount > 0).
    pub fn is_alive(&self) -> bool {
        self.refcount.load(Ordering::Acquire) > 0
    }

    /// Get the value pointer.
    ///
    /// # Safety
    /// The slot must be alive (refcount > 0).
    pub unsafe fn value_ptr(&self) -> *mut u8 {
        *self.value.get()
    }

    /// Get the value size.
    pub fn size(&self) -> usize {
        self.size.load(Ordering::Acquire)
    }

    /// Increment the reference count.
    ///
    /// # Panics
    /// Panics if the refcount was zero (double-increment of dead slot).
    pub fn increment(&self) {
        let old = self.refcount.fetch_add(1, Ordering::Relaxed);
        if old == 0 {
            panic!("PersistentSlot: increment of zero refcount");
        }
    }

    /// Decrement the reference count.
    ///
    /// Returns `true` if this was the last reference (refcount became 0).
    pub fn decrement(&self) -> bool {
        let old = self.refcount.fetch_sub(1, Ordering::Release);
        if old == 1 {
            // Last reference dropped - acquire to synchronize with other decrements
            std::sync::atomic::fence(Ordering::Acquire);
            true
        } else if old == 0 {
            panic!("PersistentSlot: decrement of zero refcount");
        } else {
            false
        }
    }

    /// Increment weak reference count.
    pub fn increment_weak(&self) {
        self.weak_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement weak reference count.
    ///
    /// Returns `true` if this was the last weak reference and refcount is also 0.
    pub fn decrement_weak(&self) -> bool {
        let old = self.weak_count.fetch_sub(1, Ordering::Release);
        old == 1 && self.refcount.load(Ordering::Acquire) == 0
    }

    /// Try to upgrade a weak reference to a strong reference.
    ///
    /// Returns `true` if upgrade succeeded, `false` if the slot is dead.
    pub fn try_upgrade_weak(&self) -> bool {
        loop {
            let current = self.refcount.load(Ordering::Acquire);
            if current == 0 {
                return false;
            }
            if self.refcount.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ).is_ok() {
                return true;
            }
        }
    }

    /// Mark the slot as pending drop.
    pub fn mark_pending_drop(&self) {
        unsafe {
            let meta = &mut *self.metadata.get();
            meta.flags |= PersistentMetadata::PENDING_DROP;
        }
    }

    /// Check if pending drop.
    pub fn is_pending_drop(&self) -> bool {
        unsafe {
            let meta = &*self.metadata.get();
            meta.flags & PersistentMetadata::PENDING_DROP != 0
        }
    }

    /// Deallocate the slot's value.
    ///
    /// # Safety
    /// Must only be called when refcount and weak_count are both 0.
    pub unsafe fn deallocate_value(&self) {
        let ptr = *self.value.get();
        if !ptr.is_null() {
            if let Some(layout) = (*self.layout.get()).take() {
                alloc::dealloc(ptr, layout);
            }
            *self.value.get() = std::ptr::null_mut();
        }
    }

    /// Get the type fingerprint for this slot.
    pub fn type_fp(&self) -> u32 {
        unsafe { (*self.metadata.get()).type_fp }
    }

    /// Extract child slot IDs from this slot's value.
    ///
    /// Uses the type registry to determine which offsets contain slot IDs.
    /// Returns an empty vector if the type is not registered.
    ///
    /// # Safety
    /// The slot must be alive (refcount > 0).
    pub unsafe fn child_slots(&self) -> Vec<u64> {
        let type_fp = self.type_fp();
        let layout = match type_registry().get(type_fp) {
            Some(layout) => layout,
            None => return Vec::new(), // Type not registered, assume no pointers
        };

        let value_ptr = self.value_ptr();
        if value_ptr.is_null() {
            return Vec::new();
        }

        let value_size = self.size();
        let mut children = Vec::with_capacity(layout.slot_offsets.len());

        for &offset in &layout.slot_offsets {
            // Safety: bounds check before reading
            if offset + std::mem::size_of::<u64>() <= value_size {
                let slot_id_ptr = value_ptr.add(offset) as *const u64;
                let slot_id = *slot_id_ptr;
                if slot_id != 0 {
                    children.push(slot_id);
                }
            }
        }

        children
    }
}

impl fmt::Debug for PersistentSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PersistentSlot")
            .field("refcount", &self.refcount())
            .field("weak_count", &self.weak_count())
            .field("size", &self.size())
            .finish()
    }
}

// Safety: PersistentSlot uses atomic operations for thread safety.
unsafe impl Send for PersistentSlot {}
unsafe impl Sync for PersistentSlot {}

// ============================================================================
// Deferred Reference Counting
// ============================================================================

/// Queue for deferred reference count decrements.
///
/// This avoids deep recursion when dropping values that contain
/// many nested references.
pub struct DeferredDecrementQueue {
    /// Queue of slots pending decrement processing.
    queue: RwLock<Vec<*const PersistentSlot>>,
    /// Maximum queue size before forcing synchronous processing.
    max_size: usize,
}

impl DeferredDecrementQueue {
    /// Default maximum queue size.
    pub const DEFAULT_MAX_SIZE: usize = 1024;

    /// Create a new deferred decrement queue.
    pub fn new(max_size: usize) -> Self {
        Self {
            queue: RwLock::new(Vec::with_capacity(max_size)),
            max_size,
        }
    }

    /// Queue a slot for deferred processing.
    ///
    /// If the queue is full, processes immediately.
    ///
    /// # Safety
    /// The slot pointer must be valid.
    pub unsafe fn enqueue(&self, slot: *const PersistentSlot) {
        let mut queue = self.queue.write();
        if queue.len() >= self.max_size {
            // Queue full - process synchronously
            drop(queue);
            self.process_batch(self.max_size / 2);
            queue = self.queue.write();
        }
        queue.push(slot);
    }

    /// Process a batch of queued decrements.
    ///
    /// Returns the number of slots processed.
    pub fn process_batch(&self, max_count: usize) -> usize {
        let batch: Vec<_> = {
            let mut queue = self.queue.write();
            let count = max_count.min(queue.len());
            queue.drain(..count).collect()
        };

        let processed = batch.len();
        for slot_ptr in batch {
            // Safety: slots in queue are valid
            unsafe {
                let slot = &*slot_ptr;
                if slot.refcount() == 0 && slot.weak_count() == 0 {
                    slot.deallocate_value();
                }
            }
        }
        processed
    }

    /// Process all queued decrements.
    pub fn process_all(&self) -> usize {
        let mut total = 0;
        loop {
            let processed = self.process_batch(100);
            if processed == 0 {
                break;
            }
            total += processed;
        }
        total
    }

    /// Get the current queue length.
    pub fn len(&self) -> usize {
        self.queue.read().len()
    }

    /// Check if the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.queue.read().is_empty()
    }
}

impl Default for DeferredDecrementQueue {
    fn default() -> Self {
        Self::new(Self::DEFAULT_MAX_SIZE)
    }
}

// Safety: Queue uses RwLock for thread safety.
unsafe impl Send for DeferredDecrementQueue {}
unsafe impl Sync for DeferredDecrementQueue {}

/// Global deferred decrement queue.
static DEFERRED_QUEUE: OnceLock<DeferredDecrementQueue> = OnceLock::new();

/// Get the global deferred decrement queue.
pub fn deferred_queue() -> &'static DeferredDecrementQueue {
    DEFERRED_QUEUE.get_or_init(DeferredDecrementQueue::default)
}

// ============================================================================
// Persistent Allocator
// ============================================================================

/// Allocator for Tier 3 (Persistent) memory.
pub struct PersistentAllocator {
    /// All active persistent slots.
    slots: RwLock<HashMap<u64, Box<PersistentSlot>>>,
    /// Next slot ID.
    next_id: AtomicU64,
    /// Statistics.
    stats: PersistentStats,
}

/// Statistics for the persistent allocator.
#[derive(Debug, Default)]
pub struct PersistentStats {
    /// Total allocations.
    pub allocations: AtomicU64,
    /// Total deallocations.
    pub deallocations: AtomicU64,
    /// Current live slots.
    pub live_slots: AtomicU64,
    /// Cycle collection runs.
    pub cycle_collections: AtomicU64,
    /// Cycles collected.
    pub cycles_collected: AtomicU64,
}

impl PersistentAllocator {
    /// Create a new persistent allocator.
    pub fn new() -> Self {
        Self {
            slots: RwLock::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            stats: PersistentStats::default(),
        }
    }

    /// Allocate a new persistent slot.
    ///
    /// Returns the slot ID.
    pub fn allocate(&self, size: usize, align: usize, type_fp: u32) -> Option<u64> {
        let layout = Layout::from_size_align(size, align).ok()?;
        let ptr = unsafe { alloc::alloc(layout) };
        if ptr.is_null() {
            return None;
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let slot = Box::new(PersistentSlot::new(ptr, size, layout, type_fp));

        self.slots.write().insert(id, slot);
        self.stats.allocations.fetch_add(1, Ordering::Relaxed);
        self.stats.live_slots.fetch_add(1, Ordering::Relaxed);

        Some(id)
    }

    /// Get a slot by ID.
    pub fn get(&self, id: u64) -> Option<*const PersistentSlot> {
        self.slots.read().get(&id).map(|s| s.as_ref() as *const _)
    }

    /// Increment reference count for a slot.
    pub fn increment(&self, id: u64) -> bool {
        if let Some(slot) = self.slots.read().get(&id) {
            slot.increment();
            true
        } else {
            false
        }
    }

    /// Decrement reference count for a slot.
    ///
    /// If this was the last reference, queues for deferred cleanup.
    pub fn decrement(&self, id: u64) {
        let slot_ptr = {
            let slots = self.slots.read();
            slots.get(&id).map(|s| s.as_ref() as *const _)
        };

        if let Some(ptr) = slot_ptr {
            let slot: &PersistentSlot = unsafe { &*ptr };
            if slot.decrement() {
                // Last reference - queue for deferred processing
                unsafe { deferred_queue().enqueue(ptr) };
                self.stats.deallocations.fetch_add(1, Ordering::Relaxed);
                self.stats.live_slots.fetch_sub(1, Ordering::Relaxed);
            }
        }
    }

    /// Remove a slot from the allocator.
    ///
    /// # Safety
    /// Only call when the slot has been fully processed.
    pub unsafe fn remove(&self, id: u64) {
        self.slots.write().remove(&id);
    }

    /// Get allocator statistics.
    pub fn stats(&self) -> &PersistentStats {
        &self.stats
    }

    /// Find the slot ID that owns the given address.
    ///
    /// Iterates all active slots and returns the ID of the slot whose
    /// value pointer matches the given address, if any.
    pub fn find_slot_by_address(&self, address: usize) -> Option<u64> {
        let slots = self.slots.read();
        for (&id, slot) in slots.iter() {
            // Safety: we only read the pointer value for comparison;
            // the slot is alive because it is in the allocator map.
            if unsafe { slot.value_ptr() } as usize == address {
                return Some(id);
            }
        }
        None
    }

    /// Get the number of live slots.
    pub fn live_count(&self) -> usize {
        self.slots.read().len()
    }

    /// Get the number of slots (alias for live_count).
    pub fn slot_count(&self) -> usize {
        self.live_count()
    }
}

impl Default for PersistentAllocator {
    fn default() -> Self {
        Self::new()
    }
}

/// Global persistent allocator.
static PERSISTENT_ALLOCATOR: OnceLock<PersistentAllocator> = OnceLock::new();

/// Get the global persistent allocator.
pub fn persistent_allocator() -> &'static PersistentAllocator {
    PERSISTENT_ALLOCATOR.get_or_init(PersistentAllocator::new)
}

// ============================================================================
// Cycle Collection
// ============================================================================

/// Cycle collector for Tier 3 memory.
///
/// Uses mark-sweep over persistent slots to detect and collect
/// reference cycles that cannot be collected by reference counting alone.
pub struct CycleCollector {
    /// Threshold for triggering collection (bytes).
    threshold: AtomicUsize,
    /// Whether collection is currently running.
    collecting: AtomicU32,
}

/// Configuration for cycle collection.
#[derive(Debug, Clone)]
pub struct CycleCollectorConfig {
    /// Memory threshold to trigger collection (bytes).
    pub threshold_bytes: usize,
    /// Maximum slots to process per batch.
    pub batch_size: usize,
}

impl Default for CycleCollectorConfig {
    fn default() -> Self {
        Self {
            threshold_bytes: 10 * 1024 * 1024, // 10 MB
            batch_size: 1000,
        }
    }
}

impl CycleCollector {
    /// Create a new cycle collector with default config.
    pub fn new() -> Self {
        Self::with_config(CycleCollectorConfig::default())
    }

    /// Create a new cycle collector with custom config.
    pub fn with_config(config: CycleCollectorConfig) -> Self {
        Self {
            threshold: AtomicUsize::new(config.threshold_bytes),
            collecting: AtomicU32::new(0),
        }
    }

    /// Check if collection is currently running.
    pub fn is_collecting(&self) -> bool {
        self.collecting.load(Ordering::Acquire) != 0
    }

    /// Get the current collection threshold in bytes.
    pub fn threshold(&self) -> usize {
        self.threshold.load(Ordering::Acquire)
    }

    /// Set the collection threshold in bytes.
    pub fn set_threshold(&self, bytes: usize) {
        self.threshold.store(bytes, Ordering::Release);
    }

    /// Check if collection should be triggered based on slot count.
    ///
    /// Returns true if the number of live slots exceeds the threshold.
    /// Note: Threshold is interpreted as slot count, not bytes.
    pub fn should_collect(&self) -> bool {
        let stats = persistent_allocator().stats();
        stats.live_slots.load(Ordering::Relaxed) as usize >= self.threshold()
    }

    /// Try to start collection.
    ///
    /// Returns `false` if collection is already running.
    fn try_start(&self) -> bool {
        self.collecting.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Relaxed).is_ok()
    }

    /// Mark collection as finished.
    fn finish(&self) {
        self.collecting.store(0, Ordering::Release);
    }

    /// Run a cycle collection.
    ///
    /// Returns the number of cycles collected.
    pub fn collect(&self, roots: &[u64]) -> usize {
        if !self.try_start() {
            return 0; // Already collecting
        }

        let allocator = persistent_allocator();

        // Phase 1: Mark all reachable from roots
        let mut marked = std::collections::HashSet::new();
        let mut worklist: Vec<u64> = roots.to_vec();

        while let Some(id) = worklist.pop() {
            if marked.contains(&id) {
                continue;
            }
            marked.insert(id);

            // Traverse object graph using type information
            if let Some(slot) = allocator.get(id) {
                unsafe {
                    // Only traverse alive slots
                    if (*slot).is_alive() {
                        let children = (*slot).child_slots();
                        for child_id in children {
                            if !marked.contains(&child_id) {
                                worklist.push(child_id);
                            }
                        }
                    }
                }
            }
        }

        // Phase 2: Sweep unmarked slots
        let mut collected = 0;
        let slots = allocator.slots.read();
        let unmarked: Vec<u64> = slots
            .iter()
            .filter(|(&id, slot)| {
                !marked.contains(&id) && slot.refcount() == 0 && !slot.is_pending_drop()
            })
            .map(|(&id, _)| id)
            .collect();
        drop(slots);

        for id in unmarked {
            // Mark for cleanup
            if let Some(slot) = allocator.get(id) {
                unsafe {
                    (*slot).mark_pending_drop();
                    (*slot).deallocate_value();
                }
            }
            unsafe { allocator.remove(id) };
            collected += 1;
        }

        allocator.stats.cycle_collections.fetch_add(1, Ordering::Relaxed);
        allocator.stats.cycles_collected.fetch_add(collected as u64, Ordering::Relaxed);

        self.finish();
        collected
    }

    /// Collect with roots extracted from suspended continuations.
    ///
    /// This is the primary entry point for cycle collection that
    /// respects algebraic effect safety. Snapshot refs represent memory
    /// addresses held by suspended effect handler continuations that
    /// must be treated as GC roots to prevent premature collection.
    pub fn collect_with_snapshot_roots(&self, snapshot_refs: &[(usize, Generation)]) -> usize {
        if snapshot_refs.is_empty() {
            return self.collect(&[]);
        }

        // Resolve snapshot addresses to slot IDs. Each snapshot ref is an
        // (address, generation) pair from a suspended continuation. We look
        // up which persistent slot owns each address and treat those slots
        // as GC roots so they survive cycle collection.
        let allocator = persistent_allocator();
        let mut roots = Vec::new();
        for &(address, _gen) in snapshot_refs {
            if let Some(slot_id) = allocator.find_slot_by_address(address) {
                roots.push(slot_id);
            }
        }
        self.collect(&roots)
    }
}

impl Default for CycleCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Global cycle collector.
static CYCLE_COLLECTOR: OnceLock<CycleCollector> = OnceLock::new();

/// Get the global cycle collector.
pub fn cycle_collector() -> &'static CycleCollector {
    CYCLE_COLLECTOR.get_or_init(CycleCollector::new)
}

/// Trigger a cycle collection with the given roots.
pub fn collect_cycles(roots: &[u64]) -> usize {
    cycle_collector().collect(roots)
}

/// Trigger a cycle collection with snapshot roots.
pub fn collect_cycles_with_snapshots(snapshots: &[GenerationSnapshot]) -> usize {
    let all_refs: Vec<_> = snapshots
        .iter()
        .flat_map(|s| s.entries().iter().copied())
        .collect();
    cycle_collector().collect_with_snapshot_roots(&all_refs)
}

// ============================================================================
// Tier 3 Public API
// ============================================================================

/// Allocate a persistent (Tier 3) value.
///
/// Returns the slot ID and a pointer to the allocated memory.
pub fn persistent_alloc(size: usize, align: usize, type_fp: u32) -> Option<(u64, *mut u8)> {
    let id = persistent_allocator().allocate(size, align, type_fp)?;
    let slot_ptr = persistent_allocator().get(id)?;
    let value_ptr = unsafe { (*slot_ptr).value_ptr() };
    Some((id, value_ptr))
}

/// Increment the reference count for a persistent slot.
pub fn persistent_increment(id: u64) -> bool {
    persistent_allocator().increment(id)
}

/// Decrement the reference count for a persistent slot.
pub fn persistent_decrement(id: u64) {
    persistent_allocator().decrement(id)
}

/// Get the reference count for a persistent slot.
pub fn persistent_refcount(id: u64) -> Option<u64> {
    persistent_allocator()
        .get(id)
        .map(|ptr| unsafe { (*ptr).refcount() })
}

/// Check if a persistent slot is alive.
pub fn persistent_is_alive(id: u64) -> bool {
    persistent_allocator()
        .get(id)
        .map(|ptr| unsafe { (*ptr).is_alive() })
        .unwrap_or(false)
}

// ============================================================================
// Tier 2: Reference-Counted Allocator (SSM Tier 2 / Persistent)
// ============================================================================

/// Slot identifier for Tier 2 allocations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Tier2SlotId(pub u64);

impl Tier2SlotId {
    /// Get the raw ID value.
    pub fn as_u64(&self) -> u64 {
        self.0
    }

    /// Create from raw value.
    pub fn from_u64(id: u64) -> Self {
        Self(id)
    }
}

/// A Tier 2 slot with reference counting.
///
/// This implements the SSM Tier 2 (Persistent) allocation as specified in
/// MEMORY_MODEL.md Section 3.4 and Section 8.
///
/// Key properties:
/// - Atomic reference counting for thread safety
/// - Weak reference support
/// - Preserves original generation for stale detection during promotion
pub struct Tier2Slot {
    /// Pointer to the actual value data.
    value: UnsafeCell<*mut u8>,
    /// Size of the value in bytes.
    size: AtomicUsize,
    /// Strong reference count (atomic for thread safety).
    refcount: AtomicU64,
    /// Weak reference count (atomic for thread safety).
    weak_count: AtomicU32,
    /// Original generation when promoted from Tier 1 (for stale detection).
    original_generation: AtomicU32,
    /// Type fingerprint for RTTI.
    type_fp: AtomicU32,
    /// Slot metadata flags.
    flags: AtomicU32,
    /// Layout for deallocation.
    layout: UnsafeCell<Option<Layout>>,
}

impl Tier2Slot {
    /// Flag indicating the value is deeply immutable (frozen).
    pub const FLAG_FROZEN: u32 = 1 << 0;
    /// Flag indicating weak references exist.
    pub const FLAG_HAS_WEAK: u32 = 1 << 1;
    /// Flag indicating the slot is pending drop.
    pub const FLAG_PENDING_DROP: u32 = 1 << 2;
    /// Flag indicating this slot was promoted from Tier 1.
    pub const FLAG_PROMOTED: u32 = 1 << 3;

    /// Create a new Tier 2 slot.
    ///
    /// # Arguments
    /// * `value_ptr` - Pointer to the allocated memory
    /// * `size` - Size of the allocation in bytes
    /// * `layout` - Memory layout for deallocation
    /// * `type_fp` - Type fingerprint
    /// * `original_generation` - Generation from Tier 1 if promoted
    pub fn new(
        value_ptr: *mut u8,
        size: usize,
        layout: Layout,
        type_fp: u32,
        original_generation: Generation,
    ) -> Self {
        Self {
            value: UnsafeCell::new(value_ptr),
            size: AtomicUsize::new(size),
            refcount: AtomicU64::new(1),
            weak_count: AtomicU32::new(0),
            original_generation: AtomicU32::new(original_generation),
            type_fp: AtomicU32::new(type_fp),
            flags: AtomicU32::new(0),
            layout: UnsafeCell::new(Some(layout)),
        }
    }

    /// Create a new slot for a value promoted from Tier 1.
    pub fn promoted(
        value_ptr: *mut u8,
        size: usize,
        layout: Layout,
        type_fp: u32,
        original_generation: Generation,
    ) -> Self {
        let slot = Self::new(value_ptr, size, layout, type_fp, original_generation);
        slot.flags.fetch_or(Self::FLAG_PROMOTED, Ordering::Release);
        slot
    }

    /// Get the current strong reference count.
    pub fn refcount(&self) -> u64 {
        self.refcount.load(Ordering::Acquire)
    }

    /// Get the current weak reference count.
    pub fn weak_count(&self) -> u32 {
        self.weak_count.load(Ordering::Acquire)
    }

    /// Check if the slot is still alive (refcount > 0).
    pub fn is_alive(&self) -> bool {
        self.refcount.load(Ordering::Acquire) > 0
    }

    /// Get the value pointer.
    ///
    /// # Safety
    /// The slot must be alive (refcount > 0).
    pub unsafe fn value_ptr(&self) -> *mut u8 {
        *self.value.get()
    }

    /// Get the value size.
    pub fn size(&self) -> usize {
        self.size.load(Ordering::Acquire)
    }

    /// Get the original generation (from Tier 1 promotion).
    pub fn original_generation(&self) -> Generation {
        self.original_generation.load(Ordering::Acquire)
    }

    /// Get the type fingerprint.
    pub fn type_fp(&self) -> u32 {
        self.type_fp.load(Ordering::Acquire)
    }

    /// Check if this slot was promoted from Tier 1.
    pub fn is_promoted(&self) -> bool {
        self.flags.load(Ordering::Acquire) & Self::FLAG_PROMOTED != 0
    }

    /// Check if this slot is frozen (deeply immutable).
    pub fn is_frozen(&self) -> bool {
        self.flags.load(Ordering::Acquire) & Self::FLAG_FROZEN != 0
    }

    /// Set the frozen flag.
    pub fn set_frozen(&self) {
        self.flags.fetch_or(Self::FLAG_FROZEN, Ordering::Release);
    }

    /// Check if pending drop.
    pub fn is_pending_drop(&self) -> bool {
        self.flags.load(Ordering::Acquire) & Self::FLAG_PENDING_DROP != 0
    }

    /// Mark as pending drop.
    pub fn mark_pending_drop(&self) {
        self.flags.fetch_or(Self::FLAG_PENDING_DROP, Ordering::Release);
    }

    /// Increment the strong reference count.
    ///
    /// # Panics
    /// Panics if the refcount was zero (incrementing dead slot).
    pub fn increment(&self) {
        let old = self.refcount.fetch_add(1, Ordering::Relaxed);
        if old == 0 {
            panic!("Tier2Slot: increment of zero refcount");
        }
        // Overflow check - u64 is large enough this shouldn't happen in practice
        if old == u64::MAX {
            panic!("Tier2Slot: refcount overflow");
        }
    }

    /// Decrement the strong reference count.
    ///
    /// Returns `true` if this was the last reference (refcount became 0).
    pub fn decrement(&self) -> bool {
        let old = self.refcount.fetch_sub(1, Ordering::Release);
        if old == 1 {
            // Last reference dropped - acquire fence to synchronize with other decrements
            std::sync::atomic::fence(Ordering::Acquire);
            true
        } else if old == 0 {
            panic!("Tier2Slot: decrement of zero refcount");
        } else {
            false
        }
    }

    /// Increment the weak reference count.
    pub fn increment_weak(&self) {
        let old = self.weak_count.fetch_add(1, Ordering::Relaxed);
        if old == 0 {
            self.flags.fetch_or(Self::FLAG_HAS_WEAK, Ordering::Release);
        }
    }

    /// Decrement the weak reference count.
    ///
    /// Returns `true` if this was the last weak reference AND refcount is also 0.
    pub fn decrement_weak(&self) -> bool {
        let old = self.weak_count.fetch_sub(1, Ordering::Release);
        if old == 0 {
            panic!("Tier2Slot: decrement of zero weak_count");
        }
        old == 1 && self.refcount.load(Ordering::Acquire) == 0
    }

    /// Try to upgrade a weak reference to a strong reference.
    ///
    /// Returns `true` if upgrade succeeded, `false` if the slot is dead.
    pub fn try_upgrade_weak(&self) -> bool {
        loop {
            let current = self.refcount.load(Ordering::Acquire);
            if current == 0 {
                return false;
            }
            match self.refcount.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(_) => continue, // Retry CAS
            }
        }
    }

    /// Deallocate the slot's value.
    ///
    /// # Safety
    /// Must only be called when refcount and weak_count are both 0.
    pub unsafe fn deallocate_value(&self) {
        let ptr = *self.value.get();
        if !ptr.is_null() {
            if let Some(layout) = (*self.layout.get()).take() {
                alloc::dealloc(ptr, layout);
            }
            *self.value.get() = std::ptr::null_mut();
        }
    }

    /// Extract child slot IDs from this slot's value.
    ///
    /// Uses the type registry to determine which offsets contain slot IDs.
    /// Returns an empty vector if the type is not registered.
    ///
    /// # Safety
    /// The slot must be alive (refcount > 0).
    pub unsafe fn child_slots(&self) -> Vec<u64> {
        let type_fp = self.type_fp();
        let layout = match type_registry().get(type_fp) {
            Some(layout) => layout,
            None => return Vec::new(),
        };

        let value_ptr = self.value_ptr();
        if value_ptr.is_null() {
            return Vec::new();
        }

        let value_size = self.size();
        let mut children = Vec::with_capacity(layout.slot_offsets.len());

        for &offset in &layout.slot_offsets {
            if offset + std::mem::size_of::<u64>() <= value_size {
                let slot_id_ptr = value_ptr.add(offset) as *const u64;
                let slot_id = *slot_id_ptr;
                if slot_id != 0 {
                    children.push(slot_id);
                }
            }
        }

        children
    }
}

impl fmt::Debug for Tier2Slot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tier2Slot")
            .field("refcount", &self.refcount())
            .field("weak_count", &self.weak_count())
            .field("size", &self.size())
            .field("original_generation", &self.original_generation())
            .field("is_promoted", &self.is_promoted())
            .field("is_frozen", &self.is_frozen())
            .finish()
    }
}

// Safety: Tier2Slot uses atomic operations for all mutable state.
unsafe impl Send for Tier2Slot {}
unsafe impl Sync for Tier2Slot {}

/// Statistics for the Tier 2 allocator.
#[derive(Debug, Default)]
pub struct Tier2Stats {
    /// Total allocations performed.
    pub allocations: AtomicU64,
    /// Total deallocations performed.
    pub deallocations: AtomicU64,
    /// Current number of live slots.
    pub live_slots: AtomicU64,
    /// Number of promotions from Tier 1.
    pub promotions: AtomicU64,
    /// Number of weak reference upgrades.
    pub weak_upgrades: AtomicU64,
    /// Number of failed weak upgrades (target was dead).
    pub weak_upgrade_failures: AtomicU64,
}

/// Allocator for Tier 2 (Persistent/Reference-Counted) memory.
///
/// This implements the SSM Tier 2 allocator as specified in MEMORY_MODEL.md.
/// It manages reference-counted allocations with support for:
/// - Thread-safe atomic reference counting
/// - Weak references
/// - Promotion from Tier 1 with generation preservation
pub struct Tier2Allocator {
    /// All active Tier 2 slots.
    slots: RwLock<HashMap<u64, Box<Tier2Slot>>>,
    /// Next slot ID.
    next_id: AtomicU64,
    /// Allocator statistics.
    stats: Tier2Stats,
}

impl Tier2Allocator {
    /// Create a new Tier 2 allocator.
    pub fn new() -> Self {
        Self {
            slots: RwLock::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            stats: Tier2Stats::default(),
        }
    }

    /// Allocate a new Tier 2 slot.
    ///
    /// # Arguments
    /// * `size` - Size of the allocation in bytes
    /// * `align` - Alignment requirement
    /// * `type_fp` - Type fingerprint for RTTI
    ///
    /// # Returns
    /// The slot ID if allocation succeeded, None if allocation failed.
    pub fn allocate(&self, size: usize, align: usize, type_fp: u32) -> Option<Tier2SlotId> {
        let layout = Layout::from_size_align(size, align).ok()?;
        let ptr = unsafe { alloc::alloc(layout) };
        if ptr.is_null() {
            return None;
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let slot = Box::new(Tier2Slot::new(ptr, size, layout, type_fp, generation::FIRST));

        self.slots.write().insert(id, slot);
        self.stats.allocations.fetch_add(1, Ordering::Relaxed);
        self.stats.live_slots.fetch_add(1, Ordering::Relaxed);

        Some(Tier2SlotId(id))
    }

    /// Promote a Tier 1 allocation to Tier 2.
    ///
    /// This is called when:
    /// - A Tier 1 value escapes its region
    /// - Generation counter overflow
    /// - Explicit `persist(value)` annotation
    ///
    /// # Arguments
    /// * `value_ptr` - Pointer to the value (will be owned by Tier 2)
    /// * `size` - Size of the value
    /// * `align` - Alignment of the value
    /// * `type_fp` - Type fingerprint
    /// * `original_generation` - Generation from the Tier 1 slot
    ///
    /// # Returns
    /// The new Tier 2 slot ID.
    pub fn promote_from_tier1(
        &self,
        value_ptr: *mut u8,
        size: usize,
        align: usize,
        type_fp: u32,
        original_generation: Generation,
    ) -> Option<Tier2SlotId> {
        if value_ptr.is_null() {
            return None;
        }

        let layout = Layout::from_size_align(size, align).ok()?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let slot = Box::new(Tier2Slot::promoted(
            value_ptr,
            size,
            layout,
            type_fp,
            original_generation,
        ));

        self.slots.write().insert(id, slot);
        self.stats.allocations.fetch_add(1, Ordering::Relaxed);
        self.stats.live_slots.fetch_add(1, Ordering::Relaxed);
        self.stats.promotions.fetch_add(1, Ordering::Relaxed);

        Some(Tier2SlotId(id))
    }

    /// Get a slot by ID.
    pub fn get(&self, id: Tier2SlotId) -> Option<*const Tier2Slot> {
        self.slots.read().get(&id.0).map(|s| s.as_ref() as *const _)
    }

    /// Increment the reference count for a slot.
    ///
    /// Returns `true` if the increment succeeded, `false` if the slot doesn't exist.
    pub fn increment(&self, id: Tier2SlotId) -> bool {
        if let Some(slot) = self.slots.read().get(&id.0) {
            slot.increment();
            true
        } else {
            false
        }
    }

    /// Decrement the reference count for a slot.
    ///
    /// If this was the last reference, the slot is queued for deferred cleanup.
    pub fn decrement(&self, id: Tier2SlotId) {
        let slot_ptr = {
            let slots = self.slots.read();
            slots.get(&id.0).map(|s| s.as_ref() as *const _)
        };

        if let Some(ptr) = slot_ptr {
            let slot: &Tier2Slot = unsafe { &*ptr };
            if slot.decrement() {
                // Last reference - queue for deferred processing
                unsafe { deferred_queue().enqueue(ptr as *const PersistentSlot) };
                self.stats.deallocations.fetch_add(1, Ordering::Relaxed);
                self.stats.live_slots.fetch_sub(1, Ordering::Relaxed);
            }
        }
    }

    /// Increment the weak reference count for a slot.
    pub fn increment_weak(&self, id: Tier2SlotId) -> bool {
        if let Some(slot) = self.slots.read().get(&id.0) {
            slot.increment_weak();
            true
        } else {
            false
        }
    }

    /// Decrement the weak reference count for a slot.
    pub fn decrement_weak(&self, id: Tier2SlotId) {
        let should_remove = {
            let slots = self.slots.read();
            if let Some(slot) = slots.get(&id.0) {
                slot.decrement_weak()
            } else {
                false
            }
        };

        if should_remove {
            // Both refcount and weak_count are 0 - remove the slot
            if let Some(slot) = self.slots.write().remove(&id.0) {
                unsafe { slot.deallocate_value() };
            }
        }
    }

    /// Try to upgrade a weak reference to a strong reference.
    ///
    /// Returns `true` if upgrade succeeded, `false` if the slot is dead or doesn't exist.
    pub fn try_upgrade_weak(&self, id: Tier2SlotId) -> bool {
        if let Some(slot) = self.slots.read().get(&id.0) {
            let result = slot.try_upgrade_weak();
            if result {
                self.stats.weak_upgrades.fetch_add(1, Ordering::Relaxed);
            } else {
                self.stats.weak_upgrade_failures.fetch_add(1, Ordering::Relaxed);
            }
            result
        } else {
            false
        }
    }

    /// Remove a slot from the allocator.
    ///
    /// # Safety
    /// Only call when the slot has been fully processed (refcount and weak_count are 0).
    pub unsafe fn remove(&self, id: Tier2SlotId) {
        self.slots.write().remove(&id.0);
    }

    /// Get allocator statistics.
    pub fn stats(&self) -> &Tier2Stats {
        &self.stats
    }

    /// Get the number of live slots.
    pub fn live_count(&self) -> usize {
        self.slots.read().len()
    }

    /// Get the reference count for a slot.
    pub fn refcount(&self, id: Tier2SlotId) -> Option<u64> {
        self.slots.read().get(&id.0).map(|s| s.refcount())
    }

    /// Get the weak reference count for a slot.
    pub fn weak_count(&self, id: Tier2SlotId) -> Option<u32> {
        self.slots.read().get(&id.0).map(|s| s.weak_count())
    }

    /// Check if a slot is alive.
    pub fn is_alive(&self, id: Tier2SlotId) -> bool {
        self.slots.read().get(&id.0).map(|s| s.is_alive()).unwrap_or(false)
    }

    /// Check if a slot was promoted from Tier 1.
    pub fn is_promoted(&self, id: Tier2SlotId) -> bool {
        self.slots.read().get(&id.0).map(|s| s.is_promoted()).unwrap_or(false)
    }

    /// Get the original generation of a promoted slot.
    pub fn original_generation(&self, id: Tier2SlotId) -> Option<Generation> {
        self.slots.read().get(&id.0).map(|s| s.original_generation())
    }

    /// Get the value pointer for a slot.
    ///
    /// # Safety
    /// The slot must be alive.
    pub unsafe fn value_ptr(&self, id: Tier2SlotId) -> Option<*mut u8> {
        self.slots.read().get(&id.0).map(|s| s.value_ptr())
    }
}

impl Default for Tier2Allocator {
    fn default() -> Self {
        Self::new()
    }
}

/// Global Tier 2 allocator.
static TIER2_ALLOCATOR: OnceLock<Tier2Allocator> = OnceLock::new();

/// Get the global Tier 2 allocator.
pub fn tier2_allocator() -> &'static Tier2Allocator {
    TIER2_ALLOCATOR.get_or_init(Tier2Allocator::new)
}

// ============================================================================
// PersistentPtr: Smart Pointer for Tier 2 Allocations
// ============================================================================

/// A reference-counted smart pointer for Tier 2 allocations.
///
/// This implements automatic reference counting for Tier 2 (Persistent) allocations.
/// - Clone increments the reference count
/// - Drop decrements the reference count (frees when zero)
///
/// # Thread Safety
/// `PersistentPtr` is `Send` and `Sync` - it can be shared across threads safely.
/// The underlying reference counts use atomic operations.
///
/// # Example
/// ```ignore
/// let ptr = PersistentPtr::<i32>::new(42)?;
/// let ptr2 = ptr.clone(); // refcount = 2
/// drop(ptr);              // refcount = 1
/// drop(ptr2);             // refcount = 0, memory freed
/// ```
pub struct PersistentPtr<T> {
    /// Slot ID in the Tier 2 allocator.
    slot_id: Tier2SlotId,
    /// Phantom data for type safety.
    _marker: std::marker::PhantomData<T>,
}

impl<T> PersistentPtr<T> {
    /// Create a new PersistentPtr by allocating and initializing a value.
    pub fn new(value: T) -> Option<Self> {
        let size = std::mem::size_of::<T>();
        let align = std::mem::align_of::<T>();
        let type_fp = type_fingerprint::<T>();

        let slot_id = tier2_allocator().allocate(size, align, type_fp)?;

        // Write the value to the allocated memory
        unsafe {
            let ptr = tier2_allocator().value_ptr(slot_id)?;
            std::ptr::write(ptr as *mut T, value);
        }

        Some(Self {
            slot_id,
            _marker: std::marker::PhantomData,
        })
    }

    /// Create a PersistentPtr from an existing Tier 2 slot.
    ///
    /// # Safety
    /// The slot must contain a valid value of type T.
    pub unsafe fn from_slot_id(slot_id: Tier2SlotId) -> Option<Self> {
        if tier2_allocator().is_alive(slot_id) {
            tier2_allocator().increment(slot_id);
            Some(Self {
                slot_id,
                _marker: std::marker::PhantomData,
            })
        } else {
            None
        }
    }

    /// Get the slot ID.
    pub fn slot_id(&self) -> Tier2SlotId {
        self.slot_id
    }

    /// Get a reference to the value.
    ///
    /// # Safety
    /// The pointer must be valid (not dropped).
    pub fn get(&self) -> &T {
        unsafe {
            let ptr = tier2_allocator().value_ptr(self.slot_id)
                .expect("PersistentPtr: slot not found");
            &*(ptr as *const T)
        }
    }

    /// Get a mutable reference to the value.
    ///
    /// # Safety
    /// The pointer must be valid and there must be no other references.
    pub fn get_mut(&mut self) -> &mut T {
        unsafe {
            let ptr = tier2_allocator().value_ptr(self.slot_id)
                .expect("PersistentPtr: slot not found");
            &mut *(ptr as *mut T)
        }
    }

    /// Get the current reference count.
    pub fn refcount(&self) -> u64 {
        tier2_allocator().refcount(self.slot_id).unwrap_or(0)
    }

    /// Check if this is the only reference.
    pub fn is_unique(&self) -> bool {
        self.refcount() == 1
    }

    /// Create a weak reference to this pointer.
    pub fn downgrade(&self) -> WeakPersistentPtr<T> {
        tier2_allocator().increment_weak(self.slot_id);
        WeakPersistentPtr {
            slot_id: self.slot_id,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<T> Clone for PersistentPtr<T> {
    fn clone(&self) -> Self {
        tier2_allocator().increment(self.slot_id);
        Self {
            slot_id: self.slot_id,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<T> Drop for PersistentPtr<T> {
    fn drop(&mut self) {
        tier2_allocator().decrement(self.slot_id);
    }
}

impl<T: fmt::Debug> fmt::Debug for PersistentPtr<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PersistentPtr")
            .field("slot_id", &self.slot_id)
            .field("refcount", &self.refcount())
            .field("value", self.get())
            .finish()
    }
}

// Safety: PersistentPtr uses atomic refcounting and is thread-safe.
unsafe impl<T: Send> Send for PersistentPtr<T> {}
unsafe impl<T: Sync> Sync for PersistentPtr<T> {}

/// A weak reference to a Tier 2 allocation.
///
/// Weak references do not prevent deallocation. They must be upgraded
/// to a strong reference (`PersistentPtr`) before use.
pub struct WeakPersistentPtr<T> {
    /// Slot ID in the Tier 2 allocator.
    slot_id: Tier2SlotId,
    /// Phantom data for type safety.
    _marker: std::marker::PhantomData<T>,
}

impl<T> WeakPersistentPtr<T> {
    /// Try to upgrade to a strong reference.
    ///
    /// Returns `Some(PersistentPtr)` if the value is still alive,
    /// `None` if the value has been deallocated.
    pub fn upgrade(&self) -> Option<PersistentPtr<T>> {
        if tier2_allocator().try_upgrade_weak(self.slot_id) {
            Some(PersistentPtr {
                slot_id: self.slot_id,
                _marker: std::marker::PhantomData,
            })
        } else {
            None
        }
    }

    /// Get the slot ID.
    pub fn slot_id(&self) -> Tier2SlotId {
        self.slot_id
    }

    /// Check if the referenced value is still alive.
    pub fn is_alive(&self) -> bool {
        tier2_allocator().is_alive(self.slot_id)
    }

    /// Get the current strong reference count.
    pub fn strong_count(&self) -> u64 {
        tier2_allocator().refcount(self.slot_id).unwrap_or(0)
    }

    /// Get the current weak reference count.
    pub fn weak_count(&self) -> u32 {
        tier2_allocator().weak_count(self.slot_id).unwrap_or(0)
    }
}

impl<T> Clone for WeakPersistentPtr<T> {
    fn clone(&self) -> Self {
        tier2_allocator().increment_weak(self.slot_id);
        Self {
            slot_id: self.slot_id,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<T> Drop for WeakPersistentPtr<T> {
    fn drop(&mut self) {
        tier2_allocator().decrement_weak(self.slot_id);
    }
}

impl<T> fmt::Debug for WeakPersistentPtr<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WeakPersistentPtr")
            .field("slot_id", &self.slot_id)
            .field("is_alive", &self.is_alive())
            .field("strong_count", &self.strong_count())
            .field("weak_count", &self.weak_count())
            .finish()
    }
}

// Safety: WeakPersistentPtr uses atomic operations and is thread-safe.
unsafe impl<T: Send> Send for WeakPersistentPtr<T> {}
unsafe impl<T: Sync> Sync for WeakPersistentPtr<T> {}

/// Compute a type fingerprint for a type.
///
/// This is a simple hash of the type name for now.
fn type_fingerprint<T>() -> u32 {
    let name = std::any::type_name::<T>();
    let mut hash: u32 = 0;
    for byte in name.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(byte as u32);
    }
    hash
}

// ============================================================================
// Tier 2 Public API
// ============================================================================

/// Allocate a Tier 2 value.
///
/// Returns the slot ID and a pointer to the allocated memory.
pub fn tier2_alloc(size: usize, align: usize, type_fp: u32) -> Option<(Tier2SlotId, *mut u8)> {
    let id = tier2_allocator().allocate(size, align, type_fp)?;
    let ptr = unsafe { tier2_allocator().value_ptr(id)? };
    Some((id, ptr))
}

/// Promote a Tier 1 value to Tier 2.
///
/// This preserves the original generation for stale detection.
pub fn tier2_promote(
    value_ptr: *mut u8,
    size: usize,
    align: usize,
    type_fp: u32,
    original_generation: Generation,
) -> Option<Tier2SlotId> {
    tier2_allocator().promote_from_tier1(value_ptr, size, align, type_fp, original_generation)
}

/// Increment the reference count for a Tier 2 slot.
pub fn tier2_increment(id: Tier2SlotId) -> bool {
    tier2_allocator().increment(id)
}

/// Decrement the reference count for a Tier 2 slot.
pub fn tier2_decrement(id: Tier2SlotId) {
    tier2_allocator().decrement(id)
}

/// Get the reference count for a Tier 2 slot.
pub fn tier2_refcount(id: Tier2SlotId) -> Option<u64> {
    tier2_allocator().refcount(id)
}

/// Check if a Tier 2 slot is alive.
pub fn tier2_is_alive(id: Tier2SlotId) -> bool {
    tier2_allocator().is_alive(id)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blood_ptr_null() {
        let ptr = BloodPtr::null();
        assert!(ptr.is_null());
        assert_eq!(ptr.address(), 0);
        assert_eq!(ptr.generation(), generation::UNINITIALIZED);
    }

    #[test]
    fn test_blood_ptr_new() {
        let ptr = BloodPtr::new(0x1000, 42, PointerMetadata::HEAP);
        assert!(!ptr.is_null());
        assert_eq!(ptr.address(), 0x1000);
        assert_eq!(ptr.generation(), 42);
        assert!(ptr.is_heap());
        assert!(!ptr.is_stack());
    }

    #[test]
    fn test_pointer_metadata() {
        let meta = PointerMetadata::STACK.union(PointerMetadata::LINEAR);
        assert!(meta.contains(PointerMetadata::STACK));
        assert!(meta.contains(PointerMetadata::LINEAR));
        assert!(!meta.contains(PointerMetadata::HEAP));
    }

    #[test]
    fn test_slot_lifecycle() {
        let slot = Slot::new();
        assert_eq!(slot.generation(), generation::FIRST);
        assert!(!slot.is_occupied());

        unsafe {
            let layout = Layout::from_size_align(64, 8).unwrap();
            let ptr = slot.allocate(layout);
            assert!(!ptr.is_null());
            assert!(slot.is_occupied());

            slot.deallocate();
            assert!(!slot.is_occupied());
            assert_eq!(slot.generation(), generation::FIRST + 1);
        }
    }

    #[test]
    fn test_slot_validation() {
        let slot = Slot::new();
        assert!(slot.validate(generation::FIRST).is_ok());
        assert!(slot.validate(generation::FIRST + 1).is_err());
    }

    #[test]
    fn test_region_allocation() {
        let region = Region::new(1024, 4096);
        assert_eq!(region.used(), 0);

        let ptr1 = region.allocate(100, 8).unwrap();
        assert!(!ptr1.is_null());
        assert!(region.used() >= 100);

        let ptr2 = region.allocate(200, 8).unwrap();
        assert!(!ptr2.is_null());
        assert!(region.used() >= 300);
    }

    #[test]
    fn test_region_reset() {
        let mut region = Region::new(1024, 4096);
        region.allocate(500, 8).unwrap();
        assert!(region.used() >= 500);

        region.reset();
        assert_eq!(region.used(), 0);
    }

    #[test]
    fn test_region_close() {
        let region = Region::new(1024, 4096);
        assert!(!region.is_closed());

        region.close();
        assert!(region.is_closed());
        assert!(region.allocate(100, 8).is_none());
    }

    /// Test that pointers remain valid after region growth.
    ///
    /// Creates a region with a small initial size (4 KB), allocates to fill it,
    /// then allocates beyond the initial capacity to trigger growth. Verifies
    /// that pointers from before the growth are still valid (readable/writable).
    #[test]
    fn test_region_pointer_stability_across_growth() {
        // Small initial size to force growth; large max to allow it.
        let initial_size = 4096;
        let max_size = 1024 * 1024; // 1 MB
        let region = Region::new(initial_size, max_size);

        // Phase 1: fill the initial capacity with known data.
        let mut pre_growth_ptrs: Vec<(*mut u8, u8)> = Vec::new();
        let alloc_size = 256;
        let mut tag: u8 = 1;

        // Allocate enough to fill initial capacity.
        let num_initial_allocs = initial_size / alloc_size;
        for _ in 0..num_initial_allocs {
            let ptr = region.allocate(alloc_size, 8)
                .expect("allocation within initial capacity should succeed");
            // Write a tag pattern to verify later.
            unsafe {
                std::ptr::write_bytes(ptr, tag, alloc_size);
            }
            pre_growth_ptrs.push((ptr, tag));
            tag = tag.wrapping_add(1);
            if tag == 0 { tag = 1; }
        }

        // Phase 2: allocate beyond initial capacity to trigger growth.
        let post_growth_ptr = region.allocate(alloc_size, 8)
            .expect("allocation triggering growth should succeed");
        assert!(!post_growth_ptr.is_null());

        // Allocate more to exercise larger growth.
        for _ in 0..50 {
            let ptr = region.allocate(alloc_size, 8)
                .expect("post-growth allocation should succeed");
            assert!(!ptr.is_null());
        }

        // Phase 3: verify all pre-growth pointers are still valid.
        for (ptr, expected_tag) in &pre_growth_ptrs {
            unsafe {
                let slice = std::slice::from_raw_parts(*ptr, alloc_size);
                for (i, byte) in slice.iter().enumerate() {
                    assert_eq!(
                        *byte, *expected_tag,
                        "pre-growth pointer {:#?} byte {} was corrupted: expected {:#x}, got {:#x}",
                        *ptr, i, *expected_tag, *byte
                    );
                }
            }
        }

        // Verify total allocation reaches well past initial size.
        assert!(
            region.used() > initial_size,
            "region should have grown past initial size: used={}, initial={}",
            region.used(),
            initial_size
        );
    }

    /// Test that a region can allocate up to near its max size.
    #[test]
    fn test_region_grows_to_max_size() {
        let max_size = 64 * 1024; // 64 KB
        let region = Region::new(4096, max_size);

        // Fill the region to near capacity.
        let chunk = 1024;
        let mut total = 0;
        while total + chunk <= max_size {
            match region.allocate(chunk, 8) {
                Some(ptr) => {
                    assert!(!ptr.is_null());
                    // Use the actual bump offset which accounts for alignment padding.
                    total = region.used();
                }
                None => break,
            }
        }

        // Should have allocated a substantial portion of max_size.
        assert!(
            region.used() > max_size / 2,
            "expected to fill at least half of max_size, got {}",
            region.used()
        );
    }

    #[test]
    fn test_generation_snapshot() {
        let ptrs = vec![
            BloodPtr::new(0x1000, 1, PointerMetadata::HEAP),
            BloodPtr::new(0x2000, 2, PointerMetadata::HEAP),
            BloodPtr::null(),
        ];

        let snapshot = GenerationSnapshot::capture(&ptrs);
        assert_eq!(snapshot.len(), 2); // Null is excluded

        // Validate with matching generations
        let result = snapshot.validate(|addr| {
            match addr {
                0x1000 => Some(1),
                0x2000 => Some(2),
                _ => None,
            }
        });
        assert!(result.is_ok());

        // Validate with mismatched generation
        let result = snapshot.validate(|addr| {
            match addr {
                0x1000 => Some(1),
                0x2000 => Some(99), // Wrong generation
                _ => None,
            }
        });
        assert!(result.is_err());
    }

    #[test]
    fn test_stale_reference_error() {
        let err = StaleReferenceError {
            expected: 5,
            actual: 10,
        };
        let msg = err.to_string();
        assert!(msg.contains("5"));
        assert!(msg.contains("10"));
    }

    #[test]
    fn test_region_id() {
        let id1 = next_region_id();
        let id2 = next_region_id();
        assert_ne!(id1, id2);
    }

    // =========================================================================
    // Slot Registry Tests
    // =========================================================================

    #[test]
    fn test_slot_registry_register() {
        let registry = SlotRegistry::new();

        // Register a new allocation
        let gen = registry.register(0x1000, 64);
        assert_eq!(gen, generation::FIRST);
        assert!(registry.is_allocated(0x1000));

        // Registering at same address should increment generation
        let gen2 = registry.register(0x1000, 128);
        assert_eq!(gen2, generation::FIRST + 1);
    }

    #[test]
    fn test_slot_registry_unregister() {
        let registry = SlotRegistry::new();

        let gen = registry.register(0x2000, 32);
        assert!(registry.is_allocated(0x2000));

        let _ = registry.unregister(0x2000);
        assert!(!registry.is_allocated(0x2000));

        // Generation should have been incremented on deallocation
        let current_gen = registry.get_generation(0x2000);
        assert_eq!(current_gen, Some(gen + 1));
    }

    #[test]
    fn test_slot_registry_validate_success() {
        let registry = SlotRegistry::new();

        let gen = registry.register(0x3000, 16);
        let result = registry.validate(0x3000, gen);
        assert!(result.is_ok());
    }

    #[test]
    fn test_slot_registry_validate_stale() {
        let registry = SlotRegistry::new();

        let gen = registry.register(0x4000, 16);
        let _ = registry.unregister(0x4000);

        // Validation should fail because the slot was freed
        let result = registry.validate(0x4000, gen);
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert_eq!(err.expected, gen);
        assert_eq!(err.actual, gen + 1); // Generation was incremented on deallocation
    }

    #[test]
    fn test_slot_registry_validate_reallocation() {
        let registry = SlotRegistry::new();

        // First allocation
        let gen1 = registry.register(0x5000, 16);
        let _ = registry.unregister(0x5000);

        // Reallocate at same address
        let gen2 = registry.register(0x5000, 32);
        assert!(gen2 > gen1);

        // Validation with old generation should fail (ABA detection)
        let result = registry.validate(0x5000, gen1);
        assert!(result.is_err());

        // Validation with new generation should succeed
        let result = registry.validate(0x5000, gen2);
        assert!(result.is_ok());
    }

    #[test]
    fn test_slot_registry_persistent_skip() {
        let registry = SlotRegistry::new();

        // Persistent references should always validate successfully
        let result = registry.validate(0x6000, generation::PERSISTENT);
        assert!(result.is_ok());
    }

    #[test]
    fn test_slot_registry_unknown_address() {
        let registry = SlotRegistry::new();

        // Unregistered address with generation FIRST is OK (might be stack/static)
        let result = registry.validate(0x9999, generation::FIRST);
        assert!(result.is_ok());

        // Unregistered address with higher generation is likely freed heap
        let result = registry.validate(0x9999, generation::FIRST + 5);
        assert!(result.is_err());
    }

    #[test]
    fn test_slot_entry_lifecycle() {
        let mut entry = SlotEntry::new(generation::FIRST, 64);
        assert!(entry.is_allocated);
        assert_eq!(entry.generation, generation::FIRST);
        assert_eq!(entry.size, 64);

        let _ = entry.deallocate();
        assert!(!entry.is_allocated);
        assert_eq!(entry.generation, generation::FIRST + 1);
    }

    #[test]
    fn test_global_slot_registry_functions() {
        // These use the global registry - use unique addresses to avoid conflicts
        let addr = 0xDEAD_BEEF_1234u64;

        let gen = register_allocation(addr, 100);
        assert!(slot_registry().is_allocated(addr));

        let looked_up = get_slot_generation(addr);
        assert_eq!(looked_up, Some(gen));

        assert!(validate_generation(addr, gen).is_ok());

        let _ = unregister_allocation(addr);
        assert!(!slot_registry().is_allocated(addr));
    }

    // =========================================================================
    // Tier 3: Persistent Slot Tests
    // =========================================================================

    #[test]
    fn test_persistent_metadata() {
        let mut meta = PersistentMetadata::new(0x1234);
        assert_eq!(meta.type_fp, 0x1234);
        assert!(!meta.is_frozen());

        meta.set_frozen();
        assert!(meta.is_frozen());
    }

    #[test]
    fn test_persistent_slot_refcount() {
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { alloc::alloc(layout) };
        assert!(!ptr.is_null());

        let slot = PersistentSlot::new(ptr, 64, layout, 0);
        assert_eq!(slot.refcount(), 1);
        assert!(slot.is_alive());

        slot.increment();
        assert_eq!(slot.refcount(), 2);

        assert!(!slot.decrement()); // Not last ref
        assert_eq!(slot.refcount(), 1);

        assert!(slot.decrement()); // Last ref
        assert_eq!(slot.refcount(), 0);
        assert!(!slot.is_alive());

        // Clean up
        unsafe { slot.deallocate_value() };
    }

    #[test]
    fn test_persistent_slot_weak() {
        let layout = Layout::from_size_align(32, 8).unwrap();
        let ptr = unsafe { alloc::alloc(layout) };
        assert!(!ptr.is_null());

        let slot = PersistentSlot::new(ptr, 32, layout, 0);
        assert_eq!(slot.weak_count(), 0);

        slot.increment_weak();
        assert_eq!(slot.weak_count(), 1);

        // Try upgrade while alive
        assert!(slot.try_upgrade_weak());
        assert_eq!(slot.refcount(), 2);

        // Clean up - decrement strong refs
        slot.decrement(); // refcount = 1
        slot.decrement(); // refcount = 0, last strong ref

        // Decrement weak - returns true because both weak_count becomes 0 and refcount is 0
        assert!(slot.decrement_weak());

        unsafe { slot.deallocate_value() };
    }

    #[test]
    fn test_persistent_alloc_decrement() {
        let result = persistent_alloc(128, 8, 0xABCD);
        assert!(result.is_some());

        let (id, ptr) = result.unwrap();
        assert!(!ptr.is_null());
        assert!(persistent_is_alive(id));
        assert_eq!(persistent_refcount(id), Some(1));

        persistent_decrement(id);
        // After decrement, slot may be queued for cleanup
    }

    #[test]
    fn test_persistent_increment_decrement() {
        let result = persistent_alloc(64, 8, 0);
        let (id, _) = result.unwrap();

        assert!(persistent_increment(id));
        assert_eq!(persistent_refcount(id), Some(2));

        persistent_decrement(id);
        assert_eq!(persistent_refcount(id), Some(1));

        persistent_decrement(id);
    }

    #[test]
    fn test_deferred_queue() {
        let queue = DeferredDecrementQueue::new(10);
        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn test_cycle_collector_not_reentrant() {
        let collector = CycleCollector::new();
        assert!(!collector.is_collecting());

        // First collection should succeed (count depends on global allocator state)
        let _count1 = collector.collect(&[]);
        // After collection completes, the collecting flag should be cleared
        assert!(!collector.is_collecting());
    }

    #[test]
    fn test_type_registry() {
        let registry = TypeRegistry::new();
        assert!(registry.is_empty());

        // Register a type with slot offsets
        let layout = TypeLayout::with_slots(24, vec![0, 8, 16]);
        registry.register(0xABCD, layout.clone());

        assert!(registry.contains(0xABCD));
        assert!(!registry.contains(0x1234));

        let retrieved = registry.get(0xABCD).unwrap();
        assert_eq!(retrieved.size, 24);
        assert_eq!(retrieved.slot_offsets, vec![0, 8, 16]);
    }

    #[test]
    fn test_cycle_collector_threshold() {
        let collector = CycleCollector::new();

        // Default threshold
        let default_threshold = collector.threshold();
        assert!(default_threshold > 0);

        // Set new threshold
        collector.set_threshold(1024);
        assert_eq!(collector.threshold(), 1024);
    }

    #[test]
    fn test_type_layout() {
        let mut layout = TypeLayout::new(16);
        assert_eq!(layout.size, 16);
        assert!(layout.slot_offsets.is_empty());

        layout.add_slot_offset(0);
        layout.add_slot_offset(8);
        assert_eq!(layout.slot_offsets, vec![0, 8]);
    }

    #[test]
    fn test_memory_tier_enum() {
        assert_eq!(MemoryTier::Stack, MemoryTier::Stack);
        assert_ne!(MemoryTier::Stack, MemoryTier::Region);
        assert_ne!(MemoryTier::Region, MemoryTier::Heap);
        assert_ne!(MemoryTier::Heap, MemoryTier::Persistent);
    }

    // =========================================================================
    // Tier 2: Reference-Counted Allocator Tests
    // =========================================================================

    #[test]
    fn test_tier2_slot_id() {
        let id = Tier2SlotId(42);
        assert_eq!(id.as_u64(), 42);
        assert_eq!(Tier2SlotId::from_u64(42), id);
    }

    #[test]
    fn test_tier2_slot_basic() {
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { alloc::alloc(layout) };
        assert!(!ptr.is_null());

        let slot = Tier2Slot::new(ptr, 64, layout, 0x1234, generation::FIRST);
        assert_eq!(slot.refcount(), 1);
        assert_eq!(slot.weak_count(), 0);
        assert!(slot.is_alive());
        assert_eq!(slot.size(), 64);
        assert_eq!(slot.type_fp(), 0x1234);
        assert_eq!(slot.original_generation(), generation::FIRST);
        assert!(!slot.is_promoted());
        assert!(!slot.is_frozen());

        unsafe { slot.deallocate_value() };
    }

    #[test]
    fn test_tier2_slot_promoted() {
        let layout = Layout::from_size_align(32, 8).unwrap();
        let ptr = unsafe { alloc::alloc(layout) };
        assert!(!ptr.is_null());

        let original_gen = 42u32;
        let slot = Tier2Slot::promoted(ptr, 32, layout, 0xABCD, original_gen);

        assert_eq!(slot.refcount(), 1);
        assert!(slot.is_alive());
        assert!(slot.is_promoted());
        assert_eq!(slot.original_generation(), original_gen);
        assert_eq!(slot.type_fp(), 0xABCD);

        unsafe { slot.deallocate_value() };
    }

    #[test]
    fn test_tier2_slot_refcount_operations() {
        let layout = Layout::from_size_align(16, 8).unwrap();
        let ptr = unsafe { alloc::alloc(layout) };
        assert!(!ptr.is_null());

        let slot = Tier2Slot::new(ptr, 16, layout, 0, generation::FIRST);
        assert_eq!(slot.refcount(), 1);

        // Increment
        slot.increment();
        assert_eq!(slot.refcount(), 2);

        slot.increment();
        assert_eq!(slot.refcount(), 3);

        // Decrement (not last)
        assert!(!slot.decrement());
        assert_eq!(slot.refcount(), 2);

        assert!(!slot.decrement());
        assert_eq!(slot.refcount(), 1);

        // Decrement (last)
        assert!(slot.decrement());
        assert_eq!(slot.refcount(), 0);
        assert!(!slot.is_alive());

        unsafe { slot.deallocate_value() };
    }

    #[test]
    fn test_tier2_slot_weak_references() {
        let layout = Layout::from_size_align(16, 8).unwrap();
        let ptr = unsafe { alloc::alloc(layout) };
        assert!(!ptr.is_null());

        let slot = Tier2Slot::new(ptr, 16, layout, 0, generation::FIRST);
        assert_eq!(slot.weak_count(), 0);

        // Add weak references
        slot.increment_weak();
        assert_eq!(slot.weak_count(), 1);

        slot.increment_weak();
        assert_eq!(slot.weak_count(), 2);

        // Upgrade weak to strong (while alive)
        assert!(slot.try_upgrade_weak());
        assert_eq!(slot.refcount(), 2);

        // Decrement weak (not last)
        assert!(!slot.decrement_weak());
        assert_eq!(slot.weak_count(), 1);

        // Drop strong refs
        slot.decrement(); // refcount = 1
        slot.decrement(); // refcount = 0

        // Try upgrade after dead - should fail
        assert!(!slot.try_upgrade_weak());

        // Decrement last weak - should return true (both weak and strong are 0)
        assert!(slot.decrement_weak());

        unsafe { slot.deallocate_value() };
    }

    #[test]
    fn test_tier2_slot_frozen() {
        let layout = Layout::from_size_align(8, 8).unwrap();
        let ptr = unsafe { alloc::alloc(layout) };
        assert!(!ptr.is_null());

        let slot = Tier2Slot::new(ptr, 8, layout, 0, generation::FIRST);
        assert!(!slot.is_frozen());

        slot.set_frozen();
        assert!(slot.is_frozen());

        // Clean up
        slot.decrement();
        unsafe { slot.deallocate_value() };
    }

    #[test]
    fn test_tier2_allocator_basic() {
        let allocator = Tier2Allocator::new();
        assert_eq!(allocator.live_count(), 0);

        // Allocate
        let id = allocator.allocate(64, 8, 0x1234).unwrap();
        assert!(allocator.is_alive(id));
        assert_eq!(allocator.refcount(id), Some(1));
        assert_eq!(allocator.live_count(), 1);

        // Increment
        assert!(allocator.increment(id));
        assert_eq!(allocator.refcount(id), Some(2));

        // Decrement (not last)
        allocator.decrement(id);
        assert_eq!(allocator.refcount(id), Some(1));

        // Decrement (last) - slot queued for cleanup
        allocator.decrement(id);
    }

    #[test]
    fn test_tier2_allocator_promotion() {
        let allocator = Tier2Allocator::new();

        // Allocate Tier 1 style memory
        let layout = Layout::from_size_align(32, 8).unwrap();
        let ptr = unsafe { alloc::alloc(layout) };
        assert!(!ptr.is_null());

        let original_gen = 100u32;
        let id = allocator.promote_from_tier1(ptr, 32, 8, 0xABCD, original_gen).unwrap();

        assert!(allocator.is_alive(id));
        assert!(allocator.is_promoted(id));
        assert_eq!(allocator.original_generation(id), Some(original_gen));
        assert_eq!(allocator.stats().promotions.load(Ordering::Relaxed), 1);

        // Clean up
        allocator.decrement(id);
    }

    #[test]
    fn test_tier2_allocator_weak_references() {
        let allocator = Tier2Allocator::new();

        let id = allocator.allocate(16, 8, 0).unwrap();
        assert_eq!(allocator.weak_count(id), Some(0));

        // Add weak reference
        assert!(allocator.increment_weak(id));
        assert_eq!(allocator.weak_count(id), Some(1));

        // Upgrade weak to strong
        assert!(allocator.try_upgrade_weak(id));
        assert_eq!(allocator.refcount(id), Some(2));
        assert_eq!(allocator.stats().weak_upgrades.load(Ordering::Relaxed), 1);

        // Clean up
        allocator.decrement(id);
        allocator.decrement(id);
        allocator.decrement_weak(id);
    }

    #[test]
    fn test_tier2_allocator_null_promotion() {
        let allocator = Tier2Allocator::new();

        // Promoting null should fail
        let result = allocator.promote_from_tier1(std::ptr::null_mut(), 32, 8, 0, generation::FIRST);
        assert!(result.is_none());
    }

    #[test]
    fn test_tier2_public_api() {
        // Test tier2_alloc
        let result = tier2_alloc(128, 8, 0xDEAD);
        assert!(result.is_some());
        let (id, ptr) = result.unwrap();
        assert!(!ptr.is_null());
        assert!(tier2_is_alive(id));
        assert_eq!(tier2_refcount(id), Some(1));

        // Test tier2_increment
        assert!(tier2_increment(id));
        assert_eq!(tier2_refcount(id), Some(2));

        // Test tier2_decrement
        tier2_decrement(id);
        assert_eq!(tier2_refcount(id), Some(1));

        tier2_decrement(id);
    }

    #[test]
    fn test_tier2_promote_api() {
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { alloc::alloc(layout) };
        assert!(!ptr.is_null());

        let original_gen = 50u32;
        let result = tier2_promote(ptr, 64, 8, 0xBEEF, original_gen);
        assert!(result.is_some());

        let id = result.unwrap();
        assert!(tier2_is_alive(id));
        assert!(tier2_allocator().is_promoted(id));
        assert_eq!(tier2_allocator().original_generation(id), Some(original_gen));

        tier2_decrement(id);
    }

    // =========================================================================
    // PersistentPtr Tests
    // =========================================================================

    #[test]
    fn test_persistent_ptr_new() {
        let ptr = PersistentPtr::new(42i32).unwrap();
        assert_eq!(*ptr.get(), 42);
        assert_eq!(ptr.refcount(), 1);
        assert!(ptr.is_unique());
    }

    #[test]
    fn test_persistent_ptr_clone() {
        let ptr1 = PersistentPtr::new(100i32).unwrap();
        assert_eq!(ptr1.refcount(), 1);

        let ptr2 = ptr1.clone();
        assert_eq!(ptr1.refcount(), 2);
        assert_eq!(ptr2.refcount(), 2);
        assert!(!ptr1.is_unique());
        assert!(!ptr2.is_unique());

        assert_eq!(*ptr1.get(), 100);
        assert_eq!(*ptr2.get(), 100);
    }

    #[test]
    fn test_persistent_ptr_drop() {
        let ptr1 = PersistentPtr::new(200i32).unwrap();
        let slot_id = ptr1.slot_id();

        {
            let ptr2 = ptr1.clone();
            assert_eq!(tier2_refcount(slot_id), Some(2));
            drop(ptr2);
        }

        assert_eq!(tier2_refcount(slot_id), Some(1));
        drop(ptr1);
        // After drop, slot may be queued for cleanup
    }

    #[test]
    fn test_persistent_ptr_get_mut() {
        let mut ptr = PersistentPtr::new(10i32).unwrap();
        assert_eq!(*ptr.get(), 10);

        *ptr.get_mut() = 20;
        assert_eq!(*ptr.get(), 20);
    }

    #[test]
    fn test_persistent_ptr_slot_id() {
        let ptr = PersistentPtr::new(0i32).unwrap();
        let slot_id = ptr.slot_id();
        assert!(tier2_is_alive(slot_id));
    }

    #[test]
    fn test_persistent_ptr_with_struct() {
        #[derive(Debug, PartialEq)]
        struct Point {
            x: i32,
            y: i32,
        }

        let ptr = PersistentPtr::new(Point { x: 10, y: 20 }).unwrap();
        assert_eq!(ptr.get().x, 10);
        assert_eq!(ptr.get().y, 20);
    }

    // =========================================================================
    // WeakPersistentPtr Tests
    // =========================================================================

    #[test]
    fn test_weak_persistent_ptr_downgrade() {
        let ptr = PersistentPtr::new(42i32).unwrap();
        let weak = ptr.downgrade();

        assert!(weak.is_alive());
        assert_eq!(weak.strong_count(), 1);
        assert_eq!(weak.weak_count(), 1);
    }

    #[test]
    fn test_weak_persistent_ptr_upgrade() {
        let ptr = PersistentPtr::new(100i32).unwrap();
        let weak = ptr.downgrade();

        // Upgrade while strong ref exists
        let upgraded = weak.upgrade().unwrap();
        assert_eq!(*upgraded.get(), 100);
        assert_eq!(upgraded.refcount(), 2);
    }

    #[test]
    fn test_weak_persistent_ptr_upgrade_after_drop() {
        let weak = {
            let ptr = PersistentPtr::new(50i32).unwrap();
            ptr.downgrade()
        };

        // Strong ref dropped, upgrade should fail
        assert!(!weak.is_alive());
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn test_weak_persistent_ptr_clone() {
        let ptr = PersistentPtr::new(0i32).unwrap();
        let weak1 = ptr.downgrade();
        let weak2 = weak1.clone();

        assert_eq!(weak1.weak_count(), 2);
        assert_eq!(weak2.weak_count(), 2);
        assert_eq!(weak1.slot_id(), weak2.slot_id());
    }

    // =========================================================================
    // Thread Safety Tests
    // =========================================================================

    #[test]
    fn test_tier2_slot_thread_safe_refcount() {
        use std::sync::Arc;
        use std::thread;

        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { alloc::alloc(layout) };
        let slot = Arc::new(Tier2Slot::new(ptr, 64, layout, 0, generation::FIRST));

        // Increment in initial thread
        slot.increment();
        slot.increment();
        assert_eq!(slot.refcount(), 3);

        let handles: Vec<_> = (0..4).map(|_| {
            let slot_clone = Arc::clone(&slot);
            thread::spawn(move || {
                for _ in 0..100 {
                    slot_clone.increment();
                    slot_clone.decrement();
                }
            })
        }).collect();

        for h in handles {
            h.join().unwrap();
        }

        // Should still have original refcount
        assert_eq!(slot.refcount(), 3);

        // Clean up
        slot.decrement();
        slot.decrement();
        slot.decrement();
        unsafe { slot.deallocate_value() };
    }

    #[test]
    fn test_persistent_ptr_send_sync() {
        use std::sync::Arc;
        use std::thread;

        let ptr = Arc::new(PersistentPtr::new(42i32).unwrap());

        let handles: Vec<_> = (0..4).map(|_| {
            let ptr_clone = Arc::clone(&ptr);
            thread::spawn(move || {
                // Read value from different threads
                assert_eq!(*ptr_clone.get(), 42);
            })
        }).collect();

        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn test_tier2_allocator_concurrent_access() {
        use std::sync::Arc;
        use std::thread;

        let allocator = Arc::new(Tier2Allocator::new());

        let handles: Vec<_> = (0..4).map(|i| {
            let alloc = Arc::clone(&allocator);
            thread::spawn(move || {
                // Each thread allocates and deallocates its own slots
                for j in 0..10 {
                    let id = alloc.allocate(16, 8, (i * 100 + j) as u32).unwrap();
                    assert!(alloc.is_alive(id));
                    alloc.decrement(id);
                }
            })
        }).collect();

        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn test_type_fingerprint() {
        // Different types should have different fingerprints
        let fp_i32 = type_fingerprint::<i32>();
        let fp_i64 = type_fingerprint::<i64>();
        let fp_string = type_fingerprint::<String>();

        assert_ne!(fp_i32, fp_i64);
        assert_ne!(fp_i32, fp_string);
        assert_ne!(fp_i64, fp_string);

        // Same type should have same fingerprint
        assert_eq!(type_fingerprint::<i32>(), type_fingerprint::<i32>());
    }

    #[test]
    fn test_tier2_stats() {
        let allocator = Tier2Allocator::new();

        assert_eq!(allocator.stats().allocations.load(Ordering::Relaxed), 0);

        let id = allocator.allocate(32, 8, 0).unwrap();
        assert_eq!(allocator.stats().allocations.load(Ordering::Relaxed), 1);
        assert_eq!(allocator.stats().live_slots.load(Ordering::Relaxed), 1);

        allocator.decrement(id);
        assert_eq!(allocator.stats().deallocations.load(Ordering::Relaxed), 1);
    }

    // ========================================================================
    // Slab Allocator Tests
    // ========================================================================

    #[test]
    fn test_size_class_for() {
        // Test boundary cases for size class mapping
        assert_eq!(size_class_for(0), 0);   // 0 -> 8 bytes (class 0)
        assert_eq!(size_class_for(1), 0);   // 1 -> 8 bytes (class 0)
        assert_eq!(size_class_for(8), 0);   // 8 -> 8 bytes (class 0)
        assert_eq!(size_class_for(9), 1);   // 9 -> 16 bytes (class 1)
        assert_eq!(size_class_for(16), 1);  // 16 -> 16 bytes (class 1)
        assert_eq!(size_class_for(17), 2);  // 17 -> 32 bytes (class 2)
        assert_eq!(size_class_for(32), 2);  // 32 -> 32 bytes (class 2)
        assert_eq!(size_class_for(64), 3);  // 64 -> 64 bytes (class 3)
        assert_eq!(size_class_for(128), 4); // 128 -> 128 bytes (class 4)
        assert_eq!(size_class_for(256), 5); // 256 -> 256 bytes (class 5)
        assert_eq!(size_class_for(512), 6); // 512 -> 512 bytes (class 6)
        assert_eq!(size_class_for(1024), 7);  // 1KB -> class 7
        assert_eq!(size_class_for(2048), 8);  // 2KB -> class 8
        assert_eq!(size_class_for(4096), 9);  // 4KB -> class 9
        assert_eq!(size_class_for(8192), 10); // 8KB -> class 10
        assert_eq!(size_class_for(16384), 11); // 16KB -> class 11
        assert_eq!(size_class_for(16385), SIZE_CLASS_LARGE); // >16KB -> large
        assert_eq!(size_class_for(100000), SIZE_CLASS_LARGE); // Large
    }

    #[test]
    fn test_slot_size_for_class() {
        // Test all valid size classes
        assert_eq!(slot_size_for_class(0), 8);
        assert_eq!(slot_size_for_class(1), 16);
        assert_eq!(slot_size_for_class(2), 32);
        assert_eq!(slot_size_for_class(3), 64);
        assert_eq!(slot_size_for_class(4), 128);
        assert_eq!(slot_size_for_class(5), 256);
        assert_eq!(slot_size_for_class(6), 512);
        assert_eq!(slot_size_for_class(7), 1024);
        assert_eq!(slot_size_for_class(8), 2048);
        assert_eq!(slot_size_for_class(9), 4096);
        assert_eq!(slot_size_for_class(10), 8192);
        assert_eq!(slot_size_for_class(11), 16384);
        // Large and invalid classes return 0
        assert_eq!(slot_size_for_class(SIZE_CLASS_LARGE), 0);
        assert_eq!(slot_size_for_class(12), 0);
        assert_eq!(slot_size_for_class(100), 0);
    }

    #[test]
    fn test_size_class_free_list_operations() {
        let mut list = SizeClassFreeList::new();

        // Initially empty
        assert!(list.pop().is_none());
        assert_eq!(list.len(), 0);
        assert!(list.is_empty());

        // Push some addresses
        list.push(0x1000);
        list.push(0x2000);
        list.push(0x3000);

        assert_eq!(list.len(), 3);
        assert!(!list.is_empty());

        // Pop returns LIFO order
        assert_eq!(list.pop(), Some(0x3000));
        assert_eq!(list.pop(), Some(0x2000));
        assert_eq!(list.pop(), Some(0x1000));
        assert!(list.pop().is_none());

        // Reuse count is tracked
        assert_eq!(list.reuse_count(), 3);
    }

    #[test]
    fn test_region_stats_tracking() {
        let stats = RegionStats::new();

        // Initially all zeros
        assert_eq!(stats.allocations(), 0);
        assert_eq!(stats.reused(), 0);
        assert_eq!(stats.bumped(), 0);
        assert_eq!(stats.deallocations(), 0);

        // Increment counters
        stats.allocations.fetch_add(5, Ordering::Relaxed);
        stats.reused.fetch_add(2, Ordering::Relaxed);
        stats.bumped.fetch_add(3, Ordering::Relaxed);
        stats.deallocations.fetch_add(1, Ordering::Relaxed);

        assert_eq!(stats.allocations(), 5);
        assert_eq!(stats.reused(), 2);
        assert_eq!(stats.bumped(), 3);
        assert_eq!(stats.deallocations(), 1);
    }

    #[test]
    fn test_region_slab_allocation_and_reuse() {
        let region = Region::new(16384, 65536);

        // Allocate a small object (8 bytes -> size class 0)
        let ptr1 = region.allocate(8, 8).unwrap();
        assert!(!ptr1.is_null());

        // Stats should show 1 allocation, 0 reused, 1 bumped
        assert_eq!(region.stats().allocations(), 1);
        assert_eq!(region.stats().reused(), 0);
        assert_eq!(region.stats().bumped(), 1);

        // Deallocate it
        let _ = region.deallocate(ptr1 as u64, 0);
        assert_eq!(region.stats().deallocations(), 1);

        // Allocate same size again - should reuse
        let ptr2 = region.allocate(8, 8).unwrap();
        assert_eq!(ptr2 as u64, ptr1 as u64); // Same address reused
        assert_eq!(region.stats().allocations(), 2);
        assert_eq!(region.stats().reused(), 1);
        assert_eq!(region.stats().bumped(), 1); // Still 1 bump
    }

    #[test]
    fn test_region_large_allocation_no_reuse() {
        let region = Region::new(65536, 262144);

        // Allocate a large object (>16KB)
        let ptr = region.allocate(20000, 8).unwrap();
        assert!(!ptr.is_null());

        // Large allocations tracked
        assert_eq!(region.stats().large_allocs.load(Ordering::Relaxed), 1);

        // Deallocate it - large allocations don't go to free list
        let result = region.deallocate(ptr as u64, SIZE_CLASS_LARGE);
        assert!(!result); // Returns false for large allocations

        // Stats show deallocation counted
        assert_eq!(region.stats().deallocations(), 1);
    }

    #[test]
    fn test_region_multiple_size_classes() {
        let region = Region::new(32768, 131072);

        // Allocate objects of different sizes
        let ptr_8 = region.allocate(8, 8).unwrap();
        let ptr_64 = region.allocate(64, 8).unwrap();
        let ptr_256 = region.allocate(256, 8).unwrap();

        assert_eq!(region.stats().allocations(), 3);
        assert_eq!(region.stats().bumped(), 3);

        // Deallocate to different free lists
        let _ = region.deallocate(ptr_8 as u64, 0);    // Class 0 (8 bytes)
        let _ = region.deallocate(ptr_64 as u64, 3);   // Class 3 (64 bytes)
        let _ = region.deallocate(ptr_256 as u64, 5);  // Class 5 (256 bytes)

        // Allocate again - should reuse from correct size classes
        let ptr_8_new = region.allocate(8, 8).unwrap();
        let ptr_64_new = region.allocate(64, 8).unwrap();

        assert_eq!(ptr_8_new as u64, ptr_8 as u64);   // Same addresses reused
        assert_eq!(ptr_64_new as u64, ptr_64 as u64);

        assert_eq!(region.stats().reused(), 2);
    }

    #[test]
    fn test_slot_entry_with_region_tracking() {
        // Test SlotEntry creation with region info
        let entry = SlotEntry::new_in_region(generation::FIRST, 64, 123);

        assert_eq!(entry.generation, generation::FIRST);
        assert_eq!(entry.size, 64);
        assert!(entry.is_allocated);
        assert_eq!(entry.size_class, 3); // 64 bytes -> class 3
        assert_eq!(entry.region_id, 123);

        // Test without region (region_id = 0)
        let entry2 = SlotEntry::new(generation::FIRST, 256);
        assert_eq!(entry2.size_class, 5); // 256 bytes -> class 5
        assert_eq!(entry2.region_id, 0);
    }

    #[test]
    fn test_slot_registry_with_region() {
        let registry = SlotRegistry::new();

        // Register with region
        let addr = 0xABCD1234u64;
        let gen = registry.register_with_region(addr, 128, 42);
        assert_eq!(gen, generation::FIRST);

        // Verify info is correct
        let info = registry.get_info(addr);
        assert!(info.is_some());
        let (size, region_id) = info.unwrap();
        assert_eq!(size, 128);
        assert_eq!(region_id, 42);

        // Unregister and verify region_id is returned
        let result = registry.unregister(addr);
        assert!(result.is_some());
        let (ret_region, ret_class) = result.unwrap();
        assert_eq!(ret_region, 42);
        assert_eq!(ret_class, 4); // 128 bytes -> class 4
    }

    #[test]
    fn test_slot_registry_get_info_not_found() {
        let registry = SlotRegistry::new();

        // Query non-existent address
        let info = registry.get_info(0xDEADBEEFu64);
        assert!(info.is_none());
    }

    #[test]
    fn test_public_api_with_region() {
        // Test the public API functions
        let addr = 0xFEEDFACEu64;

        // Register with region
        let gen = register_allocation_with_region(addr, 64, 999);
        assert!(gen >= generation::FIRST);

        // Get info
        let info = get_slot_info(addr);
        assert!(info.is_some());
        let (size, region_id) = info.unwrap();
        assert_eq!(size, 64);
        assert_eq!(region_id, 999);

        // Unregister
        let result = unregister_allocation(addr);
        assert!(result.is_some());
        let (ret_region, _) = result.unwrap();
        assert_eq!(ret_region, 999);
    }
}
