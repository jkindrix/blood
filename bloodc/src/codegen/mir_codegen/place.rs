//! MIR place code generation.
//!
//! This module handles compilation of MIR places (memory locations) to LLVM IR.
//! Places represent lvalues - locations that can be read from or written to.

use inkwell::values::{BasicValueEnum, PointerValue};
use inkwell::types::BasicType;

use crate::diagnostics::Diagnostic;
use crate::hir::{Type, TypeKind};
use crate::mir::body::MirBody;
use crate::mir::types::{Place, PlaceElem};
use crate::mir::{EscapeResults, EscapeState};

use super::CodegenContext;

/// Extension trait for MIR place compilation.
pub trait MirPlaceCodegen<'ctx, 'a> {
    /// Get a pointer to a MIR place.
    fn compile_mir_place(
        &mut self,
        place: &Place,
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
    ) -> Result<PointerValue<'ctx>, Vec<Diagnostic>>;

    /// Load a value from a MIR place (with optional generation check).
    fn compile_mir_place_load(
        &mut self,
        place: &Place,
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>>;

    /// Compute the effective type of a place after applying projections.
    fn compute_place_type(&self, base_ty: &Type, projections: &[PlaceElem]) -> Type;
}

impl<'ctx, 'a> MirPlaceCodegen<'ctx, 'a> for CodegenContext<'ctx, 'a> {
    fn compile_mir_place(
        &mut self,
        place: &Place,
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
    ) -> Result<PointerValue<'ctx>, Vec<Diagnostic>> {
        let base_ptr = *self.locals.get(&place.local).ok_or_else(|| {
            vec![Diagnostic::error(
                format!("Local _{} not found", place.local.index),
                body.span,
            )]
        })?;

        // Track the current type as we process projections
        let local_info = body.locals.get(place.local.index as usize)
            .expect("ICE: MIR local not found in body during codegen");
        let base_ty = local_info.ty.clone();
        let mut current_ty = base_ty.clone();

        let mut current_ptr = base_ptr;
        let mut is_inside_variant = false;  // Track if we're inside an enum variant (after Downcast)

        // Check if this is a closure __env local with Field projections.
        // If so, we need to cast the i8* to the captures struct type first.
        let is_closure_env = local_info.name.as_deref() == Some("__env");
        let has_field_projections = place.projection.iter().any(|p| matches!(p, PlaceElem::Field(_)));

        if is_closure_env && has_field_projections {
            // Load the i8* from the alloca
            let env_i8_ptr = self.builder.build_load(current_ptr, "env_ptr")
                .map_err(|e| vec![Diagnostic::error(
                    format!("LLVM load error: {}", e), body.span
                )])?.into_pointer_value();

            // Get the captures struct type from the MIR type (which is a Tuple)
            let captures_llvm_ty = self.lower_type(&base_ty);
            let captures_ptr_ty = captures_llvm_ty.ptr_type(inkwell::AddressSpace::default());

            // Cast i8* to captures struct pointer
            current_ptr = self.builder.build_pointer_cast(env_i8_ptr, captures_ptr_ty, "env_captures_ptr")
                .map_err(|e| vec![Diagnostic::error(
                    format!("LLVM pointer cast error: {}", e), body.span
                )])?;
        }

        for elem in &place.projection {
            current_ptr = match elem {
                PlaceElem::Deref => {
                    // Update current_ty to the inner type (dereference the reference/pointer)
                    current_ty = match current_ty.kind() {
                        TypeKind::Ref { inner, .. } => inner.clone(),
                        TypeKind::Ptr { inner, .. } => inner.clone(),
                        _ => current_ty.clone(), // Keep as-is if not a reference type
                    };

                    // Load the pointer value
                    let loaded = self.builder.build_load(current_ptr, "deref")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM load error: {}", e), body.span
                        )])?;
                    let ptr_val = loaded.into_pointer_value();

                    // Check if we should skip generation checks for this local.
                    // NoEscape locals are stack-allocated and safe by lexical scoping.
                    let should_skip_gen_check = escape_results
                        .map(|er| er.get(place.local) == EscapeState::NoEscape)
                        .unwrap_or(false);

                    // If this is a region-allocated pointer and the local escapes,
                    // validate generation before use
                    if !should_skip_gen_check {
                    if let Some(&gen_alloca) = self.local_generations.get(&place.local) {
                        let i32_ty = self.context.i32_type();
                        let i64_ty = self.context.i64_type();

                        // Load the expected generation
                        let expected_gen = self.builder.build_load(gen_alloca, "expected_gen")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM load error: {}", e), body.span
                            )])?.into_int_value();

                        // Convert pointer to address for validation
                        let address = self.builder.build_ptr_to_int(ptr_val, i64_ty, "ptr_addr")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM ptr_to_int error: {}", e), body.span
                            )])?;

                        // Call blood_validate_generation(address, expected_gen)
                        let validate_fn = self.module.get_function("blood_validate_generation")
                            .ok_or_else(|| vec![Diagnostic::error(
                                "blood_validate_generation not declared", body.span
                            )])?;

                        let result = self.builder.build_call(
                            validate_fn,
                            &[address.into(), expected_gen.into()],
                            "gen_check"
                        ).map_err(|e| vec![Diagnostic::error(
                            format!("LLVM call error: {}", e), body.span
                        )])?.try_as_basic_value()
                            .left()
                            .ok_or_else(|| vec![Diagnostic::error(
                                "blood_validate_generation returned void", body.span
                            )])?.into_int_value();

                        // Check if stale (result != 0)
                        let is_stale = self.builder.build_int_compare(
                            inkwell::IntPredicate::NE,
                            result,
                            i32_ty.const_int(0, false),
                            "is_stale"
                        ).map_err(|e| vec![Diagnostic::error(
                            format!("LLVM compare error: {}", e), body.span
                        )])?;

                        // Create blocks for valid and stale paths
                        let fn_value = self.current_fn.ok_or_else(|| {
                            vec![Diagnostic::error("No current function", body.span)]
                        })?;
                        let valid_bb = self.context.append_basic_block(fn_value, "gen_valid");
                        let stale_bb = self.context.append_basic_block(fn_value, "gen_stale");

                        self.builder.build_conditional_branch(is_stale, stale_bb, valid_bb)
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM branch error: {}", e), body.span
                            )])?;

                        // Stale path: abort
                        self.builder.position_at_end(stale_bb);
                        let panic_fn = self.module.get_function("blood_stale_reference_panic")
                            .ok_or_else(|| vec![Diagnostic::error(
                                "blood_stale_reference_panic not declared", body.span
                            )])?;
                        self.builder.build_call(
                            panic_fn,
                            &[expected_gen.into(), result.into()],
                            ""
                        ).map_err(|e| vec![Diagnostic::error(
                            format!("LLVM call error: {}", e), body.span
                        )])?;
                        self.builder.build_unreachable()
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM unreachable error: {}", e), body.span
                            )])?;

                        // Continue on valid path
                        self.builder.position_at_end(valid_bb);
                    }
                    }

                    ptr_val
                }

                PlaceElem::Field(idx) => {
                    // Check if we're accessing an enum variant field (need to offset by 1 for tag)
                    let actual_idx = if is_inside_variant {
                        *idx + 1  // Offset by 1 to skip the discriminant tag
                    } else {
                        *idx
                    };

                    // Check if this is a reference to a struct (MIR may not emit explicit Deref)
                    let is_ref_to_struct = matches!(
                        current_ty.kind(),
                        TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. }
                            if matches!(inner.kind(), TypeKind::Adt { .. } | TypeKind::Tuple(_))
                    );

                    // Update current_ty to the field type (handle both direct and through-reference)
                    let effective_ty = match current_ty.kind() {
                        TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => inner.clone(),
                        _ => current_ty.clone(),
                    };
                    current_ty = match effective_ty.kind() {
                        TypeKind::Tuple(fields) => {
                            fields.get(*idx as usize).cloned().unwrap_or(current_ty.clone())
                        }
                        TypeKind::Adt { .. } => {
                            // For ADT types, we'd need field type lookup
                            // For now, keep current_ty as placeholder (field access works via GEP)
                            current_ty.clone()
                        }
                        _ => current_ty.clone(),
                    };

                    // Get struct element pointer
                    if is_ref_to_struct {
                        // Reference to struct: load pointer then struct_gep
                        let struct_ptr = self.builder.build_load(current_ptr, "struct_ptr")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM load error: {}", e), body.span
                            )])?.into_pointer_value();
                        self.builder.build_struct_gep(
                            struct_ptr,
                            actual_idx,
                            &format!("field_{}", idx)
                        ).map_err(|e| vec![Diagnostic::error(
                            format!("LLVM GEP error: {} (place={:?}, ty={:?})", e, place, effective_ty), body.span
                        )])?
                    } else {
                        self.builder.build_struct_gep(
                            current_ptr,
                            actual_idx,
                            &format!("field_{}", idx)
                        ).map_err(|e| vec![Diagnostic::error(
                            format!("LLVM GEP error: {} (place={:?}, ty={:?})", e, place, current_ty), body.span
                        )])?
                    }
                }

                PlaceElem::Index(idx_local) => {
                    let idx_ptr = self.locals.get(idx_local).ok_or_else(|| {
                        vec![Diagnostic::error(
                            format!("Index local _{} not found", idx_local.index),
                            body.span,
                        )]
                    })?;
                    let idx_val = self.builder.build_load(*idx_ptr, "idx")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM load error: {}", e), body.span
                        )])?;

                    // Check if this is a direct array or a reference to an array
                    let (is_direct_array, is_ref_to_array) = match current_ty.kind() {
                        TypeKind::Array { .. } | TypeKind::Slice { .. } => (true, false),
                        TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                            (false, matches!(inner.kind(), TypeKind::Array { .. } | TypeKind::Slice { .. }))
                        }
                        _ => (false, false),
                    };

                    // Update current_ty to element type (handle both direct and through-reference)
                    current_ty = match current_ty.kind() {
                        TypeKind::Array { element, .. } => element.clone(),
                        TypeKind::Slice { element } => element.clone(),
                        TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                            match inner.kind() {
                                TypeKind::Array { element, .. } => element.clone(),
                                TypeKind::Slice { element } => element.clone(),
                                _ => current_ty.clone(),
                            }
                        }
                        _ => current_ty.clone(),
                    };

                    unsafe {
                        if is_direct_array {
                            // Direct array: current_ptr is [N x T]*, use two-index GEP
                            let zero = self.context.i64_type().const_zero();
                            self.builder.build_in_bounds_gep(
                                current_ptr,
                                &[zero, idx_val.into_int_value()],
                                "idx_gep"
                            ).map_err(|e| vec![Diagnostic::error(
                                format!("LLVM GEP error: {}", e), body.span
                            )])?
                        } else if is_ref_to_array {
                            // Reference to array: current_ptr is [N x T]**, load to get [N x T]*
                            // then use two-index GEP
                            let array_ptr = self.builder.build_load(current_ptr, "array_ptr")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM load error: {}", e), body.span
                                )])?.into_pointer_value();
                            let zero = self.context.i64_type().const_zero();
                            self.builder.build_in_bounds_gep(
                                array_ptr,
                                &[zero, idx_val.into_int_value()],
                                "idx_gep"
                            ).map_err(|e| vec![Diagnostic::error(
                                format!("LLVM GEP error: {}", e), body.span
                            )])?
                        } else {
                            // Other pointer type: single-index GEP
                            self.builder.build_in_bounds_gep(
                                current_ptr,
                                &[idx_val.into_int_value()],
                                "idx_gep"
                            ).map_err(|e| vec![Diagnostic::error(
                                format!("LLVM GEP error: {}", e), body.span
                            )])?
                        }
                    }
                }

                PlaceElem::ConstantIndex { offset, min_length: _, from_end } => {
                    // Check if this is a direct array or a reference to an array
                    let (is_direct_array, is_ref_to_array) = match current_ty.kind() {
                        TypeKind::Array { .. } | TypeKind::Slice { .. } => (true, false),
                        TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                            (false, matches!(inner.kind(), TypeKind::Array { .. } | TypeKind::Slice { .. }))
                        }
                        _ => (false, false),
                    };

                    // Get the effective array type for from_end calculations
                    let effective_array_ty = match current_ty.kind() {
                        TypeKind::Array { .. } => current_ty.clone(),
                        TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => inner.clone(),
                        _ => current_ty.clone(),
                    };

                    let idx = if *from_end {
                        // For from_end, compute actual index as array_length - offset - 1
                        let array_len = match effective_array_ty.kind() {
                            TypeKind::Array { size, .. } => size,
                            _ => {
                                return Err(vec![Diagnostic::error(
                                    format!("ConstantIndex from_end requires array type, got {:?}", current_ty),
                                    body.span,
                                )]);
                            }
                        };
                        // offset is the distance from the end, so index = array_len - 1 - offset
                        let actual_idx = array_len - 1 - *offset;
                        self.context.i64_type().const_int(actual_idx, false)
                    } else {
                        self.context.i64_type().const_int(*offset, false)
                    };

                    // Update current_ty to element type (handle both direct and through-reference)
                    current_ty = match current_ty.kind() {
                        TypeKind::Array { element, .. } => element.clone(),
                        TypeKind::Slice { element } => element.clone(),
                        TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                            match inner.kind() {
                                TypeKind::Array { element, .. } => element.clone(),
                                TypeKind::Slice { element } => element.clone(),
                                _ => current_ty.clone(),
                            }
                        }
                        _ => current_ty.clone(),
                    };

                    unsafe {
                        if is_direct_array {
                            // Direct array: current_ptr is [N x T]*, use two-index GEP
                            let zero = self.context.i64_type().const_zero();
                            self.builder.build_in_bounds_gep(current_ptr, &[zero, idx], "const_idx")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM GEP error: {}", e), body.span
                                )])?
                        } else if is_ref_to_array {
                            // Reference to array: load pointer then two-index GEP
                            let array_ptr = self.builder.build_load(current_ptr, "array_ptr")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM load error: {}", e), body.span
                                )])?.into_pointer_value();
                            let zero = self.context.i64_type().const_zero();
                            self.builder.build_in_bounds_gep(array_ptr, &[zero, idx], "const_idx")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM GEP error: {}", e), body.span
                                )])?
                        } else {
                            // Other type: single-index GEP
                            self.builder.build_in_bounds_gep(current_ptr, &[idx], "const_idx")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM GEP error: {}", e), body.span
                                )])?
                        }
                    }
                }

                PlaceElem::Subslice { from, to, from_end: _ } => {
                    // Check if this is a direct array or a reference to an array
                    let (is_direct_array, is_ref_to_array) = match current_ty.kind() {
                        TypeKind::Array { .. } | TypeKind::Slice { .. } => (true, false),
                        TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                            (false, matches!(inner.kind(), TypeKind::Array { .. } | TypeKind::Slice { .. }))
                        }
                        _ => (false, false),
                    };

                    let idx = self.context.i64_type().const_int(*from, false);
                    let _ = to; // End index for slice length calculation

                    // For subslice, the type remains array/slice (just a different view)
                    // but the GEP behavior depends on whether we have an array pointer

                    unsafe {
                        if is_direct_array {
                            let zero = self.context.i64_type().const_zero();
                            self.builder.build_in_bounds_gep(current_ptr, &[zero, idx], "subslice")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM GEP error: {}", e), body.span
                                )])?
                        } else if is_ref_to_array {
                            // Reference to array: load pointer then two-index GEP
                            let array_ptr = self.builder.build_load(current_ptr, "array_ptr")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM load error: {}", e), body.span
                                )])?.into_pointer_value();
                            let zero = self.context.i64_type().const_zero();
                            self.builder.build_in_bounds_gep(array_ptr, &[zero, idx], "subslice")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM GEP error: {}", e), body.span
                                )])?
                        } else {
                            self.builder.build_in_bounds_gep(current_ptr, &[idx], "subslice")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM GEP error: {}", e), body.span
                                )])?
                        }
                    }
                }

                PlaceElem::Downcast(_variant_idx) => {
                    // Downcast is logically an assertion that we have the right variant.
                    // The actual field offset is handled by subsequent Field projections.
                    // Mark that we're inside a variant so Field knows to offset by 1 for the tag.
                    is_inside_variant = true;
                    current_ptr  // Return the same pointer
                }
            };
        }

        Ok(current_ptr)
    }

    fn compile_mir_place_load(
        &mut self,
        place: &Place,
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        let ptr = self.compile_mir_place(place, body, escape_results)?;

        // Generation checks for region-tier allocations are implemented in
        // compile_mir_place() for PlaceElem::Deref. When dereferencing a pointer
        // that was allocated via blood_alloc_or_abort (Region/Persistent tier),
        // the local_generations map contains the generation storage location.
        // The Deref handler validates via blood_validate_generation and panics
        // on stale reference detection.
        //
        // Stack tier (NoEscape) values skip generation checks entirely as they
        // are guaranteed safe by lexical scoping - escape_results is passed to
        // compile_mir_place which checks escape state before emitting gen checks.

        // Load value from pointer
        let loaded = self.builder.build_load(ptr, "load")
            .map_err(|e| vec![Diagnostic::error(
                format!("LLVM load error: {}", e), body.span
            )])?;

        Ok(loaded)
    }

    fn compute_place_type(&self, base_ty: &Type, projections: &[PlaceElem]) -> Type {
        let mut current_ty = base_ty.clone();

        for proj in projections {
            current_ty = match proj {
                PlaceElem::Deref => {
                    // Dereference: unwrap Ref, Ptr, or Box types
                    match current_ty.kind() {
                        TypeKind::Ref { inner, .. } => inner.clone(),
                        TypeKind::Ptr { inner, .. } => inner.clone(),
                        // For other types, keep the type (should not happen in valid MIR)
                        _ => current_ty,
                    }
                }
                PlaceElem::Field(idx) => {
                    // Field access: get the field type from struct/tuple
                    match current_ty.kind() {
                        TypeKind::Tuple(tys) => {
                            tys.get(*idx as usize).cloned().unwrap_or(current_ty)
                        }
                        // For ADT types, we'd need DefId lookup to get field types
                        // For now, return current type (length queries on ADT fields
                        // will fail with an appropriate error message)
                        TypeKind::Adt { .. } => current_ty,
                        // For other types, keep the type
                        _ => current_ty,
                    }
                }
                PlaceElem::Index(_) | PlaceElem::ConstantIndex { .. } => {
                    // Array/slice indexing: get the element type
                    match current_ty.kind() {
                        TypeKind::Array { element, .. } => element.clone(),
                        TypeKind::Slice { element } => element.clone(),
                        // For other types, keep the type
                        _ => current_ty,
                    }
                }
                PlaceElem::Subslice { .. } => {
                    // Subslice keeps the same slice type
                    current_ty
                }
                PlaceElem::Downcast(variant_idx) => {
                    // Downcast to a specific enum variant - keep the type
                    let _ = variant_idx;
                    current_ty
                }
            };
        }

        current_ty
    }
}
