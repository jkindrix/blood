//! Effect system benchmarks using criterion.
//!
//! Benchmarks for continuation creation, resumption, registry operations,
//! and generation snapshot validation. These validate the performance targets
//! from SPECIFICATION.md §11.7.
//!
//! Run with: cargo bench --bench effects_bench

use blood_runtime::continuation::{
    Continuation, EffectContext,
    register_continuation, take_continuation, has_continuation,
};
use blood_runtime::memory::{BloodPtr, GenerationSnapshot, PointerMetadata, generation};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

/// Benchmark continuation creation
fn bench_continuation_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("continuation_creation");

    // Simple closure continuation
    group.bench_function("simple_closure", |b| {
        b.iter(|| {
            black_box(Continuation::new(|x: i32| x + 1))
        });
    });

    // Continuation with captured state
    group.bench_function("capturing_closure", |b| {
        let captured_value = vec![1, 2, 3, 4, 5];
        b.iter(|| {
            let v = captured_value.clone();
            black_box(Continuation::new(move |x: i32| x + v.len() as i32))
        });
    });

    // String processing continuation
    group.bench_function("string_closure", |b| {
        b.iter(|| {
            black_box(Continuation::new(|s: String| format!("Hello, {}!", s)))
        });
    });

    group.finish();
}

/// Benchmark continuation resumption
fn bench_continuation_resume(c: &mut Criterion) {
    let mut group = c.benchmark_group("continuation_resume");

    // Simple integer resume
    group.bench_function("resume_int", |b| {
        b.iter_custom(|iters| {
            let start = std::time::Instant::now();
            for _ in 0..iters {
                let k = Continuation::new(|x: i32| x + 1);
                let result: i32 = k.resume(41);
                black_box(result);
            }
            start.elapsed()
        });
    });

    // String resume
    group.bench_function("resume_string", |b| {
        b.iter_custom(|iters| {
            let start = std::time::Instant::now();
            for _ in 0..iters {
                let k = Continuation::new(|s: String| s.len());
                let result: usize = k.resume("hello".to_string());
                black_box(result);
            }
            start.elapsed()
        });
    });

    // Try resume (checks consumed state)
    group.bench_function("try_resume", |b| {
        b.iter_custom(|iters| {
            let start = std::time::Instant::now();
            for _ in 0..iters {
                let k = Continuation::new(|x: i32| x * 2);
                let result: Option<i32> = k.try_resume(21);
                black_box(result);
            }
            start.elapsed()
        });
    });

    group.finish();
}

/// Benchmark continuation registry operations
fn bench_continuation_registry(c: &mut Criterion) {
    let mut group = c.benchmark_group("continuation_registry");

    // Register and take cycle
    group.bench_function("register_take_cycle", |b| {
        b.iter(|| {
            let k = Continuation::new(|x: i32| x);
            let r = register_continuation(k);
            let taken = take_continuation(r);
            black_box(taken)
        });
    });

    // Register only
    group.bench_function("register_only", |b| {
        b.iter_custom(|iters| {
            let mut refs = Vec::with_capacity(iters as usize);
            let start = std::time::Instant::now();
            for _ in 0..iters {
                let k = Continuation::new(|x: i32| x);
                refs.push(register_continuation(k));
            }
            let elapsed = start.elapsed();
            // Cleanup
            for r in refs {
                take_continuation(r);
            }
            elapsed
        });
    });

    // Has continuation check
    group.bench_function("has_continuation_check", |b| {
        let k = Continuation::new(|x: i32| x);
        let r = register_continuation(k);
        b.iter(|| {
            black_box(has_continuation(r))
        });
        // Cleanup
        take_continuation(r);
    });

    group.finish();
}

/// Benchmark EffectContext operations
fn bench_effect_context(c: &mut Criterion) {
    let mut group = c.benchmark_group("effect_context");

    // Create tail-resumptive context
    group.bench_function("create_tail_resumptive", |b| {
        b.iter(|| {
            black_box(EffectContext::tail_resumptive())
        });
    });

    // Create context with continuation
    group.bench_function("create_with_continuation", |b| {
        b.iter_custom(|iters| {
            let start = std::time::Instant::now();
            for _ in 0..iters {
                let k = Continuation::new(|x: i32| x);
                let r = register_continuation(k);
                let ctx = EffectContext::with_continuation(r);
                black_box(ctx);
                take_continuation(r);
            }
            start.elapsed()
        });
    });

    // Set resume value
    group.bench_function("set_resume_value", |b| {
        let mut ctx = EffectContext::tail_resumptive();
        let mut i = 0i64;
        b.iter(|| {
            ctx.set_resume_value(black_box(i));
            i = i.wrapping_add(1);
            black_box(ctx.resume_value)
        });
    });

    group.finish();
}

/// Benchmark GenerationSnapshot operations (used for effect handler safety)
fn bench_generation_snapshot(c: &mut Criterion) {
    let mut group = c.benchmark_group("generation_snapshot");

    // Create empty snapshot
    group.bench_function("create_empty", |b| {
        b.iter(|| {
            black_box(GenerationSnapshot::new())
        });
    });

    // Capture snapshot from pointers
    for count in [1, 5, 10, 20] {
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(
            BenchmarkId::new("capture", count),
            &count,
            |b, &count| {
                let ptrs: Vec<BloodPtr> = (0..count)
                    .map(|i| BloodPtr::new(0x1000 * (i + 1), i as u32 + 1, PointerMetadata::HEAP))
                    .collect();
                b.iter(|| {
                    black_box(GenerationSnapshot::capture(&ptrs))
                });
            },
        );
    }

    // Add entries incrementally
    for count in [1, 5, 10] {
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(
            BenchmarkId::new("add_entries", count),
            &count,
            |b, &count| {
                let ptrs: Vec<BloodPtr> = (0..count)
                    .map(|i| BloodPtr::new(0x1000 * (i + 1), i as u32 + 1, PointerMetadata::HEAP))
                    .collect();
                b.iter(|| {
                    let mut snapshot = GenerationSnapshot::new();
                    for ptr in &ptrs {
                        snapshot.add(ptr);
                    }
                    black_box(snapshot)
                });
            },
        );
    }

    group.finish();
}

/// Benchmark GenerationSnapshot validation (critical for effect safety)
fn bench_snapshot_validation(c: &mut Criterion) {
    let mut group = c.benchmark_group("snapshot_validation");

    // Validate with all valid references
    for count in [1, 5, 10, 20] {
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(
            BenchmarkId::new("validate_all_valid", count),
            &count,
            |b, &count| {
                let ptrs: Vec<BloodPtr> = (0..count)
                    .map(|i| BloodPtr::new(0x1000 * (i + 1), i as u32 + 1, PointerMetadata::HEAP))
                    .collect();
                let snapshot = GenerationSnapshot::capture(&ptrs);

                b.iter(|| {
                    let result = snapshot.validate(|addr| {
                        // Simulate looking up generation - all valid
                        let index = (addr / 0x1000) - 1;
                        Some(index as u32 + 1)
                    });
                    black_box(result)
                });
            },
        );
    }

    // Validate with one stale reference (early exit)
    for count in [5, 10, 20] {
        group.bench_with_input(
            BenchmarkId::new("validate_one_stale", count),
            &count,
            |b, &count| {
                let ptrs: Vec<BloodPtr> = (0..count)
                    .map(|i| BloodPtr::new(0x1000 * (i + 1), i as u32 + 1, PointerMetadata::HEAP))
                    .collect();
                let snapshot = GenerationSnapshot::capture(&ptrs);

                b.iter(|| {
                    let result = snapshot.validate(|addr| {
                        // First reference is stale, others valid
                        let index = (addr / 0x1000) - 1;
                        if index == 0 {
                            Some(99) // Wrong generation
                        } else {
                            Some(index as u32 + 1)
                        }
                    });
                    black_box(result)
                });
            },
        );
    }

    group.finish();
}

/// Benchmark BloodPtr operations
fn bench_blood_ptr(c: &mut Criterion) {
    let mut group = c.benchmark_group("blood_ptr");

    // Create null pointer
    group.bench_function("create_null", |b| {
        b.iter(|| {
            black_box(BloodPtr::null())
        });
    });

    // Create pointer with metadata
    group.bench_function("create_with_metadata", |b| {
        b.iter(|| {
            black_box(BloodPtr::new(
                0x1000,
                generation::FIRST,
                PointerMetadata::HEAP.union(PointerMetadata::LINEAR),
            ))
        });
    });

    // Check pointer properties
    group.bench_function("check_properties", |b| {
        let ptr = BloodPtr::new(0x1000, 42, PointerMetadata::HEAP);
        b.iter(|| {
            black_box((
                ptr.is_null(),
                ptr.is_heap(),
                ptr.is_stack(),
                ptr.is_linear(),
                ptr.generation(),
            ))
        });
    });

    group.finish();
}

/// Benchmark snapshot overhead in effect-heavy program patterns (SNAP-003)
///
/// This simulates realistic effect-heavy workloads where snapshots are
/// captured and validated frequently. Measures total overhead from:
/// - Effect operation setup
/// - Snapshot capture
/// - Handler execution
/// - Snapshot validation on resume
fn bench_effect_heavy_snapshot_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("effect_heavy_snapshot");

    // Simulate simple State effect pattern (get/put operations)
    // Most common pattern: tail-resumptive handler, single captured ref
    group.bench_function("state_pattern_single_ref", |b| {
        let ptr = BloodPtr::new(0x1000, 42, PointerMetadata::HEAP);
        b.iter(|| {
            // Capture snapshot before effect
            let snapshot = GenerationSnapshot::capture(&[ptr]);

            // Simulate effect operation (read state)
            let state_value = black_box(42i64);

            // Validate on resume (generation still valid)
            let valid = snapshot.validate(|addr| {
                if addr == 0x1000 { Some(42) } else { None }
            });

            black_box((state_value, valid))
        });
    });

    // Multiple captured references (common in real programs)
    for ref_count in [3, 5, 10, 20] {
        group.throughput(Throughput::Elements(ref_count as u64));
        group.bench_with_input(
            BenchmarkId::new("capture_validate_cycle", ref_count),
            &ref_count,
            |b, &count| {
                let ptrs: Vec<BloodPtr> = (0..count)
                    .map(|i| BloodPtr::new(0x1000 * (i + 1), i as u32 + 1, PointerMetadata::HEAP))
                    .collect();

                b.iter(|| {
                    // Capture snapshot
                    let snapshot = GenerationSnapshot::capture(&ptrs);

                    // Simulate handler work
                    let work = black_box(42 + count);

                    // Validate on resume
                    let valid = snapshot.validate(|addr| {
                        let index = (addr / 0x1000) - 1;
                        Some(index as u32 + 1)
                    });

                    black_box((work, valid))
                });
            },
        );
    }

    // Nested effect handlers (each captures and validates)
    for depth in [2, 3, 5] {
        group.bench_with_input(
            BenchmarkId::new("nested_handlers", depth),
            &depth,
            |b, &depth| {
                // Each nesting level has its own reference
                let ptrs: Vec<BloodPtr> = (0..depth)
                    .map(|i| BloodPtr::new(0x1000 * (i + 1), i as u32 + 1, PointerMetadata::HEAP))
                    .collect();

                b.iter(|| {
                    // Simulate nested handler capture/validate cycle
                    let mut snapshots = Vec::with_capacity(depth);

                    // Capture phase (handlers installed inner-to-outer)
                    for i in 0..depth {
                        snapshots.push(GenerationSnapshot::capture(&ptrs[0..=i]));
                    }

                    // Handler execution
                    let work = black_box(depth * 10);

                    // Validate phase (handlers resumed outer-to-inner)
                    let mut valid = true;
                    for snapshot in snapshots.iter().rev() {
                        valid = valid && snapshot.validate(|addr| {
                            let index = (addr / 0x1000) - 1;
                            Some(index as u32 + 1)
                        }).is_ok();
                    }

                    black_box((work, valid))
                });
            },
        );
    }

    // Sequential effect operations (common in imperative code)
    for op_count in [10, 50, 100] {
        group.throughput(Throughput::Elements(op_count as u64));
        group.bench_with_input(
            BenchmarkId::new("sequential_ops", op_count),
            &op_count,
            |b, &count| {
                // Single reference captured across all operations
                let ptr = BloodPtr::new(0x1000, 42, PointerMetadata::HEAP);

                b.iter(|| {
                    let mut total = 0i64;
                    for i in 0..count {
                        // Each operation: capture, execute, validate
                        let snapshot = GenerationSnapshot::capture(&[ptr]);
                        total = total.wrapping_add(i as i64);
                        let _ = snapshot.validate(|addr| {
                            if addr == 0x1000 { Some(42) } else { None }
                        });
                    }
                    black_box(total)
                });
            },
        );
    }

    // Effect with closure capture (closure environment in snapshot)
    group.bench_function("closure_capture_pattern", |b| {
        // Simulate closure environment with multiple captures
        let env_ptrs: Vec<BloodPtr> = (0..5)
            .map(|i| BloodPtr::new(0x2000 * (i + 1), i as u32 + 10, PointerMetadata::HEAP))
            .collect();

        b.iter(|| {
            // Effect operation with closure that captures environment
            let snapshot = GenerationSnapshot::capture(&env_ptrs);

            // Simulate closure execution
            let closure_result = black_box(env_ptrs.len() * 42);

            // Validate closure captures are still valid
            let valid = snapshot.validate(|addr| {
                let index = (addr / 0x2000) - 1;
                Some(index as u32 + 10)
            });

            black_box((closure_result, valid))
        });
    });

    // Effect with stale detection (validation finds invalid ref)
    group.bench_function("stale_detection", |b| {
        let ptrs: Vec<BloodPtr> = (0..5)
            .map(|i| BloodPtr::new(0x1000 * (i + 1), i as u32 + 1, PointerMetadata::HEAP))
            .collect();

        b.iter(|| {
            let snapshot = GenerationSnapshot::capture(&ptrs);

            // Validation detects stale reference (middle pointer invalidated)
            let result = snapshot.validate(|addr| {
                let index = (addr / 0x1000) - 1;
                if index == 2 {
                    Some(999) // Changed generation = stale
                } else {
                    Some(index as u32 + 1)
                }
            });

            black_box(result.is_err())
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_continuation_creation,
    bench_continuation_resume,
    bench_continuation_registry,
    bench_effect_context,
    bench_generation_snapshot,
    bench_snapshot_validation,
    bench_blood_ptr,
    bench_effect_heavy_snapshot_overhead,
);
criterion_main!(benches);
