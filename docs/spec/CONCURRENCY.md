# Blood Concurrency Specification

**Version**: 0.4.0
**Status**: Specification target
**Last Updated**: 2026-02-28

**Revision 0.4.0 Changes** (ADR-036 — Cohesive Concurrency Model):
- Unified naming: `Fiber` throughout (replaced `Async` in §8); `Async` effect removed
- Added `Cancel` effect with ADR-036 semantics (§4.5, §8.1): cooperative, separate from `Fiber`
- Replaced `Send`/`Sync` traits with tier-based fiber-crossing rules (§2.4, §8.1)
- Added `spawn_blocking` operation for FFI interop (§2.4, §8.1)
- Added handler finalization (`finally` clause) integration (§4.6)
- Added generation snapshot cost model (§9.4)
- Updated preemption mechanism to compiler-inserted safepoints (§3.5)
- Added deep/shallow handler concurrency semantics (§8.2)
- Added fiber-local storage via `State` effect (§8.4)
- Added streams via effect composition (§8.5)
- Added priority inversion mitigation (§3.6)
- Added five-pillar leverage summary (§8.6)
- See ADR-036 for full design rationale

**Revision 0.3.0 Changes**:
- Added implementation status link
- Updated implementation status to reflect runtime integration

This document specifies Blood's concurrency model, including fiber semantics, scheduling, synchronization primitives, and parallel execution.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Fiber Model](#2-fiber-model) — Fiber-crossing rules, spawn_blocking (ADR-036)
3. [Scheduler](#3-scheduler) — Safepoint preemption, priority inheritance (ADR-036)
4. [Fiber Lifecycle](#4-fiber-lifecycle) — Cancel effect, handler finalization (ADR-036)
5. [Communication](#5-communication)
6. [Synchronization](#6-synchronization)
7. [Parallel Primitives](#7-parallel-primitives)
8. [Effect Integration](#8-effect-integration) — Cohesive model, deep/shallow, streams (ADR-036)
9. [Memory Model](#9-memory-model) — Generation snapshot cost model (ADR-036)
10. [Platform Mapping](#10-platform-mapping)
11. [Runtime Linking Requirements](#11-runtime-linking-requirements)

---

## 1. Overview

### 1.1 Design Goals

Blood's concurrency model provides:

1. **Lightweight Concurrency** — Millions of concurrent fibers
2. **Cooperative Scheduling** — Predictable yield points
3. **Memory Safety** — No data races by construction
4. **Effect Integration** — Concurrency as an effect
5. **Structured Concurrency** — Child fibers complete before parent

### 1.2 Related Specifications

- [SPECIFICATION.md](./SPECIFICATION.md) — Core language specification
- [MEMORY_MODEL.md](./MEMORY_MODEL.md) — Region-fiber isolation rules
- [STDLIB.md](./STDLIB.md) — Fiber and Channel effects
- [FORMAL_SEMANTICS.md](./FORMAL_SEMANTICS.md) — Effect handler semantics
- [FFI.md](./FFI.md) — FFI interaction with fibers
- [ROADMAP.md](./ROADMAP.md) — Runtime implementation phases

### 1.3 Implementation Status

The following table tracks implementation status of concurrency subsystems:

| Component | Status | Location | Notes |
|-----------|--------|----------|-------|
| FiberId, FiberState | ✅ Implemented | `blood-runtime/src/fiber.rs` | Core fiber identity |
| FiberConfig | ✅ Implemented | `blood-runtime/src/fiber.rs` | Stack size, priority |
| FiberStack | ✅ Implemented | `blood-runtime/src/fiber.rs` | Growable stack |
| WakeCondition | ✅ Implemented | `blood-runtime/src/fiber.rs` | Channel, timer, IO |
| Scheduler | ✅ Implemented | `blood-runtime/src/scheduler.rs` | Work-stealing M:N |
| Worker threads | ✅ Implemented | `blood-runtime/src/scheduler.rs` | Per-core workers |
| blood_scheduler_* exports | ✅ Integrated | `blood-runtime/src/ffi_exports.rs` | Runtime scheduler FFI |
| MPMC channels | ✅ Implemented | `blood-runtime/src/channel.rs` | Bounded/unbounded |
| I/O reactor | ✅ Implemented | `blood-runtime/src/io.rs` | Platform-native async |
| Platform: Linux epoll | ✅ Implemented | `blood-runtime/src/io.rs` | Fallback driver |
| Platform: Linux io_uring | ✅ Implemented | `blood-runtime/src/io.rs` | Primary Linux driver |
| Platform: macOS kqueue | ✅ Implemented | `blood-runtime/src/io.rs` | Primary macOS driver |
| Platform: Windows IOCP | ✅ Implemented | `blood-runtime/src/io.rs` | Primary Windows driver |
| Fiber effect syntax | ✅ Implemented | `stdlib/effects/fiber.blood` | Per §2.4 specification |
| Structured concurrency | ✅ Implemented | `stdlib/effects/fiber.blood` | Nursery, FiberScope, par_map, etc. |
| Select/await syntax | ✅ Implemented | `stdlib/effects/fiber.blood` | SelectBuilder, await_first, select_timeout |

**Legend**: ✅ Implemented | 🔶 Partial | 📋 Designed | ❌ Not Started

### 1.4 Concurrency Philosophy

| Aspect | Blood Approach |
|--------|----------------|
| **Unit of Concurrency** | Fibers (stackful coroutines) |
| **Scheduling** | M:N cooperative with preemption points |
| **Communication** | Channels (typed, bounded) |
| **Shared State** | By default: none. Opt-in via `Synchronized<T>` |
| **Memory** | Fiber-local regions, shared via Tier 3 |

### 1.5 Comparison with Other Models

| Feature | Blood | Go | Erlang | Rust async |
|---------|-------|----|----|------------|
| **Concurrency Unit** | Fiber | Goroutine | Process | Task |
| **Stack** | Growable | Growable | Per-process | Stackless |
| **Scheduling** | M:N | M:N | M:N | M:N |
| **Communication** | Channels | Channels | Messages | Channels |
| **Shared Memory** | Opt-in | Yes | None | Yes (unsafe) |
| **GC** | None | Yes | Yes | None |

---

## 2. Fiber Model

### 2.1 What is a Fiber?

A **fiber** is a lightweight, cooperatively-scheduled unit of execution:

```
┌──────────────────────────────────────────────────────────────┐
│                           FIBER                               │
├──────────────────────────────────────────────────────────────┤
│                                                               │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐       │
│  │ Stack       │    │ Registers   │    │ State       │       │
│  │ (growable)  │    │ (saved)     │    │             │       │
│  ├─────────────┤    ├─────────────┤    ├─────────────┤       │
│  │ Local       │    │ PC, SP      │    │ Running     │       │
│  │ Variables   │    │ FP, etc.    │    │ Suspended   │       │
│  │             │    │             │    │ Completed   │       │
│  └─────────────┘    └─────────────┘    └─────────────┘       │
│                                                               │
│  ┌─────────────────────────────────────────────────────┐     │
│  │ Regions (Fiber-Local Memory)                         │     │
│  │ - Stack allocations                                  │     │
│  │ - Heap allocations (Tier 1)                          │     │
│  │ - Cannot be accessed by other fibers                 │     │
│  └─────────────────────────────────────────────────────┘     │
│                                                               │
└──────────────────────────────────────────────────────────────┘
```

### 2.2 Fiber Properties

| Property | Value | Status |
|----------|-------|--------|
| **Initial Stack Size** | 8 KB (configurable) | Design target |
| **Maximum Stack Size** | 1 MB (configurable) | Design target |
| **Stack Growth** | On-demand, 2x growth factor | Design target |
| **Context Switch Cost** | ~50-100 ns (register save/restore) | Unvalidated¹ |
| **Memory Overhead** | ~1-2 KB per suspended fiber | Unvalidated¹ |

¹ Performance estimates based on similar fiber implementations (Go goroutines, Tokio tasks). Actual performance will be validated during implementation.

### 2.3 Fiber Structure

```rust
struct Fiber {
    // Identity
    id: FiberId,
    parent: Option<FiberId>,

    // Execution state
    state: FiberState,
    stack: Stack,
    registers: SavedRegisters,

    // Scheduling
    priority: Priority,
    wake_condition: Option<WakeCondition>,

    // Memory
    local_regions: Vec<RegionId>,
    tier3_refs: Vec<Hash>,  // Shared data references

    // Effect handling
    installed_handlers: Vec<HandlerId>,
    suspended_at: Option<EffectOp>,

    // Debugging
    name: Option<String>,
    created_at: Timestamp,
}

enum FiberState {
    /// Ready to run
    Runnable,

    /// Currently executing on a worker thread
    Running(WorkerId),

    /// Waiting for an event
    Suspended(WakeCondition),

    /// Waiting for child fibers
    Joining(Vec<FiberId>),

    /// Completed successfully
    Completed(Value),

    /// Failed with error
    Failed(Error),

    /// Cancelled
    Cancelled,
}

enum WakeCondition {
    /// Channel has data
    ChannelReadable(ChannelId),

    /// Channel has space
    ChannelWritable(ChannelId),

    /// Timer expired
    Timeout(Instant),

    /// I/O ready
    IoReady(Fd, IoInterest),

    /// Effect resumed
    EffectResumed,

    /// Any of these conditions
    Any(Vec<WakeCondition>),
}
```

### 2.4 Fiber Creation

```blood
effect Fiber {
    /// Spawn a new fiber (compiler checks captured values are fiber-transferable)
    op spawn<T>(f: fn() -> T / {Fiber}) -> FiberHandle<T>;

    /// Spawn with configuration
    op spawn_with<T>(
        config: FiberConfig,
        f: fn() -> T / {Fiber}
    ) -> FiberHandle<T>;

    /// Spawn on a dedicated OS thread (for blocking FFI — ADR-036 Sub-8)
    op spawn_blocking<T>(f: fn() -> T) -> FiberHandle<T>;

    /// Get current fiber's handle
    op current() -> FiberHandle<()>;

    /// Yield to scheduler
    op yield();

    /// Sleep for duration
    op sleep(duration: Duration);

    /// Join a fiber (wait for completion)
    op join<T>(handle: FiberHandle<T>) -> T;
}

struct FiberConfig {
    name: Option<String>,
    stack_size: usize,
    priority: Priority,
}

struct FiberHandle<T> {
    id: FiberId,
    // Phantom type for result
    _phantom: PhantomData<T>,
}
```

### 2.5 Fiber Syntax

```blood
fn example() / {Fiber, IO} {
    // Spawn a fiber
    let handle = spawn(|| {
        heavy_computation()
    });

    // Do other work concurrently
    let local_result = light_computation();

    // Wait for fiber to complete
    let fiber_result = join(handle);

    (local_result, fiber_result)
}

// Named fiber with configuration
fn configured_example() / {Fiber} {
    let handle = spawn_with(
        FiberConfig {
            name: Some("worker"),
            stack_size: 64 * 1024,  // 64 KB
            priority: Priority::High,
        },
        || { work() }
    );

    join(handle)
}

// Blocking FFI interop (ADR-036 Sub-8)
fn ffi_example() / {Fiber} {
    // Runs on dedicated OS thread, outside fiber scheduler
    let result = spawn_blocking(|| {
        // Safe to call blocking C library functions here
        c_blocking_read(fd, buf, len)
    });

    join(result)
}
```

---

## 3. Scheduler

### 3.1 M:N Scheduling

Blood uses M:N scheduling: M fibers mapped to N OS threads.

```
┌─────────────────────────────────────────────────────────────────┐
│                       RUNTIME SCHEDULER                          │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌───────────────────────────────────────────────────────┐      │
│  │                   RUN QUEUES                           │      │
│  │                                                        │      │
│  │  Global: [ F1 ]──[ F5 ]──[ F9 ]                        │      │
│  │                                                        │      │
│  │  Local (Worker 0): [ F2 ]──[ F6 ]                      │      │
│  │  Local (Worker 1): [ F3 ]──[ F7 ]                      │      │
│  │  Local (Worker 2): [ F4 ]──[ F8 ]                      │      │
│  │                                                        │      │
│  └───────────────────────────────────────────────────────┘      │
│                          ↓                                       │
│  ┌───────────────────────────────────────────────────────┐      │
│  │                   WORKER THREADS                       │      │
│  │                                                        │      │
│  │  ┌─────────┐    ┌─────────┐    ┌─────────┐            │      │
│  │  │ Worker 0│    │ Worker 1│    │ Worker 2│    ...     │      │
│  │  │ (Core 0)│    │ (Core 1)│    │ (Core 2)│            │      │
│  │  └─────────┘    └─────────┘    └─────────┘            │      │
│  │       ↓              ↓              ↓                  │      │
│  │  OS Thread 0    OS Thread 1    OS Thread 2             │      │
│  │                                                        │      │
│  └───────────────────────────────────────────────────────┘      │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### 3.2 Scheduler Structure

```rust
struct Scheduler {
    // Worker threads
    workers: Vec<Worker>,
    num_workers: usize,

    // Global run queue (for load balancing)
    global_queue: ConcurrentQueue<FiberId>,

    // Sleeping fibers (timer heap)
    timer_heap: BinaryHeap<(Instant, FiberId)>,

    // I/O reactor
    io_reactor: IoReactor,

    // Statistics
    stats: SchedulerStats,

    // Shutdown coordination
    shutdown: AtomicBool,
}

struct Worker {
    id: WorkerId,

    // Local run queue (work stealing)
    local_queue: WorkStealingQueue<FiberId>,

    // Currently running fiber
    current_fiber: Option<FiberId>,

    // Random number generator (for work stealing)
    rng: FastRng,

    // Statistics
    stats: WorkerStats,
}
```

### 3.3 Scheduling Algorithm

```
SCHEDULER_LOOP(worker):
    LOOP:
        // 1. Check local queue first
        fiber ← worker.local_queue.pop()

        IF fiber.is_none():
            // 2. Try global queue
            fiber ← scheduler.global_queue.pop()

        IF fiber.is_none():
            // 3. Try to steal from other workers
            victim ← worker.rng.select_victim(scheduler.workers)
            fiber ← victim.local_queue.steal()

        IF fiber.is_none():
            // 4. Park until work available
            PARK(worker)
            CONTINUE

        // Run the fiber
        result ← RUN_FIBER(fiber)

        MATCH result:
            | Yielded → worker.local_queue.push(fiber)
            | Suspended(cond) → REGISTER_WAKE(fiber, cond)
            | Completed(val) → COMPLETE_FIBER(fiber, val)
            | Failed(err) → FAIL_FIBER(fiber, err)
```

### 3.4 Yield Points

Fibers yield cooperatively at defined points:

| Yield Point | Trigger |
|-------------|---------|
| `yield()` | Explicit yield |
| `join(handle)` | Waiting for another fiber |
| `channel.send()` | Channel full |
| `channel.recv()` | Channel empty |
| `sleep(duration)` | Timer |
| `perform(effect)` | Effect operation |
| Function call | Optional preemption check |

### 3.5 Preemption via Compiler-Inserted Safepoints

Blood uses compiler-inserted safepoints for preemption, matching Go 1.14's approach. Safepoints are inserted at:

1. **Loop back-edges** — ensures long-running loops can be preempted
2. **Function prologues** — ensures deep call chains can be preempted

Each safepoint checks a per-fiber preemption flag set by the scheduler when the fiber's quantum expires:

```llvm
; Safepoint check (~1 cycle when not preempting, branch predicted not-taken)
%preempt = load i8, ptr %fiber.preempt_flag
%should_yield = icmp ne i8 %preempt, 0
br i1 %should_yield, label %yield_point, label %continue
```

**Cost**: ~1 cycle per safepoint when not preempting. Code size increase ~1-2%.

**Safepoint disabling**: `#[unchecked(preemption)]` (extending RFC-S, ADR-031) disables safepoint insertion in performance-critical code. The programmer accepts starvation risk.

**Why safepoints over signals**: Signal-based preemption (SIGALRM) is unpredictable in delivery point and interacts with FFI signal handlers. Safepoints are predictable (only fire at known locations) and don't interfere with external C libraries.

```blood
fn long_loop() / {Fiber} {
    for i in 0..1_000_000 {
        // Compiler-inserted safepoint here (loop back-edge)
        compute(i);
    }
}

#[unchecked(preemption)]
fn hot_inner_loop(data: &[f64]) -> f64 {
    // No safepoints — maximum throughput, starvation risk accepted
    let mut sum = 0.0;
    for i in 0..data.len() {
        sum += data[i];
    }
    sum
}
```

### 3.6 Priority Scheduling

```rust
enum Priority {
    Low = 0,
    Normal = 1,      // Default
    High = 2,
    Critical = 3,    // For system fibers
}

impl Scheduler {
    fn select_fiber(&self) -> Option<FiberId> {
        // Higher priority fibers run first
        for priority in [Critical, High, Normal, Low] {
            if let Some(fiber) = self.get_runnable(priority) {
                return Some(fiber);
            }
        }
        None
    }
}
```

**Priority inversion mitigation** (ADR-036): The default scheduler uses priority inheritance — when a high-priority fiber joins (waits on) a low-priority fiber, the low-priority fiber inherits the higher priority. This prevents medium-priority fibers from starving the high-priority one. Priority ceiling (resources carry a priority ceiling as a type-level property) is available for real-time scheduler handlers. Both are handler implementation choices, not language-level changes.

---

## 4. Fiber Lifecycle

### 4.1 State Machine

```
                    spawn()
                       │
                       ▼
              ┌────────────────┐
              │   Runnable     │◄──────────┐
              └───────┬────────┘           │
                      │                     │
                 schedule                   │ wake
                      │                     │
                      ▼                     │
              ┌────────────────┐           │
              │    Running     │           │
              └───────┬────────┘           │
                      │                     │
         ┌────────────┼────────────┐       │
         │            │            │       │
    complete     suspend        yield      │
         │            │            │       │
         ▼            ▼            │       │
┌──────────────┐ ┌──────────────┐ │       │
│  Completed   │ │  Suspended   │─┴───────┘
└──────────────┘ └──────────────┘
         │                │
         │           cancel
         │                │
         ▼                ▼
      (done)       ┌──────────────┐
                   │  Cancelled   │
                   └──────────────┘
```

### 4.2 Spawn Operation

```
SPAWN(f):
    // 1. Allocate fiber
    fiber_id ← allocate_fiber_id()
    stack ← allocate_stack(INITIAL_STACK_SIZE)

    fiber ← Fiber {
        id: fiber_id,
        parent: current_fiber_id(),
        state: Runnable,
        stack,
        // ...
    }

    // 2. Initialize stack with trampoline
    setup_trampoline(fiber, f)

    // 3. Add to parent's children (structured concurrency)
    current_fiber.children.push(fiber_id)

    // 4. Add to run queue
    scheduler.local_queue.push(fiber_id)

    // 5. Return handle
    RETURN FiberHandle { id: fiber_id }
```

### 4.3 Join Operation

```
JOIN(handle):
    target ← get_fiber(handle.id)

    MATCH target.state:
        | Completed(value) →
            RETURN value

        | Failed(error) →
            RAISE error

        | Cancelled →
            RAISE CancelledError

        | _ →
            // Suspend current fiber until target completes
            current_fiber.state ← Joining([handle.id])
            YIELD_TO_SCHEDULER()

            // When resumed, target has completed
            RETURN JOIN(handle)  // Retry
```

### 4.4 Structured Concurrency

All child fibers must complete before their parent:

```blood
fn structured_example() / {Fiber} {
    let h1 = spawn(|| task1());
    let h2 = spawn(|| task2());

    // Implicit: parent waits for h1, h2 before returning
    let r1 = join(h1);
    let r2 = join(h2);

    (r1, r2)
}
// h1 and h2 guaranteed complete here

// Nursery pattern for explicit scoping
fn nursery_example() / {Fiber} {
    nursery(|scope| {
        scope.spawn(|| task1());
        scope.spawn(|| task2());
        scope.spawn(|| task3());
        // All tasks complete when nursery exits
    })
}
```

### 4.5 Cancellation (ADR-036 Sub-2)

Cancellation is a separate `Cancel` effect, distinct from `Fiber`. This makes cancellation points visible in function signatures (ADR-036 Sub-2).

```blood
effect Cancel {
    /// Check if cancelled — yields if cancelled, resumes if not
    op check_cancelled() -> unit;
}
```

**Visibility in types**: `fn work() / {Fiber, Cancel}` has cancellation points. `fn work() / {Fiber}` runs to completion — no cooperative cancellation possible.

**Cancellation protocol**:
1. A parent scope requests cancellation of a child (sets a flag)
2. The child's `Cancel` handler checks the flag when `check_cancelled()` is performed
3. If cancelled, the handler does NOT resume the child's continuation — the child terminates
4. If not cancelled, the handler resumes normally
5. Cancellation only occurs at explicit `check_cancelled()` points — it is cooperative

```blood
fn cancellable_task() / {Fiber, Cancel} {
    for item in items {
        check_cancelled();  // Cancellation point — visible in signature
        process(item);
    }
}

fn cancel_example() / {Fiber} {
    nursery(|scope| {
        let handle = scope.spawn(|| cancellable_task());

        sleep(Duration::seconds(5));
        scope.cancel(handle);  // Sets cancellation flag

        // Handler finalization (finally) ensures cleanup
    })
}
```

**Cancellation safety guarantees** (ADR-036 Sub-3):
1. **Memory safety**: Fiber-local regions bulk-deallocated (O(1)). No other fiber holds references (MEMORY_MODEL.md Theorem 5).
2. **Resource safety**: Linear values must be consumed. Compiler ensures cleanup code runs or rejects the program.
3. **Handler finalization**: All nested handler `finally` clauses run in reverse order.
4. **No cross-fiber corruption**: Region isolation prevents any cancelled fiber from affecting another fiber's state.

Implementation:

```
CANCEL(handle):
    fiber ← get_fiber(handle.id)

    // Set cancellation flag
    fiber.cancel_requested ← true

    // If suspended, wake it up
    IF fiber.state == Suspended(_):
        fiber.state ← Runnable
        scheduler.enqueue(fiber.id)

    // Cancellation is cooperative — fiber must reach check_cancelled()
    // Cancel handler then does NOT resume the continuation
```

### 4.6 Handler Finalization on Scope Exit

When a fiber is cancelled or a handler scope exits abnormally, all nested handler `finally` clauses run in reverse nesting order (innermost first). See GRAMMAR.md §3.4.3 and ADR-036 Sub-4.

```blood
deep handler ManagedDB for Database {
    let conn: linear Connection

    return(x) { x }

    finally {
        conn.close()  // Guaranteed to run on any scope exit
    }

    op query(sql) {
        let result = conn.execute(sql)
        resume(result)
    }
}
```

**Key rules**:
- `finally` runs in the enclosing handler context (may perform effects from enclosing scopes, NOT from the handler being torn down)
- `finally` clauses are non-cancellable — `Cancel` handler is not installed around them
- Normal exit: `return` runs, then `finally`
- Abnormal exit: `finally` only

---

## 5. Communication

### 5.1 Channels

Channels are typed, bounded queues for fiber communication:

```blood
effect Channel<T> {
    /// Create a new channel
    op channel<T>(capacity: usize) -> (Sender<T>, Receiver<T>);

    /// Send a value (blocks if full)
    op send(value: T);

    /// Receive a value (blocks if empty)
    op recv() -> T;

    /// Try operations (non-blocking)
    op try_send(value: T) -> Result<(), Full<T>>;
    op try_recv() -> Result<T, Empty>;
}

fn channel_example() / {Fiber, Channel<i32>} {
    let (tx, rx) = channel(10);  // Capacity 10

    spawn(move || {
        for i in 0..100 {
            tx.send(i);  // Blocks if full
        }
        drop(tx);  // Close sender
    });

    loop {
        match rx.try_recv() {
            Ok(value) => process(value),
            Err(Empty) => yield(),
            Err(Closed) => break,
        }
    }
}
```

### 5.2 Channel Implementation

```rust
struct Channel<T> {
    // Bounded buffer
    buffer: ArrayQueue<T>,
    capacity: usize,

    // Waiting senders/receivers
    waiting_senders: WaitList<FiberId>,
    waiting_receivers: WaitList<FiberId>,

    // State
    closed: AtomicBool,
    sender_count: AtomicUsize,
    receiver_count: AtomicUsize,
}

impl<T> Channel<T> {
    fn send(&self, value: T) -> Result<(), Closed> {
        loop {
            if self.closed.load() {
                return Err(Closed);
            }

            if self.buffer.push(value).is_ok() {
                // Wake a waiting receiver
                if let Some(fiber) = self.waiting_receivers.pop() {
                    scheduler.wake(fiber);
                }
                return Ok(());
            }

            // Buffer full - wait
            self.waiting_senders.push(current_fiber_id());
            suspend(ChannelWritable(self.id));
        }
    }

    fn recv(&self) -> Result<T, Closed> {
        loop {
            if let Some(value) = self.buffer.pop() {
                // Wake a waiting sender
                if let Some(fiber) = self.waiting_senders.pop() {
                    scheduler.wake(fiber);
                }
                return Ok(value);
            }

            if self.closed.load() && self.buffer.is_empty() {
                return Err(Closed);
            }

            // Buffer empty - wait
            self.waiting_receivers.push(current_fiber_id());
            suspend(ChannelReadable(self.id));
        }
    }
}
```

### 5.3 Channel Patterns

```blood
// Fan-out: one producer, multiple consumers
fn fan_out() / {Fiber} {
    let (tx, rx) = channel(100);

    // Spawn workers
    for _ in 0..4 {
        let rx = rx.clone();
        spawn(move || worker(rx));
    }

    // Produce work
    for item in work_items {
        tx.send(item);
    }
}

// Fan-in: multiple producers, one consumer
fn fan_in() / {Fiber} {
    let (tx, rx) = channel(100);

    // Spawn producers
    for source in sources {
        let tx = tx.clone();
        spawn(move || producer(source, tx));
    }
    drop(tx);  // Drop original sender

    // Consume all
    while let Ok(item) = rx.recv() {
        process(item);
    }
}

// Pipeline: chain of processing stages
fn pipeline() / {Fiber} {
    let (tx1, rx1) = channel(10);
    let (tx2, rx2) = channel(10);
    let (tx3, rx3) = channel(10);

    spawn(|| stage1(tx1));
    spawn(|| stage2(rx1, tx2));
    spawn(|| stage3(rx2, tx3));

    collect(rx3)
}
```

### 5.4 Select

Wait on multiple channel operations:

```blood
fn select_example() / {Fiber} {
    let (tx1, rx1) = channel(10);
    let (tx2, rx2) = channel(10);

    loop {
        select! {
            value = rx1.recv() => {
                handle_type1(value);
            },
            value = rx2.recv() => {
                handle_type2(value);
            },
            default => {
                // No channel ready
                yield();
            },
            timeout(Duration::seconds(1)) => {
                // Timeout
                break;
            },
        }
    }
}
```

---

## 6. Synchronization

### 6.1 Mutex

For shared mutable state (use sparingly):

```blood
struct Mutex<T> {
    value: UnsafeCell<T>,
    locked: AtomicBool,
    waiters: WaitList<FiberId>,
}

impl<T> Mutex<T> {
    fn new(value: T) -> Mutex<T> { ... }

    fn lock(&self) -> MutexGuard<T> / {Fiber} {
        loop {
            if self.locked.compare_exchange(false, true).is_ok() {
                return MutexGuard { mutex: self };
            }
            // Wait for unlock
            self.waiters.push(current_fiber_id());
            suspend(MutexUnlocked(self.id));
        }
    }

    fn try_lock(&self) -> Option<MutexGuard<T>> {
        if self.locked.compare_exchange(false, true).is_ok() {
            Some(MutexGuard { mutex: self })
        } else {
            None
        }
    }
}

impl<T> Drop for MutexGuard<T> {
    fn drop(&mut self) {
        self.mutex.locked.store(false);
        // Wake one waiter
        if let Some(fiber) = self.mutex.waiters.pop() {
            scheduler.wake(fiber);
        }
    }
}
```

### 6.2 RwLock

Reader-writer lock:

```blood
struct RwLock<T> {
    value: UnsafeCell<T>,
    state: AtomicIsize,  // Positive = readers, -1 = writer
    waiting_writers: WaitList<FiberId>,
    waiting_readers: WaitList<FiberId>,
}

impl<T> RwLock<T> {
    fn read(&self) -> ReadGuard<T> / {Fiber} { ... }
    fn write(&self) -> WriteGuard<T> / {Fiber} { ... }
}
```

### 6.3 Semaphore

```blood
struct Semaphore {
    permits: AtomicUsize,
    waiters: WaitList<FiberId>,
}

impl Semaphore {
    fn new(permits: usize) -> Semaphore { ... }

    fn acquire(&self) / {Fiber} {
        loop {
            let current = self.permits.load();
            if current > 0 {
                if self.permits.compare_exchange(current, current - 1).is_ok() {
                    return;
                }
            } else {
                self.waiters.push(current_fiber_id());
                suspend(SemaphoreAvailable(self.id));
            }
        }
    }

    fn release(&self) {
        self.permits.fetch_add(1);
        if let Some(fiber) = self.waiters.pop() {
            scheduler.wake(fiber);
        }
    }
}
```

### 6.4 Barrier

```blood
struct Barrier {
    count: usize,
    waiting: AtomicUsize,
    generation: AtomicUsize,
    waiters: WaitList<FiberId>,
}

impl Barrier {
    fn wait(&self) / {Fiber} {
        let gen = self.generation.load();
        let arrived = self.waiting.fetch_add(1) + 1;

        if arrived == self.count {
            // Last to arrive - release all
            self.waiting.store(0);
            self.generation.fetch_add(1);
            for fiber in self.waiters.drain() {
                scheduler.wake(fiber);
            }
        } else {
            // Wait for others
            self.waiters.push(current_fiber_id());
            loop {
                suspend(BarrierReleased(self.id));
                if self.generation.load() != gen {
                    break;
                }
            }
        }
    }
}
```

### 6.5 Once

```blood
struct Once {
    state: AtomicU8,  // 0 = uninitialized, 1 = initializing, 2 = initialized
    waiters: WaitList<FiberId>,
}

impl Once {
    fn call_once<F: FnOnce()>(&self, f: F) / {Fiber} {
        match self.state.load() {
            2 => return,  // Already initialized
            1 => {
                // Another fiber is initializing - wait
                self.waiters.push(current_fiber_id());
                suspend(OnceInitialized(self.id));
                return;
            }
            0 => {
                if self.state.compare_exchange(0, 1).is_ok() {
                    f();
                    self.state.store(2);
                    for fiber in self.waiters.drain() {
                        scheduler.wake(fiber);
                    }
                } else {
                    self.call_once(f);  // Retry
                }
            }
        }
    }
}
```

---

## 7. Parallel Primitives

### 7.1 Parallel Iterators

```blood
trait ParallelIterator<T> {
    fn par_map<U>(self, f: fn(T) -> U) -> Vec<U> / {Fiber};
    fn par_filter(self, f: fn(&T) -> bool) -> Vec<T> / {Fiber};
    fn par_reduce(self, identity: T, f: fn(T, T) -> T) -> T / {Fiber};
    fn par_for_each(self, f: fn(T)) / {Fiber};
}

impl ParallelIterator<T> for Vec<T> {
    fn par_map<U>(self, f: fn(T) -> U) -> Vec<U> / {Fiber} {
        let num_chunks = num_workers();
        let chunk_size = (self.len() + num_chunks - 1) / num_chunks;

        let results: Vec<FiberHandle<Vec<U>>> = self
            .chunks(chunk_size)
            .map(|chunk| spawn(move || chunk.iter().map(&f).collect()))
            .collect();

        results.into_iter()
            .flat_map(|h| join(h))
            .collect()
    }
}

// Usage
fn parallel_example() / {Fiber} {
    let data: Vec<i32> = (0..1_000_000).collect();

    let squared: Vec<i32> = data.par_map(|x| x * x);

    let sum: i32 = squared.par_reduce(0, |a, b| a + b);

    sum
}
```

### 7.2 Parallel Scope

```blood
fn parallel_scope<R>(f: fn(&Scope) -> R) -> R / {Fiber} {
    let scope = Scope::new();
    let result = f(&scope);
    scope.wait_all();  // Structured concurrency
    result
}

struct Scope {
    fibers: Vec<FiberHandle<()>>,
}

impl Scope {
    fn spawn(&mut self, f: fn() / {Fiber}) {
        self.fibers.push(spawn(f));
    }

    fn wait_all(&self) / {Fiber} {
        for handle in &self.fibers {
            join(handle.clone());
        }
    }
}

// Usage
fn scope_example() / {Fiber} {
    let data = vec![1, 2, 3, 4, 5];
    let results = Mutex::new(Vec::new());

    parallel_scope(|scope| {
        for item in data {
            scope.spawn(move || {
                let r = compute(item);
                results.lock().push(r);
            });
        }
    });

    results.into_inner()
}
```

### 7.3 Work Stealing

```blood
/// Work-stealing deque for load balancing
struct WorkStealingDeque<T> {
    // Owner pushes/pops from bottom
    bottom: AtomicIsize,

    // Stealers steal from top
    top: AtomicIsize,

    // Circular buffer
    buffer: AtomicPtr<[T]>,
}

impl<T> WorkStealingDeque<T> {
    /// Owner: push to bottom
    fn push(&self, item: T) { ... }

    /// Owner: pop from bottom
    fn pop(&self) -> Option<T> { ... }

    /// Thief: steal from top
    fn steal(&self) -> Option<T> { ... }
}
```

---

## 8. Effect Integration (ADR-036)

### 8.1 Concurrency as Effects

All concurrency operations are effects. The `Async` effect from earlier spec versions is removed — `Fiber` is the sole concurrency effect (ADR-036 Preliminary).

```blood
effect Fiber {
    op spawn<T>(f: fn() -> T / {Fiber}) -> FiberHandle<T>;
    op spawn_with<T>(config: FiberConfig, f: fn() -> T / {Fiber}) -> FiberHandle<T>;
    op spawn_blocking<T>(f: fn() -> T) -> FiberHandle<T>;
    op current() -> FiberHandle<()>;
    op yield();
    op sleep(duration: Duration);
    op join<T>(handle: FiberHandle<T>) -> T;
}

effect Cancel {
    op check_cancelled() -> unit;
}

effect Channel<T> {
    op channel(capacity: usize) -> (Sender<T>, Receiver<T>);
    op send(value: T);
    op recv() -> T;
}
```

**Fiber-crossing rules** (replaces Rust-style `Send`/`Sync` traits): Whether a value can cross fiber boundaries is determined automatically by the compiler from the value's **memory tier**. No explicit trait bounds are needed — the compiler checks at `spawn` call sites that all captured values are fiber-transferable.

| Memory Tier | Transferable? | Shareable? | Rationale |
|-------------|--------------|-----------|-----------|
| Tier 0 (stack) | Yes | No | Pure value — copy/move semantics |
| Tier 1 (region), mutable | No | No | Fiber-local region — region isolation invariant |
| Tier 1 (region), Frozen | Yes | Yes | Deeply immutable — no mutation hazard |
| Tier 2/3 (persistent) | Yes | Yes | Ref-counted, designed for cross-fiber sharing |
| Linear values | Yes (transfer) | No | Unique ownership moves to target fiber |
| Raw pointers | No | No | No safety guarantees — requires `@unsafe` |

> **Design note**: Blood does not have `Send` or `Sync` traits. In Rust, these traits exist because the type system has no concept of memory tiers — `Send`/`Sync` encode thread-safety information that Blood's tier system already captures. Blood's compiler derives fiber-crossing safety from the type's allocation tier, which is fundamentally more precise because it is based on the actual memory model rather than manually-maintained trait implementations. See GRAMMAR.md §3.4.1 for the design note on this decision.

### 8.2 Deep/Shallow Handler Concurrency Semantics

The deep/shallow handler distinction has specific concurrency implications (ADR-036 Sub-1):

**Deep handler** = recursive interception. Every `spawn` in the entire subtree is intercepted. This is *structural supervision*: the handler cannot be escaped by nested spawns.

**Shallow handler** = one-shot interception. Handles exactly one `spawn`, then the continuation runs without the handler.

```blood
// Deep handler: nursery pattern — supervises all spawns in subtree
deep handler Nursery for Fiber {
    let scheduler: Scheduler
    let children: Vec<FiberHandle<()>>

    return(x) {
        // Wait for all children before returning
        for child in children {
            scheduler.join(child);
        }
        x
    }

    finally {
        // Cancel remaining children on scope exit
        for child in children {
            scheduler.cancel(child);
        }
    }

    op spawn(f) {
        let handle = scheduler.spawn(f);
        children.push(handle);
        resume(handle)
    }

    op yield() {
        scheduler.yield_current();
        resume(())
    }

    op sleep(duration) {
        scheduler.sleep_current(duration);
        resume(())
    }

    op join(handle) {
        let result = scheduler.join(handle);
        resume(result)
    }
}

// Run concurrent computation
fn run<T>(f: fn() -> T / {Fiber}) -> T {
    let scheduler = Scheduler::new();
    with Nursery { scheduler, children: Vec::new() } handle {
        f()
    }
}
```

| Pattern | Handler Type | Formal Property |
|---------|-------------|-----------------|
| Nursery (supervise all) | Deep | Cannot be escaped — all spawns intercepted |
| One-shot spawn-and-join | Shallow | Handles exactly one spawn |
| Spawn with inspection | Shallow + re-install | Inspects each spawn before proceeding |
| Supervisor (isolate failures) | Deep + per-child error handling | Isolates child failures |

### 8.3 Fiber + Region Interaction

From MEMORY_MODEL.md Section 7.8:

```blood
// Regions are fiber-local
fn region_fiber_example() / {Fiber} {
    region local_data {
        let buffer = allocate_buffer();  // In local_data region

        // WRONG: Cannot share mutable region reference (not fiber-transferable)
        // spawn(|| use_buffer(&buffer));  // COMPILE ERROR: mutable Tier 1 reference is not fiber-transferable

        // CORRECT: Promote to Tier 3 (Frozen is fiber-transferable + shareable)
        let shared = persist(buffer.clone());
        spawn(|| use_buffer(&shared));

        // CORRECT: Linear transfer (moves ownership)
        let linear_buf = move_to_linear(buffer);
        spawn(move || consume_buffer(linear_buf));
    }
}
```

### 8.4 Fiber-Local Storage via State Effect

Fiber-local storage is modeled as a `State` effect scoped to the fiber's handler lifetime (ADR-036):

```blood
deep handler FiberLocal<T> for State<T> {
    let value: T

    return(x) { x }
    op get() { resume(value) }
    op set(new_val) { value = new_val; resume(()) }
}
```

`get()` and `set()` are tail-resumptive, so ADR-028's optimization applies — fiber-local access compiles to a direct memory read with zero effect dispatch overhead. This is both principled (visible in types as `/ {State<Config>}`) and zero-cost.

### 8.5 Streams via Effect Composition

Streams emerge from composing `Yield<T>` (generators) with `Fiber` (concurrency). No new abstraction needed (ADR-036 Sub-6):

```blood
// A stream: yields values, may suspend between them
fn sensor_readings() / {Yield<Reading>, Fiber} {
    loop {
        let reading = read_sensor()     // May suspend (Fiber)
        yield(reading)                  // Produce value (Yield<T>)
        sleep(Duration::seconds(1))     // Suspend between values (Fiber)
    }
}

// Consumer handles Yield<T> to receive values
fn consume_readings() / {Fiber} {
    with handle_readings handle {
        sensor_readings()
    }
}
```

**Backpressure**: The `Yield<T>` handler controls when to resume the producer. Delaying resumption = backpressure. Channels provide explicit backpressure via bounded capacity.

### 8.6 Five-Pillar Leverage Summary

Blood's concurrency model leverages all five language pillars (ADR-036):

| Pillar | Concurrency Role |
|--------|-----------------|
| **Effects** | `Fiber`, `Cancel`, `Yield` — concurrency as effect composition |
| **Handlers** | Deep/shallow = supervision patterns; `finally` = cleanup; handler scope = task scope |
| **Regions** | Fiber-local memory, O(1) bulk dealloc on cancellation, generation snapshots O(R_mutable) |
| **Linear types** | Cancellation safety, resource cleanup enforcement, ownership transfer |
| **Multiple dispatch** | Spawn strategy, channel transfer, observability specialization |
| **Content addressing** | Handler composition hashing, deterministic replay, pure fiber deduplication |

---

## 9. Memory Model

### 9.1 Fiber Memory Isolation

Each fiber has isolated memory:

| Memory Type | Visibility | Sharing Mechanism |
|-------------|------------|-------------------|
| Stack | Fiber-local | None |
| Tier 1 (Region) | Fiber-local | None (by design) |
| Tier 3 (Persistent) | Global | Explicit sharing |
| Channels | Shared | Message passing |

### 9.2 Data Race Prevention

Blood prevents data races by construction:

```
┌─────────────────────────────────────────────────────────────┐
│              DATA RACE PREVENTION                            │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  1. No shared mutable state by default                       │
│     - Fiber-local regions cannot be accessed by others       │
│     - Compiler rejects cross-fiber region references         │
│                                                              │
│  2. Tier 3 sharing requires:                                 │
│     - Frozen (immutable): Read-only, safe to share           │
│     - Synchronized<T>: Mutex-protected mutable               │
│                                                              │
│  3. Channels transfer ownership:                             │
│     - Sent value moves from sender to receiver               │
│     - No aliasing across fiber boundary                      │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

### 9.3 Memory Ordering

For atomics and synchronization:

| Ordering | Guarantee |
|----------|-----------|
| Relaxed | No ordering (only atomicity) |
| Acquire | Reads cannot move before |
| Release | Writes cannot move after |
| AcqRel | Both acquire and release |
| SeqCst | Total ordering (strongest) |

```blood
fn atomic_example() {
    let counter = AtomicI32::new(0);

    // Relaxed: just need atomicity
    counter.fetch_add(1, Ordering::Relaxed);

    // Release: publish updates
    data.store(value, Ordering::Relaxed);
    flag.store(true, Ordering::Release);

    // Acquire: see published updates
    if flag.load(Ordering::Acquire) {
        let v = data.load(Ordering::Relaxed);
    }
}
```

### 9.4 Generation Snapshot Cost Model (ADR-036)

During fiber context switching, generation snapshots validate that references haven't been invalidated during suspension. The snapshot uses bulk region-level comparison, not per-reference comparison.

**Specification**: Each fiber maintains `RegionSnapshot = Vec<(RegionId, Generation)>` captured at suspend, validated at resume.

| Tier | In snapshot? | Reason |
|------|-------------|--------|
| Tier 0 (stack) | No | Stack frames are fiber-local by construction |
| Tier 1 (region), mutable access | Yes | May be invalidated during suspension |
| Tier 1 (region), Frozen access | No | Immutable — generation counter never advances |
| Tier 2/3 (persistent) | No | Uses refcounting, not generations |

**Cost**: O(R_mutable) where R_mutable = count of mutable Tier 1 regions with live references. For the vast majority of fibers (those that only mutate their own region), R_mutable = 1. This is effectively O(1).

**Validation**: One integer comparison per snapshot entry (~4 cycles per entry, per MEMORY_MODEL.md estimates). Total context switch overhead from generation validation: ~4 cycles for typical fibers.

---

## 10. Platform Mapping

### 10.1 Worker Thread Mapping

| Platform | Worker Threads | Notes |
|----------|---------------|-------|
| Linux | `sched_setaffinity` | Core pinning |
| macOS | `pthread_setaffinity_np` | Limited |
| Windows | `SetThreadAffinityMask` | Full support |
| WASM | Single-threaded | Web Workers planned |

### 10.2 I/O Integration

| Platform | I/O Mechanism |
|----------|--------------|
| Linux | `io_uring` (preferred), `epoll` |
| macOS | `kqueue` |
| Windows | `IOCP` |
| WASM | Browser event loop |

```blood
// Platform-abstracted I/O
effect IO {
    op read(fd: Fd, buf: &mut [u8]) -> Result<usize, IoError>;
    op write(fd: Fd, buf: &[u8]) -> Result<usize, IoError>;
    op accept(socket: Socket) -> Result<Socket, IoError>;
    op connect(addr: SocketAddr) -> Result<Socket, IoError>;
}
```

### 10.3 Stack Management

```rust
struct Stack {
    // Guard page at bottom (for overflow detection)
    guard: *mut u8,

    // Usable stack area
    base: *mut u8,
    size: usize,

    // Current stack pointer
    sp: *mut u8,
}

impl Stack {
    fn new(size: usize) -> Stack {
        // Allocate with guard page
        let total = size + PAGE_SIZE;
        let ptr = mmap(total, PROT_READ | PROT_WRITE);

        // Mark guard page as inaccessible
        mprotect(ptr, PAGE_SIZE, PROT_NONE);

        Stack {
            guard: ptr,
            base: ptr.add(PAGE_SIZE),
            size,
            sp: ptr.add(total),
        }
    }

    fn grow(&mut self) {
        // Double the stack size
        let new_size = self.size * 2;
        if new_size > MAX_STACK_SIZE {
            panic!("Stack overflow");
        }
        // Reallocate and copy
        // ...
    }
}
```

---

## 11. Runtime Linking Requirements

### 11.1 Overview

Blood programs using concurrency features must link against the Blood runtime library. This section specifies the linking requirements for different platforms and build configurations.

### 11.2 Required Runtime Libraries

| Library | Description | Location |
|---------|-------------|----------|
| `libblood_runtime.a` | Static runtime library | `blood-runtime/target/release/` |
| `libblood_runtime.so` | Dynamic runtime library (Linux) | `blood-runtime/target/release/` |
| `libblood_runtime.dylib` | Dynamic runtime library (macOS) | `blood-runtime/target/release/` |
| `blood_runtime.dll` | Dynamic runtime library (Windows) | `blood-runtime/target/release/` |

### 11.3 Required Symbols

The following FFI symbols must be available at link time for concurrency features:

| Symbol | Purpose | Header |
|--------|---------|--------|
| `blood_scheduler_init` | Initialize the scheduler | `ffi_exports.rs` |
| `blood_scheduler_shutdown` | Clean shutdown | `ffi_exports.rs` |
| `blood_fiber_spawn` | Spawn a new fiber | `ffi_exports.rs` |
| `blood_fiber_yield` | Yield current fiber | `ffi_exports.rs` |
| `blood_fiber_await` | Wait for fiber completion | `ffi_exports.rs` |
| `blood_channel_create` | Create a channel | `ffi_exports.rs` |
| `blood_channel_send` | Send to channel | `ffi_exports.rs` |
| `blood_channel_recv` | Receive from channel | `ffi_exports.rs` |
| `blood_io_reactor_init` | Initialize I/O reactor | `ffi_exports.rs` |

### 11.4 Platform-Specific Linking

#### Linux

```bash
# Static linking (recommended for deployment)
clang program.o -L/path/to/blood-runtime/target/release \
    -lblood_runtime -lpthread -ldl -lm -o program

# Dynamic linking
clang program.o -L/path/to/blood-runtime/target/release \
    -Wl,-rpath,/path/to/blood-runtime/target/release \
    -lblood_runtime -lpthread -ldl -lm -o program

# With io_uring support (Linux 5.1+)
clang program.o -L/path/to/blood-runtime/target/release \
    -lblood_runtime -lpthread -ldl -lm -luring -o program
```

**Required system libraries**:
- `pthread` — Thread primitives
- `dl` — Dynamic loading (for FFI)
- `m` — Math library
- `uring` — io_uring support (optional, for async I/O)

#### macOS

```bash
# Static linking
clang program.o -L/path/to/blood-runtime/target/release \
    -lblood_runtime -lpthread -ldl -lm -framework CoreFoundation -o program

# Dynamic linking
clang program.o -L/path/to/blood-runtime/target/release \
    -Wl,-rpath,@executable_path/../lib \
    -lblood_runtime -lpthread -ldl -lm -framework CoreFoundation -o program
```

**Required frameworks**:
- `CoreFoundation` — System services
- `pthread` — Thread primitives

#### Windows

```cmd
REM Static linking
link program.obj /LIBPATH:C:\path\to\blood-runtime\target\release ^
    blood_runtime.lib ws2_32.lib userenv.lib bcrypt.lib ntdll.lib /OUT:program.exe

REM Dynamic linking
link program.obj /LIBPATH:C:\path\to\blood-runtime\target\release ^
    blood_runtime.dll.lib ws2_32.lib userenv.lib /OUT:program.exe
```

**Required system libraries**:
- `ws2_32.lib` — Windows Sockets (networking)
- `userenv.lib` — User environment
- `bcrypt.lib` — Cryptographic primitives
- `ntdll.lib` — NT system calls (for IOCP)

### 11.5 Build System Integration

#### Cargo (Rust projects using Blood)

```toml
[dependencies]
blood-runtime = { path = "../blood-runtime" }

[build-dependencies]
cc = "1.0"
```

#### CMake

```cmake
find_library(BLOOD_RUNTIME blood_runtime
    PATHS ${BLOOD_SDK}/lib
    REQUIRED)

target_link_libraries(my_program PRIVATE ${BLOOD_RUNTIME})

if(UNIX AND NOT APPLE)
    target_link_libraries(my_program PRIVATE pthread dl m)
elseif(APPLE)
    target_link_libraries(my_program PRIVATE pthread dl m
        "-framework CoreFoundation")
elseif(WIN32)
    target_link_libraries(my_program PRIVATE ws2_32 userenv bcrypt ntdll)
endif()
```

#### Blood Build Tool

The `blood build` command handles runtime linking automatically:

```bash
# Default: static linking
blood build program.blood -o program

# Explicit dynamic linking
blood build program.blood -o program --link-mode=dynamic

# Cross-compilation
blood build program.blood -o program --target=x86_64-unknown-linux-gnu
```

### 11.6 Runtime Initialization

Programs must initialize the runtime before using concurrency features:

```c
// C FFI initialization (generated by compiler)
int main(int argc, char** argv) {
    // Initialize runtime (scheduler, I/O reactor, etc.)
    blood_runtime_init(argc, argv);

    // Run the Blood main function
    int result = blood_main();

    // Clean shutdown
    blood_runtime_shutdown();

    return result;
}
```

The Blood compiler automatically generates this wrapper when compiling executables.

### 11.7 Minimal Runtime (Embedded)

For resource-constrained environments, a minimal runtime is available:

```bash
blood build program.blood -o program --runtime=minimal
```

Minimal runtime excludes:
- I/O reactor (no async I/O)
- Multi-worker scheduling (single-threaded)
- Debug symbols and tracing

**Minimal runtime size**: ~50 KB (stripped)

### 11.8 Verification

To verify runtime linking is correct:

```bash
# Linux: Check symbols
nm -u program | grep blood_

# macOS: Check symbols
nm -u program | grep blood_

# Windows: Check imports
dumpbin /imports program.exe | findstr blood_
```

All `blood_*` symbols should be resolved (not undefined).

---

## Appendix A: Scheduler Tuning

| Parameter | Default | Description |
|-----------|---------|-------------|
| `WORKERS` | CPU count | Worker threads |
| `INITIAL_STACK` | 8 KB | Initial fiber stack |
| `MAX_STACK` | 1 MB | Maximum fiber stack |
| `GLOBAL_QUEUE_SIZE` | 1024 | Global queue capacity |
| `LOCAL_QUEUE_SIZE` | 256 | Per-worker queue capacity |
| `STEAL_BATCH` | 32 | Fibers stolen at once |
| `PREEMPT_INTERVAL` | 10 ms | Preemption check interval |

---

## Appendix B: Debugging

```blood
// Fiber debugging
fn debug_fibers() / {Fiber, IO} {
    let stats = scheduler_stats();
    println("Active fibers: {}", stats.active);
    println("Suspended fibers: {}", stats.suspended);
    println("Total spawned: {}", stats.total_spawned);

    for fiber in all_fibers() {
        println("Fiber {}: {:?}", fiber.id, fiber.state);
        if let Some(name) = fiber.name {
            println("  Name: {}", name);
        }
        println("  Stack usage: {} bytes", fiber.stack_usage());
    }
}
```

---

## Appendix C: References

Concurrency model draws from:

- [Fiber (computer science) - Wikipedia](https://en.wikipedia.org/wiki/Fiber_(computer_science))
- [Naughty Dog's Fiber-Based Job System](https://www.gdcvault.com/play/1022186/Parallelizing-the-Naughty-Dog-Engine)
- [Tokio Scheduler Design](https://tokio.rs/blog/2019-10-scheduler)
- [Go Scheduler Design](https://morsmachine.dk/go-scheduler)
- [Project Loom (Java Virtual Threads)](https://openjdk.org/projects/loom/)

---

*This document is part of the Blood Language Specification.*
