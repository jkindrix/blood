//! Scheduler benchmarks using criterion.
//!
//! Run with: cargo bench --bench scheduler_bench

use blood_runtime::scheduler::Scheduler;
use blood_runtime::fiber::{Fiber, FiberConfig, FiberState};
use blood_runtime::SchedulerConfig;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn bench_fiber_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("fiber_creation");

    // Benchmark creating fibers with default config
    for count in [10, 100, 1000] {
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(
            BenchmarkId::new("create_fibers", count),
            &count,
            |b, &count| {
                b.iter(|| {
                    let fibers: Vec<Fiber> = (0..count)
                        .map(|i| Fiber::new(move || {
                            black_box(i);
                        }, FiberConfig::default()))
                        .collect();
                    black_box(fibers)
                });
            },
        );
    }

    group.finish();
}

fn bench_scheduler_spawn(c: &mut Criterion) {
    let mut group = c.benchmark_group("scheduler_spawn");

    // Single spawn
    group.bench_function("spawn_single", |b| {
        let scheduler = Scheduler::new(SchedulerConfig::default());
        b.iter(|| {
            let id = scheduler.spawn(|| {
                black_box(42);
            });
            black_box(id)
        });
    });

    // Batch spawn
    for batch_size in [10, 100] {
        group.throughput(Throughput::Elements(batch_size as u64));
        group.bench_with_input(
            BenchmarkId::new("spawn_batch", batch_size),
            &batch_size,
            |b, &batch_size| {
                let scheduler = Scheduler::new(SchedulerConfig::default());
                b.iter(|| {
                    let ids: Vec<_> = (0..batch_size)
                        .map(|i| scheduler.spawn(move || {
                            black_box(i);
                        }))
                        .collect();
                    black_box(ids)
                });
            },
        );
    }

    group.finish();
}

fn bench_fiber_state_transitions(c: &mut Criterion) {
    let mut group = c.benchmark_group("fiber_state");

    // Test FiberState comparisons
    group.bench_function("state_comparison", |b| {
        let state1 = FiberState::Runnable;
        let state2 = FiberState::Running;
        b.iter(|| {
            black_box(state1 == state2);
            black_box(state1 != state2);
        });
    });

    group.finish();
}

fn bench_scheduler_config(c: &mut Criterion) {
    let mut group = c.benchmark_group("scheduler_config");

    // Benchmark scheduler creation with different configs
    for worker_count in [1, 2, 4, 8] {
        group.bench_with_input(
            BenchmarkId::new("create_scheduler", worker_count),
            &worker_count,
            |b, &worker_count| {
                b.iter(|| {
                    let config = SchedulerConfig {
                        num_workers: worker_count,
                        ..Default::default()
                    };
                    let scheduler = Scheduler::new(config);
                    black_box(scheduler)
                });
            },
        );
    }

    group.finish();
}

fn bench_concurrent_counter(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent_counter");

    // Simulate concurrent fiber workload
    for fiber_count in [10, 100] {
        group.throughput(Throughput::Elements(fiber_count as u64));
        group.bench_with_input(
            BenchmarkId::new("atomic_increments", fiber_count),
            &fiber_count,
            |b, &fiber_count| {
                b.iter(|| {
                    let counter = Arc::new(AtomicUsize::new(0));
                    let scheduler = Scheduler::new(SchedulerConfig::default());

                    for _ in 0..fiber_count {
                        let counter_clone = Arc::clone(&counter);
                        scheduler.spawn(move || {
                            counter_clone.fetch_add(1, Ordering::SeqCst);
                        });
                    }

                    black_box(counter.load(Ordering::SeqCst))
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_fiber_creation,
    bench_scheduler_spawn,
    bench_fiber_state_transitions,
    bench_scheduler_config,
    bench_concurrent_counter,
);
criterion_main!(benches);
