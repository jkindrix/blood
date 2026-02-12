//! MIR rvalue code generation.
//!
//! This module handles compilation of MIR rvalues (expressions that produce values)
//! to LLVM IR.

use inkwell::intrinsics::Intrinsic;
use inkwell::values::{BasicValue, BasicValueEnum};
use inkwell::types::{BasicType, BasicTypeEnum};
use inkwell::{AddressSpace, IntPredicate};

use crate::diagnostics::Diagnostic;
use crate::hir::{PrimitiveTy, Type, TypeKind};
use crate::mir::body::MirBody;
use crate::mir::types::{
    Rvalue, Operand, Constant, ConstantKind, BinOp, UnOp, AggregateKind,
};
use crate::hir::LocalId;
use crate::mir::{EscapeResults, EscapeState};

use crate::ice_err;

use super::place::MirPlaceCodegen;
use super::types::MirTypesCodegen;
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

    /// Compile a MIR rvalue with destination local context.
    /// This is used for closure creation where we need to know if the closure escapes.
    fn compile_mir_rvalue_with_dest(
        &mut self,
        rvalue: &Rvalue,
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
        dest_local: Option<LocalId>,
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
        // Delegate to the version with destination local context (None = unknown)
        self.compile_mir_rvalue_with_dest(rvalue, body, escape_results, None)
    }

    fn compile_mir_rvalue_with_dest(
        &mut self,
        rvalue: &Rvalue,
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
        dest_local: Option<LocalId>,
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
                let is_float = self.is_float_type(&operand_ty);
                let is_signed = self.is_signed_type(&operand_ty);
                let lhs = self.compile_mir_operand(left, body, escape_results)?;
                let rhs = self.compile_mir_operand(right, body, escape_results)?;
                self.compile_binary_op(*op, lhs, rhs, is_float, is_signed)
            }

            Rvalue::CheckedBinaryOp { op, left, right } => {
                // Checked operations return (result, overflow_flag) tuple
                let operand_ty = self.get_operand_type(left, body);
                let is_signed = self.is_signed_type(&operand_ty);
                let lhs = self.compile_mir_operand(left, body, escape_results)?;
                let rhs = self.compile_mir_operand(right, body, escape_results)?;
                self.compile_checked_binary_op(*op, lhs, rhs, is_signed)
            }

            Rvalue::UnaryOp { op, operand } => {
                let val = self.compile_mir_operand(operand, body, escape_results)?;
                self.compile_unary_op(*op, val)
            }

            Rvalue::Cast { operand, target_ty } => {
                let source_ty = self.get_operand_type(operand, body);
                let val = self.compile_mir_operand(operand, body, escape_results)?;
                self.compile_mir_cast(val, &source_ty, target_ty)
            }

            Rvalue::Discriminant(place) => {
                let mut ptr = self.compile_mir_place(place, body, escape_results)?;

                // Get the Blood type of the enum to determine its LLVM representation
                // Must compute the type AFTER applying projections (e.g., Deref for &Option<T>)
                let base_ty = &body.locals[place.local_unchecked().index() as usize].ty;
                let mut place_ty = self.compute_place_type(base_ty, &place.projection);

                // If the place type is a reference/pointer, we need to load through it
                // to get at the actual enum value. This happens when MIR emits
                // Discriminant(_1) where _1: &Enum without a Deref projection.
                while matches!(place_ty.kind(), TypeKind::Ref { .. } | TypeKind::Ptr { .. }) {
                    let inner = match place_ty.kind() {
                        TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => inner.clone(),
                        _ => unreachable!(),
                    };
                    // Load the pointer from the current location
                    let loaded = self.builder.build_load(ptr, "deref_discr")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM load error: {}", e), self.current_span()
                        )])?;
                    if let Some(inst) = loaded.as_instruction_value() {
                        let _ = inst.set_alignment(8); // pointer alignment
                    }
                    ptr = loaded.into_pointer_value();
                    place_ty = inner;
                }

                let llvm_ty = self.lower_type(&place_ty);

                // Check if the enum is represented as a struct (has payload) or bare i32 (tag-only)
                if llvm_ty.is_struct_type() {
                    // Enum with payload: { i32 tag, payload... }
                    // Load discriminant from first field (field 0 is the tag/discriminant)
                    // First, cast the pointer to the struct type to ensure proper typing
                    let struct_ty = llvm_ty.into_struct_type();
                    let struct_ptr_ty = struct_ty.ptr_type(inkwell::AddressSpace::default());
                    let typed_ptr = self.builder.build_pointer_cast(ptr, struct_ptr_ty, "enum_ptr")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM pointer cast error: {}", e), self.current_span()
                        )])?;

                    let discr_ptr = self.builder.build_struct_gep(typed_ptr, 0, "discr_ptr")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM struct gep error: {}", e), self.current_span()
                        )])?;
                    let discr = self.builder.build_load(discr_ptr, "discr")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM load error: {}", e), self.current_span()
                        )])?;
                    // Set proper alignment for discriminant load (i32 = 4 bytes)
                    if let Some(inst) = discr.as_instruction_value() {
                        let _ = inst.set_alignment(4);
                    }
                    Ok(discr)
                } else {
                    // Tag-only enum: represented as bare i32
                    // Cast pointer to i32* to ensure proper load
                    let i32_type = self.context.i32_type();
                    let i32_ptr_type = i32_type.ptr_type(inkwell::AddressSpace::default());
                    let typed_ptr = self.builder.build_pointer_cast(ptr, i32_ptr_type, "discr_ptr")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM pointer cast error: {}", e), self.current_span()
                        )])?;
                    let discr = self.builder.build_load(typed_ptr, "discr")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM load error: {}", e), self.current_span()
                        )])?;
                    // Set proper alignment for discriminant load (i32 = 4 bytes)
                    if let Some(inst) = discr.as_instruction_value() {
                        let _ = inst.set_alignment(4);
                    }
                    Ok(discr)
                }
            }

            Rvalue::Len(place) => {
                self.compile_len_rvalue(place, body, escape_results)
            }

            Rvalue::VecLen(place) => {
                self.compile_vec_len_rvalue(place, body, escape_results)
            }

            Rvalue::Aggregate { kind, operands } => {
                self.compile_aggregate(kind, operands, body, escape_results, dest_local)
            }

            Rvalue::NullCheck(operand) => {
                let val = self.compile_mir_operand(operand, body, escape_results)?;
                if let BasicValueEnum::PointerValue(ptr) = val {
                    let null = ptr.get_type().const_null();
                    let is_null = self.builder.build_int_compare(
                        IntPredicate::NE,
                        self.builder.build_ptr_to_int(ptr, self.context.i64_type(), "ptr_int")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?,
                        self.builder.build_ptr_to_int(null, self.context.i64_type(), "null_int")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?,
                        "not_null"
                    ).map_err(|e| vec![Diagnostic::error(
                        format!("LLVM compare error: {}", e), self.current_span()
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

            Rvalue::StringIndex { base, index } => {
                // String indexing: call str_char_at_index(str_ptr, index) -> {i32, i32}
                let base_val = self.compile_mir_operand(base, body, escape_results)?;
                let index_val = self.compile_mir_operand(index, body, escape_results)?;

                let func = self.module.get_function("str_char_at_index")
                    .ok_or_else(|| vec![Diagnostic::error(
                        "Runtime function 'str_char_at_index' not declared",
                        body.span,
                    )])?;

                // Convert index to i64 if needed
                let idx_i64 = match index_val {
                    BasicValueEnum::IntValue(iv) => {
                        if iv.get_type().get_bit_width() == 64 {
                            iv
                        } else {
                            self.builder.build_int_z_extend(iv, self.context.i64_type(), "idx.zext")
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), body.span)])?
                        }
                    }
                    _ => return Err(vec![Diagnostic::error("String index must be integer", body.span)]),
                };

                // Get a pointer to the string struct for the function call
                // If base_val is a struct value, we need to allocate it on the stack
                let str_ptr = match base_val {
                    BasicValueEnum::PointerValue(ptr) => ptr,
                    BasicValueEnum::StructValue(sv) => {
                        let alloca = self.builder
                            .build_alloca(sv.get_type(), "str.tmp")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), body.span)])?;
                        self.builder.build_store(alloca, sv)
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), body.span)])?;
                        alloca
                    }
                    _ => return Err(vec![Diagnostic::error("String indexing requires string type", body.span)]),
                };

                // Call str_char_at_index(str_ptr, index)
                let result = self.builder
                    .build_call(func, &[str_ptr.into(), idx_i64.into()], "char_at_result")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), body.span)])?
                    .try_as_basic_value()
                    .left()
                    .ok_or_else(|| vec![Diagnostic::error("str_char_at_index should return value", body.span)])?;

                // Result is {i32 tag, i32 value} - extract tag and value
                // tag=0 means None (out of bounds), tag=1 means Some(char)
                let result_struct = result.into_struct_value();

                // Extract the tag to check for out-of-bounds
                let tag = self.builder
                    .build_extract_value(result_struct, 0, "tag")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), body.span)])?
                    .into_int_value();

                // Check if tag == 0 (None/out-of-bounds)
                let zero = self.context.i32_type().const_zero();
                let is_none = self.builder
                    .build_int_compare(IntPredicate::EQ, tag, zero, "is_none")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), body.span)])?;

                // Get current function for creating basic blocks
                let fn_value = self.current_fn.ok_or_else(|| {
                    vec![Diagnostic::error("No current function for string index bounds check", body.span)]
                })?;

                // Create basic blocks for bounds check
                let panic_bb = self.context.append_basic_block(fn_value, "str_index_oob");
                let continue_bb = self.context.append_basic_block(fn_value, "str_index_ok");

                // Branch: if tag == 0, panic; else continue
                self.builder.build_conditional_branch(is_none, panic_bb, continue_bb)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), body.span)])?;

                // Panic block: call blood_panic with out-of-bounds message
                self.builder.position_at_end(panic_bb);

                let panic_fn = self.module.get_function("blood_panic")
                    .unwrap_or_else(|| {
                        let void_type = self.context.void_type();
                        let i8_type = self.context.i8_type();
                        let i8_ptr_type = i8_type.ptr_type(AddressSpace::default());
                        let panic_type = void_type.fn_type(&[i8_ptr_type.into()], false);
                        self.module.add_function("blood_panic", panic_type, None)
                    });

                let msg_global = self.builder
                    .build_global_string_ptr("string index out of bounds", "str_oob_msg")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), body.span)])?;

                self.builder.build_call(panic_fn, &[msg_global.as_pointer_value().into()], "")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), body.span)])?;

                self.builder.build_unreachable()
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), body.span)])?;

                // Continue block: extract and return the char value
                self.builder.position_at_end(continue_bb);

                let char_val = self.builder
                    .build_extract_value(result_struct, 1, "char_val")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), body.span)])?;

                Ok(char_val)
            }

            Rvalue::ArrayToSlice { array_ref, array_len } => {
                // Array-to-slice coercion: &[T; N] -> &[T]
                // Creates a fat pointer struct { T*, i64 } from an array reference.

                // The array reference is a pointer to [N x T]
                let array_ptr_val = self.compile_mir_operand(array_ref, body, escape_results)?;
                let array_ptr = match array_ptr_val {
                    BasicValueEnum::PointerValue(ptr) => ptr,
                    _ => return Err(vec![Diagnostic::error(
                        "ArrayToSlice expects pointer value for array reference",
                        body.span,
                    )]),
                };

                // Create the length constant
                let len_val = self.context.i64_type().const_int(*array_len, false);

                // Build the fat pointer struct { T*, i64 }
                // First, get the element pointer type
                let elem_ptr = unsafe {
                    self.builder.build_in_bounds_gep(
                        array_ptr,
                        &[self.context.i64_type().const_zero(), self.context.i64_type().const_zero()],
                        "slice_data_ptr"
                    ).map_err(|e| vec![Diagnostic::error(
                        format!("LLVM GEP error: {}", e),
                        body.span,
                    )])?
                };

                // Create the slice struct type { T*, i64 }
                let slice_struct_type = self.context.struct_type(
                    &[elem_ptr.get_type().into(), self.context.i64_type().into()],
                    false,
                );

                // Build the struct value
                let mut slice_struct = slice_struct_type.get_undef();
                slice_struct = self.builder
                    .build_insert_value(slice_struct, elem_ptr, 0, "slice.ptr")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), body.span)])?
                    .into_struct_value();
                slice_struct = self.builder
                    .build_insert_value(slice_struct, len_val, 1, "slice.len")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), body.span)])?
                    .into_struct_value();

                Ok(slice_struct.into())
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
        let base_ty = body.locals[place.local_unchecked().index() as usize].ty.clone();

        // Compute the effective type after applying projections
        let effective_ty = self.compute_place_type(&base_ty, &place.projection);

        // Extract length based on the type
        match effective_ty.kind() {
            TypeKind::Array { size, .. } => {
                // For arrays, return the static size as a usize (i64)
                let concrete_size = size.as_u64().unwrap_or(0);
                let len_val = self.context.i64_type().const_int(concrete_size, false);
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
                    format!("LLVM struct gep error: {}", e), self.current_span()
                )])?;

                // Load the length value
                let len_val = self.builder.build_load(len_ptr, "slice_len")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM load error: {}", e), self.current_span()
                    )])?;

                Ok(len_val)
            }
            TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                // For references/pointers to arrays, extract from the inner type
                match inner.kind() {
                    TypeKind::Array { size, .. } => {
                        let concrete_size = size.as_u64().unwrap_or(0);
                        let len_val = self.context.i64_type().const_int(concrete_size, false);
                        Ok(len_val.into())
                    }
                    TypeKind::Slice { .. } => {
                        // For &[T], the local contains the fat pointer struct { ptr*, len } directly.
                        // We need to GEP to field 1 (length) and load it.
                        let slice_storage_ptr = self.compile_mir_place(place, body, escape_results)?;

                        // Get pointer to the length field (index 1) in the fat pointer struct
                        let len_ptr = self.builder.build_struct_gep(
                            slice_storage_ptr,
                            1,
                            "slice_len_ptr"
                        ).map_err(|e| vec![Diagnostic::error(
                            format!("LLVM struct gep error: {}", e), self.current_span()
                        )])?;

                        // Load the length value
                        let len_val = self.builder.build_load(len_ptr, "slice_len")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM load error: {}", e), self.current_span()
                            )])?;

                        Ok(len_val)
                    }
                    _ => {
                        Err(vec![Diagnostic::error(
                            format!("Cannot compute length of type {:?}", inner.kind()),
                            self.current_span()
                        )])
                    }
                }
            }
            _ => {
                Err(vec![Diagnostic::error(
                    format!("Cannot compute length of type {:?}", effective_ty.kind()),
                    self.current_span()
                )])
            }
        }
    }

    /// Compile Vec length extraction.
    /// The place holds a reference to a Vec (&Vec<T> or &mut Vec<T>).
    /// Vec layout: { ptr: *mut u8, len: i64, capacity: i64 }
    /// We extract field 1 (len).
    fn compile_vec_len_rvalue(
        &mut self,
        place: &crate::mir::types::Place,
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        // Get the pointer to the Vec struct (the reference)
        let vec_ptr = self.compile_mir_place(place, body, escape_results)?;

        // The reference to Vec points to the Vec struct { ptr, len, capacity }
        // We need to load through the reference to get the Vec struct pointer,
        // then GEP to field 1 (len) and load it.

        // Load the Vec struct pointer from the reference
        let vec_struct_ptr = self.builder.build_load(vec_ptr, "vec_ptr")
            .map_err(|e| vec![Diagnostic::error(
                format!("LLVM load error: {}", e), self.current_span()
            )])?;

        // If it's a pointer value, use it to access the len field
        if let BasicValueEnum::PointerValue(struct_ptr) = vec_struct_ptr {
            // GEP to field 1 (len) of the Vec struct
            let len_ptr = self.builder.build_struct_gep(
                struct_ptr,
                1,
                "vec_len_ptr"
            ).map_err(|e| vec![Diagnostic::error(
                format!("LLVM struct gep error: {}", e), self.current_span()
            )])?;

            // Load the length value
            let len_val = self.builder.build_load(len_ptr, "vec_len")
                .map_err(|e| vec![Diagnostic::error(
                    format!("LLVM load error: {}", e), self.current_span()
                )])?;

            Ok(len_val)
        } else if let BasicValueEnum::StructValue(sv) = vec_struct_ptr {
            // If we got a struct value directly, extract field 1
            let len_val = self.builder.build_extract_value(sv, 1, "vec_len")
                .map_err(|e| vec![Diagnostic::error(
                    format!("LLVM extract error: {}", e), self.current_span()
                )])?;
            Ok(len_val)
        } else {
            // The place directly holds a Vec struct, try GEP on it
            let len_ptr = self.builder.build_struct_gep(
                vec_ptr,
                1,
                "vec_len_ptr"
            ).map_err(|e| vec![Diagnostic::error(
                format!("LLVM struct gep error for Vec: {}", e), self.current_span()
            )])?;

            let len_val = self.builder.build_load(len_ptr, "vec_len")
                .map_err(|e| vec![Diagnostic::error(
                    format!("LLVM load error: {}", e), self.current_span()
                )])?;

            Ok(len_val)
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
                self.current_span()
            )])?;

        // Extract the generation field (index 1) from the struct
        if blood_ptr_val.is_struct_value() {
            let struct_val = blood_ptr_val.into_struct_value();
            let gen_val = self.builder
                .build_extract_value(struct_val, 1, "generation")
                .map_err(|e| vec![Diagnostic::error(
                    format!("Failed to extract generation field from BloodPtr: {}", e),
                    self.current_span()
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
                self.current_span()
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
                    self.current_span()
                )])?
            }
        } else {
            return Err(vec![Diagnostic::error(
                "MakeGenPtr address must be a pointer or integer".to_string(),
                self.current_span()
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
                    self.current_span()
                )])?
            }
        } else {
            return Err(vec![Diagnostic::error(
                "MakeGenPtr generation must be an integer".to_string(),
                self.current_span()
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
                    self.current_span()
                )])?
            }
        } else {
            return Err(vec![Diagnostic::error(
                "MakeGenPtr metadata must be an integer".to_string(),
                self.current_span()
            )]);
        };

        // Build the BloodPtr struct value
        let mut blood_ptr_val = blood_ptr_type.get_undef();
        blood_ptr_val = self.builder
            .build_insert_value(blood_ptr_val, addr_i64, 0, "with_addr")
            .map_err(|e| vec![Diagnostic::error(
                format!("Failed to insert address into BloodPtr: {}", e),
                self.current_span()
            )])?
            .into_struct_value();
        blood_ptr_val = self.builder
            .build_insert_value(blood_ptr_val, gen_i32, 1, "with_gen")
            .map_err(|e| vec![Diagnostic::error(
                format!("Failed to insert generation into BloodPtr: {}", e),
                self.current_span()
            )])?
            .into_struct_value();
        blood_ptr_val = self.builder
            .build_insert_value(blood_ptr_val, meta_i32, 2, "with_meta")
            .map_err(|e| vec![Diagnostic::error(
                format!("Failed to insert metadata into BloodPtr: {}", e),
                self.current_span()
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
        is_signed: bool,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        if is_float {
            self.compile_float_binary_op(op, lhs, rhs)
        } else {
            self.compile_int_binary_op(op, lhs, rhs, is_signed)
        }
    }

    /// Compile an integer binary operation.
    fn compile_int_binary_op(
        &mut self,
        op: BinOp,
        lhs: BasicValueEnum<'ctx>,
        rhs: BasicValueEnum<'ctx>,
        is_signed: bool,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        let lhs_int = lhs.into_int_value();
        let rhs_int = rhs.into_int_value();

        // Handle type mismatches for boolean operations (i1 vs i8)
        // This can happen when one operand is a loaded bool (i1) and the other
        // is a constant with a different integer type (e.g., i8 from unit type).
        let (lhs_int, rhs_int) = if lhs_int.get_type().get_bit_width() != rhs_int.get_type().get_bit_width() {
            let lhs_width = lhs_int.get_type().get_bit_width();
            let rhs_width = rhs_int.get_type().get_bit_width();

            // For boolean operations, coerce both to i1 (bool type)
            if matches!(op, BinOp::BitAnd | BinOp::BitOr) && (lhs_width == 1 || rhs_width == 1) {
                let bool_ty = self.context.bool_type();
                let new_lhs = if lhs_width == 1 {
                    lhs_int
                } else {
                    // Truncate to i1 (non-zero becomes true)
                    self.builder.build_int_truncate(lhs_int, bool_ty, "bool_trunc")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM truncate error: {}", e), self.current_span()
                        )])?
                };
                let new_rhs = if rhs_width == 1 {
                    rhs_int
                } else {
                    // Truncate to i1 (non-zero becomes true)
                    self.builder.build_int_truncate(rhs_int, bool_ty, "bool_trunc")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM truncate error: {}", e), self.current_span()
                        )])?
                };
                (new_lhs, new_rhs)
            } else {
                // For other operations, extend the smaller operand to match the larger
                let target_ty = if lhs_width > rhs_width {
                    lhs_int.get_type()
                } else {
                    rhs_int.get_type()
                };
                let new_lhs = if lhs_width < rhs_width {
                    if is_signed {
                        self.builder.build_int_s_extend(lhs_int, target_ty, "sext")
                    } else {
                        self.builder.build_int_z_extend(lhs_int, target_ty, "zext")
                    }.map_err(|e| vec![Diagnostic::error(
                            format!("LLVM extend error: {}", e), self.current_span()
                        )])?
                } else {
                    lhs_int
                };
                let new_rhs = if rhs_width < lhs_width {
                    if is_signed {
                        self.builder.build_int_s_extend(rhs_int, target_ty, "sext")
                    } else {
                        self.builder.build_int_z_extend(rhs_int, target_ty, "zext")
                    }.map_err(|e| vec![Diagnostic::error(
                            format!("LLVM extend error: {}", e), self.current_span()
                        )])?
                } else {
                    rhs_int
                };
                (new_lhs, new_rhs)
            }
        } else {
            (lhs_int, rhs_int)
        };

        let result = match op {
            BinOp::Add => self.builder.build_int_add(lhs_int, rhs_int, "add"),
            BinOp::Sub => self.builder.build_int_sub(lhs_int, rhs_int, "sub"),
            BinOp::Mul => self.builder.build_int_mul(lhs_int, rhs_int, "mul"),
            BinOp::Div => if is_signed {
                self.builder.build_int_signed_div(lhs_int, rhs_int, "sdiv")
            } else {
                self.builder.build_int_unsigned_div(lhs_int, rhs_int, "udiv")
            },
            BinOp::Rem => if is_signed {
                self.builder.build_int_signed_rem(lhs_int, rhs_int, "srem")
            } else {
                self.builder.build_int_unsigned_rem(lhs_int, rhs_int, "urem")
            },
            BinOp::BitAnd => self.builder.build_and(lhs_int, rhs_int, "and"),
            BinOp::BitOr => self.builder.build_or(lhs_int, rhs_int, "or"),
            BinOp::BitXor => self.builder.build_xor(lhs_int, rhs_int, "xor"),
            BinOp::Shl => self.builder.build_left_shift(lhs_int, rhs_int, "shl"),
            BinOp::Shr => self.builder.build_right_shift(lhs_int, rhs_int, is_signed, "shr"),
            BinOp::Eq => self.builder.build_int_compare(IntPredicate::EQ, lhs_int, rhs_int, "eq"),
            BinOp::Ne => self.builder.build_int_compare(IntPredicate::NE, lhs_int, rhs_int, "ne"),
            BinOp::Lt => self.builder.build_int_compare(
                if is_signed { IntPredicate::SLT } else { IntPredicate::ULT },
                lhs_int, rhs_int, "lt"
            ),
            BinOp::Le => self.builder.build_int_compare(
                if is_signed { IntPredicate::SLE } else { IntPredicate::ULE },
                lhs_int, rhs_int, "le"
            ),
            BinOp::Gt => self.builder.build_int_compare(
                if is_signed { IntPredicate::SGT } else { IntPredicate::UGT },
                lhs_int, rhs_int, "gt"
            ),
            BinOp::Ge => self.builder.build_int_compare(
                if is_signed { IntPredicate::SGE } else { IntPredicate::UGE },
                lhs_int, rhs_int, "ge"
            ),
            BinOp::Offset => {
                // Pointer offset - treat as add for now
                self.builder.build_int_add(lhs_int, rhs_int, "offset")
            }
        }.map_err(|e| vec![Diagnostic::error(
            format!("LLVM binary op error: {}", e), self.current_span()
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
                .map_err(|e| vec![Diagnostic::error(format!("LLVM float add error: {}", e), self.current_span())])?
                .into(),
            BinOp::Sub => self.builder.build_float_sub(lhs_float, rhs_float, "fsub")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM float sub error: {}", e), self.current_span())])?
                .into(),
            BinOp::Mul => self.builder.build_float_mul(lhs_float, rhs_float, "fmul")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM float mul error: {}", e), self.current_span())])?
                .into(),
            BinOp::Div => self.builder.build_float_div(lhs_float, rhs_float, "fdiv")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM float div error: {}", e), self.current_span())])?
                .into(),
            BinOp::Rem => self.builder.build_float_rem(lhs_float, rhs_float, "frem")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM float rem error: {}", e), self.current_span())])?
                .into(),
            BinOp::Eq => self.builder.build_float_compare(FloatPredicate::OEQ, lhs_float, rhs_float, "feq")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM float compare error: {}", e), self.current_span())])?
                .into(),
            BinOp::Ne => self.builder.build_float_compare(FloatPredicate::ONE, lhs_float, rhs_float, "fne")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM float compare error: {}", e), self.current_span())])?
                .into(),
            BinOp::Lt => self.builder.build_float_compare(FloatPredicate::OLT, lhs_float, rhs_float, "flt")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM float compare error: {}", e), self.current_span())])?
                .into(),
            BinOp::Le => self.builder.build_float_compare(FloatPredicate::OLE, lhs_float, rhs_float, "fle")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM float compare error: {}", e), self.current_span())])?
                .into(),
            BinOp::Gt => self.builder.build_float_compare(FloatPredicate::OGT, lhs_float, rhs_float, "fgt")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM float compare error: {}", e), self.current_span())])?
                .into(),
            BinOp::Ge => self.builder.build_float_compare(FloatPredicate::OGE, lhs_float, rhs_float, "fge")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM float compare error: {}", e), self.current_span())])?
                .into(),
            // Bitwise operations not supported for floats
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr | BinOp::Offset => {
                return Err(vec![Diagnostic::error(
                    format!("bitwise operation {:?} not supported for floating-point types", op),
                    self.current_span(),
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
                let result = self.compile_binary_op(op, lhs, rhs, false, is_signed)?;
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
                        format!("LLVM insert_value error: {}", e), self.current_span()
                    )])?
                    .into_struct_value();
                struct_val = self.builder.build_insert_value(struct_val, no_overflow, 1, "checked_overflow")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM insert_value error: {}", e), self.current_span()
                    )])?
                    .into_struct_value();
                return Ok(struct_val.into());
            }
        };

        // Get the LLVM intrinsic
        let intrinsic = Intrinsic::find(intrinsic_name).ok_or_else(|| {
            vec![Diagnostic::error(
                format!("LLVM intrinsic {} not found", intrinsic_name),
                self.current_span(),
            )]
        })?;

        // Get the declaration for this specific type
        let intrinsic_fn = intrinsic
            .get_declaration(self.module, &[int_type.into()])
            .ok_or_else(|| {
                vec![Diagnostic::error(
                    format!("Could not get declaration for {} intrinsic", intrinsic_name),
                    self.current_span(),
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
                format!("LLVM call error: {}", e), self.current_span()
            )])?;

        // The intrinsic returns {iN, i1} - extract as a struct value
        let result_struct = call_result.try_as_basic_value().left().ok_or_else(|| {
            vec![Diagnostic::error(
                "Overflow intrinsic did not return a value".to_string(),
                self.current_span(),
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
                        format!("LLVM not error: {}", e), self.current_span()
                    )])?;
                Ok(result.into())
            }
            UnOp::Neg => {
                // Neg applies to both integers and floats
                match val {
                    BasicValueEnum::IntValue(int_val) => {
                        let result = self.builder.build_int_neg(int_val, "neg")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM int neg error: {}", e), self.current_span()
                            )])?;
                        Ok(result.into())
                    }
                    BasicValueEnum::FloatValue(float_val) => {
                        let result = self.builder.build_float_neg(float_val, "fneg")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM float neg error: {}", e), self.current_span()
                            )])?;
                        Ok(result.into())
                    }
                    _ => Err(vec![Diagnostic::error(
                        format!("Cannot negate value of type {:?}", val.get_type()),
                        self.current_span()
                    )])
                }
            }
        }
    }

    /// Compile a type cast from MIR.
    pub(super) fn compile_mir_cast(
        &mut self,
        val: BasicValueEnum<'ctx>,
        source_ty: &Type,
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
                    // Extending: choose sign-extend vs zero-extend based on source type signedness
                    let is_signed = self.is_signed_type(source_ty);
                    if is_signed {
                        // Signed int: sign extend (i32 -> i64)
                        let cast = self.builder.build_int_s_extend(int_val, int_ty, "sext")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM sext error: {}", e), self.current_span()
                            )])?;
                        Ok(cast.into())
                    } else {
                        // Unsigned int or bool: zero extend (u32 -> u64)
                        let cast = self.builder.build_int_z_extend(int_val, int_ty, "zext")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM zext error: {}", e), self.current_span()
                            )])?;
                        Ok(cast.into())
                    }
                } else {
                    // Truncating
                    let cast = self.builder.build_int_truncate(int_val, int_ty, "trunc")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM trunc error: {}", e), self.current_span()
                        )])?;
                    Ok(cast.into())
                }
            }

            // Float to Int (signed)
            (BasicValueEnum::FloatValue(float_val), BasicTypeEnum::IntType(int_ty)) => {
                let cast = self.builder.build_float_to_signed_int(float_val, int_ty, "fptosi")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM fptosi error: {}", e), self.current_span()
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
                        format!("LLVM sitofp error: {}", e), self.current_span()
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
                            format!("LLVM fpcast error: {}", e), self.current_span()
                        )])?;
                    Ok(cast.into())
                }
            }

            // Pointer to Pointer
            (BasicValueEnum::PointerValue(ptr_val), BasicTypeEnum::PointerType(ptr_ty)) => {
                let cast = self.builder.build_pointer_cast(ptr_val, ptr_ty, "ptrcast")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM ptrcast error: {}", e), self.current_span()
                    )])?;
                Ok(cast.into())
            }

            // Pointer to Int (for raw address operations)
            (BasicValueEnum::PointerValue(ptr_val), BasicTypeEnum::IntType(int_ty)) => {
                let cast = self.builder.build_ptr_to_int(ptr_val, int_ty, "ptrtoint")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM ptrtoint error: {}", e), self.current_span()
                    )])?;
                Ok(cast.into())
            }

            // Int to Pointer
            (BasicValueEnum::IntValue(int_val), BasicTypeEnum::PointerType(ptr_ty)) => {
                let cast = self.builder.build_int_to_ptr(int_val, ptr_ty, "inttoptr")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM inttoptr error: {}", e), self.current_span()
                    )])?;
                Ok(cast.into())
            }

            // Fat pointer (struct) to thin pointer - extract data pointer from field 0
            // This handles cases like &str -> *const u8 where &str is { i8*, i64 }
            (BasicValueEnum::StructValue(struct_val), BasicTypeEnum::PointerType(ptr_ty)) => {
                // Extract the data pointer from field 0 of the fat pointer
                let data_ptr = self.builder.build_extract_value(struct_val, 0, "fat_ptr_data")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM extract_value error: {}", e), self.current_span()
                    )])?;
                // Cast to the target pointer type if needed
                if let BasicValueEnum::PointerValue(ptr_val) = data_ptr {
                    let cast = self.builder.build_pointer_cast(ptr_val, ptr_ty, "fat_to_thin_ptr")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM ptrcast error: {}", e), self.current_span()
                        )])?;
                    Ok(cast.into())
                } else {
                    Err(vec![Diagnostic::error(
                        format!("Fat pointer field 0 is not a pointer: {:?}", data_ptr.get_type()),
                        self.current_span()
                    )])
                }
            }

            // Fat pointer (struct) to Int - extract field 0 (fn_ptr or data_ptr) and convert to u64
            // Handles `fn as u64` where fn is { fn_ptr: i8*, env_ptr: i8* }
            (BasicValueEnum::StructValue(struct_val), BasicTypeEnum::IntType(int_ty)) => {
                let field0 = self.builder.build_extract_value(struct_val, 0, "struct_field0")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM extract_value error: {}", e), self.current_span()
                    )])?;
                if let BasicValueEnum::PointerValue(ptr_val) = field0 {
                    let cast = self.builder.build_ptr_to_int(ptr_val, int_ty, "fat_to_int")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM ptrtoint error: {}", e), self.current_span()
                        )])?;
                    Ok(cast.into())
                } else if let BasicValueEnum::IntValue(int_val) = field0 {
                    let cast = self.builder.build_int_cast(int_val, int_ty, "field0_intcast")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM intcast error: {}", e), self.current_span()
                        )])?;
                    Ok(cast.into())
                } else {
                    Err(vec![Diagnostic::error(
                        format!("Cannot cast struct field 0 type {:?} to integer", field0.get_type()),
                        self.current_span()
                    )])
                }
            }

            // Same type, no cast needed
            _ if val.get_type() == target_llvm => Ok(val),

            // Unsupported cast
            _ => {
                Err(vec![Diagnostic::error(
                    format!("Unsupported cast from {:?} to {:?}", val.get_type(), target_llvm),
                    self.current_span()
                )])
            }
        }
    }

    /// Compile an aggregate value (struct, tuple, array).
    ///
    /// For closure aggregates, `dest_local` is used to check escape analysis.
    /// If the closure escapes (is returned from a function or stored in a
    /// heap-allocated location), its captured environment must be heap-allocated
    /// to survive after the creating function returns.
    pub(super) fn compile_aggregate(
        &mut self,
        kind: &AggregateKind,
        operands: &[Operand],
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
        dest_local: Option<LocalId>,
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
                                format!("LLVM insert error: {}", e), self.current_span()
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
                                format!("LLVM insert error: {}", e), self.current_span()
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
                                    format!("LLVM insert error: {}", e), self.current_span()
                                )])?
                                .into_struct_value();
                        }
                        Ok(agg.into())
                    }
                } else if self.enum_defs.contains_key(def_id) {
                    // Enum variant - first field is tag, followed by payload fields
                    // For enums with heterogeneous variant payloads (different sizes/types),
                    // we use alloca + pointer casting since insertvalue requires exact type match.
                    let variant_index = variant_idx.ok_or_else(|| vec![ice_err!(
                        self.current_span(),
                        "enum construction without variant index";
                        "def_id" => def_id
                    )])?;

                    // Get the full enum type using lower_type with concrete type arguments
                    let enum_ty = Type::adt(*def_id, type_args.clone());
                    let full_enum_llvm_ty = self.lower_type(&enum_ty);

                    // Build the aggregate with proper padding
                    let tag = self.context.i32_type().const_int(variant_index as u64, false);

                    if let BasicTypeEnum::StructType(struct_ty) = full_enum_llvm_ty {
                        // Check if we can use direct insertvalue (types match) or need alloca approach
                        let types_match = vals.iter().enumerate().all(|(i, val)| {
                            if let Some(field_ty) = struct_ty.get_field_type_at_index((i + 1) as u32) {
                                let matches = val.get_type() == field_ty;
                                if std::env::var("BLOOD_DEBUG_ENUM").is_ok() {
                                    eprintln!("[Enum] Field {}: val_ty={:?} field_ty={:?} matches={}",
                                        i, val.get_type().print_to_string(), field_ty.print_to_string(), matches);
                                }
                                matches
                            } else {
                                false
                            }
                        });

                        if std::env::var("BLOOD_DEBUG_ENUM").is_ok() {
                            eprintln!("[Enum] variant_index={}, types_match={}, vals.len()={}, struct_ty={:?}",
                                variant_index, types_match, vals.len(), struct_ty.print_to_string());
                        }

                        if types_match && !vals.is_empty() {
                            // Fast path: variant field types match struct field types
                            let mut agg = struct_ty.get_undef();
                            agg = self.builder.build_insert_value(agg, tag, 0, "enum_tag")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM insert error: {}", e), self.current_span()
                                )])?
                                .into_struct_value();
                            for (i, val) in vals.iter().enumerate() {
                                agg = self.builder.build_insert_value(agg, *val, (i + 1) as u32, &format!("enum_field_{}", i))
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM insert error: {}", e), self.current_span()
                                    )])?
                                    .into_struct_value();
                            }
                            Ok(agg.into())
                        } else if vals.is_empty() {
                            // Unit variant - just set tag
                            let mut agg = struct_ty.get_undef();
                            agg = self.builder.build_insert_value(agg, tag, 0, "enum_tag")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM insert error: {}", e), self.current_span()
                                )])?
                                .into_struct_value();
                            Ok(agg.into())
                        } else {
                            // Slow path: variant field types don't match struct field types
                            // Use alloca + GEP + bitcast to store fields in the payload area
                            //
                            // IMPORTANT: The alloca MUST be placed in the entry block of the function.
                            // Allocas in non-entry blocks may not get proper stack alignment (the stack
                            // pointer may not be 16-byte aligned when execution reaches that point).
                            // This caused SIGSEGV when storing i128 values that require 16-byte alignment.
                            let alloca = {
                                // Save current insertion point
                                let current_block = self.builder.get_insert_block()
                                    .ok_or_else(|| vec![Diagnostic::error(
                                        "No current block for alloca".to_string(), self.current_span()
                                    )])?;

                                // Get the entry block of the current function
                                let function = current_block.get_parent()
                                    .ok_or_else(|| vec![Diagnostic::error(
                                        "No parent function for current block".to_string(), self.current_span()
                                    )])?;
                                let entry_block = function.get_first_basic_block()
                                    .ok_or_else(|| vec![Diagnostic::error(
                                        "No entry block in function".to_string(), self.current_span()
                                    )])?;

                                // Position at the start of entry block for the alloca
                                // We insert after any existing instructions (other allocas)
                                if let Some(first_inst) = entry_block.get_first_instruction() {
                                    self.builder.position_before(&first_inst);
                                } else {
                                    self.builder.position_at_end(entry_block);
                                }

                                // Build the alloca in the entry block
                                let alloca = self.builder.build_alloca(struct_ty, "enum_tmp")
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM alloca error: {}", e), self.current_span()
                                    )])?;

                                // Set proper alignment for the alloca based on struct type.
                                // This is critical for types containing i128/u128 which require
                                // 16-byte alignment.
                                let alloca_alignment = self.get_type_alignment_for_size(struct_ty.into()) as u32;
                                if let Some(inst) = alloca.as_instruction() {
                                    let _ = inst.set_alignment(alloca_alignment);
                                }

                                // Restore the original insertion point
                                self.builder.position_at_end(current_block);

                                alloca
                            };

                            // Store tag at field 0
                            let tag_ptr = self.builder.build_struct_gep(alloca, 0, "tag_ptr")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM GEP error: {}", e), self.current_span()
                                )])?;
                            let tag_store = self.builder.build_store(tag_ptr, tag)
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM store error: {}", e), self.current_span()
                                )])?;
                            let _ = tag_store.set_alignment(4); // i32 tag alignment

                            // Get pointer to payload area (field 1)
                            let payload_ptr = self.builder.build_struct_gep(alloca, 1, "payload_ptr")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM GEP error: {}", e), self.current_span()
                                )])?;

                            // Build the actual variant payload struct type
                            let variant_field_types: Vec<BasicTypeEnum> = vals.iter().map(|v| v.get_type()).collect();
                            let variant_struct_ty = self.context.struct_type(&variant_field_types, false);

                            // Cast payload pointer to variant struct pointer
                            let variant_ptr = self.builder.build_pointer_cast(
                                payload_ptr,
                                variant_struct_ty.ptr_type(AddressSpace::default()),
                                "variant_payload_ptr"
                            ).map_err(|e| vec![Diagnostic::error(
                                format!("LLVM pointer cast error: {}", e), self.current_span()
                            )])?;

                            // Store each field
                            for (i, val) in vals.iter().enumerate() {
                                let field_ptr = self.builder.build_struct_gep(variant_ptr, i as u32, &format!("field_{}_ptr", i))
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM GEP error: {}", e), self.current_span()
                                    )])?;
                                let field_store = self.builder.build_store(field_ptr, *val)
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM store error: {}", e), self.current_span()
                                    )])?;
                                // Set proper alignment for enum field store.
                                // Use natural alignment — the alloca has correct alignment set.
                                let alignment = self.get_type_alignment_for_value(*val);
                                let _ = field_store.set_alignment(alignment);
                            }

                            // Load and return the full enum struct
                            let result = self.builder.build_load(alloca, "enum_val")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM load error: {}", e), self.current_span()
                                )])?;
                            // Set proper alignment for the aggregate load.
                            // Use natural alignment — the alloca has correct alignment set.
                            if let Some(inst) = result.as_instruction_value() {
                                let alignment = self.get_type_alignment_for_value(result);
                                let _ = inst.set_alignment(alignment);
                            }
                            Ok(result)
                        }
                    } else {
                        // Enum type is just the tag (all variants are unit variants)
                        Ok(tag.into())
                    }
                } else {
                    Err(vec![Diagnostic::error(
                        format!("Unknown ADT {:?}", def_id), self.current_span()
                    )])
                }
            }

            AggregateKind::Closure { def_id } => {
                let i8_ptr_ty = self.context.i8_type().ptr_type(AddressSpace::default());

                // Get the closure function pointer
                let fn_ptr = if let Some(&fn_value) = self.functions.get(def_id) {
                    self.builder.build_pointer_cast(
                        fn_value.as_global_value().as_pointer_value(),
                        i8_ptr_ty,
                        "closure.fn_ptr"
                    ).map_err(|e| vec![Diagnostic::error(
                        format!("LLVM pointer cast error: {}", e), self.current_span()
                    )])?
                } else {
                    return Err(vec![Diagnostic::error(
                        format!("Closure function {:?} not found", def_id), self.current_span()
                    )]);
                };

                // Check if this closure qualifies for inline environment optimization.
                // Inline closures store captures directly in the closure struct:
                //   { fn_ptr: i8*, capture_0: T0, capture_1: T1, ... }
                // instead of through a separate env_ptr allocation:
                //   { fn_ptr: i8*, env_ptr: i8* }
                // This eliminates one alloca and improves cache locality.
                let use_inline_env = !vals.is_empty()
                    && self.should_inline_closure_env(def_id, dest_local);

                if use_inline_env {
                    // Inline environment: captures stored directly in closure struct.
                    // Struct layout: { fn_ptr: i8*, capture_0: T0, capture_1: T1, ... }
                    let mut field_types: Vec<inkwell::types::BasicTypeEnum> = Vec::with_capacity(vals.len() + 1);
                    field_types.push(i8_ptr_ty.into());
                    for val in vals.iter() {
                        field_types.push(val.get_type());
                    }
                    let inline_struct_ty = self.context.struct_type(&field_types, false);

                    let mut closure_val = inline_struct_ty.get_undef();
                    closure_val = self.builder.build_insert_value(closure_val, fn_ptr, 0, "closure.fn_ptr")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM insert error: {}", e), self.current_span()
                        )])?
                        .into_struct_value();
                    for (i, val) in vals.iter().enumerate() {
                        closure_val = self.builder.build_insert_value(
                            closure_val, *val, (i + 1) as u32,
                            &format!("closure.capture_{}", i)
                        )
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM insert error: {}", e), self.current_span()
                        )])?
                        .into_struct_value();
                    }

                    Ok(closure_val.into())
                } else {
                    // Standard fat pointer: { fn_ptr: i8*, env_ptr: i8* }
                    let closure_struct_ty = self.context.struct_type(
                        &[i8_ptr_ty.into(), i8_ptr_ty.into()], false
                    );

                    // Check if the closure escapes (is returned from a function or stored
                    // in a heap-allocated location). If so, we must heap-allocate the
                    // captured environment so it survives after the creating function returns.
                    let closure_escapes = dest_local
                        .and_then(|local| escape_results.map(|er| er.get(local)))
                        .map(|state| state != EscapeState::NoEscape)
                        .unwrap_or(true); // Conservative: assume escapes if unknown

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
                                    format!("LLVM insert error: {}", e), self.current_span()
                                )])?
                                .into_struct_value();
                        }

                        if closure_escapes {
                            // Closure escapes - heap-allocate the environment
                            let i32_ty = self.context.i32_type();
                            let alloc_fn = self.module.get_function("blood_alloc_or_abort")
                                .ok_or_else(|| vec![Diagnostic::error(
                                    "Runtime function blood_alloc_or_abort not found", self.current_span()
                                )])?;

                            let size_of_captures = captures_struct_ty.size_of()
                                .ok_or_else(|| vec![Diagnostic::error(
                                    "Cannot determine size of closure captures struct", self.current_span()
                                )])?;

                            let gen_alloca = self.builder.build_alloca(i32_ty, "closure_env_gen")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM alloca error: {}", e), self.current_span()
                                )])?;

                            let heap_addr = self.builder.build_call(
                                alloc_fn,
                                &[size_of_captures.into(), gen_alloca.into()],
                                "closure_env_addr"
                            ).map_err(|e| vec![Diagnostic::error(
                                format!("LLVM call error: {}", e), self.current_span()
                            )])?
                            .try_as_basic_value()
                            .left()
                            .ok_or_else(|| vec![Diagnostic::error(
                                "blood_alloc_or_abort did not return a value", self.current_span()
                            )])?
                            .into_int_value();

                            let heap_ptr = self.builder.build_int_to_ptr(
                                heap_addr,
                                captures_struct_ty.ptr_type(AddressSpace::default()),
                                "closure_env_ptr"
                            ).map_err(|e| vec![Diagnostic::error(
                                format!("LLVM int_to_ptr error: {}", e), self.current_span()
                            )])?;

                            self.builder.build_store(heap_ptr, captures_val)
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM store error: {}", e), self.current_span()
                                )])?;

                            self.builder.build_pointer_cast(heap_ptr, i8_ptr_ty, "env_ptr")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM pointer cast error: {}", e), self.current_span()
                                )])?
                        } else {
                            // Closure doesn't escape - use stack allocation
                            let captures_alloca = self.builder.build_alloca(captures_struct_ty, "closure_env")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM alloca error: {}", e), self.current_span()
                                )])?;
                            self.builder.build_store(captures_alloca, captures_val)
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM store error: {}", e), self.current_span()
                                )])?;

                            self.builder.build_pointer_cast(captures_alloca, i8_ptr_ty, "env_ptr")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM pointer cast error: {}", e), self.current_span()
                                )])?
                        }
                    };

                    // Build the closure fat pointer struct { fn_ptr, env_ptr }
                    let mut closure_val = closure_struct_ty.get_undef();
                    closure_val = self.builder.build_insert_value(closure_val, fn_ptr, 0, "closure.with_fn")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM insert error: {}", e), self.current_span()
                        )])?
                        .into_struct_value();
                    closure_val = self.builder.build_insert_value(closure_val, env_ptr, 1, "closure.with_env")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM insert error: {}", e), self.current_span()
                        )])?
                        .into_struct_value();

                    Ok(closure_val.into())
                }
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
                                format!("LLVM insert error: {}", e), self.current_span()
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
                            self.current_span()
                        )]);
                    }
                    let struct_ty = self.context.struct_type(
                        &[elem_ty, elem_ty, self.context.bool_type().into()],
                        false
                    );
                    let mut range_val = struct_ty.get_undef();
                    range_val = self.builder.build_insert_value(range_val, vals[0], 0, "start")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM insert error: {}", e), self.current_span())])?
                        .into_struct_value();
                    range_val = self.builder.build_insert_value(range_val, vals[1], 1, "end")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM insert error: {}", e), self.current_span())])?
                        .into_struct_value();
                    range_val = self.builder.build_insert_value(range_val, vals[2], 2, "exhausted")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM insert error: {}", e), self.current_span())])?
                        .into_struct_value();
                    Ok(range_val.into())
                } else {
                    // Range: { start, end }
                    if vals.len() != 2 {
                        return Err(vec![Diagnostic::error(
                            format!("Range expects 2 fields, got {}", vals.len()),
                            self.current_span()
                        )]);
                    }
                    let struct_ty = self.context.struct_type(&[elem_ty, elem_ty], false);
                    let mut range_val = struct_ty.get_undef();
                    range_val = self.builder.build_insert_value(range_val, vals[0], 0, "start")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM insert error: {}", e), self.current_span())])?
                        .into_struct_value();
                    range_val = self.builder.build_insert_value(range_val, vals[1], 1, "end")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM insert error: {}", e), self.current_span())])?
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
                    if int_ty.get_bit_width() > 64 {
                        // Use arbitrary precision to avoid truncating i128 values to u64
                        let bits = *v as u128;
                        let lo = bits as u64;
                        let hi = (bits >> 64) as u64;
                        Ok(int_ty.const_int_arbitrary_precision(&[lo, hi]).into())
                    } else {
                        Ok(int_ty.const_int(*v as u64, *v < 0).into())
                    }
                } else {
                    Ok(self.context.i64_type().const_int(*v as u64, *v < 0).into())
                }
            }

            ConstantKind::Uint(v) => {
                let llvm_ty = self.lower_type(&constant.ty);
                if let BasicTypeEnum::IntType(int_ty) = llvm_ty {
                    if int_ty.get_bit_width() > 64 {
                        // Use arbitrary precision to avoid truncating u128 values to u64
                        let lo = *v as u64;
                        let hi = (*v >> 64) as u64;
                        Ok(int_ty.const_int_arbitrary_precision(&[lo, hi]).into())
                    } else {
                        Ok(int_ty.const_int(*v as u64, false).into())
                    }
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
                        format!("LLVM string error: {}", e), self.current_span()
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

            ConstantKind::ByteString(bytes) => {
                // Create global byte array constant and byte slice {ptr, len}
                let array_type = self.context.i8_type().array_type(bytes.len() as u32);
                let global = self.module.add_global(array_type, None, "bytes");
                global.set_initializer(&self.context.const_string(bytes, false));
                global.set_constant(true);

                // Cast array pointer to i8*
                let ptr = self.builder.build_pointer_cast(
                    global.as_pointer_value(),
                    self.context.i8_type().ptr_type(inkwell::AddressSpace::default()),
                    "bytes_ptr"
                ).map_err(|e| vec![Diagnostic::error(
                    format!("LLVM pointer cast error: {}", e), self.current_span()
                )])?;
                let len = self.context.i64_type().const_int(bytes.len() as u64, false);

                // Create byte slice struct {ptr, len}
                let slice_type = self.context.struct_type(
                    &[self.context.i8_type().ptr_type(inkwell::AddressSpace::default()).into(),
                      self.context.i64_type().into()],
                    false
                );
                let slice_val = slice_type.const_named_struct(&[ptr.into(), len.into()]);
                Ok(slice_val.into())
            }

            ConstantKind::Unit => {
                Ok(self.context.i8_type().const_int(0, false).into())
            }

            ConstantKind::FnDef(def_id) => {
                // Function reference - get the function pointer
                // Create a fat pointer { fn_ptr, env_ptr } to match the fn() type representation.
                // For plain functions (no captures), env_ptr is null.
                //
                // IMPORTANT: Plain functions are compiled with signature (params...) -> ret,
                // but fn() pointers use calling convention (env_ptr, params...) -> ret.
                // We need a wrapper function that accepts env_ptr and forwards to the original.
                if let Some(wrapper_fn) = self.get_or_create_fn_ptr_wrapper(*def_id) {
                    let fn_ptr = wrapper_fn.as_global_value().as_pointer_value();
                    let ptr_type = self.context.i8_type().ptr_type(inkwell::AddressSpace::default());
                    let null_env = ptr_type.const_null();

                    // Create fat pointer struct { wrapper_fn_ptr, null_env }
                    let fat_ptr_type = self.context.struct_type(&[ptr_type.into(), ptr_type.into()], false);
                    let fat_ptr = fat_ptr_type.const_named_struct(&[fn_ptr.into(), null_env.into()]);
                    Ok(fat_ptr.into())
                } else if self.functions.contains_key(def_id) {
                    // Wrapper creation failed but function exists - this shouldn't happen
                    Err(vec![Diagnostic::error(
                        format!("Failed to create fn pointer wrapper for {:?}", def_id), self.current_span()
                    )])
                } else {
                    Err(vec![Diagnostic::error(
                        format!("Unknown function {:?}", def_id), self.current_span()
                    )])
                }
            }

            ConstantKind::ConstDef(def_id) => {
                // Const item reference - load from global constant
                if let Some(global) = self.const_globals.get(def_id) {
                    let val = self.builder.build_load(global.as_pointer_value(), "const_load")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM load error: {}", e), self.current_span()
                        )])?;
                    Ok(val)
                } else {
                    Err(vec![Diagnostic::error(
                        format!("Unknown const {:?}", def_id), self.current_span()
                    )])
                }
            }

            ConstantKind::StaticDef(def_id) => {
                // Static item reference - load from global variable
                if let Some(global) = self.static_globals.get(def_id) {
                    let val = self.builder.build_load(global.as_pointer_value(), "static_load")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM load error: {}", e), self.current_span()
                        )])?;
                    Ok(val)
                } else {
                    Err(vec![Diagnostic::error(
                        format!("Unknown static {:?}", def_id), self.current_span()
                    )])
                }
            }

            ConstantKind::ZeroSized => {
                Ok(self.context.i8_type().const_int(0, false).into())
            }

            ConstantKind::ConstParam(id) => {
                panic!("unsubstituted const param {:?} in codegen — monomorphization should have replaced this", id);
            }
        }
    }

    /// Get the type of an MIR operand.
    ///
    /// This handles projections on places, returning the type after applying
    /// all projections (e.g., field access on tuples/structs).
    pub(super) fn get_operand_type(&self, operand: &Operand, body: &MirBody) -> Type {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                let base_ty = &body.locals[place.local_unchecked().index() as usize].ty;
                if place.projection.is_empty() {
                    base_ty.clone()
                } else {
                    // Compute the type after applying projections
                    self.compute_place_type(base_ty, &place.projection)
                }
            }
            Operand::Constant(constant) => constant.ty.clone(),
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
