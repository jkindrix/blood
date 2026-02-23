//! # Effect Inference
//!
//! Implements automatic effect signature inference for Blood functions.
//!
//! ## Overview
//!
//! When a function doesn't have an explicit effect annotation, the effect inferencer
//! traverses the function body to determine which effects are actually performed.
//! This enables ergonomic effect usage while maintaining type safety.
//!
//! ## Algorithm
//!
//! 1. Traverse the function body (HIR)
//! 2. Collect all `perform` operations
//! 3. Track handlers that reduce (handle) effects
//! 4. Compute the minimal effect row: performed - handled
//!
//! ## Effect Polymorphism
//!
//! Effect inference also handles row polymorphism by detecting:
//! - Calls to functions with polymorphic effect signatures
//! - Higher-order functions that may propagate caller effects
//!
//! ## Example
//!
//! ```text
//! // No effect annotation - effects inferred
//! fn counter() -> i32 {
//!     let x = get()     // Perform State.get
//!     put(x + 1)        // Perform State.put
//!     get()             // Perform State.get
//! }
//!
//! // Inferred effect row: {State<i32>}
//! ```
//!
//! ## Handler Reduction
//!
//! ```text
//! fn safe_divide(x: i32, y: i32) -> Option<i32> {
//!     handle {
//!         if y == 0 {
//!             throw()    // Perform Error.throw
//!         } else {
//!             x / y
//!         }
//!     } with ErrorHandler
//! }
//!
//! // The handler handles Error, so inferred effects: {} (pure)
//! ```

use std::collections::HashSet;

use super::row::{EffectRef, EffectRow, RowVar};
use crate::hir::{Body, DefId, Expr, ExprKind, Stmt};

/// Effect inferencer that traverses HIR to collect performed effects.
///
/// The inferencer maintains:
/// - A set of effects that are performed
/// - A set of effects that are handled (reduced by handlers)
/// - A row variable counter for polymorphism
#[derive(Debug)]
pub struct EffectInferencer {
    /// Counter for generating fresh row variables.
    row_var_counter: u32,
}

/// Context for tracking effect information during inference.
///
/// This is separate from EffectInferencer to allow borrowing
/// during recursive traversal.
#[derive(Debug, Clone, Default)]
pub struct InferenceContext {
    /// Effects that are performed in the current scope.
    performed: HashSet<EffectRef>,
    /// Effects that are handled in the current scope.
    handled: HashSet<DefId>,
    /// Whether this function may be row-polymorphic.
    is_polymorphic: bool,
    /// Row variable for polymorphism (if applicable).
    /// Reserved for future use when we need to track pre-assigned row variables.
    #[allow(dead_code)] // Infrastructure for pre-assigned row variable tracking
    row_var: Option<RowVar>,
}

impl EffectInferencer {
    /// Create a new effect inferencer.
    pub fn new() -> Self {
        Self {
            row_var_counter: 0,
        }
    }

    /// Generate a fresh row variable for effect polymorphism.
    pub fn fresh_row_var(&mut self) -> RowVar {
        let var = RowVar::new(self.row_var_counter);
        self.row_var_counter += 1;
        var
    }

    /// Infer the effect row for a function body.
    ///
    /// This is the main entry point for effect inference. It traverses the
    /// function body and returns the minimal effect row needed.
    ///
    /// # Arguments
    ///
    /// * `body` - The function body (HIR)
    ///
    /// # Returns
    ///
    /// The inferred effect row. If the function is pure, returns an empty row.
    /// If the function uses effects, returns a row containing those effects.
    /// If the function may be polymorphic, the row will have a row variable.
    pub fn infer_effects(&mut self, body: &Body) -> EffectRow {
        let mut ctx = InferenceContext::default();
        self.infer_expr(&body.expr, &mut ctx);

        // Build the effect row from collected effects
        self.build_effect_row(ctx)
    }

    /// Infer effects from an expression.
    ///
    /// This handles all expression kinds and recursively traverses the AST.
    fn infer_expr(&mut self, expr: &Expr, ctx: &mut InferenceContext) {
        match &expr.kind {
            // Effect operations - the core of effect inference
            ExprKind::Perform { effect_id, args, .. } => {
                // Add this effect to the performed set
                let effect_ref = self.type_args_from_effect(expr, *effect_id);
                ctx.performed.insert(effect_ref);

                // Infer effects in arguments
                for arg in args {
                    self.infer_expr(arg, ctx);
                }
            }

            // Handle expressions reduce (handle) effects
            ExprKind::Handle { body, handler_id, handler_instance } => {
                // Create a child context for the handled body
                let mut child_ctx = InferenceContext::default();
                self.infer_expr(body, &mut child_ctx);

                // The handler reduces its effect - we need to know which effect
                // the handler handles. This requires looking up the handler.
                // For now, we mark the handler_id as handled.
                ctx.handled.insert(*handler_id);

                // Effects performed in the body that are NOT handled propagate up
                for effect in child_ctx.performed {
                    if !ctx.handled.contains(&effect.def_id) {
                        ctx.performed.insert(effect);
                    }
                }

                // Infer effects in the handler instance (if any)
                self.infer_expr(handler_instance, ctx);
            }

            // Inline handle expressions - like Handle but with inline handler definitions
            ExprKind::InlineHandle { body, handlers } => {
                // Create a child context for the handled body
                let mut child_ctx = InferenceContext::default();
                self.infer_expr(body, &mut child_ctx);

                // Mark all handled effects as handled
                for handler in handlers {
                    ctx.handled.insert(handler.effect_id);
                }

                // Effects performed in the body that are NOT handled propagate up
                for effect in child_ctx.performed {
                    if !ctx.handled.contains(&effect.def_id) {
                        ctx.performed.insert(effect);
                    }
                }

                // Infer effects in handler bodies
                for handler in handlers {
                    self.infer_expr(&handler.body, ctx);
                }
            }

            // Block/Region expressions
            ExprKind::Block { stmts, expr: tail }
            | ExprKind::Region { stmts, expr: tail, .. } => {
                for stmt in stmts {
                    self.infer_stmt(stmt, ctx);
                }
                if let Some(tail_expr) = tail {
                    self.infer_expr(tail_expr, ctx);
                }
            }

            // Control flow
            ExprKind::If { condition, then_branch, else_branch } => {
                self.infer_expr(condition, ctx);
                self.infer_expr(then_branch, ctx);
                if let Some(else_br) = else_branch {
                    self.infer_expr(else_br, ctx);
                }
            }

            ExprKind::Match { scrutinee, arms } => {
                self.infer_expr(scrutinee, ctx);
                for arm in arms {
                    if let Some(ref guard) = arm.guard {
                        self.infer_expr(guard, ctx);
                    }
                    self.infer_expr(&arm.body, ctx);
                }
            }

            ExprKind::Loop { body, .. } => {
                self.infer_expr(body, ctx);
            }

            ExprKind::While { condition, body, .. } => {
                self.infer_expr(condition, ctx);
                self.infer_expr(body, ctx);
            }

            // Function calls may propagate effects
            ExprKind::Call { callee, args } => {
                // If calling a local (higher-order function), we may be polymorphic
                if matches!(&callee.kind, ExprKind::Local(_)) {
                    ctx.is_polymorphic = true;
                }
                self.infer_expr(callee, ctx);
                for arg in args {
                    self.infer_expr(arg, ctx);
                }
            }

            ExprKind::MethodCall { receiver, args, .. } => {
                self.infer_expr(receiver, ctx);
                for arg in args {
                    self.infer_expr(arg, ctx);
                }
            }

            // Binary and unary operations
            ExprKind::Binary { left, right, .. } => {
                self.infer_expr(left, ctx);
                self.infer_expr(right, ctx);
            }

            ExprKind::Unary { operand, .. } => {
                self.infer_expr(operand, ctx);
            }

            // Compound expressions
            ExprKind::Tuple(exprs) | ExprKind::Array(exprs) => {
                for e in exprs {
                    self.infer_expr(e, ctx);
                }
            }

            ExprKind::Repeat { value, .. } => {
                self.infer_expr(value, ctx);
            }

            ExprKind::Struct { fields, base, .. } => {
                for field in fields {
                    self.infer_expr(&field.value, ctx);
                }
                if let Some(base_expr) = base {
                    self.infer_expr(base_expr, ctx);
                }
            }

            ExprKind::Variant { fields, .. } => {
                for field in fields {
                    self.infer_expr(field, ctx);
                }
            }

            ExprKind::Record { fields } => {
                for field in fields {
                    self.infer_expr(&field.value, ctx);
                }
            }

            // Return and control flow
            ExprKind::Return(Some(e)) => {
                self.infer_expr(e, ctx);
            }

            ExprKind::Break { value: Some(e), .. } => {
                self.infer_expr(e, ctx);
            }

            ExprKind::Assign { target, value } => {
                self.infer_expr(target, ctx);
                self.infer_expr(value, ctx);
            }

            // Field and index access
            ExprKind::Field { base, .. } => {
                self.infer_expr(base, ctx);
            }

            ExprKind::Index { base, index } => {
                self.infer_expr(base, ctx);
                self.infer_expr(index, ctx);
            }

            // Reference operations
            ExprKind::Borrow { expr: inner, .. } |
            ExprKind::Deref(inner) |
            ExprKind::AddrOf { expr: inner, .. } |
            ExprKind::Cast { expr: inner, .. } |
            ExprKind::Unsafe(inner) => {
                self.infer_expr(inner, ctx);
            }

            ExprKind::Let { init, .. } => {
                self.infer_expr(init, ctx);
            }

            ExprKind::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.infer_expr(s, ctx);
                }
                if let Some(e) = end {
                    self.infer_expr(e, ctx);
                }
            }

            // Resume may indicate we're in a handler context
            ExprKind::Resume { value } => {
                if let Some(v) = value {
                    self.infer_expr(v, ctx);
                }
            }

            // Closures have their own effect inference
            ExprKind::Closure { .. } => {
                // Closures are analyzed separately
                // Their effects are captured in the closure type
            }

            // Macro expansion expressions
            ExprKind::MacroExpansion { args, named_args, .. } => {
                for arg in args {
                    self.infer_expr(arg, ctx);
                }
                for (_, arg) in named_args {
                    self.infer_expr(arg, ctx);
                }
            }
            ExprKind::VecLiteral(exprs) => {
                for e in exprs {
                    self.infer_expr(e, ctx);
                }
            }
            ExprKind::VecRepeat { value, count } => {
                self.infer_expr(value, ctx);
                self.infer_expr(count, ctx);
            }
            ExprKind::Assert { condition, message } => {
                self.infer_expr(condition, ctx);
                if let Some(msg) = message {
                    self.infer_expr(msg, ctx);
                }
            }
            ExprKind::Dbg(inner) => {
                self.infer_expr(inner, ctx);
            }
            ExprKind::SliceLen(inner) => {
                self.infer_expr(inner, ctx);
            }
            ExprKind::VecLen(inner) => {
                self.infer_expr(inner, ctx);
            }
            ExprKind::ArrayToSlice { expr, .. } => {
                self.infer_expr(expr, ctx);
            }

            // Leaf expressions - no effects
            ExprKind::Literal(_) |
            ExprKind::Local(_) |
            ExprKind::Def(_) |
            ExprKind::MethodFamily { .. } |
            ExprKind::ConstParam(_) |
            ExprKind::Continue { .. } |
            ExprKind::Return(None) |
            ExprKind::Break { value: None, .. } |
            ExprKind::Default |
            ExprKind::Error => {
                // No effects
            }
        }
    }

    /// Infer effects from a statement.
    fn infer_stmt(&mut self, stmt: &Stmt, ctx: &mut InferenceContext) {
        match stmt {
            Stmt::Expr(expr) => {
                self.infer_expr(expr, ctx);
            }
            Stmt::Let { init: Some(expr), .. } => {
                self.infer_expr(expr, ctx);
            }
            Stmt::Let { init: None, .. } => {
                // No effects
            }
            Stmt::Item(_) => {
                // Nested items have their own effect inference
            }
        }
    }

    /// Extract type arguments from an effect reference.
    ///
    /// Currently returns an EffectRef without type arguments. Effects are uniquely
    /// identified by their DefId, which is sufficient for effect row tracking and
    /// handler matching.
    ///
    /// # Future Enhancement: Generic Effect Type Arguments
    ///
    /// To support distinguishing between different instantiations of the same
    /// generic effect (e.g., `State<i32>` vs `State<String>`), this function
    /// would need to:
    ///
    /// 1. Have `ExprKind::Perform` store the effect's type arguments (currently
    ///    only stored locally during type checking in `infer_perform`)
    /// 2. Or re-derive type arguments from the expression's type context
    ///
    /// This would enable more precise effect tracking where `State<i32>` and
    /// `State<String>` are treated as distinct effects in the row.
    ///
    /// For most use cases, the current behavior (tracking by DefId only) is
    /// correct since handlers typically handle all instantiations of an effect.
    fn type_args_from_effect(&self, _expr: &Expr, effect_id: DefId) -> EffectRef {
        EffectRef::new(effect_id)
    }

    /// Build the final effect row from the inference context.
    fn build_effect_row(&mut self, ctx: InferenceContext) -> EffectRow {
        // Filter out handled effects
        let remaining: Vec<EffectRef> = ctx.performed
            .into_iter()
            .filter(|e| !ctx.handled.contains(&e.def_id))
            .collect();

        if remaining.is_empty() && !ctx.is_polymorphic {
            // Pure function
            EffectRow::pure()
        } else {
            let mut row = EffectRow::pure();
            for effect in remaining {
                row.add_effect(effect);
            }
            if ctx.is_polymorphic {
                // Add a row variable for polymorphism
                let row_var = self.fresh_row_var();
                row.set_row_var(row_var);
            }
            row
        }
    }
}

impl Default for EffectInferencer {
    fn default() -> Self {
        Self::new()
    }
}

/// Infer effects for a function body with handler tracking.
///
/// This is a convenience function that creates an inferencer and runs inference.
pub fn infer_effects(body: &Body) -> EffectRow {
    let mut inferencer = EffectInferencer::new();
    inferencer.infer_effects(body)
}

/// Verify that inferred effects are a subset of declared effects.
///
/// This is used when a function has an explicit effect annotation to ensure
/// the function doesn't perform undeclared effects.
///
/// # Arguments
///
/// * `inferred` - The inferred effect row from the function body
/// * `declared` - The declared effect row from the function signature
///
/// # Returns
///
/// `Ok(())` if inferred effects are valid, otherwise `Err` with a list of
/// undeclared effects.
pub fn verify_effects_subset(
    inferred: &EffectRow,
    declared: &EffectRow,
) -> Result<(), Vec<EffectRef>> {
    let mut undeclared = Vec::new();

    for effect in inferred.effects() {
        // Check if this effect is declared or if declared is polymorphic
        if !declared.contains(effect) && !declared.is_polymorphic() {
            undeclared.push(effect.clone());
        }
    }

    if undeclared.is_empty() {
        Ok(())
    } else {
        Err(undeclared)
    }
}

/// Effect inference result with additional metadata.
#[derive(Debug, Clone)]
pub struct InferenceResult {
    /// The inferred effect row.
    pub effect_row: EffectRow,
    /// Whether the function performs any effects.
    pub has_effects: bool,
    /// Whether the function is row-polymorphic.
    pub is_polymorphic: bool,
    /// Effects that were handled within the function.
    pub handled_effects: HashSet<DefId>,
}

impl InferenceResult {
    /// Create an inference result from an effect row.
    pub fn from_row(row: EffectRow) -> Self {
        Self {
            has_effects: !row.is_pure(),
            is_polymorphic: row.is_polymorphic(),
            handled_effects: HashSet::new(),
            effect_row: row,
        }
    }
}

/// Extended effect inferencer that tracks more detailed information.
///
/// This is used for type checking integration where we need to track
/// which effects are performed, which are handled, and provide detailed
/// error reporting.
#[derive(Debug)]
pub struct DetailedEffectInferencer {
    /// The base inferencer.
    inner: EffectInferencer,
    /// Effects handled in the current function.
    handled: HashSet<DefId>,
    /// Mapping from handler DefId to the effect it handles.
    handler_effects: std::collections::HashMap<DefId, DefId>,
}

impl DetailedEffectInferencer {
    /// Create a new detailed effect inferencer.
    pub fn new() -> Self {
        Self {
            inner: EffectInferencer::new(),
            handled: HashSet::new(),
            handler_effects: std::collections::HashMap::new(),
        }
    }

    /// Register a handler and the effect it handles.
    ///
    /// This information is used during inference to properly reduce effects
    /// when a handler is encountered.
    pub fn register_handler(&mut self, handler_id: DefId, effect_id: DefId) {
        self.handler_effects.insert(handler_id, effect_id);
    }

    /// Infer effects with detailed tracking.
    ///
    /// This returns an InferenceResult with additional metadata about
    /// which effects were handled.
    pub fn infer_effects_detailed(&mut self, body: &Body) -> InferenceResult {
        let mut ctx = InferenceContext::default();
        self.inner.infer_expr(&body.expr, &mut ctx);

        // Track handled effects
        for handler_id in &ctx.handled {
            if let Some(&effect_id) = self.handler_effects.get(handler_id) {
                self.handled.insert(effect_id);
            }
        }

        let row = self.inner.build_effect_row(ctx.clone());

        InferenceResult {
            has_effects: !row.is_pure(),
            is_polymorphic: row.is_polymorphic(),
            handled_effects: self.handled.clone(),
            effect_row: row,
        }
    }

    /// Get the inner inferencer for row variable generation.
    pub fn inner_mut(&mut self) -> &mut EffectInferencer {
        &mut self.inner
    }
}

impl Default for DetailedEffectInferencer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use crate::hir::{Local, LocalId, LiteralValue, Type};
    use crate::span::Span;

    /// Create a test expression with the given kind.
    fn make_expr(kind: ExprKind) -> Expr {
        Expr {
            kind,
            ty: Type::unit(),
            span: Span::dummy(),
        }
    }

    /// Create a test body with the given expression.
    fn make_body(expr: Expr) -> Body {
        Body {
            locals: vec![Local {
                id: LocalId::new(0),
                ty: Type::unit(),
                mutable: false,
                name: Some("_return".to_string()),
                span: Span::dummy(),
            }],
            param_count: 0,
            expr,
            span: Span::dummy(),
            tuple_destructures: HashMap::new(),
        }
    }

    #[test]
    fn test_pure_function_inference() {
        // A function with just a literal is pure
        let expr = make_expr(ExprKind::Literal(LiteralValue::Int(42)));
        let body = make_body(expr);

        let row = infer_effects(&body);
        assert!(row.is_pure(), "Expected pure function, got: {:?}", row);
    }

    #[test]
    fn test_single_effect_inference() {
        // A function with one perform operation
        let effect_id = DefId::new(1);
        let expr = make_expr(ExprKind::Perform {
            effect_id,
            op_index: 0,
            args: vec![],
            type_args: vec![],
        });
        let body = make_body(expr);

        let row = infer_effects(&body);
        assert!(!row.is_pure(), "Expected effectful function");
        assert!(row.effects().any(|e| e.def_id == effect_id),
            "Expected effect {:?} in row", effect_id);
    }

    #[test]
    fn test_multiple_effects_inference() {
        // A function with multiple perform operations
        let effect1 = DefId::new(1);
        let effect2 = DefId::new(2);

        let block = make_expr(ExprKind::Block {
            stmts: vec![
                Stmt::Expr(make_expr(ExprKind::Perform {
                    effect_id: effect1,
                    op_index: 0,
                    args: vec![],
                    type_args: vec![],
                })),
            ],
            expr: Some(Box::new(make_expr(ExprKind::Perform {
                effect_id: effect2,
                op_index: 0,
                args: vec![],
                type_args: vec![],
            }))),
        });
        let body = make_body(block);

        let row = infer_effects(&body);
        assert!(!row.is_pure(), "Expected effectful function");

        let effects: Vec<_> = row.effects().collect();
        assert_eq!(effects.len(), 2, "Expected 2 effects, got: {:?}", effects);
    }

    #[test]
    fn test_effect_reduction_by_handler() {
        // A function with a handle expression that handles an effect
        let effect_id = DefId::new(1);
        let handler_id = DefId::new(2);

        let handled_body = make_expr(ExprKind::Perform {
            effect_id,
            op_index: 0,
            args: vec![],
            type_args: vec![],
        });

        let handler_instance = make_expr(ExprKind::Struct {
            def_id: handler_id,
            fields: vec![],
            base: None,
        });

        let handle_expr = make_expr(ExprKind::Handle {
            body: Box::new(handled_body),
            handler_id,
            handler_instance: Box::new(handler_instance),
        });
        let body = make_body(handle_expr);

        // Register the handler
        let mut inferencer = DetailedEffectInferencer::new();
        inferencer.register_handler(handler_id, effect_id);

        let result = inferencer.infer_effects_detailed(&body);

        // The effect should be handled - the handler's effect_id should be tracked
        assert!(result.handled_effects.contains(&effect_id),
            "Expected effect {:?} to be tracked as handled", effect_id);
    }

    #[test]
    fn test_polymorphic_detection() {
        // A function that calls a local (higher-order function)
        let local_id = LocalId::new(1);

        let call_expr = make_expr(ExprKind::Call {
            callee: Box::new(make_expr(ExprKind::Local(local_id))),
            args: vec![],
        });
        let body = make_body(call_expr);

        let row = infer_effects(&body);
        assert!(row.is_polymorphic(),
            "Expected polymorphic function due to higher-order call");
    }

    #[test]
    fn test_nested_effects() {
        // Effects inside control flow
        let effect_id = DefId::new(1);

        let if_expr = make_expr(ExprKind::If {
            condition: Box::new(make_expr(ExprKind::Literal(LiteralValue::Bool(true)))),
            then_branch: Box::new(make_expr(ExprKind::Perform {
                effect_id,
                op_index: 0,
                args: vec![],
                type_args: vec![],
            })),
            else_branch: Some(Box::new(make_expr(ExprKind::Literal(LiteralValue::Int(0))))),
        });
        let body = make_body(if_expr);

        let row = infer_effects(&body);
        assert!(!row.is_pure(), "Expected effectful function");
        assert!(row.effects().any(|e| e.def_id == effect_id),
            "Expected effect {:?} in row", effect_id);
    }

    #[test]
    fn test_verify_effects_subset_valid() {
        let effect = EffectRef::new(DefId::new(1));
        let inferred = EffectRow::single(effect.clone());
        let mut declared = EffectRow::pure();
        declared.add_effect(effect);

        let result = verify_effects_subset(&inferred, &declared);
        assert!(result.is_ok(), "Expected valid subset");
    }

    #[test]
    fn test_verify_effects_subset_invalid() {
        let effect1 = EffectRef::new(DefId::new(1));
        let effect2 = EffectRef::new(DefId::new(2));

        let inferred = EffectRow::single(effect1.clone());
        let declared = EffectRow::single(effect2);

        let result = verify_effects_subset(&inferred, &declared);
        assert!(result.is_err(), "Expected invalid subset");
        let undeclared = result.unwrap_err();
        assert_eq!(undeclared.len(), 1);
        assert_eq!(undeclared[0].def_id, effect1.def_id);
    }

    #[test]
    fn test_verify_effects_subset_polymorphic() {
        // Polymorphic declared effects accept any inferred effects
        let effect = EffectRef::new(DefId::new(1));
        let inferred = EffectRow::single(effect);
        let declared = EffectRow::polymorphic(RowVar::new(0));

        let result = verify_effects_subset(&inferred, &declared);
        assert!(result.is_ok(), "Polymorphic declaration should accept any effects");
    }

    #[test]
    fn test_fresh_row_var() {
        let mut inferencer = EffectInferencer::new();
        let var1 = inferencer.fresh_row_var();
        let var2 = inferencer.fresh_row_var();

        assert_ne!(var1.0, var2.0, "Row variables should be unique");
    }

    #[test]
    fn test_inference_result_from_row() {
        let effect = EffectRef::new(DefId::new(1));
        let row = EffectRow::single(effect);

        let result = InferenceResult::from_row(row);
        assert!(result.has_effects);
        assert!(!result.is_polymorphic);
    }
}
