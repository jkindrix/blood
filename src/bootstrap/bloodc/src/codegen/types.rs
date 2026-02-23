//! Type lowering for code generation.
//!
//! This module handles the conversion from HIR types to LLVM types.
//! The main work is done in CodegenContext; this module provides
//! type utilities and helpers.

use crate::hir::{Type, TypeKind, PrimitiveTy};
use crate::ice;

/// Calculate the size of a type in bytes (for layout purposes).
///
/// Returns 0 for zero-sized types (unit, never) and error types.
/// Panics on type variables that should have been resolved before codegen.
pub fn type_size(ty: &Type) -> usize {
    match ty.kind() {
        TypeKind::Primitive(prim) => primitive_size(prim),
        TypeKind::Tuple(types) => types.iter().map(type_size).sum(),
        TypeKind::Array { element, size } => type_size(element) * (size.as_u64().unwrap_or(0) as usize),
        TypeKind::Slice { .. } => 16, // fat pointer (ptr + len)
        TypeKind::Ref { .. } | TypeKind::Ptr { .. } => 8, // 64-bit pointer
        TypeKind::Fn { .. } => 8, // function pointer
        TypeKind::Closure { .. } => 8, // closure (function pointer + captured environment)
        TypeKind::Adt { .. } => 8, // Placeholder - ADT sizes should be computed from field layout
        TypeKind::Range { element, inclusive } => {
            // Range<T>: { start: T, end: T } or RangeInclusive<T>: { start: T, end: T, exhausted: bool }
            let elem_size = type_size(element);
            if *inclusive {
                elem_size * 2 + 1 // start + end + exhausted (bool)
            } else {
                elem_size * 2 // start + end
            }
        }
        TypeKind::Never => 0, // uninhabited type, zero-sized
        TypeKind::Error => 0, // error recovery, treated as zero-sized
        TypeKind::DynTrait { .. } => 16, // fat pointer (data ptr + vtable ptr)
        TypeKind::Infer(var_id) => {
            // Type variable should be resolved before codegen
            ice!("type_size called on unresolved type variable"; "var_id" => var_id);
            0
        }
        TypeKind::Param(param) => {
            // Type parameter should be monomorphized before codegen
            ice!("type_size called on unmonomorphized type parameter"; "param" => param);
            8 // assume pointer-sized for safety
        }
        TypeKind::Record { fields, .. } => {
            // Record size is sum of field sizes (simplified - doesn't account for padding)
            fields.iter().map(|f| type_size(&f.ty)).sum()
        }
        TypeKind::Forall { body, .. } => {
            // Forall types should be instantiated before codegen
            // Use body type size as fallback
            type_size(body)
        }
        TypeKind::Ownership { inner, .. } => {
            // Ownership qualifiers are compile-time only
            // Runtime size is the same as the inner type
            type_size(inner)
        }
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
        PrimitiveTy::Never => 0, // never type is zero-sized
    }
}

/// Calculate alignment requirements for a type.
///
/// Returns 1 for zero-sized types and error types.
/// Reports ICE for type variables that should have been resolved before codegen.
pub fn type_alignment(ty: &Type) -> usize {
    match ty.kind() {
        TypeKind::Primitive(prim) => primitive_size(prim).max(1),
        TypeKind::Tuple(types) => types.iter()
            .map(type_alignment)
            .max()
            .unwrap_or(1),
        TypeKind::Array { element, .. } => type_alignment(element),
        TypeKind::Slice { .. } => 8, // fat pointer alignment
        TypeKind::Ref { .. } | TypeKind::Ptr { .. } => 8,
        TypeKind::Fn { .. } => 8, // function pointer alignment
        TypeKind::Closure { .. } => 8, // closure alignment
        TypeKind::Adt { .. } => 8, // conservative default - should compute from fields
        TypeKind::Range { element, .. } => type_alignment(element), // align to element
        TypeKind::Never => 1, // zero-sized, minimal alignment
        TypeKind::Error => 1, // error recovery
        TypeKind::DynTrait { .. } => 8, // fat pointer alignment
        TypeKind::Infer(var_id) => {
            ice!("type_alignment called on unresolved type variable"; "var_id" => var_id);
            8 // conservative default
        }
        TypeKind::Param(param) => {
            ice!("type_alignment called on unmonomorphized type parameter"; "param" => param);
            8 // conservative default
        }
        TypeKind::Record { fields, .. } => {
            // Record alignment is max of field alignments
            fields.iter()
                .map(|f| type_alignment(&f.ty))
                .max()
                .unwrap_or(1)
        }
        TypeKind::Forall { body, .. } => {
            // Forall types should be instantiated before codegen
            // Use body type alignment as fallback
            type_alignment(body)
        }
        TypeKind::Ownership { inner, .. } => {
            // Ownership qualifiers are compile-time only
            // Runtime alignment is the same as the inner type
            type_alignment(inner)
        }
    }
}
