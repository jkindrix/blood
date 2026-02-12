//! Closure compilation for codegen.
//!
//! This module handles compilation of closures:
//! - Environment struct creation for captures
//! - Closure function generation
//! - Fat pointer creation { fn_ptr, env_ptr }

use inkwell::values::BasicValueEnum;
use inkwell::types::{BasicType, BasicTypeEnum};
use inkwell::AddressSpace;

use crate::hir::{self, Type};
use crate::diagnostics::Diagnostic;
use crate::span::Span;
use crate::ice_err;

use super::CodegenContext;

impl<'ctx, 'a> CodegenContext<'ctx, 'a> {
    /// Compile a closure expression: `|x| x + 1`
    ///
    /// Closures are compiled as:
    /// 1. An environment struct containing captured variables
    /// 2. A function that takes the environment as its first parameter
    /// 3. A fat pointer struct containing (fn_ptr, env_ptr)
    pub(super) fn compile_closure(
        &mut self,
        body_id: hir::BodyId,
        captures: &[hir::Capture],
        closure_ty: &Type,
        span: Span,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        use crate::hir::TypeKind;

        // Get the closure body
        let body = self.closure_bodies.get(&body_id).cloned()
            .ok_or_else(|| vec![Diagnostic::error(
                format!("Closure body not found: {:?}", body_id),
                span,
            )])?;

        // Generate a unique name for this closure
        let closure_name = format!("__closure_{}", self.closure_counter);
        self.closure_counter += 1;

        // Get closure parameter and return types from the closure type
        let (param_types, return_ty): (Vec<Type>, Type) = match closure_ty.kind() {
            TypeKind::Fn { params, ret, .. } => {
                (params.clone(), (*ret).clone())
            }
            _ => {
                // Infer from body if type isn't Fn
                let params: Vec<Type> = body.params()
                    .map(|l| l.ty.clone())
                    .collect();
                let return_ty = body.expr.ty.clone();
                (params, return_ty)
            }
        };

        // Create environment struct type from captures using actual types
        let i8_ptr_type = self.context.ptr_type(AddressSpace::default());
        let mut env_types: Vec<BasicTypeEnum<'ctx>> = Vec::with_capacity(captures.len());
        for cap in captures {
            if let Some(alloca) = self.locals.get(&cap.local_id) {
                // With opaque pointers, recover the allocated type from the alloca instruction
                let alloca_type: BasicTypeEnum<'ctx> = alloca
                    .as_instruction()
                    .and_then(|inst| inst.get_allocated_type().ok())
                    .ok_or_else(|| vec![ice_err!(
                        span,
                        "closure capture alloca has no recoverable type";
                        "local_id" => cap.local_id
                    )])?;
                env_types.push(alloca_type);
            }
            // Note: If local_id not found, we skip it. This matches the previous
            // filter_map behavior but could potentially be an error condition too.
        }

        let env_struct_type = if env_types.is_empty() {
            self.context.struct_type(&[], false)
        } else {
            self.context.struct_type(&env_types, false)
        };

        // Create function type: (env_ptr, params...) -> return_type
        let mut fn_param_types: Vec<BasicTypeEnum<'ctx>> = vec![i8_ptr_type.into()];
        for param_ty in &param_types {
            fn_param_types.push(self.lower_type(param_ty));
        }

        let fn_type = if return_ty.is_unit() {
            self.context.void_type().fn_type(
                &fn_param_types.iter().map(|t| (*t).into()).collect::<Vec<_>>(),
                false
            )
        } else {
            let ret_llvm = self.lower_type(&return_ty);
            ret_llvm.fn_type(
                &fn_param_types.iter().map(|t| (*t).into()).collect::<Vec<_>>(),
                false
            )
        };

        // Create the closure function
        let fn_value = self.module.add_function(&closure_name, fn_type, None);

        // Save current function context
        let saved_fn = self.current_fn;
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_local_types = std::mem::take(&mut self.local_types);
        let saved_loop_stack = std::mem::take(&mut self.loop_stack);

        // Compile the closure body
        self.current_fn = Some(fn_value);
        let entry = self.context.append_basic_block(fn_value, "entry");
        self.builder.position_at_end(entry);

        // Set up environment access - first parameter is the environment pointer
        // With LLVM 18 opaque pointers, env_ptr is already an opaque ptr - no cast needed
        let env_ptr = fn_value.get_first_param()
            .ok_or_else(|| vec![Diagnostic::error("Closure missing env parameter", span)])?
            .into_pointer_value();

        // Map captured variables from environment struct
        for (i, cap) in captures.iter().enumerate() {
            if let Some(&outer_alloca) = saved_locals.get(&cap.local_id) {
                // With opaque pointers, recover the allocated type from the alloca instruction
                let cap_basic_type: BasicTypeEnum<'ctx> = outer_alloca
                    .as_instruction()
                    .and_then(|inst| inst.get_allocated_type().ok())
                    .ok_or_else(|| vec![ice_err!(
                        span,
                        "captured variable alloca has no recoverable type";
                        "local_id" => cap.local_id
                    )])?;

                let zero = self.context.i32_type().const_int(0, false);
                let idx = self.context.i32_type().const_int(i as u64, false);
                let cap_ptr = unsafe {
                    self.builder.build_gep(env_struct_type, env_ptr, &[zero, idx], "cap.ptr")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                };
                let local_alloca = self.builder
                    .build_alloca(cap_basic_type, "cap.local")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                let cap_val = self.builder.build_load(cap_basic_type, cap_ptr, "cap.val")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                self.builder.build_store(local_alloca, cap_val)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                self.locals.insert(cap.local_id, local_alloca);
                self.local_types.insert(cap.local_id, cap_basic_type);
            }
        }

        // Set up parameters (skip first env param)
        let param_locals: Vec<_> = body.params().collect();
        for (i, local) in param_locals.iter().enumerate() {
            let param_idx = (i + 1) as u32;
            if let Some(param_val) = fn_value.get_nth_param(param_idx) {
                let param_llvm_type: BasicTypeEnum = param_val.get_type();
                let alloca = self.builder
                    .build_alloca(param_llvm_type, &format!("param.{}", i))
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                self.builder.build_store(alloca, param_val)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                self.locals.insert(local.id, alloca);
                self.local_types.insert(local.id, param_llvm_type);
            }
        }

        // Compile the body expression
        let result = self.compile_expr(&body.expr)?;

        // Return the result
        if return_ty.is_unit() || result.is_none() {
            self.builder.build_return(None)
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
        } else if let Some(ret_val) = result {
            self.builder.build_return(Some(&ret_val))
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
        }

        // Set WeakODR linkage now that the closure function has a body.
        // This allows the linker to merge duplicate definitions when the same
        // closure is compiled into multiple object files (per-definition mode).
        // We use WeakODR instead of LinkOnceODR because LinkOnceODR can be
        // stripped by LLVM optimization when there are no local callers.
        use inkwell::module::Linkage;
        fn_value.set_linkage(Linkage::WeakODR);

        // Restore context
        self.current_fn = saved_fn;
        self.locals = saved_locals;
        self.local_types = saved_local_types;
        self.loop_stack = saved_loop_stack;

        // Position builder back at saved location
        if let Some(fn_val) = saved_fn {
            if let Some(last_block) = fn_val.get_last_basic_block() {
                self.builder.position_at_end(last_block);
            }
        }

        // Create environment and populate with captured values
        let env_alloca = self.builder
            .build_alloca(env_struct_type, "closure.env")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

        for (i, cap) in captures.iter().enumerate() {
            if let Some(&cap_ptr) = self.locals.get(&cap.local_id) {
                let zero = self.context.i32_type().const_int(0, false);
                let idx = self.context.i32_type().const_int(i as u64, false);
                let field_ptr = unsafe {
                    self.builder.build_gep(env_struct_type, env_alloca, &[zero, idx], "env.field")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                };
                // Load the captured value and store directly with its actual type
                // Recover the alloca's type for the load instruction (opaque pointers)
                let alloca_ty = cap_ptr
                    .as_instruction()
                    .and_then(|inst| inst.get_allocated_type().ok())
                    .ok_or_else(|| vec![ice_err!(
                        span,
                        "local alloca has no recoverable type for capture load";
                        "local_id" => cap.local_id
                    )])?;
                let val = self.builder.build_load(alloca_ty, cap_ptr, "cap.val")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                self.builder.build_store(field_ptr, val)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
            }
        }

        // Create closure struct: { fn_ptr, env_ptr }
        let fn_ptr = fn_value.as_global_value().as_pointer_value();
        let fn_ptr_as_i8 = self.builder
            .build_pointer_cast(fn_ptr, i8_ptr_type, "fn.ptr")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
        let env_ptr_as_i8 = self.builder
            .build_pointer_cast(env_alloca, i8_ptr_type, "env.ptr")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

        let closure_struct_type = self.context.struct_type(&[i8_ptr_type.into(), i8_ptr_type.into()], false);
        let mut closure_val = closure_struct_type.get_undef();
        closure_val = self.builder
            .build_insert_value(closure_val, fn_ptr_as_i8, 0, "closure.fn")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
            .into_struct_value();
        closure_val = self.builder
            .build_insert_value(closure_val, env_ptr_as_i8, 1, "closure.env")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
            .into_struct_value();

        Ok(Some(closure_val.into()))
    }

    // NOTE: Generational pointer operations (compile_deref, compile_borrow, compile_addr_of)
    // are defined in expr.rs since they're expression-level operations.
    // The emit_generation_check and emit_generation_check_or_panic functions
    // were removed because the MIR codegen path has its own implementation
    // (see mir_codegen.rs: emit_generation_check). The deprecated HIR path
    // does not emit generation checks.
}
