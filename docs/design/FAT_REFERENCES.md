# Fat Reference Implementation Plan

**Status:** IN PROGRESS
**Date:** 2026-03-15
**Supersedes:** DD-1 header-field approach for struct fields
**Prerequisite:** Runtime madvise fix (7e5f541)

## Decision

`&T` becomes `{ ptr, i32 }` in LLVM IR for ALL references. Generation is
captured at reference creation time and travels with the reference.
Stack-tier elision (replacing `{ ptr, i32 }` with `ptr` for NoEscape refs)
is a future optimization — correctness first, performance second.

## Why This Changed

DD-1's header-field approach (generation at `[ptr-8]`, thin 64-bit pointers)
works for locals but breaks for struct fields containing references. The
generation must be captured at creation time and stored alongside the pointer.
Re-reading from the header at load time is unsound (ABA problem).

DD-1's rejection of fat pointers was based on CHERI research showing 1.4-2x
slowdown for ALL-pointers-128-bit. Blood only fattens `&T` (not raw pointers,
function pointers, or vtable pointers), and stack-tier elision will keep
most references thin. The CHERI comparison doesn't apply.

## Implementation Steps

### Step 1: type_to_llvm (codegen_size.blood)

`&T` → `{ ptr, i32 }` for thin refs, `{ ptr, i64 }` remains for &str/&[T].
`&dyn Trait` → `{ ptr, ptr, i32 }` (data ptr, vtable ptr, generation).

Change `type_to_llvm_fast_id` and `type_to_llvm_with_ctx_id` to return
`{ ptr, i32 }` instead of `ptr` for Ref types.

### Step 2: type_size (codegen_size.blood)

`sizeof(&T)` → 12 bytes (8 ptr + 4 gen, padded to 16 with alignment).

### Step 3: Rvalue.Ref codegen (codegen_expr.blood)

Currently: `store ptr %addr, ptr %dest`
Change to: construct `{ ptr, i32 }` pair — `%addr` from place, `%gen` from
`load i32, ptr getelementptr(i8, %addr, -8)`. For stack pointers (no header),
use generation 0 (sentinel for "unchecked").

### Step 4: PlaceElem.Deref codegen (codegen_expr.blood)

Currently: `%ptr = load ptr, ptr %ref_local`
Change to: `%pair = load { ptr, i32 }, ptr %ref_local` then
`%ptr = extractvalue { ptr, i32 } %pair, 0` and
`%gen = extractvalue { ptr, i32 } %pair, 1`.
Compare `%gen` against `load i32, ptr getelementptr(i8, %ptr, -8)`.
If gen == 0, skip check (stack-tier sentinel).
If mismatch, call blood_stale_reference_panic.

### Step 5: Function signatures

Every function parameter/return that is `&T` changes from `ptr` to
`{ ptr, i32 }`. All call sites must construct/destructure the pair.

### Step 6: Struct field access

Struct fields of type `&T` are `{ ptr, i32 }` in the struct layout.
GEP offsets change. Field loads return the pair. No special-case codegen
needed — it falls out from the type change.

### Step 7: FFI boundaries

At FFI call/return sites, strip generation: `extractvalue { ptr, i32 } %ref, 0`
to get plain `ptr`. On FFI return, wrap: `insertvalue { ptr, i32 } undef, ptr %p, 0`
with generation 0 (unchecked FFI pointer).

## Go/No-Go

Build the selfhost with fat references. Measure build time.
If regression > 15%, implement stack-tier elision before proceeding.

## Scope

~1,500-2,000 lines of codegen changes. Touches:
- codegen_size.blood (type layout)
- codegen_expr.blood (Ref creation, Deref, field access)
- codegen_term.blood (Call parameter/return handling)
- codegen_stmt.blood (Assign, handler state)
- codegen.blood (function signature emission)
