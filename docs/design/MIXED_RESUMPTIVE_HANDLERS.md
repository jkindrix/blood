# DD-3: Mixed-Resumptive Handler Semantics

**Status:** CLOSED — Option B selected
**Date:** 2026-03-15
**Blocks:** codegen_term.blood, codegen/mir_codegen/terminator.rs

## Problem

A handler may contain ops where some always resume and some never resume:

```blood
effect Resource<T> {
    op acquire() -> T;    // Always resumes with a value
    op release(r: T);     // Never resumes (cleanup, abort)
}
```

The current implementation classifies the entire handler as either tail-resumptive (TR) or non-tail-resumptive (NTR) using a blanket `all_tail_resumptive` flag. When ANY op is NTR, the whole handler gets NTR treatment — meaning every perform allocates a continuation, even for ops that always resume.

## Current State

- **Bootstrap:** Per-op `OpImplInfo.is_tail_resumptive` exists. MIR Perform terminator `is_tail_resumptive` is set per-op by checking effect name against standard effects. Handler-level `all_tail_resumptive` drives PushHandler abort target.
- **Selfhost:** `is_tail_resumptive` is **hardcoded to false** on all Perform terminators (mir_term.blood:328). Every perform allocates a continuation.

## Decision: Option B — Per-Op Resume Classification at Perform Site

### Handler level (PushHandler)

Keep blanket `all_tail_resumptive` for the abort target decision at PushHandler. If any op is NTR, set up setjmp. This is correct and the overhead (one setjmp per handler scope) is negligible.

### Perform level (per-op)

Each Perform terminator sets `is_tail_resumptive` independently based on the specific op being performed:
- If op always resumes (tail position) → `is_tail_resumptive: true` → skip continuation allocation
- If op might not resume → `is_tail_resumptive: false` → create continuation
- If unknown/conditionally resumptive → `false` (conservative, safe)

### Selfhost fix

Wire per-op `is_tail_resumptive` from `effect_evidence.OperationEvidence` into MIR Perform terminator construction. Currently hardcoded to `false`.

### Rationale

- Infrastructure already exists in both compilers — just needs wiring
- ~30 lines of change in the selfhost
- Eliminates unnecessary continuation allocation for TR ops in mixed handlers
- Conservative default (false) ensures correctness for unknown effects
- No new language semantics — this is purely an optimization of existing behavior

### Rejected Alternatives

**Option A (keep blanket):** Correct but wastes continuation allocation on every perform. The selfhost performs thousands of effect operations during compilation — unnecessary overhead.

**Option C (per-op setjmp):** Maximum performance but requires restructuring codegen to emit setjmp at perform sites instead of handler scopes. ~200 lines, high risk of new bugs, marginal benefit over Option B.
