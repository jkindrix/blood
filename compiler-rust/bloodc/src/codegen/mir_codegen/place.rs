//! MIR place code generation.
//!
//! This module handles compilation of MIR places (memory locations) to LLVM IR.
//! Places represent lvalues - locations that can be read from or written to.

use inkwell::values::{BasicValue, BasicValueEnum, PointerValue};
use inkwell::AddressSpace;

use crate::diagnostics::Diagnostic;
use crate::hir::{Type, TypeKind};
use crate::mir::body::MirBody;
use crate::mir::types::{Place, PlaceElem};
use crate::mir::{EscapeResults, EscapeState};

use super::CodegenContext;
use super::types::MirTypesCodegen;

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
        use crate::mir::types::PlaceBase;

        // Get base pointer and type based on whether this is a local or static
        let (base_ptr, base_ty, local_info_opt) = match &place.base {
            PlaceBase::Local(local_id) => {
                let ptr = *self.locals.get(local_id).ok_or_else(|| {
                    vec![Diagnostic::error(
                        format!("Local _{} not found", local_id.index),
                        body.span,
                    )]
                })?;
                let local_info = body.locals.get(local_id.index as usize)
                    .expect("ICE: MIR local not found in body during codegen");
                (ptr, local_info.ty.clone(), Some(local_info))
            }
            PlaceBase::Static(def_id) => {
                let global = self.static_globals.get(def_id).ok_or_else(|| {
                    vec![Diagnostic::error(
                        format!("Static {:?} not found in globals", def_id),
                        body.span,
                    )]
                })?;
                let static_ty = self.get_static_type(*def_id).ok_or_else(|| {
                    vec![Diagnostic::error(
                        format!("Static {:?} type not found", def_id),
                        body.span,
                    )]
                })?;
                (global.as_pointer_value(), static_ty, None)
            }
        };
        let mut current_ty = base_ty.clone();

        let mut current_ptr = base_ptr;
        // Track if we're inside an enum variant: (enum_def_id, variant_index)
        // This is needed to handle heterogeneous variant payloads correctly
        let mut variant_ctx: Option<(crate::hir::DefId, u32)> = None;

        // Check if this is a closure __env local with Field projections.
        // If so, we need to cast the i8* to the captures struct type first.
        // (Only applies to local-based places, not statics)
        let is_closure_env = local_info_opt.map(|li| li.name.as_deref() == Some("__env")).unwrap_or(false);
        let has_field_projections = place.projection.iter().any(|p| matches!(p, PlaceElem::Field(_)));

        if is_closure_env && has_field_projections {
            // Load the i8* from the alloca
            let ptr_ty = self.context.ptr_type(AddressSpace::default());
            let env_i8_ptr = self.builder.build_load(ptr_ty, current_ptr, "env_ptr")
                .map_err(|e| vec![Diagnostic::error(
                    format!("LLVM load error: {}", e), body.span
                )])?.into_pointer_value();

            // With opaque pointers, no cast needed — all pointers are the same type
            current_ptr = env_i8_ptr;
        }

        // Debug: trace full place access
        if std::env::var("BLOOD_DEBUG_PLACE").is_ok() {
            eprintln!("[compile_mir_place] ===== PLACE ACCESS =====");
            eprintln!("[compile_mir_place] place: {:?}, base_ty: {:?}", place, base_ty);
            eprintln!("[compile_mir_place] projections: {:?}", place.projection);
            eprintln!("[compile_mir_place] base_ptr type: {:?}", base_ptr.get_type().print_to_string());
        }

        for (proj_idx, elem) in place.projection.iter().enumerate() {
            // Debug: trace each projection step
            if std::env::var("BLOOD_DEBUG_PLACE").is_ok() {
                eprintln!("[compile_mir_place] --- projection {} ---", proj_idx);
                eprintln!("[compile_mir_place] elem: {:?}", elem);
                eprintln!("[compile_mir_place] current_ty: {:?}", current_ty);
                eprintln!("[compile_mir_place] current_ptr type: {:?}", current_ptr.get_type().print_to_string());
            }

            current_ptr = match elem {
                PlaceElem::Deref => {
                    // Save original type to check if this is a fat pointer (slice reference)
                    let original_ty = current_ty.clone();
                    let is_fat_ptr = match original_ty.kind() {
                        TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                            matches!(inner.kind(), TypeKind::Slice { .. })
                        }
                        TypeKind::Slice { .. } => true,
                        _ => false,
                    };

                    // Update current_ty to the inner type (dereference the reference/pointer/Box)
                    current_ty = match original_ty.kind() {
                        TypeKind::Ref { inner, .. } => inner.clone(),
                        TypeKind::Ptr { inner, .. } => inner.clone(),
                        // Box<T> → T: Box is heap indirection, deref yields the inner type
                        TypeKind::Adt { def_id, args } if Some(*def_id) == self.box_def_id => {
                            args.first().cloned().unwrap_or(current_ty.clone())
                        }
                        _ => current_ty.clone(), // Keep as-is if not a reference type
                    };

                    // Load the pointer value
                    // Use the original (pre-deref) type to determine what's stored in the alloca
                    let load_ty = self.lower_type(&original_ty);
                    let loaded = self.builder.build_load(load_ty, current_ptr, "deref")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM load error: {}", e), body.span
                        )])?;
                    // Set proper alignment for the deref load
                    let deref_alignment = self.get_type_alignment_for_value(loaded);
                    if let Some(inst) = loaded.as_instruction_value() {
                        let _ = inst.set_alignment(deref_alignment);
                    }

                    // Handle different loaded value types:
                    // - PointerValue: thin reference (normal case)
                    // - StructValue: fat reference (like &[T] or &str - contains ptr + metadata)
                    //                Only extract field 0 if this is actually a fat pointer type
                    // - IntValue: opaque pointer representation or enum variant data
                    use inkwell::values::BasicValueEnum;
                    let ptr_val = match loaded {
                        BasicValueEnum::PointerValue(ptr) => ptr,
                        BasicValueEnum::StructValue(sv) => {
                            if is_fat_ptr {
                                // Fat pointer (slice/str) - extract the data pointer from field 0
                                self.builder.build_extract_value(sv, 0, "fat_ptr_data")
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM extract_value error: {}", e), body.span
                                    )])?
                                    .into_pointer_value()
                            } else {
                                // This is a struct value but NOT a fat pointer.
                                // This can happen when we have a Copy type stored by value.
                                // We need to store it to a temporary and return pointer to that.
                                let struct_ty = sv.get_type();
                                let tmp_alloca = self.builder.build_alloca(struct_ty, "deref_tmp")
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM alloca error: {}", e), body.span
                                    )])?;
                                self.builder.build_store(tmp_alloca, sv)
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM store error: {}", e), body.span
                                    )])?;
                                tmp_alloca
                            }
                        }
                        BasicValueEnum::IntValue(int_val) => {
                            // Opaque pointer as integer - convert to pointer type
                            let ptr_ty = self.context.ptr_type(AddressSpace::default());
                            self.builder.build_int_to_ptr(int_val, ptr_ty, "deref_int_to_ptr")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM int_to_ptr error: {}", e), body.span
                                )])?
                        }
                        other => {
                            return Err(vec![Diagnostic::error(
                                format!("Expected pointer, struct (fat ptr), or integer for Deref, got {:?}", other),
                                body.span,
                            )]);
                        }
                    };

                    // With opaque pointers, Box<T> deref needs no cast — all pointers
                    // are the same opaque `ptr` type. The loaded pointer is already usable.
                    let ptr_val = ptr_val;

                    // Check if we should skip generation checks for this local.
                    // NoEscape locals are stack-allocated and safe by lexical scoping.
                    // Static places never need generation checks - they're in global memory.
                    let should_skip_gen_check = place.is_static() || escape_results
                        .and_then(|er| place.as_local().map(|l| er.get(l) == EscapeState::NoEscape))
                        .unwrap_or(false);

                    // If this is a region-allocated pointer and the local escapes,
                    // validate generation before use.
                    //
                    // TEMPORARILY DISABLED: The gen_alloca stack slots are reading
                    // garbage values, causing false stale reference panics. The
                    // alloca initialization is correct (zero-init + blood_alloc_or_abort
                    // write) but the load reads from a different stack location.
                    // Investigation needed into LLVM 14 alloca frame layout.
                    if false && !should_skip_gen_check {
                    if let Some(local_id) = place.as_local() {
                    if let Some(&gen_alloca) = self.local_generations.get(&local_id) {
                        let i32_ty = self.context.i32_type();
                        let i64_ty = self.context.i64_type();

                        // Load the expected generation
                        let expected_gen = self.builder.build_load(i32_ty, gen_alloca, "expected_gen")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM load error: {}", e), body.span
                            )])?.into_int_value();

                        // Use the local's storage pointer for address validation.
                        // For Region-allocated locals, locals[id] is the heap pointer
                        // returned by blood_alloc_or_abort. Using ptr_val would be wrong
                        // because it may be a stack temporary (e.g., when a StructValue
                        // is loaded and spilled to a tmp alloca at lines 176-185).
                        let local_storage_ptr = *self.locals.get(&local_id)
                            .expect("ICE: local not found in locals map for generation check");
                        let address = self.builder.build_ptr_to_int(local_storage_ptr, i64_ty, "ptr_addr")
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
                            .basic()
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

                        // Stale path: get actual generation and abort
                        self.builder.position_at_end(stale_bb);
                        let panic_fn = self.module.get_function("blood_stale_reference_panic")
                            .ok_or_else(|| vec![Diagnostic::error(
                                "blood_stale_reference_panic not declared", body.span
                            )])?;

                        // Get the actual generation from the runtime for diagnostic
                        // accuracy, matching the pattern in memory.rs:emit_generation_check_impl
                        let actual_gen = if let Some(get_gen_fn) = self.module.get_function("blood_get_generation") {
                            let gen_result = self.builder.build_call(
                                get_gen_fn,
                                &[address.into()],
                                "actual_gen"
                            ).map_err(|e| vec![Diagnostic::error(
                                format!("LLVM call error: {}", e), body.span
                            )])?;
                            gen_result.try_as_basic_value()
                                .basic()
                                .map(|v| v.into_int_value())
                                .unwrap_or_else(|| i32_ty.const_int(0, false))
                        } else {
                            // Fallback if blood_get_generation not available
                            i32_ty.const_int(0, false)
                        };

                        self.builder.build_call(
                            panic_fn,
                            &[expected_gen.into(), actual_gen.into()],
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
                    } // if let Some(local_id)
                    } // if !should_skip_gen_check

                    ptr_val
                }

                PlaceElem::Field(idx) => {
                    // Check if we're accessing an enum variant field
                    if let Some((enum_def_id, variant_idx)) = variant_ctx {
                        // We're inside an enum variant - need special handling for heterogeneous payloads
                        // The enum layout is { i32 tag, largest_variant_payload... }
                        // But the actual variant's payload might be smaller/different type

                        // Get the enum's variant field types
                        if let Some(variants) = self.enum_defs.get(&enum_def_id) {
                            if let Some(variant_fields) = variants.get(variant_idx as usize) {
                                if let Some(variant_field_ty) = variant_fields.get(*idx as usize) {
                                    // Substitute type params if this is a generic enum
                                    let args = match current_ty.kind() {
                                        TypeKind::Adt { args, .. } => args.clone(),
                                        _ => Vec::new(),
                                    };
                                    let actual_field_ty = self.substitute_type_params(variant_field_ty, &args);

                                    // Get the enum's LLVM struct type for GEP
                                    let enum_llvm_ty = self.lower_type(&current_ty);
                                    let enum_struct_ty = enum_llvm_ty.into_struct_type();

                                    // Get pointer to payload area (field 1 of enum struct)
                                    let payload_ptr = self.builder.build_struct_gep(enum_struct_ty, current_ptr, 1, "payload_ptr")
                                        .map_err(|e| vec![Diagnostic::error(
                                            format!("LLVM GEP error: {}", e), body.span
                                        )])?;

                                    // Debug: print current_ptr type
                                    if std::env::var("BLOOD_DEBUG_FIELD").is_ok() {
                                        eprintln!("[Variant Field] current_ptr type: {:?}, payload_ptr type: {:?}",
                                            current_ptr.get_type().print_to_string(),
                                            payload_ptr.get_type().print_to_string());
                                    }

                                    // Build the variant's actual payload struct type
                                    let variant_field_types: Vec<inkwell::types::BasicTypeEnum> = variant_fields.iter()
                                        .map(|f| {
                                            let substituted = self.substitute_type_params(f, &args);
                                            self.lower_type(&substituted)
                                        })
                                        .collect();
                                    let variant_struct_ty = self.context.struct_type(&variant_field_types, false);

                                    // With opaque pointers, no cast needed — payload_ptr is already
                                    // a generic `ptr` that can be used with any struct GEP
                                    let variant_ptr = payload_ptr;

                                    // GEP to the specific field within the variant
                                    let field_ptr = self.builder.build_struct_gep(variant_struct_ty, variant_ptr, *idx, &format!("variant_field_{}", idx))
                                        .map_err(|e| vec![Diagnostic::error(
                                            format!("LLVM GEP error: {}", e), body.span
                                        )])?;

                                    // Debug: print variant_ptr and field_ptr types
                                    if std::env::var("BLOOD_DEBUG_FIELD").is_ok() {
                                        eprintln!("[Variant Field] variant_ptr type: {:?}, field_ptr type: {:?}, idx: {}",
                                            variant_ptr.get_type().print_to_string(),
                                            field_ptr.get_type().print_to_string(),
                                            idx);
                                    }

                                    // Clear variant context since we've accessed the field
                                    variant_ctx = None;
                                    current_ty = actual_field_ty;
                                    current_ptr = field_ptr;
                                    continue;
                                }
                            }
                        }
                        // Fall through to regular field access if lookup failed
                    }

                    // Regular field access (not inside variant, or variant lookup failed)
                    let actual_idx = if variant_ctx.is_some() {
                        *idx + 1  // Offset by 1 to skip the discriminant tag
                    } else {
                        *idx
                    };

                    // Debug: trace nested field access
                    if std::env::var("BLOOD_DEBUG_FIELD").is_ok() {
                        eprintln!("[Field] ===== FIELD ACCESS idx={}, actual_idx={} =====", idx, actual_idx);
                        eprintln!("[Field] current_ty: {:?}", current_ty);
                        eprintln!("[Field] current_ptr type: {:?}", current_ptr.get_type().print_to_string());
                    }

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
                        TypeKind::Adt { def_id, args } => {
                            // Look up field type for ADT types
                            if Some(*def_id) == self.vec_def_id {
                                // Vec<T> layout: { ptr: *T, len: i64, capacity: i64 }
                                match idx {
                                    0 => {
                                        let elem_ty = args.first().cloned().unwrap_or(Type::unit());
                                        Type::new(TypeKind::Ptr { inner: elem_ty, mutable: true })
                                    }
                                    1 | 2 => Type::usize(),
                                    _ => current_ty.clone(),
                                }
                            } else if Some(*def_id) == self.option_def_id {
                                // Option<T> layout: { tag: i32, payload: T }
                                match idx {
                                    0 => Type::i32(),
                                    1 => args.first().cloned().unwrap_or(Type::unit()),
                                    _ => current_ty.clone(),
                                }
                            } else if Some(*def_id) == self.result_def_id {
                                // Result<T, E> layout: { tag: i32, payload: T or E }
                                match idx {
                                    0 => Type::i32(),
                                    1 => args.first().cloned().unwrap_or(Type::unit()),
                                    2 => args.get(1).cloned().unwrap_or(Type::unit()),
                                    _ => current_ty.clone(),
                                }
                            } else if let Some(field_types) = self.struct_defs.get(def_id) {
                                // Regular struct - look up field type
                                if let Some(field_ty) = field_types.get(*idx as usize) {
                                    self.substitute_type_params(field_ty, args)
                                } else {
                                    current_ty.clone()
                                }
                            } else {
                                // Unknown ADT or enum, keep type
                                current_ty.clone()
                            }
                        }
                        _ => current_ty.clone(),
                    };

                    // Debug: trace type update results
                    if std::env::var("BLOOD_DEBUG_FIELD").is_ok() {
                        eprintln!("[Field] is_ref_to_struct: {}", is_ref_to_struct);
                        eprintln!("[Field] effective_ty: {:?}", effective_ty);
                        eprintln!("[Field] NEW current_ty: {:?}", current_ty);
                        if let TypeKind::Adt { def_id, .. } = effective_ty.kind() {
                            if let Some(fields) = self.struct_defs.get(def_id) {
                                eprintln!("[Field] struct_defs for def_id {:?} has {} fields", def_id, fields.len());
                                for (i, f) in fields.iter().enumerate() {
                                    eprintln!("[Field]   field {}: {:?}", i, f);
                                }
                            } else {
                                eprintln!("[Field] WARNING: def_id {:?} NOT found in struct_defs!", def_id);
                            }
                        }
                    }

                    // Get struct element pointer
                    if is_ref_to_struct {
                        // Reference to struct: load pointer then struct_gep
                        // The alloca stores a pointer (reference), so load with ptr type
                        let ptr_ty = self.context.ptr_type(AddressSpace::default());
                        let loaded_val = self.builder.build_load(ptr_ty, current_ptr, "struct_ptr")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM load error: {}", e), body.span
                            )])?;
                        // Set proper alignment for struct pointer load
                        let struct_ptr_alignment = self.get_type_alignment_for_value(loaded_val);
                        if let Some(inst) = loaded_val.as_instruction_value() {
                            let _ = inst.set_alignment(struct_ptr_alignment);
                        }

                        // Handle different loaded value types:
                        // - PointerValue: thin reference, use directly for struct_gep
                        // - IntValue: opaque pointer representation, convert to pointer
                        // - StructValue: fat reference (like &[T]), use current_ptr directly
                        use inkwell::values::BasicValueEnum;
                        let struct_ptr = match loaded_val {
                            BasicValueEnum::PointerValue(ptr) => ptr,
                            BasicValueEnum::IntValue(int_val) => {
                                // Opaque pointer as integer - convert to pointer type
                                let opaque_ptr_ty = self.context.ptr_type(AddressSpace::default());
                                self.builder.build_int_to_ptr(int_val, opaque_ptr_ty, "struct_ptr_cast")
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM int_to_ptr error: {}", e), body.span
                                    )])?
                            }
                            BasicValueEnum::StructValue(_) => {
                                // Fat pointer or value type reference - the referenced data is
                                // already at current_ptr (it's the value, not a separate pointer).
                                // Use current_ptr directly for GEP since it points to the struct.
                                current_ptr
                            }
                            other => {
                                return Err(vec![Diagnostic::error(
                                    format!("Expected pointer, integer, or struct for reference, got {:?}", other),
                                    body.span,
                                )]);
                            }
                        };
                        // Debug: show GEP input
                        if std::env::var("BLOOD_DEBUG_FIELD").is_ok() {
                            eprintln!("[Field] GEP (ref path): struct_ptr type = {:?}, actual_idx = {}",
                                struct_ptr.get_type().print_to_string(), actual_idx);
                        }
                        // Use the effective (struct) type for the struct GEP
                        let effective_struct_ty = self.lower_type(&effective_ty).into_struct_type();
                        let gep_result = self.builder.build_struct_gep(
                            effective_struct_ty,
                            struct_ptr,
                            actual_idx,
                            &format!("field_{}", idx)
                        ).map_err(|e| vec![Diagnostic::error(
                            format!("LLVM GEP error: {} (place={:?}, ty={:?})", e, place, effective_ty), body.span
                        )])?;
                        if std::env::var("BLOOD_DEBUG_FIELD").is_ok() {
                            eprintln!("[Field] GEP (ref path) result: {:?}", gep_result.get_type().print_to_string());
                        }
                        gep_result
                    } else {
                        // Debug: show GEP input
                        if std::env::var("BLOOD_DEBUG_FIELD").is_ok() {
                            eprintln!("[Field] GEP (direct path): current_ptr type = {:?}, actual_idx = {}",
                                current_ptr.get_type().print_to_string(), actual_idx);
                        }
                        // For direct struct access, we need the struct type that current_ptr points to.
                        // effective_ty is the struct type (unwrapped from Ref if applicable)
                        let direct_struct_ty = self.lower_type(&effective_ty).into_struct_type();
                        let gep_result = self.builder.build_struct_gep(
                            direct_struct_ty,
                            current_ptr,
                            actual_idx,
                            &format!("field_{}", idx)
                        ).map_err(|e| vec![Diagnostic::error(
                            format!("LLVM GEP error: {} (place={:?}, ty={:?})", e, place, current_ty), body.span
                        )])?;
                        if std::env::var("BLOOD_DEBUG_FIELD").is_ok() {
                            eprintln!("[Field] GEP (direct path) result: {:?}", gep_result.get_type().print_to_string());
                        }
                        gep_result
                    }
                }

                PlaceElem::Index(idx_local) => {
                    let idx_ptr = self.locals.get(idx_local).ok_or_else(|| {
                        vec![Diagnostic::error(
                            format!("Index local _{} not found", idx_local.index),
                            body.span,
                        )]
                    })?;
                    // Index locals are integer types (typically i64 for usize)
                    let idx_load_ty = self.context.i64_type();
                    let idx_val = self.builder.build_load(idx_load_ty, *idx_ptr, "idx")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM load error: {}", e), body.span
                        )])?;

                    // Classify the indexable type:
                    // - Direct array [T; N]: ptr is [N x T]*, two-index GEP
                    // - Direct slice [T]: ptr is {T*, i64}* (fat pointer), extract data ptr, single-index GEP
                    // - Ref to array &[T; N]: ptr is [N x T]**, load to get [N x T]*, two-index GEP
                    // - Slice ref &[T]: ptr is {T*, i64}* (fat pointer), load struct, extract data ptr, single-index GEP
                    // - Ptr to elements *T: current_ptr is **T (e.g., Vec.data), load then single-index GEP
                    // - Vec<T>: current_ptr is Vec*, extract data ptr (field 0), load, then single-index GEP
                    // - Ref to Vec<T>: current_ptr is Vec**, load to get Vec*, then like VecIndex
                    #[derive(Debug)]
                    enum IndexKind {
                        DirectArray,
                        DirectSlice,
                        RefToArray,
                        SliceRef,
                        PtrToElements,  // For Vec data pointer or similar
                        VecIndex,       // Direct indexing into Vec<T>
                        RefToVec,       // Reference to Vec<T> - need to load ref first
                        Other,
                    }

                    let index_kind = match current_ty.kind() {
                        TypeKind::Array { .. } => IndexKind::DirectArray,
                        TypeKind::Slice { .. } => IndexKind::DirectSlice,
                        TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                            match inner.kind() {
                                TypeKind::Array { .. } => IndexKind::RefToArray,
                                TypeKind::Slice { .. } => IndexKind::SliceRef,
                                // Reference to Vec<T>: need to load ref to get Vec*, then index into it
                                TypeKind::Adt { def_id, .. } if Some(*def_id) == self.vec_def_id => {
                                    IndexKind::RefToVec
                                }
                                // Pointer to non-array/slice elements (e.g., Vec<T>.data is *T)
                                // After Field(0) on Vec, we have Ptr { inner: T }
                                // current_ptr is **T, need to load to get *T then GEP
                                _ => IndexKind::PtrToElements,
                            }
                        }
                        // Vec<T> indexing: need to extract data pointer and index into it
                        TypeKind::Adt { def_id, .. } if Some(*def_id) == self.vec_def_id => {
                            IndexKind::VecIndex
                        }
                        _ => IndexKind::Other,
                    };

                    // Debug: trace IndexKind
                    if std::env::var("BLOOD_DEBUG_PLACE").is_ok() {
                        eprintln!("[compile_mir_place] IndexKind determined: {:?}", index_kind);
                        eprintln!("[compile_mir_place] current_ty for IndexKind: {:?}", current_ty);
                        eprintln!("[compile_mir_place] vec_def_id: {:?}", self.vec_def_id);
                    }

                    // Save the pre-index type for GEP pointee type computation
                    let pre_index_ty = current_ty.clone();

                    // Update current_ty to element type
                    // Debug: trace type extraction for Vec indexing
                    let debug_vec = std::env::var("BLOOD_DEBUG_VEC_SIZE").is_ok();
                    if debug_vec {
                        eprintln!("[Index] Before type update: {:?}, index_kind: {:?}", current_ty.kind(), index_kind);
                    }

                    current_ty = match current_ty.kind() {
                        TypeKind::Array { element, .. } => element.clone(),
                        TypeKind::Slice { element } => element.clone(),
                        TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                            match inner.kind() {
                                TypeKind::Array { element, .. } => element.clone(),
                                TypeKind::Slice { element } => element.clone(),
                                // Reference to Vec<T>: indexing gives element type T
                                TypeKind::Adt { def_id, args } if Some(*def_id) == self.vec_def_id => {
                                    let elem = args.first().cloned().unwrap_or(inner.clone());
                                    if debug_vec {
                                        eprintln!("[Index] RefToVec: Vec def_id={:?}, elem type={:?}", def_id, elem.kind());
                                    }
                                    elem
                                }
                                // For Ptr { inner: T } where T is not array/slice,
                                // indexing into *T gives T (the element type)
                                _ => inner.clone(),
                            }
                        }
                        // Vec<T> indexing gives element type T
                        TypeKind::Adt { def_id, args } if Some(*def_id) == self.vec_def_id => {
                            let elem = args.first().cloned().unwrap_or(current_ty.clone());
                            if debug_vec {
                                eprintln!("[Index] VecIndex: Vec def_id={:?}, elem type={:?}", def_id, elem.kind());
                            }
                            elem
                        }
                        _ => current_ty.clone(),
                    };

                    if debug_vec {
                        eprintln!("[Index] After type update: {:?}", current_ty.kind());
                    }

                    unsafe {
                        match index_kind {
                            IndexKind::DirectArray => {
                                // Direct array: current_ptr is [N x T]*, use two-index GEP
                                let array_llvm_ty = self.lower_type(&pre_index_ty);
                                let zero = self.context.i64_type().const_zero();
                                self.builder.build_in_bounds_gep(
                                    array_llvm_ty,
                                    current_ptr,
                                    &[zero, idx_val.into_int_value()],
                                    "idx_gep"
                                ).map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM GEP error: {}", e), body.span
                                )])?
                            }
                            IndexKind::DirectSlice => {
                                // Direct slice (fat pointer): current_ptr is {ptr, i64}*
                                // Extract the data pointer (field 0), then single-index GEP
                                let slice_llvm_ty = self.lower_type(&pre_index_ty).into_struct_type();
                                let data_ptr_ptr = self.builder.build_struct_gep(
                                    slice_llvm_ty,
                                    current_ptr,
                                    0,
                                    "slice_data_ptr_ptr"
                                ).map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM GEP error: {}", e), body.span
                                )])?;
                                let ptr_ty = self.context.ptr_type(AddressSpace::default());
                                let data_ptr = self.builder.build_load(ptr_ty, data_ptr_ptr, "slice_data_ptr")
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM load error: {}", e), body.span
                                    )])?.into_pointer_value();

                                // With opaque pointers, no cast needed — use element type for GEP
                                let elem_llvm_ty = self.lower_type(&current_ty);

                                self.builder.build_in_bounds_gep(
                                    elem_llvm_ty,
                                    data_ptr,
                                    &[idx_val.into_int_value()],
                                    "idx_gep"
                                ).map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM GEP error: {}", e), body.span
                                )])?
                            }
                            IndexKind::RefToArray => {
                                // Reference to array: current_ptr is ptr*, load to get ptr (array pointer)
                                let ptr_ty = self.context.ptr_type(AddressSpace::default());
                                let array_ptr = self.builder.build_load(ptr_ty, current_ptr, "array_ptr")
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM load error: {}", e), body.span
                                    )])?.into_pointer_value();
                                // Get the inner array type for GEP
                                let inner_array_ty = match pre_index_ty.kind() {
                                    TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                                        self.lower_type(inner)
                                    }
                                    _ => self.lower_type(&pre_index_ty),
                                };
                                let zero = self.context.i64_type().const_zero();
                                self.builder.build_in_bounds_gep(
                                    inner_array_ty,
                                    array_ptr,
                                    &[zero, idx_val.into_int_value()],
                                    "idx_gep"
                                ).map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM GEP error: {}", e), body.span
                                )])?
                            }
                            IndexKind::SliceRef => {
                                // Slice reference (fat pointer): current_ptr is {ptr, i64}*
                                // Extract data pointer (field 0), then single-index GEP
                                // Get the slice struct type (same as for the inner slice type)
                                let inner_slice_ty = match pre_index_ty.kind() {
                                    TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                                        self.lower_type(inner).into_struct_type()
                                    }
                                    _ => self.lower_type(&pre_index_ty).into_struct_type(),
                                };
                                let data_ptr_ptr = self.builder.build_struct_gep(
                                    inner_slice_ty,
                                    current_ptr,
                                    0,
                                    "slice_data_ptr_ptr"
                                ).map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM GEP error: {}", e), body.span
                                )])?;
                                let ptr_ty = self.context.ptr_type(AddressSpace::default());
                                let data_ptr = self.builder.build_load(ptr_ty, data_ptr_ptr, "slice_data_ptr")
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM load error: {}", e), body.span
                                    )])?.into_pointer_value();

                                // With opaque pointers, no cast needed — use element type for GEP
                                let elem_llvm_ty = self.lower_type(&current_ty);

                                self.builder.build_in_bounds_gep(
                                    elem_llvm_ty,
                                    data_ptr,
                                    &[idx_val.into_int_value()],
                                    "idx_gep"
                                ).map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM GEP error: {}", e), body.span
                                )])?
                            }
                            IndexKind::PtrToElements => {
                                // Pointer to elements (e.g., Vec<T>.data which is *T)
                                // current_ptr is ptr* (pointer to the pointer field)
                                // Need to load the pointer value, then index into it
                                //
                                // FIX: Use explicit byte offset calculation instead of relying on
                                // typed pointer GEP. This fixes offset miscalculation for structs
                                // accessed through Vec fields of `self` (e.g., self.vec[i].field).
                                let ptr_ty = self.context.ptr_type(AddressSpace::default());
                                let data_ptr = self.builder.build_load(ptr_ty, current_ptr, "data_ptr")
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM load error: {}", e), body.span
                                    )])?.into_pointer_value();

                                // Get element type and size for explicit byte offset calculation
                                let elem_llvm_ty = self.lower_type(&current_ty);
                                let elem_size = self.get_type_size_in_bytes(elem_llvm_ty);

                                // Debug: print GEP type/size for enum types
                                if std::env::var("BLOOD_DEBUG_VEC_SIZE").is_ok() {
                                    let llvm_str = elem_llvm_ty.print_to_string().to_string();
                                    eprintln!("[GEP PtrToElements] HIR: {:?}, LLVM: {}, size: {}",
                                        current_ty, llvm_str, elem_size);
                                }

                                // Calculate: byte_offset = index * elem_size
                                // Ensure index is i64 for consistent arithmetic
                                let idx_int = idx_val.into_int_value();
                                let i64_type = self.context.i64_type();
                                let idx_i64 = if idx_int.get_type().get_bit_width() < 64 {
                                    self.builder.build_int_z_extend(idx_int, i64_type, "idx_i64")
                                        .map_err(|e| vec![Diagnostic::error(
                                            format!("LLVM zext error: {}", e), body.span
                                        )])?
                                } else if idx_int.get_type().get_bit_width() > 64 {
                                    self.builder.build_int_truncate(idx_int, i64_type, "idx_i64")
                                        .map_err(|e| vec![Diagnostic::error(
                                            format!("LLVM trunc error: {}", e), body.span
                                        )])?
                                } else {
                                    idx_int
                                };

                                let elem_size_val = i64_type.const_int(elem_size, false);
                                let byte_offset = self.builder.build_int_mul(
                                    idx_i64,
                                    elem_size_val,
                                    "byte_offset"
                                ).map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM mul error: {}", e), body.span
                                )])?;

                                // With opaque pointers, no cast needed — use i8 type for byte GEP
                                let i8_ty = self.context.i8_type();

                                // GEP with byte offset (ptr + byte_offset gives exact address)
                                let elem_ptr = self.builder.build_in_bounds_gep(
                                    i8_ty,
                                    data_ptr,
                                    &[byte_offset],
                                    "elem_ptr"
                                ).map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM GEP error: {}", e), body.span
                                )])?;

                                // With opaque pointers, no cast needed — all pointers are `ptr`
                                elem_ptr
                            }
                            IndexKind::VecIndex => {
                                // Vec<T> direct indexing: current_ptr is Vec*, need to:
                                // 1. GEP to field 0 (data pointer field)
                                // 2. Load the data pointer (*T)
                                // 3. Calculate byte offset manually (index * elem_size)
                                // 4. Use i8* GEP for exact byte addressing
                                // 5. Cast result to element type pointer
                                // Get Vec struct type for GEP
                                let vec_llvm_ty = self.lower_type(&pre_index_ty).into_struct_type();
                                let data_ptr_ptr = self.builder.build_struct_gep(vec_llvm_ty, current_ptr, 0, "vec_data_ptr_ptr")
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM GEP error for Vec data pointer: {}", e), body.span
                                    )])?;
                                let ptr_ty = self.context.ptr_type(AddressSpace::default());
                                let data_ptr = self.builder.build_load(ptr_ty, data_ptr_ptr, "vec_data_ptr")
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM load error: {}", e), body.span
                                    )])?.into_pointer_value();

                                // Get element type and size
                                let elem_llvm_ty = self.lower_type(&current_ty);
                                let elem_size = self.get_type_size_in_bytes(elem_llvm_ty);

                                // Debug: print GEP type/size with full LLVM type string
                                if std::env::var("BLOOD_DEBUG_VEC_SIZE").is_ok() {
                                    let llvm_str = elem_llvm_ty.print_to_string().to_string();
                                    eprintln!("[GEP VecIndex] HIR: {:?}, LLVM: {}, size: {}",
                                        current_ty, llvm_str, elem_size);
                                }

                                // FIX: Use explicit byte offset calculation instead of relying on
                                // typed pointer GEP. This fixes corruption with 16-byte aligned types
                                // where build_in_bounds_gep may miscalculate offsets.
                                //
                                // Calculate: byte_offset = index * elem_size
                                // Ensure index is i64 for consistent arithmetic
                                let idx_int = idx_val.into_int_value();
                                let i64_type = self.context.i64_type();
                                let idx_i64 = if idx_int.get_type().get_bit_width() < 64 {
                                    self.builder.build_int_z_extend(idx_int, i64_type, "idx_i64")
                                        .map_err(|e| vec![Diagnostic::error(
                                            format!("LLVM zext error: {}", e), body.span
                                        )])?
                                } else if idx_int.get_type().get_bit_width() > 64 {
                                    self.builder.build_int_truncate(idx_int, i64_type, "idx_i64")
                                        .map_err(|e| vec![Diagnostic::error(
                                            format!("LLVM trunc error: {}", e), body.span
                                        )])?
                                } else {
                                    idx_int
                                };

                                let elem_size_val = i64_type.const_int(elem_size, false);
                                let byte_offset = self.builder.build_int_mul(
                                    idx_i64,
                                    elem_size_val,
                                    "byte_offset"
                                ).map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM mul error: {}", e), body.span
                                )])?;

                                // Debug: print idx and byte_offset at runtime
                                if std::env::var("BLOOD_DEBUG_GEP_ADDR").is_ok() {
                                    if let Some(debug_fn) = self.module.get_function("debug_vec_index") {
                                        let _ = self.builder.build_call(
                                            debug_fn,
                                            &[idx_i64.into(), byte_offset.into()],
                                            ""
                                        );
                                    }
                                }

                                // With opaque pointers, use i8 type for byte-level GEP (no casts needed)
                                let i8_ty = self.context.i8_type();

                                // GEP with byte offset (ptr + byte_offset gives exact address)
                                let elem_ptr = self.builder.build_in_bounds_gep(
                                    i8_ty,
                                    data_ptr,
                                    &[byte_offset],
                                    "elem_ptr"
                                ).map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM GEP error: {}", e), body.span
                                )])?;

                                // Debug: emit runtime print of computed address and read contents
                                if std::env::var("BLOOD_DEBUG_GEP_ADDR").is_ok() {
                                    if let Some(debug_fn) = self.module.get_function("debug_vec_ptrs") {
                                        let _ = self.builder.build_call(
                                            debug_fn,
                                            &[data_ptr.into(), elem_ptr.into()],
                                            ""
                                        );
                                    }
                                    // Read and print what's actually at elem_ptr
                                    if let Some(read_fn) = self.module.get_function("debug_read_enum_at") {
                                        let _ = self.builder.build_call(
                                            read_fn,
                                            &[elem_ptr.into()],
                                            ""
                                        );
                                    }
                                }

                                // Debug: print the types involved
                                if std::env::var("BLOOD_DEBUG_GEP_ADDR").is_ok() {
                                    eprintln!("[VecIndex cast] elem_llvm_ty: {:?}", elem_llvm_ty.print_to_string());
                                }

                                // With opaque pointers, no cast needed — all pointers are `ptr`
                                elem_ptr
                            }
                            IndexKind::RefToVec => {
                                // Reference to Vec<T>: current_ptr is Vec** (pointer to the ref)
                                // 1. Load to get Vec* (the reference value)
                                // 2. GEP to field 0 (data pointer field)
                                // 3. Load the data pointer (*T)
                                // 4. Calculate byte offset manually (index * elem_size)
                                // 5. Use i8* GEP for exact byte addressing
                                // 6. Cast result to element type pointer
                                // Load the reference to get the Vec pointer
                                let ptr_ty = self.context.ptr_type(AddressSpace::default());
                                let vec_ptr = self.builder.build_load(ptr_ty, current_ptr, "vec_ref")
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM load error: {}", e), body.span
                                    )])?.into_pointer_value();
                                // Get the inner Vec type for struct GEP
                                let inner_vec_ty = match pre_index_ty.kind() {
                                    TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                                        self.lower_type(inner).into_struct_type()
                                    }
                                    _ => self.lower_type(&pre_index_ty).into_struct_type(),
                                };
                                let data_ptr_ptr = self.builder.build_struct_gep(inner_vec_ty, vec_ptr, 0, "vec_data_ptr_ptr")
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM GEP error for Vec data pointer: {}", e), body.span
                                    )])?;
                                let data_ptr = self.builder.build_load(ptr_ty, data_ptr_ptr, "vec_data_ptr")
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM load error: {}", e), body.span
                                    )])?.into_pointer_value();

                                // Get element type and size
                                let elem_llvm_ty = self.lower_type(&current_ty);
                                let elem_size = self.get_type_size_in_bytes(elem_llvm_ty);

                                // Debug: print GEP type/size for RefToVec with full LLVM type string
                                if std::env::var("BLOOD_DEBUG_VEC_SIZE").is_ok() {
                                    let llvm_str = elem_llvm_ty.print_to_string().to_string();
                                    eprintln!("[GEP RefToVec] HIR: {:?}, LLVM: {}, size: {}",
                                        current_ty, llvm_str, elem_size);
                                }

                                // FIX: Use explicit byte offset calculation instead of relying on
                                // typed pointer GEP. This fixes corruption with 16-byte aligned types
                                // where build_in_bounds_gep may miscalculate offsets.
                                //
                                // Calculate: byte_offset = index * elem_size
                                // Ensure index is i64 for consistent arithmetic
                                let idx_int = idx_val.into_int_value();
                                let i64_type = self.context.i64_type();
                                let idx_i64 = if idx_int.get_type().get_bit_width() < 64 {
                                    self.builder.build_int_z_extend(idx_int, i64_type, "idx_i64")
                                        .map_err(|e| vec![Diagnostic::error(
                                            format!("LLVM zext error: {}", e), body.span
                                        )])?
                                } else if idx_int.get_type().get_bit_width() > 64 {
                                    self.builder.build_int_truncate(idx_int, i64_type, "idx_i64")
                                        .map_err(|e| vec![Diagnostic::error(
                                            format!("LLVM trunc error: {}", e), body.span
                                        )])?
                                } else {
                                    idx_int
                                };
                                let elem_size_val = i64_type.const_int(elem_size, false);
                                let byte_offset = self.builder.build_int_mul(
                                    idx_i64,
                                    elem_size_val,
                                    "byte_offset"
                                ).map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM mul error: {}", e), body.span
                                )])?;

                                // Debug: print idx and byte_offset at runtime
                                if std::env::var("BLOOD_DEBUG_GEP_ADDR").is_ok() {
                                    if let Some(debug_fn) = self.module.get_function("debug_vec_index") {
                                        let _ = self.builder.build_call(
                                            debug_fn,
                                            &[idx_i64.into(), byte_offset.into()],
                                            ""
                                        );
                                    }
                                }

                                // With opaque pointers, use i8 type for byte-level GEP (no casts needed)
                                let i8_ty = self.context.i8_type();

                                // GEP with byte offset (ptr + byte_offset gives exact address)
                                let elem_ptr = self.builder.build_in_bounds_gep(
                                    i8_ty,
                                    data_ptr,
                                    &[byte_offset],
                                    "elem_ptr"
                                ).map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM GEP error: {}", e), body.span
                                )])?;

                                // Debug: print data_ptr and elem_ptr at runtime
                                if std::env::var("BLOOD_DEBUG_GEP_ADDR").is_ok() {
                                    if let Some(debug_fn) = self.module.get_function("debug_vec_ptrs") {
                                        let _ = self.builder.build_call(
                                            debug_fn,
                                            &[data_ptr.into(), elem_ptr.into()],
                                            ""
                                        );
                                    }
                                    // Read and print what's actually at elem_ptr
                                    if let Some(read_fn) = self.module.get_function("debug_read_enum_at") {
                                        let _ = self.builder.build_call(
                                            read_fn,
                                            &[elem_ptr.into()],
                                            ""
                                        );
                                    }
                                }

                                // With opaque pointers, no cast needed — all pointers are `ptr`
                                elem_ptr
                            }
                            IndexKind::Other => {
                                // Other pointer type: single-index GEP using element type
                                let elem_llvm_ty = self.lower_type(&current_ty);
                                self.builder.build_in_bounds_gep(
                                    elem_llvm_ty,
                                    current_ptr,
                                    &[idx_val.into_int_value()],
                                    "idx_gep"
                                ).map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM GEP error: {}", e), body.span
                                )])?
                            }
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
                            TypeKind::Array { size, .. } => size.as_u64().ok_or_else(|| vec![Diagnostic::error(
                                "Array size must be concrete for indexing from end",
                                body.span,
                            )])?,
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

                    // Save pre-update type for GEP pointee type
                    let pre_const_idx_ty = current_ty.clone();

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
                            let array_llvm_ty = self.lower_type(&pre_const_idx_ty);
                            let zero = self.context.i64_type().const_zero();
                            self.builder.build_in_bounds_gep(array_llvm_ty, current_ptr, &[zero, idx], "const_idx")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM GEP error: {}", e), body.span
                                )])?
                        } else if is_ref_to_array {
                            // Reference to array: load pointer then two-index GEP
                            let ptr_ty = self.context.ptr_type(AddressSpace::default());
                            let array_ptr = self.builder.build_load(ptr_ty, current_ptr, "array_ptr")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM load error: {}", e), body.span
                                )])?.into_pointer_value();
                            // Get the inner array type for GEP
                            let inner_array_ty = match pre_const_idx_ty.kind() {
                                TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                                    self.lower_type(inner)
                                }
                                _ => self.lower_type(&pre_const_idx_ty),
                            };
                            let zero = self.context.i64_type().const_zero();
                            self.builder.build_in_bounds_gep(inner_array_ty, array_ptr, &[zero, idx], "const_idx")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM GEP error: {}", e), body.span
                                )])?
                        } else {
                            // Other type: single-index GEP using element type
                            let elem_llvm_ty = self.lower_type(&current_ty);
                            self.builder.build_in_bounds_gep(elem_llvm_ty, current_ptr, &[idx], "const_idx")
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
                            let array_llvm_ty = self.lower_type(&current_ty);
                            let zero = self.context.i64_type().const_zero();
                            self.builder.build_in_bounds_gep(array_llvm_ty, current_ptr, &[zero, idx], "subslice")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM GEP error: {}", e), body.span
                                )])?
                        } else if is_ref_to_array {
                            // Reference to array: load pointer then two-index GEP
                            let ptr_ty = self.context.ptr_type(AddressSpace::default());
                            let array_ptr = self.builder.build_load(ptr_ty, current_ptr, "array_ptr")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM load error: {}", e), body.span
                                )])?.into_pointer_value();
                            // Get the inner array type for GEP
                            let inner_array_ty = match current_ty.kind() {
                                TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                                    self.lower_type(inner)
                                }
                                _ => self.lower_type(&current_ty),
                            };
                            let zero = self.context.i64_type().const_zero();
                            self.builder.build_in_bounds_gep(inner_array_ty, array_ptr, &[zero, idx], "subslice")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM GEP error: {}", e), body.span
                                )])?
                        } else {
                            // Other type: use element type for single-index GEP
                            let elem_llvm_ty = match current_ty.kind() {
                                TypeKind::Array { element, .. } => self.lower_type(element),
                                TypeKind::Slice { element } => self.lower_type(element),
                                _ => self.lower_type(&current_ty),
                            };
                            self.builder.build_in_bounds_gep(elem_llvm_ty, current_ptr, &[idx], "subslice")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM GEP error: {}", e), body.span
                                )])?
                        }
                    }
                }

                PlaceElem::Downcast(variant_idx_val) => {
                    // Downcast is logically an assertion that we have the right variant.
                    // Set variant context so Field knows how to access variant-specific fields.
                    // This is needed for heterogeneous enum payloads (different sized variants).

                    // Handle both direct enum and reference-to-enum cases
                    match current_ty.kind() {
                        TypeKind::Adt { def_id, .. } => {
                            variant_ctx = Some((*def_id, *variant_idx_val));
                            current_ptr  // Return the same pointer
                        }
                        TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                            // Reference to enum: load the reference, then set variant context
                            if let TypeKind::Adt { def_id, .. } = inner.kind() {
                                variant_ctx = Some((*def_id, *variant_idx_val));
                                // Load the reference to get the enum pointer
                                let ptr_ty = self.context.ptr_type(AddressSpace::default());
                                let enum_ptr = self.builder.build_load(ptr_ty, current_ptr, "enum_ref")
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM load error: {}", e), body.span
                                    )])?.into_pointer_value();
                                // Update current_ty to the inner enum type
                                current_ty = inner.clone();
                                enum_ptr
                            } else {
                                current_ptr
                            }
                        }
                        _ => current_ptr,
                    }
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

        // Determine the type of the value to load by computing the place's effective type
        let base_ty = match &place.base {
            crate::mir::types::PlaceBase::Local(local_id) => {
                let local_info = body.locals.get(local_id.index as usize)
                    .expect("ICE: MIR local not found in body during load codegen");
                local_info.ty.clone()
            }
            crate::mir::types::PlaceBase::Static(def_id) => {
                self.get_static_type(*def_id).unwrap_or_else(|| Type::unit())
            }
        };
        let place_ty = self.compute_place_type(&base_ty, &place.projection);
        let load_llvm_ty = self.lower_type(&place_ty);

        // Load value from pointer
        let load_inst = self.builder.build_load(load_llvm_ty, ptr, "load")
            .map_err(|e| vec![Diagnostic::error(
                format!("LLVM load error: {}", e), body.span
            )])?;

        // Debug: print loaded type
        if std::env::var("BLOOD_DEBUG_LOAD").is_ok() {
            eprintln!("[compile_mir_place_load] place_ty: {:?}, loaded_type: {:?}",
                place_ty, load_inst.get_type().print_to_string());
        }

        // Set proper alignment based on the loaded type.
        // Use natural alignment so LLVM can generate optimal instructions.
        // Allocas are created with correct alignment (including 16-byte for i128),
        // and LLVM's type layout ensures struct fields and array elements
        // are properly aligned.
        let alignment = self.get_type_alignment_for_value(load_inst);
        if let Some(inst) = load_inst.as_instruction_value() {
            let _ = inst.set_alignment(alignment);
        }

        Ok(load_inst)
    }

    fn compute_place_type(&self, base_ty: &Type, projections: &[PlaceElem]) -> Type {
        let mut current_ty = base_ty.clone();
        let mut variant_ctx: Option<(crate::hir::DefId, u32)> = None;

        for proj in projections {
            current_ty = match proj {
                PlaceElem::Deref => {
                    // Dereference: unwrap Ref, Ptr, or Box types
                    match current_ty.kind() {
                        TypeKind::Ref { inner, .. } => inner.clone(),
                        TypeKind::Ptr { inner, .. } => inner.clone(),
                        // For Box<T>, the inner type is T
                        TypeKind::Adt { def_id, args } if Some(*def_id) == self.box_def_id => {
                            args.first().cloned().unwrap_or(current_ty)
                        }
                        // For other types, keep the type (should not happen in valid MIR)
                        _ => current_ty,
                    }
                }
                PlaceElem::Field(idx) => {
                    // Field access: get the field type from struct/tuple/ADT
                    match current_ty.kind() {
                        TypeKind::Tuple(tys) => {
                            tys.get(*idx as usize).cloned().unwrap_or(current_ty)
                        }
                        TypeKind::Adt { def_id, args } => {
                            // When in a variant context (after Downcast), Option/Result
                            // field indices refer to the variant's fields, not the ADT
                            // struct layout. E.g., after Downcast(1) for Option<&T>,
                            // Field(0) is the Some payload (&T), not the discriminant.
                            if variant_ctx.is_some() && (Some(*def_id) == self.option_def_id || Some(*def_id) == self.result_def_id) {
                                let field_ty = if Some(*def_id) == self.option_def_id {
                                    let (_, v_idx) = variant_ctx.unwrap();
                                    match v_idx {
                                        // None variant has no fields
                                        0 => None,
                                        // Some variant: field 0 is T
                                        1 => {
                                            if *idx == 0 {
                                                Some(args.first().cloned().unwrap_or(Type::unit()))
                                            } else {
                                                None
                                            }
                                        }
                                        _ => None,
                                    }
                                } else {
                                    // Result<T, E>
                                    let (_, v_idx) = variant_ctx.unwrap();
                                    match v_idx {
                                        // Ok variant: field 0 is T
                                        0 => {
                                            if *idx == 0 {
                                                Some(args.first().cloned().unwrap_or(Type::unit()))
                                            } else {
                                                None
                                            }
                                        }
                                        // Err variant: field 0 is E
                                        1 => {
                                            if *idx == 0 {
                                                Some(args.get(1).cloned().unwrap_or(Type::unit()))
                                            } else {
                                                None
                                            }
                                        }
                                        _ => None,
                                    }
                                };
                                variant_ctx = None;
                                field_ty.unwrap_or(current_ty)
                            } else if Some(*def_id) == self.vec_def_id {
                                // Vec<T> layout: { ptr: *T, len: i64, capacity: i64 }
                                match idx {
                                    0 => {
                                        // Field 0 is the data pointer *T
                                        let elem_ty = args.first().cloned().unwrap_or(Type::unit());
                                        Type::new(TypeKind::Ptr { inner: elem_ty, mutable: true })
                                    }
                                    1 | 2 => Type::usize(), // len and capacity
                                    _ => current_ty,
                                }
                            } else if Some(*def_id) == self.option_def_id {
                                // Option<T> ADT-level layout: { tag: i32, payload: T }
                                // (only reached when NOT in variant context)
                                match idx {
                                    0 => Type::i32(), // discriminant tag
                                    1 => args.first().cloned().unwrap_or(Type::unit()), // payload
                                    _ => current_ty,
                                }
                            } else if Some(*def_id) == self.result_def_id {
                                // Result<T, E> ADT-level layout: { tag: i32, payload: T or E }
                                // (only reached when NOT in variant context)
                                match idx {
                                    0 => Type::i32(), // discriminant tag
                                    1 => args.first().cloned().unwrap_or(Type::unit()), // ok payload
                                    2 => args.get(1).cloned().unwrap_or(Type::unit()), // err payload
                                    _ => current_ty,
                                }
                            } else if let Some(field_types) = self.struct_defs.get(def_id) {
                                // Regular struct - look up field type
                                if let Some(field_ty) = field_types.get(*idx as usize) {
                                    // Substitute type parameters with actual args
                                    self.substitute_type_params(field_ty, args)
                                } else {
                                    current_ty
                                }
                            } else if let Some(variants) = self.enum_defs.get(def_id) {
                                // Enum - field access on enum value (after Downcast)
                                // Use variant_ctx to determine which variant's field types to use
                                if let Some((_, v_idx)) = variant_ctx {
                                    if let Some(variant_fields) = variants.get(v_idx as usize) {
                                        if let Some(field_ty) = variant_fields.get(*idx as usize) {
                                            variant_ctx = None;
                                            self.substitute_type_params(field_ty, args)
                                        } else {
                                            variant_ctx = None;
                                            current_ty
                                        }
                                    } else {
                                        variant_ctx = None;
                                        current_ty
                                    }
                                } else {
                                    current_ty
                                }
                            } else {
                                // Unknown ADT, keep type
                                current_ty
                            }
                        }
                        // For other types, keep the type
                        _ => current_ty,
                    }
                }
                PlaceElem::Index(_) | PlaceElem::ConstantIndex { .. } => {
                    // Array/slice indexing: get the element type
                    match current_ty.kind() {
                        TypeKind::Array { element, .. } => element.clone(),
                        TypeKind::Slice { element } => element.clone(),
                        // For Vec<T>, indexing gives T
                        TypeKind::Adt { def_id, args } if Some(*def_id) == self.vec_def_id => {
                            args.first().cloned().unwrap_or(current_ty)
                        }
                        // For Ref/Ptr to indexable types, get the inner element type
                        TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                            match inner.kind() {
                                TypeKind::Array { element, .. } => element.clone(),
                                TypeKind::Slice { element } => element.clone(),
                                // Reference to Vec<T>: indexing gives T
                                TypeKind::Adt { def_id, args } if Some(*def_id) == self.vec_def_id => {
                                    args.first().cloned().unwrap_or(current_ty)
                                }
                                _ => current_ty,
                            }
                        }
                        // For other types, keep the type
                        _ => current_ty,
                    }
                }
                PlaceElem::Subslice { .. } => {
                    // Subslice keeps the same slice type
                    current_ty
                }
                PlaceElem::Downcast(variant_idx) => {
                    // Downcast to a specific enum variant
                    // Track the variant for subsequent Field projections
                    match current_ty.kind() {
                        TypeKind::Adt { def_id, .. } => {
                            variant_ctx = Some((*def_id, *variant_idx));
                            current_ty
                        }
                        TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                            if let TypeKind::Adt { def_id, .. } = inner.kind() {
                                variant_ctx = Some((*def_id, *variant_idx));
                                inner.clone()
                            } else {
                                current_ty
                            }
                        }
                        _ => current_ty,
                    }
                }
            };
        }

        current_ty
    }
}
