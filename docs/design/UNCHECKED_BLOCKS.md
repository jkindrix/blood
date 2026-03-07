# Design Evaluation: Unchecked Blocks

**Status:** Accepted
**Date:** 2026-03-07
**Spec ref:** GRAMMAR.md §5.4, SAFETY_LEVELS.md (RFC)

## Decision Summary

`unchecked(checks) { ... }` is a **compiler directive** that elides specific runtime check insertions during codegen. It is NOT an effect handler.

## Design Decisions

### 1. Syntactic block, not effect handler

**Considered:** Modeling runtime checks as algebraic effects, with `unchecked` installing a no-op handler.

**Rejected because:**
- Generation checks cost 1-2 cycles; effect dispatch costs 10-50 cycles. Making the *checked* path slower inverts the design goal.
- Safety checks are precondition assertions, not side effects. A bounds check doesn't *do* something — it *assumes* something.
- No existing language (including Koka, the poster child for effect systems) models safety checks as effects.
- Koka's `unsafe-total` erases *user-defined* effects, not compiler-inserted runtime checks.

**Decision:** `unchecked` is a syntactic block that sets codegen flags. The compiler elides check insertion for the specified checks within the block's scope.

### 2. Five check kinds

| Check | What it elides | Current implementation |
|-------|---------------|----------------------|
| `bounds` | Array/slice bounds checking | Partial (string indexing) |
| `overflow` | Integer overflow intrinsics → plain arithmetic | LLVM overflow intrinsics |
| `generation` | `blood_check_generation()` / `blood_validate_generation()` calls | Fully implemented |
| `null` | Null pointer guards | Not yet implemented |
| `alignment` | Alignment verification | Not yet implemented |

Only `overflow` and `generation` have existing check infrastructure to elide. `bounds` is partial. `null` and `alignment` are reserved for future use — parsing them is valid, but they have no codegen effect yet.

### 3. Conditional mode

```blood
unchecked(bounds, overflow, when = "release") {
    // Checks skipped only when compiling with --release
    // In debug builds, all checks remain active
}
```

`when = "release"` makes the unchecked directive conditional on the build profile. This enables writing performance-critical code that retains safety checks during development.

### 4. No propagation across effect boundaries

`unchecked` state is **purely lexical**. When a handler resumes a continuation, the continuation runs with full checks regardless of the handler's unchecked context.

**Rationale:** A handler might capture a continuation and resume it in a different context (different region, different thread). The unchecked assertion "I've proven this check won't fire" applies to the handler's lexical scope, not to arbitrary future execution contexts.

### 5. Distinction from `@unsafe`

- **`@unsafe`** gates fundamentally unsafe operations the type system cannot verify (raw pointer dereference, type punning, inline assembly). It marks "the compiler cannot help you here."
- **`unchecked(checks)`** disables specific runtime checks the compiler normally inserts. The operation is type-safe; only the validation is skipped. It marks "I've proven this check won't fire; skip it for performance."

Both are auditable via grep. Effects remain tracked in both contexts.

## Implementation

### Syntax

```ebnf
UncheckedBlock ::= 'unchecked' '(' UncheckedChecks (',' 'when' '=' StringLiteral)? ')' Block
UncheckedChecks ::= Ident (',' Ident)*
```

### Pipeline

1. **Parser** — `unchecked` as contextual keyword. Parse check names + optional `when` clause. Produce `ExprKind::Unchecked { checks, when_condition, body }`.
2. **HIR** — Lower transparently. Validate check names (E0500 for unknown checks).
3. **MIR** — `EnterUnchecked(checks)` / `ExitUnchecked` statements bracket the body. These carry the resolved check set.
4. **Codegen** — Maintain `active_unchecked: HashSet<UncheckedCheck>` stack. Skip check emission when the relevant check is active. For `when = "release"`, only activate when `--release` flag is set.

### Check elision points

- **overflow:** `Rvalue::CheckedBinaryOp` → emit `Rvalue::BinaryOp` (plain arithmetic, no overflow intrinsic)
- **generation:** `PlaceElem::Deref` → skip `emit_generation_check()` / `blood_validate_generation()` call
- **bounds:** Index expressions → skip bounds comparison branch (when implemented)
