//! Channel benchmarks using criterion.
//!
//! Run with: cargo bench --bench channel_bench

use blood_runtime::channel::{bounded, unbounded};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::thread;

fn bench_channel_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("channel_creation");

    // Benchmark bounded channel creation
    for capacity in [1, 10, 100, 1000] {
        group.bench_with_input(
            BenchmarkId::new("bounded", capacity),
            &capacity,
            |b, &capacity| {
                b.iter(|| {
                    let (tx, rx) = bounded::<i32>(capacity);
                    black_box((tx, rx))
                });
            },
        );
    }

    // Benchmark unbounded channel creation
    group.bench_function("unbounded", |b| {
        b.iter(|| {
            let (tx, rx) = unbounded::<i32>();
            black_box((tx, rx))
        });
    });

    group.finish();
}

fn bench_send_recv(c: &mut Criterion) {
    let mut group = c.benchmark_group("send_recv");

    // Single send/recv
    group.bench_function("single_bounded", |b| {
        let (tx, rx) = bounded(1024);
        b.iter(|| {
            tx.send(black_box(42)).unwrap();
            black_box(rx.recv().unwrap())
        });
    });

    group.bench_function("single_unbounded", |b| {
        let (tx, rx) = unbounded();
        b.iter(|| {
            tx.send(black_box(42)).unwrap();
            black_box(rx.recv().unwrap())
        });
    });

    // Batch send/recv
    for batch_size in [10, 100, 1000] {
        group.throughput(Throughput::Elements(batch_size as u64));
        group.bench_with_input(
            BenchmarkId::new("batch_bounded", batch_size),
            &batch_size,
            |b, &batch_size| {
                let (tx, rx) = bounded(batch_size * 2);
                b.iter(|| {
                    for i in 0..batch_size {
                        tx.send(black_box(i)).unwrap();
                    }
                    for _ in 0..batch_size {
                        black_box(rx.recv().unwrap());
                    }
                });
            },
        );
    }

    group.finish();
}

fn bench_try_send_recv(c: &mut Criterion) {
    let mut group = c.benchmark_group("try_send_recv");

    group.bench_function("try_single", |b| {
        let (tx, rx) = bounded(1024);
        b.iter(|| {
            tx.try_send(black_box(42)).unwrap();
            black_box(rx.try_recv().unwrap())
        });
    });

    // Try operations on full/empty channels
    group.bench_function("try_send_full", |b| {
        let (tx, _rx) = bounded::<i32>(1);
        tx.send(0).unwrap();
        b.iter(|| {
            black_box(tx.try_send(42))
        });
    });

    group.bench_function("try_recv_empty", |b| {
        let (_tx, rx) = bounded::<i32>(1);
        b.iter(|| {
            black_box(rx.try_recv())
        });
    });

    group.finish();
}

fn bench_mpmc(c: &mut Criterion) {
    let mut group = c.benchmark_group("mpmc");

    // Multi-producer
    for producer_count in [2, 4] {
        group.throughput(Throughput::Elements(100));
        group.bench_with_input(
            BenchmarkId::new("multi_producer", producer_count),
            &producer_count,
            |b, &producer_count| {
                b.iter(|| {
                    let (tx, rx) = bounded(producer_count * 100);
                    let handles: Vec<_> = (0..producer_count)
                        .map(|p| {
                            let tx = tx.clone();
                            thread::spawn(move || {
                                for i in 0..100 / producer_count {
                                    tx.send(p * 100 + i).unwrap();
                                }
                            })
                        })
                        .collect();

                    for handle in handles {
                        handle.join().unwrap();
                    }

                    drop(tx);
                    let collected: Vec<_> = rx.into_iter().collect();
                    black_box(collected)
                });
            },
        );
    }

    // Multi-consumer
    for consumer_count in [2, 4] {
        group.throughput(Throughput::Elements(100));
        group.bench_with_input(
            BenchmarkId::new("multi_consumer", consumer_count),
            &consumer_count,
            |b, &consumer_count| {
                b.iter(|| {
                    let (tx, rx) = bounded(100);

                    // Send all messages first
                    for i in 0..100 {
                        tx.send(i).unwrap();
                    }
                    drop(tx);

                    let handles: Vec<_> = (0..consumer_count)
                        .map(|_| {
                            let rx = rx.clone();
                            thread::spawn(move || {
                                let mut received = Vec::new();
                                while let Ok(v) = rx.recv() {
                                    received.push(v);
                                }
                                received
                            })
                        })
                        .collect();

                    let results: Vec<_> = handles
                        .into_iter()
                        .map(|h| h.join().unwrap())
                        .collect();
                    black_box(results)
                });
            },
        );
    }

    group.finish();
}

fn bench_channel_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("channel_operations");

    // len/is_empty/is_full checks
    group.bench_function("len_check", |b| {
        let (tx, rx) = bounded::<i32>(100);
        for i in 0..50 {
            tx.send(i).unwrap();
        }
        b.iter(|| {
            black_box(tx.len());
            black_box(rx.len());
        });
    });

    group.bench_function("is_empty_check", |b| {
        let (tx, rx) = bounded::<i32>(100);
        b.iter(|| {
            black_box(tx.is_empty());
            black_box(rx.is_empty());
        });
    });

    group.bench_function("is_full_check", |b| {
        let (tx, rx) = bounded::<i32>(100);
        b.iter(|| {
            black_box(tx.is_full());
            black_box(rx.is_full());
        });
    });

    group.bench_function("capacity_check", |b| {
        let (tx, rx) = bounded::<i32>(100);
        b.iter(|| {
            black_box(tx.capacity());
            black_box(rx.capacity());
        });
    });

    group.finish();
}

fn bench_channel_clone(c: &mut Criterion) {
    let mut group = c.benchmark_group("channel_clone");

    group.bench_function("clone_sender", |b| {
        let (tx, _rx) = bounded::<i32>(100);
        b.iter(|| {
            black_box(tx.clone())
        });
    });

    group.bench_function("clone_receiver", |b| {
        let (_tx, rx) = bounded::<i32>(100);
        b.iter(|| {
            black_box(rx.clone())
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_channel_creation,
    bench_send_recv,
    bench_try_send_recv,
    bench_mpmc,
    bench_channel_operations,
    bench_channel_clone,
);
criterion_main!(benches);
