# Blood Effect System Tutorial

A practical guide to using algebraic effects in Blood, from basics to advanced patterns.

## Table of Contents

1. [Introduction](#1-introduction)
2. [Basic Effects](#2-basic-effects)
3. [Handlers](#3-handlers)
4. [Common Patterns](#4-common-patterns)
5. [Effect Composition](#5-effect-composition)
6. [Performance Considerations](#6-performance-considerations)
7. [Best Practices](#7-best-practices)

---

## 1. Introduction

### What Are Algebraic Effects?

Algebraic effects are a principled way to handle side effects that separates:
- **What** effects a computation can perform (effect declarations)
- **How** those effects are handled (handler implementations)

This separation enables:
- **Composable effects**: Combine multiple effects naturally
- **Testable code**: Swap handlers for mocking
- **Explicit control flow**: No hidden exceptions
- **Resource safety**: Effects integrate with Blood's memory model

### Effects in Blood vs. Other Languages

| Feature | Blood | Rust | Go | Haskell |
|---------|-------|------|-----|---------|
| Effect tracking | Type-level | None | None | Monads |
| Handler composition | Built-in | Manual | Manual | MTL |
| Resume/non-resume | Both | N/A | N/A | Limited |
| Memory integration | Generational snapshots | N/A | N/A | N/A |

---

## 2. Basic Effects

### Declaring Effects

Effects are declared with the `effect` keyword:

```blood
// Simple effect with one operation
effect Log {
    op log(message: str) -> ()
}

// Effect with multiple operations
effect Counter {
    op increment() -> ()
    op get_count() -> i32
}

// Generic effect with type parameter
effect State<S> {
    op get() -> S
    op put(new_state: S) -> ()
}

// Effect with error type parameter
effect Error<E> {
    op raise(err: E) -> never  // 'never' means no resume
}
```

### Using Effects in Functions

Functions declare their effects after the return type with `/`:

```blood
// Function that uses the Log effect
fn greet(name: str) -> () / Log {
    log("Hello, ")
    log(name)
}

// Function that uses multiple effects
fn process(data: i32) -> i32 / {Log, Counter} {
    log("Processing...")
    increment()
    data * 2
}

// Pure function (no effects)
fn add(a: i32, b: i32) -> i32 / pure {
    a + b
}
```

### Performing Effect Operations

Call effect operations like regular functions:

```blood
fn example() -> i32 / State<i32> {
    let current = get()        // Perform get() operation
    put(current + 1)           // Perform put() operation
    get()                      // Return the new value
}
```

---

## 3. Handlers

Handlers define how effects are implemented. Blood supports two handler types:

### Deep Handlers (Default)

Deep handlers automatically handle all operations in continuations:

```blood
deep handler CounterImpl for Counter {
    // Handler state
    let mut count: i32

    // Return clause: what to do when computation finishes
    return(x) { x }

    // Operation implementations
    op increment() {
        count = count + 1
        resume(())  // Continue the computation
    }

    op get_count() {
        resume(count)  // Continue with the count value
    }
}
```

### Shallow Handlers

Shallow handlers handle one operation at a time (advanced):

```blood
shallow handler OnceCounter for Counter {
    let mut count: i32

    return(x) { x }

    op increment() {
        count = count + 1
        resume(())  // Only handles this one operation
    }

    op get_count() {
        resume(count)
    }
}
```

### Using Handlers with `handle`

Wrap effectful computations with handlers:

```blood
fn main() {
    // Use a handler to run effectful code
    let result = with CounterImpl { count: 0 } handle {
        increment()
        increment()
        get_count()  // Returns 2
    }

    println_int(result)  // Prints: 2
}
```

---

## 4. Common Patterns

### Pattern 1: State Management

```blood
effect State<S> {
    op get() -> S
    op put(s: S) -> ()
    op modify(f: fn(S) -> S) -> ()
}

deep handler LocalState<S> for State<S> {
    let mut state: S

    return(x) { x }

    op get() {
        resume(state)
    }

    op put(s) {
        state = s
        resume(())
    }

    op modify(f) {
        state = f(state)
        resume(())
    }
}

// Usage
fn increment_counter() -> i32 / State<i32> {
    let current = get()
    put(current + 1)
    get()
}

fn main() {
    let final_value = with LocalState { state: 0 } handle {
        increment_counter()
        increment_counter()
        increment_counter()  // Returns 3
    }
}
```

### Pattern 2: Error Handling

```blood
effect Error<E> {
    op raise(err: E) -> never
}

deep handler TryHandler<E, T> for Error<E> {
    return(x) { Ok(x) }

    op raise(err) {
        // Don't resume - return error immediately
        Err(err)
    }
}

// Usage
fn divide(a: i32, b: i32) -> i32 / Error<str> {
    if b == 0 {
        raise("Division by zero")
    }
    a / b
}

fn main() {
    let result = with TryHandler {} handle {
        divide(10, 2)  // Ok(5)
    }

    let error_result = with TryHandler {} handle {
        divide(10, 0)  // Err("Division by zero")
    }
}
```

### Pattern 3: Logging/Tracing

```blood
effect Log {
    op debug(msg: str) -> ()
    op info(msg: str) -> ()
    op error(msg: str) -> ()
}

deep handler ConsoleLogger for Log {
    return(x) { x }

    op debug(msg) {
        print_str("[DEBUG] ")
        println_str(msg)
        resume(())
    }

    op info(msg) {
        print_str("[INFO] ")
        println_str(msg)
        resume(())
    }

    op error(msg) {
        print_str("[ERROR] ")
        println_str(msg)
        resume(())
    }
}

// Silent handler for testing
deep handler SilentLogger for Log {
    return(x) { x }

    op debug(msg) { resume(()) }
    op info(msg) { resume(()) }
    op error(msg) { resume(()) }
}

// Usage in tests
fn test_my_function() {
    with SilentLogger {} handle {
        my_function_that_logs()
    }
}
```

### Pattern 4: Resource Acquisition

```blood
effect Resource<R> {
    op acquire() -> R
    op release(r: R) -> ()
}

deep handler FileResource for Resource<FileHandle> {
    return(x) { x }

    op acquire() {
        let handle = open_file("data.txt")
        resume(handle)
    }

    op release(handle) {
        close_file(handle)
        resume(())
    }
}

// Using bracket pattern
fn with_resource<R, T>(action: fn(R) -> T / Resource<R>) -> T / Resource<R> {
    let resource = acquire()
    let result = action(resource)
    release(resource)
    result
}
```

### Pattern 5: Non-Determinism/Choice

```blood
effect Choice {
    op choose<T>(options: [T]) -> T
    op fail() -> never
}

deep handler FirstChoice for Choice {
    return(x) { Some(x) }

    op choose(options) {
        if options.is_empty() {
            None
        } else {
            resume(options[0])
        }
    }

    op fail() {
        None
    }
}

// Collect all choices
deep handler AllChoices<T> for Choice {
    return(x) { vec![x] }

    op choose(options) {
        let mut results = vec![]
        for option in options {
            let sub_results = resume(option)
            results.extend(sub_results)
        }
        results
    }

    op fail() {
        vec![]
    }
}
```

---

## 5. Effect Composition

### Nesting Handlers

Handle multiple effects by nesting handlers:

```blood
fn complex_computation() -> i32 / {State<i32>, Error<str>, Log} {
    info("Starting computation")
    let value = get()
    if value < 0 {
        raise("Negative value not allowed")
    }
    put(value * 2)
    info("Computation complete")
    get()
}

fn main() {
    let result = with TryHandler {} handle {
        with LocalState { state: 5 } handle {
            with ConsoleLogger {} handle {
                complex_computation()
            }
        }
    }
    // result: Ok(10)
}
```

### Effect Row Polymorphism

Write functions that work with any additional effects:

```blood
// This function adds Log to any existing effect row
fn log_and_run<R, T>(
    action: fn() -> T / {Log | R}
) -> T / {Log | R} {
    info("Starting action")
    let result = action()
    info("Action complete")
    result
}
```

---

## 6. Performance Considerations

Understanding effect system performance is crucial for writing efficient Blood code. This section provides measured costs, optimization strategies, and guidance on when effects matter for performance.

### 6.1 Effect System Cost Model

Blood's effect system uses **evidence passing** (based on ICFP'21 research), where handlers are represented as evidence vectors passed to effectful functions. This provides predictable, measurable costs.

#### Measured Costs (x86-64, 3GHz)

All measurements from `blood-runtime/benches/runtime_bench.rs`:

| Operation | Time | Cycles | Notes |
|-----------|------|--------|-------|
| **Evidence Vector** |
| Create empty vector | ~5.4ns | ~17 | One-time per handler block |
| Push handler | ~635ps | ~2 | Per nested handler |
| Lookup (depth 3) | ~386ps | ~1.2 | Common case |
| Lookup (depth 10) | ~1.7ns | ~5 | Deep nesting |
| **Handler Operations** |
| Handle expression overhead | ~498ps | ~1.5 | Per `with ... handle` |
| Nested depth 3 | ~834ps | ~2.6 | Three nested handlers |
| Nested depth 10 | ~2.4ns | ~8 | Deep nesting (rare) |
| **Resume Strategies** |
| Tail-resumptive | ~423ps | ~1.3 | **Near-zero overhead** |
| Continuation create | ~48ns | ~150 | Non-tail resume |
| Continuation resume | ~20.5ns | ~65 | One-shot resume |
| Multi-shot clone | ~56ns | ~175 | Resuming multiple times |
| **Generation Snapshots** |
| Capture (per reference) | ~5 cycles | ~1.7ns | Suspension point |
| Validate (per reference) | ~4 cycles | ~1.3ns | Resume validation |

### 6.2 Handler Classification

Blood handlers fall into distinct performance categories:

#### Tail-Resumptive Handlers (~1.3 cycles overhead)

When `resume()` appears in tail position, the compiler optimizes to a direct call:

```blood
// TAIL-RESUMPTIVE: Near-zero overhead
deep handler FastState<T> for State<T> {
    let mut state: T

    return(x) { x }

    op get() {
        resume(state)  // Tail position ✓
    }

    op put(s) {
        state = s;
        resume(())     // Tail position ✓
    }
}

// Usage costs: ~1.3 cycles per get/put
fn counter() -> i32 / State<i32> {
    let x = get();    // ~1.3 cycles
    put(x + 1);       // ~1.3 cycles
    get()             // ~1.3 cycles
}
```

These effects compile to efficient function calls:
- `State<T>` (get/put)
- `Reader<T>` (ask)
- `Writer<T>` (tell)
- `Log` (log messages)

#### Continuation-Based Handlers (~65 cycles overhead)

Non-tail `resume()` requires capturing a continuation:

```blood
// NON-TAIL: Requires continuation capture
deep handler TracingState<T> for State<T> {
    let mut state: T
    let mut history: Vec<T>

    op get() {
        history.push(state);
        let result = resume(state);  // NOT tail position
        println!("get returned {}", result);
        result
    }
}

// Usage costs: ~65 cycles per get (continuation overhead)
```

Use continuation-based handlers when you need:
- Post-processing after resume
- Multi-shot semantics (backtracking, non-determinism)
- Complex control flow (coroutines, async)

#### Multi-Shot Handlers (~175 cycles per resume)

Resuming multiple times clones the continuation:

```blood
// MULTI-SHOT: Each resume clones continuation
deep handler NonDet for Choose {
    op choose<T>(a: T, b: T) -> T {
        let result_a = resume(a);   // First shot
        let result_b = resume(b);   // Second shot (clones)
        combine(result_a, result_b)
    }
}
```

Multi-shot handlers are powerful but expensive. Use sparingly.

### 6.3 Generation Snapshot Costs

When effects suspend computation, Blood captures generation numbers for memory safety. This section explains the snapshot mechanism and its performance characteristics in detail.

#### 6.3.1 What Snapshots Capture

A **generation snapshot** records the expected generation values of all generational references that are:
1. Live at the suspension point (still needed after resume)
2. Not persistent references (immutable globals, string literals)
3. Not null pointers

```blood
fn use_across_suspension() -> i32 / Async {
    let data = Box::new(42);      // Heap allocation with generation G
    let global = &SOME_CONSTANT;  // Persistent - NOT captured
    perform Async.yield();        // Captures: [(data.address, G)]
    // On resume: validate data.generation == G
    *data                         // Safe access
}
```

**Snapshot entry structure (16 bytes per reference):**
```
┌─────────────────────────────────────────────────┐
│ address: u64        │ Address of the allocation │
├─────────────────────┼───────────────────────────┤
│ generation: u32     │ Expected generation value │
├─────────────────────┼───────────────────────────┤
│ local_id: u32       │ Source variable (errors)  │
└─────────────────────┴───────────────────────────┘
```

#### 6.3.2 Snapshot Cost Model

**Costs scale linearly with captured references:**

| References Captured | Capture Cost | Validation Cost | Total Overhead |
|---------------------|--------------|-----------------|----------------|
| 0 (pure computation) | 0 | 0 | 0 |
| 1 reference | ~5 cycles (~1.7ns) | ~4 cycles (~1.3ns) | ~3ns |
| 10 references | ~50 cycles (~17ns) | ~40 cycles (~13ns) | ~30ns |
| 100 references | ~500 cycles (~170ns) | ~400 cycles (~130ns) | ~300ns |

**Per-reference operations:**

| Operation | Time | Description |
|-----------|------|-------------|
| Read generation | ~2 cycles | Load 32-bit value from pointer |
| Store entry | ~3 cycles | Write 16 bytes to snapshot buffer |
| Validate entry | ~4 cycles | Load, compare, branch |

#### 6.3.3 What Gets Excluded from Snapshots

Blood's compiler automatically **excludes** certain values to minimize overhead:

1. **Persistent pointers** (generation = `PERSISTENT_MARKER`):
   - String literals: `"hello"`
   - Static/const references: `&SOME_CONSTANT`
   - Immutable global data

2. **Null pointers**: Empty Option values, uninitialized slots

3. **Dead references**: Liveness analysis determines which references are actually used after the suspension point

4. **Stack-only locals**: Values that don't escape through the effect

```blood
fn optimized_snapshot() / Async {
    let temp = compute_temp();      // NOT captured (dead after perform)
    let msg = "logging";            // NOT captured (persistent)
    let opt: Option<&Data> = None;  // NOT captured (null)
    let data = get_data();          // CAPTURED (live after perform)

    perform Async.yield();          // Snapshot contains only: data

    process(data);                  // Uses data after resume
}
```

#### 6.3.4 Liveness Analysis Optimization

Blood uses **dataflow liveness analysis** to minimize snapshot size:

```blood
// Without optimization: would capture ALL refs in scope
fn naive_capture() / Async {
    let a = get_ref();  // Captured (used after)
    let b = get_ref();  // Captured (used after)
    let c = get_ref();  // Captured (used after)
    let d = get_ref();  // Dead (not used after)

    perform Async.yield();

    process(a, b, c);   // d is dead - not captured!
}
```

**Liveness algorithm** (from rustc dataflow analysis):
```
live_out(block) = ∪ live_in(successor) for all successors
live_in(block) = use(block) ∪ (live_out(block) - def(block))
```

This reduces snapshot size by 20-40% in typical code.

#### 6.3.5 Memory Overhead

**Snapshot buffer allocation:**

| Handler Type | Buffer Size | Allocation |
|--------------|-------------|------------|
| Tail-resumptive | 0 bytes | None (no suspension) |
| Continuation (≤8 refs) | 128 bytes | Stack-allocated |
| Continuation (>8 refs) | n × 16 bytes | Heap-allocated |

**Memory layout in continuation:**
```
┌─────────────────────────────────────────────────────────┐
│ Continuation Frame                                      │
├─────────────────────────────────────────────────────────┤
│ return_address: *const ()        │ 8 bytes             │
│ saved_frame_pointer: *mut ()     │ 8 bytes             │
│ handler_state: HandlerState      │ Variable            │
│ snapshot_count: u32              │ 4 bytes             │
│ snapshot_entries: [SnapshotEntry]│ n × 16 bytes        │
│ captured_locals: [u8]            │ Variable            │
└─────────────────────────────────────────────────────────┘
```

#### 6.3.6 Validation Failure Behavior

When validation fails (stale reference detected):

1. **StaleReference effect** is raised with details
2. Handler can intercept and handle gracefully
3. If unhandled, program panics with diagnostic

```blood
effect StaleReference {
    op stale(ptr: *const (), expected: u32, actual: u32) -> !
}

// User code can handle stale references
with StaleHandler {} handle {
    let data = do_effect_stuff();  // Might have stale ref
    *data  // Validated here
}
// If stale: StaleHandler's op is called
```

**Validation cost breakdown:**
```
For each entry:
  1. Load current generation from address    [2 cycles]
  2. Compare with expected generation        [1 cycle]
  3. Branch if mismatch                      [1 cycle]
Total: ~4 cycles per reference
```

#### 6.3.7 Optimization Strategies

**Strategy 1: Minimize live references at suspension points:**

```blood
// SLOW: Many references live at suspension
fn slow_process(data: &[Item]) / Async {
    let refs: Vec<&Item> = data.iter().collect();  // 1000 refs
    for r in refs {
        perform Async.yield();  // Captures 1000 refs each time!
        process(r);
    }
}

// FAST: Minimal references at suspension
fn fast_process(data: &[Item]) / Async {
    for i in 0..data.len() {
        perform Async.yield();  // Captures only index + data base
        process(&data[i]);
    }
}
```

**Strategy 2: Process data before suspension:**

```blood
// SLOW: Live reference across suspension
fn slow_transform() / Async {
    let data = get_large_data();   // Large allocation
    perform Async.yield();          // Snapshot includes data
    transform(data)
}

// FAST: Transform before suspension
fn fast_transform() / Async {
    let result = {
        let data = get_large_data();
        transform(data)  // data dies here
    };
    perform Async.yield();  // Empty snapshot
    result
}
```

**Strategy 3: Use indices instead of references:**

```blood
// SLOW: Vector of references
fn slow_collect() / Async {
    let items: Vec<&Data> = collect_refs();  // n refs
    perform Async.yield();  // Captures n refs
    process_all(&items)
}

// FAST: Vector of indices into shared storage
fn fast_collect(storage: &Storage) / Async {
    let indices: Vec<u32> = collect_indices();  // No refs!
    perform Async.yield();  // Captures only: storage (1 ref)
    for i in indices {
        process(storage.get(i));
    }
}
```

#### 6.3.8 When Snapshots Are Free

Snapshots have **zero cost** in these scenarios:

1. **Tail-resumptive handlers**: No suspension, no capture
2. **Pure computations**: No heap references in scope
3. **Immediate resume**: No references alive at suspension
4. **Persistent-only code**: Only uses constants/literals

```blood
// Zero snapshot cost:
shallow handler Counter for Count {
    let mut count: i32 = 0

    op increment() {
        count += 1;
        resume(())  // Tail-resumptive: no snapshot
    }
}

// Also zero cost:
fn pure_compute() -> i32 / Math {
    let a = 1 + 2;          // Stack values
    let b = perform Math.sqrt(16.0);  // No heap refs
    a + b as i32            // No refs to capture
}
```

### 6.4 Handler Nesting Costs

Each nested handler adds lookup overhead:

```blood
// 1 handler: ~1.5 cycles overhead
with Logger {} handle {
    compute()
}

// 3 handlers: ~2.6 cycles overhead
with Logger {} handle {
    with State { initial: 0 } handle {
        with Error {} handle {
            compute()
        }
    }
}

// 10 handlers: ~8 cycles overhead (rare in practice)
```

**Optimization: Combine related handlers:**

```blood
// COMBINED: Single handler for multiple effects
deep handler AppHandler for {Log, State<AppState>, Error<AppError>} {
    let mut state: AppState
    let mut logs: Vec<String>

    op log(msg) { logs.push(msg); resume(()) }
    op get() { resume(state) }
    op put(s) { state = s; resume(()) }
    op throw(e) { Err(e) }
}

// One handler lookup instead of three
with AppHandler { state: initial, logs: vec![] } handle {
    run_application()
}
```

### 6.5 Effect Categories by Performance

| Effect Type | Typical Cost | Example Effects |
|-------------|--------------|-----------------|
| **Pure** | 0 cycles | No effects (`/ pure`) |
| **Tail-resumptive** | ~1-2 cycles | State, Reader, Writer, Log |
| **One-shot continuation** | ~65 cycles | Error (with cleanup), Yield |
| **Multi-shot** | ~175+ cycles | Choose, NonDet, Amb |
| **With snapshots** | +5 cycles/ref | Any effect with heap refs |

### 6.6 When Effects Matter

**Effects rarely matter in practice** because:

1. Most effects are tail-resumptive (~1 cycle)
2. Effect cost is dwarfed by I/O, allocation, etc.
3. The clarity benefits outweigh small costs

**Benchmark first!** Profile before optimizing:

```blood
// DON'T: Premature optimization
fn process() -> i32 {
    let mut sum = 0;
    for i in 0..1000 {
        sum += compute(i);  // Avoided effects for "performance"
    }
    sum
}

// DO: Clear code, benchmark if needed
fn process() -> i32 / Log {
    let mut sum = 0;
    for i in 0..1000 {
        log(format!("processing {}", i));  // ~1 cycle overhead
        sum += compute(i);
    }
    sum
}
// The logging adds ~1000 cycles = ~333ns total
// If compute() takes 1µs each, logging is 0.03% overhead
```

### 6.7 Optimization Strategies

#### Strategy 1: Use Tail-Resumptive Handlers

```blood
// FAST: Tail-resumptive
op get() { resume(state) }           // ~1 cycle

// SLOW: Non-tail (unnecessary)
op get() { let r = resume(state); r } // ~65 cycles
```

#### Strategy 2: Batch Effect Operations

```blood
// SLOW: Effect per item
fn sum_items(items: &[i32]) -> i32 / Log {
    let mut total = 0;
    for item in items {
        log(format!("adding {}", item));  // 1000 log calls
        total += item;
    }
    total
}

// FAST: Batch logging
fn sum_items(items: &[i32]) -> i32 / Log {
    log(format!("summing {} items", items.len()));  // 1 log call
    items.iter().sum()
}
```

#### Strategy 3: Minimize Captured References

```blood
// SLOW: Capture large structure
fn process(data: &LargeStruct) / Async {
    perform Async.yield();  // Captures all of data's refs
    use_data(data);
}

// FAST: Extract needed data first
fn process(data: &LargeStruct) / Async {
    let value = data.key_field;  // Extract what's needed
    perform Async.yield();       // Only captures 'value'
    use_value(value);
}
```

#### Strategy 4: Avoid Deep Nesting

```blood
// SLOW: Deep nesting
fn nested_example() {
    with H1 {} handle {
        with H2 {} handle {
            with H3 {} handle {
                with H4 {} handle {
                    with H5 {} handle {
                        compute()  // 5 lookups per effect
                    }
                }
            }
        }
    }
}

// FAST: Combine handlers
fn flat_example() {
    with CombinedHandler {} handle {
        compute()  // 1 lookup per effect
    }
}
```

### 6.8 Performance Comparison with Other Approaches

| Approach | Overhead per Operation | Notes |
|----------|------------------------|-------|
| Direct function call | ~0.5 cycles | Baseline |
| Blood tail-resumptive | ~1.3 cycles | 2.6x baseline |
| Blood continuation | ~65 cycles | Complex control flow |
| Rust Result (ok path) | ~1-2 cycles | Compare to tail-resumptive |
| Rust Result (err path) | ~10-20 cycles | Compare to continuation |
| Exception (Java) | ~1000+ cycles | Orders of magnitude more |
| Virtual dispatch | ~5-10 cycles | Compare to evidence lookup |

Blood's effect system is competitive with or faster than traditional alternatives.

---

## 7. Best Practices

### DO: Declare Effects Explicitly

```blood
// Good: effects are explicit
fn read_config() -> Config / {IO, Error<IOError>} {
    // ...
}

// Bad: hiding effects in untyped code
fn read_config() -> Config {  // What effects?!
    // ...
}
```

### DO: Use Handler Composition

```blood
// Good: compose handlers
fn main() {
    with ErrorHandler {} handle {
        with StateHandler { state: initial } handle {
            run_application()
        }
    }
}
```

### DON'T: Resume Multiple Times (Usually)

```blood
// Dangerous: multi-shot handlers are advanced
op choose(a, b) {
    resume(a)  // First shot
    resume(b)  // Second shot - advanced!
}
```

### DO: Handle All Operations

```blood
// Good: all operations handled
deep handler Complete for MyEffect {
    op foo() { resume(()) }
    op bar() { resume(()) }
    op baz() { resume(()) }  // All three!
}

// Bad: incomplete handler
deep handler Incomplete for MyEffect {
    op foo() { resume(()) }
    // bar and baz missing - error!
}
```

### DO: Use Effects for Dependency Injection

```blood
// Production handler
deep handler ProdDatabase for Database {
    op query(sql) { /* real DB */ }
}

// Test handler
deep handler MockDatabase for Database {
    op query(sql) { /* return test data */ }
}

// Same code, different handlers
fn test_my_service() {
    with MockDatabase {} handle {
        my_service()  // Uses mock DB
    }
}
```

---

## Related Documentation

- [SPECIFICATION.md §4](./SPECIFICATION.md#4-effect-system) — Formal effect system rules
- [FORMAL_SEMANTICS.md](./FORMAL_SEMANTICS.md) — Operational semantics of effects
- [MEMORY_MODEL.md §5.3](./MEMORY_MODEL.md) — Generation snapshots for effect safety
- [examples/algebraic_effects.blood](../examples/algebraic_effects.blood) — Comprehensive example code

---

*Last updated: 2026-01-13*
