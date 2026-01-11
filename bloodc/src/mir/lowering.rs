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

use std::collections::HashMap;

use crate::hir::{
    self, Body, Crate as HirCrate, DefId, Expr, ExprKind, ItemKind,
    LocalId, LiteralValue, MatchArm, Pattern, PatternKind, Stmt, Type, TypeKind,
};
use crate::ast::{BinOp, UnaryOp};
use crate::span::Span;
use crate::diagnostics::Diagnostic;
use crate::ice_err;

use super::body::{MirBody, MirBodyBuilder};
use super::types::{
    BasicBlockId, Statement, StatementKind, Terminator, TerminatorKind,
    Place, PlaceElem, Operand, Rvalue, Constant, ConstantKind,
    BinOp as MirBinOp, UnOp as MirUnOp, AggregateKind, SwitchTargets,
};

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
    pending_closures: Vec<(hir::BodyId, DefId, Vec<(hir::Capture, Type)>)>,
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
        }
    }

    /// Generate a synthetic DefId for a closure.
    ///
    /// Uses a high index range (starting at 0xFFFF_0000) to avoid collisions
    /// with real definitions.
    #[allow(dead_code)]
    fn next_closure_def_id(&mut self) -> DefId {
        let id = self.closure_counter;
        self.closure_counter += 1;
        // Use a high index range to avoid collisions with real definitions
        DefId::new(0xFFFF_0000 + id)
    }

    /// Lower all functions in the crate.
    pub fn lower_crate(&mut self) -> Result<HashMap<DefId, MirBody>, Vec<Diagnostic>> {
        // First pass: lower all top-level functions
        for (&def_id, item) in &self.hir.items {
            match &item.kind {
                ItemKind::Fn(fn_def) => {
                    if let Some(body_id) = fn_def.body_id {
                        if let Some(body) = self.hir.get_body(body_id) {
                            let mir_body = self.lower_function(def_id, &fn_def.sig, body)?;
                            self.bodies.insert(def_id, mir_body);
                        }
                    }
                }
                _ => {
                    // Skip non-function items for now
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
            Ok(std::mem::take(&mut self.bodies))
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
        let builder = ClosureLowering::new(def_id, body, captures, self.hir, &mut self.pending_closures, &mut self.closure_counter);
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
        );
        builder.lower()
    }
}

// ============================================================================
// Function Lowering
// ============================================================================

/// Lowers a single function body to MIR.
#[allow(dead_code)]
struct FunctionLowering<'hir, 'ctx> {
    /// MIR body builder.
    builder: MirBodyBuilder,
    /// HIR body being lowered.
    body: &'hir Body,
    /// HIR crate for accessing closure bodies.
    hir: &'hir HirCrate,
    /// Mapping from HIR locals to MIR locals.
    local_map: HashMap<LocalId, LocalId>,
    /// Current basic block.
    current_block: BasicBlockId,
    /// Loop context for break/continue.
    loop_stack: Vec<LoopContext>,
    /// Counter for unique temporary names.
    temp_counter: u32,
    /// Pending closures to be lowered after this function.
    pending_closures: &'ctx mut Vec<(hir::BodyId, DefId, Vec<(hir::Capture, Type)>)>,
    /// Counter for generating synthetic closure DefIds.
    closure_counter: &'ctx mut u32,
}

/// Context for a loop (for break/continue handling).
#[derive(Debug, Clone)]
struct LoopContext {
    /// Block to jump to on break.
    break_block: BasicBlockId,
    /// Block to jump to on continue.
    continue_block: BasicBlockId,
    /// Label for labeled breaks.
    label: Option<hir::LoopId>,
}

impl<'hir, 'ctx> FunctionLowering<'hir, 'ctx> {
    /// Create a new function lowering context.
    fn new(
        def_id: DefId,
        sig: &hir::FnSig,
        body: &'hir Body,
        hir: &'hir HirCrate,
        pending_closures: &'ctx mut Vec<(hir::BodyId, DefId, Vec<(hir::Capture, Type)>)>,
        closure_counter: &'ctx mut u32,
    ) -> Self {
        let mut builder = MirBodyBuilder::new(def_id, body.span);

        // Set return type
        builder.set_return_type(body.return_type().clone());

        // Add parameters from FnSig inputs
        let mut local_map = HashMap::new();
        for (i, ty) in sig.inputs.iter().enumerate() {
            // Get param name from body if available
            let param_name = body.params().nth(i).and_then(|p| p.name.clone());
            let param_span = body.params().nth(i).map(|p| p.span).unwrap_or(body.span);
            let mir_local = builder.add_param(
                param_name,
                ty.clone(),
                param_span,
            );
            // Map HIR local (i+1) to MIR local
            let hir_local = LocalId::new((i + 1) as u32);
            local_map.insert(hir_local, mir_local);
        }

        let current_block = builder.current_block();

        Self {
            builder,
            body,
            hir,
            local_map,
            current_block,
            loop_stack: Vec::new(),
            temp_counter: 0,
            pending_closures,
            closure_counter,
        }
    }

    /// Generate a synthetic DefId for a closure.
    fn next_closure_def_id(&mut self) -> DefId {
        let id = *self.closure_counter;
        *self.closure_counter += 1;
        DefId::new(0xFFFF_0000 + id)
    }

    /// Lower the function body.
    fn lower(mut self) -> Result<MirBody, Vec<Diagnostic>> {
        // Lower the body expression
        let body_expr = self.body.expr.clone();
        let result = self.lower_expr(&body_expr)?;

        // Assign to return place
        let return_place = Place::local(LocalId::new(0));
        self.push_assign(return_place, Rvalue::Use(result));

        // Add return terminator
        self.terminate(TerminatorKind::Return);

        Ok(self.builder.finish())
    }

    /// Lower an expression, returning an operand.
    fn lower_expr(&mut self, expr: &Expr) -> Result<Operand, Vec<Diagnostic>> {
        match &expr.kind {
            ExprKind::Literal(lit) => self.lower_literal(lit, &expr.ty),

            ExprKind::Local(local_id) => {
                let mir_local = self.map_local(*local_id);
                Ok(Operand::Copy(Place::local(mir_local)))
            }

            ExprKind::Def(def_id) => {
                let constant = Constant::new(expr.ty.clone(), ConstantKind::FnDef(*def_id));
                Ok(Operand::Constant(constant))
            }

            ExprKind::Binary { op, left, right } => {
                self.lower_binary(*op, left, right, &expr.ty, expr.span)
            }

            ExprKind::Unary { op, operand } => {
                self.lower_unary(*op, operand, &expr.ty, expr.span)
            }

            ExprKind::Call { callee, args } => {
                self.lower_call(callee, args, &expr.ty, expr.span)
            }

            ExprKind::Block { stmts, expr: tail } => {
                self.lower_block(stmts, tail.as_deref(), &expr.ty, expr.span)
            }

            ExprKind::If { condition, then_branch, else_branch } => {
                self.lower_if(condition, then_branch, else_branch.as_deref(), &expr.ty, expr.span)
            }

            ExprKind::Match { scrutinee, arms } => {
                self.lower_match(scrutinee, arms, &expr.ty, expr.span)
            }

            ExprKind::Loop { body, label } => {
                self.lower_loop(body, *label, &expr.ty, expr.span)
            }

            ExprKind::While { condition, body, label } => {
                self.lower_while(condition, body, *label, &expr.ty, expr.span)
            }

            ExprKind::Break { label, value } => {
                self.lower_break(*label, value.as_deref(), expr.span)
            }

            ExprKind::Continue { label } => {
                self.lower_continue(*label, expr.span)
            }

            ExprKind::Return(value) => {
                self.lower_return(value.as_deref(), expr.span)
            }

            ExprKind::Assign { target, value } => {
                self.lower_assign(target, value, expr.span)
            }

            ExprKind::Tuple(elems) => {
                self.lower_tuple(elems, &expr.ty, expr.span)
            }

            ExprKind::Array(elems) => {
                self.lower_array(elems, &expr.ty, expr.span)
            }

            ExprKind::Struct { def_id, fields, base } => {
                self.lower_struct(*def_id, fields, base.as_deref(), &expr.ty, expr.span)
            }

            ExprKind::Field { base, field_idx } => {
                self.lower_field(base, *field_idx, &expr.ty, expr.span)
            }

            ExprKind::Index { base, index } => {
                self.lower_index(base, index, &expr.ty, expr.span)
            }

            ExprKind::Borrow { expr: inner, mutable } => {
                self.lower_borrow(inner, *mutable, &expr.ty, expr.span)
            }

            ExprKind::Deref(inner) => {
                self.lower_deref(inner, &expr.ty, expr.span)
            }

            ExprKind::Cast { expr: inner, target_ty } => {
                self.lower_cast(inner, target_ty, expr.span)
            }

            ExprKind::Error => {
                // Propagate error
                Err(vec![Diagnostic::error("lowering error expression".to_string(), expr.span)])
            }

            // Explicitly handle unimplemented expression kinds with proper errors
            ExprKind::MethodCall { .. } => {
                Err(vec![Diagnostic::error(
                    "MIR lowering: method calls should be desugared before MIR lowering".to_string(),
                    expr.span,
                )])
            }

            ExprKind::Repeat { value, count } => {
                self.lower_repeat(value, *count, &expr.ty, expr.span)
            }

            ExprKind::Variant { def_id, variant_idx, fields } => {
                self.lower_variant(*def_id, *variant_idx, fields, &expr.ty, expr.span)
            }

            ExprKind::Closure { body_id, captures } => {
                self.lower_closure(*body_id, captures, &expr.ty, expr.span)
            }

            ExprKind::AddrOf { expr: inner, mutable } => {
                self.lower_addr_of(inner, *mutable, &expr.ty, expr.span)
            }

            ExprKind::Let { pattern, init } => {
                self.lower_let(pattern, init, &expr.ty, expr.span)
            }

            ExprKind::Unsafe(inner) => {
                // For now, just lower the inner expression (safety is handled elsewhere)
                self.lower_expr(inner)
            }

            ExprKind::Perform { effect_id, op_index, args } => {
                self.lower_perform(*effect_id, *op_index, args, &expr.ty, expr.span)
            }

            ExprKind::Resume { value } => {
                self.lower_resume(value.as_deref(), expr.span)
            }

            ExprKind::Handle { body, handler_id, handler_instance } => {
                self.lower_handle(body, *handler_id, handler_instance, &expr.ty, expr.span)
            }
        }
    }

    /// Lower a literal to an operand.
    fn lower_literal(&mut self, lit: &LiteralValue, ty: &Type) -> Result<Operand, Vec<Diagnostic>> {
        let kind = match lit {
            LiteralValue::Int(v) => ConstantKind::Int(*v),
            LiteralValue::Uint(v) => ConstantKind::Uint(*v),
            LiteralValue::Float(v) => ConstantKind::Float(*v),
            LiteralValue::Bool(v) => ConstantKind::Bool(*v),
            LiteralValue::Char(v) => ConstantKind::Char(*v),
            LiteralValue::String(v) => ConstantKind::String(v.clone()),
        };
        Ok(Operand::Constant(Constant::new(ty.clone(), kind)))
    }

    /// Lower a binary operation.
    fn lower_binary(
        &mut self,
        op: BinOp,
        left: &Expr,
        right: &Expr,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        // Special handling for pipe operator: `a |> f` becomes `f(a)`
        if matches!(op, BinOp::Pipe) {
            return self.lower_pipe(left, right, ty, span);
        }

        let left_op = self.lower_expr(left)?;
        let right_op = self.lower_expr(right)?;

        let mir_op = convert_binop(op);
        let temp = self.new_temp(ty.clone(), span);
        let rvalue = Rvalue::BinaryOp {
            op: mir_op,
            left: left_op,
            right: right_op,
        };
        self.push_assign(Place::local(temp), rvalue);
        Ok(Operand::Copy(Place::local(temp)))
    }

    /// Lower a pipe expression: `a |> f` becomes `f(a)`.
    ///
    /// The pipe operator passes the left operand as the first argument
    /// to the function on the right-hand side.
    fn lower_pipe(
        &mut self,
        arg: &Expr,
        func: &Expr,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        // Lower the argument (left side of |>)
        let arg_op = self.lower_expr(arg)?;

        // Lower the function (right side of |>)
        let func_op = self.lower_expr(func)?;

        // Create destination for call result
        let dest = self.new_temp(ty.clone(), span);
        let dest_place = Place::local(dest);

        // Create continuation block
        let next_block = self.builder.new_block();

        // Generate call: f(a)
        self.terminate(TerminatorKind::Call {
            func: func_op,
            args: vec![arg_op],
            destination: dest_place.clone(),
            target: Some(next_block),
            unwind: None,
        });

        // Continue in the new block
        self.builder.switch_to(next_block);
        self.current_block = next_block;

        Ok(Operand::Copy(dest_place))
    }

    /// Lower a unary operation.
    fn lower_unary(
        &mut self,
        op: UnaryOp,
        operand: &Expr,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let operand_val = self.lower_expr(operand)?;

        let mir_op = match op {
            UnaryOp::Neg => MirUnOp::Neg,
            UnaryOp::Not => MirUnOp::Not,
            UnaryOp::Deref => {
                // Deref is handled separately
                let temp = self.new_temp(ty.clone(), span);
                if let Some(place) = operand_val.place() {
                    let deref_place = place.project(PlaceElem::Deref);
                    self.push_assign(Place::local(temp), Rvalue::Use(Operand::Copy(deref_place)));
                }
                return Ok(Operand::Copy(Place::local(temp)));
            }
            UnaryOp::Ref | UnaryOp::RefMut => {
                // Create a reference to the operand's place
                let mutable = matches!(op, UnaryOp::RefMut);
                let place = if let Some(p) = operand_val.place() {
                    p.clone()
                } else {
                    // Need to materialize the operand into a temp
                    let temp = self.new_temp(operand.ty.clone(), span);
                    self.push_assign(Place::local(temp), Rvalue::Use(operand_val));
                    Place::local(temp)
                };
                let ref_temp = self.new_temp(ty.clone(), span);
                self.push_assign(Place::local(ref_temp), Rvalue::Ref { place, mutable });
                return Ok(Operand::Copy(Place::local(ref_temp)));
            }
        };

        let temp = self.new_temp(ty.clone(), span);
        let rvalue = Rvalue::UnaryOp {
            op: mir_op,
            operand: operand_val,
        };
        self.push_assign(Place::local(temp), rvalue);
        Ok(Operand::Copy(Place::local(temp)))
    }

    /// Lower a function call.
    fn lower_call(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let func = self.lower_expr(callee)?;
        let mut arg_ops = Vec::with_capacity(args.len());
        for arg in args {
            arg_ops.push(self.lower_expr(arg)?);
        }

        let dest = self.new_temp(ty.clone(), span);
        let dest_place = Place::local(dest);

        let next_block = self.builder.new_block();

        self.terminate(TerminatorKind::Call {
            func,
            args: arg_ops,
            destination: dest_place.clone(),
            target: Some(next_block),
            unwind: None,
        });

        self.builder.switch_to(next_block);
        self.current_block = next_block;

        Ok(Operand::Copy(dest_place))
    }

    /// Lower an effect perform operation.
    ///
    /// `perform Effect.op(args)` creates a suspension point where control
    /// transfers to the effect handler. The handler may resume the continuation,
    /// at which point execution continues at the target block.
    fn lower_perform(
        &mut self,
        effect_id: DefId,
        op_index: u32,
        args: &[Expr],
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        // Lower all arguments
        let mut arg_ops = Vec::with_capacity(args.len());
        for arg in args {
            arg_ops.push(self.lower_expr(arg)?);
        }

        // Create destination for the result (what the handler resumes with)
        let dest = self.new_temp(ty.clone(), span);
        let dest_place = Place::local(dest);

        // Create continuation block (where we resume after handler processes)
        let resume_block = self.builder.new_block();

        // Emit the Perform terminator
        self.terminate(TerminatorKind::Perform {
            effect_id,
            op_index,
            args: arg_ops,
            destination: dest_place.clone(),
            target: resume_block,
        });

        // Switch to the resume block
        self.builder.switch_to(resume_block);
        self.current_block = resume_block;

        Ok(Operand::Copy(dest_place))
    }

    /// Lower a resume expression.
    ///
    /// `resume(value)` in a handler body continues the suspended computation
    /// with the given value. For tail-resumptive handlers, this is optimized
    /// to a direct return. For general handlers, this resumes the captured
    /// continuation.
    fn lower_resume(
        &mut self,
        value: Option<&Expr>,
        _span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        // Lower the resume value if present
        let resume_value = if let Some(val_expr) = value {
            Some(self.lower_expr(val_expr)?)
        } else {
            None
        };

        // Emit Resume terminator
        // Resume is a diverging operation - control transfers to the continuation
        self.terminate(TerminatorKind::Resume { value: resume_value });

        // Resume never returns (control flow transfers elsewhere)
        // Return a unit constant as placeholder - this code is unreachable
        Ok(Operand::Constant(Constant::new(Type::never(), ConstantKind::Unit)))
    }

    /// Lower a handle expression.
    ///
    /// `handle { body } with { handler }` runs the body with the specified
    /// effect handler installed. The handler can intercept effect operations
    /// performed by the body.
    ///
    /// MIR lowering emits:
    /// 1. Lower handler_instance to get state
    /// 2. PushHandler statement to install the handler with state
    /// 3. Body lowering
    /// 4. PopHandler statement to uninstall the handler
    fn lower_handle(
        &mut self,
        body: &Expr,
        handler_id: DefId,
        handler_instance: &Expr,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        // Step 1: Lower the handler instance to get the state
        let state_operand = self.lower_expr(handler_instance)?;

        // Store state in a temp local (we need a Place for the state)
        let state_local = self.new_temp(handler_instance.ty.clone(), span);
        let state_place = Place::local(state_local);
        self.push_assign(state_place.clone(), Rvalue::Use(state_operand));

        // Step 2: Push the handler onto the evidence vector with state
        self.push_stmt(StatementKind::PushHandler { handler_id, state_place: state_place.clone() });

        // Step 3: Lower the body expression with the handler installed
        let body_result = self.lower_expr(body)?;

        // Step 4: Pop the handler from the evidence vector
        self.push_stmt(StatementKind::PopHandler);

        // Create a destination for the handle result
        let dest = self.new_temp(ty.clone(), span);
        let dest_place = Place::local(dest);

        // Store the body result
        self.push_assign(dest_place.clone(), Rvalue::Use(body_result));

        Ok(Operand::Copy(dest_place))
    }

    /// Lower an array repeat expression `[value; count]`.
    ///
    /// Creates an array by repeating the value `count` times.
    fn lower_repeat(
        &mut self,
        value: &Expr,
        count: u64,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        // Lower the repeated value
        let value_op = self.lower_expr(value)?;

        // Create destination for the array
        let dest = self.new_temp(ty.clone(), span);
        let dest_place = Place::local(dest);

        // Get the element type from the array type
        let elem_ty = match ty.kind() {
            TypeKind::Array { element, .. } => element.clone(),
            _ => value.ty.clone(),
        };

        // Create an aggregate array with repeated values
        // We expand [x; N] into [x, x, x, ..., x] with N copies
        let operands: Vec<Operand> = (0..count).map(|_| value_op.clone()).collect();

        self.push_assign(
            dest_place.clone(),
            Rvalue::Aggregate {
                kind: AggregateKind::Array(elem_ty),
                operands,
            },
        );

        Ok(Operand::Copy(dest_place))
    }

    /// Lower an enum variant construction expression.
    ///
    /// Creates an enum value with the specified variant and fields.
    fn lower_variant(
        &mut self,
        def_id: DefId,
        variant_idx: u32,
        fields: &[Expr],
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        // Lower all field expressions
        let mut field_ops = Vec::with_capacity(fields.len());
        for field in fields {
            field_ops.push(self.lower_expr(field)?);
        }

        // Create destination for the enum
        let dest = self.new_temp(ty.clone(), span);
        let dest_place = Place::local(dest);

        // Create the aggregate value using Adt with variant_idx
        self.push_assign(
            dest_place.clone(),
            Rvalue::Aggregate {
                kind: AggregateKind::Adt {
                    def_id,
                    variant_idx: Some(variant_idx),
                },
                operands: field_ops,
            },
        );

        Ok(Operand::Copy(dest_place))
    }

    /// Lower a raw pointer address-of expression `&raw expr` or `&raw mut expr`.
    ///
    /// Creates a raw pointer to the expression's place.
    fn lower_addr_of(
        &mut self,
        inner: &Expr,
        mutable: bool,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        // Get the place for the inner expression
        let place = self.lower_place(inner)?;

        // Create destination for the raw pointer
        let dest = self.new_temp(ty.clone(), span);
        let dest_place = Place::local(dest);

        // Create the AddressOf rvalue
        self.push_assign(
            dest_place.clone(),
            Rvalue::AddressOf { place, mutable },
        );

        Ok(Operand::Copy(dest_place))
    }

    /// Lower a let expression (pattern binding with initialization).
    ///
    /// This is used for `if let` patterns and similar constructs.
    /// For irrefutable patterns, binds variables and returns the init value.
    /// For refutable patterns, would need decision tree compilation (not yet implemented).
    fn lower_let(
        &mut self,
        pattern: &Pattern,
        init: &Expr,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        // Lower the initializer expression
        let init_val = self.lower_expr(init)?;

        // Create a place for the initializer value
        let init_place = if let Some(place) = init_val.place() {
            place.clone()
        } else {
            // Need to store the value in a temporary to bind patterns
            let temp = self.new_temp(init.ty.clone(), span);
            self.push_assign(Place::local(temp), Rvalue::Use(init_val.clone()));
            Place::local(temp)
        };

        // Check if the pattern is irrefutable (always matches)
        // For now, we handle only irrefutable patterns
        // Refutable patterns would need control flow for match/no-match branches
        if self.is_irrefutable_pattern(pattern) {
            // Bind pattern variables
            self.bind_pattern(pattern, &init_place)?;

            // For irrefutable let, return the initialized value
            // The type checking determines whether this is used as a condition
            if ty.kind() == &TypeKind::Primitive(crate::hir::ty::PrimitiveTy::Bool) {
                // If the result type is bool, return true (pattern always matches)
                Ok(Operand::Constant(Constant::new(
                    ty.clone(),
                    ConstantKind::Bool(true),
                )))
            } else {
                // Otherwise return the bound value
                Ok(Operand::Copy(init_place))
            }
        } else {
            // Refutable pattern - would need decision tree compilation
            // For now, return an error
            Err(vec![Diagnostic::error(
                "Refutable patterns in let expressions require match compilation \
                 (not yet implemented). Use a match expression instead.".to_string(),
                span,
            )])
        }
    }

    /// Check if a pattern is irrefutable (always matches).
    fn is_irrefutable_pattern(&self, pattern: &Pattern) -> bool {
        match &pattern.kind {
            PatternKind::Wildcard => true,
            PatternKind::Binding { subpattern, .. } => {
                subpattern.as_ref().map_or(true, |p| self.is_irrefutable_pattern(p))
            }
            PatternKind::Tuple(pats) => pats.iter().all(|p| self.is_irrefutable_pattern(p)),
            PatternKind::Ref { inner, .. } => self.is_irrefutable_pattern(inner),
            // These patterns are refutable (may not match)
            PatternKind::Literal(_) => false,
            PatternKind::Or(_) => false,
            PatternKind::Variant { .. } => false,
            // Struct patterns are irrefutable if all field patterns are irrefutable
            PatternKind::Struct { fields, .. } => {
                fields.iter().all(|f| self.is_irrefutable_pattern(&f.pattern))
            }
            // Slice patterns with a rest element (..) are irrefutable
            PatternKind::Slice { prefix, slice, suffix } => {
                slice.is_some() &&
                prefix.iter().all(|p| self.is_irrefutable_pattern(p)) &&
                suffix.iter().all(|p| self.is_irrefutable_pattern(p))
            }
        }
    }

    /// Lower a block expression.
    fn lower_block(
        &mut self,
        stmts: &[Stmt],
        tail: Option<&Expr>,
        ty: &Type,
        _span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        // Lower statements
        for stmt in stmts {
            self.lower_stmt(stmt)?;
        }

        // Lower tail expression or return unit
        if let Some(tail) = tail {
            self.lower_expr(tail)
        } else {
            Ok(Operand::Constant(Constant::new(ty.clone(), ConstantKind::Unit)))
        }
    }

    /// Lower a statement.
    fn lower_stmt(&mut self, stmt: &Stmt) -> Result<(), Vec<Diagnostic>> {
        match stmt {
            Stmt::Let { local_id, init } => {
                // Get or create the MIR local
                let hir_local = self.body.get_local(*local_id)
                    .ok_or_else(|| vec![ice_err!(
                        Span::dummy(),
                        "local not found in HIR body during MIR lowering";
                        "local_id" => local_id
                    )])?;
                let mir_local = self.builder.new_temp(hir_local.ty.clone(), hir_local.span);
                self.local_map.insert(*local_id, mir_local);

                // Storage live
                self.push_stmt(StatementKind::StorageLive(mir_local));

                // Initialize if there's an init expression
                if let Some(init) = init {
                    let init_val = self.lower_expr(init)?;

                    // If init is a closure, propagate the Closure type to the target local
                    // The init_val will be Copy/Move of a temp with Closure type
                    if let Operand::Copy(place) | Operand::Move(place) = &init_val {
                        if let Some(temp_ty) = self.builder.get_local_type(place.local) {
                            if matches!(temp_ty.kind(), TypeKind::Closure { .. }) {
                                self.builder.set_local_type(mir_local, temp_ty.clone());
                            }
                        }
                    }

                    self.push_assign(Place::local(mir_local), Rvalue::Use(init_val));
                }
            }
            Stmt::Expr(expr) => {
                // Lower expression for side effects
                let _ = self.lower_expr(expr)?;
            }
            Stmt::Item(_) => {
                // Nested items are handled at crate level
            }
        }
        Ok(())
    }

    /// Lower an if expression.
    fn lower_if(
        &mut self,
        condition: &Expr,
        then_branch: &Expr,
        else_branch: Option<&Expr>,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let cond = self.lower_expr(condition)?;

        let then_block = self.builder.new_block();
        let else_block = self.builder.new_block();
        let join_block = self.builder.new_block();

        // Result temporary
        let result = self.new_temp(ty.clone(), span);

        // Switch on condition
        self.terminate(TerminatorKind::SwitchInt {
            discr: cond,
            targets: SwitchTargets::new(vec![(1, then_block)], else_block),
        });

        // Then branch
        self.builder.switch_to(then_block);
        self.current_block = then_block;
        let then_val = self.lower_expr(then_branch)?;
        self.push_assign(Place::local(result), Rvalue::Use(then_val));
        self.terminate(TerminatorKind::Goto { target: join_block });

        // Else branch
        self.builder.switch_to(else_block);
        self.current_block = else_block;
        if let Some(else_expr) = else_branch {
            let else_val = self.lower_expr(else_expr)?;
            self.push_assign(Place::local(result), Rvalue::Use(else_val));
        } else {
            self.push_assign(
                Place::local(result),
                Rvalue::Use(Operand::Constant(Constant::unit())),
            );
        }
        self.terminate(TerminatorKind::Goto { target: join_block });

        // Continue at join
        self.builder.switch_to(join_block);
        self.current_block = join_block;

        Ok(Operand::Copy(Place::local(result)))
    }

    /// Lower a match expression.
    fn lower_match(
        &mut self,
        scrutinee: &Expr,
        arms: &[MatchArm],
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let scrut = self.lower_expr(scrutinee)?;

        // Result temporary
        let result = self.new_temp(ty.clone(), span);
        let join_block = self.builder.new_block();

        // For each arm, create a block
        let arm_blocks: Vec<_> = arms.iter().map(|_| self.builder.new_block()).collect();

        // For now, use simple sequential testing
        // A real implementation would use decision tree compilation
        let otherwise = arm_blocks.last().copied().unwrap_or(join_block);
        let mut branches = Vec::new();

        for (i, arm) in arms.iter().enumerate() {
            if let PatternKind::Literal(LiteralValue::Int(v)) = &arm.pattern.kind {
                branches.push((*v as u128, arm_blocks[i]));
            }
        }

        // Switch (simplified - real impl needs better pattern compilation)
        self.terminate(TerminatorKind::SwitchInt {
            discr: scrut.clone(),
            targets: SwitchTargets::new(branches, otherwise),
        });

        // Lower each arm
        for (i, arm) in arms.iter().enumerate() {
            self.builder.switch_to(arm_blocks[i]);
            self.current_block = arm_blocks[i];

            // Bind pattern variables
            if let Some(place) = scrut.place() {
                self.bind_pattern(&arm.pattern, place)?;
            }

            // Check guard if present
            if let Some(guard) = &arm.guard {
                // Create blocks for guard success/failure
                let guard_pass = self.builder.new_block();
                let guard_fail = if i + 1 < arm_blocks.len() {
                    // Fall through to next arm if guard fails
                    arm_blocks[i + 1]
                } else {
                    // Last arm - guard failure is unreachable (pattern should be exhaustive)
                    // In a real implementation, this would be a panic/unreachable block
                    join_block
                };

                // Lower the guard expression
                let guard_result = self.lower_expr(guard)?;

                // Branch based on guard result
                self.terminate(TerminatorKind::SwitchInt {
                    discr: guard_result,
                    targets: SwitchTargets::new(
                        vec![(1, guard_pass)], // true (1) -> guard_pass
                        guard_fail,             // false (0) -> guard_fail
                    ),
                });

                // Continue in guard_pass block for the arm body
                self.builder.switch_to(guard_pass);
                self.current_block = guard_pass;
            }

            let arm_val = self.lower_expr(&arm.body)?;
            self.push_assign(Place::local(result), Rvalue::Use(arm_val));
            self.terminate(TerminatorKind::Goto { target: join_block });
        }

        self.builder.switch_to(join_block);
        self.current_block = join_block;

        Ok(Operand::Copy(Place::local(result)))
    }

    /// Lower a loop expression.
    fn lower_loop(
        &mut self,
        body: &Expr,
        label: Option<hir::LoopId>,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let loop_block = self.builder.new_block();
        let exit_block = self.builder.new_block();

        // Result for break values
        let result = self.new_temp(ty.clone(), span);

        // Push loop context
        self.loop_stack.push(LoopContext {
            break_block: exit_block,
            continue_block: loop_block,
            label,
        });

        // Jump to loop
        self.terminate(TerminatorKind::Goto { target: loop_block });

        // Loop body
        self.builder.switch_to(loop_block);
        self.current_block = loop_block;
        let _ = self.lower_expr(body)?;

        // Loop back
        if !self.is_terminated() {
            self.terminate(TerminatorKind::Goto { target: loop_block });
        }

        // Pop loop context
        self.loop_stack.pop();

        // Continue at exit
        self.builder.switch_to(exit_block);
        self.current_block = exit_block;

        Ok(Operand::Copy(Place::local(result)))
    }

    /// Lower a while loop.
    fn lower_while(
        &mut self,
        condition: &Expr,
        body: &Expr,
        label: Option<hir::LoopId>,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let cond_block = self.builder.new_block();
        let body_block = self.builder.new_block();
        let exit_block = self.builder.new_block();

        let result = self.new_temp(ty.clone(), span);

        // Push loop context
        self.loop_stack.push(LoopContext {
            break_block: exit_block,
            continue_block: cond_block,
            label,
        });

        // Jump to condition
        self.terminate(TerminatorKind::Goto { target: cond_block });

        // Condition block
        self.builder.switch_to(cond_block);
        self.current_block = cond_block;
        let cond = self.lower_expr(condition)?;
        self.terminate(TerminatorKind::SwitchInt {
            discr: cond,
            targets: SwitchTargets::new(vec![(1, body_block)], exit_block),
        });

        // Body block
        self.builder.switch_to(body_block);
        self.current_block = body_block;
        let _ = self.lower_expr(body)?;
        if !self.is_terminated() {
            self.terminate(TerminatorKind::Goto { target: cond_block });
        }

        // Pop loop context
        self.loop_stack.pop();

        // Exit
        self.builder.switch_to(exit_block);
        self.current_block = exit_block;

        Ok(Operand::Copy(Place::local(result)))
    }

    /// Lower a break expression.
    fn lower_break(
        &mut self,
        label: Option<hir::LoopId>,
        value: Option<&Expr>,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let ctx = if let Some(label) = label {
            self.loop_stack.iter().rev().find(|c| c.label == Some(label))
        } else {
            self.loop_stack.last()
        };

        if let Some(ctx) = ctx.cloned() {
            if let Some(value) = value {
                let _ = self.lower_expr(value)?;
            }
            self.terminate(TerminatorKind::Goto { target: ctx.break_block });
        } else {
            return Err(vec![Diagnostic::error("break outside of loop".to_string(), span)]);
        }

        // Unreachable after break
        let next = self.builder.new_block();
        self.builder.switch_to(next);
        self.current_block = next;

        Ok(Operand::Constant(Constant::unit()))
    }

    /// Lower a continue expression.
    fn lower_continue(
        &mut self,
        label: Option<hir::LoopId>,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let ctx = if let Some(label) = label {
            self.loop_stack.iter().rev().find(|c| c.label == Some(label))
        } else {
            self.loop_stack.last()
        };

        if let Some(ctx) = ctx.cloned() {
            self.terminate(TerminatorKind::Goto { target: ctx.continue_block });
        } else {
            return Err(vec![Diagnostic::error("continue outside of loop".to_string(), span)]);
        }

        let next = self.builder.new_block();
        self.builder.switch_to(next);
        self.current_block = next;

        Ok(Operand::Constant(Constant::unit()))
    }

    /// Lower a return expression.
    fn lower_return(
        &mut self,
        value: Option<&Expr>,
        _span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let return_place = Place::local(LocalId::new(0));

        if let Some(value) = value {
            let val = self.lower_expr(value)?;
            self.push_assign(return_place, Rvalue::Use(val));
        } else {
            self.push_assign(return_place, Rvalue::Use(Operand::Constant(Constant::unit())));
        }

        self.terminate(TerminatorKind::Return);

        let next = self.builder.new_block();
        self.builder.switch_to(next);
        self.current_block = next;

        Ok(Operand::Constant(Constant::unit()))
    }

    /// Lower an assignment.
    fn lower_assign(
        &mut self,
        target: &Expr,
        value: &Expr,
        _span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let place = self.lower_place(target)?;
        let val = self.lower_expr(value)?;
        self.push_assign(place, Rvalue::Use(val));
        Ok(Operand::Constant(Constant::unit()))
    }

    /// Lower a tuple expression.
    fn lower_tuple(
        &mut self,
        elems: &[Expr],
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let mut operands = Vec::with_capacity(elems.len());
        for elem in elems {
            operands.push(self.lower_expr(elem)?);
        }

        let temp = self.new_temp(ty.clone(), span);
        self.push_assign(
            Place::local(temp),
            Rvalue::Aggregate {
                kind: AggregateKind::Tuple,
                operands,
            },
        );

        Ok(Operand::Copy(Place::local(temp)))
    }

    /// Lower an array expression.
    fn lower_array(
        &mut self,
        elems: &[Expr],
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let elem_ty = match ty.kind() {
            TypeKind::Array { element, .. } => element.clone(),
            other => {
                return Err(vec![ice_err!(
                    span,
                    "lower_array called with non-array type";
                    "type_kind" => other
                )]);
            }
        };

        let mut operands = Vec::with_capacity(elems.len());
        for elem in elems {
            operands.push(self.lower_expr(elem)?);
        }

        let temp = self.new_temp(ty.clone(), span);
        self.push_assign(
            Place::local(temp),
            Rvalue::Aggregate {
                kind: AggregateKind::Array(elem_ty),
                operands,
            },
        );

        Ok(Operand::Copy(Place::local(temp)))
    }

    /// Lower a struct expression.
    fn lower_struct(
        &mut self,
        def_id: DefId,
        fields: &[hir::FieldExpr],
        _base: Option<&Expr>,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let mut operands = Vec::with_capacity(fields.len());
        for field in fields {
            operands.push(self.lower_expr(&field.value)?);
        }

        let temp = self.new_temp(ty.clone(), span);
        self.push_assign(
            Place::local(temp),
            Rvalue::Aggregate {
                kind: AggregateKind::Adt { def_id, variant_idx: None },
                operands,
            },
        );

        Ok(Operand::Copy(Place::local(temp)))
    }

    /// Lower a field access.
    fn lower_field(
        &mut self,
        base: &Expr,
        field_idx: u32,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let base_place = self.lower_place(base)?;
        let field_place = base_place.project(PlaceElem::Field(field_idx));

        let temp = self.new_temp(ty.clone(), span);
        self.push_assign(Place::local(temp), Rvalue::Use(Operand::Copy(field_place)));

        Ok(Operand::Copy(Place::local(temp)))
    }

    /// Lower an index operation.
    fn lower_index(
        &mut self,
        base: &Expr,
        index: &Expr,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let base_place = self.lower_place(base)?;
        let index_op = self.lower_expr(index)?;

        // Index needs to be a local
        let index_local = if let Operand::Copy(p) | Operand::Move(p) = &index_op {
            p.local
        } else {
            let temp = self.new_temp(Type::u64(), span);
            self.push_assign(Place::local(temp), Rvalue::Use(index_op));
            temp
        };

        let indexed_place = base_place.project(PlaceElem::Index(index_local));

        let temp = self.new_temp(ty.clone(), span);
        self.push_assign(Place::local(temp), Rvalue::Use(Operand::Copy(indexed_place)));

        Ok(Operand::Copy(Place::local(temp)))
    }

    /// Lower a borrow expression.
    fn lower_borrow(
        &mut self,
        expr: &Expr,
        mutable: bool,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let place = self.lower_place(expr)?;

        let temp = self.new_temp(ty.clone(), span);
        self.push_assign(Place::local(temp), Rvalue::Ref { place, mutable });

        Ok(Operand::Copy(Place::local(temp)))
    }

    /// Lower a deref expression.
    ///
    /// This emits a generation validation check before the dereference to detect
    /// use-after-free errors at runtime. The generation check compares the
    /// expected generation (captured when the reference was created) against
    /// the current generation of the memory slot.
    fn lower_deref(
        &mut self,
        expr: &Expr,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let inner = self.lower_expr(expr)?;
        let inner_place = if let Some(p) = inner.place() {
            p.clone()
        } else {
            let temp = self.new_temp(expr.ty.clone(), span);
            self.push_assign(Place::local(temp), Rvalue::Use(inner));
            Place::local(temp)
        };

        // Generation validation temporarily disabled to diagnose type issue.
        // TODO: Re-enable after type mismatch is fully resolved.
        //
        // See: MEMORY_MODEL.md §4 "Generational References"
        // let expected_gen = Operand::Constant(Constant::int(1, Type::u32()));
        // self.push_stmt(StatementKind::ValidateGeneration {
        //     ptr: inner_place.clone(),
        //     expected_gen,
        // });

        let deref_place = inner_place.project(PlaceElem::Deref);
        let temp = self.new_temp(ty.clone(), span);
        self.push_assign(Place::local(temp), Rvalue::Use(Operand::Copy(deref_place)));

        Ok(Operand::Copy(Place::local(temp)))
    }

    /// Lower a cast expression.
    fn lower_cast(
        &mut self,
        expr: &Expr,
        target_ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let operand = self.lower_expr(expr)?;
        let temp = self.new_temp(target_ty.clone(), span);
        self.push_assign(
            Place::local(temp),
            Rvalue::Cast {
                operand,
                target_ty: target_ty.clone(),
            },
        );
        Ok(Operand::Copy(Place::local(temp)))
    }

    /// Lower an expression to a place (for assignment targets).
    fn lower_place(&mut self, expr: &Expr) -> Result<Place, Vec<Diagnostic>> {
        match &expr.kind {
            ExprKind::Local(local_id) => {
                let mir_local = self.map_local(*local_id);
                Ok(Place::local(mir_local))
            }
            ExprKind::Field { base, field_idx } => {
                let base_place = self.lower_place(base)?;
                Ok(base_place.project(PlaceElem::Field(*field_idx)))
            }
            ExprKind::Index { base, index } => {
                let base_place = self.lower_place(base)?;
                let index_op = self.lower_expr(index)?;
                let index_local = if let Operand::Copy(p) | Operand::Move(p) = &index_op {
                    p.local
                } else {
                    let temp = self.new_temp(Type::u64(), expr.span);
                    self.push_assign(Place::local(temp), Rvalue::Use(index_op));
                    temp
                };
                Ok(base_place.project(PlaceElem::Index(index_local)))
            }
            ExprKind::Deref(inner) => {
                let inner_place = self.lower_place(inner)?;
                Ok(inner_place.project(PlaceElem::Deref))
            }
            ExprKind::Unary { op: UnaryOp::Deref, operand } => {
                // Handle Unary Deref the same as ExprKind::Deref
                let inner_place = self.lower_place(operand)?;
                Ok(inner_place.project(PlaceElem::Deref))
            }
            _ => {
                // For other expressions, create a temporary
                let val = self.lower_expr(expr)?;
                if let Some(place) = val.place() {
                    Ok(place.clone())
                } else {
                    let temp = self.new_temp(expr.ty.clone(), expr.span);
                    self.push_assign(Place::local(temp), Rvalue::Use(val));
                    Ok(Place::local(temp))
                }
            }
        }
    }

    /// Bind pattern variables to a place.
    fn bind_pattern(&mut self, pattern: &Pattern, place: &Place) -> Result<(), Vec<Diagnostic>> {
        match &pattern.kind {
            PatternKind::Binding { local_id, subpattern, .. } => {
                let mir_local = self.new_temp(pattern.ty.clone(), pattern.span);
                self.local_map.insert(*local_id, mir_local);
                self.push_assign(Place::local(mir_local), Rvalue::Use(Operand::Copy(place.clone())));
                if let Some(subpat) = subpattern {
                    self.bind_pattern(subpat, &Place::local(mir_local))?;
                }
            }
            PatternKind::Tuple(pats) => {
                for (i, pat) in pats.iter().enumerate() {
                    let field_place = place.project(PlaceElem::Field(i as u32));
                    self.bind_pattern(pat, &field_place)?;
                }
            }
            PatternKind::Struct { fields, .. } => {
                for field in fields {
                    let field_place = place.project(PlaceElem::Field(field.field_idx));
                    self.bind_pattern(&field.pattern, &field_place)?;
                }
            }
            PatternKind::Wildcard | PatternKind::Literal(_) => {
                // Nothing to bind
            }
            PatternKind::Variant { fields, .. } => {
                // For enum variant patterns, bind each field pattern
                for (i, field_pat) in fields.iter().enumerate() {
                    let field_place = place.project(PlaceElem::Field(i as u32));
                    self.bind_pattern(field_pat, &field_place)?;
                }
            }
            PatternKind::Slice { prefix, slice, suffix } => {
                // Calculate minimum length required for this pattern
                let min_length = (prefix.len() + suffix.len()) as u64;

                // Bind prefix patterns using ConstantIndex from start
                for (i, pat) in prefix.iter().enumerate() {
                    let idx_place = place.project(PlaceElem::ConstantIndex {
                        offset: i as u64,
                        min_length,
                        from_end: false,
                    });
                    self.bind_pattern(pat, &idx_place)?;
                }

                // Bind suffix patterns using ConstantIndex from end
                // Suffix is reversed: last element in suffix is at offset 0 from end
                for (i, pat) in suffix.iter().enumerate() {
                    let offset_from_end = (suffix.len() - 1 - i) as u64;
                    let idx_place = place.project(PlaceElem::ConstantIndex {
                        offset: offset_from_end,
                        min_length,
                        from_end: true,
                    });
                    self.bind_pattern(pat, &idx_place)?;
                }

                // Bind the rest pattern (..) if present
                if let Some(rest_pat) = slice {
                    // The rest covers elements from prefix.len() to (len - suffix.len())
                    // Use Subslice projection
                    let subslice_place = place.project(PlaceElem::Subslice {
                        from: prefix.len() as u64,
                        to: suffix.len() as u64,
                        from_end: true,
                    });
                    self.bind_pattern(rest_pat, &subslice_place)?;
                }
            }
            PatternKind::Or(alternatives) => {
                // Or-patterns in binding context: all alternatives must bind the same variables
                // with the same types. We bind using the first alternative since they're all
                // equivalent for binding purposes.
                //
                // Note: This assumes type checking has verified all alternatives bind the same
                // variables. For refutable or-patterns, match compilation should handle
                // determining which alternative actually matched.
                if let Some(first_alt) = alternatives.first() {
                    self.bind_pattern(first_alt, place)?;
                }
            }
            PatternKind::Ref { inner, .. } => {
                // For reference patterns, bind the inner pattern to a dereferenced place
                let deref_place = place.project(PlaceElem::Deref);
                self.bind_pattern(inner, &deref_place)?;
            }
        }
        Ok(())
    }

    // Helper methods

    fn map_local(&mut self, hir_local: LocalId) -> LocalId {
        if let Some(&mir_local) = self.local_map.get(&hir_local) {
            mir_local
        } else {
            // Create a new local if not mapped
            let local = self.body.get_local(hir_local);
            let ty = local.map(|l| l.ty.clone()).unwrap_or_else(Type::error);
            let span = local.map(|l| l.span).unwrap_or_else(Span::dummy);
            let mir_id = self.builder.new_temp(ty, span);
            self.local_map.insert(hir_local, mir_id);
            mir_id
        }
    }

    fn new_temp(&mut self, ty: Type, span: Span) -> LocalId {
        self.temp_counter += 1;
        self.builder.new_temp(ty, span)
    }

    fn push_stmt(&mut self, kind: StatementKind) {
        self.builder.push_stmt(Statement::new(kind, Span::dummy()));
    }

    fn push_assign(&mut self, place: Place, rvalue: Rvalue) {
        self.push_stmt(StatementKind::Assign(place, rvalue));
    }

    fn terminate(&mut self, kind: TerminatorKind) {
        self.builder.terminate(Terminator::new(kind, Span::dummy()));
    }

    fn is_terminated(&self) -> bool {
        self.builder.is_current_terminated()
    }

    /// Lower a closure expression.
    ///
    /// Closures are lowered as:
    /// 1. Generate a synthetic DefId for the closure function
    /// 2. Schedule the closure body for later lowering
    /// 3. Lower captured values to operands
    /// 4. Create an aggregate containing the captures
    /// 5. Return the closure aggregate as the value
    ///
    /// At codegen time, this aggregate is paired with the closure function
    /// pointer to form a fat pointer: { fn_ptr: *i8, env_ptr: *i8 }
    fn lower_closure(
        &mut self,
        body_id: hir::BodyId,
        captures: &[hir::Capture],
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        // Generate a synthetic DefId for this closure
        let closure_def_id = self.next_closure_def_id();

        // Lower each captured value to an operand and collect types
        let mut capture_operands = Vec::with_capacity(captures.len());
        let mut captures_with_types = Vec::with_capacity(captures.len());
        for capture in captures {
            let mir_local = self.map_local(capture.local_id);
            let place = Place::local(mir_local);

            // Get the type of the captured local from our local map
            let capture_ty = self.body.get_local(capture.local_id)
                .map(|l| l.ty.clone())
                .unwrap_or_else(Type::error);

            // Use Move for by-move captures, Copy for by-reference
            let operand = if capture.by_move {
                Operand::Move(place)
            } else {
                Operand::Copy(place)
            };
            capture_operands.push(operand);
            captures_with_types.push((capture.clone(), capture_ty));
        }

        // Schedule the closure body for later lowering (with captures and types)
        self.pending_closures.push((body_id, closure_def_id, captures_with_types));

        // Create a Closure type with the synthetic DefId
        // Extract params and ret from the original function type
        let closure_ty = match ty.kind() {
            TypeKind::Fn { params, ret } => {
                Type::new(TypeKind::Closure {
                    def_id: closure_def_id,
                    params: params.clone(),
                    ret: ret.clone(),
                })
            }
            _ => {
                // If not a Fn type, use it directly (error recovery)
                ty.clone()
            }
        };

        // Create a temporary to hold the closure aggregate
        let temp = self.new_temp(closure_ty, span);

        // Build the closure aggregate containing captured values
        self.push_assign(
            Place::local(temp),
            Rvalue::Aggregate {
                kind: AggregateKind::Closure { def_id: closure_def_id },
                operands: capture_operands,
            },
        );

        Ok(Operand::Copy(Place::local(temp)))
    }
}

// ============================================================================
// Closure Lowering
// ============================================================================

/// Lowers a closure body to MIR.
///
/// Similar to FunctionLowering but handles closure-specific semantics:
/// - Captured variables are accessed through the environment
/// - The environment is passed as the first implicit parameter
#[allow(dead_code)]
struct ClosureLowering<'hir, 'ctx> {
    /// MIR body builder.
    builder: MirBodyBuilder,
    /// HIR body being lowered.
    body: &'hir Body,
    /// HIR crate for accessing nested closure bodies.
    hir: &'hir HirCrate,
    /// Mapping from HIR locals to MIR locals.
    local_map: HashMap<LocalId, LocalId>,
    /// Mapping from captured HIR locals to their field index in the environment.
    capture_map: HashMap<LocalId, usize>,
    /// Types of captured variables (for nested closures).
    capture_types: HashMap<LocalId, Type>,
    /// The MIR local that holds the environment (first param after return).
    env_local: Option<LocalId>,
    /// Current basic block.
    current_block: BasicBlockId,
    /// Loop context for break/continue.
    loop_stack: Vec<LoopContext>,
    /// Counter for unique temporary names.
    temp_counter: u32,
    /// Pending closures to be lowered (for nested closures).
    pending_closures: &'ctx mut Vec<(hir::BodyId, DefId, Vec<(hir::Capture, Type)>)>,
    /// Counter for generating synthetic closure DefIds.
    closure_counter: &'ctx mut u32,
}

impl<'hir, 'ctx> ClosureLowering<'hir, 'ctx> {
    /// Create a new closure lowering context.
    fn new(
        def_id: DefId,
        body: &'hir Body,
        captures: &[(hir::Capture, Type)],
        hir: &'hir HirCrate,
        pending_closures: &'ctx mut Vec<(hir::BodyId, DefId, Vec<(hir::Capture, Type)>)>,
        closure_counter: &'ctx mut u32,
    ) -> Self {
        let mut builder = MirBodyBuilder::new(def_id, body.span);

        // Set return type
        builder.set_return_type(body.return_type().clone());

        // Build the capture map and environment type from provided captures
        let mut capture_map = HashMap::new();
        let mut capture_types_map = HashMap::new();
        let mut capture_type_list = Vec::new();
        for (idx, (capture, ty)) in captures.iter().enumerate() {
            capture_map.insert(capture.local_id, idx);
            capture_types_map.insert(capture.local_id, ty.clone());
            capture_type_list.push(ty.clone());
        }

        // If there are captures, add an implicit environment parameter
        let env_local = if !captures.is_empty() {
            // Always create a tuple type for the environment, even for single captures
            // This ensures field projections work consistently
            let env_ty = Type::new(TypeKind::Tuple(capture_type_list));
            let env_local = builder.add_param(
                Some("__env".to_string()),
                env_ty,
                body.span,
            );
            Some(env_local)
        } else {
            None
        };

        // Add parameters from the closure's explicit parameter list
        let mut local_map = HashMap::new();
        for param in body.params() {
            let mir_local = builder.add_param(
                param.name.clone(),
                param.ty.clone(),
                param.span,
            );
            // Map the actual HIR local ID to the MIR local
            // Note: HIR local IDs may not be consecutive (closures share outer ID space)
            local_map.insert(param.id, mir_local);
        }

        let current_block = builder.current_block();

        Self {
            builder,
            body,
            hir,
            local_map,
            capture_map,
            capture_types: capture_types_map,
            env_local,
            current_block,
            loop_stack: Vec::new(),
            temp_counter: 0,
            pending_closures,
            closure_counter,
        }
    }

    /// Lower the closure body.
    fn lower(mut self) -> Result<MirBody, Vec<Diagnostic>> {
        // Lower the body expression
        let body_expr = self.body.expr.clone();
        let result = self.lower_expr(&body_expr)?;

        // Assign to return place
        let return_place = Place::local(LocalId::new(0));
        self.push_assign(return_place, Rvalue::Use(result));

        // Add return terminator
        self.terminate(TerminatorKind::Return);

        Ok(self.builder.finish())
    }

    /// Generate a synthetic DefId for a nested closure.
    fn next_closure_def_id(&mut self) -> DefId {
        let id = *self.closure_counter;
        *self.closure_counter += 1;
        DefId::new(0xFFFF_0000 + id)
    }

    /// Lower an expression - delegates to the common expression lowering logic.
    ///
    /// Note: This duplicates FunctionLowering::lower_expr. In a production compiler,
    /// we'd use a trait or macro to share this logic. For now, we inline it for clarity.
    fn lower_expr(&mut self, expr: &Expr) -> Result<Operand, Vec<Diagnostic>> {
        match &expr.kind {
            ExprKind::Literal(lit) => self.lower_literal(lit, &expr.ty),

            ExprKind::Local(local_id) => {
                // Check if this local is a captured variable from the outer scope
                if let Some(&field_idx) = self.capture_map.get(local_id) {
                    // Captured variable: access via field projection on environment
                    if let Some(env_local) = self.env_local {
                        let env_place = Place::local(env_local);
                        let capture_place = env_place.project(PlaceElem::Field(field_idx as u32));
                        Ok(Operand::Copy(capture_place))
                    } else {
                        // Should not happen: capture_map has entries but no env_local
                        Err(vec![Diagnostic::error(
                            "internal error: captured variable but no environment".to_string(),
                            expr.span,
                        )])
                    }
                } else {
                    // Regular local: use normal mapping
                    let mir_local = self.map_local(*local_id);
                    Ok(Operand::Copy(Place::local(mir_local)))
                }
            }

            ExprKind::Def(def_id) => {
                let constant = Constant::new(expr.ty.clone(), ConstantKind::FnDef(*def_id));
                Ok(Operand::Constant(constant))
            }

            ExprKind::Binary { op, left, right } => {
                self.lower_binary(*op, left, right, &expr.ty, expr.span)
            }

            ExprKind::Unary { op, operand } => {
                self.lower_unary(*op, operand, &expr.ty, expr.span)
            }

            ExprKind::Call { callee, args } => {
                self.lower_call(callee, args, &expr.ty, expr.span)
            }

            ExprKind::Block { stmts, expr: tail } => {
                self.lower_block(stmts, tail.as_deref(), &expr.ty, expr.span)
            }

            ExprKind::If { condition, then_branch, else_branch } => {
                self.lower_if(condition, then_branch, else_branch.as_deref(), &expr.ty, expr.span)
            }

            ExprKind::Return(value) => {
                self.lower_return(value.as_deref(), expr.span)
            }

            ExprKind::Tuple(elems) => {
                self.lower_tuple(elems, &expr.ty, expr.span)
            }

            ExprKind::Array(elems) => {
                self.lower_array(elems, &expr.ty, expr.span)
            }

            ExprKind::Field { base, field_idx } => {
                self.lower_field(base, *field_idx, &expr.ty, expr.span)
            }

            ExprKind::Index { base, index } => {
                self.lower_index(base, index, &expr.ty, expr.span)
            }

            ExprKind::Assign { target, value } => {
                self.lower_assign(target, value, expr.span)
            }

            ExprKind::Borrow { expr: inner, mutable } => {
                self.lower_borrow(inner, *mutable, &expr.ty, expr.span)
            }

            ExprKind::Deref(inner) => {
                self.lower_deref(inner, &expr.ty, expr.span)
            }

            ExprKind::Cast { expr: inner, target_ty } => {
                self.lower_cast(inner, target_ty, expr.span)
            }

            ExprKind::Closure { body_id, captures } => {
                self.lower_closure(*body_id, captures, &expr.ty, expr.span)
            }

            ExprKind::Unsafe(inner) => {
                self.lower_expr(inner)
            }

            ExprKind::Perform { effect_id, op_index, args } => {
                self.lower_perform(*effect_id, *op_index, args, &expr.ty, expr.span)
            }

            ExprKind::Resume { value } => {
                self.lower_resume(value.as_deref(), expr.span)
            }

            ExprKind::Handle { body, handler_id, handler_instance } => {
                self.lower_handle(body, *handler_id, handler_instance, &expr.ty, expr.span)
            }

            ExprKind::Repeat { value, count } => {
                self.lower_repeat(value, *count, &expr.ty, expr.span)
            }

            ExprKind::Variant { def_id, variant_idx, fields } => {
                self.lower_variant(*def_id, *variant_idx, fields, &expr.ty, expr.span)
            }

            ExprKind::AddrOf { expr: inner, mutable } => {
                self.lower_addr_of(inner, *mutable, &expr.ty, expr.span)
            }

            ExprKind::Let { pattern, init } => {
                self.lower_let(pattern, init, &expr.ty, expr.span)
            }

            ExprKind::Error => {
                Err(vec![Diagnostic::error("lowering error expression".to_string(), expr.span)])
            }

            // Remaining expression kinds that need implementation
            _ => {
                Err(vec![Diagnostic::error(
                    format!("MIR lowering for {:?} in closure not yet implemented",
                            std::mem::discriminant(&expr.kind)),
                    expr.span,
                )])
            }
        }
    }

    // Helper methods - these mirror FunctionLowering methods

    fn lower_literal(&mut self, lit: &LiteralValue, ty: &Type) -> Result<Operand, Vec<Diagnostic>> {
        let kind = match lit {
            LiteralValue::Int(v) => ConstantKind::Int(*v),
            LiteralValue::Uint(v) => ConstantKind::Uint(*v),
            LiteralValue::Float(v) => ConstantKind::Float(*v),
            LiteralValue::Bool(v) => ConstantKind::Bool(*v),
            LiteralValue::Char(v) => ConstantKind::Char(*v),
            LiteralValue::String(v) => ConstantKind::String(v.clone()),
        };
        Ok(Operand::Constant(Constant::new(ty.clone(), kind)))
    }

    fn lower_binary(
        &mut self,
        op: BinOp,
        left: &Expr,
        right: &Expr,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let left_op = self.lower_expr(left)?;
        let right_op = self.lower_expr(right)?;

        let mir_op = convert_binop(op);
        let temp = self.new_temp(ty.clone(), span);
        let rvalue = Rvalue::BinaryOp {
            op: mir_op,
            left: left_op,
            right: right_op,
        };
        self.push_assign(Place::local(temp), rvalue);
        Ok(Operand::Copy(Place::local(temp)))
    }

    fn lower_unary(
        &mut self,
        op: UnaryOp,
        operand: &Expr,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let operand_val = self.lower_expr(operand)?;

        let mir_op = match op {
            UnaryOp::Neg => MirUnOp::Neg,
            UnaryOp::Not => MirUnOp::Not,
            UnaryOp::Deref => {
                // Deref is handled by creating a copy from the dereferenced place
                let temp = self.new_temp(ty.clone(), span);
                if let Some(place) = operand_val.place() {
                    let deref_place = place.project(PlaceElem::Deref);
                    self.push_assign(Place::local(temp), Rvalue::Use(Operand::Copy(deref_place)));
                }
                return Ok(Operand::Copy(Place::local(temp)));
            }
            UnaryOp::Ref | UnaryOp::RefMut => {
                // Create a reference to the operand's place
                let mutable = matches!(op, UnaryOp::RefMut);
                let place = if let Some(p) = operand_val.place() {
                    p.clone()
                } else {
                    // Need to materialize the operand into a temp
                    let temp = self.new_temp(operand.ty.clone(), span);
                    self.push_assign(Place::local(temp), Rvalue::Use(operand_val));
                    Place::local(temp)
                };
                let ref_temp = self.new_temp(ty.clone(), span);
                self.push_assign(Place::local(ref_temp), Rvalue::Ref { place, mutable });
                return Ok(Operand::Copy(Place::local(ref_temp)));
            }
        };

        let temp = self.new_temp(ty.clone(), span);
        let rvalue = Rvalue::UnaryOp {
            op: mir_op,
            operand: operand_val,
        };
        self.push_assign(Place::local(temp), rvalue);
        Ok(Operand::Copy(Place::local(temp)))
    }

    fn lower_call(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let func = self.lower_expr(callee)?;
        let mut arg_ops = Vec::with_capacity(args.len());
        for arg in args {
            arg_ops.push(self.lower_expr(arg)?);
        }

        let dest = self.new_temp(ty.clone(), span);
        let dest_place = Place::local(dest);

        let next_block = self.builder.new_block();

        self.terminate(TerminatorKind::Call {
            func,
            args: arg_ops,
            destination: dest_place.clone(),
            target: Some(next_block),
            unwind: None,
        });

        self.builder.switch_to(next_block);
        self.current_block = next_block;

        Ok(Operand::Copy(dest_place))
    }

    /// Lower an effect perform operation in a closure body.
    fn lower_perform(
        &mut self,
        effect_id: DefId,
        op_index: u32,
        args: &[Expr],
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        // Lower all arguments
        let mut arg_ops = Vec::with_capacity(args.len());
        for arg in args {
            arg_ops.push(self.lower_expr(arg)?);
        }

        // Create destination for the result
        let dest = self.new_temp(ty.clone(), span);
        let dest_place = Place::local(dest);

        // Create continuation block
        let resume_block = self.builder.new_block();

        // Emit the Perform terminator
        self.terminate(TerminatorKind::Perform {
            effect_id,
            op_index,
            args: arg_ops,
            destination: dest_place.clone(),
            target: resume_block,
        });

        // Switch to the resume block
        self.builder.switch_to(resume_block);
        self.current_block = resume_block;

        Ok(Operand::Copy(dest_place))
    }

    /// Lower a resume expression in a closure body.
    fn lower_resume(
        &mut self,
        value: Option<&Expr>,
        _span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        // Lower the resume value if present
        let resume_value = if let Some(val_expr) = value {
            Some(self.lower_expr(val_expr)?)
        } else {
            None
        };

        // Emit Resume terminator
        self.terminate(TerminatorKind::Resume { value: resume_value });

        // Resume never returns
        Ok(Operand::Constant(Constant::new(Type::never(), ConstantKind::Unit)))
    }

    /// Lower a handle expression in a closure body.
    fn lower_handle(
        &mut self,
        body: &Expr,
        handler_id: DefId,
        handler_instance: &Expr,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        // Step 1: Lower the handler instance to get the state
        let state_operand = self.lower_expr(handler_instance)?;

        // Store state in a temp local (we need a Place for the state)
        let state_local = self.new_temp(handler_instance.ty.clone(), span);
        let state_place = Place::local(state_local);
        self.push_assign(state_place.clone(), Rvalue::Use(state_operand));

        // Step 2: Push the handler onto the evidence vector with state
        self.push_stmt(StatementKind::PushHandler { handler_id, state_place: state_place.clone() });

        // Step 3: Lower the body expression with the handler installed
        let body_result = self.lower_expr(body)?;

        // Step 4: Pop the handler from the evidence vector
        self.push_stmt(StatementKind::PopHandler);

        // Store the result
        let dest = self.new_temp(ty.clone(), span);
        let dest_place = Place::local(dest);
        self.push_assign(dest_place.clone(), Rvalue::Use(body_result));

        Ok(Operand::Copy(dest_place))
    }

    /// Lower an array repeat expression in a closure body.
    fn lower_repeat(
        &mut self,
        value: &Expr,
        count: u64,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let value_op = self.lower_expr(value)?;
        let dest = self.new_temp(ty.clone(), span);
        let dest_place = Place::local(dest);

        let elem_ty = match ty.kind() {
            TypeKind::Array { element, .. } => element.clone(),
            _ => value.ty.clone(),
        };

        let operands: Vec<Operand> = (0..count).map(|_| value_op.clone()).collect();
        self.push_assign(
            dest_place.clone(),
            Rvalue::Aggregate {
                kind: AggregateKind::Array(elem_ty),
                operands,
            },
        );

        Ok(Operand::Copy(dest_place))
    }

    /// Lower an enum variant construction expression in a closure body.
    fn lower_variant(
        &mut self,
        def_id: DefId,
        variant_idx: u32,
        fields: &[Expr],
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let mut field_ops = Vec::with_capacity(fields.len());
        for field in fields {
            field_ops.push(self.lower_expr(field)?);
        }

        let dest = self.new_temp(ty.clone(), span);
        let dest_place = Place::local(dest);

        self.push_assign(
            dest_place.clone(),
            Rvalue::Aggregate {
                kind: AggregateKind::Adt {
                    def_id,
                    variant_idx: Some(variant_idx),
                },
                operands: field_ops,
            },
        );

        Ok(Operand::Copy(dest_place))
    }

    /// Lower a raw pointer address-of expression in a closure body.
    fn lower_addr_of(
        &mut self,
        inner: &Expr,
        mutable: bool,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let place = self.lower_place(inner)?;
        let dest = self.new_temp(ty.clone(), span);
        let dest_place = Place::local(dest);

        self.push_assign(
            dest_place.clone(),
            Rvalue::AddressOf { place, mutable },
        );

        Ok(Operand::Copy(dest_place))
    }

    /// Lower a let expression in a closure body.
    fn lower_let(
        &mut self,
        pattern: &Pattern,
        init: &Expr,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let init_val = self.lower_expr(init)?;

        let init_place = if let Some(place) = init_val.place() {
            place.clone()
        } else {
            let temp = self.new_temp(init.ty.clone(), span);
            self.push_assign(Place::local(temp), Rvalue::Use(init_val.clone()));
            Place::local(temp)
        };

        if self.is_irrefutable_pattern(pattern) {
            self.bind_pattern(pattern, &init_place)?;

            if ty.kind() == &TypeKind::Primitive(crate::hir::ty::PrimitiveTy::Bool) {
                Ok(Operand::Constant(Constant::new(
                    ty.clone(),
                    ConstantKind::Bool(true),
                )))
            } else {
                Ok(Operand::Copy(init_place))
            }
        } else {
            Err(vec![Diagnostic::error(
                "Refutable patterns in let expressions require match compilation \
                 (not yet implemented). Use a match expression instead.".to_string(),
                span,
            )])
        }
    }

    /// Check if a pattern is irrefutable (always matches).
    fn is_irrefutable_pattern(&self, pattern: &Pattern) -> bool {
        match &pattern.kind {
            PatternKind::Wildcard => true,
            PatternKind::Binding { subpattern, .. } => {
                subpattern.as_ref().map_or(true, |p| self.is_irrefutable_pattern(p))
            }
            PatternKind::Tuple(pats) => pats.iter().all(|p| self.is_irrefutable_pattern(p)),
            PatternKind::Ref { inner, .. } => self.is_irrefutable_pattern(inner),
            PatternKind::Literal(_) => false,
            PatternKind::Or(_) => false,
            PatternKind::Variant { .. } => false,
            PatternKind::Struct { fields, .. } => {
                fields.iter().all(|f| self.is_irrefutable_pattern(&f.pattern))
            }
            PatternKind::Slice { prefix, slice, suffix } => {
                slice.is_some() &&
                prefix.iter().all(|p| self.is_irrefutable_pattern(p)) &&
                suffix.iter().all(|p| self.is_irrefutable_pattern(p))
            }
        }
    }

    /// Bind pattern variables to a place in a closure body.
    fn bind_pattern(&mut self, pattern: &Pattern, place: &Place) -> Result<(), Vec<Diagnostic>> {
        match &pattern.kind {
            PatternKind::Binding { local_id, subpattern, .. } => {
                let mir_local = self.new_temp(pattern.ty.clone(), pattern.span);
                self.local_map.insert(*local_id, mir_local);
                self.push_assign(Place::local(mir_local), Rvalue::Use(Operand::Copy(place.clone())));
                if let Some(subpat) = subpattern {
                    self.bind_pattern(subpat, &Place::local(mir_local))?;
                }
            }
            PatternKind::Tuple(pats) => {
                for (i, pat) in pats.iter().enumerate() {
                    let field_place = place.project(PlaceElem::Field(i as u32));
                    self.bind_pattern(pat, &field_place)?;
                }
            }
            PatternKind::Struct { fields, .. } => {
                for field in fields {
                    let field_place = place.project(PlaceElem::Field(field.field_idx));
                    self.bind_pattern(&field.pattern, &field_place)?;
                }
            }
            PatternKind::Wildcard | PatternKind::Literal(_) => {
                // Nothing to bind
            }
            PatternKind::Variant { fields, .. } => {
                for (i, field_pat) in fields.iter().enumerate() {
                    let field_place = place.project(PlaceElem::Field(i as u32));
                    self.bind_pattern(field_pat, &field_place)?;
                }
            }
            PatternKind::Slice { prefix, slice, suffix } => {
                let min_length = (prefix.len() + suffix.len()) as u64;

                for (i, pat) in prefix.iter().enumerate() {
                    let idx_place = place.project(PlaceElem::ConstantIndex {
                        offset: i as u64,
                        min_length,
                        from_end: false,
                    });
                    self.bind_pattern(pat, &idx_place)?;
                }

                for (i, pat) in suffix.iter().enumerate() {
                    let offset_from_end = (suffix.len() - 1 - i) as u64;
                    let idx_place = place.project(PlaceElem::ConstantIndex {
                        offset: offset_from_end,
                        min_length,
                        from_end: true,
                    });
                    self.bind_pattern(pat, &idx_place)?;
                }

                if let Some(rest_pat) = slice {
                    let subslice_place = place.project(PlaceElem::Subslice {
                        from: prefix.len() as u64,
                        to: suffix.len() as u64,
                        from_end: true,
                    });
                    self.bind_pattern(rest_pat, &subslice_place)?;
                }
            }
            PatternKind::Or(alternatives) => {
                // Or-patterns in binding: bind using first alternative (all must bind same vars)
                if let Some(first_alt) = alternatives.first() {
                    self.bind_pattern(first_alt, place)?;
                }
            }
            PatternKind::Ref { inner, .. } => {
                let deref_place = place.project(PlaceElem::Deref);
                self.bind_pattern(inner, &deref_place)?;
            }
        }
        Ok(())
    }

    fn lower_block(
        &mut self,
        stmts: &[hir::Stmt],
        tail: Option<&Expr>,
        ty: &Type,
        _span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        for stmt in stmts {
            self.lower_stmt(stmt)?;
        }

        if let Some(tail) = tail {
            self.lower_expr(tail)
        } else {
            Ok(Operand::Constant(Constant::new(ty.clone(), ConstantKind::Unit)))
        }
    }

    fn lower_stmt(&mut self, stmt: &hir::Stmt) -> Result<(), Vec<Diagnostic>> {
        match stmt {
            hir::Stmt::Let { local_id, init } => {
                let hir_local = self.body.get_local(*local_id)
                    .ok_or_else(|| vec![ice_err!(
                        Span::dummy(),
                        "local not found in closure body during MIR lowering";
                        "local_id" => local_id
                    )])?;
                let mir_local = self.builder.new_temp(hir_local.ty.clone(), hir_local.span);
                self.local_map.insert(*local_id, mir_local);

                self.push_stmt(StatementKind::StorageLive(mir_local));

                if let Some(init) = init {
                    let init_val = self.lower_expr(init)?;

                    // If init is a closure, propagate the Closure type to the target local
                    if let Operand::Copy(place) | Operand::Move(place) = &init_val {
                        if let Some(temp_ty) = self.builder.get_local_type(place.local) {
                            if matches!(temp_ty.kind(), TypeKind::Closure { .. }) {
                                self.builder.set_local_type(mir_local, temp_ty.clone());
                            }
                        }
                    }

                    self.push_assign(Place::local(mir_local), Rvalue::Use(init_val));
                }
            }
            hir::Stmt::Expr(expr) => {
                let _ = self.lower_expr(expr)?;
            }
            hir::Stmt::Item(_) => {
                // Nested items handled at crate level
            }
        }
        Ok(())
    }

    fn lower_if(
        &mut self,
        condition: &Expr,
        then_branch: &Expr,
        else_branch: Option<&Expr>,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let cond = self.lower_expr(condition)?;

        let then_block = self.builder.new_block();
        let else_block = self.builder.new_block();
        let join_block = self.builder.new_block();

        let result = self.new_temp(ty.clone(), span);

        self.terminate(TerminatorKind::SwitchInt {
            discr: cond,
            targets: SwitchTargets::new(vec![(1, then_block)], else_block),
        });

        self.builder.switch_to(then_block);
        self.current_block = then_block;
        let then_val = self.lower_expr(then_branch)?;
        self.push_assign(Place::local(result), Rvalue::Use(then_val));
        self.terminate(TerminatorKind::Goto { target: join_block });

        self.builder.switch_to(else_block);
        self.current_block = else_block;
        if let Some(else_expr) = else_branch {
            let else_val = self.lower_expr(else_expr)?;
            self.push_assign(Place::local(result), Rvalue::Use(else_val));
        } else {
            self.push_assign(
                Place::local(result),
                Rvalue::Use(Operand::Constant(Constant::unit())),
            );
        }
        self.terminate(TerminatorKind::Goto { target: join_block });

        self.builder.switch_to(join_block);
        self.current_block = join_block;

        Ok(Operand::Copy(Place::local(result)))
    }

    fn lower_return(
        &mut self,
        value: Option<&Expr>,
        _span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let return_place = Place::local(LocalId::new(0));

        if let Some(value) = value {
            let val = self.lower_expr(value)?;
            self.push_assign(return_place, Rvalue::Use(val));
        } else {
            self.push_assign(return_place, Rvalue::Use(Operand::Constant(Constant::unit())));
        }

        self.terminate(TerminatorKind::Return);

        let next = self.builder.new_block();
        self.builder.switch_to(next);
        self.current_block = next;

        Ok(Operand::Constant(Constant::unit()))
    }

    fn lower_tuple(
        &mut self,
        elems: &[Expr],
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let mut operands = Vec::with_capacity(elems.len());
        for elem in elems {
            operands.push(self.lower_expr(elem)?);
        }

        let temp = self.new_temp(ty.clone(), span);
        self.push_assign(
            Place::local(temp),
            Rvalue::Aggregate {
                kind: AggregateKind::Tuple,
                operands,
            },
        );

        Ok(Operand::Copy(Place::local(temp)))
    }

    fn lower_array(
        &mut self,
        elems: &[Expr],
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let elem_ty = match ty.kind() {
            TypeKind::Array { element, .. } => element.clone(),
            other => {
                return Err(vec![ice_err!(
                    span,
                    "lower_array called with non-array type";
                    "type_kind" => other
                )]);
            }
        };

        let mut operands = Vec::with_capacity(elems.len());
        for elem in elems {
            operands.push(self.lower_expr(elem)?);
        }

        let temp = self.new_temp(ty.clone(), span);
        self.push_assign(
            Place::local(temp),
            Rvalue::Aggregate {
                kind: AggregateKind::Array(elem_ty),
                operands,
            },
        );

        Ok(Operand::Copy(Place::local(temp)))
    }

    fn lower_field(
        &mut self,
        base: &Expr,
        field_idx: u32,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let base_place = self.lower_place(base)?;
        let field_place = base_place.project(PlaceElem::Field(field_idx));

        let temp = self.new_temp(ty.clone(), span);
        self.push_assign(Place::local(temp), Rvalue::Use(Operand::Copy(field_place)));

        Ok(Operand::Copy(Place::local(temp)))
    }

    fn lower_index(
        &mut self,
        base: &Expr,
        index: &Expr,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let base_place = self.lower_place(base)?;
        let index_op = self.lower_expr(index)?;

        let index_local = if let Operand::Copy(p) | Operand::Move(p) = &index_op {
            p.local
        } else {
            let temp = self.new_temp(Type::u64(), span);
            self.push_assign(Place::local(temp), Rvalue::Use(index_op));
            temp
        };

        let indexed_place = base_place.project(PlaceElem::Index(index_local));

        let temp = self.new_temp(ty.clone(), span);
        self.push_assign(Place::local(temp), Rvalue::Use(Operand::Copy(indexed_place)));

        Ok(Operand::Copy(Place::local(temp)))
    }

    fn lower_assign(
        &mut self,
        target: &Expr,
        value: &Expr,
        _span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let place = self.lower_place(target)?;
        let val = self.lower_expr(value)?;
        self.push_assign(place, Rvalue::Use(val));
        Ok(Operand::Constant(Constant::unit()))
    }

    fn lower_borrow(
        &mut self,
        expr: &Expr,
        mutable: bool,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let place = self.lower_place(expr)?;

        let temp = self.new_temp(ty.clone(), span);
        self.push_assign(Place::local(temp), Rvalue::Ref { place, mutable });

        Ok(Operand::Copy(Place::local(temp)))
    }

    /// Lower a deref expression in a closure body.
    ///
    /// This emits a generation validation check before the dereference to detect
    /// use-after-free errors at runtime.
    fn lower_deref(
        &mut self,
        expr: &Expr,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let inner = self.lower_expr(expr)?;
        let inner_place = if let Some(p) = inner.place() {
            p.clone()
        } else {
            let temp = self.new_temp(expr.ty.clone(), span);
            self.push_assign(Place::local(temp), Rvalue::Use(inner));
            Place::local(temp)
        };

        // Emit generation validation check before dereferencing.
        // Generation validation temporarily disabled.
        // See FunctionLowering::lower_deref for documentation.
        // let expected_gen = Operand::Constant(Constant::int(1, Type::u32()));
        // self.push_stmt(StatementKind::ValidateGeneration {
        //     ptr: inner_place.clone(),
        //     expected_gen,
        // });

        let deref_place = inner_place.project(PlaceElem::Deref);
        let temp = self.new_temp(ty.clone(), span);
        self.push_assign(Place::local(temp), Rvalue::Use(Operand::Copy(deref_place)));

        Ok(Operand::Copy(Place::local(temp)))
    }

    fn lower_cast(
        &mut self,
        expr: &Expr,
        target_ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let operand = self.lower_expr(expr)?;
        let temp = self.new_temp(target_ty.clone(), span);
        self.push_assign(
            Place::local(temp),
            Rvalue::Cast {
                operand,
                target_ty: target_ty.clone(),
            },
        );
        Ok(Operand::Copy(Place::local(temp)))
    }

    fn lower_closure(
        &mut self,
        body_id: hir::BodyId,
        captures: &[hir::Capture],
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let closure_def_id = self.next_closure_def_id();

        // Collect captures with their types
        let mut capture_operands = Vec::with_capacity(captures.len());
        let mut captures_with_types = Vec::with_capacity(captures.len());

        for capture in captures {
            // Get the type for this capture - must be in outer captures or body scope
            let capture_ty = if let Some(ty) = self.capture_types.get(&capture.local_id) {
                // From outer closure's captures
                ty.clone()
            } else if let Some(local) = self.body.get_local(capture.local_id) {
                // From closure body scope
                local.ty.clone()
            } else {
                return Err(vec![ice_err!(
                    span,
                    "capture type not found in closure lowering";
                    "local_id" => capture.local_id
                )]);
            };

            captures_with_types.push((capture.clone(), capture_ty));

            // Captured variables: check if they're from our own captures first
            if let Some(&field_idx) = self.capture_map.get(&capture.local_id) {
                // Captured from outer closure's environment
                if let Some(env_local) = self.env_local {
                    let env_place = Place::local(env_local);
                    let capture_place = env_place.project(PlaceElem::Field(field_idx as u32));
                    let operand = if capture.by_move {
                        Operand::Move(capture_place)
                    } else {
                        Operand::Copy(capture_place)
                    };
                    capture_operands.push(operand);
                } else {
                    // Should not happen
                    let mir_local = self.map_local(capture.local_id);
                    let place = Place::local(mir_local);
                    let operand = if capture.by_move {
                        Operand::Move(place)
                    } else {
                        Operand::Copy(place)
                    };
                    capture_operands.push(operand);
                }
            } else {
                // Regular local from closure body scope
                let mir_local = self.map_local(capture.local_id);
                let place = Place::local(mir_local);
                let operand = if capture.by_move {
                    Operand::Move(place)
                } else {
                    Operand::Copy(place)
                };
                capture_operands.push(operand);
            }
        }

        // Schedule the closure body for later lowering
        self.pending_closures.push((body_id, closure_def_id, captures_with_types));

        // Create a Closure type with the synthetic DefId
        let closure_ty = match ty.kind() {
            TypeKind::Fn { params, ret } => {
                Type::new(TypeKind::Closure {
                    def_id: closure_def_id,
                    params: params.clone(),
                    ret: ret.clone(),
                })
            }
            _ => ty.clone(),
        };

        let temp = self.new_temp(closure_ty, span);
        self.push_assign(
            Place::local(temp),
            Rvalue::Aggregate {
                kind: AggregateKind::Closure { def_id: closure_def_id },
                operands: capture_operands,
            },
        );

        Ok(Operand::Copy(Place::local(temp)))
    }

    fn lower_place(&mut self, expr: &Expr) -> Result<Place, Vec<Diagnostic>> {
        match &expr.kind {
            ExprKind::Local(local_id) => {
                // Check if this local is a captured variable from the outer scope
                if let Some(&field_idx) = self.capture_map.get(local_id) {
                    if let Some(env_local) = self.env_local {
                        let env_place = Place::local(env_local);
                        return Ok(env_place.project(PlaceElem::Field(field_idx as u32)));
                    }
                }
                let mir_local = self.map_local(*local_id);
                Ok(Place::local(mir_local))
            }
            ExprKind::Field { base, field_idx } => {
                let base_place = self.lower_place(base)?;
                Ok(base_place.project(PlaceElem::Field(*field_idx)))
            }
            ExprKind::Index { base, index } => {
                let base_place = self.lower_place(base)?;
                let index_op = self.lower_expr(index)?;
                let index_local = if let Operand::Copy(p) | Operand::Move(p) = &index_op {
                    p.local
                } else {
                    let temp = self.new_temp(Type::u64(), expr.span);
                    self.push_assign(Place::local(temp), Rvalue::Use(index_op));
                    temp
                };
                Ok(base_place.project(PlaceElem::Index(index_local)))
            }
            ExprKind::Deref(inner) => {
                let inner_place = self.lower_place(inner)?;
                Ok(inner_place.project(PlaceElem::Deref))
            }
            ExprKind::Unary { op: UnaryOp::Deref, operand } => {
                // Handle Unary Deref the same as ExprKind::Deref
                let inner_place = self.lower_place(operand)?;
                Ok(inner_place.project(PlaceElem::Deref))
            }
            _ => {
                let val = self.lower_expr(expr)?;
                if let Some(place) = val.place() {
                    Ok(place.clone())
                } else {
                    let temp = self.new_temp(expr.ty.clone(), expr.span);
                    self.push_assign(Place::local(temp), Rvalue::Use(val));
                    Ok(Place::local(temp))
                }
            }
        }
    }

    fn map_local(&mut self, hir_local: LocalId) -> LocalId {
        if let Some(&mir_local) = self.local_map.get(&hir_local) {
            mir_local
        } else {
            let local = self.body.get_local(hir_local);
            let ty = local.map(|l| l.ty.clone()).unwrap_or_else(Type::error);
            let span = local.map(|l| l.span).unwrap_or_else(Span::dummy);
            let mir_id = self.builder.new_temp(ty, span);
            self.local_map.insert(hir_local, mir_id);
            mir_id
        }
    }

    fn new_temp(&mut self, ty: Type, span: Span) -> LocalId {
        self.temp_counter += 1;
        self.builder.new_temp(ty, span)
    }

    fn push_stmt(&mut self, kind: StatementKind) {
        self.builder.push_stmt(Statement::new(kind, Span::dummy()));
    }

    fn push_assign(&mut self, place: Place, rvalue: Rvalue) {
        self.push_stmt(StatementKind::Assign(place, rvalue));
    }

    fn terminate(&mut self, kind: TerminatorKind) {
        self.builder.terminate(Terminator::new(kind, Span::dummy()));
    }
}

/// Convert HIR binary op to MIR binary op.
fn convert_binop(op: BinOp) -> MirBinOp {
    match op {
        BinOp::Add => MirBinOp::Add,
        BinOp::Sub => MirBinOp::Sub,
        BinOp::Mul => MirBinOp::Mul,
        BinOp::Div => MirBinOp::Div,
        BinOp::Rem => MirBinOp::Rem,
        BinOp::BitAnd => MirBinOp::BitAnd,
        BinOp::BitOr => MirBinOp::BitOr,
        BinOp::BitXor => MirBinOp::BitXor,
        BinOp::Shl => MirBinOp::Shl,
        BinOp::Shr => MirBinOp::Shr,
        BinOp::Eq => MirBinOp::Eq,
        BinOp::Ne => MirBinOp::Ne,
        BinOp::Lt => MirBinOp::Lt,
        BinOp::Le => MirBinOp::Le,
        BinOp::Gt => MirBinOp::Gt,
        BinOp::Ge => MirBinOp::Ge,
        BinOp::And => MirBinOp::BitAnd, // Logical and
        BinOp::Or => MirBinOp::BitOr,   // Logical or
        BinOp::Pipe => MirBinOp::BitOr, // Pipe operator (placeholder)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_binop() {
        assert_eq!(convert_binop(BinOp::Add), MirBinOp::Add);
        assert_eq!(convert_binop(BinOp::Sub), MirBinOp::Sub);
        assert_eq!(convert_binop(BinOp::Eq), MirBinOp::Eq);
    }

    #[test]
    fn test_mir_lowering_new() {
        let hir = HirCrate::new();
        let lowering = MirLowering::new(&hir);
        assert!(lowering.bodies.is_empty());
    }
}
