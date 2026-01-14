# Blood Compiler: Action Items Checklist (v2)

**Generated**: 2026-01-14
**Predecessor**: ACTION_ITEMS_V1_COMPLETED.md (118 items completed, 1 partial)
**Status**: Active tracking document

---

## Priority Legend

| Priority | Meaning | Timeline |
|----------|---------|----------|
| **P0** | Critical - Blocks fundamental use cases | Immediate |
| **P1** | High - Required for beta readiness | Near-term |
| **P2** | Medium - Required for 1.0 release | Mid-term |
| **P3** | Low - Quality of life improvements | Long-term |

---

## 1. Pointer Optimization Implementation [P1]

Derived from PTR-002 and PTR-004 investigations in v1.

### 1.1 Stack Pointer Verification [P1]

PTR-002 Phase 1: Verify stack-promotable values use Tier 0.

- [x] **PTR-IMPL-001**: Verify codegen emits Tier 0 (64-bit thin pointer) for `EscapeState::NoEscape` values
  - ✅ VERIFIED: `codegen/mir_codegen/mod.rs:236-263` correctly allocates based on escape tier
  - ✅ `MemoryTier::Stack` → `build_alloca` (LLVM stack allocation, thin pointer)
  - ✅ `MemoryTier::Region` → `allocate_with_blood_alloc` (heap with generation tracking)
  - ✅ `get_local_tier()` correctly calls `EscapeResults::recommended_tier()`
  - ✅ Generation checks skipped for NoEscape locals (`place.rs:102-110`)
  - ✅ Additional finding: Primitive/Copy types are ALWAYS stack-promotable regardless of escape state
    (correct semantics: they are copied by value, not referenced)
- [x] **PTR-IMPL-002**: Add tests verifying Tier 0 allocation in generated code
  - ✅ Added 6 tests in `codegen/mir_codegen/tests.rs`:
    - `test_escape_analysis_affects_allocation_tier` - verifies escape→tier mapping
    - `test_stack_promotable_excludes_effect_captured` - non-primitive effect capture
    - `test_get_local_tier_respects_escape_results` - tier selection logic
    - `test_codegen_uses_escape_results_for_allocation` - integration test
    - `test_generation_check_skipped_for_noescape` - gen check optimization
  - ✅ Fixed 5 existing tests in `mir/escape.rs` to use non-primitive types
    (tests were incorrectly using i32 which is always stack-promotable)
- [x] **PTR-IMPL-003**: Profile before/after to confirm overhead reduction
  - ✅ EXCEEDED EXPECTATIONS: Blood now matches C performance exactly (1.0x ratio)
  - Previous: Blood was ~1000x slower than C
  - After escape analysis + release mode: Blood matches C exactly
  - Benchmark results:
    | Benchmark | Blood (--release) | C (-O3) | Ratio |
    |-----------|-------------------|---------|-------|
    | Pure computation (10M iters) | 9ms | 9ms | 1.0x |
    | Reference access (10M iters) | 5ms | 5ms | 1.0x |
  - Key changes: Primitive types always stack-promoted, --release enables inlining

### 1.2 Persistent Tier Thin Pointers [P2]

PTR-002 Phase 2: Thin pointers for persistent tier.

- [ ] **PTR-IMPL-004**: Implement `PersistentPtr` as 64-bit thin pointer with RC header
  - Current persistent tier exists but uses 128-bit pointers
  - Can skip generation check for `PERSISTENT_MARKER (0xFFFFFFFF)`
  - Consider using bit 63 as PERSISTENT flag
- [ ] **PTR-IMPL-005**: Add `PtrKind { Fat, Stack, Persistent }` to MIR representation
  - Track pointer kind through MIR for codegen optimization
- [ ] **PTR-IMPL-006**: Update codegen to emit appropriate pointer type based on `PtrKind`

### 1.3 FFI Pointer Optimization [P3]

PTR-002 Phase 4: FFI marshaling optimization.

- [ ] **PTR-IMPL-007**: Keep 64-bit throughout FFI data structures
  - Only convert to BloodPtr when re-exported to Blood code
  - Avoid unnecessary 128-bit wrapping at FFI boundary

---

## 2. Effect System Optimizations [P2]

Derived from EFF-001 audit findings. EFF-002 (caching) already implemented.

### 2.1 Static Evidence Optimization [P2] — HIGH IMPACT

- [ ] **EFF-OPT-001**: Implement compile-time evidence pre-allocation
  - When handlers are compile-time known, pre-allocate evidence vector
  - Detect constant handler installations (no runtime variation)
  - Emit static evidence data in generated code
- [ ] **EFF-OPT-002**: Add MIR pass to identify static handler patterns
  - Analyze `handle` expressions for constant handler references
  - Mark evidence as "static" when all handlers are known

### 2.2 Inline Small Evidence [P2] — MEDIUM IMPACT

- [ ] **EFF-OPT-003**: Pass 1-2 handlers inline instead of via pointer
  - Most effect usage has shallow handler stacks
  - Inline single-handler case to avoid indirection
- [ ] **EFF-OPT-004**: Specialize evidence passing for common patterns
  - State effect: single handler, inline
  - Reader/Writer: single handler, inline
  - Multi-effect: use pointer to array

### 2.3 Stack-Allocated Evidence [P2] — MEDIUM IMPACT

- [ ] **EFF-OPT-005**: Use alloca for lexically-scoped handlers
  - When handler scope is lexically bounded (no escape)
  - Avoid heap allocation, use stack frame
- [ ] **EFF-OPT-006**: Add escape analysis for handler evidence
  - Track whether evidence escapes handler scope
  - Mark for stack allocation when safe

### 2.4 Handler Deduplication [P3] — LOW IMPACT

- [ ] **EFF-OPT-007**: Detect identical handler patterns across call sites
  - Content-addressed hashing of handler configurations
  - Share evidence vectors for identical patterns
  - Building on EFF-002 caching infrastructure

---

## 3. Closure Optimization [P1]

CLOS-003 from v1 is partially complete. Infrastructure exists, full optimization deferred.

### 3.1 Inline Small Closures [P1]

- [ ] **CLOS-IMPL-001**: Modify closure ABI to inline small environments
  - Current: `{ fn_ptr: i8*, env_ptr: i8* }` with separate alloca
  - Optimal: `{ fn_ptr: i8*, env: [captures inline] }` for ≤16 bytes
  - Threshold identified by `ClosureAnalyzer` (CLOS-001)
- [ ] **CLOS-IMPL-002**: Update `codegen/mir_codegen/rvalue.rs` for inline captures
  - Emit inline capture storage instead of separate allocation
  - Handle both inline and pointer-based based on environment size
- [ ] **CLOS-IMPL-003**: Update `codegen/mir_codegen/terminator.rs` for inline call
  - Read captures from inline storage on call
- [ ] **CLOS-IMPL-004**: Update `mir/lowering/closure.rs` for capture access
  - Generate correct field projections for inline vs pointer access

---

## 4. Self-Hosting Progress [P1]

SELF-001-003 complete. Continue toward full self-hosting.

### 4.1 Blood Parser Verification [P2]

- [ ] **SELF-VERIFY-001**: Verify `blood-std/std/compiler/parser.blood` manually
  - Review 2992-line parser implementation for correctness
  - Cannot run until self-hosting progresses
  - Document any identified issues for fixing
- [ ] **SELF-VERIFY-002**: Create test suite in Blood for parser
  - Will execute once Blood can run Blood code

### 4.2 Type Checker Implementation [P1]

- [ ] **SELF-004**: Implement type checker in Blood (`blood-std/std/compiler/typeck.blood`)
  - Required components:
    - Type representation (`Type`, `TypeVar`, `TypeScheme`)
    - Unification algorithm
    - Effect row types and effect polymorphism
    - Trait constraint solving
    - Pattern exhaustiveness checking
  - Estimated: ~3000-4000 lines based on Rust implementation
- [ ] **SELF-004a**: Implement core type representation
- [ ] **SELF-004b**: Implement unification with effect rows
- [ ] **SELF-004c**: Implement trait resolution
- [ ] **SELF-004d**: Implement exhaustiveness checking
- [ ] **SELF-004e**: Implement type inference for expressions

### 4.3 Bootstrap [P1]

- [ ] **SELF-005**: Bootstrap - compile Blood compiler with Blood compiler
  - Requires: lexer (done), parser (done), type checker (SELF-004), codegen
  - Multi-stage bootstrap:
    1. Rust compiler compiles Blood compiler written in Blood
    2. Blood compiler (stage 1) compiles itself
    3. Blood compiler (stage 2) compiles itself
    4. Verify stage 2 == stage 3 (fixed point)

---

## 5. Formal Verification [P3]

Mechanized proofs for core safety properties.

### 5.1 Type Soundness [P3]

- [ ] **FORMAL-001**: Formalize type soundness proof in Coq/Agda
  - Progress: well-typed terms step or are values
  - Preservation: stepping preserves types
  - Model Blood's core calculus with effects

### 5.2 Effect Safety [P3]

- [ ] **FORMAL-002**: Formalize effect safety theorem
  - Effect rows track all possible effects
  - Handler provides evidence for all performed effects
  - Pure functions perform no effects

### 5.3 Memory Safety [P3]

- [ ] **FORMAL-003**: Formalize generation snapshot correctness
  - Snapshot captures current generation state
  - Validation detects any invalidated references
  - No use-after-free when validation passes

### 5.4 Invariant Verification [P3]

- [ ] **FORMAL-004**: Mechanically verify key safety invariants
  - Affine values used at most once
  - Linear values used exactly once
  - Frozen regions immutable

---

## 6. MIR Lowering Deduplication [P2]

DUP-001 audit found 70-85% duplication between function.rs and closure.rs.
DUP-002 extracted some utilities, but more remains.

### 6.1 Expression Lowering Trait [P2]

- [ ] **DUP-IMPL-001**: Extract `ExprLowering` trait for shared expression lowering
  - 35+ methods duplicated between `function.rs` and `closure.rs`
  - Trait with default implementations for common cases
  - Override only for closure-specific capture handling
- [ ] **DUP-IMPL-002**: Unify loop context handling
  - function.rs: `Vec<LoopContext>`
  - closure.rs: `HashMap<LoopId, ...>`
  - Create shared abstraction

### 6.2 Pattern Lowering Extraction [P2]

- [ ] **DUP-IMPL-003**: Move pattern matching logic to `util.rs`
  - `test_pattern`, `test_pattern_tuple`, `test_pattern_fields`
  - `test_pattern_struct_fields`, `test_pattern_or`, `test_pattern_slice`
  - `bind_pattern`
  - Already have `lower_literal_to_constant` and `is_irrefutable_pattern`

---

## 7. Closure Chain Optimization [P2]

Identified in PERF-007 hot path profiling.

- [ ] **PERF-IMPL-001**: Optimize closure chain escape analysis
  - Current: O(n²) behavior for closure chains (50→83µs, 100→307µs, 500→6.3ms)
  - Solution: Use worklist algorithm instead of iterative propagation
  - Location: `mir/escape.rs`

---

## Summary Statistics

**Status as of 2026-01-14:**

| Category | P0 | P1 | P2 | P3 | Total | Done |
|----------|----|----|----|----|-------|------|
| Pointer Optimization | 0 | 0 | 3 | 1 | **4** | 3 |
| Effect Optimizations | 0 | 0 | 6 | 1 | **7** | 0 |
| Closure Optimization | 0 | 4 | 0 | 0 | **4** | 0 |
| Self-Hosting | 0 | 7 | 2 | 0 | **9** | 0 |
| Formal Verification | 0 | 0 | 0 | 4 | **4** | 0 |
| MIR Deduplication | 0 | 0 | 3 | 0 | **3** | 0 |
| Performance Optimization | 0 | 0 | 1 | 0 | **1** | 0 |
| **Total** | **0** | **11** | **15** | **6** | **32** | **3** |

**Recently Completed (Section 1.1 - Stack Pointer Verification):**
- PTR-IMPL-001: Verified codegen emits Tier 0 for NoEscape values
- PTR-IMPL-002: Added tests verifying stack vs region allocation
- PTR-IMPL-003: Profiled performance - Blood now matches C exactly (1.0x)

**Carried Forward from v1:**
- CLOS-003 (partial) → CLOS-IMPL-001-004
- PTR-002 phases → PTR-IMPL-001-003
- PTR-004 phases → PTR-IMPL-004-006
- EFF-001 findings → EFF-OPT-001-007
- SELF-004, SELF-005 → SELF-004, SELF-004a-e, SELF-005
- FORMAL-001-004 → unchanged
- DUP-001 recommendations → DUP-IMPL-001-003
- PERF-007 findings → PERF-IMPL-001

**Items Written but Unverifiable:**
- SELF-003 (Blood parser in Blood) - Cannot be tested until self-hosting progresses

---

## Recommended Execution Order

### Phase 1: Low-Hanging Fruit (Verification & Optimization)
1. PTR-IMPL-001-003: Verify stack pointers work (may already work, just needs verification)
2. CLOS-IMPL-001-004: Inline small closures (infrastructure exists)
3. PERF-IMPL-001: Closure chain worklist optimization

### Phase 2: Self-Hosting Progress
1. SELF-004a-e: Type checker in Blood (incremental)
2. SELF-VERIFY-001-002: Parser verification
3. SELF-005: Bootstrap attempt

### Phase 3: Effect System Polish
1. EFF-OPT-001-002: Static evidence (highest impact)
2. EFF-OPT-003-004: Inline small evidence
3. EFF-OPT-005-006: Stack-allocated evidence

### Phase 4: Code Quality
1. DUP-IMPL-001-003: MIR lowering deduplication
2. PTR-IMPL-004-006: Persistent tier thin pointers

### Phase 5: Long-term
1. FORMAL-001-004: Formal verification (when resources permit)
2. PTR-IMPL-007: FFI pointer optimization
3. EFF-OPT-007: Handler deduplication

---

## Archive Reference

For completed work (114 items), see: `ACTION_ITEMS_V1_COMPLETED.md`

Key completions include:
- All P0 items (persistent memory tier, FFI, integration tests)
- All standard library core types
- All user documentation
- Performance benchmarks and validation
- Developer tooling (LSP, formatter, REPL, VS Code extension)
- Package manager foundation
- Self-hosting lexer and parser (in Blood)
- Error handling audit
- Fuzzing and property testing infrastructure

---

*This document should be reviewed and updated as items are completed.*
