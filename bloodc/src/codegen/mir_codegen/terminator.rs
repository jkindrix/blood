//! MIR terminator code generation.
//!
//! This module handles compilation of MIR terminators to LLVM IR.
//! Terminators are the final instructions in a basic block that control
//! program flow (branches, calls, returns, etc.).

use std::collections::HashMap;

use inkwell::basic_block::BasicBlock;
use inkwell::types::BasicType;
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum};
use inkwell::{AddressSpace, IntPredicate};

use crate::diagnostics::Diagnostic;
use crate::hir::LocalId;
use crate::mir::body::MirBody;
use crate::mir::types::{
    BasicBlockId, Terminator, TerminatorKind,
    Operand, Constant, ConstantKind, Place,
};
use crate::mir::EscapeResults;
use crate::ice_err;

use super::rvalue::MirRvalueCodegen;
use super::place::MirPlaceCodegen;
use super::memory::MirMemoryCodegen;
use super::CodegenContext;

/// Extension trait for MIR terminator compilation.
pub trait MirTerminatorCodegen<'ctx, 'a> {
    /// Compile a MIR terminator.
    fn compile_mir_terminator(
        &mut self,
        term: &Terminator,
        body: &MirBody,
        llvm_blocks: &HashMap<BasicBlockId, BasicBlock<'ctx>>,
        escape_results: Option<&EscapeResults>,
    ) -> Result<(), Vec<Diagnostic>>;
}

impl<'ctx, 'a> MirTerminatorCodegen<'ctx, 'a> for CodegenContext<'ctx, 'a> {
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
                self.compile_call_terminator(func, args, destination, target.as_ref(), body, llvm_blocks, escape_results, term.span)?;
            }

            TerminatorKind::Assert { cond, expected, msg, target, unwind: _ } => {
                self.compile_assert_terminator(cond, *expected, msg, target, body, llvm_blocks, escape_results, term.span)?;
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

            TerminatorKind::Perform { effect_id, op_index, args, destination, target, is_tail_resumptive } => {
                self.compile_perform_terminator(
                    effect_id, *op_index, args, destination, target,
                    *is_tail_resumptive, body, llvm_blocks, escape_results, term.span
                )?;
            }

            TerminatorKind::Resume { value } => {
                self.compile_resume_terminator(value.as_ref(), body, escape_results, term.span)?;
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
}

// Helper implementations for complex terminators
impl<'ctx, 'a> CodegenContext<'ctx, 'a> {
    fn compile_call_terminator(
        &mut self,
        func: &Operand,
        args: &[Operand],
        destination: &Place,
        target: Option<&BasicBlockId>,
        body: &MirBody,
        llvm_blocks: &HashMap<BasicBlockId, BasicBlock<'ctx>>,
        escape_results: Option<&EscapeResults>,
        span: crate::span::Span,
    ) -> Result<(), Vec<Diagnostic>> {
        // Compile arguments
        let arg_vals: Vec<BasicValueEnum> = args.iter()
            .map(|arg| self.compile_mir_operand(arg, body, escape_results))
            .collect::<Result<_, _>>()?;

        let arg_metas: Vec<_> = arg_vals.iter().map(|v| (*v).into()).collect();

        // Helper to extract closure DefId from a place's type
        let get_closure_def_id = |place: &Place, body: &MirBody| -> Option<crate::hir::DefId> {
            let local = body.locals.get(place.local.index() as usize)?;
            match local.ty.kind() {
                crate::hir::TypeKind::Closure { def_id, .. } => Some(*def_id),
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
                            format!("LLVM call error: {}", e), span
                        )])?
                } else if let Some(builtin_name) = self.builtin_fns.get(def_id) {
                    // Builtin function call - lookup runtime function by name
                    if let Some(fn_value) = self.module.get_function(builtin_name) {
                        self.builder.build_call(fn_value, &arg_metas, "builtin_call")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM call error: {}", e), span
                            )])?
                    } else {
                        return Err(vec![Diagnostic::error(
                            format!("Runtime function '{}' not declared", builtin_name), span
                        )]);
                    }
                } else {
                    return Err(vec![Diagnostic::error(
                        format!("Function {:?} not found", def_id), span
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
                                format!("LLVM pointer cast error: {}", e), span
                            )])?;
                            self.builder.build_load(typed_ptr, "env_load")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM load error: {}", e), span
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
                                format!("LLVM call error: {}", e), span
                            )])?
                    } else {
                        return Err(vec![Diagnostic::error(
                            format!("Closure function {:?} not found", closure_def_id), span
                        )]);
                    }
                } else {
                    // Indirect call through function pointer
                    let func_val = self.compile_mir_operand(func, body, escape_results)?;
                    let fn_ptr = if let BasicValueEnum::PointerValue(ptr) = func_val {
                        ptr
                    } else {
                        return Err(vec![Diagnostic::error(
                            "Expected function pointer for call", span
                        )]);
                    };

                    // Try to convert to CallableValue for indirect call
                    let callable = inkwell::values::CallableValue::try_from(fn_ptr)
                        .map_err(|_| vec![Diagnostic::error(
                            "Invalid function pointer for call", span
                        )])?;

                    self.builder.build_call(callable, &arg_metas, "call_result")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM call error: {}", e), span
                        )])?
                }
            }
            Operand::Constant(_) => {
                // Non-function constant used as function
                return Err(vec![Diagnostic::error(
                    "Expected callable value (function, closure, or function pointer)", span
                )]);
            }
        };

        // Store result to destination
        let dest_ptr = self.compile_mir_place(destination, body)?;
        if let Some(ret_val) = call_result.try_as_basic_value().left() {
            self.builder.build_store(dest_ptr, ret_val)
                .map_err(|e| vec![Diagnostic::error(
                    format!("LLVM store error: {}", e), span
                )])?;
        }

        // Branch to continuation
        if let Some(target_bb_id) = target {
            let target_bb = llvm_blocks.get(target_bb_id).ok_or_else(|| {
                vec![Diagnostic::error("Call target block not found", span)]
            })?;
            self.builder.build_unconditional_branch(*target_bb)
                .map_err(|e| vec![Diagnostic::error(
                    format!("LLVM branch error: {}", e), span
                )])?;
        }

        Ok(())
    }

    fn compile_assert_terminator(
        &mut self,
        cond: &Operand,
        expected: bool,
        msg: &str,
        target: &BasicBlockId,
        body: &MirBody,
        llvm_blocks: &HashMap<BasicBlockId, BasicBlock<'ctx>>,
        escape_results: Option<&EscapeResults>,
        span: crate::span::Span,
    ) -> Result<(), Vec<Diagnostic>> {
        let cond_val = self.compile_mir_operand(cond, body, escape_results)?;
        let cond_int = cond_val.into_int_value();

        let expected_int = self.context.bool_type().const_int(expected as u64, false);
        let check = self.builder.build_int_compare(
            IntPredicate::EQ,
            cond_int,
            expected_int,
            "assert_check"
        ).map_err(|e| vec![Diagnostic::error(
            format!("LLVM compare error: {}", e), span
        )])?;

        let current_fn = self.current_fn.ok_or_else(|| {
            vec![Diagnostic::error("No current function", span)]
        })?;

        let assert_ok_bb = self.context.append_basic_block(current_fn, "assert_ok");
        let assert_fail_bb = self.context.append_basic_block(current_fn, "assert_fail");

        self.builder.build_conditional_branch(check, assert_ok_bb, assert_fail_bb)
            .map_err(|e| vec![Diagnostic::error(
                format!("LLVM branch error: {}", e), span
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
            msg
        };
        let msg_global = self.builder
            .build_global_string_ptr(msg_str, "assert_msg")
            .map_err(|e| vec![Diagnostic::error(
                format!("LLVM global string error: {}", e), span
            )])?;

        // Call blood_panic (noreturn)
        self.builder.build_call(panic_fn, &[msg_global.as_pointer_value().into()], "")
            .map_err(|e| vec![Diagnostic::error(
                format!("LLVM call error: {}", e), span
            )])?;

        // Unreachable after panic (helps LLVM optimization)
        self.builder.build_unreachable()
            .map_err(|e| vec![Diagnostic::error(
                format!("LLVM unreachable error: {}", e), span
            )])?;

        // Assert ok path - continue to target
        self.builder.position_at_end(assert_ok_bb);
        let target_bb = llvm_blocks.get(target).ok_or_else(|| {
            vec![Diagnostic::error("Assert target block not found", span)]
        })?;
        self.builder.build_unconditional_branch(*target_bb)
            .map_err(|e| vec![Diagnostic::error(
                format!("LLVM branch error: {}", e), span
            )])?;

        Ok(())
    }

    fn compile_perform_terminator(
        &mut self,
        effect_id: &crate::hir::DefId,
        op_index: u32,
        args: &[Operand],
        destination: &Place,
        target: &BasicBlockId,
        is_tail_resumptive: bool,
        body: &MirBody,
        llvm_blocks: &HashMap<BasicBlockId, BasicBlock<'ctx>>,
        escape_results: Option<&EscapeResults>,
        span: crate::span::Span,
    ) -> Result<(), Vec<Diagnostic>> {
        // Effect operation - emit runtime call with snapshot capture
        let i32_ty = self.context.i32_type();
        let i64_ty = self.context.i64_type();

        // For non-tail-resumptive effects (like Error.throw), we need to capture
        // the continuation so the handler can suspend and resume later.
        //
        // Tail-resumptive effects (like State.get/put, IO.print) don't need this
        // because they always resume immediately after the operation completes.
        if !is_tail_resumptive {
            // Non-tail-resumptive effects require continuation capture.
            // Currently we fall through to the synchronous path which works
            // as long as the handler resumes immediately (which Error.throw doesn't).
            //
            // Full continuation capture would require:
            // 1. Generate LLVM function for "rest of computation" from target block
            // 2. Pack live variables into a context struct
            // 3. Call blood_continuation_create(callback, context)
            // 4. Store continuation handle in effect context before blood_perform
            // 5. Handler retrieves continuation and calls blood_continuation_resume
            // 6. **Region Suspension**: For each active region scope containing effect-
            //    captured allocations, call blood_continuation_add_suspended_region()
            //    to defer deallocation until the continuation is resumed or dropped.
            //    The runtime handles this automatically via:
            //    - blood_continuation_add_suspended_region(cont_id, region_id) at capture
            //    - blood_continuation_resume_with_regions() handles restoration on resume
            //
            // For now, non-tail-resumptive effects work correctly if the handler
            // either resumes immediately or is a final control effect (like throw).
        }

        // Compile arguments
        let arg_vals: Vec<_> = args.iter()
            .map(|a| self.compile_mir_operand(a, body, escape_results))
            .collect::<Result<_, _>>()?;

        // Create generation snapshot before performing the effect
        // The snapshot captures the current generations of all generational
        // references that may be accessed after resuming.
        let snapshot_create = self.module.get_function("blood_snapshot_create")
            .ok_or_else(|| vec![Diagnostic::error(
                "Runtime function blood_snapshot_create not found", span
            )])?;

        let snapshot = self.builder.build_call(snapshot_create, &[], "snapshot")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM call error: {}", e), span)])?
            .try_as_basic_value()
            .left()
            .ok_or_else(|| vec![Diagnostic::error("snapshot_create returned void", span)])?
            .into_int_value();

        // Add entries to snapshot for effect-captured locals
        // These are locals that contain generational references that may be
        // accessed after the continuation resumes.
        let snapshot_add_entry = self.module.get_function("blood_snapshot_add_entry")
            .ok_or_else(|| vec![Diagnostic::error(
                "Runtime function blood_snapshot_add_entry not found", span
            )])?;

        if let Some(escape) = escape_results {
            for local in &body.locals {
                // Check if this local is effect-captured and might contain a genref
                if escape.is_effect_captured(local.id) && super::types::type_may_contain_genref_impl(&local.ty) {
                    // Get the local's storage
                    if let Some(&local_ptr) = self.locals.get(&local.id) {
                        // Load the pointer value and extract address/generation.
                        // - For 128-bit BloodPtr: extract address from high 64 bits,
                        //   generation from bits 32-63
                        // - For 64-bit pointers: use ptr-to-int for address,
                        //   look up generation via blood_get_generation runtime call
                        let ptr_val = self.builder.build_load(local_ptr, &format!("cap_{}", local.id.index))
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM load error: {}", e), span)])?;

                        // Check if it's a pointer type (we can convert to int)
                        if ptr_val.is_pointer_value() {
                            let address = self.builder.build_ptr_to_int(
                                ptr_val.into_pointer_value(),
                                i64_ty,
                                "addr"
                            ).map_err(|e| vec![Diagnostic::error(format!("LLVM ptr_to_int error: {}", e), span)])?;

                            // Get the actual generation from the slot registry
                            let generation = self.get_generation_for_address(address, i32_ty, span)?;

                            self.builder.build_call(
                                snapshot_add_entry,
                                &[snapshot.into(), address.into(), generation.into()],
                                ""
                            ).map_err(|e| vec![Diagnostic::error(format!("LLVM call error: {}", e), span)])?;
                        } else if ptr_val.is_int_value() {
                            // If it's already an int (could be packed pointer), use it directly
                            let int_val = ptr_val.into_int_value();
                            let bit_width = int_val.get_type().get_bit_width();

                            // Handle 128-bit BloodPtr: extract address (bits 64-127) and generation (bits 32-63)
                            if bit_width == 128 {
                                // Extract address from high 64 bits
                                let address = self.builder.build_right_shift(
                                    int_val,
                                    int_val.get_type().const_int(64, false),
                                    false,
                                    "addr_extract"
                                ).map_err(|e| vec![Diagnostic::error(format!("LLVM shift error: {}", e), span)])?;
                                let address = self.builder.build_int_truncate(address, i64_ty, "addr")
                                    .map_err(|e| vec![Diagnostic::error(format!("LLVM truncate error: {}", e), span)])?;

                                // Extract generation from bits 32-63 (next 32 bits after metadata)
                                let gen_shifted = self.builder.build_right_shift(
                                    int_val,
                                    int_val.get_type().const_int(32, false),
                                    false,
                                    "gen_shift"
                                ).map_err(|e| vec![Diagnostic::error(format!("LLVM shift error: {}", e), span)])?;
                                let generation = self.builder.build_int_truncate(gen_shifted, i32_ty, "gen")
                                    .map_err(|e| vec![Diagnostic::error(format!("LLVM truncate error: {}", e), span)])?;

                                self.builder.build_call(
                                    snapshot_add_entry,
                                    &[snapshot.into(), address.into(), generation.into()],
                                    ""
                                ).map_err(|e| vec![Diagnostic::error(format!("LLVM call error: {}", e), span)])?;
                            } else {
                                // 64-bit pointer: extend if needed and look up generation
                                let address = if bit_width < 64 {
                                    self.builder.build_int_z_extend(int_val, i64_ty, "addr")
                                        .map_err(|e| vec![Diagnostic::error(format!("LLVM extend error: {}", e), span)])?
                                } else {
                                    int_val
                                };

                                // Get the actual generation from the slot registry
                                let generation = self.get_generation_for_address(address, i32_ty, span)?;

                                self.builder.build_call(
                                    snapshot_add_entry,
                                    &[snapshot.into(), address.into(), generation.into()],
                                    ""
                                ).map_err(|e| vec![Diagnostic::error(format!("LLVM call error: {}", e), span)])?;
                            }
                        }
                        // For non-pointer types, skip (they don't contain genrefs)
                    }
                }
            }
        }

        // Call blood_perform with effect_id, op_index, args
        let perform_fn = self.module.get_function("blood_perform")
            .ok_or_else(|| vec![Diagnostic::error(
                "Runtime function blood_perform not found", span
            )])?;

        // Pack arguments into array for perform call
        let arg_count = i64_ty.const_int(arg_vals.len() as u64, false);
        let effect_id_val = i64_ty.const_int(effect_id.index as u64, false);
        let op_index_val = i32_ty.const_int(op_index as u64, false);

        // Create args array on stack if needed
        let args_ptr = if arg_vals.is_empty() {
            i64_ty.ptr_type(AddressSpace::default()).const_null()
        } else {
            let array_ty = i64_ty.array_type(arg_vals.len() as u32);
            let args_alloca = self.builder.build_alloca(array_ty, "perform_args")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM alloca error: {}", e), span)])?;

            // Store each argument (converted to i64)
            // Use build_gep with [0, idx] for array element access
            let zero = i64_ty.const_int(0, false);
            for (i, val) in arg_vals.iter().enumerate() {
                let idx = i64_ty.const_int(i as u64, false);
                let gep = unsafe {
                    self.builder.build_gep(args_alloca, &[zero, idx], &format!("arg_{}", i))
                }.map_err(|e| vec![Diagnostic::error(format!("LLVM GEP error: {}", e), span)])?;
                let val_i64 = self.builder.build_int_z_extend(val.into_int_value(), i64_ty, "arg_i64")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM extend error: {}", e), span)])?;
                self.builder.build_store(gep, val_i64)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM store error: {}", e), span)])?;
            }

            self.builder.build_pointer_cast(
                args_alloca,
                i64_ty.ptr_type(AddressSpace::default()),
                "args_ptr"
            ).map_err(|e| vec![Diagnostic::error(format!("LLVM cast error: {}", e), span)])?
        };

        // Create continuation for the handler to resume to.
        // For tail-resumptive handlers, this is ignored (handler just returns).
        // For non-tail-resumptive handlers, this allows the handler to continue
        // executing code after resume() returns.
        let continuation_val = self.create_perform_continuation()?;

        let result = self.builder.build_call(
            perform_fn,
            &[effect_id_val.into(), op_index_val.into(), args_ptr.into(), arg_count.into(), continuation_val.into()],
            "perform_result"
        ).map_err(|e| vec![Diagnostic::error(format!("LLVM call error: {}", e), span)])?;

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
                    span,
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
                            format!("LLVM truncate error: {}", e), span
                        )])?.into()
                } else if dest_bits > 64 {
                    // Zero-extend i64 to larger integer type
                    self.builder.build_int_z_extend(result_i64, dest_int_ty, "perform_ext")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM extend error: {}", e), span
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
                    format!("LLVM int_to_ptr error: {}", e), span
                )])?.into()
            } else {
                // For other types (struct, etc.), use the value directly
                // This may fail if types don't match, but that indicates a bug elsewhere
                result_val
            };

            self.builder.build_store(dest_ptr, converted_result)
                .map_err(|e| vec![Diagnostic::error(format!("LLVM store error: {}", e), span)])?;
        }

        // Validate snapshot on return from effect
        // This checks that all generational references are still valid
        let snapshot_validate = self.module.get_function("blood_snapshot_validate")
            .ok_or_else(|| vec![Diagnostic::error(
                "Runtime function blood_snapshot_validate not found", span
            )])?;

        let validation_result = self.builder.build_call(
            snapshot_validate,
            &[snapshot.into()],
            "validation"
        ).map_err(|e| vec![Diagnostic::error(format!("LLVM call error: {}", e), span)])?
            .try_as_basic_value()
            .left()
            .ok_or_else(|| vec![Diagnostic::error("snapshot_validate returned void", span)])?
            .into_int_value();

        // Destroy snapshot after validation
        let snapshot_destroy = self.module.get_function("blood_snapshot_destroy")
            .ok_or_else(|| vec![Diagnostic::error(
                "Runtime function blood_snapshot_destroy not found", span
            )])?;
        self.builder.build_call(snapshot_destroy, &[snapshot.into()], "")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM call error: {}", e), span)])?;

        // Check if validation failed
        let fn_value = self.current_fn.ok_or_else(|| {
            vec![Diagnostic::error("No current function", span)]
        })?;

        let continue_bb = self.context.append_basic_block(fn_value, "snapshot_valid");
        let stale_bb = self.context.append_basic_block(fn_value, "snapshot_stale");

        let is_valid = self.builder.build_int_compare(
            IntPredicate::EQ,
            validation_result,
            i64_ty.const_int(0, false),
            "is_valid"
        ).map_err(|e| vec![Diagnostic::error(format!("LLVM compare error: {}", e), span)])?;

        self.builder.build_conditional_branch(is_valid, continue_bb, stale_bb)
            .map_err(|e| vec![Diagnostic::error(format!("LLVM branch error: {}", e), span)])?;

        // Stale path - panic
        self.builder.position_at_end(stale_bb);
        let panic_fn = self.module.get_function("blood_stale_reference_panic")
            .ok_or_else(|| vec![Diagnostic::error(
                "Runtime function blood_stale_reference_panic not found", span
            )])?;
        self.builder.build_call(panic_fn, &[i32_ty.const_int(0, false).into(), i32_ty.const_int(0, false).into()], "")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM call error: {}", e), span)])?;
        self.builder.build_unreachable()
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

        // Continue to target on valid path
        self.builder.position_at_end(continue_bb);
        let target_bb = llvm_blocks.get(target).ok_or_else(|| {
            vec![Diagnostic::error("Perform target not found", span)]
        })?;
        self.builder.build_unconditional_branch(*target_bb)
            .map_err(|e| vec![Diagnostic::error(format!("LLVM branch error: {}", e), span)])?;

        Ok(())
    }

    fn compile_resume_terminator(
        &mut self,
        value: Option<&Operand>,
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
        span: crate::span::Span,
    ) -> Result<(), Vec<Diagnostic>> {
        // Resume from effect handler - validate snapshot before returning
        let fn_value = self.current_fn.ok_or_else(|| {
            vec![Diagnostic::error("No current function for Resume", span)]
        })?;

        // Store return value first (if any)
        if let Some(val_op) = value {
            let val = self.compile_mir_operand(val_op, body, escape_results)?;
            if let Some(&ret_ptr) = self.locals.get(&LocalId::new(0)) {
                self.builder.build_store(ret_ptr, val)
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM store error: {}", e), span
                    )])?;
            }
        }

        // Get the snapshot from effect context
        let get_snapshot_fn = self.module.get_function("blood_effect_context_get_snapshot")
            .ok_or_else(|| vec![Diagnostic::error(
                "Runtime function blood_effect_context_get_snapshot not found", span
            )])?;

        let snapshot = self.builder.build_call(get_snapshot_fn, &[], "snapshot")
            .map_err(|e| vec![Diagnostic::error(
                format!("LLVM call error: {}", e), span
            )])?
            .try_as_basic_value()
            .left()
            .ok_or_else(|| vec![Diagnostic::error(
                "blood_effect_context_get_snapshot returned void", span
            )])?;

        // Check if snapshot is null (no validation needed for tail-resumptive handlers)
        let i64_ty = self.context.i64_type();
        let null_snapshot = i64_ty.const_zero();
        let snapshot_is_null = self.builder.build_int_compare(
            inkwell::IntPredicate::EQ,
            snapshot.into_int_value(),
            null_snapshot,
            "snapshot_is_null"
        ).map_err(|e| vec![Diagnostic::error(
            format!("LLVM compare error: {}", e), span
        )])?;

        // Create basic blocks for validation path
        let validate_bb = self.context.append_basic_block(fn_value, "resume_validate");
        let stale_bb = self.context.append_basic_block(fn_value, "resume_stale");
        let ok_bb = self.context.append_basic_block(fn_value, "resume_ok");

        // Branch: if null snapshot, skip validation; otherwise validate
        self.builder.build_conditional_branch(snapshot_is_null, ok_bb, validate_bb)
            .map_err(|e| vec![Diagnostic::error(
                format!("LLVM branch error: {}", e), span
            )])?;

        // Validation block: call blood_snapshot_validate
        self.builder.position_at_end(validate_bb);
        let validate_fn = self.module.get_function("blood_snapshot_validate")
            .ok_or_else(|| vec![Diagnostic::error(
                "Runtime function blood_snapshot_validate not found", span
            )])?;

        let stale_index = self.builder.build_call(validate_fn, &[snapshot.into()], "stale_index")
            .map_err(|e| vec![Diagnostic::error(
                format!("LLVM call error: {}", e), span
            )])?
            .try_as_basic_value()
            .left()
            .ok_or_else(|| vec![Diagnostic::error(
                "blood_snapshot_validate returned void", span
            )])?;

        // Check if validation passed (stale_index == 0)
        let is_valid = self.builder.build_int_compare(
            inkwell::IntPredicate::EQ,
            stale_index.into_int_value(),
            i64_ty.const_zero(),
            "is_valid"
        ).map_err(|e| vec![Diagnostic::error(
            format!("LLVM compare error: {}", e), span
        )])?;

        self.builder.build_conditional_branch(is_valid, ok_bb, stale_bb)
            .map_err(|e| vec![Diagnostic::error(
                format!("LLVM branch error: {}", e), span
            )])?;

        // Stale block: call panic function
        self.builder.position_at_end(stale_bb);
        let panic_fn = self.module.get_function("blood_snapshot_stale_panic")
            .ok_or_else(|| vec![Diagnostic::error(
                "Runtime function blood_snapshot_stale_panic not found", span
            )])?;

        self.builder.build_call(panic_fn, &[snapshot.into(), stale_index.into()], "")
            .map_err(|e| vec![Diagnostic::error(
                format!("LLVM call error: {}", e), span
            )])?;

        self.builder.build_unreachable()
            .map_err(|e| vec![Diagnostic::error(
                format!("LLVM unreachable error: {}", e), span
            )])?;

        // OK block: return from handler
        self.builder.position_at_end(ok_bb);
        self.builder.build_return(None)
            .map_err(|e| vec![Diagnostic::error(
                format!("LLVM return error: {}", e), span
            )])?;

        Ok(())
    }
}
