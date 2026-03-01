# Blood Memory Model User Guide

A practical guide to understanding and working with Blood's memory system.

## Table of Contents

1. [Overview](#1-overview)
2. [Memory Tiers](#2-memory-tiers)
3. [Generational References](#3-generational-references)
4. [Regions](#4-regions)
5. [Working with Effects](#5-working-with-effects)
6. [Performance Tips](#6-performance-tips)
7. [Common Patterns](#7-common-patterns)
8. [Troubleshooting](#8-troubleshooting)

---

## 1. Overview

### What Makes Blood Different

Blood provides memory safety **without garbage collection** and **without borrow checking**. Instead, it uses a generational reference system that:

- **Detects use-after-free at runtime** (not compile time)
- **Allows aliasing** (multiple references to the same data)
- **Requires no lifetime annotations** in most code
- **Supports first-class effects** that can suspend computation

### The Mental Model

Think of Blood's memory like a library with ID cards:

1. Every piece of data gets a **generation number** (like a library card version)
2. Every reference stores the **expected generation**
3. When you access data, Blood checks if the generations match
4. If they don't match, the data was freed and reallocated - you have a stale reference

This is simpler than borrow checking because you don't need to prove at compile time that references don't outlive data. The tradeoff is a small runtime check.

---

## 2. Memory Tiers

Blood has three memory tiers, each optimized for different lifetimes:

### Tier 0: Stack Memory

```blood
fn example() {
    let x: i32 = 42;     // Stack allocated
    let point = Point { x: 1, y: 2 };  // Also stack
    // No generation check needed - lifetime is lexically scoped
}
```

**Characteristics:**
- Fastest access (no generation check)
- Automatic cleanup when scope ends
- Cannot escape the function

**When to use:** Local variables, temporary calculations, small structs

### Tier 1: Region Memory

```blood
fn process_data() {
    region data_region {
        // Everything allocated here uses the region
        let items = Vec::new();  // Region allocated
        items.push(1);
        items.push(2);
        // All region memory freed when block ends
    }
}
```

**Characteristics:**
- Bulk deallocation (entire region freed at once)
- Generation checks for safety
- Great for temporary data with known lifetime

**When to use:** Processing pipelines, request handling, temporary collections

### Tier 2: Persistent Memory

```blood
fn create_cache() -> &Cache {
    // Persistent allocation for long-lived data
    let cache = persistent Cache::new();
    cache  // Can safely return
}
```

**Characteristics:**
- Reference counted
- Automatic cycle collection
- Generation checks for safety
- Lives as long as references exist

**When to use:** Global state, caches, data structures with unknown lifetime

---

## 3. Generational References

### How They Work

Every heap-allocated object has a **generation number**. When you create a reference, it captures the current generation:

```blood
let data = Box::new(42);  // data has generation 7
let ref1 = &data;         // ref1 stores: address + generation 7

// Later, if data is freed and the slot reused:
// The slot now has generation 8
// ref1 still has generation 7
// Accessing ref1 detects the mismatch -> error
```

### Reference Types

```blood
// Immutable reference (can have many)
let r1: &T = &value;
let r2: &T = &value;  // Fine! Multiple readers allowed

// Mutable reference (exclusive access)
let m: &mut T = &mut value;
// m.field = 42;  // Exclusive access to mutate
```

### The 128-bit Pointer

Blood uses 128-bit "fat pointers" that contain:
- 64 bits: memory address
- 32 bits: generation number
- 32 bits: metadata (tier, flags)

This doubles pointer size compared to C/Rust, but enables runtime safety checking.

---

## 4. Regions

Regions provide **bulk deallocation** - all memory in a region is freed together:

### Basic Region Usage

```blood
fn process_request(request: Request) -> Response {
    region temp {
        // Parse request (temporary allocations)
        let parsed = parse_json(request.body);

        // Process (more temporary allocations)
        let result = compute(parsed);

        // Build response
        Response::new(result)
    }
    // All temporary allocations freed here
}
```

### Nested Regions

```blood
region outer {
    let data = load_data();

    region inner {
        let processed = transform(data);
        // inner region memory freed here
    }

    // data still valid (in outer region)
    save(data);
}
// outer region memory freed here
```

### Region + Effects

When an effect handler suspends execution, regions are **preserved**:

```blood
fn fiber_process() -> Result<Data> / Fiber {
    region temp {
        let partial = start_processing();

        suspend fetch_more_data();  // Suspends here
        // Region temp is preserved across suspension!

        finish_processing(partial)  // partial still valid
    }
}
```

---

## 5. Working with Effects

### Generation Snapshots

When effects suspend, Blood captures a **generation snapshot** of all live references. This ensures memory safety across suspension points:

```blood
fn example() -> i32 / State<&Data> {
    let data_ref = get();      // Reference to handler state

    put(new_data);             // Effect might allocate

    // data_ref is validated when resumed
    // If the memory was reallocated, this would fail safely
    data_ref.value
}
```

### Safe Patterns

```blood
// GOOD: Use value types across effect boundaries when possible
fn safe_pattern() -> i32 / Counter {
    let value = get_count();   // Copy the i32 value
    increment();               // Effect operation
    value + get_count()        // Using the copied value
}

// CAREFUL: References across effect boundaries
fn careful_pattern() -> i32 / State<&mut Vec<i32>> {
    let vec_ref = get();       // Reference captured
    // Generation snapshot validates this on resume
    vec_ref.len() as i32
}
```

### Multi-shot Handlers

Multi-shot handlers (like `Choice`) can resume continuations multiple times. Special restrictions apply:

```blood
// Linear values CANNOT be used in multi-shot handlers
fn choose_example() -> i32 / Choice {
    let x: linear Resource = acquire();  // ERROR if Choice is multi-shot
    choose([1, 2, 3])
}

// Affine values are OK (can be dropped)
fn affine_example() -> i32 / Choice {
    let x: affine FileHandle = open("test.txt");  // OK
    let choice = choose([1, 2, 3]);
    drop(x);  // Can be dropped
    choice
}
```

---

## 6. Performance Tips

### When 128-bit Pointers Are Fine

- **Business logic**: The safety check (~4 cycles) is negligible
- **I/O-bound code**: Waiting for network/disk dwarfs pointer overhead
- **Moderate data structures**: Trees, graphs, linked lists with <10k nodes
- **Application code**: Games, servers, desktop apps

### When to Optimize

- **Inner loops processing millions of elements**: Consider arrays instead of linked structures
- **Cache-sensitive algorithms**: Use contiguous memory (Vec, arrays)
- **Numeric computation**: Use stack allocation, avoid heap when possible

### Optimization Strategies

```blood
// PREFER: Contiguous arrays over linked lists
let items: [i32; 1000] = ...;  // Single allocation, no pointers

// PREFER: Stack allocation for small, fixed-size data
let point = Point { x: 1.0, y: 2.0 };  // No generation check

// PREFER: Regions for temporary data
region processing {
    let temp_vec = Vec::with_capacity(1000);
    // Process...
}  // Bulk free

// PREFER: Value types over references when small
fn process(p: Point) { ... }  // Copy instead of reference
```

### Measured Overhead

From benchmarks (see ACTION_ITEMS.md for details):
- **Generation check**: ~4 cycles per dereference
- **Stack dereference**: Near-zero overhead
- **Linked list traversal**: ~10-15% slower than raw pointers
- **Effect handler entry**: ~1.5 cycles (tail-resumptive)

---

## 7. Common Patterns

### Builder Pattern

```blood
struct Config {
    host: str,
    port: i32,
    timeout: i32,
}

struct ConfigBuilder {
    host: str,
    port: i32,
    timeout: i32,
}

impl ConfigBuilder {
    fn new() -> ConfigBuilder {
        ConfigBuilder {
            host: "localhost",
            port: 8080,
            timeout: 30,
        }
    }

    fn host(mut self, h: str) -> ConfigBuilder {
        self.host = h;
        self
    }

    fn build(self) -> Config {
        Config {
            host: self.host,
            port: self.port,
            timeout: self.timeout,
        }
    }
}

// Usage
let config = ConfigBuilder::new()
    .host("example.com")
    .build();
```

### Resource Management with Effects

```blood
effect Resource<R> {
    op acquire() -> R
    op release(r: R) -> ()
}

fn with_file<T>(path: str, f: fn(&File) -> T / Resource<File>) -> T / IO {
    let file = acquire();
    let result = f(&file);
    release(file);
    result
}

// Usage
fn process_file() -> str / {Resource<File>, IO} {
    with_file("data.txt", |file| {
        file.read_to_string()
    })
}
```

### State Management

```blood
effect State<S> {
    op get() -> S
    op put(s: S) -> ()
    op modify(f: fn(S) -> S) -> ()
}

fn counter_example() -> i32 / State<i32> {
    let current = get();
    put(current + 1);
    get()
}

// Handler
deep handler Counter for State<i32> {
    let mut state: i32

    return(x) { x }

    op get() { resume(state) }
    op put(s) { state = s; resume(()) }
    op modify(f) { state = f(state); resume(()) }
}

fn main() {
    let result = with Counter { state: 0 } handle {
        counter_example();  // returns 1
        counter_example();  // returns 2
        counter_example()   // returns 3
    };
    println_int(result);  // prints 3
}
```

---

## 8. Troubleshooting

### Stale Reference Error

```
ERROR: Stale reference detected
  Expected generation: 42
  Actual generation: 43
  Location: src/main.blood:15
```

**Cause**: You accessed memory that was freed and reallocated.

**Fix**:
1. Check if the reference escaped its scope
2. Ensure data outlives all references to it
3. Consider using regions to control lifetime

### Effect Handler Not Found

```
ERROR: No handler for effect State<Config>
  At: src/config.blood:23
```

**Cause**: Effect operation performed without an installed handler.

**Fix**: Wrap the effectful code in a `handle` expression:

```blood
// Wrong:
let value = get();  // No handler!

// Right:
let value = with StateHandler { state: initial } handle {
    get()
};
```

### Linear Value Dropped

```
ERROR: Linear value must be consumed
  Type: linear Connection
  At: src/db.blood:45
```

**Cause**: A linear value went out of scope without being used.

**Fix**: Either use the value or explicitly transfer ownership:

```blood
// Wrong:
fn example() {
    let conn: linear Connection = connect();
    // conn dropped without use!
}

// Right:
fn example() {
    let conn: linear Connection = connect();
    conn.close();  // Properly consumed
}
```

### Multi-shot Handler with Linear Value

```
ERROR: Cannot use linear value in multi-shot handler
  Type: linear Resource
  Handler: Choice (multi-shot)
```

**Cause**: Multi-shot handlers can resume multiple times, but linear values can only be used once.

**Fix**: Use affine types or restructure to avoid the conflict:

```blood
// Wrong:
fn example() -> i32 / Choice {
    let r: linear Resource = acquire();
    choose([1, 2, 3])  // r might be duplicated!
}

// Right:
fn example() -> i32 / Choice {
    let choice = choose([1, 2, 3]);  // Choose first
    let r: linear Resource = acquire();  // Then acquire
    use_resource(r);
    choice
}
```

---

## Related Documentation

- [SPECIFICATION.md](./SPECIFICATION.md) - Formal language specification
- [MEMORY_MODEL.md](./MEMORY_MODEL.md) - Technical memory model details
- [EFFECTS_TUTORIAL.md](./EFFECTS_TUTORIAL.md) - Effect system tutorial
- [GETTING_STARTED.md](./GETTING_STARTED.md) - Quick start guide

---

*Last updated: 2026-01-13*
