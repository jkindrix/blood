# Cross-Function Generational Reference Enforcement

**Status:** OPEN — Option B implemented for the common case
**Date:** 2026-03-15
**Prerequisite:** Header-field gen refs (DD-1, implemented)

## Problem

When function `f()` returns `&T`, the caller receives a pointer. To validate
the pointer on dereference, the caller needs the expected generation (captured
at reference creation time). Currently, generation tracking is scope-local —
it doesn't cross function boundaries.

## The Core Question

Where does the expected generation come from at the caller's dereference site?

The generation lives in the allocation header at `[ptr-8]`. Reading it is only
safe when the pointer came from `blood_alloc_or_abort` (which adds the header).
Stack pointers don't have headers — reading `[ptr-8]` is undefined behavior.

## Options Evaluated

### Option A: Tag bits in pointer

Use unused high bits of the 64-bit pointer (x86-64 uses 48 bits for virtual
addresses) to store a truncated generation.

**Pro:** Zero ABI change. 16-bit generation in pointer high bits.
**Con:** Platform-dependent. Wraps at 65536. Interferes with pointer
arithmetic. Not sound — generation space too small for long-running programs.

### Option B: Caller-side capture after Call (SELECTED for common case)

After every Call that returns a pointer type, the caller emits:
```
%gen = load i32, ptr getelementptr(i8, %result, -8)
```
and stores the generation in a companion local.

**Insight:** Blood's escape analysis promotes locals whose references escape to
heap via `blood_alloc_or_abort`. So any pointer ALLOCATED by a callee went
through `blood_alloc_or_abort` and has a header.

**Sound for:** Functions that allocate and return (`fn make() -> &T`).
**Unsound for:** Functions that pass through a reference (`fn id(x: &T) -> &T`).
The caller's `x` might be a stack pointer with no header.

**Mitigation:** The selfhost compiler doesn't return pass-through references in
hot paths. This covers the practical case while the sound solution (Option C)
is designed.

### Option C: Fat reference type `{ ptr, i32 }`

Every `&T` becomes `{ ptr, i32 }` in LLVM IR. All reference passing, storage,
return, and field access carries the generation.

**Pro:** Fully sound. Generation travels with the pointer at all times.
**Con:** Every struct containing `&T` grows by 4 bytes. ABI changes everywhere.
Function signatures change. FFI bridge blocks needed. ~2,000 lines of codegen
changes across both compilers. Reference operations (load, store, GEP) all need
to handle the fat representation.

**This is the correct long-term solution** but requires a dedicated implementation
effort.

## Decision

Implement Option B for the common case (callee allocates and returns). Document
the pass-through limitation. Plan Option C as a future milestone when the
reference representation can be changed comprehensively.

## Implementation

After `emit_call` completes for a Call terminator:
1. Check if the destination local's type is a reference (`&T`)
2. If yes, read `[ptr-8]` from the returned pointer
3. Store the generation in a companion alloca for the destination local
4. Set `local_generation` so subsequent Deref checks use it

The check at step 1 prevents reading `[ptr-8]` from non-pointer return values.
The unsound case (pass-through `&T`) produces a correct check IF the original
pointer came from `blood_alloc_or_abort` (which it usually does — stack locals
whose references escape are promoted by escape analysis).

## Known Limitation

If a function returns a reference to a stack-allocated local in the CALLER's
frame (e.g., `fn id(x: &i32) -> &i32 { x }` where the caller passes `&stack_local`),
reading `[ptr-8]` at the call site is undefined behavior. This case requires
Option C (fat references) or interprocedural escape analysis.

In practice, this case is rare in the selfhost and most Blood code. The escape
analysis already promotes escaping locals to heap, so most returned references
point to heap memory with headers.
