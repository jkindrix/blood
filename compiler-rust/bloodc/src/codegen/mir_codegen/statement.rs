//! MIR statement code generation.
//!
//! This module handles compilation of MIR statements to LLVM IR.

use inkwell::types::BasicType;
use inkwell::values::BasicValueEnum;
use inkwell::{AddressSpace, IntPredicate};

use crate::diagnostics::Diagnostic;
use crate::effects::evidence::HandlerStateKind;
use crate::hir::TypeKind;
use crate::mir::body::MirBody;
use crate::mir::ptr::MemoryTier;
use crate::mir::static_evidence::InlineEvidenceMode;
use crate::mir::types::{Statement, StatementKind, Rvalue, Operand};
use crate::mir::EscapeResults;

use super::rvalue::MirRvalueCodegen;
use super::place::MirPlaceCodegen;
use super::types::MirTypesCodegen;
use super::CodegenContext;

/// Extension trait for MIR statement compilation.
pub trait MirStatementCodegen<'ctx, 'a> {
    /// Compile a MIR statement.
    fn compile_mir_statement(
        &mut self,
        stmt: &Statement,
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
    ) -> Result<(), Vec<Diagnostic>>;
}

impl<'ctx, 'a> MirStatementCodegen<'ctx, 'a> for CodegenContext<'ctx, 'a> {
    fn compile_mir_statement(
        &mut self,
        stmt: &Statement,
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
    ) -> Result<(), Vec<Diagnostic>> {
        match &stmt.kind {
            StatementKind::Assign(place, rvalue) => {
                // Check if the rvalue is from a Never-typed source (diverging expression).
                // Assignments from Never-typed sources are unreachable code - the diverging
                // expression never returns, so this assignment will never execute.
                // We skip codegen for these to avoid LLVM type mismatches (Never is i8,
                // but destination might be a complex ADT).
                //
                // Only Use(Copy/Move) can reference a Never-typed place — other rvalue kinds
                // (BinaryOp, UnaryOp, Aggregate, Ref, Cast, Len, Discriminant) produce concrete
                // typed results and cannot produce a Never-typed value directly.
                let is_never_source = match rvalue {
                    Rvalue::Use(Operand::Copy(src_place)) | Rvalue::Use(Operand::Move(src_place)) => {
                        let src_ty = self.get_place_base_type(src_place, body)?;
                        let src_effective_ty = self.compute_place_type(&src_ty, &src_place.projection);
                        matches!(src_effective_ty.kind(), TypeKind::Never)
                    }
                    _ => false,
                };

                if is_never_source {
                    // This assignment is unreachable - the source expression diverges.
                    // We can safely skip the store. Optionally emit unreachable, but
                    // only if we're certain the block continues (which we're not here).
                    // The MIR should end with an unreachable terminator after diverging.
                    return Ok(());
                }

                // Pass the destination local to enable escape analysis for closures.
                // This allows the codegen to decide whether to heap-allocate closure
                // environments (for escaping closures) or stack-allocate them.
                let value = self.compile_mir_rvalue_with_dest(
                    rvalue,
                    body,
                    escape_results,
                    place.as_local(),
                )?;
                let ptr = self.compile_mir_place(place, body, escape_results)?;

                // Debug: trace assignment
                if std::env::var("BLOOD_DEBUG_ASSIGN").is_ok() {
                    eprintln!("[Assign] rvalue: {:?}", rvalue);
                    eprintln!("[Assign] value type: {:?}", value.get_type().print_to_string());
                    eprintln!("[Assign] dest ptr type: {:?}", ptr.get_type().print_to_string());
                }

                // Compute the destination's LLVM type for store conversion.
                // With opaque pointers, we must compute this explicitly since PointerType
                // no longer carries element type information.
                let dest_base_ty = self.get_place_base_type(place, body)?;
                let dest_effective_ty = self.compute_place_type(&dest_base_ty, &place.projection);
                let dest_llvm_ty = self.lower_type(&dest_effective_ty);

                // Convert value to match destination type if needed
                let converted_value = self.convert_value_for_store(value, dest_llvm_ty, stmt.span)?;
                let store_inst = self.builder.build_store(ptr, converted_value)
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM store error: {}", e), stmt.span
                    )])?;
                // Set proper alignment for the store based on the value's type.
                // Use natural alignment so LLVM can generate optimal instructions.
                // Allocas are created with correct alignment (including 16-byte for i128),
                // and LLVM's type layout ensures struct fields and array elements
                // are properly aligned.
                let alignment = self.get_type_alignment_for_value(converted_value);
                let _ = store_inst.set_alignment(alignment);
            }

            StatementKind::StorageLive(_local) => {
                // Storage annotations - can be used for LLVM lifetime intrinsics
                // For now, no-op since we allocate at function start
            }

            StatementKind::StorageDead(local) => {
                // If this local was region-allocated (has entry in local_generations),
                // we must unregister it to invalidate its generation. This enables
                // use-after-free detection: any subsequent dereference with the old
                // generation will fail validation.
                if let Some(&gen_alloca) = self.local_generations.get(local) {
                    // Get the local's pointer storage
                    if let Some(&local_ptr) = self.locals.get(local) {
                        let i64_ty = self.context.i64_type();

                        // Load the address from the local's storage
                        let loaded = self.builder.build_load(self.context.ptr_type(AddressSpace::default()), local_ptr, "local_addr")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM load error: {}", e), stmt.span
                            )])?;

                        // Convert pointer to i64 address for unregister call
                        let address = if loaded.is_pointer_value() {
                            self.builder.build_ptr_to_int(
                                loaded.into_pointer_value(),
                                i64_ty,
                                "addr_for_unregister"
                            ).map_err(|e| vec![Diagnostic::error(
                                format!("LLVM ptr_to_int error: {}", e), stmt.span
                            )])?
                        } else if loaded.is_int_value() {
                            // Already an integer (the address itself)
                            loaded.into_int_value()
                        } else {
                            // For other types, use the pointer itself
                            self.builder.build_ptr_to_int(
                                local_ptr,
                                i64_ty,
                                "addr_for_unregister"
                            ).map_err(|e| vec![Diagnostic::error(
                                format!("LLVM ptr_to_int error: {}", e), stmt.span
                            )])?
                        };

                        // Call blood_unregister_allocation to invalidate the generation
                        let unregister_fn = self.module.get_function("blood_unregister_allocation")
                            .ok_or_else(|| vec![Diagnostic::error(
                                "Runtime function blood_unregister_allocation not found",
                                stmt.span
                            )])?;

                        self.builder.build_call(unregister_fn, &[address.into()], "")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM call error: {}", e), stmt.span
                            )])?;

                        // Remove from local_generations to prevent double-unregister
                        // Note: We don't have &mut self here, so we rely on the local
                        // not being used after StorageDead (which is a correctness invariant)
                    }
                    let _ = gen_alloca; // Suppress unused warning
                }

                // If this local was persistent-allocated (has entry in persistent_slot_ids),
                // we must decrement its reference count. This enables automatic RC lifecycle:
                // when the local goes out of scope, the persistent slot's refcount decreases,
                // and the slot is freed when refcount reaches zero.
                if let Some(&slot_id_alloca) = self.persistent_slot_ids.get(local) {
                    let i64_ty = self.context.i64_type();

                    // Load the slot ID from the alloca
                    let slot_id = self.builder.build_load(i64_ty, slot_id_alloca, "persistent_slot_id")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM load error: {}", e), stmt.span
                        )])?
                        .into_int_value();

                    // Only decrement if slot_id != 0 (0 means allocation failed)
                    let is_valid = self.builder.build_int_compare(
                        IntPredicate::NE,
                        slot_id,
                        i64_ty.const_int(0, false),
                        "slot_id_valid"
                    ).map_err(|e| vec![Diagnostic::error(
                        format!("LLVM compare error: {}", e), stmt.span
                    )])?;

                    let current_fn = self.current_fn.ok_or_else(|| {
                        vec![Diagnostic::error("No current function", stmt.span)]
                    })?;

                    let decrement_bb = self.context.append_basic_block(current_fn, "persistent_decrement");
                    let after_bb = self.context.append_basic_block(current_fn, "after_persistent_decrement");

                    self.builder.build_conditional_branch(is_valid, decrement_bb, after_bb)
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM branch error: {}", e), stmt.span
                        )])?;

                    // Decrement block
                    self.builder.position_at_end(decrement_bb);

                    // Get or declare blood_persistent_decrement(id: u64)
                    let decrement_fn = self.module.get_function("blood_persistent_decrement")
                        .unwrap_or_else(|| {
                            let fn_type = self.context.void_type().fn_type(&[i64_ty.into()], false);
                            self.module.add_function("blood_persistent_decrement", fn_type, None)
                        });

                    self.builder.build_call(decrement_fn, &[slot_id.into()], "")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM call error: {}", e), stmt.span
                        )])?;

                    self.builder.build_unconditional_branch(after_bb)
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM branch error: {}", e), stmt.span
                        )])?;

                    // Continue after decrement
                    self.builder.position_at_end(after_bb);
                }
            }

            StatementKind::Drop(place) => {
                // Drop value - free memory if heap allocated
                // Get the address of the place
                let ptr = self.compile_mir_place(place, body, escape_results)?;

                // Get the type to determine size
                let place_ty = &body.locals[place.local_unchecked().index as usize].ty;
                let llvm_ty = self.lower_type(place_ty);
                let size = llvm_ty.size_of()
                    .map(|s| s.const_truncate_or_bit_cast(self.context.i64_type()))
                    .unwrap_or_else(|| self.context.i64_type().const_int(0, false));

                // For reference types, call blood_free to deallocate
                if place_ty.is_ref() {
                    if let Some(free_fn) = self.module.get_function("blood_free") {
                        // Load the pointer value
                        // Load the pointer/value from the reference place.
                        // Reference types typically store pointer values, but may store
                        // BloodPtr (i128) fat pointers. We load as the lowered type.
                        let drop_load_ty = self.lower_type(place_ty);
                        let ptr_val = self.builder.build_load(drop_load_ty, ptr, "drop_val")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM load error: {}", e), stmt.span)])?;

                        // Convert to i64 address
                        let address = if ptr_val.is_pointer_value() {
                            self.builder.build_ptr_to_int(
                                ptr_val.into_pointer_value(),
                                self.context.i64_type(),
                                "drop_addr"
                            ).map_err(|e| vec![Diagnostic::error(format!("LLVM ptr_to_int error: {}", e), stmt.span)])?
                        } else if ptr_val.is_int_value() {
                            let int_val = ptr_val.into_int_value();
                            let bit_width = int_val.get_type().get_bit_width();
                            if bit_width == 128 {
                                // Extract address from high 64 bits of BloodPtr
                                let shifted = self.builder.build_right_shift(
                                    int_val,
                                    int_val.get_type().const_int(64, false),
                                    false,
                                    "addr_extract"
                                ).map_err(|e| vec![Diagnostic::error(format!("LLVM shift error: {}", e), stmt.span)])?;
                                self.builder.build_int_truncate(shifted, self.context.i64_type(), "addr")
                                    .map_err(|e| vec![Diagnostic::error(format!("LLVM truncate error: {}", e), stmt.span)])?
                            } else if bit_width < 64 {
                                self.builder.build_int_z_extend(int_val, self.context.i64_type(), "addr")
                                    .map_err(|e| vec![Diagnostic::error(format!("LLVM zext error: {}", e), stmt.span)])?
                            } else {
                                int_val
                            }
                        } else {
                            // Not a freeable type, use zero address (blood_free ignores null)
                            self.context.i64_type().const_int(0, false)
                        };

                        // Call blood_free(address, size)
                        self.builder.build_call(free_fn, &[address.into(), size.into()], "")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM call error: {}", e), stmt.span)])?;
                    }
                }
                // For non-reference types, no deallocation needed
            }

            StatementKind::IncrementGeneration(place) => {
                // Increment generation counter for a slot
                // This is used when freeing/reallocating
                // Requires: blood_increment_generation(address: *void) runtime call
                let ptr = self.compile_mir_place(place, body, escape_results)?;

                // Get or declare the runtime function
                let increment_fn = self.module.get_function("blood_increment_generation")
                    .ok_or_else(|| vec![Diagnostic::error(
                        "Runtime function blood_increment_generation not found. \
                         IncrementGeneration requires this function to be declared.",
                        stmt.span
                    )])?;

                // Call the runtime function to increment the generation
                self.builder.build_call(increment_fn, &[ptr.into()], "")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM call error: {}", e), stmt.span
                    )])?;
            }

            StatementKind::CaptureSnapshot(local) => {
                // CaptureSnapshot statements are intentionally no-ops in codegen.
                //
                // The snapshot lifecycle is handled entirely by the Perform terminator:
                // 1. Perform creates a snapshot via blood_snapshot_create()
                // 2. Perform iterates effect-captured locals (from escape analysis)
                // 3. For each captured local, Perform calls blood_snapshot_add_entry()
                // 4. After the effect operation returns, Perform validates the snapshot
                // 5. Perform destroys the snapshot via blood_snapshot_destroy()
                //
                // CaptureSnapshot statements exist in MIR for:
                // - Future optimization passes that may want per-statement granularity
                // - Documentation of which values are being captured
                // - Alternative backends that prefer statement-level capture
                //
                // The current LLVM backend uses bulk capture at Perform time instead.
                let _ = local;
            }

            StatementKind::ValidateGeneration { ptr, expected_gen } => {
                // Check if generation validation can be skipped based on escape analysis.
                // For stack-allocated values (NoEscape), generation checks are unnecessary
                // because the reference is guaranteed valid within the scope.
                // Static places never need generation checks.
                let should_skip = ptr.is_static() || if let (Some(results), Some(local)) = (escape_results, ptr.as_local()) {
                    // Check if this local is stack-promotable (NoEscape and not effect-captured)
                    results.stack_promotable.contains(&local)
                } else {
                    false
                };

                if !should_skip {
                    // Validate generation check for region-allocated values
                    let ptr_val = self.compile_mir_place(ptr, body, escape_results)?;
                    let expected = self.compile_mir_operand(expected_gen, body, escape_results)?;
                    if let inkwell::values::BasicValueEnum::IntValue(gen_val) = expected {
                        super::memory::emit_generation_check_impl(self, ptr_val, gen_val, stmt.span)?;
                    }
                }
                // Else: Stack-allocated value - skip generation check (optimization)
            }

            StatementKind::PushHandler { handler_id, state_place, state_kind, allocation_tier, inline_mode } => {
                // Push effect handler onto the evidence vector with instance state
                //
                // STATIC EVIDENCE OPTIMIZATION (EFF-OPT-001):
                // When state_kind is Stateless, Constant, or ZeroInit, we can use
                // optimized paths that avoid runtime allocation:
                // - Stateless: Handler has no state, minimal overhead
                // - Constant: State is compile-time known, can be embedded
                // - ZeroInit: State is zero-initialized, can use BSS
                // - Dynamic: Must use full runtime allocation (current path)
                //
                // Currently all state kinds use the dynamic path. When static evidence
                // optimization is implemented, Stateless/Constant/ZeroInit can skip
                // runtime allocation.
                // EFF-OPT-001: State kind determines how the state pointer is prepared.
                // Optimized state kinds avoid heap allocation for handler state.
                // The resolved_state_ptr is set below based on state_kind.
                let _ = state_kind; // Used below when computing resolved_state_ptr

                // STACK ALLOCATION OPTIMIZATION (EFF-OPT-005/006):
                // When allocation_tier is Stack, the handler is lexically scoped
                // (no Perform/Resume that could capture the continuation).
                // We can skip cloning the evidence vector (blood_evidence_create)
                // and push directly onto the existing one, avoiding heap allocation.
                // NOTE: allocation_tier is already used below to differentiate stack vs region paths.
                //
                // INLINE EVIDENCE OPTIMIZATION (EFF-OPT-003/004):
                // When inline_mode is Inline or SpecializedPair, the handler evidence
                // can potentially be passed directly without going through the vector.
                // - Inline: Single handler in scope, can use direct passing
                // - SpecializedPair: Two handlers, can use fixed-size pair structure
                // - Vector: Three or more handlers, must use heap-allocated vector
                //
                // Currently all modes use the vector-based implementation. When Inline mode
                // optimization is implemented, single-handler cases can skip the evidence
                // vector entirely and pass the handler entry directly in registers/stack.
                let i64_ty = self.context.i64_type();
                let i8_ptr_ty = self.context.ptr_type(AddressSpace::default());

                // Look up the handler's effect_id
                let handler_info = self.handler_defs.get(handler_id).ok_or_else(|| {
                    vec![Diagnostic::error(
                        format!("Internal error: no handler info for DefId({})", handler_id.index),
                        stmt.span,
                    )]
                })?;
                let effect_id = handler_info.effect_id;

                // Get the state pointer from state_place
                // state_place is a simple local (no projections), so we look it up directly
                let state_local = state_place.as_local().ok_or_else(|| {
                    vec![Diagnostic::error("Handler state must be a local", stmt.span)]
                })?;
                let state_ptr = *self.locals.get(&state_local).ok_or_else(|| {
                    vec![Diagnostic::error(
                        format!("Local _{} not found for handler state", state_local.index),
                        stmt.span,
                    )]
                })?;

                // Declare or get evidence functions
                let ev_current = self.module.get_function("blood_evidence_current")
                    .unwrap_or_else(|| {
                        let fn_type = i8_ptr_ty.fn_type(&[], false);
                        self.module.add_function("blood_evidence_current", fn_type, None)
                    });
                let ev_create = self.module.get_function("blood_evidence_create")
                    .unwrap_or_else(|| {
                        let fn_type = i8_ptr_ty.fn_type(&[], false);
                        self.module.add_function("blood_evidence_create", fn_type, None)
                    });
                let ev_push_with_state = self.module.get_function("blood_evidence_push_with_state")
                    .unwrap_or_else(|| {
                        let fn_type = self.context.void_type().fn_type(
                            &[i8_ptr_ty.into(), i64_ty.into(), i8_ptr_ty.into()],
                            false
                        );
                        self.module.add_function("blood_evidence_push_with_state", fn_type, None)
                    });
                let ev_set_current = self.module.get_function("blood_evidence_set_current")
                    .unwrap_or_else(|| {
                        let fn_type = self.context.void_type().fn_type(&[i8_ptr_ty.into()], false);
                        self.module.add_function("blood_evidence_set_current", fn_type, None)
                    });

                // Get current evidence vector
                let current_ev = self.builder.build_call(ev_current, &[], "current_ev")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM call error: {}", e), stmt.span
                    )])?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| vec![Diagnostic::error(
                        "blood_evidence_current returned void",
                        stmt.span
                    )])?;

                // Check if current evidence is null
                let is_null = self.builder.build_is_null(
                    current_ev.into_pointer_value(),
                    "ev_is_null"
                ).map_err(|e| vec![Diagnostic::error(
                    format!("LLVM error: {}", e), stmt.span
                )])?;

                // Get current function for creating blocks
                let current_fn = self.current_fn.ok_or_else(|| {
                    vec![Diagnostic::error("No current function", stmt.span)]
                })?;

                // STACK ALLOCATION OPTIMIZATION (EFF-OPT-005/006):
                // For stack-tier handlers, we can push directly onto the existing
                // evidence vector without cloning (avoiding heap allocation).
                // For region-tier handlers, we must clone to support potential escapes.
                let ev = if *allocation_tier == MemoryTier::Stack {
                    // Stack-tier: Push directly onto existing evidence, create only if null
                    let create_block = self.context.append_basic_block(current_fn, "stack_create_ev");
                    let use_block = self.context.append_basic_block(current_fn, "stack_use_ev");
                    let merge_block = self.context.append_basic_block(current_fn, "stack_merge_ev");

                    // Branch: if null, create new evidence; else use existing directly
                    self.builder.build_conditional_branch(is_null, create_block, use_block)
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM error: {}", e), stmt.span
                        )])?;

                    // Create block: evidence is null, create new one
                    self.builder.position_at_end(create_block);
                    let new_ev = self.builder.build_call(ev_create, &[], "new_evidence")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM call error: {}", e), stmt.span
                        )])?
                        .try_as_basic_value()
                        .basic()
                        .ok_or_else(|| vec![Diagnostic::error(
                            "blood_evidence_create returned void",
                            stmt.span
                        )])?;
                    // Set as current since we created a new one
                    self.builder.build_call(ev_set_current, &[new_ev.into()], "")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM call error: {}", e), stmt.span
                        )])?;
                    self.builder.build_unconditional_branch(merge_block)
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM error: {}", e), stmt.span
                        )])?;
                    let create_block_end = self.builder.get_insert_block()
                        .ok_or_else(|| vec![Diagnostic::error(
                            "LLVM builder state invalid after branch".to_string(), stmt.span
                        )])?;

                    // Use block: evidence exists, use it directly (NO clone, NO set_current)
                    // This is the key optimization: we skip blood_evidence_create's clone
                    self.builder.position_at_end(use_block);
                    self.builder.build_unconditional_branch(merge_block)
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM error: {}", e), stmt.span
                        )])?;
                    let use_block_end = self.builder.get_insert_block()
                        .ok_or_else(|| vec![Diagnostic::error(
                            "LLVM builder state invalid after branch".to_string(), stmt.span
                        )])?;

                    // Merge block: phi to select the evidence pointer
                    self.builder.position_at_end(merge_block);
                    let ev_phi = self.builder.build_phi(i8_ptr_ty, "evidence")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM error: {}", e), stmt.span
                        )])?;
                    ev_phi.add_incoming(&[
                        (&new_ev, create_block_end),
                        (&current_ev, use_block_end),
                    ]);
                    ev_phi.as_basic_value()
                } else {
                    // Region-tier: Always clone evidence vector (existing behavior)
                    // This ensures proper isolation for handlers that may escape
                    let create_block = self.context.append_basic_block(current_fn, "region_create_ev");
                    let clone_block = self.context.append_basic_block(current_fn, "region_clone_ev");
                    let merge_block = self.context.append_basic_block(current_fn, "region_merge_ev");

                    // Branch: if null, create new; else clone existing
                    self.builder.build_conditional_branch(is_null, create_block, clone_block)
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM error: {}", e), stmt.span
                        )])?;

                    // Create block: evidence is null, create new one
                    self.builder.position_at_end(create_block);
                    let new_ev = self.builder.build_call(ev_create, &[], "new_evidence")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM call error: {}", e), stmt.span
                        )])?
                        .try_as_basic_value()
                        .basic()
                        .ok_or_else(|| vec![Diagnostic::error(
                            "blood_evidence_create returned void",
                            stmt.span
                        )])?;
                    self.builder.build_unconditional_branch(merge_block)
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM error: {}", e), stmt.span
                        )])?;
                    let create_block_end = self.builder.get_insert_block()
                        .ok_or_else(|| vec![Diagnostic::error(
                            "LLVM builder state invalid after branch".to_string(), stmt.span
                        )])?;

                    // Clone block: evidence exists, clone it via blood_evidence_create
                    // blood_evidence_create clones the current vector when one exists
                    self.builder.position_at_end(clone_block);
                    let cloned_ev = self.builder.build_call(ev_create, &[], "cloned_evidence")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM call error: {}", e), stmt.span
                        )])?
                        .try_as_basic_value()
                        .basic()
                        .ok_or_else(|| vec![Diagnostic::error(
                            "blood_evidence_create returned void",
                            stmt.span
                        )])?;
                    self.builder.build_unconditional_branch(merge_block)
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM error: {}", e), stmt.span
                        )])?;
                    let clone_block_end = self.builder.get_insert_block()
                        .ok_or_else(|| vec![Diagnostic::error(
                            "LLVM builder state invalid after branch".to_string(), stmt.span
                        )])?;

                    // Merge block: phi to select the evidence pointer
                    self.builder.position_at_end(merge_block);
                    let ev_phi = self.builder.build_phi(i8_ptr_ty, "evidence")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM error: {}", e), stmt.span
                        )])?;
                    ev_phi.add_incoming(&[
                        (&new_ev, create_block_end),
                        (&cloned_ev, clone_block_end),
                    ]);
                    let ev = ev_phi.as_basic_value();

                    // Set as current evidence vector (needed for region-tier since we cloned)
                    self.builder.build_call(ev_set_current, &[ev.into()], "")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM call error: {}", e), stmt.span
                        )])?;

                    ev
                };

                // Push handler with effect_id and state pointer
                let effect_id_val = i64_ty.const_int(effect_id.index as u64, false);

                // EFF-OPT-001: Resolve state pointer based on handler state kind.
                // Stateless handlers use null, avoiding any allocation overhead.
                // Constant/ZeroInit handlers use stack allocation, avoiding heap allocation.
                // Dynamic handlers use the full state place (which may be heap-allocated).
                //
                // UNIFORM i64 STATE LAYOUT:
                // Handler op bodies are compiled once per handler definition (not per
                // instantiation), so they use a uniform {i64, i64, ...} layout for
                // state fields. We create a "shadow" alloca with this i64 layout,
                // copy the typed state fields into it (with zero-extension), and pass
                // the shadow pointer to the evidence system. This bridges the typed
                // caller-side state struct with the handler body's i64 view.
                let state_void_ptr = match state_kind {
                    HandlerStateKind::Stateless => {
                        // No state needed - pass null pointer
                        i8_ptr_ty.const_null()
                    }
                    HandlerStateKind::Constant | HandlerStateKind::ZeroInit | HandlerStateKind::Dynamic => {
                        // Look up the handler's state field types from struct_defs
                        let field_count = self.struct_defs.get(handler_id)
                            .map(|fields| fields.len())
                            .unwrap_or(0);

                        if field_count == 0 {
                            // No state fields - just cast the typed pointer
                            self.builder.build_pointer_cast(
                                state_ptr,
                                i8_ptr_ty,
                                "state_void_ptr"
                            ).map_err(|e| vec![Diagnostic::error(
                                format!("LLVM error: {}", e), stmt.span
                            )])?
                        } else {
                            // Create shadow alloca with TYPED layout matching handler bodies.
                            // For concrete types, use their actual LLVM type (e.g. {i8*, i64, i64} for String).
                            // For unresolved generic type params, fall back to i64.
                            // This layout MUST match handler_state_field_types() used by
                            // compile_handler_op_body and compile_return_clause.
                            let handler_field_types = self.struct_defs.get(handler_id).cloned()
                                .unwrap_or_default();
                            let shadow_field_llvm_types: Vec<inkwell::types::BasicTypeEnum> = handler_field_types.iter()
                                .map(|t| if t.has_type_vars() { i64_ty.into() } else { self.lower_type(t) })
                                .collect();
                            let shadow_struct_type = self.context.struct_type(&shadow_field_llvm_types, false);
                            let shadow_alloca = self.builder
                                .build_alloca(shadow_struct_type, "state_shadow")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM error: {}", e), stmt.span
                                )])?;

                            // Build the typed struct type for GEP into the original state
                            let typed_field_llvm_types: Vec<inkwell::types::BasicTypeEnum> = handler_field_types.iter()
                                .map(|t| self.lower_type(t))
                                .collect();
                            let typed_struct_type = self.context.struct_type(&typed_field_llvm_types, false);
                            // With opaque pointers, all pointers are the same type.
                            // The pointer cast is an identity operation but kept for clarity.
                            let opaque_ptr_type = self.context.ptr_type(AddressSpace::default());
                            let typed_struct_ptr = self.builder
                                .build_pointer_cast(state_ptr, opaque_ptr_type, "typed_state_for_shadow")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM error: {}", e), stmt.span
                                )])?;

                            // Copy each field from typed struct to shadow struct
                            for idx in 0..field_count {
                                // GEP into typed struct to get field value
                                let field_ptr = self.builder
                                    .build_struct_gep(typed_struct_type, typed_struct_ptr, idx as u32, &format!("state_field_{}_ptr", idx))
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM error: {}", e), stmt.span
                                    )])?;
                                let field_val = self.builder
                                    .build_load(typed_field_llvm_types[idx], field_ptr, &format!("state_field_{}", idx))
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM error: {}", e), stmt.span
                                    )])?;

                                // GEP into shadow struct
                                let shadow_field_ptr = self.builder
                                    .build_struct_gep(shadow_struct_type, shadow_alloca, idx as u32, &format!("shadow_{}_ptr", idx))
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM error: {}", e), stmt.span
                                    )])?;

                                let shadow_field_ty = shadow_field_llvm_types[idx];
                                let source_field_ty = typed_field_llvm_types[idx];

                                // If source and shadow field types match, store directly.
                                // This handles aggregate types (String, Vec, etc.) correctly.
                                if source_field_ty == shadow_field_ty {
                                    self.builder.build_store(shadow_field_ptr, field_val)
                                        .map_err(|e| vec![Diagnostic::error(
                                            format!("LLVM error: {}", e), stmt.span
                                        )])?;
                                } else {
                                    // Shadow is i64 (generic fallback), convert source value to i64
                                    let field_as_i64 = match field_val {
                                        BasicValueEnum::IntValue(iv) => {
                                            let bw = iv.get_type().get_bit_width();
                                            if bw == 64 {
                                                iv
                                            } else if bw < 64 {
                                                self.builder.build_int_z_extend(iv, i64_ty, &format!("state_field_{}_zext", idx))
                                                    .map_err(|e| vec![Diagnostic::error(
                                                        format!("LLVM error: {}", e), stmt.span
                                                    )])?
                                            } else {
                                                self.builder.build_int_truncate(iv, i64_ty, &format!("state_field_{}_trunc", idx))
                                                    .map_err(|e| vec![Diagnostic::error(
                                                        format!("LLVM error: {}", e), stmt.span
                                                    )])?
                                            }
                                        }
                                        BasicValueEnum::PointerValue(pv) => {
                                            self.builder.build_ptr_to_int(pv, i64_ty, &format!("state_field_{}_ptoi", idx))
                                                .map_err(|e| vec![Diagnostic::error(
                                                    format!("LLVM error: {}", e), stmt.span
                                                )])?
                                        }
                                        BasicValueEnum::FloatValue(fv) => {
                                            self.builder.build_bit_cast(fv, i64_ty, &format!("state_field_{}_fcast", idx))
                                                .map_err(|e| vec![Diagnostic::error(
                                                    format!("LLVM error: {}", e), stmt.span
                                                )])?
                                                .into_int_value()
                                        }
                                        BasicValueEnum::StructValue(_) | BasicValueEnum::ArrayValue(_) | BasicValueEnum::VectorValue(_) | BasicValueEnum::ScalableVectorValue(_) => {
                                            return Err(vec![Diagnostic::error(
                                                format!(
                                                    "ICE: handler state field {} has i64 shadow layout but source value is aggregate type {:?}; \
                                                     cannot convert to i64. This indicates a type variable that should have been monomorphized.",
                                                    idx, field_val.get_type()
                                                ),
                                                stmt.span,
                                            )]);
                                        }
                                    };

                                    self.builder.build_store(shadow_field_ptr, field_as_i64)
                                        .map_err(|e| vec![Diagnostic::error(
                                            format!("LLVM error: {}", e), stmt.span
                                        )])?;
                                }
                            }

                            // Store shadow alloca for use by CallReturnClause
                            self.handler_state_shadows.insert(state_local, shadow_alloca);

                            // Cast shadow to i8* (void*) for evidence
                            self.builder.build_pointer_cast(
                                shadow_alloca,
                                i8_ptr_ty,
                                "state_void_ptr"
                            ).map_err(|e| vec![Diagnostic::error(
                                format!("LLVM error: {}", e), stmt.span
                            )])?
                        }
                    }
                };
                self.builder.build_call(
                    ev_push_with_state,
                    &[ev.into(), effect_id_val.into(), state_void_ptr.into()],
                    ""
                ).map_err(|e| vec![Diagnostic::error(
                    format!("LLVM call error: {}", e), stmt.span
                )])?;

                // EFF-OPT-003: For single-handler scopes (InlineEvidenceMode::Inline),
                // set the inline evidence hint for fast-path dispatch in blood_perform.
                // The handler is still pushed onto the evidence vector for correctness,
                // but blood_perform checks the inline hint first (O(1) vs O(n) search).
                if *inline_mode == InlineEvidenceMode::Inline {
                    let ev_set_inline = self.module.get_function("blood_evidence_set_inline")
                        .unwrap_or_else(|| {
                            let fn_type = self.context.void_type().fn_type(
                                &[i64_ty.into()],
                                false
                            );
                            self.module.add_function("blood_evidence_set_inline", fn_type, None)
                        });

                    self.builder.build_call(
                        ev_set_inline,
                        &[effect_id_val.into()],
                        ""
                    ).map_err(|e| vec![Diagnostic::error(
                        format!("LLVM call error: {}", e), stmt.span
                    )])?;
                }
            }

            StatementKind::PushInlineHandler { effect_id, operations, allocation_tier, inline_mode } => {
                // Push inline effect handler onto the evidence vector
                //
                // Inline handlers are defined directly in try/with expressions rather than
                // as named handler declarations. This implementation pushes the handler
                // similarly to PushHandler but with inline operation functions.
                //
                // INLINE EVIDENCE MODE (EFF-OPT-003/004):
                // The inline_mode parameter specifies the optimal evidence passing strategy:
                // - Inline: Single handler scope. The handler is pushed to the evidence vector
                //   AND a thread-local inline hint is set for O(1) dispatch in blood_perform.
                // - SpecializedPair: Two handlers, uses standard vector path (per spec).
                // - Vector: Three or more handlers, uses heap-allocated vector.
                //
                // All modes push to the evidence vector for correctness. The Inline mode
                // additionally calls blood_evidence_set_inline to enable fast-path dispatch.

                let i64_ty = self.context.i64_type();
                let i8_ptr_ty = self.context.ptr_type(AddressSpace::default());

                // Declare or get evidence functions
                let ev_current = self.module.get_function("blood_evidence_current")
                    .unwrap_or_else(|| {
                        let fn_type = i8_ptr_ty.fn_type(&[], false);
                        self.module.add_function("blood_evidence_current", fn_type, None)
                    });
                let ev_create = self.module.get_function("blood_evidence_create")
                    .unwrap_or_else(|| {
                        let fn_type = i8_ptr_ty.fn_type(&[], false);
                        self.module.add_function("blood_evidence_create", fn_type, None)
                    });
                let ev_set_current = self.module.get_function("blood_evidence_set_current")
                    .unwrap_or_else(|| {
                        let fn_type = self.context.void_type().fn_type(&[i8_ptr_ty.into()], false);
                        self.module.add_function("blood_evidence_set_current", fn_type, None)
                    });

                // Get current evidence vector
                let current_ev = self.builder.build_call(ev_current, &[], "current_ev")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM call error: {}", e), stmt.span
                    )])?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| vec![Diagnostic::error(
                        "blood_evidence_current returned void",
                        stmt.span
                    )])?;

                // Check if current evidence is null
                let is_null = self.builder.build_is_null(
                    current_ev.into_pointer_value(),
                    "ev_is_null"
                ).map_err(|e| vec![Diagnostic::error(
                    format!("LLVM error: {}", e), stmt.span
                )])?;

                // Get current function for creating blocks
                let current_fn = self.current_fn.ok_or_else(|| {
                    vec![Diagnostic::error("No current function", stmt.span)]
                })?;

                // Handle evidence creation based on allocation tier
                let ev = if *allocation_tier == MemoryTier::Stack {
                    // Stack-tier: Push directly onto existing evidence
                    let create_block = self.context.append_basic_block(current_fn, "inline_create_ev");
                    let use_block = self.context.append_basic_block(current_fn, "inline_use_ev");
                    let merge_block = self.context.append_basic_block(current_fn, "inline_merge_ev");

                    self.builder.build_conditional_branch(is_null, create_block, use_block)
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM error: {}", e), stmt.span
                        )])?;

                    // Create block
                    self.builder.position_at_end(create_block);
                    let new_ev = self.builder.build_call(ev_create, &[], "new_evidence")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM call error: {}", e), stmt.span
                        )])?
                        .try_as_basic_value()
                        .basic()
                        .ok_or_else(|| vec![Diagnostic::error(
                            "blood_evidence_create returned void",
                            stmt.span
                        )])?;
                    self.builder.build_call(ev_set_current, &[new_ev.into()], "")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM call error: {}", e), stmt.span
                        )])?;
                    self.builder.build_unconditional_branch(merge_block)
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM error: {}", e), stmt.span
                        )])?;
                    let create_block_end = self.builder.get_insert_block()
                        .ok_or_else(|| vec![Diagnostic::error(
                            "LLVM builder state invalid".to_string(), stmt.span
                        )])?;

                    // Use block
                    self.builder.position_at_end(use_block);
                    self.builder.build_unconditional_branch(merge_block)
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM error: {}", e), stmt.span
                        )])?;
                    let use_block_end = self.builder.get_insert_block()
                        .ok_or_else(|| vec![Diagnostic::error(
                            "LLVM builder state invalid".to_string(), stmt.span
                        )])?;

                    // Merge block
                    self.builder.position_at_end(merge_block);
                    let ev_phi = self.builder.build_phi(i8_ptr_ty, "evidence")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM error: {}", e), stmt.span
                        )])?;
                    ev_phi.add_incoming(&[
                        (&new_ev, create_block_end),
                        (&current_ev, use_block_end),
                    ]);
                    ev_phi.as_basic_value()
                } else {
                    // Region-tier: Clone evidence vector
                    let create_block = self.context.append_basic_block(current_fn, "inline_region_create");
                    let clone_block = self.context.append_basic_block(current_fn, "inline_region_clone");
                    let merge_block = self.context.append_basic_block(current_fn, "inline_region_merge");

                    self.builder.build_conditional_branch(is_null, create_block, clone_block)
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM error: {}", e), stmt.span
                        )])?;

                    // Create block
                    self.builder.position_at_end(create_block);
                    let new_ev = self.builder.build_call(ev_create, &[], "new_evidence")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM call error: {}", e), stmt.span
                        )])?
                        .try_as_basic_value()
                        .basic()
                        .ok_or_else(|| vec![Diagnostic::error(
                            "blood_evidence_create returned void",
                            stmt.span
                        )])?;
                    self.builder.build_unconditional_branch(merge_block)
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM error: {}", e), stmt.span
                        )])?;
                    let create_block_end = self.builder.get_insert_block()
                        .ok_or_else(|| vec![Diagnostic::error(
                            "LLVM builder state invalid".to_string(), stmt.span
                        )])?;

                    // Clone block
                    self.builder.position_at_end(clone_block);
                    let cloned_ev = self.builder.build_call(ev_create, &[], "cloned_evidence")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM call error: {}", e), stmt.span
                        )])?
                        .try_as_basic_value()
                        .basic()
                        .ok_or_else(|| vec![Diagnostic::error(
                            "blood_evidence_create returned void",
                            stmt.span
                        )])?;
                    self.builder.build_unconditional_branch(merge_block)
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM error: {}", e), stmt.span
                        )])?;
                    let clone_block_end = self.builder.get_insert_block()
                        .ok_or_else(|| vec![Diagnostic::error(
                            "LLVM builder state invalid".to_string(), stmt.span
                        )])?;

                    // Merge block
                    self.builder.position_at_end(merge_block);
                    let ev_phi = self.builder.build_phi(i8_ptr_ty, "evidence")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM error: {}", e), stmt.span
                        )])?;
                    ev_phi.add_incoming(&[
                        (&new_ev, create_block_end),
                        (&cloned_ev, clone_block_end),
                    ]);
                    let ev = ev_phi.as_basic_value();

                    self.builder.build_call(ev_set_current, &[ev.into()], "")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM call error: {}", e), stmt.span
                        )])?;

                    ev
                };

                // Compile inline handler operation bodies and register with runtime
                // All operations are compiled first, then registered together via blood_evidence_register

                // Declare or get the evidence registration function
                // Signature: void blood_evidence_register(
                //   EvidenceHandle ev, i64 effect_id, *const *const c_void ops, i64 op_count
                // )
                let ev_register = self.module.get_function("blood_evidence_register")
                    .unwrap_or_else(|| {
                        // ops is **void (pointer to array of function pointers)
                        let ptr_ptr_ty = self.context.ptr_type(AddressSpace::default());
                        let fn_type = self.context.void_type().fn_type(
                            &[i8_ptr_ty.into(), i64_ty.into(), ptr_ptr_ty.into(), i64_ty.into()],
                            false,
                        );
                        self.module.add_function("blood_evidence_register", fn_type, None)
                    });

                // Compile all handler operations and collect function pointers
                let mut compiled_ops: Vec<inkwell::values::PointerValue<'ctx>> = Vec::new();

                // Save the current insertion point
                let saved_block = self.builder.get_insert_block();

                for op in operations {
                    // Look up the handler body from inline_handler_bodies
                    if let Some(handler_body) = self.inline_handler_bodies.get(&op.synthetic_fn_def_id).cloned() {
                        // Compile the inline handler body to an LLVM function
                        let handler_fn = self.compile_inline_handler_op_body(
                            op.synthetic_fn_def_id,
                            &handler_body,
                        )?;

                        // Get the function pointer
                        let fn_ptr = handler_fn.as_global_value().as_pointer_value();
                        compiled_ops.push(fn_ptr);
                    } else {
                        // Handler body not found - use null pointer (will cause runtime error if called)
                        compiled_ops.push(i8_ptr_ty.const_null());
                    }
                }

                // Restore the insertion point
                if let Some(block) = saved_block {
                    self.builder.position_at_end(block);
                }

                // Build array of operation function pointers on the stack
                let op_count = operations.len();
                let ptr_ptr_ty = self.context.ptr_type(AddressSpace::default());

                if op_count > 0 {
                    // Allocate array for operation function pointers
                    let array_ty = i8_ptr_ty.array_type(op_count as u32);
                    let ops_array = self.builder.build_alloca(array_ty, "ops_array")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM alloca error: {}", e), stmt.span
                        )])?;

                    // Store each function pointer in the array
                    for (i, fn_ptr) in compiled_ops.iter().enumerate() {
                        // Get pointer to array element
                        let elem_ptr = unsafe {
                            self.builder.build_gep(
                                array_ty,
                                ops_array,
                                &[
                                    i64_ty.const_zero(),
                                    i64_ty.const_int(i as u64, false),
                                ],
                                &format!("op_ptr_{}", i)
                            ).map_err(|e| vec![Diagnostic::error(
                                format!("LLVM GEP error: {}", e), stmt.span
                            )])?
                        };

                        // Cast function pointer to i8*
                        let fn_as_i8ptr = self.builder.build_pointer_cast(
                            *fn_ptr,
                            i8_ptr_ty,
                            &format!("fn_ptr_{}", i)
                        ).map_err(|e| vec![Diagnostic::error(
                            format!("LLVM cast error: {}", e), stmt.span
                        )])?;

                        self.builder.build_store(elem_ptr, fn_as_i8ptr)
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM store error: {}", e), stmt.span
                            )])?;
                    }

                    // Cast array pointer to **void for blood_evidence_register
                    let ops_ptr = self.builder.build_pointer_cast(
                        ops_array,
                        ptr_ptr_ty,
                        "ops_ptr"
                    ).map_err(|e| vec![Diagnostic::error(
                        format!("LLVM cast error: {}", e), stmt.span
                    )])?;

                    // Register all operations with the evidence system
                    let effect_id_val = i64_ty.const_int(effect_id.index as u64, false);
                    let op_count_val = i64_ty.const_int(op_count as u64, false);

                    self.builder.build_call(
                        ev_register,
                        &[ev.into(), effect_id_val.into(), ops_ptr.into(), op_count_val.into()],
                        ""
                    ).map_err(|e| vec![Diagnostic::error(
                        format!("LLVM call error: {}", e), stmt.span
                    )])?;
                }

                // Build captures struct for handler state
                // Collect all unique captures across all operations
                let mut all_captures: Vec<&crate::mir::InlineHandlerCapture> = Vec::new();
                let mut seen_locals = std::collections::HashSet::new();
                for op in operations {
                    for capture in &op.captures {
                        if seen_locals.insert(capture.local_id) {
                            all_captures.push(capture);
                        }
                    }
                }

                // Create state pointer - either captures struct or NULL
                let state_ptr = if all_captures.is_empty() {
                    // No captures - use NULL state
                    i8_ptr_ty.const_null()
                } else {
                    // Allocate captures struct on the stack (array of pointers)
                    // Each element is a pointer to a captured variable
                    let captures_count = all_captures.len();
                    let captures_array_ty = i8_ptr_ty.array_type(captures_count as u32);
                    let captures_alloca = self.builder.build_alloca(captures_array_ty, "captures")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM alloca error: {}", e), stmt.span
                        )])?;

                    // Store pointer to each captured variable
                    for (idx, capture) in all_captures.iter().enumerate() {
                        // Look up the local variable's alloca
                        let local_ptr = self.locals.get(&capture.local_id)
                            .ok_or_else(|| vec![Diagnostic::error(
                                format!("Captured local {:?} not found in current scope", capture.local_id),
                                stmt.span
                            )])?;

                        // Get pointer to array element
                        let elem_ptr = unsafe {
                            self.builder.build_gep(
                                captures_array_ty,
                                captures_alloca,
                                &[i64_ty.const_zero(), i64_ty.const_int(idx as u64, false)],
                                &format!("capture_{}_slot", idx)
                            ).map_err(|e| vec![Diagnostic::error(
                                format!("LLVM GEP error: {}", e), stmt.span
                            )])?
                        };

                        // Cast the local pointer to i8* and store it
                        let local_as_ptr = self.builder.build_pointer_cast(
                            *local_ptr,
                            i8_ptr_ty,
                            &format!("capture_{}_ptr", idx)
                        ).map_err(|e| vec![Diagnostic::error(
                            format!("LLVM cast error: {}", e), stmt.span
                        )])?;

                        self.builder.build_store(elem_ptr, local_as_ptr)
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM store error: {}", e), stmt.span
                            )])?;
                    }

                    // Cast captures array to i8*
                    self.builder.build_pointer_cast(
                        captures_alloca,
                        i8_ptr_ty,
                        "captures_ptr"
                    ).map_err(|e| vec![Diagnostic::error(
                        format!("LLVM cast error: {}", e), stmt.span
                    )])?
                };

                // Set state for the handler entry that was pushed by blood_evidence_register
                // blood_evidence_register already pushed an entry with null state,
                // so we just need to set the captures state on it
                if !all_captures.is_empty() {
                    let ev_set_state = self.module.get_function("blood_evidence_set_state")
                        .unwrap_or_else(|| {
                            let fn_type = self.context.void_type().fn_type(
                                &[i8_ptr_ty.into(), i8_ptr_ty.into()],
                                false,
                            );
                            self.module.add_function("blood_evidence_set_state", fn_type, None)
                        });

                    self.builder.build_call(
                        ev_set_state,
                        &[ev.into(), state_ptr.into()],
                        ""
                    ).map_err(|e| vec![Diagnostic::error(
                        format!("LLVM call error: {}", e), stmt.span
                    )])?;
                }

                // EFF-OPT-003: For single-handler scopes (InlineEvidenceMode::Inline),
                // set the inline evidence hint for fast-path dispatch in blood_perform.
                // The handler is still pushed onto the evidence vector for correctness,
                // but blood_perform checks the inline hint first (O(1) vs O(n) search).
                if *inline_mode == InlineEvidenceMode::Inline {
                    let ev_set_inline = self.module.get_function("blood_evidence_set_inline")
                        .unwrap_or_else(|| {
                            let fn_type = self.context.void_type().fn_type(
                                &[i64_ty.into()],
                                false
                            );
                            self.module.add_function("blood_evidence_set_inline", fn_type, None)
                        });

                    let inline_effect_id_val = i64_ty.const_int(effect_id.index as u64, false);
                    self.builder.build_call(
                        ev_set_inline,
                        &[inline_effect_id_val.into()],
                        ""
                    ).map_err(|e| vec![Diagnostic::error(
                        format!("LLVM call error: {}", e), stmt.span
                    )])?;
                }
            }

            StatementKind::PopHandler => {
                // Pop effect handler from the evidence vector
                let i8_ptr_ty = self.context.ptr_type(AddressSpace::default());

                // Declare or get evidence functions
                let ev_pop = self.module.get_function("blood_evidence_pop")
                    .unwrap_or_else(|| {
                        let fn_type = self.context.void_type().fn_type(&[i8_ptr_ty.into()], false);
                        self.module.add_function("blood_evidence_pop", fn_type, None)
                    });
                let ev_current = self.module.get_function("blood_evidence_current")
                    .unwrap_or_else(|| {
                        let fn_type = i8_ptr_ty.fn_type(&[], false);
                        self.module.add_function("blood_evidence_current", fn_type, None)
                    });

                // Get current evidence vector
                let ev = self.builder.build_call(ev_current, &[], "current_ev")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM call error: {}", e), stmt.span
                    )])?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| vec![Diagnostic::error(
                        "blood_evidence_current returned void",
                        stmt.span
                    )])?;

                // Pop the handler
                self.builder.build_call(ev_pop, &[ev.into()], "")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM call error: {}", e), stmt.span
                    )])?;

                // EFF-OPT-003: Clear inline evidence hint on handler pop.
                // The hint was set by PushHandler/PushInlineHandler for Inline mode;
                // clearing on pop ensures stale hints don't persist after scope exit.
                let ev_clear_inline = self.module.get_function("blood_evidence_clear_inline")
                    .unwrap_or_else(|| {
                        let fn_type = self.context.void_type().fn_type(&[], false);
                        self.module.add_function("blood_evidence_clear_inline", fn_type, None)
                    });
                self.builder.build_call(ev_clear_inline, &[], "")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM call error: {}", e), stmt.span
                    )])?;
            }

            StatementKind::CallReturnClause { handler_id: _, handler_name, body_result, state_place, destination } => {
                // Call the handler's return clause to transform the body result
                let i64_ty = self.context.i64_type();
                let i8_ptr_ty = self.context.ptr_type(AddressSpace::default());

                // Generate return clause function name (content-based naming for cache compatibility)
                // The handler_name is embedded in MIR for proper content hashing
                let return_fn_name = format!("{}_return", handler_name);

                // Declare or get the return clause function
                let return_fn = self.module.get_function(&return_fn_name)
                    .unwrap_or_else(|| {
                        // Signature: fn(result: i64, state_ptr: *void) -> i64
                        let fn_type = i64_ty.fn_type(&[i64_ty.into(), i8_ptr_ty.into()], false);
                        self.module.add_function(&return_fn_name, fn_type, None)
                    });

                // Compile the body result operand
                let body_result_val = self.compile_mir_operand(body_result, body, escape_results)?;

                // Convert body result to i64
                let result_i64 = match body_result_val {
                    BasicValueEnum::IntValue(iv) => {
                        if iv.get_type().get_bit_width() == 64 {
                            iv
                        } else {
                            self.builder.build_int_s_extend(iv, i64_ty, "result_ext")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM extend error: {}", e), stmt.span
                                )])?
                        }
                    }
                    BasicValueEnum::PointerValue(pv) => {
                        self.builder.build_ptr_to_int(pv, i64_ty, "result_ptr_int")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM ptr_to_int error: {}", e), stmt.span
                            )])?
                    }
                    BasicValueEnum::FloatValue(fv) => {
                        self.builder.build_bit_cast(fv, i64_ty, "result_float_bits")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM bitcast error: {}", e), stmt.span
                            )])?
                            .into_int_value()
                    }
                    BasicValueEnum::ArrayValue(_) | BasicValueEnum::StructValue(_) | BasicValueEnum::VectorValue(_) | BasicValueEnum::ScalableVectorValue(_) => {
                        return Err(vec![Diagnostic::error(
                            "ICE: handler return clause received aggregate value that cannot be converted to i64".to_string(),
                            stmt.span
                        )]);
                    }
                };

                // Get state pointer: prefer shadow i64 alloca if one was created
                // by PushHandler (for uniform i64 state layout), otherwise fall back
                // to the typed state_place.
                let state_void_ptr = if let Some(state_local) = state_place.as_local() {
                    if let Some(&shadow_ptr) = self.handler_state_shadows.get(&state_local) {
                        // Use shadow i64 alloca (matches handler body's i64 layout)
                        self.builder.build_pointer_cast(shadow_ptr, i8_ptr_ty, "state_void_ptr")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM cast error: {}", e), stmt.span
                            )])?
                    } else {
                        let state_ptr = self.compile_mir_place(state_place, body, escape_results)?;
                        self.builder.build_pointer_cast(state_ptr, i8_ptr_ty, "state_void_ptr")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM cast error: {}", e), stmt.span
                            )])?
                    }
                } else {
                    let state_ptr = self.compile_mir_place(state_place, body, escape_results)?;
                    self.builder.build_pointer_cast(state_ptr, i8_ptr_ty, "state_void_ptr")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM cast error: {}", e), stmt.span
                        )])?
                };

                // Call return clause: fn(result: i64, state_ptr: *void) -> i64
                let call_result = self.builder.build_call(
                    return_fn,
                    &[result_i64.into(), state_void_ptr.into()],
                    "return_clause_result"
                ).map_err(|e| vec![Diagnostic::error(
                    format!("LLVM call error: {}", e), stmt.span
                )])?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| vec![Diagnostic::error(
                        "Return clause returned void", stmt.span
                    )])?;

                // Store result in destination
                let dest_ptr = self.compile_mir_place(destination, body, escape_results)?;

                // Get destination type and convert i64 result if needed
                let dest_ty = self.get_place_base_type(destination, body)?;
                let dest_ty = &dest_ty;
                let dest_llvm_ty = self.lower_type(dest_ty);

                let converted_result = if dest_llvm_ty.is_int_type() {
                    let dest_int_ty = dest_llvm_ty.into_int_type();
                    let result_i64 = call_result.into_int_value();

                    if dest_int_ty.get_bit_width() == 64 {
                        call_result
                    } else if dest_int_ty.get_bit_width() < 64 {
                        // Truncate i64 to destination type
                        self.builder.build_int_truncate(result_i64, dest_int_ty, "ret_trunc")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM truncate error: {}", e), stmt.span
                            )])?
                            .into()
                    } else {
                        // Extend to larger type (rare)
                        self.builder.build_int_s_extend(result_i64, dest_int_ty, "ret_ext")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM extend error: {}", e), stmt.span
                            )])?
                            .into()
                    }
                } else if dest_llvm_ty.is_pointer_type() {
                    // Convert i64 to pointer
                    self.builder.build_int_to_ptr(
                        call_result.into_int_value(),
                        dest_llvm_ty.into_pointer_type(),
                        "ret_ptr"
                    ).map_err(|e| vec![Diagnostic::error(
                        format!("LLVM int_to_ptr error: {}", e), stmt.span
                    )])?
                    .into()
                } else {
                    // For other types, use as-is
                    call_result
                };

                self.builder.build_store(dest_ptr, converted_result)
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM store error: {}", e), stmt.span
                    )])?;
            }

            StatementKind::Nop => {
                // No operation
            }
        }

        Ok(())
    }
}
