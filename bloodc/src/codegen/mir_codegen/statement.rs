//! MIR statement code generation.
//!
//! This module handles compilation of MIR statements to LLVM IR.

use inkwell::types::BasicType;
use inkwell::values::BasicValueEnum;
use inkwell::AddressSpace;

use crate::diagnostics::Diagnostic;
use crate::mir::body::MirBody;
use crate::mir::ptr::MemoryTier;
use crate::mir::types::{Statement, StatementKind};
use crate::mir::EscapeResults;

use super::rvalue::MirRvalueCodegen;
use super::place::MirPlaceCodegen;
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
                let value = self.compile_mir_rvalue(rvalue, body, escape_results)?;
                let ptr = self.compile_mir_place(place, body, escape_results)?;
                self.builder.build_store(ptr, value)
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM store error: {}", e), stmt.span
                    )])?;
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
                        let loaded = self.builder.build_load(local_ptr, "local_addr")
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
            }

            StatementKind::Drop(place) => {
                // Drop value - free memory if heap allocated
                // Get the address of the place
                let ptr = self.compile_mir_place(place, body, escape_results)?;

                // Get the type to determine size
                let place_ty = &body.locals[place.local.index as usize].ty;
                let llvm_ty = self.lower_type(place_ty);
                let size = llvm_ty.size_of()
                    .map(|s| s.const_cast(self.context.i64_type(), false))
                    .unwrap_or_else(|| self.context.i64_type().const_int(0, false));

                // For reference types, call blood_free to deallocate
                if place_ty.is_ref() {
                    if let Some(free_fn) = self.module.get_function("blood_free") {
                        // Load the pointer value
                        let ptr_val = self.builder.build_load(ptr, "drop_val")
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
                let should_skip = if let Some(results) = escape_results {
                    // Get the base local from the place
                    let local = ptr.local;
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

            StatementKind::PushHandler { handler_id, state_place, state_kind, allocation_tier } => {
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
                // STACK ALLOCATION OPTIMIZATION (EFF-OPT-005/006):
                // When allocation_tier is Stack, the handler is lexically scoped
                // (no Perform/Resume that could capture the continuation).
                // We can skip cloning the evidence vector (blood_evidence_create)
                // and push directly onto the existing one, avoiding heap allocation.
                //
                // TODO: Implement optimized paths for static handlers.
                let _ = state_kind; // Suppress unused warning until static optimization is implemented

                let i64_ty = self.context.i64_type();
                let i8_ptr_ty = self.context.i8_type().ptr_type(AddressSpace::default());

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
                let state_ptr = *self.locals.get(&state_place.local).ok_or_else(|| {
                    vec![Diagnostic::error(
                        format!("Local _{} not found for handler state", state_place.local.index),
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
                    .left()
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
                        .left()
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
                        .left()
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
                        .left()
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
                // Cast state_ptr to i8* (void*)
                let state_void_ptr = self.builder.build_pointer_cast(
                    state_ptr,
                    i8_ptr_ty,
                    "state_void_ptr"
                ).map_err(|e| vec![Diagnostic::error(
                    format!("LLVM error: {}", e), stmt.span
                )])?;
                self.builder.build_call(
                    ev_push_with_state,
                    &[ev.into(), effect_id_val.into(), state_void_ptr.into()],
                    ""
                ).map_err(|e| vec![Diagnostic::error(
                    format!("LLVM call error: {}", e), stmt.span
                )])?;
            }

            StatementKind::PopHandler => {
                // Pop effect handler from the evidence vector
                let i8_ptr_ty = self.context.i8_type().ptr_type(AddressSpace::default());

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
                    .left()
                    .ok_or_else(|| vec![Diagnostic::error(
                        "blood_evidence_current returned void",
                        stmt.span
                    )])?;

                // Pop the handler
                self.builder.build_call(ev_pop, &[ev.into()], "")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM call error: {}", e), stmt.span
                    )])?;
            }

            StatementKind::CallReturnClause { handler_id, body_result, state_place, destination } => {
                // Call the handler's return clause to transform the body result
                let i64_ty = self.context.i64_type();
                let i8_ptr_ty = self.context.i8_type().ptr_type(AddressSpace::default());

                // Generate return clause function name
                let return_fn_name = format!("handler_{}_return", handler_id.index);

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
                    _ => i64_ty.const_zero(),
                };

                // Get state pointer from state_place
                let state_ptr = self.compile_mir_place(state_place, body, escape_results)?;
                let state_void_ptr = self.builder.build_pointer_cast(state_ptr, i8_ptr_ty, "state_void_ptr")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM cast error: {}", e), stmt.span
                    )])?;

                // Call return clause: fn(result: i64, state_ptr: *void) -> i64
                let call_result = self.builder.build_call(
                    return_fn,
                    &[result_i64.into(), state_void_ptr.into()],
                    "return_clause_result"
                ).map_err(|e| vec![Diagnostic::error(
                    format!("LLVM call error: {}", e), stmt.span
                )])?
                    .try_as_basic_value()
                    .left()
                    .ok_or_else(|| vec![Diagnostic::error(
                        "Return clause returned void", stmt.span
                    )])?;

                // Store result in destination
                let dest_ptr = self.compile_mir_place(destination, body, escape_results)?;

                // Get destination type and convert i64 result if needed
                let dest_ty = &body.locals[destination.local.index as usize].ty;
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
