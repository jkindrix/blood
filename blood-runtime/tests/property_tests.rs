//! Property-based tests for the Blood runtime.
//!
//! Uses proptest to generate random inputs and verify invariants hold.

use blood_runtime::memory::{BloodPtr, GenerationSnapshot, PointerMetadata, generation, Slot, Generation};
use proptest::prelude::*;

/// Strategy for generating valid generations (non-zero, non-reserved)
fn valid_generation() -> impl Strategy<Value = Generation> {
    (generation::FIRST..generation::OVERFLOW_GUARD)
}

/// Strategy for generating valid addresses (page-aligned, non-null)
fn valid_address() -> impl Strategy<Value = usize> {
    (1usize..=0x1000).prop_map(|n| n * 4096)
}

/// Strategy for generating PointerMetadata
fn pointer_metadata() -> impl Strategy<Value = PointerMetadata> {
    prop_oneof![
        Just(PointerMetadata::NONE),
        Just(PointerMetadata::HEAP),
        Just(PointerMetadata::STACK),
        Just(PointerMetadata::LINEAR),
        Just(PointerMetadata::HEAP.union(PointerMetadata::LINEAR)),
        Just(PointerMetadata::STACK.union(PointerMetadata::LINEAR)),
    ]
}

/// Strategy for generating BloodPtr
fn blood_ptr() -> impl Strategy<Value = BloodPtr> {
    (valid_address(), valid_generation(), pointer_metadata())
        .prop_map(|(addr, gen, meta)| BloodPtr::new(addr, gen, meta))
}

proptest! {
    /// BloodPtr roundtrip: address and generation are preserved
    #[test]
    fn blood_ptr_roundtrip(addr in valid_address(), gen in valid_generation(), meta in pointer_metadata()) {
        let ptr = BloodPtr::new(addr, gen, meta);
        prop_assert_eq!(ptr.address(), addr);
        prop_assert_eq!(ptr.generation(), gen);
    }

    /// BloodPtr null detection is consistent
    #[test]
    fn blood_ptr_null_consistency(ptr in blood_ptr()) {
        // Non-null pointers created with valid addresses should not be null
        prop_assert!(!ptr.is_null());
    }

    /// Null pointer is always null
    #[test]
    fn null_ptr_is_null(_seed in 0u32..1000) {
        let null = BloodPtr::null();
        prop_assert!(null.is_null());
        prop_assert_eq!(null.address(), 0);
    }

    /// GenerationSnapshot capture preserves all pointers
    #[test]
    fn snapshot_capture_preserves_pointers(ptrs in prop::collection::vec(blood_ptr(), 0..50)) {
        // Filter to unique addresses (snapshot stores only one generation per address)
        let mut seen_addrs = std::collections::HashSet::new();
        let unique_ptrs: Vec<_> = ptrs.iter()
            .filter(|p| !p.is_null() && seen_addrs.insert(p.address()))
            .cloned()
            .collect();

        let snapshot = GenerationSnapshot::capture(&unique_ptrs);

        // Build a proper lookup map
        let lookup: std::collections::HashMap<usize, Generation> = unique_ptrs.iter()
            .map(|p| (p.address(), p.generation()))
            .collect();

        // Validate against the full lookup
        let valid = snapshot.validate(|addr| lookup.get(&addr).copied());
        prop_assert!(valid.is_ok());

        // Also verify the snapshot has the expected number of entries
        prop_assert_eq!(snapshot.entries().len(), unique_ptrs.len());
    }

    /// GenerationSnapshot validates correctly when all generations match
    #[test]
    fn snapshot_validates_matching_generations(ptrs in prop::collection::vec(blood_ptr(), 1..20)) {
        // Filter to unique addresses (snapshot stores only one generation per address)
        let mut seen_addrs = std::collections::HashSet::new();
        let unique_ptrs: Vec<_> = ptrs.iter()
            .filter(|p| !p.is_null() && seen_addrs.insert(p.address()))
            .cloned()
            .collect();

        if unique_ptrs.is_empty() {
            return Ok(());
        }

        let snapshot = GenerationSnapshot::capture(&unique_ptrs);

        // Build a proper lookup map
        let lookup: std::collections::HashMap<usize, Generation> = unique_ptrs.iter()
            .map(|p| (p.address(), p.generation()))
            .collect();

        // Validate against the full lookup
        let valid = snapshot.validate(|addr| lookup.get(&addr).copied());
        prop_assert!(valid.is_ok());
    }

    /// GenerationSnapshot detects stale references
    #[test]
    fn snapshot_detects_stale_reference(
        ptrs in prop::collection::vec(blood_ptr(), 2..10),
        stale_idx in 0usize..10
    ) {
        // Filter to unique addresses
        let mut seen_addrs = std::collections::HashSet::new();
        let unique_ptrs: Vec<_> = ptrs.iter()
            .filter(|p| !p.is_null() && seen_addrs.insert(p.address()))
            .cloned()
            .collect();

        if unique_ptrs.len() < 2 {
            return Ok(());
        }

        let stale_idx = stale_idx % unique_ptrs.len();
        let stale_ptr = &unique_ptrs[stale_idx];
        let snapshot = GenerationSnapshot::capture(&unique_ptrs);

        // Build lookup with one wrong generation
        let mut lookup: std::collections::HashMap<usize, Generation> = unique_ptrs.iter()
            .map(|p| (p.address(), p.generation()))
            .collect();

        // Corrupt the stale entry
        lookup.insert(stale_ptr.address(), stale_ptr.generation().wrapping_add(1));

        // Validate - should fail due to stale reference
        let valid = snapshot.validate(|addr| lookup.get(&addr).copied());

        // Should detect the stale reference
        prop_assert!(valid.is_err(), "Should detect stale reference at index {}", stale_idx);
    }

    /// Empty snapshot always validates
    #[test]
    fn empty_snapshot_always_valid(_seed in 0u32..1000) {
        let snapshot = GenerationSnapshot::new();
        let valid = snapshot.validate(|_| None);
        prop_assert!(valid.is_ok());
    }

    /// Slot allocation produces valid generations
    #[test]
    fn slot_allocation_valid_generation(_seed in 0u32..100) {
        let slot = Slot::new();
        let gen = slot.generation();

        // Generation should be valid (not reserved)
        prop_assert!(gen >= generation::FIRST);
        prop_assert!(gen < generation::OVERFLOW_GUARD);
    }

    /// Slot validate succeeds with matching generation
    #[test]
    fn slot_validate_matching(_seed in 0u32..100) {
        let slot = Slot::new();
        let gen = slot.generation();
        prop_assert!(slot.validate(gen).is_ok());
    }

    /// Slot validate fails with mismatched generation
    #[test]
    fn slot_validate_mismatched(gen in valid_generation()) {
        let slot = Slot::new();
        let slot_gen = slot.generation();

        // If the random generation happens to match, skip
        if gen == slot_gen {
            return Ok(());
        }

        prop_assert!(slot.validate(gen).is_err());
    }

    /// PointerMetadata union is associative
    #[test]
    fn metadata_union_associative(
        a in pointer_metadata(),
        b in pointer_metadata(),
        c in pointer_metadata()
    ) {
        let ab_c = a.union(b).union(c);
        let a_bc = a.union(b.union(c));
        prop_assert_eq!(ab_c.bits(), a_bc.bits());
    }

    /// PointerMetadata union is commutative
    #[test]
    fn metadata_union_commutative(a in pointer_metadata(), b in pointer_metadata()) {
        let ab = a.union(b);
        let ba = b.union(a);
        prop_assert_eq!(ab.bits(), ba.bits());
    }

    /// PointerMetadata contains is reflexive after union
    #[test]
    fn metadata_contains_after_union(a in pointer_metadata(), b in pointer_metadata()) {
        let ab = a.union(b);
        prop_assert!(ab.contains(a));
        prop_assert!(ab.contains(b));
    }
}

#[cfg(test)]
mod stress_tests {
    use super::*;
    use std::sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}};
    use std::thread;

    /// Stress test for concurrent slot access
    #[test]
    fn stress_concurrent_slot_validation() {
        const NUM_THREADS: usize = 4;
        const ITERATIONS: usize = 10_000;

        let slots: Arc<Vec<Slot>> = Arc::new((0..100).map(|_| Slot::new()).collect());
        let generations: Arc<Vec<Generation>> = Arc::new(slots.iter().map(|s| s.generation()).collect());
        let errors = Arc::new(AtomicUsize::new(0));
        let running = Arc::new(AtomicBool::new(true));

        let handles: Vec<_> = (0..NUM_THREADS)
            .map(|_| {
                let slots = Arc::clone(&slots);
                let generations = Arc::clone(&generations);
                let errors = Arc::clone(&errors);
                let running = Arc::clone(&running);

                thread::spawn(move || {
                    let mut local_errors = 0;
                    for i in 0..ITERATIONS {
                        let idx = i % slots.len();
                        let slot = &slots[idx];
                        let expected_gen = generations[idx];

                        if slot.validate(expected_gen).is_err() {
                            local_errors += 1;
                        }

                        if !running.load(Ordering::Relaxed) {
                            break;
                        }
                    }
                    errors.fetch_add(local_errors, Ordering::Relaxed);
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        running.store(false, Ordering::Relaxed);
        assert_eq!(errors.load(Ordering::Relaxed), 0, "No validation errors should occur with matching generations");
    }

    /// Stress test for generation snapshot validation under concurrent reads
    #[test]
    fn stress_snapshot_concurrent_validation() {
        const NUM_THREADS: usize = 4;
        const ITERATIONS: usize = 1_000;

        // Create pointers with known generations
        let ptrs: Vec<BloodPtr> = (0..100)
            .map(|i| BloodPtr::new((i + 1) * 4096, (i + 1) as u32, PointerMetadata::HEAP))
            .collect();

        let snapshot = Arc::new(GenerationSnapshot::capture(&ptrs));
        let ptrs = Arc::new(ptrs);
        let errors = Arc::new(AtomicUsize::new(0));

        let handles: Vec<_> = (0..NUM_THREADS)
            .map(|_| {
                let snapshot = Arc::clone(&snapshot);
                let ptrs = Arc::clone(&ptrs);
                let errors = Arc::clone(&errors);

                thread::spawn(move || {
                    let mut local_errors = 0;
                    for _ in 0..ITERATIONS {
                        let valid = snapshot.validate(|addr| {
                            for ptr in ptrs.iter() {
                                if ptr.address() == addr {
                                    return Some(ptr.generation());
                                }
                            }
                            None
                        });

                        if valid.is_err() {
                            local_errors += 1;
                        }
                    }
                    errors.fetch_add(local_errors, Ordering::Relaxed);
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(errors.load(Ordering::Relaxed), 0, "Snapshot validation should succeed consistently");
    }
}
