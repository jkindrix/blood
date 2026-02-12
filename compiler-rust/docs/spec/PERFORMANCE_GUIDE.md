# Blood Performance Guide

Best practices for writing high-performance Blood code, with measured benchmarks and optimization strategies.

## Table of Contents

1. [Understanding Blood's Performance Model](#1-understanding-bloods-performance-model)
2. [Measured Costs](#2-measured-costs)
3. [Memory Optimization](#3-memory-optimization)
4. [Effect Handler Optimization](#4-effect-handler-optimization)
5. [Data Structure Choices](#5-data-structure-choices)
6. [Common Anti-Patterns](#6-common-anti-patterns)
7. [Profiling and Measurement](#7-profiling-and-measurement)
8. [When Optimization Matters](#8-when-optimization-matters)

---

## 1. Understanding Blood's Performance Model

### The Safety-Performance Tradeoff

Blood provides memory safety through **generational references** rather than borrow checking or garbage collection. This means:

| Approach | Compile Cost | Runtime Cost | Memory Cost |
|----------|--------------|--------------|-------------|
| **Borrow Checking** (Rust) | High | Zero | Zero |
| **Garbage Collection** (Go, Java) | Low | Variable (GC pauses) | 20-50% overhead |
| **Generational References** (Blood) | Low | ~4 cycles/check | 2x pointer size |

Blood's approach gives you simpler code with predictable small runtime costs.

### Where Costs Come From

1. **Generation Checks**: ~4 cycles per heap pointer dereference
2. **Pointer Size**: 128-bit pointers (2x memory for pointer-heavy structures)
3. **Effect Handlers**: Near-zero for tail-resumptive, ~65 cycles for continuation-based
4. **Region Management**: Cheap push/pop, free bulk deallocation

---

## 2. Measured Costs

All measurements from `bloodc/benches/runtime_bench.rs` on a modern x86-64 CPU.

### Generation Check Costs

| Operation | Time | Cycles (3GHz) |
|-----------|------|---------------|
| Inline generation compare | ~129ps | <1 |
| Full slot lookup (hash table) | ~1.27ns | ~4 |
| Stack dereference (Tier 0) | ~222ps | <1 |
| Persistent generation bypass | ~425ps | ~1.3 |

**Key insight**: Stack (Tier 0) memory has near-zero overhead. Heap memory costs ~4 cycles per access.

### Effect Handler Costs

| Operation | Time | Cycles |
|-----------|------|--------|
| Evidence vector create | ~5.4ns | ~17 |
| Handler push | ~635ps | ~2 |
| Handler lookup (depth 3) | ~386ps | ~1.2 |
| Handler lookup (depth 10) | ~1.7ns | ~5 |
| Handle expression overhead | ~498ps | ~1.5 |
| Tail-resumptive resume | ~423ps | ~1.3 |
| Continuation create | ~48ns | ~150 |
| Continuation resume | ~20.5ns | ~65 |

**Key insight**: Tail-resumptive handlers (State, Reader, Writer) have near-zero overhead. Only handlers that need to capture continuations pay significant costs.

### Pointer Overhead

| Comparison | 64-bit | 128-bit | Overhead |
|------------|--------|---------|----------|
| Pointer size | 8 bytes | 16 bytes | 2x |
| Cache line capacity | 8 ptrs | 4 ptrs | 50% reduction |
| Linked list 1000 nodes | ~1.1µs | ~1.24µs | ~13% |
| Sequential array 10k elements | ~716ns | ~1.43µs | ~2x |

**Key insight**: Sequential memory access shows 2x overhead due to cache bandwidth. Pointer-chasing shows ~10-15% overhead because computation dominates memory access.

---

## 3. Memory Optimization

### Use Stack Allocation

Stack memory (Tier 0) has **zero generation check overhead**:

```blood
// FAST: Stack allocation
fn compute_distance(p1: Point, p2: Point) -> f64 {
    let dx = p2.x - p1.x;  // Stack
    let dy = p2.y - p1.y;  // Stack
    sqrt(dx * dx + dy * dy)
}

// SLOWER: Unnecessary heap allocation
fn compute_distance_slow(p1: &Point, p2: &Point) -> f64 {
    let dx = p2.x - p1.x;  // Requires generation check
    let dy = p2.y - p1.y;  // Requires generation check
    sqrt(dx * dx + dy * dy)
}
```

### Use Regions for Temporary Data

Regions provide bulk deallocation with minimal overhead:

```blood
// GOOD: Region for temporary processing
fn process_batch(items: &[Item]) -> Summary {
    region temp {
        let results = Vec::with_capacity(items.len());
        for item in items {
            results.push(process_single(item));
        }
        summarize(results)
    }  // All temp memory freed in O(1)
}

// LESS EFFICIENT: Individual allocations
fn process_batch_slow(items: &[Item]) -> Summary {
    let mut results = Vec::new();
    for item in items {
        results.push(process_single(item));  // Each may allocate
    }
    summarize(results)
    // Results freed individually or by GC-like mechanism
}
```

### Prefer Value Types for Small Data

Copying small values is cheaper than pointer indirection:

```blood
// FAST: Copy small structs
struct Point { x: f32, y: f32 }  // 8 bytes total

fn distance(p1: Point, p2: Point) -> f32 {  // Pass by value
    let dx = p2.x - p1.x;
    let dy = p2.y - p1.y;
    sqrt(dx * dx + dy * dy)
}

// Rule of thumb: Pass by value if size <= 16 bytes
struct SmallData { a: i64, b: i64 }  // 16 bytes - still worth copying
```

---

## 4. Effect Handler Optimization

### Use Tail-Resumptive Handlers When Possible

Tail-resumptive handlers compile to simple function calls (~1.3 cycles):

```blood
// FAST: Tail-resumptive (resume in tail position)
deep handler FastState<T> for State<T> {
    let mut state: T

    return(x) { x }

    op get() {
        resume(state)  // Tail position - optimized
    }

    op put(s) {
        state = s;
        resume(())     // Tail position - optimized
    }
}

// SLOWER: Non-tail-resumptive (needs continuation)
deep handler SlowState<T> for State<T> {
    let mut state: T

    return(x) { x }

    op get() {
        let result = resume(state);  // Not tail - needs continuation
        log("got state");
        result
    }
}
```

### Minimize Handler Nesting

Each nested handler adds lookup overhead:

```blood
// OK: 3 nested handlers (~2.6 cycles total overhead)
let result = with Logger {} handle {
    with StateHandler { state: 0 } handle {
        with ErrorHandler {} handle {
            compute()
        }
    }
};

// BETTER: Combine handlers when possible
deep handler CombinedHandler for {Log, State<i32>, Error<str>} {
    // Single handler for all three effects
    // Reduces nesting overhead
}
```

### Avoid Unnecessary Effect Boundaries

```blood
// SLOW: Effect boundary per iteration
fn process_items(items: &[Item]) -> i32 / Counter {
    let mut total = 0;
    for item in items {
        increment();  // Effect call each iteration
        total += process(item);
    }
    total
}

// FAST: Batch effect operations
fn process_items_fast(items: &[Item]) -> i32 / Counter {
    for item in items {
        // Pure computation
        process(item);
    }
    increment_by(items.len());  // Single effect call
    items.len() as i32
}
```

---

## 5. Data Structure Choices

### Prefer Arrays Over Linked Structures

Arrays have better cache behavior with 128-bit pointers:

```blood
// FAST: Contiguous array
let items: [Item; 1000] = ...;
for item in items {
    process(item);  // Sequential memory access
}

// SLOWER: Linked list (pointer-chasing)
let mut node: &ListNode = head;
while node.next.is_some() {
    process(node.value);  // Cache miss likely
    node = node.next.unwrap();
}
```

### Use Vec<T> Instead of Linked Lists

```blood
// GOOD: Vec for dynamic arrays
let mut items: Vec<Item> = Vec::new();
items.push(item1);
items.push(item2);

// Performance:
// - Push: amortized O(1)
// - Access by index: O(1) + ~4 cycles generation check
// - Iteration: Sequential memory access (cache-friendly)

// AVOID: Linked lists for general use
// - Push: O(1) but with allocation
// - Access by index: O(n)
// - Iteration: Pointer-chasing (cache-unfriendly)
```

### Tree vs. HashMap

For lookups, HashMap is usually faster than trees:

```blood
// FAST: HashMap for key-value lookup
let map: HashMap<str, Value> = HashMap::new();
map.insert("key", value);
let v = map.get("key");  // O(1) average

// SLOWER: Tree for sorted iteration
let tree: BTreeMap<str, Value> = BTreeMap::new();
tree.insert("key", value);
let v = tree.get("key");  // O(log n) with pointer-chasing
```

Use trees when you need:
- Sorted iteration
- Range queries
- Predictable worst-case performance

---

## 6. Common Anti-Patterns

### Anti-Pattern: Heap Allocation in Hot Loops

```blood
// BAD: Allocation every iteration
fn process_slow(n: i32) -> i32 {
    let mut sum = 0;
    for i in 0..n {
        let temp = Box::new(i);  // ALLOCATION IN LOOP
        sum += *temp;
    }
    sum
}

// GOOD: Stack allocation
fn process_fast(n: i32) -> i32 {
    let mut sum = 0;
    for i in 0..n {
        let temp = i;  // Stack
        sum += temp;
    }
    sum
}
```

### Anti-Pattern: Excessive Reference Chasing

```blood
// BAD: Multiple indirections
struct Bad {
    data: Box<Box<Box<i32>>>,  // 3 generation checks to read
}

fn read_bad(b: &Bad) -> i32 {
    ***b.data  // 3 dereferences = ~12 cycles overhead
}

// GOOD: Direct storage
struct Good {
    data: i32,  // Direct storage
}

fn read_good(g: &Good) -> i32 {
    g.data  // 1 generation check = ~4 cycles
}
```

### Anti-Pattern: Pointer-Heavy Structures for Small Data

```blood
// BAD: Linked list for small, fixed-size data
struct BadPoint {
    x: Box<f32>,
    y: Box<f32>,
    z: Box<f32>,
}

// GOOD: Inline storage
struct GoodPoint {
    x: f32,
    y: f32,
    z: f32,
}
```

### Anti-Pattern: Effect Operations in Inner Loops

```blood
// BAD: Effect per element
fn sum_with_logging(items: &[i32]) -> i32 / Log {
    let mut sum = 0;
    for item in items {
        log(format!("Processing {}", item));  // EFFECT EVERY ITERATION
        sum += item;
    }
    sum
}

// GOOD: Batch logging
fn sum_with_logging_fast(items: &[i32]) -> i32 / Log {
    log(format!("Processing {} items", items.len()));  // Once
    let sum = items.iter().sum();
    log(format!("Sum: {}", sum));  // Once
    sum
}
```

---

## 7. Profiling and Measurement

### Use Criterion for Microbenchmarks

```rust
// In Cargo.toml:
// [dev-dependencies]
// criterion = "0.5"

use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn benchmark_my_function(c: &mut Criterion) {
    c.bench_function("my_function", |b| {
        b.iter(|| my_function(black_box(input)))
    });
}

criterion_group!(benches, benchmark_my_function);
criterion_main!(benches);
```

### Measure Before Optimizing

1. **Identify the bottleneck** - Profile to find where time is spent
2. **Measure baseline** - Benchmark the current performance
3. **Apply optimization** - Make targeted changes
4. **Verify improvement** - Re-benchmark to confirm

### Key Metrics to Track

- **Throughput**: Operations per second
- **Latency**: Time per operation
- **Memory usage**: Peak and average allocation
- **Cache misses**: Use `perf stat` on Linux

---

## 8. When Optimization Matters

### Always Optimize

- **Algorithmic complexity**: O(n²) → O(n log n) always wins
- **Memory leaks**: Always fix
- **Obvious waste**: Remove unnecessary allocations

### Sometimes Optimize

- **Hot loops**: Profile first, optimize if >10% of runtime
- **Frequently called functions**: Inline or optimize if measured
- **Data structure choice**: Consider cache behavior for large datasets

### Rarely Optimize

- **Cold code paths**: Error handling, initialization
- **I/O-bound code**: Network/disk latency dominates
- **Small datasets**: Setup costs dominate for n < 100

### Never Optimize

- **Unmeasured code**: Always profile first
- **Clear code into clever code**: Maintainability matters
- **Single-digit percentages**: Unless at scale

### The 80/20 Rule

80% of execution time is spent in 20% of code. Focus optimization efforts on:

1. Inner loops
2. Frequently called functions
3. Hot data structures
4. Allocation patterns

---

## Summary: Quick Reference

### Do This

| Situation | Best Practice |
|-----------|---------------|
| Small data | Pass by value |
| Temporary data | Use regions |
| Collections | Use `Vec<T>` |
| Effect handlers | Make them tail-resumptive |
| Hot loops | Avoid allocations |

### Avoid This

| Anti-Pattern | Why |
|--------------|-----|
| Linked lists | Poor cache behavior |
| Deep nesting (Box<Box<T>>) | Multiple generation checks |
| Effects in inner loops | Handler overhead per iteration |
| Premature optimization | Profile first |

### Typical Costs

| Operation | Cost |
|-----------|------|
| Generation check | ~4 cycles |
| Effect handler entry | ~1.5 cycles |
| Continuation resume | ~65 cycles |
| Region creation | ~5ns |

---

## Related Documentation

- [MEMORY_GUIDE.md](./MEMORY_GUIDE.md) - Memory model user guide
- [MEMORY_MODEL.md](./MEMORY_MODEL.md) - Technical memory model
- [EFFECTS_TUTORIAL.md](./EFFECTS_TUTORIAL.md) - Effect system tutorial
- [ACTION_ITEMS.md](./ACTION_ITEMS.md) - Detailed benchmark results

---

*Last updated: 2026-01-13*
