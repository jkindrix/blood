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
                let ptr = self.compile_mir_place(place, body)?;
                Ok(ptr.into())
            }

            Rvalue::AddressOf { place, mutable: _ } => {
                let ptr = self.compile_mir_place(place, body)?;
                Ok(ptr.into())
            }

            Rvalue::BinaryOp { op, left, right } => {
                let lhs = self.compile_mir_operand(left, body, escape_results)?;
                let rhs = self.compile_mir_operand(right, body, escape_results)?;
                self.compile_binary_op(*op, lhs, rhs)
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
                let ptr = self.compile_mir_place(place, body)?;
                // Load discriminant from first field
                let discr = self.builder.build_load(ptr, "discr")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM load error: {}", e), Span::dummy()
                    )])?;
                Ok(discr)
            }

            Rvalue::Len(place) => {
                self.compile_len_rvalue(place, body)
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
                self.compile_read_generation(place, body)
            }

            Rvalue::MakeGenPtr { address, generation, metadata } => {
                self.compile_make_gen_ptr(address, generation, metadata, body, escape_results)
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
                let slice_ptr = self.compile_mir_place(place, body)?;

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
                        let ref_ptr = self.compile_mir_place(place, body)?;
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
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        // Read generation from a generational pointer (BloodPtr)
        // BloodPtr structure: { address: i64, generation: i32, metadata: i32 }
        // Generation is at field index 1
        let ptr = self.compile_mir_place(place, body)?;

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
                let result = self.compile_binary_op(op, lhs, rhs)?;
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
        let val_int = val.into_int_value();

        let result = match op {
            UnOp::Not => self.builder.build_not(val_int, "not"),
            UnOp::Neg => self.builder.build_int_neg(val_int, "neg"),
        }.map_err(|e| vec![Diagnostic::error(
            format!("LLVM unary op error: {}", e), Span::dummy()
        )])?;

        Ok(result.into())
    }

    /// Compile a type cast from MIR.
    pub(super) fn compile_mir_cast(
        &mut self,
        val: BasicValueEnum<'ctx>,
        target_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        let target_llvm = self.lower_type(target_ty);

        // For now, just use int casts
        if let (BasicValueEnum::IntValue(int_val), BasicTypeEnum::IntType(int_ty)) = (val, target_llvm) {
            let cast = self.builder.build_int_cast(int_val, int_ty, "cast")
                .map_err(|e| vec![Diagnostic::error(
                    format!("LLVM cast error: {}", e), Span::dummy()
                )])?;
            Ok(cast.into())
        } else {
            // For other types, return as-is for now
            Ok(val)
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

            AggregateKind::Adt { def_id, variant_idx } => {
                // Look up struct/enum definition
                if let Some(field_types) = self.struct_defs.get(def_id) {
                    let llvm_fields: Vec<_> = field_types.iter()
                        .map(|t| self.lower_type(t))
                        .collect();
                    let struct_ty = self.context.struct_type(&llvm_fields, false);
                    let mut agg = struct_ty.get_undef();
                    for (i, val) in vals.iter().enumerate() {
                        agg = self.builder.build_insert_value(agg, *val, i as u32, &format!("field_{}", i))
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM insert error: {}", e), Span::dummy()
                            )])?
                            .into_struct_value();
                    }
                    Ok(agg.into())
                } else if let Some(_variants) = self.enum_defs.get(def_id) {
                    // Enum variant - first field is tag
                    let variant_index = variant_idx.ok_or_else(|| vec![ice_err!(
                        Span::dummy(),
                        "enum construction without variant index";
                        "def_id" => def_id
                    )])?;
                    let tag = self.context.i32_type().const_int(variant_index as u64, false);
                    let mut all_vals = vec![tag.into()];
                    all_vals.extend(vals);

                    let types: Vec<_> = all_vals.iter().map(|v| v.get_type()).collect();
                    let struct_ty = self.context.struct_type(&types, false);
                    let mut agg = struct_ty.get_undef();
                    for (i, val) in all_vals.iter().enumerate() {
                        agg = self.builder.build_insert_value(agg, *val, i as u32, &format!("enum_{}", i))
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM insert error: {}", e), Span::dummy()
                            )])?
                            .into_struct_value();
                    }
                    Ok(agg.into())
                } else {
                    Err(vec![Diagnostic::error(
                        format!("Unknown ADT {:?}", def_id), Span::dummy()
                    )])
                }
            }

            AggregateKind::Closure { def_id: _ } => {
                // Closure is a pointer to captured environment struct
                // The closure function is called directly and receives this pointer
                let i8_ptr_ty = self.context.i8_type().ptr_type(AddressSpace::default());

                if vals.is_empty() {
                    // No captures - use null pointer
                    Ok(i8_ptr_ty.const_null().into())
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
                    let env_ptr = self.builder.build_pointer_cast(captures_alloca, i8_ptr_ty, "env_ptr")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM pointer cast error: {}", e), Span::dummy()
                        )])?;

                    Ok(env_ptr.into())
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
                Ok(self.context.f64_type().const_float(*v).into())
            }

            ConstantKind::Bool(v) => {
                Ok(self.context.bool_type().const_int(*v as u64, false).into())
            }

            ConstantKind::Char(v) => {
                Ok(self.context.i32_type().const_int(*v as u64, false).into())
            }

            ConstantKind::String(s) => {
                let global = self.builder.build_global_string_ptr(s, "str")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM string error: {}", e), Span::dummy()
                    )])?;
                Ok(global.as_pointer_value().into())
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
}
