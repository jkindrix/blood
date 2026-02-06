# Second-Gen Binary Segfault: Root Cause Analysis

## Summary

The second-generation self-hosted binary (`second_gen`) segfaults immediately at startup during `intern_keywords()`. The root cause is that **all Vec element sizes, enum payload sizes, and other ADT-dependent size computations** use `codegen_types::type_size_bytes()` which returns 8 (pointer size) for all ADTs, instead of using the context-aware `codegen_stmt::type_size_with_ctx()` which correctly queries the ADT registry.

**Important update (2026-02-06):** The original analysis identified alloca undersizing as the primary issue. After the ADT registry and `type_to_llvm_with_ctx()` were implemented, struct allocas are now **correctly sized**. The remaining issue is that all `vec_new`/`vec_push`/`vec_pop`/`option_unwrap` calls still pass `i64 8` regardless of actual element type, and enum layouts don't use context-aware sizing.

## Crash Location

```
#0  __memcpy_avx_unaligned_erms    (memcpy with bogus destination pointer)
#1  vec_push                       (rdi = 0x47ffffffec228 — garbage pointer)
#2  def436_intern_keywords         (StringInterner::intern_keywords)
#3  def435_new                     (StringInterner::new)
#4  def530_init_global_interner
#5  blood_main
```

## Current State (Post-ADT Registry)

### What's CORRECT

- **Struct allocas**: ADT registry + `type_to_llvm_with_ctx()` produces proper struct type allocas (e.g., `alloca { ptr, { ptr, i64, i64 } }` for StringInterner)
- **Struct field types**: Nested ADT fields resolve to their actual LLVM types
- **Struct literal stores**: Stores fit in the correctly-sized allocas

### What's BROKEN

- **Vec element sizes**: ALL `vec_new`/`vec_push`/`vec_pop` calls pass `i64 8` regardless of actual element type
- **Option unwrap/as_ref sizes**: Use `codegen_types::type_size_bytes()` → always 8 for ADTs
- **Enum payload sizes**: `build_enum_layout()` uses `codegen_types::type_size_bytes()` and `codegen_types::type_to_llvm()` — returns 8/ptr for ADTs
- **Cast sizes**: `codegen_types::get_layout()` for casts — returns 8 for ADTs
- **Drop sizes**: `codegen_types::type_size_bytes()` for drop — returns 8 for ADTs

### Evidence

Reference IR (blood-rust) uses varied Vec element sizes: 24, 56, 72, 88, 272, 416.
Self-hosted IR uses `i64 8` for every single vec_new/vec_push call (638 vec_new, 657 vec_push — all with i64 8).

## Root Function

`codegen_types::get_layout()` at line 257 returns `TypeLayout::pointer()` (size=8) for ALL ADTs. The `type_size_bytes()` function calls `get_layout().size`, propagating the wrong size to all callers.

The context-aware alternative `codegen_stmt::type_size_with_ctx()` at line 289 correctly uses the ADT registry via `adt_size_bytes()` → `lookup_struct()`/`lookup_enum()` → `compute_struct_size()`.

## Fix Applied

Replaced all uses of `codegen_types::type_size_bytes()` (and `codegen_types::get_layout()`) with `codegen_stmt::type_size_with_ctx()` in the following files:

| File | Call Sites | Changes |
|------|------------|---------|
| `codegen_term.blood` | 7 | vec_new, vec_push (3), vec_pop, option_unwrap, option_as_ref |
| `codegen_expr.blood` | 8 | get_element_size_from_hir (5 internal + 4 callers), cast sizes (2) |
| `codegen.blood` | 4 + new function | build_enum_layout_with_ctx, enum rebuild pass in populate_adt_registry |
| `codegen_stmt.blood` | 1 | Drop size computation |

Also added `build_enum_layout_with_ctx()` function and enum rebuild pass in `populate_adt_registry()` to ensure enum payload sizes use context-aware ADT sizing.

## Comparison with Blood-Rust (Reference Compiler)

Blood-rust computes correct element sizes because its type system has full struct layout information at codegen time.

| Aspect | Blood-Rust (correct) | Self-Hosted (before fix) | Self-Hosted (after fix) |
|--------|---------------------|-------------------------|------------------------|
| Vec\<String\> elem_size | 24 | 8 | 24 (expected) |
| Vec\<Token\> elem_size | varies | 8 | correct (expected) |
| Enum payload sizes | computed from fields | 8 for ADT fields | computed from registry |
| Struct allocas | correct | correct (already fixed) | correct |

## Verification

After fixing, verify with:
```bash
./build_selfhost.sh full          # Rebuild everything
./build_selfhost.sh verify        # Check declarations and IR
./second_gen version              # Should not segfault
./build_selfhost.sh test          # Smoke tests should pass
```

Check specifically:
1. `vec_new`/`vec_push` calls in second_gen.ll have VARIED element sizes (not all i64 8)
2. Enum types in second_gen.ll have correct payload sizes
3. All 3 FileCheck tests still pass
4. Declaration diff: 0 unexpected mismatches
