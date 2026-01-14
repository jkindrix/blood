# Blood Effects LLVM Codegen Specification

**Version**: 0.1.0
**Status**: Specified
**Implementation**: `bloodc/src/codegen/context/effects.rs`
**Last Updated**: 2026-01-14

This document specifies how Blood's algebraic effect system is compiled to LLVM IR, based on the evidence-passing model.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Evidence-Passing Model](#2-evidence-passing-model)
3. [Runtime Interface](#3-runtime-interface)
4. [Perform Compilation](#4-perform-compilation)
5. [Resume Compilation](#5-resume-compilation)
6. [Handle Compilation](#6-handle-compilation)
7. [Continuation Management](#7-continuation-management)
8. [Optimizations](#8-optimizations)
9. [Implementation Details](#9-implementation-details)

---

## 1. Overview

### 1.1 Compilation Model

Blood compiles effects using **evidence-passing** semantics:

| Source Construct | LLVM Representation |
|------------------|---------------------|
| `perform E.op(args)` | Call through evidence vector |
| `resume(value)` | Return or continuation call |
| `with handler handle body` | Evidence vector setup/teardown |

### 1.2 Related Specifications

- [FORMAL_SEMANTICS.md](./FORMAL_SEMANTICS.md) - Effect operational semantics
- [MIR_LOWERING.md](./MIR_LOWERING.md) - MIR effect operations
- [SPECIFICATION.md](./SPECIFICATION.md) - Effect system design

### 1.3 Academic Foundation

Implementation based on:
- [Generalized Evidence Passing for Effect Handlers](https://dl.acm.org/doi/10.1145/3473576) (ICFP'21)
- [Zero-Overhead Lexical Effect Handlers](https://doi.org/10.1145/3763177) (OOPSLA'25)

---

## 2. Evidence-Passing Model

### 2.1 Concept

Rather than searching for handlers at runtime, Blood passes evidence (handler pointers) explicitly through the call stack:

```
Traditional (dynamic lookup):
    perform E.op(x) → search stack for handler → invoke

Evidence-passing:
    perform E.op(x) → evidence_vector.lookup(E) → invoke handler
```

### 2.2 Evidence Vector

```c
// Runtime representation
typedef struct {
    uint64_t effect_id;        // Effect type identifier
    void* handler_state;       // Handler state pointer
    handler_vtable_t* vtable;  // Handler operation table
} evidence_entry_t;

typedef struct {
    evidence_entry_t* entries;
    size_t count;
    size_t capacity;
} evidence_vector_t;
```

### 2.3 Thread-Local Evidence

The current evidence vector is stored in thread-local storage:

```c
__thread evidence_vector_t* current_evidence;
```

---

## 3. Runtime Interface

### 3.1 Evidence Management

| Function | Signature | Purpose |
|----------|-----------|---------|
| `blood_evidence_create` | `() -> *void` | Create new evidence vector |
| `blood_evidence_destroy` | `(*void) -> void` | Destroy evidence vector |
| `blood_evidence_current` | `() -> *void` | Get current thread's evidence |
| `blood_evidence_set_current` | `(*void) -> void` | Set thread's evidence |
| `blood_evidence_push` | `(*void, u64) -> void` | Push handler onto evidence |
| `blood_evidence_push_with_state` | `(*void, u64, *void) -> void` | Push handler with state |
| `blood_evidence_pop` | `(*void) -> void` | Pop handler from evidence |

### 3.2 Effect Invocation

| Function | Signature | Purpose |
|----------|-----------|---------|
| `blood_perform` | `(u64, u32, *i64, u64, u64) -> i64` | Invoke effect operation |

Parameters for `blood_perform`:
1. `effect_id: u64` - Effect type identifier
2. `op_index: u32` - Operation index within effect
3. `args: *i64` - Pointer to argument array
4. `arg_count: u64` - Number of arguments
5. `continuation: u64` - Continuation handle (0 for tail-resumptive)

### 3.3 Continuation Operations

| Function | Signature | Purpose |
|----------|-----------|---------|
| `blood_continuation_create` | `(fn_ptr, *void) -> u64` | Create one-shot continuation |
| `blood_continuation_create_multishot` | `(fn_ptr, *void) -> u64` | Create multi-shot continuation |
| `blood_continuation_resume` | `(u64, i64) -> i64` | Resume continuation with value |
| `blood_continuation_clone` | `(u64) -> u64` | Clone continuation (multi-shot) |
| `blood_continuation_destroy` | `(u64) -> void` | Destroy continuation |

---

## 4. Perform Compilation

### 4.1 Algorithm

```
COMPILE perform E.op(arg₁, ..., argₙ):

    1. Compile arguments to i64 values
    2. Allocate stack array for arguments
    3. Store arguments in array
    4. Create continuation (if non-tail-resumptive)
    5. Call blood_perform(effect_id, op_index, args_ptr, n, continuation)
    6. Convert result from i64 to expected type
```

### 4.2 LLVM IR Pattern

```llvm
; perform State.get()
;
; Arguments:
;   effect_id = 42
;   op_index = 0 (get)
;   args = empty
;   continuation = identity continuation

define i64 @example_perform() {
entry:
    ; No arguments - pass null
    %args = null

    ; Create identity continuation
    %cont = call i64 @blood_continuation_create(
        ptr @__blood_identity_continuation,
        ptr null
    )

    ; Invoke perform
    %result = call i64 @blood_perform(
        i64 42,        ; effect_id (State)
        i32 0,         ; op_index (get)
        ptr %args,     ; arguments
        i64 0,         ; arg_count
        i64 %cont      ; continuation
    )

    ret i64 %result
}
```

### 4.3 Argument Marshalling

All arguments are converted to i64 for uniform passing:

```llvm
; Integer (smaller than 64-bit): sign-extend
%arg_i32 = sext i32 %val to i64

; Float: bitcast
%arg_float = bitcast double %val to i64

; Pointer: ptrtoint
%arg_ptr = ptrtoint ptr %val to i64
```

### 4.4 Result Conversion

Results are converted back from i64:

```llvm
; Integer (smaller than 64-bit): truncate
%result_i32 = trunc i64 %result to i32

; Float: bitcast
%result_float = bitcast i64 %result to double

; Pointer: inttoptr
%result_ptr = inttoptr i64 %result to ptr
```

---

## 5. Resume Compilation

### 5.1 Resume Modes

Blood supports two resume compilation modes:

| Mode | Description | When Used |
|------|-------------|-----------|
| **Tail-resumptive** | Direct return | Simple handlers, tail position |
| **Continuation-based** | Call continuation | Non-tail position, multi-shot |

### 5.2 Algorithm

```
COMPILE resume(value):

    1. Compile value expression
    2. Convert to i64

    IF continuation_context IS NOT NULL:
        3a. Load continuation handle
        3b. Check if tail-resumptive (handle == 0)
        3c. IF tail-resumptive:
              - Build return instruction
            ELSE:
              - IF multi-shot: clone continuation
              - Call blood_continuation_resume(handle, value)
              - Convert result to expected type
    ELSE:
        3d. Build return instruction (shallow handler)
```

### 5.3 LLVM IR Pattern (Continuation-based)

```llvm
define i64 @handler_op_impl(i64 %arg) {
entry:
    ; Load continuation from context
    %cont_ptr = load ptr, ptr @handler_continuation_context
    %cont_handle = load i64, ptr %cont_ptr

    ; Check if tail-resumptive
    %is_tail = icmp eq i64 %cont_handle, 0
    br i1 %is_tail, label %tail_path, label %cont_path

tail_path:
    ; Tail-resumptive: just return
    %result = ; compute result
    ret i64 %result

cont_path:
    ; Continuation-based: call resume
    %resume_val = ; compute resume value
    %result = call i64 @blood_continuation_resume(
        i64 %cont_handle,
        i64 %resume_val
    )
    br label %merge

merge:
    ; Continue with result
    %phi = phi i64 [%result, %cont_path]
    ret i64 %phi
}
```

### 5.4 Multi-Shot Handlers

For multi-shot handlers, continuations must be cloned before use:

```llvm
cont_path:
    ; Clone continuation for multi-shot handler
    %cloned_cont = call i64 @blood_continuation_clone(i64 %cont_handle)

    ; Resume with cloned continuation
    %result = call i64 @blood_continuation_resume(
        i64 %cloned_cont,
        i64 %resume_val
    )
```

---

## 6. Handle Compilation

### 6.1 Algorithm

```
COMPILE with handler handle body:

    1. Save current evidence vector
    2. Create new evidence vector
    3. Compile handler instance to get state pointer
    4. Push handler onto new evidence vector
    5. Set new evidence as current
    6. Compile body expression
    7. Pop handler from evidence vector
    8. Restore previous evidence vector
    9. Destroy new evidence vector
    10. Return body result
```

### 6.2 LLVM IR Pattern

```llvm
define i64 @example_with_handler() {
entry:
    ; 1. Save current evidence
    %saved_ev = call ptr @blood_evidence_current()

    ; 2. Create new evidence vector
    %new_ev = call ptr @blood_evidence_create()

    ; 3. Compile handler state
    %state = alloca %HandlerState
    ; ... initialize state ...

    ; 4. Push handler with state
    call void @blood_evidence_push_with_state(
        ptr %new_ev,
        i64 42,           ; effect_id
        ptr %state        ; state pointer
    )

    ; 5. Set as current
    call void @blood_evidence_set_current(ptr %new_ev)

    ; 6. Compile body
    %body_result = call i64 @body_function()

    ; 7. Pop handler
    call void @blood_evidence_pop(ptr %new_ev)

    ; 8. Restore previous evidence
    call void @blood_evidence_set_current(ptr %saved_ev)

    ; 9. Destroy new evidence
    call void @blood_evidence_destroy(ptr %new_ev)

    ; 10. Return result
    ret i64 %body_result
}
```

### 6.3 Handler State Layout

Handler state is compiled as a struct:

```llvm
; handler MyHandler for State<i32> {
;     state: i32 = 0
; }

%MyHandler.State = type { i32 }

; Initialization
%state = alloca %MyHandler.State
store i32 0, ptr %state  ; Initialize state
```

---

## 7. Continuation Management

### 7.1 Continuation Representation

```c
typedef struct {
    continuation_fn callback;  // Function to call on resume
    void* context;             // Captured context
    bool consumed;             // One-shot tracking
} continuation_t;
```

### 7.2 Identity Continuation

For simple performs (result used directly), an identity continuation suffices:

```llvm
define internal i64 @__blood_identity_continuation(i64 %value, ptr %ctx) {
entry:
    ret i64 %value
}
```

### 7.3 Capturing Continuations

For non-tail-resumptive handlers, the "rest of computation" is captured:

```
; Conceptual: perform State.get() + 1
;
; Without optimization:
;   continuation = λv. v + 1
;   blood_perform(State, get, [], continuation)
;
; With tail-resumptive optimization:
;   result = blood_perform(State, get, [], 0)
;   return result + 1
```

### 7.4 Multi-Shot Semantics

Multi-shot continuations allow multiple resumes:

```llvm
; Clone before each resume to enable multi-shot
%cont1 = call i64 @blood_continuation_clone(i64 %original)
%result1 = call i64 @blood_continuation_resume(i64 %cont1, i64 %val1)

%cont2 = call i64 @blood_continuation_clone(i64 %original)
%result2 = call i64 @blood_continuation_resume(i64 %cont2, i64 %val2)
```

---

## 8. Optimizations

### 8.1 Tail-Resumptive Optimization

**Condition**: Resume is in tail position and called exactly once.

```blood
// Tail-resumptive: resume is the final expression
handler StateHandler {
    get() { resume(state) }
    put(x) { state = x; resume(()) }
}
```

**Optimization**: Skip continuation creation; use direct return:

```llvm
; Without optimization (continuation-based)
%cont = call i64 @blood_continuation_create(...)
%result = call i64 @blood_perform(..., i64 %cont)

; With tail-resumptive optimization
%result = call i64 @blood_perform(..., i64 0)  ; 0 = tail-resumptive
```

### 8.2 Static Evidence (EFF-OPT-001)

**Condition**: Handler state is stateless, constant, or zero-initialized.

```blood
// Stateless handler
handler LogHandler {
    log(msg) { println(msg); resume(()) }
}
```

**Optimization**: Use pre-allocated static evidence structure:

```llvm
; Static evidence (allocated at compile time)
@__static_LogHandler_evidence = internal constant %Evidence {
    i64 17,       ; effect_id
    ptr null,     ; no state needed
    ptr @LogHandler_vtable
}
```

### 8.3 Stack Allocation (EFF-OPT-005/006)

**Condition**: Handler doesn't escape its lexical scope.

**Optimization**: Allocate evidence on stack instead of heap:

```llvm
; Stack-allocated evidence (no heap allocation)
%evidence = alloca %Evidence
store i64 42, ptr %evidence
store ptr %state, ptr getelementptr(%Evidence, ptr %evidence, i32 0, i32 1)
```

### 8.4 Inline Evidence (EFF-OPT-003/004)

**Condition**: Single handler or pair of handlers.

```blood
// Single handler case
with StateHandler handle { body }
```

**Optimization**: Pass evidence directly without vector:

```llvm
; Without optimization (vector-based)
%ev = call ptr @blood_evidence_create()
call void @blood_evidence_push(ptr %ev, i64 42)

; With inline optimization (direct passing)
; Evidence passed in registers, no heap allocation
```

### 8.5 Optimization Decision Tree

```
Is handler stateless?
├─ YES → Static Evidence (EFF-OPT-001)
└─ NO
   ├─ Does handler escape?
   │  ├─ YES → Heap allocation (default)
   │  └─ NO → Stack allocation (EFF-OPT-005/006)
   │
   └─ Handler count?
      ├─ 1 handler → Inline single (EFF-OPT-003)
      ├─ 2 handlers → Inline pair (EFF-OPT-004)
      └─ 3+ handlers → Vector (default)
```

---

## 9. Implementation Details

### 9.1 File Locations

| File | Purpose |
|------|---------|
| `codegen/context/effects.rs` | Main effects codegen |
| `mir/static_evidence.rs` | Static evidence analysis |
| `mir/escape.rs` | Escape analysis for handlers |

### 9.2 Key Functions

| Function | Location | Purpose |
|----------|----------|---------|
| `compile_perform` | `effects.rs:334` | Compile perform expression |
| `compile_resume` | `effects.rs:500` | Compile resume expression |
| `compile_handle` | `effects.rs:694` | Compile handle expression |
| `create_perform_continuation` | `effects.rs:232` | Create continuation for perform |
| `is_handler_tail_resumptive` | `effects.rs:31` | Check if handler is tail-resumptive |

### 9.3 Type Conversions

| Blood Type | LLVM Type | Conversion to i64 | Conversion from i64 |
|------------|-----------|-------------------|---------------------|
| `i8`, `i16`, `i32` | `i8`, `i16`, `i32` | `sext` | `trunc` |
| `i64` | `i64` | identity | identity |
| `f32`, `f64` | `float`, `double` | `bitcast` | `bitcast` |
| `*T` | `ptr` | `ptrtoint` | `inttoptr` |
| `()` | `{}` | `0` | (ignored) |

### 9.4 Error Handling

Effects codegen produces diagnostics for:
- Missing runtime functions
- Unsupported argument types
- Invalid continuation contexts
- Resume outside handler

---

## References

1. **Generalized Evidence Passing for Effect Handlers**
   - Xie, N., et al. (ICFP 2021)
   - [ACM DL](https://dl.acm.org/doi/10.1145/3473576)

2. **Zero-Overhead Lexical Effect Handlers**
   - (OOPSLA 2025)
   - [DOI](https://doi.org/10.1145/3763177)

3. **Koka Language**
   - [Effect Handlers in Koka](https://koka-lang.github.io/koka/doc/book.html#sec-handlers)

4. **LLVM Language Reference**
   - [LLVM IR](https://llvm.org/docs/LangRef.html)

---

*Last updated: 2026-01-14*
