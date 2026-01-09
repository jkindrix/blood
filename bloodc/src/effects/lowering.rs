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
use std::collections::HashMap;

/// Effect lowering context.
///
/// Manages the translation of effectful HIR to effect-free code.
/// This is the main coordinator for Phase 2's effect compilation.
pub struct EffectLowering {
    /// Registered effect definitions.
    effects: HashMap<DefId, EffectInfo>,
    /// Mapping from effect DefId to its operations (for backward compat).
    effect_ops: HashMap<DefId, Vec<OperationInfo>>,
    /// Mapping from function DefId to its evidence requirements.
    evidence_reqs: HashMap<DefId, EvidenceRequirement>,
    /// Mapping from handler DefId to its compiled form.
    handlers: HashMap<DefId, HandlerInfo>,
    /// Counter for generating fresh variable names (used in Phase 2.4).
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
}

/// Information about an operation implementation in a handler.
#[derive(Debug, Clone)]
pub struct OpImplInfo {
    /// The operation being implemented.
    pub operation_id: DefId,
    /// Whether this implementation is tail-resumptive.
    pub is_tail_resumptive: bool,
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
    pub fn lower_handler_decl(&mut self, item: &Item) -> Option<HandlerInfo> {
        match &item.kind {
            ItemKind::Handler { effect, operations, .. } => {
                let op_impls: Vec<OpImplInfo> = operations
                    .iter()
                    .map(|op| self.lower_handler_op(op))
                    .collect();

                // Tail-resumptive analysis not yet available in HIR
                let all_tail = false;

                let info = HandlerInfo {
                    def_id: item.def_id,
                    effect_id: self.resolve_effect_type(effect),
                    kind: HandlerKind::Deep, // Default to deep handlers
                    op_impls,
                    all_tail_resumptive: all_tail,
                };

                self.handlers.insert(item.def_id, info.clone());
                Some(info)
            }
            _ => None,
        }
    }

    /// Lower a handler operation to OpImplInfo.
    fn lower_handler_op(&self, op: &HandlerOp) -> OpImplInfo {
        OpImplInfo {
            operation_id: DefId::new(0), // Resolved during type checking
            is_tail_resumptive: false,   // Analyzed during lowering pass
            body_id: Some(op.body_id),
        }
    }

    /// Resolve an effect type to its DefId.
    fn resolve_effect_type(&self, ty: &Type) -> DefId {
        // For now, assume effect types are ADTs with the effect DefId
        match ty.kind() {
            crate::hir::TypeKind::Adt { def_id, .. } => *def_id,
            _ => DefId::new(0), // Placeholder for unresolved
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
        let polymorphic = false; // TODO: Detect row polymorphism
        let req = EvidenceRequirement {
            effects,
            polymorphic,
            effect_row: None,
        };
        self.evidence_reqs.insert(def_id, req.clone());
        req
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
                        _ => {}
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
            ExprKind::Handle { body, .. } => {
                self.collect_effects_recursive(body, effects);
            }
            ExprKind::Resume { value: Some(e) } => {
                self.collect_effects_recursive(e, effects);
            }
            // Leaf expressions and others that don't contain subexpressions
            _ => {}
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
            _ => item.clone(),
        }
    }

    /// Transform an effectful function by adding evidence.
    fn transform_effectful_function(&mut self, item: &Item, _req: &EvidenceRequirement) -> Item {
        // TODO: Implement function transformation
        // 1. Add evidence parameter
        // 2. Transform body to use evidence
        item.clone()
    }

    /// Lower a `perform` operation to an evidence lookup.
    pub fn lower_perform(
        &mut self,
        effect_id: DefId,
        operation: &str,
        _args: Vec<Expr>,
    ) -> LoweringResult {
        // Look up the operation
        if let Some(ops) = self.effect_ops.get(&effect_id) {
            if let Some(_op) = ops.iter().find(|o| o.name == operation) {
                // Transform to evidence lookup
                // ev.effect.operation(args)
                // TODO: Generate proper evidence lookup expression
            }
        }

        // Placeholder - return an empty tuple expression (unit)
        LoweringResult {
            expr: Expr {
                kind: ExprKind::Tuple(Vec::new()),
                ty: Type::unit(),
                span: crate::span::Span::dummy(),
            },
            needs_evidence: true,
        }
    }

    /// Lower a `with...handle` block.
    pub fn lower_handler_block(
        &mut self,
        _handler_kind: HandlerKind,
        _handler_id: DefId,
        _body: Expr,
    ) -> LoweringResult {
        // TODO: Implement handler block lowering
        // 1. Create evidence for the handler
        // 2. Push evidence scope
        // 3. Execute body with evidence
        // 4. Handle return clause

        LoweringResult {
            expr: Expr {
                kind: ExprKind::Tuple(Vec::new()),
                ty: Type::unit(),
                span: crate::span::Span::dummy(),
            },
            needs_evidence: false,
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
    pub fn build_evidence(&self, effects: &[DefId]) -> EvidenceVector {
        let mut ev = EvidenceVector::new();
        for &effect_id in effects {
            // TODO: Look up actual handler implementations
            ev.add(
                super::row::EffectRef::new(effect_id),
                DefId::new(0), // Placeholder
            );
        }
        ev
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
    fn test_build_evidence() {
        let lowering = EffectLowering::new();
        let effects = vec![DefId::new(1), DefId::new(2)];

        let ev = lowering.build_evidence(&effects);

        assert_eq!(ev.len(), 2);
    }
}
