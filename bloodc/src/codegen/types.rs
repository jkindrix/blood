//! Type lowering for code generation.
//!
//! This module handles the conversion from HIR types to LLVM types.
//! The main work is done in CodegenContext; this module provides
//! type utilities and helpers.

use crate::hir::{Type, TypeKind, PrimitiveTy};

/// Calculate the size of a type in bytes (for layout purposes).
pub fn type_size(ty: &Type) -> usize {
    match ty.kind() {
        TypeKind::Primitive(prim) => primitive_size(prim),
        TypeKind::Tuple(types) => types.iter().map(type_size).sum(),
        TypeKind::Array { element, size } => type_size(element) * (*size as usize),
        TypeKind::Ref { .. } | TypeKind::Ptr { .. } => 8, // 64-bit pointer
        TypeKind::Adt { .. } => 8, // Placeholder
        _ => 0,
    }
}

/// Calculate the size of a primitive type.
fn primitive_size(prim: &PrimitiveTy) -> usize {
    match prim {
        PrimitiveTy::Bool => 1,
        PrimitiveTy::Char => 4,
        PrimitiveTy::Int(int_ty) => match int_ty {
            crate::hir::def::IntTy::I8 => 1,
            crate::hir::def::IntTy::I16 => 2,
            crate::hir::def::IntTy::I32 => 4,
            crate::hir::def::IntTy::I64 => 8,
            crate::hir::def::IntTy::I128 => 16,
            crate::hir::def::IntTy::Isize => 8,
        },
        PrimitiveTy::Uint(uint_ty) => match uint_ty {
            crate::hir::def::UintTy::U8 => 1,
            crate::hir::def::UintTy::U16 => 2,
            crate::hir::def::UintTy::U32 => 4,
            crate::hir::def::UintTy::U64 => 8,
            crate::hir::def::UintTy::U128 => 16,
            crate::hir::def::UintTy::Usize => 8,
        },
        PrimitiveTy::Float(float_ty) => match float_ty {
            crate::hir::def::FloatTy::F32 => 4,
            crate::hir::def::FloatTy::F64 => 8,
        },
        PrimitiveTy::Str => 16, // fat pointer (ptr + len)
        PrimitiveTy::String => 24, // ptr + len + cap
        PrimitiveTy::Unit => 0, // zero-sized type
    }
}

/// Calculate alignment requirements for a type.
pub fn type_alignment(ty: &Type) -> usize {
    match ty.kind() {
        TypeKind::Primitive(prim) => primitive_size(prim).max(1),
        TypeKind::Tuple(types) => types.iter()
            .map(type_alignment)
            .max()
            .unwrap_or(1),
        TypeKind::Array { element, .. } => type_alignment(element),
        TypeKind::Ref { .. } | TypeKind::Ptr { .. } => 8,
        _ => 8,
    }
}
