//! MIR rvalue code generation.
//!
//! This module handles compilation of MIR rvalues (expressions that produce values)
//! to LLVM IR.

use inkwell::intrinsics::Intrinsic;
use inkwell::values::BasicValueEnum;
use inkwell::types::{BasicType, BasicTypeEnum};
use inkwell::{AddressSpace, IntPredicate};

use crate::diagnostics::Diagnostic;
use crate::hir::{PrimitiveTy, Type, TypeKind};
use crate::mir::body::MirBody;
use crate::mir::types::{
    Rvalue, Operand, Constant, ConstantKind, BinOp, UnOp, AggregateKind,
};
use crate::mir::EscapeResults;
use crate::span::Span;
use crate::ice_err;

use super::place::MirPlaceCodegen;
use super::CodegenContext;

/// Extension trait for MIR rvalue compilation.
pub trait MirRvalueCodegen<'ctx, 'a> {
    /// Compile a MIR rvalue to produce a value.
    fn compile_mir_rvalue(
        &mut self,
        rvalue: &Rvalue,
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>>;

    /// Compile a MIR operand.
    fn compile_mir_operand(
        &mut self,
        operand: &Operand,
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>>;
}

impl<'ctx, 'a> MirRvalueCodegen<'ctx, 'a> for CodegenContext<'ctx, 'a> {
    fn compile_mir_rvalue(
        &mut self,
        rvalue: &Rvalue,
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        match rvalue {
            Rvalue::Use(operand) => {
                self.compile_mir_operand(operand, body, escape_results)
            }

            Rvalue::Ref { place, mutable: _ } => {
                let ptr = self.compile_mir_place(place, body, escape_results)?;
                Ok(ptr.into())
            }

            Rvalue::AddressOf { place, mutable: _ } => {
                let ptr = self.compile_mir_place(place, body, escape_results)?;
                Ok(ptr.into())
            }

            Rvalue::BinaryOp { op, left, right } => {
                let operand_ty = self.get_operand_type(left, body);
                let is_float = self.is_float_type(operand_ty);
                let lhs = self.compile_mir_operand(left, body, escape_results)?;
                let rhs = self.compile_mir_operand(right, body, escape_results)?;
                self.compile_binary_op(*op, lhs, rhs, is_float)
            }

            Rvalue::CheckedBinaryOp { op, left, right } => {
                // Checked operations return (result, overflow_flag) tuple
                let operand_ty = self.get_operand_type(left, body);
                let is_signed = self.is_signed_type(operand_ty);
                let lhs = self.compile_mir_operand(left, body, escape_results)?;
                let rhs = self.compile_mir_operand(right, body, escape_results)?;
                self.compile_checked_binary_op(*op, lhs, rhs, is_signed)
            }

            Rvalue::UnaryOp { op, operand } => {
                let val = self.compile_mir_operand(operand, body, escape_results)?;
                self.compile_unary_op(*op, val)
            }

            Rvalue::Cast { operand, target_ty } => {
                let val = self.compile_mir_operand(operand, body, escape_results)?;
                self.compile_mir_cast(val, target_ty)
            }

            Rvalue::Discriminant(place) => {
                let ptr = self.compile_mir_place(place, body, escape_results)?;

                // Get the Blood type of the enum to determine its LLVM representation
                let base_ty = &body.locals[place.local.index() as usize].ty;
                let llvm_ty = self.lower_type(base_ty);

                // Check if the enum is represented as a struct (has payload) or bare i32 (tag-only)
                if llvm_ty.is_struct_type() {
                    // Enum with payload: { i32 tag, payload... }
                    // Load discriminant from first field (field 0 is the tag/discriminant)
                    let discr_ptr = self.builder.build_struct_gep(ptr, 0, "discr_ptr")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM struct gep error: {}", e), Span::dummy()
                        )])?;
                    let discr = self.builder.build_load(discr_ptr, "discr")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM load error: {}", e), Span::dummy()
                        )])?;
                    Ok(discr)
                } else {
                    // Tag-only enum: represented as bare i32
                    // Just load the value directly
                    let discr = self.builder.build_load(ptr, "discr")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM load error: {}", e), Span::dummy()
                        )])?;
                    Ok(discr)
                }
            }

            Rvalue::Len(place) => {
                self.compile_len_rvalue(place, body, escape_results)
            }

            Rvalue::Aggregate { kind, operands } => {
                self.compile_aggregate(kind, operands, body, escape_results)
            }

            Rvalue::NullCheck(operand) => {
                let val = self.compile_mir_operand(operand, body, escape_results)?;
                if let BasicValueEnum::PointerValue(ptr) = val {
                    let null = ptr.get_type().const_null();
                    let is_null = self.builder.build_int_compare(
                        IntPredicate::NE,
                        self.builder.build_ptr_to_int(ptr, self.context.i64_type(), "ptr_int")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?,
                        self.builder.build_ptr_to_int(null, self.context.i64_type(), "null_int")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?,
                        "not_null"
                    ).map_err(|e| vec![Diagnostic::error(
                        format!("LLVM compare error: {}", e), Span::dummy()
                    )])?;
                    Ok(is_null.into())
                } else {
                    Ok(self.context.bool_type().const_int(1, false).into())
                }
            }

            Rvalue::ReadGeneration(place) => {
                self.compile_read_generation(place, body, escape_results)
            }

            Rvalue::MakeGenPtr { address, generation, metadata } => {
                self.compile_make_gen_ptr(address, generation, metadata, body, escape_results)
            }

            Rvalue::ZeroInit(ty) => {
                // Create a zero-initialized value of the given type
                let llvm_ty = self.lower_type(ty);
                let zero = llvm_ty.const_zero();
                Ok(zero)
            }
        }
    }

    fn compile_mir_operand(
        &mut self,
        operand: &Operand,
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                self.compile_mir_place_load(place, body, escape_results)
            }

            Operand::Constant(constant) => {
                self.compile_constant(constant)
            }
        }
    }
}

// Helper implementations
impl<'ctx, 'a> CodegenContext<'ctx, 'a> {
    fn compile_len_rvalue(
        &mut self,
        place: &crate::mir::types::Place,
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        // Array/slice length computation
        // For arrays, we extract the static size from the type
        // For slices, we load the length from the fat pointer (field 1 of slice struct)

        // Get the base type from the local
        let base_ty = body.locals[place.local.index() as usize].ty.clone();

        // Compute the effective type after applying projections
        let effective_ty = self.compute_place_type(&base_ty, &place.projection);

        // Extract length based on the type
        match effective_ty.kind() {
            TypeKind::Array { size, .. } => {
                // For arrays, return the static size as a usize (i64)
                let len_val = self.context.i64_type().const_int(*size, false);
                Ok(len_val.into())
            }
            TypeKind::Slice { .. } => {
                // For slices, extract the length from the fat pointer struct
                // A slice is represented as { ptr: *element, len: i64 }
                // We need to load the slice value and extract field 1 (len)
                let slice_ptr = self.compile_mir_place(place, body, escape_results)?;

                // Get pointer to the length field (index 1)
                let len_ptr = self.builder.build_struct_gep(
                    slice_ptr,
                    1,
                    "slice_len_ptr"
                ).map_err(|e| vec![Diagnostic::error(
                    format!("LLVM struct gep error: {}", e), Span::dummy()
                )])?;

                // Load the length value
                let len_val = self.builder.build_load(len_ptr, "slice_len")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM load error: {}", e), Span::dummy()
                    )])?;

                Ok(len_val)
            }
            TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                // For references/pointers to arrays, extract from the inner type
                match inner.kind() {
                    TypeKind::Array { size, .. } => {
                        let len_val = self.context.i64_type().const_int(*size, false);
                        Ok(len_val.into())
                    }
                    TypeKind::Slice { .. } => {
                        // For &[T] or *[T], load the pointer and extract length from fat pointer
                        // First, load the pointer to get the slice value
                        let ref_ptr = self.compile_mir_place(place, body, escape_results)?;
                        let slice_ptr = self.builder.build_load(ref_ptr, "slice_deref")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM load error: {}", e), Span::dummy()
                            )])?.into_pointer_value();

                        // Get pointer to the length field (index 1)
                        let len_ptr = self.builder.build_struct_gep(
                            slice_ptr,
                            1,
                            "slice_len_ptr"
                        ).map_err(|e| vec![Diagnostic::error(
                            format!("LLVM struct gep error: {}", e), Span::dummy()
                        )])?;

                        // Load the length value
                        let len_val = self.builder.build_load(len_ptr, "slice_len")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM load error: {}", e), Span::dummy()
                            )])?;

                        Ok(len_val)
                    }
                    _ => {
                        Err(vec![Diagnostic::error(
                            format!("Cannot compute length of type {:?}", inner.kind()),
                            Span::dummy()
                        )])
                    }
                }
            }
            _ => {
                Err(vec![Diagnostic::error(
                    format!("Cannot compute length of type {:?}", effective_ty.kind()),
                    Span::dummy()
                )])
            }
        }
    }

    fn compile_read_generation(
        &mut self,
        place: &crate::mir::types::Place,
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        // Read generation from a generational pointer (BloodPtr)
        // BloodPtr structure: { address: i64, generation: i32, metadata: i32 }
        // Generation is at field index 1
        let ptr = self.compile_mir_place(place, body, escape_results)?;

        // Load the BloodPtr struct
        let blood_ptr_val = self.builder.build_load(ptr, "blood_ptr")
            .map_err(|e| vec![Diagnostic::error(
                format!("LLVM load error: {}", e),
                Span::dummy()
            )])?;

        // Extract the generation field (index 1) from the struct
        if blood_ptr_val.is_struct_value() {
            let struct_val = blood_ptr_val.into_struct_value();
            let gen_val = self.builder
                .build_extract_value(struct_val, 1, "generation")
                .map_err(|e| vec![Diagnostic::error(
                    format!("Failed to extract generation field from BloodPtr: {}", e),
                    Span::dummy()
                )])?;
            Ok(gen_val)
        } else {
            // The place might be a raw pointer, not a BloodPtr struct
            // In that case, return generation::FIRST (1) as a fallback for stack pointers
            let i32_ty = self.context.i32_type();
            Ok(i32_ty.const_int(1, false).into())
        }
    }

    fn compile_make_gen_ptr(
        &mut self,
        address: &Operand,
        generation: &Operand,
        metadata: &Operand,
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        // Create a generational pointer (128-bit BloodPtr)
        // BloodPtr structure: { address: i64, generation: i32, metadata: i32 }
        let addr_val = self.compile_mir_operand(address, body, escape_results)?;
        let gen_val = self.compile_mir_operand(generation, body, escape_results)?;
        let meta_val = self.compile_mir_operand(metadata, body, escape_results)?;

        // Create the BloodPtr struct type: { i64, i32, i32 }
        let i64_ty = self.context.i64_type();
        let i32_ty = self.context.i32_type();
        let blood_ptr_type = self.context.struct_type(
            &[i64_ty.into(), i32_ty.into(), i32_ty.into()],
            false
        );

        // Ensure operands have correct types
        let addr_i64 = if addr_val.is_pointer_value() {
            // Convert pointer to i64
            self.builder.build_ptr_to_int(
                addr_val.into_pointer_value(),
                i64_ty,
                "addr_as_i64"
            ).map_err(|e| vec![Diagnostic::error(
                format!("LLVM ptr_to_int error: {}", e),
                Span::dummy()
            )])?
        } else if addr_val.is_int_value() {
            let int_val = addr_val.into_int_value();
            if int_val.get_type().get_bit_width() == 64 {
                int_val
            } else {
                // Zero-extend or truncate to i64
                self.builder.build_int_z_extend_or_bit_cast(
                    int_val,
                    i64_ty,
                    "addr_i64"
                ).map_err(|e| vec![Diagnostic::error(
                    format!("LLVM int cast error: {}", e),
                    Span::dummy()
                )])?
            }
        } else {
            return Err(vec![Diagnostic::error(
                "MakeGenPtr address must be a pointer or integer".to_string(),
                Span::dummy()
            )]);
        };

        let gen_i32 = if gen_val.is_int_value() {
            let int_val = gen_val.into_int_value();
            if int_val.get_type().get_bit_width() == 32 {
                int_val
            } else {
                self.builder.build_int_truncate_or_bit_cast(
                    int_val,
                    i32_ty,
                    "gen_i32"
                ).map_err(|e| vec![Diagnostic::error(
                    format!("LLVM int cast error: {}", e),
                    Span::dummy()
                )])?
            }
        } else {
            return Err(vec![Diagnostic::error(
                "MakeGenPtr generation must be an integer".to_string(),
                Span::dummy()
            )]);
        };

        let meta_i32 = if meta_val.is_int_value() {
            let int_val = meta_val.into_int_value();
            if int_val.get_type().get_bit_width() == 32 {
                int_val
            } else {
                self.builder.build_int_truncate_or_bit_cast(
                    int_val,
                    i32_ty,
                    "meta_i32"
                ).map_err(|e| vec![Diagnostic::error(
                    format!("LLVM int cast error: {}", e),
                    Span::dummy()
                )])?
            }
        } else {
            return Err(vec![Diagnostic::error(
                "MakeGenPtr metadata must be an integer".to_string(),
                Span::dummy()
            )]);
        };

        // Build the BloodPtr struct value
        let mut blood_ptr_val = blood_ptr_type.get_undef();
        blood_ptr_val = self.builder
            .build_insert_value(blood_ptr_val, addr_i64, 0, "with_addr")
            .map_err(|e| vec![Diagnostic::error(
                format!("Failed to insert address into BloodPtr: {}", e),
                Span::dummy()
            )])?
            .into_struct_value();
        blood_ptr_val = self.builder
            .build_insert_value(blood_ptr_val, gen_i32, 1, "with_gen")
            .map_err(|e| vec![Diagnostic::error(
                format!("Failed to insert generation into BloodPtr: {}", e),
                Span::dummy()
            )])?
            .into_struct_value();
        blood_ptr_val = self.builder
            .build_insert_value(blood_ptr_val, meta_i32, 2, "with_meta")
            .map_err(|e| vec![Diagnostic::error(
                format!("Failed to insert metadata into BloodPtr: {}", e),
                Span::dummy()
            )])?
            .into_struct_value();

        Ok(blood_ptr_val.into())
    }

    /// Compile a binary operation.
    pub(super) fn compile_binary_op(
        &mut self,
        op: BinOp,
        lhs: BasicValueEnum<'ctx>,
        rhs: BasicValueEnum<'ctx>,
        is_float: bool,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        if is_float {
            self.compile_float_binary_op(op, lhs, rhs)
        } else {
            self.compile_int_binary_op(op, lhs, rhs)
        }
    }

    /// Compile an integer binary operation.
    fn compile_int_binary_op(
        &mut self,
        op: BinOp,
        lhs: BasicValueEnum<'ctx>,
        rhs: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        let lhs_int = lhs.into_int_value();
        let rhs_int = rhs.into_int_value();

        let result = match op {
            BinOp::Add => self.builder.build_int_add(lhs_int, rhs_int, "add"),
            BinOp::Sub => self.builder.build_int_sub(lhs_int, rhs_int, "sub"),
            BinOp::Mul => self.builder.build_int_mul(lhs_int, rhs_int, "mul"),
            BinOp::Div => self.builder.build_int_signed_div(lhs_int, rhs_int, "div"),
            BinOp::Rem => self.builder.build_int_signed_rem(lhs_int, rhs_int, "rem"),
            BinOp::BitAnd => self.builder.build_and(lhs_int, rhs_int, "and"),
            BinOp::BitOr => self.builder.build_or(lhs_int, rhs_int, "or"),
            BinOp::BitXor => self.builder.build_xor(lhs_int, rhs_int, "xor"),
            BinOp::Shl => self.builder.build_left_shift(lhs_int, rhs_int, "shl"),
            BinOp::Shr => self.builder.build_right_shift(lhs_int, rhs_int, true, "shr"),
            BinOp::Eq => self.builder.build_int_compare(IntPredicate::EQ, lhs_int, rhs_int, "eq"),
            BinOp::Ne => self.builder.build_int_compare(IntPredicate::NE, lhs_int, rhs_int, "ne"),
            BinOp::Lt => self.builder.build_int_compare(IntPredicate::SLT, lhs_int, rhs_int, "lt"),
            BinOp::Le => self.builder.build_int_compare(IntPredicate::SLE, lhs_int, rhs_int, "le"),
            BinOp::Gt => self.builder.build_int_compare(IntPredicate::SGT, lhs_int, rhs_int, "gt"),
            BinOp::Ge => self.builder.build_int_compare(IntPredicate::SGE, lhs_int, rhs_int, "ge"),
            BinOp::Offset => {
                // Pointer offset - treat as add for now
                self.builder.build_int_add(lhs_int, rhs_int, "offset")
            }
        }.map_err(|e| vec![Diagnostic::error(
            format!("LLVM binary op error: {}", e), Span::dummy()
        )])?;

        Ok(result.into())
    }

    /// Compile a floating-point binary operation.
    fn compile_float_binary_op(
        &mut self,
        op: BinOp,
        lhs: BasicValueEnum<'ctx>,
        rhs: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        use inkwell::FloatPredicate;

        let lhs_float = lhs.into_float_value();
        let rhs_float = rhs.into_float_value();

        let result: BasicValueEnum<'ctx> = match op {
            BinOp::Add => self.builder.build_float_add(lhs_float, rhs_float, "fadd")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM float add error: {}", e), Span::dummy())])?
                .into(),
            BinOp::Sub => self.builder.build_float_sub(lhs_float, rhs_float, "fsub")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM float sub error: {}", e), Span::dummy())])?
                .into(),
            BinOp::Mul => self.builder.build_float_mul(lhs_float, rhs_float, "fmul")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM float mul error: {}", e), Span::dummy())])?
                .into(),
            BinOp::Div => self.builder.build_float_div(lhs_float, rhs_float, "fdiv")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM float div error: {}", e), Span::dummy())])?
                .into(),
            BinOp::Rem => self.builder.build_float_rem(lhs_float, rhs_float, "frem")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM float rem error: {}", e), Span::dummy())])?
                .into(),
            BinOp::Eq => self.builder.build_float_compare(FloatPredicate::OEQ, lhs_float, rhs_float, "feq")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM float compare error: {}", e), Span::dummy())])?
                .into(),
            BinOp::Ne => self.builder.build_float_compare(FloatPredicate::ONE, lhs_float, rhs_float, "fne")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM float compare error: {}", e), Span::dummy())])?
                .into(),
            BinOp::Lt => self.builder.build_float_compare(FloatPredicate::OLT, lhs_float, rhs_float, "flt")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM float compare error: {}", e), Span::dummy())])?
                .into(),
            BinOp::Le => self.builder.build_float_compare(FloatPredicate::OLE, lhs_float, rhs_float, "fle")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM float compare error: {}", e), Span::dummy())])?
                .into(),
            BinOp::Gt => self.builder.build_float_compare(FloatPredicate::OGT, lhs_float, rhs_float, "fgt")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM float compare error: {}", e), Span::dummy())])?
                .into(),
            BinOp::Ge => self.builder.build_float_compare(FloatPredicate::OGE, lhs_float, rhs_float, "fge")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM float compare error: {}", e), Span::dummy())])?
                .into(),
            // Bitwise operations not supported for floats
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr | BinOp::Offset => {
                return Err(vec![Diagnostic::error(
                    format!("bitwise operation {:?} not supported for floating-point types", op),
                    Span::dummy(),
                )]);
            }
        };

        Ok(result)
    }

    /// Compile a checked binary operation using LLVM overflow intrinsics.
    ///
    /// Returns a struct `(result, overflow_flag)` where overflow_flag is true
    /// if the operation overflowed.
    pub(super) fn compile_checked_binary_op(
        &mut self,
        op: BinOp,
        lhs: BasicValueEnum<'ctx>,
        rhs: BasicValueEnum<'ctx>,
        is_signed: bool,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        let lhs_int = lhs.into_int_value();
        let rhs_int = rhs.into_int_value();
        let int_type = lhs_int.get_type();

        // Determine the intrinsic name based on operation and signedness
        let intrinsic_name = match (op, is_signed) {
            (BinOp::Add, true) => "llvm.sadd.with.overflow",
            (BinOp::Add, false) => "llvm.uadd.with.overflow",
            (BinOp::Sub, true) => "llvm.ssub.with.overflow",
            (BinOp::Sub, false) => "llvm.usub.with.overflow",
            (BinOp::Mul, true) => "llvm.smul.with.overflow",
            (BinOp::Mul, false) => "llvm.umul.with.overflow",
            // For operations without overflow intrinsics, fall back to unchecked
            // and return (result, false)
            _ => {
                let result = self.compile_binary_op(op, lhs, rhs, false)?;
                // Build a struct with result and false (no overflow)
                let bool_type = self.context.bool_type();
                let no_overflow = bool_type.const_zero();
                let struct_type = self.context.struct_type(
                    &[int_type.into(), bool_type.into()],
                    false,
                );
                let mut struct_val = struct_type.get_undef();
                struct_val = self.builder.build_insert_value(struct_val, result.into_int_value(), 0, "checked_result")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM insert_value error: {}", e), Span::dummy()
                    )])?
                    .into_struct_value();
                struct_val = self.builder.build_insert_value(struct_val, no_overflow, 1, "checked_overflow")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM insert_value error: {}", e), Span::dummy()
                    )])?
                    .into_struct_value();
                return Ok(struct_val.into());
            }
        };

        // Get the LLVM intrinsic
        let intrinsic = Intrinsic::find(intrinsic_name).ok_or_else(|| {
            vec![Diagnostic::error(
                format!("LLVM intrinsic {} not found", intrinsic_name),
                Span::dummy(),
            )]
        })?;

        // Get the declaration for this specific type
        let intrinsic_fn = intrinsic
            .get_declaration(self.module, &[int_type.into()])
            .ok_or_else(|| {
                vec![Diagnostic::error(
                    format!("Could not get declaration for {} intrinsic", intrinsic_name),
                    Span::dummy(),
                )]
            })?;

        // Call the intrinsic
        let call_result = self.builder
            .build_call(
                intrinsic_fn,
                &[lhs_int.into(), rhs_int.into()],
                "checked_op",
            )
            .map_err(|e| vec![Diagnostic::error(
                format!("LLVM call error: {}", e), Span::dummy()
            )])?;

        // The intrinsic returns {iN, i1} - extract as a struct value
        let result_struct = call_result.try_as_basic_value().left().ok_or_else(|| {
            vec![Diagnostic::error(
                "Overflow intrinsic did not return a value".to_string(),
                Span::dummy(),
            )]
        })?;

        Ok(result_struct)
    }

    /// Compile a unary operation.
    pub(super) fn compile_unary_op(
        &mut self,
        op: UnOp,
        val: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        match op {
            UnOp::Not => {
                // Not only applies to integers (booleans)
                let val_int = val.into_int_value();
                let result = self.builder.build_not(val_int, "not")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM not error: {}", e), Span::dummy()
                    )])?;
                Ok(result.into())
            }
            UnOp::Neg => {
                // Neg applies to both integers and floats
                match val {
                    BasicValueEnum::IntValue(int_val) => {
                        let result = self.builder.build_int_neg(int_val, "neg")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM int neg error: {}", e), Span::dummy()
                            )])?;
                        Ok(result.into())
                    }
                    BasicValueEnum::FloatValue(float_val) => {
                        let result = self.builder.build_float_neg(float_val, "fneg")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM float neg error: {}", e), Span::dummy()
                            )])?;
                        Ok(result.into())
                    }
                    _ => Err(vec![Diagnostic::error(
                        format!("Cannot negate value of type {:?}", val.get_type()),
                        Span::dummy()
                    )])
                }
            }
        }
    }

    /// Compile a type cast from MIR.
    pub(super) fn compile_mir_cast(
        &mut self,
        val: BasicValueEnum<'ctx>,
        target_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        let target_llvm = self.lower_type(target_ty);

        match (val, target_llvm) {
            // Int to Int
            (BasicValueEnum::IntValue(int_val), BasicTypeEnum::IntType(int_ty)) => {
                let src_bits = int_val.get_type().get_bit_width();
                let dst_bits = int_ty.get_bit_width();

                if src_bits == dst_bits {
                    Ok(int_val.into())
                } else if src_bits < dst_bits {
                    // Extending: use zero-extend for bools (i1), sign-extend for other ints
                    if src_bits == 1 {
                        // Bool to larger int: zero extend
                        let cast = self.builder.build_int_z_extend(int_val, int_ty, "zext")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM zext error: {}", e), Span::dummy()
                            )])?;
                        Ok(cast.into())
                    } else {
                        // Regular signed int: sign extend
                        let cast = self.builder.build_int_s_extend(int_val, int_ty, "sext")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM sext error: {}", e), Span::dummy()
                            )])?;
                        Ok(cast.into())
                    }
                } else {
                    // Truncating
                    let cast = self.builder.build_int_truncate(int_val, int_ty, "trunc")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM trunc error: {}", e), Span::dummy()
                        )])?;
                    Ok(cast.into())
                }
            }

            // Float to Int (signed)
            (BasicValueEnum::FloatValue(float_val), BasicTypeEnum::IntType(int_ty)) => {
                let cast = self.builder.build_float_to_signed_int(float_val, int_ty, "fptosi")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM fptosi error: {}", e), Span::dummy()
                    )])?;
                Ok(cast.into())
            }

            // Int to Float
            (BasicValueEnum::IntValue(int_val), BasicTypeEnum::FloatType(float_ty)) => {
                // Determine if the integer is signed based on bit width
                // Blood's i32/i64 are signed, u32/u64 are unsigned
                // For now, assume signed conversion
                let cast = self.builder.build_signed_int_to_float(int_val, float_ty, "sitofp")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM sitofp error: {}", e), Span::dummy()
                    )])?;
                Ok(cast.into())
            }

            // Float to Float
            (BasicValueEnum::FloatValue(float_val), BasicTypeEnum::FloatType(float_ty)) => {
                // Compare the float types directly
                if float_val.get_type() == float_ty {
                    Ok(float_val.into())
                } else {
                    let cast = self.builder.build_float_cast(float_val, float_ty, "fpcast")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM fpcast error: {}", e), Span::dummy()
                        )])?;
                    Ok(cast.into())
                }
            }

            // Pointer to Pointer
            (BasicValueEnum::PointerValue(ptr_val), BasicTypeEnum::PointerType(ptr_ty)) => {
                let cast = self.builder.build_pointer_cast(ptr_val, ptr_ty, "ptrcast")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM ptrcast error: {}", e), Span::dummy()
                    )])?;
                Ok(cast.into())
            }

            // Pointer to Int (for raw address operations)
            (BasicValueEnum::PointerValue(ptr_val), BasicTypeEnum::IntType(int_ty)) => {
                let cast = self.builder.build_ptr_to_int(ptr_val, int_ty, "ptrtoint")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM ptrtoint error: {}", e), Span::dummy()
                    )])?;
                Ok(cast.into())
            }

            // Int to Pointer
            (BasicValueEnum::IntValue(int_val), BasicTypeEnum::PointerType(ptr_ty)) => {
                let cast = self.builder.build_int_to_ptr(int_val, ptr_ty, "inttoptr")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM inttoptr error: {}", e), Span::dummy()
                    )])?;
                Ok(cast.into())
            }

            // Same type, no cast needed
            _ if val.get_type() == target_llvm => Ok(val),

            // Unsupported cast
            _ => {
                Err(vec![Diagnostic::error(
                    format!("Unsupported cast from {:?} to {:?}", val.get_type(), target_llvm),
                    Span::dummy()
                )])
            }
        }
    }

    /// Compile an aggregate value (struct, tuple, array).
    pub(super) fn compile_aggregate(
        &mut self,
        kind: &AggregateKind,
        operands: &[Operand],
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        let vals: Vec<BasicValueEnum> = operands.iter()
            .map(|op| self.compile_mir_operand(op, body, escape_results))
            .collect::<Result<_, _>>()?;

        match kind {
            AggregateKind::Tuple => {
                if vals.is_empty() {
                    // Unit tuple
                    Ok(self.context.i8_type().const_int(0, false).into())
                } else {
                    // Create struct for tuple
                    let types: Vec<_> = vals.iter().map(|v| v.get_type()).collect();
                    let struct_ty = self.context.struct_type(&types, false);
                    let mut agg = struct_ty.get_undef();
                    for (i, val) in vals.iter().enumerate() {
                        agg = self.builder.build_insert_value(agg, *val, i as u32, &format!("tuple_{}", i))
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM insert error: {}", e), Span::dummy()
                            )])?
                            .into_struct_value();
                    }
                    Ok(agg.into())
                }
            }

            AggregateKind::Array(_elem_ty) => {
                if vals.is_empty() {
                    let array_ty = self.context.i32_type().array_type(0);
                    Ok(array_ty.get_undef().into())
                } else {
                    let elem_ty = vals[0].get_type();
                    let array_ty = elem_ty.array_type(vals.len() as u32);
                    let mut agg = array_ty.get_undef();
                    for (i, val) in vals.iter().enumerate() {
                        agg = self.builder.build_insert_value(agg, *val, i as u32, &format!("arr_{}", i))
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM insert error: {}", e), Span::dummy()
                            )])?
                            .into_array_value();
                    }
                    Ok(agg.into())
                }
            }

            AggregateKind::Adt { def_id, variant_idx, type_args } => {
                // Look up struct/enum definition
                if self.struct_defs.contains_key(def_id) {
                    // Use the concrete types of the operand values directly.
                    // This handles generic handlers correctly - the operands have already
                    // been compiled with the concrete type arguments, so their LLVM types
                    // are correct. We don't need to look up and substitute generic field types.
                    if vals.is_empty() {
                        // Unit struct - use i8 placeholder
                        Ok(self.context.i8_type().const_int(0, false).into())
                    } else {
                        let types: Vec<_> = vals.iter().map(|v| v.get_type()).collect();
                        let struct_ty = self.context.struct_type(&types, false);
                        let mut agg = struct_ty.get_undef();
                        for (i, val) in vals.iter().enumerate() {
                            agg = self.builder.build_insert_value(agg, *val, i as u32, &format!("field_{}", i))
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM insert error: {}", e), Span::dummy()
                                )])?
                                .into_struct_value();
                        }
                        Ok(agg.into())
                    }
                } else if self.enum_defs.contains_key(def_id) {
                    // Enum variant - first field is tag, followed by payload fields
                    // IMPORTANT: The constructed value must match the full enum layout,
                    // which is { tag, max_variant_payload... }. Variants with fewer fields
                    // than the largest variant must be padded.
                    let variant_index = variant_idx.ok_or_else(|| vec![ice_err!(
                        Span::dummy(),
                        "enum construction without variant index";
                        "def_id" => def_id
                    )])?;

                    // Get the full enum type using lower_type with concrete type arguments
                    let enum_ty = Type::adt(*def_id, type_args.clone());
                    let full_enum_llvm_ty = self.lower_type(&enum_ty);

                    // Build the aggregate with proper padding
                    let tag = self.context.i32_type().const_int(variant_index as u64, false);

                    if let BasicTypeEnum::StructType(struct_ty) = full_enum_llvm_ty {
                        // Full enum type is a struct { tag, payload... }
                        let mut agg = struct_ty.get_undef();

                        // Insert tag at index 0
                        agg = self.builder.build_insert_value(agg, tag, 0, "enum_tag")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM insert error: {}", e), Span::dummy()
                            )])?
                            .into_struct_value();

                        // Insert actual variant fields (starting at index 1)
                        for (i, val) in vals.iter().enumerate() {
                            agg = self.builder.build_insert_value(agg, *val, (i + 1) as u32, &format!("enum_field_{}", i))
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM insert error: {}", e), Span::dummy()
                                )])?
                                .into_struct_value();
                        }

                        // Remaining fields are already undef from get_undef(), which is correct
                        // padding for variants with fewer fields than the maximum.
                        Ok(agg.into())
                    } else {
                        // Enum type is just the tag (all variants are unit variants)
                        Ok(tag.into())
                    }
                } else {
                    Err(vec![Diagnostic::error(
                        format!("Unknown ADT {:?}", def_id), Span::dummy()
                    )])
                }
            }

            AggregateKind::Closure { def_id } => {
                // Closure is a fat pointer: { fn_ptr: i8*, env_ptr: i8* }
                // fn_ptr points to the closure function
                // env_ptr points to the captured environment (or null if no captures)
                let i8_ptr_ty = self.context.i8_type().ptr_type(AddressSpace::default());
                let closure_struct_ty = self.context.struct_type(&[i8_ptr_ty.into(), i8_ptr_ty.into()], false);

                // Get the closure function pointer
                let fn_ptr = if let Some(&fn_value) = self.functions.get(def_id) {
                    self.builder.build_pointer_cast(
                        fn_value.as_global_value().as_pointer_value(),
                        i8_ptr_ty,
                        "closure.fn_ptr"
                    ).map_err(|e| vec![Diagnostic::error(
                        format!("LLVM pointer cast error: {}", e), Span::dummy()
                    )])?
                } else {
                    return Err(vec![Diagnostic::error(
                        format!("Closure function {:?} not found", def_id), Span::dummy()
                    )]);
                };

                // Build the environment pointer
                let env_ptr = if vals.is_empty() {
                    // No captures - use null pointer
                    i8_ptr_ty.const_null()
                } else {
                    // Build a struct with captured values
                    let types: Vec<_> = vals.iter().map(|v| v.get_type()).collect();
                    let captures_struct_ty = self.context.struct_type(&types, false);
                    let mut captures_val = captures_struct_ty.get_undef();
                    for (i, val) in vals.iter().enumerate() {
                        captures_val = self.builder.build_insert_value(captures_val, *val, i as u32, &format!("capture_{}", i))
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM insert error: {}", e), Span::dummy()
                            )])?
                            .into_struct_value();
                    }

                    // Allocate space and store the captures struct
                    let captures_alloca = self.builder.build_alloca(captures_struct_ty, "closure_env")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM alloca error: {}", e), Span::dummy()
                        )])?;
                    self.builder.build_store(captures_alloca, captures_val)
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM store error: {}", e), Span::dummy()
                        )])?;

                    // Cast to i8* for the closure type
                    self.builder.build_pointer_cast(captures_alloca, i8_ptr_ty, "env_ptr")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM pointer cast error: {}", e), Span::dummy()
                        )])?
                };

                // Build the closure fat pointer struct { fn_ptr, env_ptr }
                let mut closure_val = closure_struct_ty.get_undef();
                closure_val = self.builder.build_insert_value(closure_val, fn_ptr, 0, "closure.with_fn")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM insert error: {}", e), Span::dummy()
                    )])?
                    .into_struct_value();
                closure_val = self.builder.build_insert_value(closure_val, env_ptr, 1, "closure.with_env")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM insert error: {}", e), Span::dummy()
                    )])?
                    .into_struct_value();

                Ok(closure_val.into())
            }

            AggregateKind::Record => {
                // Anonymous record is like a tuple - struct with fields in order
                if vals.is_empty() {
                    // Empty record - use i8 placeholder
                    Ok(self.context.i8_type().const_int(0, false).into())
                } else {
                    let types: Vec<_> = vals.iter().map(|v| v.get_type()).collect();
                    let struct_ty = self.context.struct_type(&types, false);
                    let mut agg = struct_ty.get_undef();
                    for (i, val) in vals.iter().enumerate() {
                        agg = self.builder.build_insert_value(agg, *val, i as u32, &format!("record_{}", i))
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM insert error: {}", e), Span::dummy()
                            )])?
                            .into_struct_value();
                    }
                    Ok(agg.into())
                }
            }

            AggregateKind::Range { element, inclusive } => {
                // Range is a struct: { start: T, end: T } or { start: T, end: T, exhausted: bool }
                let elem_ty = self.lower_type(element);

                if *inclusive {
                    // RangeInclusive: { start, end, exhausted }
                    if vals.len() != 3 {
                        return Err(vec![Diagnostic::error(
                            format!("RangeInclusive expects 3 fields, got {}", vals.len()),
                            Span::dummy()
                        )]);
                    }
                    let struct_ty = self.context.struct_type(
                        &[elem_ty, elem_ty, self.context.bool_type().into()],
                        false
                    );
                    let mut range_val = struct_ty.get_undef();
                    range_val = self.builder.build_insert_value(range_val, vals[0], 0, "start")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM insert error: {}", e), Span::dummy())])?
                        .into_struct_value();
                    range_val = self.builder.build_insert_value(range_val, vals[1], 1, "end")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM insert error: {}", e), Span::dummy())])?
                        .into_struct_value();
                    range_val = self.builder.build_insert_value(range_val, vals[2], 2, "exhausted")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM insert error: {}", e), Span::dummy())])?
                        .into_struct_value();
                    Ok(range_val.into())
                } else {
                    // Range: { start, end }
                    if vals.len() != 2 {
                        return Err(vec![Diagnostic::error(
                            format!("Range expects 2 fields, got {}", vals.len()),
                            Span::dummy()
                        )]);
                    }
                    let struct_ty = self.context.struct_type(&[elem_ty, elem_ty], false);
                    let mut range_val = struct_ty.get_undef();
                    range_val = self.builder.build_insert_value(range_val, vals[0], 0, "start")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM insert error: {}", e), Span::dummy())])?
                        .into_struct_value();
                    range_val = self.builder.build_insert_value(range_val, vals[1], 1, "end")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM insert error: {}", e), Span::dummy())])?
                        .into_struct_value();
                    Ok(range_val.into())
                }
            }
        }
    }

    /// Compile a MIR constant.
    pub(super) fn compile_constant(
        &mut self,
        constant: &Constant,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        match &constant.kind {
            ConstantKind::Int(v) => {
                let llvm_ty = self.lower_type(&constant.ty);
                if let BasicTypeEnum::IntType(int_ty) = llvm_ty {
                    Ok(int_ty.const_int(*v as u64, *v < 0).into())
                } else {
                    Ok(self.context.i64_type().const_int(*v as u64, *v < 0).into())
                }
            }

            ConstantKind::Uint(v) => {
                let llvm_ty = self.lower_type(&constant.ty);
                if let BasicTypeEnum::IntType(int_ty) = llvm_ty {
                    Ok(int_ty.const_int(*v as u64, false).into())
                } else {
                    Ok(self.context.i64_type().const_int(*v as u64, false).into())
                }
            }

            ConstantKind::Float(v) => {
                // Check the type to determine if it's f32 or f64
                let llvm_ty = self.lower_type(&constant.ty);
                if let BasicTypeEnum::FloatType(float_ty) = llvm_ty {
                    Ok(float_ty.const_float(*v).into())
                } else {
                    // Fallback to f64
                    Ok(self.context.f64_type().const_float(*v).into())
                }
            }

            ConstantKind::Bool(v) => {
                Ok(self.context.bool_type().const_int(*v as u64, false).into())
            }

            ConstantKind::Char(v) => {
                Ok(self.context.i32_type().const_int(*v as u64, false).into())
            }

            ConstantKind::String(s) => {
                // Create global string constant and str slice {ptr, len}
                let global = self.builder.build_global_string_ptr(s, "str")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM string error: {}", e), Span::dummy()
                    )])?;
                let ptr = global.as_pointer_value();
                let len = self.context.i64_type().const_int(s.len() as u64, false);

                // Create str slice struct {ptr, len}
                let str_type = self.context.struct_type(
                    &[self.context.i8_type().ptr_type(inkwell::AddressSpace::default()).into(),
                      self.context.i64_type().into()],
                    false
                );
                let str_val = str_type.const_named_struct(&[ptr.into(), len.into()]);
                Ok(str_val.into())
            }

            ConstantKind::Unit => {
                Ok(self.context.i8_type().const_int(0, false).into())
            }

            ConstantKind::FnDef(def_id) => {
                // Function reference - get the function pointer
                if let Some(&fn_val) = self.functions.get(def_id) {
                    Ok(fn_val.as_global_value().as_pointer_value().into())
                } else {
                    Err(vec![Diagnostic::error(
                        format!("Unknown function {:?}", def_id), Span::dummy()
                    )])
                }
            }

            ConstantKind::ConstDef(def_id) => {
                // Const item reference - load from global constant
                if let Some(global) = self.const_globals.get(def_id) {
                    let val = self.builder.build_load(global.as_pointer_value(), "const_load")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM load error: {}", e), Span::dummy()
                        )])?;
                    Ok(val)
                } else {
                    Err(vec![Diagnostic::error(
                        format!("Unknown const {:?}", def_id), Span::dummy()
                    )])
                }
            }

            ConstantKind::StaticDef(def_id) => {
                // Static item reference - load from global variable
                if let Some(global) = self.static_globals.get(def_id) {
                    let val = self.builder.build_load(global.as_pointer_value(), "static_load")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM load error: {}", e), Span::dummy()
                        )])?;
                    Ok(val)
                } else {
                    Err(vec![Diagnostic::error(
                        format!("Unknown static {:?}", def_id), Span::dummy()
                    )])
                }
            }

            ConstantKind::ZeroSized => {
                Ok(self.context.i8_type().const_int(0, false).into())
            }
        }
    }

    /// Get the type of an MIR operand.
    pub(super) fn get_operand_type<'b>(&self, operand: &'b Operand, body: &'b MirBody) -> &'b Type {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                &body.locals[place.local.index() as usize].ty
            }
            Operand::Constant(constant) => &constant.ty,
        }
    }

    /// Check if a type is signed (for overflow intrinsic selection).
    pub(super) fn is_signed_type(&self, ty: &Type) -> bool {
        match ty.kind() {
            TypeKind::Primitive(PrimitiveTy::Int(_)) => true,
            TypeKind::Primitive(PrimitiveTy::Uint(_)) => false,
            // Default to signed for other types (conservative)
            _ => true,
        }
    }

    /// Check if a type is a floating-point type.
    pub(super) fn is_float_type(&self, ty: &Type) -> bool {
        matches!(ty.kind(), TypeKind::Primitive(PrimitiveTy::Float(_)))
    }
}
