//! MIR place code generation.
//!
//! This module handles compilation of MIR places (memory locations) to LLVM IR.
//! Places represent lvalues - locations that can be read from or written to.

use inkwell::values::{BasicValueEnum, PointerValue};

use crate::diagnostics::Diagnostic;
use crate::hir::{Type, TypeKind};
use crate::mir::body::MirBody;
use crate::mir::types::{Place, PlaceElem};
use crate::mir::EscapeResults;

use super::CodegenContext;

/// Extension trait for MIR place compilation.
pub trait MirPlaceCodegen<'ctx, 'a> {
    /// Get a pointer to a MIR place.
    fn compile_mir_place(
        &mut self,
        place: &Place,
        body: &MirBody,
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
    ) -> Result<PointerValue<'ctx>, Vec<Diagnostic>> {
        let base_ptr = *self.locals.get(&place.local).ok_or_else(|| {
            vec![Diagnostic::error(
                format!("Local _{} not found", place.local.index),
                body.span,
            )]
        })?;

        let mut current_ptr = base_ptr;

        for elem in &place.projection {
            current_ptr = match elem {
                PlaceElem::Deref => {
                    // Load the pointer value
                    let loaded = self.builder.build_load(current_ptr, "deref")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM load error: {}", e), body.span
                        )])?;
                    let ptr_val = loaded.into_pointer_value();

                    // If this is a region-allocated pointer, validate generation before use
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

                    ptr_val
                }

                PlaceElem::Field(idx) => {
                    // Get struct element pointer
                    self.builder.build_struct_gep(
                        current_ptr,
                        *idx,
                        &format!("field_{}", idx)
                    ).map_err(|e| vec![Diagnostic::error(
                        format!("LLVM GEP error: {}", e), body.span
                    )])?
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

                    unsafe {
                        self.builder.build_in_bounds_gep(
                            current_ptr,
                            &[idx_val.into_int_value()],
                            "idx_gep"
                        ).map_err(|e| vec![Diagnostic::error(
                            format!("LLVM GEP error: {}", e), body.span
                        )])?
                    }
                }

                PlaceElem::ConstantIndex { offset, min_length: _, from_end } => {
                    let idx = if *from_end {
                        self.context.i64_type().const_int((!*offset).wrapping_add(1), false)
                    } else {
                        self.context.i64_type().const_int(*offset, false)
                    };
                    unsafe {
                        self.builder.build_in_bounds_gep(current_ptr, &[idx], "const_idx")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM GEP error: {}", e), body.span
                            )])?
                    }
                }

                PlaceElem::Subslice { from, to, from_end: _ } => {
                    let idx = self.context.i64_type().const_int(*from, false);
                    let _ = to; // End index for slice length calculation
                    unsafe {
                        self.builder.build_in_bounds_gep(current_ptr, &[idx], "subslice")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM GEP error: {}", e), body.span
                            )])?
                    }
                }

                PlaceElem::Downcast(variant_idx) => {
                    // For enum downcast, skip past the tag
                    self.builder.build_struct_gep(
                        current_ptr,
                        variant_idx + 1, // Skip tag field
                        &format!("variant_{}", variant_idx)
                    ).map_err(|e| vec![Diagnostic::error(
                        format!("LLVM GEP error: {}", e), body.span
                    )])?
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
        let ptr = self.compile_mir_place(place, body)?;

        // Generation checks for region-tier allocations are implemented in
        // compile_mir_place() for PlaceElem::Deref. When dereferencing a pointer
        // that was allocated via blood_alloc_or_abort (Region/Persistent tier),
        // the local_generations map contains the generation storage location.
        // The Deref handler validates via blood_validate_generation and panics
        // on stale reference detection.
        //
        // Stack tier (NoEscape) values skip generation checks entirely as they
        // are guaranteed safe by lexical scoping.
        let _ = (escape_results,); // Escape results used in compile_mir_place

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
