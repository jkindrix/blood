//! Expression compilation for codegen.
//!
//! This module handles compilation of all HIR expression kinds to LLVM IR.

use inkwell::values::{BasicValueEnum, BasicMetadataValueEnum};
use inkwell::IntPredicate;
use inkwell::FloatPredicate;
use inkwell::AddressSpace;
use inkwell::types::{BasicTypeEnum, BasicType, BasicMetadataTypeEnum};

use crate::hir::{self, DefId, LocalId, Type, TypeKind, PrimitiveTy};
use crate::diagnostics::Diagnostic;
use crate::span::Span;

use super::CodegenContext;

impl<'ctx, 'a> CodegenContext<'ctx, 'a> {
    /// Compile an expression.
    pub fn compile_expr(&mut self, expr: &hir::Expr) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        use hir::ExprKind::*;

        match &expr.kind {
            Literal(lit) => self.compile_literal_with_type(lit, &expr.ty).map(Some),
            Local(local_id) => self.compile_local_load(*local_id).map(Some),
            Binary { op, left, right } => self.compile_binary(op, left, right).map(Some),
            Unary { op, operand } => self.compile_unary(op, operand).map(Some),
            Call { callee, args } => self.compile_call(callee, args),
            MethodCall { receiver, method, args } => {
                // Method call: receiver.method(args)
                // Compiled as method(receiver, args) with potential dynamic dispatch
                self.compile_method_call(receiver, *method, args, &expr.ty)
            }
            Block { stmts, expr: tail_expr } => self.compile_block(stmts, tail_expr.as_deref()),
            If { condition, then_branch, else_branch } => {
                self.compile_if(condition, then_branch, else_branch.as_deref(), &expr.ty)
            }
            While { condition, body, .. } => {
                self.compile_while(condition, body)?;
                Ok(None)
            }
            Return(value) => {
                self.compile_return(value.as_deref())?;
                Ok(None)
            }
            Assign { target, value } => {
                self.compile_assign(target, value)?;
                Ok(None)
            }
            Tuple(exprs) => {
                // Empty tuple is unit type, return None
                if exprs.is_empty() {
                    return Ok(None);
                }
                // For non-empty tuples, compile all expressions and build a struct
                let values: Vec<_> = exprs.iter()
                    .filter_map(|e| self.compile_expr(e).ok().flatten())
                    .collect();
                if values.is_empty() {
                    Ok(None)
                } else if values.len() == 1 {
                    // Safe: we just verified len == 1, so pop() returns Some
                    Ok(values.into_iter().next())
                } else {
                    // Build a struct for the tuple
                    let types: Vec<BasicTypeEnum> = values.iter()
                        .map(|v| v.get_type())
                        .collect();
                    let struct_type = self.context.struct_type(&types, false);
                    let mut struct_val = struct_type.get_undef();
                    for (i, val) in values.into_iter().enumerate() {
                        struct_val = self.builder
                            .build_insert_value(struct_val, val, i as u32, "tuple")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?
                            .into_struct_value();
                    }
                    Ok(Some(struct_val.into()))
                }
            }
            Def(def_id) => {
                // Reference to a definition - could be function, const, or static
                if let Some(fn_val) = self.functions.get(def_id) {
                    // Function reference - return the function pointer
                    Ok(Some(fn_val.as_global_value().as_pointer_value().into()))
                } else if let Some(global) = self.const_globals.get(def_id) {
                    // Const reference - load the value
                    let pointee_ty: BasicTypeEnum = global.get_value_type().try_into()
                        .map_err(|_| vec![Diagnostic::error("Cannot determine const global pointee type", self.current_span())])?;
                    let val = self.builder
                        .build_load(pointee_ty, global.as_pointer_value(), "const_load")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                    Ok(Some(val))
                } else if let Some(global) = self.static_globals.get(def_id) {
                    // Static reference - load the value
                    let pointee_ty: BasicTypeEnum = global.get_value_type().try_into()
                        .map_err(|_| vec![Diagnostic::error("Cannot determine static global pointee type", self.current_span())])?;
                    let val = self.builder
                        .build_load(pointee_ty, global.as_pointer_value(), "static_load")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                    Ok(Some(val))
                } else {
                    // Unknown definition - might not be compiled yet, return None for now
                    Ok(None)
                }
            }
            Struct { def_id: _, fields, base } => {
                // Compile struct construction (with optional base for struct update syntax)
                self.compile_struct_expr(&expr.ty, fields, base.as_deref())
            }
            Variant { def_id, variant_idx, fields } => {
                // Compile enum variant construction
                self.compile_variant(*def_id, *variant_idx, fields, &expr.ty)
            }
            Field { base, field_idx } => {
                // Compile field access
                self.compile_field_access(base, *field_idx)
            }
            Match { scrutinee, arms } => {
                // Compile match expression
                self.compile_match(scrutinee, arms, &expr.ty)
            }
            Loop { body, label } => {
                // Compile infinite loop
                self.compile_loop(body, *label, &expr.ty)
            }
            Break { label, value } => {
                // Compile break statement
                self.compile_break(*label, value.as_deref())?;
                Ok(None)
            }
            Continue { label } => {
                // Compile continue statement
                self.compile_continue(*label)?;
                Ok(None)
            }
            Array(elements) => {
                // Compile array literal
                self.compile_array(elements, &expr.ty)
            }
            Index { base, index } => {
                // Compile array/slice indexing
                self.compile_index(base, index)
            }
            Cast { expr: inner, target_ty } => {
                // Compile type cast
                self.compile_cast(inner, target_ty)
            }
            Perform { effect_id, op_index, args, type_args: _ } => {
                // Effect operation: perform Effect.op(args)
                // After evidence translation, this calls through the evidence vector.
                // For now, we emit a placeholder that will be filled in during
                // full effects system integration (Phase 2.4).
                self.compile_perform(*effect_id, *op_index, args, &expr.ty)
            }
            Resume { value } => {
                // Resume continuation in handler.
                // For tail-resumptive handlers, this is just a return.
                // For general handlers, this requires continuation capture (Phase 2.3).
                self.compile_resume(value.as_deref(), &expr.ty)
            }
            Handle { body, handler_id, handler_instance } => {
                // Handle expression: runs body with handler installed.
                // This sets up the evidence vector and runs the body.
                if std::env::var("BLOOD_DEBUG_EFFECTS").is_ok() {
                    eprintln!("DEBUG compile_expr: Handle expression, handler_id={:?}", handler_id);
                }
                self.compile_handle(body, *handler_id, handler_instance, &expr.ty)
            }
            Deref(inner) => {
                // Dereference: *x
                // For generational references, this should check the generation
                // before accessing the pointed-to value.
                self.compile_deref(inner, expr.span)
            }
            Borrow { expr: inner, mutable: _ } => {
                // Borrow: &x or &mut x
                // Creates a reference to the given expression.
                // For generational references, this captures the current generation.
                self.compile_borrow(inner)
            }
            AddrOf { expr: inner, mutable: _ } => {
                // Address-of: raw pointer creation
                // Creates a raw pointer without generation tracking.
                self.compile_addr_of(inner)
            }
            Closure { body_id, captures } => {
                // Closure: |x| x + 1
                // Creates a function for the body and a struct for captures.
                self.compile_closure(*body_id, captures, &expr.ty, expr.span)
            }
            Unsafe(inner) => {
                // Unsafe block: @unsafe { ... }
                // Compiles the inner expression without generation checks.
                // The type system ensures that only appropriate operations
                // are allowed in unsafe blocks (e.g., raw pointer dereference,
                // FFI calls).
                self.compile_expr(inner)
            }
            Region { stmts, expr: tail_expr, .. } => {
                // Region blocks are handled by the MIR path. If reached here
                // (non-MIR codegen), compile as a plain block.
                self.compile_block(stmts, tail_expr.as_deref())
            }
            _ => {
                self.errors.push(Diagnostic::error(
                    format!("Unsupported expression kind: {:?}", std::mem::discriminant(&expr.kind)),
                    expr.span,
                ));
                Ok(None)
            }
        }
    }

    /// Compile a literal with type information.
    pub(super) fn compile_literal_with_type(&self, lit: &hir::LiteralValue, ty: &hir::Type) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        match lit {
            hir::LiteralValue::Int(val) => {
                // Use the actual type to determine the LLVM integer type
                let int_type = match ty.kind() {
                    hir::TypeKind::Primitive(hir::PrimitiveTy::Int(int_ty)) => {
                        use hir::def::IntTy;
                        match int_ty {
                            IntTy::I8 => self.context.i8_type(),
                            IntTy::I16 => self.context.i16_type(),
                            IntTy::I32 => self.context.i32_type(),
                            IntTy::I64 => self.context.i64_type(),
                            IntTy::I128 => self.context.i128_type(),
                            IntTy::Isize => self.context.i64_type(),
                        }
                    }
                    hir::TypeKind::Primitive(hir::PrimitiveTy::Uint(uint_ty)) => {
                        use hir::def::UintTy;
                        match uint_ty {
                            UintTy::U8 => self.context.i8_type(),
                            UintTy::U16 => self.context.i16_type(),
                            UintTy::U32 => self.context.i32_type(),
                            UintTy::U64 => self.context.i64_type(),
                            UintTy::U128 => self.context.i128_type(),
                            UintTy::Usize => self.context.i64_type(),
                        }
                    }
                    // Default to i32 if type is unresolved or inferred
                    _ => self.context.i32_type(),
                };
                Ok(int_type.const_int(*val as u64, true).into())
            }
            hir::LiteralValue::Uint(val) => {
                // Use the actual type to determine the LLVM integer type
                let int_type = match ty.kind() {
                    hir::TypeKind::Primitive(hir::PrimitiveTy::Uint(uint_ty)) => {
                        use hir::def::UintTy;
                        match uint_ty {
                            UintTy::U8 => self.context.i8_type(),
                            UintTy::U16 => self.context.i16_type(),
                            UintTy::U32 => self.context.i32_type(),
                            UintTy::U64 => self.context.i64_type(),
                            UintTy::U128 => self.context.i128_type(),
                            UintTy::Usize => self.context.i64_type(),
                        }
                    }
                    hir::TypeKind::Primitive(hir::PrimitiveTy::Int(int_ty)) => {
                        use hir::def::IntTy;
                        match int_ty {
                            IntTy::I8 => self.context.i8_type(),
                            IntTy::I16 => self.context.i16_type(),
                            IntTy::I32 => self.context.i32_type(),
                            IntTy::I64 => self.context.i64_type(),
                            IntTy::I128 => self.context.i128_type(),
                            IntTy::Isize => self.context.i64_type(),
                        }
                    }
                    // Default to i32 if type is unresolved or inferred
                    _ => self.context.i32_type(),
                };
                Ok(int_type.const_int(*val as u64, false).into())
            }
            hir::LiteralValue::Float(val) => {
                // Check if it's f32 or f64
                let is_f32 = matches!(
                    ty.kind(),
                    hir::TypeKind::Primitive(hir::PrimitiveTy::Float(hir::def::FloatTy::F32))
                );
                if is_f32 {
                    Ok(self.context.f32_type().const_float(*val).into())
                } else {
                    Ok(self.context.f64_type().const_float(*val).into())
                }
            }
            hir::LiteralValue::Bool(val) => {
                Ok(self.context.bool_type().const_int(*val as u64, false).into())
            }
            hir::LiteralValue::Char(val) => {
                Ok(self.context.i32_type().const_int(*val as u64, false).into())
            }
            hir::LiteralValue::String(s) => {
                // Create global string constant and str slice {ptr, len}
                let global = self.builder.build_global_string_ptr(s, "str")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                let ptr = global.as_pointer_value();
                let len = self.context.i64_type().const_int(s.len() as u64, false);

                // Create str slice struct {ptr, len}
                let str_type = self.context.struct_type(
                    &[self.context.ptr_type(inkwell::AddressSpace::default()).into(),
                      self.context.i64_type().into()],
                    false
                );
                let str_val = str_type.const_named_struct(&[ptr.into(), len.into()]);
                Ok(str_val.into())
            }
            hir::LiteralValue::ByteString(bytes) => {
                // Create global byte array constant and byte slice {ptr, len}
                let byte_values: Vec<_> = bytes.iter()
                    .map(|&b| self.context.i8_type().const_int(b as u64, false))
                    .collect();
                let array_type = self.context.i8_type().array_type(bytes.len() as u32);
                let const_array = self.context.i8_type().const_array(&byte_values);

                let global = self.module.add_global(array_type, None, "bytes");
                global.set_initializer(&const_array);
                global.set_constant(true);

                // Cast array pointer to ptr
                let ptr = self.builder.build_pointer_cast(
                    global.as_pointer_value(),
                    self.context.ptr_type(inkwell::AddressSpace::default()),
                    "bytes_ptr"
                ).map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                let len = self.context.i64_type().const_int(bytes.len() as u64, false);

                // Create byte slice struct {ptr, len}
                let slice_type = self.context.struct_type(
                    &[self.context.ptr_type(inkwell::AddressSpace::default()).into(),
                      self.context.i64_type().into()],
                    false
                );
                let slice_val = slice_type.const_named_struct(&[ptr.into(), len.into()]);
                Ok(slice_val.into())
            }
        }
    }

    /// Compile a literal (defaults to f64 for floats).
    /// Use `compile_literal_with_type` when type information is available.
    pub(super) fn compile_literal(&self, lit: &hir::LiteralValue) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        // Default to f64 type for floats when type is not specified
        self.compile_literal_with_type(lit, &hir::Type::f64())
    }

    /// Load a local variable.
    pub(super) fn compile_local_load(&self, local_id: LocalId) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        let alloca = self.locals.get(&local_id)
            .ok_or_else(|| vec![Diagnostic::error(
                format!("Local variable {:?} not found", local_id),
                self.current_span(),
            )])?;

        // Get the pointee type: prefer explicit local_types map, fall back to alloca inspection
        let pointee_ty = self.local_types.get(&local_id).copied()
            .or_else(|| alloca.as_instruction()
                .and_then(|inst| inst.get_allocated_type().ok()))
            .ok_or_else(|| vec![Diagnostic::error(
                format!("Cannot determine type of local {:?}", local_id),
                self.current_span(),
            )])?;

        self.builder.build_load(pointee_ty, *alloca, "load")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])
    }

    /// Compile a binary operation.
    pub(super) fn compile_binary(
        &mut self,
        op: &crate::ast::BinOp,
        left: &hir::Expr,
        right: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        use crate::ast::BinOp::*;

        // Special handling for pipe operator: a |> f === f(a)
        // The right operand should be a function, and left is the argument
        if matches!(op, Pipe) {
            return self.compile_pipe(left, right);
        }

        // Short-circuit evaluation for logical && and ||.
        if matches!(op, And | Or) {
            return self.compile_short_circuit(op, left, right);
        }

        let lhs = self.compile_expr(left)?
            .ok_or_else(|| vec![Diagnostic::error("Expected value for binary op", left.span)])?;
        let rhs = self.compile_expr(right)?
            .ok_or_else(|| vec![Diagnostic::error("Expected value for binary op", right.span)])?;

        // Check if operands are floats
        let is_float = matches!(left.ty.kind(), TypeKind::Primitive(PrimitiveTy::Float(_)));

        // Check if operands are unsigned integers
        let is_unsigned = matches!(left.ty.kind(), TypeKind::Primitive(PrimitiveTy::Uint(_)));

        if is_float {
            // Float operations
            let lhs_float = lhs.into_float_value();
            let rhs_float = rhs.into_float_value();

            // Handle arithmetic operations (return FloatValue)
            match op {
                Add => {
                    return self.builder.build_float_add(lhs_float, rhs_float, "fadd")
                        .map(|v| v.into())
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())]);
                }
                Sub => {
                    return self.builder.build_float_sub(lhs_float, rhs_float, "fsub")
                        .map(|v| v.into())
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())]);
                }
                Mul => {
                    return self.builder.build_float_mul(lhs_float, rhs_float, "fmul")
                        .map(|v| v.into())
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())]);
                }
                Div => {
                    return self.builder.build_float_div(lhs_float, rhs_float, "fdiv")
                        .map(|v| v.into())
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())]);
                }
                Rem => {
                    return self.builder.build_float_rem(lhs_float, rhs_float, "frem")
                        .map(|v| v.into())
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())]);
                }
                // Comparison and bitwise ops are handled in the second match below
                // And/Or are handled by short-circuit before reaching here
                Eq | Ne | Lt | Le | Gt | Ge | And | Or | BitAnd | BitOr | BitXor | Shl | Shr | Pipe => {
                    // Fall through to comparison/error handling below
                }
            }

            // Handle comparison operations (return IntValue - i1)
            let result = match op {
                Eq => self.builder.build_float_compare(FloatPredicate::OEQ, lhs_float, rhs_float, "feq"),
                Ne => self.builder.build_float_compare(FloatPredicate::ONE, lhs_float, rhs_float, "fne"),
                Lt => self.builder.build_float_compare(FloatPredicate::OLT, lhs_float, rhs_float, "flt"),
                Le => self.builder.build_float_compare(FloatPredicate::OLE, lhs_float, rhs_float, "fle"),
                Gt => self.builder.build_float_compare(FloatPredicate::OGT, lhs_float, rhs_float, "fgt"),
                Ge => self.builder.build_float_compare(FloatPredicate::OGE, lhs_float, rhs_float, "fge"),
                // Bitwise and logical ops don't make sense for floats
                And | Or | BitAnd | BitOr | BitXor | Shl | Shr => {
                    return Err(vec![Diagnostic::error(
                        "Bitwise/logical operations not supported for float types",
                        left.span,
                    )]);
                }
                // Pipe is handled specially at the start of compile_binary
                Pipe => unreachable!("Pipe operator handled before operand compilation"),
                // Arithmetic ops already handled above in the float branch
                Add | Sub | Mul | Div | Rem => unreachable!("arithmetic ops should be handled in float branch above"),
            };

            result
                .map(|v| v.into())
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])
        } else {
            // Integer operations
            let lhs_int = lhs.into_int_value();
            let rhs_int = rhs.into_int_value();

            let result = match op {
                Add => self.builder.build_int_add(lhs_int, rhs_int, "add"),
                Sub => self.builder.build_int_sub(lhs_int, rhs_int, "sub"),
                Mul => self.builder.build_int_mul(lhs_int, rhs_int, "mul"),
                Div => {
                    if is_unsigned {
                        self.builder.build_int_unsigned_div(lhs_int, rhs_int, "udiv")
                    } else {
                        self.builder.build_int_signed_div(lhs_int, rhs_int, "sdiv")
                    }
                }
                Rem => {
                    if is_unsigned {
                        self.builder.build_int_unsigned_rem(lhs_int, rhs_int, "urem")
                    } else {
                        self.builder.build_int_signed_rem(lhs_int, rhs_int, "srem")
                    }
                }
                Eq => self.builder.build_int_compare(IntPredicate::EQ, lhs_int, rhs_int, "eq"),
                Ne => self.builder.build_int_compare(IntPredicate::NE, lhs_int, rhs_int, "ne"),
                Lt => {
                    if is_unsigned {
                        self.builder.build_int_compare(IntPredicate::ULT, lhs_int, rhs_int, "ult")
                    } else {
                        self.builder.build_int_compare(IntPredicate::SLT, lhs_int, rhs_int, "slt")
                    }
                }
                Le => {
                    if is_unsigned {
                        self.builder.build_int_compare(IntPredicate::ULE, lhs_int, rhs_int, "ule")
                    } else {
                        self.builder.build_int_compare(IntPredicate::SLE, lhs_int, rhs_int, "sle")
                    }
                }
                Gt => {
                    if is_unsigned {
                        self.builder.build_int_compare(IntPredicate::UGT, lhs_int, rhs_int, "ugt")
                    } else {
                        self.builder.build_int_compare(IntPredicate::SGT, lhs_int, rhs_int, "sgt")
                    }
                }
                Ge => {
                    if is_unsigned {
                        self.builder.build_int_compare(IntPredicate::UGE, lhs_int, rhs_int, "uge")
                    } else {
                        self.builder.build_int_compare(IntPredicate::SGE, lhs_int, rhs_int, "sge")
                    }
                }
                // And/Or handled by compile_short_circuit before operand evaluation
                And | Or => unreachable!("Logical &&/|| handled by short-circuit before operand eval"),
                BitAnd => self.builder.build_and(lhs_int, rhs_int, "bitand"),
                BitOr => self.builder.build_or(lhs_int, rhs_int, "bitor"),
                BitXor => self.builder.build_xor(lhs_int, rhs_int, "bitxor"),
                Shl => self.builder.build_left_shift(lhs_int, rhs_int, "shl"),
                Shr => {
                    // Arithmetic shift for signed, logical shift for unsigned
                    self.builder.build_right_shift(lhs_int, rhs_int, !is_unsigned, "shr")
                }
                // Pipe is handled specially at the start of compile_binary
                Pipe => unreachable!("Pipe operator handled before operand compilation"),
            };

            result
                .map(|v| v.into())
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])
        }
    }

    /// Compile short-circuit logical operators (&& and ||).
    ///
    /// `a && b` → evaluate a; if false, result=false (skip b); else result=b.
    /// `a || b` → evaluate a; if true, result=true (skip b); else result=b.
    ///
    /// Uses conditional branch + phi node to avoid evaluating the RHS
    /// when the LHS determines the result.
    fn compile_short_circuit(
        &mut self,
        op: &crate::ast::BinOp,
        left: &hir::Expr,
        right: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        use crate::ast::BinOp::*;

        let lhs = self.compile_expr(left)?
            .ok_or_else(|| vec![Diagnostic::error("Expected value for logical op LHS", left.span)])?;
        let lhs_bool = lhs.into_int_value();

        let current_fn = self.builder.get_insert_block()
            .ok_or_else(|| vec![Diagnostic::error("No current block for short-circuit", left.span)])?
            .get_parent()
            .ok_or_else(|| vec![Diagnostic::error("No parent function for short-circuit", left.span)])?;

        let eval_rhs_block = self.context.append_basic_block(current_fn, "sc_rhs");
        let short_circuit_block = self.context.append_basic_block(current_fn, "sc_short");
        let merge_block = self.context.append_basic_block(current_fn, "sc_merge");

        // Branch based on LHS.
        // &&: true → eval RHS, false → short-circuit (result=false)
        // ||: true → short-circuit (result=true), false → eval RHS
        match op {
            And => {
                self.builder.build_conditional_branch(lhs_bool, eval_rhs_block, short_circuit_block)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), left.span)])?;
            }
            Or => {
                self.builder.build_conditional_branch(lhs_bool, short_circuit_block, eval_rhs_block)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), left.span)])?;
            }
            _ => unreachable!(),
        }

        // Eval RHS block
        self.builder.position_at_end(eval_rhs_block);
        let rhs = self.compile_expr(right)?
            .ok_or_else(|| vec![Diagnostic::error("Expected value for logical op RHS", right.span)])?;
        let rhs_bool = rhs.into_int_value();
        // The RHS evaluation may have changed the current block (nested short-circuits, calls, etc.)
        let rhs_end_block = self.builder.get_insert_block()
            .ok_or_else(|| vec![Diagnostic::error("No current block after RHS eval", right.span)])?;
        self.builder.build_unconditional_branch(merge_block)
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), right.span)])?;

        // Short-circuit block: result is the constant (false for &&, true for ||)
        self.builder.position_at_end(short_circuit_block);
        let short_val = match op {
            And => self.context.bool_type().const_int(0, false),
            Or => self.context.bool_type().const_int(1, false),
            _ => unreachable!(),
        };
        self.builder.build_unconditional_branch(merge_block)
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), left.span)])?;

        // Merge block: phi node selects the result
        self.builder.position_at_end(merge_block);
        let phi = self.builder.build_phi(self.context.bool_type(), "sc_result")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), left.span)])?;
        phi.add_incoming(&[
            (&rhs_bool, rhs_end_block),
            (&short_val, short_circuit_block),
        ]);

        Ok(phi.as_basic_value())
    }

    /// Compile a unary operation.
    pub(super) fn compile_unary(
        &mut self,
        op: &crate::ast::UnaryOp,
        operand: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        use crate::ast::UnaryOp::*;

        // Handle reference operators before evaluating operand —
        // these need the lvalue (address), not the rvalue.
        match op {
            Ref | RefMut => {
                return self.compile_borrow(operand)?
                    .ok_or_else(|| vec![Diagnostic::error(
                        "Cannot take reference of void expression",
                        operand.span,
                    )]);
            }
            Deref => {
                // Dereference: evaluate operand to a pointer, then load through it.
                let val = self.compile_expr(operand)?
                    .ok_or_else(|| vec![Diagnostic::error(
                        "Expected value for dereference", operand.span
                    )])?;
                if let BasicValueEnum::PointerValue(ptr) = val {
                    // Determine the pointee type from the HIR type of the operand
                    let pointee_ty = match operand.ty.kind() {
                        TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                            self.lower_type(inner)
                        }
                        _ => {
                            // Fallback: try to get from alloca instruction
                            ptr.as_instruction()
                                .and_then(|inst| inst.get_allocated_type().ok())
                                .unwrap_or_else(|| self.context.i8_type().into())
                        }
                    };
                    let loaded = self.builder.build_load(pointee_ty, ptr, "deref")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM load error: {}", e), self.current_span()
                        )])?;
                    return Ok(loaded);
                }
                return Err(vec![Diagnostic::error(
                    format!("Cannot dereference non-pointer type: {:?}", val.get_type()),
                    operand.span,
                )]);
            }
            Neg | Not => {} // fall through to value-based evaluation below
        }

        let val = self.compile_expr(operand)?
            .ok_or_else(|| vec![Diagnostic::error("Expected value for unary op", operand.span)])?;

        // Check if operand is a float
        let is_float = matches!(operand.ty.kind(), TypeKind::Primitive(PrimitiveTy::Float(_)));

        match op {
            Neg => {
                if is_float {
                    let float_val = val.into_float_value();
                    self.builder.build_float_neg(float_val, "fneg")
                        .map(|v| v.into())
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])
                } else {
                    let int_val = val.into_int_value();
                    self.builder.build_int_neg(int_val, "neg")
                        .map(|v| v.into())
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])
                }
            }
            Not => {
                if is_float {
                    return Err(vec![Diagnostic::error(
                        "Bitwise NOT not supported for float types",
                        operand.span,
                    )]);
                }
                let int_val = val.into_int_value();
                self.builder.build_not(int_val, "not")
                    .map(|v| v.into())
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])
            }
            // Ref, RefMut, Deref handled above before operand evaluation
            Ref | RefMut | Deref => unreachable!(),
        }
    }

    /// Compile a function call.
    pub(super) fn compile_call(
        &mut self,
        callee: &hir::Expr,
        args: &[hir::Expr],
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        // First, try to get a direct function reference
        if let hir::ExprKind::Def(def_id) = &callee.kind {
            if let Some(&fn_value) = self.functions.get(def_id) {
                // Direct function call - compile args with type coercion
                let param_types = fn_value.get_type().get_param_types();
                let mut compiled_args = Vec::new();
                for (i, arg) in args.iter().enumerate() {
                    if let Some(val) = self.compile_expr(arg)? {
                        let coerced = if let Some(param_type) = param_types.get(i) {
                            if val.is_pointer_value() && param_type.is_pointer_type() {
                                let ptr_val = val.into_pointer_value();
                                let expected_ptr_type = param_type.into_pointer_type();
                                if ptr_val.get_type() != expected_ptr_type {
                                    self.builder
                                        .build_pointer_cast(ptr_val, expected_ptr_type, "arg_ptr_cast")
                                        .map_err(|e| vec![Diagnostic::error(format!("LLVM pointer cast error: {}", e), self.current_span())])?
                                        .into()
                                } else {
                                    val
                                }
                            } else if val.is_struct_value() && param_type.is_pointer_type() {
                                // Struct value → pointer: alloca + store + pass pointer
                                let struct_val = val.into_struct_value();
                                let alloca = self.builder
                                    .build_alloca(struct_val.get_type(), "call_arg_tmp")
                                    .map_err(|e| vec![Diagnostic::error(format!("LLVM alloca error: {}", e), self.current_span())])?;
                                self.builder.build_store(alloca, struct_val)
                                    .map_err(|e| vec![Diagnostic::error(format!("LLVM store error: {}", e), self.current_span())])?;
                                let expected_ptr_type = param_type.into_pointer_type();
                                if alloca.get_type() != expected_ptr_type {
                                    self.builder
                                        .build_pointer_cast(alloca, expected_ptr_type, "call_arg_ptr_cast")
                                        .map_err(|e| vec![Diagnostic::error(format!("LLVM pointer cast error: {}", e), self.current_span())])?
                                        .into()
                                } else {
                                    alloca.into()
                                }
                            } else if val.is_int_value() && param_type.is_int_type() {
                                let val_int = val.into_int_value();
                                let param_int_type = param_type.into_int_type();
                                if val_int.get_type().get_bit_width() < param_int_type.get_bit_width() {
                                    self.builder
                                        .build_int_z_extend(val_int, param_int_type, "zext")
                                        .map_err(|e| vec![Diagnostic::error(format!("LLVM zext error: {}", e), self.current_span())])?
                                        .into()
                                } else {
                                    val
                                }
                            } else {
                                val
                            }
                        } else {
                            val
                        };
                        compiled_args.push(coerced.into());
                    }
                }

                let call = self.builder
                    .build_call(fn_value, &compiled_args, "call")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

                return Ok(call.try_as_basic_value().basic());
            }

            // Check if this is a builtin function call
            if let Some(builtin_name) = self.builtin_fns.get(def_id).cloned() {
                if let Some(fn_value) = self.module.get_function(&builtin_name) {
                    // Builtin function call - compile args and call runtime function
                    let fn_type = fn_value.get_type();
                    let param_types = fn_type.get_param_types();
                    let mut compiled_args = Vec::new();
                    for (i, arg) in args.iter().enumerate() {
                        if let Some(val) = self.compile_expr(arg)? {
                            let converted_val = if let Some(param_type) = param_types.get(i) {
                                if val.is_int_value() && param_type.is_int_type() {
                                    let val_int = val.into_int_value();
                                    let param_int_type = param_type.into_int_type();
                                    // Zero-extend if arg is smaller (e.g., i1 -> i32)
                                    if val_int.get_type().get_bit_width() < param_int_type.get_bit_width() {
                                        self.builder
                                            .build_int_z_extend(val_int, param_int_type, "zext")
                                            .map_err(|e| vec![Diagnostic::error(format!("LLVM zext error: {}", e), self.current_span())])?
                                            .into()
                                    } else {
                                        val
                                    }
                                } else if val.is_pointer_value() && param_type.is_pointer_type() {
                                    // Cast pointer types if they don't match (e.g., typed ptr -> i8*)
                                    let ptr_val = val.into_pointer_value();
                                    let expected_ptr_type = param_type.into_pointer_type();
                                    if ptr_val.get_type() != expected_ptr_type {
                                        self.builder
                                            .build_pointer_cast(ptr_val, expected_ptr_type, "arg_ptr_cast")
                                            .map_err(|e| vec![Diagnostic::error(format!("LLVM pointer cast error: {}", e), self.current_span())])?
                                            .into()
                                    } else {
                                        val
                                    }
                                } else if val.is_struct_value() && param_type.is_pointer_type() {
                                    // Struct value needs to be passed by pointer (e.g., &str -> ptr for runtime ABI).
                                    // Allocate stack slot, store the struct, and pass the pointer.
                                    let struct_val = val.into_struct_value();
                                    let alloca = self.builder
                                        .build_alloca(struct_val.get_type(), "builtin_arg_tmp")
                                        .map_err(|e| vec![Diagnostic::error(format!("LLVM alloca error: {}", e), self.current_span())])?;
                                    self.builder.build_store(alloca, struct_val)
                                        .map_err(|e| vec![Diagnostic::error(format!("LLVM store error: {}", e), self.current_span())])?;
                                    let expected_ptr_type = param_type.into_pointer_type();
                                    if alloca.get_type() != expected_ptr_type {
                                        self.builder
                                            .build_pointer_cast(alloca, expected_ptr_type, "builtin_arg_ptr_cast")
                                            .map_err(|e| vec![Diagnostic::error(format!("LLVM pointer cast error: {}", e), self.current_span())])?
                                            .into()
                                    } else {
                                        alloca.into()
                                    }
                                } else if val.is_int_value() && param_type.is_pointer_type() {
                                    // Int value needs to be passed by pointer.
                                    // Allocate stack slot, store the int, and pass the pointer.
                                    let int_val = val.into_int_value();
                                    let alloca = self.builder
                                        .build_alloca(int_val.get_type(), "builtin_int_arg_tmp")
                                        .map_err(|e| vec![Diagnostic::error(format!("LLVM alloca error: {}", e), self.current_span())])?;
                                    self.builder.build_store(alloca, int_val)
                                        .map_err(|e| vec![Diagnostic::error(format!("LLVM store error: {}", e), self.current_span())])?;
                                    let expected_ptr_type = param_type.into_pointer_type();
                                    if alloca.get_type() != expected_ptr_type {
                                        self.builder
                                            .build_pointer_cast(alloca, expected_ptr_type, "builtin_int_arg_ptr_cast")
                                            .map_err(|e| vec![Diagnostic::error(format!("LLVM pointer cast error: {}", e), self.current_span())])?
                                            .into()
                                    } else {
                                        alloca.into()
                                    }
                                } else if val.is_array_value() && param_type.is_pointer_type() {
                                    // Array value needs to be passed by pointer.
                                    let array_val = val.into_array_value();
                                    let alloca = self.builder
                                        .build_alloca(array_val.get_type(), "builtin_arr_arg_tmp")
                                        .map_err(|e| vec![Diagnostic::error(format!("LLVM alloca error: {}", e), self.current_span())])?;
                                    self.builder.build_store(alloca, array_val)
                                        .map_err(|e| vec![Diagnostic::error(format!("LLVM store error: {}", e), self.current_span())])?;
                                    let expected_ptr_type = param_type.into_pointer_type();
                                    if alloca.get_type() != expected_ptr_type {
                                        self.builder
                                            .build_pointer_cast(alloca, expected_ptr_type, "builtin_arr_arg_ptr_cast")
                                            .map_err(|e| vec![Diagnostic::error(format!("LLVM pointer cast error: {}", e), self.current_span())])?
                                            .into()
                                    } else {
                                        alloca.into()
                                    }
                                } else {
                                    val
                                }
                            } else {
                                val
                            };
                            compiled_args.push(converted_val.into());
                        }
                    }

                    // Handle out-pointer builtins: these take an output buffer pointer
                    // and return void. The HIR has 0 user args but the runtime function
                    // expects a pointer argument for the output value.
                    let mut output_buffer_alloca = None;
                    let opaque_ptr_type = self.context.ptr_type(AddressSpace::default());
                    match builtin_name.as_str() {
                        "string_new" => {
                            // string_new(out: *void) -> void
                            let string_ty = self.context.struct_type(&[
                                opaque_ptr_type.into(),
                                self.context.i64_type().into(),
                                self.context.i64_type().into(),
                            ], false);
                            let out_alloca = self.builder
                                .build_alloca(string_ty, "string_out")
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM alloca error: {}", e), self.current_span())])?;
                            output_buffer_alloca = Some(out_alloca);
                            let out_ptr = self.builder
                                .build_pointer_cast(out_alloca, opaque_ptr_type, "out_ptr_cast")
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM pointer cast error: {}", e), self.current_span())])?;
                            compiled_args.push(out_ptr.into());
                        }
                        "str_to_string" => {
                            // str_to_string(s: &str, out: *void) -> void
                            // User args already compiled (the &str). Append out pointer.
                            let string_ty = self.context.struct_type(&[
                                opaque_ptr_type.into(),
                                self.context.i64_type().into(),
                                self.context.i64_type().into(),
                            ], false);
                            let out_alloca = self.builder
                                .build_alloca(string_ty, "str_to_string_out")
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM alloca error: {}", e), self.current_span())])?;
                            output_buffer_alloca = Some(out_alloca);
                            let out_ptr = self.builder
                                .build_pointer_cast(out_alloca, opaque_ptr_type, "out_ptr_cast")
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM pointer cast error: {}", e), self.current_span())])?;
                            compiled_args.push(out_ptr.into());
                        }
                        _ => {}
                    }

                    let call = self.builder
                        .build_call(fn_value, &compiled_args, "builtin_call")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

                    // If we used an output buffer, load and return the result
                    if let Some(out_alloca) = output_buffer_alloca {
                        let out_pointee_ty = out_alloca.as_instruction()
                            .and_then(|inst| inst.get_allocated_type().ok())
                            .ok_or_else(|| vec![Diagnostic::error("Cannot determine out-pointer alloca type", self.current_span())])?;
                        let result = self.builder
                            .build_load(out_pointee_ty, out_alloca, "out_result")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM load error: {}", e), self.current_span())])?;
                        return Ok(Some(result));
                    }

                    return Ok(call.try_as_basic_value().basic());
                } else {
                    return Err(vec![Diagnostic::error(
                        format!("Runtime function '{}' not declared", builtin_name),
                        callee.span,
                    )]);
                }
            }
        }

        // Check if this is a closure call (callee is a Closure type with environment)
        if matches!(callee.ty.kind(), TypeKind::Closure { .. }) {
            return self.compile_closure_call(callee, args);
        }

        // Check if this is a plain function pointer call (TypeKind::Fn - no environment)
        if matches!(callee.ty.kind(), TypeKind::Fn { .. }) {
            return self.compile_fn_ptr_call(callee, args);
        }

        Err(vec![Diagnostic::error(
            "Cannot determine function to call",
            callee.span,
        )])
    }

    /// Compile a plain function pointer call: calling a function pointer stored in a variable.
    ///
    /// Function pointers (`fn(args) -> ret`) are now fat pointers `{ fn_ptr, env_ptr }`
    /// to support closures being passed as fn() parameters. For plain functions,
    /// env_ptr is null. We extract both, prepend env_ptr to args, and call the function.
    pub(super) fn compile_fn_ptr_call(
        &mut self,
        callee: &hir::Expr,
        args: &[hir::Expr],
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        let i8_ptr_type = self.context.ptr_type(AddressSpace::default());

        // Compile the callee to get the fat pointer { fn_ptr, env_ptr }
        let fn_val = self.compile_expr(callee)?
            .ok_or_else(|| vec![Diagnostic::error("Expected function pointer value", callee.span)])?;

        // Extract fn_ptr and env_ptr from the fat pointer struct
        let (fn_ptr, env_ptr) = match fn_val {
            BasicValueEnum::StructValue(sv) => {
                let fn_ptr = self.builder.build_extract_value(sv, 0, "fn.ptr")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM extract error: {}", e), callee.span)])?
                    .into_pointer_value();
                let env_ptr = self.builder.build_extract_value(sv, 1, "fn.env")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM extract error: {}", e), callee.span)])?;
                (fn_ptr, env_ptr)
            }
            BasicValueEnum::PointerValue(ptr) => {
                // If it's a pointer, load the struct first
                // Determine the fat pointer struct type: { ptr, ptr }
                let fat_ptr_struct_ty = self.context.struct_type(
                    &[i8_ptr_type.into(), i8_ptr_type.into()], false
                );
                let loaded = self.builder.build_load(fat_ptr_struct_ty, ptr, "fn.load")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM load error: {}", e), callee.span)])?;
                let sv = loaded.into_struct_value();
                let fn_ptr = self.builder.build_extract_value(sv, 0, "fn.ptr")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM extract error: {}", e), callee.span)])?
                    .into_pointer_value();
                let env_ptr = self.builder.build_extract_value(sv, 1, "fn.env")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM extract error: {}", e), callee.span)])?;
                (fn_ptr, env_ptr)
            }
            _ => {
                return Err(vec![Diagnostic::error(
                    format!("Expected fn struct or pointer, got {:?}", fn_val.get_type()),
                    callee.span,
                )]);
            }
        };

        // Compile arguments - prepend env_ptr
        let mut compiled_args: Vec<BasicMetadataValueEnum> = Vec::with_capacity(args.len() + 1);
        compiled_args.push(env_ptr.into());
        for arg in args {
            if let Some(val) = self.compile_expr(arg)? {
                compiled_args.push(val.into());
            }
        }

        // Get the function type from the callee's Blood type
        let (param_types, return_ty) = match callee.ty.kind() {
            TypeKind::Fn { params, ret, .. } => (params.clone(), (*ret).clone()),
            _ => {
                return Err(vec![Diagnostic::error(
                    "Expected function type for fn pointer call",
                    callee.span,
                )]);
            }
        };

        // Build LLVM function type: (env_ptr, params...) -> ret
        let mut llvm_param_types: Vec<BasicMetadataTypeEnum> = Vec::with_capacity(param_types.len() + 1);
        llvm_param_types.push(i8_ptr_type.into()); // env_ptr
        for p in &param_types {
            llvm_param_types.push(self.lower_type(p).into());
        }

        let fn_type = if return_ty.is_unit() {
            self.context.void_type().fn_type(&llvm_param_types, false)
        } else {
            let ret_type = self.lower_type(&return_ty);
            ret_type.fn_type(&llvm_param_types, false)
        };

        // Build the indirect call using build_indirect_call with the function type
        let call = self.builder
            .build_indirect_call(
                fn_type,
                fn_ptr,
                &compiled_args,
                "fn_ptr_call",
            )
            .map_err(|e| vec![Diagnostic::error(format!("LLVM call error: {}", e), callee.span)])?;

        Ok(call.try_as_basic_value().basic())
    }

    /// Compile a closure call: calling a closure value stored in a variable.
    ///
    /// Closure values are fat pointers: { fn_ptr: *i8, env_ptr: *i8 }
    /// We extract the function pointer, cast it to the appropriate type,
    /// and call with (env_ptr, args...).
    pub(super) fn compile_closure_call(
        &mut self,
        callee: &hir::Expr,
        args: &[hir::Expr],
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        let i8_ptr_type = self.context.ptr_type(AddressSpace::default());

        // Compile the closure expression to get the closure struct
        let closure_val = self.compile_expr(callee)?
            .ok_or_else(|| vec![Diagnostic::error("Expected closure value", callee.span)])?;

        // Extract function pointer and environment pointer from the closure struct
        let (fn_ptr, env_ptr) = match closure_val {
            BasicValueEnum::StructValue(sv) => {
                let fn_ptr = self.builder
                    .build_extract_value(sv, 0, "closure.fn")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), callee.span)])?
                    .into_pointer_value();
                let env_ptr = self.builder
                    .build_extract_value(sv, 1, "closure.env")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), callee.span)])?
                    .into_pointer_value();
                (fn_ptr, env_ptr)
            }
            BasicValueEnum::PointerValue(ptr) => {
                // If it's a pointer to a closure struct, load it first
                // Closure struct type: { ptr, ptr }
                let closure_struct_ty = self.context.struct_type(
                    &[i8_ptr_type.into(), i8_ptr_type.into()], false
                );
                let loaded = self.builder.build_load(closure_struct_ty, ptr, "closure.load")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), callee.span)])?;
                let sv = loaded.into_struct_value();
                let fn_ptr = self.builder
                    .build_extract_value(sv, 0, "closure.fn")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), callee.span)])?
                    .into_pointer_value();
                let env_ptr = self.builder
                    .build_extract_value(sv, 1, "closure.env")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), callee.span)])?
                    .into_pointer_value();
                (fn_ptr, env_ptr)
            }
            _ => {
                return Err(vec![Diagnostic::error(
                    format!("Expected closure struct, got {:?}", closure_val.get_type()),
                    callee.span,
                )]);
            }
        };

        // Compile arguments
        let mut compiled_args: Vec<BasicMetadataValueEnum> = vec![env_ptr.into()];
        for arg in args {
            if let Some(val) = self.compile_expr(arg)? {
                compiled_args.push(val.into());
            }
        }

        // Build function type for the closure
        let (param_types, return_ty) = match callee.ty.kind() {
            TypeKind::Fn { params, ret, .. } => (params.clone(), (*ret).clone()),
            _ => {
                return Err(vec![Diagnostic::error(
                    "Expected function type for closure call",
                    callee.span,
                )]);
            }
        };

        // Build LLVM function type: (env_ptr, params...) -> ret
        let mut fn_param_types: Vec<inkwell::types::BasicMetadataTypeEnum> = vec![i8_ptr_type.into()];
        for param_ty in &param_types {
            fn_param_types.push(self.lower_type(param_ty).into());
        }

        let fn_type = if return_ty.is_unit() {
            self.context.void_type().fn_type(&fn_param_types, false)
        } else {
            let ret_llvm = self.lower_type(&return_ty);
            ret_llvm.fn_type(&fn_param_types, false)
        };

        // Build the indirect call using build_indirect_call with the function type
        let call_site = self.builder
            .build_indirect_call(
                fn_type,
                fn_ptr,
                &compiled_args,
                "closure_call",
            )
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), callee.span)])?;

        Ok(call_site.try_as_basic_value().basic())
    }

    /// Compile a method call: `receiver.method(args)`
    ///
    /// This compiles to `method(receiver, args...)`. For dynamic dispatch
    /// based on the receiver's type, the dispatch table would be consulted.
    pub(super) fn compile_method_call(
        &mut self,
        receiver: &hir::Expr,
        method_id: DefId,
        args: &[hir::Expr],
        result_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        // Check if receiver is a dyn Trait - use vtable dispatch
        if let TypeKind::DynTrait { trait_id, .. } = receiver.ty.kind() {
            return self.compile_vtable_dispatch(
                receiver,
                *trait_id,
                method_id,
                args,
                result_ty,
            );
        }

        // Compile the receiver (becomes first argument)
        let receiver_val = self.compile_expr(receiver)?
            .ok_or_else(|| vec![Diagnostic::error(
                "Expected value for method receiver",
                receiver.span,
            )])?;

        // Compile remaining arguments
        let mut compiled_args: Vec<BasicMetadataValueEnum> = vec![receiver_val.into()];
        for arg in args {
            if let Some(val) = self.compile_expr(arg)? {
                compiled_args.push(val.into());
            }
        }

        // Check if this method requires dynamic dispatch
        if let Some(&method_slot) = self.dynamic_dispatch_methods.get(&method_id) {
            // Dynamic dispatch path: lookup implementation at runtime
            self.compile_dynamic_dispatch(
                receiver,
                &receiver_val,
                method_slot,
                &compiled_args,
                result_ty,
            )
        } else {
            // Static dispatch: call the statically resolved method directly
            let fn_value = self.functions.get(&method_id).copied()
                .ok_or_else(|| vec![Diagnostic::error(
                    format!("Method not found: {:?}", method_id),
                    receiver.span,
                )])?;

            // Coerce args to match parameter types (e.g., typed ptr -> i8* for builtins,
            // struct value -> pointer for &self method semantics)
            let method_param_types = fn_value.get_type().get_param_types();
            for (i, arg) in compiled_args.iter_mut().enumerate() {
                if let Some(param_type) = method_param_types.get(i) {
                    if param_type.is_pointer_type() {
                        let expected_ptr_type = param_type.into_pointer_type();
                        match arg {
                            BasicMetadataValueEnum::PointerValue(ptr) => {
                                if ptr.get_type() != expected_ptr_type {
                                    *arg = self.builder
                                        .build_pointer_cast(*ptr, expected_ptr_type, "method_ptr_cast")
                                        .map_err(|e| vec![Diagnostic::error(
                                            format!("LLVM error: {}", e), receiver.span
                                        )])?
                                        .into();
                                }
                            }
                            BasicMetadataValueEnum::StructValue(struct_val) => {
                                // Parameter expects pointer but we have struct value.
                                // Allocate on stack and pass pointer (&self method semantics).
                                let alloca = self.builder
                                    .build_alloca(struct_val.get_type(), "method_arg_tmp")
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM alloca error: {}", e), receiver.span
                                    )])?;
                                self.builder.build_store(alloca, *struct_val)
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM store error: {}", e), receiver.span
                                    )])?;
                                if alloca.get_type() != expected_ptr_type {
                                    *arg = self.builder
                                        .build_pointer_cast(alloca, expected_ptr_type, "method_ptr_cast")
                                        .map_err(|e| vec![Diagnostic::error(
                                            format!("LLVM error: {}", e), receiver.span
                                        )])?
                                        .into();
                                } else {
                                    *arg = alloca.into();
                                }
                            }
                            BasicMetadataValueEnum::IntValue(int_val) => {
                                // Parameter expects pointer but we have int value.
                                // Allocate on stack and pass pointer.
                                let alloca = self.builder
                                    .build_alloca(int_val.get_type(), "method_int_arg_tmp")
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM alloca error: {}", e), receiver.span
                                    )])?;
                                self.builder.build_store(alloca, *int_val)
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM store error: {}", e), receiver.span
                                    )])?;
                                if alloca.get_type() != expected_ptr_type {
                                    *arg = self.builder
                                        .build_pointer_cast(alloca, expected_ptr_type, "method_ptr_cast")
                                        .map_err(|e| vec![Diagnostic::error(
                                            format!("LLVM error: {}", e), receiver.span
                                        )])?
                                        .into();
                                } else {
                                    *arg = alloca.into();
                                }
                            }
                            BasicMetadataValueEnum::ArrayValue(array_val) => {
                                // Parameter expects pointer but we have array value.
                                // Allocate on stack and pass pointer.
                                let alloca = self.builder
                                    .build_alloca(array_val.get_type(), "method_arr_arg_tmp")
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM alloca error: {}", e), receiver.span
                                    )])?;
                                self.builder.build_store(alloca, *array_val)
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM store error: {}", e), receiver.span
                                    )])?;
                                if alloca.get_type() != expected_ptr_type {
                                    *arg = self.builder
                                        .build_pointer_cast(alloca, expected_ptr_type, "method_ptr_cast")
                                        .map_err(|e| vec![Diagnostic::error(
                                            format!("LLVM error: {}", e), receiver.span
                                        )])?
                                        .into();
                                } else {
                                    *arg = alloca.into();
                                }
                            }
                            BasicMetadataValueEnum::FloatValue(_)
                            | BasicMetadataValueEnum::VectorValue(_)
                            | BasicMetadataValueEnum::ScalableVectorValue(_)
                            | BasicMetadataValueEnum::MetadataValue(_) => {
                                return Err(vec![Diagnostic::error(
                                    "Cannot pass float/vector/metadata value where pointer is expected",
                                    receiver.span,
                                )]);
                            }
                        }
                    }
                }
            }

            let call = self.builder
                .build_call(fn_value, &compiled_args, "method_call")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), receiver.span)])?;

            Ok(call.try_as_basic_value().basic())
        }
    }

    /// Compile the pipe operator: `a |> f` compiles to `f(a)`.
    pub(super) fn compile_pipe(
        &mut self,
        left: &hir::Expr,
        right: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        // The right expression should be a function reference
        // Compile a call with left as the single argument
        if let hir::ExprKind::Def(def_id) = &right.kind {
            if let Some(&fn_value) = self.functions.get(def_id) {
                let arg_val = self.compile_expr(left)?
                    .ok_or_else(|| vec![Diagnostic::error("Expected value for pipe argument", left.span)])?;

                let call = self.builder
                    .build_call(fn_value, &[arg_val.into()], "pipe_call")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

                return call.try_as_basic_value().basic()
                    .ok_or_else(|| vec![Diagnostic::error("Pipe function returned void", right.span)]);
            }
        }

        Err(vec![Diagnostic::error(
            "Pipe operator right-hand side must be a function",
            right.span,
        )])
    }

    /// Compile a struct construction expression.
    ///
    /// If `base` is provided (struct update syntax: `S { field: val, ..base }`),
    /// we start with the base struct value and overwrite the specified fields.
    pub(super) fn compile_struct_expr(
        &mut self,
        result_ty: &Type,
        fields: &[hir::FieldExpr],
        base: Option<&hir::Expr>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        // Get the LLVM type for the struct
        let struct_llvm_type = self.lower_type(result_ty);
        let struct_type = struct_llvm_type.into_struct_type();

        // Start with base value if provided, otherwise use undef
        let mut struct_val = if let Some(base_expr) = base {
            let base_val = self.compile_expr(base_expr)?
                .ok_or_else(|| vec![Diagnostic::error(
                    "Expected value for struct base expression",
                    base_expr.span,
                )])?;
            base_val.into_struct_value()
        } else {
            struct_type.get_undef()
        };

        // Overwrite each explicitly specified field
        for field in fields {
            let field_val = self.compile_expr(&field.value)?
                .ok_or_else(|| vec![Diagnostic::error(
                    "Expected value for struct field",
                    field.value.span,
                )])?;

            struct_val = self.builder
                .build_insert_value(struct_val, field_val, field.field_idx, "field")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?
                .into_struct_value();
        }

        Ok(Some(struct_val.into()))
    }

    /// Compile an enum variant construction expression.
    pub(super) fn compile_variant(
        &mut self,
        _def_id: DefId,
        variant_idx: u32,
        fields: &[hir::Expr],
        result_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        // Get the LLVM type for the enum
        let enum_llvm_type = self.lower_type(result_ty);

        // Handle unit enums (no payload fields) - lowered to just i32 tag
        if enum_llvm_type.is_int_type() {
            // Unit enum - just return the tag value
            let tag = self.context.i32_type().const_int(variant_idx as u64, false);
            return Ok(Some(tag.into()));
        }

        // Create an undefined enum value (struct with tag + payload)
        let enum_type = enum_llvm_type.into_struct_type();
        let mut enum_val = enum_type.get_undef();

        // Set the discriminant (tag) as the first field
        let tag = self.context.i32_type().const_int(variant_idx as u64, false);
        enum_val = self.builder
            .build_insert_value(enum_val, tag, 0, "tag")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?
            .into_struct_value();

        // Fill in the variant's fields starting at index 1
        for (i, field_expr) in fields.iter().enumerate() {
            let field_val = self.compile_expr(field_expr)?
                .ok_or_else(|| vec![Diagnostic::error(
                    "Expected value for variant field",
                    field_expr.span,
                )])?;

            // Field index is i + 1 because index 0 is the tag
            enum_val = self.builder
                .build_insert_value(enum_val, field_val, (i + 1) as u32, "variant_field")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?
                .into_struct_value();
        }

        Ok(Some(enum_val.into()))
    }

    /// Compile a field access expression.
    pub(super) fn compile_field_access(
        &mut self,
        base: &hir::Expr,
        field_idx: u32,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        let base_val = self.compile_expr(base)?
            .ok_or_else(|| vec![Diagnostic::error("Expected struct value", base.span)])?;

        // Extract the field from the struct
        let struct_val = base_val.into_struct_value();
        let field_val = self.builder
            .build_extract_value(struct_val, field_idx, "field")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

        Ok(Some(field_val))
    }

    /// Compile an array literal.
    pub(super) fn compile_array(
        &mut self,
        elements: &[hir::Expr],
        result_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        if elements.is_empty() {
            // Empty array - return undefined array value
            let llvm_type = self.lower_type(result_ty);
            let arr_type = llvm_type.into_array_type();
            return Ok(Some(arr_type.get_undef().into()));
        }

        // Get element type from first element
        let elem_type = self.lower_type(&elements[0].ty);
        let arr_type = elem_type.array_type(elements.len() as u32);
        let mut arr_val = arr_type.get_undef();

        for (i, elem) in elements.iter().enumerate() {
            let elem_val = self.compile_expr(elem)?
                .ok_or_else(|| vec![Diagnostic::error("Expected array element value", elem.span)])?;

            arr_val = self.builder
                .build_insert_value(arr_val, elem_val, i as u32, "arr.elem")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?
                .into_array_value();
        }

        Ok(Some(arr_val.into()))
    }

    /// Compile array/slice indexing.
    pub(super) fn compile_index(
        &mut self,
        base: &hir::Expr,
        index: &hir::Expr,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        let base_val = self.compile_expr(base)?
            .ok_or_else(|| vec![Diagnostic::error("Expected array value", base.span)])?;
        let index_val = self.compile_expr(index)?
            .ok_or_else(|| vec![Diagnostic::error("Expected index value", index.span)])?;

        let idx = index_val.into_int_value();

        // Handle based on base type
        match base_val {
            BasicValueEnum::ArrayValue(arr) => {
                // For array values, we need to use extract_value with constant index
                // or store to memory and use GEP for dynamic index
                // For simplicity, if index is constant, use extract_value
                if let Some(const_idx) = idx.get_zero_extended_constant() {
                    let elem = self.builder
                        .build_extract_value(arr, const_idx as u32, "arr.idx")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                    Ok(Some(elem))
                } else {
                    // Dynamic index - allocate array on stack and use GEP
                    let arr_type = arr.get_type();
                    let alloca = self.builder
                        .build_alloca(arr_type, "arr.tmp")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                    self.builder.build_store(alloca, arr)
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

                    let zero = self.context.i32_type().const_int(0, false);
                    let elem_type = arr_type.get_element_type();
                    let ptr = unsafe {
                        self.builder.build_gep(arr_type, alloca, &[zero, idx], "arr.elem.ptr")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?
                    };

                    let elem = self.builder.build_load(elem_type, ptr, "arr.elem")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                    Ok(Some(elem))
                }
            }
            BasicValueEnum::PointerValue(ptr) => {
                // Check if this is a string reference - handle with str_char_at_index
                let is_string = match base.ty.kind() {
                    TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                        matches!(inner.kind(),
                            TypeKind::Primitive(hir::PrimitiveTy::Str) |
                            TypeKind::Primitive(hir::PrimitiveTy::String))
                    }
                    _ => false,
                };

                if is_string {
                    // Call str_char_at_index(ptr, idx) -> {i32, i32} (Option<char>)
                    let func = self.module.get_function("str_char_at_index")
                        .ok_or_else(|| vec![Diagnostic::error(
                            "Runtime function 'str_char_at_index' not declared",
                            base.span,
                        )])?;

                    // Convert index to i64 for the function call
                    let idx_i64 = if idx.get_type().get_bit_width() == 64 {
                        idx
                    } else {
                        self.builder.build_int_z_extend(idx, self.context.i64_type(), "idx.zext")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?
                    };

                    // Call str_char_at_index(str_ptr, index)
                    let result = self.builder
                        .build_call(func, &[ptr.into(), idx_i64.into()], "char_at_result")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?
                        .try_as_basic_value()
                        .basic()
                        .ok_or_else(|| vec![Diagnostic::error("str_char_at_index should return value", self.current_span())])?;

                    // Result is {i32 tag, i32 value} - extract tag and value
                    // tag=0 means None (out of bounds), tag=1 means Some(char)
                    let result_struct = result.into_struct_value();

                    // Extract the tag to check for out-of-bounds
                    let tag = self.builder
                        .build_extract_value(result_struct, 0, "tag")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?
                        .into_int_value();

                    // Check if tag == 0 (None/out-of-bounds)
                    let zero = self.context.i32_type().const_zero();
                    let is_none = self.builder
                        .build_int_compare(IntPredicate::EQ, tag, zero, "is_none")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

                    // Get current function for creating basic blocks
                    let fn_value = self.current_fn.ok_or_else(|| {
                        vec![Diagnostic::error("No current function for string index bounds check", self.current_span())]
                    })?;

                    // Create basic blocks for bounds check
                    let panic_bb = self.context.append_basic_block(fn_value, "str_index_oob");
                    let continue_bb = self.context.append_basic_block(fn_value, "str_index_ok");

                    // Branch: if tag == 0, panic; else continue
                    self.builder.build_conditional_branch(is_none, panic_bb, continue_bb)
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

                    // Panic block: call blood_panic with out-of-bounds message
                    self.builder.position_at_end(panic_bb);

                    let panic_fn = self.module.get_function("blood_panic")
                        .unwrap_or_else(|| {
                            let void_type = self.context.void_type();
                            let ptr_type = self.context.ptr_type(AddressSpace::default());
                            let panic_type = void_type.fn_type(&[ptr_type.into()], false);
                            self.module.add_function("blood_panic", panic_type, None)
                        });

                    let msg_global = self.builder
                        .build_global_string_ptr("string index out of bounds", "str_oob_msg")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

                    self.builder.build_call(panic_fn, &[msg_global.as_pointer_value().into()], "")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

                    self.builder.build_unreachable()
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

                    // Continue block: extract and return the char value
                    self.builder.position_at_end(continue_bb);

                    let char_val = self.builder
                        .build_extract_value(result_struct, 1, "char_val")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

                    Ok(Some(char_val))
                } else {
                    // Check if this is a pointer to an array (from &[T; N] or &[T])
                    // In that case we need two-index GEP: [0, idx] to first dereference
                    // the pointer to array, then index into the array elements
                    let is_array_ref = match base.ty.kind() {
                        TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                            matches!(inner.kind(), TypeKind::Array { .. } | TypeKind::Slice { .. })
                        }
                        TypeKind::Array { .. } | TypeKind::Slice { .. } => true,
                        _ => false,
                    };

                    // Determine element type from HIR type for loading
                    let (gep_pointee_ty, elem_load_ty) = match base.ty.kind() {
                        TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                            match inner.kind() {
                                TypeKind::Array { element, .. } | TypeKind::Slice { element } => {
                                    let elem_ty = self.lower_type(element);
                                    (self.lower_type(inner), elem_ty)
                                }
                                _ => {
                                    let elem_ty = self.lower_type(inner);
                                    (elem_ty, elem_ty)
                                }
                            }
                        }
                        TypeKind::Array { element, .. } | TypeKind::Slice { element } => {
                            let arr_ty = self.lower_type(&base.ty);
                            let elem_ty = self.lower_type(element);
                            (arr_ty, elem_ty)
                        }
                        _ => {
                            let ty = self.lower_type(&base.ty);
                            (ty, ty)
                        }
                    };

                    let elem_ptr = unsafe {
                        if is_array_ref {
                            // Two-index GEP for array pointers: [0, idx]
                            let zero = self.context.i64_type().const_zero();
                            self.builder.build_in_bounds_gep(gep_pointee_ty, ptr, &[zero, idx], "arr.elem.ptr")
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?
                        } else {
                            // Single-index GEP for element pointers
                            self.builder.build_gep(gep_pointee_ty, ptr, &[idx], "ptr.idx")
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?
                        }
                    };
                    let elem = self.builder.build_load(elem_load_ty, elem_ptr, "elem")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                    Ok(Some(elem))
                }
            }
            BasicValueEnum::StructValue(struct_val) => {
                // Check if this is a string struct (Str = {ptr, len} or String = {ptr, len, cap})
                let is_string = match base.ty.kind() {
                    TypeKind::Primitive(hir::PrimitiveTy::Str) => true,
                    TypeKind::Primitive(hir::PrimitiveTy::String) => true,
                    TypeKind::Ref { inner, .. } => matches!(
                        inner.kind(),
                        TypeKind::Primitive(hir::PrimitiveTy::Str) |
                        TypeKind::Primitive(hir::PrimitiveTy::String)
                    ),
                    _ => false,
                };

                if is_string {
                    // Allocate the string struct on stack to get a pointer
                    let alloca = self.builder
                        .build_alloca(struct_val.get_type(), "str.tmp")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                    self.builder.build_store(alloca, struct_val)
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

                    // Call str_char_at_index(ptr, idx) -> {i32, i32} (Option<char>)
                    let func = self.module.get_function("str_char_at_index")
                        .ok_or_else(|| vec![Diagnostic::error(
                            "Runtime function 'str_char_at_index' not declared",
                            base.span,
                        )])?;

                    // Convert index to i64 for the function call
                    let idx_i64 = if idx.get_type().get_bit_width() == 64 {
                        idx
                    } else {
                        self.builder.build_int_z_extend(idx, self.context.i64_type(), "idx.zext")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?
                    };

                    // Call str_char_at_index(str_ptr, index)
                    let result = self.builder
                        .build_call(func, &[alloca.into(), idx_i64.into()], "char_at_result")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?
                        .try_as_basic_value()
                        .basic()
                        .ok_or_else(|| vec![Diagnostic::error("str_char_at_index should return value", self.current_span())])?;

                    // Result is {i32 tag, i32 value} - extract tag and value
                    // tag=0 means None (out of bounds), tag=1 means Some(char)
                    let result_struct = result.into_struct_value();

                    // Extract the tag to check for out-of-bounds
                    let tag = self.builder
                        .build_extract_value(result_struct, 0, "tag")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?
                        .into_int_value();

                    // Check if tag == 0 (None/out-of-bounds)
                    let zero = self.context.i32_type().const_zero();
                    let is_none = self.builder
                        .build_int_compare(IntPredicate::EQ, tag, zero, "is_none")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

                    // Get current function for creating basic blocks
                    let fn_value = self.current_fn.ok_or_else(|| {
                        vec![Diagnostic::error("No current function for string index bounds check", self.current_span())]
                    })?;

                    // Create basic blocks for bounds check
                    let panic_bb = self.context.append_basic_block(fn_value, "str_index_oob");
                    let continue_bb = self.context.append_basic_block(fn_value, "str_index_ok");

                    // Branch: if tag == 0, panic; else continue
                    self.builder.build_conditional_branch(is_none, panic_bb, continue_bb)
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

                    // Panic block: call blood_panic with out-of-bounds message
                    self.builder.position_at_end(panic_bb);

                    let panic_fn = self.module.get_function("blood_panic")
                        .unwrap_or_else(|| {
                            let void_type = self.context.void_type();
                            let ptr_type = self.context.ptr_type(AddressSpace::default());
                            let panic_type = void_type.fn_type(&[ptr_type.into()], false);
                            self.module.add_function("blood_panic", panic_type, None)
                        });

                    let msg_global = self.builder
                        .build_global_string_ptr("string index out of bounds", "str_oob_msg")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

                    self.builder.build_call(panic_fn, &[msg_global.as_pointer_value().into()], "")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

                    self.builder.build_unreachable()
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

                    // Continue block: extract and return the char value
                    self.builder.position_at_end(continue_bb);

                    let char_val = self.builder
                        .build_extract_value(result_struct, 1, "char_val")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

                    Ok(Some(char_val))
                } else {
                    Err(vec![Diagnostic::error(
                        "Cannot index struct type (not a string)",
                        base.span,
                    )])
                }
            }
            _ => {
                Err(vec![Diagnostic::error(
                    "Cannot index non-array type",
                    base.span,
                )])
            }
        }
    }

    /// Compile a type cast expression.
    pub(super) fn compile_cast(
        &mut self,
        expr: &hir::Expr,
        target_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        // Special case: casting to dyn Trait (trait object coercion)
        if let TypeKind::DynTrait { trait_id, .. } = target_ty.kind() {
            return self.compile_trait_object_coercion(expr, *trait_id, target_ty);
        }

        let val = self.compile_expr(expr)?
            .ok_or_else(|| vec![Diagnostic::error("Expected value for cast", expr.span)])?;

        let target_llvm = self.lower_type(target_ty);

        // Perform cast based on source and target types
        match (val, target_llvm) {
            // Int to int (sign extend, zero extend, or truncate)
            (BasicValueEnum::IntValue(int_val), BasicTypeEnum::IntType(target_int)) => {
                let source_bits = int_val.get_type().get_bit_width();
                let target_bits = target_int.get_bit_width();

                let result = if target_bits > source_bits {
                    // Extend: use sign-extend for signed types, zero-extend for unsigned
                    let is_signed = matches!(expr.ty.kind(), TypeKind::Primitive(PrimitiveTy::Int(_)));
                    if is_signed {
                        self.builder.build_int_s_extend(int_val, target_int, "sext")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?
                    } else {
                        self.builder.build_int_z_extend(int_val, target_int, "zext")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?
                    }
                } else if target_bits < source_bits {
                    // Truncate
                    self.builder.build_int_truncate(int_val, target_int, "trunc")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?
                } else {
                    // Same size - no-op
                    int_val
                };

                Ok(Some(result.into()))
            }
            // Float to float
            (BasicValueEnum::FloatValue(float_val), BasicTypeEnum::FloatType(target_float)) => {
                let result = self.builder.build_float_cast(float_val, target_float, "fcast")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                Ok(Some(result.into()))
            }
            // Int to float
            (BasicValueEnum::IntValue(int_val), BasicTypeEnum::FloatType(target_float)) => {
                let result = self.builder.build_signed_int_to_float(int_val, target_float, "sitofp")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                Ok(Some(result.into()))
            }
            // Float to int
            (BasicValueEnum::FloatValue(float_val), BasicTypeEnum::IntType(target_int)) => {
                let result = self.builder.build_float_to_signed_int(float_val, target_int, "fptosi")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                Ok(Some(result.into()))
            }
            // Pointer casts
            (BasicValueEnum::PointerValue(ptr), BasicTypeEnum::PointerType(target_ptr)) => {
                let result = self.builder.build_pointer_cast(ptr, target_ptr, "ptrcast")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                Ok(Some(result.into()))
            }
            // Int to pointer
            (BasicValueEnum::IntValue(int_val), BasicTypeEnum::PointerType(target_ptr)) => {
                let result = self.builder.build_int_to_ptr(int_val, target_ptr, "inttoptr")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                Ok(Some(result.into()))
            }
            // Pointer to int
            (BasicValueEnum::PointerValue(ptr), BasicTypeEnum::IntType(target_int)) => {
                let result = self.builder.build_ptr_to_int(ptr, target_int, "ptrtoint")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                Ok(Some(result.into()))
            }
            (val, target_llvm) => {
                // Unsupported cast - this is an error, not just a warning
                Err(vec![Diagnostic::error(
                    format!(
                        "unsupported cast: cannot cast {} to {}",
                        self.format_basic_value_kind(&val),
                        self.format_basic_type_kind(&target_llvm)
                    ),
                    expr.span,
                )])
            }
        }
    }

    /// Format a BasicValueEnum kind for error messages.
    fn format_basic_value_kind(&self, val: &BasicValueEnum<'ctx>) -> &'static str {
        match val {
            BasicValueEnum::IntValue(_) => "integer",
            BasicValueEnum::FloatValue(_) => "float",
            BasicValueEnum::PointerValue(_) => "pointer",
            BasicValueEnum::ArrayValue(_) => "array",
            BasicValueEnum::VectorValue(_) => "vector",
            BasicValueEnum::ScalableVectorValue(_) => "scalable_vector",
            BasicValueEnum::StructValue(_) => "struct",
        }
    }

    /// Format a BasicTypeEnum kind for error messages.
    fn format_basic_type_kind(&self, ty: &BasicTypeEnum<'ctx>) -> &'static str {
        match ty {
            BasicTypeEnum::IntType(_) => "integer",
            BasicTypeEnum::FloatType(_) => "float",
            BasicTypeEnum::PointerType(_) => "pointer",
            BasicTypeEnum::ArrayType(_) => "array",
            BasicTypeEnum::VectorType(_) => "vector",
            BasicTypeEnum::ScalableVectorType(_) => "scalable_vector",
            BasicTypeEnum::StructType(_) => "struct",
        }
    }

    /// Compile a dereference expression: `*x`
    ///
    /// For generational references, this validates the generation before
    /// accessing the pointed-to value. Currently implements basic pointer
    /// load without generation checking (scaffolded for future integration).
    pub(super) fn compile_deref(
        &mut self,
        inner: &hir::Expr,
        span: Span,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        let ptr_val = self.compile_expr(inner)?
            .ok_or_else(|| vec![Diagnostic::error("Expected pointer value for dereference", span)])?;

        match ptr_val {
            BasicValueEnum::PointerValue(ptr) => {
                // NOTE: Full generational pointer support for HIR codegen path.
                //
                // The MIR codegen path (mir_codegen.rs) has full BloodPtr support via:
                // - Rvalue::MakeGenPtr: creates 128-bit BloodPtr struct
                // - Rvalue::ReadGeneration: extracts generation from BloodPtr
                // - emit_generation_check_or_panic: validates generation on dereference
                //
                // For the HIR path, we use thin pointers as a simpler fallback.
                // This is safe for stack-allocated values (generation is always 1)
                // and for code that goes through MIR lowering before reaching here.
                //
                // Full implementation would require:
                // 1. Detecting BloodPtr vs thin pointer (based on type metadata)
                // 2. Extracting expected generation from BloodPtr struct (field 1)
                // 3. Calling blood_get_generation runtime function
                // 4. Comparing and panicking on mismatch

                // Determine pointee type from the HIR type of the inner expression
                let pointee_ty = match inner.ty.kind() {
                    TypeKind::Ref { inner: ref_inner, .. } | TypeKind::Ptr { inner: ref_inner, .. } => {
                        self.lower_type(ref_inner)
                    }
                    _ => {
                        // Fallback: try to get from alloca instruction
                        ptr.as_instruction()
                            .and_then(|inst| inst.get_allocated_type().ok())
                            .unwrap_or_else(|| self.context.i8_type().into())
                    }
                };
                let loaded = self.builder.build_load(pointee_ty, ptr, "deref")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                Ok(Some(loaded))
            }
            _ => {
                Err(vec![Diagnostic::error(
                    format!("Cannot dereference non-pointer type: {:?}", ptr_val.get_type()),
                    span,
                )])
            }
        }
    }

    /// Compile a borrow expression: `&x` or `&mut x`
    ///
    /// Creates a reference to the given expression. For generational references,
    /// this would capture the current generation of the allocation.
    pub(super) fn compile_borrow(
        &mut self,
        inner: &hir::Expr,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        // For local variables, we get the address of their stack allocation.
        // For other expressions, we need to evaluate and store them first.
        match &inner.kind {
            hir::ExprKind::Local(local_id) => {
                // Get the stack slot for this local
                if let Some(&ptr) = self.locals.get(local_id) {
                    // NOTE: Full generational pointer support for HIR codegen path.
                    //
                    // For MIR-based compilation, see Rvalue::MakeGenPtr which creates
                    // 128-bit BloodPtr structs with address, generation, and metadata.
                    //
                    // For the HIR path, we return thin pointers. This is safe because:
                    // - Stack locals always have generation 1 (never deallocated mid-scope)
                    // - Region-allocated values go through MIR lowering which uses BloodPtr
                    //
                    // Full implementation would:
                    // 1. Create BloodPtr struct { i64 addr, i32 gen, i32 metadata }
                    // 2. Set generation to 1 for stack, or read from region header
                    Ok(Some(ptr.into()))
                } else {
                    Err(vec![Diagnostic::error(
                        "Local variable not found for borrow",
                        inner.span,
                    )])
                }
            }
            hir::ExprKind::Field { base, field_idx } => {
                // Borrow of a field - compute address of the field
                let base_val = self.compile_expr(base)?
                    .ok_or_else(|| vec![Diagnostic::error("Expected struct value", base.span)])?;

                // Determine the struct pointee type from HIR
                let struct_pointee_ty = self.lower_type(&base.ty);

                match base_val {
                    BasicValueEnum::PointerValue(ptr) => {
                        // GEP to get field address
                        let zero = self.context.i32_type().const_int(0, false);
                        let field_index = self.context.i32_type().const_int(*field_idx as u64, false);
                        let field_ptr = unsafe {
                            self.builder.build_gep(struct_pointee_ty, ptr, &[zero, field_index], "field.ptr")
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), inner.span)])?
                        };
                        Ok(Some(field_ptr.into()))
                    }
                    BasicValueEnum::StructValue(struct_val) => {
                        // Need to store struct first to get address
                        let struct_type = struct_val.get_type();
                        let alloca = self.builder
                            .build_alloca(struct_type, "struct.tmp")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), inner.span)])?;
                        self.builder.build_store(alloca, struct_val)
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), inner.span)])?;

                        let zero = self.context.i32_type().const_int(0, false);
                        let field_index = self.context.i32_type().const_int(*field_idx as u64, false);
                        let field_ptr = unsafe {
                            self.builder.build_gep(struct_type, alloca, &[zero, field_index], "field.ptr")
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), inner.span)])?
                        };
                        Ok(Some(field_ptr.into()))
                    }
                    _ => {
                        Err(vec![Diagnostic::error(
                            "Cannot borrow field of non-struct type",
                            inner.span,
                        )])
                    }
                }
            }
            hir::ExprKind::Index { base, index } => {
                // Borrow of an array element - compute element address
                let base_val = self.compile_expr(base)?
                    .ok_or_else(|| vec![Diagnostic::error("Expected array value", base.span)])?;
                let index_val = self.compile_expr(index)?
                    .ok_or_else(|| vec![Diagnostic::error("Expected index value", index.span)])?;

                let idx = index_val.into_int_value();

                match base_val {
                    BasicValueEnum::PointerValue(ptr) => {
                        // Determine the pointee type from the base's HIR type
                        let pointee_ty = match base.ty.kind() {
                            TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                                self.lower_type(inner)
                            }
                            _ => self.lower_type(&base.ty),
                        };
                        let elem_ptr = unsafe {
                            self.builder.build_gep(pointee_ty, ptr, &[idx], "elem.ptr")
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), inner.span)])?
                        };
                        Ok(Some(elem_ptr.into()))
                    }
                    BasicValueEnum::ArrayValue(arr) => {
                        // Store array first to get address
                        let arr_type = arr.get_type();
                        let alloca = self.builder
                            .build_alloca(arr_type, "arr.tmp")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), inner.span)])?;
                        self.builder.build_store(alloca, arr)
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), inner.span)])?;

                        let zero = self.context.i32_type().const_int(0, false);
                        let elem_ptr = unsafe {
                            self.builder.build_gep(arr_type, alloca, &[zero, idx], "elem.ptr")
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), inner.span)])?
                        };
                        Ok(Some(elem_ptr.into()))
                    }
                    _ => {
                        Err(vec![Diagnostic::error(
                            "Cannot borrow index of non-array type",
                            inner.span,
                        )])
                    }
                }
            }
            _ => {
                // For other expressions, evaluate and store in a temporary
                let val = self.compile_expr(inner)?
                    .ok_or_else(|| vec![Diagnostic::error("Cannot borrow void expression", inner.span)])?;

                let ty = val.get_type();
                let alloca = self.builder
                    .build_alloca(ty, "borrow.tmp")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), inner.span)])?;
                self.builder.build_store(alloca, val)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), inner.span)])?;

                Ok(Some(alloca.into()))
            }
        }
    }

    /// Compile an address-of expression for raw pointers.
    ///
    /// Creates a raw pointer without generation tracking.
    pub(super) fn compile_addr_of(
        &mut self,
        inner: &hir::Expr,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        // For addr_of, we use the same logic as borrow but without
        // any generation tracking (raw pointers don't have generations).
        self.compile_borrow(inner)
    }
}
