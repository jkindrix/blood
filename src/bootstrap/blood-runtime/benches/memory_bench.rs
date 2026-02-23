//! Memory management benchmarks using criterion.
//!
//! Run with: cargo bench --bench memory_bench

use blood_runtime::memory::{
    BloodPtr, PointerMetadata, MemoryTier,
    Slot, Region, GenerationSnapshot,
    generation,
    // Persistent tier (Tier 2)
    persistent_allocator, persistent_alloc, persistent_increment, persistent_decrement,
    // Cycle collection
    cycle_collector, collect_cycles,
    // Tier2 allocator
    tier2_allocator, tier2_alloc, tier2_increment, tier2_decrement,
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

/// Benchmark generation checks in hot loop (per SPECIFICATION.md §11.7.2)
fn bench_gen_check_hot(c: &mut Criterion) {
    let mut group = c.benchmark_group("gen_check");

    // Hot path: generation check in tight loop
    group.bench_function("hot", |b| {
        let slot = Slot::new();
        let gen = slot.generation();
        b.iter(|| {
            // Simulate checking generation on every dereference
            for _ in 0..100 {
                let _ = black_box(slot.validate(gen));
            }
        });
    });

    // Compare to baseline: raw pointer check (always succeeds)
    group.bench_function("raw_ptr_baseline", |b| {
        let ptr: *const i32 = &42;
        b.iter(|| {
            for _ in 0..100 {
                black_box(!ptr.is_null());
            }
        });
    });

    group.finish();
}

/// Benchmark generation checks with cache miss simulation (per SPECIFICATION.md §11.7.2)
fn bench_gen_check_cold(c: &mut Criterion) {
    let mut group = c.benchmark_group("gen_check_cold");

    // Cold path: different slots accessed non-sequentially
    for count in [10, 100, 1000] {
        group.bench_with_input(
            BenchmarkId::new("scattered_access", count),
            &count,
            |b, &count| {
                let slots: Vec<Slot> = (0..count).map(|_| Slot::new()).collect();
                let gens: Vec<_> = slots.iter().map(|s| s.generation()).collect();

                // Create a shuffled access pattern to simulate cache misses
                let mut indices: Vec<usize> = (0..count).collect();
                // Simple deterministic shuffle
                for i in 0..count {
                    indices.swap(i, (i * 7 + 13) % count);
                }

                b.iter(|| {
                    for &idx in &indices {
                        let _ = black_box(slots[idx].validate(gens[idx]));
                    }
                });
            },
        );
    }

    group.finish();
}

/// Benchmark atomic operations baseline (for future RC benchmarks per SPECIFICATION.md §11.7.2)
fn bench_atomic_operations(c: &mut Criterion) {
    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

    let mut group = c.benchmark_group("atomic_operations");

    // Atomic increment (baseline for rc_increment)
    group.bench_function("atomic_increment", |b| {
        let counter = AtomicUsize::new(0);
        b.iter(|| {
            black_box(counter.fetch_add(1, Ordering::Relaxed))
        });
    });

    // Atomic decrement (baseline for rc_decrement)
    group.bench_function("atomic_decrement", |b| {
        let counter = AtomicUsize::new(1000000);
        b.iter(|| {
            black_box(counter.fetch_sub(1, Ordering::Relaxed))
        });
    });

    // Atomic compare-exchange (for generation updates)
    group.bench_function("atomic_cas", |b| {
        let counter = AtomicU32::new(0);
        let mut expected = 0u32;
        b.iter(|| {
            let result = counter.compare_exchange_weak(
                expected,
                expected + 1,
                Ordering::AcqRel,
                Ordering::Relaxed,
            );
            if let Ok(v) = result {
                expected = v + 1;
            }
            black_box(result)
        });
    });

    // Non-atomic increment (for comparison)
    group.bench_function("non_atomic_increment", |b| {
        let mut counter = 0usize;
        b.iter(|| {
            counter += 1;
            black_box(counter)
        });
    });

    group.finish();
}

/// Benchmark persistent tier allocation (IMPL-007)
fn bench_persistent_allocation(c: &mut Criterion) {
    let mut group = c.benchmark_group("persistent_allocation");

    // Compare persistent vs region allocation
    group.bench_function("persistent_alloc_64", |b| {
        b.iter(|| {
            let result = persistent_alloc(64, 8, 0);
            if let Some((id, _ptr)) = result {
                persistent_decrement(id); // Clean up
            }
            black_box(result)
        });
    });

    group.bench_function("region_alloc_64", |b| {
        let mut region = Region::new(1024 * 1024, 1024 * 1024);
        b.iter(|| {
            black_box(region.allocate(64, 8))
        });
    });

    // Persistent allocator stats
    group.bench_function("allocator_stats", |b| {
        b.iter(|| {
            black_box(persistent_allocator().stats())
        });
    });

    group.finish();
}

/// Benchmark reference counting operations (IMPL-007)
fn bench_refcount_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("refcount_operations");

    // Pre-allocate a slot for refcount benchmarks
    let (id, _ptr) = persistent_alloc(64, 8, 0xBEEF).expect("allocation should succeed");

    // Increment benchmark
    group.bench_function("increment", |b| {
        b.iter(|| {
            black_box(persistent_increment(id))
        });
    });

    // Note: decrement is tested in a way that doesn't free the slot
    // by incrementing before each decrement
    group.bench_function("increment_decrement_cycle", |b| {
        b.iter(|| {
            persistent_increment(id);
            persistent_decrement(id);
        });
    });

    // Clean up the test allocation
    drop(group);
    // Decrement to match initial refcount of 1
    persistent_decrement(id);
}

/// Benchmark tier2 allocator operations (IMPL-007)
fn bench_tier2_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("tier2_operations");

    // Tier2 allocation
    group.bench_function("tier2_alloc_64", |b| {
        b.iter(|| {
            let result = tier2_alloc(64, 8, 0);
            if let Some((id, _ptr)) = result {
                tier2_decrement(id); // Clean up
            }
            black_box(result)
        });
    });

    // Tier2 stats
    group.bench_function("tier2_stats", |b| {
        b.iter(|| {
            black_box(tier2_allocator().stats())
        });
    });

    // Pre-allocate for refcount tests
    if let Some((id, _ptr)) = tier2_alloc(64, 8, 0xCAFE) {
        group.bench_function("tier2_increment", |b| {
            b.iter(|| {
                black_box(tier2_increment(id))
            });
        });

        group.bench_function("tier2_inc_dec_cycle", |b| {
            b.iter(|| {
                tier2_increment(id);
                tier2_decrement(id);
            });
        });

        // Clean up
        drop(group);
        tier2_decrement(id);
    } else {
        group.finish();
    }
}

/// Benchmark cycle collection (IMPL-007)
fn bench_cycle_collection(c: &mut Criterion) {
    let mut group = c.benchmark_group("cycle_collection");

    // Empty collection (baseline)
    group.bench_function("empty_collect", |b| {
        b.iter(|| {
            black_box(collect_cycles(&[]))
        });
    });

    // Collection with roots
    group.bench_function("collect_with_roots", |b| {
        // Allocate some objects as roots
        let mut roots = Vec::new();
        for _ in 0..10 {
            if let Some((id, _ptr)) = persistent_alloc(64, 8, 0) {
                roots.push(id);
            }
        }

        b.iter(|| {
            black_box(collect_cycles(&roots))
        });

        // Clean up
        for id in roots {
            persistent_decrement(id);
        }
    });

    // Collector threshold check
    group.bench_function("should_collect", |b| {
        b.iter(|| {
            black_box(cycle_collector().should_collect())
        });
    });

    group.finish();
}

/// Benchmark generation check vs reference counting overhead (IMPL-007)
/// This compares the cost of generational safety checks vs RC overhead
fn bench_gen_vs_rc_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("gen_vs_rc_overhead");

    // Generation check (Region tier)
    let slot = Slot::new();
    let gen = slot.generation();

    group.bench_function("generation_check", |b| {
        b.iter(|| {
            black_box(slot.validate(gen))
        });
    });

    // Reference counting (Persistent tier) - increment/decrement pair
    if let Some((id, _ptr)) = persistent_alloc(64, 8, 0) {
        group.bench_function("refcount_pair", |b| {
            b.iter(|| {
                persistent_increment(id);
                persistent_decrement(id);
            });
        });

        // Single increment (approximates read cost with atomic)
        group.bench_function("refcount_increment_only", |b| {
            b.iter(|| {
                black_box(persistent_increment(id))
            });
        });

        drop(group);
        persistent_decrement(id);
    } else {
        group.finish();
    }
}

// ============================================================================
// PTR-001: Memory Profiling for Pointer-Heavy Data Structures
// ============================================================================

/// Simulated linked list node with 64-bit raw pointers
#[repr(C)]
struct RawLinkedNode64 {
    data: u64,
    next: *mut RawLinkedNode64,
}

/// Simulated linked list node with 128-bit Blood pointers
#[repr(C)]
struct BloodLinkedNode128 {
    data: u64,
    next: BloodPtr, // 128-bit fat pointer
}

/// Simulated binary tree node with 64-bit raw pointers
#[repr(C)]
struct RawTreeNode64 {
    data: u64,
    left: *mut RawTreeNode64,
    right: *mut RawTreeNode64,
}

/// Simulated binary tree node with 128-bit Blood pointers
#[repr(C)]
struct BloodTreeNode128 {
    data: u64,
    left: BloodPtr,  // 128-bit
    right: BloodPtr, // 128-bit
}

/// Simulated graph node with 64-bit raw pointers (4 neighbors)
#[repr(C)]
struct RawGraphNode64 {
    data: u64,
    neighbors: [*mut RawGraphNode64; 4],
}

/// Simulated graph node with 128-bit Blood pointers (4 neighbors)
#[repr(C)]
struct BloodGraphNode128 {
    data: u64,
    neighbors: [BloodPtr; 4],
}

/// Benchmark memory usage of pointer-heavy data structures (PTR-001)
fn bench_pointer_heavy_memory(c: &mut Criterion) {
    let mut group = c.benchmark_group("pointer_heavy_memory");

    // ---- Size Comparisons ----
    // These benchmarks measure the memory overhead of different pointer sizes

    // Linked list node sizes
    let raw_list_size = std::mem::size_of::<RawLinkedNode64>();
    let blood_list_size = std::mem::size_of::<BloodLinkedNode128>();
    println!("Linked List Node: 64-bit={} bytes, 128-bit={} bytes, overhead={}%",
        raw_list_size, blood_list_size,
        ((blood_list_size as f64 / raw_list_size as f64) - 1.0) * 100.0);

    // Binary tree node sizes
    let raw_tree_size = std::mem::size_of::<RawTreeNode64>();
    let blood_tree_size = std::mem::size_of::<BloodTreeNode128>();
    println!("Binary Tree Node: 64-bit={} bytes, 128-bit={} bytes, overhead={}%",
        raw_tree_size, blood_tree_size,
        ((blood_tree_size as f64 / raw_tree_size as f64) - 1.0) * 100.0);

    // Graph node sizes (4 neighbors)
    let raw_graph_size = std::mem::size_of::<RawGraphNode64>();
    let blood_graph_size = std::mem::size_of::<BloodGraphNode128>();
    println!("Graph Node (4 nbrs): 64-bit={} bytes, 128-bit={} bytes, overhead={}%",
        raw_graph_size, blood_graph_size,
        ((blood_graph_size as f64 / raw_graph_size as f64) - 1.0) * 100.0);

    // ---- Allocation Benchmarks ----

    // Linked list: allocate N nodes
    for count in [100, 1000, 10000] {
        group.throughput(Throughput::Elements(count as u64));

        group.bench_with_input(
            BenchmarkId::new("linked_list_64bit", count),
            &count,
            |b, &count| {
                b.iter(|| {
                    let mut nodes: Vec<Box<RawLinkedNode64>> = Vec::with_capacity(count);
                    for _ in 0..count {
                        nodes.push(Box::new(RawLinkedNode64 {
                            data: 42,
                            next: std::ptr::null_mut(),
                        }));
                    }
                    // Link them together
                    for i in 0..count - 1 {
                        let next_ptr = nodes[i + 1].as_mut() as *mut _;
                        nodes[i].next = next_ptr;
                    }
                    black_box(nodes)
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("linked_list_128bit", count),
            &count,
            |b, &count| {
                b.iter(|| {
                    let mut nodes: Vec<Box<BloodLinkedNode128>> = Vec::with_capacity(count);
                    for _ in 0..count {
                        nodes.push(Box::new(BloodLinkedNode128 {
                            data: 42,
                            next: BloodPtr::null(),
                        }));
                    }
                    // Link them together using BloodPtr
                    for i in 0..count - 1 {
                        let next_addr = nodes[i + 1].as_ref() as *const _ as usize;
                        nodes[i].next = BloodPtr::new(next_addr, generation::FIRST, PointerMetadata::HEAP);
                    }
                    black_box(nodes)
                });
            },
        );
    }

    // Binary tree: allocate N nodes
    for count in [100, 1000, 10000] {
        group.throughput(Throughput::Elements(count as u64));

        group.bench_with_input(
            BenchmarkId::new("binary_tree_64bit", count),
            &count,
            |b, &count| {
                b.iter(|| {
                    let nodes: Vec<Box<RawTreeNode64>> = (0..count)
                        .map(|_| Box::new(RawTreeNode64 {
                            data: 42,
                            left: std::ptr::null_mut(),
                            right: std::ptr::null_mut(),
                        }))
                        .collect();
                    black_box(nodes)
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("binary_tree_128bit", count),
            &count,
            |b, &count| {
                b.iter(|| {
                    let nodes: Vec<Box<BloodTreeNode128>> = (0..count)
                        .map(|_| Box::new(BloodTreeNode128 {
                            data: 42,
                            left: BloodPtr::null(),
                            right: BloodPtr::null(),
                        }))
                        .collect();
                    black_box(nodes)
                });
            },
        );
    }

    // Graph: allocate N nodes with 4 neighbor slots
    for count in [100, 1000] {
        group.throughput(Throughput::Elements(count as u64));

        group.bench_with_input(
            BenchmarkId::new("graph_4neighbors_64bit", count),
            &count,
            |b, &count| {
                b.iter(|| {
                    let nodes: Vec<Box<RawGraphNode64>> = (0..count)
                        .map(|_| Box::new(RawGraphNode64 {
                            data: 42,
                            neighbors: [std::ptr::null_mut(); 4],
                        }))
                        .collect();
                    black_box(nodes)
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("graph_4neighbors_128bit", count),
            &count,
            |b, &count| {
                b.iter(|| {
                    let nodes: Vec<Box<BloodGraphNode128>> = (0..count)
                        .map(|_| Box::new(BloodGraphNode128 {
                            data: 42,
                            neighbors: [BloodPtr::null(); 4],
                        }))
                        .collect();
                    black_box(nodes)
                });
            },
        );
    }

    group.finish();
}

/// Benchmark memory overhead percentage calculation (PTR-001)
fn bench_memory_overhead_analysis(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_overhead_analysis");

    // Raw pointer size vs BloodPtr size
    let raw_ptr_size = std::mem::size_of::<*const ()>();
    let blood_ptr_size = std::mem::size_of::<BloodPtr>();
    let overhead_ratio = blood_ptr_size as f64 / raw_ptr_size as f64;

    println!("\n=== Memory Overhead Analysis (PTR-001) ===");
    println!("Raw pointer size: {} bytes", raw_ptr_size);
    println!("BloodPtr size: {} bytes", blood_ptr_size);
    println!("Pointer overhead ratio: {:.2}x", overhead_ratio);

    // Calculate memory overhead for different pointer densities
    struct DataStructure {
        name: &'static str,
        data_bytes: usize,
        num_pointers: usize,
    }

    let structures = [
        DataStructure { name: "Linked List Node", data_bytes: 8, num_pointers: 1 },
        DataStructure { name: "Doubly-Linked Node", data_bytes: 8, num_pointers: 2 },
        DataStructure { name: "Binary Tree Node", data_bytes: 8, num_pointers: 2 },
        DataStructure { name: "Quad Tree Node", data_bytes: 8, num_pointers: 4 },
        DataStructure { name: "Graph Node (8 edges)", data_bytes: 8, num_pointers: 8 },
        DataStructure { name: "Skip List Node (4 lvl)", data_bytes: 8, num_pointers: 4 },
        DataStructure { name: "B-Tree Node (16 keys)", data_bytes: 128, num_pointers: 17 },
        DataStructure { name: "Trie Node (26 children)", data_bytes: 8, num_pointers: 26 },
    ];

    println!("\n{:<25} {:>12} {:>12} {:>12}", "Structure", "64-bit", "128-bit", "Overhead");
    println!("{:-<25} {:->12} {:->12} {:->12}", "", "", "", "");

    for s in &structures {
        let size_64 = s.data_bytes + s.num_pointers * raw_ptr_size;
        let size_128 = s.data_bytes + s.num_pointers * blood_ptr_size;
        let overhead = ((size_128 as f64 / size_64 as f64) - 1.0) * 100.0;
        println!("{:<25} {:>10} B {:>10} B {:>10.1}%", s.name, size_64, size_128, overhead);
    }

    // Benchmark: Measure actual allocation time vs size
    group.bench_function("blood_ptr_array_1000", |b| {
        b.iter(|| {
            let arr: Vec<BloodPtr> = (0..1000usize)
                .map(|i| BloodPtr::new(i, generation::FIRST, PointerMetadata::HEAP))
                .collect();
            black_box(arr)
        });
    });

    group.bench_function("raw_ptr_array_1000", |b| {
        b.iter(|| {
            let arr: Vec<*const ()> = (0..1000usize)
                .map(|i| i as *const ())
                .collect();
            black_box(arr)
        });
    });

    // Memory bandwidth comparison: copying arrays of pointers
    group.bench_function("copy_blood_ptrs_1000", |b| {
        let src: Vec<BloodPtr> = (0..1000usize)
            .map(|i| BloodPtr::new(i, generation::FIRST, PointerMetadata::HEAP))
            .collect();
        b.iter(|| {
            let dst = src.clone();
            black_box(dst)
        });
    });

    group.bench_function("copy_raw_ptrs_1000", |b| {
        let src: Vec<*const ()> = (0..1000)
            .map(|i| i as *const ())
            .collect();
        b.iter(|| {
            let dst = src.clone();
            black_box(dst)
        });
    });

    group.finish();
}

/// Benchmark cache behavior with different pointer sizes (PTR-001)
fn bench_cache_effects(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_effects");

    // Sequential access pattern - measures cache line utilization
    // 64-bit pointers: 8 per cache line (64B)
    // 128-bit pointers: 4 per cache line (64B)

    for count in [64, 256, 1024, 4096] {
        group.throughput(Throughput::Elements(count as u64));

        group.bench_with_input(
            BenchmarkId::new("sequential_64bit", count),
            &count,
            |b, &count| {
                let ptrs: Vec<*const ()> = (0..count)
                    .map(|i| i as *const ())
                    .collect();
                b.iter(|| {
                    let mut sum = 0usize;
                    for p in &ptrs {
                        sum = sum.wrapping_add(*p as usize);
                    }
                    black_box(sum)
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("sequential_128bit", count),
            &count,
            |b, &count| {
                let ptrs: Vec<BloodPtr> = (0..count)
                    .map(|i| BloodPtr::new(i, generation::FIRST, PointerMetadata::HEAP))
                    .collect();
                b.iter(|| {
                    let mut sum = 0usize;
                    for p in &ptrs {
                        sum = sum.wrapping_add(p.address());
                    }
                    black_box(sum)
                });
            },
        );
    }

    // Random access pattern - measures TLB and cache miss behavior
    for count in [256, 1024, 4096] {
        group.throughput(Throughput::Elements(count as u64));

        // Create a shuffled index pattern
        let mut indices: Vec<usize> = (0..count).collect();
        for i in 0..count {
            indices.swap(i, (i * 7 + 13) % count);
        }

        group.bench_with_input(
            BenchmarkId::new("random_64bit", count),
            &count,
            |b, &count| {
                let ptrs: Vec<*const ()> = (0..count)
                    .map(|i| i as *const ())
                    .collect();
                b.iter(|| {
                    let mut sum = 0usize;
                    for &idx in &indices {
                        sum = sum.wrapping_add(ptrs[idx] as usize);
                    }
                    black_box(sum)
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("random_128bit", count),
            &count,
            |b, &count| {
                let ptrs: Vec<BloodPtr> = (0..count)
                    .map(|i| BloodPtr::new(i, generation::FIRST, PointerMetadata::HEAP))
                    .collect();
                b.iter(|| {
                    let mut sum = 0usize;
                    for &idx in &indices {
                        sum = sum.wrapping_add(ptrs[idx].address());
                    }
                    black_box(sum)
                });
            },
        );
    }

    group.finish();
}

// =============================================================================
// Memory Pressure Benchmarks (PERF-004)
// =============================================================================
// These benchmarks test behavior under memory pressure scenarios:
// - Near-capacity allocation
// - High fragmentation
// - Allocation/deallocation churn
// - Working set pressure

/// Benchmark allocation under near-capacity conditions (PERF-004)
fn bench_memory_pressure_allocation(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_pressure_allocation");

    // Test allocation performance when region is nearly full
    // This simulates real-world scenarios where memory is constrained

    // Scenario 1: Allocate until 90% capacity, then benchmark additional allocations
    group.bench_function("near_capacity_small_allocs", |b| {
        // Create a region with limited capacity
        let mut region = Region::new(64 * 1024, 64 * 1024); // 64KB region

        // Fill to 90% capacity with 64-byte blocks
        let fill_count = (64 * 1024 * 9) / (64 * 10); // ~90% of 64KB / 64 bytes per alloc
        for _ in 0..fill_count {
            let _ = region.allocate(64, 8);
        }

        b.iter(|| {
            // Measure allocation time when nearly full
            let ptr = region.allocate(64, 8);
            black_box(ptr)
        });
    });

    // Scenario 2: Allocation after fragmentation
    group.bench_function("fragmented_allocation", |b| {
        let mut region = Region::new(256 * 1024, 256 * 1024); // 256KB region
        let mut ptrs = Vec::new();

        // Allocate many small blocks
        for _ in 0..1000 {
            if let Some(ptr) = region.allocate(64, 8) {
                ptrs.push(ptr);
            }
        }

        // Free every other block to create fragmentation
        for i in (0..ptrs.len()).step_by(2) {
            // Note: Region doesn't support individual deallocation, so this simulates
            // the scenario where slots are freed but memory is fragmented
        }

        b.iter(|| {
            // Allocate in fragmented region
            let ptr = region.allocate(128, 8); // Slightly larger allocation
            black_box(ptr)
        });
    });

    group.finish();
}

/// Benchmark allocation/deallocation churn (PERF-004)
fn bench_memory_pressure_churn(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_pressure_churn");

    // Scenario: Rapid alloc/dealloc cycles typical of real programs
    for size in [16, 64, 256, 1024] {
        group.bench_with_input(
            BenchmarkId::new("slot_churn", size),
            &size,
            |b, &size| {
                let slot = Slot::new();
                let layout = Layout::from_size_align(size, 8).unwrap();
                b.iter(|| {
                    unsafe {
                        // Allocate and immediately deallocate - simulates temporary objects
                        let ptr = slot.allocate(layout);
                        black_box(ptr);
                        slot.deallocate();
                    }
                });
            },
        );
    }

    // Mixed size allocation pattern (more realistic)
    group.bench_function("mixed_size_churn", |b| {
        let slots: Vec<Slot> = (0..100).map(|_| Slot::new()).collect();
        let sizes = [16, 32, 64, 128, 256, 512, 1024, 2048];

        b.iter(|| {
            for (i, slot) in slots.iter().enumerate() {
                let size = sizes[i % sizes.len()];
                let layout = Layout::from_size_align(size, 8).unwrap();
                unsafe {
                    let ptr = slot.allocate(layout);
                    black_box(ptr);
                    slot.deallocate();
                }
            }
        });
    });

    group.finish();
}

/// Benchmark generation validation under pressure (PERF-004)
fn bench_memory_pressure_generation_checks(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_pressure_gen_checks");

    // Simulate heavy pointer validation load
    // Real programs may have many live references to validate

    for ptr_count in [100, 1000, 10000] {
        // Create many slots and pointers
        let slots: Vec<Slot> = (0..ptr_count).map(|_| Slot::new()).collect();
        let ptrs: Vec<BloodPtr> = slots.iter()
            .enumerate()
            .map(|(i, slot)| BloodPtr::new(
                i * 1000,  // Simulated address
                slot.generation(),
                PointerMetadata::HEAP,
            ))
            .collect();

        group.throughput(Throughput::Elements(ptr_count as u64));

        group.bench_with_input(
            BenchmarkId::new("validate_all", ptr_count),
            &(slots.clone(), ptrs.clone()),
            |b, (slots, ptrs)| {
                b.iter(|| {
                    let mut valid_count = 0usize;
                    for (slot, ptr) in slots.iter().zip(ptrs.iter()) {
                        if slot.validate(ptr.generation()) {
                            valid_count += 1;
                        }
                    }
                    black_box(valid_count)
                });
            },
        );
    }

    // Benchmark with stale pointers (invalid generations)
    group.bench_function("validate_stale_pointers", |b| {
        let slot = Slot::new();
        let old_gen = slot.generation();
        let layout = Layout::from_size_align(64, 8).unwrap();

        // Allocate and deallocate to invalidate generation
        unsafe {
            let _ = slot.allocate(layout);
            slot.deallocate();
        }

        let stale_ptr = BloodPtr::new(0x1000, old_gen, PointerMetadata::HEAP);

        b.iter(|| {
            // Validate stale pointer - should fail quickly
            let valid = slot.validate(stale_ptr.generation());
            black_box(valid)
        });
    });

    group.finish();
}

/// Benchmark persistent tier under memory pressure (PERF-004)
fn bench_memory_pressure_persistent_tier(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_pressure_persistent");

    // Scenario: Many objects promoted to persistent tier with high refcount churn
    for count in [100, 500, 1000] {
        group.bench_with_input(
            BenchmarkId::new("increment_decrement_churn", count),
            &count,
            |b, &count| {
                // Allocate persistent objects
                let layout = Layout::from_size_align(64, 8).unwrap();
                let ptrs: Vec<_> = (0..count)
                    .filter_map(|_| persistent_alloc(layout))
                    .collect();

                b.iter(|| {
                    // Heavy refcount churn - increment and decrement all
                    for ptr in &ptrs {
                        persistent_increment(*ptr);
                    }
                    for ptr in &ptrs {
                        persistent_decrement(*ptr);
                    }
                    black_box(ptrs.len())
                });

                // Cleanup
                for ptr in ptrs {
                    persistent_decrement(ptr);
                }
            },
        );
    }

    // Cycle collection under pressure
    group.bench_function("cycle_collection_pressure", |b| {
        let layout = Layout::from_size_align(64, 8).unwrap();

        b.iter(|| {
            // Create objects that might form cycles
            let mut ptrs = Vec::new();
            for _ in 0..50 {
                if let Some(ptr) = persistent_alloc(layout) {
                    ptrs.push(ptr);
                }
            }

            // Simulate cycle creation (in reality would set fields)
            // For benchmarking, we just create the objects

            // Run collection
            let collected = collect_cycles();
            black_box(collected);

            // Cleanup
            for ptr in ptrs {
                persistent_decrement(ptr);
            }
        });
    });

    group.finish();
}

/// Benchmark snapshot creation under pressure (PERF-004)
fn bench_memory_pressure_snapshots(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_pressure_snapshots");

    // Scenarios with many live references (deep handler nesting, large working sets)
    for reference_count in [10, 50, 100, 500] {
        group.bench_with_input(
            BenchmarkId::new("snapshot_many_refs", reference_count),
            &reference_count,
            |b, &count| {
                // Create slots and their pointers
                let slots: Vec<Slot> = (0..count).map(|_| Slot::new()).collect();
                let generations: Vec<u32> = slots.iter().map(|s| s.generation()).collect();

                b.iter(|| {
                    // Create snapshot capturing all references
                    let snapshot = GenerationSnapshot::capture(&generations);
                    black_box(snapshot)
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("snapshot_validate_all", reference_count),
            &reference_count,
            |b, &count| {
                let slots: Vec<Slot> = (0..count).map(|_| Slot::new()).collect();
                let generations: Vec<u32> = slots.iter().map(|s| s.generation()).collect();
                let snapshot = GenerationSnapshot::capture(&generations);

                b.iter(|| {
                    // Validate all references in snapshot
                    let mut all_valid = true;
                    for (slot, &gen) in slots.iter().zip(generations.iter()) {
                        all_valid &= slot.validate(gen);
                    }
                    black_box(all_valid)
                });
            },
        );
    }

    group.finish();
}

/// Benchmark working set size effects (PERF-004)
fn bench_memory_pressure_working_set(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_pressure_working_set");
    group.sample_size(10); // Fewer samples for memory-intensive benchmarks

    // Different working set sizes to measure cache effects
    for kb in [64, 256, 1024, 4096] {
        let element_count = (kb * 1024) / std::mem::size_of::<BloodPtr>();

        group.throughput(Throughput::Bytes((kb * 1024) as u64));

        group.bench_with_input(
            BenchmarkId::new("sequential_access_kb", kb),
            &element_count,
            |b, &count| {
                let ptrs: Vec<BloodPtr> = (0..count)
                    .map(|i| BloodPtr::new(i, generation::FIRST, PointerMetadata::HEAP))
                    .collect();

                b.iter(|| {
                    let mut sum = 0usize;
                    for ptr in &ptrs {
                        sum = sum.wrapping_add(ptr.address());
                        // Also check generation (simulates real usage)
                        black_box(ptr.generation());
                    }
                    black_box(sum)
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("random_access_kb", kb),
            &element_count,
            |b, &count| {
                let ptrs: Vec<BloodPtr> = (0..count)
                    .map(|i| BloodPtr::new(i, generation::FIRST, PointerMetadata::HEAP))
                    .collect();

                // Generate pseudo-random access pattern
                let indices: Vec<usize> = (0..count)
                    .map(|i| (i * 2654435761) % count) // Knuth's multiplicative hash
                    .collect();

                b.iter(|| {
                    let mut sum = 0usize;
                    for &idx in &indices {
                        let ptr = &ptrs[idx];
                        sum = sum.wrapping_add(ptr.address());
                        black_box(ptr.generation());
                    }
                    black_box(sum)
                });
            },
        );
    }

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
    bench_gen_check_hot,
    bench_gen_check_cold,
    bench_atomic_operations,
    // Persistent tier benchmarks (IMPL-007)
    bench_persistent_allocation,
    bench_refcount_operations,
    bench_tier2_operations,
    bench_cycle_collection,
    bench_gen_vs_rc_overhead,
    // Pointer-heavy data structure profiling (PTR-001)
    bench_pointer_heavy_memory,
    bench_memory_overhead_analysis,
    bench_cache_effects,
    // Memory pressure benchmarks (PERF-004)
    bench_memory_pressure_allocation,
    bench_memory_pressure_churn,
    bench_memory_pressure_generation_checks,
    bench_memory_pressure_persistent_tier,
    bench_memory_pressure_snapshots,
    bench_memory_pressure_working_set,
);
criterion_main!(benches);
