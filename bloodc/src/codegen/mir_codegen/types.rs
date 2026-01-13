//! MIR type-related code generation utilities.
//!
//! This module provides helper functions for type analysis and size computation
//! used in MIR code generation.

use inkwell::types::BasicTypeEnum;

use crate::hir::{Type, TypeKind};

use super::CodegenContext;

/// Extension trait for MIR type-related operations.
pub trait MirTypesCodegen<'ctx, 'a> {
    /// Get the size of an LLVM type in bytes.
    fn get_type_size_in_bytes(&self, ty: BasicTypeEnum<'ctx>) -> u64;
}

impl<'ctx, 'a> MirTypesCodegen<'ctx, 'a> for CodegenContext<'ctx, 'a> {
    fn get_type_size_in_bytes(&self, ty: BasicTypeEnum<'ctx>) -> u64 {
        match ty {
            BasicTypeEnum::IntType(t) => (t.get_bit_width() as u64).div_ceil(8),
            BasicTypeEnum::FloatType(_) => 4,
            BasicTypeEnum::PointerType(_) => 8, // 64-bit pointers
            BasicTypeEnum::ArrayType(t) => {
                let elem_size = self.get_type_size_in_bytes(t.get_element_type());
                elem_size * t.len() as u64
            }
            BasicTypeEnum::StructType(t) => {
                // Sum of field sizes (simplified - doesn't account for padding)
                let mut size = 0u64;
                for i in 0..t.count_fields() {
                    if let Some(field_ty) = t.get_field_type_at_index(i) {
                        size += self.get_type_size_in_bytes(field_ty);
                    }
                }
                size.max(1) // At least 1 byte for empty struct
            }
            BasicTypeEnum::VectorType(t) => {
                let elem_size = self.get_type_size_in_bytes(t.get_element_type());
                elem_size * t.get_size() as u64
            }
        }
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
    }
}
