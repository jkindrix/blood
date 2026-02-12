//! MIR terminator code generation.
//!
//! This module handles compilation of MIR terminators to LLVM IR.
//! Terminators are the final instructions in a basic block that control
//! program flow (branches, calls, returns, etc.).

use std::collections::HashMap;

use inkwell::basic_block::BasicBlock;
use inkwell::intrinsics::Intrinsic;
use inkwell::types::{BasicMetadataTypeEnum, BasicType};
use inkwell::values::{BasicMetadataValueEnum, BasicValue, BasicValueEnum};
use inkwell::{AddressSpace, IntPredicate};

use super::types::MirTypesCodegen;

use crate::diagnostics::Diagnostic;
use crate::hir::LocalId;
use crate::mir::body::MirBody;
use crate::mir::types::{
    BasicBlockId, Terminator, TerminatorKind,
    Operand, Constant, ConstantKind, Place,
};
use crate::mir::EscapeResults;
use crate::{ice, ice_err};

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
                // Check if this is the main function (needs i32 return for C runtime)
                let is_main = self.current_fn_def_id == self.main_fn_def_id && self.main_fn_def_id.is_some();

                if let Some(&ret_ptr) = self.locals.get(&ret_local) {
                    let ret_ty = body.return_type();
                    if !ret_ty.is_unit() {
                        let ret_llvm_ty = self.lower_type(&ret_ty);
                        let ret_val = self.builder.build_load(ret_llvm_ty, ret_ptr, "ret_val")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM load error: {}", e), term.span
                            )])?;
                        // Set proper alignment for return value load.
                        // Use natural alignment — allocas have correct alignment set.
                        let alignment = self.get_type_alignment_for_value(ret_val);
                        if let Some(inst) = ret_val.as_instruction_value() {
                            let _ = inst.set_alignment(alignment);
                        }
                        self.builder.build_return(Some(&ret_val))
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM return error: {}", e), term.span
                            )])?;
                    } else if is_main {
                        // main must return i32 for C runtime - return 0 on success
                        let zero = self.context.i32_type().const_int(0, false);
                        self.builder.build_return(Some(&zero))
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM return error: {}", e), term.span
                            )])?;
                    } else {
                        self.builder.build_return(None)
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM return error: {}", e), term.span
                            )])?;
                    }
                } else if is_main {
                    // main must return i32 for C runtime - return 0 on success
                    let zero = self.context.i32_type().const_int(0, false);
                    self.builder.build_return(Some(&zero))
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM return error: {}", e), term.span
                        )])?;
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
                let ptr = self.compile_mir_place(place, body, escape_results)?;
                let store_inst = self.builder.build_store(ptr, new_val)
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM store error: {}", e), term.span
                    )])?;
                // Set proper alignment for DropAndReplace store.
                // Use natural alignment — allocas have correct alignment set.
                let alignment = self.get_type_alignment_for_value(new_val);
                let _ = store_inst.set_alignment(alignment);

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
                let _ptr_val = self.compile_mir_place(ptr, body, escape_results)?;

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
    // Compiler-internal: decomposing would reduce clarity
    #[allow(clippy::too_many_arguments)]
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
        // Get the expected parameter types from the function being called
        let expected_param_types: Vec<crate::hir::Type> = match func {
            Operand::Constant(Constant { kind: ConstantKind::FnDef(_def_id), ty }) => {
                // Get function signature from type
                if let crate::hir::TypeKind::Fn { params, .. } = ty.kind() {
                    params.clone()
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        };

        // Compile arguments, converting closures to fn pointers where needed
        let mut arg_vals: Vec<BasicValueEnum> = Vec::with_capacity(args.len());
        for (i, arg) in args.iter().enumerate() {
            let val = self.compile_mir_operand(arg, body, escape_results)?;

            // Check if we need to convert a closure to a function pointer
            let arg_ty = self.get_operand_type(arg, body);
            let expected_ty = expected_param_types.get(i);

            let converted_val = match (arg_ty.kind(), expected_ty.map(|t| t.kind())) {
                (crate::hir::TypeKind::Closure { .. }, Some(crate::hir::TypeKind::Fn { .. })) => {
                    // Closure to Fn conversion: both are now { fn_ptr, env_ptr } fat pointers
                    // Just pass the closure struct directly - no extraction needed
                    val
                }
                _ => val,
            };

            arg_vals.push(converted_val);
        }

        let arg_metas: Vec<_> = arg_vals.iter().map(|v| (*v).into()).collect();

        // Helper to extract closure DefId from a place's type
        let get_closure_def_id = |place: &Place, body: &MirBody| -> Option<crate::hir::DefId> {
            let local = body.locals.get(place.local_unchecked().index() as usize)?;
            match local.ty.kind() {
                crate::hir::TypeKind::Closure { def_id, .. } => Some(*def_id),
                _ => None,
            }
        };

        // Handle different function operand types
        let call_result = match func {
            Operand::Constant(Constant { kind: ConstantKind::FnDef(def_id), ty }) => {
                // Direct function call
                if let Some(&fn_value) = self.functions.get(def_id) {
                    // Convert arguments to match parameter types
                    // This handles cases like passing a struct value to a function expecting &T
                    let fn_type = fn_value.get_type();
                    let param_types = fn_type.get_param_types();
                    let mut converted_args: Vec<BasicMetadataValueEnum> = Vec::with_capacity(arg_vals.len());

                    for (i, val) in arg_vals.iter().enumerate() {
                        let converted = if let Some(param_type) = param_types.get(i) {
                            if val.is_int_value() && param_type.is_int_type() {
                                let val_int = val.into_int_value();
                                let param_int_type = param_type.into_int_type();
                                // Zero-extend if arg is smaller (e.g., i1 -> i32)
                                if val_int.get_type().get_bit_width() < param_int_type.get_bit_width() {
                                    self.builder
                                        .build_int_z_extend(val_int, param_int_type, "zext")
                                        .map(|v| v.into())
                                        .unwrap_or((*val).into())
                                } else {
                                    (*val).into()
                                }
                            } else if param_type.is_pointer_type() && val.is_struct_value() {
                                // Parameter expects a pointer but we have a struct value
                                // Allocate on stack and pass pointer (for &self method semantics)
                                let struct_val = val.into_struct_value();
                                let alloca = self.builder
                                    .build_alloca(struct_val.get_type(), "arg_tmp")
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM alloca error: {}", e), span
                                    )])?;
                                // Set alignment on alloca (request full alignment)
                                let natural_alignment = self.get_type_alignment_for_value(struct_val.into());
                                if let Some(inst) = alloca.as_instruction() {
                                    let _ = inst.set_alignment(natural_alignment);
                                }
                                let store_inst = self.builder.build_store(alloca, struct_val)
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM store error: {}", e), span
                                    )])?;
                                // Use natural alignment — alloca has correct alignment set.
                                let _ = store_inst.set_alignment(natural_alignment);
                                // Bitcast to expected pointer type if needed
                                let expected_ptr_type = param_type.into_pointer_type();
                                if alloca.get_type() != expected_ptr_type {
                                    self.builder.build_pointer_cast(alloca, expected_ptr_type, "ptr_cast")
                                        .map(|p| p.into())
                                        .unwrap_or(alloca.into())
                                } else {
                                    alloca.into()
                                }
                            } else if param_type.is_pointer_type() && val.is_pointer_value() {
                                // Parameter expects pointer and we have pointer - cast if needed
                                let ptr_val = val.into_pointer_value();
                                let expected_ptr_type = param_type.into_pointer_type();
                                if ptr_val.get_type() != expected_ptr_type {
                                    self.builder.build_pointer_cast(ptr_val, expected_ptr_type, "ptr_cast")
                                        .map(|p| p.into())
                                        .unwrap_or((*val).into())
                                } else {
                                    (*val).into()
                                }
                            } else if param_type.is_pointer_type() && val.is_int_value() {
                                // Parameter expects pointer but we have int - allocate on stack
                                let int_val = val.into_int_value();
                                let alloca = self.builder
                                    .build_alloca(int_val.get_type(), "int_arg_tmp")
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM alloca error: {}", e), span
                                    )])?;
                                // Set alignment on alloca
                                let alignment = self.get_type_alignment_for_value(int_val.into());
                                if let Some(inst) = alloca.as_instruction() {
                                    let _ = inst.set_alignment(alignment);
                                }
                                let store_inst = self.builder.build_store(alloca, int_val)
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM store error: {}", e), span
                                    )])?;
                                let _ = store_inst.set_alignment(alignment);
                                let expected_ptr_type = param_type.into_pointer_type();
                                if alloca.get_type() != expected_ptr_type {
                                    self.builder.build_pointer_cast(alloca, expected_ptr_type, "ptr_cast")
                                        .map(|p| p.into())
                                        .unwrap_or(alloca.into())
                                } else {
                                    alloca.into()
                                }
                            } else if param_type.is_pointer_type() && val.is_array_value() {
                                // Parameter expects pointer but we have array value
                                let array_val = val.into_array_value();
                                let alloca = self.builder
                                    .build_alloca(array_val.get_type(), "array_arg_tmp")
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM alloca error: {}", e), span
                                    )])?;
                                // Set alignment on alloca (request full alignment)
                                let natural_alignment = self.get_type_alignment_for_value(array_val.into());
                                if let Some(inst) = alloca.as_instruction() {
                                    let _ = inst.set_alignment(natural_alignment);
                                }
                                let store_inst = self.builder.build_store(alloca, array_val)
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM store error: {}", e), span
                                    )])?;
                                // Use natural alignment — alloca has correct alignment set.
                                let _ = store_inst.set_alignment(natural_alignment);
                                let expected_ptr_type = param_type.into_pointer_type();
                                if alloca.get_type() != expected_ptr_type {
                                    self.builder.build_pointer_cast(alloca, expected_ptr_type, "ptr_cast")
                                        .map(|p| p.into())
                                        .unwrap_or(alloca.into())
                                } else {
                                    alloca.into()
                                }
                            } else {
                                (*val).into()
                            }
                        } else {
                            (*val).into()
                        };
                        converted_args.push(converted);
                    }

                    self.builder.build_call(fn_value, &converted_args, "call_result")
                        .map_err(|e| vec![Diagnostic::error(
                            format!("LLVM call error: {}", e), span
                        )])?
                } else if let Some(builtin_name) = self.builtin_fns.get(def_id) {
                    // Check for pointer intrinsics first - these compile to direct load/store
                    // This avoids function call overhead for every array access
                    let is_ptr_read = builtin_name.starts_with("ptr_read_");
                    let is_ptr_write = builtin_name.starts_with("ptr_write_");

                    if is_ptr_read {
                        // ptr_read_TYPE(ptr: u64) -> TYPE
                        // Compile to direct LLVM load instruction
                        let ptr_arg = arg_vals.first().ok_or_else(|| {
                            vec![Diagnostic::error(
                                "ptr_read requires a pointer argument".to_string(), span
                            )]
                        })?.into_int_value();

                        // Determine the element type based on the suffix
                        let elem_type: inkwell::types::BasicTypeEnum = match builtin_name.as_str() {
                            "ptr_read_i32" => self.context.i32_type().into(),
                            "ptr_read_i64" => self.context.i64_type().into(),
                            "ptr_read_u64" => self.context.i64_type().into(), // u64 represented as i64
                            "ptr_read_u8" => self.context.i8_type().into(),
                            "ptr_read_f64" => self.context.f64_type().into(),
                            _ => return Err(vec![Diagnostic::error(
                                format!("Unknown ptr_read variant: {}", builtin_name), span
                            )]),
                        };

                        // Cast integer to pointer: inttoptr i64 %ptr to ptr
                        let ptr_type = self.context.ptr_type(AddressSpace::default());
                        let typed_ptr = self.builder.build_int_to_ptr(ptr_arg, ptr_type, "ptr_cast")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM int_to_ptr error: {}", e), span
                            )])?;

                        // Load value from pointer
                        let loaded_val = self.builder.build_load(elem_type, typed_ptr, "ptr_load")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM load error: {}", e), span
                            )])?;

                        // Store result to destination
                        let dest_ptr = self.compile_mir_place(destination, body, escape_results)?;
                        let store_inst = self.builder.build_store(dest_ptr, loaded_val)
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM store error: {}", e), span
                            )])?;
                        // Set proper alignment for ptr_read result store.
                        // Use natural alignment — memory is properly aligned.
                        let alignment = self.get_type_alignment_for_value(loaded_val);
                        let _ = store_inst.set_alignment(alignment);

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
                        return Ok(());
                    }

                    if is_ptr_write {
                        // ptr_write_TYPE(ptr: u64, value: TYPE) -> ()
                        // Compile to direct LLVM store instruction
                        if arg_vals.len() < 2 {
                            return Err(vec![Diagnostic::error(
                                "ptr_write requires pointer and value arguments".to_string(), span
                            )]);
                        }
                        let ptr_arg = arg_vals[0].into_int_value();
                        let value_arg = arg_vals[1];

                        // Validate the ptr_write variant (the element type is implicit in the value)
                        let _elem_type: inkwell::types::BasicTypeEnum = match builtin_name.as_str() {
                            "ptr_write_i32" => self.context.i32_type().into(),
                            "ptr_write_i64" => self.context.i64_type().into(),
                            "ptr_write_u64" => self.context.i64_type().into(), // u64 represented as i64
                            "ptr_write_u8" => self.context.i8_type().into(),
                            "ptr_write_f64" => self.context.f64_type().into(),
                            _ => return Err(vec![Diagnostic::error(
                                format!("Unknown ptr_write variant: {}", builtin_name), span
                            )]),
                        };

                        // Cast integer to pointer: inttoptr i64 %ptr to ptr
                        let ptr_type = self.context.ptr_type(AddressSpace::default());
                        let typed_ptr = self.builder.build_int_to_ptr(ptr_arg, ptr_type, "ptr_cast")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM int_to_ptr error: {}", e), span
                            )])?;

                        // Store value to pointer
                        self.builder.build_store(typed_ptr, value_arg)
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM store error: {}", e), span
                            )])?;

                        // Branch to continuation (ptr_write returns void, no destination store needed)
                        if let Some(target_bb_id) = target {
                            let target_bb = llvm_blocks.get(target_bb_id).ok_or_else(|| {
                                vec![Diagnostic::error("Call target block not found", span)]
                            })?;
                            self.builder.build_unconditional_branch(*target_bb)
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM branch error: {}", e), span
                                )])?;
                        }
                        return Ok(());
                    }

                    // Check for math intrinsics - these compile to hardware instructions
                    let intrinsic_name = match builtin_name.as_str() {
                        "sqrt" => Some("llvm.sqrt.f64"),
                        "sqrt_f32" => Some("llvm.sqrt.f32"),
                        "abs" => Some("llvm.fabs.f64"),
                        "abs_f32" => Some("llvm.fabs.f32"),
                        "floor" => Some("llvm.floor.f64"),
                        "ceil" => Some("llvm.ceil.f64"),
                        "sin" => Some("llvm.sin.f64"),
                        "cos" => Some("llvm.cos.f64"),
                        "exp" => Some("llvm.exp.f64"),
                        "log" => Some("llvm.log.f64"),
                        "pow" => Some("llvm.pow.f64"),
                        _ => None,
                    };

                    if let Some(intrinsic_name) = intrinsic_name {
                        // Use LLVM intrinsic for math functions
                        let intrinsic = Intrinsic::find(intrinsic_name).ok_or_else(|| {
                            vec![Diagnostic::error(
                                format!("LLVM intrinsic {} not found", intrinsic_name), span
                            )]
                        })?;

                        // Get the argument type for the intrinsic declaration
                        let arg_type = if let Some(arg) = arg_vals.first() {
                            arg.get_type()
                        } else {
                            return Err(vec![Diagnostic::error(
                                format!("Math intrinsic {} requires at least one argument", builtin_name), span
                            )]);
                        };

                        let intrinsic_fn = intrinsic
                            .get_declaration(self.module, &[arg_type])
                            .ok_or_else(|| {
                                vec![Diagnostic::error(
                                    format!("Could not get declaration for {} intrinsic", intrinsic_name), span
                                )]
                            })?;

                        self.builder.build_call(intrinsic_fn, &arg_metas, "math_intrinsic")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM intrinsic call error: {}", e), span
                            )])?
                    } else if let Some(fn_value) = self.module.get_function(builtin_name) {
                        // Builtin function call - lookup runtime function by name
                        // Convert args for C runtime (e.g., i1 -> i32 for print_bool)
                        let fn_type = fn_value.get_type();
                        let param_types = fn_type.get_param_types();
                        let mut converted_args: Vec<BasicMetadataValueEnum> = Vec::with_capacity(arg_vals.len());

                        // Track output buffer allocation for functions that return via out pointer
                        // Tuple: (pointer to alloca, type stored in the alloca)
                        let mut output_buffer_alloca: Option<(inkwell::values::PointerValue, inkwell::types::BasicTypeEnum)> = None;

                        // Check for special built-in methods that need additional arguments
                        // Note: vec_new and vec_with_capacity are handled separately below
                        // because they get elem_size from the destination type, not from args
                        let needs_elem_size = matches!(
                            builtin_name.as_str(),
                            "box_new" | "box_into_inner" |
                            "vec_push" | "vec_pop" | "vec_contains" |
                            "vec_reverse" | "vec_get" | "vec_get_ptr" | "vec_free" |
                            "vec_first" | "vec_last" |
                            "option_unwrap" | "option_try" | "option_expect" | "option_unwrap_or" |
                            "option_ok_or" | "option_and" | "option_or" | "option_xor" |
                            "option_as_ref" | "option_as_mut" | "option_take" | "option_replace" |
                            "result_unwrap" | "result_unwrap_err" | "result_try" |
                            "result_ok" | "result_err" | "result_expect" | "result_expect_err" |
                            "result_unwrap_or" | "result_and" | "result_or" |
                            "result_as_ref" | "result_as_mut"
                        );

                        for (i, val) in arg_vals.iter().enumerate() {
                            let converted = if let Some(param_type) = param_types.get(i) {
                                if val.is_int_value() && param_type.is_int_type() {
                                    let val_int = val.into_int_value();
                                    let param_int_type = param_type.into_int_type();
                                    // Zero-extend if arg is smaller (e.g., i1 -> i32)
                                    if val_int.get_type().get_bit_width() < param_int_type.get_bit_width() {
                                        self.builder
                                            .build_int_z_extend(val_int, param_int_type, "zext")
                                            .map(|v| v.into())
                                            .unwrap_or((*val).into())
                                    } else {
                                        (*val).into()
                                    }
                                } else if param_type.is_pointer_type() && val.is_struct_value() {
                                    // Parameter expects a pointer but we have a struct value
                                    // Allocate on stack and pass pointer (for &self method semantics)
                                    let struct_val = val.into_struct_value();
                                    let alloca = self.builder
                                        .build_alloca(struct_val.get_type(), "method_self_tmp")
                                        .map_err(|e| vec![Diagnostic::error(
                                            format!("LLVM alloca error: {}", e), span
                                        )])?;
                                    // Set alignment on alloca (request full alignment)
                                    let natural_alignment = self.get_type_alignment_for_value(struct_val.into());
                                    if let Some(inst) = alloca.as_instruction() {
                                        let _ = inst.set_alignment(natural_alignment);
                                    }
                                    let store_inst = self.builder.build_store(alloca, struct_val)
                                        .map_err(|e| vec![Diagnostic::error(
                                            format!("LLVM store error: {}", e), span
                                        )])?;
                                    // Use natural alignment — alloca has correct alignment set.
                                    let _ = store_inst.set_alignment(natural_alignment);
                                    // Bitcast to expected pointer type (e.g., i8* for void*)
                                    let expected_ptr_type = param_type.into_pointer_type();
                                    if alloca.get_type() != expected_ptr_type {
                                        self.builder.build_pointer_cast(alloca, expected_ptr_type, "ptr_cast")
                                            .map(|p| p.into())
                                            .unwrap_or(alloca.into())
                                    } else {
                                        alloca.into()
                                    }
                                } else if param_type.is_pointer_type() && val.is_pointer_value() {
                                    // Parameter expects pointer and we have pointer - cast if needed
                                    let ptr_val = val.into_pointer_value();
                                    let expected_ptr_type = param_type.into_pointer_type();
                                    if ptr_val.get_type() != expected_ptr_type {
                                        self.builder.build_pointer_cast(ptr_val, expected_ptr_type, "ptr_cast")
                                            .map(|p| p.into())
                                            .unwrap_or((*val).into())
                                    } else {
                                        (*val).into()
                                    }
                                } else if param_type.is_pointer_type() && val.is_int_value() {
                                    // Parameter expects pointer but we have int - allocate on stack and pass pointer
                                    // This handles Box::new(42) where we need to pass a pointer to the int value
                                    let int_val = val.into_int_value();
                                    let alloca = self.builder
                                        .build_alloca(int_val.get_type(), "int_arg_tmp")
                                        .map_err(|e| vec![Diagnostic::error(
                                            format!("LLVM alloca error: {}", e), span
                                        )])?;
                                    // Set alignment on alloca
                                    let alignment = self.get_type_alignment_for_value(int_val.into());
                                    if let Some(inst) = alloca.as_instruction() {
                                        let _ = inst.set_alignment(alignment);
                                    }
                                    let store_inst = self.builder.build_store(alloca, int_val)
                                        .map_err(|e| vec![Diagnostic::error(
                                            format!("LLVM store error: {}", e), span
                                        )])?;
                                    let _ = store_inst.set_alignment(alignment);
                                    // Bitcast to expected pointer type (e.g., i8* for void*)
                                    let expected_ptr_type = param_type.into_pointer_type();
                                    if alloca.get_type() != expected_ptr_type {
                                        self.builder.build_pointer_cast(alloca, expected_ptr_type, "ptr_cast")
                                            .map(|p| p.into())
                                            .unwrap_or(alloca.into())
                                    } else {
                                        alloca.into()
                                    }
                                } else {
                                    (*val).into()
                                }
                            } else {
                                (*val).into()
                            };
                            converted_args.push(converted);
                        }

                        // Add element size argument for methods that need it
                        if needs_elem_size && !args.is_empty() {
                            // Determine element size from the type.
                            //
                            // NOTE ON FALLBACK VALUES: Throughout this match, we use `8` as a fallback
                            // size when type information cannot be extracted (e.g., when a generic type
                            // doesn't have type arguments). This is pointer-sized (64-bit) and is a
                            // conservative default that:
                            // - Will work correctly for pointer-sized types
                            // - May allocate extra space for smaller types (safe but wasteful)
                            // - Could underallocate for types larger than 8 bytes (potential bug)
                            //
                            // These fallbacks should only trigger on malformed types or ICE conditions.
                            // In normal compilation, type information should always be available.
                            // The catch-all arm logs an ICE if an unhandled builtin is encountered.
                            let elem_size: u64 = match builtin_name.as_str() {
                                "box_new" => {
                                    // For box_new, the first (and only) arg is the value to box
                                    let value_ty = self.get_operand_type(&args[0], body);
                                    let llvm_ty = self.lower_type(&value_ty);
                                    self.get_type_size_in_bytes(llvm_ty)
                                },
                                "box_into_inner" => {
                                    // For box_into_inner, first arg is Box<T>, extract T's size
                                    let box_ty = self.get_operand_type(&args[0], body);
                                    // Get T from Box<T>
                                    if let crate::hir::TypeKind::Adt { args: type_args, .. } = box_ty.kind() {
                                        if let Some(inner_ty) = type_args.first() {
                                            let llvm_ty = self.lower_type(inner_ty);
                                            self.get_type_size_in_bytes(llvm_ty)
                                        } else {
                                            8
                                        }
                                    } else {
                                        8
                                    }
                                },
                                "vec_push" | "vec_contains" => {
                                    // Second arg is the element
                                    // CRITICAL: Use the HIR type to compute elem_size, which will
                                    // be consistent with what GEP uses when reading elements back.
                                    // We must use lower_type() here to match the GEP path in place.rs
                                    // which also uses lower_type() for the element type.
                                    let size = if args.len() >= 2 {
                                        let elem_ty = self.get_operand_type(&args[1], body);
                                        let llvm_ty = self.lower_type(&elem_ty);
                                        let size = self.get_type_size_in_bytes(llvm_ty);

                                        // Debug: print size calculation with full LLVM type string
                                        if std::env::var("BLOOD_DEBUG_VEC_SIZE").is_ok() {
                                            let llvm_str = llvm_ty.print_to_string().to_string();
                                            eprintln!("[vec_push size] HIR: {:?}, LLVM: {}, size: {}",
                                                elem_ty, llvm_str, size);
                                        }
                                        size
                                    } else {
                                        8 // Default size
                                    };
                                    size
                                },
                                "vec_pop" | "vec_reverse" | "vec_get" | "vec_get_ptr" | "vec_free" |
                                "vec_first" | "vec_last" => {
                                    // First arg is Vec<T>, extract T's size from the type
                                    let vec_ty = self.get_operand_type(&args[0], body);
                                    // Strip reference if present
                                    let inner_ty = match vec_ty.kind() {
                                        crate::hir::TypeKind::Ref { inner, .. } => inner.clone(),
                                        _ => vec_ty.clone(),
                                    };
                                    // Get element type from Vec<T>
                                    if let crate::hir::TypeKind::Adt { args: type_args, .. } = inner_ty.kind() {
                                        if let Some(elem_ty) = type_args.first() {
                                            let llvm_ty = self.lower_type(elem_ty);
                                            self.get_type_size_in_bytes(llvm_ty)
                                        } else {
                                            8
                                        }
                                    } else {
                                        8
                                    }
                                },
                                "option_unwrap" | "option_try" | "option_expect" | "option_unwrap_or" |
                                "option_ok_or" | "option_as_ref" | "option_as_mut" | "option_take" => {
                                    // First arg is Option<T>, extract T's size from the type
                                    let opt_ty = self.get_operand_type(&args[0], body);
                                    // Strip reference if present
                                    let inner_ty = match opt_ty.kind() {
                                        crate::hir::TypeKind::Ref { inner, .. } => inner.clone(),
                                        _ => opt_ty.clone(),
                                    };
                                    // Get payload type from Option<T>
                                    if let crate::hir::TypeKind::Adt { args: type_args, .. } = inner_ty.kind() {
                                        if let Some(payload_ty) = type_args.first() {
                                            let llvm_ty = self.lower_type(payload_ty);
                                            let size = self.get_type_size_in_bytes(llvm_ty);
                                            let align = self.get_type_alignment_for_size(llvm_ty);
                                            (align << 32) | size
                                        } else {
                                            8
                                        }
                                    } else {
                                        8
                                    }
                                },
                                "option_and" => {
                                    // Second arg is Option<U>, extract U's size (other_size)
                                    if args.len() >= 2 {
                                        let other_ty = self.get_operand_type(&args[1], body);
                                        let inner_ty = match other_ty.kind() {
                                            crate::hir::TypeKind::Ref { inner, .. } => inner.clone(),
                                            _ => other_ty.clone(),
                                        };
                                        // Get the whole Option<U> size for other_size
                                        let llvm_ty = self.lower_type(&inner_ty);
                                        self.get_type_size_in_bytes(llvm_ty)
                                    } else {
                                        8
                                    }
                                },
                                "option_or" | "option_xor" => {
                                    // First arg is Option<T>, extract the whole Option<T> size
                                    let opt_ty = self.get_operand_type(&args[0], body);
                                    let inner_ty = match opt_ty.kind() {
                                        crate::hir::TypeKind::Ref { inner, .. } => inner.clone(),
                                        _ => opt_ty.clone(),
                                    };
                                    let llvm_ty = self.lower_type(&inner_ty);
                                    self.get_type_size_in_bytes(llvm_ty)
                                },
                                "option_replace" => {
                                    // First arg is &mut Option<T>, second is T value
                                    // Need T's size (payload_size)
                                    let opt_ty = self.get_operand_type(&args[0], body);
                                    let inner_ty = match opt_ty.kind() {
                                        crate::hir::TypeKind::Ref { inner, .. } => inner.clone(),
                                        _ => opt_ty.clone(),
                                    };
                                    if let crate::hir::TypeKind::Adt { args: type_args, .. } = inner_ty.kind() {
                                        if let Some(payload_ty) = type_args.first() {
                                            let llvm_ty = self.lower_type(payload_ty);
                                            let size = self.get_type_size_in_bytes(llvm_ty);
                                            let align = self.get_type_alignment_for_size(llvm_ty);
                                            (align << 32) | size
                                        } else {
                                            8
                                        }
                                    } else {
                                        8
                                    }
                                },
                                "result_unwrap" | "result_try" => {
                                    // First arg is Result<T, E>, extract T's size (Ok payload)
                                    let res_ty = self.get_operand_type(&args[0], body);
                                    let inner_ty = match res_ty.kind() {
                                        crate::hir::TypeKind::Ref { inner, .. } => inner.clone(),
                                        _ => res_ty.clone(),
                                    };
                                    // Get T (first type arg) from Result<T, E>
                                    if let crate::hir::TypeKind::Adt { args: type_args, .. } = inner_ty.kind() {
                                        if let Some(ok_ty) = type_args.first() {
                                            let llvm_ty = self.lower_type(ok_ty);
                                            let size = self.get_type_size_in_bytes(llvm_ty);
                                            let align = self.get_type_alignment_for_size(llvm_ty);
                                            (align << 32) | size
                                        } else {
                                            8
                                        }
                                    } else {
                                        8
                                    }
                                },
                                "result_unwrap_err" | "result_expect_err" => {
                                    // First arg is Result<T, E>, extract E's size (Err payload)
                                    let res_ty = self.get_operand_type(&args[0], body);
                                    let inner_ty = match res_ty.kind() {
                                        crate::hir::TypeKind::Ref { inner, .. } => inner.clone(),
                                        _ => res_ty.clone(),
                                    };
                                    // Get E (second type arg) from Result<T, E>
                                    if let crate::hir::TypeKind::Adt { args: type_args, .. } = inner_ty.kind() {
                                        if type_args.len() >= 2 {
                                            let llvm_ty = self.lower_type(&type_args[1]);
                                            let size = self.get_type_size_in_bytes(llvm_ty);
                                            let align = self.get_type_alignment_for_size(llvm_ty);
                                            (align << 32) | size
                                        } else {
                                            8
                                        }
                                    } else {
                                        8
                                    }
                                },
                                "result_err" => {
                                    // For result_err, we need the E's size (error type)
                                    let res_ty = self.get_operand_type(&args[0], body);
                                    let inner_ty = match res_ty.kind() {
                                        crate::hir::TypeKind::Ref { inner, .. } => inner.clone(),
                                        _ => res_ty.clone(),
                                    };
                                    if let crate::hir::TypeKind::Adt { args: type_args, .. } = inner_ty.kind() {
                                        if type_args.len() >= 2 {
                                            let llvm_ty = self.lower_type(&type_args[1]);
                                            let size = self.get_type_size_in_bytes(llvm_ty);
                                            let align = self.get_type_alignment_for_size(llvm_ty);
                                            (align << 32) | size
                                        } else {
                                            8
                                        }
                                    } else {
                                        8
                                    }
                                },
                                "result_ok" | "result_expect" | "result_unwrap_or" => {
                                    // First arg is Result<T, E>, extract T's size (Ok payload)
                                    let res_ty = self.get_operand_type(&args[0], body);
                                    let inner_ty = match res_ty.kind() {
                                        crate::hir::TypeKind::Ref { inner, .. } => inner.clone(),
                                        _ => res_ty.clone(),
                                    };
                                    if let crate::hir::TypeKind::Adt { args: type_args, .. } = inner_ty.kind() {
                                        if let Some(ok_ty) = type_args.first() {
                                            let llvm_ty = self.lower_type(ok_ty);
                                            let size = self.get_type_size_in_bytes(llvm_ty);
                                            let align = self.get_type_alignment_for_size(llvm_ty);
                                            (align << 32) | size
                                        } else {
                                            8
                                        }
                                    } else {
                                        8
                                    }
                                },
                                "result_and" => {
                                    // result_and(res, other, other_size, err_size, out)
                                    // Second arg is Result<U, E>, need other_size (whole Result<U, E> size)
                                    if args.len() >= 2 {
                                        let other_ty = self.get_operand_type(&args[1], body);
                                        let inner_ty = match other_ty.kind() {
                                            crate::hir::TypeKind::Ref { inner, .. } => inner.clone(),
                                            _ => other_ty.clone(),
                                        };
                                        let llvm_ty = self.lower_type(&inner_ty);
                                        self.get_type_size_in_bytes(llvm_ty)
                                    } else {
                                        8
                                    }
                                },
                                "result_or" => {
                                    // result_or(res, other, ok_size, other_size, out)
                                    // First arg is Result<T, E>, need T's size (ok_size)
                                    let res_ty = self.get_operand_type(&args[0], body);
                                    let inner_ty = match res_ty.kind() {
                                        crate::hir::TypeKind::Ref { inner, .. } => inner.clone(),
                                        _ => res_ty.clone(),
                                    };
                                    if let crate::hir::TypeKind::Adt { args: type_args, .. } = inner_ty.kind() {
                                        if let Some(ok_ty) = type_args.first() {
                                            let llvm_ty = self.lower_type(ok_ty);
                                            let size = self.get_type_size_in_bytes(llvm_ty);
                                            let align = self.get_type_alignment_for_size(llvm_ty);
                                            (align << 32) | size
                                        } else {
                                            8
                                        }
                                    } else {
                                        8
                                    }
                                },
                                "result_as_ref" | "result_as_mut" => {
                                    // result_as_ref/as_mut(res, ok_size, err_size, out)
                                    // First arg is Result<T, E>, need T's size (ok_size)
                                    let res_ty = self.get_operand_type(&args[0], body);
                                    let inner_ty = match res_ty.kind() {
                                        crate::hir::TypeKind::Ref { inner, .. } => inner.clone(),
                                        _ => res_ty.clone(),
                                    };
                                    if let crate::hir::TypeKind::Adt { args: type_args, .. } = inner_ty.kind() {
                                        if let Some(ok_ty) = type_args.first() {
                                            let llvm_ty = self.lower_type(ok_ty);
                                            let size = self.get_type_size_in_bytes(llvm_ty);
                                            let align = self.get_type_alignment_for_size(llvm_ty);
                                            (align << 32) | size
                                        } else {
                                            8
                                        }
                                    } else {
                                        8
                                    }
                                },
                                other => {
                                    // This is a defensive fallback - all builtins in needs_elem_size should be
                                    // explicitly handled above. If we reach this branch, it indicates a new
                                    // builtin was added to needs_elem_size but not handled in the match.
                                    // Using pointer-size (8 bytes) as conservative default, but log an ICE.
                                    ice!(
                                        "Unhandled builtin in element size calculation. Add explicit handling in terminator.rs.";
                                        "builtin" => other,
                                        "default_size" => 8
                                    );
                                    8
                                },
                            };

                            let size_val = self.context.i64_type().const_int(elem_size, false);
                            converted_args.push(size_val.into());

                            // For Option/Result unwrap/try methods, Vec first/last/get, and Box into_inner, we need an output buffer
                            if matches!(builtin_name.as_str(), "option_unwrap" | "option_try" |
                                "option_expect" | "option_unwrap_or" | "option_ok_or" |
                                "option_and" | "option_or" | "option_xor" |
                                "option_as_ref" | "option_as_mut" | "option_take" | "option_replace" |
                                "result_unwrap" | "result_unwrap_err" | "result_try" |
                                "result_ok" | "result_err" | "result_expect" | "result_expect_err" |
                                "result_unwrap_or" | "result_and" | "result_or" |
                                "result_as_ref" | "result_as_mut" |
                                "vec_first" | "vec_last" | "vec_pop" | "vec_get" |
                                "box_into_inner") {
                                // Get the type to determine the output buffer type
                                let container_ty = self.get_operand_type(&args[0], body);
                                let inner_ty = match container_ty.kind() {
                                    crate::hir::TypeKind::Ref { inner, .. } => inner.clone(),
                                    _ => container_ty.clone(),
                                };

                                let i8_ptr_type = self.context.ptr_type(AddressSpace::default());

                                // Handle expect/expect_err string message argument
                                if matches!(builtin_name.as_str(), "option_expect" | "result_expect" | "result_expect_err") {
                                    // Second argument is &str (fat pointer: ptr + len)
                                    // Already in converted_args, but we need to extract ptr and len
                                    if args.len() >= 2 {
                                        let str_val = &arg_vals[1];
                                        if str_val.is_struct_value() {
                                            // Fat pointer struct {ptr, len}
                                            let struct_val = str_val.into_struct_value();
                                            let str_ptr = self.builder
                                                .build_extract_value(struct_val, 0, "str_ptr")
                                                .map_err(|e| vec![Diagnostic::error(
                                                    format!("LLVM extract error: {}", e), span
                                                )])?;
                                            let str_len = self.builder
                                                .build_extract_value(struct_val, 1, "str_len")
                                                .map_err(|e| vec![Diagnostic::error(
                                                    format!("LLVM extract error: {}", e), span
                                                )])?;
                                            // Cast ptr to i8*
                                            let str_ptr_cast = if str_ptr.is_pointer_value() {
                                                self.builder.build_pointer_cast(
                                                    str_ptr.into_pointer_value(),
                                                    i8_ptr_type,
                                                    "msg_ptr_cast"
                                                ).map(|p| p.into()).unwrap_or(str_ptr)
                                            } else {
                                                str_ptr
                                            };
                                            converted_args.push(str_ptr_cast.into());
                                            converted_args.push(str_len.into());
                                        } else {
                                            // Fallback: null ptr and 0 len
                                            let null_ptr = i8_ptr_type.const_null();
                                            converted_args.push(null_ptr.into());
                                            converted_args.push(self.context.i64_type().const_int(0, false).into());
                                        }
                                    }
                                }

                                // Handle unwrap_or default value argument
                                if matches!(builtin_name.as_str(), "option_unwrap_or" | "result_unwrap_or") {
                                    // Second argument is the default value T
                                    if args.len() >= 2 {
                                        let default_val = &arg_vals[1];
                                        // Allocate on stack and pass pointer
                                        if default_val.is_int_value() {
                                            let int_val = default_val.into_int_value();
                                            let alloca = self.builder
                                                .build_alloca(int_val.get_type(), "default_tmp")
                                                .map_err(|e| vec![Diagnostic::error(
                                                    format!("LLVM alloca error: {}", e), span
                                                )])?;
                                            self.builder.build_store(alloca, int_val)
                                                .map_err(|e| vec![Diagnostic::error(
                                                    format!("LLVM store error: {}", e), span
                                                )])?;
                                            let ptr = self.builder
                                                .build_pointer_cast(alloca, i8_ptr_type, "default_ptr_cast")
                                                .map_err(|e| vec![Diagnostic::error(
                                                    format!("LLVM pointer cast error: {}", e), span
                                                )])?;
                                            converted_args.push(ptr.into());
                                        } else if default_val.is_struct_value() {
                                            let struct_val = default_val.into_struct_value();
                                            let alloca = self.builder
                                                .build_alloca(struct_val.get_type(), "default_tmp")
                                                .map_err(|e| vec![Diagnostic::error(
                                                    format!("LLVM alloca error: {}", e), span
                                                )])?;
                                            self.builder.build_store(alloca, struct_val)
                                                .map_err(|e| vec![Diagnostic::error(
                                                    format!("LLVM store error: {}", e), span
                                                )])?;
                                            let ptr = self.builder
                                                .build_pointer_cast(alloca, i8_ptr_type, "default_ptr_cast")
                                                .map_err(|e| vec![Diagnostic::error(
                                                    format!("LLVM pointer cast error: {}", e), span
                                                )])?;
                                            converted_args.push(ptr.into());
                                        } else if default_val.is_pointer_value() {
                                            let ptr = self.builder
                                                .build_pointer_cast(default_val.into_pointer_value(), i8_ptr_type, "default_ptr_cast")
                                                .map_err(|e| vec![Diagnostic::error(
                                                    format!("LLVM pointer cast error: {}", e), span
                                                )])?;
                                            converted_args.push(ptr.into());
                                        } else {
                                            // Fallback: null ptr
                                            converted_args.push(i8_ptr_type.const_null().into());
                                        }
                                    }
                                }

                                // Handle option_ok_or: needs error value ptr and error size
                                if builtin_name.as_str() == "option_ok_or" {
                                    // Second argument is the error value E
                                    if args.len() >= 2 {
                                        let err_val = &arg_vals[1];
                                        // Allocate on stack and pass pointer
                                        if err_val.is_int_value() {
                                            let int_val = err_val.into_int_value();
                                            let alloca = self.builder
                                                .build_alloca(int_val.get_type(), "err_tmp")
                                                .map_err(|e| vec![Diagnostic::error(
                                                    format!("LLVM alloca error: {}", e), span
                                                )])?;
                                            self.builder.build_store(alloca, int_val)
                                                .map_err(|e| vec![Diagnostic::error(
                                                    format!("LLVM store error: {}", e), span
                                                )])?;
                                            let ptr = self.builder
                                                .build_pointer_cast(alloca, i8_ptr_type, "err_ptr_cast")
                                                .map_err(|e| vec![Diagnostic::error(
                                                    format!("LLVM pointer cast error: {}", e), span
                                                )])?;
                                            converted_args.push(ptr.into());
                                            // Add error size
                                            let err_size = self.get_type_size_in_bytes(int_val.get_type().into());
                                            converted_args.push(self.context.i64_type().const_int(err_size, false).into());
                                        } else if err_val.is_struct_value() {
                                            let struct_val = err_val.into_struct_value();
                                            let alloca = self.builder
                                                .build_alloca(struct_val.get_type(), "err_tmp")
                                                .map_err(|e| vec![Diagnostic::error(
                                                    format!("LLVM alloca error: {}", e), span
                                                )])?;
                                            self.builder.build_store(alloca, struct_val)
                                                .map_err(|e| vec![Diagnostic::error(
                                                    format!("LLVM store error: {}", e), span
                                                )])?;
                                            let ptr = self.builder
                                                .build_pointer_cast(alloca, i8_ptr_type, "err_ptr_cast")
                                                .map_err(|e| vec![Diagnostic::error(
                                                    format!("LLVM pointer cast error: {}", e), span
                                                )])?;
                                            converted_args.push(ptr.into());
                                            // Add error size
                                            let err_size = self.get_type_size_in_bytes(struct_val.get_type().into());
                                            converted_args.push(self.context.i64_type().const_int(err_size, false).into());
                                        } else if err_val.is_pointer_value() {
                                            let ptr = self.builder
                                                .build_pointer_cast(err_val.into_pointer_value(), i8_ptr_type, "err_ptr_cast")
                                                .map_err(|e| vec![Diagnostic::error(
                                                    format!("LLVM pointer cast error: {}", e), span
                                                )])?;
                                            converted_args.push(ptr.into());
                                            // Add default error size
                                            converted_args.push(self.context.i64_type().const_int(8, false).into());
                                        } else {
                                            // Fallback
                                            converted_args.push(i8_ptr_type.const_null().into());
                                            converted_args.push(self.context.i64_type().const_int(8, false).into());
                                        }
                                    }
                                }

                                // Handle result_and: needs err_size as 4th argument
                                if builtin_name.as_str() == "result_and" {
                                    // Get err_size from the first Result<T, E>
                                    let res_ty = self.get_operand_type(&args[0], body);
                                    let res_inner = match res_ty.kind() {
                                        crate::hir::TypeKind::Ref { inner, .. } => inner.clone(),
                                        _ => res_ty.clone(),
                                    };
                                    let err_size = if let crate::hir::TypeKind::Adt { args: type_args, .. } = res_inner.kind() {
                                        if type_args.len() >= 2 {
                                            let llvm_ty = self.lower_type(&type_args[1]);
                                            let size = self.get_type_size_in_bytes(llvm_ty);
                                            let align = self.get_type_alignment_for_size(llvm_ty);
                                            (align << 32) | size
                                        } else {
                                            8
                                        }
                                    } else {
                                        8
                                    };
                                    converted_args.push(self.context.i64_type().const_int(err_size, false).into());
                                }

                                // Handle result_or: needs other_size as 4th argument
                                if builtin_name.as_str() == "result_or" {
                                    // Get the whole Result<T, F> size from second argument
                                    if args.len() >= 2 {
                                        let other_ty = self.get_operand_type(&args[1], body);
                                        let other_inner = match other_ty.kind() {
                                            crate::hir::TypeKind::Ref { inner, .. } => inner.clone(),
                                            _ => other_ty.clone(),
                                        };
                                        let llvm_ty = self.lower_type(&other_inner);
                                        let other_size = self.get_type_size_in_bytes(llvm_ty);
                                        converted_args.push(self.context.i64_type().const_int(other_size, false).into());
                                    }
                                }

                                // Handle result_as_ref/as_mut: needs err_size as 3rd argument
                                if matches!(builtin_name.as_str(), "result_as_ref" | "result_as_mut") {
                                    // Get err_size from Result<T, E>
                                    let res_ty = self.get_operand_type(&args[0], body);
                                    let res_inner = match res_ty.kind() {
                                        crate::hir::TypeKind::Ref { inner, .. } => inner.clone(),
                                        _ => res_ty.clone(),
                                    };
                                    let err_size = if let crate::hir::TypeKind::Adt { args: type_args, .. } = res_inner.kind() {
                                        if type_args.len() >= 2 {
                                            let llvm_ty = self.lower_type(&type_args[1]);
                                            let size = self.get_type_size_in_bytes(llvm_ty);
                                            let align = self.get_type_alignment_for_size(llvm_ty);
                                            (align << 32) | size
                                        } else {
                                            8
                                        }
                                    } else {
                                        8
                                    };
                                    converted_args.push(self.context.i64_type().const_int(err_size, false).into());
                                }

                                // Get the appropriate payload type based on the method
                                let payload_llvm_ty = if let crate::hir::TypeKind::Adt { args: type_args, .. } = inner_ty.kind() {
                                    match builtin_name.as_str() {
                                        // Option<T> and Result<T, E>.unwrap/try/expect return T (first type arg)
                                        "option_unwrap" | "option_try" | "option_expect" | "option_unwrap_or" |
                                        "result_unwrap" | "result_try" | "result_expect" | "result_unwrap_or" => {
                                            if let Some(payload_ty) = type_args.first() {
                                                self.lower_type(payload_ty)
                                            } else {
                                                self.context.i64_type().into()
                                            }
                                        }
                                        // Result<T, E>.unwrap_err/expect_err returns E (second type arg)
                                        "result_unwrap_err" | "result_expect_err" => {
                                            if type_args.len() >= 2 {
                                                self.lower_type(&type_args[1])
                                            } else {
                                                self.context.i64_type().into()
                                            }
                                        }
                                        // result_ok returns Option<T>, so output is Option struct
                                        "result_ok" => {
                                            // Option<T> = { tag: i32, payload: T }
                                            if let Some(ok_ty) = type_args.first() {
                                                let payload_ty = self.lower_type(ok_ty);
                                                // Create Option struct type: { i32, payload_ty }
                                                self.context.struct_type(&[
                                                    self.context.i32_type().into(),
                                                    payload_ty,
                                                ], false).into()
                                            } else {
                                                self.context.i64_type().into()
                                            }
                                        }
                                        // result_err returns Option<E>, so output is Option struct
                                        "result_err" => {
                                            // Option<E> = { tag: i32, payload: E }
                                            if type_args.len() >= 2 {
                                                let payload_ty = self.lower_type(&type_args[1]);
                                                // Create Option struct type: { i32, payload_ty }
                                                self.context.struct_type(&[
                                                    self.context.i32_type().into(),
                                                    payload_ty,
                                                ], false).into()
                                            } else {
                                                self.context.i64_type().into()
                                            }
                                        }
                                        // option_ok_or returns Result<T, E>, so output is Result struct
                                        "option_ok_or" => {
                                            // Result<T, E> = { tag: i32, payload: max(T, E) }
                                            // For simplicity, we'll use T's type since E might be inferred
                                            if let Some(ok_ty) = type_args.first() {
                                                let payload_ty = self.lower_type(ok_ty);
                                                // Create Result struct type: { i32, payload_ty }
                                                self.context.struct_type(&[
                                                    self.context.i32_type().into(),
                                                    payload_ty,
                                                ], false).into()
                                            } else {
                                                self.context.i64_type().into()
                                            }
                                        }
                                        // vec_first/vec_last/vec_get return Option<&T>, so output is Option struct with pointer
                                        "vec_first" | "vec_last" | "vec_get" => {
                                            // Option<&T> = { tag: i32, payload: *T }
                                            // Use the actual element type pointer for proper type matching
                                            // With opaque pointers, all pointer types are the same
                                            let ptr_type = self.context.ptr_type(AddressSpace::default());
                                            self.context.struct_type(&[
                                                self.context.i32_type().into(),
                                                ptr_type.into(),
                                            ], false).into()
                                        }
                                        // vec_pop returns Option<T>, so output is Option struct
                                        "vec_pop" => {
                                            // Option<T> = { tag: i32, payload: T }
                                            if let Some(elem_ty) = type_args.first() {
                                                let payload_ty = self.lower_type(elem_ty);
                                                self.context.struct_type(&[
                                                    self.context.i32_type().into(),
                                                    payload_ty,
                                                ], false).into()
                                            } else {
                                                self.context.i64_type().into()
                                            }
                                        }
                                        // box_into_inner returns T (the inner type of Box<T>)
                                        "box_into_inner" => {
                                            if let Some(inner_ty) = type_args.first() {
                                                self.lower_type(inner_ty)
                                            } else {
                                                self.context.i64_type().into()
                                            }
                                        }
                                        // option_and returns Option<U> where U is second arg's inner type
                                        "option_and" => {
                                            // Output is Option<U>, need to get U from second arg's type
                                            if args.len() >= 2 {
                                                let other_ty = self.get_operand_type(&args[1], body);
                                                let other_inner = match other_ty.kind() {
                                                    crate::hir::TypeKind::Ref { inner, .. } => inner.clone(),
                                                    _ => other_ty.clone(),
                                                };
                                                self.lower_type(&other_inner)
                                            } else {
                                                self.context.i64_type().into()
                                            }
                                        }
                                        // option_or/xor returns Option<T>
                                        "option_or" | "option_xor" | "option_take" | "option_replace" => {
                                            // Output is Option<T>
                                            if let Some(payload_ty) = type_args.first() {
                                                let t_llvm = self.lower_type(payload_ty);
                                                self.context.struct_type(&[
                                                    self.context.i32_type().into(),
                                                    t_llvm,
                                                ], false).into()
                                            } else {
                                                self.context.i64_type().into()
                                            }
                                        }
                                        // option_as_ref returns Option<&T>
                                        "option_as_ref" => {
                                            let ptr_type = self.context.ptr_type(AddressSpace::default());
                                            self.context.struct_type(&[
                                                self.context.i32_type().into(),
                                                ptr_type.into(),
                                            ], false).into()
                                        }
                                        // option_as_mut returns Option<&mut T>
                                        "option_as_mut" => {
                                            let ptr_type = self.context.ptr_type(AddressSpace::default());
                                            self.context.struct_type(&[
                                                self.context.i32_type().into(),
                                                ptr_type.into(),
                                            ], false).into()
                                        }
                                        // result_and returns Result<U, E> - output is the second Result type
                                        "result_and" => {
                                            if args.len() >= 2 {
                                                let other_ty = self.get_operand_type(&args[1], body);
                                                let other_inner = match other_ty.kind() {
                                                    crate::hir::TypeKind::Ref { inner, .. } => inner.clone(),
                                                    _ => other_ty.clone(),
                                                };
                                                self.lower_type(&other_inner)
                                            } else {
                                                self.context.i64_type().into()
                                            }
                                        }
                                        // result_or returns Result<T, F>
                                        "result_or" => {
                                            if args.len() >= 2 {
                                                let other_ty = self.get_operand_type(&args[1], body);
                                                let other_inner = match other_ty.kind() {
                                                    crate::hir::TypeKind::Ref { inner, .. } => inner.clone(),
                                                    _ => other_ty.clone(),
                                                };
                                                self.lower_type(&other_inner)
                                            } else {
                                                self.context.i64_type().into()
                                            }
                                        }
                                        // result_as_ref/as_mut returns Result<&T, &E> or Result<&mut T, &mut E>
                                        "result_as_ref" | "result_as_mut" => {
                                            // Result<&T, &E> = { tag: i32, payload: *void (pointer) }
                                            let ptr_type = self.context.ptr_type(AddressSpace::default());
                                            self.context.struct_type(&[
                                                self.context.i32_type().into(),
                                                ptr_type.into(),
                                            ], false).into()
                                        }
                                        other => {
                                            // Defensive fallback - all builtins in the output buffer check
                                            // should be explicitly handled above. Using i64 as conservative
                                            // default, but log an ICE to catch missing cases.
                                            ice!(
                                                "Unhandled builtin in output buffer type calculation. Add explicit handling in terminator.rs.";
                                                "builtin" => other,
                                                "default_type" => "i64"
                                            );
                                            self.context.i64_type().into()
                                        },
                                    }
                                } else {
                                    self.context.i64_type().into()
                                };

                                // Allocate output buffer on stack
                                let out_alloca = self.builder
                                    .build_alloca(payload_llvm_ty, "unwrap_out")
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM alloca error: {}", e), span
                                    )])?;

                                // Save for later use after call
                                output_buffer_alloca = Some((out_alloca, payload_llvm_ty));

                                // With opaque pointers, no cast needed - just use the pointer directly
                                let out_ptr = self.builder
                                    .build_pointer_cast(out_alloca, i8_ptr_type, "out_ptr_cast")
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM pointer cast error: {}", e), span
                                    )])?;
                                converted_args.push(out_ptr.into());
                            }
                        }

                        // Handle string_find and string_rfind which need output buffer for Option<usize>
                        // These don't need elem_size, just an output buffer
                        if matches!(builtin_name.as_str(), "string_find" | "string_rfind") {
                            let i8_ptr_type = self.context.ptr_type(AddressSpace::default());
                            // Option<usize> = { tag: i32, payload: i64 }
                            let option_usize_ty = self.context.struct_type(&[
                                self.context.i32_type().into(),
                                self.context.i64_type().into(),
                            ], false);

                            let out_alloca = self.builder
                                .build_alloca(option_usize_ty, "find_out")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM alloca error: {}", e), span
                                )])?;

                            // Save for later use after call
                            output_buffer_alloca = Some((out_alloca, option_usize_ty.into()));

                            let out_ptr = self.builder
                                .build_pointer_cast(out_alloca, i8_ptr_type, "out_ptr_cast")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM pointer cast error: {}", e), span
                                )])?;
                            converted_args.push(out_ptr.into());
                        }

                        // Handle string_new which needs output buffer for String struct
                        // String::new() has no args, but needs output buffer for the String value
                        if builtin_name.as_str() == "string_new" {
                            let i8_ptr_type = self.context.ptr_type(AddressSpace::default());
                            // String = { ptr: *i8, len: i64, capacity: i64 }
                            let string_ty = self.context.struct_type(&[
                                i8_ptr_type.into(),
                                self.context.i64_type().into(),
                                self.context.i64_type().into(),
                            ], false);

                            let out_alloca = self.builder
                                .build_alloca(string_ty, "string_out")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM alloca error: {}", e), span
                                )])?;

                            // Save for later use after call
                            output_buffer_alloca = Some((out_alloca, string_ty.into()));

                            let out_ptr = self.builder
                                .build_pointer_cast(out_alloca, i8_ptr_type, "out_ptr_cast")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM pointer cast error: {}", e), span
                                )])?;
                            converted_args.push(out_ptr.into());
                        }

                        // Handle str_to_string which needs output buffer for String struct
                        // str.to_string() takes &str input (already in args), needs output buffer
                        if builtin_name.as_str() == "str_to_string" {
                            let i8_ptr_type = self.context.ptr_type(AddressSpace::default());
                            // String = { ptr: *i8, len: i64, capacity: i64 }
                            let string_ty = self.context.struct_type(&[
                                i8_ptr_type.into(),
                                self.context.i64_type().into(),
                                self.context.i64_type().into(),
                            ], false);

                            let out_alloca = self.builder
                                .build_alloca(string_ty, "str_to_string_out")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM alloca error: {}", e), span
                                )])?;

                            // Save for later use after call
                            output_buffer_alloca = Some((out_alloca, string_ty.into()));

                            let out_ptr = self.builder
                                .build_pointer_cast(out_alloca, i8_ptr_type, "out_ptr_cast")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM pointer cast error: {}", e), span
                                )])?;
                            converted_args.push(out_ptr.into());
                        }

                        // Handle vec_new which needs elem_size and output buffer
                        // Vec::new() has no args, but runtime needs elem_size and output buffer
                        if builtin_name.as_str() == "vec_new" {
                            let i8_ptr_type = self.context.ptr_type(AddressSpace::default());
                            // Vec = { ptr: *i8, len: i64, capacity: i64 }
                            let vec_ty = self.context.struct_type(&[
                                i8_ptr_type.into(),
                                self.context.i64_type().into(),
                                self.context.i64_type().into(),
                            ], false);

                            // Allocate output buffer for Vec struct
                            let out_alloca = self.builder
                                .build_alloca(vec_ty, "vec_out")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM alloca error: {}", e), span
                                )])?;
                            // Set explicit 8-byte alignment for Vec struct (contains pointers and i64s)
                            if let Some(inst) = out_alloca.as_instruction() {
                                let _ = inst.set_alignment(8);
                            }

                            // Save for later use after call
                            output_buffer_alloca = Some((out_alloca, vec_ty.into()));

                            // Get element type from destination (Vec<T> -> T)
                            let dest_ty = self.get_place_base_type(destination, body)?;
                            let elem_size: u64 = if let crate::hir::TypeKind::Adt { args: type_args, .. } = dest_ty.kind() {
                                if let Some(elem_ty) = type_args.first() {
                                    let llvm_ty = self.lower_type(elem_ty);
                                    self.get_type_size_in_bytes(llvm_ty)
                                } else {
                                    8 // Default pointer size
                                }
                            } else {
                                8 // Default pointer size
                            };
                            let elem_size_val = self.context.i64_type().const_int(elem_size, false);
                            converted_args.push(elem_size_val.into());

                            // Add output buffer pointer
                            let out_ptr = self.builder
                                .build_pointer_cast(out_alloca, i8_ptr_type, "vec_out_ptr_cast")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM pointer cast error: {}", e), span
                                )])?;
                            converted_args.push(out_ptr.into());
                        }

                        // Handle vec_with_capacity which needs elem_size, capacity, and output buffer
                        // Vec::with_capacity(cap) -> vec_with_capacity(elem_size, cap, out)
                        if builtin_name.as_str() == "vec_with_capacity" {
                            let i8_ptr_type = self.context.ptr_type(AddressSpace::default());
                            // Vec = { ptr: *i8, len: i64, capacity: i64 }
                            let vec_ty = self.context.struct_type(&[
                                i8_ptr_type.into(),
                                self.context.i64_type().into(),
                                self.context.i64_type().into(),
                            ], false);

                            // Allocate output buffer for Vec struct
                            let out_alloca = self.builder
                                .build_alloca(vec_ty, "vec_out")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM alloca error: {}", e), span
                                )])?;
                            // Set explicit 8-byte alignment for Vec struct (contains pointers and i64s)
                            if let Some(inst) = out_alloca.as_instruction() {
                                let _ = inst.set_alignment(8);
                            }

                            // Save for later use after call
                            output_buffer_alloca = Some((out_alloca, vec_ty.into()));

                            // Get element type from destination (Vec<T> -> T)
                            let dest_ty = self.get_place_base_type(destination, body)?;
                            let elem_size: u64 = if let crate::hir::TypeKind::Adt { args: type_args, .. } = dest_ty.kind() {
                                if let Some(elem_ty) = type_args.first() {
                                    let llvm_ty = self.lower_type(elem_ty);
                                    self.get_type_size_in_bytes(llvm_ty)
                                } else {
                                    8 // Default pointer size
                                }
                            } else {
                                8 // Default pointer size
                            };
                            let elem_size_val = self.context.i64_type().const_int(elem_size, false);
                            // Insert elem_size at the beginning, capacity is already in converted_args
                            converted_args.insert(0, elem_size_val.into());

                            // Add output buffer pointer at the end
                            let out_ptr = self.builder
                                .build_pointer_cast(out_alloca, i8_ptr_type, "vec_out_ptr_cast")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM pointer cast error: {}", e), span
                                )])?;
                            converted_args.push(out_ptr.into());
                        }

                        // Make the call
                        let call_instr = self.builder.build_call(fn_value, &converted_args, "builtin_call")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM call error: {}", e), span
                            )])?;

                        // Track active regions for effect suspension.
                        // When a region is activated, push its ID onto the stack.
                        // When deactivated, pop. This allows compile_perform_terminator
                        // to emit blood_continuation_add_suspended_region calls.
                        match builtin_name.as_str() {
                            "blood_region_activate" => {
                                // First argument is the region_id (i64)
                                if let Some(region_id_val) = arg_vals.first() {
                                    self.active_regions.push(region_id_val.into_int_value());
                                }
                            }
                            "blood_region_deactivate" => {
                                self.active_regions.pop();
                            }
                            _ => {}
                        }

                        // If we used an output buffer, load the result from it and store to destination
                        if let Some((out_alloca, out_ty)) = output_buffer_alloca {
                            let dest_ptr = self.compile_mir_place(destination, body, escape_results)?;
                            let dest_hir_ty = self.get_place_base_type(destination, body)?;
                            let dest_llvm_ty = self.lower_type(&dest_hir_ty);
                            let result = self.builder.build_load(out_ty, out_alloca, "out_result")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM load error: {}", e), span
                                )])?;
                            // Convert result to match destination type if needed
                            let converted_result = self.convert_value_for_store(result, dest_llvm_ty, span)?;
                            let store_inst = self.builder.build_store(dest_ptr, converted_result)
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM store error: {}", e), span
                                )])?;
                            // Set proper alignment for output buffer result store.
                            // Use natural alignment — memory is properly aligned.
                            let alignment = self.get_type_alignment_for_value(converted_result);
                            let _ = store_inst.set_alignment(alignment);

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
                            return Ok(());
                        }

                        // For non-output-buffer calls, return the call instruction value
                        // (this will be void for void functions, which is handled below)
                        call_instr
                    } else {
                        return Err(vec![Diagnostic::error(
                            format!("Runtime function '{}' not declared", builtin_name), span
                        )]);
                    }
                } else {
                    // Function not found - this is likely a generic function.
                    // Try to monomorphize it on-demand with concrete types from the call.
                    if let crate::hir::TypeKind::Fn { params, ret, const_args, .. } = ty.kind() {
                        // For const-generic functions, the function type may still have
                        // Param sizes in array types. Use actual argument types from the
                        // call operands which have concrete sizes.
                        let concrete_params: Vec<_> = if args.len() == params.len() {
                            args.iter().map(|arg| self.get_operand_type(arg, body)).collect()
                        } else {
                            params.clone()
                        };
                        // Use destination type for concrete return if available
                        let concrete_ret = self.get_place_base_type(destination, body)
                            .unwrap_or_else(|_| ret.clone());
                        // Attempt monomorphization, passing explicit const args from turbofish
                        if let Some(mono_fn) = self.monomorphize_function_with_const_args(*def_id, &concrete_params, &concrete_ret, const_args) {
                            self.builder.build_call(mono_fn, &arg_metas, "mono_call")
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM call error: {}", e), span
                                )])?
                        } else {
                            // Monomorphization failed
                            return Err(vec![Diagnostic::error(
                                format!("Failed to monomorphize generic function {:?}.\n\
                                         Concrete type at call site: Fn {{ params: {:?}, ret: {:?} }}\n\
                                         This may indicate missing MIR body or type substitution error.",
                                        def_id, concrete_params, ret),
                                span
                            )]);
                        }
                    } else {
                        return Err(vec![Diagnostic::error(
                            format!("Generic function {:?} called with non-function type: {:?}",
                                    def_id, ty),
                            span
                        )]);
                    }
                }
            }
            // Check for closure call: func is Copy/Move of a local with Closure type
            Operand::Copy(place) | Operand::Move(place) => {
                if let Some(closure_def_id) = get_closure_def_id(place, body) {
                    // Closure call - call the closure function
                    if let Some(&fn_value) = self.functions.get(&closure_def_id) {
                        // Check if the closure function expects an env parameter
                        // Closures without captures have signature (args...) -> ret
                        // Closures with captures have signature (env*, args...) -> ret
                        let fn_param_count = fn_value.count_params() as usize;
                        let has_env_param = fn_param_count == args.len() + 1;

                        // Get parameter types for argument conversion
                        let fn_type = fn_value.get_type();
                        let param_types = fn_type.get_param_types();

                        let full_args: Vec<BasicMetadataValueEnum> = if has_env_param {
                            // Closure with captures - extract and prepend env pointer
                            let closure_val = self.compile_mir_operand(func, body, escape_results)?;

                            // Check if this closure uses inline environment storage.
                            // Inline closures have struct { fn_ptr, capture_0, capture_1, ... }
                            // instead of { fn_ptr, env_ptr }.
                            let is_inline = self.should_inline_closure_env(
                                &closure_def_id,
                                place.as_local(),
                            );

                            let env_ptr = if is_inline {
                                // Inline environment: extract captures from struct fields 1..N,
                                // build a captures struct, store to alloca, and pass pointer.
                                let sv = if let BasicValueEnum::StructValue(sv) = closure_val {
                                    sv
                                } else {
                                    let ptr = closure_val.into_pointer_value();
                                    let closure_hir_ty = self.get_operand_type(func, body);
                                    let closure_llvm_ty = self.lower_type(&closure_hir_ty);
                                    self.builder.build_load(closure_llvm_ty, ptr, "closure.load")
                                        .map_err(|e| vec![Diagnostic::error(
                                            format!("LLVM load error: {}", e), span
                                        )])?
                                        .into_struct_value()
                                };

                                let struct_ty = sv.get_type();
                                let num_fields = struct_ty.count_fields();
                                // Fields 1..N are captures (field 0 is fn_ptr)
                                let num_captures = num_fields - 1;

                                if num_captures > 0 {
                                    // Extract capture values and build captures struct
                                    let mut capture_types = Vec::with_capacity(num_captures as usize);
                                    let mut capture_vals = Vec::with_capacity(num_captures as usize);
                                    for i in 0..num_captures {
                                        let val = self.builder.build_extract_value(
                                            sv, i + 1, &format!("inline_cap_{}", i)
                                        ).map_err(|e| vec![Diagnostic::error(
                                            format!("LLVM extract error: {}", e), span
                                        )])?;
                                        capture_types.push(val.get_type());
                                        capture_vals.push(val);
                                    }

                                    let captures_ty = self.context.struct_type(&capture_types, false);
                                    let mut captures_val = captures_ty.get_undef();
                                    for (i, val) in capture_vals.iter().enumerate() {
                                        captures_val = self.builder.build_insert_value(
                                            captures_val, *val, i as u32,
                                            &format!("rebuild_cap_{}", i)
                                        ).map_err(|e| vec![Diagnostic::error(
                                            format!("LLVM insert error: {}", e), span
                                        )])?
                                        .into_struct_value();
                                    }

                                    // Store to alloca and cast to i8* for env_ptr
                                    let env_alloca = self.builder.build_alloca(captures_ty, "inline_env")
                                        .map_err(|e| vec![Diagnostic::error(
                                            format!("LLVM alloca error: {}", e), span
                                        )])?;
                                    self.builder.build_store(env_alloca, captures_val)
                                        .map_err(|e| vec![Diagnostic::error(
                                            format!("LLVM store error: {}", e), span
                                        )])?;

                                    let i8_ptr_ty = self.context.ptr_type(AddressSpace::default());
                                    self.builder.build_pointer_cast(env_alloca, i8_ptr_ty, "inline_env_ptr")
                                        .map_err(|e| vec![Diagnostic::error(
                                            format!("LLVM pointer cast error: {}", e), span
                                        )])?
                                        .into()
                                } else {
                                    let i8_ptr_ty = self.context.ptr_type(AddressSpace::default());
                                    i8_ptr_ty.const_null().into()
                                }
                            } else {
                                // Standard fat pointer: extract env_ptr from field 1
                                if let BasicValueEnum::StructValue(sv) = closure_val {
                                    self.builder.build_extract_value(sv, 1, "closure.env")
                                        .map_err(|e| vec![Diagnostic::error(
                                            format!("LLVM extract error: {}", e), span
                                        )])?
                                } else {
                                    let ptr = closure_val.into_pointer_value();
                                    let closure_hir_ty = self.get_operand_type(func, body);
                                    let closure_llvm_ty = self.lower_type(&closure_hir_ty);
                                    let loaded = self.builder.build_load(closure_llvm_ty, ptr, "closure.load")
                                        .map_err(|e| vec![Diagnostic::error(
                                            format!("LLVM load error: {}", e), span
                                        )])?;
                                    let sv = loaded.into_struct_value();
                                    self.builder.build_extract_value(sv, 1, "closure.env")
                                        .map_err(|e| vec![Diagnostic::error(
                                            format!("LLVM extract error: {}", e), span
                                        )])?
                                }
                            };

                            let mut full_args = Vec::with_capacity(args.len() + 1);
                            full_args.push(env_ptr.into());
                            // Convert remaining args (skip param index 0 which is env_ptr)
                            for (i, val) in arg_vals.iter().enumerate() {
                                if let Some(param_type) = param_types.get(i + 1) {
                                    if let Ok(basic_ty) = (*param_type).try_into() {
                                        full_args.push(self.convert_arg_for_param(*val, basic_ty, span)?);
                                    } else {
                                        full_args.push((*val).into());
                                    }
                                } else {
                                    full_args.push((*val).into());
                                }
                            }
                            full_args
                        } else {
                            // Closure without captures - convert all args
                            let mut converted = Vec::with_capacity(arg_vals.len());
                            for (i, val) in arg_vals.iter().enumerate() {
                                if let Some(param_type) = param_types.get(i) {
                                    if let Ok(basic_ty) = (*param_type).try_into() {
                                        converted.push(self.convert_arg_for_param(*val, basic_ty, span)?);
                                    } else {
                                        converted.push((*val).into());
                                    }
                                } else {
                                    converted.push((*val).into());
                                }
                            }
                            converted
                        };

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
                    // Indirect call through function pointer (fn() type)
                    // fn() is now a fat pointer { fn_ptr, env_ptr } to support closures with captures
                    let func_val = self.compile_mir_operand(func, body, escape_results)?;

                    // Extract fn_ptr and env_ptr from the fat pointer struct
                    let (fn_ptr, env_ptr) = if let BasicValueEnum::StructValue(sv) = func_val {
                        let fn_ptr = self.builder.build_extract_value(sv, 0, "fn.ptr")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM extract error: {}", e), span
                            )])?
                            .into_pointer_value();
                        let env_ptr = self.builder.build_extract_value(sv, 1, "fn.env")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM extract error: {}", e), span
                            )])?;
                        (fn_ptr, env_ptr)
                    } else if let BasicValueEnum::PointerValue(ptr) = func_val {
                        // If it's stored via pointer, load the struct first
                        let func_hir_ty = self.get_operand_type(func, body);
                        let func_llvm_ty = self.lower_type(&func_hir_ty);
                        let loaded = self.builder.build_load(func_llvm_ty, ptr, "fn.load")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM load error: {}", e), span
                            )])?;
                        let sv = loaded.into_struct_value();
                        let fn_ptr = self.builder.build_extract_value(sv, 0, "fn.ptr")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM extract error: {}", e), span
                            )])?
                            .into_pointer_value();
                        let env_ptr = self.builder.build_extract_value(sv, 1, "fn.env")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM extract error: {}", e), span
                            )])?;
                        (fn_ptr, env_ptr)
                    } else {
                        return Err(vec![Diagnostic::error(
                            format!("Expected fn struct or pointer for call, got {:?}", func_val.get_type()), span
                        )]);
                    };

                    // Build function type: (env_ptr, args...) -> ret
                    // Get the return type from the func operand's type
                    let func_ty = self.get_operand_type(func, body);
                    let i8_ptr_ty = self.context.ptr_type(AddressSpace::default());

                    let (fn_type, llvm_param_types) = if let crate::hir::TypeKind::Fn { params, ret, .. } = func_ty.kind() {
                        let mut param_types: Vec<BasicMetadataTypeEnum> = Vec::with_capacity(params.len() + 1);
                        let mut llvm_params: Vec<inkwell::types::BasicTypeEnum> = Vec::with_capacity(params.len() + 1);
                        param_types.push(i8_ptr_ty.into()); // env_ptr
                        llvm_params.push(i8_ptr_ty.into());
                        for p in params {
                            let llvm_ty = self.lower_type(p);
                            param_types.push(llvm_ty.into());
                            llvm_params.push(llvm_ty);
                        }
                        let fn_ty = if ret.is_unit() {
                            self.context.void_type().fn_type(&param_types, false)
                        } else {
                            let ret_type = self.lower_type(ret);
                            ret_type.fn_type(&param_types, false)
                        };
                        (fn_ty, llvm_params)
                    } else {
                        return Err(vec![Diagnostic::error(
                            "Expected Fn type for indirect call", span
                        )]);
                    };

                    // Build args with env_ptr prepended (closures expect env as first arg)
                    // Convert arguments to match expected parameter types
                    let mut full_args: Vec<BasicMetadataValueEnum> = Vec::with_capacity(args.len() + 1);
                    full_args.push(env_ptr.into());
                    for (i, val) in arg_vals.iter().enumerate() {
                        // param index i+1 because param 0 is env_ptr
                        if let Some(param_type) = llvm_param_types.get(i + 1) {
                            full_args.push(self.convert_arg_for_param(*val, *param_type, span)?);
                        } else {
                            full_args.push((*val).into());
                        }
                    }

                    // Use build_indirect_call with the function type and pointer
                    // (opaque pointers: no need to cast fn_ptr to typed pointer)
                    self.builder.build_indirect_call(fn_type, fn_ptr, &full_args, "call_result")
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
        let dest_ptr = self.compile_mir_place(destination, body, escape_results)?;
        let call_dest_hir_ty = self.get_place_base_type(destination, body)?;
        let call_dest_llvm_ty = self.lower_type(&call_dest_hir_ty);
        if let Some(ret_val) = call_result.try_as_basic_value().basic() {
            // Convert return value to destination type if needed
            let converted_val = self.convert_value_for_store(ret_val, call_dest_llvm_ty, span)?;
            let store_inst = self.builder.build_store(dest_ptr, converted_val)
                .map_err(|e| vec![Diagnostic::error(
                    format!("LLVM store error: {}", e), span
                )])?;
            // Set proper alignment based on the value type.
            // Use natural alignment — allocas and heap allocations are properly aligned.
            let alignment = self.get_type_alignment_for_value(converted_val);
            let _ = store_inst.set_alignment(alignment);
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

    // Compiler-internal: decomposing would reduce clarity
    #[allow(clippy::too_many_arguments)]
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
                let ptr_type = self.context.ptr_type(AddressSpace::default());
                let panic_type = void_type.fn_type(&[ptr_type.into()], false);
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

    // Compiler-internal: decomposing would reduce clarity
    #[allow(clippy::too_many_arguments)]
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
        // For non-tail-resumptive effects, suspend any active regions so they
        // stay alive across continuation capture. The continuation handle is
        // created below (at create_perform_continuation) and the suspension
        // calls are emitted there. We record whether suspension is needed here.
        let needs_region_suspension = !is_tail_resumptive && !self.active_regions.is_empty();

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
            .basic()
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
                        let local_llvm_ty = self.lower_type(&local.ty);
                        let ptr_val = self.builder.build_load(local_llvm_ty, local_ptr, &format!("cap_{}", local.id.index))
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
            self.context.ptr_type(AddressSpace::default()).const_null()
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
                    self.builder.build_gep(array_ty, args_alloca, &[zero, idx], &format!("arg_{}", i))
                }.map_err(|e| vec![Diagnostic::error(format!("LLVM GEP error: {}", e), span)])?;

                // Convert value to i64 based on its type
                let val_i64 = match *val {
                    BasicValueEnum::IntValue(iv) => {
                        if iv.get_type().get_bit_width() == 64 {
                            iv
                        } else {
                            self.builder.build_int_z_extend(iv, i64_ty, "arg_i64")
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM extend error: {}", e), span)])?
                        }
                    }
                    BasicValueEnum::FloatValue(fv) => {
                        self.builder.build_bit_cast(fv, i64_ty, "float_as_i64")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM bitcast error: {}", e), span)])?
                            .into_int_value()
                    }
                    BasicValueEnum::PointerValue(pv) => {
                        self.builder.build_ptr_to_int(pv, i64_ty, "ptr_as_i64")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM ptr_to_int error: {}", e), span)])?
                    }
                    BasicValueEnum::StructValue(sv) => {
                        // Allocate stack space and store the struct
                        let struct_alloca = self.builder.build_alloca(sv.get_type(), &format!("struct_arg_{}", i))
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM alloca error: {}", e), span)])?;
                        self.builder.build_store(struct_alloca, sv)
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM store error: {}", e), span)])?;
                        // Pass pointer as i64
                        self.builder.build_ptr_to_int(struct_alloca, i64_ty, "struct_ptr_as_i64")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM ptr_to_int error: {}", e), span)])?
                    }
                    BasicValueEnum::ArrayValue(av) => {
                        // Allocate stack space and store the array
                        let array_alloca = self.builder.build_alloca(av.get_type(), &format!("array_arg_{}", i))
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM alloca error: {}", e), span)])?;
                        self.builder.build_store(array_alloca, av)
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM store error: {}", e), span)])?;
                        // Pass pointer as i64
                        self.builder.build_ptr_to_int(array_alloca, i64_ty, "array_ptr_as_i64")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM ptr_to_int error: {}", e), span)])?
                    }
                    BasicValueEnum::VectorValue(_) | BasicValueEnum::ScalableVectorValue(_) => {
                        return Err(vec![ice_err!(
                            span,
                            "unsupported argument type in perform expression";
                            "type" => "VectorValue/ScalableVectorValue",
                            "expected" => "IntValue, FloatValue, PointerValue, StructValue, or ArrayValue"
                        )]);
                    }
                };

                self.builder.build_store(gep, val_i64)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM store error: {}", e), span)])?;
            }

            // With opaque pointers, all pointers are the same type - no cast needed
            args_alloca
        };

        // Create continuation for the handler to resume to.
        // For tail-resumptive handlers, this is ignored (handler just returns).
        // For non-tail-resumptive handlers, this allows the handler to continue
        // executing code after resume() returns.
        let continuation_val = self.create_perform_continuation()?;

        // For non-tail-resumptive effects inside region blocks, suspend the
        // active regions so they aren't destroyed while the continuation exists.
        // Each active region is associated with the continuation so that
        // blood_continuation_resume_with_regions() can restore them on resume.
        if needs_region_suspension {
            let add_suspended_fn = self.module.get_function("blood_continuation_add_suspended_region")
                .ok_or_else(|| vec![Diagnostic::error(
                    "Runtime function blood_continuation_add_suspended_region not found", span
                )])?;

            // Clone the region list to avoid borrow issues (active_regions
            // is on `self` which we also mutate via builder calls).
            let regions: Vec<_> = self.active_regions.clone();
            for region_id in &regions {
                self.builder.build_call(
                    add_suspended_fn,
                    &[continuation_val.into(), (*region_id).into()],
                    "",
                ).map_err(|e| vec![Diagnostic::error(
                    format!("LLVM call error: {}", e), span
                )])?;
            }
        }

        let result = self.builder.build_call(
            perform_fn,
            &[effect_id_val.into(), op_index_val.into(), args_ptr.into(), arg_count.into(), continuation_val.into()],
            "perform_result"
        ).map_err(|e| vec![Diagnostic::error(format!("LLVM call error: {}", e), span)])?;

        // Store result to destination with type conversion
        // blood_perform returns i64, but destination may be a different type.
        // Get the destination type and convert accordingly.
        let dest_ty = self.get_place_base_type(destination, body)?;
        let dest_llvm_ty = self.lower_type(&dest_ty);

        // Skip storing for unit type (empty struct) - there's no actual value to store
        let is_unit_type = dest_ty.is_unit();

        if !is_unit_type {
            let dest_ptr = self.compile_mir_place(destination, body, escape_results)?;
            let result_val = result.try_as_basic_value()
                .basic()
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
            } else if dest_llvm_ty.is_struct_type() {
                // For struct types (including enums), the bits are packed into the i64.
                // Unpack by storing i64 to stack, bitcasting to struct*, and loading.
                let result_i64 = result_val.into_int_value();
                let i64_ty = self.context.i64_type();
                let dest_struct_ty = dest_llvm_ty.into_struct_type();

                // Allocate temp storage for the i64
                let temp_alloca = self.builder.build_alloca(i64_ty, "perform_temp")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM alloca error: {}", e), span
                    )])?;

                // Store the i64 (which contains packed struct bits)
                self.builder.build_store(temp_alloca, result_i64)
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM store error: {}", e), span
                    )])?;

                // With opaque pointers, no bitcast needed - load struct type directly from the alloca
                self.builder.build_load(dest_struct_ty, temp_alloca, "perform_struct")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM load error: {}", e), span
                    )])?
            } else {
                // For other types, use the value directly
                // This may fail if types don't match, but that indicates a bug elsewhere
                result_val
            };

            let store_inst = self.builder.build_store(dest_ptr, converted_result)
                .map_err(|e| vec![Diagnostic::error(format!("LLVM store error: {}", e), span)])?;
            // Set proper alignment for Perform result store.
            // Use natural alignment — memory is properly aligned.
            let alignment = self.get_type_alignment_for_value(converted_result);
            let _ = store_inst.set_alignment(alignment);
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
            .basic()
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
            .basic()
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
            .basic()
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

    /// Convert a single argument value to match the expected parameter type.
    /// This handles cases like passing a struct value to a function expecting a pointer.
    fn convert_arg_for_param(
        &mut self,
        val: BasicValueEnum<'ctx>,
        param_type: inkwell::types::BasicTypeEnum<'ctx>,
        span: crate::span::Span,
    ) -> Result<BasicMetadataValueEnum<'ctx>, Vec<Diagnostic>> {
        if val.is_int_value() && param_type.is_int_type() {
            let val_int = val.into_int_value();
            let param_int_type = param_type.into_int_type();
            // Zero-extend if arg is smaller (e.g., i1 -> i32)
            if val_int.get_type().get_bit_width() < param_int_type.get_bit_width() {
                Ok(self.builder
                    .build_int_z_extend(val_int, param_int_type, "zext")
                    .map(|v| v.into())
                    .unwrap_or(val.into()))
            } else {
                Ok(val.into())
            }
        } else if param_type.is_pointer_type() && val.is_struct_value() {
            // Parameter expects a pointer but we have a struct value
            let struct_val = val.into_struct_value();
            let alloca = self.builder
                .build_alloca(struct_val.get_type(), "arg_tmp")
                .map_err(|e| vec![Diagnostic::error(
                    format!("LLVM alloca error: {}", e), span
                )])?;
            self.builder.build_store(alloca, struct_val)
                .map_err(|e| vec![Diagnostic::error(
                    format!("LLVM store error: {}", e), span
                )])?;
            let expected_ptr_type = param_type.into_pointer_type();
            if alloca.get_type() != expected_ptr_type {
                Ok(self.builder.build_pointer_cast(alloca, expected_ptr_type, "ptr_cast")
                    .map(|p| p.into())
                    .unwrap_or(alloca.into()))
            } else {
                Ok(alloca.into())
            }
        } else if param_type.is_pointer_type() && val.is_pointer_value() {
            // Parameter expects pointer and we have pointer - cast if needed
            let ptr_val = val.into_pointer_value();
            let expected_ptr_type = param_type.into_pointer_type();
            if ptr_val.get_type() != expected_ptr_type {
                Ok(self.builder.build_pointer_cast(ptr_val, expected_ptr_type, "ptr_cast")
                    .map(|p| p.into())
                    .unwrap_or(val.into()))
            } else {
                Ok(val.into())
            }
        } else if param_type.is_pointer_type() && val.is_int_value() {
            // Parameter expects pointer but we have int - allocate on stack
            let int_val = val.into_int_value();
            let alloca = self.builder
                .build_alloca(int_val.get_type(), "int_arg_tmp")
                .map_err(|e| vec![Diagnostic::error(
                    format!("LLVM alloca error: {}", e), span
                )])?;
            self.builder.build_store(alloca, int_val)
                .map_err(|e| vec![Diagnostic::error(
                    format!("LLVM store error: {}", e), span
                )])?;
            let expected_ptr_type = param_type.into_pointer_type();
            if alloca.get_type() != expected_ptr_type {
                Ok(self.builder.build_pointer_cast(alloca, expected_ptr_type, "ptr_cast")
                    .map(|p| p.into())
                    .unwrap_or(alloca.into()))
            } else {
                Ok(alloca.into())
            }
        } else if param_type.is_pointer_type() && val.is_array_value() {
            // Parameter expects pointer but we have array value
            let array_val = val.into_array_value();
            let alloca = self.builder
                .build_alloca(array_val.get_type(), "array_arg_tmp")
                .map_err(|e| vec![Diagnostic::error(
                    format!("LLVM alloca error: {}", e), span
                )])?;
            self.builder.build_store(alloca, array_val)
                .map_err(|e| vec![Diagnostic::error(
                    format!("LLVM store error: {}", e), span
                )])?;
            let expected_ptr_type = param_type.into_pointer_type();
            if alloca.get_type() != expected_ptr_type {
                Ok(self.builder.build_pointer_cast(alloca, expected_ptr_type, "ptr_cast")
                    .map(|p| p.into())
                    .unwrap_or(alloca.into()))
            } else {
                Ok(alloca.into())
            }
        } else {
            Ok(val.into())
        }
    }

    /// Convert a value to match the expected destination type for a store.
    /// This handles cases like integer width mismatches, pointer/int conversions, etc.
    ///
    /// `dest_type` is the LLVM type of the value that will be stored (the pointee type
    /// of the destination pointer). With opaque pointers, this must be provided explicitly
    /// since pointer types no longer carry element type information.
    pub(super) fn convert_value_for_store(
        &mut self,
        val: BasicValueEnum<'ctx>,
        dest_type: inkwell::types::BasicTypeEnum<'ctx>,
        span: crate::span::Span,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        // Integer type conversions (truncate or extend)
        if val.is_int_value() && dest_type.is_int_type() {
            let val_int = val.into_int_value();
            let dest_int_type = dest_type.into_int_type();
            let val_width = val_int.get_type().get_bit_width();
            let dest_width = dest_int_type.get_bit_width();

            if val_width > dest_width {
                // Truncate
                return Ok(self.builder.build_int_truncate(val_int, dest_int_type, "store_trunc")
                    .map(|v| v.into())
                    .unwrap_or(val));
            } else if val_width < dest_width {
                // Zero-extend
                return Ok(self.builder.build_int_z_extend(val_int, dest_int_type, "store_zext")
                    .map(|v| v.into())
                    .unwrap_or(val));
            }
        }

        // With opaque pointers, all pointer types are identical - no cast needed
        // (pointer-to-pointer casts are no-ops)

        // Integer to pointer conversion (for opaque pointers represented as integers)
        if val.is_int_value() && dest_type.is_pointer_type() {
            let int_val = val.into_int_value();
            let ptr_type = dest_type.into_pointer_type();
            return Ok(self.builder.build_int_to_ptr(int_val, ptr_type, "store_int_to_ptr")
                .map(|p| p.into())
                .unwrap_or(val));
        }

        // Pointer to integer conversion
        if val.is_pointer_value() && dest_type.is_int_type() {
            let ptr_val = val.into_pointer_value();
            let int_type = dest_type.into_int_type();
            return Ok(self.builder.build_ptr_to_int(ptr_val, int_type, "store_ptr_to_int")
                .map(|i| i.into())
                .unwrap_or(val));
        }

        // Float width conversions
        if val.is_float_value() && dest_type.is_float_type() {
            let float_val = val.into_float_value();
            let dest_float_type = dest_type.into_float_type();
            if float_val.get_type() != dest_float_type {
                return Ok(self.builder.build_float_cast(float_val, dest_float_type, "store_float_cast")
                    .map(|f| f.into())
                    .unwrap_or(val));
            }
        }

        // Struct value to pointer (store to temporary then return pointer)
        // Note: This is for cases where dest expects a pointer but we have a struct value
        // This shouldn't happen in normal store operations, but handle it defensively
        if val.is_struct_value() && dest_type.is_pointer_type() {
            let struct_val = val.into_struct_value();
            let alloca = self.builder
                .build_alloca(struct_val.get_type(), "store_struct_tmp")
                .map_err(|e| vec![Diagnostic::error(
                    format!("LLVM alloca error: {}", e), span
                )])?;
            self.builder.build_store(alloca, struct_val)
                .map_err(|e| vec![Diagnostic::error(
                    format!("LLVM store error: {}", e), span
                )])?;
            return Ok(alloca.into());
        }

        // Struct-to-struct conversion with different field types
        // This handles cases like structs with different integer width fields
        if val.is_struct_value() && dest_type.is_struct_type() {
            let struct_val = val.into_struct_value();
            let val_struct_type = struct_val.get_type();
            let dest_struct_type = dest_type.into_struct_type();

            // Check if same number of fields
            if val_struct_type.count_fields() == dest_struct_type.count_fields() {
                let field_count = val_struct_type.count_fields();
                let mut needs_conversion = false;

                // Check if any field types differ
                for i in 0..field_count {
                    let val_field_type = val_struct_type.get_field_type_at_index(i);
                    let dest_field_type = dest_struct_type.get_field_type_at_index(i);
                    if val_field_type != dest_field_type {
                        needs_conversion = true;
                        break;
                    }
                }

                if needs_conversion {
                    // Build a new struct with converted fields
                    let mut new_struct = dest_struct_type.get_undef();

                    for i in 0..field_count {
                        let field_val = self.builder.build_extract_value(struct_val, i, &format!("field_{}", i))
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM extract_value error: {}", e), span
                            )])?;

                        if let Some(_dest_field_type) = dest_struct_type.get_field_type_at_index(i) {
                            // With opaque pointers, pointer fields are always the same type
                            // so no conversion is needed for pointer fields
                            new_struct = self.builder.build_insert_value(new_struct, field_val, i, &format!("insert_{}", i))
                                .map_err(|e| vec![Diagnostic::error(
                                    format!("LLVM insert_value error: {}", e), span
                                )])?
                                .into_struct_value();
                        }
                    }

                    return Ok(new_struct.into());
                }
            }
        }

        // Pointer value being stored to struct destination: load the value from pointer.
        // This handles cases where an enum variant field pointer is incorrectly passed as
        // a value instead of being loaded first. This can happen with complex projection
        // chains like Downcast + Field on reference types.
        if val.is_pointer_value() && dest_type.is_struct_type() {
            let ptr_val = val.into_pointer_value();
            let dest_struct_ty = dest_type.into_struct_type();
            // Load the struct from the pointer
            let loaded = self.builder.build_load(dest_struct_ty, ptr_val, "store_ptr_load")
                .map_err(|e| vec![Diagnostic::error(
                    format!("LLVM load error: {}", e), span
                )])?;
            return Ok(loaded);
        }

        // No conversion needed or not possible
        Ok(val)
    }
}
