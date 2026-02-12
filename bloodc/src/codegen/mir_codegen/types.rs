//! MIR type-related code generation utilities.
//!
//! This module provides helper functions for type analysis and size computation
//! used in MIR code generation.

use inkwell::types::BasicTypeEnum;
use inkwell::values::BasicValueEnum;

use crate::hir::{Type, TypeKind};

use super::CodegenContext;

/// Extension trait for MIR type-related operations.
pub trait MirTypesCodegen<'ctx, 'a> {
    /// Get the size of an LLVM type in bytes.
    fn get_type_size_in_bytes(&self, ty: BasicTypeEnum<'ctx>) -> u64;

    /// Get the alignment of an LLVM type in bytes.
    fn get_type_alignment_for_size(&self, ty: BasicTypeEnum<'ctx>) -> u64;

    /// Get the alignment of a value based on its type.
    /// Returns alignment suitable for store/load instructions.
    fn get_type_alignment_for_value(&self, val: BasicValueEnum<'ctx>) -> u32;
}

impl<'ctx, 'a> MirTypesCodegen<'ctx, 'a> for CodegenContext<'ctx, 'a> {
    fn get_type_size_in_bytes(&self, ty: BasicTypeEnum<'ctx>) -> u64 {
        // Use LLVM's size_of() when available - this ensures we match LLVM's actual layout
        // which is critical for operations like memcpy in vec_push
        let size_opt: Option<inkwell::values::IntValue<'ctx>> = match ty {
            BasicTypeEnum::ArrayType(t) => t.size_of(),
            BasicTypeEnum::FloatType(t) => Some(t.size_of()),
            BasicTypeEnum::IntType(t) => Some(t.size_of()),
            BasicTypeEnum::PointerType(t) => Some(t.size_of()),
            BasicTypeEnum::StructType(t) => t.size_of(),
            BasicTypeEnum::VectorType(t) => t.size_of(),
        };
        if let Some(size_val) = size_opt {
            // LLVM's size_of returns a constant IntValue - extract the constant
            if let Some(size) = size_val.get_zero_extended_constant() {
                return size;
            }
        }

        // Fallback to manual calculation if LLVM size_of isn't available
        match ty {
            BasicTypeEnum::IntType(t) => (t.get_bit_width() as u64).div_ceil(8),
            BasicTypeEnum::FloatType(t) => {
                // LLVM FloatType can be various sizes (16-bit half, 32-bit float, 64-bit double, etc.)
                // We compare against known types from the context to determine the size.
                let f32_ty = self.context.f32_type();
                let f64_ty = self.context.f64_type();
                let f128_ty = self.context.f128_type();
                let f16_ty = self.context.f16_type();

                if t == f64_ty {
                    8  // 64-bit double
                } else if t == f32_ty {
                    4  // 32-bit float
                } else if t == f128_ty {
                    16 // 128-bit quad precision
                } else if t == f16_ty {
                    2  // 16-bit half
                } else {
                    // Unknown float type - use target data layout as fallback
                    // For safety, assume 8 bytes (double) as conservative default
                    8
                }
            }
            BasicTypeEnum::PointerType(_) => 8, // 64-bit pointers
            BasicTypeEnum::ArrayType(t) => {
                let elem_size = self.get_type_size_in_bytes(t.get_element_type());
                elem_size * t.len() as u64
            }
            BasicTypeEnum::StructType(t) => {
                // Compute struct size with proper alignment padding
                let mut offset = 0u64;
                let mut max_align = 1u64;

                for i in 0..t.count_fields() {
                    if let Some(field_ty) = t.get_field_type_at_index(i) {
                        let field_size = self.get_type_size_in_bytes(field_ty);
                        let field_align = self.get_type_alignment_for_size(field_ty);

                        // Update max alignment
                        if field_align > max_align {
                            max_align = field_align;
                        }

                        // Add padding to align this field
                        let padding = (field_align - (offset % field_align)) % field_align;
                        offset += padding + field_size;
                    }
                }

                // Add final padding to round up to struct alignment
                let final_padding = (max_align - (offset % max_align)) % max_align;
                (offset + final_padding).max(1)
            }
            BasicTypeEnum::VectorType(t) => {
                let elem_size = self.get_type_size_in_bytes(t.get_element_type());
                elem_size * t.get_size() as u64
            }
        }
    }

    fn get_type_alignment_for_size(&self, ty: BasicTypeEnum<'ctx>) -> u64 {
        match ty {
            BasicTypeEnum::IntType(int_ty) => {
                let bits = int_ty.get_bit_width();
                // LLVM 14's default x86_64 data layout does NOT include i128:128,
                // so i128 has ABI alignment of 8 bytes (same as i64).
                // We MUST match this because:
                //   1. LLVM 14's C API resets the module data layout to the
                //      TargetMachine's default during LLVMTargetMachineEmitToFile
                //   2. GEP offsets are computed using the TargetMachine's layout
                //   3. If we annotate loads/stores with align 16 but GEP computes
                //      offset 8 for i128 in a struct, the actual address is only
                //      8-byte aligned, causing SIGSEGV from aligned SSE/AVX moves
                // Alignment is min(size, 8) bytes for all integer types.
                std::cmp::min((bits as u64).div_ceil(8), 8).max(1)
            }
            BasicTypeEnum::FloatType(float_ty) => {
                if float_ty == self.context.f32_type() {
                    4
                } else if float_ty == self.context.f16_type() {
                    2
                } else {
                    8
                }
            }
            BasicTypeEnum::PointerType(_) => 8,
            BasicTypeEnum::ArrayType(arr_ty) => {
                // Array alignment is the alignment of its element type
                self.get_type_alignment_for_size(arr_ty.get_element_type())
            }
            BasicTypeEnum::StructType(st) => {
                // Struct alignment is the max alignment of any field
                let mut max_align = 1u64;
                for i in 0..st.count_fields() {
                    if let Some(field_ty) = st.get_field_type_at_index(i) {
                        let field_align = self.get_type_alignment_for_size(field_ty);
                        if field_align > max_align {
                            max_align = field_align;
                        }
                    }
                }
                max_align
            }
            BasicTypeEnum::VectorType(_) => 16,
        }
    }

    fn get_type_alignment_for_value(&self, val: BasicValueEnum<'ctx>) -> u32 {
        // Get the type of the value and compute alignment
        let ty = match val {
            BasicValueEnum::IntValue(v) => BasicTypeEnum::IntType(v.get_type()),
            BasicValueEnum::FloatValue(v) => BasicTypeEnum::FloatType(v.get_type()),
            BasicValueEnum::PointerValue(v) => BasicTypeEnum::PointerType(v.get_type()),
            BasicValueEnum::StructValue(v) => BasicTypeEnum::StructType(v.get_type()),
            BasicValueEnum::ArrayValue(v) => BasicTypeEnum::ArrayType(v.get_type()),
            BasicValueEnum::VectorValue(v) => BasicTypeEnum::VectorType(v.get_type()),
        };
        self.get_type_alignment_for_size(ty) as u32
    }
}

/// Check if a type may contain generational references.
///
/// Used to determine which locals need snapshot capture during
/// effect operations.
pub(super) fn type_may_contain_genref_impl(ty: &Type) -> bool {
    match &*ty.kind {
        // Pointer and reference types may contain generational references
        TypeKind::Ptr { .. } | TypeKind::Ref { .. } => true,

        // Arrays and slices may contain genrefs if their element type does
        TypeKind::Array { ref element, .. } => type_may_contain_genref_impl(element),
        TypeKind::Slice { ref element } => type_may_contain_genref_impl(element),

        // Tuples may contain genrefs if any element does
        TypeKind::Tuple(elems) => elems.iter().any(type_may_contain_genref_impl),

        // ADTs (structs, enums) might contain genrefs - be conservative
        TypeKind::Adt { .. } => true,

        // Functions might capture genrefs (as function pointers to closures)
        TypeKind::Fn { .. } => true,

        // Closures might capture genrefs in their environment
        TypeKind::Closure { .. } => true,

        // Range types may contain genrefs if their element type does
        TypeKind::Range { ref element, .. } => type_may_contain_genref_impl(element),

        // Primitives don't contain genrefs
        TypeKind::Primitive(_) => false,

        // Never and Error types don't contain genrefs
        TypeKind::Never | TypeKind::Error => false,

        // Trait objects may contain genrefs (data pointer)
        TypeKind::DynTrait { .. } => true,

        // Inference and type parameters - be conservative
        TypeKind::Infer(_) | TypeKind::Param(_) => true,

        // Records may contain genrefs if any field does
        TypeKind::Record { fields, .. } => fields.iter().any(|f| type_may_contain_genref_impl(&f.ty)),

        // Forall types may contain genrefs if body does
        TypeKind::Forall { body, .. } => type_may_contain_genref_impl(body),

        // Ownership-qualified types delegate to the inner type
        TypeKind::Ownership { inner, .. } => type_may_contain_genref_impl(inner),
    }
}
