//! Runtime performance benchmarks using criterion.
//!
//! Benchmarks for generational reference checks, memory allocation,
//! and other runtime operations.
//!
//! Run with: cargo bench --bench runtime_bench
//!
//! PERF-001: Benchmark generation check in compiled Blood code

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};

/// Simulates the generation check operation inline (for comparison).
/// This represents the theoretical minimum cost with direct comparison.
#[inline(always)]
fn inline_generation_check(expected: u32, actual: u32) -> bool {
    // Persistent generation (0xFFFFFFFF) is always valid
    if expected == 0xFFFFFFFF {
        return true;
    }
    expected == actual
}

/// Simulates the slot lookup operation.
/// The real implementation uses a hash table with linear probing.
struct SlotEntry {
    address: u64,
    generation: u32,
    _size: u32,
    in_use: bool,
}

const SLOT_REGISTRY_SIZE: usize = 16384;

struct SlotRegistry {
    slots: Vec<SlotEntry>,
}

impl SlotRegistry {
    fn new() -> Self {
        let mut slots = Vec::with_capacity(SLOT_REGISTRY_SIZE);
        for _ in 0..SLOT_REGISTRY_SIZE {
            slots.push(SlotEntry {
                address: 0,
                generation: 0,
                _size: 0,
                in_use: false,
            });
        }
        SlotRegistry { slots }
    }

    #[inline]
    fn hash(addr: u64) -> usize {
        ((addr >> 3) ^ (addr >> 17)) as usize % SLOT_REGISTRY_SIZE
    }

    fn register(&mut self, addr: u64, gen: u32) {
        let mut idx = Self::hash(addr);
        let start = idx;
        loop {
            if !self.slots[idx].in_use {
                self.slots[idx] = SlotEntry {
                    address: addr,
                    generation: gen,
                    _size: 8,
                    in_use: true,
                };
                return;
            }
            idx = (idx + 1) % SLOT_REGISTRY_SIZE;
            if idx == start {
                panic!("Slot registry full");
            }
        }
    }

    #[inline]
    fn find_slot(&self, addr: u64) -> Option<&SlotEntry> {
        let mut idx = Self::hash(addr);
        let start = idx;
        loop {
            if self.slots[idx].in_use && self.slots[idx].address == addr {
                return Some(&self.slots[idx]);
            }
            if !self.slots[idx].in_use {
                return None;
            }
            idx = (idx + 1) % SLOT_REGISTRY_SIZE;
            if idx == start {
                return None;
            }
        }
    }

    /// Validate generation (matches blood_validate_generation semantics)
    #[inline]
    fn validate_generation(&self, addr: u64, expected_gen: i32) -> i32 {
        // Returns 0 if valid, 1 if stale
        if expected_gen as u32 == 0xFFFFFFFF {
            return 0; // GENERATION_PERSISTENT always valid
        }
        if addr == 0 {
            return 1; // Null is stale
        }

        if let Some(slot) = self.find_slot(addr) {
            if slot.generation == expected_gen as u32 {
                return 0; // Valid
            }
        }
        1 // Stale or not found
    }
}

fn bench_inline_generation_check(c: &mut Criterion) {
    let mut group = c.benchmark_group("generation_check_inline");

    // Benchmark the theoretical minimum: just a comparison
    group.bench_function("direct_comparison", |b| {
        let expected = black_box(42u32);
        let actual = black_box(42u32);
        b.iter(|| inline_generation_check(expected, actual))
    });

    // Benchmark with persistent generation
    group.bench_function("persistent_generation", |b| {
        let expected = black_box(0xFFFFFFFFu32);
        let actual = black_box(42u32);
        b.iter(|| inline_generation_check(expected, actual))
    });

    group.finish();
}

fn bench_slot_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("generation_check_slot_lookup");

    // Set up a slot registry with some entries
    let mut registry = SlotRegistry::new();

    // Register 1000 addresses
    for i in 0..1000u64 {
        let addr = 0x1000 + i * 16; // Simulate aligned allocations
        registry.register(addr, (i as u32) + 1);
    }

    // Benchmark lookup of existing address (fast path)
    let test_addr = 0x1000u64 + 500 * 16;
    let test_gen = 501i32;

    group.bench_function("existing_address", |b| {
        b.iter(|| registry.validate_generation(black_box(test_addr), black_box(test_gen)))
    });

    // Benchmark lookup of non-existing address
    let missing_addr = 0xDEADBEEF_u64;
    group.bench_function("missing_address", |b| {
        b.iter(|| registry.validate_generation(black_box(missing_addr), black_box(1)))
    });

    // Benchmark persistent generation (skips lookup)
    group.bench_function("persistent_skip_lookup", |b| {
        b.iter(|| registry.validate_generation(black_box(test_addr), black_box(-1i32)))
    });

    // Benchmark with generation mismatch (stale reference)
    group.bench_function("stale_reference", |b| {
        b.iter(|| registry.validate_generation(black_box(test_addr), black_box(999)))
    });

    group.finish();
}

fn bench_slot_registry_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("slot_registry_scaling");

    // Test how lookup performance scales with number of entries
    for count in [100, 500, 1000, 5000, 10000].iter() {
        let mut registry = SlotRegistry::new();

        for i in 0..*count as u64 {
            let addr = 0x1000 + i * 16;
            registry.register(addr, (i as u32) + 1);
        }

        // Lookup address in the middle of the range
        let test_addr = 0x1000u64 + (*count / 2) as u64 * 16;
        let test_gen = (*count / 2 + 1) as i32;

        group.bench_with_input(
            BenchmarkId::new("lookup", count),
            count,
            |b, _| {
                b.iter(|| registry.validate_generation(black_box(test_addr), black_box(test_gen)))
            }
        );
    }

    group.finish();
}

/// Benchmark the full dereference path cost estimation.
/// This simulates what happens during a pointer dereference in Blood.
fn bench_dereference_simulation(c: &mut Criterion) {
    let mut group = c.benchmark_group("dereference_simulation");

    let mut registry = SlotRegistry::new();

    // Register some allocations
    for i in 0..1000u64 {
        let addr = 0x1000 + i * 16;
        registry.register(addr, (i as u32) + 1);
    }

    // Simulate a "fat pointer" (128-bit)
    struct FatPointer {
        address: u64,
        generation: u32,
        _metadata: u32,
    }

    let ptr = FatPointer {
        address: 0x1000 + 500 * 16,
        generation: 501,
        _metadata: 0,
    };

    // Benchmark the full dereference sequence:
    // 1. Extract address and generation from fat pointer
    // 2. Validate generation via slot lookup
    // 3. If valid, return address for actual memory access
    group.bench_function("full_dereference_check", |b| {
        b.iter(|| {
            let addr = black_box(ptr.address);
            let gen = black_box(ptr.generation as i32);
            let result = registry.validate_generation(addr, gen);
            if result != 0 {
                // Would panic in real code
                black_box(0u64)
            } else {
                black_box(addr)
            }
        })
    });

    // Benchmark tier-0 (stack) path: no generation check needed
    group.bench_function("stack_dereference_no_check", |b| {
        let stack_value = black_box(42i64);
        b.iter(|| {
            // Just access the value directly - no generation check
            black_box(stack_value)
        })
    });

    group.finish();
}

// ============================================================================
// PERF-002: Effect Handler Overhead Benchmarks
// ============================================================================

/// Evidence entry for effect handler dispatch.
/// Stores handler index and state pointer.
#[derive(Clone)]
struct EvidenceEntry {
    registry_index: u64,
    state: *mut u8,
}

/// Evidence vector - stack of active effect handlers.
struct EvidenceVector {
    entries: Vec<EvidenceEntry>,
}

impl EvidenceVector {
    fn new() -> Self {
        EvidenceVector { entries: Vec::with_capacity(8) }
    }

    #[inline]
    fn push(&mut self, registry_index: u64, state: *mut u8) {
        self.entries.push(EvidenceEntry { registry_index, state });
    }

    #[inline]
    fn pop(&mut self) -> Option<EvidenceEntry> {
        self.entries.pop()
    }

    #[inline]
    fn find_handler(&self, effect_id: u64) -> Option<&EvidenceEntry> {
        // Linear search from top of stack (most recent handler first)
        self.entries.iter().rev().find(|e| e.registry_index == effect_id)
    }

    #[inline]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Simulated effect handler registry entry.
#[derive(Clone)]
struct HandlerRegistryEntry {
    effect_id: i64,
    op_count: usize,
    ops: Vec<fn(i64) -> i64>, // Simulated operation function pointers
}

/// Simulated effect handler registry.
struct HandlerRegistry {
    entries: Vec<HandlerRegistryEntry>,
}

impl HandlerRegistry {
    fn new() -> Self {
        HandlerRegistry { entries: Vec::with_capacity(32) }
    }

    fn register(&mut self, effect_id: i64, op_count: usize) -> usize {
        let ops: Vec<fn(i64) -> i64> = (0..op_count).map(|_| identity_op as fn(i64) -> i64).collect();
        let index = self.entries.len();
        self.entries.push(HandlerRegistryEntry { effect_id, op_count, ops });
        index
    }

    #[inline]
    fn lookup_by_effect(&self, effect_id: i64) -> Option<&HandlerRegistryEntry> {
        self.entries.iter().find(|e| e.effect_id == effect_id)
    }

    #[inline]
    fn get(&self, index: usize) -> Option<&HandlerRegistryEntry> {
        self.entries.get(index)
    }
}

/// Identity operation for benchmarking.
fn identity_op(x: i64) -> i64 { x }

/// Simulated continuation for effect handlers.
struct SimulatedContinuation {
    id: u64,
    callback: fn(i64) -> i64,
    consumed: bool,
}

impl SimulatedContinuation {
    fn new(id: u64) -> Self {
        SimulatedContinuation {
            id,
            callback: identity_op,
            consumed: false,
        }
    }

    #[inline]
    fn resume(&mut self, value: i64) -> i64 {
        self.consumed = true;
        (self.callback)(value)
    }
}

/// Continuation registry for multi-shot handlers.
struct ContinuationRegistry {
    continuations: std::collections::HashMap<u64, SimulatedContinuation>,
    next_id: u64,
}

impl ContinuationRegistry {
    fn new() -> Self {
        ContinuationRegistry {
            continuations: std::collections::HashMap::new(),
            next_id: 1,
        }
    }

    #[inline]
    fn create(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.continuations.insert(id, SimulatedContinuation::new(id));
        id
    }

    #[inline]
    fn take(&mut self, id: u64) -> Option<SimulatedContinuation> {
        self.continuations.remove(&id)
    }

    #[inline]
    fn clone_continuation(&mut self, id: u64) -> Option<u64> {
        if self.continuations.contains_key(&id) {
            let new_id = self.next_id;
            self.next_id += 1;
            self.continuations.insert(new_id, SimulatedContinuation::new(new_id));
            Some(new_id)
        } else {
            None
        }
    }
}

fn bench_evidence_vector_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("effect_evidence_vector");

    // Benchmark evidence vector creation
    group.bench_function("create", |b| {
        b.iter(|| {
            black_box(EvidenceVector::new())
        })
    });

    // Benchmark push operation
    group.bench_function("push", |b| {
        let mut ev = EvidenceVector::new();
        b.iter(|| {
            ev.push(black_box(1), std::ptr::null_mut());
            // Keep from growing unboundedly
            if ev.len() > 100 {
                ev.entries.clear();
            }
        })
    });

    // Benchmark pop operation
    group.bench_function("pop", |b| {
        b.iter_batched(
            || {
                let mut ev = EvidenceVector::new();
                for i in 0..10 {
                    ev.push(i, std::ptr::null_mut());
                }
                ev
            },
            |mut ev| {
                black_box(ev.pop())
            },
            criterion::BatchSize::SmallInput,
        )
    });

    // Benchmark handler lookup (shallow stack)
    group.bench_function("lookup_depth_3", |b| {
        let mut ev = EvidenceVector::new();
        for i in 0..3 {
            ev.push(i, std::ptr::null_mut());
        }
        b.iter(|| {
            black_box(ev.find_handler(black_box(1)))
        })
    });

    // Benchmark handler lookup (deep stack)
    group.bench_function("lookup_depth_10", |b| {
        let mut ev = EvidenceVector::new();
        for i in 0..10 {
            ev.push(i, std::ptr::null_mut());
        }
        b.iter(|| {
            black_box(ev.find_handler(black_box(5)))
        })
    });

    group.finish();
}

fn bench_handler_registry_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("effect_handler_registry");

    // Set up a registry with some handlers
    let mut registry = HandlerRegistry::new();
    for i in 0..20 {
        registry.register(i, 3); // 3 ops per handler
    }

    // Benchmark handler lookup by effect ID
    group.bench_function("lookup_by_effect", |b| {
        b.iter(|| {
            black_box(registry.lookup_by_effect(black_box(10)))
        })
    });

    // Benchmark handler lookup by index (direct)
    group.bench_function("lookup_by_index", |b| {
        b.iter(|| {
            black_box(registry.get(black_box(10)))
        })
    });

    // Benchmark the full handler dispatch path:
    // 1. Look up handler by effect ID
    // 2. Get operation function pointer
    // 3. Call operation
    group.bench_function("full_dispatch", |b| {
        b.iter(|| {
            let effect_id = black_box(10i64);
            let op_index = black_box(1usize);
            let arg = black_box(42i64);

            if let Some(handler) = registry.lookup_by_effect(effect_id) {
                if let Some(op) = handler.ops.get(op_index) {
                    black_box(op(arg))
                } else {
                    0
                }
            } else {
                0
            }
        })
    });

    group.finish();
}

fn bench_continuation_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("effect_continuation");

    // Benchmark continuation creation
    group.bench_function("create", |b| {
        let mut registry = ContinuationRegistry::new();
        b.iter(|| {
            black_box(registry.create())
        })
    });

    // Benchmark continuation resume (one-shot)
    group.bench_function("resume_oneshot", |b| {
        b.iter_batched(
            || SimulatedContinuation::new(1),
            |mut cont| {
                black_box(cont.resume(black_box(42)))
            },
            criterion::BatchSize::SmallInput,
        )
    });

    // Benchmark continuation clone (for multi-shot handlers)
    group.bench_function("clone_multishot", |b| {
        let mut registry = ContinuationRegistry::new();
        let original_id = registry.create();
        b.iter(|| {
            black_box(registry.clone_continuation(black_box(original_id)))
        })
    });

    // Benchmark continuation take from registry
    group.bench_function("take_from_registry", |b| {
        b.iter_batched(
            || {
                let mut registry = ContinuationRegistry::new();
                let id = registry.create();
                (registry, id)
            },
            |(mut registry, id)| {
                black_box(registry.take(id))
            },
            criterion::BatchSize::SmallInput,
        )
    });

    group.finish();
}

fn bench_effect_handle_expression(c: &mut Criterion) {
    let mut group = c.benchmark_group("effect_handle_expression");

    // Benchmark the full handle { body } with { handler } sequence:
    // 1. Save current evidence
    // 2. Create new evidence vector
    // 3. Push handler
    // 4. Set as current
    // 5. Execute body (simulated)
    // 6. Pop handler
    // 7. Restore evidence
    // 8. Destroy evidence

    let mut handler_registry = HandlerRegistry::new();
    handler_registry.register(1, 3);

    group.bench_function("full_handle_overhead", |b| {
        b.iter(|| {
            // Simulate the full handle expression overhead

            // 1. Save current evidence (just a pointer copy)
            let _saved_ev: Option<*const EvidenceVector> = None;

            // 2. Create new evidence vector
            let mut ev = EvidenceVector::new();

            // 3. Push handler
            ev.push(black_box(1), std::ptr::null_mut());

            // 4. Set as current (simulated)
            let _current = &ev;

            // 5. Body execution (simulated as just returning a value)
            let body_result = black_box(42i64);

            // 6. Pop handler
            let _ = ev.pop();

            // 7. Restore evidence (simulated)
            // 8. Destroy evidence (happens via Drop)

            black_box(body_result)
        })
    });

    // Benchmark nested handler overhead
    group.bench_function("nested_handlers_depth_3", |b| {
        b.iter(|| {
            let mut ev = EvidenceVector::new();

            // Push 3 nested handlers
            for i in 0..3u64 {
                ev.push(black_box(i), std::ptr::null_mut());
            }

            // Body execution
            let body_result = black_box(42i64);

            // Pop all handlers
            for _ in 0..3 {
                let _ = ev.pop();
            }

            black_box(body_result)
        })
    });

    group.finish();
}

fn bench_tail_resumptive_vs_continuation(c: &mut Criterion) {
    let mut group = c.benchmark_group("effect_resume_strategies");

    // Tail-resumptive: just a function return (optimized path)
    group.bench_function("tail_resumptive", |b| {
        b.iter(|| {
            // Tail-resumptive handlers just return the value
            let resume_value = black_box(42i64);
            black_box(resume_value)
        })
    });

    // Continuation-based: requires registry lookup and resume
    group.bench_function("continuation_based", |b| {
        b.iter_batched(
            || {
                let mut registry = ContinuationRegistry::new();
                let id = registry.create();
                (registry, id)
            },
            |(mut registry, id)| {
                let cont = registry.take(id).unwrap();
                let mut cont = cont;
                black_box(cont.resume(black_box(42)))
            },
            criterion::BatchSize::SmallInput,
        )
    });

    group.finish();
}

// ============================================================================
// PERF-003 & PTR-006: 128-bit vs 64-bit Pointer Benchmarks
// ============================================================================

/// 64-bit thin pointer (standard Rust pointer)
#[repr(C)]
struct ThinPointer64 {
    address: u64,
}

/// 128-bit fat pointer with generation (Blood's generational pointer)
#[repr(C)]
struct FatPointer128 {
    address: u64,
    generation: u32,
    _metadata: u32,
}

/// Linked list node with 64-bit pointers
struct ListNode64 {
    value: i64,
    next: Option<Box<ListNode64>>,
}

/// Linked list node with 128-bit pointers (simulated)
struct ListNode128 {
    value: i64,
    next_address: u64,
    next_generation: u32,
    _padding: u32,
}

impl ListNode64 {
    fn new(value: i64) -> Self {
        ListNode64 { value, next: None }
    }

    fn append(&mut self, value: i64) {
        match &mut self.next {
            Some(next) => next.append(value),
            None => self.next = Some(Box::new(ListNode64::new(value))),
        }
    }
}

/// Binary tree node with 64-bit pointers
struct TreeNode64 {
    value: i64,
    left: Option<Box<TreeNode64>>,
    right: Option<Box<TreeNode64>>,
}

/// Binary tree node with 128-bit pointers (simulated)
struct TreeNode128 {
    value: i64,
    left_address: u64,
    left_generation: u32,
    right_address: u64,
    right_generation: u32,
    _padding: u32,
}

impl TreeNode64 {
    fn new(value: i64) -> Self {
        TreeNode64 { value, left: None, right: None }
    }

    fn insert(&mut self, value: i64) {
        if value < self.value {
            match &mut self.left {
                Some(left) => left.insert(value),
                None => self.left = Some(Box::new(TreeNode64::new(value))),
            }
        } else {
            match &mut self.right {
                Some(right) => right.insert(value),
                None => self.right = Some(Box::new(TreeNode64::new(value))),
            }
        }
    }

    fn sum(&self) -> i64 {
        let mut total = self.value;
        if let Some(left) = &self.left {
            total += left.sum();
        }
        if let Some(right) = &self.right {
            total += right.sum();
        }
        total
    }
}

fn bench_pointer_size_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("pointer_size_comparison");

    // Compare memory footprint (measured indirectly via allocation)
    group.bench_function("thin_pointer_64_size", |b| {
        b.iter(|| {
            black_box(std::mem::size_of::<ThinPointer64>())
        })
    });

    group.bench_function("fat_pointer_128_size", |b| {
        b.iter(|| {
            black_box(std::mem::size_of::<FatPointer128>())
        })
    });

    // Compare node sizes
    group.bench_function("list_node_64_size", |b| {
        b.iter(|| {
            black_box(std::mem::size_of::<ListNode64>())
        })
    });

    group.bench_function("list_node_128_size", |b| {
        b.iter(|| {
            black_box(std::mem::size_of::<ListNode128>())
        })
    });

    group.bench_function("tree_node_64_size", |b| {
        b.iter(|| {
            black_box(std::mem::size_of::<TreeNode64>())
        })
    });

    group.bench_function("tree_node_128_size", |b| {
        b.iter(|| {
            black_box(std::mem::size_of::<TreeNode128>())
        })
    });

    group.finish();
}

fn bench_linked_list_traversal(c: &mut Criterion) {
    let mut group = c.benchmark_group("linked_list_traversal");

    // Build lists of various sizes
    for size in [100, 1000, 10000].iter() {
        // Build 64-bit pointer list
        let mut head = ListNode64::new(0);
        for i in 1..*size {
            head.append(i);
        }

        let bench_name = format!("traverse_64bit_{}", size);
        group.bench_function(&bench_name, |b| {
            b.iter(|| {
                let mut sum = 0i64;
                let mut current: &ListNode64 = &head;
                sum += current.value;
                while let Some(ref next) = current.next {
                    current = next;
                    sum += current.value;
                }
                black_box(sum)
            })
        });
    }

    // Simulate 128-bit pointer overhead with generation check
    let mut registry = SlotRegistry::new();

    for size in [100, 1000, 10000].iter() {
        // Register allocations for each node
        for i in 0..*size as u64 {
            let addr = 0x10000 + i * 32; // 32 bytes per node (with 128-bit pointers)
            registry.register(addr, (i as u32) + 1);
        }

        let bench_name = format!("traverse_128bit_with_gen_check_{}", size);
        group.bench_with_input(
            BenchmarkId::new("traverse_128bit_with_gen_check", size),
            size,
            |b, &size| {
                b.iter(|| {
                    let mut sum = 0i64;
                    for i in 0..size as u64 {
                        let addr = black_box(0x10000 + i * 32);
                        let expected_gen = black_box((i as i32) + 1);
                        // Simulate generation check for each pointer dereference
                        let result = registry.validate_generation(addr, expected_gen);
                        if result == 0 {
                            sum += i as i64; // Simulated value read
                        }
                    }
                    black_box(sum)
                })
            }
        );
    }

    group.finish();
}

fn bench_tree_traversal(c: &mut Criterion) {
    let mut group = c.benchmark_group("tree_traversal");

    // Build trees of various depths
    for depth in [5, 10, 15].iter() {
        let node_count = (1i64 << *depth) - 1; // 2^depth - 1 nodes

        // Build 64-bit pointer tree
        let mut root = TreeNode64::new(node_count / 2);
        // Insert values to create a balanced-ish tree
        for i in 0..node_count {
            if i != node_count / 2 {
                root.insert(i);
            }
        }

        let bench_name = format!("sum_64bit_depth_{}", depth);
        group.bench_function(&bench_name, |b| {
            b.iter(|| {
                black_box(root.sum())
            })
        });
    }

    // Simulate 128-bit pointer tree traversal overhead
    let mut registry = SlotRegistry::new();

    for depth in [5, 10, 15].iter() {
        let node_count = (1usize << *depth) - 1;

        // Register allocations for tree nodes
        for i in 0..std::cmp::min(node_count, SLOT_REGISTRY_SIZE / 2) {
            let addr = 0x20000 + i as u64 * 40; // 40 bytes per tree node with 128-bit pointers
            registry.register(addr, (i as u32) + 1);
        }

        let bench_name = format!("sum_128bit_with_gen_check_depth_{}", depth);
        group.bench_with_input(
            BenchmarkId::new("sum_128bit_with_gen_check", depth),
            &node_count,
            |b, &node_count| {
                b.iter(|| {
                    let mut sum = 0i64;
                    // Simulate tree traversal with generation checks
                    for i in 0..std::cmp::min(node_count, SLOT_REGISTRY_SIZE / 2) {
                        let addr = black_box(0x20000 + i as u64 * 40);
                        let expected_gen = black_box((i as i32) + 1);
                        // Two generation checks per node (left and right pointers)
                        let _ = registry.validate_generation(addr, expected_gen);
                        let _ = registry.validate_generation(addr, expected_gen);
                        sum += i as i64;
                    }
                    black_box(sum)
                })
            }
        );
    }

    group.finish();
}

fn bench_cache_efficiency(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_efficiency");

    // Measure array traversal with different pointer sizes
    // 64-bit: 8 pointers per cache line (64 bytes)
    // 128-bit: 4 pointers per cache line
    let cache_line_size = 64;

    // Array of 64-bit values (8 per cache line)
    let array_64: Vec<u64> = (0..10000).map(|i| i as u64).collect();

    group.bench_function("array_64bit_sequential", |b| {
        b.iter(|| {
            let mut sum = 0u64;
            for &val in &array_64 {
                sum += val;
            }
            black_box(sum)
        })
    });

    // Array of 128-bit values (4 per cache line)
    let array_128: Vec<FatPointer128> = (0..10000)
        .map(|i| FatPointer128 { address: i as u64, generation: (i as u32), _metadata: 0 })
        .collect();

    group.bench_function("array_128bit_sequential", |b| {
        b.iter(|| {
            let mut sum = 0u64;
            for ptr in &array_128 {
                sum += ptr.address;
            }
            black_box(sum)
        })
    });

    // Calculate theoretical cache line efficiency
    let ptrs_per_line_64 = cache_line_size / std::mem::size_of::<u64>();
    let ptrs_per_line_128 = cache_line_size / std::mem::size_of::<FatPointer128>();

    group.bench_function("cache_line_capacity_64bit", |b| {
        b.iter(|| {
            black_box(ptrs_per_line_64)
        })
    });

    group.bench_function("cache_line_capacity_128bit", |b| {
        b.iter(|| {
            black_box(ptrs_per_line_128)
        })
    });

    group.finish();
}

/// EFF-004: Profile evidence lookup overhead in hot paths
///
/// These benchmarks measure evidence lookup performance in realistic hot path
/// scenarios where lookups happen repeatedly in tight loops.
fn bench_evidence_hot_path(c: &mut Criterion) {
    let mut group = c.benchmark_group("evidence_hot_path");

    // Hot path: repeated lookups for same effect (common in State handler)
    group.bench_function("repeated_same_effect_100", |b| {
        let mut ev = EvidenceVector::new();
        for i in 0..5 {
            ev.push(i, std::ptr::null_mut());
        }
        b.iter(|| {
            let mut sum = 0u64;
            for _ in 0..100 {
                if ev.find_handler(black_box(2)).is_some() {
                    sum = sum.wrapping_add(1);
                }
            }
            black_box(sum)
        })
    });

    // Hot path: alternating between two effects (common in State + Error)
    group.bench_function("alternating_effects_100", |b| {
        let mut ev = EvidenceVector::new();
        for i in 0..5 {
            ev.push(i, std::ptr::null_mut());
        }
        b.iter(|| {
            let mut sum = 0u64;
            for i in 0..100 {
                let effect_id = if i % 2 == 0 { 1 } else { 3 };
                if ev.find_handler(black_box(effect_id)).is_some() {
                    sum = sum.wrapping_add(1);
                }
            }
            black_box(sum)
        })
    });

    // Hot path with varying stack depths
    for depth in [2, 5, 10, 20] {
        group.bench_with_input(
            BenchmarkId::new("lookup_100x_depth", depth),
            &depth,
            |b, &depth| {
                let mut ev = EvidenceVector::new();
                for i in 0..depth {
                    ev.push(i as u64, std::ptr::null_mut());
                }
                // Always look for handler in the middle
                let target = (depth / 2) as u64;
                b.iter(|| {
                    let mut sum = 0u64;
                    for _ in 0..100 {
                        if ev.find_handler(black_box(target)).is_some() {
                            sum = sum.wrapping_add(1);
                        }
                    }
                    black_box(sum)
                })
            },
        );
    }

    // Hot path: lookup miss (effect not handled - must scan entire stack)
    for depth in [5, 10, 20] {
        group.bench_with_input(
            BenchmarkId::new("lookup_miss_100x_depth", depth),
            &depth,
            |b, &depth| {
                let mut ev = EvidenceVector::new();
                for i in 0..depth {
                    ev.push(i as u64, std::ptr::null_mut());
                }
                // Look for effect that doesn't exist
                b.iter(|| {
                    let mut sum = 0u64;
                    for _ in 0..100 {
                        if ev.find_handler(black_box(999)).is_none() {
                            sum = sum.wrapping_add(1);
                        }
                    }
                    black_box(sum)
                })
            },
        );
    }

    // Simulated hot loop: effect op followed by state update
    // This mimics: for i in 0..n { state.put(state.get() + i) }
    group.bench_function("state_loop_pattern_100", |b| {
        let mut ev = EvidenceVector::new();
        ev.push(0, std::ptr::null_mut()); // State handler
        ev.push(1, std::ptr::null_mut()); // Outer handler

        b.iter(|| {
            let mut state = 0i64;
            for i in 0..100 {
                // get() - lookup state handler
                if ev.find_handler(black_box(0)).is_some() {
                    let current = state;
                    // put() - lookup state handler again
                    if ev.find_handler(black_box(0)).is_some() {
                        state = current + i;
                    }
                }
            }
            black_box(state)
        })
    });

    // Cache thrashing: many different effect lookups
    group.bench_function("many_different_effects_100", |b| {
        let mut ev = EvidenceVector::new();
        for i in 0..10 {
            ev.push(i, std::ptr::null_mut());
        }
        b.iter(|| {
            let mut sum = 0u64;
            for i in 0..100 {
                let effect_id = (i % 10) as u64;
                if ev.find_handler(black_box(effect_id)).is_some() {
                    sum = sum.wrapping_add(1);
                }
            }
            black_box(sum)
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_inline_generation_check,
    bench_slot_lookup,
    bench_slot_registry_scaling,
    bench_dereference_simulation,
    // PERF-002: Effect handler benchmarks
    bench_evidence_vector_operations,
    bench_handler_registry_operations,
    bench_continuation_operations,
    bench_effect_handle_expression,
    bench_tail_resumptive_vs_continuation,
    // EFF-004: Evidence hot path benchmarks
    bench_evidence_hot_path,
    // PERF-003 & PTR-006: 128-bit pointer overhead benchmarks
    bench_pointer_size_overhead,
    bench_linked_list_traversal,
    bench_tree_traversal,
    bench_cache_efficiency,
);
criterion_main!(benches);
