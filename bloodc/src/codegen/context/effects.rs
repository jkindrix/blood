//! Effects system codegen.
//!
//! This module handles compilation of:
//! - Perform expressions (effect operation invocations)
//! - Resume expressions (continuation resumption)
//! - Handle expressions (handler installation)
//!
//! Implementation follows:
//! - [Generalized Evidence Passing for Effect Handlers](https://dl.acm.org/doi/10.1145/3473576) (ICFP'21)
//! - [Zero-Overhead Lexical Effect Handlers](https://doi.org/10.1145/3763177) (OOPSLA'25)

use inkwell::values::BasicValueEnum;

use crate::hir::{self, Type, DefId};
use crate::diagnostics::Diagnostic;
use crate::span::Span;
use crate::ice_err;

use super::CodegenContext;

/// Analyze whether all resumes in a handler body are in tail position.
///
/// A resume is in tail position if it's the last expression evaluated before
/// returning from the handler operation. Tail-resumptive handlers can use a
/// simple return-based implementation, while non-tail-resumptive handlers
/// require continuation capture.
///
/// Note: This function is preserved for future handler optimization work.
/// Currently all handlers use a uniform compilation strategy.
#[allow(dead_code)]
pub fn is_handler_tail_resumptive(body: &hir::Body) -> bool {
    // Check if all resumes in the body are in tail position
    check_expr_tail_resumptive(&body.expr, true)
}

/// Check if all resumes in an expression are in tail position.
///
/// `in_tail_position` indicates whether the current expression is in tail position.
#[allow(dead_code)]
fn check_expr_tail_resumptive(expr: &hir::Expr, in_tail_position: bool) -> bool {
    use hir::ExprKind::*;

    match &expr.kind {
        // Resume in tail position is fine; resume not in tail position means non-tail-resumptive
        Resume { .. } => in_tail_position,

        // Block: only the final expression is in tail position
        Block { stmts, expr: final_expr } => {
            // Check statements - they are NOT in tail position
            for stmt in stmts {
                match stmt {
                    hir::Stmt::Expr(e) => {
                        if !check_expr_tail_resumptive(e, false) {
                            return false;
                        }
                    }
                    hir::Stmt::Let { init: Some(e), .. } => {
                        if !check_expr_tail_resumptive(e, false) {
                            return false;
                        }
                    }
                    _ => {}
                }
            }
            // Check final expression - it IS in tail position if block is
            if let Some(e) = final_expr {
                check_expr_tail_resumptive(e, in_tail_position)
            } else {
                true
            }
        }

        // If: both branches inherit tail position
        If { condition, then_branch, else_branch } => {
            // Condition is not in tail position
            if !check_expr_tail_resumptive(condition, false) {
                return false;
            }
            // Both branches inherit tail position
            if !check_expr_tail_resumptive(then_branch, in_tail_position) {
                return false;
            }
            if let Some(else_br) = else_branch {
                if !check_expr_tail_resumptive(else_br, in_tail_position) {
                    return false;
                }
            }
            true
        }

        // Match: all arms inherit tail position
        Match { scrutinee, arms } => {
            // Scrutinee is not in tail position
            if !check_expr_tail_resumptive(scrutinee, false) {
                return false;
            }
            // All arm bodies inherit tail position
            for arm in arms {
                if let Some(ref guard) = arm.guard {
                    if !check_expr_tail_resumptive(guard, false) {
                        return false;
                    }
                }
                if !check_expr_tail_resumptive(&arm.body, in_tail_position) {
                    return false;
                }
            }
            true
        }

        // Loop bodies are not in tail position (they return to the loop)
        Loop { body, .. } => check_expr_tail_resumptive(body, false),
        While { condition, body, .. } => {
            check_expr_tail_resumptive(condition, false) &&
            check_expr_tail_resumptive(body, false)
        }

        // Return: the value is in tail position (semantically)
        Return(val) => {
            if let Some(v) = val {
                check_expr_tail_resumptive(v, true)
            } else {
                true
            }
        }

        // Binary, unary, calls - operands are not in tail position
        Binary { left, right, .. } => {
            check_expr_tail_resumptive(left, false) &&
            check_expr_tail_resumptive(right, false)
        }
        Unary { operand, .. } => check_expr_tail_resumptive(operand, false),
        Call { callee, args } => {
            check_expr_tail_resumptive(callee, false) &&
            args.iter().all(|a| check_expr_tail_resumptive(a, false))
        }
        MethodCall { receiver, args, .. } => {
            check_expr_tail_resumptive(receiver, false) &&
            args.iter().all(|a| check_expr_tail_resumptive(a, false))
        }

        // Assignment - value is not in tail position
        Assign { target, value } => {
            check_expr_tail_resumptive(target, false) &&
            check_expr_tail_resumptive(value, false)
        }

        // Field, index access
        Field { base, .. } => check_expr_tail_resumptive(base, false),
        Index { base, index } => {
            check_expr_tail_resumptive(base, false) &&
            check_expr_tail_resumptive(index, false)
        }

        // Tuple, array, struct construction
        Tuple(elems) => elems.iter().all(|e| check_expr_tail_resumptive(e, false)),
        Array(elems) => elems.iter().all(|e| check_expr_tail_resumptive(e, false)),
        Repeat { value, .. } => check_expr_tail_resumptive(value, false),
        Struct { fields, base, .. } => {
            fields.iter().all(|f| check_expr_tail_resumptive(&f.value, false)) &&
            base.as_ref().map_or(true, |b| check_expr_tail_resumptive(b, false))
        }
        Variant { fields, .. } => fields.iter().all(|e| check_expr_tail_resumptive(e, false)),

        // Range
        Range { start, end, .. } => {
            start.as_ref().map_or(true, |s| check_expr_tail_resumptive(s, false)) &&
            end.as_ref().map_or(true, |e| check_expr_tail_resumptive(e, false))
        }

        // Cast
        Cast { expr, .. } => check_expr_tail_resumptive(expr, false),

        // Borrow, deref, addr_of
        Borrow { expr, .. } | AddrOf { expr, .. } => check_expr_tail_resumptive(expr, false),
        Deref(inner) => check_expr_tail_resumptive(inner, false),

        // Unsafe block: inner inherits tail position
        Unsafe(inner) => check_expr_tail_resumptive(inner, in_tail_position),

        // Let expression (let x = e in body) - init is not in tail position
        Let { init, .. } => check_expr_tail_resumptive(init, false),

        // Break with value - value is not in tail position
        Break { value, .. } => {
            value.as_ref().map_or(true, |v| check_expr_tail_resumptive(v, false))
        }

        // Perform - args are not in tail position
        Perform { args, .. } => args.iter().all(|a| check_expr_tail_resumptive(a, false)),

        // Handle - body and handler_instance are not in tail position
        Handle { body, handler_instance, .. } => {
            check_expr_tail_resumptive(body, false) &&
            check_expr_tail_resumptive(handler_instance, false)
        }

        // Closures capture their environment; body is analyzed separately
        Closure { .. } => true,

        // Leaf nodes - no resume inside
        Literal(_) | Local(_) | Def(_) | Continue { .. } | Error => true,
    }
}

impl<'ctx, 'a> CodegenContext<'ctx, 'a> {
    /// Create a continuation for a perform expression.
    ///
    /// This creates a continuation that represents "the rest of the computation"
    /// after the handler resumes. The continuation is called by the handler's
    /// `resume(value)` expression via `blood_continuation_resume`.
    ///
    /// For simple performs (where the perform result is used directly), this is
    /// an identity continuation that just returns its argument.
    pub(crate) fn create_perform_continuation(&mut self) -> Result<inkwell::values::IntValue<'ctx>, Vec<Diagnostic>> {
        let i64_type = self.context.i64_type();
        let i8_ptr_type = self.context.i8_type().ptr_type(inkwell::AddressSpace::default());

        // Get or create the identity continuation function
        // Signature: extern "C" fn(value: i64, context: *mut void) -> i64
        let identity_cont_fn = self.get_or_create_identity_continuation()?;

        // Get blood_continuation_create function
        let cont_create_fn = self.module.get_function("blood_continuation_create")
            .unwrap_or_else(|| {
                // Declare: fn(callback: fn(i64, *void) -> i64, context: *void) -> i64
                let callback_type = i64_type.fn_type(
                    &[i64_type.into(), i8_ptr_type.into()],
                    false,
                );
                let callback_ptr_type = callback_type.ptr_type(inkwell::AddressSpace::default());
                let fn_type = i64_type.fn_type(
                    &[callback_ptr_type.into(), i8_ptr_type.into()],
                    false,
                );
                self.module.add_function("blood_continuation_create", fn_type, None)
            });

        // Call blood_continuation_create(identity_cont_fn, null)
        let callback_ptr = identity_cont_fn.as_global_value().as_pointer_value();
        let null_context = i8_ptr_type.const_null();

        let call_result = self.builder
            .build_call(
                cont_create_fn,
                &[callback_ptr.into(), null_context.into()],
                "continuation",
            )
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error creating continuation: {}", e), Span::dummy())])?;

        let cont_handle = call_result.try_as_basic_value().left()
            .ok_or_else(|| vec![Diagnostic::error(
                "blood_continuation_create returned void".to_string(),
                Span::dummy(),
            )])?
            .into_int_value();

        Ok(cont_handle)
    }

    /// Get or create the identity continuation function.
    ///
    /// This function simply returns its first argument (the resume value).
    /// It's used for simple performs where no additional computation is needed
    /// after the handler resumes.
    pub(crate) fn get_or_create_identity_continuation(&mut self) -> Result<inkwell::values::FunctionValue<'ctx>, Vec<Diagnostic>> {
        let fn_name = "__blood_identity_continuation";

        // Check if already created
        if let Some(fn_val) = self.module.get_function(fn_name) {
            return Ok(fn_val);
        }

        let i64_type = self.context.i64_type();
        let i8_ptr_type = self.context.i8_type().ptr_type(inkwell::AddressSpace::default());

        // Create the function: extern "C" fn(value: i64, context: *mut void) -> i64
        let fn_type = i64_type.fn_type(&[i64_type.into(), i8_ptr_type.into()], false);
        let fn_val = self.module.add_function(fn_name, fn_type, None);

        // Mark as internal linkage (not exported)
        fn_val.set_linkage(inkwell::module::Linkage::Internal);

        // Create entry block
        let entry = self.context.append_basic_block(fn_val, "entry");

        // Save current builder position
        let saved_block = self.builder.get_insert_block();

        // Build the function body
        self.builder.position_at_end(entry);

        // Get the value parameter (first argument) and return it
        let value_param = fn_val.get_nth_param(0)
            .ok_or_else(|| vec![Diagnostic::error(
                "Identity continuation missing value parameter".to_string(),
                Span::dummy(),
            )])?
            .into_int_value();

        self.builder.build_return(Some(&value_param))
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

        // Restore builder position
        if let Some(block) = saved_block {
            self.builder.position_at_end(block);
        }

        Ok(fn_val)
    }

    /// Compile a perform expression: `perform Effect.op(args)`
    ///
    /// In the evidence passing model (ICFP'21), this becomes a call through
    /// the evidence vector via `blood_perform` runtime function.
    pub(super) fn compile_perform(
        &mut self,
        effect_id: DefId,
        op_index: u32,
        args: &[hir::Expr],
        result_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        let i64_type = self.context.i64_type();
        let i32_type = self.context.i32_type();

        // Get blood_perform runtime function
        let perform_fn = self.module.get_function("blood_perform")
            .ok_or_else(|| vec![Diagnostic::error(
                "Runtime function blood_perform not found".to_string(),
                Span::dummy(),
            )])?;

        // Compile arguments and pack into an i64 array
        let mut compiled_args = Vec::with_capacity(args.len());
        for arg in args {
            if let Some(val) = self.compile_expr(arg)? {
                // Convert to i64 for uniform argument passing
                let i64_val = match val {
                    BasicValueEnum::IntValue(iv) => {
                        if iv.get_type().get_bit_width() == 64 {
                            iv
                        } else {
                            self.builder
                                .build_int_s_extend(iv, i64_type, "arg_ext")
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
                        }
                    }
                    BasicValueEnum::FloatValue(fv) => {
                        // Bitcast float to i64 for passing through uniform interface
                        self.builder
                            .build_bit_cast(fv, i64_type, "float_as_i64")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
                            .into_int_value()
                    }
                    BasicValueEnum::PointerValue(pv) => {
                        self.builder
                            .build_ptr_to_int(pv, i64_type, "ptr_as_i64")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
                    }
                    other => {
                        return Err(vec![ice_err!(
                            arg.span,
                            "unsupported argument type in perform expression";
                            "type" => other.get_type(),
                            "expected" => "IntValue, FloatValue, or PointerValue"
                        )]);
                    }
                };
                compiled_args.push(i64_val);
            }
        }

        // Allocate stack space for arguments array
        let arg_count = compiled_args.len();
        let args_array = if arg_count > 0 {
            let array_type = i64_type.array_type(arg_count as u32);
            let args_alloca = self.builder
                .build_alloca(array_type, "perform_args")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

            // Store each argument in the array
            let zero = i32_type.const_zero();
            for (i, arg_val) in compiled_args.iter().enumerate() {
                let idx = i32_type.const_int(i as u64, false);
                let gep = unsafe {
                    self.builder.build_gep(
                        args_alloca,
                        &[zero, idx],
                        &format!("arg_ptr_{}", i),
                    )
                }.map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

                self.builder
                    .build_store(gep, *arg_val)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
            }

            // Get pointer to first element
            unsafe {
                self.builder.build_gep(
                    args_alloca,
                    &[zero, zero],
                    "args_ptr",
                )
            }.map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
        } else {
            // No arguments - pass null pointer
            i64_type.ptr_type(inkwell::AddressSpace::default()).const_null()
        };

        // Call blood_perform(effect_id, op_index, args, arg_count, continuation)
        let effect_id_val = i64_type.const_int(effect_id.index as u64, false);
        let op_index_val = i32_type.const_int(op_index as u64, false);
        let arg_count_val = i64_type.const_int(arg_count as u64, false);

        // Create continuation for non-tail-resumptive handlers
        // The continuation represents "what to do after the handler resumes us"
        // For simple cases (perform as final expression), this is just identity
        let continuation_val = self.create_perform_continuation()?;

        let call_result = self.builder
            .build_call(
                perform_fn,
                &[
                    effect_id_val.into(),
                    op_index_val.into(),
                    args_array.into(),
                    arg_count_val.into(),
                    continuation_val.into(),
                ],
                "perform_result",
            )
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

        // Get the result value and convert to appropriate type
        if result_ty.is_unit() {
            Ok(None)
        } else {
            let result_i64 = call_result.try_as_basic_value().left()
                .ok_or_else(|| vec![Diagnostic::error(
                    "blood_perform returned void unexpectedly".to_string(),
                    Span::dummy(),
                )])?;

            // Convert result from i64 to expected type
            let result_llvm_type = self.lower_type(result_ty);
            let converted_result = if result_llvm_type.is_int_type() {
                let result_int_type = result_llvm_type.into_int_type();
                if result_int_type.get_bit_width() == 64 {
                    result_i64
                } else {
                    self.builder
                        .build_int_truncate(result_i64.into_int_value(), result_int_type, "result_trunc")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
                        .into()
                }
            } else if result_llvm_type.is_float_type() {
                self.builder
                    .build_bit_cast(result_i64, result_llvm_type, "result_float")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
            } else if result_llvm_type.is_pointer_type() {
                self.builder
                    .build_int_to_ptr(result_i64.into_int_value(), result_llvm_type.into_pointer_type(), "result_ptr")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
                    .into()
            } else {
                result_i64
            };

            Ok(Some(converted_result))
        }
    }

    /// Compile a resume expression: `resume(value)`
    ///
    /// For tail-resumptive handlers (shallow handlers, or deep handlers in tail position),
    /// resume is a simple return.
    ///
    /// For non-tail-resumptive handlers (deep handlers), resume calls the continuation
    /// and returns its result.
    pub(super) fn compile_resume(
        &mut self,
        value: Option<&hir::Expr>,
        result_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        let i64_type = self.context.i64_type();
        let expected_llvm_ty = self.lower_type(result_ty);

        // Compile the value to resume with
        let resume_value = if let Some(val_expr) = value {
            let val = self.compile_expr(val_expr)?;
            if let Some(ret_val) = val {
                // Convert to i64 for uniform interface
                match ret_val {
                    BasicValueEnum::IntValue(iv) => {
                        if iv.get_type().get_bit_width() == 64 {
                            iv
                        } else {
                            self.builder
                                .build_int_s_extend(iv, i64_type, "resume_ext")
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
                        }
                    }
                    BasicValueEnum::PointerValue(pv) => {
                        self.builder
                            .build_ptr_to_int(pv, i64_type, "resume_ptr_int")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
                    }
                    BasicValueEnum::StructValue(_sv) => {
                        // For unit type (empty struct), use 0
                        i64_type.const_zero()
                    }
                    BasicValueEnum::FloatValue(fv) => {
                        self.builder
                            .build_bit_cast(fv, i64_type, "resume_float")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
                            .into_int_value()
                    }
                    _ => i64_type.const_zero(),
                }
            } else {
                i64_type.const_zero()
            }
        } else {
            i64_type.const_zero()
        };

        // Check if we have a continuation (deep handler)
        if let Some(cont_ptr) = self.current_continuation {
            // Load the continuation handle
            let cont_handle = self.builder
                .build_load(cont_ptr, "cont_handle")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
                .into_int_value();

            // Check if continuation is 0 (tail-resumptive) or non-zero (call continuation)
            let zero = i64_type.const_zero();
            let is_tail_resumptive = self.builder
                .build_int_compare(inkwell::IntPredicate::EQ, cont_handle, zero, "is_tail")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

            // Create blocks for branching
            let fn_value = self.current_fn.ok_or_else(|| vec![Diagnostic::error(
                "resume called outside of function".to_string(), Span::dummy()
            )])?;
            let tail_block = self.context.append_basic_block(fn_value, "resume_tail");
            let cont_block = self.context.append_basic_block(fn_value, "resume_cont");
            let merge_block = self.context.append_basic_block(fn_value, "resume_merge");

            self.builder.build_conditional_branch(is_tail_resumptive, tail_block, cont_block)
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

            // Tail-resumptive path: just return the value
            self.builder.position_at_end(tail_block);
            self.builder.build_return(Some(&resume_value))
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

            // Continuation path: call blood_continuation_resume
            self.builder.position_at_end(cont_block);
            let cont_resume_fn = self.module.get_function("blood_continuation_resume")
                .unwrap_or_else(|| {
                    // Declare blood_continuation_resume: (handle: i64, value: i64) -> i64
                    let fn_type = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
                    self.module.add_function("blood_continuation_resume", fn_type, None)
                });

            // Call blood_continuation_resume(cont_handle, resume_value)
            let call_result = self.builder
                .build_call(cont_resume_fn, &[cont_handle.into(), resume_value.into()], "cont_result")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

            let cont_result = call_result.try_as_basic_value().left()
                .ok_or_else(|| vec![Diagnostic::error(
                    "blood_continuation_resume returned void".to_string(),
                    Span::dummy(),
                )])?;

            // Branch to merge block (continuation path doesn't return, it continues)
            self.builder.build_unconditional_branch(merge_block)
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

            // Position at merge block for code after resume
            self.builder.position_at_end(merge_block);

            // Create phi node to merge the continuation result
            // Note: tail path doesn't reach here (it returns), so only cont_result matters
            let phi = self.builder
                .build_phi(i64_type, "resume_result")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
            phi.add_incoming(&[(&cont_result, cont_block)]);

            // Convert i64 result to expected type
            let phi_value = phi.as_basic_value().into_int_value();
            let converted_result = if expected_llvm_ty.is_int_type() {
                let target_int_type = expected_llvm_ty.into_int_type();
                if target_int_type.get_bit_width() == 64 {
                    phi_value.into()
                } else {
                    self.builder
                        .build_int_truncate(phi_value, target_int_type, "resume_trunc")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
                        .into()
                }
            } else if expected_llvm_ty.is_float_type() {
                self.builder
                    .build_bit_cast(phi_value, expected_llvm_ty, "resume_float")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
            } else if expected_llvm_ty.is_pointer_type() {
                self.builder
                    .build_int_to_ptr(phi_value, expected_llvm_ty.into_pointer_type(), "resume_ptr")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
                    .into()
            } else {
                // Default: return as-is (may need more cases)
                phi_value.into()
            };

            Ok(Some(converted_result))
        } else {
            // Shallow handler or no continuation context: just return the value
            let fn_value = self.current_fn.ok_or_else(|| vec![Diagnostic::error(
                "resume called outside of function".to_string(), Span::dummy()
            )])?;
            let fn_ret_ty = fn_value.get_type().get_return_type();

            if fn_ret_ty == Some(i64_type.into()) {
                self.builder.build_return(Some(&resume_value))
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
            } else {
                self.builder.build_return(None)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
            }

            // For tail-resumptive, resume doesn't produce a usable value (control transfers)
            Ok(None)
        }
    }

    /// Compile a handle expression: `handle { body } with { handler }`
    ///
    /// This sets up the evidence vector and runs the body with the handler
    /// installed.
    pub(super) fn compile_handle(
        &mut self,
        body: &hir::Expr,
        handler_id: DefId,
        handler_instance: &hir::Expr,
        result_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        // Phase 2.4: Evidence vector setup
        //
        // 1. Save current evidence vector
        // 2. Create new evidence vector
        // 3. Compile handler instance to get state pointer
        // 4. Push handler onto evidence vector with state
        // 5. Set as current evidence vector
        // 6. Compile body
        // 7. Pop handler from evidence vector
        // 8. Restore previous evidence vector
        // 9. Return result

        let i64_type = self.context.i64_type();
        let i8_ptr_type = self.context.i8_type().ptr_type(inkwell::AddressSpace::default());

        // Get runtime functions
        let ev_create = self.module.get_function("blood_evidence_create")
            .ok_or_else(|| vec![Diagnostic::error(
                "Runtime function blood_evidence_create not found".to_string(),
                Span::dummy(),
            )])?;
        let ev_push_with_state = self.module.get_function("blood_evidence_push_with_state")
            .unwrap_or_else(|| {
                let fn_type = self.context.void_type().fn_type(
                    &[i8_ptr_type.into(), i64_type.into(), i8_ptr_type.into()],
                    false
                );
                self.module.add_function("blood_evidence_push_with_state", fn_type, None)
            });
        let ev_set_current = self.module.get_function("blood_evidence_set_current")
            .unwrap_or_else(|| {
                let fn_type = self.context.void_type().fn_type(&[i8_ptr_type.into()], false);
                self.module.add_function("blood_evidence_set_current", fn_type, None)
            });
        let ev_current = self.module.get_function("blood_evidence_current")
            .unwrap_or_else(|| {
                let fn_type = i8_ptr_type.fn_type(&[], false);
                self.module.add_function("blood_evidence_current", fn_type, None)
            });
        let ev_pop = self.module.get_function("blood_evidence_pop")
            .ok_or_else(|| vec![Diagnostic::error(
                "Runtime function blood_evidence_pop not found".to_string(),
                Span::dummy(),
            )])?;
        let ev_destroy = self.module.get_function("blood_evidence_destroy")
            .ok_or_else(|| vec![Diagnostic::error(
                "Runtime function blood_evidence_destroy not found".to_string(),
                Span::dummy(),
            )])?;

        // Save current evidence vector
        let saved_ev = self.builder.build_call(ev_current, &[], "saved_evidence")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
            .try_as_basic_value()
            .left()
            .ok_or_else(|| vec![Diagnostic::error(
                "blood_evidence_current returned void".to_string(),
                Span::dummy(),
            )])?;

        // Create new evidence vector
        let ev = self.builder.build_call(ev_create, &[], "evidence")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
            .try_as_basic_value()
            .left()
            .ok_or_else(|| vec![Diagnostic::error(
                "blood_evidence_create returned void".to_string(),
                Span::dummy(),
            )])?;

        // Compile handler instance to get state pointer
        let state_ptr = if let Some(state_val) = self.compile_expr(handler_instance)? {
            // Cast to void* for runtime
            match state_val {
                BasicValueEnum::PointerValue(pv) => {
                    self.builder.build_pointer_cast(pv, i8_ptr_type, "state_void_ptr")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
                        .into()
                }
                _ => i8_ptr_type.const_null().into()
            }
        } else {
            i8_ptr_type.const_null().into()
        };

        // Push handler with effect_id and state pointer
        // Note: handler_id is the handler DefId, but we need the effect_id for the runtime lookup
        // For now, use handler_id.index as the effect_id since handlers are 1:1 with effects in simple cases
        let effect_id_val = i64_type.const_int(handler_id.index as u64, false);
        self.builder.build_call(
            ev_push_with_state,
            &[ev.into(), effect_id_val.into(), state_ptr],
            ""
        ).map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

        // Set as current evidence vector
        self.builder.build_call(ev_set_current, &[ev.into()], "")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

        // Compile the body
        let result = self.compile_expr(body)?;

        // Pop handler from evidence vector
        self.builder.build_call(ev_pop, &[ev.into()], "")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

        // Restore previous evidence vector (or null)
        self.builder.build_call(ev_set_current, &[saved_ev.into()], "")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

        // Destroy evidence vector
        self.builder.build_call(ev_destroy, &[ev.into()], "")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

        // Return result with proper type
        if result_ty.is_unit() {
            Ok(None)
        } else {
            Ok(result)
        }
    }
}
