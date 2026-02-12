# BUG-009: Effect Handler State Limited to ≤ 8-Byte Types

**Status:** FIXED
**Last Updated:** 2026-02-06
**Fix Commits:** `10261fc` (typed handler state layout) + `b25403e` (out-pointer builtins + pointer casting) + BUG-010 fix (struct-to-pointer coercion in compile_call)

---

## EXECUTIVE SUMMARY

Handler state fields larger than 8 bytes (e.g., `String` which is `{ ptr, i64, i64 }` = 24 bytes) generated invalid LLVM IR because blood-rust serialized all state fields uniformly as `i64`. This has been fixed with typed handler state layout, out-pointer builtins, and comprehensive type coercion in all call paths.

| Pattern | Status | Test Coverage |
|---------|--------|---------------|
| Immutable String state | Fixed | `t03_effect_handler_string_state.blood` |
| Mutable String state (assignment) | Fixed | `t03_effect_handler_string_mut.blood` |
| Free function calls with op params | Fixed | `t03_effect_handler_string_state.blood` (`print_str(chunk)`) |
| Delegation to regular functions | Fixed | Verified in self-hosted compiler |
| `&mut` references in handler ops | Fixed | Verified in self-hosted compiler |
| Method calls on state with op `&str` param | Fixed (BUG-010) | `t05_handler_push_str.blood` |

---

## Original Bug

### Symptom

Any handler with state fields larger than 8 bytes caused LLVM verification errors or generated invalid IR:

```blood
deep handler MyHandler for SomeEffect {
    let mut buffer: String  // 24 bytes — was broken

    // ...
}
```

### Root Cause

`handlers.rs` used a uniform `[i64 x N]` layout for all handler state, regardless of actual field types. This meant:
- `String` (24 bytes = `{ ptr, i64, i64 }`) was stored as `[i64 x 3]`
- Loads and stores used `i64` types instead of the actual struct types
- Type mismatches caused LLVM verification failures

### Fix 1: Typed Handler State Layout (`10261fc`)

`handlers.rs` now computes the correct LLVM type for each state field instead of using uniform `[i64 x N]`. Direct loads/stores use the actual field types when they match.

### Fix 2: Out-Pointer Builtins + Pointer Casting (`b25403e`)

`expr.rs` now supports:
- `string_new` / `str_to_string` via out-pointer pattern (functions that return aggregates by writing to an output pointer)
- Pointer type coercion for method calls (`compile_method_call`)
- `Ref` / `RefMut` / `Deref` handling in `compile_unary`

### Fix 3: Struct-to-Pointer Coercion in compile_call (BUG-010)

The type checker rewrites builtin method calls (like `buffer.push_str(text)`) to regular function `Call` expressions (not `MethodCall`). These route through `compile_call`, which had only int-to-int and pointer-to-pointer coercion. Added struct-to-pointer, int-to-pointer, and array-to-pointer coercion to both the builtin call path and the direct function call path.

**Root cause of BUG-010:** `compile_call`'s builtin function path loaded the `&str` op parameter as a struct value `{ i8*, i64 }` and passed it by value, but `string_push_str` expects both arguments as pointers.

---

## All Verified Working Patterns

### 1. Immutable String State
```blood
deep handler FileEmitter for Emit {
    let path: String        // 24-byte state field works

    return(x) { x }

    op emit(chunk) {
        print_str(chunk);   // Free function call with op param
        resume(())
    }
}
```

### 2. Mutable String State (Assignment)
```blood
deep handler StringSwapper for Swap {
    let mut stored: String  // Mutable 24-byte state works

    return(x) { x }

    op swap() {
        stored = String::new();  // Assignment to mutable state
        resume(())
    }
}
```

### 3. Delegation to Helper Functions
```blood
fn emit_ir_to_file(path: &str, ir: &str) {
    file_append_string(path, ir);
}

deep handler FileEmitter for EmitIR {
    let output_path: String

    return(x) { x }

    op emit_section(ir) {
        emit_ir_to_file(output_path.as_str(), ir);  // Delegation works
        resume(())
    }
}
```

### 4. Method Calls on State with Op Parameters (BUG-010 fixed)
```blood
deep handler BufferedEmitter for Emit {
    let mut buffer: String

    return(x) {
        print_str(buffer.as_str());
        x
    }

    op emit(text) {
        buffer.push_str(text);  // Now works correctly
        resume(())
    }
}
```

---

## Impact on Self-Hosted Compiler

The self-hosted Blood compiler's `BufferedFileEmitter` pattern (accumulate IR in a `String` buffer, flush once) is now unblocked. This replaces N `file_append_string` syscalls with a single write.
