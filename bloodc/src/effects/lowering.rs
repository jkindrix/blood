//! # Effect Lowering
//!
//! Translates effectful HIR to effect-free code via evidence passing.
//!
//! ## Translation Process
//!
//! The effect lowering pass transforms effectful code by:
//!
//! 1. Adding evidence parameters to effectful functions
//! 2. Replacing `perform` operations with evidence lookups
//! 3. Transforming `with...handle` blocks into handler invocations
//! 4. Applying tail-resumptive optimizations where applicable
//!
//! ## Technical Approach
//!
//! Based on [Generalized Evidence Passing for Effect Handlers](https://dl.acm.org/doi/10.1145/3473576)
//! (ICFP'21) and [Efficient Compilation of Algebraic Effect Handlers](https://dl.acm.org/doi/10.1145/3485479)
//! (OOPSLA'21).
//!
//! ## Example Translation
//!
//! ```text
//! // Before lowering
//! fn counter() / {State<i32>} -> i32 {
//!     let x = get()
//!     put(x + 1)
//!     get()
//! }
//!
//! // After lowering
//! fn counter(ev: Evidence) -> i32 {
//!     let x = ev.state.get()
//!     ev.state.put(x + 1)
//!     ev.state.get()
//! }
//! ```
//!
//! ## Effect Declaration Lowering
//!
//! Effect declarations are lowered to operation tables for evidence lookup:
//!
//! ```text
//! // Source
//! effect State<T> {
//!     fn get() -> T
//!     fn put(value: T) -> ()
//! }
//!
//! // Lowered to operation table
//! EffectInfo {
//!     def_id: ...,
//!     name: "State",
//!     type_params: ["T"],
//!     operations: [
//!         OperationInfo { name: "get", params: [], return_ty: T },
//!         OperationInfo { name: "put", params: [T], return_ty: () },
//!     ],
//! }
//! ```

use super::evidence::EvidenceVector;
use super::handler::HandlerKind;
use super::row::EffectRow;
use crate::hir::{DefId, Expr, ExprKind, Item, ItemKind, Type, Generics, EffectOp, HandlerOp};
use crate::ice;
use std::collections::HashMap;

/// Effect lowering context.
///
/// Manages the translation of effectful HIR to effect-free code.
/// This is the main coordinator for effect compilation.
pub struct EffectLowering {
    /// Registered effect definitions.
    effects: HashMap<DefId, EffectInfo>,
    /// Mapping from effect DefId to its operations (for backward compat).
    effect_ops: HashMap<DefId, Vec<OperationInfo>>,
    /// Mapping from function DefId to its evidence requirements.
    evidence_reqs: HashMap<DefId, EvidenceRequirement>,
    /// Mapping from handler DefId to its compiled form.
    handlers: HashMap<DefId, HandlerInfo>,
    /// Counter for generating fresh variable names.
    #[allow(dead_code)]
    fresh_counter: u64,
}

/// Complete information about an effect declaration.
#[derive(Debug, Clone)]
pub struct EffectInfo {
    /// The effect's definition ID.
    pub def_id: DefId,
    /// Effect name.
    pub name: String,
    /// Type parameters (e.g., T in State<T>).
    pub generics: Option<Generics>,
    /// Effects this effect extends (inheritance).
    pub extends: Vec<DefId>,
    /// Operations defined by this effect.
    pub operations: Vec<OperationInfo>,
    /// Evidence index in the evidence vector.
    pub evidence_index: Option<usize>,
}

/// Information about an effect operation.
#[derive(Debug, Clone)]
pub struct OperationInfo {
    /// The operation DefId.
    pub def_id: DefId,
    /// Operation name.
    pub name: String,
    /// Parameter types.
    pub params: Vec<Type>,
    /// Return type.
    pub return_ty: Type,
    /// Index within the effect's operation table.
    pub op_index: usize,
}

/// Evidence requirement for a function.
#[derive(Debug, Clone)]
pub struct EvidenceRequirement {
    /// Effects that require evidence.
    pub effects: Vec<DefId>,
    /// Whether the function is polymorphic in effects.
    pub polymorphic: bool,
    /// The effect row for this function.
    pub effect_row: Option<EffectRow>,
}

/// Compiled handler information.
#[derive(Debug, Clone)]
pub struct HandlerInfo {
    /// Handler definition ID.
    pub def_id: DefId,
    /// The effect being handled.
    pub effect_id: DefId,
    /// Handler kind (deep/shallow).
    pub kind: HandlerKind,
    /// Operation implementations.
    pub op_impls: Vec<OpImplInfo>,
    /// Whether all operations are tail-resumptive.
    pub all_tail_resumptive: bool,
    /// Return clause body ID, if present.
    pub return_clause: Option<crate::hir::BodyId>,
}

/// Information about an operation implementation in a handler.
#[derive(Debug, Clone)]
pub struct OpImplInfo {
    /// The operation being implemented.
    pub operation_id: DefId,
    /// Whether this implementation is tail-resumptive.
    pub is_tail_resumptive: bool,
    /// Number of resume calls in the implementation.
    /// 0 = abort handler, 1 = single-shot, >1 = multi-shot
    pub resume_count: usize,
    /// Body expression ID (for lowering).
    pub body_id: Option<crate::hir::BodyId>,
}

/// Result of lowering an expression.
#[derive(Debug)]
pub struct LoweringResult {
    /// The lowered expression.
    pub expr: Expr,
    /// Whether evidence is needed.
    pub needs_evidence: bool,
    /// Error message if lowering failed.
    pub error: Option<LoweringError>,
}

/// Error from effect lowering.
#[derive(Debug, Clone)]
pub struct LoweringError {
    /// Error message.
    pub message: String,
    /// Whether this is an internal compiler error (should not happen with valid code).
    pub is_ice: bool,
}

impl LoweringError {
    /// Create a new lowering error.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            is_ice: false,
        }
    }

    /// Create an internal compiler error.
    pub fn ice(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            is_ice: true,
        }
    }
}

impl EffectLowering {
    /// Create a new effect lowering context.
    pub fn new() -> Self {
        Self {
            effects: HashMap::new(),
            effect_ops: HashMap::new(),
            evidence_reqs: HashMap::new(),
            handlers: HashMap::new(),
            fresh_counter: 0,
        }
    }

    // ========================================================================
    // Effect Declaration Lowering
    // ========================================================================

    /// Lower an effect declaration item to EffectInfo.
    ///
    /// This extracts effect metadata from HIR and registers it for
    /// evidence passing compilation.
    pub fn lower_effect_decl(&mut self, item: &Item) -> Option<EffectInfo> {
        match &item.kind {
            ItemKind::Effect { generics, operations } => {
                let ops: Vec<OperationInfo> = operations
                    .iter()
                    .enumerate()
                    .map(|(idx, op)| self.lower_effect_op(op, idx))
                    .collect();

                let info = EffectInfo {
                    def_id: item.def_id,
                    name: item.name.clone(),
                    generics: Some(generics.clone()),
                    extends: Vec::new(), // Effect inheritance tracked separately
                    operations: ops.clone(),
                    evidence_index: None,
                };

                // Register in both maps for compatibility
                self.effects.insert(item.def_id, info.clone());
                self.effect_ops.insert(item.def_id, ops);

                Some(info)
            }
            _ => None,
        }
    }

    /// Lower an effect operation to OperationInfo.
    fn lower_effect_op(&self, op: &EffectOp, index: usize) -> OperationInfo {
        OperationInfo {
            def_id: op.def_id,
            name: op.name.clone(),
            params: op.inputs.clone(),
            return_ty: op.output.clone(),
            op_index: index,
        }
    }

    /// Lower a handler declaration item to HandlerInfo.
    ///
    /// Returns an error if the handler references an unresolved effect type
    /// or if any operation cannot be resolved.
    ///
    /// The `bodies` parameter is used for tail-resumptive analysis. If None,
    /// all operations are conservatively assumed to be non-tail-resumptive.
    pub fn lower_handler_decl(
        &mut self,
        item: &Item,
        bodies: Option<&std::collections::HashMap<crate::hir::BodyId, crate::hir::Body>>,
    ) -> Result<HandlerInfo, LoweringError> {
        match &item.kind {
            ItemKind::Handler { kind, effect, operations, return_clause, .. } => {
                let effect_id = match self.resolve_effect_type(effect) {
                    Some(id) => id,
                    None => {
                        // Effect type could not be resolved - this is a type checking error
                        // that should have been caught earlier
                        return Err(LoweringError::ice(format!(
                            "Handler references unresolved effect type at {:?}. \
                             Type checking should have validated the effect exists.",
                            item.span
                        )));
                    }
                };

                // Collect operation implementations, propagating any errors
                let mut op_impls = Vec::with_capacity(operations.len());
                for op in operations {
                    op_impls.push(self.lower_handler_op(op, effect_id, bodies)?);
                }

                // Tail-resumptive analysis: check if all ops are tail-resumptive
                let all_tail = op_impls.iter().all(|op| op.is_tail_resumptive);

                // Convert hir::HandlerKind to effects::handler::HandlerKind
                let handler_kind = match kind {
                    crate::hir::HandlerKind::Deep => HandlerKind::Deep,
                    crate::hir::HandlerKind::Shallow => HandlerKind::Shallow,
                };

                let info = HandlerInfo {
                    def_id: item.def_id,
                    effect_id,
                    kind: handler_kind,
                    op_impls,
                    all_tail_resumptive: all_tail,
                    return_clause: return_clause.as_ref().map(|rc| rc.body_id),
                };

                self.handlers.insert(item.def_id, info.clone());
                Ok(info)
            }
            _ => Err(LoweringError::new(format!(
                "Expected handler item, got {:?}",
                item.kind
            ))),
        }
    }

    /// Lower a handler operation to OpImplInfo.
    ///
    /// Returns an error if the operation cannot be found in the effect definition.
    ///
    /// If `bodies` is provided, performs tail-resumptive analysis on the operation body.
    fn lower_handler_op(
        &self,
        op: &HandlerOp,
        effect_id: DefId,
        bodies: Option<&std::collections::HashMap<crate::hir::BodyId, crate::hir::Body>>,
    ) -> Result<OpImplInfo, LoweringError> {
        // Look up the effect info
        let effect_info = match self.effects.get(&effect_id) {
            Some(info) => info,
            None => {
                return Err(LoweringError::ice(format!(
                    "Effect {:?} not registered during handler operation lowering. \
                     This is an internal compiler error.",
                    effect_id
                )));
            }
        };

        // Look up the operation in the effect's operations
        let operation_id = match effect_info.operations.iter()
            .find(|op_info| op_info.name == op.name)
            .map(|op_info| op_info.def_id)
        {
            Some(id) => id,
            None => {
                return Err(LoweringError::ice(format!(
                    "Handler operation '{}' not found in effect {:?}. \
                     Type checking should have validated this operation exists.",
                    op.name, effect_id
                )));
            }
        };

        // Analyze if this operation is tail-resumptive and count resumes
        let (is_tail_resumptive, resume_count) = bodies
            .and_then(|b| b.get(&op.body_id))
            .map(|body| {
                let is_tail = super::handler::analyze_tail_resumptive(&body.expr);
                let count = super::handler::count_resumes_in_expr(&body.expr);
                (is_tail, count)
            })
            .unwrap_or((false, 0));

        Ok(OpImplInfo {
            operation_id,
            is_tail_resumptive,
            resume_count,
            body_id: Some(op.body_id),
        })
    }

    /// Resolve an effect type to its DefId.
    /// Returns None if the type is not a valid effect type (not an ADT).
    fn resolve_effect_type(&self, ty: &Type) -> Option<DefId> {
        match ty.kind() {
            crate::hir::TypeKind::Adt { def_id, .. } => Some(*def_id),
            _ => None,
        }
    }

    // ========================================================================
    // Effect Registration and Lookup
    // ========================================================================

    /// Register an effect and its operations (legacy API).
    pub fn register_effect(&mut self, effect_id: DefId, operations: Vec<OperationInfo>) {
        self.effect_ops.insert(effect_id, operations);
    }

    /// Get effect info by DefId.
    pub fn get_effect(&self, effect_id: DefId) -> Option<&EffectInfo> {
        self.effects.get(&effect_id)
    }

    /// Get handler info by DefId.
    pub fn get_handler(&self, handler_id: DefId) -> Option<&HandlerInfo> {
        self.handlers.get(&handler_id)
    }

    /// Find all handlers for a given effect.
    pub fn handlers_for_effect(&self, effect_id: DefId) -> Vec<&HandlerInfo> {
        self.handlers
            .values()
            .filter(|h| h.effect_id == effect_id)
            .collect()
    }

    // ========================================================================
    // Function Analysis
    // ========================================================================

    /// Analyze a function's effect requirements.
    pub fn analyze_function(&mut self, def_id: DefId, body: &Expr) -> EvidenceRequirement {
        let effects = self.collect_effects(body);
        // Detect row polymorphism: if the function calls parameters that might be closures,
        // it could be propagating their effects. This is a heuristic - full detection
        // would require tracking effect variables during type checking.
        let polymorphic = self.detect_row_polymorphism(body);
        let req = EvidenceRequirement {
            effects,
            polymorphic,
            effect_row: None,
        };
        self.evidence_reqs.insert(def_id, req.clone());
        req
    }

    /// Detect if a function body might be row-polymorphic.
    ///
    /// A function is considered potentially row-polymorphic if it:
    /// 1. Calls a local variable (which might be a closure parameter)
    /// 2. Uses handle expressions (which capture and transform effects)
    ///
    /// This is a conservative heuristic - it may over-approximate polymorphism
    /// but won't miss actual polymorphic functions.
    fn detect_row_polymorphism(&self, expr: &Expr) -> bool {
        self.detect_row_poly_recursive(expr)
    }

    fn detect_row_poly_recursive(&self, expr: &Expr) -> bool {
        use crate::hir::Stmt;

        match &expr.kind {
            // Handle expressions are inherently effect-polymorphic
            ExprKind::Handle { .. } => true,

            // Check if callee is a local variable (could be a closure parameter)
            ExprKind::Call { callee, args } => {
                // If calling a local variable, it might be a closure with effects
                let callee_is_local = matches!(&callee.kind, ExprKind::Local(_));
                if callee_is_local {
                    return true;
                }
                // Otherwise, recurse into subexpressions
                self.detect_row_poly_recursive(callee)
                    || args.iter().any(|a| self.detect_row_poly_recursive(a))
            }

            // Recurse into compound expressions
            ExprKind::Block { stmts, expr: tail } => {
                stmts.iter().any(|stmt| match stmt {
                    Stmt::Expr(e) => self.detect_row_poly_recursive(e),
                    Stmt::Let { init: Some(e), .. } => self.detect_row_poly_recursive(e),
                    // Let without init and Item statements don't contain row-polymorphic calls
                    Stmt::Let { init: None, .. } => false,
                    Stmt::Item(_) => false,
                }) || tail.as_ref().is_some_and(|e| self.detect_row_poly_recursive(e))
            }
            ExprKind::If { condition, then_branch, else_branch } => {
                self.detect_row_poly_recursive(condition)
                    || self.detect_row_poly_recursive(then_branch)
                    || else_branch.as_ref().is_some_and(|e| self.detect_row_poly_recursive(e))
            }
            ExprKind::Match { scrutinee, arms } => {
                self.detect_row_poly_recursive(scrutinee)
                    || arms.iter().any(|a| self.detect_row_poly_recursive(&a.body))
            }
            ExprKind::Loop { body, .. } | ExprKind::While { body, .. } => {
                self.detect_row_poly_recursive(body)
            }
            ExprKind::Binary { left, right, .. } => {
                self.detect_row_poly_recursive(left) || self.detect_row_poly_recursive(right)
            }
            ExprKind::Unary { operand, .. } => self.detect_row_poly_recursive(operand),
            ExprKind::Tuple(exprs) => exprs.iter().any(|e| self.detect_row_poly_recursive(e)),
            ExprKind::Return(Some(e)) | ExprKind::Assign { value: e, .. } => {
                self.detect_row_poly_recursive(e)
            }
            ExprKind::Perform { args, .. } => {
                args.iter().any(|a| self.detect_row_poly_recursive(a))
            }
            ExprKind::Resume { value: Some(e) } => self.detect_row_poly_recursive(e),
            ExprKind::Resume { value: None } => false,
            ExprKind::Return(None) => false,
            // Handle is already matched above (line 439) as inherently polymorphic
            ExprKind::Break { value, .. } => {
                value.as_ref().is_some_and(|e| self.detect_row_poly_recursive(e))
            }
            ExprKind::Field { base, .. } => self.detect_row_poly_recursive(base),
            ExprKind::Index { base, index } => {
                self.detect_row_poly_recursive(base) || self.detect_row_poly_recursive(index)
            }
            ExprKind::Array(exprs) => exprs.iter().any(|e| self.detect_row_poly_recursive(e)),
            ExprKind::Repeat { value, .. } => self.detect_row_poly_recursive(value),
            ExprKind::Struct { fields, base, .. } => {
                fields.iter().any(|f| self.detect_row_poly_recursive(&f.value))
                    || base.as_ref().is_some_and(|e| self.detect_row_poly_recursive(e))
            }
            ExprKind::Variant { fields, .. } => {
                fields.iter().any(|e| self.detect_row_poly_recursive(e))
            }
            ExprKind::Record { fields } => {
                fields.iter().any(|f| self.detect_row_poly_recursive(&f.value))
            }
            ExprKind::Cast { expr, .. } => self.detect_row_poly_recursive(expr),
            ExprKind::Borrow { expr, .. } => self.detect_row_poly_recursive(expr),
            ExprKind::Deref(expr) => self.detect_row_poly_recursive(expr),
            ExprKind::AddrOf { expr, .. } => self.detect_row_poly_recursive(expr),
            ExprKind::Let { init, .. } => self.detect_row_poly_recursive(init),
            ExprKind::Unsafe(expr) => self.detect_row_poly_recursive(expr),
            ExprKind::Range { start, end, .. } => {
                start.as_ref().is_some_and(|e| self.detect_row_poly_recursive(e))
                    || end.as_ref().is_some_and(|e| self.detect_row_poly_recursive(e))
            }
            ExprKind::MethodCall { receiver, args, .. } => {
                self.detect_row_poly_recursive(receiver)
                    || args.iter().any(|a| self.detect_row_poly_recursive(a))
            }
            ExprKind::Closure { .. } => {
                // Closures have their own effect analysis
                false
            }
            // Leaf expressions are not polymorphic
            ExprKind::Literal(_) | ExprKind::Local(_) | ExprKind::Def(_)
            | ExprKind::MethodFamily { .. } | ExprKind::Continue { .. } | ExprKind::Error => false,
        }
    }

    /// Analyze a function with its declared effect row.
    pub fn analyze_function_with_row(
        &mut self,
        def_id: DefId,
        effect_row: EffectRow,
    ) -> EvidenceRequirement {
        let effects: Vec<DefId> = effect_row
            .effects()
            .map(|e| e.def_id)
            .collect();
        let polymorphic = effect_row.is_polymorphic();
        let req = EvidenceRequirement {
            effects,
            polymorphic,
            effect_row: Some(effect_row),
        };
        self.evidence_reqs.insert(def_id, req.clone());
        req
    }

    /// Collect all effects used in an expression.
    fn collect_effects(&self, expr: &Expr) -> Vec<DefId> {
        let mut effects = Vec::new();
        self.collect_effects_recursive(expr, &mut effects);
        effects
    }

    /// Recursively collect effects from an expression.
    ///
    /// Note: Effect expression kinds (Perform, WithHandle) will be added
    /// as part of the Phase 2 HIR extensions. For now, we traverse
    /// known expression kinds.
    fn collect_effects_recursive(&self, expr: &Expr, effects: &mut Vec<DefId>) {
        use crate::hir::Stmt;

        match &expr.kind {
            ExprKind::Block { stmts, expr: tail } => {
                for stmt in stmts {
                    match stmt {
                        Stmt::Expr(e) => self.collect_effects_recursive(e, effects),
                        Stmt::Let { init: Some(e), .. } => {
                            self.collect_effects_recursive(e, effects);
                        }
                        // Let without init and Item statements don't contain effect operations
                        Stmt::Let { init: None, .. } => {}
                        Stmt::Item(_) => {}
                    }
                }
                if let Some(e) = tail {
                    self.collect_effects_recursive(e, effects);
                }
            }
            ExprKind::If { condition, then_branch, else_branch } => {
                self.collect_effects_recursive(condition, effects);
                self.collect_effects_recursive(then_branch, effects);
                if let Some(else_br) = else_branch {
                    self.collect_effects_recursive(else_br, effects);
                }
            }
            ExprKind::Match { scrutinee, arms } => {
                self.collect_effects_recursive(scrutinee, effects);
                for arm in arms {
                    self.collect_effects_recursive(&arm.body, effects);
                }
            }
            ExprKind::Call { callee, args } => {
                self.collect_effects_recursive(callee, effects);
                for arg in args {
                    self.collect_effects_recursive(arg, effects);
                }
            }
            ExprKind::Binary { left, right, .. } => {
                self.collect_effects_recursive(left, effects);
                self.collect_effects_recursive(right, effects);
            }
            ExprKind::Unary { operand, .. } => {
                self.collect_effects_recursive(operand, effects);
            }
            ExprKind::Tuple(exprs) => {
                for e in exprs {
                    self.collect_effects_recursive(e, effects);
                }
            }
            ExprKind::Loop { body, .. } | ExprKind::While { body, .. } => {
                self.collect_effects_recursive(body, effects);
            }
            ExprKind::Return(Some(e)) | ExprKind::Assign { value: e, .. } => {
                self.collect_effects_recursive(e, effects);
            }
            // Effect operations - this is where we actually collect effects
            ExprKind::Perform { effect_id, args, .. } => {
                effects.push(*effect_id);
                for arg in args {
                    self.collect_effects_recursive(arg, effects);
                }
            }
            ExprKind::Handle { body, handler_instance, .. } => {
                self.collect_effects_recursive(body, effects);
                self.collect_effects_recursive(handler_instance, effects);
            }
            ExprKind::Resume { value: Some(e) } => {
                self.collect_effects_recursive(e, effects);
            }
            ExprKind::Resume { value: None } => {}
            ExprKind::Return(None) => {}
            ExprKind::Break { value, .. } => {
                if let Some(e) = value {
                    self.collect_effects_recursive(e, effects);
                }
            }
            ExprKind::Field { base, .. } => {
                self.collect_effects_recursive(base, effects);
            }
            ExprKind::Index { base, index } => {
                self.collect_effects_recursive(base, effects);
                self.collect_effects_recursive(index, effects);
            }
            ExprKind::Array(exprs) => {
                for e in exprs {
                    self.collect_effects_recursive(e, effects);
                }
            }
            ExprKind::Repeat { value, .. } => {
                self.collect_effects_recursive(value, effects);
            }
            ExprKind::Struct { fields, base, .. } => {
                for field in fields {
                    self.collect_effects_recursive(&field.value, effects);
                }
                if let Some(b) = base {
                    self.collect_effects_recursive(b, effects);
                }
            }
            ExprKind::Variant { fields, .. } => {
                for field in fields {
                    self.collect_effects_recursive(field, effects);
                }
            }
            ExprKind::Record { fields } => {
                for field in fields {
                    self.collect_effects_recursive(&field.value, effects);
                }
            }
            ExprKind::Cast { expr, .. } => {
                self.collect_effects_recursive(expr, effects);
            }
            ExprKind::Borrow { expr, .. } => {
                self.collect_effects_recursive(expr, effects);
            }
            ExprKind::Deref(expr) => {
                self.collect_effects_recursive(expr, effects);
            }
            ExprKind::AddrOf { expr, .. } => {
                self.collect_effects_recursive(expr, effects);
            }
            ExprKind::Let { init, .. } => {
                self.collect_effects_recursive(init, effects);
            }
            ExprKind::Unsafe(expr) => {
                self.collect_effects_recursive(expr, effects);
            }
            ExprKind::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.collect_effects_recursive(s, effects);
                }
                if let Some(e) = end {
                    self.collect_effects_recursive(e, effects);
                }
            }
            ExprKind::MethodCall { receiver, args, .. } => {
                self.collect_effects_recursive(receiver, effects);
                for arg in args {
                    self.collect_effects_recursive(arg, effects);
                }
            }
            ExprKind::Closure { .. } => {
                // Closures have their own effect analysis
            }
            // Leaf expressions don't contain effect operations
            ExprKind::Literal(_) | ExprKind::Local(_) | ExprKind::Def(_)
            | ExprKind::MethodFamily { .. } | ExprKind::Continue { .. } | ExprKind::Error => {}
        }
    }

    /// Lower a function item by adding evidence parameters.
    pub fn lower_function(&mut self, item: &Item) -> Item {
        match &item.kind {
            ItemKind::Fn(fn_def) => {
                // Only analyze if there's a body
                if fn_def.body_id.is_some() {
                    let req = EvidenceRequirement {
                        effects: Vec::new(),
                        polymorphic: false,
                        effect_row: None,
                    };
                    self.evidence_reqs.insert(item.def_id, req.clone());
                    if !req.effects.is_empty() || req.polymorphic {
                        // Add evidence parameter
                        return self.transform_effectful_function(item, &req);
                    }
                }
                // Pure function, no transformation needed
                item.clone()
            }
            // Non-function items are not transformed for evidence passing
            ItemKind::Struct(_)
            | ItemKind::Enum(_)
            | ItemKind::TypeAlias { .. }
            | ItemKind::Const { .. }
            | ItemKind::Static { .. }
            | ItemKind::Trait { .. }
            | ItemKind::Impl { .. }
            | ItemKind::Effect { .. }
            | ItemKind::Handler { .. }
            | ItemKind::ExternFn(_)
            | ItemKind::Bridge(_)
            | ItemKind::Module(_) => item.clone(),
        }
    }

    /// Transform an effectful function by adding evidence parameter.
    ///
    /// Current: Runtime evidence passing (implicit).
    /// The evidence parameter is implicit - the runtime manages the evidence
    /// vector as thread-local state. Functions don't need modification.
    ///
    /// Future: Compile-time evidence passing (optimization).
    /// For optimization, we could add an explicit evidence parameter:
    /// ```text
    /// fn foo() / {State<i32>} -> i32
    /// becomes:
    /// fn foo(ev: *Evidence) -> i32
    /// ```
    fn transform_effectful_function(&mut self, item: &Item, req: &EvidenceRequirement) -> Item {
        // Current: Runtime evidence passing - no function signature changes needed.
        // The runtime's blood_evidence_current() provides the evidence vector.
        //
        // Future optimization: Add explicit evidence parameter for
        // zero-overhead effect handling when the handler is known at compile time.

        // Store the requirement for later use in codegen
        if let ItemKind::Fn(_) = &item.kind {
            self.evidence_reqs.insert(item.def_id, req.clone());
        }

        // Return unchanged - runtime handles evidence implicitly
        item.clone()
    }

    /// Lower a `perform` operation to an evidence lookup.
    ///
    /// Transforms: `perform Effect.operation(args)`
    /// To: `ExprKind::Perform { effect_id, op_index, args }`
    ///
    /// The codegen then translates this to a call to `blood_perform(effect_id, op_index, args)`.
    pub fn lower_perform(
        &mut self,
        effect_id: DefId,
        operation: &str,
        args: Vec<Expr>,
    ) -> LoweringResult {
        // Look up the effect operations
        let ops = match self.effect_ops.get(&effect_id) {
            Some(ops) => ops,
            None => {
                // Effect not registered - this is an ICE because type checking
                // should have validated that the effect exists
                ice!("effect not found during lowering";
                     "effect_id" => effect_id,
                     "note" => "type checking should have validated the effect exists");
                let error = LoweringError::ice(format!(
                    "Effect {:?} not found during lowering",
                    effect_id
                ));
                return LoweringResult {
                    expr: Expr {
                        kind: ExprKind::Error,
                        ty: Type::error(),
                        span: crate::span::Span::dummy(),
                    },
                    needs_evidence: false,
                    error: Some(error),
                };
            }
        };

        // Look up the operation index
        let (op_index, op_info) = match ops.iter().enumerate().find(|(_, o)| o.name == operation) {
            Some((idx, op)) => (idx as u32, op),
            None => {
                // Operation not found in effect - this is an ICE because type checking
                // should have validated that the operation exists
                ice!("operation not found in effect";
                     "operation" => operation,
                     "effect_id" => effect_id,
                     "note" => "type checking should have validated the operation exists");
                let error = LoweringError::ice(format!(
                    "Operation '{}' not found in effect {:?}",
                    operation, effect_id
                ));
                return LoweringResult {
                    expr: Expr {
                        kind: ExprKind::Error,
                        ty: Type::error(),
                        span: crate::span::Span::dummy(),
                    },
                    needs_evidence: false,
                    error: Some(error),
                };
            }
        };

        // Get the return type from the operation signature
        let return_ty = op_info.return_ty.clone();

        // Create the Perform expression
        LoweringResult {
            expr: Expr {
                kind: ExprKind::Perform {
                    effect_id,
                    op_index,
                    args,
                },
                ty: return_ty,
                span: crate::span::Span::dummy(),
            },
            needs_evidence: true,
            error: None,
        }
    }

    /// Lower a `with...handle` block.
    ///
    /// Transforms: `handle { body } with HandlerName`
    /// To: `ExprKind::Handle { body, handler_id, handler_instance }`
    ///
    /// The codegen then:
    /// 1. Creates an evidence vector via `blood_evidence_create()`
    /// 2. Evaluates handler_instance to get state pointer
    /// 3. Pushes the handler via `blood_evidence_push()`
    /// 4. Compiles the body
    /// 5. Pops the handler via `blood_evidence_pop()`
    /// 6. Destroys the evidence vector via `blood_evidence_destroy()`
    pub fn lower_handler_block(
        &mut self,
        _handler_kind: HandlerKind,
        handler_id: DefId,
        handler_instance: Expr,
        body: Expr,
    ) -> LoweringResult {
        // Get the return type from the body
        let return_ty = body.ty.clone();

        // Create the Handle expression
        LoweringResult {
            expr: Expr {
                kind: ExprKind::Handle {
                    body: Box::new(body),
                    handler_id,
                    handler_instance: Box::new(handler_instance),
                },
                ty: return_ty,
                span: crate::span::Span::dummy(),
            },
            needs_evidence: false, // Handler provides its own evidence
            error: None,
        }
    }

    /// Generate a fresh variable name (used in Phase 2.4).
    #[allow(dead_code)]
    fn fresh_name(&mut self, prefix: &str) -> String {
        let id = self.fresh_counter;
        self.fresh_counter += 1;
        format!("{}_{}", prefix, id)
    }

    /// Check if a function requires evidence.
    pub fn requires_evidence(&self, def_id: DefId) -> bool {
        self.evidence_reqs
            .get(&def_id)
            .map(|req| !req.effects.is_empty() || req.polymorphic)
            .unwrap_or(false)
    }

    /// Build evidence vector for a handler block.
    ///
    /// For each required effect, looks up registered handlers and adds them
    /// to the evidence vector. Returns an error if any effect has no handler.
    pub fn build_evidence(&self, effects: &[DefId]) -> Result<EvidenceVector, LoweringError> {
        let mut ev = EvidenceVector::new();
        for &effect_id in effects {
            // Look up handlers for this effect
            let handlers_for_effect = self.handlers_for_effect(effect_id);

            // Require a handler for each effect
            let handler_id = match handlers_for_effect.first() {
                Some(h) => h.def_id,
                None => {
                    // No handler found for this effect - this is a user error
                    return Err(LoweringError::new(format!(
                        "No handler found for effect {:?}. \
                         Effects must be handled before they can be performed.",
                        effect_id
                    )));
                }
            };

            ev.add(
                super::row::EffectRef::new(effect_id),
                handler_id,
            );
        }
        Ok(ev)
    }

    /// Build evidence with specific handler assignments.
    ///
    /// This is used when the caller knows exactly which handler to use
    /// for each effect (e.g., from a `handle` block).
    pub fn build_evidence_with_handlers(
        &self,
        effect_handler_pairs: &[(DefId, DefId)],
    ) -> EvidenceVector {
        let mut ev = EvidenceVector::new();
        for &(effect_id, handler_id) in effect_handler_pairs {
            ev.add(
                super::row::EffectRef::new(effect_id),
                handler_id,
            );
        }
        ev
    }

    /// Get all registered effects.
    pub fn all_effects(&self) -> impl Iterator<Item = &EffectInfo> {
        self.effects.values()
    }

    /// Get all registered handlers.
    pub fn all_handlers(&self) -> impl Iterator<Item = &HandlerInfo> {
        self.handlers.values()
    }
}

impl Default for EffectLowering {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_effect_lowering_new() {
        let lowering = EffectLowering::new();
        assert!(lowering.effect_ops.is_empty());
    }

    #[test]
    fn test_register_effect() {
        let mut lowering = EffectLowering::new();
        let effect_id = DefId::new(1);

        lowering.register_effect(
            effect_id,
            vec![OperationInfo {
                def_id: DefId::new(2),
                name: "get".to_string(),
                params: vec![],
                return_ty: Type::i32(),
                op_index: 0,
            }],
        );

        assert!(lowering.effect_ops.contains_key(&effect_id));
    }

    #[test]
    fn test_fresh_name() {
        let mut lowering = EffectLowering::new();

        let name1 = lowering.fresh_name("ev");
        let name2 = lowering.fresh_name("ev");

        assert_ne!(name1, name2);
    }

    #[test]
    fn test_build_evidence_no_handlers() {
        let lowering = EffectLowering::new();
        let effects = vec![DefId::new(1), DefId::new(2)];

        // Without registered handlers, build_evidence should return Err
        let ev = lowering.build_evidence(&effects);
        assert!(ev.is_err(), "build_evidence should return Err when no handlers are registered");
        let err = ev.unwrap_err();
        assert!(!err.is_ice, "Missing handler is a user error, not an ICE");
    }

    #[test]
    fn test_build_evidence_empty() {
        let lowering = EffectLowering::new();

        // Empty effects list should succeed with empty evidence
        let ev = lowering.build_evidence(&[]);
        assert!(ev.is_ok());
        assert_eq!(ev.unwrap().len(), 0);
    }
}
