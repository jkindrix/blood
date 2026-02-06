# Second-Gen Binary Segfault: Root Cause Analysis

## Summary

The second-generation self-hosted binary (`second_gen`) segfaults immediately at startup during `intern_keywords()`. The root cause is a **stack buffer overflow** caused by the self-hosted compiler's codegen emitting undersized `alloca` instructions for ADT (struct/enum) locals. The compiler allocates 8-byte pointer-sized slots but then stores full inline struct values (up to 48+ bytes) into them, corrupting the stack.

## Crash Location

```
#0  __memcpy_avx_unaligned_erms    (memcpy with bogus destination pointer)
#1  vec_push                       (rdi = 0x47ffffffec228 — garbage pointer)
#2  def436_intern_keywords         (StringInterner::intern_keywords)
#3  def435_new                     (StringInterner::new)
#4  def530_init_global_interner
#5  blood_main
```

The `memcpy` destination (`rdi`) is `0x47ffffffec228` — an invalid address. This is the Vec's data pointer, which was corrupted by a prior stack overflow.

## Root Cause: Alloca Undersizing for ADTs

### The ADT-as-Pointer Model

The self-hosted compiler treats **all ADTs as pointer-sized** in its type layout system:

```blood
// codegen_types.blood:257
&hir_ty::TypeKind::Adt { def_id: _, args: _ } => TypeLayout::pointer(),
```

`TypeLayout::pointer()` = `{ size: 8, align: 8, llvm_type: "ptr" }`.

This means every local variable of ADT type gets `alloca ptr` (8 bytes), regardless of the actual struct size.

### The Inconsistency

While alloca uses the pointer model (8 bytes), **struct literal construction** stores the full inline struct:

```llvm
; In StringInterner::new() — self-hosted output (second_gen.ll)
%_5 = alloca ptr                                            ; 8 bytes allocated
; ... later ...
store { ptr, { ptr, i64, i64 } } %tmp11, ptr %_5           ; 32 bytes written!
```

The `{ ptr, { ptr, i64, i64 } }` type is the inline representation of `StringInterner { strings: Vec<String>, hash_index: HashMapU64U32 }`:
- Field 0: `ptr` (Vec<String> — a heap pointer, 8 bytes)
- Field 1: `{ ptr, i64, i64 }` (HashMapU64U32 — inline 24 bytes)
- **Total: 32 bytes** written into an **8-byte** allocation

### Stack Corruption Chain

In `StringInterner::new()`:

```llvm
; Step 1: Allocate locals (all 8 bytes due to ADT-as-pointer)
%_5 = alloca ptr          ; 8 bytes — will hold StringInterner
%_6 = alloca ptr          ; 8 bytes — adjacent on stack

; Step 2: Build the struct with its two fields
%tmp7  = load ptr, ptr %_2           ; Vec pointer (field 0)
%tmp8  = getelementptr ..., ptr %_1, i64 0, i32 0
store ptr %tmp7, ptr %tmp8           ; store field 0
%tmp9  = load { ptr, i64, i64 }, ptr %_3    ; HashMap (field 1)
%tmp10 = getelementptr ..., ptr %_1, i64 0, i32 1
store { ptr, i64, i64 } %tmp9, ptr %tmp10  ; store field 1

; Step 3: Load the full struct and store to %_5
%tmp11 = load { ptr, { ptr, i64, i64 } }, ptr %_1    ; load 32 bytes
store { ptr, { ptr, i64, i64 } } %tmp11, ptr %_5     ; OVERFLOW: 32 → 8 bytes!

; Step 4: Copy to %_6 (also overflows)
%tmp12 = load { ptr, { ptr, i64, i64 } }, ptr %_5    ; reads corrupted data
store { ptr, { ptr, i64, i64 } } %tmp12, ptr %_6     ; another 32 → 8 byte overflow

; Step 5: Pass to intern_keywords via pointer to %_6
store ptr %_6, ptr %_7
%tmp13 = load ptr, ptr %_7
call void @def436_intern_keywords(ptr %tmp13)
```

The 32-byte store at Step 3 writes 24 bytes beyond `%_5`'s allocation, corrupting `%_6`, `%_7`, and potentially other stack variables. By the time `intern_keywords` tries to access the Vec through the corrupted struct, the pointer is garbage.

### Same Issue in `init_global_interner()`

```llvm
%_2 = alloca ptr          ; 8 bytes
; ...
%tmp2 = getelementptr { { ptr, { ptr, i64, i64 } } }, ptr %_2, i64 0, i32 0
store { ptr, { ptr, i64, i64 } } %tmp1, ptr %tmp2    ; 32 bytes → 8 byte alloca!
%tmp3 = load ptr, ptr %_2                             ; reads garbage
store ptr %tmp3, ptr %_1                              ; stores garbage to GLOBAL_INTERNER
```

## Comparison with Blood-Rust (Reference Compiler)

Blood-rust uses **properly-sized inline structs** for all allocas:

```llvm
; Blood-rust's StringInterner::new() — reference_ir.ll
%_1_stack = alloca { { i8*, i64, i64 }, { { i8*, i64, i64 }, i64, i64 } }, align 8
; ^^^ 48 bytes — correct for the full StringInterner struct
```

| Aspect | Blood-Rust (correct) | Self-Hosted (buggy) |
|--------|---------------------|---------------------|
| StringInterner alloca | 48 bytes (inline struct) | 8 bytes (`alloca ptr`) |
| Vec\<String\> alloca | 24 bytes (`{ ptr, i64, i64 }`) | 8 bytes (`alloca ptr`) |
| Struct literal store | Fits in alloca | **Overflows alloca** |
| vec_push elem_size | 24 (size of String) | 8 (pointer size) |

## Scope of the Bug

This is **not isolated** to `StringInterner`. Every ADT struct construction in the self-hosted compiler's output is affected:

- Any function that constructs a struct literal (e.g., `MyStruct { field1: val1, field2: val2 }`)
- Any function that returns a struct by value
- Any function that pattern-matches and reconstructs structs
- Any `Option::Some(large_struct)` wrapping

The crash happens to surface in `intern_keywords` because it's the first code executed at startup, but the bug is systemic.

## The `vec_push` Element Size Issue

A secondary issue: `vec_push` is called with `elem_size = 8` for `Vec<String>`:

```llvm
call void @vec_push(ptr %tmp369, ptr %tmp371, i64 8)
```

In the self-hosted model, `String` is an ADT → pointer → size 8. So the Vec stores 8-byte pointers rather than 24-byte inline strings. This is **internally consistent** within the pointer model — each Vec element IS an 8-byte pointer to a heap-allocated String.

However, this means the self-hosted compiler's `Vec<String>` has different semantics than blood-rust's `Vec<String>`:
- Blood-rust: Vec stores inline `{ ptr, i64, i64 }` elements (24 bytes each)
- Self-hosted: Vec stores pointer elements (8 bytes each)

This difference means Vec data layouts are incompatible between the two compilers, which would cause issues if data is shared across the boundary.

## Code Locations

| Component | File | Line | Description |
|-----------|------|------|-------------|
| ADT layout | `codegen_types.blood` | 257 | `Adt => TypeLayout::pointer()` — returns size=8 for all ADTs |
| Alloca emission | `codegen_stmt.blood` | 521 | `type_to_llvm_with_ctx` called to determine alloca type |
| Stack local emit | `codegen_stmt.blood` | 558-568 | `emit_stack_local` emits `alloca` with the (undersized) type |
| Struct literal gen | (codegen_expr.blood) | — | Emits `store { full_struct_type }` into ptr-sized allocas |
| vec_push handler | `codegen_term.blood` | 673-734 | Computes elem_size from `type_size_bytes` (returns 8 for ADTs) |
| Type size query | `codegen_types.blood` | 340-341 | `type_size_bytes` → `get_layout(ty).size` |

## Possible Fixes

### Option A: Full Inline Struct Layout (Recommended)

Change `get_layout` for ADTs to compute and return the actual struct layout (field sizes, alignment, padding) instead of always returning pointer size. This would make the self-hosted compiler's output match blood-rust's output.

**Pros:**
- Fixes the bug completely and systemically
- Makes self-hosted output ABI-compatible with blood-rust
- Vec element sizes become correct
- All struct construction paths work naturally

**Cons:**
- Large change — requires computing struct layouts from field types
- Every codegen path that assumes ADTs are pointers needs updating
- Need to handle recursive/self-referential types (use pointers for those)
- May uncover other assumptions in the codegen about ADT representation

**Implementation sketch:**
```blood
// codegen_types.blood — get_layout for ADTs
&hir_ty::TypeKind::Adt { def_id, args } => {
    // Look up the struct definition to get field types
    // Compute total size = sum of field sizes with alignment padding
    // Return TypeLayout with actual size, max alignment, and LLVM struct type
    compute_adt_layout(ctx, def_id, args)
}
```

### Option B: Always Heap-Allocate ADTs (Consistent Pointer Model)

Ensure the codegen never stores ADT values inline — always heap-allocate and store/load only pointers. Struct literal construction would allocate on the heap and return a pointer.

**Pros:**
- Keeps the existing pointer model consistent
- Smaller change — just fix struct construction paths

**Cons:**
- More heap allocations = slower runtime
- ABI incompatible with blood-rust (different calling conventions)
- Still need correct Vec element sizes for runtime interop

### Option C: Targeted Alloca Fix (Surgical)

For locals that receive struct literal stores, emit the alloca with the actual struct type. Keep the pointer model for most operations but fix the overflow.

**Pros:**
- Smallest change, fixes the immediate crash

**Cons:**
- Doesn't fix the systemic inconsistency
- Vec element sizes still wrong
- Other subtle bugs may remain due to size mismatches
- Hard to determine which locals need the fix (analysis required)

## Recommendation

**Option A** is the correct long-term fix. The ADT-as-pointer model is fundamentally inconsistent — it works for some operations (loads, stores of pointers) but breaks for others (struct construction, function returns, Vec element sizing). Blood-rust already computes proper struct layouts, and the self-hosted compiler should match.

The fix would primarily involve:
1. Adding struct field layout computation to `codegen_types.blood`
2. Updating `get_layout` for `TypeKind::Adt` to use computed layouts
3. Testing that all codegen paths handle the larger types correctly
4. Verifying vec_push element sizes become correct

## Verification

After fixing, verify with:
```bash
./build_selfhost.sh full          # Rebuild everything
./build_selfhost.sh verify        # Check declarations and IR
./second_gen version              # Should not segfault
./build_selfhost.sh test          # Smoke tests should pass
```

The existing verification infrastructure (`check_declarations.sh`, FileCheck tests) will catch any ABI regressions introduced by the layout changes.
