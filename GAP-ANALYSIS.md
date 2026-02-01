 Technical Gap Analysis: Self-Hosted vs Bootstrap Compiler

  1. Missing Features

  Features present in the bootstrap compiler but absent from the self-hosted compiler.

  1.1 Lexer & Parser
  ID: LP-1
  Feature: Bridge declaration parsing
  Severity: High
  Details: AST BridgeDecl exists but parse_declaration() has no arm for Bridge token
  ────────────────────────────────────────
  ID: LP-2
  Feature: Macro declaration parsing
  Severity: High
  Details: AST MacroDecl exists but parse_declaration() has no arm for Macro token
  ────────────────────────────────────────
  ID: LP-3
  Feature: Macro call expressions
  Severity: High
  Details: ExprKind::MacroCall in AST but no parsing for format!, vec!, println!, etc.
  ────────────────────────────────────────
  ID: LP-4
  Feature: if let expression parsing
  Severity: High
  Details: AST has IfLet but parser never generates it
  ────────────────────────────────────────
  ID: LP-5
  Feature: while let expression parsing
  Severity: High
  Details: AST has WhileLet but parser never generates it
  ────────────────────────────────────────
  ID: LP-6
  Feature: Or-patterns
  Severity: High
  Details: PatternKind::Or in AST but `
  ────────────────────────────────────────
  ID: LP-7
  Feature: Compound assignment (+=, -=)
  Severity: Medium
  Details: Tokens exist but not in precedence table
  ────────────────────────────────────────
  ID: LP-8
  Feature: Range expressions
  Severity: Medium
  Details: ../..= not in infix precedence for expressions
  ────────────────────────────────────────
  ID: LP-9
  Feature: move closures
  Severity: Medium
  Details: Always is_move: false
  ────────────────────────────────────────
  ID: LP-10
  Feature: Loop labels
  Severity: Low
  Details: Always None for Loop, While, For, Break, Continue
  ────────────────────────────────────────
  ID: LP-11
  Feature: Function qualifiers
  Severity: Medium
  Details: const fn, async fn, unsafe fn not parsed
  ────────────────────────────────────────
  ID: LP-12
  Feature: Negative literal patterns
  Severity: Medium
  Details: -42 not handled in pattern position
  ────────────────────────────────────────
  ID: LP-13
  Feature: Range patterns
  Severity: Medium
  Details: 0..=9 not parsed
  ────────────────────────────────────────
  ID: LP-14
  Feature: Unclosed block comment error
  Severity: Medium
  Details: Lexer silently eats source to EOF
  ────────────────────────────────────────
  ID: LP-15
  Feature: \x## and \u{####} string escapes
  Severity: Low
  Details: Not implemented in lexer
  ────────────────────────────────────────
  ID: LP-16
  Feature: Doc comment to attribute conversion
  Severity: Low
  Details: Comments skipped rather than becoming #[doc = "..."]
  1.2 HIR & Name Resolution
  ID: HR-1
  Feature: TypeKind::Closure
  Severity: Medium
  Details: No dedicated closure type; closures use function types
  ────────────────────────────────────────
  ID: HR-2
  Feature: TypeKind::Range
  Severity: Medium
  Details: No built-in range type
  ────────────────────────────────────────
  ID: HR-3
  Feature: TypeKind::DynTrait
  Severity: Medium
  Details: No trait object types
  ────────────────────────────────────────
  ID: HR-4
  Feature: ExprKind::Region
  Severity: High
  Details: Region blocks lowered as plain blocks, losing region semantics
  ────────────────────────────────────────
  ID: HR-5
  Feature: ExprKind::InlineHandle
  Severity: High
  Details: TryWith lowers to Expr::error()
  ────────────────────────────────────────
  ID: HR-6
  Feature: ExprKind::MacroExpansion / VecLiteral / Assert / Dbg
  Severity: High
  Details: No macro expansion HIR nodes
  ────────────────────────────────────────
  ID: HR-7
  Feature: ExprKind::SliceLen / VecLen
  Severity: Medium
  Details: No compiler intrinsics for .len()
  ────────────────────────────────────────
  ID: HR-8
  Feature: ExprKind::ArrayToSlice
  Severity: Medium
  Details: No array-to-slice coercion node
  ────────────────────────────────────────
  ID: HR-9
  Feature: ExprKind::MethodFamily
  Severity: Medium
  Details: No multiple dispatch
  ────────────────────────────────────────
  ID: HR-10
  Feature: ExprKind::Let
  Severity: Low
  Details: No let-in-expression (let-else)
  ────────────────────────────────────────
  ID: HR-11
  Feature: ExprKind::Borrow / Deref
  Severity: Low
  Details: Uses AddrOf only
  ────────────────────────────────────────
  ID: HR-12
  Feature: Const generic array sizes
  Severity: Medium
  Details: Array size is u64 not ConstValue
  ────────────────────────────────────────
  ID: HR-13
  Feature: Module re-exports
  Severity: Medium
  Details: No pub use re-export tracking
  ────────────────────────────────────────
  ID: HR-14
  Feature: Multiple dispatch resolution
  Severity: Medium
  Details: No Binding::Methods or MethodRegistry
  ────────────────────────────────────────
  ID: HR-15
  Feature: Unified Res enum
  Severity: Low
  Details: No single resolution result type
  ────────────────────────────────────────
  ID: HR-16
  Feature: DefKind::AssocFn, Closure, Local, Field
  Severity: Low
  Details: Missing DefKind variants
  ────────────────────────────────────────
  ID: HR-17
  Feature: Visibility in DefInfo
  Severity: Low
  Details: Not tracked during resolution
  1.3 Type Checking
  ID: TC-1
  Feature: Expected type propagation
  Severity: High
  Details: check_expr doesn't thread expected type into branches/blocks
  ────────────────────────────────────────
  ID: TC-2
  Feature: Numeric literal defaulting
  Severity: High
  Details: Unsuffixed 42→i32, 3.14→f64 not implemented
  ────────────────────────────────────────
  ID: TC-3
  Feature: Trait bound verification
  Severity: High
  Details: T: Display checking absent
  ────────────────────────────────────────
  ID: TC-4
  Feature: Builtin trait implementations
  Severity: High
  Details: No Copy/Clone/Sized/etc. checking
  ────────────────────────────────────────
  ID: TC-5
  Feature: Coherence checking
  Severity: Medium
  Details: No overlapping impl detection
  ────────────────────────────────────────
  ID: TC-6
  Feature: Auto-ref/auto-deref in method resolution
  Severity: High
  Details: Only strips references, never adds &/&mut
  ────────────────────────────────────────
  ID: TC-7
  Feature: Multiple dispatch
  Severity: Medium
  Details: No specificity ordering or ambiguity detection
  ────────────────────────────────────────
  ID: TC-8
  Feature: Where clause bounds
  Severity: Medium
  Details: Not tracked or checked
  ────────────────────────────────────────
  ID: TC-9
  Feature: Type parameter bounds at call sites
  Severity: Medium
  Details: Bounds not checked when calling generics
  ────────────────────────────────────────
  ID: TC-10
  Feature: Const generic parameters
  Severity: Medium
  Details: Not supported
  ────────────────────────────────────────
  ID: TC-11
  Feature: Lifetime parameters
  Severity: Medium
  Details: Not supported
  ────────────────────────────────────────
  ID: TC-12
  Feature: Type alias resolution
  Severity: Medium
  Details: Not supported in type checker
  ────────────────────────────────────────
  ID: TC-13
  Feature: Closure-to-function type unification
  Severity: Medium
  Details: Not handled
  ────────────────────────────────────────
  ID: TC-14
  Feature: Unit/empty-tuple equivalence
  Severity: Low
  Details: Primitive(Unit) == Tuple([]) not checked
  ────────────────────────────────────────
  ID: TC-15
  Feature: Unreachable match arm detection
  Severity: Low
  Details: Not implemented
  ────────────────────────────────────────
  ID: TC-16
  Feature: Const item path lookup
  Severity: Medium
  Details: Cannot reference named constants in array sizes
  ────────────────────────────────────────
  ID: TC-17
  Feature: If/else and block evaluation in const context
  Severity: Low
  Details: Not supported
  ────────────────────────────────────────
  ID: TC-18
  Feature: Linearity checking
  Severity: Medium
  Details: Linear/affine type enforcement absent
  ────────────────────────────────────────
  ID: TC-19
  Feature: FFI validation
  Severity: Medium
  Details: No FFI-safe type checking
  1.4 MIR Generation
  ID: MR-1
  Feature: Generational pointer statements
  Severity: High
  Details: IncrementGeneration, CaptureSnapshot, ValidateGeneration absent
  ────────────────────────────────────────
  ID: MR-2
  Feature: Generational pointer rvalues
  Severity: High
  Details: ReadGeneration, MakeGenPtr, NullCheck absent
  ────────────────────────────────────────
  ID: MR-3
  Feature: DropAndReplace terminator
  Severity: Medium
  Details: Not present
  ────────────────────────────────────────
  ID: MR-4
  Feature: StaleReference terminator
  Severity: High
  Details: No stale reference trap
  ────────────────────────────────────────
  ID: MR-5
  Feature: VecLen rvalue
  Severity: Low
  Details: Not present
  ────────────────────────────────────────
  ID: MR-6
  Feature: StringIndex rvalue
  Severity: Low
  Details: Not present
  ────────────────────────────────────────
  ID: MR-7
  Feature: BinOp::Offset
  Severity: Low
  Details: No pointer arithmetic
  ────────────────────────────────────────
  ID: MR-8
  Feature: PlaceBase::Static
  Severity: Medium
  Details: Places only support locals, not statics
  ────────────────────────────────────────
  ID: MR-9
  Feature: MIR Visitor trait
  Severity: Medium
  Details: No traversal/analysis framework
  ────────────────────────────────────────
  ID: MR-10
  Feature: Escape analysis
  Severity: High
  Details: No EscapeAnalyzer
  ────────────────────────────────────────
  ID: MR-11
  Feature: Closure environment analysis
  Severity: Medium
  Details: No ClosureAnalyzer
  ────────────────────────────────────────
  ID: MR-12
  Feature: Generation snapshots
  Severity: High
  Details: No SnapshotAnalyzer
  ────────────────────────────────────────
  ID: MR-13
  Feature: 128-bit generational pointer types
  Severity: High
  Details: BloodPtr, PtrMetadata, MemoryTier absent
  ────────────────────────────────────────
  ID: MR-14
  Feature: Handler deduplication
  Severity: Low
  Details: No HandlerFingerprint
  ────────────────────────────────────────
  ID: MR-15
  Feature: Match guard evaluation
  Severity: Medium
  Details: Guard field exists but not lowered
  1.5 Codegen & Runtime
  ID: CG-1
  Feature: LLVM optimization passes
  Severity: High
  Details: No in-process pass manager; relies on external llc-14
  ────────────────────────────────────────
  ID: CG-2
  Feature: Escape analysis + memory tier assignment
  Severity: High
  Details: All locals stack-allocated; region/persistent paths are dead code
  ────────────────────────────────────────
  ID: CG-3
  Feature: Generation check emission
  Severity: High
  Details: blood_validate_generation declared but never called on dereference
  ────────────────────────────────────────
  ID: CG-4
  Feature: Closure function generation
  Severity: High
  Details: Closures are data-only aggregates; no function pointer emitted
  ────────────────────────────────────────
  ID: CG-5
  Feature: Full evidence-passing effects
  Severity: High
  Details: Simplified push/pop/perform; no evidence, no tail-resumptive optimization
  ────────────────────────────────────────
  ID: CG-6
  Feature: Dynamic dispatch / VTables
  Severity: High
  Details: Functions declared but never called
  ────────────────────────────────────────
  ID: CG-7
  Feature: Generic monomorphization
  Severity: High
  Details: All type params mapped to ptr
  ────────────────────────────────────────
  ID: CG-8
  Feature: Incremental compilation
  Severity: Medium
  Details: Full recompilation every build
  ────────────────────────────────────────
  ID: CG-9
  Feature: Const/static item compilation
  Severity: Medium
  Details: Not emitted as globals
  ────────────────────────────────────────
  ID: CG-10
  Feature: Fiber/continuation support
  Severity: Medium
  Details: No fiber runtime functions
  ────────────────────────────────────────
  ID: CG-11
  Feature: Runtime lifecycle
  Severity: Medium
  Details: No blood_runtime_init/shutdown
  ────────────────────────────────────────
  ID: CG-12
  Feature: ~43 runtime function declarations
  Severity: High
  Details: Missing I/O, assertion, evidence, fiber, scheduler, lifecycle functions
  ---
  2. Incorrect Implementations

  Features present in both compilers but with different (potentially wrong) behavior.
  ID: IC-1
  Feature: For-loop desugaring
  Details: Bootstrap: iterator-based (IntoIterator::into_iter() → next() → Option match).
    Self-hosted: index-based (let i = start; while i < end). Only integer ranges work.
  ────────────────────────────────────────
  ID: IC-2
  Feature: Literal pattern values
  Details: Self-hosted parse_pattern_kind creates placeholder values (val: 0, bits: 0, empty
    strings) instead of parsing actual source values
  ────────────────────────────────────────
  ID: IC-3
  Feature: Self parameter parsing
  Details: Self-hosted checks is_ref_self via parser.check(TokenKind::And) but does not consume
  the
     & token, so &self and &mut self are not correctly distinguished
  ────────────────────────────────────────
  ID: IC-4
  Feature: Resume expression
  Details: Bootstrap: Resume { value: Option<Operand> } (optional). Self-hosted: Resume { value:
    Operand } (required). Unit-resume unsupported.
  ────────────────────────────────────────
  ID: IC-5
  Feature: Break values lost
  Details: Self-hosted lower_break_expr() evaluates break value with destination_ignore() instead
    of storing to loop result place
  ────────────────────────────────────────
  ID: IC-6
  Feature: O(n) substitution lookup
  Details: Self-hosted uses linear Vec scan instead of HashMap for type variable substitutions
  ────────────────────────────────────────
  ID: IC-7
  Feature: Forall unification
  Details: Self-hosted directly unifies bodies without alpha-renaming; bootstrap creates fresh
    variables
  ────────────────────────────────────────
  ID: IC-8
  Feature: Function call ABI
  Details: Self-hosted passes all args as i64 via ptrtoint; incorrect for floats, struct-by-value,

    multi-arg calling conventions
  ────────────────────────────────────────
  ID: IC-9
  Feature: String literal representation
  Details: Self-hosted: raw ptr. Bootstrap: { ptr, i64 } slice. Code expecting .len() will fail.
  ────────────────────────────────────────
  ID: IC-10
  Feature: Type size/layout fallback
  Details: Self-hosted defaults unknown types to { size: 8, align: 8 }. DynTrait (should be 16),
    Range, Record, Forall, Ownership all get wrong sizes.
  ────────────────────────────────────────
  ID: IC-11
  Feature: Enum discriminant fallback
  Details: Self-hosted stores i64 directly when enum not in ADT registry, potentially overwriting
    payload
  ────────────────────────────────────────
  ID: IC-12
  Feature: StorageDead protocol mismatch
  Details: Bootstrap uses generational invalidation; self-hosted calls blood_unregister_allocation

    / blood_persistent_decrement — different runtime protocol
  ────────────────────────────────────────
  ID: IC-13
  Feature: Function pointer coercion
  Details: Self-hosted only checks parameter count, does not verify parameter types or return type
  ────────────────────────────────────────
  ID: IC-14
  Feature: Record type rest syntax
  Details: Bootstrap uses `
  ────────────────────────────────────────
  ID: IC-15
  Feature: Enum variant construction
  Details: Bootstrap has explicit ExprKind::Variant; self-hosted routes through Struct path, which

    may fail for non-struct-like variants
  ---
  3. Stubs

  Code that is defined but produces no-ops, errors, placeholders, or is never invoked.

  3.1 Parser Stubs (AST nodes defined, never generated)
  ┌─────────────────────┬────────────────────┬───────────────────────────────────────────────┐
  │      AST Type       │      Location      │                     Notes                     │
  ├─────────────────────┼────────────────────┼───────────────────────────────────────────────┤
  │ ExprKind::IfLet     │ ast.blood:672      │ Parser never produces                         │
  ├─────────────────────┼────────────────────┼───────────────────────────────────────────────┤
  │ ExprKind::WhileLet  │ ast.blood:691      │ Parser never produces                         │
  ├─────────────────────┼────────────────────┼───────────────────────────────────────────────┤
  │ ExprKind::AssignOp  │ ast.blood:661      │ Compound assignment not parsed                │
  ├─────────────────────┼────────────────────┼───────────────────────────────────────────────┤
  │ ExprKind::Range     │ ast.blood:648      │ Not in expression precedence                  │
  ├─────────────────────┼────────────────────┼───────────────────────────────────────────────┤
  │ ExprKind::MacroCall │ ast.blood:740      │ No macro call parsing                         │
  ├─────────────────────┼────────────────────┼───────────────────────────────────────────────┤
  │ ExprKind::Paren     │ ast.blood:738      │ Parens return inner directly                  │
  ├─────────────────────┼────────────────────┼───────────────────────────────────────────────┤
  │ PatternKind::Or     │ ast.blood:919      │ Or-patterns not parsed                        │
  ├─────────────────────┼────────────────────┼───────────────────────────────────────────────┤
  │ PatternKind::Range  │ ast.blood:920      │ Range patterns not parsed                     │
  ├─────────────────────┼────────────────────┼───────────────────────────────────────────────┤
  │ Declaration::Bridge │ ast.blood:81       │ Bridge keyword not dispatched                 │
  ├─────────────────────┼────────────────────┼───────────────────────────────────────────────┤
  │ Declaration::Macro  │ ast.blood:84       │ Macro keyword not dispatched                  │
  ├─────────────────────┼────────────────────┼───────────────────────────────────────────────┤
  │ All Bridge* types   │ ast.blood:338-449  │ Entire bridge system defined, never populated │
  ├─────────────────────┼────────────────────┼───────────────────────────────────────────────┤
  │ All Macro* types    │ ast.blood:967-1079 │ Entire macro system defined, never populated  │
  └─────────────────────┴────────────────────┴───────────────────────────────────────────────┘
  3.2 HIR Stubs
  Item: TryWith lowering
  Location: hir_lower_expr.blood
  Notes: Dispatched but produces Expr::error()
  ────────────────────────────────────────
  Item: MacroDef
  Location: hir_item.blood
  Notes: Structure exists; described as "placeholder for now"
  ────────────────────────────────────────
  Item: Forall type (full)
  Location: hir_ty.blood
  Notes: Type exists but Skolemization and subsumption checking not implemented
  3.3 Type Checking Stubs
  Item: TraitInfo / TraitImplInfo
  Location: typeck_info.blood
  Notes: Structures defined and registered but no trait bound checking uses them
  ────────────────────────────────────────
  Item: Coercion::Deref
  Location: typeck_types.blood:153-168
  Notes: Defined in enum but never constructed by try_coerce
  ────────────────────────────────────────
  Item: Coercion::ClosureToFnPtr
  Location: typeck_types.blood:153-168
  Notes: Defined in enum but never constructed
  ────────────────────────────────────────
  Item: CheckMode::Coerce
  Location: typeck_types.blood:128
  Notes: Defined but never used
  ────────────────────────────────────────
  Item: EffectInfo / EffectOpInfo
  Location: typeck_info.blood
  Notes: Registered but no effect subsumption at function boundaries
  3.4 MIR Stubs
  Item: Or-pattern matching
  Location: mir_lower_pattern.blood:304-307
  Notes: pattern_test() returns None, treating as irrefutable
  ────────────────────────────────────────
  Item: Range pattern matching
  Location: mir_lower_pattern.blood:309-312
  Notes: pattern_test() returns None, treating as irrefutable
  ────────────────────────────────────────
  Item: Slice pattern matching (refutable)
  Location: mir_lower_pattern.blood:299-302
  Notes: pattern_test() returns None, treating as irrefutable
  ────────────────────────────────────────
  Item: Try expression
  Location: mir_lower_expr.blood:1699-1709
  Notes: lower_try_expr() delegates to lower_expr() with no error propagation
  3.5 Codegen Stubs
  Item: Drop implementation
  Location: codegen_stmt.blood:79-114
  Notes: Only handles region-allocated refs; no destructor/drop glue
  ────────────────────────────────────────
  Item: Deinit statement
  Location: codegen_stmt.blood:115-118
  Notes: No-op comment only
  ────────────────────────────────────────
  Item: Assert terminator
  Location: codegen_term.blood:~280-310
  Notes: Hardcoded message, no source location or values
  ────────────────────────────────────────
  Item: Builtin method declarations
  Location: codegen.blood intrinsics
  Notes: String::len, Vec::push, etc. declared but no runtime library provides them
  ────────────────────────────────────────
  Item: Snapshot functions
  Location: codegen.blood intrinsics
  Notes: blood_snapshot_create/restore declared but never called
  ────────────────────────────────────────
  Item: Dispatch functions
  Location: codegen.blood intrinsics
  Notes: blood_dispatch_register/lookup declared but never used
  ---
  Summary Counts
  ┌───────────────────────────┬────────────────────┐
  │         Category          │       Count        │
  ├───────────────────────────┼────────────────────┤
  │ Missing Features          │ ~75 distinct items │
  ├───────────────────────────┼────────────────────┤
  │ Incorrect Implementations │ 15 items           │
  ├───────────────────────────┼────────────────────┤
  │ Stubs                     │ ~30 items          │
  └───────────────────────────┴────────────────────┘
  The highest-impact gaps cluster around: (1) generational memory safety (escape analysis,
  generation checks, memory tiers), (2) the effect/handler system (evidence passing,
  tail-resumptive optimization), (3) closure compilation (no function pointer generation), (4)
  generic monomorphization (all params → ptr), and (5) trait system (no bound checking, no
  coherence, no auto-ref). These represent the core semantic gaps between a bootstrap compiler
  that enforces Blood's safety guarantees and a self-hosted compiler that currently compiles
  structure but not safety.

