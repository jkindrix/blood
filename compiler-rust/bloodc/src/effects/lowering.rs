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
use super::infer::InferenceResult;
use super::row::EffectRow;
use crate::hir::{DefId, Expr, ExprKind, Item, ItemKind, Type, Generics, EffectOp, HandlerOp, Body};
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
    /// Handler name (used for content-based function naming).
    pub name: String,
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

                // Validate shallow handlers cannot multi-resume
                if handler_kind == HandlerKind::Shallow {
                    for (op_impl, op_hir) in op_impls.iter().zip(operations.iter()) {
                        if op_impl.resume_count > 1 {
                            return Err(LoweringError::new(format!(
                                "shallow handler cannot resume more than once \
                                 (found {} resume calls in operation '{}'); \
                                 use a deep handler for multi-shot semantics",
                                op_impl.resume_count, op_hir.name
                            )));
                        }
                    }
                }

                let info = HandlerInfo {
                    def_id: item.def_id,
                    name: item.name.clone(),
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

    /// Analyze a function's effect requirements by traversing its body.
    ///
    /// Delegates to `effects::infer::infer_effects` for a single-pass traversal
    /// that collects both performed effects and detects row polymorphism, avoiding
    /// the previous two-pass approach (separate effect collection + polymorphism
    /// detection).
    pub fn analyze_function(&mut self, def_id: DefId, body: &Body) -> EvidenceRequirement {
        let row = super::infer::infer_effects(body);
        let effects: Vec<DefId> = row.effects().map(|e| e.def_id).collect();
        let polymorphic = row.is_polymorphic();
        let req = EvidenceRequirement {
            effects,
            polymorphic,
            effect_row: Some(row),
        };
        self.evidence_reqs.insert(def_id, req.clone());
        req
    }

    /// Analyze a function from pre-computed inference results.
    ///
    /// Use this when the caller has already run effect inference (e.g., from
    /// `EffectInferencer::infer_effects` or `DetailedEffectInferencer`), to
    /// avoid re-traversing the HIR.
    pub fn analyze_function_from_inference(
        &mut self,
        def_id: DefId,
        result: &InferenceResult,
    ) -> EvidenceRequirement {
        let effects: Vec<DefId> = result.effect_row.effects().map(|e| e.def_id).collect();
        let req = EvidenceRequirement {
            effects,
            polymorphic: result.is_polymorphic,
            effect_row: Some(result.effect_row.clone()),
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
                // Type::error() is correct ICE recovery — the real error is propagated
                // via the `error` field, and the Error expr/type prevents further codegen
                // from misinterpreting this result.
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
                // Type::error() is correct ICE recovery — the real error is propagated
                // via the `error` field, and the Error expr/type prevents further codegen
                // from misinterpreting this result.
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
                    type_args: vec![],
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
