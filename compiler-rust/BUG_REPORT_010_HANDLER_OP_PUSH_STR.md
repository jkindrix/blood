# BUG-010: Handler Op Body Passes `&str` Argument by Value Instead of by Pointer to `string_push_str`

**Status:** FIXED

## Summary

When `push_str` is called inside a deep handler op body with the op's `&str` parameter as the argument, blood-rust generates incorrect LLVM IR. The `&str` parameter (type `{ i8*, i64 }`) is loaded as a struct value and passed by value to `string_push_str`, but the C ABI requires it to be passed by pointer. This causes an LLVM verification failure.

**Root cause:** The type checker rewrites builtin method calls (`buffer.push_str(text)`) to regular `Call` expressions (not `MethodCall`). These route through `compile_call`'s builtin path, which only had int-to-int and pointer-to-pointer coercion — missing struct-to-pointer coercion.

**Fix:** Added struct-to-pointer, int-to-pointer, and array-to-pointer coercion to `compile_call` (both the builtin function path and the direct function call path). Test: `t05_handler_push_str.blood`.

## Severity

**Critical** — Blocks the BufferedFileEmitter optimization for the self-hosted compiler's codegen pipeline (replacing N `file_append_string` syscalls with a single write).

## Environment

- **blood-rust version**: Current HEAD (includes BUG-009 fixes `10261fc` + `b25403e`)
- **Platform**: Linux x86_64 (Debian)
- **LLVM version**: 14.0.6

## Reproduction

### Minimal Test Case

```blood
// File: handler_push_str_repro.blood
// EXPECT: hello world

effect Emit {
    op emit(text: &str) -> ();
}

deep handler BufferedEmitter for Emit {
    let mut buffer: String

    return(x) {
        print_str(buffer.as_str());
        x
    }

    op emit(text) {
        buffer.push_str(text);
        resume(())
    }
}

fn do_work() -> bool / {Emit} {
    perform Emit.emit("hello ");
    perform Emit.emit("world\n");
    true
}

fn main() -> i32 {
    let _result: bool = with BufferedEmitter { buffer: String::new() } handle {
        do_work()
    };
    0
}
```

### Steps to Reproduce

```bash
blood build handler_push_str_repro.blood
```

### Expected Result

Compiles and links successfully. When run, prints `hello world` and exits with code 0.

### Actual Result

```
Error: LLVM verification failed for def DefId(336): Call parameter type does not match function signature!
  %load = load { i8*, i64 }, { i8*, i64 }* %text, align 8
 { i8*, i64 }*  call void @string_push_str(i8* %arg_ptr_cast, { i8*, i64 } %load)

Build failed: 1 codegen errors.
```

## Root Cause Analysis

### The ABI Contract

`string_push_str` is declared as:

```llvm
declare void @string_push_str(ptr %self, ptr %str)
```

Both arguments must be **pointers to stack allocas**:
- `%self`: pointer to the `String` struct (`{ ptr, i64, i64 }`)
- `%str`: pointer to the `&str` fat pointer (`{ i8*, i64 }`)

### What Goes Wrong

Inside the handler op body, the op parameter `text` (type `&str`) arrives through the handler's continuation protocol as an `i64`, gets cast to a pointer, and is loaded from a typed alloca. The compilation path is roughly:

1. **Op parameter arrives as `i64`** (handler protocol converts all params to uniform `i64` representation)
2. **Stored into a typed alloca** via `compile_handler_op_body` (`handlers.rs:605-711`):
   ```llvm
   %text = alloca { i8*, i64 }
   ; ... cast from i64, store into %text ...
   ```
3. **Loaded via `compile_local_load`** (`expr.rs:346-355`) when used in an expression:
   ```llvm
   %load = load { i8*, i64 }, { i8*, i64 }* %text, align 8
   ```
4. **Passed to `string_push_str`** — but the loaded **struct value** `{ i8*, i64 }` is passed directly instead of being re-wrapped as a pointer to the alloca.

### Expected LLVM IR

```llvm
; The &str is already in an alloca — just pass the alloca pointer
call void @string_push_str(ptr %buffer_ptr, ptr %text)
```

### Actual LLVM IR (incorrect)

```llvm
%load = load { i8*, i64 }, { i8*, i64 }* %text, align 8
call void @string_push_str(ptr %arg_ptr_cast, { i8*, i64 } %load)
;                                              ^^^^^^^^^^^^^^^^
;                        struct value passed by value, should be ptr
```

### Why the BUG-009 Fix Doesn't Cover This

Commit `b25403e` added struct-value-to-pointer coercion in `compile_method_call` (`expr.rs:1017-1039`), but this coercion is gated behind a `param_type.is_pointer_type()` check at line 1004:

```rust
if param_type.is_pointer_type() {
    // Only enters this branch if LLVM parameter type is already ptr
    match arg {
        BasicMetadataValueEnum::StructValue(struct_val) => {
            // Coercion logic: alloca + store + pass pointer
        }
        // ...
    }
}
```

When `string_push_str` expects `ptr` for the `&str` argument, the condition `param_type.is_pointer_type()` should be `true` and the coercion should trigger. The fact that it doesn't suggests either:

1. The op parameter load path bypasses `compile_method_call` entirely and goes through a different call emission path (e.g., `compile_call` at `expr.rs:620-660` which lacks this coercion), or
2. The method resolution for `push_str` on `buffer` (a mutable handler state field) routes through a code path that doesn't apply the b25403e fix, or
3. The `param_type` retrieved for the second argument doesn't register as a pointer type due to how handler op body compilation sets up the function signature.

### Affected Code Paths

| Component | File | Function | Lines | Description |
|-----------|------|----------|-------|-------------|
| Op param loading | `handlers.rs` | `compile_handler_op_body` | 605-711 | Converts i64 op args to typed allocas |
| Local loading | `expr.rs` | `compile_local_load` | 346-355 | Loads struct value from alloca |
| Method call coercion | `expr.rs` | `compile_method_call` | 999-1097 | Partial fix from b25403e (conditional) |
| Function call coercion | `expr.rs` | `compile_call` | 620-660 | No struct-to-pointer coercion |

## Relationship to BUG-009

BUG-009 (`10261fc` + `b25403e`) fixed:
- Typed handler state layout (String, Vec, aggregates > 8 bytes) ✅
- `String::new()` assignment in handler op bodies (out-pointer builtins) ✅
- Pointer type coercion for method calls (partial) ⚠️

BUG-010 is an **uncovered edge case** of the BUG-009 fix: method calls on mutable handler state where the argument is the **op's own parameter** of an aggregate type (`&str` = `{ i8*, i64 }`). The existing test cases for BUG-009 don't exercise this pattern:

| Test Case | Pattern | Covers BUG-010? |
|-----------|---------|-----------------|
| `t03_effect_handler_string_mut.blood` | `stored = String::new()` in op body | No — assignment, not method call with op param |
| `t03_effect_handler_string_state.blood` | `print_str(chunk)` in op body | No — free function call, not method call on state |

Neither test calls a method on mutable state with the op's aggregate-type parameter.

## Impact

### Blocked Work

The self-hosted Blood compiler's codegen pipeline currently uses a `FileEmitter` handler that makes N `file_append_string` syscalls (one per codegen batch). The fix is a `BufferedFileEmitter` that accumulates all IR in a mutable `String` buffer and flushes once:

```blood
deep handler BufferedFileEmitter for EmitIR {
    let output_path: String
    let mut buffer: String

    return(x) {
        file_append_string(output_path.as_str(), buffer.as_str());
        x
    }

    op emit_section(ir) {
        buffer.push_str(ir);   // <-- Triggers BUG-010
        resume(())
    }
}
```

This is the natural, correct implementation for buffered effect-handled I/O. Without this fix, the compiler must either:
- Use N syscalls (current `FileEmitter` approach), or
- Delegate to a helper function as a workaround (violates Rule 4: no workarounds for compiler bugs)

### Broader Impact

Any handler that calls a method on mutable state with an op parameter of aggregate type will hit this bug. Common patterns affected:

```blood
// Collecting items
op add(item: SomeStruct) {
    items.push(item);       // BUG-010: SomeStruct passed by value
    resume(())
}

// String building
op append(text: &str) {
    result.push_str(text);  // BUG-010: &str passed by value
    resume(())
}

// Map insertion
op insert(key: &str, value: SomeType) {
    map.insert(key, value); // Likely same issue
    resume(())
}
```

## Suggested Fix

The `&str` parameter is **already in an alloca** inside the handler op body (set up by `compile_handler_op_body`). The fix should avoid loading the struct value and instead pass the alloca pointer directly when the callee expects a pointer argument.

Two possible approaches:

### Approach A: Fix at the Method Call Site

In `compile_method_call` (or wherever `push_str` is compiled for handler ops), when the argument is a struct value and the parameter expects a pointer:
1. Allocate a temporary alloca
2. Store the struct value into it
3. Pass the alloca pointer

This is what `b25403e` does at lines 1017-1039, but the condition guarding it needs to be broadened or the code path needs to ensure this branch is actually reached for handler op parameter arguments.

### Approach B: Fix at the Local Load Site

In the handler op body compilation, when a local of aggregate type is referenced as an argument to a function/method call, pass the **alloca pointer** instead of loading the value. This avoids the redundant load-then-re-store cycle entirely.

### Suggested Test Case

Add to `tests/ground-truth/`:

```blood
// Test: push_str on mutable handler state with &str op parameter (BUG-010)
// Verifies that aggregate-type op parameters are correctly passed by pointer
// when used as arguments to method calls on mutable handler state.
// EXPECT: hello world
effect Emit {
    op emit(text: &str) -> ();
}

deep handler BufferedEmitter for Emit {
    let mut buffer: String

    return(x) {
        print_str(buffer.as_str());
        x
    }

    op emit(text) {
        buffer.push_str(text);
        resume(())
    }
}

fn do_work() -> i32 / {Emit} {
    perform Emit.emit("hello ");
    perform Emit.emit("world\n");
    42
}

fn main() -> i32 {
    let result: i32 = with BufferedEmitter { buffer: String::new() } handle {
        do_work()
    };
    if result != 42 { return 1; }
    0
}
```

## Related Files

- `bloodc/src/codegen/context/handlers.rs` — Handler op body compilation
- `bloodc/src/codegen/context/expr.rs` — Expression compilation (method calls, local loads)
- `bloodc/src/codegen/mir_codegen/terminator.rs` — MIR call terminator codegen
- `tests/ground-truth/t03_effect_handler_string_mut.blood` — Existing BUG-009 test (doesn't cover this)
- `tests/ground-truth/t03_effect_handler_string_state.blood` — Existing BUG-009 test (doesn't cover this)
