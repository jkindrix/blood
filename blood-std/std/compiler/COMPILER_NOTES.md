# Blood Self-Hosted Compiler - Design Notes

This document captures the design decisions, known limitations, and architectural rationale for the Blood self-hosted compiler.

## Architecture Overview

The compiler follows a standard multi-phase architecture:

```
Source → Lexer → Parser → AST → HIR → Type Check → MIR → Codegen → LLVM
```

### Module Organization

| Phase | Module(s) | Responsibility |
|-------|-----------|----------------|
| Lexing | `lexer.blood`, `token.blood` | Source → Token stream |
| Parsing | `parser*.blood` (6 files) | Tokens → AST |
| Name Resolution | `resolve.blood` | Scope tracking, name binding |
| HIR Lowering | `hir_lower*.blood` (6 files) | AST → HIR with resolved names |
| Type Checking | `typeck*.blood` (4 files), `unify.blood` | HIR → Typed HIR |
| MIR Lowering | `mir_lower*.blood` (5 files) | Typed HIR → MIR |
| Code Generation | `codegen*.blood` (6 files) | MIR → LLVM IR |
| Infrastructure | `common.blood`, `interner.blood`, `driver.blood`, `reporter.blood`, `source.blood`, `main.blood` | Shared types, string interning, driver, diagnostics |

### Shared Types

Core types are defined once in `common.blood` and imported by other modules:

| Type | Purpose |
|------|---------|
| `Span` | Source location (start, end, line, column) |
| `Symbol` | Interned string identifier |
| `SpannedSymbol` | Symbol with source location |
| `SpannedString` | String with source location |
| `OrderedFloat` | Float with ordering support |
| `BinOp` | Binary operators |
| `UnaryOp` | Unary operators |

---

## Design Decisions

### 1. No Shortcuts Principle

The compiler follows a "zero shortcuts" philosophy:
- Every match arm must be exhaustive with proper handling
- Every error case must be reported
- No silent failures or placeholder returns
- Every feature must be complete or explicitly error with "not yet implemented"

### 2. Type Duplication Strategy

Some types are intentionally duplicated across modules due to blood-rust limitations:

**Destination enum** (mir_lower_ctx.blood, mir_lower_expr.blood):
- Reason: blood-rust doesn't fully support cross-module enum constructor calls
- Both definitions must be kept in sync manually
- Documented in source with NOTE comments

### 3. Large File Acceptance

Some files exceed the typical 600-line guideline but are accepted due to:
- Good internal organization with clear section comments
- Tight coupling that would create circular dependencies if split
- Stable, well-tested code

Current large files:
- `hir_lower_expr.blood` (~1641 lines) - Expression/pattern/control flow lowering
- `unify.blood` (~1232 lines) - Type unification with union-find
- `typeck_expr.blood` (~1113 lines) - Expression type checking
- `ast.blood` (~1070 lines) - All AST node types

### 4. Qualified Path Resolution

Multi-segment paths (e.g., `module::Type`) are resolved by:
1. Looking up the first segment in the current scope
2. For each subsequent segment, searching definitions whose parent is the previous segment's DefId
3. Returning the final DefId with type arguments from the last segment

This requires definitions to track their parent (DefInfo.parent) during registration.

### 5. Const Expression Evaluation

Array sizes and repeat counts are evaluated at compile time using `const_eval.blood`:

**Supported:**
- Integer literals
- Basic arithmetic: +, -, *, /, %
- Bitwise operations: &, |, ^
- Unary negation and not
- Parenthesized expressions

**Not Supported:**
- Shift operations (blood-rust type semantics issue)
- Comparison operators (return bool, not int)
- Const variable references
- Function calls

---

## Known Limitations (Phase 2 Features)

These features are intentionally deferred to Phase 2 and will return explicit "not yet implemented" errors:

### Effect System (TODO-01, TODO-12, TODO-13, TODO-17)

| Feature | Location | Status |
|---------|----------|--------|
| Effect row unification | unify.blood | ✅ Implemented |
| Effect row variables | hir_lower_type.blood | ⚠️ Partial (basic support) |
| Effect type inference | typeck.blood | ⚠️ Basic only |
| Effect handler codegen | codegen_stmt.blood, codegen_term.blood | ✅ Runtime stubs implemented |

**Rationale**: Full effect system requires:
- Row polymorphism infrastructure (partial)
- Effect handler type checking (basic)
- Effect discharge verification (not implemented)

### Row Polymorphism (TODO-02, TODO-08)

| Feature | Location | Status |
|---------|----------|--------|
| Row polymorphism for records | hir_ty.blood | Not implemented |
| Row variable handling in type lowering | hir_lower_type.blood | Not implemented |

**Rationale**: Row polymorphism for extensible records requires:
- Row variable tracking
- Structural unification with row extension
- Type inference for partial records

### Advanced Type Features (TODO-10, TODO-15, TODO-16)

| Feature | Location | Status |
|---------|----------|--------|
| Forall type handling | hir_lower_type.blood | Lowered as body only |
| Const generics | hir_lower_type.blood | Not implemented |
| Ownership qualifiers | hir_lower_type.blood | Stripped during lowering |

**Rationale**: These features require significant type system infrastructure.

---

## Feature Status Matrix

### Core Features

| Feature | Status | Notes |
|---------|--------|-------|
| Lexing | ✅ Complete | Full Blood token set |
| Parsing | ✅ Complete | All Blood syntax |
| Name resolution | ✅ Complete | Single and qualified paths |
| HIR lowering | ✅ Complete | All expression/statement/item kinds |
| Type inference | ✅ Mostly Complete | HM-style with unification; generic instantiation implemented |
| Type checking | ✅ Mostly Complete | Expression, pattern, and item type checking |
| Method resolution | ✅ Implemented | Impl block registration and method lookup |
| Generic instantiation | ✅ Implemented | Type parameter substitution for generic calls |
| MIR lowering | ✅ Mostly Complete | Pattern matching and variant indices improved |
| Code generation | ⚠️ Partial | LLVM IR output; type-aware casts and string constants added |
| Const evaluation | ✅ Complete | Array sizes, repeat counts |

### Type System Features

| Feature | Status | Notes |
|---------|--------|-------|
| Path resolution | ✅ Implemented | Looks up fn_sigs, consts, statics, enums |
| Function type inference | ✅ Implemented | Generic functions instantiate with fresh inference vars |
| Struct field access | ✅ Implemented | ADT field lookup with type substitution |
| Type substitution | ✅ Implemented | TypeParamSubst for generic instantiation |
| Type coercion | ✅ Implemented | &mut T -> &T, array unsize, fn pointer coercion |
| Trait bound checking | ✅ Implemented | Basic trait registry and obligation resolution |
| Pattern exhaustiveness | ✅ Implemented | Pattern matrix algorithm for match completeness |
| Effect row unification | ✅ Implemented | Effect set comparison with row variable support |

### Code Generation Features

| Feature | Status | Notes |
|---------|--------|-------|
| Integer operations | ✅ Implemented | Type-aware operations with proper LLVM types |
| Float operations | ✅ Implemented | fadd/fsub/fmul/fdiv/frem with fcmp |
| Cast operations | ✅ Implemented | trunc, zext, sext, fptrunc, fpext, etc. |
| String constants | ✅ Implemented | String table with global string literals |
| Function calls | ✅ Implemented | Type-aware parameter and return types |
| Effect runtime | ✅ Implemented | Runtime stub declarations for effect handlers |

### Advanced Features (Phase 2)

| Feature | Status | Priority |
|---------|--------|----------|
| Effect row unification | ✅ Implemented | - |
| Row polymorphism | ❌ Not implemented | LOW |
| Forall types | ❌ Not implemented | LOW |
| Const generics | ❌ Not implemented | LOW |
| Effect row variables | ⚠️ Partial | LOW |
| Pattern exhaustiveness | ✅ Implemented | - |
| Ownership tracking | ❌ Not implemented | MEDIUM |
| Local item handling | ❌ Not implemented | MEDIUM |

---

## Blood-Rust Limitations

The self-hosted compiler works around these blood-rust compiler limitations:

| Limitation | Workaround |
|------------|------------|
| Cross-module enum constructors | Duplicate enum definitions with sync notes |
| `use` imports after declarations | Use qualified paths instead |
| Some keywords as field names | Rename fields (e.g., `mod_decl` instead of `module`) |

---

## Testing Strategy

### Compilation Verification

All compiler files must pass blood-rust type checking:
```bash
for f in blood-std/std/compiler/*.blood; do
  blood check "$f"
done
```

### End-to-End Testing

Test programs are compiled with the self-hosted compiler and verified against expected output.

---

## Contributing

When modifying the compiler:

1. **Compile before commit** - Every file must pass `blood check`
2. **Keep definitions in sync** - Update all duplicated types together
3. **Update this document** - Add new limitations or design decisions
4. **Follow the zero shortcuts principle** - No silent failures

---

## Version History

- **Initial version**: Complete compiler pipeline, all files type-check
- **2024-01**: Added qualified path resolution
- **2024-01**: Added const expression evaluation for arrays
- **2026-01**: Gap resolution phase:
  - Added ConstInfo, StaticInfo, ImplInfo registries to TypeChecker (typeck.blood)
  - Added TypeParamSubst for generic type parameter substitution (unify.blood)
  - Fixed path resolution to look up actual types from registries (typeck_expr.blood)
  - Implemented generic function instantiation with fresh inference variables
  - Implemented method resolution via impl block lookup
  - Fixed ADT field type lookup with type argument substitution
  - Added emit_cast instruction to codegen_ctx.blood
  - Added type-aware cast operations in codegen_expr.blood
  - Added string constant table with global string literal support
  - Added variant_index to ResolvedPath and DefInfo for proper enum variant handling
  - Updated COMPILER_NOTES.md with accurate feature status
- **2026-01**: Complete gap resolution phase 2:
  - Implemented type-aware code generation with float operations (codegen_expr.blood)
  - Added emit_operand_typed, emit_binop_typed, emit_unop_typed for proper LLVM types
  - Implemented type coercion: &mut T -> &T, [T;N] -> [T], fn item -> fn pointer
  - Implemented trait bound resolution with trait registry and obligation solver
  - Implemented pattern exhaustiveness checking with pattern matrix algorithm
  - Implemented effect row unification for function types
  - Implemented effect handler code generation with runtime stub declarations
  - Added blood_push_handler, blood_pop_handler, blood_perform, blood_resume runtime stubs
