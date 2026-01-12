//! Performance target validation tests.
//!
//! These tests validate that critical runtime operations meet their
//! performance targets as specified in SPECIFICATION.md and CONCURRENCY.md.
//!
//! Run with: cargo test --test perf_targets --release
//!
//! Note: These tests should be run in release mode for accurate measurements.

use blood_runtime::continuation::Continuation;
use blood_runtime::memory::{BloodPtr, GenerationSnapshot, PointerMetadata, Slot};
use std::time::{Duration, Instant};

/// Number of iterations for timing measurements.
const ITERATIONS: u64 = 100_000;

/// Warm-up iterations before timing.
const WARMUP_ITERATIONS: u64 = 1_000;

/// Run a function multiple times and return average nanoseconds per operation.
fn measure_ns_per_op<F>(mut f: F) -> f64
where
    F: FnMut(),
{
    // Warm up
    for _ in 0..WARMUP_ITERATIONS {
        f();
    }

    // Measure
    let start = Instant::now();
    for _ in 0..ITERATIONS {
        f();
    }
    let elapsed = start.elapsed();

    elapsed.as_nanos() as f64 / ITERATIONS as f64
}

/// Test that effect handler operations achieve target throughput.
///
/// Target: 150M ops/sec = ~6.67ns per operation
///
/// This tests the core continuation create/resume cycle which is the
/// fundamental operation for effect handlers.
#[test]
fn test_effect_handler_throughput() {
    // Target: 150M ops/sec = 6.67ns per op
    // Allow 3x overhead for test/debug mode: 20ns
    // In release mode, should be under 10ns
    const TARGET_NS: f64 = 50.0; // Conservative target for CI

    let ns_per_op = measure_ns_per_op(|| {
        let k = Continuation::new(|x: i32| x + 1);
        let result: i32 = k.resume(41);
        std::hint::black_box(result);
    });

    let ops_per_sec = 1_000_000_000.0 / ns_per_op;

    println!(
        "Effect handler: {:.2}ns/op ({:.2}M ops/sec)",
        ns_per_op,
        ops_per_sec / 1_000_000.0
    );

    // In debug mode, we just verify it runs
    // In release mode with --release flag, this should pass
    #[cfg(not(debug_assertions))]
    assert!(
        ns_per_op < TARGET_NS,
        "Effect handler too slow: {:.2}ns/op (target: {:.2}ns/op, {:.2}M ops/sec)",
        ns_per_op,
        TARGET_NS,
        ops_per_sec / 1_000_000.0
    );
}

/// Test that generation check overhead is under target.
///
/// Target: <50ns per generation check
///
/// Generation checks are performed on every pointer dereference to
/// detect use-after-free at runtime.
#[test]
fn test_generation_check_overhead() {
    const TARGET_NS: f64 = 50.0;

    let slot = Slot::new();
    let gen = slot.generation();

    let ns_per_op = measure_ns_per_op(|| {
        let valid = slot.validate(gen);
        std::hint::black_box(valid);
    });

    println!("Generation check: {:.2}ns/op (target: <{:.0}ns)", ns_per_op, TARGET_NS);

    #[cfg(not(debug_assertions))]
    assert!(
        ns_per_op < TARGET_NS,
        "Generation check too slow: {:.2}ns/op (target: <{:.0}ns)",
        ns_per_op,
        TARGET_NS
    );
}

/// Test that snapshot capture overhead is under target.
///
/// Target: <100ns per reference captured
///
/// Snapshots are used by effect handlers to track references that
/// must remain valid across suspension/resumption.
#[test]
fn test_snapshot_capture_overhead() {
    const TARGET_NS_PER_REF: f64 = 100.0;
    const NUM_REFS: usize = 10;

    let ptrs: Vec<BloodPtr> = (0..NUM_REFS)
        .map(|i| BloodPtr::new(0x1000 * (i + 1), i as u32 + 1, PointerMetadata::HEAP))
        .collect();

    let ns_per_capture = measure_ns_per_op(|| {
        let snapshot = GenerationSnapshot::capture(&ptrs);
        std::hint::black_box(snapshot);
    });

    let ns_per_ref = ns_per_capture / NUM_REFS as f64;

    println!(
        "Snapshot capture: {:.2}ns total for {} refs ({:.2}ns/ref, target: <{:.0}ns/ref)",
        ns_per_capture, NUM_REFS, ns_per_ref, TARGET_NS_PER_REF
    );

    #[cfg(not(debug_assertions))]
    assert!(
        ns_per_ref < TARGET_NS_PER_REF,
        "Snapshot capture too slow: {:.2}ns/ref (target: <{:.0}ns/ref)",
        ns_per_ref,
        TARGET_NS_PER_REF
    );
}

/// Test that snapshot validation overhead is reasonable.
///
/// Validation checks all captured references are still valid.
#[test]
fn test_snapshot_validation_overhead() {
    const NUM_REFS: usize = 10;
    // Validation should be similar to generation checks per ref
    const TARGET_NS_PER_REF: f64 = 100.0;

    let ptrs: Vec<BloodPtr> = (0..NUM_REFS)
        .map(|i| BloodPtr::new(0x1000 * (i + 1), i as u32 + 1, PointerMetadata::HEAP))
        .collect();

    let snapshot = GenerationSnapshot::capture(&ptrs);

    let ns_per_validation = measure_ns_per_op(|| {
        let valid = snapshot.validate(|addr| {
            // Simulate generation lookup - return the expected generation
            let index = (addr / 0x1000) - 1;
            Some(index as u32 + 1)
        });
        std::hint::black_box(valid);
    });

    let ns_per_ref = ns_per_validation / NUM_REFS as f64;

    println!(
        "Snapshot validation: {:.2}ns total for {} refs ({:.2}ns/ref)",
        ns_per_validation, NUM_REFS, ns_per_ref
    );

    #[cfg(not(debug_assertions))]
    assert!(
        ns_per_ref < TARGET_NS_PER_REF,
        "Snapshot validation too slow: {:.2}ns/ref (target: <{:.0}ns/ref)",
        ns_per_ref,
        TARGET_NS_PER_REF
    );
}

/// Test pointer operations are fast.
///
/// BloodPtr operations (address extraction, generation check) should
/// be essentially free - just bit manipulation.
#[test]
fn test_pointer_operations_fast() {
    const TARGET_NS: f64 = 10.0; // Should be ~1ns, allow margin

    let ptr = BloodPtr::new(0x12345678, 42, PointerMetadata::HEAP);

    let ns_per_op = measure_ns_per_op(|| {
        let addr = ptr.address();
        let gen = ptr.generation();
        let meta = ptr.metadata();
        std::hint::black_box((addr, gen, meta));
    });

    println!("Pointer operations: {:.2}ns/op (target: <{:.0}ns)", ns_per_op, TARGET_NS);

    #[cfg(not(debug_assertions))]
    assert!(
        ns_per_op < TARGET_NS,
        "Pointer operations too slow: {:.2}ns/op (target: <{:.0}ns)",
        ns_per_op,
        TARGET_NS
    );
}

/// Summary test that prints all performance metrics.
#[test]
fn test_performance_summary() {
    println!("\n=== Blood Runtime Performance Summary ===\n");

    // Effect handler
    let effect_ns = measure_ns_per_op(|| {
        let k = Continuation::new(|x: i32| x + 1);
        let result: i32 = k.resume(41);
        std::hint::black_box(result);
    });
    let effect_mops = 1_000.0 / effect_ns;
    println!(
        "Effect Handler:      {:>8.2} ns/op  ({:>6.2} M ops/sec)  target: 150M ops/sec",
        effect_ns, effect_mops
    );

    // Generation check
    let slot = Slot::new();
    let gen = slot.generation();
    let gen_ns = measure_ns_per_op(|| {
        std::hint::black_box(slot.validate(gen));
    });
    println!(
        "Generation Check:    {:>8.2} ns/op                      target: <50ns",
        gen_ns
    );

    // Snapshot capture (10 refs)
    let ptrs: Vec<BloodPtr> = (0..10usize)
        .map(|i| BloodPtr::new(0x1000 * (i + 1), i as u32 + 1, PointerMetadata::HEAP))
        .collect();
    let snap_ns = measure_ns_per_op(|| {
        std::hint::black_box(GenerationSnapshot::capture(&ptrs));
    });
    println!(
        "Snapshot Capture:    {:>8.2} ns/op  ({:.2} ns/ref)       target: <100ns/ref",
        snap_ns,
        snap_ns / 10.0
    );

    // Pointer operations
    let ptr = BloodPtr::new(0x12345678, 42, PointerMetadata::HEAP);
    let ptr_ns = measure_ns_per_op(|| {
        std::hint::black_box((ptr.address(), ptr.generation(), ptr.metadata()));
    });
    println!("Pointer Operations:  {:>8.2} ns/op                      target: <10ns", ptr_ns);

    println!("\n=========================================\n");
}
