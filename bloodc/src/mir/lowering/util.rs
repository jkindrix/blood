//! Utility functions and traits for MIR lowering.
//!
//! This module provides shared utility functions and traits for HIR→MIR lowering.
//! These are used by both `FunctionLowering` and `ClosureLowering` to avoid code
//! duplication.
//!
//! ## Traits
//!
//! - [`ExprLowering`]: Core trait for expression lowering shared between functions and closures
//!
//! ## Functions
//!
//! - [`convert_binop`]: Convert AST binary operator to MIR binary operator
//! - [`convert_unop`]: Convert AST unary operator to MIR unary operator
//! - [`lower_literal_to_constant`]: Convert HIR literal to MIR constant
//! - [`is_irrefutable_pattern`]: Check if a pattern always matches

use std::collections::HashMap;

use crate::ast::{BinOp, UnaryOp};
use crate::diagnostics::Diagnostic;
use crate::hir::{
    self, Body, Crate as HirCrate, DefId, Expr, ExprKind, FieldExpr, LocalId,
    LiteralValue, LoopId, MatchArm, Pattern, PatternKind, RecordFieldExpr, Stmt, Type, TypeKind,
};
use crate::mir::body::MirBodyBuilder;
use crate::mir::static_evidence::{
    analyze_handler_state, analyze_handler_allocation_tier,
    analyze_inline_evidence_mode, InlineEvidenceContext,
};
use crate::mir::types::{
    BasicBlockId, Statement, StatementKind, Terminator, TerminatorKind,
    Place, PlaceElem, Operand, Rvalue, Constant, ConstantKind, AggregateKind, SwitchTargets,
    BinOp as MirBinOp, UnOp as MirUnOp,
};
use crate::span::Span;

// ============================================================================
// Operator Conversion
// ============================================================================

/// Convert HIR binary op to MIR binary op.
pub fn convert_binop(op: BinOp) -> MirBinOp {
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

/// Convert HIR unary op to MIR unary op.
///
/// Returns `None` for operators that require special handling:
/// - `Deref`: Creates a dereferenced place projection
/// - `Ref`/`RefMut`: Creates a reference to a place
///
/// These operators are handled directly in the lowering code.
pub fn convert_unop(op: UnaryOp) -> Option<MirUnOp> {
    match op {
        UnaryOp::Neg => Some(MirUnOp::Neg),
        UnaryOp::Not => Some(MirUnOp::Not),
        // These require special place-based handling
        UnaryOp::Deref | UnaryOp::Ref | UnaryOp::RefMut => None,
    }
}

// ============================================================================
// Literal Conversion
// ============================================================================

/// Convert a literal value to a MIR constant.
///
/// This is a pure utility function used during expression lowering
/// to convert HIR literal values into MIR constants.
pub fn lower_literal_to_constant(lit: &LiteralValue, ty: &Type) -> Constant {
    let kind = match lit {
        LiteralValue::Int(v) => ConstantKind::Int(*v),
        LiteralValue::Uint(v) => ConstantKind::Int(*v as i128),
        LiteralValue::Float(v) => ConstantKind::Float(*v),
        LiteralValue::Bool(v) => ConstantKind::Bool(*v),
        LiteralValue::Char(v) => ConstantKind::Char(*v),
        LiteralValue::String(v) => ConstantKind::String(v.clone()),
        LiteralValue::ByteString(v) => ConstantKind::ByteString(v.clone()),
    };
    Constant::new(ty.clone(), kind)
}

// ============================================================================
// Pattern Analysis
// ============================================================================

/// Check if a pattern is irrefutable (always matches).
///
/// An irrefutable pattern is one that will match any value of its type.
/// This includes:
/// - Wildcard patterns (`_`)
/// - Simple bindings (`x`)
/// - Tuple patterns with all irrefutable sub-patterns
/// - Reference patterns with irrefutable inner patterns
/// - Struct patterns with all irrefutable field patterns
/// - Slice patterns with a rest element (`..`)
///
/// Refutable patterns (which may not match) include:
/// - Literal patterns (`1`, `"hello"`)
/// - Or patterns (`a | b`)
/// - Variant patterns (`Some(x)`)
/// - Range patterns (`1..10`)
pub fn is_irrefutable_pattern(pattern: &Pattern) -> bool {
    match &pattern.kind {
        PatternKind::Wildcard => true,
        PatternKind::Binding { subpattern, .. } => {
            subpattern.as_ref().map_or(true, |p| is_irrefutable_pattern(p))
        }
        PatternKind::Tuple(pats) => pats.iter().all(is_irrefutable_pattern),
        PatternKind::Ref { inner, .. } => is_irrefutable_pattern(inner),
        // These patterns are refutable (may not match)
        PatternKind::Literal(_) => false,
        PatternKind::Or(_) => false,
        PatternKind::Variant { .. } => false,
        PatternKind::Range { .. } => false,
        // Struct patterns are irrefutable if all field patterns are irrefutable
        PatternKind::Struct { fields, .. } => {
            fields.iter().all(|f| is_irrefutable_pattern(&f.pattern))
        }
        // Slice patterns with a rest element (..) are irrefutable
        PatternKind::Slice { prefix, slice, suffix } => {
            slice.is_some() &&
            prefix.iter().all(is_irrefutable_pattern) &&
            suffix.iter().all(is_irrefutable_pattern)
        }
    }
}

// ============================================================================
// Loop Context
// ============================================================================

/// Unified loop context information.
///
/// This struct provides a unified view of loop context that works for both
/// `FunctionLowering` (which uses a Vec stack) and `ClosureLowering` (which uses
/// a HashMap for labeled loops).
#[derive(Debug, Clone)]
pub struct LoopContextInfo {
    /// Block to jump to on break.
    pub break_block: BasicBlockId,
    /// Block to jump to on continue.
    pub continue_block: BasicBlockId,
    /// Optional place to store break values.
    pub result_place: Option<Place>,
}

// ============================================================================
// Expression Lowering Trait
// ============================================================================

/// Core trait for expression lowering shared between functions and closures.
///
/// This trait provides:
/// - Abstract accessor methods for accessing lowering state
/// - Default implementations for all expression lowering methods
/// - Context-specific behavior through required methods
///
/// Both `FunctionLowering` and `ClosureLowering` implement this trait,
/// overriding only the context-specific methods.
pub trait ExprLowering {
    // ========================================================================
    // Required Accessor Methods
    // ========================================================================

    /// Get a mutable reference to the MIR body builder.
    fn builder_mut(&mut self) -> &mut MirBodyBuilder;

    /// Get an immutable reference to the MIR body builder.
    fn builder(&self) -> &MirBodyBuilder;

    /// Get the HIR body being lowered.
    fn body(&self) -> &Body;

    /// Get the HIR crate.
    fn hir(&self) -> &HirCrate;

    /// Get a mutable reference to the local map.
    fn local_map_mut(&mut self) -> &mut HashMap<LocalId, LocalId>;

    /// Get an immutable reference to the local map.
    fn local_map(&self) -> &HashMap<LocalId, LocalId>;

    /// Get a mutable reference to the current block.
    fn current_block_mut(&mut self) -> &mut BasicBlockId;

    /// Get the current block.
    fn current_block(&self) -> BasicBlockId;

    /// Get a mutable reference to the temp counter.
    fn temp_counter_mut(&mut self) -> &mut u32;

    /// Get a mutable reference to pending closures.
    fn pending_closures_mut(&mut self) -> &mut Vec<(hir::BodyId, DefId, Vec<(hir::Capture, Type)>)>;

    /// Get a mutable reference to the closure counter.
    fn closure_counter_mut(&mut self) -> &mut u32;

    // ========================================================================
    // Loop Context Methods (required - different implementations)
    // ========================================================================

    /// Push a loop context for the given label.
    fn push_loop_context(&mut self, label: Option<LoopId>, ctx: LoopContextInfo);

    /// Pop the current loop context.
    fn pop_loop_context(&mut self, label: Option<LoopId>);

    /// Get the loop context for break/continue (by label or current).
    fn get_loop_context(&self, label: Option<LoopId>) -> Option<LoopContextInfo>;

    // ========================================================================
    // Capture Methods (optional - only closures implement)
    // ========================================================================

    /// Check if a local is a captured variable and return its field index.
    /// Returns None for function lowering, Some(field_idx) for closure lowering.
    fn get_capture_field(&self, _local_id: LocalId) -> Option<usize> {
        None
    }

    /// Get the environment local for captured variables.
    /// Returns None for function lowering, Some(local_id) for closure lowering.
    fn get_env_local(&self) -> Option<LocalId> {
        None
    }

    // ========================================================================
    // Binary Expression Special Handling (optional)
    // ========================================================================

    /// Handle pipe operator specifically. Returns None if not supported.
    /// FunctionLowering handles this, ClosureLowering returns None.
    fn lower_pipe(
        &mut self,
        _arg: &Expr,
        _func: &Expr,
        _ty: &Type,
        _span: Span,
    ) -> Option<Result<Operand, Vec<Diagnostic>>> {
        None
    }

    // ========================================================================
    // Helper Methods (default implementations)
    // ========================================================================

    /// Generate a synthetic DefId for a closure.
    fn next_closure_def_id(&mut self) -> DefId {
        let id = *self.closure_counter_mut();
        *self.closure_counter_mut() += 1;
        DefId::new(0xFFFF_0000 + id)
    }

    /// Map a HIR local to a MIR local, creating if necessary.
    fn map_local(&mut self, hir_local: LocalId) -> LocalId {
        if let Some(&mir_local) = self.local_map().get(&hir_local) {
            mir_local
        } else {
            let local = self.body().get_local(hir_local)
                .expect("ICE: HIR local not found in body during MIR lowering");
            let ty = local.ty.clone();
            let span = local.span;
            let mir_id = self.builder_mut().new_temp(ty, span);
            self.local_map_mut().insert(hir_local, mir_id);
            mir_id
        }
    }

    /// Create a new temporary local.
    fn new_temp(&mut self, ty: Type, span: Span) -> LocalId {
        *self.temp_counter_mut() += 1;
        self.builder_mut().new_temp(ty, span)
    }

    /// Push a statement to the current block.
    fn push_stmt(&mut self, kind: StatementKind) {
        self.builder_mut().push_stmt(Statement::new(kind, Span::dummy()));
    }

    /// Push an assignment statement.
    fn push_assign(&mut self, place: Place, rvalue: Rvalue) {
        self.push_stmt(StatementKind::Assign(place, rvalue));
    }

    /// Terminate the current block.
    fn terminate(&mut self, kind: TerminatorKind) {
        self.builder_mut().terminate(Terminator::new(kind, Span::dummy()));
    }

    /// Check if the current block is terminated.
    fn is_terminated(&self) -> bool {
        self.builder().is_current_terminated()
    }

    // ========================================================================
    // Expression Lowering Methods (default implementations)
    // ========================================================================

    /// Lower an expression to an operand.
    fn lower_expr(&mut self, expr: &Expr) -> Result<Operand, Vec<Diagnostic>> {
        match &expr.kind {
            ExprKind::Literal(lit) => self.lower_literal(lit, &expr.ty),

            ExprKind::Local(local_id) => {
                // Check if this local is a captured variable
                if let Some(field_idx) = self.get_capture_field(*local_id) {
                    // Captured variable: access via field projection on environment
                    if let Some(env_local) = self.get_env_local() {
                        let env_place = Place::local(env_local);
                        let capture_place = env_place.project(PlaceElem::Field(field_idx as u32));
                        Ok(Operand::Copy(capture_place))
                    } else {
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
                let constant_kind = if let Some(item) = self.hir().get_item(*def_id) {
                    match &item.kind {
                        hir::ItemKind::Const { .. } => ConstantKind::ConstDef(*def_id),
                        hir::ItemKind::Static { .. } => ConstantKind::StaticDef(*def_id),
                        _ => ConstantKind::FnDef(*def_id),
                    }
                } else {
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

            ExprKind::Record { fields } => {
                self.lower_record(fields, &expr.ty, expr.span)
            }

            ExprKind::Default => {
                let temp = self.new_temp(expr.ty.clone(), expr.span);
                let rvalue = Rvalue::ZeroInit(expr.ty.clone());
                self.push_assign(Place::local(temp), rvalue);
                Ok(Operand::Copy(Place::local(temp)))
            }

            ExprKind::Error => {
                Err(vec![Diagnostic::error("lowering error expression".to_string(), expr.span)])
            }

            ExprKind::MethodCall { .. } => {
                Err(vec![Diagnostic::error(
                    "MIR lowering: method calls should be desugared before MIR lowering".to_string(),
                    expr.span,
                )])
            }

            ExprKind::MethodFamily { name, candidates } => {
                Err(vec![Diagnostic::error(
                    format!(
                        "cannot reference method family '{}' without a call (has {} overloads)",
                        name,
                        candidates.len()
                    ),
                    expr.span,
                )])
            }

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

    /// Lower a literal to an operand.
    fn lower_literal(&mut self, lit: &LiteralValue, ty: &Type) -> Result<Operand, Vec<Diagnostic>> {
        let kind = match lit {
            LiteralValue::Int(v) => ConstantKind::Int(*v),
            LiteralValue::Uint(v) => ConstantKind::Uint(*v),
            LiteralValue::Float(v) => ConstantKind::Float(*v),
            LiteralValue::Bool(v) => ConstantKind::Bool(*v),
            LiteralValue::Char(v) => ConstantKind::Char(*v),
            LiteralValue::String(v) => ConstantKind::String(v.clone()),
            LiteralValue::ByteString(v) => ConstantKind::ByteString(v.clone()),
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
        // Check if this is a pipe operator that should be handled specially
        if matches!(op, BinOp::Pipe) {
            if let Some(result) = self.lower_pipe(left, right, ty, span) {
                return result;
            }
            // Fall through to regular binary op if pipe is not supported
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
                let temp = self.new_temp(ty.clone(), span);
                if let Some(place) = operand_val.place() {
                    let deref_place = place.project(PlaceElem::Deref);
                    self.push_assign(Place::local(temp), Rvalue::Use(Operand::Copy(deref_place)));
                }
                return Ok(Operand::Copy(Place::local(temp)));
            }
            UnaryOp::Ref | UnaryOp::RefMut => {
                let mutable = matches!(op, UnaryOp::RefMut);
                let place = if let Some(p) = operand_val.place() {
                    p.clone()
                } else {
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

        let next_block = self.builder_mut().new_block();

        self.terminate(TerminatorKind::Call {
            func,
            args: arg_ops,
            destination: dest_place.clone(),
            target: Some(next_block),
            unwind: None,
        });

        self.builder_mut().switch_to(next_block);
        *self.current_block_mut() = next_block;

        Ok(Operand::Copy(dest_place))
    }

    /// Lower a block expression.
    fn lower_block(
        &mut self,
        stmts: &[Stmt],
        tail: Option<&Expr>,
        _ty: &Type,
        _span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        for stmt in stmts {
            self.lower_stmt(stmt)?;
        }

        if let Some(expr) = tail {
            self.lower_expr(expr)
        } else {
            Ok(Operand::Constant(Constant::unit()))
        }
    }

    /// Lower a statement.
    ///
    /// Note: This is a simplified default implementation. Implementors may override
    /// this for more complex handling (e.g., tuple destructuring patterns).
    fn lower_stmt(&mut self, stmt: &Stmt) -> Result<(), Vec<Diagnostic>> {
        match stmt {
            Stmt::Let { local_id, init } => {
                // Get or create the MIR local
                let mir_local = self.map_local(*local_id);

                // Storage live
                self.push_stmt(StatementKind::StorageLive(mir_local));

                // Initialize if there's an init expression
                if let Some(init) = init {
                    let init_val = self.lower_expr(init)?;
                    self.push_assign(Place::local(mir_local), Rvalue::Use(init_val));
                }
            }
            Stmt::Expr(expr) => {
                let _ = self.lower_expr(expr)?;
            }
            Stmt::Item(_) => {
                // Item statements are handled elsewhere
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

        let then_block = self.builder_mut().new_block();
        let else_block = self.builder_mut().new_block();
        let join_block = self.builder_mut().new_block();

        let result = self.new_temp(ty.clone(), span);

        self.terminate(TerminatorKind::SwitchInt {
            discr: cond,
            targets: SwitchTargets::new(vec![(1, then_block)], else_block),
        });

        // Then branch
        self.builder_mut().switch_to(then_block);
        *self.current_block_mut() = then_block;
        let then_val = self.lower_expr(then_branch)?;
        self.push_assign(Place::local(result), Rvalue::Use(then_val));
        if !self.is_terminated() {
            self.terminate(TerminatorKind::Goto { target: join_block });
        }

        // Else branch
        self.builder_mut().switch_to(else_block);
        *self.current_block_mut() = else_block;
        if let Some(else_expr) = else_branch {
            let else_val = self.lower_expr(else_expr)?;
            self.push_assign(Place::local(result), Rvalue::Use(else_val));
        } else {
            self.push_assign(Place::local(result), Rvalue::Use(Operand::Constant(Constant::unit())));
        }
        if !self.is_terminated() {
            self.terminate(TerminatorKind::Goto { target: join_block });
        }

        // Join
        self.builder_mut().switch_to(join_block);
        *self.current_block_mut() = join_block;

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
        let loop_block = self.builder_mut().new_block();
        let exit_block = self.builder_mut().new_block();

        let result = self.new_temp(ty.clone(), span);

        // Push loop context
        self.push_loop_context(label, LoopContextInfo {
            break_block: exit_block,
            continue_block: loop_block,
            result_place: Some(Place::local(result)),
        });

        // Jump to loop
        self.terminate(TerminatorKind::Goto { target: loop_block });

        // Loop body
        self.builder_mut().switch_to(loop_block);
        *self.current_block_mut() = loop_block;
        let _ = self.lower_expr(body)?;

        // Loop back
        if !self.is_terminated() {
            self.terminate(TerminatorKind::Goto { target: loop_block });
        }

        // Pop loop context
        self.pop_loop_context(label);

        // Continue at exit
        self.builder_mut().switch_to(exit_block);
        *self.current_block_mut() = exit_block;

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
        let cond_block = self.builder_mut().new_block();
        let body_block = self.builder_mut().new_block();
        let exit_block = self.builder_mut().new_block();

        let result = self.new_temp(ty.clone(), span);

        // Push loop context
        self.push_loop_context(label, LoopContextInfo {
            break_block: exit_block,
            continue_block: cond_block,
            result_place: Some(Place::local(result)),
        });

        // Jump to condition
        self.terminate(TerminatorKind::Goto { target: cond_block });

        // Condition block
        self.builder_mut().switch_to(cond_block);
        *self.current_block_mut() = cond_block;
        let cond = self.lower_expr(condition)?;
        self.terminate(TerminatorKind::SwitchInt {
            discr: cond,
            targets: SwitchTargets::new(vec![(1, body_block)], exit_block),
        });

        // Body block
        self.builder_mut().switch_to(body_block);
        *self.current_block_mut() = body_block;
        let _ = self.lower_expr(body)?;
        if !self.is_terminated() {
            self.terminate(TerminatorKind::Goto { target: cond_block });
        }

        // Pop loop context
        self.pop_loop_context(label);

        // Exit
        self.builder_mut().switch_to(exit_block);
        *self.current_block_mut() = exit_block;

        Ok(Operand::Copy(Place::local(result)))
    }

    /// Lower a break expression.
    fn lower_break(
        &mut self,
        label: Option<LoopId>,
        value: Option<&Expr>,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let ctx = self.get_loop_context(label);

        if let Some(ctx) = ctx {
            if let Some(value) = value {
                let val = self.lower_expr(value)?;
                if let Some(rp) = &ctx.result_place {
                    self.push_assign(rp.clone(), Rvalue::Use(val));
                }
            }
            self.terminate(TerminatorKind::Goto { target: ctx.break_block });
        } else {
            return Err(vec![Diagnostic::error("break outside of loop".to_string(), span)]);
        }

        // Unreachable after break
        let next = self.builder_mut().new_block();
        self.builder_mut().switch_to(next);
        *self.current_block_mut() = next;

        Ok(Operand::Constant(Constant::unit()))
    }

    /// Lower a continue expression.
    fn lower_continue(
        &mut self,
        label: Option<LoopId>,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let ctx = self.get_loop_context(label);

        if let Some(ctx) = ctx {
            self.terminate(TerminatorKind::Goto { target: ctx.continue_block });
        } else {
            return Err(vec![Diagnostic::error("continue outside of loop".to_string(), span)]);
        }

        let next = self.builder_mut().new_block();
        self.builder_mut().switch_to(next);
        *self.current_block_mut() = next;

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

        let next = self.builder_mut().new_block();
        self.builder_mut().switch_to(next);
        *self.current_block_mut() = next;

        Ok(Operand::Constant(Constant::unit()))
    }

    /// Lower an assignment expression.
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
        let mut operands = Vec::with_capacity(elems.len());
        for elem in elems {
            operands.push(self.lower_expr(elem)?);
        }

        // Get element type from array type or first element
        let elem_ty = match ty.kind() {
            TypeKind::Array { element, .. } => element.clone(),
            _ => {
                if let Some(first) = elems.first() {
                    first.ty.clone()
                } else {
                    Type::unit()
                }
            }
        };

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

    /// Lower a field access expression.
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

    /// Lower an index expression.
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

        let index_place = base_place.project(PlaceElem::Index(index_local));
        let temp = self.new_temp(ty.clone(), span);
        self.push_assign(Place::local(temp), Rvalue::Use(Operand::Copy(index_place)));
        Ok(Operand::Copy(Place::local(temp)))
    }

    /// Lower a borrow expression.
    fn lower_borrow(
        &mut self,
        inner: &Expr,
        mutable: bool,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let place = self.lower_place(inner)?;
        let temp = self.new_temp(ty.clone(), span);
        self.push_assign(Place::local(temp), Rvalue::Ref { place, mutable });
        Ok(Operand::Copy(Place::local(temp)))
    }

    /// Lower a dereference expression with generation validation.
    ///
    /// This emits a generation validation check before the dereference to detect
    /// use-after-free errors at runtime. The generation check compares the
    /// expected generation (captured when the reference was created) against
    /// the current generation of the memory slot.
    fn lower_deref(
        &mut self,
        inner: &Expr,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let inner_val = self.lower_expr(inner)?;
        let inner_place = if let Some(p) = inner_val.place() {
            p.clone()
        } else {
            let temp = self.new_temp(inner.ty.clone(), span);
            self.push_assign(Place::local(temp), Rvalue::Use(inner_val));
            Place::local(temp)
        };

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
        inner: &Expr,
        target_ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let inner_val = self.lower_expr(inner)?;
        let temp = self.new_temp(target_ty.clone(), span);
        self.push_assign(
            Place::local(temp),
            Rvalue::Cast {
                operand: inner_val,
                target_ty: target_ty.clone(),
            },
        );
        Ok(Operand::Copy(Place::local(temp)))
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

    /// Lower a variant construction expression.
    fn lower_variant(
        &mut self,
        def_id: DefId,
        variant_idx: u32,
        fields: &[Expr],
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let mut operands = Vec::with_capacity(fields.len());
        for field in fields {
            operands.push(self.lower_expr(field)?);
        }

        // Extract type arguments from the ADT type
        let type_args = if let TypeKind::Adt { args, .. } = ty.kind() {
            args.clone()
        } else {
            vec![]
        };

        let temp = self.new_temp(ty.clone(), span);
        self.push_assign(
            Place::local(temp),
            Rvalue::Aggregate {
                kind: AggregateKind::Adt {
                    def_id,
                    variant_idx: Some(variant_idx),
                    type_args,
                },
                operands,
            },
        );

        Ok(Operand::Copy(Place::local(temp)))
    }

    /// Lower an address-of expression.
    fn lower_addr_of(
        &mut self,
        inner: &Expr,
        mutable: bool,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let place = self.lower_place(inner)?;
        let temp = self.new_temp(ty.clone(), span);
        self.push_assign(
            Place::local(temp),
            Rvalue::AddressOf { place, mutable },
        );
        Ok(Operand::Copy(Place::local(temp)))
    }

    /// Lower a let expression.
    fn lower_let(
        &mut self,
        pattern: &Pattern,
        init: &Expr,
        _ty: &Type,
        _span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let init_val = self.lower_expr(init)?;
        let temp = self.new_temp(init.ty.clone(), init.span);
        self.push_assign(Place::local(temp), Rvalue::Use(init_val));
        self.bind_pattern(pattern, &Place::local(temp))?;
        Ok(Operand::Constant(Constant::unit()))
    }

    /// Lower a struct construction expression.
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
            vec![]
        };

        let temp = self.new_temp(ty.clone(), span);
        self.push_assign(
            Place::local(temp),
            Rvalue::Aggregate {
                kind: AggregateKind::Adt {
                    def_id,
                    variant_idx: None,
                    type_args,
                },
                operands,
            },
        );

        Ok(Operand::Copy(Place::local(temp)))
    }

    /// Lower a record (anonymous struct) expression.
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
                kind: AggregateKind::Tuple, // Records are structurally tuples
                operands,
            },
        );

        Ok(Operand::Copy(Place::local(temp)))
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

    /// Lower a perform expression (effect operation).
    fn lower_perform(
        &mut self,
        effect_id: DefId,
        op_index: u32,
        args: &[Expr],
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        // Lower all argument expressions
        let mut arg_ops = Vec::with_capacity(args.len());
        for arg in args {
            arg_ops.push(self.lower_expr(arg)?);
        }

        // Create destination for the result
        let dest = self.new_temp(ty.clone(), span);
        let dest_place = Place::local(dest);

        // Create continuation block
        let next_block = self.builder_mut().new_block();

        // Generate perform terminator
        // Note: is_tail_resumptive defaults to false, meaning this might suspend
        self.terminate(TerminatorKind::Perform {
            effect_id,
            op_index,
            args: arg_ops,
            destination: dest_place.clone(),
            target: next_block,
            is_tail_resumptive: false,
        });

        // Continue in the new block
        self.builder_mut().switch_to(next_block);
        *self.current_block_mut() = next_block;

        Ok(Operand::Copy(dest_place))
    }

    /// Lower a resume expression.
    fn lower_resume(
        &mut self,
        value: Option<&Expr>,
        _span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let resume_value = if let Some(v) = value {
            self.lower_expr(v)?
        } else {
            Operand::Constant(Constant::unit())
        };

        // Create unreachable block (resume doesn't return normally)
        let unreachable_block = self.builder_mut().new_block();

        self.terminate(TerminatorKind::Resume { value: Some(resume_value) });

        self.builder_mut().switch_to(unreachable_block);
        *self.current_block_mut() = unreachable_block;

        // Resume never returns, so we return a never-type placeholder
        Ok(Operand::Constant(Constant::unit()))
    }

    /// Lower a handle expression (effect handler).
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
        // EFF-OPT-003/004: Inline evidence mode
        // Note: The trait default implementation uses conservative Vector mode
        // because it doesn't have access to handler_depth. The struct-level
        // implementations (FunctionLowering, ClosureLowering) override this
        // to use the proper inline mode based on their handler_depth field.
        let inline_context = InlineEvidenceContext::new();
        let inline_mode = analyze_inline_evidence_mode(body, &inline_context, allocation_tier);

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
            inline_mode,
        });

        // Step 3: Lower the body expression with the handler installed
        let body_result = self.lower_expr(body)?;

        // Step 4: Pop the handler from the evidence vector
        self.push_stmt(StatementKind::PopHandler);

        // Step 5: Call the return clause to transform the body result
        // The return clause function is {handler_name}_return (content-based naming)
        let handler_name = self.hir().get_item(handler_id)
            .map(|item| item.name.clone())
            .unwrap_or_else(|| format!("unknown_handler_{}", handler_id.index));
        let dest = self.new_temp(ty.clone(), span);
        let dest_place = Place::local(dest);

        self.push_stmt(StatementKind::CallReturnClause {
            handler_id,
            handler_name,
            state_place,
            body_result,
            destination: dest_place.clone(),
        });

        Ok(Operand::Copy(dest_place))
    }

    /// Lower an expression to a place (for assignment targets).
    fn lower_place(&mut self, expr: &Expr) -> Result<Place, Vec<Diagnostic>> {
        match &expr.kind {
            ExprKind::Local(local_id) => {
                // Check for captured variable
                if let Some(field_idx) = self.get_capture_field(*local_id) {
                    if let Some(env_local) = self.get_env_local() {
                        let env_place = Place::local(env_local);
                        Ok(env_place.project(PlaceElem::Field(field_idx as u32)))
                    } else {
                        Err(vec![Diagnostic::error(
                            "internal error: captured variable but no environment".to_string(),
                            expr.span,
                        )])
                    }
                } else {
                    let mir_local = self.map_local(*local_id);
                    Ok(Place::local(mir_local))
                }
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

    /// Lower a match expression.
    fn lower_match(
        &mut self,
        scrutinee: &Expr,
        arms: &[MatchArm],
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let scrutinee_val = self.lower_expr(scrutinee)?;

        // Materialize scrutinee to a place
        let scrutinee_place = if let Some(place) = scrutinee_val.place() {
            place.clone()
        } else {
            let temp = self.new_temp(scrutinee.ty.clone(), scrutinee.span);
            self.push_assign(Place::local(temp), Rvalue::Use(scrutinee_val));
            Place::local(temp)
        };

        // Create result place
        let result = self.new_temp(ty.clone(), span);
        let join_block = self.builder_mut().new_block();

        // Create blocks for each arm
        let mut arm_blocks: Vec<BasicBlockId> = Vec::with_capacity(arms.len());
        for _ in arms {
            arm_blocks.push(self.builder_mut().new_block());
        }
        let fallthrough_block = self.builder_mut().new_block();

        // Lower pattern tests
        let mut current_test_block = self.current_block();
        for (i, arm) in arms.iter().enumerate() {
            let arm_block = arm_blocks[i];
            let next_test_block = if i + 1 < arms.len() {
                self.builder_mut().new_block()
            } else {
                fallthrough_block
            };

            self.builder_mut().switch_to(current_test_block);
            *self.current_block_mut() = current_test_block;

            let matches = self.test_pattern(&arm.pattern, &scrutinee_place)?;

            self.terminate(TerminatorKind::SwitchInt {
                discr: matches,
                targets: SwitchTargets::new(vec![(1, arm_block)], next_test_block),
            });

            current_test_block = next_test_block;
        }

        // Fallthrough unreachable
        self.builder_mut().switch_to(fallthrough_block);
        *self.current_block_mut() = fallthrough_block;
        self.terminate(TerminatorKind::Unreachable);

        // Lower arm bodies
        for (i, arm) in arms.iter().enumerate() {
            self.builder_mut().switch_to(arm_blocks[i]);
            *self.current_block_mut() = arm_blocks[i];

            self.bind_pattern(&arm.pattern, &scrutinee_place)?;

            // Handle guard
            if let Some(guard) = &arm.guard {
                let guard_pass = self.builder_mut().new_block();
                let guard_fail = if i + 1 < arms.len() {
                    arm_blocks[i + 1]
                } else {
                    fallthrough_block
                };

                let guard_result = self.lower_expr(guard)?;
                self.terminate(TerminatorKind::SwitchInt {
                    discr: guard_result,
                    targets: SwitchTargets::new(vec![(1, guard_pass)], guard_fail),
                });

                self.builder_mut().switch_to(guard_pass);
                *self.current_block_mut() = guard_pass;
            }

            let arm_val = self.lower_expr(&arm.body)?;
            self.push_assign(Place::local(result), Rvalue::Use(arm_val));
            self.terminate(TerminatorKind::Goto { target: join_block });
        }

        self.builder_mut().switch_to(join_block);
        *self.current_block_mut() = join_block;

        Ok(Operand::Copy(Place::local(result)))
    }

    /// Test a pattern against a place, returning a boolean operand.
    fn test_pattern(&mut self, pattern: &Pattern, place: &Place) -> Result<Operand, Vec<Diagnostic>> {
        match &pattern.kind {
            PatternKind::Wildcard => {
                Ok(Operand::Constant(Constant::new(Type::bool(), ConstantKind::Bool(true))))
            }
            PatternKind::Binding { subpattern, .. } => {
                if let Some(sub) = subpattern {
                    self.test_pattern(sub, place)
                } else {
                    Ok(Operand::Constant(Constant::new(Type::bool(), ConstantKind::Bool(true))))
                }
            }
            PatternKind::Literal(lit) => {
                let lit_const = lower_literal_to_constant(lit, &pattern.ty);
                let lit_op = Operand::Constant(lit_const);
                let place_val = Operand::Copy(place.clone());

                let result = self.new_temp(Type::bool(), pattern.span);
                self.push_assign(
                    Place::local(result),
                    Rvalue::BinaryOp {
                        op: MirBinOp::Eq,
                        left: place_val,
                        right: lit_op,
                    },
                );
                Ok(Operand::Copy(Place::local(result)))
            }
            PatternKind::Tuple(pats) => {
                self.test_pattern_tuple(pats, place, pattern.span)
            }
            PatternKind::Variant { variant_idx, fields, .. } => {
                self.test_pattern_variant(*variant_idx, fields, place, pattern.span)
            }
            PatternKind::Struct { fields, .. } => {
                self.test_pattern_struct(fields, place, pattern.span)
            }
            PatternKind::Or(alts) => {
                self.test_pattern_or(alts, place, pattern.span)
            }
            PatternKind::Slice { prefix, slice, suffix } => {
                self.test_pattern_slice(prefix, slice.as_deref(), suffix, place, pattern.span)
            }
            PatternKind::Range { start, end, inclusive } => {
                self.test_pattern_range(start.as_ref(), end.as_ref(), *inclusive, place, &pattern.ty, pattern.span)
            }
            PatternKind::Ref { inner, .. } => {
                let deref_place = place.project(PlaceElem::Deref);
                self.test_pattern(inner, &deref_place)
            }
        }
    }

    /// Test a tuple pattern.
    fn test_pattern_tuple(
        &mut self,
        pats: &[Pattern],
        place: &Place,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        if pats.is_empty() {
            return Ok(Operand::Constant(Constant::new(Type::bool(), ConstantKind::Bool(true))));
        }

        let mut result = self.test_pattern(&pats[0], &place.project(PlaceElem::Field(0)))?;

        for (i, pat) in pats.iter().enumerate().skip(1) {
            let field_place = place.project(PlaceElem::Field(i as u32));
            let field_result = self.test_pattern(pat, &field_place)?;

            let combined = self.new_temp(Type::bool(), span);
            self.push_assign(
                Place::local(combined),
                Rvalue::BinaryOp {
                    op: MirBinOp::BitAnd,
                    left: result,
                    right: field_result,
                },
            );
            result = Operand::Copy(Place::local(combined));
        }

        Ok(result)
    }

    /// Test a variant pattern.
    fn test_pattern_variant(
        &mut self,
        variant_idx: u32,
        fields: &[Pattern],
        place: &Place,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        // First check discriminant using Rvalue::Discriminant
        let discr_temp = self.new_temp(Type::u32(), span);
        self.push_assign(Place::local(discr_temp), Rvalue::Discriminant(place.clone()));

        let expected = Operand::Constant(Constant::new(Type::u32(), ConstantKind::Int(variant_idx as i128)));

        let discr_matches = self.new_temp(Type::bool(), span);
        self.push_assign(
            Place::local(discr_matches),
            Rvalue::BinaryOp {
                op: MirBinOp::Eq,
                left: Operand::Copy(Place::local(discr_temp)),
                right: expected,
            },
        );

        if fields.is_empty() {
            return Ok(Operand::Copy(Place::local(discr_matches)));
        }

        // Then check fields
        let variant_place = place.project(PlaceElem::Downcast(variant_idx));
        let mut result = Operand::Copy(Place::local(discr_matches));

        for (i, field_pat) in fields.iter().enumerate() {
            let field_place = variant_place.project(PlaceElem::Field(i as u32));
            let field_result = self.test_pattern(field_pat, &field_place)?;

            let combined = self.new_temp(Type::bool(), span);
            self.push_assign(
                Place::local(combined),
                Rvalue::BinaryOp {
                    op: MirBinOp::BitAnd,
                    left: result,
                    right: field_result,
                },
            );
            result = Operand::Copy(Place::local(combined));
        }

        Ok(result)
    }

    /// Test a struct pattern.
    fn test_pattern_struct(
        &mut self,
        fields: &[hir::FieldPattern],
        place: &Place,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        if fields.is_empty() {
            return Ok(Operand::Constant(Constant::new(Type::bool(), ConstantKind::Bool(true))));
        }

        let mut result = self.test_pattern(
            &fields[0].pattern,
            &place.project(PlaceElem::Field(fields[0].field_idx)),
        )?;

        for field in fields.iter().skip(1) {
            let field_place = place.project(PlaceElem::Field(field.field_idx));
            let field_result = self.test_pattern(&field.pattern, &field_place)?;

            let combined = self.new_temp(Type::bool(), span);
            self.push_assign(
                Place::local(combined),
                Rvalue::BinaryOp {
                    op: MirBinOp::BitAnd,
                    left: result,
                    right: field_result,
                },
            );
            result = Operand::Copy(Place::local(combined));
        }

        Ok(result)
    }

    /// Test an or-pattern.
    fn test_pattern_or(
        &mut self,
        alts: &[Pattern],
        place: &Place,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        if alts.is_empty() {
            return Ok(Operand::Constant(Constant::new(Type::bool(), ConstantKind::Bool(false))));
        }

        let mut result = self.test_pattern(&alts[0], place)?;

        for alt in alts.iter().skip(1) {
            let alt_result = self.test_pattern(alt, place)?;

            let combined = self.new_temp(Type::bool(), span);
            self.push_assign(
                Place::local(combined),
                Rvalue::BinaryOp {
                    op: MirBinOp::BitOr,
                    left: result,
                    right: alt_result,
                },
            );
            result = Operand::Copy(Place::local(combined));
        }

        Ok(result)
    }

    /// Test a slice pattern.
    fn test_pattern_slice(
        &mut self,
        prefix: &[Pattern],
        slice: Option<&Pattern>,
        suffix: &[Pattern],
        place: &Place,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let min_len = prefix.len() + suffix.len();

        // Get slice length using Rvalue::Len
        let len_temp = self.new_temp(Type::u64(), span);
        self.push_assign(Place::local(len_temp), Rvalue::Len(place.clone()));

        // Check minimum length
        let min_len_const = Operand::Constant(Constant::new(Type::u64(), ConstantKind::Int(min_len as i128)));

        let len_check = if slice.is_some() {
            // With rest pattern: length >= min_len
            let check = self.new_temp(Type::bool(), span);
            self.push_assign(
                Place::local(check),
                Rvalue::BinaryOp {
                    op: MirBinOp::Ge,
                    left: Operand::Copy(Place::local(len_temp)),
                    right: min_len_const,
                },
            );
            Operand::Copy(Place::local(check))
        } else {
            // Without rest: length == min_len
            let check = self.new_temp(Type::bool(), span);
            self.push_assign(
                Place::local(check),
                Rvalue::BinaryOp {
                    op: MirBinOp::Eq,
                    left: Operand::Copy(Place::local(len_temp)),
                    right: min_len_const,
                },
            );
            Operand::Copy(Place::local(check))
        };

        let mut result = len_check;

        // Check prefix patterns
        for (i, pat) in prefix.iter().enumerate() {
            let elem_place = place.project(PlaceElem::ConstantIndex {
                offset: i as u64,
                min_length: min_len as u64,
                from_end: false,
            });
            let pat_result = self.test_pattern(pat, &elem_place)?;

            let combined = self.new_temp(Type::bool(), span);
            self.push_assign(
                Place::local(combined),
                Rvalue::BinaryOp {
                    op: MirBinOp::BitAnd,
                    left: result,
                    right: pat_result,
                },
            );
            result = Operand::Copy(Place::local(combined));
        }

        // Check suffix patterns
        for (i, pat) in suffix.iter().enumerate() {
            let offset_from_end = (suffix.len() - 1 - i) as u64;
            let elem_place = place.project(PlaceElem::ConstantIndex {
                offset: offset_from_end,
                min_length: min_len as u64,
                from_end: true,
            });
            let pat_result = self.test_pattern(pat, &elem_place)?;

            let combined = self.new_temp(Type::bool(), span);
            self.push_assign(
                Place::local(combined),
                Rvalue::BinaryOp {
                    op: MirBinOp::BitAnd,
                    left: result,
                    right: pat_result,
                },
            );
            result = Operand::Copy(Place::local(combined));
        }

        // Check rest pattern if present
        if let Some(rest_pat) = slice {
            let subslice_place = place.project(PlaceElem::Subslice {
                from: prefix.len() as u64,
                to: suffix.len() as u64,
                from_end: true,
            });
            let rest_result = self.test_pattern(rest_pat, &subslice_place)?;

            let combined = self.new_temp(Type::bool(), span);
            self.push_assign(
                Place::local(combined),
                Rvalue::BinaryOp {
                    op: MirBinOp::BitAnd,
                    left: result,
                    right: rest_result,
                },
            );
            result = Operand::Copy(Place::local(combined));
        }

        Ok(result)
    }

    /// Test a range pattern.
    fn test_pattern_range(
        &mut self,
        start: Option<&Box<Pattern>>,
        end: Option<&Box<Pattern>>,
        inclusive: bool,
        place: &Place,
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let place_val = Operand::Copy(place.clone());

        let mut result: Option<Operand> = None;

        // Check start bound: value >= start
        if let Some(start_pat) = start {
            if let PatternKind::Literal(lit) = &start_pat.kind {
                let start_const = lower_literal_to_constant(lit, ty);
                let start_check = self.new_temp(Type::bool(), span);
                self.push_assign(
                    Place::local(start_check),
                    Rvalue::BinaryOp {
                        op: MirBinOp::Ge,
                        left: place_val.clone(),
                        right: Operand::Constant(start_const),
                    },
                );
                result = Some(Operand::Copy(Place::local(start_check)));
            }
        }

        // Check end bound: value < end or value <= end
        if let Some(end_pat) = end {
            if let PatternKind::Literal(lit) = &end_pat.kind {
                let end_const = lower_literal_to_constant(lit, ty);
                let end_op = if inclusive { MirBinOp::Le } else { MirBinOp::Lt };
                let end_check = self.new_temp(Type::bool(), span);
                self.push_assign(
                    Place::local(end_check),
                    Rvalue::BinaryOp {
                        op: end_op,
                        left: place_val,
                        right: Operand::Constant(end_const),
                    },
                );

                if let Some(prev) = result {
                    let combined = self.new_temp(Type::bool(), span);
                    self.push_assign(
                        Place::local(combined),
                        Rvalue::BinaryOp {
                            op: MirBinOp::BitAnd,
                            left: prev,
                            right: Operand::Copy(Place::local(end_check)),
                        },
                    );
                    result = Some(Operand::Copy(Place::local(combined)));
                } else {
                    result = Some(Operand::Copy(Place::local(end_check)));
                }
            }
        }

        Ok(result.unwrap_or_else(|| Operand::Constant(Constant::new(Type::bool(), ConstantKind::Bool(true)))))
    }

    /// Bind pattern variables to a place.
    fn bind_pattern(&mut self, pattern: &Pattern, place: &Place) -> Result<(), Vec<Diagnostic>> {
        match &pattern.kind {
            PatternKind::Wildcard => {
                // Nothing to bind
            }
            PatternKind::Binding { local_id, mutable, subpattern } => {
                let mir_local = self.map_local(*local_id);

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

                if let Some(sub) = subpattern {
                    self.bind_pattern(sub, &Place::local(mir_local))?;
                }
            }
            PatternKind::Literal(_) => {
                // Literals don't bind
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
            PatternKind::Variant { variant_idx, fields, .. } => {
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
                if let Some(first_alt) = alternatives.first() {
                    self.bind_pattern(first_alt, place)?;
                }
            }
            PatternKind::Ref { inner, .. } => {
                let deref_place = place.project(PlaceElem::Deref);
                self.bind_pattern(inner, &deref_place)?;
            }
            PatternKind::Range { .. } => {
                // Range patterns don't bind variables
            }
        }
        Ok(())
    }

    // ========================================================================
    // Control-Flow Pattern Testing Methods
    // ========================================================================
    //
    // These methods emit control flow directly based on pattern matching.
    // Unlike the Operand-returning test_pattern methods above, these take
    // on_match/on_no_match blocks and emit branches directly.

    /// Test a pattern against a place, emitting control flow.
    ///
    /// On successful match, control flows to `on_match`.
    /// On failure, control flows to `on_no_match`.
    fn test_pattern_cf(
        &mut self,
        pattern: &Pattern,
        place: &Place,
        on_match: BasicBlockId,
        on_no_match: BasicBlockId,
        span: Span,
    ) -> Result<(), Vec<Diagnostic>> {
        match &pattern.kind {
            PatternKind::Wildcard => {
                // Always matches
                self.terminate(TerminatorKind::Goto { target: on_match });
            }

            PatternKind::Binding { subpattern, .. } => {
                // Binding always succeeds; test subpattern if present
                if let Some(subpat) = subpattern {
                    self.test_pattern_cf(subpat, place, on_match, on_no_match, span)?;
                } else {
                    self.terminate(TerminatorKind::Goto { target: on_match });
                }
            }

            PatternKind::Literal(lit) => {
                // Compare the value with the literal
                let lit_const = lower_literal_to_constant(lit, &pattern.ty);
                let lit_operand = Operand::Constant(lit_const);
                let value_operand = Operand::Copy(place.clone());

                // Create comparison
                let cmp_result = self.new_temp(Type::bool(), span);
                self.push_assign(
                    Place::local(cmp_result),
                    Rvalue::BinaryOp {
                        op: MirBinOp::Eq,
                        left: value_operand,
                        right: lit_operand,
                    },
                );

                // Branch based on comparison
                self.terminate(TerminatorKind::SwitchInt {
                    discr: Operand::Copy(Place::local(cmp_result)),
                    targets: SwitchTargets::new(vec![(1, on_match)], on_no_match),
                });
            }

            PatternKind::Variant { variant_idx, fields, .. } => {
                // Get discriminant and compare with expected variant
                let discr_temp = self.new_temp(Type::i32(), span);
                self.push_assign(
                    Place::local(discr_temp),
                    Rvalue::Discriminant(place.clone()),
                );

                if fields.is_empty() {
                    // No fields to test, just check discriminant
                    self.terminate(TerminatorKind::SwitchInt {
                        discr: Operand::Copy(Place::local(discr_temp)),
                        targets: SwitchTargets::new(
                            vec![(*variant_idx as u128, on_match)],
                            on_no_match,
                        ),
                    });
                } else {
                    // Need to check discriminant first, then test field patterns
                    let fields_test_block = self.builder_mut().new_block();
                    self.terminate(TerminatorKind::SwitchInt {
                        discr: Operand::Copy(Place::local(discr_temp)),
                        targets: SwitchTargets::new(
                            vec![(*variant_idx as u128, fields_test_block)],
                            on_no_match,
                        ),
                    });

                    // Test each field pattern on the downcasted variant
                    self.builder_mut().switch_to(fields_test_block);
                    *self.current_block_mut() = fields_test_block;
                    let variant_place = place.project(PlaceElem::Downcast(*variant_idx));
                    self.test_pattern_fields_cf(fields, &variant_place, on_match, on_no_match, span)?;
                }
            }

            PatternKind::Tuple(pats) => {
                // Test each element pattern sequentially
                self.test_pattern_tuple_cf(pats, place, on_match, on_no_match, span)?;
            }

            PatternKind::Struct { fields, .. } => {
                // Test each field pattern sequentially
                let field_pats: Vec<_> = fields.iter()
                    .map(|f| (f.field_idx, &f.pattern))
                    .collect();
                self.test_pattern_struct_fields_cf(&field_pats, place, on_match, on_no_match, span)?;
            }

            PatternKind::Or(alternatives) => {
                // Try each alternative; succeed if any matches
                if alternatives.is_empty() {
                    self.terminate(TerminatorKind::Goto { target: on_no_match });
                } else {
                    self.test_pattern_or_cf(alternatives, place, on_match, on_no_match, span)?;
                }
            }

            PatternKind::Ref { inner, .. } => {
                // Dereference and test inner pattern
                let deref_place = place.project(PlaceElem::Deref);
                self.test_pattern_cf(inner, &deref_place, on_match, on_no_match, span)?;
            }

            PatternKind::Slice { prefix, slice, suffix } => {
                // For slices, check length first, then test element patterns
                self.test_pattern_slice_cf(prefix, slice, suffix, place, on_match, on_no_match, span)?;
            }

            PatternKind::Range { start, end, inclusive } => {
                // Range pattern: check if value is within range
                let value_operand = Operand::Copy(place.clone());

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
                    // No bounds - always matches
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

    /// Test a sequence of tuple element patterns with control flow.
    fn test_pattern_tuple_cf(
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

        // Save the original block - this is where the first test must go
        let original_block = self.current_block();

        // Create intermediate blocks for each pattern test
        let mut next_block = final_match;
        for (i, pat) in pats.iter().enumerate().rev() {
            let field_place = place.project(PlaceElem::Field(i as u32));
            if i == 0 {
                // First pattern test - must use the ORIGINAL block
                self.builder_mut().switch_to(original_block);
                *self.current_block_mut() = original_block;
                self.test_pattern_cf(pat, &field_place, next_block, on_no_match, span)?;
            } else {
                // Create a new block for this test
                let test_block = self.builder_mut().new_block();
                self.builder_mut().switch_to(test_block);
                *self.current_block_mut() = test_block;
                self.test_pattern_cf(pat, &field_place, next_block, on_no_match, span)?;
                next_block = test_block;
            }
        }
        Ok(())
    }

    /// Test field patterns for variant (positional fields) with control flow.
    fn test_pattern_fields_cf(
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

        // Save the original block - this is where the first test must go
        let original_block = self.current_block();

        let mut current_target = final_match;
        for (i, pat) in pats.iter().enumerate().rev() {
            let field_place = place.project(PlaceElem::Field(i as u32));
            if i == 0 {
                // First pattern in sequence - must use the ORIGINAL block
                self.builder_mut().switch_to(original_block);
                *self.current_block_mut() = original_block;
                self.test_pattern_cf(pat, &field_place, current_target, on_no_match, span)?;
            } else {
                let next_block = self.builder_mut().new_block();
                self.builder_mut().switch_to(next_block);
                *self.current_block_mut() = next_block;
                self.test_pattern_cf(pat, &field_place, current_target, on_no_match, span)?;
                current_target = next_block;
            }
        }
        Ok(())
    }

    /// Test struct field patterns (named fields) with control flow.
    fn test_pattern_struct_fields_cf(
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

        // Save the original block - this is where the first test must go
        let original_block = self.current_block();

        let mut current_target = final_match;
        for (i, (field_idx, pat)) in fields.iter().enumerate().rev() {
            let field_place = place.project(PlaceElem::Field(*field_idx));
            if i == 0 {
                // First pattern in sequence - must use the ORIGINAL block
                self.builder_mut().switch_to(original_block);
                *self.current_block_mut() = original_block;
                self.test_pattern_cf(pat, &field_place, current_target, on_no_match, span)?;
            } else {
                let next_block = self.builder_mut().new_block();
                self.builder_mut().switch_to(next_block);
                *self.current_block_mut() = next_block;
                self.test_pattern_cf(pat, &field_place, current_target, on_no_match, span)?;
                current_target = next_block;
            }
        }
        Ok(())
    }

    /// Test or-pattern alternatives with control flow.
    fn test_pattern_or_cf(
        &mut self,
        alternatives: &[Pattern],
        place: &Place,
        on_match: BasicBlockId,
        final_no_match: BasicBlockId,
        span: Span,
    ) -> Result<(), Vec<Diagnostic>> {
        // Try each alternative; if any matches, go to on_match
        // If all fail, go to final_no_match
        for (i, alt) in alternatives.iter().enumerate() {
            let next_try = if i + 1 < alternatives.len() {
                self.builder_mut().new_block()
            } else {
                final_no_match
            };
            self.test_pattern_cf(alt, place, on_match, next_try, span)?;
            if i + 1 < alternatives.len() {
                self.builder_mut().switch_to(next_try);
                *self.current_block_mut() = next_try;
            }
        }
        Ok(())
    }

    /// Test slice pattern with control flow.
    fn test_pattern_slice_cf(
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

        // Check length first
        let len_temp = self.new_temp(Type::usize(), span);
        self.push_assign(
            Place::local(len_temp),
            Rvalue::Len(place.clone()),
        );

        // Compare length
        let len_ok = self.new_temp(Type::bool(), span);
        if slice.is_some() {
            // With rest pattern: len >= min_len
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
            // Without rest: len == min_len
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

        // Branch on length check
        let elements_block = self.builder_mut().new_block();
        self.terminate(TerminatorKind::SwitchInt {
            discr: Operand::Copy(Place::local(len_ok)),
            targets: SwitchTargets::new(vec![(1, elements_block)], on_no_match),
        });

        // Test prefix and suffix patterns
        self.builder_mut().switch_to(elements_block);
        *self.current_block_mut() = elements_block;

        if prefix.is_empty() && suffix.is_empty() {
            self.terminate(TerminatorKind::Goto { target: on_match });
        } else {
            let mut current_target = on_match;

            // Test suffix patterns (in reverse order)
            for (i, pat) in suffix.iter().enumerate().rev() {
                let offset_from_end = (suffix.len() - 1 - i) as u64;
                let idx_place = place.project(PlaceElem::ConstantIndex {
                    offset: offset_from_end,
                    min_length: min_len,
                    from_end: true,
                });
                if i == 0 && prefix.is_empty() {
                    self.test_pattern_cf(pat, &idx_place, current_target, on_no_match, span)?;
                } else if i == 0 {
                    let prefix_block = self.builder_mut().new_block();
                    self.test_pattern_cf(pat, &idx_place, prefix_block, on_no_match, span)?;
                    self.builder_mut().switch_to(prefix_block);
                    *self.current_block_mut() = prefix_block;
                    current_target = on_match;
                } else {
                    let next_block = self.builder_mut().new_block();
                    self.builder_mut().switch_to(next_block);
                    *self.current_block_mut() = next_block;
                    self.test_pattern_cf(pat, &idx_place, current_target, on_no_match, span)?;
                    current_target = next_block;
                }
            }

            // Test prefix patterns (in reverse order)
            // Save the starting block - this is where the first prefix test (i=0) must go
            let prefix_start_block = self.current_block();

            for (i, pat) in prefix.iter().enumerate().rev() {
                let idx_place = place.project(PlaceElem::ConstantIndex {
                    offset: i as u64,
                    min_length: min_len,
                    from_end: false,
                });
                if i == 0 {
                    // For the first pattern (i=0), switch back to the starting block
                    // This ensures elements_block (or prefix_block from suffix) gets terminated
                    self.builder_mut().switch_to(prefix_start_block);
                    *self.current_block_mut() = prefix_start_block;
                    self.test_pattern_cf(pat, &idx_place, current_target, on_no_match, span)?;
                } else {
                    let next_block = self.builder_mut().new_block();
                    self.builder_mut().switch_to(next_block);
                    *self.current_block_mut() = next_block;
                    self.test_pattern_cf(pat, &idx_place, current_target, on_no_match, span)?;
                    current_target = next_block;
                }
            }
        }

        Ok(())
    }

    /// Bind pattern variables to a place with control flow semantics.
    ///
    /// This version creates new temps directly and inserts them into the local map,
    /// which is needed for proper pattern binding in match arms.
    fn bind_pattern_cf(&mut self, pattern: &Pattern, place: &Place) -> Result<(), Vec<Diagnostic>> {
        match &pattern.kind {
            PatternKind::Binding { local_id, mutable, subpattern } => {
                let mir_local = self.new_temp(pattern.ty.clone(), pattern.span);
                self.local_map_mut().insert(*local_id, mir_local);

                // Check if this is a ref binding
                if pattern.ty.is_ref() {
                    self.push_assign(
                        Place::local(mir_local),
                        Rvalue::Ref {
                            place: place.clone(),
                            mutable: *mutable,
                        },
                    );
                } else {
                    self.push_assign(
                        Place::local(mir_local),
                        Rvalue::Use(Operand::Copy(place.clone())),
                    );
                }

                if let Some(subpat) = subpattern {
                    self.bind_pattern_cf(subpat, &Place::local(mir_local))?;
                }
            }
            PatternKind::Tuple(pats) => {
                for (i, pat) in pats.iter().enumerate() {
                    let field_place = place.project(PlaceElem::Field(i as u32));
                    self.bind_pattern_cf(pat, &field_place)?;
                }
            }
            PatternKind::Struct { fields, .. } => {
                for field in fields {
                    let field_place = place.project(PlaceElem::Field(field.field_idx));
                    self.bind_pattern_cf(&field.pattern, &field_place)?;
                }
            }
            PatternKind::Wildcard | PatternKind::Literal(_) | PatternKind::Range { .. } => {
                // Nothing to bind
            }
            PatternKind::Variant { variant_idx, fields, .. } => {
                let variant_place = place.project(PlaceElem::Downcast(*variant_idx));
                for (i, field_pat) in fields.iter().enumerate() {
                    let field_place = variant_place.project(PlaceElem::Field(i as u32));
                    self.bind_pattern_cf(field_pat, &field_place)?;
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
                    self.bind_pattern_cf(pat, &idx_place)?;
                }

                for (i, pat) in suffix.iter().enumerate() {
                    let offset_from_end = (suffix.len() - 1 - i) as u64;
                    let idx_place = place.project(PlaceElem::ConstantIndex {
                        offset: offset_from_end,
                        min_length,
                        from_end: true,
                    });
                    self.bind_pattern_cf(pat, &idx_place)?;
                }

                if let Some(rest_pat) = slice {
                    let subslice_place = place.project(PlaceElem::Subslice {
                        from: prefix.len() as u64,
                        to: suffix.len() as u64,
                        from_end: true,
                    });
                    self.bind_pattern_cf(rest_pat, &subslice_place)?;
                }
            }
            PatternKind::Or(alternatives) => {
                if let Some(first_alt) = alternatives.first() {
                    self.bind_pattern_cf(first_alt, place)?;
                }
            }
            PatternKind::Ref { inner, .. } => {
                let deref_place = place.project(PlaceElem::Deref);
                self.bind_pattern_cf(inner, &deref_place)?;
            }
        }
        Ok(())
    }

    /// Lower a closure expression.
    fn lower_closure(
        &mut self,
        body_id: hir::BodyId,
        captures: &[hir::Capture],
        ty: &Type,
        span: Span,
    ) -> Result<Operand, Vec<Diagnostic>> {
        let closure_def_id = self.next_closure_def_id();

        let mut capture_operands = Vec::with_capacity(captures.len());
        let mut captures_with_types = Vec::with_capacity(captures.len());

        for capture in captures {
            // For closures, we need to handle both local and captured variables
            let operand = if let Some(field_idx) = self.get_capture_field(capture.local_id) {
                // This is a re-capture of an outer closure's capture
                if let Some(env_local) = self.get_env_local() {
                    let env_place = Place::local(env_local);
                    let capture_place = env_place.project(PlaceElem::Field(field_idx as u32));
                    if capture.by_move {
                        Operand::Move(capture_place)
                    } else {
                        Operand::Copy(capture_place)
                    }
                } else {
                    return Err(vec![Diagnostic::error(
                        "internal error: captured variable but no environment".to_string(),
                        span,
                    )]);
                }
            } else {
                // Regular local
                let mir_local = self.map_local(capture.local_id);
                let place = Place::local(mir_local);
                if capture.by_move {
                    Operand::Move(place)
                } else {
                    Operand::Copy(place)
                }
            };

            let capture_ty = self.body().get_local(capture.local_id)
                .map(|l| l.ty.clone())
                .unwrap_or_else(Type::error);

            capture_operands.push(operand);
            captures_with_types.push((capture.clone(), capture_ty));
        }

        self.pending_closures_mut().push((body_id, closure_def_id, captures_with_types));

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
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::{DefId, FieldPattern, LocalId as HirLocalId};
    use crate::span::Span;

    #[test]
    fn test_convert_binop() {
        assert_eq!(convert_binop(BinOp::Add), MirBinOp::Add);
        assert_eq!(convert_binop(BinOp::Sub), MirBinOp::Sub);
        assert_eq!(convert_binop(BinOp::Eq), MirBinOp::Eq);
        assert_eq!(convert_binop(BinOp::And), MirBinOp::BitAnd);
        assert_eq!(convert_binop(BinOp::Or), MirBinOp::BitOr);
    }

    #[test]
    fn test_convert_unop() {
        assert_eq!(convert_unop(UnaryOp::Neg), Some(MirUnOp::Neg));
        assert_eq!(convert_unop(UnaryOp::Not), Some(MirUnOp::Not));
        // Special operators return None
        assert_eq!(convert_unop(UnaryOp::Deref), None);
        assert_eq!(convert_unop(UnaryOp::Ref), None);
        assert_eq!(convert_unop(UnaryOp::RefMut), None);
    }

    #[test]
    fn test_lower_literal_to_constant() {
        let int_lit = LiteralValue::Int(42);
        let int_const = lower_literal_to_constant(&int_lit, &Type::i64());
        assert!(matches!(int_const.kind, ConstantKind::Int(42)));

        let bool_lit = LiteralValue::Bool(true);
        let bool_const = lower_literal_to_constant(&bool_lit, &Type::bool());
        assert!(matches!(bool_const.kind, ConstantKind::Bool(true)));

        let string_lit = LiteralValue::String("hello".to_string());
        let string_const = lower_literal_to_constant(&string_lit, &Type::string());
        assert!(matches!(string_const.kind, ConstantKind::String(ref s) if s == "hello"));
    }

    fn make_pattern(kind: PatternKind) -> Pattern {
        Pattern {
            kind,
            ty: Type::i64(),
            span: Span::dummy(),
        }
    }

    #[test]
    fn test_is_irrefutable_wildcard() {
        let pat = make_pattern(PatternKind::Wildcard);
        assert!(is_irrefutable_pattern(&pat));
    }

    #[test]
    fn test_is_irrefutable_binding() {
        // Simple binding is irrefutable
        let pat = make_pattern(PatternKind::Binding {
            local_id: HirLocalId::new(1),
            mutable: false,
            subpattern: None,
        });
        assert!(is_irrefutable_pattern(&pat));

        // Binding with irrefutable subpattern is irrefutable
        let subpat = Box::new(make_pattern(PatternKind::Wildcard));
        let pat = make_pattern(PatternKind::Binding {
            local_id: HirLocalId::new(2),
            mutable: false,
            subpattern: Some(subpat),
        });
        assert!(is_irrefutable_pattern(&pat));

        // Binding with refutable subpattern is refutable
        let subpat = Box::new(make_pattern(PatternKind::Literal(LiteralValue::Int(42))));
        let pat = make_pattern(PatternKind::Binding {
            local_id: HirLocalId::new(3),
            mutable: false,
            subpattern: Some(subpat),
        });
        assert!(!is_irrefutable_pattern(&pat));
    }

    #[test]
    fn test_is_irrefutable_tuple() {
        // Empty tuple is irrefutable
        let pat = make_pattern(PatternKind::Tuple(vec![]));
        assert!(is_irrefutable_pattern(&pat));

        // Tuple with all irrefutable patterns is irrefutable
        let pat = make_pattern(PatternKind::Tuple(vec![
            make_pattern(PatternKind::Wildcard),
            make_pattern(PatternKind::Binding {
                local_id: HirLocalId::new(1),
                mutable: false,
                subpattern: None,
            }),
        ]));
        assert!(is_irrefutable_pattern(&pat));

        // Tuple with any refutable pattern is refutable
        let pat = make_pattern(PatternKind::Tuple(vec![
            make_pattern(PatternKind::Wildcard),
            make_pattern(PatternKind::Literal(LiteralValue::Int(42))),
        ]));
        assert!(!is_irrefutable_pattern(&pat));
    }

    #[test]
    fn test_is_refutable_literal() {
        let pat = make_pattern(PatternKind::Literal(LiteralValue::Int(42)));
        assert!(!is_irrefutable_pattern(&pat));
    }

    #[test]
    fn test_is_refutable_variant() {
        let pat = make_pattern(PatternKind::Variant {
            def_id: DefId::new(1),
            variant_idx: 0,
            fields: vec![],
        });
        assert!(!is_irrefutable_pattern(&pat));
    }

    #[test]
    fn test_is_irrefutable_struct() {
        // Struct with all irrefutable field patterns is irrefutable
        let pat = make_pattern(PatternKind::Struct {
            def_id: DefId::new(1),
            fields: vec![
                FieldPattern {
                    field_idx: 0,
                    pattern: make_pattern(PatternKind::Wildcard),
                },
            ],
        });
        assert!(is_irrefutable_pattern(&pat));

        // Struct with refutable field pattern is refutable
        let pat = make_pattern(PatternKind::Struct {
            def_id: DefId::new(1),
            fields: vec![
                FieldPattern {
                    field_idx: 0,
                    pattern: make_pattern(PatternKind::Literal(LiteralValue::Int(42))),
                },
            ],
        });
        assert!(!is_irrefutable_pattern(&pat));
    }

    #[test]
    fn test_is_irrefutable_slice() {
        // Slice with rest element is irrefutable
        let pat = make_pattern(PatternKind::Slice {
            prefix: vec![make_pattern(PatternKind::Wildcard)],
            slice: Some(Box::new(make_pattern(PatternKind::Wildcard))),
            suffix: vec![],
        });
        assert!(is_irrefutable_pattern(&pat));

        // Slice without rest element is refutable (must match exact length)
        let pat = make_pattern(PatternKind::Slice {
            prefix: vec![make_pattern(PatternKind::Wildcard)],
            slice: None,
            suffix: vec![],
        });
        assert!(!is_irrefutable_pattern(&pat));
    }

    // ============================================================================
    // Property Tests for MIR Lowering Correctness (FUZZ-004)
    // ============================================================================
    //
    // These property tests verify structural invariants of MIR lowering utilities.
    // They complement the unit tests above by testing algebraic properties rather
    // than specific examples.

    use proptest::prelude::*;

    // ------------------------------------------------------------------------
    // Binary Operator Property Tests
    // ------------------------------------------------------------------------

    /// Generate all AST binary operators.
    fn all_ast_binops() -> Vec<BinOp> {
        vec![
            BinOp::Add, BinOp::Sub, BinOp::Mul, BinOp::Div, BinOp::Rem,
            BinOp::Eq, BinOp::Ne, BinOp::Lt, BinOp::Le, BinOp::Gt, BinOp::Ge,
            BinOp::And, BinOp::Or,
            BinOp::BitAnd, BinOp::BitOr, BinOp::BitXor, BinOp::Shl, BinOp::Shr,
            BinOp::Pipe,
        ]
    }

    /// Generate all MIR binary operators (for validation).
    fn all_mir_binops() -> Vec<MirBinOp> {
        vec![
            MirBinOp::Add, MirBinOp::Sub, MirBinOp::Mul, MirBinOp::Div, MirBinOp::Rem,
            MirBinOp::BitAnd, MirBinOp::BitOr, MirBinOp::BitXor,
            MirBinOp::Shl, MirBinOp::Shr,
            MirBinOp::Eq, MirBinOp::Ne, MirBinOp::Lt, MirBinOp::Le, MirBinOp::Gt, MirBinOp::Ge,
            MirBinOp::Offset,
        ]
    }

    // PROPERTY: BinOp conversion is total - every AST BinOp maps to some MIR BinOp
    #[test]
    fn test_property_binop_conversion_total() {
        for ast_op in all_ast_binops() {
            let mir_op = convert_binop(ast_op);
            // Verify result is a valid MIR binop (it compiles, so it's valid)
            // This ensures we don't panic and always produce a result
            let _ = mir_op;
        }
    }

    // PROPERTY: BinOp conversion produces valid MIR operators
    #[test]
    fn test_property_binop_conversion_valid_output() {
        let valid_mir_ops = all_mir_binops();
        for ast_op in all_ast_binops() {
            let mir_op = convert_binop(ast_op);
            assert!(
                valid_mir_ops.contains(&mir_op),
                "convert_binop({:?}) produced invalid MIR operator: {:?}",
                ast_op, mir_op
            );
        }
    }

    // PROPERTY: Arithmetic operators map to arithmetic MIR operators
    #[test]
    fn test_property_arithmetic_binop_preservation() {
        let arithmetic_pairs = vec![
            (BinOp::Add, MirBinOp::Add),
            (BinOp::Sub, MirBinOp::Sub),
            (BinOp::Mul, MirBinOp::Mul),
            (BinOp::Div, MirBinOp::Div),
            (BinOp::Rem, MirBinOp::Rem),
        ];
        for (ast_op, expected_mir_op) in arithmetic_pairs {
            assert_eq!(
                convert_binop(ast_op), expected_mir_op,
                "Arithmetic operator {:?} should map to {:?}", ast_op, expected_mir_op
            );
        }
    }

    // PROPERTY: Comparison operators map to comparison MIR operators
    #[test]
    fn test_property_comparison_binop_preservation() {
        let comparison_pairs = vec![
            (BinOp::Eq, MirBinOp::Eq),
            (BinOp::Ne, MirBinOp::Ne),
            (BinOp::Lt, MirBinOp::Lt),
            (BinOp::Le, MirBinOp::Le),
            (BinOp::Gt, MirBinOp::Gt),
            (BinOp::Ge, MirBinOp::Ge),
        ];
        for (ast_op, expected_mir_op) in comparison_pairs {
            assert_eq!(
                convert_binop(ast_op), expected_mir_op,
                "Comparison operator {:?} should map to {:?}", ast_op, expected_mir_op
            );
        }
    }

    // PROPERTY: Bitwise operators map to bitwise MIR operators
    #[test]
    fn test_property_bitwise_binop_preservation() {
        let bitwise_pairs = vec![
            (BinOp::BitAnd, MirBinOp::BitAnd),
            (BinOp::BitOr, MirBinOp::BitOr),
            (BinOp::BitXor, MirBinOp::BitXor),
            (BinOp::Shl, MirBinOp::Shl),
            (BinOp::Shr, MirBinOp::Shr),
        ];
        for (ast_op, expected_mir_op) in bitwise_pairs {
            assert_eq!(
                convert_binop(ast_op), expected_mir_op,
                "Bitwise operator {:?} should map to {:?}", ast_op, expected_mir_op
            );
        }
    }

    // ------------------------------------------------------------------------
    // Unary Operator Property Tests
    // ------------------------------------------------------------------------

    /// Generate all AST unary operators.
    fn all_ast_unops() -> Vec<UnaryOp> {
        vec![UnaryOp::Neg, UnaryOp::Not, UnaryOp::Deref, UnaryOp::Ref, UnaryOp::RefMut]
    }

    // PROPERTY: UnaryOp conversion is total - every AST UnaryOp either maps to
    // Some(MirUnOp) or None (for place-based operations)
    #[test]
    fn test_property_unop_conversion_total() {
        for ast_op in all_ast_unops() {
            // This should never panic
            let _ = convert_unop(ast_op);
        }
    }

    // PROPERTY: Simple unary operators (Neg, Not) always produce Some
    #[test]
    fn test_property_simple_unop_produces_some() {
        assert!(convert_unop(UnaryOp::Neg).is_some(), "Neg should map to Some");
        assert!(convert_unop(UnaryOp::Not).is_some(), "Not should map to Some");
    }

    // PROPERTY: Place-based unary operators (Deref, Ref, RefMut) always produce None
    #[test]
    fn test_property_place_unop_produces_none() {
        assert!(convert_unop(UnaryOp::Deref).is_none(), "Deref should map to None");
        assert!(convert_unop(UnaryOp::Ref).is_none(), "Ref should map to None");
        assert!(convert_unop(UnaryOp::RefMut).is_none(), "RefMut should map to None");
    }

    // PROPERTY: UnaryOp conversion preserves operator semantics
    #[test]
    fn test_property_unop_semantic_preservation() {
        assert_eq!(convert_unop(UnaryOp::Neg), Some(MirUnOp::Neg));
        assert_eq!(convert_unop(UnaryOp::Not), Some(MirUnOp::Not));
    }

    // ------------------------------------------------------------------------
    // Literal Conversion Property Tests
    // ------------------------------------------------------------------------

    // PROPERTY: Integer literal conversion preserves value
    proptest! {
        #[test]
        fn test_property_int_literal_value_preserved(value in any::<i64>()) {
            let lit = LiteralValue::Int(value as i128);
            let constant = lower_literal_to_constant(&lit, &Type::i64());
            match constant.kind {
                ConstantKind::Int(v) => assert_eq!(v, value as i128),
                _ => panic!("Expected Int constant"),
            }
        }
    }

    // PROPERTY: Unsigned literal conversion preserves value (within range)
    proptest! {
        #[test]
        fn test_property_uint_literal_value_preserved(value in 0u64..=i128::MAX as u64) {
            let lit = LiteralValue::Uint(value as u128);
            let constant = lower_literal_to_constant(&lit, &Type::u64());
            match constant.kind {
                ConstantKind::Int(v) => assert_eq!(v, value as i128),
                _ => panic!("Expected Int constant"),
            }
        }
    }

    // PROPERTY: Float literal conversion preserves value
    proptest! {
        #[test]
        fn test_property_float_literal_value_preserved(value in any::<f64>().prop_filter("not NaN", |v| !v.is_nan())) {
            let lit = LiteralValue::Float(value);
            let constant = lower_literal_to_constant(&lit, &Type::f64());
            match constant.kind {
                ConstantKind::Float(v) => {
                    if value.is_infinite() {
                        assert!(v.is_infinite() && v.signum() == value.signum());
                    } else {
                        assert_eq!(v, value);
                    }
                }
                _ => panic!("Expected Float constant"),
            }
        }
    }

    // PROPERTY: Boolean literal conversion preserves value
    proptest! {
        #[test]
        fn test_property_bool_literal_value_preserved(value in any::<bool>()) {
            let lit = LiteralValue::Bool(value);
            let constant = lower_literal_to_constant(&lit, &Type::bool());
            match constant.kind {
                ConstantKind::Bool(v) => assert_eq!(v, value),
                _ => panic!("Expected Bool constant"),
            }
        }
    }

    // PROPERTY: Char literal conversion preserves value
    proptest! {
        #[test]
        fn test_property_char_literal_value_preserved(value in any::<char>()) {
            let lit = LiteralValue::Char(value);
            let constant = lower_literal_to_constant(&lit, &Type::char());
            match constant.kind {
                ConstantKind::Char(v) => assert_eq!(v, value),
                _ => panic!("Expected Char constant"),
            }
        }
    }

    // PROPERTY: String literal conversion preserves value
    proptest! {
        #[test]
        fn test_property_string_literal_value_preserved(value in ".*") {
            let lit = LiteralValue::String(value.clone());
            let constant = lower_literal_to_constant(&lit, &Type::string());
            match &constant.kind {
                ConstantKind::String(v) => assert_eq!(v, &value),
                _ => panic!("Expected String constant"),
            }
        }
    }

    // PROPERTY: Literal conversion preserves type
    #[test]
    fn test_property_literal_type_preserved() {
        let test_cases: Vec<(LiteralValue, Type)> = vec![
            (LiteralValue::Int(42), Type::i32()),
            (LiteralValue::Int(-100), Type::i64()),
            (LiteralValue::Uint(42), Type::u32()),
            (LiteralValue::Float(3.14), Type::f32()),
            (LiteralValue::Float(2.718), Type::f64()),
            (LiteralValue::Bool(true), Type::bool()),
            (LiteralValue::Char('x'), Type::char()),
            (LiteralValue::String("hello".to_string()), Type::string()),
        ];

        for (lit, ty) in test_cases {
            let constant = lower_literal_to_constant(&lit, &ty);
            assert_eq!(
                constant.ty, ty,
                "Type should be preserved for literal {:?}", lit
            );
        }
    }

    // ------------------------------------------------------------------------
    // Pattern Irrefutability Property Tests
    // ------------------------------------------------------------------------

    // PROPERTY: Wildcard is always irrefutable
    #[test]
    fn test_property_wildcard_always_irrefutable() {
        let pat = make_pattern(PatternKind::Wildcard);
        assert!(is_irrefutable_pattern(&pat), "Wildcard must always be irrefutable");
    }

    // PROPERTY: Simple binding without subpattern is always irrefutable
    proptest! {
        #[test]
        fn test_property_simple_binding_always_irrefutable(
            local_id in 0u32..1000,
            mutable in any::<bool>()
        ) {
            let pat = make_pattern(PatternKind::Binding {
                local_id: HirLocalId::new(local_id),
                mutable,
                subpattern: None,
            });
            assert!(is_irrefutable_pattern(&pat), "Simple binding must be irrefutable");
        }
    }

    // PROPERTY: Literal patterns are always refutable
    #[test]
    fn test_property_literal_always_refutable() {
        let literals = vec![
            LiteralValue::Int(0),
            LiteralValue::Int(42),
            LiteralValue::Int(-1),
            LiteralValue::Uint(0),
            LiteralValue::Float(0.0),
            LiteralValue::Bool(true),
            LiteralValue::Bool(false),
            LiteralValue::Char('a'),
            LiteralValue::String("".to_string()),
        ];

        for lit in literals {
            let pat = make_pattern(PatternKind::Literal(lit.clone()));
            assert!(!is_irrefutable_pattern(&pat), "Literal {:?} must be refutable", lit);
        }
    }

    // PROPERTY: Variant patterns are always refutable
    proptest! {
        #[test]
        fn test_property_variant_always_refutable(
            def_id in 0u32..1000,
            variant_idx in 0u32..100
        ) {
            let pat = make_pattern(PatternKind::Variant {
                def_id: DefId::new(def_id),
                variant_idx,
                fields: vec![],
            });
            assert!(!is_irrefutable_pattern(&pat), "Variant pattern must be refutable");
        }
    }

    // PROPERTY: Or patterns are always refutable (by design choice)
    #[test]
    fn test_property_or_pattern_always_refutable() {
        // Even Or(wildcard, wildcard) is considered refutable in this implementation
        let pat = make_pattern(PatternKind::Or(vec![
            make_pattern(PatternKind::Wildcard),
            make_pattern(PatternKind::Wildcard),
        ]));
        assert!(!is_irrefutable_pattern(&pat), "Or pattern must be refutable");
    }

    // PROPERTY: Range patterns are always refutable
    #[test]
    fn test_property_range_pattern_always_refutable() {
        let pat = make_pattern(PatternKind::Range {
            start: None,
            end: None,
            inclusive: true,
        });
        assert!(!is_irrefutable_pattern(&pat), "Range pattern must be refutable");
    }

    // PROPERTY: Empty tuple is irrefutable
    #[test]
    fn test_property_empty_tuple_irrefutable() {
        let pat = make_pattern(PatternKind::Tuple(vec![]));
        assert!(is_irrefutable_pattern(&pat), "Empty tuple must be irrefutable");
    }

    // PROPERTY: Tuple of wildcards is irrefutable
    proptest! {
        #[test]
        fn test_property_tuple_of_wildcards_irrefutable(n in 0usize..10) {
            let pats: Vec<Pattern> = (0..n)
                .map(|_| make_pattern(PatternKind::Wildcard))
                .collect();
            let pat = make_pattern(PatternKind::Tuple(pats));
            assert!(is_irrefutable_pattern(&pat), "Tuple of wildcards must be irrefutable");
        }
    }

    // PROPERTY: Tuple with any literal element is refutable
    proptest! {
        #[test]
        fn test_property_tuple_with_literal_refutable(
            n in 1usize..5,
            lit_pos in 0usize..5
        ) {
            let lit_pos = lit_pos % n;  // Ensure lit_pos is valid
            let pats: Vec<Pattern> = (0..n)
                .map(|i| {
                    if i == lit_pos {
                        make_pattern(PatternKind::Literal(LiteralValue::Int(42)))
                    } else {
                        make_pattern(PatternKind::Wildcard)
                    }
                })
                .collect();
            let pat = make_pattern(PatternKind::Tuple(pats));
            assert!(!is_irrefutable_pattern(&pat), "Tuple with literal must be refutable");
        }
    }

    // PROPERTY: Ref pattern preserves irrefutability of inner pattern
    #[test]
    fn test_property_ref_preserves_irrefutability() {
        // Ref of wildcard is irrefutable
        let irrefutable_ref = make_pattern(PatternKind::Ref {
            mutable: false,
            inner: Box::new(make_pattern(PatternKind::Wildcard)),
        });
        assert!(is_irrefutable_pattern(&irrefutable_ref), "Ref of wildcard must be irrefutable");

        // Ref of literal is refutable
        let refutable_ref = make_pattern(PatternKind::Ref {
            mutable: true,
            inner: Box::new(make_pattern(PatternKind::Literal(LiteralValue::Int(42)))),
        });
        assert!(!is_irrefutable_pattern(&refutable_ref), "Ref of literal must be refutable");
    }

    // PROPERTY: Struct with all wildcard fields is irrefutable
    #[test]
    fn test_property_struct_all_wildcards_irrefutable() {
        let pat = make_pattern(PatternKind::Struct {
            def_id: DefId::new(1),
            fields: vec![
                FieldPattern {
                    field_idx: 0,
                    pattern: make_pattern(PatternKind::Wildcard),
                },
                FieldPattern {
                    field_idx: 1,
                    pattern: make_pattern(PatternKind::Wildcard),
                },
            ],
        });
        assert!(is_irrefutable_pattern(&pat), "Struct with all wildcards must be irrefutable");
    }

    // PROPERTY: Empty struct (no fields) is irrefutable
    #[test]
    fn test_property_empty_struct_irrefutable() {
        let pat = make_pattern(PatternKind::Struct {
            def_id: DefId::new(1),
            fields: vec![],
        });
        assert!(is_irrefutable_pattern(&pat), "Empty struct must be irrefutable");
    }

    // PROPERTY: Slice with rest element and irrefutable prefix/suffix is irrefutable
    #[test]
    fn test_property_slice_with_rest_irrefutable() {
        let pat = make_pattern(PatternKind::Slice {
            prefix: vec![
                make_pattern(PatternKind::Wildcard),
                make_pattern(PatternKind::Binding {
                    local_id: HirLocalId::new(1),
                    mutable: false,
                    subpattern: None,
                }),
            ],
            slice: Some(Box::new(make_pattern(PatternKind::Wildcard))),
            suffix: vec![make_pattern(PatternKind::Wildcard)],
        });
        assert!(is_irrefutable_pattern(&pat), "Slice with rest and irrefutable elements must be irrefutable");
    }

    // PROPERTY: Slice without rest element is always refutable
    proptest! {
        #[test]
        fn test_property_slice_without_rest_refutable(n in 0usize..5) {
            let pats: Vec<Pattern> = (0..n)
                .map(|_| make_pattern(PatternKind::Wildcard))
                .collect();
            let pat = make_pattern(PatternKind::Slice {
                prefix: pats,
                slice: None,
                suffix: vec![],
            });
            assert!(!is_irrefutable_pattern(&pat), "Slice without rest must be refutable");
        }
    }

    // PROPERTY: Binding with irrefutable subpattern is irrefutable
    #[test]
    fn test_property_binding_subpattern_irrefutability() {
        // Binding with wildcard subpattern is irrefutable
        let irrefutable = make_pattern(PatternKind::Binding {
            local_id: HirLocalId::new(1),
            mutable: false,
            subpattern: Some(Box::new(make_pattern(PatternKind::Wildcard))),
        });
        assert!(is_irrefutable_pattern(&irrefutable), "Binding @ _ must be irrefutable");

        // Binding with literal subpattern is refutable
        let refutable = make_pattern(PatternKind::Binding {
            local_id: HirLocalId::new(2),
            mutable: false,
            subpattern: Some(Box::new(make_pattern(PatternKind::Literal(LiteralValue::Int(0))))),
        });
        assert!(!is_irrefutable_pattern(&refutable), "Binding @ 0 must be refutable");
    }

    // PROPERTY: Deeply nested irrefutable patterns are irrefutable
    #[test]
    fn test_property_deeply_nested_irrefutable() {
        // (((_,), _), _) should be irrefutable
        let inner_tuple = make_pattern(PatternKind::Tuple(vec![
            make_pattern(PatternKind::Wildcard),
        ]));
        let middle_tuple = make_pattern(PatternKind::Tuple(vec![
            inner_tuple,
            make_pattern(PatternKind::Wildcard),
        ]));
        let outer_tuple = make_pattern(PatternKind::Tuple(vec![
            middle_tuple,
            make_pattern(PatternKind::Wildcard),
        ]));
        assert!(is_irrefutable_pattern(&outer_tuple), "Deeply nested wildcards must be irrefutable");
    }

    // PROPERTY: Single refutable element in deep nesting makes whole pattern refutable
    #[test]
    fn test_property_deeply_nested_single_refutable() {
        // (((42,), _), _) should be refutable because of the 42
        let inner_tuple = make_pattern(PatternKind::Tuple(vec![
            make_pattern(PatternKind::Literal(LiteralValue::Int(42))),
        ]));
        let middle_tuple = make_pattern(PatternKind::Tuple(vec![
            inner_tuple,
            make_pattern(PatternKind::Wildcard),
        ]));
        let outer_tuple = make_pattern(PatternKind::Tuple(vec![
            middle_tuple,
            make_pattern(PatternKind::Wildcard),
        ]));
        assert!(!is_irrefutable_pattern(&outer_tuple), "Deeply nested literal makes pattern refutable");
    }
}
