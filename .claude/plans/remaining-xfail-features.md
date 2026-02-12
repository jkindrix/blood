# Remaining XFAIL Features - Implementation Specifications

This document details the 12 remaining XFAIL tests and the features required to fix them. Each feature is specified with sufficient detail for an individual agent to implement it.

---

## Feature 1: Bridge FFI Named Syntax (EASY)

**Test:** `t05_bridge_ffi.blood`
**Effort:** ~30 minutes
**Files:** `bloodc/src/parser/item.rs`

### Problem
The `bridge` keyword requires a name: `bridge "C" MyBridge { ... }` but the test uses `bridge "C" { ... }` without a name. The parser should allow an optional name, auto-generating one when omitted (like `extern` blocks do).

### Current Code
```rust
// bloodc/src/parser/item.rs:1328-1335
// Parse bridge name
let name = if self.check(TokenKind::Ident) || self.check(TokenKind::TypeIdent) {
    self.advance();
    self.spanned_symbol()
} else {
    self.error_expected("bridge name");  // ERROR: should be optional
    Spanned::new(self.intern(""), self.current.span)
};
```

### Required Changes
1. Make the bridge name optional in `parse_bridge_decl()` at line 1328
2. When no name is provided, auto-generate one like extern blocks do:
   ```rust
   let name = if self.check(TokenKind::Ident) || self.check(TokenKind::TypeIdent) {
       self.advance();
       self.spanned_symbol()
   } else {
       // Auto-generate name like extern blocks: __bridge_{line}_{col}
       let generated = format!("__bridge_{}_{}", start.start_line, start.start_col);
       Spanned::new(self.intern(&generated), start)
   };
   ```

### Verification
Test should compile and run, printing `42`.

---

## Feature 2: Const Generic Integer Literal Arguments (MEDIUM)

**Test:** `t05_const_generics_multi.blood`
**Effort:** ~2 hours
**Files:** `bloodc/src/parser/types.rs`, `bloodc/src/ast.rs`, `bloodc/src/typeck/context/expr.rs`

### Problem
The syntax `make_matrix::<3, 4>()` fails because `parse_type_args()` only accepts lifetimes and types, not integer literals. Const generic arguments need to accept integer literals.

### Current Code
```rust
// bloodc/src/parser/types.rs:365-368
let arg = if self.check(TokenKind::Lifetime) {
    TypeArg::Lifetime(self.spanned_lifetime_symbol())
} else {
    TypeArg::Type(self.parse_type())  // Only handles types, not integers
};
```

### Required Changes

1. **Extend `ast::TypeArg` enum** (if not already present):
   ```rust
   pub enum TypeArg {
       Type(Type),
       Lifetime(Spanned<Symbol>),
       Const(Expr),  // Add this variant for const expressions
   }
   ```

2. **Update `parse_type_args()`** in `types.rs`:
   ```rust
   let arg = if self.check(TokenKind::Lifetime) {
       TypeArg::Lifetime(self.spanned_lifetime_symbol())
   } else if self.check(TokenKind::IntLit) || self.check(TokenKind::Minus) {
       // Integer literal or negative integer for const generics
       TypeArg::Const(self.parse_expr())
   } else if self.check(TokenKind::LBrace) {
       // Block expression for const generics: ::<{ N + 1 }>
       TypeArg::Const(self.parse_expr())
   } else {
       TypeArg::Type(self.parse_type())
   };
   ```

3. **Update type argument processing** in `typeck/context/expr.rs` and wherever `TypeArg` is matched to handle the `Const` variant.

4. **Update const generic resolution** to evaluate const expressions at compile time.

### Verification
Test should compile and print `12` (3 * 4).

---

## Feature 3: Trait Default Method Monomorphization (MEDIUM-HARD)

**Test:** `t05_trait_default_method.blood`
**Effort:** ~4 hours
**Files:** `bloodc/src/codegen/context/mod.rs`, `bloodc/src/mir/lowering/function.rs`

### Problem
Type checking resolves default trait methods correctly, but codegen fails:
```
LLVM verification failed: Call parameter type does not match function signature!
  %load = load i8*, i8** %_1_ptr, align 8
 { i32 }  %call_result = call i32 @"blood$def333$name$D_Person"(i8* %load)
```

The default method `greet(self)` has a generic `Self` type but is being called with `Person`. The body needs to be monomorphized (specialized) for the concrete type.

### Current Behavior
- Type checker returns `trait_method.def_id` for the default method
- Codegen tries to call the generic trait method directly
- LLVM fails because `Self` type doesn't match `Person`

### Required Changes

1. **Create monomorphized copies of default method bodies** during codegen:
   - When a call to a default trait method is detected (method's DefId belongs to a trait, not an impl)
   - Create a specialized version with `Self` substituted for the concrete type
   - Cache monomorphized versions to avoid duplicates

2. **In `bloodc/src/codegen/context/mod.rs`**:
   ```rust
   // When generating a call to a method:
   fn compile_method_call(&mut self, ...) {
       let method_def_id = ...;

       // Check if this is a default trait method
       if self.is_default_trait_method(method_def_id) {
           // Get or create monomorphized version for the concrete receiver type
           let mono_def_id = self.get_or_create_monomorphized_method(
               method_def_id,
               receiver_concrete_type,
           );
           // Call the monomorphized version instead
           self.call_function(mono_def_id, args)
       } else {
           self.call_function(method_def_id, args)
       }
   }
   ```

3. **Add monomorphization cache** to `CodegenContext`:
   ```rust
   /// Cache of monomorphized trait default methods.
   /// Key: (trait_method_def_id, concrete_self_type)
   /// Value: monomorphized function's DefId and LLVM FunctionValue
   mono_cache: HashMap<(DefId, Type), (DefId, FunctionValue<'ctx>)>,
   ```

4. **Implement `get_or_create_monomorphized_method`**:
   - Clone the method's MIR body
   - Substitute all occurrences of `Self` type with the concrete type
   - Generate a new function with mangled name (include concrete type)
   - Compile the substituted body

### Verification
Test should compile and print `6` (5 + 1).

---

## Feature 4: Inline Handler Syntax (MEDIUM)

**Tests:** `t06_err_affine_multi_shot.blood`, `t06_err_linear_deep_handler.blood`
**Effort:** ~3 hours
**Files:** `bloodc/src/parser/expr.rs`, `bloodc/src/ast.rs`

### Problem
Two inline handler syntaxes aren't implemented:
1. `handle { body } with Effect { op name(resume) -> T { ... } }`
2. `with Effect { op name() -> T { ... } } do { body }`

### Required Changes

1. **Add AST nodes for inline handlers**:
   ```rust
   pub enum ExprKind {
       // Existing...

       /// Inline handle expression: `handle { body } with Effect { ops... }`
       InlineHandle {
           body: Box<Expr>,
           effect: TypePath,
           operations: Vec<InlineHandlerOp>,
       },

       /// Inline with-do expression: `with Effect { ops... } do { body }`
       InlineWithDo {
           effect: TypePath,
           operations: Vec<InlineHandlerOp>,
           body: Box<Expr>,
       },
   }

   pub struct InlineHandlerOp {
       pub name: Spanned<Symbol>,
       pub params: Vec<Param>,  // Including optional `resume` param
       pub return_type: Option<Type>,
       pub body: Block,
       pub span: Span,
   }
   ```

2. **Parse `handle { } with Effect { }`** in `parse_primary_expr()`:
   - Detect `TokenKind::Handle` followed by `{`
   - Parse body block
   - Expect `with` keyword
   - Parse effect path
   - Parse inline operation definitions in `{ }`

3. **Parse `with Effect { } do { }`** variant:
   - After `with` keyword, if followed by effect path and `{` (not handler constructor)
   - Parse effect and inline operations
   - Expect `do` keyword
   - Parse body block

4. **Lower inline handlers to named handlers** during type checking or create specific HIR representation.

### Verification
Both tests should fail compilation with appropriate linearity errors (E0310 for affine, linearity error for linear).

---

## Feature 5: Higher-Rank Polymorphism (HARD)

**Test:** `t05_forall_type.blood`
**Effort:** ~8 hours
**Files:** Multiple across parser, typeck, and potentially codegen

### Problem
The syntax `F: forall<T> Fn(T) -> T` requires higher-rank trait bounds, which need:
1. Parsing `forall<T>` in trait bound position
2. Type representation for universally quantified types
3. Subtyping/unification rules for higher-rank types

### Syntax
```blood
fn apply<F: forall<T> Fn(T) -> T>(f: F, x: i32) -> i32 {
    f(x)
}
```

### Required Changes

1. **Parser**: Extend trait bound parsing to accept `forall<...>` prefix:
   ```rust
   // In parse_trait_bound():
   let forall_params = if self.try_consume(TokenKind::Forall) {
       Some(self.parse_type_params())
   } else {
       None
   };
   let trait_path = self.parse_type_path();
   ```

2. **AST**: Add `forall_params: Option<TypeParams>` to `TraitBound`

3. **HIR Type representation**: Add `Forall` variant to `TypeKind`:
   ```rust
   TypeKind::Forall {
       params: Vec<TyVarId>,
       inner: Box<Type>,
   }
   ```

4. **Type checking**: Implement higher-rank unification:
   - When checking if `T` satisfies `forall<U> Fn(U) -> U`
   - Instantiate with fresh type variables
   - Check the instantiated bound

### Verification
Test should compile and run successfully.

---

## Feature 6: Method Overloading / Multiple Dispatch (HARD)

**Test:** `t05_method_overload.blood`
**Effort:** ~6 hours
**Files:** `bloodc/src/typeck/context/collect.rs`, `bloodc/src/typeck/context/expr.rs`, `bloodc/src/codegen/`

### Problem
Blood should support method overloading where multiple methods with the same name can coexist if they have different parameter types:
```blood
impl Printer {
    fn print(self, x: i32) -> () { ... }
    fn print(self, x: bool) -> () { ... }  // Currently: E0214 duplicate
}
```

### Current Behavior
The compiler rejects the second `print` method as a duplicate definition.

### Required Changes

1. **Change method registration** to allow multiple methods with same name:
   - Use method signature (name + parameter types) as key, not just name
   - Store methods in a multimap structure

2. **Update method resolution** to find best matching overload:
   - At call site, gather all methods with matching name
   - Filter by parameter type compatibility
   - If exactly one matches: use it
   - If zero match: E0601 "no applicable method"
   - If multiple match: E0602 "ambiguous dispatch"

3. **Name mangling** must include parameter types to distinguish overloads in codegen.

4. **Error handling**: Add proper diagnostics for ambiguous calls.

### Verification
Test should compile and print `42` then `true`.

---

## Feature 7: User-Defined Macros (HARD)

**Test:** `t05_user_macro.blood`
**Effort:** ~12 hours
**Files:** New module `bloodc/src/macro_def/`, parser changes, expansion system

### Problem
User-defined declarative macros need full implementation:
```blood
macro my_assert {
    ($cond:expr) => {
        if !$cond { panic!("assertion failed"); }
    };
}
```

### Required Changes

1. **Parser for macro definitions**:
   - Parse `macro name { ... }` declarations
   - Parse macro rules: `(pattern) => { template }`
   - Parse matchers: `$name:kind` where kind is `expr`, `ty`, `ident`, etc.

2. **Macro representation**:
   ```rust
   pub struct MacroDef {
       pub name: Symbol,
       pub rules: Vec<MacroRule>,
   }

   pub struct MacroRule {
       pub matcher: MacroMatcher,
       pub template: TokenStream,
   }
   ```

3. **Macro expansion**:
   - Match invocation against rules
   - Capture fragments
   - Substitute into template
   - Re-parse expanded tokens

4. **Hygiene** (optional for basic implementation):
   - Track macro expansion context
   - Prevent accidental variable capture

### Verification
Test should compile and run without panicking.

---

## Feature 8: Type Inference Ambiguity Detection (MEDIUM)

**Test:** `t06_err_cannot_infer.blood`
**Effort:** ~2 hours
**Files:** `bloodc/src/typeck/context/check.rs`, `bloodc/src/typeck/infer.rs`

### Problem
```blood
let x = id(id);  // What type is x? id: fn<T>(T) -> T, so id(id) = id, but which T?
```
The compiler should detect that type variable `T` cannot be uniquely determined.

### Current Behavior
Compilation proceeds with unresolved type variables, failing later in codegen.

### Required Changes

1. **After type inference**, check for unresolved type variables:
   ```rust
   fn check_inference_complete(&self, expr: &hir::Expr) -> Result<(), TypeError> {
       if expr.ty.has_unresolved_vars() {
           return Err(TypeError::new(
               TypeErrorKind::CannotInfer { ty: format!("{}", expr.ty) },
               expr.span,
           ));
       }
       Ok(())
   }
   ```

2. **Add error type**:
   ```rust
   TypeErrorKind::CannotInfer { ty: String }
   // E0XXX: cannot infer type for expression; type annotations needed
   ```

3. **Run check after function body type-checking** completes.

### Verification
Test should fail with "cannot infer type" error.

---

## ~~Feature 9: Borrow Checker~~ (REMOVED)

**Status:** REMOVED per ADR-001

Blood uses generational references for runtime memory safety instead of
compile-time borrow checking. The test `t06_err_double_mut_borrow.blood`
has been deleted as it tested Rust-style semantics that Blood explicitly
does not implement.

See: `docs/spec/DECISIONS.md` - ADR-001: Use Generational References Instead of Borrow Checking

---

## ~~Feature 10: Move Semantics Enforcement~~ (REMOVED)

**Status:** REMOVED per ADR-014

Blood uses Mutable Value Semantics (MVS) with copy-by-default, not Rust-style
move semantics. The test `t06_err_use_after_move.blood` has been deleted.

For resource types requiring single-use semantics, Blood provides opt-in
`linear` and `affine` type qualifiers (see E0801-E0805 linearity errors).

See: `docs/spec/DECISIONS.md` - ADR-014: Adopt Hybrid Mutable Value Semantics

---

## NOTE: The following was the original Feature 10 content, preserved for reference:

~~**Test:** `t06_err_use_after_move.blood`~~
~~**Effort:** ~8 hours~~

~~### Problem~~
~~```blood~~
~~let d: Data = Data { value: 42 };~~
~~let a: i32 = consume(d);  // d is moved here~~
~~let b: i32 = consume(d);  // Should fail: d already moved~~
~~```~~

Under Blood's MVS, `d` is COPIED into each call, so this code is valid.
For linear semantics, use: `let d: linear Data = ...`

---

## DELETED Feature 10 Content (was: Move Semantics)

The following content is preserved for historical reference only:
   - Passed by value to function
   - Used in move context (not `Copy` type)

3. **Check before use**:
   - If value is in `moved` set, emit error

4. **Handle control flow**:
   - Conditionally moved values
   - Loops

5. **Error type**:
   ```rust
   TypeErrorKind::UseAfterMove { name: String }
   // E0XXX: use of moved value `d`
   ```

### Verification
Test should fail with "use of moved value" error.

---

## Feature 11: FFI Portability Warnings as Errors (EASY)

**Test:** `t06_err_ffi_portability.blood`
**Effort:** ~1 hour
**Files:** `bloodc/src/typeck/ffi.rs`, `bloodc/src/main.rs`

### Problem
The `usize` type in FFI context generates a warning (E0502) but doesn't fail compilation. The test expects compilation to fail.

### Options

**Option A: Make FFI portability warnings errors by default**
- Change `FfiSafety::Warning` handling to emit errors instead
- This is stricter but matches test expectations

**Option B: Add `--deny-warnings` flag**
- Keep warnings as warnings
- Add CLI flag to treat warnings as errors
- Update test to use flag or change test expectations

**Option C: Change test expectations**
- Update test to not expect compilation failure
- Add mechanism to check for warnings in test harness

### Recommended: Option A
In `bloodc/src/typeck/ffi.rs`:
```rust
pub fn check_ffi_type(&mut self, ty: &Type, span: Span, context: &str) {
    match self.validate_type(ty) {
        FfiSafety::Safe => {}
        FfiSafety::Warning(msg) => {
            // Change from warning to error
            self.diagnostics.push(Diagnostic::error(format!(
                "type `{}` in {} has FFI portability issues: {}", ty, context, msg
            ), span));
        }
        FfiSafety::Unsafe(msg) => { ... }
    }
}
```

### Verification
Test should fail with FFI portability error for `usize`.

---

## Summary: Implementation Priority

### Quick Wins (< 2 hours each)
1. **Feature 1: Bridge FFI syntax** - Make name optional
2. **Feature 11: FFI portability** - Make warnings errors

### Medium Effort (2-4 hours each)
3. **Feature 2: Const generic integers** - Parser + type system
4. **Feature 8: Inference ambiguity** - Add check after inference
5. **Feature 3: Default method monomorphization** - Codegen specialization

### Significant Effort (4-8 hours each)
6. **Feature 4: Inline handler syntax** - Parser + lowering
7. **Feature 6: Method overloading** - Resolution + mangling

### Major Features (8+ hours each)
8. **Feature 5: Higher-rank types** - Type system extension
9. **Feature 7: User macros** - Full macro system

### REMOVED (per ADR-001 and ADR-014)
- ~~Feature 9: Borrow checker~~ - Blood uses generational refs, not borrow checking
- ~~Feature 10: Move semantics~~ - Blood uses MVS (copy by default), not move semantics

---

## Test Verification Commands

After implementing each feature, run:
```bash
./tests/ground-truth/run_tests.sh ./target/release/blood <test_name>
```

Full suite:
```bash
./tests/ground-truth/run_tests.sh ./target/release/blood
```

Current status: 299 passed, 9 expected failures, 1 crash
Target: 309+ passed, 0 expected failures
