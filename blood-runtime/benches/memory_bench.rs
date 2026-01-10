//! Memory management benchmarks using criterion.
//!
//! Run with: cargo bench --bench memory_bench

use blood_runtime::memory::{
    BloodPtr, PointerMetadata, MemoryTier,
    Slot, Region, GenerationSnapshot,
    generation,
};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::alloc::Layout;

fn bench_blood_ptr_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("blood_ptr_creation");

    group.bench_function("null_ptr", |b| {
        b.iter(|| {
            black_box(BloodPtr::null())
        });
    });

    group.bench_function("with_generation", |b| {
        let gen = generation::FIRST;
        b.iter(|| {
            black_box(BloodPtr::new(0x1000, gen, PointerMetadata::NONE))
        });
    });

    group.bench_function("metadata_none", |b| {
        b.iter(|| {
            black_box(PointerMetadata::NONE)
        });
    });

    group.bench_function("metadata_stack", |b| {
        b.iter(|| {
            black_box(PointerMetadata::STACK)
        });
    });

    group.finish();
}

fn bench_blood_ptr_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("blood_ptr_operations");

    let gen = generation::FIRST;
    let ptr = BloodPtr::new(0x1000, gen, PointerMetadata::HEAP);

    group.bench_function("address", |b| {
        b.iter(|| {
            black_box(ptr.address())
        });
    });

    group.bench_function("generation", |b| {
        b.iter(|| {
            black_box(ptr.generation())
        });
    });

    group.bench_function("is_null", |b| {
        b.iter(|| {
            black_box(ptr.is_null())
        });
    });

    group.bench_function("metadata_contains", |b| {
        let meta = PointerMetadata::HEAP.union(PointerMetadata::LINEAR);
        b.iter(|| {
            black_box(meta.contains(PointerMetadata::HEAP));
            black_box(meta.contains(PointerMetadata::LINEAR));
        });
    });

    group.finish();
}

fn bench_slot_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("slot_operations");

    group.bench_function("creation", |b| {
        b.iter(|| {
            black_box(Slot::new())
        });
    });

    group.bench_function("generation", |b| {
        let slot = Slot::new();
        b.iter(|| {
            black_box(slot.generation())
        });
    });

    group.bench_function("is_occupied", |b| {
        let slot = Slot::new();
        b.iter(|| {
            black_box(slot.is_occupied())
        });
    });

    group.bench_function("validate_generation", |b| {
        let slot = Slot::new();
        let gen = slot.generation();
        b.iter(|| {
            black_box(slot.validate(gen))
        });
    });

    group.bench_function("allocate_deallocate_cycle", |b| {
        let slot = Slot::new();
        let layout = Layout::from_size_align(64, 8).unwrap();
        b.iter(|| {
            unsafe {
                let ptr = slot.allocate(layout);
                black_box(ptr);
                slot.deallocate();
            }
        });
    });

    group.finish();
}

fn bench_region_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("region_operations");

    // Region creation
    group.bench_function("creation", |b| {
        b.iter(|| {
            black_box(Region::new(4096, 1024 * 1024))
        });
    });

    // Single allocation
    group.bench_function("single_alloc", |b| {
        let mut region = Region::new(1024 * 1024, 1024 * 1024);
        b.iter(|| {
            let ptr = region.allocate(64, 8);
            black_box(ptr)
        });
    });

    // Batch allocation
    for count in [10, 100] {
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(
            BenchmarkId::new("batch_alloc", count),
            &count,
            |b, &count| {
                b.iter(|| {
                    let mut region = Region::new(1024 * 1024, 1024 * 1024);
                    let ptrs: Vec<_> = (0..count)
                        .filter_map(|_| region.allocate(64, 8))
                        .collect();
                    black_box(ptrs)
                });
            },
        );
    }

    // Region reset
    group.bench_function("reset", |b| {
        b.iter(|| {
            let mut region = Region::new(4096, 1024 * 1024);
            // Allocate some objects
            for _ in 0..10 {
                region.allocate(64, 8);
            }
            region.reset();
            black_box(())
        });
    });

    // Region info
    group.bench_function("used", |b| {
        let mut region = Region::new(4096, 1024 * 1024);
        for _ in 0..10 {
            region.allocate(64, 8);
        }
        b.iter(|| {
            black_box(region.used())
        });
    });

    group.bench_function("capacity", |b| {
        let region = Region::new(4096, 1024 * 1024);
        b.iter(|| {
            black_box(region.capacity())
        });
    });

    group.finish();
}

fn bench_generation_snapshot(c: &mut Criterion) {
    let mut group = c.benchmark_group("generation_snapshot");

    group.bench_function("creation", |b| {
        b.iter(|| {
            black_box(GenerationSnapshot::new())
        });
    });

    group.bench_function("add_pointer", |b| {
        let ptr = BloodPtr::new(0x1000, generation::FIRST, PointerMetadata::HEAP);
        b.iter(|| {
            let mut snapshot = GenerationSnapshot::new();
            snapshot.add(&ptr);
            black_box(snapshot)
        });
    });

    // Batch capture
    for count in [10, 100] {
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(
            BenchmarkId::new("capture_batch", count),
            &count,
            |b, &count| {
                let ptrs: Vec<BloodPtr> = (0..count)
                    .map(|i| BloodPtr::new(i * 64, generation::FIRST, PointerMetadata::HEAP))
                    .collect();
                b.iter(|| {
                    black_box(GenerationSnapshot::capture(&ptrs))
                });
            },
        );
    }

    group.finish();
}

fn bench_memory_tier_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_tier");

    group.bench_function("tier_comparison", |b| {
        b.iter(|| {
            black_box(MemoryTier::Stack == MemoryTier::Region);
            black_box(MemoryTier::Region == MemoryTier::Heap);
            black_box(MemoryTier::Heap == MemoryTier::Stack);
        });
    });

    group.finish();
}

fn bench_pointer_metadata(c: &mut Criterion) {
    let mut group = c.benchmark_group("pointer_metadata");

    group.bench_function("from_bits", |b| {
        b.iter(|| {
            black_box(PointerMetadata::from_bits(0b1111))
        });
    });

    group.bench_function("bits", |b| {
        let meta = PointerMetadata::HEAP.union(PointerMetadata::LINEAR);
        b.iter(|| {
            black_box(meta.bits())
        });
    });

    group.bench_function("union", |b| {
        b.iter(|| {
            black_box(PointerMetadata::HEAP.union(PointerMetadata::LINEAR))
        });
    });

    group.bench_function("contains", |b| {
        let meta = PointerMetadata::HEAP.union(PointerMetadata::LINEAR);
        b.iter(|| {
            black_box(meta.contains(PointerMetadata::HEAP))
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_blood_ptr_creation,
    bench_blood_ptr_operations,
    bench_slot_operations,
    bench_region_operations,
    bench_generation_snapshot,
    bench_memory_tier_operations,
    bench_pointer_metadata,
);
criterion_main!(benches);
