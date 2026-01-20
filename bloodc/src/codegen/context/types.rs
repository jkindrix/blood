//! Type lowering for LLVM codegen.
//!
//! This module handles conversion from Blood types to LLVM types.

use inkwell::types::{BasicTypeEnum, BasicType, BasicMetadataTypeEnum};
use inkwell::AddressSpace;

use crate::hir::{Type, TypeKind, PrimitiveTy};
use crate::hir::def::{IntTy, UintTy};

use super::CodegenContext;

impl<'ctx, 'a> CodegenContext<'ctx, 'a> {
    /// Lower a Blood type to an LLVM type.
    pub fn lower_type(&self, ty: &Type) -> BasicTypeEnum<'ctx> {
        match ty.kind() {
            TypeKind::Primitive(prim) => self.lower_primitive(prim),
            TypeKind::Tuple(fields) if fields.is_empty() => {
                // Unit type - use i8 as placeholder
                self.context.i8_type().into()
            }
            TypeKind::Tuple(fields) => {
                let field_types: Vec<BasicTypeEnum> = fields.iter()
                    .map(|f| self.lower_type(f))
                    .collect();
                self.context.struct_type(&field_types, false).into()
            }
            TypeKind::Array { element, size } => {
                let elem_type = self.lower_type(element);
                elem_type.array_type(*size as u32).into()
            }
            TypeKind::Slice { element: _ } => {
                // Slices are { ptr, len }
                // Use opaque pointer (i8*) for the element pointer to avoid recursive
                // type lowering. This is correct because slices store a pointer to
                // elements, not the elements themselves, and modern LLVM uses opaque
                // pointers anyway.
                let ptr_type = self.context.i8_type().ptr_type(AddressSpace::default());
                let len_type = self.context.i64_type();
                self.context.struct_type(&[ptr_type.into(), len_type.into()], false).into()
            }
            TypeKind::Ref { .. } | TypeKind::Ptr { .. } => {
                // Use opaque pointer (i8*) for all reference and pointer types.
                // This avoids infinite recursion for recursive types like linked lists
                // and is consistent with modern LLVM's opaque pointer semantics.
                self.context.i8_type().ptr_type(AddressSpace::default()).into()
            }
            TypeKind::Adt { def_id, args } => {
                // Look up struct or enum definition
                if let Some(fields) = self.struct_defs.get(def_id) {
                    // Substitute type parameters with concrete type arguments
                    let field_types: Vec<BasicTypeEnum> = fields.iter()
                        .map(|f| {
                            let substituted = self.substitute_type_params(f, args);
                            self.lower_type(&substituted)
                        })
                        .collect();
                    if field_types.is_empty() {
                        // Unit struct - use i8 placeholder
                        self.context.i8_type().into()
                    } else {
                        self.context.struct_type(&field_types, false).into()
                    }
                } else if let Some(variants) = self.enum_defs.get(def_id) {
                    // Enum: { i32 tag, payload_fields... }
                    // For generic enums, substitute type parameters with concrete args
                    // Find the variant with the largest substituted fields
                    let mut max_variant_types: Vec<BasicTypeEnum> = Vec::new();
                    let mut max_size = 0usize;

                    for variant_fields in variants {
                        let mut variant_types: Vec<BasicTypeEnum> = Vec::new();
                        let mut variant_size = 0usize;

                        for field_ty in variant_fields {
                            let substituted = self.substitute_type_params(field_ty, args);
                            let llvm_ty = self.lower_type(&substituted);
                            // Approximate size by type bit width
                            let size = match llvm_ty {
                                BasicTypeEnum::IntType(t) => t.get_bit_width() as usize,
                                BasicTypeEnum::FloatType(_) => 64,
                                BasicTypeEnum::PointerType(_) => 64,
                                BasicTypeEnum::StructType(t) => t.count_fields() as usize * 64,
                                BasicTypeEnum::ArrayType(t) => t.len() as usize * 64,
                                BasicTypeEnum::VectorType(_) => 128,
                            };
                            variant_types.push(llvm_ty);
                            variant_size += size;
                        }

                        if variant_size > max_size {
                            max_size = variant_size;
                            max_variant_types = variant_types;
                        }
                    }

                    let mut enum_fields: Vec<BasicTypeEnum> = Vec::with_capacity(1 + max_variant_types.len());
                    // Tag
                    enum_fields.push(self.context.i32_type().into());
                    // Payload fields from largest variant
                    enum_fields.extend(max_variant_types);

                    if enum_fields.len() == 1 {
                        // No payload fields - just tag
                        self.context.i32_type().into()
                    } else {
                        self.context.struct_type(&enum_fields, false).into()
                    }
                } else {
                    // Unknown ADT - use pointer placeholder
                    self.context.i8_type().ptr_type(AddressSpace::default()).into()
                }
            }
            TypeKind::Fn { .. } => {
                // Function types are fat pointers: { fn_ptr, env_ptr }
                // This allows closures with captures to be passed as fn() parameters.
                // When a regular function (no captures) is passed, env_ptr is null.
                // When calling through fn(), the env_ptr is passed as the first argument.
                let ptr_type = self.context.i8_type().ptr_type(AddressSpace::default());
                self.context.struct_type(&[ptr_type.into(), ptr_type.into()], false).into()
            }
            TypeKind::DynTrait { .. } => {
                // Trait object: fat pointer { data_ptr, vtable_ptr }
                let ptr_type = self.context.i8_type().ptr_type(AddressSpace::default());
                self.context.struct_type(&[ptr_type.into(), ptr_type.into()], false).into()
            }
            TypeKind::Never => {
                // Never type - use i8 as placeholder (will never actually be used)
                self.context.i8_type().into()
            }
            TypeKind::Infer(_) | TypeKind::Error => {
                // Error/unresolved types - use i8 placeholder
                self.context.i8_type().into()
            }
            TypeKind::Param(_) => {
                // Generic parameters should be monomorphized before codegen
                // Use pointer as placeholder for now
                self.context.i8_type().ptr_type(AddressSpace::default()).into()
            }
            TypeKind::Closure { .. } => {
                // Closure: fat pointer { fn_ptr, env_ptr }
                let ptr_type = self.context.i8_type().ptr_type(AddressSpace::default());
                self.context.struct_type(&[ptr_type.into(), ptr_type.into()], false).into()
            }
            TypeKind::Range { element, .. } => {
                // Range: { start, end } of the element type
                // For simplicity, use { i64, i64 } for now
                let elem_type = self.lower_type(element);
                self.context.struct_type(&[elem_type, elem_type], false).into()
            }
            TypeKind::Record { fields, .. } => {
                // Record: anonymous struct with named fields
                if fields.is_empty() {
                    // Empty record - use i8 placeholder
                    self.context.i8_type().into()
                } else {
                    let field_types: Vec<BasicTypeEnum> = fields.iter()
                        .map(|f| self.lower_type(&f.ty))
                        .collect();
                    self.context.struct_type(&field_types, false).into()
                }
            }
            TypeKind::Forall { body, .. } => {
                // Forall types should be instantiated before codegen.
                // Lower the body type as fallback.
                self.lower_type(body)
            }
            TypeKind::Ownership { inner, .. } => {
                // Ownership qualifiers are compile-time only.
                // Runtime representation is the same as the inner type.
                self.lower_type(inner)
            }
        }
    }

    /// Lower a primitive type to LLVM.
    pub(super) fn lower_primitive(&self, prim: &PrimitiveTy) -> BasicTypeEnum<'ctx> {
        match prim {
            PrimitiveTy::Bool => self.context.bool_type().into(),
            PrimitiveTy::Char => self.context.i32_type().into(),
            PrimitiveTy::Int(int_ty) => match int_ty {
                IntTy::I8 => self.context.i8_type().into(),
                IntTy::I16 => self.context.i16_type().into(),
                IntTy::I32 => self.context.i32_type().into(),
                IntTy::I64 => self.context.i64_type().into(),
                IntTy::I128 => self.context.i128_type().into(),
                IntTy::Isize => self.context.i64_type().into(),
            },
            PrimitiveTy::Uint(uint_ty) => match uint_ty {
                UintTy::U8 => self.context.i8_type().into(),
                UintTy::U16 => self.context.i16_type().into(),
                UintTy::U32 => self.context.i32_type().into(),
                UintTy::U64 => self.context.i64_type().into(),
                UintTy::U128 => self.context.i128_type().into(),
                UintTy::Usize => self.context.i64_type().into(),
            },
            PrimitiveTy::Float(crate::hir::def::FloatTy::F32) => self.context.f32_type().into(),
            PrimitiveTy::Float(crate::hir::def::FloatTy::F64) => self.context.f64_type().into(),
            PrimitiveTy::Str => {
                // String slice: { ptr, len }
                let ptr_type = self.context.i8_type().ptr_type(AddressSpace::default());
                let len_type = self.context.i64_type();
                self.context.struct_type(&[ptr_type.into(), len_type.into()], false).into()
            }
            PrimitiveTy::String => {
                // Heap-allocated string: { ptr, len, capacity }
                let ptr_type = self.context.i8_type().ptr_type(AddressSpace::default());
                let len_type = self.context.i64_type();
                self.context.struct_type(&[ptr_type.into(), len_type.into(), len_type.into()], false).into()
            }
            PrimitiveTy::Unit => {
                // Unit type - use i8 as placeholder
                self.context.i8_type().into()
            }
            PrimitiveTy::Never => {
                // Never type (!) - use void-like type
                // This should never be instantiated at runtime
                self.context.i8_type().into()
            }
        }
    }

    /// Create an LLVM function type from a Blood function signature.
    pub(super) fn fn_type_from_sig(&self, sig: &crate::hir::FnSig) -> inkwell::types::FunctionType<'ctx> {
        let param_types: Vec<BasicMetadataTypeEnum> = sig.inputs.iter()
            .map(|p| self.lower_type(p).into())
            .collect();

        if sig.output.is_unit() {
            self.context.void_type().fn_type(&param_types, false)
        } else {
            let ret_type = self.lower_type(&sig.output);
            ret_type.fn_type(&param_types, false)
        }
    }

    /// Substitute type parameters in a type with concrete type arguments.
    ///
    /// This is used for monomorphizing generic types during codegen.
    /// Type parameters are indexed positionally, so `Param(0)` gets args[0], etc.
    pub(crate) fn substitute_type_params(&self, ty: &Type, args: &[Type]) -> Type {
        match ty.kind() {
            TypeKind::Param(idx) => {
                // Substitute type parameter with concrete argument
                // TyVarId.0 gives the u32 index
                if let Some(arg) = args.get(idx.0 as usize) {
                    arg.clone()
                } else {
                    // No substitution available, return as-is
                    ty.clone()
                }
            }
            TypeKind::Tuple(fields) => {
                let substituted: Vec<Type> = fields.iter()
                    .map(|f| self.substitute_type_params(f, args))
                    .collect();
                Type::tuple(substituted)
            }
            TypeKind::Array { element, size } => {
                let substituted = self.substitute_type_params(element, args);
                Type::array(substituted, *size)
            }
            TypeKind::Slice { element } => {
                let substituted = self.substitute_type_params(element, args);
                Type::slice(substituted)
            }
            TypeKind::Ref { inner, mutable } => {
                let substituted = self.substitute_type_params(inner, args);
                Type::reference(substituted, *mutable)
            }
            TypeKind::Ptr { inner, mutable } => {
                let substituted = self.substitute_type_params(inner, args);
                Type::new(TypeKind::Ptr { inner: substituted, mutable: *mutable })
            }
            TypeKind::Adt { def_id, args: inner_args } => {
                // Recursively substitute in the ADT's own type arguments
                let substituted_args: Vec<Type> = inner_args.iter()
                    .map(|a| self.substitute_type_params(a, args))
                    .collect();
                Type::adt(*def_id, substituted_args)
            }
            TypeKind::Fn { params, ret, .. } => {
                let substituted_params: Vec<Type> = params.iter()
                    .map(|p| self.substitute_type_params(p, args))
                    .collect();
                let substituted_ret = self.substitute_type_params(ret, args);
                Type::function(substituted_params, substituted_ret)
            }
            // Primitive types and other non-generic types return as-is
            _ => ty.clone(),
        }
    }
}
