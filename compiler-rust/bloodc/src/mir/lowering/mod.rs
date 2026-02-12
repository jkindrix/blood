//! # HIR to MIR Lowering
//!
//! This module implements the translation from HIR (High-level IR) to
//! MIR (Mid-level IR).
//!
//! ## Lowering Process
//!
//! The lowering process transforms:
//! - Nested expressions → flat statements with temporaries
//! - Implicit control flow → explicit CFG edges
//! - Pattern matching → switch + goto
//! - Loops → CFG cycles
//!
//! ## Design References
//!
//! - [Rust MIR Lowering](https://rustc-dev-guide.rust-lang.org/mir/construction.html)
//! - Blood HIR in `hir/` module
//!
//! ## Example Lowering
//!
//! ```text
//! // HIR (nested)
//! let x = if cond { a + b } else { c };
//!
//! // MIR (flat CFG)
//! bb0:
//!     _1 = cond
//!     switchInt(_1) -> [true: bb1, false: bb2]
//! bb1:
//!     _2 = a
//!     _3 = b
//!     _4 = Add(_2, _3)
//!     _0 = _4
//!     goto -> bb3
//! bb2:
//!     _0 = c
//!     goto -> bb3
//! bb3:
//!     // continue...
//! ```

mod closure;
mod function;
mod util;

use std::collections::HashMap;

use crate::hir::{
    self, Body, Crate as HirCrate, DefId, ItemKind, Type,
};
use crate::diagnostics::Diagnostic;

use super::body::MirBody;
use super::types::BasicBlockId;

/// A pending closure to be lowered: (body_id, synthetic def_id, captures with types).
pub type PendingClosures = Vec<(hir::BodyId, DefId, Vec<(hir::Capture, Type)>)>;

/// A captured variable for inline handler bodies.
#[derive(Debug, Clone)]
pub struct InlineHandlerCaptureInfo {
    /// The HIR local ID being captured.
    pub local_id: hir::LocalId,
    /// The type of the captured variable.
    pub ty: Type,
    /// Whether this is a mutable capture (for assignment).
    pub is_mutable: bool,
}

/// Information about an inline handler operation body needed for codegen.
///
/// This stores the HIR expression for inline handler bodies (try/with) so they
/// can be compiled to LLVM functions during code generation.
#[derive(Debug, Clone)]
pub struct InlineHandlerBody {
    /// The DefId of the parent function containing this handler.
    /// Used to generate unique symbol names for inline handlers.
    pub parent_def_id: DefId,
    /// The effect being handled.
    pub effect_id: DefId,
    /// The operation name.
    pub op_name: String,
    /// The operation index within the effect.
    pub op_index: u32,
    /// Parameter local IDs for binding operation parameters.
    pub params: Vec<hir::LocalId>,
    /// Parameter types.
    pub param_types: Vec<Type>,
    /// Return type of the operation.
    pub return_type: Type,
    /// The handler body expression.
    pub body: hir::Expr,
    /// Variables captured from the enclosing scope.
    pub captures: Vec<InlineHandlerCaptureInfo>,
}

/// Map from synthetic DefId to inline handler body info.
pub type InlineHandlerBodies = HashMap<DefId, InlineHandlerBody>;

pub use closure::ClosureLowering;
pub use function::FunctionLowering;
pub use util::{convert_binop, convert_unop, lower_literal_to_constant, is_irrefutable_pattern, ExprLowering, LoopContextInfo};

// ============================================================================
// MIR Lowering Pass
// ============================================================================

/// Lowers HIR to MIR.
#[derive(Debug)]
pub struct MirLowering<'hir> {
    /// The HIR crate being lowered.
    hir: &'hir HirCrate,
    /// Lowered MIR bodies.
    bodies: HashMap<DefId, MirBody>,
    /// Collected diagnostics.
    diagnostics: Vec<Diagnostic>,
    /// Counter for generating synthetic closure DefIds.
    closure_counter: u32,
    /// Pending closure bodies to be lowered (body_id, synthetic def_id, captures with types).
    pending_closures: PendingClosures,
    /// Inline handler bodies collected during lowering.
    /// Maps synthetic DefId to the handler body info for codegen.
    inline_handler_bodies: InlineHandlerBodies,
}

impl<'hir> MirLowering<'hir> {
    /// Create a new MIR lowering pass.
    pub fn new(hir: &'hir HirCrate) -> Self {
        Self {
            hir,
            bodies: HashMap::new(),
            diagnostics: Vec::new(),
            closure_counter: 0,
            pending_closures: Vec::new(),
            inline_handler_bodies: HashMap::new(),
        }
    }

    /// Lower all functions in the crate.
    ///
    /// Returns a tuple of:
    /// - MIR bodies for all functions and closures
    /// - Inline handler bodies for codegen (try/with blocks)
    pub fn lower_crate(&mut self) -> Result<(HashMap<DefId, MirBody>, InlineHandlerBodies), Vec<Diagnostic>> {
        // First pass: lower all top-level functions
        // Sort by DefId for deterministic closure counter assignment.
        // HashMap iteration is non-deterministic; since closures discovered during
        // lowering get sequential synthetic DefIds from a shared counter, the
        // iteration order determines which DefIds closures receive. Sorting ensures
        // stable DefId assignment across runs, which is critical for incremental
        // compilation (object files embed DefId-based symbol names).
        let mut sorted_items: Vec<_> = self.hir.items.iter().collect();
        sorted_items.sort_by_key(|(&def_id, _)| def_id.index());
        for (&def_id, item) in sorted_items {
            match &item.kind {
                ItemKind::Fn(fn_def) => {
                    if let Some(body_id) = fn_def.body_id {
                        if let Some(body) = self.hir.get_body(body_id) {
                            let mir_body = self.lower_function(def_id, &fn_def.sig, body)?;
                            self.bodies.insert(def_id, mir_body);
                        }
                    }
                }
                ItemKind::Trait { items, .. } => {
                    // Lower trait default methods - methods with bodies
                    for trait_item in items {
                        if let hir::TraitItemKind::Fn(sig, Some(body_id)) = &trait_item.kind {
                            if let Some(body) = self.hir.get_body(*body_id) {
                                let mir_body = self.lower_function(trait_item.def_id, sig, body)?;
                                self.bodies.insert(trait_item.def_id, mir_body);
                            }
                        }
                    }
                }
                _ => {
                    // Skip non-function items:
                    // - Struct/Enum/Union: Type declarations with no runtime code
                    // - Const/Static: Initializers are evaluated by codegen's evaluate_const_expr(),
                    //   not lowered to MIR. Static items become global variables, not functions.
                    // - TypeAlias/Impl/Module: Resolved during type checking
                    // - ExternFn/Bridge: FFI declarations handled separately in codegen
                    // - Handler/Effect: Effect system items have special codegen paths
                }
            }
        }

        // Second pass: lower any pending closures discovered during first pass
        // Process iteratively since closures can contain nested closures
        while !self.pending_closures.is_empty() {
            let pending = std::mem::take(&mut self.pending_closures);
            for (body_id, closure_def_id, captures) in pending {
                if let Some(body) = self.hir.get_body(body_id) {
                    let mir_body = self.lower_closure_body(closure_def_id, body, &captures)?;
                    self.bodies.insert(closure_def_id, mir_body);
                }
            }
        }

        if self.diagnostics.is_empty() {
            Ok((std::mem::take(&mut self.bodies), std::mem::take(&mut self.inline_handler_bodies)))
        } else {
            Err(std::mem::take(&mut self.diagnostics))
        }
    }

    /// Lower a closure body to MIR.
    fn lower_closure_body(
        &mut self,
        def_id: DefId,
        body: &Body,
        captures: &[(hir::Capture, Type)],
    ) -> Result<MirBody, Vec<Diagnostic>> {
        // Closure bodies are lowered similarly to functions
        // The captures become implicit parameters accessed via the environment
        let builder = ClosureLowering::new(
            def_id,
            body,
            captures,
            self.hir,
            &mut self.pending_closures,
            &mut self.closure_counter,
            &mut self.inline_handler_bodies,
        );
        builder.lower()
    }

    /// Lower a single function.
    fn lower_function(
        &mut self,
        def_id: DefId,
        sig: &hir::FnSig,
        body: &Body,
    ) -> Result<MirBody, Vec<Diagnostic>> {
        let builder = FunctionLowering::new(
            def_id,
            sig,
            body,
            self.hir,
            &mut self.pending_closures,
            &mut self.closure_counter,
            &mut self.inline_handler_bodies,
        );
        builder.lower()
    }
}

/// Context for a loop (for break/continue handling).
#[derive(Debug, Clone)]
pub(super) struct LoopContext {
    /// Block to jump to on break.
    pub break_block: BasicBlockId,
    /// Block to jump to on continue.
    pub continue_block: BasicBlockId,
    /// Label for labeled breaks.
    pub label: Option<hir::LoopId>,
    /// LocalId to store break value (if any).
    pub result_local: Option<hir::LocalId>,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;
    use crate::typeck::check_program;

    #[test]
    fn test_mir_lowering_new() {
        let hir = HirCrate::new();
        let lowering = MirLowering::new(&hir);
        assert!(lowering.bodies.is_empty());
    }

    /// Helper to parse, type-check, and lower to MIR.
    fn source_to_mir(source: &str) -> Result<(HirCrate, HashMap<DefId, MirBody>), Vec<Diagnostic>> {
        let mut parser = Parser::new(source);
        let program = parser.parse_program().map_err(|e| {
            e.into_iter().map(|err| {
                Diagnostic::error(err.message, crate::span::Span::default())
            }).collect::<Vec<_>>()
        })?;

        let interner = parser.take_interner();
        let hir = check_program(&program, source, interner)?;

        let mut lowering = MirLowering::new(&hir);
        let (bodies, _inline_handlers) = lowering.lower_crate()?;

        Ok((hir, bodies))
    }

    /// Helper to assert MIR lowering succeeds.
    fn assert_lowers(source: &str) {
        match source_to_mir(source) {
            Ok(_) => {}
            Err(errors) => {
                panic!(
                    "Expected MIR lowering to succeed, but got {} error(s):\n{}",
                    errors.len(),
                    errors.iter()
                        .map(|e| format!("  - {}", e.message))
                        .collect::<Vec<_>>()
                        .join("\n")
                );
            }
        }
    }

    /// Helper to get MIR body for main function.
    fn get_main_mir(source: &str) -> MirBody {
        let (hir, bodies) = source_to_mir(source).expect("source_to_mir failed");
        // Find main function
        for (def_id, item) in &hir.items {
            if matches!(item.kind, hir::ItemKind::Fn(_)) && item.name == "main" {
                return bodies.get(def_id).cloned().expect("main body not found");
            }
        }
        panic!("main function not found in HIR");
    }

    // ============================================================
    // BASIC LOWERING TESTS
    // ============================================================

    #[test]
    fn test_lower_empty_function() {
        assert_lowers("fn main() {}");
    }

    #[test]
    fn test_lower_return_literal() {
        assert_lowers("fn main() -> i32 { 42 }");
    }

    #[test]
    fn test_lower_let_binding() {
        assert_lowers("fn main() { let x = 42; }");
    }

    #[test]
    fn test_lower_arithmetic() {
        assert_lowers("fn main() -> i32 { 1 + 2 * 3 - 4 / 2 }");
    }

    #[test]
    fn test_lower_comparison() {
        assert_lowers("fn main() -> bool { 1 < 2 }");
    }

    #[test]
    fn test_lower_logical() {
        assert_lowers("fn main() -> bool { true && false || !true }");
    }

    // ============================================================
    // CONTROL FLOW LOWERING TESTS
    // ============================================================

    #[test]
    fn test_lower_if_expression() {
        assert_lowers("fn main() -> i32 { if true { 1 } else { 2 } }");
    }

    #[test]
    fn test_lower_if_no_else() {
        assert_lowers("fn main() { if true { let x = 1; } }");
    }

    #[test]
    fn test_lower_nested_if() {
        assert_lowers(r#"
            fn main() -> i32 {
                if true {
                    if false { 1 } else { 2 }
                } else {
                    3
                }
            }
        "#);
    }

    #[test]
    fn test_lower_while_loop() {
        assert_lowers(r#"
            fn main() {
                let mut x = 0;
                while x < 10 {
                    x = x + 1;
                }
            }
        "#);
    }

    #[test]
    fn test_lower_loop() {
        assert_lowers(r#"
            fn main() -> i32 {
                let mut x = 0;
                loop {
                    x = x + 1;
                    if x >= 10 { break x; }
                }
            }
        "#);
    }

    #[test]
    fn test_lower_loop_break() {
        assert_lowers(r#"
            fn main() {
                loop { break; }
            }
        "#);
    }

    #[test]
    fn test_lower_loop_continue() {
        assert_lowers(r#"
            fn main() {
                let mut x = 0;
                loop {
                    x = x + 1;
                    if x < 5 { continue; }
                    if x >= 10 { break; }
                }
            }
        "#);
    }

    // ============================================================
    // MATCH EXPRESSION LOWERING TESTS
    // ============================================================

    #[test]
    fn test_lower_match_literal() {
        assert_lowers(r#"
            fn main() -> i32 {
                let x = 1;
                match x {
                    0 => 0,
                    1 => 1,
                    _ => 2,
                }
            }
        "#);
    }

    #[test]
    fn test_lower_match_binding() {
        assert_lowers(r#"
            fn main() -> i32 {
                let x = 42;
                match x {
                    n => n + 1,
                }
            }
        "#);
    }

    #[test]
    fn test_lower_match_tuple() {
        assert_lowers(r#"
            fn main() -> i32 {
                let t = (1, 2);
                match t {
                    (a, b) => a + b,
                }
            }
        "#);
    }

    // ============================================================
    // FUNCTION CALL LOWERING TESTS
    // ============================================================

    #[test]
    fn test_lower_function_call() {
        assert_lowers(r#"
            fn add(a: i32, b: i32) -> i32 { a + b }
            fn main() -> i32 { add(1, 2) }
        "#);
    }

    #[test]
    fn test_lower_recursive_function() {
        assert_lowers(r#"
            fn factorial(n: i32) -> i32 {
                if n <= 1 { 1 } else { n * factorial(n - 1) }
            }
            fn main() -> i32 { factorial(5) }
        "#);
    }

    // ============================================================
    // STRUCT LOWERING TESTS
    // ============================================================

    #[test]
    fn test_lower_struct_construction() {
        assert_lowers(r#"
            struct Point { x: i32, y: i32 }
            fn main() { let p = Point { x: 10, y: 20 }; }
        "#);
    }

    #[test]
    fn test_lower_struct_field_access() {
        assert_lowers(r#"
            struct Point { x: i32, y: i32 }
            fn main() -> i32 {
                let p = Point { x: 10, y: 20 };
                p.x + p.y
            }
        "#);
    }

    // ============================================================
    // ENUM LOWERING TESTS
    // ============================================================

    #[test]
    fn test_lower_enum_unit_variant() {
        assert_lowers(r#"
            enum Color { Red, Green, Blue }
            fn main() { let c = Color::Red; }
        "#);
    }

    #[test]
    fn test_lower_enum_tuple_variant() {
        assert_lowers(r#"
            enum MyOption { Some(i32), None }
            fn main() { let x = MyOption::Some(42); }
        "#);
    }

    // ============================================================
    // TUPLE LOWERING TESTS
    // ============================================================

    #[test]
    fn test_lower_tuple_construction() {
        assert_lowers("fn main() { let t = (1, 2, 3); }");
    }

    #[test]
    fn test_lower_tuple_index() {
        assert_lowers("fn main() -> i32 { let t = (1, 2, 3); t.0 + t.1 }");
    }

    // ============================================================
    // ARRAY LOWERING TESTS
    // ============================================================

    #[test]
    fn test_lower_array_literal() {
        assert_lowers("fn main() { let arr = [1, 2, 3]; }");
    }

    #[test]
    fn test_lower_array_repeat() {
        assert_lowers("fn main() { let arr = [0; 10]; }");
    }

    #[test]
    fn test_lower_array_index() {
        assert_lowers("fn main() -> i32 { let arr = [1, 2, 3]; arr[0] }");
    }

    // ============================================================
    // REFERENCE LOWERING TESTS
    // ============================================================

    #[test]
    fn test_lower_reference() {
        assert_lowers("fn main() { let x = 42; let r = &x; }");
    }

    #[test]
    fn test_lower_mutable_reference() {
        assert_lowers("fn main() { let mut x = 42; let r = &mut x; }");
    }

    #[test]
    fn test_lower_dereference() {
        assert_lowers(r#"
            fn main() -> i32 {
                let x = 42;
                let r = &x;
                *r
            }
        "#);
    }

    // ============================================================
    // CLOSURE LOWERING TESTS
    // ============================================================

    #[test]
    fn test_lower_closure_no_capture() {
        assert_lowers("fn main() { let f = |x: i32| x + 1; }");
    }

    #[test]
    fn test_lower_closure_with_capture() {
        assert_lowers(r#"
            fn main() {
                let y = 10;
                let f = |x| x + y;
            }
        "#);
    }

    // ============================================================
    // MIR VERIFICATION TESTS
    // ============================================================

    #[test]
    fn test_mir_has_return_block() {
        let body = get_main_mir("fn main() {}");
        // Check that there's at least one block with Return terminator
        let has_return = body.basic_blocks.iter().any(|bb| {
            bb.terminator.as_ref().map(|t| t.kind.is_return()).unwrap_or(false)
        });
        assert!(has_return, "MIR should have a Return terminator");
    }

    #[test]
    fn test_mir_all_blocks_terminated() {
        let body = get_main_mir("fn main() -> i32 { if true { 1 } else { 2 } }");
        assert!(body.is_complete(), "All MIR blocks should be terminated");
    }

    #[test]
    fn test_mir_has_locals() {
        let body = get_main_mir("fn main() { let x = 42; let y = x + 1; }");
        // Should have return place + temporaries + user variables
        assert!(body.locals.len() >= 3, "MIR should have locals for variables");
    }

    #[test]
    fn test_mir_if_creates_multiple_blocks() {
        let body = get_main_mir("fn main() -> i32 { if true { 1 } else { 2 } }");
        // if-else should create at least 3 blocks: entry, then, else (plus join)
        assert!(body.basic_blocks.len() >= 3, "if-else should create multiple blocks");
    }

    #[test]
    fn test_mir_loop_creates_cycle() {
        let body = get_main_mir(r#"
            fn main() {
                let mut i = 0;
                while i < 10 { i = i + 1; }
            }
        "#);
        // A loop should have at least 3 blocks: before, body, after
        assert!(body.basic_blocks.len() >= 3, "while loop should create multiple blocks");
    }

    #[test]
    fn test_mir_entry_block_is_zero() {
        let body = get_main_mir("fn main() {}");
        // Entry block should always be bb0
        assert_eq!(BasicBlockId::ENTRY.0, 0);
        assert!(!body.basic_blocks.is_empty(), "Should have at least entry block");
    }

    #[test]
    fn test_mir_return_place_is_local_zero() {
        let body = get_main_mir("fn main() -> i32 { 42 }");
        assert_eq!(body.return_place().index, 0);
        assert_eq!(*body.return_type(), Type::i32());
    }

    #[test]
    fn test_mir_params_follow_return_place() {
        let (hir, bodies) = source_to_mir("fn foo(x: i32, y: i32) -> i32 { x + y } fn main() { foo(1, 2); }").unwrap();
        // Find foo function
        for (def_id, item) in &hir.items {
            if matches!(item.kind, hir::ItemKind::Fn(_)) && item.name == "foo" {
                let body = bodies.get(def_id).expect("foo body not found");
                assert_eq!(body.param_count, 2);
                // Params should be at indices 1 and 2
                assert!(body.locals.len() >= 3); // return + 2 params
                assert!(body.locals[1].is_param());
                assert!(body.locals[2].is_param());
                return;
            }
        }
        panic!("foo function not found");
    }

    #[test]
    fn test_mir_reachability() {
        let body = get_main_mir("fn main() -> i32 { if true { 1 } else { 2 } }");
        // Entry block should be reachable
        assert!(body.is_reachable(BasicBlockId::ENTRY));
        // Most blocks should be reachable in simple if-else without early returns
        let reachable_count = body.block_ids()
            .filter(|&bb_id| body.is_reachable(bb_id))
            .count();
        assert!(reachable_count >= 3, "At least 3 blocks should be reachable in if-else");
    }

    #[test]
    fn test_mir_predecessors_consistency() {
        let body = get_main_mir(r#"
            fn main() -> i32 {
                if true { 1 } else { 2 }
            }
        "#);
        let preds = body.predecessors();
        // Entry block should have no predecessors
        assert!(preds.get(&BasicBlockId::ENTRY).map(|v| v.is_empty()).unwrap_or(true));
    }
}
