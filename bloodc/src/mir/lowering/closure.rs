//! Closure lowering from HIR to MIR.

use std::collections::HashMap;

use crate::hir::{
    self, Body, Crate as HirCrate, DefId, Expr, ExprKind, FieldExpr, LoopId, MatchArm,
    LocalId, LiteralValue, Pattern, PatternKind, RecordFieldExpr, Type, TypeKind,
};
use crate::ast::{BinOp, UnaryOp};
use crate::span::Span;
use crate::diagnostics::Diagnostic;
use crate::effects::std_effects::StandardEffects;
use crate::ice_err;

use crate::mir::body::MirBodyBuilder;
use crate::mir::body::MirBody;
use crate::mir::types::{
    BasicBlockId, Statement, StatementKind, Terminator, TerminatorKind,
    Place, PlaceElem, Operand, Rvalue, Constant, ConstantKind,
    BinOp as MirBinOp, UnOp as MirUnOp, AggregateKind, SwitchTargets,
};

use super::util::{convert_binop, lower_literal_to_constant, is_irrefutable_pattern};

// ============================================================================
// Closure Lowering
// ============================================================================

/// Lowers a closure body to MIR.
///
/// Similar to FunctionLowering but handles closure-specific semantics:
/// - Captured variables are accessed through the environment
/// - The environment is passed as the first implicit parameter
pub struct ClosureLowering<'hir, 'ctx> {
    /// MIR body builder.
    builder: MirBodyBuilder,
    /// HIR body being lowered.
    body: &'hir Body,
    /// HIR crate for accessing nested closure bodies (reserved for future use).
    #[allow(dead_code)]
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
    /// Loop context stack for break/continue (for labeled loops).
    loop_contexts: HashMap<LoopId, (BasicBlockId, BasicBlockId, Option<Place>)>,
    /// Current loop context (break_block, continue_block, result_place).
    current_loop: Option<(BasicBlockId, BasicBlockId, Option<Place>)>,
    /// Counter for unique temporary names.
    temp_counter: u32,
    /// Pending closures to be lowered (for nested closures).
    pending_closures: &'ctx mut Vec<(hir::BodyId, DefId, Vec<(hir::Capture, Type)>)>,
    /// Counter for generating synthetic closure DefIds.
    closure_counter: &'ctx mut u32,
}

impl<'hir, 'ctx> ClosureLowering<'hir, 'ctx> {
    /// Create a new closure lowering context.
    pub fn new(
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

        // Only add an implicit environment parameter if the closure has captures.
        // Closures without captures have the same signature as regular functions,
        // allowing them to be used directly as function pointers.
        let env_local = if !captures.is_empty() {
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
            loop_contexts: HashMap::new(),
            current_loop: None,
            temp_counter: 0,
            pending_closures,
            closure_counter,
        }
    }

    /// Lower the closure body.
    pub fn lower(mut self) -> Result<MirBody, Vec<Diagnostic>> {
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

            ExprKind::Struct { def_id, fields, base } => {
                self.lower_struct(*def_id, fields, base.as_deref(), &expr.ty, expr.span)
            }

            ExprKind::Record { fields } => {
                self.lower_record(fields, &expr.ty, expr.span)
            }

            ExprKind::MethodCall { .. } => {
                Err(vec![Diagnostic::error(
                    "method calls should be desugared before MIR lowering".to_string(),
                    expr.span,
                )])
            }

            ExprKind::Range { start, end, inclusive } => {
                self.lower_range(start.as_deref(), end.as_deref(), *inclusive, &expr.ty, expr.span)
            }

            ExprKind::MethodFamily { name, candidates } => {
                // Method families should be resolved during type checking
                Err(vec![Diagnostic::error(
                    format!("unresolved method family '{}' with {} candidates", name, candidates.len()),
                    expr.span,
                )])
            }

            ExprKind::Default => {
                // Create a default value - for now, return a zero-initialized value
                // In the future, this should call Default::default() trait method
                let temp = self.new_temp(expr.ty.clone(), expr.span);
                let rvalue = Rvalue::ZeroInit(expr.ty.clone());
                self.push_assign(Place::local(temp), rvalue);
                Ok(Operand::Copy(Place::local(temp)))
            }

            ExprKind::Error => {
                Err(vec![Diagnostic::error("lowering error expression".to_string(), expr.span)])
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

        // Determine tail-resumptiveness
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

        // Extract type arguments from the enum type
        let type_args = if let TypeKind::Adt { args, .. } = ty.kind() {
            args.clone()
        } else {
            Vec::new()
        };

        let dest = self.new_temp(ty.clone(), span);
        let dest_place = Place::local(dest);

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

        if is_irrefutable_pattern(pattern) {
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
            // Refutable pattern - generate pattern test
            let result = self.new_temp(Type::bool(), span);

            let match_block = self.builder.new_block();
            let no_match_block = self.builder.new_block();
            let join_block = self.builder.new_block();

            self.test_pattern(pattern, &init_place, match_block, no_match_block, span)?;

            self.builder.switch_to(match_block);
            self.current_block = match_block;
            self.bind_pattern(pattern, &init_place)?;
            self.push_assign(
                Place::local(result),
                Rvalue::Use(Operand::Constant(Constant::new(
                    Type::bool(),
                    ConstantKind::Bool(true),
                ))),
            );
            self.terminate(TerminatorKind::Goto { target: join_block });

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

            self.builder.switch_to(join_block);
            self.current_block = join_block;

            Ok(Operand::Copy(Place::local(result)))
        }
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

        // For each arm, create body blocks
        let arm_body_blocks: Vec<_> = arms.iter().map(|_| self.builder.new_block()).collect();

        // Test each arm's pattern sequentially
        for (i, arm) in arms.iter().enumerate() {
            let next_arm_test = if i + 1 < arms.len() {
                self.builder.new_block()
            } else {
                unreachable_block
            };

            self.test_pattern(&arm.pattern, &scrut_place, arm_body_blocks[i], next_arm_test, span)?;

            if i + 1 < arms.len() {
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
            self.bind_pattern(&arm.pattern, &scrut_place)?;

            // Check guard if present
            if let Some(guard) = &arm.guard {
                let guard_pass = self.builder.new_block();
                let guard_fail = if i + 1 < arms.len() {
                    arm_body_blocks[i + 1]
                } else {
                    unreachable_block
                };

                let guard_result = self.lower_expr(guard)?;
                self.terminate(TerminatorKind::SwitchInt {
                    discr: guard_result,
                    targets: SwitchTargets::new(vec![(1, guard_pass)], guard_fail),
                });

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
        label: Option<LoopId>,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let loop_block = self.builder.new_block();
        let exit_block = self.builder.new_block();

        // Result for break values
        let result = self.new_temp(ty.clone(), span);

        // Save and set loop context (exit_block, continue_block, result_place)
        let loop_ctx = (exit_block, loop_block, Some(Place::local(result)));
        let old_loop = self.current_loop.take();
        self.current_loop = Some(loop_ctx.clone());
        if let Some(label) = label {
            self.loop_contexts.insert(label, loop_ctx);
        }

        // Jump to loop
        self.terminate(TerminatorKind::Goto { target: loop_block });

        // Loop body
        self.builder.switch_to(loop_block);
        self.current_block = loop_block;
        let _ = self.lower_expr(body)?;

        // Loop back (only if block isn't already terminated)
        if !self.builder.is_current_terminated() {
            self.terminate(TerminatorKind::Goto { target: loop_block });
        }

        // Restore loop context
        if let Some(label) = label {
            self.loop_contexts.remove(&label);
        }
        self.current_loop = old_loop;

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
        label: Option<LoopId>,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let cond_block = self.builder.new_block();
        let body_block = self.builder.new_block();
        let exit_block = self.builder.new_block();

        let result = self.new_temp(ty.clone(), span);

        // Save and set loop context (exit_block, continue_block, result_place)
        let loop_ctx = (exit_block, cond_block, Some(Place::local(result)));
        let old_loop = self.current_loop.take();
        self.current_loop = Some(loop_ctx.clone());
        if let Some(label) = label {
            self.loop_contexts.insert(label, loop_ctx);
        }

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
        if !self.builder.is_current_terminated() {
            self.terminate(TerminatorKind::Goto { target: cond_block });
        }

        // Restore loop context
        if let Some(label) = label {
            self.loop_contexts.remove(&label);
        }
        self.current_loop = old_loop;

        // Exit
        self.builder.switch_to(exit_block);
        self.current_block = exit_block;

        Ok(Operand::Copy(Place::local(result)))
    }

    /// Lower a break expression.
    fn lower_break(
        &mut self,
        label: Option<LoopId>,
        value: Option<&Expr>,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        // Get loop context (exit_block, continue_block, result_place)
        let ctx = if let Some(label) = label {
            self.loop_contexts.get(&label).cloned()
        } else {
            self.current_loop.clone()
        };

        if let Some((exit_block, _continue_block, result_place)) = ctx {
            if let Some(value) = value {
                let val = self.lower_expr(value)?;
                if let Some(rp) = &result_place {
                    self.push_assign(rp.clone(), Rvalue::Use(val));
                }
            }
            self.terminate(TerminatorKind::Goto { target: exit_block });
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
        label: Option<LoopId>,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        // Get loop context (exit_block, continue_block, result_place)
        let ctx = if let Some(label) = label {
            self.loop_contexts.get(&label).cloned()
        } else {
            self.current_loop.clone()
        };

        if let Some((_exit_block, continue_block, _result_place)) = ctx {
            self.terminate(TerminatorKind::Goto { target: continue_block });
        } else {
            return Err(vec![Diagnostic::error("continue outside of loop".to_string(), span)]);
        }

        let next = self.builder.new_block();
        self.builder.switch_to(next);
        self.current_block = next;

        Ok(Operand::Constant(Constant::unit()))
    }

    /// Lower a struct expression.
    fn lower_struct(
        &mut self,
        def_id: DefId,
        fields: &[FieldExpr],
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

    /// Lower a range expression.
    fn lower_range(
        &mut self,
        start: Option<&Expr>,
        end: Option<&Expr>,
        inclusive: bool,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let dest = self.new_temp(ty.clone(), span);
        let dest_place = Place::local(dest);

        // Get the element type from the Range type
        let elem_ty = match ty.kind() {
            TypeKind::Range { element, .. } => element.clone(),
            _ => {
                if let Some(s) = start {
                    s.ty.clone()
                } else if let Some(e) = end {
                    e.ty.clone()
                } else {
                    Type::unit()
                }
            }
        };

        let mut operands = Vec::new();

        // Lower start
        if let Some(s) = start {
            let start_op = self.lower_expr(s)?;
            operands.push(start_op);
        } else {
            operands.push(Operand::Constant(Constant::new(
                elem_ty.clone(),
                ConstantKind::Int(0),
            )));
        }

        // Lower end
        if let Some(e) = end {
            let end_op = self.lower_expr(e)?;
            operands.push(end_op);
        } else {
            operands.push(Operand::Constant(Constant::new(
                elem_ty.clone(),
                ConstantKind::Int(i64::MAX as i128),
            )));
        }

        // For inclusive ranges, add exhausted field
        if inclusive {
            operands.push(Operand::Constant(Constant::new(
                Type::bool(),
                ConstantKind::Bool(false),
            )));
        }

        self.push_assign(
            dest_place.clone(),
            Rvalue::Aggregate {
                kind: AggregateKind::Range { element: elem_ty, inclusive },
                operands,
            },
        );

        Ok(Operand::Copy(dest_place))
    }

    /// Generate MIR to test if a pattern matches a value (closure variant).
    fn test_pattern(
        &mut self,
        pattern: &Pattern,
        place: &Place,
        on_match: BasicBlockId,
        on_no_match: BasicBlockId,
        span: Span,
    ) -> Result<(), Vec<Diagnostic>> {
        match &pattern.kind {
            PatternKind::Wildcard => {
                self.terminate(TerminatorKind::Goto { target: on_match });
            }

            PatternKind::Binding { subpattern, .. } => {
                if let Some(subpat) = subpattern {
                    self.test_pattern(subpat, place, on_match, on_no_match, span)?;
                } else {
                    self.terminate(TerminatorKind::Goto { target: on_match });
                }
            }

            PatternKind::Literal(lit) => {
                let lit_const = lower_literal_to_constant(lit, &pattern.ty);
                let lit_operand = Operand::Constant(lit_const);
                let value_operand = Operand::Copy(place.clone());

                let cmp_result = self.new_temp(Type::bool(), span);
                self.push_assign(
                    Place::local(cmp_result),
                    Rvalue::BinaryOp {
                        op: MirBinOp::Eq,
                        left: value_operand,
                        right: lit_operand,
                    },
                );

                self.terminate(TerminatorKind::SwitchInt {
                    discr: Operand::Copy(Place::local(cmp_result)),
                    targets: SwitchTargets::new(vec![(1, on_match)], on_no_match),
                });
            }

            PatternKind::Variant { variant_idx, fields, .. } => {
                let discr_temp = self.new_temp(Type::i32(), span);
                self.push_assign(
                    Place::local(discr_temp),
                    Rvalue::Discriminant(place.clone()),
                );

                if fields.is_empty() {
                    self.terminate(TerminatorKind::SwitchInt {
                        discr: Operand::Copy(Place::local(discr_temp)),
                        targets: SwitchTargets::new(
                            vec![(*variant_idx as u128, on_match)],
                            on_no_match,
                        ),
                    });
                } else {
                    let fields_test_block = self.builder.new_block();
                    self.terminate(TerminatorKind::SwitchInt {
                        discr: Operand::Copy(Place::local(discr_temp)),
                        targets: SwitchTargets::new(
                            vec![(*variant_idx as u128, fields_test_block)],
                            on_no_match,
                        ),
                    });

                    // Test each field pattern on the downcasted variant
                    // The Downcast projection tells codegen we're inside a variant,
                    // so Field projections will be offset by 1 for the discriminant tag.
                    self.builder.switch_to(fields_test_block);
                    self.current_block = fields_test_block;
                    let variant_place = place.project(PlaceElem::Downcast(*variant_idx));
                    self.test_pattern_fields(fields, &variant_place, on_match, on_no_match, span)?;
                }
            }

            PatternKind::Tuple(pats) => {
                self.test_pattern_tuple(pats, place, on_match, on_no_match, span)?;
            }

            PatternKind::Struct { fields, .. } => {
                let field_pats: Vec<_> = fields.iter()
                    .map(|f| (f.field_idx, &f.pattern))
                    .collect();
                self.test_pattern_struct_fields(&field_pats, place, on_match, on_no_match, span)?;
            }

            PatternKind::Or(alternatives) => {
                if alternatives.is_empty() {
                    self.terminate(TerminatorKind::Goto { target: on_no_match });
                } else {
                    self.test_pattern_or(alternatives, place, on_match, on_no_match, span)?;
                }
            }

            PatternKind::Ref { inner, .. } => {
                let deref_place = place.project(PlaceElem::Deref);
                self.test_pattern(inner, &deref_place, on_match, on_no_match, span)?;
            }

            PatternKind::Slice { prefix, slice, suffix } => {
                self.test_pattern_slice(prefix, slice, suffix, place, on_match, on_no_match, span)?;
            }

            PatternKind::Range { start, end, inclusive } => {
                // Range pattern: check if value is within range
                let value_operand = Operand::Copy(place.clone());

                // Generate range check: start <= value && value < end (or value <= end if inclusive)
                let mut checks = Vec::new();

                // Check lower bound: value >= start
                if let Some(start_pat) = start {
                    if let PatternKind::Literal(lit) = &start_pat.kind {
                        let start_const = lower_literal_to_constant(lit, &pattern.ty);
                        let cmp_result = self.new_temp(Type::bool(), span);
                        self.push_assign(
                            Place::local(cmp_result),
                            Rvalue::BinaryOp {
                                op: MirBinOp::Ge,
                                left: value_operand.clone(),
                                right: Operand::Constant(start_const),
                            },
                        );
                        checks.push(cmp_result);
                    }
                }

                // Check upper bound: value < end (or value <= end if inclusive)
                if let Some(end_pat) = end {
                    if let PatternKind::Literal(lit) = &end_pat.kind {
                        let end_const = lower_literal_to_constant(lit, &pattern.ty);
                        let cmp_result = self.new_temp(Type::bool(), span);
                        let cmp_op = if *inclusive { MirBinOp::Le } else { MirBinOp::Lt };
                        self.push_assign(
                            Place::local(cmp_result),
                            Rvalue::BinaryOp {
                                op: cmp_op,
                                left: value_operand,
                                right: Operand::Constant(end_const),
                            },
                        );
                        checks.push(cmp_result);
                    }
                }

                // Combine checks with AND
                if checks.is_empty() {
                    // No bounds - always matches (shouldn't happen in practice)
                    self.terminate(TerminatorKind::Goto { target: on_match });
                } else if checks.len() == 1 {
                    // Single check
                    self.terminate(TerminatorKind::SwitchInt {
                        discr: Operand::Copy(Place::local(checks[0])),
                        targets: SwitchTargets::new(vec![(1, on_match)], on_no_match),
                    });
                } else {
                    // Multiple checks - AND them together
                    let combined = self.new_temp(Type::bool(), span);
                    self.push_assign(
                        Place::local(combined),
                        Rvalue::BinaryOp {
                            op: MirBinOp::BitAnd,
                            left: Operand::Copy(Place::local(checks[0])),
                            right: Operand::Copy(Place::local(checks[1])),
                        },
                    );
                    self.terminate(TerminatorKind::SwitchInt {
                        discr: Operand::Copy(Place::local(combined)),
                        targets: SwitchTargets::new(vec![(1, on_match)], on_no_match),
                    });
                }
            }
        }
        Ok(())
    }

    fn test_pattern_tuple(
        &mut self,
        pats: &[Pattern],
        place: &Place,
        final_match: BasicBlockId,
        on_no_match: BasicBlockId,
        span: Span,
    ) -> Result<(), Vec<Diagnostic>> {
        if pats.is_empty() {
            self.terminate(TerminatorKind::Goto { target: final_match });
            return Ok(());
        }

        let mut next_block = final_match;
        for (i, pat) in pats.iter().enumerate().rev() {
            let field_place = place.project(PlaceElem::Field(i as u32));
            if i == 0 {
                self.test_pattern(pat, &field_place, next_block, on_no_match, span)?;
            } else {
                let test_block = self.builder.new_block();
                self.builder.switch_to(test_block);
                self.current_block = test_block;
                self.test_pattern(pat, &field_place, next_block, on_no_match, span)?;
                next_block = test_block;
            }
        }
        Ok(())
    }

    fn test_pattern_fields(
        &mut self,
        pats: &[Pattern],
        place: &Place,
        final_match: BasicBlockId,
        on_no_match: BasicBlockId,
        span: Span,
    ) -> Result<(), Vec<Diagnostic>> {
        if pats.is_empty() {
            self.terminate(TerminatorKind::Goto { target: final_match });
            return Ok(());
        }

        let mut current_target = final_match;
        for (i, pat) in pats.iter().enumerate().rev() {
            let field_place = place.project(PlaceElem::Field(i as u32));
            if i == 0 {
                self.test_pattern(pat, &field_place, current_target, on_no_match, span)?;
            } else {
                let next_block = self.builder.new_block();
                self.builder.switch_to(next_block);
                self.current_block = next_block;
                self.test_pattern(pat, &field_place, current_target, on_no_match, span)?;
                current_target = next_block;
            }
        }
        Ok(())
    }

    fn test_pattern_struct_fields(
        &mut self,
        fields: &[(u32, &Pattern)],
        place: &Place,
        final_match: BasicBlockId,
        on_no_match: BasicBlockId,
        span: Span,
    ) -> Result<(), Vec<Diagnostic>> {
        if fields.is_empty() {
            self.terminate(TerminatorKind::Goto { target: final_match });
            return Ok(());
        }

        let mut current_target = final_match;
        for (i, (field_idx, pat)) in fields.iter().enumerate().rev() {
            let field_place = place.project(PlaceElem::Field(*field_idx));
            if i == 0 {
                self.test_pattern(pat, &field_place, current_target, on_no_match, span)?;
            } else {
                let next_block = self.builder.new_block();
                self.builder.switch_to(next_block);
                self.current_block = next_block;
                self.test_pattern(pat, &field_place, current_target, on_no_match, span)?;
                current_target = next_block;
            }
        }
        Ok(())
    }

    fn test_pattern_or(
        &mut self,
        alternatives: &[Pattern],
        place: &Place,
        on_match: BasicBlockId,
        final_no_match: BasicBlockId,
        span: Span,
    ) -> Result<(), Vec<Diagnostic>> {
        for (i, alt) in alternatives.iter().enumerate() {
            let next_try = if i + 1 < alternatives.len() {
                self.builder.new_block()
            } else {
                final_no_match
            };
            self.test_pattern(alt, place, on_match, next_try, span)?;
            if i + 1 < alternatives.len() {
                self.builder.switch_to(next_try);
                self.current_block = next_try;
            }
        }
        Ok(())
    }

    fn test_pattern_slice(
        &mut self,
        prefix: &[Pattern],
        slice: &Option<Box<Pattern>>,
        suffix: &[Pattern],
        place: &Place,
        on_match: BasicBlockId,
        on_no_match: BasicBlockId,
        span: Span,
    ) -> Result<(), Vec<Diagnostic>> {
        let min_len = (prefix.len() + suffix.len()) as u64;

        let len_temp = self.new_temp(Type::usize(), span);
        self.push_assign(
            Place::local(len_temp),
            Rvalue::Len(place.clone()),
        );

        let len_ok = self.new_temp(Type::bool(), span);
        if slice.is_some() {
            self.push_assign(
                Place::local(len_ok),
                Rvalue::BinaryOp {
                    op: MirBinOp::Ge,
                    left: Operand::Copy(Place::local(len_temp)),
                    right: Operand::Constant(Constant::new(
                        Type::usize(),
                        ConstantKind::Int(min_len as i128),
                    )),
                },
            );
        } else {
            self.push_assign(
                Place::local(len_ok),
                Rvalue::BinaryOp {
                    op: MirBinOp::Eq,
                    left: Operand::Copy(Place::local(len_temp)),
                    right: Operand::Constant(Constant::new(
                        Type::usize(),
                        ConstantKind::Int(min_len as i128),
                    )),
                },
            );
        }

        let elements_block = self.builder.new_block();
        self.terminate(TerminatorKind::SwitchInt {
            discr: Operand::Copy(Place::local(len_ok)),
            targets: SwitchTargets::new(vec![(1, elements_block)], on_no_match),
        });

        self.builder.switch_to(elements_block);
        self.current_block = elements_block;

        if prefix.is_empty() && suffix.is_empty() {
            self.terminate(TerminatorKind::Goto { target: on_match });
        } else {
            let mut current_target = on_match;
            for (i, pat) in prefix.iter().enumerate().rev() {
                let idx_place = place.project(PlaceElem::ConstantIndex {
                    offset: i as u64,
                    min_length: min_len,
                    from_end: false,
                });
                if i == 0 && suffix.is_empty() {
                    self.test_pattern(pat, &idx_place, current_target, on_no_match, span)?;
                } else if i == 0 {
                    let suffix_block = self.builder.new_block();
                    self.test_pattern(pat, &idx_place, suffix_block, on_no_match, span)?;
                    self.builder.switch_to(suffix_block);
                    self.current_block = suffix_block;
                    current_target = on_match;
                } else {
                    let next_block = self.builder.new_block();
                    self.builder.switch_to(next_block);
                    self.current_block = next_block;
                    self.test_pattern(pat, &idx_place, current_target, on_no_match, span)?;
                    current_target = next_block;
                }
            }

            for (i, pat) in suffix.iter().enumerate().rev() {
                let offset_from_end = (suffix.len() - 1 - i) as u64;
                let idx_place = place.project(PlaceElem::ConstantIndex {
                    offset: offset_from_end,
                    min_length: min_len,
                    from_end: true,
                });
                if i == 0 {
                    self.test_pattern(pat, &idx_place, current_target, on_no_match, span)?;
                } else {
                    let next_block = self.builder.new_block();
                    self.builder.switch_to(next_block);
                    self.current_block = next_block;
                    self.test_pattern(pat, &idx_place, current_target, on_no_match, span)?;
                    current_target = next_block;
                }
            }
        }

        Ok(())
    }

    // NOTE: `lower_literal_to_constant` and `is_irrefutable_pattern` are now shared
    // utility functions in `util.rs`. Use `lower_literal_to_constant()` and
    // `is_irrefutable_pattern()` directly.

    /// Bind pattern variables to a place in a closure body.
    fn bind_pattern(&mut self, pattern: &Pattern, place: &Place) -> Result<(), Vec<Diagnostic>> {
        match &pattern.kind {
            PatternKind::Binding { local_id, mutable, subpattern } => {
                let mir_local = self.new_temp(pattern.ty.clone(), pattern.span);
                self.local_map.insert(*local_id, mir_local);

                // Check if this is a ref binding (pattern type is a reference)
                // In that case, we need to create a reference to the place instead of copying
                if pattern.ty.is_ref() {
                    // This is a ref binding (e.g., ref x or ref rest @ ..)
                    // Create a reference to the place
                    self.push_assign(
                        Place::local(mir_local),
                        Rvalue::Ref {
                            place: place.clone(),
                            mutable: *mutable,
                        },
                    );
                } else {
                    // Regular binding - copy the value
                    self.push_assign(
                        Place::local(mir_local),
                        Rvalue::Use(Operand::Copy(place.clone())),
                    );
                }

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
            PatternKind::Wildcard | PatternKind::Literal(_) | PatternKind::Range { .. } => {
                // Nothing to bind - these patterns only test values
            }
            PatternKind::Variant { variant_idx, fields, .. } => {
                // For enum variant patterns, first downcast to the variant,
                // then bind each field pattern.
                // The Downcast projection tells codegen we're inside a variant,
                // so Field projections will be offset by 1 for the discriminant tag.
                let variant_place = place.project(PlaceElem::Downcast(*variant_idx));
                for (i, field_pat) in fields.iter().enumerate() {
                    let field_place = variant_place.project(PlaceElem::Field(i as u32));
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

        // Generation validation for memory safety.
        // See: MEMORY_MODEL.md section 4 "Generational References"
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
            let local = self.body.get_local(hir_local)
                .expect("ICE: HIR local not found in closure body during MIR lowering");
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
}
