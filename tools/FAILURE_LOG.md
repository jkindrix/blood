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
| 2026-02-20 | runtime | Effect handler tests (26) crash with exit code 1 | `blood_push_handler`/`blood_pop_handler`/`blood_resume` not in static lib — blood-rust generates inline via LLVM | Blocked: needs runtime library changes |
| 2026-02-20 | parser | 3 closure tests fail (COMPILE_FAIL) | Closure syntax not implemented in self-hosted parser | Feature gap |
| 2026-02-20 | codegen | 5 effect tests SIGSEGV | Effect handler codegen incomplete in first_gen | Feature gap |

---

## Resolved Issues

| Date | Category | Symptom | Root Cause | Resolution | Files |
|------|----------|---------|------------|------------|-------|
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
