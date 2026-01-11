//! MIR-based code generation.
//!
//! This module compiles MIR (Mid-level Intermediate Representation) to LLVM IR.
//! MIR provides explicit control flow graphs and flattened expressions, making
//! it easier to implement optimizations like:
//!
//! - **Escape analysis**: Skip generation checks for non-escaping values
//! - **Tier-based allocation**: Use stack vs region based on escape state
//! - **Generation validation**: Insert checks for region-allocated values
//!
//! # Architecture
//!
//! ```text
//! MIR Body -> Basic Blocks -> LLVM Basic Blocks
//!          -> Statements   -> LLVM Instructions
//!          -> Terminators  -> LLVM Terminators
//! ```
//!
//! # Memory Tier Allocation Strategy
//!
//! | Memory Tier | Escape State | Allocation Method | Generation Checks |
//! |-------------|--------------|-------------------|-------------------|
//! | Stack (0)   | NoEscape     | LLVM `alloca`     | NO - safe by construction |
//! | Region (1)  | ArgEscape    | `blood_alloc_or_abort` | YES - on every dereference |
//! | Region (1)  | GlobalEscape | `blood_alloc_or_abort` | YES - on every dereference |
//! | Region (1)  | Effect-captured | `blood_alloc_or_abort` | YES - on every dereference |
//!
//! # Generation Check Flow (for Region-allocated values)
//!
//! ```text
//! 1. Allocation:
//!    address = blood_alloc_or_abort(size, &generation)
//!    // Registers in slot registry, returns address and generation
//!
//! 2. On Dereference:
//!    result = blood_validate_generation(address, expected_generation)
//!    if result != 0:
//!        blood_stale_reference_panic(expected_gen, actual_gen)  // aborts
//!    // Continue with dereference
//!
//! 3. On Free (automatic at scope exit):
//!    blood_unregister_allocation(address)
//!    // Invalidates the address in slot registry
//!    // Future validation with old generation returns 1 (stale)
//! ```
//!
//! # Integration with Escape Analysis
//!
//! When escape analysis results are available, the MIR codegen:
//! 1. Queries `recommended_tier(local)` for each local
//! 2. Allocates stack storage for NoEscape locals (thin pointers)
//! 3. Allocates region storage for ArgEscape/GlobalEscape via `blood_alloc_or_abort`
//! 4. Stores generation in `local_generations` map for later validation
//! 5. On dereference, emits `blood_validate_generation` call for region-allocated values
//! 6. Branches to panic path on stale reference detection

use std::collections::HashMap;

use inkwell::basic_block::BasicBlock;
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, IntValue, PointerValue};
use inkwell::types::{BasicType, BasicTypeEnum};
use inkwell::{AddressSpace, IntPredicate};

use crate::diagnostics::Diagnostic;
use crate::hir::{DefId, LocalId, Type, TypeKind};
use crate::mir::body::MirBody;
use crate::mir::types::{
    BasicBlockId, StatementKind, Statement, Terminator, TerminatorKind,
    Place, PlaceElem, Operand, Rvalue, BinOp, UnOp, Constant, ConstantKind,
    AggregateKind,
};
use crate::mir::{EscapeResults, EscapeState, MemoryTier};
use crate::span::Span;
use crate::{ice, ice_err};

use super::CodegenContext;

/// Extension trait for MIR compilation on CodegenContext.
pub trait MirCodegen<'ctx, 'a> {
    /// Compile a MIR function body.
    fn compile_mir_body(
        &mut self,
        def_id: DefId,
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
    ) -> Result<(), Vec<Diagnostic>>;

    /// Compile a single MIR basic block.
    fn compile_mir_block(
        &mut self,
        body: &MirBody,
        block_id: BasicBlockId,
        llvm_blocks: &HashMap<BasicBlockId, BasicBlock<'ctx>>,
        escape_results: Option<&EscapeResults>,
    ) -> Result<(), Vec<Diagnostic>>;

    /// Compile a MIR statement.
    fn compile_mir_statement(
        &mut self,
        stmt: &Statement,
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
    ) -> Result<(), Vec<Diagnostic>>;

    /// Compile a MIR terminator.
    fn compile_mir_terminator(
        &mut self,
        term: &Terminator,
        body: &MirBody,
        llvm_blocks: &HashMap<BasicBlockId, BasicBlock<'ctx>>,
        escape_results: Option<&EscapeResults>,
    ) -> Result<(), Vec<Diagnostic>>;

    /// Compile a MIR rvalue to produce a value.
    fn compile_mir_rvalue(
        &mut self,
        rvalue: &Rvalue,
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>>;

    /// Compile a MIR operand.
    fn compile_mir_operand(
        &mut self,
        operand: &Operand,
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>>;

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

    /// Determine the memory tier for a local based on escape analysis.
    fn get_local_tier(
        &self,
        local: LocalId,
        escape_results: Option<&EscapeResults>,
    ) -> MemoryTier;

    /// Check if generation checks can be skipped for a local.
    fn should_skip_gen_check(
        &self,
        local: LocalId,
        escape_results: Option<&EscapeResults>,
    ) -> bool;

    /// Emit a generation check for a pointer dereference.
    fn emit_generation_check(
        &mut self,
        ptr: PointerValue<'ctx>,
        expected_gen: IntValue<'ctx>,
        span: Span,
    ) -> Result<(), Vec<Diagnostic>>;

    /// Check if a type may contain generational references.
    ///
    /// Used to determine which locals need snapshot capture during
    /// effect operations.
    fn type_may_contain_genref(&self, ty: &Type) -> bool;

    /// Allocate memory using blood_alloc for Region/Persistent tier.
    ///
    /// This calls the runtime's blood_alloc function which:
    /// 1. Allocates memory on the heap
    /// 2. Registers it in the slot registry
    /// 3. Returns the address and generation metadata
    ///
    /// Returns a pointer to the allocated memory.
    fn allocate_with_blood_alloc(
        &mut self,
        llvm_ty: BasicTypeEnum<'ctx>,
        local_id: LocalId,
        span: Span,
    ) -> Result<PointerValue<'ctx>, Vec<Diagnostic>>;
}

impl<'ctx, 'a> MirCodegen<'ctx, 'a> for CodegenContext<'ctx, 'a> {
    fn compile_mir_body(
        &mut self,
        def_id: DefId,
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
    ) -> Result<(), Vec<Diagnostic>> {
        let fn_value = *self.functions.get(&def_id).ok_or_else(|| {
            vec![Diagnostic::error(
                "Internal error: function not declared for MIR compilation",
                body.span,
            )]
        })?;

        self.current_fn = Some(fn_value);
        self.locals.clear();
        self.local_generations.clear();

        // Create LLVM basic blocks for all MIR blocks
        let mut llvm_blocks: HashMap<BasicBlockId, BasicBlock<'ctx>> = HashMap::new();
        for (bb_id, _) in body.blocks() {
            let name = format!("bb{}", bb_id.0);
            let llvm_bb = self.context.append_basic_block(fn_value, &name);
            llvm_blocks.insert(bb_id, llvm_bb);
        }

        // Position at entry block
        let entry_bb = llvm_blocks.get(&BasicBlockId::ENTRY).ok_or_else(|| {
            vec![Diagnostic::error("MIR body has no entry block", body.span)]
        })?;
        self.builder.position_at_end(*entry_bb);

        // Allocate locals based on escape analysis
        for local in &body.locals {
            let tier = self.get_local_tier(local.id, escape_results);
            let llvm_ty = self.lower_type(&local.ty);

            let alloca = match tier {
                MemoryTier::Stack => {
                    // Stack allocation - thin pointer, no generation check needed
                    // This is the fast path for non-escaping values
                    self.builder.build_alloca(
                        llvm_ty,
                        &format!("_{}_{}", local.id.index, tier_name(tier))
                    ).map_err(|e| vec![Diagnostic::error(
                        format!("LLVM alloca error: {}", e), body.span
                    )])?
                }
                MemoryTier::Region | MemoryTier::Persistent => {
                    // Region allocation - use blood_alloc for generational tracking
                    // This is the safe path for escaping values that need generation checks
                    self.allocate_with_blood_alloc(llvm_ty, local.id, body.span)?
                }
                MemoryTier::Reserved => {
                    // Reserved tier should never be returned by escape analysis.
                    // If we reach here, something is wrong with the escape analysis or
                    // a future feature is using Reserved without implementing it.
                    return Err(vec![ice_err!(
                        body.span,
                        "MemoryTier::Reserved reached during allocation";
                        "local" => format!("_{}", local.id.index),
                        "note" => "escape analysis should never return Reserved tier"
                    )]);
                }
            };

            self.locals.insert(local.id, alloca);
        }

        // Store function parameters
        for (i, param_id) in body.param_ids().enumerate() {
            if let Some(&alloca) = self.locals.get(&param_id) {
                let param_value = fn_value.get_nth_param(i as u32)
                    .ok_or_else(|| vec![Diagnostic::error(
                        format!("Parameter {} not found", i), body.span
                    )])?;
                self.builder.build_store(alloca, param_value)
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM store error: {}", e), body.span
                    )])?;
            }
        }

        // Compile each basic block
        for (bb_id, _) in body.blocks() {
            self.compile_mir_block(body, bb_id, &llvm_blocks, escape_results)?;
        }

        self.current_fn = None;
        Ok(())
    }

    fn compile_mir_block(
        &mut self,
        body: &MirBody,
        block_id: BasicBlockId,
        llvm_blocks: &HashMap<BasicBlockId, BasicBlock<'ctx>>,
        escape_results: Option<&EscapeResults>,
    ) -> Result<(), Vec<Diagnostic>> {
        let llvm_bb = *llvm_blocks.get(&block_id).ok_or_else(|| {
            vec![Diagnostic::error(
                format!("No LLVM block for {}", block_id),
                body.span,
            )]
        })?;

        self.builder.position_at_end(llvm_bb);

        let block_data = body.get_block(block_id).ok_or_else(|| {
            vec![Diagnostic::error(
                format!("MIR block {} not found", block_id),
                body.span,
            )]
        })?;

        // Compile statements
        for stmt in &block_data.statements {
            self.compile_mir_statement(stmt, body, escape_results)?;
        }

        // Compile terminator
        if let Some(term) = &block_data.terminator {
            self.compile_mir_terminator(term, body, llvm_blocks, escape_results)?;
        } else {
            // Unterminated block - add unreachable
            self.builder.build_unreachable()
                .map_err(|e| vec![Diagnostic::error(
                    format!("LLVM unreachable error: {}", e), body.span
                )])?;
        }

        Ok(())
    }

    fn compile_mir_statement(
        &mut self,
        stmt: &Statement,
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
    ) -> Result<(), Vec<Diagnostic>> {
        match &stmt.kind {
            StatementKind::Assign(place, rvalue) => {
                let value = self.compile_mir_rvalue(rvalue, body, escape_results)?;
                let ptr = self.compile_mir_place(place, body)?;
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
                if let Some(&gen_alloca) = self.local_generations.get(&local) {
                    // Get the local's pointer storage
                    if let Some(&local_ptr) = self.locals.get(&local) {
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
                // Drop value - run destructor if applicable
                // For now, just load the value to ensure it's "used"
                let _value = self.compile_mir_place_load(place, body, escape_results)?;
                // In the future: emit destructor call based on type
            }

            StatementKind::IncrementGeneration(place) => {
                // Increment generation counter for a slot
                // This is used when freeing/reallocating
                // Requires: blood_increment_generation(address: *void) runtime call
                let ptr = self.compile_mir_place(place, body)?;

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
                // Capture generation snapshot for effect handler
                // This emits code to add the local's address and generation to the
                // current effect's snapshot. The snapshot is stored in a thread-local
                // context during effect operations.
                //
                // For now, this is a placeholder that logs the capture intent.
                // Full implementation requires:
                // 1. Access to the current effect context's snapshot handle
                // 2. Extraction of address/generation from BloodPtr
                // 3. Calling blood_snapshot_add_entry
                //
                // The Perform terminator handles the full snapshot lifecycle,
                // so individual CaptureSnapshot statements are less critical.
                let _ = local;
            }

            StatementKind::ValidateGeneration { ptr, expected_gen } => {
                // Validate generation check
                let ptr_val = self.compile_mir_place(ptr, body)?;
                let expected = self.compile_mir_operand(expected_gen, body, escape_results)?;
                if let BasicValueEnum::IntValue(gen_val) = expected {
                    self.emit_generation_check(ptr_val, gen_val, stmt.span)?;
                }
            }

            StatementKind::PushHandler { handler_id, state_place } => {
                // Push effect handler onto the evidence vector with instance state
                // This calls blood_evidence_push_with_state with effect_id and state pointer
                let i64_ty = self.context.i64_type();
                let i8_ptr_ty = self.context.i8_type().ptr_type(AddressSpace::default());

                // Look up the handler's effect_id
                let handler_info = self.handler_defs.get(&handler_id).ok_or_else(|| {
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

                // Create evidence vector if not already set
                let ev = self.builder.build_call(ev_create, &[], "evidence")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM call error: {}", e), stmt.span
                    )])?
                    .try_as_basic_value()
                    .left()
                    .ok_or_else(|| vec![Diagnostic::error(
                        "blood_evidence_create returned void",
                        stmt.span
                    )])?;

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

                // Set as current evidence vector
                self.builder.build_call(ev_set_current, &[ev.into()], "")
                    .map_err(|e| vec![Diagnostic::error(
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

            StatementKind::Nop => {
                // No operation
            }
        }

        Ok(())
    }

    fn compile_mir_terminator(
        &mut self,
        term: &Terminator,
        body: &MirBody,
        llvm_blocks: &HashMap<BasicBlockId, BasicBlock<'ctx>>,
        escape_results: Option<&EscapeResults>,
    ) -> Result<(), Vec<Diagnostic>> {
        match &term.kind {
            TerminatorKind::Goto { target } => {
                let target_bb = llvm_blocks.get(target).ok_or_else(|| {
                    vec![Diagnostic::error(format!("Target block {} not found", target), term.span)]
                })?;
                self.builder.build_unconditional_branch(*target_bb)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM branch error: {}", e), term.span)])?;
            }

            TerminatorKind::SwitchInt { discr, targets } => {
                let discr_val = self.compile_mir_operand(discr, body, escape_results)?;
                let discr_int = discr_val.into_int_value();

                let otherwise_bb = *llvm_blocks.get(&targets.otherwise).ok_or_else(|| {
                    vec![Diagnostic::error("Otherwise block not found", term.span)]
                })?;

                let cases: Vec<_> = targets.branches.iter()
                    .filter_map(|(val, bb_id)| {
                        let bb = llvm_blocks.get(bb_id)?;
                        let val_const = discr_int.get_type().const_int(*val as u64, false);
                        Some((val_const, *bb))
                    })
                    .collect();

                self.builder.build_switch(discr_int, otherwise_bb, &cases)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM switch error: {}", e), term.span)])?;
            }

            TerminatorKind::Return => {
                // Load return value from _0 and return
                let ret_local = LocalId::new(0);
                if let Some(&ret_ptr) = self.locals.get(&ret_local) {
                    let ret_ty = body.return_type();
                    if !ret_ty.is_unit() {
                        let ret_val = self.builder.build_load(ret_ptr, "ret_val")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM load error: {}", e), term.span
                            )])?;
                        self.builder.build_return(Some(&ret_val))
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM return error: {}", e), term.span
                            )])?;
                    } else {
                        self.builder.build_return(None)
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM return error: {}", e), term.span
                            )])?;
                    }
                } else {
                    self.builder.build_return(None)
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM return error: {}", e), term.span
                        )])?;
                }
            }

            TerminatorKind::Unreachable => {
                self.builder.build_unreachable()
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM unreachable error: {}", e), term.span
                    )])?;
            }

            TerminatorKind::Call { func, args, destination, target, unwind: _ } => {
                // Compile arguments
                let arg_vals: Vec<BasicValueEnum> = args.iter()
                    .map(|arg| self.compile_mir_operand(arg, body, escape_results))
                    .collect::<Result<_, _>>()?;

                let arg_metas: Vec<_> = arg_vals.iter().map(|v| (*v).into()).collect();

                // Helper to extract closure DefId from a place's type
                let get_closure_def_id = |place: &Place, body: &MirBody| -> Option<DefId> {
                    let local = body.locals.get(place.local.index() as usize)?;
                    match local.ty.kind() {
                        TypeKind::Closure { def_id, .. } => Some(def_id.clone()),
                        _ => None,
                    }
                };

                // Handle different function operand types
                let call_result = match func {
                    Operand::Constant(Constant { kind: ConstantKind::FnDef(def_id), .. }) => {
                        // Direct function call
                        if let Some(&fn_value) = self.functions.get(def_id) {
                            self.builder.build_call(fn_value, &arg_metas, "call_result")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM call error: {}", e), term.span
                                )])?
                        } else if let Some(builtin_name) = self.builtin_fns.get(def_id) {
                            // Builtin function call - lookup runtime function by name
                            if let Some(fn_value) = self.module.get_function(builtin_name) {
                                self.builder.build_call(fn_value, &arg_metas, "builtin_call")
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM call error: {}", e), term.span
                                    )])?
                            } else {
                                return Err(vec![Diagnostic::error(
                                    format!("Runtime function '{}' not declared", builtin_name), term.span
                                )]);
                            }
                        } else {
                            return Err(vec![Diagnostic::error(
                                format!("Function {:?} not found", def_id), term.span
                            )]);
                        }
                    }
                    // Check for closure call: func is Copy/Move of a local with Closure type
                    Operand::Copy(place) | Operand::Move(place) => {
                        if let Some(closure_def_id) = get_closure_def_id(place, body) {
                            // Closure call - call the closure function with environment as first arg
                            if let Some(&fn_value) = self.functions.get(&closure_def_id) {
                                // Get the closure value (i8* pointer to captures struct)
                                let closure_ptr = self.compile_mir_operand(func, body, escape_results)?;
                                let closure_ptr = closure_ptr.into_pointer_value();

                                // Get the expected env type from the function's first parameter
                                let env_arg = if let Some(first_param) = fn_value.get_first_param() {
                                    // Cast i8* to the correct struct pointer type and load
                                    let first_param_ptr_ty = first_param.get_type().ptr_type(AddressSpace::default());
                                    let typed_ptr = self.builder.build_pointer_cast(
                                        closure_ptr,
                                        first_param_ptr_ty,
                                        "env_typed_ptr"
                                    ).map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM pointer cast error: {}", e), term.span
                                    )])?;
                                    self.builder.build_load(typed_ptr, "env_load")
                                        .map_err(|e| vec![Diagnostic::error(
                                            format!("LLVM load error: {}", e), term.span
                                        )])?
                                } else {
                                    // No parameters means no captures, pass null
                                    self.context.i8_type().ptr_type(AddressSpace::default()).const_null().into()
                                };

                                // Prepend the closure environment to the arguments
                                let mut full_args: Vec<BasicMetadataValueEnum> = Vec::with_capacity(args.len() + 1);
                                full_args.push(env_arg.into());
                                full_args.extend(arg_metas.iter().cloned());

                                self.builder.build_call(fn_value, &full_args, "closure_call")
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM call error: {}", e), term.span
                                    )])?
                            } else {
                                return Err(vec![Diagnostic::error(
                                    format!("Closure function {:?} not found", closure_def_id), term.span
                                )]);
                            }
                        } else {
                            // Indirect call through function pointer
                            let func_val = self.compile_mir_operand(func, body, escape_results)?;
                            let fn_ptr = if let BasicValueEnum::PointerValue(ptr) = func_val {
                                ptr
                            } else {
                                return Err(vec![Diagnostic::error(
                                    "Expected function pointer for call", term.span
                                )]);
                            };

                            // Try to convert to CallableValue for indirect call
                            let callable = inkwell::values::CallableValue::try_from(fn_ptr)
                                .map_err(|_| vec![Diagnostic::error(
                                    "Invalid function pointer for call", term.span
                                )])?;

                            self.builder.build_call(callable, &arg_metas, "call_result")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM call error: {}", e), term.span
                                )])?
                        }
                    }
                    Operand::Constant(_) => {
                        // Non-function constant used as function
                        return Err(vec![Diagnostic::error(
                            "Expected callable value (function, closure, or function pointer)", term.span
                        )]);
                    }
                };

                // Store result to destination
                let dest_ptr = self.compile_mir_place(destination, body)?;
                if let Some(ret_val) = call_result.try_as_basic_value().left() {
                    self.builder.build_store(dest_ptr, ret_val)
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM store error: {}", e), term.span
                        )])?;
                }

                // Branch to continuation
                if let Some(target_bb_id) = target {
                    let target_bb = llvm_blocks.get(target_bb_id).ok_or_else(|| {
                        vec![Diagnostic::error("Call target block not found", term.span)]
                    })?;
                    self.builder.build_unconditional_branch(*target_bb)
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM branch error: {}", e), term.span
                        )])?;
                }
            }

            TerminatorKind::Assert { cond, expected, msg, target, unwind: _ } => {
                let cond_val = self.compile_mir_operand(cond, body, escape_results)?;
                let cond_int = cond_val.into_int_value();

                let expected_int = self.context.bool_type().const_int(*expected as u64, false);
                let check = self.builder.build_int_compare(
                    IntPredicate::EQ,
                    cond_int,
                    expected_int,
                    "assert_check"
                ).map_err(|e| vec![Diagnostic::error(
                    format!("LLVM compare error: {}", e), term.span
                )])?;

                let current_fn = self.current_fn.ok_or_else(|| {
                    vec![Diagnostic::error("No current function", term.span)]
                })?;

                let assert_ok_bb = self.context.append_basic_block(current_fn, "assert_ok");
                let assert_fail_bb = self.context.append_basic_block(current_fn, "assert_fail");

                self.builder.build_conditional_branch(check, assert_ok_bb, assert_fail_bb)
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM branch error: {}", e), term.span
                    )])?;

                // Assert fail path - call blood_panic with message
                self.builder.position_at_end(assert_fail_bb);

                // Get or create the blood_panic function
                let panic_fn = self.module.get_function("blood_panic")
                    .unwrap_or_else(|| {
                        let void_type = self.context.void_type();
                        let i8_type = self.context.i8_type();
                        let i8_ptr_type = i8_type.ptr_type(AddressSpace::default());
                        let panic_type = void_type.fn_type(&[i8_ptr_type.into()], false);
                        self.module.add_function("blood_panic", panic_type, None)
                    });

                // Create a global string constant for the message
                let msg_str = if msg.is_empty() {
                    "assertion failed"
                } else {
                    msg.as_str()
                };
                let msg_global = self.builder
                    .build_global_string_ptr(msg_str, "assert_msg")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM global string error: {}", e), term.span
                    )])?;

                // Call blood_panic (noreturn)
                self.builder.build_call(panic_fn, &[msg_global.as_pointer_value().into()], "")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM call error: {}", e), term.span
                    )])?;

                // Unreachable after panic (helps LLVM optimization)
                self.builder.build_unreachable()
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM unreachable error: {}", e), term.span
                    )])?;

                // Assert ok path - continue to target
                self.builder.position_at_end(assert_ok_bb);
                let target_bb = llvm_blocks.get(target).ok_or_else(|| {
                    vec![Diagnostic::error("Assert target block not found", term.span)]
                })?;
                self.builder.build_unconditional_branch(*target_bb)
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM branch error: {}", e), term.span
                    )])?;
            }

            TerminatorKind::DropAndReplace { place, value, target, unwind: _ } => {
                // First drop the existing value
                let _ = self.compile_mir_place_load(place, body, escape_results)?;

                // Then store the new value
                let new_val = self.compile_mir_operand(value, body, escape_results)?;
                let ptr = self.compile_mir_place(place, body)?;
                self.builder.build_store(ptr, new_val)
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM store error: {}", e), term.span
                    )])?;

                // Continue to target
                let target_bb = llvm_blocks.get(target).ok_or_else(|| {
                    vec![Diagnostic::error("DropAndReplace target not found", term.span)]
                })?;
                self.builder.build_unconditional_branch(*target_bb)
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM branch error: {}", e), term.span
                    )])?;
            }

            TerminatorKind::Perform { effect_id, op_index, args, destination, target } => {
                // Effect operation - emit runtime call with snapshot capture
                let i32_ty = self.context.i32_type();
                let i64_ty = self.context.i64_type();

                // Compile arguments
                let arg_vals: Vec<_> = args.iter()
                    .map(|a| self.compile_mir_operand(a, body, escape_results))
                    .collect::<Result<_, _>>()?;

                // Create generation snapshot before performing the effect
                // The snapshot captures the current generations of all generational
                // references that may be accessed after resuming.
                let snapshot_create = self.module.get_function("blood_snapshot_create")
                    .ok_or_else(|| vec![Diagnostic::error(
                        "Runtime function blood_snapshot_create not found", term.span
                    )])?;

                let snapshot = self.builder.build_call(snapshot_create, &[], "snapshot")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM call error: {}", e), term.span)])?
                    .try_as_basic_value()
                    .left()
                    .ok_or_else(|| vec![Diagnostic::error("snapshot_create returned void", term.span)])?
                    .into_int_value();

                // Add entries to snapshot for effect-captured locals
                // These are locals that contain generational references that may be
                // accessed after the continuation resumes.
                let snapshot_add_entry = self.module.get_function("blood_snapshot_add_entry")
                    .ok_or_else(|| vec![Diagnostic::error(
                        "Runtime function blood_snapshot_add_entry not found", term.span
                    )])?;

                if let Some(escape) = escape_results {
                    for local in &body.locals {
                        // Check if this local is effect-captured and might contain a genref
                        if escape.is_effect_captured(local.id) && self.type_may_contain_genref(&local.ty) {
                            // Get the local's storage
                            if let Some(&local_ptr) = self.locals.get(&local.id) {
                                // For now, we assume the local stores a pointer value.
                                // Full 128-bit pointer support would extract address and generation
                                // from the BloodPtr structure. For now, we use the pointer address
                                // and a placeholder generation.
                                //
                                // TODO: When 128-bit BloodPtr is implemented:
                                // 1. Load the BloodPtr struct from local_ptr
                                // 2. Extract address field (bits 0-63)
                                // 3. Extract generation field (bits 64-95)
                                // 4. Call blood_snapshot_add_entry(snapshot, address, generation)
                                //
                                // For now, use pointer-to-int conversion as address with gen=1
                                let ptr_val = self.builder.build_load(local_ptr, &format!("cap_{}", local.id.index))
                                    .map_err(|e| vec![Diagnostic::error(format!("LLVM load error: {}", e), term.span)])?;

                                // Check if it's a pointer type (we can convert to int)
                                if ptr_val.is_pointer_value() {
                                    let address = self.builder.build_ptr_to_int(
                                        ptr_val.into_pointer_value(),
                                        i64_ty,
                                        "addr"
                                    ).map_err(|e| vec![Diagnostic::error(format!("LLVM ptr_to_int error: {}", e), term.span)])?;

                                    // Use generation 1 (FIRST) as placeholder until 128-bit pointers
                                    let generation = i32_ty.const_int(1, false);

                                    self.builder.build_call(
                                        snapshot_add_entry,
                                        &[snapshot.into(), address.into(), generation.into()],
                                        ""
                                    ).map_err(|e| vec![Diagnostic::error(format!("LLVM call error: {}", e), term.span)])?;
                                } else if ptr_val.is_int_value() {
                                    // If it's already an int (could be packed pointer), use it directly
                                    let int_val = ptr_val.into_int_value();
                                    let address = if int_val.get_type().get_bit_width() < 64 {
                                        self.builder.build_int_z_extend(int_val, i64_ty, "addr")
                                            .map_err(|e| vec![Diagnostic::error(format!("LLVM extend error: {}", e), term.span)])?
                                    } else {
                                        int_val
                                    };
                                    let generation = i32_ty.const_int(1, false);

                                    self.builder.build_call(
                                        snapshot_add_entry,
                                        &[snapshot.into(), address.into(), generation.into()],
                                        ""
                                    ).map_err(|e| vec![Diagnostic::error(format!("LLVM call error: {}", e), term.span)])?;
                                }
                                // For non-pointer types, skip (they don't contain genrefs)
                            }
                        }
                    }
                }

                // Call blood_perform with effect_id, op_index, args
                let perform_fn = self.module.get_function("blood_perform")
                    .ok_or_else(|| vec![Diagnostic::error(
                        "Runtime function blood_perform not found", term.span
                    )])?;

                // Pack arguments into array for perform call
                let arg_count = i64_ty.const_int(arg_vals.len() as u64, false);
                let effect_id_val = i64_ty.const_int(effect_id.index as u64, false);
                let op_index_val = i32_ty.const_int(*op_index as u64, false);

                // Create args array on stack if needed
                let args_ptr = if arg_vals.is_empty() {
                    i64_ty.ptr_type(AddressSpace::default()).const_null()
                } else {
                    let array_ty = i64_ty.array_type(arg_vals.len() as u32);
                    let args_alloca = self.builder.build_alloca(array_ty, "perform_args")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM alloca error: {}", e), term.span)])?;

                    // Store each argument (converted to i64)
                    // Use build_gep with [0, idx] for array element access
                    let zero = i64_ty.const_int(0, false);
                    for (i, val) in arg_vals.iter().enumerate() {
                        let idx = i64_ty.const_int(i as u64, false);
                        let gep = unsafe {
                            self.builder.build_gep(args_alloca, &[zero, idx], &format!("arg_{}", i))
                        }.map_err(|e| vec![Diagnostic::error(format!("LLVM GEP error: {}", e), term.span)])?;
                        let val_i64 = self.builder.build_int_z_extend(val.into_int_value(), i64_ty, "arg_i64")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM extend error: {}", e), term.span)])?;
                        self.builder.build_store(gep, val_i64)
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM store error: {}", e), term.span)])?;
                    }

                    self.builder.build_pointer_cast(
                        args_alloca,
                        i64_ty.ptr_type(AddressSpace::default()),
                        "args_ptr"
                    ).map_err(|e| vec![Diagnostic::error(format!("LLVM cast error: {}", e), term.span)])?
                };

                let result = self.builder.build_call(
                    perform_fn,
                    &[effect_id_val.into(), op_index_val.into(), args_ptr.into(), arg_count.into()],
                    "perform_result"
                ).map_err(|e| vec![Diagnostic::error(format!("LLVM call error: {}", e), term.span)])?;

                // Store result to destination with type conversion
                // blood_perform returns i64, but destination may be a different type.
                // Get the destination type and convert accordingly.
                let dest_local = &body.locals[destination.local.index() as usize];
                let dest_llvm_ty = self.lower_type(&dest_local.ty);

                // Skip storing for unit type (empty struct) - there's no actual value to store
                let is_unit_type = dest_local.ty.is_unit();

                if !is_unit_type {
                    let dest_ptr = self.compile_mir_place(destination, body)?;
                    let result_val = result.try_as_basic_value()
                        .left()
                        .ok_or_else(|| vec![ice_err!(
                            term.span,
                            "blood_perform returned void unexpectedly";
                            "expected" => "i64 return value from runtime function"
                        )])?;

                    let converted_result: BasicValueEnum = if dest_llvm_ty.is_int_type() {
                        let dest_int_ty = dest_llvm_ty.into_int_type();
                        let result_i64 = result_val.into_int_value();
                        let dest_bits = dest_int_ty.get_bit_width();

                        if dest_bits < 64 {
                            // Truncate i64 to smaller integer type
                            self.builder.build_int_truncate(result_i64, dest_int_ty, "perform_trunc")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM truncate error: {}", e), term.span
                                )])?.into()
                        } else if dest_bits > 64 {
                            // Zero-extend i64 to larger integer type
                            self.builder.build_int_z_extend(result_i64, dest_int_ty, "perform_ext")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM extend error: {}", e), term.span
                                )])?.into()
                        } else {
                            // Same size, use directly
                            result_val
                        }
                    } else if dest_llvm_ty.is_pointer_type() {
                        // Convert i64 to pointer
                        let result_i64 = result_val.into_int_value();
                        self.builder.build_int_to_ptr(
                            result_i64,
                            dest_llvm_ty.into_pointer_type(),
                            "perform_ptr"
                        ).map_err(|e| vec![Diagnostic::error(
                            format!("LLVM int_to_ptr error: {}", e), term.span
                        )])?.into()
                    } else {
                        // For other types (struct, etc.), use the value directly
                        // This may fail if types don't match, but that indicates a bug elsewhere
                        result_val
                    };

                    self.builder.build_store(dest_ptr, converted_result)
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM store error: {}", e), term.span)])?;
                }

                // Validate snapshot on return from effect
                // This checks that all generational references are still valid
                let snapshot_validate = self.module.get_function("blood_snapshot_validate")
                    .ok_or_else(|| vec![Diagnostic::error(
                        "Runtime function blood_snapshot_validate not found", term.span
                    )])?;

                let validation_result = self.builder.build_call(
                    snapshot_validate,
                    &[snapshot.into()],
                    "validation"
                ).map_err(|e| vec![Diagnostic::error(format!("LLVM call error: {}", e), term.span)])?
                    .try_as_basic_value()
                    .left()
                    .ok_or_else(|| vec![Diagnostic::error("snapshot_validate returned void", term.span)])?
                    .into_int_value();

                // Destroy snapshot after validation
                let snapshot_destroy = self.module.get_function("blood_snapshot_destroy")
                    .ok_or_else(|| vec![Diagnostic::error(
                        "Runtime function blood_snapshot_destroy not found", term.span
                    )])?;
                self.builder.build_call(snapshot_destroy, &[snapshot.into()], "")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM call error: {}", e), term.span)])?;

                // Check if validation failed
                let fn_value = self.current_fn.ok_or_else(|| {
                    vec![Diagnostic::error("No current function", term.span)]
                })?;

                let continue_bb = self.context.append_basic_block(fn_value, "snapshot_valid");
                let stale_bb = self.context.append_basic_block(fn_value, "snapshot_stale");

                let is_valid = self.builder.build_int_compare(
                    IntPredicate::EQ,
                    validation_result,
                    i64_ty.const_int(0, false),
                    "is_valid"
                ).map_err(|e| vec![Diagnostic::error(format!("LLVM compare error: {}", e), term.span)])?;

                self.builder.build_conditional_branch(is_valid, continue_bb, stale_bb)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM branch error: {}", e), term.span)])?;

                // Stale path - panic
                self.builder.position_at_end(stale_bb);
                let panic_fn = self.module.get_function("blood_stale_reference_panic")
                    .ok_or_else(|| vec![Diagnostic::error(
                        "Runtime function blood_stale_reference_panic not found", term.span
                    )])?;
                self.builder.build_call(panic_fn, &[i32_ty.const_int(0, false).into(), i32_ty.const_int(0, false).into()], "")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM call error: {}", e), term.span)])?;
                self.builder.build_unreachable()
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), term.span)])?;

                // Continue to target on valid path
                self.builder.position_at_end(continue_bb);
                let target_bb = llvm_blocks.get(target).ok_or_else(|| {
                    vec![Diagnostic::error("Perform target not found", term.span)]
                })?;
                self.builder.build_unconditional_branch(*target_bb)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM branch error: {}", e), term.span)])?;
            }

            TerminatorKind::Resume { value } => {
                // Resume from effect handler - return value to continuation
                if let Some(val_op) = value {
                    let val = self.compile_mir_operand(val_op, body, escape_results)?;
                    // Store return value in return place
                    if let Some(&ret_ptr) = self.locals.get(&LocalId::new(0)) {
                        self.builder.build_store(ret_ptr, val)
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM store error: {}", e), term.span
                            )])?;
                    }
                }
                // Return from handler - the caller will resume the continuation
                self.builder.build_return(None)
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM return error: {}", e), term.span
                    )])?;
            }

            TerminatorKind::StaleReference { ptr, expected, actual } => {
                // Stale reference detected - raise effect or panic
                // Compile the place to get the pointer value (for diagnostics)
                let _ptr_val = self.compile_mir_place(ptr, body)?;

                // Get the panic function
                let panic_fn = self.module.get_function("blood_stale_reference_panic")
                    .ok_or_else(|| vec![Diagnostic::error(
                        "Runtime function blood_stale_reference_panic not found", term.span
                    )])?;

                // Create constant values for expected and actual generations
                let i32_ty = self.context.i32_type();
                let expected_int = i32_ty.const_int(*expected as u64, false);
                let actual_int = i32_ty.const_int(*actual as u64, false);

                // Call the panic handler with expected and actual generations
                self.builder.build_call(panic_fn, &[expected_int.into(), actual_int.into()], "")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM call error: {}", e), term.span
                    )])?;

                // After panic, code is unreachable
                self.builder.build_unreachable()
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM unreachable error: {}", e), term.span
                    )])?;
            }
        }

        Ok(())
    }

    fn compile_mir_rvalue(
        &mut self,
        rvalue: &Rvalue,
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        match rvalue {
            Rvalue::Use(operand) => {
                self.compile_mir_operand(operand, body, escape_results)
            }

            Rvalue::Ref { place, mutable: _ } => {
                let ptr = self.compile_mir_place(place, body)?;
                Ok(ptr.into())
            }

            Rvalue::AddressOf { place, mutable: _ } => {
                let ptr = self.compile_mir_place(place, body)?;
                Ok(ptr.into())
            }

            Rvalue::BinaryOp { op, left, right } => {
                let lhs = self.compile_mir_operand(left, body, escape_results)?;
                let rhs = self.compile_mir_operand(right, body, escape_results)?;
                self.compile_binary_op(*op, lhs, rhs)
            }

            Rvalue::CheckedBinaryOp { op, left, right } => {
                // For now, same as unchecked - TODO: return tuple with overflow flag
                let lhs = self.compile_mir_operand(left, body, escape_results)?;
                let rhs = self.compile_mir_operand(right, body, escape_results)?;
                self.compile_binary_op(*op, lhs, rhs)
            }

            Rvalue::UnaryOp { op, operand } => {
                let val = self.compile_mir_operand(operand, body, escape_results)?;
                self.compile_unary_op(*op, val)
            }

            Rvalue::Cast { operand, target_ty } => {
                let val = self.compile_mir_operand(operand, body, escape_results)?;
                self.compile_mir_cast(val, target_ty)
            }

            Rvalue::Discriminant(place) => {
                let ptr = self.compile_mir_place(place, body)?;
                // Load discriminant from first field
                let discr = self.builder.build_load(ptr, "discr")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM load error: {}", e), Span::dummy()
                    )])?;
                Ok(discr)
            }

            Rvalue::Len(place) => {
                // Array/slice length computation
                // For arrays, we extract the static size from the type
                // For slices, we load the length from the fat pointer (not yet implemented)

                // Get the base type from the local
                let base_ty = body.locals[place.local.index() as usize].ty.clone();

                // Compute the effective type after applying projections
                let effective_ty = self.compute_place_type(&base_ty, &place.projection);

                // Extract length based on the type
                match effective_ty.kind() {
                    TypeKind::Array { size, .. } => {
                        // For arrays, return the static size as a usize (i64)
                        let len_val = self.context.i64_type().const_int(*size, false);
                        Ok(len_val.into())
                    }
                    TypeKind::Slice { .. } => {
                        // For slices, we need to extract the length from the fat pointer
                        // A slice is represented as (ptr, len), so we need to load the second element
                        // This requires fat pointer support which is complex to implement correctly
                        Err(vec![Diagnostic::error(
                            "Slice length extraction not yet implemented. \
                             Slices are fat pointers requiring special handling.",
                            Span::dummy()
                        )])
                    }
                    TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
                        // For references/pointers to arrays, extract from the inner type
                        match inner.kind() {
                            TypeKind::Array { size, .. } => {
                                let len_val = self.context.i64_type().const_int(*size, false);
                                Ok(len_val.into())
                            }
                            TypeKind::Slice { .. } => {
                                Err(vec![Diagnostic::error(
                                    "Slice length extraction through reference not yet implemented.",
                                    Span::dummy()
                                )])
                            }
                            _ => {
                                Err(vec![Diagnostic::error(
                                    format!("Cannot compute length of type {:?}", inner.kind()),
                                    Span::dummy()
                                )])
                            }
                        }
                    }
                    _ => {
                        Err(vec![Diagnostic::error(
                            format!("Cannot compute length of type {:?}", effective_ty.kind()),
                            Span::dummy()
                        )])
                    }
                }
            }

            Rvalue::Aggregate { kind, operands } => {
                self.compile_aggregate(kind, operands, body, escape_results)
            }

            Rvalue::NullCheck(operand) => {
                let val = self.compile_mir_operand(operand, body, escape_results)?;
                if let BasicValueEnum::PointerValue(ptr) = val {
                    let null = ptr.get_type().const_null();
                    let is_null = self.builder.build_int_compare(
                        IntPredicate::NE,
                        self.builder.build_ptr_to_int(ptr, self.context.i64_type(), "ptr_int")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?,
                        self.builder.build_ptr_to_int(null, self.context.i64_type(), "null_int")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?,
                        "not_null"
                    ).map_err(|e| vec![Diagnostic::error(
                        format!("LLVM compare error: {}", e), Span::dummy()
                    )])?;
                    Ok(is_null.into())
                } else {
                    Ok(self.context.bool_type().const_int(1, false).into())
                }
            }

            Rvalue::ReadGeneration(place) => {
                // Read generation from a generational pointer (BloodPtr)
                // BloodPtr structure: { address: i64, generation: i32, metadata: i32 }
                // Generation is at field index 1
                let ptr = self.compile_mir_place(place, body)?;

                // Load the BloodPtr struct
                let blood_ptr_val = self.builder.build_load(ptr, "blood_ptr")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM load error: {}", e),
                        Span::dummy()
                    )])?;

                // Extract the generation field (index 1) from the struct
                if blood_ptr_val.is_struct_value() {
                    let struct_val = blood_ptr_val.into_struct_value();
                    let gen_val = self.builder
                        .build_extract_value(struct_val, 1, "generation")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("Failed to extract generation field from BloodPtr: {}", e),
                            Span::dummy()
                        )])?;
                    Ok(gen_val)
                } else {
                    // The place might be a raw pointer, not a BloodPtr struct
                    // In that case, return generation::FIRST (1) as a fallback for stack pointers
                    let i32_ty = self.context.i32_type();
                    Ok(i32_ty.const_int(1, false).into())
                }
            }

            Rvalue::MakeGenPtr { address, generation, metadata } => {
                // Create a generational pointer (128-bit BloodPtr)
                // BloodPtr structure: { address: i64, generation: i32, metadata: i32 }
                let addr_val = self.compile_mir_operand(address, body, escape_results)?;
                let gen_val = self.compile_mir_operand(generation, body, escape_results)?;
                let meta_val = self.compile_mir_operand(metadata, body, escape_results)?;

                // Create the BloodPtr struct type: { i64, i32, i32 }
                let i64_ty = self.context.i64_type();
                let i32_ty = self.context.i32_type();
                let blood_ptr_type = self.context.struct_type(
                    &[i64_ty.into(), i32_ty.into(), i32_ty.into()],
                    false
                );

                // Ensure operands have correct types
                let addr_i64 = if addr_val.is_pointer_value() {
                    // Convert pointer to i64
                    self.builder.build_ptr_to_int(
                        addr_val.into_pointer_value(),
                        i64_ty,
                        "addr_as_i64"
                    ).map_err(|e| vec![Diagnostic::error(
                        format!("LLVM ptr_to_int error: {}", e),
                        Span::dummy()
                    )])?
                } else if addr_val.is_int_value() {
                    let int_val = addr_val.into_int_value();
                    if int_val.get_type().get_bit_width() == 64 {
                        int_val
                    } else {
                        // Zero-extend or truncate to i64
                        self.builder.build_int_z_extend_or_bit_cast(
                            int_val,
                            i64_ty,
                            "addr_i64"
                        ).map_err(|e| vec![Diagnostic::error(
                            format!("LLVM int cast error: {}", e),
                            Span::dummy()
                        )])?
                    }
                } else {
                    return Err(vec![Diagnostic::error(
                        "MakeGenPtr address must be a pointer or integer".to_string(),
                        Span::dummy()
                    )]);
                };

                let gen_i32 = if gen_val.is_int_value() {
                    let int_val = gen_val.into_int_value();
                    if int_val.get_type().get_bit_width() == 32 {
                        int_val
                    } else {
                        self.builder.build_int_truncate_or_bit_cast(
                            int_val,
                            i32_ty,
                            "gen_i32"
                        ).map_err(|e| vec![Diagnostic::error(
                            format!("LLVM int cast error: {}", e),
                            Span::dummy()
                        )])?
                    }
                } else {
                    return Err(vec![Diagnostic::error(
                        "MakeGenPtr generation must be an integer".to_string(),
                        Span::dummy()
                    )]);
                };

                let meta_i32 = if meta_val.is_int_value() {
                    let int_val = meta_val.into_int_value();
                    if int_val.get_type().get_bit_width() == 32 {
                        int_val
                    } else {
                        self.builder.build_int_truncate_or_bit_cast(
                            int_val,
                            i32_ty,
                            "meta_i32"
                        ).map_err(|e| vec![Diagnostic::error(
                            format!("LLVM int cast error: {}", e),
                            Span::dummy()
                        )])?
                    }
                } else {
                    return Err(vec![Diagnostic::error(
                        "MakeGenPtr metadata must be an integer".to_string(),
                        Span::dummy()
                    )]);
                };

                // Build the BloodPtr struct value
                let mut blood_ptr_val = blood_ptr_type.get_undef();
                blood_ptr_val = self.builder
                    .build_insert_value(blood_ptr_val, addr_i64, 0, "with_addr")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("Failed to insert address into BloodPtr: {}", e),
                        Span::dummy()
                    )])?
                    .into_struct_value();
                blood_ptr_val = self.builder
                    .build_insert_value(blood_ptr_val, gen_i32, 1, "with_gen")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("Failed to insert generation into BloodPtr: {}", e),
                        Span::dummy()
                    )])?
                    .into_struct_value();
                blood_ptr_val = self.builder
                    .build_insert_value(blood_ptr_val, meta_i32, 2, "with_meta")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("Failed to insert metadata into BloodPtr: {}", e),
                        Span::dummy()
                    )])?
                    .into_struct_value();

                Ok(blood_ptr_val.into())
            }
        }
    }

    fn compile_mir_operand(
        &mut self,
        operand: &Operand,
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                self.compile_mir_place_load(place, body, escape_results)
            }

            Operand::Constant(constant) => {
                self.compile_constant(constant)
            }
        }
    }

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

    fn get_local_tier(
        &self,
        local: LocalId,
        escape_results: Option<&EscapeResults>,
    ) -> MemoryTier {
        if let Some(results) = escape_results {
            results.recommended_tier(local)
        } else {
            // Default to Region if no escape analysis available
            MemoryTier::Region
        }
    }

    fn should_skip_gen_check(
        &self,
        local: LocalId,
        escape_results: Option<&EscapeResults>,
    ) -> bool {
        if let Some(results) = escape_results {
            let state = results.get(local);
            state == EscapeState::NoEscape && !results.is_effect_captured(local)
        } else {
            false // Conservative: always check if no analysis
        }
    }

    fn emit_generation_check(
        &mut self,
        ptr: PointerValue<'ctx>,
        expected_gen: IntValue<'ctx>,
        span: Span,
    ) -> Result<(), Vec<Diagnostic>> {
        // Emit a generation check by calling the runtime function.
        //
        // The runtime function `blood_validate_generation` handles:
        // 1. Looking up the current generation from the slot registry
        // 2. Comparing with the expected generation
        // 3. Returns 0 if valid, 1 if stale
        //
        // If the check fails, we call `blood_stale_reference_panic` which aborts.

        let i32_ty = self.context.i32_type();
        let i64_ty = self.context.i64_type();

        // Get the validation function - uses slot registry for address-based lookup
        let validate_fn = self.module.get_function("blood_validate_generation")
            .ok_or_else(|| vec![Diagnostic::error(
                "blood_validate_generation not declared", span
            )])?;

        // Convert pointer to i64 address for the runtime call
        let address = self.builder.build_ptr_to_int(ptr, i64_ty, "ptr_addr")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

        // Call blood_validate_generation(address: i64, expected_gen: i32) -> i32
        // Returns: 0 = valid, 1 = stale (generation mismatch)
        let result = self.builder.build_call(
            validate_fn,
            &[address.into(), expected_gen.into()],
            "gen_check"
        ).map_err(|e| vec![Diagnostic::error(format!("LLVM call error: {}", e), span)])?;

        let is_stale = result.try_as_basic_value()
            .left()
            .ok_or_else(|| vec![Diagnostic::error("Generation check returned void", span)])?
            .into_int_value();

        // Create blocks for valid and stale paths
        let fn_value = self.current_fn.ok_or_else(|| {
            vec![Diagnostic::error("No current function", span)]
        })?;

        let valid_bb = self.context.append_basic_block(fn_value, "gen_valid");
        let stale_bb = self.context.append_basic_block(fn_value, "gen_stale");

        // Compare: is_stale == 0 (valid if result is 0)
        let zero = i32_ty.const_int(0, false);
        let is_valid = self.builder.build_int_compare(
            IntPredicate::EQ,
            is_stale,
            zero,
            "is_valid"
        ).map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

        self.builder.build_conditional_branch(is_valid, valid_bb, stale_bb)
            .map_err(|e| vec![Diagnostic::error(format!("LLVM branch error: {}", e), span)])?;

        // Stale path: call panic handler
        self.builder.position_at_end(stale_bb);
        let panic_fn = self.module.get_function("blood_stale_reference_panic")
            .ok_or_else(|| vec![Diagnostic::error(
                "blood_stale_reference_panic not declared", span
            )])?;

        // Get current generation for the error message
        if let Some(get_gen_fn) = self.module.get_function("blood_get_generation") {
            let gen_call_result = self.builder.build_call(
                get_gen_fn,
                &[address.into()],
                "actual_gen"
            ).map_err(|e| vec![Diagnostic::error(format!("LLVM call error: {}", e), span)])?;

            let actual_gen = match gen_call_result.try_as_basic_value().left() {
                Some(val) => val.into_int_value(),
                None => {
                    // blood_get_generation returned void unexpectedly - this is an ICE
                    // but we're already in a panic path, so log and continue with fallback
                    ice!("blood_get_generation returned void unexpectedly";
                         "span" => span,
                         "fallback" => "using 0 for panic message");
                    i32_ty.const_int(0, false)
                }
            };

            self.builder.build_call(panic_fn, &[expected_gen.into(), actual_gen.into()], "")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM call error: {}", e), span)])?;
        } else {
            // blood_get_generation not available - use expected as fallback for both args
            // This is acceptable as we're in a panic path and need some value for the error message
            self.builder.build_call(panic_fn, &[expected_gen.into(), expected_gen.into()], "")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM call error: {}", e), span)])?;
        }

        self.builder.build_unreachable()
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

        // Continue at valid block
        self.builder.position_at_end(valid_bb);

        Ok(())
    }

    fn type_may_contain_genref(&self, ty: &Type) -> bool {
        use crate::hir::TypeKind;

        match &*ty.kind {
            // Pointer and reference types may contain generational references
            TypeKind::Ptr { .. } | TypeKind::Ref { .. } => true,

            // Arrays and slices may contain genrefs if their element type does
            TypeKind::Array { ref element, .. } => self.type_may_contain_genref(element),
            TypeKind::Slice { ref element } => self.type_may_contain_genref(element),

            // Tuples may contain genrefs if any element does
            TypeKind::Tuple(elems) => elems.iter().any(|e| self.type_may_contain_genref(e)),

            // ADTs (structs, enums) might contain genrefs - be conservative
            TypeKind::Adt { .. } => true,

            // Functions might capture genrefs (as function pointers to closures)
            TypeKind::Fn { .. } => true,

            // Closures might capture genrefs in their environment
            TypeKind::Closure { .. } => true,

            // Primitives don't contain genrefs
            TypeKind::Primitive(_) => false,

            // Never and Error types don't contain genrefs
            TypeKind::Never | TypeKind::Error => false,

            // Inference and type parameters - be conservative
            TypeKind::Infer(_) | TypeKind::Param(_) => true,
        }
    }

    fn allocate_with_blood_alloc(
        &mut self,
        llvm_ty: BasicTypeEnum<'ctx>,
        local_id: LocalId,
        span: Span,
    ) -> Result<PointerValue<'ctx>, Vec<Diagnostic>> {
        // Use blood_alloc_or_abort for region/persistent allocation.
        // This function aborts on failure, so no conditional branches needed.

        let i32_ty = self.context.i32_type();
        let i64_ty = self.context.i64_type();

        // Get the blood_alloc_or_abort function
        let alloc_fn = self.module.get_function("blood_alloc_or_abort")
            .ok_or_else(|| vec![Diagnostic::error(
                "Runtime function blood_alloc_or_abort not found", span
            )])?;

        // Calculate size of the type
        let type_size = self.get_type_size_in_bytes(llvm_ty);
        let size_val = i64_ty.const_int(type_size, false);

        // Create stack alloca for the generation output parameter
        let gen_alloca = self.builder.build_alloca(i32_ty, &format!("_{}_gen", local_id.index))
            .map_err(|e| vec![Diagnostic::error(
                format!("LLVM alloca error: {}", e), span
            )])?;

        // Call blood_alloc_or_abort(size, &out_generation) -> address
        let address = self.builder.build_call(
            alloc_fn,
            &[size_val.into(), gen_alloca.into()],
            &format!("_{}_addr", local_id.index)
        ).map_err(|e| vec![Diagnostic::error(
            format!("LLVM call error: {}", e), span
        )])?
            .try_as_basic_value()
            .left()
            .ok_or_else(|| vec![Diagnostic::error(
                "blood_alloc_or_abort returned void", span
            )])?
            .into_int_value();

        // Convert the address (i64) to a pointer of the appropriate type
        let typed_ptr = self.builder.build_int_to_ptr(
            address,
            llvm_ty.ptr_type(AddressSpace::default()),
            &format!("_{}_ptr", local_id.index)
        ).map_err(|e| vec![Diagnostic::error(
            format!("LLVM int_to_ptr error: {}", e), span
        )])?;

        // Store the generation in local_generations map for later validation
        // (The generation is stored in gen_alloca and can be loaded when needed)
        self.local_generations.insert(local_id, gen_alloca);

        Ok(typed_ptr)
    }
}

// Helper functions

fn tier_name(tier: MemoryTier) -> &'static str {
    match tier {
        MemoryTier::Stack => "stack",
        MemoryTier::Region => "region",
        MemoryTier::Persistent => "persistent",
        MemoryTier::Reserved => "reserved",
    }
}

impl<'ctx, 'a> CodegenContext<'ctx, 'a> {
    /// Compute the effective type of a place after applying projections.
    ///
    /// This walks through the projection chain and computes what type the
    /// place represents after all field accesses, dereferences, and indexing.
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

    /// Get the size of an LLVM type in bytes.
    ///
    /// This will be used for blood_alloc when Region tier allocation is properly
    /// implemented with MIR block restructuring.
    #[allow(dead_code)]
    fn get_type_size_in_bytes(&self, ty: BasicTypeEnum<'ctx>) -> u64 {
        match ty {
            BasicTypeEnum::IntType(t) => (t.get_bit_width() as u64 + 7) / 8,
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

    /// Compile a binary operation.
    fn compile_binary_op(
        &mut self,
        op: BinOp,
        lhs: BasicValueEnum<'ctx>,
        rhs: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        let lhs_int = lhs.into_int_value();
        let rhs_int = rhs.into_int_value();

        let result = match op {
            BinOp::Add => self.builder.build_int_add(lhs_int, rhs_int, "add"),
            BinOp::Sub => self.builder.build_int_sub(lhs_int, rhs_int, "sub"),
            BinOp::Mul => self.builder.build_int_mul(lhs_int, rhs_int, "mul"),
            BinOp::Div => self.builder.build_int_signed_div(lhs_int, rhs_int, "div"),
            BinOp::Rem => self.builder.build_int_signed_rem(lhs_int, rhs_int, "rem"),
            BinOp::BitAnd => self.builder.build_and(lhs_int, rhs_int, "and"),
            BinOp::BitOr => self.builder.build_or(lhs_int, rhs_int, "or"),
            BinOp::BitXor => self.builder.build_xor(lhs_int, rhs_int, "xor"),
            BinOp::Shl => self.builder.build_left_shift(lhs_int, rhs_int, "shl"),
            BinOp::Shr => self.builder.build_right_shift(lhs_int, rhs_int, true, "shr"),
            BinOp::Eq => self.builder.build_int_compare(IntPredicate::EQ, lhs_int, rhs_int, "eq"),
            BinOp::Ne => self.builder.build_int_compare(IntPredicate::NE, lhs_int, rhs_int, "ne"),
            BinOp::Lt => self.builder.build_int_compare(IntPredicate::SLT, lhs_int, rhs_int, "lt"),
            BinOp::Le => self.builder.build_int_compare(IntPredicate::SLE, lhs_int, rhs_int, "le"),
            BinOp::Gt => self.builder.build_int_compare(IntPredicate::SGT, lhs_int, rhs_int, "gt"),
            BinOp::Ge => self.builder.build_int_compare(IntPredicate::SGE, lhs_int, rhs_int, "ge"),
            BinOp::Offset => {
                // Pointer offset - treat as add for now
                self.builder.build_int_add(lhs_int, rhs_int, "offset")
            }
        }.map_err(|e| vec![Diagnostic::error(
            format!("LLVM binary op error: {}", e), Span::dummy()
        )])?;

        Ok(result.into())
    }

    /// Compile a unary operation.
    fn compile_unary_op(
        &mut self,
        op: UnOp,
        val: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        let val_int = val.into_int_value();

        let result = match op {
            UnOp::Not => self.builder.build_not(val_int, "not"),
            UnOp::Neg => self.builder.build_int_neg(val_int, "neg"),
        }.map_err(|e| vec![Diagnostic::error(
            format!("LLVM unary op error: {}", e), Span::dummy()
        )])?;

        Ok(result.into())
    }

    /// Compile a type cast from MIR.
    fn compile_mir_cast(
        &mut self,
        val: BasicValueEnum<'ctx>,
        target_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        let target_llvm = self.lower_type(target_ty);

        // For now, just use int casts
        if let (BasicValueEnum::IntValue(int_val), BasicTypeEnum::IntType(int_ty)) = (val, target_llvm) {
            let cast = self.builder.build_int_cast(int_val, int_ty, "cast")
                .map_err(|e| vec![Diagnostic::error(
                    format!("LLVM cast error: {}", e), Span::dummy()
                )])?;
            Ok(cast.into())
        } else {
            // For other types, return as-is for now
            Ok(val)
        }
    }

    /// Compile an aggregate value (struct, tuple, array).
    fn compile_aggregate(
        &mut self,
        kind: &AggregateKind,
        operands: &[Operand],
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        let vals: Vec<BasicValueEnum> = operands.iter()
            .map(|op| self.compile_mir_operand(op, body, escape_results))
            .collect::<Result<_, _>>()?;

        match kind {
            AggregateKind::Tuple => {
                if vals.is_empty() {
                    // Unit tuple
                    Ok(self.context.i8_type().const_int(0, false).into())
                } else {
                    // Create struct for tuple
                    let types: Vec<_> = vals.iter().map(|v| v.get_type()).collect();
                    let struct_ty = self.context.struct_type(&types, false);
                    let mut agg = struct_ty.get_undef();
                    for (i, val) in vals.iter().enumerate() {
                        agg = self.builder.build_insert_value(agg, *val, i as u32, &format!("tuple_{}", i))
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM insert error: {}", e), Span::dummy()
                            )])?
                            .into_struct_value();
                    }
                    Ok(agg.into())
                }
            }

            AggregateKind::Array(_elem_ty) => {
                if vals.is_empty() {
                    let array_ty = self.context.i32_type().array_type(0);
                    Ok(array_ty.get_undef().into())
                } else {
                    let elem_ty = vals[0].get_type();
                    let array_ty = elem_ty.array_type(vals.len() as u32);
                    let mut agg = array_ty.get_undef();
                    for (i, val) in vals.iter().enumerate() {
                        agg = self.builder.build_insert_value(agg, *val, i as u32, &format!("arr_{}", i))
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM insert error: {}", e), Span::dummy()
                            )])?
                            .into_array_value();
                    }
                    Ok(agg.into())
                }
            }

            AggregateKind::Adt { def_id, variant_idx } => {
                // Look up struct/enum definition
                if let Some(field_types) = self.struct_defs.get(def_id) {
                    let llvm_fields: Vec<_> = field_types.iter()
                        .map(|t| self.lower_type(t))
                        .collect();
                    let struct_ty = self.context.struct_type(&llvm_fields, false);
                    let mut agg = struct_ty.get_undef();
                    for (i, val) in vals.iter().enumerate() {
                        agg = self.builder.build_insert_value(agg, *val, i as u32, &format!("field_{}", i))
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM insert error: {}", e), Span::dummy()
                            )])?
                            .into_struct_value();
                    }
                    Ok(agg.into())
                } else if let Some(_variants) = self.enum_defs.get(def_id) {
                    // Enum variant - first field is tag
                    let variant_index = variant_idx.ok_or_else(|| vec![ice_err!(
                        Span::dummy(),
                        "enum construction without variant index";
                        "def_id" => def_id
                    )])?;
                    let tag = self.context.i32_type().const_int(variant_index as u64, false);
                    let mut all_vals = vec![tag.into()];
                    all_vals.extend(vals);

                    let types: Vec<_> = all_vals.iter().map(|v| v.get_type()).collect();
                    let struct_ty = self.context.struct_type(&types, false);
                    let mut agg = struct_ty.get_undef();
                    for (i, val) in all_vals.iter().enumerate() {
                        agg = self.builder.build_insert_value(agg, *val, i as u32, &format!("enum_{}", i))
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM insert error: {}", e), Span::dummy()
                            )])?
                            .into_struct_value();
                    }
                    Ok(agg.into())
                } else {
                    Err(vec![Diagnostic::error(
                        format!("Unknown ADT {:?}", def_id), Span::dummy()
                    )])
                }
            }

            AggregateKind::Closure { def_id: _ } => {
                // Closure is a pointer to captured environment struct
                // The closure function is called directly and receives this pointer
                let i8_ptr_ty = self.context.i8_type().ptr_type(AddressSpace::default());

                if vals.is_empty() {
                    // No captures - use null pointer
                    Ok(i8_ptr_ty.const_null().into())
                } else {
                    // Build a struct with captured values
                    let types: Vec<_> = vals.iter().map(|v| v.get_type()).collect();
                    let captures_struct_ty = self.context.struct_type(&types, false);
                    let mut captures_val = captures_struct_ty.get_undef();
                    for (i, val) in vals.iter().enumerate() {
                        captures_val = self.builder.build_insert_value(captures_val, *val, i as u32, &format!("capture_{}", i))
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM insert error: {}", e), Span::dummy()
                            )])?
                            .into_struct_value();
                    }

                    // Allocate space and store the captures struct
                    let captures_alloca = self.builder.build_alloca(captures_struct_ty, "closure_env")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM alloca error: {}", e), Span::dummy()
                        )])?;
                    self.builder.build_store(captures_alloca, captures_val)
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM store error: {}", e), Span::dummy()
                        )])?;

                    // Cast to i8* for the closure type
                    let env_ptr = self.builder.build_pointer_cast(captures_alloca, i8_ptr_ty, "env_ptr")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM pointer cast error: {}", e), Span::dummy()
                        )])?;

                    Ok(env_ptr.into())
                }
            }
        }
    }

    /// Compile a MIR constant.
    fn compile_constant(
        &mut self,
        constant: &Constant,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        match &constant.kind {
            ConstantKind::Int(v) => {
                let llvm_ty = self.lower_type(&constant.ty);
                if let BasicTypeEnum::IntType(int_ty) = llvm_ty {
                    Ok(int_ty.const_int(*v as u64, *v < 0).into())
                } else {
                    Ok(self.context.i64_type().const_int(*v as u64, *v < 0).into())
                }
            }

            ConstantKind::Uint(v) => {
                let llvm_ty = self.lower_type(&constant.ty);
                if let BasicTypeEnum::IntType(int_ty) = llvm_ty {
                    Ok(int_ty.const_int(*v as u64, false).into())
                } else {
                    Ok(self.context.i64_type().const_int(*v as u64, false).into())
                }
            }

            ConstantKind::Float(v) => {
                Ok(self.context.f64_type().const_float(*v).into())
            }

            ConstantKind::Bool(v) => {
                Ok(self.context.bool_type().const_int(*v as u64, false).into())
            }

            ConstantKind::Char(v) => {
                Ok(self.context.i32_type().const_int(*v as u64, false).into())
            }

            ConstantKind::String(s) => {
                let global = self.builder.build_global_string_ptr(s, "str")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM string error: {}", e), Span::dummy()
                    )])?;
                Ok(global.as_pointer_value().into())
            }

            ConstantKind::Unit => {
                Ok(self.context.i8_type().const_int(0, false).into())
            }

            ConstantKind::FnDef(def_id) => {
                // Function reference - get the function pointer
                if let Some(&fn_val) = self.functions.get(def_id) {
                    Ok(fn_val.as_global_value().as_pointer_value().into())
                } else {
                    Err(vec![Diagnostic::error(
                        format!("Unknown function {:?}", def_id), Span::dummy()
                    )])
                }
            }

            ConstantKind::ZeroSized => {
                Ok(self.context.i8_type().const_int(0, false).into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tier_name() {
        assert_eq!(tier_name(MemoryTier::Stack), "stack");
        assert_eq!(tier_name(MemoryTier::Region), "region");
        assert_eq!(tier_name(MemoryTier::Persistent), "persistent");
    }
}
