# Blood Concurrency Specification

**Version**: 0.2.0-draft
**Status**: Active Development
**Last Updated**: 2026-01-09

This document specifies Blood's concurrency model, including fiber semantics, scheduling, synchronization primitives, and parallel execution.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Fiber Model](#2-fiber-model)
3. [Scheduler](#3-scheduler)
4. [Fiber Lifecycle](#4-fiber-lifecycle)
5. [Communication](#5-communication)
6. [Synchronization](#6-synchronization)
7. [Parallel Primitives](#7-parallel-primitives)
8. [Effect Integration](#8-effect-integration)
9. [Memory Model](#9-memory-model)
10. [Platform Mapping](#10-platform-mapping)

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

### 1.3 Concurrency Philosophy

| Aspect | Blood Approach |
|--------|----------------|
| **Unit of Concurrency** | Fibers (stackful coroutines) |
| **Scheduling** | M:N cooperative with preemption points |
| **Communication** | Channels (typed, bounded) |
| **Shared State** | By default: none. Opt-in via `Synchronized<T>` |
| **Memory** | Fiber-local regions, shared via Tier 3 |

### 1.3 Comparison with Other Models

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
    /// Spawn a new fiber
    op spawn<T>(f: fn() -> T / {Fiber | ε}) -> FiberHandle<T>;

    /// Spawn with configuration
    op spawn_with<T>(
        config: FiberConfig,
        f: fn() -> T / {Fiber | ε}
    ) -> FiberHandle<T>;

    /// Get current fiber's handle
    op current() -> FiberHandle<()>;

    /// Yield to scheduler
    op yield();

    /// Sleep for duration
    op sleep(duration: Duration);
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
    let fiber_result = await(handle);

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

    await(handle)
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
| `await(handle)` | Waiting for another fiber |
| `channel.send()` | Channel full |
| `channel.recv()` | Channel empty |
| `sleep(duration)` | Timer |
| `perform(effect)` | Effect operation |
| Function call | Optional preemption check |

### 3.5 Preemption

For long-running computations, Blood inserts preemption checks:

```blood
// Compiler inserts preemption checks at:
// - Function entry (if loop body)
// - Loop back edges
// - After N instructions (configurable)

#[preemption_check]
fn long_loop() / {Fiber} {
    for i in 0..1_000_000 {
        // Preemption check inserted here by compiler
        compute(i);
    }
}
```

Implementation:

```
PREEMPTION_CHECK():
    IF current_fiber.should_yield:
        current_fiber.should_yield ← false
        yield()
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

### 4.3 Await Operation

```
AWAIT(handle):
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
            RETURN AWAIT(handle)  // Retry
```

### 4.4 Structured Concurrency

All child fibers must complete before their parent:

```blood
fn structured_example() / {Fiber} {
    let h1 = spawn(|| task1());
    let h2 = spawn(|| task2());

    // Implicit: parent waits for h1, h2 before returning
    let r1 = await(h1);
    let r2 = await(h2);

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

### 4.5 Cancellation

```blood
effect Cancel {
    /// Check if cancellation requested
    op is_cancelled() -> bool;

    /// Throw if cancelled
    op check_cancelled();
}

fn cancellable_task() / {Fiber, Cancel} {
    for item in items {
        check_cancelled();  // Cancellation point
        process(item);
    }
}

fn cancel_example() / {Fiber} {
    let handle = spawn(|| cancellable_task());

    // Cancel after timeout
    sleep(Duration::seconds(5));
    cancel(handle);

    // Wait for cancellation to complete
    match await_cancellable(handle) {
        Ok(result) => result,
        Err(Cancelled) => default_value(),
    }
}
```

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

    // Cancellation is cooperative - fiber must check
```

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
    fn par_map<U, F>(self, f: F) -> ParMap<T, U, F>
    where F: Fn(T) -> U + Send;

    fn par_filter<F>(self, f: F) -> ParFilter<T, F>
    where F: Fn(&T) -> bool + Send;

    fn par_reduce<F>(self, identity: T, f: F) -> T
    where F: Fn(T, T) -> T + Send;

    fn par_for_each<F>(self, f: F)
    where F: Fn(T) + Send;
}

impl<T: Send> ParallelIterator<T> for Vec<T> {
    fn par_map<U, F>(self, f: F) -> Vec<U> / {Fiber}
    where F: Fn(T) -> U + Send {
        let num_chunks = num_workers();
        let chunk_size = (self.len() + num_chunks - 1) / num_chunks;

        let results: Vec<FiberHandle<Vec<U>>> = self
            .chunks(chunk_size)
            .map(|chunk| spawn(move || chunk.iter().map(&f).collect()))
            .collect();

        results.into_iter()
            .flat_map(|h| await(h))
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
    fn spawn<F>(&mut self, f: F)
    where F: FnOnce() + Send {
        self.fibers.push(spawn(f));
    }

    fn wait_all(&self) / {Fiber} {
        for handle in &self.fibers {
            await(handle.clone());
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

## 8. Effect Integration

### 8.1 Concurrency as Effects

All concurrency operations are effects:

```blood
effect Fiber {
    op spawn<T>(f: fn() -> T / {Fiber | ε}) -> FiberHandle<T>;
    op current() -> FiberHandle<()>;
    op yield();
    op sleep(duration: Duration);
}

effect Async {
    op await<T>(handle: FiberHandle<T>) -> T;
    op select<T>(handles: Vec<FiberHandle<T>>) -> (usize, T);
    op timeout<T>(duration: Duration, f: fn() -> T / {Async | ε}) -> Result<T, Timeout>;
}

effect Channel<T> {
    op channel(capacity: usize) -> (Sender<T>, Receiver<T>);
    op send(value: T);
    op recv() -> T;
}
```

### 8.2 Effect Handler Integration

```blood
// Handler for running concurrent code
deep handler FiberRuntime for Fiber {
    let scheduler: Scheduler

    return(x) { x }

    op spawn(f) {
        let handle = scheduler.spawn(f);
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
}

// Run concurrent computation
fn run<T>(f: fn() -> T / {Fiber}) -> T {
    let scheduler = Scheduler::new();
    with FiberRuntime { scheduler } handle {
        f()
    }
}
```

### 8.3 Fiber + Region Interaction

From MEMORY_MODEL.md Section 7.8:

```blood
// Regions are fiber-local
fn region_fiber_example() / {Fiber} {
    region local_data {
        let buffer = allocate_buffer();  // In local_data region

        // WRONG: Cannot share region reference
        // spawn(|| use_buffer(&buffer));  // COMPILE ERROR

        // CORRECT: Promote to Tier 3
        let shared = persist(buffer.clone());
        spawn(|| use_buffer(&shared));
    }
}
```

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
effect AsyncIO {
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
