# Failure History Log

**Purpose:** Machine-readable log of past failures, root causes, and resolutions. Prevents future sessions from re-discovering the same issues. Updated after each debugging session.

**Format:** Each entry is a row in the table below. Entries are in reverse chronological order (newest first).

---

## How to Use This Log

1. **Before debugging a new issue:** Search this log for similar symptoms
2. **After resolving an issue:** Add a new entry at the top of the table
3. **Key fields:**
   - `Date` — when the issue was encountered/resolved
   - `Category` — codegen, typeck, parser, hir, mir, runtime, abi, build
   - `Symptom` — what was observed (error message, crash, wrong output)
   - `Root Cause` — what actually went wrong
   - `Resolution` — how it was fixed
   - `Files` — which files were modified

---

## Active Issues (Unresolved)

| Date | Category | Symptom | Root Cause | Status |
|------|----------|---------|------------|--------|
| 2026-04-07 | runtime | Latent `&str`-lifetime bug in first_gen: selfhost holds an `&str` past a `String` reallocation somewhere during HIR lowering. Only detected when `rt_blood_alloc_simple` gen tracking is enabled (fd43ec7's intent). | Unknown specific site. When String's data buffer grows, `ensure_cap` calls `alloc_simple` for new buffer and `free_simple` for old; old buffer's gen increments, invalidating `&str` that should have been updated. | Open. GAP-1 (stale `&str` detection) is currently REVERTED because of this. Reproduce by temporarily changing `alloc.blood:rt_blood_alloc_simple` to call `rt_blood_register_allocation_tagged(addr, size, 2)`, rebuild runtime, then rebuild first_gen — first_gen will panic during self-compilation. |
| 2026-03-09 | codegen | LLVM verification "Instruction does not dominate all uses" when user-defined effect named `StaleReference` is nested with other handlers | `StaleReference` is a built-in effect (ID 0x1004) with special runtime codegen; user-defined effect with same name collides with built-in handler registration | Workaround: use different effect name (e.g., `StaleAccess`) |
| 2026-03-09 | codegen | Two named handlers for same effect type both execute first handler's op code | `blood_evidence_push_with_state` searched registry by effect_id, always found first match; also registry index globals used Internal linkage, invisible across incremental compilation object files | Resolved: see below |
| 2026-03-09 | typeck | Functions with effectful code inside `with...handle` blocks still inferred as performing those effects | Effect inference scans function body without subtracting effects handled by in-scope handlers | Workaround: extract body into separate function with explicit `/ {Effect}` annotation |
| 2026-02-20 | runtime | Effect handler tests (26) crash with exit code 1 | `blood_push_handler`/`blood_pop_handler`/`blood_resume` not in static lib — blood-rust generates inline via LLVM | Blocked: needs runtime library changes |
| 2026-02-20 | parser | 3 closure tests fail (COMPILE_FAIL) | Closure syntax not implemented in self-hosted parser | Feature gap |
| 2026-02-20 | codegen | 5 effect tests SIGSEGV | Effect handler codegen incomplete in first_gen | Feature gap |

---

## Resolved Issues

| Date | Category | Symptom | Root Cause | Resolution | Files |
|------|----------|---------|------------|------------|-------|
| 2026-04-08 | mir | Nested closure inside another closure's body: inner closure function definition not emitted. `llc-18: use of undefined value '@blood_closure_<id>'`. Simple closures and captured closures work; only nesting fails. Reproducer was committed as XFAIL at `tests/golden/t04_nested_closure_xfail.blood`. | `mir_lower_expr.blood:lower_closure_expr` at line 2488 and `lower_inline_handle_expr` at 2618 created a nested `MirLowerCtx`, lowered the closure body into it, then called plain `finish()` which consumed the ctx and silently dropped `closure_names`/`closure_mir`/`closure_is_handler_op`. Any closures discovered while lowering the body were in those vecs and got thrown away. Top-level lowering in mir_lower.blood:82,158,246,298 already had the correct extract-before-finish pattern; the nested sites just didn't use it. | Added `finish_nested(self, parent: &mut MirLowerCtx)` method that bitwise-copies the three closure vecs before consuming `self.builder`, then re-registers each entry on the parent ctx via `register_closure_body`/`register_inline_handler_body`. Both nested sites now call `finish_nested(ctx)` instead of `finish()`. Transitive propagation verified through 3 nesting levels. Tests: `t04_nested_closure.blood` (un-XFAIL'd), new `t04_doubly_nested_closure.blood`. Gate: second_gen == third_gen byte-identical. | `mir_lower_ctx.blood`, `mir_lower_expr.blood`, `tests/golden/t04_nested_closure.blood`, `tests/golden/t04_doubly_nested_closure.blood` (commit 2b6d72e) |
| 2026-04-08 | typeck | Compiler silently accepted function calls with wrong arity in external module bodies (single-file and main-file bodies were always checked correctly). Surfaced by fd43ec7's `rt_blood_register_allocation(addr, size, 2)` — 3 args to a 2-arg function — slipping through typeck and producing `trunc i32 to i64` in the emitted LLVM IR, which llc rejected. Tracked as NEW-4 in audit. | Misdiagnosed originally as "typeck does not validate arity." The arity check at `typeck_expr.blood:1252` has always worked for main-file bodies. The actual bug was in `typeck_driver.blood:790-793`: Phase 2b runs typeck on external module bodies to record method resolutions, then **discards all accumulated errors** via a truncate-back-to-main_error_count loop, on the theory that "each module is authoritative only when compiled as the main file." That theory was partially right (cross-module paths produce false positives for unresolved names, type mismatches, etc.) but applied indiscriminately to every error class including arity. The runtime is never compiled as a main file, so its arity mismatches were always discarded. | Narrow fix: selectively keep only `ArityMismatch` errors from Phase 2b; discard other error classes unchanged. Arity errors are sound regardless of scope completeness because the check only fires when the callee has resolved to a concrete `Fn` type — if the signature is known, the arg count is authoritative. Other error classes remain discarded until a deeper future fix addresses Phase 2b's cross-module scope AND the error reporter's lack of multi-module span handling. Tests: new golden `tests/golden/t06_err_wrong_arity.blood`; cross-module case exercised implicitly by runtime build during gate. Verification: re-applying fd43ec7 now produces E0205 with exit 1. | `typeck_driver.blood`, `tests/golden/t06_err_wrong_arity.blood` (commit f6285a5) |
| 2026-04-07 | runtime | `WAS_RESUMED` leak across nested effect dispatch: handler body doing nested resumed perform then aborting without resume would have its intended abort silently dropped | `WAS_RESUMED` is a single static global; nested perform cleared it, inner handler set it via resume, outer check saw the leaked value | Save/restore outer's `WAS_RESUMED` around handler dispatch; read this-handler's value into stack-local before restoring outer's state | `rt_effect.blood` (commit 8b9e48b) |
| 2026-04-07 | runtime | `blood_realloc` silently copied 0 bytes and freed the old buffer when called with an unregistered address (registry_lookup_size returned 0) | `registry_lookup_size` returns 0 for unknown addresses; realloc's min(0, new_size) = 0 path ran copy_size = 0 | Added explicit panic when `old_addr > 1 && old_size == 0`. Callers must only pass addresses obtained from blood_alloc/blood_alloc_or_abort | `alloc.blood` (commit 7279988) |
| 2026-04-07 | runtime | Capacity doubling in `rt_string.blood` and `rt_vec.blood` could overflow i64 when cap > i64::MAX/2 and silently wrap to a negative value | `cap * 2` without overflow check; subsequent `if new_cap < 16 { new_cap = 16 }` masked the wrap | Added explicit overflow guards before the multiply; panic cleanly if cap or needed exceed 0x3FFF_FFFF_FFFF_FFFF; added elem_size × new_cap overflow check for vec | `rt_string.blood`, `rt_vec.blood` (commit a415e58) |
| 2026-04-07 | build | `build_selfhost.sh` silently packaged stale runtime `lib.o` as "success" when llc failed to rebuild the runtime archive. Hid runtime-source regressions indefinitely | `llc-18 ... 2>&1 \| grep -v ... \|\| true` — the `\|\| true` at the end of the pipeline unconditionally returned 0 regardless of llc's exit code | Capture llc's exit code separately via PIPESTATUS[0] (disabling pipefail locally for the pipe), die on non-zero, verify lib.o exists before packaging | `build_selfhost.sh` (commit 99af113), `build_first_gen_blood.sh` (commit 43229ab) |
| 2026-04-07 | typeck | Bootstrap (Rust) unify tests failing (5 tests) after April 5 commit added permissive coercions (i32 ≌ i64, &mut T → &T asymmetric) | Coercions added directly to `unify_inner` violated unification invariants: symmetry (`unify(a,b) ⇔ unify(b,a)`) and constructor distinctness | Removed both coercion arms from unify.rs. Coercions should live in `check_expr` (coercion phase), not in unification proper. Regresses "bootstrap compiles selfhost" state from 1bc9c95 but restores invariants | `src/bootstrap/bloodc/src/typeck/unify.rs` (commit a1a6813) |
| 2026-04-07 | runtime | Runtime build had been silently failing for days: `fd43ec7` added a 3-arg call to 2-arg `rt_blood_register_allocation` in `rt_blood_alloc_simple`. Compiler (NEW-4) accepted it and emitted `trunc i32 to i64` (invalid LLVM IR, can't widen with truncation) | The intent was to register alloc_simple buffers in the gen table with source=2 (for stale &str detection). The correct call was `rt_blood_register_allocation_tagged(addr, size, 2)`. The arity bug + error swallowing combined to produce an archive from stale lib.o for days | Reverted `alloc.blood` to pre-fd43ec7 state (no registration). Re-enabling GAP-1 requires first fixing a latent first_gen `&str`-lifetime bug that surfaces when gen tracking is enabled | `runtime/blood-runtime/alloc.blood` (commit 5a6ee57) |
| 2026-04-07 | codegen | Monomorphization deduplication silently dropped duplicate function names as a "workaround for missing proper monomorphization" | Dead code in practice (no duplicates observed in any current build). Landmine: if it ever fired, it would silently drop code | Made the dedup loud: emits stderr warning identifying the duplicated name as a mangling or driver bug. Suffixes IR comment with `(bug)` | `codegen.blood` (commit aea02b3) |
| 2026-03-09 | codegen/runtime | Two named handlers for same effect type both execute first handler's ops | Two causes: (1) `blood_evidence_push_with_state` searched registry by effect_id, always returning first match; (2) per-handler registry index globals used Internal linkage, invisible across incremental compilation object files | Added `blood_evidence_push_by_index(ev, registry_index, state)` runtime function; `blood_evidence_register` now returns i64 registry index; codegen stores index in per-handler Common-linkage globals; PushHandler uses `push_by_index` with loaded index | `ffi_exports.rs`, `handlers.rs`, `mod.rs`, `statement.rs` |
| 2026-02-15 | codegen | Trait default method calls dispatch to void stub | DefId resolves to trait abstract method, not concrete impl | Call remapping table: `inject_default_methods` builds `CallRemapEntry`, applied in `extract_direct_fn_name` via `ctx.remap_def_id()` | `typeck_driver.blood`, `codegen_term.blood`, `main.blood` |
| 2026-02-15 | codegen | Stateless effect handlers crash on null state pointer | Codegen accessed handler state for stateless handlers | Added null state pointer guard in codegen | `codegen.blood` |
| 2026-02-14 | typeck | Resume type mismatch not caught | Missing resume validation in shallow handlers | Added `resume_ty`, `in_shallow_handler`, `resume_count` fields on TypeChecker; `setup_handler_op_context()` sets context | `typeck_driver.blood` |
| 2026-02-14 | codegen | String runtime calls have wrong ABI | `string_push_str`/`string_as_str`/`string_as_bytes` passed `{ ptr, i64 }` by value, expected ptr to stack | Fixed declarations in codegen.blood, call emission in codegen_term.blood | `codegen.blood`, `codegen_term.blood` |
| 2026-02-13 | codegen | Generic ADT statics/allocas wrong size (8 bytes) | `type_to_llvm_with_ctx` ignored generic args for ADTs | Check args in `codegen_stmt.blood`, compute `{ i8, [payload x i8] }` | `codegen_stmt.blood`, `codegen_streaming.blood` |
| 2026-02-13 | codegen | ADT layout sizes wrong (`Box<T>` inflated) | `populate_adt_registry` called before `register_builtin_adts` | Reorder calls in `main.blood` | `main.blood` |
| 2026-02-12 | codegen | ALL ADTs report type size = 8 | `codegen_types::type_size_bytes()` returned 8 for all ADTs | Replaced with `codegen_stmt::type_size_with_ctx()`, registered builtins, added `rebuild_adt_layouts()` | `codegen_types.blood`, `codegen_stmt.blood` |
| 2026-02-12 | codegen | Vec indexing doesn't load data pointer | Index projection skipped Vec data pointer load | Added `is_vec_like_type`, `resolve_index_element_type`, `hir_to_llvm_with_ctx` | `codegen_expr.blood` |
| 2026-02-11 | codegen | Array type size wrong in llvm_type_size | `llvm_type_size()` didn't handle `[N x T]` — returned 8 | Added `parse_array_size()` function | `codegen_types.blood` |
| 2026-02-11 | codegen | Invalid bitcast for different-size integer casts | `emit_cast_with_types` fallback used `bitcast` | Delegate to `emit_cast_from_llvm_types` | `codegen_expr.blood` |
| 2026-02-10 | mir | Enum ref match always takes first arm | `pattern_test()` returned None for `PatternKind::Ref` | Recurse into inner pattern with Deref projection | `mir_lower_pattern.blood` |
| 2026-02-10 | mir | Vec indexing through references fails | Base type was `Infer(TyVarId)`, never matched `Ref` | Use `ctx.resolve_type()` before type checks | `mir_lower_expr.blood` |
| 2026-02-09 | codegen | Field access defaults to index 0 | `expr_to_place` didn't call `lookup_field_idx()` for assignments | Added field index lookup for assignment targets | `mir_lower_expr.blood` |
| 2026-02-09 | hir | Option variant ordering reversed | None=1, Some=0 (should be None=0, Some=1) | Swap variant indices in hir_lower.blood | `hir_lower.blood` |
| 2026-02-08 | hir | Static variable initialization fails | 5 root causes: interner mismatch, @ prefix, missing @, stub generation, body filtering | Fix all 5 issues in HIR lowering and codegen | Multiple files |
| 2026-02-08 | lexer | Character literal `'a'` lexed as lifetime | Lexer didn't peek ahead for closing `'` | Added peek-ahead logic in lexer | `lexer.blood` |
| 2026-02-07 | typeck | Pattern field resolutions missing | Type checker didn't record field resolutions for struct/enum patterns | Added `record_field_resolution()` in propagate_struct_pattern_fields | `typeck_expr.blood` |
| 2026-02-07 | typeck | Field resolution cross-file collision | `span_start` alone caused overlapping keys across files | Composite key: `(body_def_id, span_start)` | `typeck_expr.blood` |
| 2026-02-07 | typeck | Chained field access collision | `self.next.kind` — both fields share `expr.span.start` | Use `name.span.start` (field name position) instead | `typeck_expr.blood` |
| 2026-02-06 | hir | Effect handler body has empty locals | Handler return/op bodies had no resolver scope | Push scope, register state fields + params | `hir_lower_item.blood` |
| 2026-02-06 | hir | Nested function names don't match | AST parser Symbol indices don't match HIR interner | Re-intern via `ctx.span_to_string(span)` + `ctx.intern()` | `hir_lower_expr.blood` |

---

## Patterns and Anti-Patterns

### Common Root Causes (check these first)

1. **Interner mismatch:** AST parser and HIR lowering use different string interners. Symbol indices from parser don't match HIR interner. Always re-intern when crossing the boundary.

2. **Unresolved types during MIR lowering:** Expression types are `Infer(TyVarId)`. Must call `ctx.resolve_type()` to get concrete types before any type-based decision.

3. **Field resolution keys:** Use `name.span.start` (field NAME position), not `expr.span.start`. Composite key: `(body_def_id, name_span_start)`.

4. **Three `type_to_llvm_with_ctx` functions:** `codegen_ctx` (method, no generics), `codegen_stmt` (standalone, full generics), `codegen_size` (standalone, no circular deps — use from `codegen_expr`).

5. **ABI mismatches:** Blood string type is `{ ptr, i64 }`. C ABI expects pointer to stack for aggregate types. Check calling convention when adding runtime function calls.

### Debugging Workflow

1. **difftest.sh** → identify DIVERGE tests
2. **minimize.sh** → reduce to minimal reproduction
3. **phase-compare.sh** → identify which phase diverges
4. **memprofile.sh** → if memory is the issue
5. **LLVM IR diff** → compare function-by-function IR
6. Add entry to this log when resolved
