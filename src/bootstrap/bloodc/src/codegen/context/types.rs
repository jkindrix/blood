//! Type lowering for LLVM codegen.
//!
//! This module handles conversion from Blood types to LLVM types.

use inkwell::types::{BasicTypeEnum, BasicType, BasicMetadataTypeEnum};
use inkwell::AddressSpace;

use crate::hir::{Type, TypeKind, PrimitiveTy};
use crate::hir::def::{IntTy, UintTy};
use crate::ice;

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
                // Get concrete size - should be monomorphized by this point
                let concrete_size = size.as_u64().unwrap_or_else(|| {
                    // If we hit a const param in codegen, something went wrong with monomorphization
                    ice!("array size must be concrete in codegen"; "size" => size);
                    0
                });
                elem_type.array_type(concrete_size as u32).into()
            }
            TypeKind::Slice { element } => {
                // Slices are { ptr, len }
                let _elem_type = self.lower_type(element);
                let ptr_type = self.context.ptr_type(AddressSpace::default());
                let len_type = self.context.i64_type();
                self.context.struct_type(&[ptr_type.into(), len_type.into()], false).into()
            }
            TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                // For unsized types (str, slices), the reference IS the fat pointer
                // (there's no extra indirection - &str = {ptr, len}, not ptr to {ptr, len})
                match inner.kind() {
                    TypeKind::Primitive(PrimitiveTy::Str) => {
                        // &str is just the {ptr, len} fat pointer
                        self.lower_primitive(&PrimitiveTy::Str)
                    }
                    TypeKind::Slice { .. } => {
                        // &[T] is just the {ptr, len} fat pointer
                        self.lower_type(inner)
                    }
                    _ => {
                        // Regular references are pointers to the inner type
                        // (opaque pointers: all pointer types are just `ptr`)
                        let _inner_type = self.lower_type(inner);
                        self.context.ptr_type(AddressSpace::default()).into()
                    }
                }
            }
            TypeKind::Adt { def_id, args } => {
                // Check for built-in types first (these have special representations)
                if Some(*def_id) == self.box_def_id {
                    // Box<T> is represented as an opaque pointer
                    self.context.ptr_type(AddressSpace::default()).into()
                } else if Some(*def_id) == self.vec_def_id {
                    // Vec<T> is { ptr: *T, len: i64, capacity: i64 }
                    // All pointers are opaque `ptr` in LLVM 18
                    let ptr_type = self.context.ptr_type(AddressSpace::default());
                    let i64_type = self.context.i64_type();
                    self.context.struct_type(&[ptr_type.into(), i64_type.into(), i64_type.into()], false).into()
                } else if Some(*def_id) == self.option_def_id {
                    // Option<T> is { tag: i32, payload: T }
                    let tag_type = self.context.i32_type();
                    if let Some(inner_ty) = args.first() {
                        let payload_type = self.lower_type(inner_ty);
                        self.context.struct_type(&[tag_type.into(), payload_type], false).into()
                    } else {
                        // Option with no type arg - just tag
                        tag_type.into()
                    }
                } else if Some(*def_id) == self.result_def_id {
                    // Result<T, E> is { tag: i32, payload: max(T, E) }
                    let tag_type = self.context.i32_type();
                    let ok_size = if let Some(ok_ty) = args.first() {
                        let llvm_ty = self.lower_type(ok_ty);
                        self.get_type_size_approx(llvm_ty)
                    } else { 0 };
                    let err_size = if args.len() > 1 {
                        let llvm_ty = self.lower_type(&args[1]);
                        self.get_type_size_approx(llvm_ty)
                    } else { 0 };
                    let payload_type = if ok_size >= err_size {
                        args.first().map(|t| self.lower_type(t))
                    } else {
                        args.get(1).map(|t| self.lower_type(t))
                    };
                    if let Some(payload) = payload_type {
                        self.context.struct_type(&[tag_type.into(), payload], false).into()
                    } else {
                        tag_type.into()
                    }
                } else if self.struct_defs.contains_key(def_id) || self.enum_defs.contains_key(def_id) {
                    // Cycle detection for recursive types (e.g. enum List { Cons(i32, List), Nil }).
                    // If we're already lowering this ADT, we've hit a recursive cycle.
                    // Recursive types have infinite size and cannot be stack-allocated;
                    // return a pointer placeholder. The type checker should have caught
                    // this, but we handle it gracefully to avoid stack overflow.
                    if !self.lowering_adts.borrow_mut().insert(ty.clone()) {
                        // Already lowering this exact ADT instantiation → true recursive cycle.
                        // Look up the type name for a useful error message.
                        let type_name = self.def_paths.get(def_id)
                            .cloned()
                            .unwrap_or_else(|| format!("{:?}", def_id));
                        self.type_lowering_errors.borrow_mut().push(
                            crate::diagnostics::Diagnostic::error(
                                format!(
                                    "recursive type `{}` has infinite size; \
                                     consider using indirection (e.g., Box<{}>)",
                                    type_name, type_name
                                ),
                                self.current_span(),
                            )
                        );
                        return self.context.ptr_type(AddressSpace::default()).into();
                    }

                    let result = if let Some(fields) = self.struct_defs.get(def_id).cloned() {
                    // Look up struct definition
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
                    } else if let Some(variants) = self.enum_defs.get(def_id).cloned() {
                        // Enum: { i32 tag, payload }
                        // The payload must have:
                        // 1. Enough size to hold the largest variant
                        // 2. Alignment at least as high as the most-aligned variant
                        //
                        // IMPORTANT: We track both max_size AND max_alignment separately because
                        // the largest variant might not have the highest alignment requirement.
                        // For example: Variant A = {i32, i32, i32} (12 bytes, 4-byte align)
                        //              Variant B = {i64} (8 bytes, 8-byte align)
                        // Using A's type as payload would misalign B's i64 access.
                        //
                        // Solution: Use an array of the highest-alignment type, sized to fit
                        // the largest variant. This ensures proper alignment for all variants.
                        let mut max_payload_size: u64 = 0;
                        let mut max_alignment: u32 = 1;
                        let mut has_payload = false;

                        for variant_fields in &variants {
                            if !variant_fields.is_empty() {
                                has_payload = true;
                                // Build the variant's struct type to get its size and alignment
                                let variant_field_types: Vec<BasicTypeEnum> = variant_fields.iter()
                                    .map(|field_ty| {
                                        let substituted = self.substitute_type_params(field_ty, args);
                                        self.lower_type(&substituted)
                                    })
                                    .collect();
                                let variant_struct_ty = self.context.struct_type(&variant_field_types, false);
                                let variant_size = self.get_type_size_approx(variant_struct_ty.into()) as u64;
                                let variant_align = self.get_max_field_alignment(&variant_field_types);

                                if variant_size > max_payload_size {
                                    max_payload_size = variant_size;
                                }
                                if variant_align > max_alignment {
                                    max_alignment = variant_align;
                                }
                            }
                        }

                        if !has_payload {
                            // No payload fields in any variant - just tag
                            self.context.i32_type().into()
                        } else {
                            // Create { i32 tag, aligned_payload }
                            // Use an array of the appropriately-aligned type to ensure
                            // proper alignment for all variant payloads.
                            let tag_type = self.context.i32_type();
                            let payload_type = if max_alignment >= 16 {
                                // Need 16-byte alignment: use [i128 x N] for i128 fields
                                let num_i128s = max_payload_size.div_ceil(16);
                                self.context.i128_type().array_type(num_i128s as u32).into()
                            } else if max_alignment >= 8 {
                                // Need 8-byte alignment: use [i64 x N]
                                let num_i64s = max_payload_size.div_ceil(8);
                                self.context.i64_type().array_type(num_i64s as u32).into()
                            } else if max_alignment >= 4 {
                                // Need 4-byte alignment: use [i32 x N]
                                let num_i32s = max_payload_size.div_ceil(4);
                                self.context.i32_type().array_type(num_i32s as u32).into()
                            } else if max_alignment >= 2 {
                                // Need 2-byte alignment: use [i16 x N]
                                let num_i16s = max_payload_size.div_ceil(2);
                                self.context.i16_type().array_type(num_i16s as u32).into()
                            } else {
                                // 1-byte alignment: use [i8 x N]
                                self.context.i8_type().array_type(max_payload_size as u32).into()
                            };
                            self.context.struct_type(&[tag_type.into(), payload_type], false).into()
                        }
                    } else {
                        unreachable!()
                    };

                    // Remove from cycle detection set
                    self.lowering_adts.borrow_mut().remove(ty);
                    result
                } else {
                    // Unknown ADT at codegen time is a compiler bug — panic with diagnostic info
                    panic!(
                        "ICE: Unknown ADT in codegen: def_id={:?}, type_args={:?}. \
                         Known structs: {}, known enums: {}",
                        def_id, args, self.struct_defs.len(), self.enum_defs.len()
                    );
                }
            }
            TypeKind::Fn { .. } => {
                // Function types are fat pointers: { fn_ptr, env_ptr }
                // This allows closures with captures to be passed as fn() parameters.
                // When a regular function (no captures) is passed, env_ptr is null.
                // When calling through fn(), the env_ptr is passed as the first argument.
                let ptr_type = self.context.ptr_type(AddressSpace::default());
                self.context.struct_type(&[ptr_type.into(), ptr_type.into()], false).into()
            }
            TypeKind::DynTrait { .. } => {
                // Trait object: fat pointer { data_ptr, vtable_ptr }
                let ptr_type = self.context.ptr_type(AddressSpace::default());
                self.context.struct_type(&[ptr_type.into(), ptr_type.into()], false).into()
            }
            TypeKind::Never => {
                // Never type - use i8 as placeholder (will never actually be used)
                self.context.i8_type().into()
            }
            TypeKind::Infer(id) => {
                // Unresolved inference variable reached codegen — type inference
                // should have resolved this. This is a hard error.
                self.type_lowering_errors.borrow_mut().push(
                    crate::diagnostics::Diagnostic::error(
                        format!(
                            "unresolved inference variable {:?} reached codegen; \
                             type inference should have resolved this",
                            id
                        ),
                        self.current_span(),
                    )
                );
                self.context.i8_type().into()
            }
            TypeKind::Error => {
                // Error recovery placeholder — type checking already reported
                // the original error, so just emit a placeholder type.
                self.context.i8_type().into()
            }
            TypeKind::Param(id) => {
                // Unsubstituted type parameter reached codegen — this is a
                // monomorphization bug. Report it instead of silently emitting i8*.
                self.type_lowering_errors.borrow_mut().push(
                    crate::diagnostics::Diagnostic::error(
                        format!(
                            "unsubstituted type parameter {:?} reached codegen; \
                             monomorphization should have replaced this",
                            id
                        ),
                        self.current_span(),
                    )
                );
                // Return opaque ptr as placeholder to allow continued error collection
                self.context.ptr_type(AddressSpace::default()).into()
            }
            TypeKind::Closure { .. } => {
                // Closure: fat pointer { fn_ptr, env_ptr }
                let ptr_type = self.context.ptr_type(AddressSpace::default());
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
                let ptr_type = self.context.ptr_type(AddressSpace::default());
                let len_type = self.context.i64_type();
                self.context.struct_type(&[ptr_type.into(), len_type.into()], false).into()
            }
            PrimitiveTy::String => {
                // Heap-allocated string: { ptr, len, capacity }
                let ptr_type = self.context.ptr_type(AddressSpace::default());
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

    /// Get the size of an LLVM type in bytes.
    ///
    /// CRITICAL: This function uses LLVM's native size_of() to ensure we get
    /// the exact same size that LLVM will use for memory operations. This is
    /// essential for enum payload sizing - if we underestimate the size, the
    /// payload array will be too small, causing memory corruption when storing
    /// larger variants.
    fn get_type_size_approx(&self, ty: BasicTypeEnum<'ctx>) -> usize {
        // Use LLVM's size_of() when available - this ensures we match LLVM's actual layout
        // which is critical for enum payload sizing and Vec element sizing
        let size_opt: Option<inkwell::values::IntValue<'ctx>> = match ty {
            BasicTypeEnum::ArrayType(t) => t.size_of(),
            BasicTypeEnum::FloatType(t) => Some(t.size_of()),
            BasicTypeEnum::IntType(t) => Some(t.size_of()),
            BasicTypeEnum::PointerType(t) => Some(t.size_of()),
            BasicTypeEnum::StructType(t) => t.size_of(),
            BasicTypeEnum::VectorType(t) => t.size_of(),
            BasicTypeEnum::ScalableVectorType(t) => t.size_of(),
        };
        if let Some(size_val) = size_opt {
            // LLVM's size_of returns a constant IntValue - extract the constant
            if let Some(size) = size_val.get_zero_extended_constant() {
                return size as usize;
            }
        }

        // Fallback to manual calculation if LLVM size_of isn't available
        match ty {
            BasicTypeEnum::IntType(t) => {
                let bits = t.get_bit_width() as usize;
                // Round up to byte boundary
                bits.div_ceil(8)
            }
            BasicTypeEnum::FloatType(t) => {
                if t == self.context.f32_type() { 4 } else { 8 }
            }
            BasicTypeEnum::PointerType(_) => 8, // 64-bit pointers
            BasicTypeEnum::StructType(st) => {
                // Compute actual struct size by summing field sizes with alignment
                let mut size: usize = 0;
                let mut max_align: usize = 1;

                for i in 0..st.count_fields() {
                    if let Some(field_ty) = st.get_field_type_at_index(i) {
                        let field_size = self.get_type_size_approx(field_ty);
                        let field_align = self.get_type_alignment(field_ty) as usize;

                        // Align current offset
                        let padding = (field_align - (size % field_align)) % field_align;
                        size += padding + field_size;

                        if field_align > max_align {
                            max_align = field_align;
                        }
                    }
                }

                // Final alignment padding
                let final_padding = (max_align - (size % max_align)) % max_align;
                size + final_padding
            }
            BasicTypeEnum::ArrayType(at) => {
                let elem_size = self.get_type_size_approx(at.get_element_type());
                elem_size * at.len() as usize
            }
            BasicTypeEnum::VectorType(vt) => {
                // SIMD vectors - compute based on element type and size
                let elem_size = self.get_type_size_approx(vt.get_element_type());
                elem_size * vt.get_size() as usize
            }
            BasicTypeEnum::ScalableVectorType(_) => {
                // Scalable vectors have runtime-determined size; use a conservative
                // estimate of 16 bytes (minimum vector register width on most targets).
                // This type should not appear in Blood's type system under normal usage.
                16
            }
        }
    }

    /// Get the maximum alignment requirement for a list of field types.
    /// This is used for enum variant alignment calculation.
    fn get_max_field_alignment(&self, field_types: &[BasicTypeEnum<'ctx>]) -> u32 {
        let mut max_align: u32 = 1;
        for ty in field_types {
            let align = self.get_type_alignment(*ty);
            if align > max_align {
                max_align = align;
            }
        }
        max_align
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
                Type::array_with_const(substituted, size.clone())
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
            // Non-generic types pass through unchanged — they contain no type parameters
            // to substitute. Closure, Range, Record, Forall, Ownership, DynTrait could
            // theoretically contain params but are not currently used in generic ADT fields.
            TypeKind::Closure { .. }
            | TypeKind::Range { .. }
            | TypeKind::Record { .. }
            | TypeKind::Forall { .. }
            | TypeKind::Ownership { .. }
            | TypeKind::DynTrait { .. }
            | TypeKind::Primitive(_)
            | TypeKind::Never
            | TypeKind::Error
            | TypeKind::Infer(_) => ty.clone(),
        }
    }
}
