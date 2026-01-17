//! MIR terminator code generation.
//!
//! This module handles compilation of MIR terminators to LLVM IR.
//! Terminators are the final instructions in a basic block that control
//! program flow (branches, calls, returns, etc.).

use std::collections::HashMap;

use inkwell::basic_block::BasicBlock;
use inkwell::intrinsics::Intrinsic;
use inkwell::types::{BasicMetadataTypeEnum, BasicType};
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
                // Check if this is the main function (needs i32 return for C runtime)
                let is_main = self.current_fn_def_id == self.main_fn_def_id && self.main_fn_def_id.is_some();

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
            let local = body.locals.get(place.local.index() as usize)?;
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
                    self.builder.build_call(fn_value, &arg_metas, "call_result")
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
                                format!("ptr_read requires a pointer argument"), span
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

                        // Cast integer to pointer: inttoptr i64 %ptr to TYPE*
                        let ptr_type = elem_type.ptr_type(AddressSpace::default());
                        let typed_ptr = self.builder.build_int_to_ptr(ptr_arg, ptr_type, "ptr_cast")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM int_to_ptr error: {}", e), span
                            )])?;

                        // Load value from pointer
                        let loaded_val = self.builder.build_load(typed_ptr, "ptr_load")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM load error: {}", e), span
                            )])?;

                        // Store result to destination
                        let dest_ptr = self.compile_mir_place(destination, body, escape_results)?;
                        self.builder.build_store(dest_ptr, loaded_val)
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM store error: {}", e), span
                            )])?;

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
                                format!("ptr_write requires pointer and value arguments"), span
                            )]);
                        }
                        let ptr_arg = arg_vals[0].into_int_value();
                        let value_arg = arg_vals[1];

                        // Determine the element type based on the suffix
                        let elem_type: inkwell::types::BasicTypeEnum = match builtin_name.as_str() {
                            "ptr_write_i32" => self.context.i32_type().into(),
                            "ptr_write_i64" => self.context.i64_type().into(),
                            "ptr_write_u64" => self.context.i64_type().into(), // u64 represented as i64
                            "ptr_write_u8" => self.context.i8_type().into(),
                            "ptr_write_f64" => self.context.f64_type().into(),
                            _ => return Err(vec![Diagnostic::error(
                                format!("Unknown ptr_write variant: {}", builtin_name), span
                            )]),
                        };

                        // Cast integer to pointer: inttoptr i64 %ptr to TYPE*
                        let ptr_type = elem_type.ptr_type(AddressSpace::default());
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
                        let converted_args: Vec<BasicMetadataValueEnum> = arg_vals.iter()
                            .enumerate()
                            .map(|(i, val)| {
                                if let Some(param_type) = param_types.get(i) {
                                    if val.is_int_value() && param_type.is_int_type() {
                                        let val_int = val.into_int_value();
                                        let param_int_type = param_type.into_int_type();
                                        // Zero-extend if arg is smaller (e.g., i1 -> i32)
                                        if val_int.get_type().get_bit_width() < param_int_type.get_bit_width() {
                                            return self.builder
                                                .build_int_z_extend(val_int, param_int_type, "zext")
                                                .map(|v| v.into())
                                                .unwrap_or((*val).into());
                                        }
                                    }
                                }
                                (*val).into()
                            })
                            .collect();

                        self.builder.build_call(fn_value, &converted_args, "builtin_call")
                            .map_err(|e| vec![Diagnostic::error(
                                format!("LLVM call error: {}", e), span
                            )])?
                    } else {
                        return Err(vec![Diagnostic::error(
                            format!("Runtime function '{}' not declared", builtin_name), span
                        )]);
                    }
                } else {
                    // Function not found - this is likely a generic function.
                    // Try to monomorphize it on-demand with concrete types from the call.
                    if let crate::hir::TypeKind::Fn { params, ret } = ty.kind() {
                        // Attempt monomorphization
                        if let Some(mono_fn) = self.monomorphize_function(*def_id, params, ret) {
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
                                        def_id, params, ret),
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

                        let full_args: Vec<BasicMetadataValueEnum> = if has_env_param {
                            // Closure with captures - extract and prepend env pointer
                            let closure_val = self.compile_mir_operand(func, body, escape_results)?;

                            // Extract the environment pointer from the closure struct (field 1)
                            // The closure struct is { fn_ptr, env_ptr }
                            let env_ptr = if let BasicValueEnum::StructValue(sv) = closure_val {
                                self.builder.build_extract_value(sv, 1, "closure.env")
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM extract error: {}", e), span
                                    )])?
                            } else {
                                // If it's a pointer, load the struct first
                                let ptr = closure_val.into_pointer_value();
                                let loaded = self.builder.build_load(ptr, "closure.load")
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM load error: {}", e), span
                                    )])?;
                                let sv = loaded.into_struct_value();
                                self.builder.build_extract_value(sv, 1, "closure.env")
                                    .map_err(|e| vec![Diagnostic::error(
                                        format!("LLVM extract error: {}", e), span
                                    )])?
                            };

                            let mut full_args = Vec::with_capacity(args.len() + 1);
                            full_args.push(env_ptr.into());
                            full_args.extend(arg_metas.iter().cloned());
                            full_args
                        } else {
                            // Closure without captures - just use the args directly
                            arg_metas.clone()
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
                        let loaded = self.builder.build_load(ptr, "fn.load")
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

                    // Build args with env_ptr prepended (closures expect env as first arg)
                    let mut full_args: Vec<BasicMetadataValueEnum> = Vec::with_capacity(args.len() + 1);
                    full_args.push(env_ptr.into());
                    full_args.extend(arg_metas.iter().cloned());

                    // Build function type: (env_ptr, args...) -> ret
                    // Get the return type from the func operand's type
                    let func_ty = self.get_operand_type(func, body);
                    let i8_ptr_ty = self.context.i8_type().ptr_type(AddressSpace::default());

                    let fn_type = if let crate::hir::TypeKind::Fn { params, ret } = func_ty.kind() {
                        let mut param_types: Vec<BasicMetadataTypeEnum> = Vec::with_capacity(params.len() + 1);
                        param_types.push(i8_ptr_ty.into()); // env_ptr
                        for p in params {
                            param_types.push(self.lower_type(p).into());
                        }
                        if ret.is_unit() {
                            self.context.void_type().fn_type(&param_types, false)
                        } else {
                            let ret_type = self.lower_type(ret);
                            ret_type.fn_type(&param_types, false)
                        }
                    } else {
                        return Err(vec![Diagnostic::error(
                            "Expected Fn type for indirect call", span
                        )]);
                    };

                    // Cast fn_ptr to the correct function pointer type
                    let typed_fn_ptr = self.builder.build_pointer_cast(
                        fn_ptr,
                        fn_type.ptr_type(AddressSpace::default()),
                        "fn.typed"
                    ).map_err(|e| vec![Diagnostic::error(
                        format!("LLVM pointer cast error: {}", e), span
                    )])?;

                    let callable = inkwell::values::CallableValue::try_from(typed_fn_ptr)
                        .map_err(|_| vec![Diagnostic::error(
                            "Invalid function pointer for call", span
                        )])?;

                    self.builder.build_call(callable, &full_args, "call_result")
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
            let dest_ptr = self.compile_mir_place(destination, body, escape_results)?;
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

                // Bitcast i64* to struct*
                let struct_ptr = self.builder.build_pointer_cast(
                    temp_alloca,
                    dest_struct_ty.ptr_type(inkwell::AddressSpace::default()),
                    "perform_struct_ptr"
                ).map_err(|e| vec![Diagnostic::error(
                    format!("LLVM pointer_cast error: {}", e), span
                )])?;

                // Load the struct value
                self.builder.build_load(struct_ptr, "perform_struct")
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM load error: {}", e), span
                    )])?
            } else {
                // For other types, use the value directly
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
