# Continuation Strategy

**Status:** Design document (not started)
**Author:** Generated from deep audit findings (2026-04-10)
**Last updated:** 2026-04-10

## Current Implementation

Blood's algebraic effect system uses three control flow mechanisms:

### 1. Tail-resumptive handlers (zero overhead)

The common case. Handler op bodies that end with `resume(value)` are compiled as direct calls. The `perform` site calls the handler function, which returns the resume value. No stack manipulation. This is the fast path for State, Reader, Writer patterns.

### 2. Non-resumptive handlers (setjmp/longjmp abort)

Handlers that don't resume (Cancel, Error, StaleReference). The handler scope installs a `setjmp` point. `perform` calls `longjmp` to unwind back to the handler. The handler's return clause provides the block's result value.

### 3. "Continuations" (identity functions)

`rt_continuation.blood:4-6` says: "The selfhost always passes `@__blood_identity_continuation` as the callback. The real control flow happens via the longjmp/abort path."

The continuation table exists. `blood_continuation_create_multishot` allocates entries. `blood_perform` passes them. But callbacks are always identity functions. Entries are marked consumed on first use and panic on second resume ("single-shot violation").

### What works

- Tail-resumptive handlers: State, Reader, Writer, logging, instrumentation
- Non-resumptive handlers: Cancel, Error, StaleReference, abort semantics
- Handler state mutation via capture pointers
- Parametric effects with type substitution
- Snapshot validation (gen-tracked locals validated after resume)
- Nested handlers with correct dispatch (evidence vector)
- Handler forwarding (`/ {Effect}` syntax)

### What doesn't work

- **Multi-shot continuations**: Panics on second resume. Cannot fork computations.
- **Deferred resume**: `resume(value)` is immediate return, not continuation capture. Cannot store a continuation for later.
- **Suspend/resume scheduling**: No ability to suspend a computation and resume it elsewhere.
- **Non-local resume**: Cannot resume from a different handler scope.
- **Stack capture**: No mechanism to save and restore the call stack between yield and resume.

## Recommended Upgrade: libmprompt

[libmprompt](https://github.com/koka-lang/libmprompt) by Daan Leijen (Microsoft Research) provides multi-prompt delimited control in C/C++. It is the continuation backend used by Koka.

### API

```c
// Types
typedef struct mp_prompt_s  mp_prompt_t;    // prompt marker (identifies handler scope)
typedef struct mp_resume_s  mp_resume_t;    // abstract resumption

// Core operations
void* mp_prompt(mp_start_fun_t* fun, void* arg);        // run fun under a fresh prompt
void* mp_yield(mp_prompt_t* p, mp_yield_fun_t* fun, void* arg);  // yield to prompt p
void* mp_resume(mp_resume_t* resume, void* arg);         // resume (at most once)
void* mp_resume_tail(mp_resume_t* resume, void* arg);    // tail resume (last action)
void  mp_resume_drop(mp_resume_t* resume);               // drop without resuming

// Multi-shot (opt-in)
mp_resume_t* mp_resume_multi(mp_resume_t* r);   // convert to multi-shot
mp_resume_t* mp_resume_dup(mp_resume_t* r);     // duplicate multi-shot resumption
```

### Gstacks (growable stacks via virtual memory)

Each prompt gets a virtual-memory-backed gstack:
- **Initial commit**: 4 KiB (one page)
- **Maximum size**: 8 MiB
- **Growth**: On-demand via page faults (or overcommit on Linux)
- **Address stability**: Gstacks never move. Stack pointers remain valid.
- **Gap**: 64 KiB no-access guard between stacks

Key property: always one logical active stack (chain of gstacks). Exceptions propagate naturally. Backtraces cross gstack boundaries correctly.

### Performance

From the libmprompt benchmark (AMD 5950X, Ubuntu 20):
- 10M prompt create + 4 context switches + 32KB stack use each: 0.932s
- ~10M "connections" per second, single-threaded
- RSS: 42 MB with 10,000 concurrent prompts

Compare: Blood's current setjmp/longjmp is essentially free for tail-resumptive (no context switch) and very cheap for non-resumptive (one longjmp). The upgrade adds cost only for non-tail-resumptive handlers that actually capture continuations.

### Platform support

Linux (x64, arm64), macOS (x64, arm64), Windows (x64), FreeBSD (x64). 64-bit only (requires large virtual address space for gstacks).

### License

MIT. Compatible with Blood's licensing.

## Integration Plan

### What stays the same

1. **Evidence vector for handler lookup** — Blood's O(1) dispatch via evidence passing is orthogonal to the continuation mechanism. Keep it.
2. **Tail-resumptive fast path** — Direct call, no prompt/yield overhead. This handles the majority of effect operations.
3. **MIR representation** — `PushHandler`, `PopHandler`, `Perform`, `Resume` MIR statements remain the same.
4. **Effect type system** — Type checking, effect rows, subtyping unchanged.

### What changes

1. **PushHandler codegen**: Currently emits `setjmp`. Replace with `mp_prompt` — each handler scope gets a fresh prompt. The handler's body executes under this prompt.

2. **Perform codegen**: Currently calls handler function directly (tail-resumptive) or `longjmp` (non-resumptive). For non-tail-resumptive, replace with `mp_yield(prompt, handler_fun, args)`. The yield captures the continuation from the perform site to the prompt.

3. **Resume codegen**: Currently returns a value up the call stack. For non-tail-resumptive, replace with `mp_resume(resume, value)`. This restores the captured continuation and returns the value at the yield point.

4. **Runtime**: Replace `rt_continuation.blood` with a thin wrapper around libmprompt. The continuation table (`blood_continuation_create_multishot`, etc.) is replaced by `mp_resume_t*` opaque pointers.

5. **Linking**: Add `libmpromptx.a` to the link step (C++ version for exception propagation).

### Migration steps

1. **Add libmprompt as a build dependency.** Build from source (cmake) or use prebuilt. Add to `build_selfhost.sh` link flags.

2. **Implement wrapper functions** in Blood runtime:
   - `blood_prompt_enter(handler_fn, arg) -> result` wraps `mp_prompt`
   - `blood_yield(prompt_marker, handler_fn, arg) -> result` wraps `mp_yield`
   - `blood_resume_one(resume, value) -> result` wraps `mp_resume`
   - `blood_resume_drop(resume)` wraps `mp_resume_drop`

3. **Update codegen for PushHandler** to emit `blood_prompt_enter` instead of `setjmp`.

4. **Update codegen for Perform** to use `blood_yield` for non-tail-resumptive operations.

5. **Update codegen for Resume** to use `blood_resume_one` for non-tail-resumptive operations.

6. **Keep tail-resumptive fast path unchanged.** The tail-resumptive optimization (direct call, no prompt) should remain — it's the common case and has zero overhead.

7. **Gate**: Full golden test suite + self-compilation + bootstrap.

### Multi-shot support

libmprompt provides `mp_resume_multi` to convert a single-shot resumption to multi-shot. This copies the gstack chain. Multi-shot resumptions can be duplicated with `mp_resume_dup`.

Blood's type system would need to distinguish linear (single-shot) and unrestricted (multi-shot) continuations. The current `linear fn` mechanism could be extended: handler operations that declare their continuation as `linear` use single-shot, others use multi-shot.

Multi-shot is Phase 2 of this upgrade — get single-shot working first.

## Alternatives Considered

### Fibers (OCaml 5 style)

OCaml 5 uses C stack fibers for effect handlers. Requires compiler support for stack switching and GC integration. Blood doesn't have a GC, making this simpler in some ways but harder in others (no write barrier, but also no fiber-safe allocation). libmprompt's gstack approach is equivalent but more portable.

### CPS transform (Effekt style)

Effekt compiles effect handlers via CPS (continuation-passing style) transformation. This avoids stack manipulation entirely but requires whole-program transformation and changes calling conventions. Incompatible with Blood's C FFI story and with the existing codegen pipeline.

### LLVM coroutines

LLVM has coroutine intrinsics (`llvm.coro.id`, `llvm.coro.suspend`, etc.) for async/await. These are designed for C++ coroutines, not general delimited control. They don't support multi-prompt, don't have well-defined interaction with `setjmp`/`longjmp`, and are tightly coupled to LLVM optimization passes. Poor fit.

### libseff (OOPSLA 2024)

Lightweight effect handlers using system-level continuations. Similar approach to libmprompt but newer. Less battle-tested. Could be considered as a future alternative.

## References

- Leijen, D. (2021). "Structured Asynchrony with Algebraic Effects." ICFP 2021.
- Xie, N. and Leijen, D. (2021). "Generalized Evidence Passing for Effect Handlers." MSR-TR-2021-5.
- libmprompt GitHub: https://github.com/koka-lang/libmprompt
- OCaml 5 effect handlers: Sivaramakrishnan et al. PLDI 2021.
- Effekt: Brachthaeuser et al. OOPSLA 2023.
- libseff: OOPSLA 2024.
