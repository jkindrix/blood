---
name: Proving Ground strategy and progress
description: 7 programs that prove Blood's pillars. Program 1 (effectful) COMPLETE. Execution order, known blockers, bugs found/fixed.
type: project
---

## Proving Ground — Development Strategy (2026-03-27)

Write programs that exercise Blood's pillars. Fix compiler bugs they reveal.

**Order:** effectful (P3) → safeio (P5) → arena (P1) → multidispatch (P4) → hashcache (P2) → transaction (P3+P5) → actors (P3+P1+P5)

**Supersedes** `.tmp/WORK.md` phases and `.tmp/ACTION_PLAN.md` as "what to work on next."

**Canonical doc:** `.tmp/PROVING_GROUND.md`

## Program 1: effectful — COMPLETE (2026-03-27)

- **What it proves:** Pillar 3 (Composability / Algebraic Effects)
- **Location:** `~/blood-projects/effectful/effectful.blood`, `tests/proving/p1_effectful.blood`
- **Features exercised:** handler swapping (4 effect pairs), effect composition (4 nested), non-resumptive abort, handler state mutation, return values through handler chains
- **Bugs found: 3, fixed: 3**

| Bug | Root Cause | Fix |
|-----|-----------|-----|
| HANDLER-SWAP | `__blood_register_handlers` grouped by effect_def_id → shared ops array | Per-handler registration with unique `@__handler_reg_idx_{handler_def_id}` globals |
| HANDLER-BODY-SHARED | `find_fn_for_body` returned shared `op_def_id` → all handler op bodies compiled to same function, worklist dedup skipped duplicates | Synthetic unique def_id (`0x80000000 + body_id`), `typeck_def_id` field on `CodegenWorkItem` to decouple naming from typeck resolution |
| UNIT-TYPE-WIDEN | Handler return clause `sext {} undef to i64` invalid for unit type | Unit → `i64 0`, ptr → `ptrtoint/inttoptr`, in all three widen/narrow paths |

**Key design decisions:**
- `typeck_def_id` on `CodegenWorkItem`: MIR lowering uses original `op_def_id` for method/field resolution (typeck side tables keyed on it), but codegen naming/signature uses the unique synthetic def_id. Body's `def_id` overridden after `lower_body` returns.
- `find_handler_op_typeck_def_id()` helper recovers original `op_def_id` from HIR handler items.
- t03_effect_handler_reperform expected output was WRONG (reflected buggy shared-body behavior). Updated to correct output (`outer:20`).

**Why:** `Counter` uses non-parametric effects (not `State<T>`) to avoid F-02 risk. Parametric effects work (tested separately via t03_effect_parametric_handlers) but cross-type-family (`Emit<i32>` + `Emit<bool>`) remains unverified.

## Remaining Blockers (from pre-start analysis)

| Blocker | Status | Notes |
|---------|--------|-------|
| MOD-HANDLER-NAME | OPEN | Not needed for P1 (single-file). Handler return clause naming collision when `mod` imports + multiple handlers. |
| F-02 parametric type leak | UNVERIFIED | Fixed for i32+i64 (f54e676, 4538eae). Untested for i32+bool (cross-type-family). |
| Effects + trait methods | PARTIAL | Simple case works (t03_effect_trait_method). Type args not inherited from trait methods. |
| Inline handler mono segfault | OPEN | Named handlers work fine. |

## Next: Program 2 — safeio (Pillar 5: Isolation)

**How to apply:** Start `~/blood-projects/safeio/`. See `.tmp/PROVING_GROUND.md` for program spec.
