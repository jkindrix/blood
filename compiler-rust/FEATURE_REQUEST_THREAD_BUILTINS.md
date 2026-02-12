# Feature Request: `thread_spawn` / `thread_join` Builtins for OS-Level Parallelism

## Summary

The self-hosted Blood compiler needs OS-level thread parallelism for codegen, but Blood's fat function pointers (`{ fn_ptr, env_ptr }`) are incompatible with C's thin function pointers required by `pthread_create`. We need `thread_spawn` and `thread_join` builtins that handle the fat-to-thin conversion internally in the runtime.

## Severity

**High** — Blocks parallel codegen in the self-hosted compiler. All structural prerequisites are complete (batched pipeline, worker contexts, string table merge). Only the thread dispatch mechanism is missing.

## Motivation

The self-hosted compiler processes ~1600 functions sequentially during codegen. Each function's IR generation is independent once MIR is lowered. A batched pipeline (commit `f7d1730` in `blood`) already groups functions into batches of 32 with decoupled stages:

```
Stage A: MIR lower 32 functions (sequential — shared HIR state)
Stage B: Codegen 32 functions (CURRENTLY sequential — should be parallel)
Stage C: Destroy 32 MIR regions
```

Worker context infrastructure is ready (`create_worker_ctx()`, `merge_string_tables()`, `remap_string_labels()` in `codegen_ctx.blood`). The only missing piece is dispatching Stage B across OS threads.

## Problem: Fat Function Pointers

Blood represents all function pointers as fat pointers to support closures:

```llvm
; Blood function pointer type (from closures.rs:241, types.rs:235)
{ i8* fn_ptr, i8* env_ptr }
```

When a Blood function is passed to a C function expecting a thin pointer, blood-rust generates a `$fnptr` wrapper (`context/mod.rs:3176`) that prepends an `env_ptr` parameter:

```llvm
; Original Blood function
define ptr @worker(ptr %arg) { ... }

; Generated wrapper (worker$fnptr)
define ptr @"worker$fnptr"(ptr %env, ptr %arg) { ... }

; What pthread_create receives
call i32 @pthread_create(ptr %tid, ptr null,
  { ptr @"worker$fnptr", ptr null },  ; <-- fat pointer, not thin
  ptr %arg)
```

`pthread_create` expects `i8* (*)(i8*)` (one parameter), but receives `{ i8*, i8* }` (a struct). The thread never starts correctly.

### Reproduction

```blood
bridge "C" pthreads {
    fn pthread_create(thread: *mut i64, attr: *mut u8, start: fn(*mut u8) -> *mut u8, arg: *mut u8) -> i32;
    fn pthread_join(thread: i64, retval: *mut *mut u8) -> i32;
}

fn worker(arg: *mut u8) -> *mut u8 {
    print_str("worker\n");
    0 as *mut u8
}

fn main() -> i32 {
    let mut tid: i64 = 0;
    pthreads::pthread_create(&mut tid as *mut i64, 0 as *mut u8, worker, 0 as *mut u8);
    pthreads::pthread_join(tid, 0 as *mut *mut u8);
    0
}
```

**Result:** Basic thread spawn works (single statement `print_str`), but any data exchange via the `arg` pointer fails because the function pointer ABI is wrong — `worker$fnptr(env, arg)` is called where `worker(arg)` was expected.

### Why the Existing Scheduler Doesn't Help

The blood-runtime's fiber scheduler (`scheduler.rs:150`, `ffi_exports.rs:4792`) provides cooperative concurrency, not OS-level parallelism. All fibers run under a single mutex (`sched.lock()` in `ffi_exports.rs:4840`), making it unsuitable for CPU-bound workloads like codegen.

`blood_scheduler_spawn` already accepts `extern "C" fn(*mut c_void)` — a thin pointer — but it dispatches to fibers, not OS threads. The calling convention is correct; the dispatch target is wrong for our use case.

## Proposed Solution

Add two new builtins that wrap `std::thread` (or raw pthreads) and handle the fat-to-thin conversion internally.

### API (Blood Side)

```blood
// Spawn an OS thread running `func(arg)`. Returns an opaque thread handle.
let handle: u64 = thread_spawn(func, arg);

// Wait for the thread to complete. Returns 0 on success.
let status: u64 = thread_join(handle);
```

Both parameters to `thread_spawn` are `u64` — the function pointer and argument are passed as opaque integers, avoiding the fat pointer problem entirely.

### Runtime Implementation

In `blood-runtime/src/ffi_exports.rs`:

```rust
/// Spawns an OS thread that calls `func_ptr(arg)`.
/// `func_ptr` is a raw function pointer (the fn_ptr field of a Blood fat pointer).
/// `arg` is an opaque u64 passed to the function.
/// Returns an opaque thread handle (Box<JoinHandle> leaked as u64), or 0 on failure.
#[no_mangle]
pub unsafe extern "C" fn blood_thread_spawn(func_ptr: u64, arg: u64) -> u64 {
    // func_ptr is the thin function pointer (field 0 of the Blood fat pointer).
    // The Blood compiler extracts it before calling this builtin.
    let f: extern "C" fn(u64) -> u64 = std::mem::transmute(func_ptr);

    match std::thread::Builder::new().spawn(move || {
        f(arg);
    }) {
        Ok(handle) => {
            let boxed = Box::new(handle);
            Box::into_raw(boxed) as u64
        }
        Err(_) => 0,
    }
}

/// Joins a thread previously spawned by blood_thread_spawn.
/// `handle` is the opaque handle returned by blood_thread_spawn.
/// Returns 0 on success, non-zero on failure.
#[no_mangle]
pub unsafe extern "C" fn blood_thread_join(handle: u64) -> u64 {
    if handle == 0 {
        return 1; // Invalid handle
    }
    let boxed: Box<std::thread::JoinHandle<()>> =
        Box::from_raw(handle as *mut std::thread::JoinHandle<()>);
    match boxed.join() {
        Ok(()) => 0,
        Err(_) => 1,
    }
}
```

### Builtin Registration

In `bloodc/src/typeck/context/builtins.rs`, following the existing pattern (e.g., `region_create` at line 165):

```rust
// OS thread primitives
self.register_builtin_fn_aliased(
    "thread_spawn", "blood_thread_spawn",
    vec![u64_ty.clone(), u64_ty.clone()], u64_ty.clone(),
);
self.register_builtin_fn_aliased(
    "thread_join", "blood_thread_join",
    vec![u64_ty.clone()], u64_ty.clone(),
);
```

### Usage Pattern from Blood

The Blood side extracts the thin pointer from the fat pointer manually:

```blood
fn codegen_worker(arg: u64) -> u64 {
    // arg is a pointer to task data, cast as needed
    let task = arg as *mut CodegenTask;
    // ... do codegen work ...
    0
}

fn dispatch_batch(tasks: &Vec<CodegenTask>) {
    let mut handles: Vec<u64> = Vec::new();

    let mut i: usize = 0;
    while i < tasks.len() {
        let task_ptr = &tasks[i] as *const CodegenTask as u64;
        let handle = thread_spawn(codegen_worker as u64, task_ptr);
        handles.push(handle);
        i = i + 1;
    }

    // Join all threads
    let mut j: usize = 0;
    while j < handles.len() {
        thread_join(handles[j]);
        j = j + 1;
    }
}
```

**Key insight:** By using `u64` for both parameters, the Blood compiler never needs to convert a fat pointer to a thin pointer in IR. The programmer casts the function to `u64` (which extracts the raw address) and the runtime `transmute`s it back to a callable.

### Codegen Consideration

The compiler must ensure that `codegen_worker as u64` extracts field 0 (the thin `fn_ptr`) from the fat pointer, not the entire struct. This may require a special case in `bloodc/src/codegen/context/mod.rs` where casting a function value to `u64` extracts the function pointer field rather than treating the struct as an integer.

**Alternative:** If `as u64` on a function already extracts field 0, no codegen change is needed. This needs verification.

## Thread Safety Considerations

The self-hosted compiler's parallel codegen has been designed for thread safety:

| Resource | During Parallel Codegen | Safety |
|----------|------------------------|--------|
| HIR/type check results | Read-only | Safe (immutable after Pass 1) |
| `def_names`, `adt_registry` | Read-only (cloned into worker) | Safe |
| MIR regions | Deactivated, data readable | Safe (no allocations, just reads) |
| Worker `CodegenCtx` | Thread-private | Safe (each thread owns its ctx) |
| String table | Worker-private, merged after join | Safe (no concurrent writes) |
| `write_buffer` / file I/O | Main thread only, after join | Safe |
| `print_str` | NOT called by workers | Safe |
| Global allocator | Rust's default (thread-safe) | Safe |

**No runtime functions are called by worker threads during codegen.** Workers only read MIR data and write to their private `CodegenCtx.output` string. All runtime interaction (file I/O, string table merge) happens on the main thread after `thread_join`.

## Alternative Approaches Considered

### 1. Fix Fat Pointer Conversion for Bridge Functions

**Approach:** When a Blood function pointer is passed to a `bridge "C"` function expecting a thin pointer, automatically extract field 0.

**Pros:** Fixes the general case; `pthread_create` would work directly.
**Cons:** Complex — must detect thin-vs-fat mismatch in type lowering. Risk of breaking other FFI patterns. The `env_ptr` would be silently dropped, which is wrong for closures with captures.

### 2. Add `@thin` Attribute to Function Declarations

**Approach:** `fn worker(arg: *mut u8) -> *mut u8 @thin { ... }` generates a thin pointer.

**Pros:** Explicit, no magic.
**Cons:** Requires parser/AST/codegen changes. Viral — any function passed to C needs the attribute.

### 3. Use `blood_scheduler_spawn` with Real Threads

**Approach:** Modify the scheduler to dispatch to OS threads instead of fibers.

**Pros:** Reuses existing infrastructure.
**Cons:** Changes scheduler semantics for all users. The cooperative scheduler has different guarantees than preemptive threads.

### 4. Builtin with `u64` Parameters (Proposed)

**Approach:** `thread_spawn(fn_ptr: u64, arg: u64) -> u64` — opaque handle, raw integers.

**Pros:** Minimal changes (2 FFI functions + 2 builtin registrations). No codegen changes if `as u64` on functions extracts the thin pointer. Matches the existing pattern for `region_create`/`region_destroy`.
**Cons:** Requires manual casting in Blood code. No type safety on the function signature.

**Recommendation:** Option 4 is the simplest path with the least risk. It follows the established pattern of `region_create` / `region_destroy` (opaque `u64` handles) and requires no codegen changes beyond builtin registration.

## Files to Modify

| File | Change | Effort |
|------|--------|--------|
| `blood-runtime/src/ffi_exports.rs` | Add `blood_thread_spawn`, `blood_thread_join` | ~30 lines |
| `bloodc/src/typeck/context/builtins.rs` | Register `thread_spawn`, `thread_join` builtins | ~10 lines |

**Optional (if `fn as u64` doesn't extract thin pointer):**

| File | Change | Effort |
|------|--------|--------|
| `bloodc/src/codegen/context/mod.rs` | Handle fat-pointer-to-u64 cast | ~15 lines |

## Verification Plan

Once implemented, verify with this test program:

```blood
fn worker(arg: u64) -> u64 {
    let data = arg as *mut i32;
    // Write 42 to shared memory
    @unsafe { *data = 42; }
    0
}

fn main() -> i32 {
    let mut result: i32 = 0;
    let handle = thread_spawn(worker as u64, &mut result as *mut i32 as u64);
    thread_join(handle);
    if result == 42 {
        print_str("thread_spawn works\n");
        0
    } else {
        print_str("thread_spawn FAILED\n");
        1
    }
}
```

Expected output: `thread_spawn works` with exit code 0.

## Impact

Unblocking this feature enables:
1. **Parallel codegen** in the self-hosted compiler (all infrastructure ready in commit `f7d1730`)
2. **General-purpose parallelism** for any Blood program needing CPU-bound work
3. **Foundation for higher-level primitives** (thread pools, parallel iterators, etc.)

Estimated speedup for self-hosted compiler codegen: 2-4x on a 4-core machine (1600 functions, batch size 32, ~50 batches with 4 threads each).
