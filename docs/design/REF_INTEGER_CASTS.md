# Design Evaluation: Reference-Integer and Function-Integer Casts

**Date**: 2026-03-05
**Status**: Evaluated
**Audit Item**: TYP-05 (from `.tmp/AUDIT.md`)
**Triggered By**: Spec alignment review — questioning whether `close-spec-to-impl` was the correct resolution

---

## 1. Question

Should Blood's `cast_compatible` relation include:
- Rule 9: `&T as usize`, `usize as &T` (reference-integer roundtrip)
- Rule 10: `fn as usize` (function pointer to integer)

These rules were added in commit `a08d99a` to match compiler behavior. This evaluation asks whether that was the right decision from first principles, independent of what the compilers currently do.

---

## 2. Blood's Reference Model

Blood references are **generational references**, not bare pointers. Per MEMORY_MODEL.md §2.1:

```
128-bit Blood Pointer:
[64-bit ADDRESS | 32-bit GENERATION | 32-bit METADATA]
```

- **Generation**: Expected generation counter (0 to 2^32-1). Compared against slot generation on dereference. Mismatch raises `StaleReference` effect.
- **Metadata**: Tier (4 bits), flags (4 bits), type fingerprint (24 bits).
- **Reserved generations**: `PERSISTENT_MARKER` (0xFFFFFFFF), `OVERFLOW_GUARD` (0xFFFFFFFE), `TOMBSTONE_GEN` (0xFFFFFFFD).

A `usize` is 64 bits. A Blood reference carries 128 bits of safety information. **Any cast from `&T` to `usize` necessarily discards generation and metadata.**

---

## 3. Analysis: `&T ↔ usize` (Rule 9)

### 3.1 Soundness Problem

Casting `&T as usize` strips generation information. Casting `usize as &T` fabricates a reference without a valid generation. The roundtrip `&T → usize → &T` creates a reference that:

1. Has no generation check capability (generation is lost or fabricated)
2. Bypasses the stale reference detection mechanism (MEMORY_MODEL.md §4.6)
3. Could refer to a region that has been reset (dangling)

This directly undermines Blood's core safety guarantee: that generational references detect use-after-free at the point of dereference.

### 3.2 The Two-Step Path Already Exists

The spec already provides a safe path for pointer-to-integer conversion:

- **Rule 7**: `&T as *const T` — strips generation, produces raw pointer (bridge context)
- **Rule 6**: `*const T as usize` — raw pointer to integer (bridge context)

The two-step path makes the safety boundary explicit: you must first acknowledge leaving the generational reference system (→ raw pointer) before entering the integer domain. This is a deliberate safety checkpoint, not bureaucratic friction.

### 3.3 FFI.md Already Forbids This

FFI.md §3.2 states:

> **Critical: Blood's `&T` and `&mut T` references are NOT FFI-safe. They carry generation metadata. Use raw pointers for FFI.**

Allowing `&T as usize` in the cast rules contradicts this design guidance. If references aren't FFI-safe because they carry metadata, they shouldn't be directly castable to integers that discard that metadata.

### 3.4 What the Self-Hosted Compiler Actually Needs

The audit justified rule 9 with "needed for self-hosting." Examining the actual uses in `src/selfhost/`:

- `def_id as u64` — DefId is an integer-like newtype, not a reference
- `symbol.index as usize` — extracting an integer field, not casting a reference
- `codegen_expr.blood:1840` — `ptrtoint` for function pointers (rule 10, not rule 9)

**No actual `&T as usize` casts were found in the self-hosted compiler.** The self-hosting justification does not hold for rule 9.

---

## 4. Analysis: `fn → usize` (Rule 10)

### 4.1 Function Pointers Are Not Generational

Unlike references, function pointers are static addresses. They have no generation, no metadata, no region scope. Casting `fn as usize` is a straightforward `ptrtoint` with no information loss.

### 4.2 Legitimate Use Case

The self-hosted compiler uses `fn as usize` in `codegen_expr.blood:1840-1858` for thread spawning APIs that require function addresses as integers. This is a real FFI/systems programming need.

### 4.3 The Two-Step Question

Should `fn as usize` require `fn → *const () → usize`? Function types don't have a natural raw pointer representation in Blood's type system (there's no `*const fn(T)->U`). The two-step path would require inventing an intermediate type that serves no safety purpose.

### 4.4 Content-Addressing Consideration

Blood uses content-addressed compilation. Function identity is based on content hashes, not addresses. `fn as usize` gives you a runtime address that has no relationship to the content hash. This is worth documenting but doesn't make the cast unsound — it just means the integer value is meaningful only at runtime.

---

## 5. Decision

### Rule 9 (`&T ↔ usize`): **REMOVE from spec. Revert to two-step path.**

**Rationale:**
1. Blood references are 128-bit generational; `usize` is 64-bit. The cast loses safety information.
2. The roundtrip `&T → usize → &T` creates references that bypass generation checking.
3. The two-step path (`&T → *const T → usize`) already exists and makes the safety boundary explicit.
4. FFI.md explicitly says references are not FFI-safe due to metadata.
5. No actual `&T as usize` uses exist in the self-hosted compiler.
6. Allowing this cast creates a backdoor around Blood's core memory safety guarantee.

### Rule 10 (`fn → usize`): **KEEP in spec, but restrict to `usize` only.**

**Rationale:**
1. Function pointers are static; no safety information is lost.
2. Legitimate use case exists (thread spawning, FFI callback registration).
3. No natural two-step path exists for function types.
4. Restricting to `usize` (not arbitrary integers) prevents meaningless narrowing casts.

The current rule 10 says "fn as usize" which is already `usize`-only. This is correct as-is.

---

## 6. Impact

### Spec Changes
- Remove rule 9 from FORMAL_SEMANTICS.md §5.10.1
- Renumber rule 10 → rule 9
- Add design note explaining why `&T as usize` is forbidden (generational reference safety)

### Compiler Changes (future, not this commit)
- Both compilers currently accept `&T as integer` — this should be tightened to reject it
- Tracked as implementation work item, not addressed here

---

## 7. Comparison with Other Languages

| Language | `&T → integer` | Rationale |
|----------|----------------|-----------|
| Rust | Requires `&T as *const T as usize` | Two-step; raw pointer is explicit |
| C | Implicit pointer decay, then cast | No safety boundary |
| Swift | Not allowed without `Unmanaged` | Reference counting safety |
| Blood (proposed) | Requires `&T as *const T as usize` | Generational reference safety |

Blood's approach matches Rust's for similar reasons: the language has a managed reference type with extra semantics (Rust: lifetimes; Blood: generations), and direct cast to integer would bypass those semantics.
