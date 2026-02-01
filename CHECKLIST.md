# Self-Hosted Compiler Parity Checklist

**Source:** GAP-ANALYSIS.md
**Rule:** Each item must be fully resolved, compiled, and verified before proceeding to the next.

---

## Phase 1: Fix Incorrect Implementations

Bugs in existing code that produce wrong behavior. These take priority over new features.

- [x] **IC-1: Literal pattern values** — `parser_pattern.blood` creates placeholder values (val:0, bits:0, empty strings) instead of parsing actual source text. Fix `parse_pattern_kind` to extract real literal values. *Fixed: moved literal extraction functions to parser_base.blood; pattern parser now calls parser_base::parse_{int,float,string,char}_from_span.*

- [x] **IC-2: Self parameter parsing** — `parse_fn_param` checks `is_ref_self` via `parser.check(TokenKind::And)` but does not consume the `&` token. `&self` and `&mut self` are not correctly distinguished from `self`. *Fixed: rewrote parse_fn_param with 4 explicit cases (&self, &mut self, self, mut self) matching bootstrap. Each case properly consumes tokens and generates correct default types.*

- [x] **IC-3: Break values lost** — `mir_lower_expr.blood` `lower_break_expr()` evaluates break value with `destination_ignore()` instead of storing to loop result place. Must store to the loop context's result place. *Fixed: added result_dest field to LoopContext, added find_break_dest method, lower_break_expr now stores break value into the loop's destination.*

- [x] **IC-4: Resume expression** — Bootstrap allows `Resume { value: Option<Operand> }` (optional). Self-hosted requires `Resume { value: Operand }` (mandatory). Unit-resume (`resume` without value) is unsupported. Make value optional. *Fixed: changed mir_term.blood Resume to Option<Operand>, updated mir_lower_expr.blood to wrap in Option::Some, updated codegen_term.blood to handle Some/None cases.*

- [x] **IC-5: Forall unification** — Self-hosted directly unifies forall bodies without alpha-renaming; bootstrap creates fresh variables. Fix to alpha-rename before unification. *Fixed: added substitute_forall_vars with nested capture prevention, alpha-renamed both forall bodies with same fresh inference vars before unifying, added Forall-vs-non-Forall instantiation, moved Forall arms before specific type arms.*

- [x] **IC-6: Function pointer coercion** — Self-hosted only checks parameter count, does not verify parameter types or return type. Add full signature checking. *Fixed: try_fn_pointer_coerce now calls checker.unify on each parameter pair and the return type, matching bootstrap behavior.*

- [x] **IC-7: Enum variant construction** — Bootstrap has explicit `ExprKind::Variant`; self-hosted routes through `Struct` path, which may fail for non-struct-like variants. Verify or add dedicated variant handling. *Fixed: expression path lowering now checks DefInfo::variant_index for both single-segment and multi-segment paths (matching pattern lowering). lower_struct_expr now uses path.variant_index instead of hardcoded 0.*

- [x] **IC-8: Record type rest syntax** — Bootstrap uses `| R` (pipe + ident) for row variables in record types. Self-hosted uses `.. name` (dot-dot + ident). Align with bootstrap syntax. *Fixed: changed parser_type.blood to use Or token instead of DotDot, added trailing row variable support after last field.*

- [ ] **IC-9: Function call ABI** — Self-hosted passes all args as `i64` via `ptrtoint`. Incorrect for floats, struct-by-value, multi-arg conventions. Emit typed LLVM arguments matching callee signature.

- [ ] **IC-10: String literal representation** — Self-hosted emits raw `ptr`. Bootstrap emits `{ ptr, i64 }` slice. Code expecting `.len()` fails. Emit proper slice representation.

- [ ] **IC-11: Type size/layout fallback** — Self-hosted defaults unknown types to `{ size: 8, align: 8 }`. `DynTrait` (16), `Range`, `Record`, `Forall`, `Ownership` all get wrong sizes. Handle all type variants.

- [ ] **IC-12: Enum discriminant fallback** — Self-hosted stores `i64` directly when enum not in ADT registry, potentially overwriting payload. Use correct discriminant size.

- [ ] **IC-13: StorageDead protocol** — Bootstrap uses generational invalidation; self-hosted calls `blood_unregister_allocation` / `blood_persistent_decrement`. Align with bootstrap protocol.

- [ ] **IC-14: For-loop desugaring** — Bootstrap: iterator-based (`IntoIterator::into_iter()` -> `next()` -> `Option` match). Self-hosted: index-based (`let i = start; while i < end`). Only integer ranges work. Implement iterator-based desugaring.

- [ ] **IC-15: O(n) substitution lookup** — Self-hosted uses linear `Vec` scan for type variable substitutions. Replace with `HashMap` for O(1) lookup.

---

## Phase 2: Parser Completeness

The parser is the foundation. Downstream phases cannot handle what the parser does not produce.

### High Severity

- [ ] **LP-4: `if let` expression parsing** — AST has `IfLet` but parser never generates it. Add `let` keyword check after `if` in `parse_if_expr`. *Resolves stub: ExprKind::IfLet (ast.blood:672).*

- [ ] **LP-5: `while let` expression parsing** — AST has `WhileLet` but parser never generates it. Add `let` keyword check after `while`. *Resolves stub: ExprKind::WhileLet (ast.blood:691).*

- [ ] **LP-6: Or-patterns** — `PatternKind::Or` in AST but `|` not checked between pattern alternatives. Add `|` parsing after each pattern. *Resolves stub: PatternKind::Or (ast.blood:919).*

- [ ] **LP-3: Macro call expressions** — `ExprKind::MacroCall` in AST but no parsing for `format!`, `vec!`, `println!`, etc. Implement macro call dispatch in expression parsing. *Resolves stub: ExprKind::MacroCall (ast.blood:740).*

- [ ] **LP-1: Bridge declaration parsing** — AST `BridgeDecl` exists but `parse_declaration()` has no arm for `Bridge` token. Add dispatch arm. *Resolves stub: Declaration::Bridge (ast.blood:81), all Bridge\* types (ast.blood:338-449).*

- [ ] **LP-2: Macro declaration parsing** — AST `MacroDecl` exists but `parse_declaration()` has no arm for `Macro` token. Add dispatch arm. *Resolves stub: Declaration::Macro (ast.blood:84), all Macro\* types (ast.blood:967-1079).*

### Medium Severity

- [ ] **LP-7: Compound assignment (`+=`, `-=`, etc.)** — Tokens exist but not in precedence table. Add to `infix_precedence` and generate `ExprKind::AssignOp`. *Resolves stub: ExprKind::AssignOp (ast.blood:661).*

- [ ] **LP-8: Range expressions** — `..`/`..=` not in infix precedence for expressions. Add range operator to expression parser. *Resolves stub: ExprKind::Range (ast.blood:648).*

- [ ] **LP-11: Function qualifiers** — `const fn`, `async fn`, `unsafe fn` not parsed. Parse qualifier keywords before `fn`.

- [ ] **LP-12: Negative literal patterns** — `-42` not handled in pattern position. Add `Minus` handling in pattern parsing.

- [ ] **LP-13: Range patterns** — `0..=9` not parsed in patterns. Add `..`/`..=` check after literal patterns. *Resolves stub: PatternKind::Range (ast.blood:920).*

- [ ] **LP-14: Unclosed block comment error** — Lexer silently eats source to EOF. Emit error token when EOF reached inside block comment.

- [ ] **LP-9: `move` closures** — Always `is_move: false`. Parse `move` keyword before closure `|`.

### Low Severity

- [ ] **LP-10: Loop labels** — Always `None` for `Loop`, `While`, `For`, `Break`, `Continue`. Parse `'label:` syntax before loop keywords.

- [ ] **LP-15: `\x##` and `\u{####}` string escapes** — Not implemented in lexer. Add hex and unicode escape parsing.

- [ ] **LP-16: Doc comment to attribute conversion** — Comments skipped rather than becoming `#[doc = "..."]` attributes. Convert during parsing.

---

## Phase 3: HIR & Name Resolution

Depends on parser producing correct AST nodes.

### High Severity

- [ ] **HR-4: `ExprKind::Region`** — Region blocks lowered as plain blocks, losing region semantics. Lower to dedicated `Region` HIR node preserving allocation tier information.

- [ ] **HR-5: `ExprKind::InlineHandle`** — `TryWith` lowers to `Expr::error()`. Implement proper inline handler lowering. *Resolves stub: TryWith lowering (hir_lower_expr.blood).*

- [ ] **HR-6: Macro expansion HIR nodes** — No `MacroExpansion`, `VecLiteral`, `VecRepeat`, `Assert`, `Dbg` HIR nodes. Implement macro desugaring during HIR lowering. *Resolves stub: MacroDef (hir_item.blood).*

### Medium Severity

- [ ] **HR-1: `TypeKind::Closure`** — No dedicated closure type; closures use function types. Add closure type with captured environment information.

- [ ] **HR-2: `TypeKind::Range`** — No built-in range type. Add range type to HIR type system.

- [ ] **HR-3: `TypeKind::DynTrait`** — No trait object types. Add trait object type to HIR.

- [ ] **HR-7: SliceLen / VecLen intrinsics** — No compiler intrinsics for `.len()`. Add dedicated HIR nodes for length intrinsics.

- [ ] **HR-8: `ExprKind::ArrayToSlice`** — No array-to-slice coercion node. Add coercion during HIR lowering.

- [ ] **HR-9: `ExprKind::MethodFamily`** — No multiple dispatch. Add method family resolution support.

- [ ] **HR-12: Const generic array sizes** — Array size is `u64` not `ConstValue`. Use `ConstValue` to support const generic parameters.

- [ ] **HR-13: Module re-exports** — No `pub use` re-export tracking in `ModuleDef`. Add `reexports` field.

- [ ] **HR-14: Multiple dispatch resolution** — No `Binding::Methods` or `MethodRegistry`. Add multiple-binding support in resolver.

### Low Severity

- [ ] **HR-10: `ExprKind::Let`** — No let-in-expression (`let-else`). Add let expression to HIR.

- [ ] **HR-11: `ExprKind::Borrow` / `Deref`** — Uses `AddrOf` only. Add explicit borrow and deref HIR nodes for clarity.

- [ ] **HR-15: Unified `Res` enum** — No single resolution result type. Add `Res` enum consolidating `Def`, `Local`, `PrimTy`, `Err`.

- [ ] **HR-16: DefKind variants** — Missing `AssocFn`, `Closure`, `Local`, `Field`. Add missing variants.

- [ ] **HR-17: Visibility in DefInfo** — Not tracked during resolution. Add `visibility` field.

---

## Phase 4: Type Checking

Depends on HIR being correct and complete.

### High Severity

- [ ] **TC-1: Expected type propagation** — `check_expr` doesn't thread expected type into branches/blocks. Propagate expected types for better inference.

- [ ] **TC-2: Numeric literal defaulting** — Unsuffixed `42` -> `i32`, `3.14` -> `f64` not implemented. Add default type assignment for unsuffixed literals.

- [ ] **TC-3: Trait bound verification** — `T: Display` checking absent. Implement trait bound checking during type checking. *Resolves stub: TraitInfo / TraitImplInfo (typeck_info.blood).*

- [ ] **TC-4: Builtin trait implementations** — No `Copy`/`Clone`/`Sized`/etc. checking. Register and check builtin trait impls.

- [ ] **TC-6: Auto-ref/auto-deref in method resolution** — Only strips references, never adds `&`/`&mut`. Implement auto-ref insertion during method lookup. *Resolves stub: Coercion::Deref (typeck_types.blood:153-168).*

### Medium Severity

- [ ] **TC-5: Coherence checking** — No overlapping impl detection. Add coherence rules.

- [ ] **TC-7: Multiple dispatch** — No specificity ordering or ambiguity detection. Implement dispatch resolution.

- [ ] **TC-8: Where clause bounds** — Not tracked or checked. Wire where clauses into bound checking.

- [ ] **TC-9: Type parameter bounds at call sites** — Bounds not checked when calling generics. Verify bounds at instantiation.

- [ ] **TC-10: Const generic parameters** — Not supported in type checker. Add const generic type support.

- [ ] **TC-11: Lifetime parameters** — Not supported. Add lifetime parameter tracking.

- [ ] **TC-12: Type alias resolution** — Not supported in type checker. Resolve type aliases during checking.

- [ ] **TC-13: Closure-to-function type unification** — Not handled. Add coercion. *Resolves stub: Coercion::ClosureToFnPtr (typeck_types.blood:153-168).*

- [ ] **TC-16: Const item path lookup** — Cannot reference named constants in array sizes. Resolve const paths in const contexts.

- [ ] **TC-18: Linearity checking** — Linear/affine type enforcement absent. Add ownership mode tracking.

- [ ] **TC-19: FFI validation** — No FFI-safe type checking. Validate types at FFI boundaries.

### Low Severity

- [ ] **TC-14: Unit/empty-tuple equivalence** — `Primitive(Unit)` == `Tuple([])` not checked. Add equivalence in unification.

- [ ] **TC-15: Unreachable match arm detection** — Not implemented. Add exhaustiveness/reachability analysis.

- [ ] **TC-17: If/else and block evaluation in const context** — Not supported. Extend const evaluator.

---

## Phase 5: MIR Generation

Depends on type checker producing correct types and resolving all expressions.

### High Severity

- [ ] **MR-1: Generational pointer statements** — `IncrementGeneration`, `CaptureSnapshot`, `ValidateGeneration` absent. Add MIR statement kinds for generational safety.

- [ ] **MR-2: Generational pointer rvalues** — `ReadGeneration`, `MakeGenPtr`, `NullCheck` absent. Add MIR rvalue kinds.

- [ ] **MR-4: StaleReference terminator** — No stale reference trap. Add terminator for generation check failures.

- [ ] **MR-10: Escape analysis** — No `EscapeAnalyzer`. Implement worklist-based escape state propagation (`NoEscape`, `ArgEscape`, `GlobalEscape`).

- [ ] **MR-12: Generation snapshots** — No `SnapshotAnalyzer`. Implement snapshot infrastructure for generational references.

- [ ] **MR-13: 128-bit generational pointer types** — `BloodPtr`, `PtrMetadata`, `MemoryTier` absent. Define pointer metadata types.

### Medium Severity

- [ ] **MR-3: DropAndReplace terminator** — Not present. Add combined drop-and-replace terminator.

- [ ] **MR-8: PlaceBase::Static** — Places only support locals, not statics. Add `Static(DefId)` to `PlaceBase`.

- [ ] **MR-9: MIR Visitor trait** — No traversal/analysis framework. Implement visitor pattern for MIR analysis passes.

- [ ] **MR-11: Closure environment analysis** — No `ClosureAnalyzer`. Implement capture analysis with inline threshold.

- [ ] **MR-15: Match guard evaluation** — Guard field exists in `MatchArm` but lowering does not evaluate guards. Emit guard condition block with fallthrough to next arm on failure. *Resolves stub: Or-pattern matching (mir_lower_pattern.blood:304-307), Range pattern matching (:309-312), Slice pattern matching (:299-302).*

### Low Severity

- [ ] **MR-5: VecLen rvalue** — Not present. Add `VecLen(Place)` rvalue.

- [ ] **MR-6: StringIndex rvalue** — Not present. Add `StringIndex { string, index }` rvalue.

- [ ] **MR-7: BinOp::Offset** — No pointer arithmetic. Add `Offset` binary operation.

- [ ] **MR-14: Handler deduplication** — No `HandlerFingerprint`. Add fingerprint-based handler dedup.

- [ ] **MR-TRY: Try expression** — `lower_try_expr()` delegates to `lower_expr()` with no error propagation. Implement proper `?` operator desugaring (match on `Result`/`Option`, propagate error). *Resolves stub: mir_lower_expr.blood:1699-1709.*

---

## Phase 6: Codegen & Runtime

Depends on MIR being correct and complete.

### High Severity

- [ ] **CG-4: Closure function generation** — Closures are data-only aggregates; no function pointer emitted. Generate closure function taking env as first arg, create `{ fn_ptr, env_ptr }` fat pointer. *Resolves stub: Closure codegen.*

- [ ] **CG-5: Full evidence-passing effects** — Simplified push/pop/perform stubs. Implement evidence create/destroy/push/pop/get, tail-resumptive optimization, handler state management.

- [ ] **CG-3: Generation check emission** — `blood_validate_generation` declared but never called on dereference. Emit generation validation on every region-tier pointer deref.

- [ ] **CG-2: Escape analysis + memory tier assignment** — All locals stack-allocated; region/persistent paths dead code. Wire escape analysis results to drive alloca vs region vs persistent allocation.

- [ ] **CG-7: Generic monomorphization** — All type params mapped to `ptr`. Implement type-specialized code generation per concrete instantiation.

- [ ] **CG-6: Dynamic dispatch / VTables** — Functions declared but never called. Emit vtable construction, type tag registration, and indirect dispatch calls.

- [ ] **CG-12: Runtime function declarations** — ~43 missing I/O, assertion, evidence, fiber, scheduler, lifecycle functions. Declare all runtime functions matching bootstrap.

- [ ] **CG-1: LLVM optimization passes** — No in-process pass manager. Evaluate whether external `llc -O2` is sufficient or if in-process passes are needed for correctness.

### Medium Severity

- [ ] **CG-9: Const/static item compilation** — Not emitted as LLVM globals. Generate global constants and static variables.

- [ ] **CG-10: Fiber/continuation support** — No fiber runtime functions. Declare and wire fiber create/suspend/resume.

- [ ] **CG-11: Runtime lifecycle** — No `blood_runtime_init`/`shutdown`. Add lifecycle calls in generated `main`.

- [ ] **CG-8: Incremental compilation** — Full recompilation every build. Add per-definition compilation with content-addressed caching.

- [ ] **CG-DROP: Drop implementation** — Only handles region-allocated refs; no destructor/drop glue. Implement field-by-field recursive drop, drop flags. *Resolves stub: codegen_stmt.blood:79-114.*

- [ ] **CG-ASSERT: Assert terminator** — Hardcoded message, no source location or values. Use `blood_assert`/`blood_assert_eq_*` with context. *Resolves stub: codegen_term.blood:~280-310.*

- [ ] **CG-DEINIT: Deinit statement** — No-op comment only. Zero memory or mark uninitialized. *Resolves stub: codegen_stmt.blood:115-118.*

### Low Severity

- [ ] **CG-BUILTIN: Builtin method runtime** — `String::len`, `Vec::push`, etc. declared but no runtime provides them. Either implement runtime library or inline these operations. *Resolves stub: codegen.blood intrinsics.*

- [ ] **CG-SNAP: Snapshot functions** — `blood_snapshot_create`/`restore` declared but never called. Wire to generation snapshot infrastructure when MR-12 is complete. *Resolves stub: codegen.blood intrinsics.*

- [ ] **CG-DISP: Dispatch function usage** — `blood_dispatch_register`/`lookup` declared but never used. Wire to dynamic dispatch when CG-6 is complete. *Resolves stub: codegen.blood intrinsics.*

---

## Progress Tracker

| Phase | Total | Done | Remaining |
|-------|-------|------|-----------|
| 1. Incorrect Implementations | 15 | 8 | 7 |
| 2. Parser Completeness | 16 | 0 | 16 |
| 3. HIR & Name Resolution | 15 | 0 | 15 |
| 4. Type Checking | 17 | 0 | 17 |
| 5. MIR Generation | 16 | 0 | 16 |
| 6. Codegen & Runtime | 17 | 0 | 17 |
| **Total** | **96** | **8** | **88** |
