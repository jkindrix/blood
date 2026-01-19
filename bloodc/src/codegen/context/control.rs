//! Control flow compilation for codegen.
//!
//! This module handles compilation of control flow constructs like
//! blocks, if expressions, loops, break/continue, and return.

use inkwell::values::BasicValueEnum;
use inkwell::IntPredicate;

use crate::hir::{self, Type};
use crate::diagnostics::Diagnostic;
use crate::span::Span;

use super::{CodegenContext, LoopContext};

impl<'ctx, 'a> CodegenContext<'ctx, 'a> {
    /// Compile a block expression.
    pub(super) fn compile_block(
        &mut self,
        stmts: &[hir::Stmt],
        tail_expr: Option<&hir::Expr>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        // Compile each statement
        for stmt in stmts {
            match stmt {
                hir::Stmt::Let { local_id, init } => {
                    // Allocate local variable
                    // Get the local info from the current function's locals
                    // For now, infer the type from the init expression or use a default
                    let llvm_type = if let Some(init_expr) = init {
                        self.lower_type(&init_expr.ty)
                    } else {
                        // Default to i32 for uninitialized locals without type info
                        self.context.i32_type().into()
                    };

                    let alloca = self.builder
                        .build_alloca(llvm_type, &format!("local_{}", local_id.index))
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

                    if let Some(init_expr) = init {
                        if let Some(init_val) = self.compile_expr(init_expr)? {
                            self.builder.build_store(alloca, init_val)
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
                        }
                    }

                    self.locals.insert(*local_id, alloca);
                }
                hir::Stmt::Expr(expr) => {
                    self.compile_expr(expr)?;
                }
                hir::Stmt::Item(_) => {
                    // Nested items handled separately
                }
                hir::Stmt::Defer { body } => {
                    // TODO: Proper defer semantics (execute at scope exit)
                    // For now, just record the deferred expression
                    // Full implementation requires cleanup blocks and scope tracking
                    let _ = body; // Silence unused warning
                }
            }
        }

        if let Some(tail) = tail_expr {
            self.compile_expr(tail)
        } else {
            Ok(None)
        }
    }

    /// Compile an if expression.
    pub(super) fn compile_if(
        &mut self,
        condition: &hir::Expr,
        then_branch: &hir::Expr,
        else_branch: Option<&hir::Expr>,
        result_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        let fn_value = self.current_fn
            .ok_or_else(|| vec![Diagnostic::error("If outside function", Span::dummy())])?;

        let cond_val = self.compile_expr(condition)?
            .ok_or_else(|| vec![Diagnostic::error("Expected condition value", condition.span)])?;

        // Convert to i1 if needed
        let cond_bool = if cond_val.is_int_value() {
            let int_val = cond_val.into_int_value();
            if int_val.get_type().get_bit_width() == 1 {
                int_val
            } else {
                self.builder.build_int_compare(
                    IntPredicate::NE,
                    int_val,
                    int_val.get_type().const_zero(),
                    "tobool",
                ).map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
            }
        } else {
            return Err(vec![Diagnostic::error("Condition must be boolean", condition.span)]);
        };

        let then_bb = self.context.append_basic_block(fn_value, "then");
        let else_bb = self.context.append_basic_block(fn_value, "else");
        let merge_bb = self.context.append_basic_block(fn_value, "merge");

        self.builder.build_conditional_branch(cond_bool, then_bb, else_bb)
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

        // Compile then branch
        self.builder.position_at_end(then_bb);
        let then_val = self.compile_expr(then_branch)?;
        self.builder.build_unconditional_branch(merge_bb)
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
        let then_bb = self.get_current_block()?;

        // Compile else branch
        self.builder.position_at_end(else_bb);
        let else_val = if let Some(else_expr) = else_branch {
            self.compile_expr(else_expr)?
        } else {
            None
        };
        self.builder.build_unconditional_branch(merge_bb)
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
        let else_bb = self.get_current_block()?;

        // Merge
        self.builder.position_at_end(merge_bb);

        // If non-unit result type, create phi node
        if !result_ty.is_unit() {
            if let (Some(then_v), Some(else_v)) = (then_val, else_val) {
                let phi = self.builder.build_phi(self.lower_type(result_ty), "ifresult")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
                phi.add_incoming(&[(&then_v, then_bb), (&else_v, else_bb)]);
                return Ok(Some(phi.as_basic_value()));
            }
        }

        Ok(None)
    }

    /// Compile a while loop.
    pub(super) fn compile_while(
        &mut self,
        condition: &hir::Expr,
        body: &hir::Expr,
    ) -> Result<(), Vec<Diagnostic>> {
        let fn_value = self.current_fn
            .ok_or_else(|| vec![Diagnostic::error("While outside function", Span::dummy())])?;

        let cond_bb = self.context.append_basic_block(fn_value, "while.cond");
        let body_bb = self.context.append_basic_block(fn_value, "while.body");
        let end_bb = self.context.append_basic_block(fn_value, "while.end");

        // Jump to condition
        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

        // Compile condition
        self.builder.position_at_end(cond_bb);
        let cond_val = self.compile_expr(condition)?
            .ok_or_else(|| vec![Diagnostic::error("Expected condition value", condition.span)])?
            .into_int_value();

        // Ensure boolean
        let cond_bool = if cond_val.get_type().get_bit_width() == 1 {
            cond_val
        } else {
            self.builder.build_int_compare(
                IntPredicate::NE,
                cond_val,
                cond_val.get_type().const_zero(),
                "tobool",
            ).map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
        };

        self.builder.build_conditional_branch(cond_bool, body_bb, end_bb)
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

        // Compile body
        self.builder.position_at_end(body_bb);
        self.compile_expr(body)?;
        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

        // Continue after loop
        self.builder.position_at_end(end_bb);

        Ok(())
    }

    /// Compile a return statement.
    pub(super) fn compile_return(&mut self, value: Option<&hir::Expr>) -> Result<(), Vec<Diagnostic>> {
        if let Some(val_expr) = value {
            if let Some(val) = self.compile_expr(val_expr)? {
                self.builder.build_return(Some(&val))
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
            } else {
                self.builder.build_return(None)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
            }
        } else {
            self.builder.build_return(None)
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
        }
        Ok(())
    }

    /// Compile an assignment.
    pub(super) fn compile_assign(&mut self, target: &hir::Expr, value: &hir::Expr) -> Result<(), Vec<Diagnostic>> {
        let val = self.compile_expr(value)?
            .ok_or_else(|| vec![Diagnostic::error("Expected value for assignment", value.span)])?;

        // Get target address
        match &target.kind {
            hir::ExprKind::Local(local_id) => {
                let alloca = self.locals.get(local_id)
                    .ok_or_else(|| vec![Diagnostic::error("Local not found", target.span)])?;
                self.builder.build_store(*alloca, val)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
            }
            _ => {
                return Err(vec![Diagnostic::error(
                    "Cannot assign to this expression",
                    target.span,
                )]);
            }
        }

        Ok(())
    }

    /// Compile a loop expression.
    pub(super) fn compile_loop(
        &mut self,
        body: &hir::Expr,
        label: Option<hir::LoopId>,
        result_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        let fn_value = self.current_fn
            .ok_or_else(|| vec![Diagnostic::error("Loop outside function", Span::dummy())])?;

        let loop_bb = self.context.append_basic_block(fn_value, "loop.body");
        let exit_bb = self.context.append_basic_block(fn_value, "loop.exit");

        // Allocate storage for break value if non-unit result type
        let break_value_store = if !result_ty.is_unit() {
            let llvm_type = self.lower_type(result_ty);
            Some(self.builder
                .build_alloca(llvm_type, "loop.result")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?)
        } else {
            None
        };

        // Push loop context
        self.loop_stack.push(LoopContext {
            continue_block: loop_bb,
            exit_block: exit_bb,
            label,
            break_value_store,
        });

        // Jump to loop body
        self.builder.build_unconditional_branch(loop_bb)
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

        // Compile loop body
        self.builder.position_at_end(loop_bb);
        self.compile_expr(body)?;

        // If body didn't terminate, loop back
        if self.get_current_block()?.get_terminator().is_none() {
            self.builder.build_unconditional_branch(loop_bb)
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
        }

        // Pop loop context
        self.loop_stack.pop();

        // Position at exit block
        self.builder.position_at_end(exit_bb);

        // Load break value if present
        if let Some(store) = break_value_store {
            let val = self.builder.build_load(store, "loop.result.load")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
            Ok(Some(val))
        } else {
            Ok(None)
        }
    }

    /// Compile a break statement.
    pub(super) fn compile_break(
        &mut self,
        label: Option<hir::LoopId>,
        value: Option<&hir::Expr>,
    ) -> Result<(), Vec<Diagnostic>> {
        // Find the loop context
        let loop_ctx = if let Some(label) = label {
            self.loop_stack.iter().rev()
                .find(|ctx| ctx.label == Some(label))
                .cloned()
                .ok_or_else(|| vec![Diagnostic::error(
                    format!("Cannot find loop with label {:?}", label),
                    Span::dummy(),
                )])?
        } else {
            self.loop_stack.last()
                .cloned()
                .ok_or_else(|| vec![Diagnostic::error(
                    "Break outside of loop",
                    Span::dummy(),
                )])?
        };

        // Compile and store break value if present
        if let Some(val_expr) = value {
            if let Some(val) = self.compile_expr(val_expr)? {
                if let Some(store) = loop_ctx.break_value_store {
                    self.builder.build_store(store, val)
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
                }
            }
        }

        // Jump to exit block
        self.builder.build_unconditional_branch(loop_ctx.exit_block)
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

        Ok(())
    }

    /// Compile a continue statement.
    pub(super) fn compile_continue(&mut self, label: Option<hir::LoopId>) -> Result<(), Vec<Diagnostic>> {
        // Find the loop context
        let loop_ctx = if let Some(label) = label {
            self.loop_stack.iter().rev()
                .find(|ctx| ctx.label == Some(label))
                .cloned()
                .ok_or_else(|| vec![Diagnostic::error(
                    format!("Cannot find loop with label {:?}", label),
                    Span::dummy(),
                )])?
        } else {
            self.loop_stack.last()
                .cloned()
                .ok_or_else(|| vec![Diagnostic::error(
                    "Continue outside of loop",
                    Span::dummy(),
                )])?
        };

        // Jump to continue block
        self.builder.build_unconditional_branch(loop_ctx.continue_block)
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

        Ok(())
    }
}
