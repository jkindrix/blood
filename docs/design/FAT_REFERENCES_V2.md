# Fat References V2: Uniform Design

**Status:** OPEN — design phase
**Date:** 2026-03-15
**Supersedes:** FAT_REFERENCES.md (V1, reverted due to unsoundness)

## Lessons from V1 (Failed Approach)

The V1 implementation changed sized `&T` from `ptr` to `{ ptr, i32 }` while
keeping unsized refs (`&str`, `&[T]`, `&dyn Trait`) as their existing fat
pointer representations. This created two problems:

1. **Two incompatible pointer representations.** Code that passes `ptr` to a
   function expecting `{ ptr, i32 }` (or vice versa) silently corrupts memory.
   The codegen had to manually juggle conversions at dozens of sites. Missing
   ONE conversion caused a third-gen bootstrap crash.

2. **[ptr-8] reads on pointers without headers.** `blood_persistent_alloc`
   didn't add the generation header. The cross-function gen capture read
   `[ptr-8]` from persistent allocations, reading garbage. Stack pointers
   also lack headers. Any `[ptr-8]` read is only safe when the allocation
   path is KNOWN to have added a header — and the codegen had no way to
   verify this at compile time.

3. **is_vec_like_type confusion.** `{ ptr, i32 }` (fat ref) matched the
   "starts with `{ ptr`" pattern used to detect Vec `{ ptr, i64, i64 }`,
   causing Vec field access on fat refs.

## Requirements for V2

### R1: Uniform representation
`&T` is ALWAYS `{ ptr, i32 }` in LLVM IR. No exceptions. No thin/fat
distinction. No manual conversion between representations.

### R2: All allocations have headers
Every allocation that can be referenced (`blood_alloc_or_abort`,
`blood_persistent_alloc`, `blood_region_alloc`, stack allocas for
escaping locals) must have a readable generation value at `[ptr-8]`.
Stack allocas use gen=0 (sentinel). Heap/region/persistent allocations
use the real generation.

### R3: No [ptr-8] reads at arbitrary sites
The generation travels IN the `{ ptr, i32 }` pair, not read from memory.
The only place `[ptr-8]` is read is at reference CREATION time (`&x`),
where the codegen reads the generation from the allocation header and
packs it into the fat ref. After that, the gen component of the fat ref
is used for all comparisons — never `[ptr-8]` again (except as the
"current generation" for the validity check at dereference time).

### R4: Vec detection must be unambiguous
`is_vec_like_type` must use the type interner (HIR type tracking) to
identify Vec, not LLVM type string pattern matching. `{ ptr, i32 }` must
NEVER be confused with any container type.

### R5: Unsized references carry generation too
`&str` becomes `{ ptr, i64, i32 }` (data + len + gen).
`&[T]` becomes `{ ptr, i64, i32 }`.
`&dyn Trait` becomes `{ ptr, ptr, i32 }` (data + vtable + gen).
Uniform: every reference type's LAST field is `i32` generation.

### R6: FFI boundary stripping
At FFI call sites, strip generation: `extractvalue { ptr, i32 } %ref, 0`.
At FFI return sites, wrap with gen=0: `insertvalue { ptr, i32 } undef, ptr %p, 0`.
This is explicit and auditable — every FFI site has the conversion.

### R7: Stack references use gen=0 universally
When `&x` is taken for a stack-allocated `x` (NoEscape tier), the gen
component is 0. The dereference check skips when gen=0 (no overhead for
stack-tier references). This is the performance escape hatch.

## Implementation Strategy

### Phase A: Type system change (type_to_llvm)
Make ALL Ref types return the fat representation. This will break
compilation of everything. That's expected.

### Phase B: Ref creation (Rvalue::Ref)
Construct `{ ptr, i32 }` pairs. For region/persistent locals, read gen
from `[ptr-8]`. For stack locals, use gen=0.

### Phase C: Deref (PlaceElem::Deref)
Extract ptr and gen from fat ref. Compare gen against `[ptr-8]` if gen != 0.

### Phase D: Function signatures
All parameters and returns of type `&T` use the fat representation.
No conversion needed — it's universal.

### Phase E: Vec detection fix
Replace `is_vec_like_type` string pattern matching with HIR type
tracking. Check `current_hir_type` for Adt(Vec/String/HashMap), not
`current_base` string matching.

### Phase F: Struct field access
Struct fields of type `&T` are `{ ptr, i32 }` in the struct layout.
GEP offsets adjust automatically from the type change. No special code.

### Phase G: FFI boundary
Add extract/insert at FFI call/return sites.

### Phase H: Unsized refs
Extend `&str`, `&[T]`, `&dyn Trait` with the i32 gen field.

## Key Difference from V1

V1 tried to be incremental — change sized refs, keep unsized, convert
at boundaries. This created a TWO-WORLD problem where every code path
had to handle both representations.

V2 is all-or-nothing. Every `&T` is fat. Every allocation has a header.
Every reference carries its generation. The codegen has ONE representation
to handle. The change is larger but UNIFORM — no special cases, no
conversion logic, no ambiguity.

## Estimated Scope

~1,500 lines of codegen changes. All in one coordinated commit.
Must pass bootstrap gate before merge.

## Prerequisites

1. All allocation paths (region, persistent, global) have 8-byte headers
2. madvise(DONTNEED) instead of munmap (already done, 7e5f541)
3. Backtrace diagnostics in panic functions (already done, ab5700e)
4. Build log capture for second_gen/third_gen compilation
