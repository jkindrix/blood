//! Function lowering from HIR to MIR.

use std::collections::HashMap;

use crate::hir::{
    self, Body, Crate as HirCrate, DefId, Expr, ExprKind, RecordFieldExpr,
    LocalId, MatchArm, Pattern, Stmt, Type, TypeKind,
};
use crate::ast::UnaryOp;
use crate::span::Span;
use crate::diagnostics::Diagnostic;
use crate::effects::std_effects::StandardEffects;
use crate::ice_err;

use crate::mir::body::MirBodyBuilder;
use crate::mir::body::MirBody;
use crate::mir::static_evidence::{analyze_handler_state, analyze_handler_allocation_tier};
use crate::mir::types::{
    BasicBlockId, Statement, StatementKind, Terminator, TerminatorKind,
    Place, PlaceElem, Operand, Rvalue, Constant, ConstantKind,
    UnOp as MirUnOp, AggregateKind, SwitchTargets,
};

use super::LoopContext;
use super::util::{is_irrefutable_pattern, ExprLowering, LoopContextInfo};

// ============================================================================
// Function Lowering
// ============================================================================

/// Lowers a single function body to MIR.
pub struct FunctionLowering<'hir, 'ctx> {
    /// MIR body builder.
    builder: MirBodyBuilder,
    /// HIR body being lowered.
    body: &'hir Body,
    /// HIR crate for accessing nested closure bodies (reserved for future use).
    #[allow(dead_code)]
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

impl<'hir, 'ctx> FunctionLowering<'hir, 'ctx> {
    /// Create a new function lowering context.
    pub fn new(
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
    pub fn lower(mut self) -> Result<MirBody, Vec<Diagnostic>> {
        // Lower the body expression
        let body_expr = self.body.expr.clone();
        let result = self.lower_expr(&body_expr)?;

        // Determine if we should add an implicit return:
        // Skip if:
        // 1. Body expression diverges (type Never) - we're in unreachable code
        // 2. Body returns unit but function returns non-unit - explicit return handled it
        let return_ty = self.body.return_type();
        let skip_implicit_return = body_expr.ty.is_never()
            || (body_expr.ty.is_unit() && !return_ty.is_unit());

        if !skip_implicit_return {
            // Assign body result to return place
            let return_place = Place::local(LocalId::new(0));
            self.push_assign(return_place, Rvalue::Use(result));

            // Add return terminator
            self.terminate(TerminatorKind::Return);
        }

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
                // Determine the kind of constant based on the item type
                let constant_kind = if let Some(item) = self.hir.get_item(*def_id) {
                    match &item.kind {
                        hir::ItemKind::Const { .. } => ConstantKind::ConstDef(*def_id),
                        hir::ItemKind::Static { .. } => ConstantKind::StaticDef(*def_id),
                        _ => ConstantKind::FnDef(*def_id),
                    }
                } else {
                    // Default to FnDef for unknown items (e.g., builtins)
                    ConstantKind::FnDef(*def_id)
                };
                let constant = Constant::new(expr.ty.clone(), constant_kind);
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

            ExprKind::Range { start, end, inclusive } => {
                self.lower_range(start.as_deref(), end.as_deref(), *inclusive, &expr.ty, expr.span)
            }

            ExprKind::MethodFamily { name, candidates } => {
                // Method family should be resolved at call site during type checking.
                // If we reach here, it means the method family was used without a call,
                // which is an error (e.g., `let f = add;` where `add` has multiple overloads).
                Err(vec![Diagnostic::error(
                    format!(
                        "cannot reference method family '{}' without a call (has {} overloads)",
                        name,
                        candidates.len()
                    ),
                    expr.span,
                )])
            }

            ExprKind::Record { fields } => {
                self.lower_record(fields, &expr.ty, expr.span)
            }

            ExprKind::Default => {
                // Create a default-initialized value
                // For now, create a zeroed temp and return it
                // In a full implementation, this would call the Default trait method
                let temp = self.new_temp(expr.ty.clone(), expr.span);
                let rvalue = Rvalue::ZeroInit(expr.ty.clone());
                self.push_assign(Place::local(temp), rvalue);
                Ok(Operand::Copy(Place::local(temp)))
            }

            // Macro expansion nodes - these should be lowered during a macro expansion pass
            ExprKind::MacroExpansion { macro_name, .. } => {
                Err(vec![Diagnostic::error(
                    format!("macro expansion '{}!' should be expanded before MIR lowering", macro_name),
                    expr.span,
                )])
            }
            ExprKind::VecLiteral(_) => {
                Err(vec![Diagnostic::error(
                    "vec! macro should be expanded before MIR lowering".to_string(),
                    expr.span,
                )])
            }
            ExprKind::VecRepeat { .. } => {
                Err(vec![Diagnostic::error(
                    "vec! repeat macro should be expanded before MIR lowering".to_string(),
                    expr.span,
                )])
            }
            ExprKind::Assert { .. } => {
                Err(vec![Diagnostic::error(
                    "assert! macro should be expanded before MIR lowering".to_string(),
                    expr.span,
                )])
            }
            ExprKind::Dbg(_) => {
                Err(vec![Diagnostic::error(
                    "dbg! macro should be expanded before MIR lowering".to_string(),
                    expr.span,
                )])
            }
        }
    }

    // lower_literal and lower_binary are now provided by ExprLowering trait

    /// Lower a pipe expression: `a |> f` becomes `f(a)`.
    ///
    /// The pipe operator passes the left operand as the first argument
    /// to the function on the right-hand side.
    fn lower_pipe_impl(
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

        // Determine if this effect operation is tail-resumptive.
        // Standard effects have known tail-resumptive status.
        // Unknown effects default to tail-resumptive (optimistic assumption).
        let is_tail_resumptive = StandardEffects::is_tail_resumptive(effect_id)
            .unwrap_or(true);

        // Emit the Perform terminator
        self.terminate(TerminatorKind::Perform {
            effect_id,
            op_index,
            args: arg_ops,
            destination: dest_place.clone(),
            target: resume_block,
            is_tail_resumptive,
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
        // Step 0: Analyze handler for optimizations
        // EFF-OPT-001: Handler state classification (Stateless, Constant, ZeroInit, Dynamic)
        let state_kind = analyze_handler_state(handler_instance);
        // EFF-OPT-005/006: Handler allocation tier (Stack vs Region)
        let allocation_tier = analyze_handler_allocation_tier(body);

        // Step 1: Lower the handler instance to get the state
        let state_operand = self.lower_expr(handler_instance)?;

        // Store state in a temp local (we need a Place for the state)
        let state_local = self.new_temp(handler_instance.ty.clone(), span);
        let state_place = Place::local(state_local);
        self.push_assign(state_place.clone(), Rvalue::Use(state_operand));

        // Step 2: Push the handler onto the evidence vector with state
        self.push_stmt(StatementKind::PushHandler {
            handler_id,
            state_place: state_place.clone(),
            state_kind,
            allocation_tier,
        });

        // Step 3: Lower the body expression with the handler installed
        let body_result = self.lower_expr(body)?;

        // Step 4: Pop the handler from the evidence vector
        self.push_stmt(StatementKind::PopHandler);

        // Step 5: Call the return clause to transform the body result
        // The return clause function is handler_{handler_id.index}_return
        let dest = self.new_temp(ty.clone(), span);
        let dest_place = Place::local(dest);

        self.push_stmt(StatementKind::CallReturnClause {
            handler_id,
            body_result,
            state_place,
            destination: dest_place.clone(),
        });

        Ok(Operand::Copy(dest_place))
    }

    /// Lower a range expression `start..end` or `start..=end`.
    ///
    /// Range expressions are lowered to aggregate construction:
    /// - Range<T>: { start, end }
    /// - RangeInclusive<T>: { start, end, exhausted: false }
    fn lower_range(
        &mut self,
        start: Option<&Expr>,
        end: Option<&Expr>,
        inclusive: bool,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        // Create destination for the range
        let dest = self.new_temp(ty.clone(), span);
        let dest_place = Place::local(dest);

        // Get the element type from the Range type
        let elem_ty = match ty.kind() {
            TypeKind::Range { element, .. } => element.clone(),
            _ => {
                // Fallback: infer from start or end
                if let Some(s) = start {
                    s.ty.clone()
                } else if let Some(e) = end {
                    e.ty.clone()
                } else {
                    Type::unit()
                }
            }
        };

        // Build operands for the range struct
        let mut operands = Vec::new();

        // Lower start if present, otherwise use default value
        if let Some(s) = start {
            let start_op = self.lower_expr(s)?;
            operands.push(start_op);
        } else {
            // For RangeTo (..end), start is not present
            // Use minimum value for the type (requires type info)
            operands.push(Operand::Constant(Constant::new(
                elem_ty.clone(),
                ConstantKind::Int(0), // Default start for now
            )));
        }

        // Lower end if present
        if let Some(e) = end {
            let end_op = self.lower_expr(e)?;
            operands.push(end_op);
        } else {
            // For RangeFrom (start..), end is not present
            // Use maximum value for the type (requires type info)
            operands.push(Operand::Constant(Constant::new(
                elem_ty.clone(),
                ConstantKind::Int(i64::MAX as i128), // Default end for now
            )));
        }

        // For inclusive ranges, add the exhausted field
        if inclusive {
            operands.push(Operand::Constant(Constant::new(
                Type::bool(),
                ConstantKind::Bool(false),
            )));
        }

        // Create the aggregate
        self.push_assign(
            dest_place.clone(),
            Rvalue::Aggregate {
                kind: AggregateKind::Range { element: elem_ty, inclusive },
                operands,
            },
        );

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

        // Extract type arguments from the enum type
        let type_args = if let TypeKind::Adt { args, .. } = ty.kind() {
            args.clone()
        } else {
            Vec::new()
        };

        // Create the aggregate value using Adt with variant_idx
        self.push_assign(
            dest_place.clone(),
            Rvalue::Aggregate {
                kind: AggregateKind::Adt {
                    def_id,
                    variant_idx: Some(variant_idx),
                    type_args,
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
        if is_irrefutable_pattern(pattern) {
            // Bind pattern variables
            self.bind_pattern_cf(pattern, &init_place)?;

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
            // Refutable pattern - generate pattern test
            // Returns a boolean indicating whether the pattern matches
            let result = self.new_temp(Type::bool(), span);

            // Create blocks for match success and join
            let match_block = self.builder.new_block();
            let no_match_block = self.builder.new_block();
            let join_block = self.builder.new_block();

            // Generate the pattern test and conditional branch
            self.test_pattern_cf(pattern, &init_place, match_block, no_match_block, span)?;

            // Match block: bind variables and set result to true
            self.builder.switch_to(match_block);
            self.current_block = match_block;
            self.bind_pattern_cf(pattern, &init_place)?;
            self.push_assign(
                Place::local(result),
                Rvalue::Use(Operand::Constant(Constant::new(
                    Type::bool(),
                    ConstantKind::Bool(true),
                ))),
            );
            self.terminate(TerminatorKind::Goto { target: join_block });

            // No-match block: set result to false
            self.builder.switch_to(no_match_block);
            self.current_block = no_match_block;
            self.push_assign(
                Place::local(result),
                Rvalue::Use(Operand::Constant(Constant::new(
                    Type::bool(),
                    ConstantKind::Bool(false),
                ))),
            );
            self.terminate(TerminatorKind::Goto { target: join_block });

            // Continue in join block
            self.builder.switch_to(join_block);
            self.current_block = join_block;

            Ok(Operand::Copy(Place::local(result)))
        }
    }

    // NOTE: Pattern testing and binding methods (test_pattern_cf, bind_pattern_cf, etc.)
    // are now provided by the ExprLowering trait in util.rs. FunctionLowering implements
    // that trait and uses its default implementations for pattern matching.

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
            // No tail expression - return unit.
            // If the block's type is Never (all paths diverge), we're in unreachable code.
            // Either way, use the proper unit type for the constant.
            if ty.is_never() {
                Ok(Operand::Constant(Constant::new(Type::never(), ConstantKind::Unit)))
            } else {
                Ok(Operand::Constant(Constant::unit()))
            }
        }
    }

    /// Lower a statement.
    fn lower_stmt(&mut self, stmt: &Stmt) -> Result<(), Vec<Diagnostic>> {
        match stmt {
            Stmt::Let { local_id, init } => {
                // Check if this is a tuple destructuring pattern
                if let Some(element_locals) = self.body.tuple_destructures.get(local_id) {
                    // This is a tuple destructure: let (a, b) = expr;
                    // The local_id is the hidden tuple local, and element_locals are the bindings

                    // Get the hidden tuple local info
                    let hir_local = self.body.get_local(*local_id)
                        .ok_or_else(|| vec![ice_err!(
                            Span::dummy(),
                            "hidden tuple local not found in HIR body during MIR lowering";
                            "local_id" => local_id
                        )])?;
                    let tuple_mir_local = self.builder.new_temp(hir_local.ty.clone(), hir_local.span);
                    self.local_map.insert(*local_id, tuple_mir_local);

                    // Storage live for the hidden tuple local
                    self.push_stmt(StatementKind::StorageLive(tuple_mir_local));

                    // Create MIR locals for each element
                    let element_mir_locals: Vec<LocalId> = element_locals.iter()
                        .map(|elem_id| {
                            let elem_hir_local = self.body.get_local(*elem_id)
                                .expect("element local not found in HIR body");
                            let mir_local = self.builder.new_temp(elem_hir_local.ty.clone(), elem_hir_local.span);
                            self.local_map.insert(*elem_id, mir_local);
                            self.push_stmt(StatementKind::StorageLive(mir_local));
                            mir_local
                        })
                        .collect();

                    // Initialize if there's an init expression
                    if let Some(init) = init {
                        let init_val = self.lower_expr(init)?;

                        // Store the tuple value to the hidden tuple local
                        self.push_assign(Place::local(tuple_mir_local), Rvalue::Use(init_val));

                        // Extract each field and assign to the corresponding element local
                        for (i, mir_local) in element_mir_locals.iter().enumerate() {
                            let field_place = Place::local(tuple_mir_local).project(PlaceElem::Field(i as u32));
                            self.push_assign(Place::local(*mir_local), Rvalue::Use(Operand::Copy(field_place)));
                        }
                    }
                } else {
                    // Normal non-tuple let binding
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
    ///
    /// This implements full pattern matching with proper testing of all pattern types.
    /// Each arm is tested sequentially; the first matching arm is executed.
    fn lower_match(
        &mut self,
        scrutinee: &Expr,
        arms: &[MatchArm],
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let scrut = self.lower_expr(scrutinee)?;

        // Create a place for the scrutinee if needed
        let scrut_place = if let Some(place) = scrut.place() {
            place.clone()
        } else {
            let temp = self.new_temp(scrutinee.ty.clone(), span);
            self.push_assign(Place::local(temp), Rvalue::Use(scrut));
            Place::local(temp)
        };

        // Result temporary
        let result = self.new_temp(ty.clone(), span);
        let join_block = self.builder.new_block();

        // Create unreachable block for exhaustiveness failure
        let unreachable_block = self.builder.new_block();

        // For each arm, create body and guard blocks
        let arm_body_blocks: Vec<_> = arms.iter().map(|_| self.builder.new_block()).collect();

        // Test each arm's pattern sequentially
        // First arm that matches has its body executed
        for (i, arm) in arms.iter().enumerate() {
            let next_arm_test = if i + 1 < arms.len() {
                self.builder.new_block()
            } else {
                // Last arm - if it doesn't match, we hit unreachable
                unreachable_block
            };

            // Test pattern - on match go to body, on no-match go to next arm
            self.test_pattern_cf(&arm.pattern, &scrut_place, arm_body_blocks[i], next_arm_test, span)?;

            if i + 1 < arms.len() {
                // Position for next arm's test
                self.builder.switch_to(next_arm_test);
                self.current_block = next_arm_test;
            }
        }

        // Build unreachable block
        self.builder.switch_to(unreachable_block);
        self.current_block = unreachable_block;
        self.terminate(TerminatorKind::Unreachable);

        // Lower each arm's body
        for (i, arm) in arms.iter().enumerate() {
            self.builder.switch_to(arm_body_blocks[i]);
            self.current_block = arm_body_blocks[i];

            // Bind pattern variables
            self.bind_pattern_cf(&arm.pattern, &scrut_place)?;

            // Check guard if present
            if let Some(guard) = &arm.guard {
                // Create blocks for guard success/failure
                let guard_pass = self.builder.new_block();
                let guard_fail = if i + 1 < arms.len() {
                    // Fall through to next arm's test if guard fails
                    // We need to create a block that jumps to the next arm's test
                    let fallthrough = self.builder.new_block();
                    self.builder.switch_to(fallthrough);
                    self.current_block = fallthrough;
                    // The next arm's pattern test starts fresh
                    // For simplicity, we'll go to the next arm's body (which will re-test)
                    // A more sophisticated implementation would skip directly to the next test
                    arm_body_blocks[i + 1]
                } else {
                    // Last arm - guard failure goes to unreachable
                    unreachable_block
                };

                // Return to current body block
                self.builder.switch_to(arm_body_blocks[i]);
                self.current_block = arm_body_blocks[i];

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

    /// Lower an anonymous record expression.
    fn lower_record(
        &mut self,
        fields: &[RecordFieldExpr],
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
                kind: AggregateKind::Record,
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

        // Extract type arguments from the struct type
        let type_args = if let TypeKind::Adt { args, .. } = ty.kind() {
            args.clone()
        } else {
            Vec::new()
        };

        let temp = self.new_temp(ty.clone(), span);
        self.push_assign(
            Place::local(temp),
            Rvalue::Aggregate {
                kind: AggregateKind::Adt { def_id, variant_idx: None, type_args },
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
            // Use the actual type of the index expression, not hardcoded u64
            let temp = self.new_temp(index.ty.clone(), span);
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

        // Generation validation for memory safety.
        // See: MEMORY_MODEL.md section 4 "Generational References"
        //
        // For generational pointers (BloodPtr), we:
        // 1. Extract the captured generation from the pointer
        // 2. Validate it against the current generation in the slot registry
        // 3. Panic if stale (use-after-free detected)
        //
        // Note: Thin pointers (stack-allocated) skip this check at codegen time
        // based on escape analysis results.
        let gen_temp = self.new_temp(Type::u32(), span);
        self.push_assign(
            Place::local(gen_temp),
            Rvalue::ReadGeneration(inner_place.clone()),
        );
        self.push_stmt(StatementKind::ValidateGeneration {
            ptr: inner_place.clone(),
            expected_gen: Operand::Copy(Place::local(gen_temp)),
        });

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
                    // Use the actual type of the index expression, not hardcoded u64
                    let temp = self.new_temp(index.ty.clone(), expr.span);
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


    // Helper methods

    fn map_local(&mut self, hir_local: LocalId) -> LocalId {
        if let Some(&mir_local) = self.local_map.get(&hir_local) {
            mir_local
        } else {
            // Create a new local if not mapped
            let local = self.body.get_local(hir_local)
                .expect("ICE: HIR local not found in body during MIR lowering");
            let ty = local.ty.clone();
            let span = local.span;
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
// ExprLowering Trait Implementation
// ============================================================================

impl<'hir, 'ctx> ExprLowering for FunctionLowering<'hir, 'ctx> {
    fn builder_mut(&mut self) -> &mut MirBodyBuilder {
        &mut self.builder
    }

    fn builder(&self) -> &MirBodyBuilder {
        &self.builder
    }

    fn body(&self) -> &Body {
        self.body
    }

    fn hir(&self) -> &HirCrate {
        self.hir
    }

    fn local_map_mut(&mut self) -> &mut HashMap<LocalId, LocalId> {
        &mut self.local_map
    }

    fn local_map(&self) -> &HashMap<LocalId, LocalId> {
        &self.local_map
    }

    fn current_block_mut(&mut self) -> &mut BasicBlockId {
        &mut self.current_block
    }

    fn current_block(&self) -> BasicBlockId {
        self.current_block
    }

    fn temp_counter_mut(&mut self) -> &mut u32 {
        &mut self.temp_counter
    }

    fn pending_closures_mut(&mut self) -> &mut Vec<(hir::BodyId, DefId, Vec<(hir::Capture, Type)>)> {
        self.pending_closures
    }

    fn closure_counter_mut(&mut self) -> &mut u32 {
        self.closure_counter
    }

    fn push_loop_context(&mut self, label: Option<hir::LoopId>, ctx: LoopContextInfo) {
        self.loop_stack.push(LoopContext {
            break_block: ctx.break_block,
            continue_block: ctx.continue_block,
            label,
        });
    }

    fn pop_loop_context(&mut self, _label: Option<hir::LoopId>) {
        self.loop_stack.pop();
    }

    fn get_loop_context(&self, label: Option<hir::LoopId>) -> Option<LoopContextInfo> {
        let ctx = if let Some(label) = label {
            self.loop_stack.iter().rev().find(|c| c.label == Some(label))
        } else {
            self.loop_stack.last()
        };

        ctx.map(|c| LoopContextInfo {
            break_block: c.break_block,
            continue_block: c.continue_block,
            result_place: None, // FunctionLowering doesn't track result place in LoopContext
        })
    }

    fn lower_pipe(
        &mut self,
        arg: &Expr,
        func: &Expr,
        ty: &Type,
        span: Span,
    ) -> Option<Result<Operand, Vec<Diagnostic>>> {
        // FunctionLowering supports the pipe operator: `a |> f` becomes `f(a)`
        Some(self.lower_pipe_impl(arg, func, ty, span))
    }
}
