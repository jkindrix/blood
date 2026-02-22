//! Handler compilation for effect handlers.
//!
//! This module handles compilation of effect handler operations,
//! return clauses, and runtime registration.

use inkwell::values::{FunctionValue, BasicValueEnum};
use inkwell::types::BasicTypeEnum;
use inkwell::AddressSpace;

use crate::hir::{self, DefId, LocalId, TypeKind};
use crate::diagnostics::Diagnostic;


use super::CodegenContext;

impl<'ctx, 'a> CodegenContext<'ctx, 'a> {
    /// Compute LLVM types for handler state struct fields.
    ///
    /// Concrete types use their actual LLVM type; unresolved generic type
    /// parameters fall back to i64 (preserving current behavior for generic handlers).
    ///
    /// All codegen paths touching handler state (compile_handler_op_body,
    /// compile_return_clause, and PushHandler in statement.rs) MUST use this
    /// to ensure layout agreement between the caller-side shadow alloca and
    /// the handler body's state struct.
    pub(crate) fn handler_state_field_types(&self, state_fields: &[hir::HandlerState]) -> Vec<BasicTypeEnum<'ctx>> {
        let i64_type: BasicTypeEnum<'ctx> = self.context.i64_type().into();
        state_fields.iter().map(|field| {
            if field.ty.has_type_vars() {
                i64_type
            } else {
                self.lower_type(&field.ty)
            }
        }).collect()
    }

    /// Declare handler operation functions (Phase 2: Effect Handlers).
    ///
    /// Each handler operation is compiled to a function with signature:
    /// `fn(state: *mut void, args: *const i64, arg_count: i64, continuation: i64) -> i64`
    ///
    /// The continuation parameter is used for non-tail-resumptive handlers (deep handlers).
    /// For tail-resumptive handlers (shallow), the continuation is 0 (unused).
    pub fn declare_handler_operations(&mut self, hir_crate: &hir::Crate) -> Result<(), Vec<Diagnostic>> {
        let i64_type = self.context.i64_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());

        // Handler operation function signature:
        // fn(state: *mut void, args: *const i64, arg_count: i64, continuation: i64) -> i64
        let handler_op_type = i64_type.fn_type(
            &[ptr_type.into(), ptr_type.into(), i64_type.into(), i64_type.into()],
            false,
        );

        // Sort items by DefId for deterministic declaration order.
        let mut sorted_items: Vec<_> = hir_crate.items.iter().collect();
        sorted_items.sort_by_key(|(&def_id, _)| def_id.index());
        for (&def_id, item) in &sorted_items {
            if let hir::ItemKind::Handler { operations, .. } = &item.kind {
                for (op_idx, handler_op) in operations.iter().enumerate() {
                    let fn_name = format!("{}_{}", item.name, handler_op.name);
                    let fn_value = self.module.add_function(&fn_name, handler_op_type, None);
                    // Set external linkage so linker resolves from other object files
                    fn_value.set_linkage(inkwell::module::Linkage::External);
                    self.handler_ops.insert((def_id, op_idx), fn_value);
                }
            }
        }

        Ok(())
    }

    /// Compile a single handler item (for per-definition compilation).
    pub fn compile_handler_item(
        &mut self,
        def_id: DefId,
        item: &hir::Item,
        hir_crate: &hir::Crate,
    ) -> Result<(), Vec<Diagnostic>> {
        if let hir::ItemKind::Handler { operations, return_clause, state, .. } = &item.kind {
            // Compile each operation (both tail-resumptive and non-tail-resumptive)
            // Non-tail-resumptive handlers use continuation capture via blood_continuation_resume
            for (op_idx, handler_op) in operations.iter().enumerate() {
                if let Some(&fn_value) = self.handler_ops.get(&(def_id, op_idx)) {
                    if let Some(body) = hir_crate.bodies.get(&handler_op.body_id) {
                        self.compile_handler_op_body(fn_value, body, handler_op, state)?;
                    }
                }
            }

            // Compile return clause if present
            if let Some(ret_clause) = return_clause {
                if let Some(body) = hir_crate.bodies.get(&ret_clause.body_id) {
                    self.compile_return_clause(def_id, &item.name, body, ret_clause, state)?;
                }
            }
        }
        Ok(())
    }

    /// Compile handler operation bodies (Phase 2: Effect Handlers).
    ///
    /// Both tail-resumptive and non-tail-resumptive handlers are supported:
    /// - Tail-resumptive: resume is a simple return
    /// - Non-tail-resumptive: resume calls blood_continuation_resume
    pub fn compile_handler_operations(&mut self, hir_crate: &hir::Crate) -> Result<(), Vec<Diagnostic>> {
        // Sort items by DefId for deterministic compilation order.
        let mut sorted_items: Vec<_> = hir_crate.items.iter().collect();
        sorted_items.sort_by_key(|(&def_id, _)| def_id.index());
        // Compile all handler operations
        for (&def_id, item) in &sorted_items {
            if let hir::ItemKind::Handler { operations, return_clause, state, .. } = &item.kind {
                // Compile each operation
                for (op_idx, handler_op) in operations.iter().enumerate() {
                    if let Some(&fn_value) = self.handler_ops.get(&(def_id, op_idx)) {
                        if let Some(body) = hir_crate.bodies.get(&handler_op.body_id) {
                            self.compile_handler_op_body(fn_value, body, handler_op, state)?;
                        }
                    }
                }

                // Compile return clause if present
                if let Some(ret_clause) = return_clause {
                    if let Some(body) = hir_crate.bodies.get(&ret_clause.body_id) {
                        self.compile_return_clause(def_id, &item.name, body, ret_clause, state)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Compile a handler's return clause.
    ///
    /// The return clause transforms the final result of the handled computation.
    /// Signature: fn(result: i64, state_ptr: *mut void) -> i64
    pub(super) fn compile_return_clause(
        &mut self,
        _handler_id: DefId,
        _handler_name: &str,
        body: &hir::Body,
        _return_clause: &hir::ReturnClause,
        state_fields: &[hir::HandlerState],
    ) -> Result<(), Vec<Diagnostic>> {
        let span = body.span;
        let i64_type = self.context.i64_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());

        // Return clause function signature: fn(result: i64, state_ptr: *void) -> i64
        let ret_clause_type = i64_type.fn_type(&[i64_type.into(), ptr_type.into()], false);
        // Use handler name for content-based naming (enables cache sharing across files)
        let fn_name = format!("{}_return", _handler_name);

        // Check if function was already declared (e.g., by compile_handle when compiling
        // the 'with ... handle' expression). If so, reuse it; otherwise create it.
        let fn_value = self.module.get_function(&fn_name).unwrap_or_else(|| {
            self.module.add_function(&fn_name, ret_clause_type, None)
        });

        // Create entry block
        let entry_block = self.context.append_basic_block(fn_value, "entry");
        self.builder.position_at_end(entry_block);

        // Save and set context
        let saved_fn = self.current_fn;
        let saved_locals = std::mem::take(&mut self.locals);
        self.current_fn = Some(fn_value);

        // Get the result parameter and bind to the param local
        let result_param = fn_value.get_nth_param(0)
            .ok_or_else(|| vec![Diagnostic::error("Missing result parameter".to_string(), span)])?;

        // Get the state pointer parameter
        let state_ptr = fn_value.get_nth_param(1)
            .ok_or_else(|| vec![Diagnostic::error("Missing state pointer parameter".to_string(), span)])?;

        // Build the handler state struct type using TYPED layout.
        // Same as compile_handler_op_body: concrete types use their actual LLVM
        // type; unresolved generic type parameters fall back to i64.
        let state_field_types: Vec<BasicTypeEnum> = self.handler_state_field_types(state_fields);
        let state_struct_type = self.context.struct_type(&state_field_types, false);

        // With opaque pointers, all pointer types are `ptr` — no cast needed,
        // but we keep the variable for clarity in GEP operations.
        let typed_state_ptr = state_ptr.into_pointer_value();

        // Build a map of state field names to their index in the struct
        let state_field_indices: std::collections::HashMap<&str, u32> = state_fields.iter()
            .enumerate()
            .map(|(i, s)| (s.name.as_str(), i as u32))
            .collect();

        // Set up ALL locals from the body (except return place at index 0)
        // Return clause bodies have: [return_place, param(x), state_fields...]
        for local in &body.locals {
            // Skip return place
            if local.id == LocalId::RETURN_PLACE {
                continue;
            }

            let local_type = self.lower_type(&local.ty);
            let local_name = local.name.as_deref().unwrap_or("local");

            // For unresolved types (Param/Infer/Error), lower_type returns i8 or i8*
            // which is wrong — it truncates values. Use i64 instead.
            let is_unresolved = matches!(local.ty.kind(),
                TypeKind::Param(_) | TypeKind::Infer(_) | TypeKind::Error);
            let effective_type: BasicTypeEnum = if is_unresolved {
                i64_type.into()
            } else {
                local_type
            };

            // Check if this local corresponds to a state field
            if let Some(&field_idx) = state_field_indices.get(local_name) {
                // This is a state field - load its value from the typed state struct.
                let field_ptr = self.builder
                    .build_struct_gep(state_struct_type, typed_state_ptr, field_idx, &format!("{}_ptr", local_name))
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

                let alloca = self.builder
                    .build_alloca(effective_type, local_name)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

                let state_field_ty = state_field_types[field_idx as usize];

                // If the state struct field type matches the effective local type,
                // load directly (no conversion needed). This handles aggregate types
                // like String ({i8*, i64, i64}) correctly.
                if state_field_ty == effective_type {
                    let loaded = self.builder
                        .build_load(effective_type, field_ptr, &format!("{}_val", local_name))
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                    self.builder.build_store(alloca, loaded)
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                } else {
                    // Generic fallback: state field is i64, convert to effective type.
                    let i64_value = self.builder
                        .build_load(i64_type, field_ptr, &format!("{}_i64_val", local_name))
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

                    let converted = if effective_type.is_int_type() {
                        let target_int = effective_type.into_int_type();
                        let i64_int = i64_value.into_int_value();
                        if target_int.get_bit_width() == 64 {
                            i64_value
                        } else if target_int.get_bit_width() < 64 {
                            self.builder.build_int_truncate(i64_int, target_int, &format!("{}_trunc", local_name))
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                                .into()
                        } else {
                            self.builder.build_int_s_extend(i64_int, target_int, &format!("{}_sext", local_name))
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                                .into()
                        }
                    } else if effective_type.is_pointer_type() {
                        self.builder.build_int_to_ptr(i64_value.into_int_value(), effective_type.into_pointer_type(), &format!("{}_ptr", local_name))
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                            .into()
                    } else if effective_type.is_float_type() {
                        self.builder.build_bit_cast(i64_value, effective_type, &format!("{}_float", local_name))
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                    } else {
                        return Err(vec![Diagnostic::error(
                            format!(
                                "ICE: handler state field '{}' has i64 layout but local has non-scalar effective type {:?}; \
                                 cannot convert. This indicates a mismatch between handler state layout and body local types.",
                                local_name, effective_type
                            ),
                            span,
                        )]);
                    };
                    self.builder.build_store(alloca, converted)
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                }

                self.locals.insert(local.id, alloca);
                self.local_types.insert(local.id, effective_type);
                continue;
            }

            // For the result param local, bind to the result parameter
            let alloca = self.builder
                .build_alloca(effective_type, local_name)
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

            // Check if this is the result parameter (first param local)
            let param_locals: Vec<_> = body.params().collect();
            if param_locals.first().map(|p| p.id) == Some(local.id) {
                // Convert from i64 if needed
                let converted_val = if effective_type.is_int_type() {
                    let int_type = effective_type.into_int_type();
                    if int_type.get_bit_width() == 64 {
                        result_param
                    } else {
                        self.builder
                            .build_int_truncate(result_param.into_int_value(), int_type, "ret_trunc")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                            .into()
                    }
                } else {
                    result_param
                };

                self.builder.build_store(alloca, converted_val)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
            } else {
                // Initialize with zero/default for other locals
                if effective_type.is_int_type() {
                    let zero_val = effective_type.into_int_type().const_zero();
                    self.builder.build_store(alloca, zero_val)
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                }
            }

            self.locals.insert(local.id, alloca);
            self.local_types.insert(local.id, effective_type);
        }

        // Compile the return clause body
        let result = self.compile_expr(&body.expr)?;

        // Return the transformed result as i64
        if let Some(ret_val) = result {
            let ret_i64 = match ret_val {
                BasicValueEnum::IntValue(iv) => {
                    if iv.get_type().get_bit_width() == 64 {
                        iv
                    } else {
                        self.builder
                            .build_int_s_extend(iv, i64_type, "ret_ext")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                    }
                }
                BasicValueEnum::PointerValue(pv) => {
                    self.builder
                        .build_ptr_to_int(pv, i64_type, "ret_ptr_int")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                }
                BasicValueEnum::FloatValue(fv) => {
                    // Bitcast float to i64 for passing through uniform interface
                    self.builder
                        .build_bit_cast(fv, i64_type, "float_as_i64")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                        .into_int_value()
                }
                BasicValueEnum::StructValue(sv) => {
                    // For small structs that fit in 64 bits, pack the bits directly into i64.
                    // This avoids lifetime issues with stack-allocated pointers.
                    let struct_type = sv.get_type();
                    let alloca = self.builder
                        .build_alloca(struct_type, "ret_struct")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                    self.builder.build_store(alloca, sv)
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

                    // With opaque pointers, load as i64 directly from the alloca
                    self.builder
                        .build_load(i64_type, alloca, "ret_struct_bits")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                        .into_int_value()
                }
                BasicValueEnum::ArrayValue(av) => {
                    // Array return: allocate on stack and return pointer as i64
                    let array_type = av.get_type();
                    let alloca = self.builder
                        .build_alloca(array_type, "ret_array")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                    self.builder.build_store(alloca, av)
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                    self.builder
                        .build_ptr_to_int(alloca, i64_type, "ret_array_ptr")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                }
                BasicValueEnum::VectorValue(_) | BasicValueEnum::ScalableVectorValue(_) => {
                    // SIMD and scalable vectors are not expected in normal handler returns
                    return Err(vec![Diagnostic::error(
                        "Vector and scalable vector return types not supported in handler return clauses".to_string(),
                        span,
                    )]);
                }
            };
            self.builder.build_return(Some(&ret_i64))
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
        } else {
            // Return the input unchanged for unit type
            self.builder.build_return(Some(&result_param.into_int_value()))
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
        }

        // Set WeakODR linkage now that the return clause function has a body.
        // This allows the linker to merge duplicate definitions when the same
        // handler is compiled into multiple object files (per-definition mode).
        // We use WeakODR instead of LinkOnceODR because LinkOnceODR can be
        // stripped by LLVM optimization when there are no local callers.
        use inkwell::module::Linkage;
        fn_value.set_linkage(Linkage::WeakODR);

        // Restore context
        self.current_fn = saved_fn;
        self.locals = saved_locals;

        Ok(())
    }

    /// Compile a single handler operation body.
    pub(super) fn compile_handler_op_body(
        &mut self,
        fn_value: FunctionValue<'ctx>,
        body: &hir::Body,
        _handler_op: &hir::HandlerOp,
        state_fields: &[hir::HandlerState],
    ) -> Result<(), Vec<Diagnostic>> {
        let span = body.span;

        // Analyze tail-resumptive status using the thorough codegen-level analysis.
        // This tracks tail position through the full expression tree.
        let is_tail_resumptive = super::effects::is_handler_tail_resumptive(body);

        // Detect if this is a multi-shot handler (has multiple resume calls)
        let resume_count = crate::effects::handler::count_resumes_in_expr(&body.expr);
        let is_multishot = resume_count > 1;

        if std::env::var("BLOOD_DEBUG_EFFECTS").is_ok() {
            eprintln!(
                "DEBUG compile_handler_op_body: tail_resumptive={}, resume_count={}, multishot={}",
                is_tail_resumptive, resume_count, is_multishot
            );
        }

        // Create entry block
        let entry_block = self.context.append_basic_block(fn_value, "entry");
        self.builder.position_at_end(entry_block);

        // Save and set context
        let saved_fn = self.current_fn;
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_continuation = self.current_continuation.take();
        let saved_is_multishot = self.is_multishot_handler;
        self.current_fn = Some(fn_value);
        self.is_multishot_handler = is_multishot;

        // Get parameters: (state: *mut void, args: *const i64, arg_count: i64, continuation: i64)
        let state_ptr = fn_value.get_nth_param(0)
            .ok_or_else(|| vec![Diagnostic::error("Missing state parameter".to_string(), span)])?;
        let args_ptr = fn_value.get_nth_param(1)
            .ok_or_else(|| vec![Diagnostic::error("Missing args parameter".to_string(), span)])?;
        let _arg_count_param = fn_value.get_nth_param(2)
            .ok_or_else(|| vec![Diagnostic::error("Missing arg_count parameter".to_string(), span)])?;
        let continuation_param = fn_value.get_nth_param(3)
            .ok_or_else(|| vec![Diagnostic::error("Missing continuation parameter".to_string(), span)])?;

        let i64_type = self.context.i64_type();

        // Store continuation parameter for use by compile_resume
        let cont_alloca = self.builder
            .build_alloca(i64_type, "continuation")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
        self.builder.build_store(cont_alloca, continuation_param)
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
        self.current_continuation = Some(cont_alloca);

        // Build the handler state struct type using TYPED layout.
        // Concrete types use their actual LLVM type; unresolved generic type
        // parameters (TypeKind::Param) fall back to i64. This enables aggregate
        // types like String ({i8*, i64, i64}) to be stored/loaded correctly
        // while preserving backward compatibility for generic handlers.
        let state_field_types: Vec<BasicTypeEnum> = self.handler_state_field_types(state_fields);
        let state_struct_type = self.context.struct_type(&state_field_types, false);

        // With opaque pointers, all pointer types are `ptr` — no cast needed,
        // but we keep the variable for clarity in GEP operations.
        let typed_state_ptr = state_ptr.into_pointer_value();

        // Build a map of state field names to their index in the struct
        let state_field_indices: std::collections::HashMap<&str, u32> = state_fields.iter()
            .enumerate()
            .map(|(i, s)| (s.name.as_str(), i as u32))
            .collect();

        // Set up ALL locals from the body (except return place at index 0)
        // Handler operation bodies have: [return_place, state_fields..., params..., resume]
        for local in &body.locals {
            // Skip return place
            if local.id == LocalId::RETURN_PLACE {
                continue;
            }

            let local_type = self.lower_type(&local.ty);
            let local_name = local.name.as_deref().unwrap_or("local");

            // For unresolved types (Param/Infer/Error), lower_type returns i8 or i8*
            // which is wrong — it truncates values. Use i64 instead, matching the
            // uniform protocol width. For concrete types (i32, etc.), keep the real type.
            let is_unresolved = matches!(local.ty.kind(),
                TypeKind::Param(_) | TypeKind::Infer(_) | TypeKind::Error);
            let effective_type: BasicTypeEnum = if is_unresolved {
                i64_type.into()
            } else {
                local_type
            };

            // Check if this is a "resume" local - it's a function type
            if local.name.as_deref() == Some("resume") {
                // Resume is a placeholder - we'll handle resume calls specially
                // Create an alloca for it but don't initialize it (it's not a real value)
                let alloca = self.builder
                    .build_alloca(i64_type, "resume_placeholder")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                self.locals.insert(local.id, alloca);
                self.local_types.insert(local.id, i64_type.into());
                continue;
            }

            // Check if this local corresponds to a state field
            if let Some(&field_idx) = state_field_indices.get(local_name) {
                // This is a state field - load its value from the typed state struct.
                let field_ptr = self.builder
                    .build_struct_gep(state_struct_type, typed_state_ptr, field_idx, &format!("{}_ptr", local_name))
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

                let alloca = self.builder
                    .build_alloca(effective_type, local_name)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

                let state_field_ty = state_field_types[field_idx as usize];

                // If the state struct field type matches the effective local type,
                // load directly (no conversion needed). This handles aggregate types
                // like String ({i8*, i64, i64}) correctly.
                if state_field_ty == effective_type {
                    let loaded = self.builder
                        .build_load(effective_type, field_ptr, &format!("{}_val", local_name))
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                    self.builder.build_store(alloca, loaded)
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                } else {
                    // Generic fallback: state field is i64, convert to effective type.
                    let i64_value = self.builder
                        .build_load(i64_type, field_ptr, &format!("{}_i64_val", local_name))
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

                    let converted = if effective_type.is_int_type() {
                        let target_int = effective_type.into_int_type();
                        let i64_int = i64_value.into_int_value();
                        if target_int.get_bit_width() == 64 {
                            i64_value
                        } else if target_int.get_bit_width() < 64 {
                            self.builder.build_int_truncate(i64_int, target_int, &format!("{}_trunc", local_name))
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                                .into()
                        } else {
                            self.builder.build_int_s_extend(i64_int, target_int, &format!("{}_sext", local_name))
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                                .into()
                        }
                    } else if effective_type.is_pointer_type() {
                        self.builder.build_int_to_ptr(i64_value.into_int_value(), effective_type.into_pointer_type(), &format!("{}_ptr", local_name))
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                            .into()
                    } else if effective_type.is_float_type() {
                        self.builder.build_bit_cast(i64_value, effective_type, &format!("{}_float", local_name))
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                    } else {
                        return Err(vec![Diagnostic::error(
                            format!(
                                "ICE: handler state field '{}' has i64 layout but local has non-scalar effective type {:?}; \
                                 cannot convert. This indicates a mismatch between handler state layout and body local types.",
                                local_name, effective_type
                            ),
                            span,
                        )]);
                    };
                    self.builder.build_store(alloca, converted)
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                }

                self.locals.insert(local.id, alloca);
                self.local_types.insert(local.id, effective_type);
                continue;
            }

            // For other locals (operation params, temporaries), allocate with effective type
            let alloca = self.builder
                .build_alloca(effective_type, local_name)
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

            // Initialize with zero/default
            if effective_type.is_int_type() {
                let zero_val = effective_type.into_int_type().const_zero();
                self.builder.build_store(alloca, zero_val)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
            }

            self.locals.insert(local.id, alloca);
            self.local_types.insert(local.id, effective_type);
        }

        // Extract operation parameters from args_ptr
        // The effect operation's parameters are passed as an array of i64 values
        // We need to identify which locals are operation params (not state fields, not resume)
        let args_ptr_val = args_ptr.into_pointer_value();
        let i32_type = self.context.i32_type();
        let _zero_i32 = i32_type.const_zero();

        // Find operation parameters: params that are NOT state fields and NOT resume
        // NOTE: We iterate through all locals (except return place) and identify op params.
        // The locals order is: [return, state_fields..., op_params..., resume, body_locals...]
        // body.param_count is the number of operation parameters.
        // We need to process exactly param_count non-state-field, non-resume locals.
        let mut arg_index: u32 = 0;
        let op_param_count = body.param_count;
        for local in body.locals.iter().skip(1) {  // Skip return place at index 0
            // Stop once we've processed all operation parameters
            if arg_index as usize >= op_param_count {
                break;
            }

            let param_name = local.name.as_deref().unwrap_or("");

            // Skip state fields and resume - they're not operation parameters
            if state_field_indices.contains_key(param_name) || param_name == "resume" {
                continue;
            }

            // This is an operation parameter - load it from args_ptr
            if let Some(&alloca) = self.locals.get(&local.id) {
                // Get pointer to args[arg_index]
                // args_ptr is a pointer to i64 values, so we only need one index
                let idx = i64_type.const_int(arg_index as u64, false);
                let arg_ptr = unsafe {
                    self.builder.build_gep(i64_type, args_ptr_val, &[idx], &format!("arg_{}_ptr", arg_index))
                }.map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

                // Load the argument value (as i64)
                let arg_val = self.builder
                    .build_load(i64_type, arg_ptr, &format!("arg_{}", arg_index))
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

                // Convert to match the parameter type.
                // Use i64 for unresolved types to avoid truncation.
                let is_param_unresolved = matches!(local.ty.kind(),
                    TypeKind::Param(_) | TypeKind::Infer(_) | TypeKind::Error);
                let param_type: BasicTypeEnum = if is_param_unresolved {
                    i64_type.into()
                } else {
                    self.lower_type(&local.ty)
                };
                let arg_int = arg_val.into_int_value();
                let final_val: BasicValueEnum = if param_type.is_int_type() {
                    let target_int_type = param_type.into_int_type();
                    if target_int_type.get_bit_width() < 64 {
                        self.builder
                            .build_int_truncate(arg_int, target_int_type, "arg_trunc")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                            .into()
                    } else if target_int_type.get_bit_width() > 64 {
                        self.builder
                            .build_int_s_extend(arg_int, target_int_type, "arg_ext")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                            .into()
                    } else {
                        arg_int.into()
                    }
                } else if param_type.is_pointer_type() {
                    // Convert i64 to pointer type
                    self.builder
                        .build_int_to_ptr(arg_int, param_type.into_pointer_type(), "arg_ptr")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                        .into()
                } else if param_type.is_struct_type() {
                    // For struct types, the i64 is a pointer to the struct - convert and load
                    let opaque_ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                    let struct_ptr = self.builder
                        .build_int_to_ptr(arg_int, opaque_ptr_type, "struct_arg_ptr")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                    self.builder
                        .build_load(param_type, struct_ptr, "struct_arg_val")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                } else if param_type.is_array_type() {
                    // For array types, the i64 is a pointer to the array - convert and load
                    let opaque_ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                    let array_ptr = self.builder
                        .build_int_to_ptr(arg_int, opaque_ptr_type, "array_arg_ptr")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                    self.builder
                        .build_load(param_type, array_ptr, "array_arg_val")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                } else if param_type.is_float_type() {
                    // Float was bitcast to i64, convert back
                    self.builder
                        .build_bit_cast(arg_int, param_type, "float_arg")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                } else {
                    // For other types, use the raw i64 (shouldn't normally happen)
                    arg_int.into()
                };

                // Store in the parameter's alloca
                self.builder.build_store(alloca, final_val)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

                arg_index += 1;
            }
        }

        // Compile the body expression
        let result = self.compile_expr(&body.expr)?;

        // Write back mutable state fields to the state struct
        // This ensures mutations during the handler op are visible to subsequent code
        for state_field in state_fields.iter() {
            if state_field.mutable {
                let field_name = state_field.name.as_str();
                if let Some(&field_idx) = state_field_indices.get(field_name) {
                    // Find the local that corresponds to this state field
                    let local = body.locals.iter().find(|l| l.name.as_deref() == Some(field_name));
                    if let Some(local) = local {
                        if let Some(&local_alloca) = self.locals.get(&local.id) {
                            // Get pointer to the state field
                            let field_ptr = self.builder
                                .build_struct_gep(state_struct_type, typed_state_ptr, field_idx, &format!("{}_writeback_ptr", field_name))
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

                            // Load current value from local (typed, e.g. i32)
                            // Compute the effective type matching what was stored in the alloca
                            let is_local_unresolved = matches!(local.ty.kind(),
                                TypeKind::Param(_) | TypeKind::Infer(_) | TypeKind::Error);
                            let local_effective_type: BasicTypeEnum = if is_local_unresolved {
                                i64_type.into()
                            } else {
                                self.lower_type(&local.ty)
                            };
                            let current_val = self.builder
                                .build_load(local_effective_type, local_alloca, &format!("{}_writeback_val", field_name))
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

                            let state_field_ty = state_field_types[field_idx as usize];

                            // If the state struct field type matches the loaded value type,
                            // store directly (no conversion needed). This handles aggregate
                            // types like String ({i8*, i64, i64}) correctly.
                            if state_field_ty == current_val.get_type() {
                                self.builder.build_store(field_ptr, current_val)
                                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                            } else {
                                // Generic fallback: state field is i64, convert value to i64.
                                let writeback_i64: BasicValueEnum = match current_val {
                                    BasicValueEnum::IntValue(iv) => {
                                        let bw = iv.get_type().get_bit_width();
                                        if bw == 64 {
                                            iv.into()
                                        } else if bw < 64 {
                                            self.builder.build_int_z_extend(iv, i64_type, &format!("{}_wb_zext", field_name))
                                                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                                                .into()
                                        } else {
                                            self.builder.build_int_truncate(iv, i64_type, &format!("{}_wb_trunc", field_name))
                                                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                                                .into()
                                        }
                                    }
                                    BasicValueEnum::PointerValue(pv) => {
                                        self.builder.build_ptr_to_int(pv, i64_type, &format!("{}_wb_ptoi", field_name))
                                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                                            .into()
                                    }
                                    BasicValueEnum::FloatValue(fv) => {
                                        self.builder.build_bit_cast(fv, i64_type, &format!("{}_wb_fcast", field_name))
                                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                                    }
                                    BasicValueEnum::StructValue(_) | BasicValueEnum::ArrayValue(_) | BasicValueEnum::VectorValue(_) | BasicValueEnum::ScalableVectorValue(_) => {
                                        return Err(vec![Diagnostic::error(
                                            format!(
                                                "ICE: handler state field '{}' has i64 layout but write-back value is aggregate type {:?} \
                                                 (struct, array, vector, or scalable vector); cannot convert to i64. This indicates a \
                                                 mismatch between handler state layout and body local types.",
                                                field_name, current_val.get_type()
                                            ),
                                            span,
                                        )]);
                                    }
                                };

                                self.builder.build_store(field_ptr, writeback_i64)
                                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                            }
                        }
                    }
                }
            }
        }

        // Check if the basic block already has a terminator (e.g., from resume)
        // If so, don't add another return
        let current_block = self.builder.get_insert_block()
            .ok_or_else(|| vec![Diagnostic::error("No current block".to_string(), span)])?;
        if current_block.get_terminator().is_some() {
            // Block already terminated (likely by resume), don't add another return
        } else {
            // Return result as i64
            if let Some(ret_val) = result {
                let ret_i64 = match ret_val {
                    BasicValueEnum::IntValue(iv) => {
                        if iv.get_type().get_bit_width() == 64 {
                            iv
                        } else {
                            self.builder
                                .build_int_s_extend(iv, i64_type, "ret_ext")
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                        }
                    }
                    BasicValueEnum::PointerValue(pv) => {
                        self.builder
                            .build_ptr_to_int(pv, i64_type, "ret_ptr_int")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                    }
                    BasicValueEnum::FloatValue(fv) => {
                        // Bitcast float to i64 for passing through uniform interface
                        self.builder
                            .build_bit_cast(fv, i64_type, "float_as_i64")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                            .into_int_value()
                    }
                    BasicValueEnum::StructValue(sv) => {
                        // For small structs that fit in 64 bits, pack the bits directly into i64.
                        // This avoids lifetime issues with stack-allocated pointers.
                        let struct_type = sv.get_type();
                        let alloca = self.builder
                            .build_alloca(struct_type, "ret_struct")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                        self.builder.build_store(alloca, sv)
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

                        // With opaque pointers, load as i64 directly from the alloca
                        self.builder
                            .build_load(i64_type, alloca, "ret_struct_bits")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                            .into_int_value()
                    }
                    BasicValueEnum::ArrayValue(av) => {
                        // Array return: allocate on stack and return pointer as i64
                        let array_type = av.get_type();
                        let alloca = self.builder
                            .build_alloca(array_type, "ret_array")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                        self.builder.build_store(alloca, av)
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                        self.builder
                            .build_ptr_to_int(alloca, i64_type, "ret_array_ptr")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                    }
                    BasicValueEnum::VectorValue(_) | BasicValueEnum::ScalableVectorValue(_) => {
                        return Err(vec![Diagnostic::error(
                            "Vector and scalable vector return types not supported in handler operations".to_string(),
                            span,
                        )]);
                    }
                };
                self.builder.build_return(Some(&ret_i64))
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
            } else {
                // Return 0 for unit type
                self.builder.build_return(Some(&i64_type.const_zero()))
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
            }
        }

        // Set WeakODR linkage now that the handler op function has a body.
        // This allows the linker to merge duplicate definitions when the same
        // handler is compiled into multiple object files (per-definition mode).
        // We use WeakODR instead of LinkOnceODR because LinkOnceODR can be
        // stripped by LLVM optimization when there are no local callers.
        use inkwell::module::Linkage;
        fn_value.set_linkage(Linkage::WeakODR);

        // Restore context
        self.current_fn = saved_fn;
        self.locals = saved_locals;
        self.current_continuation = saved_continuation;
        self.is_multishot_handler = saved_is_multishot;

        Ok(())
    }

    /// Register handlers with the runtime's effect registry.
    ///
    /// Generates code to call blood_evidence_register for each handler during
    /// module initialization.
    pub fn register_handlers_with_runtime(&mut self) -> Result<(), Vec<Diagnostic>> {
        // Skip if no handlers to register
        if self.handler_defs.is_empty() {
            return Ok(());
        }

        let i64_type = self.context.i64_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let void_type = self.context.void_type();

        // Get or declare the registration function
        let register_fn = self.module.get_function("blood_evidence_register")
            .unwrap_or_else(|| {
                let fn_type = void_type.fn_type(
                    &[
                        ptr_type.into(), // evidence handle
                        i64_type.into(), // effect_id
                        ptr_type.into(), // ops array (pointer to pointer array)
                        i64_type.into(), // op_count
                    ],
                    false,
                );
                self.module.add_function("blood_evidence_register", fn_type, None)
            });

        // Create a global constructor function to register handlers at startup
        // Use LinkOnceODR linkage so the linker can merge duplicates if this
        // function exists in multiple object files (e.g., both whole_module.o
        // and handler_registration.o).
        let init_fn_type = void_type.fn_type(&[], false);
        use inkwell::module::Linkage;
        let init_fn = self.module.add_function("__blood_register_handlers", init_fn_type, Some(Linkage::LinkOnceODR));

        let entry = self.context.append_basic_block(init_fn, "entry");
        self.builder.position_at_end(entry);

        // For each handler, build an operations array and register it
        for (handler_id, handler_info) in &self.handler_defs.clone() {
            let op_count = handler_info.op_impls.len();
            if op_count == 0 {
                continue;
            }

            // Create array of function pointers
            let array_type = ptr_type.array_type(op_count as u32);
            let ops_alloca = self.builder
                .build_alloca(array_type, "handler_ops")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

            let i32_type = self.context.i32_type();
            let zero = i32_type.const_zero();

            for op_idx in 0..op_count {
                if let Some(&fn_value) = self.handler_ops.get(&(*handler_id, op_idx)) {
                    let idx = i32_type.const_int(op_idx as u64, false);
                    let gep = unsafe {
                        self.builder.build_gep(array_type, ops_alloca, &[zero, idx], "op_ptr")
                    }.map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

                    // With opaque pointers, all pointers are the same type — no cast needed.
                    let fn_ptr = fn_value.as_global_value().as_pointer_value();

                    self.builder.build_store(gep, fn_ptr)
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
                }
            }

            // Get pointer to first element
            let ops_ptr = unsafe {
                self.builder.build_gep(array_type, ops_alloca, &[zero, zero], "ops_ptr")
            }.map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

            // Call blood_evidence_register(null, effect_id, ops, op_count)
            // Note: We pass null for evidence handle - registration is global
            let effect_id_val = i64_type.const_int(handler_info.effect_id.index as u64, false);
            let op_count_val = i64_type.const_int(op_count as u64, false);
            let null_ev = ptr_type.const_null();

            self.builder.build_call(
                register_fn,
                &[null_ev.into(), effect_id_val.into(), ops_ptr.into(), op_count_val.into()],
                "",
            ).map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
        }

        self.builder.build_return(None)
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

        // Add to llvm.global_ctors so it runs at program startup
        self.add_global_constructor(init_fn)?;

        Ok(())
    }

    /// Compile an inline handler operation body to an LLVM function.
    ///
    /// This is similar to compile_handler_op_body but works with inline handlers
    /// where we have the HIR expression directly rather than a Body with locals.
    ///
    /// Handler operation signature:
    /// `fn(state: *mut void, args: *const i64, arg_count: i64, continuation: i64) -> i64`
    pub fn compile_inline_handler_op_body(
        &mut self,
        synthetic_def_id: DefId,
        handler_body: &crate::mir::InlineHandlerBody,
    ) -> Result<FunctionValue<'ctx>, Vec<Diagnostic>> {
        use inkwell::values::BasicValueEnum;

        let span = handler_body.body.span;
        let i64_type = self.context.i64_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());

        // Handler operation function signature:
        // fn(state: *mut void, args: *const i64, arg_count: i64, continuation: i64) -> i64
        let handler_op_type = i64_type.fn_type(
            &[ptr_type.into(), ptr_type.into(), i64_type.into(), i64_type.into()],
            false,
        );

        // Generate unique function name for this inline handler operation
        // Include parent_def_id to ensure uniqueness across different functions
        // that handle the same effect operation
        let parent_path = self.def_paths.get(&handler_body.parent_def_id)
            .map(|p| p.replace("::", "$"))
            .unwrap_or_else(|| format!("{}", handler_body.parent_def_id.index()));
        let effect_path = self.def_paths.get(&handler_body.effect_id)
            .map(|p| p.replace("::", "$"))
            .unwrap_or_else(|| format!("{}", handler_body.effect_id.index()));
        let fn_name = format!(
            "blood_inline_handler${}${}${}${}",
            parent_path,
            effect_path,
            handler_body.op_name,
            synthetic_def_id.index()
        );

        // Check if function already exists (from previous compilation)
        if let Some(existing_fn) = self.module.get_function(&fn_name) {
            return Ok(existing_fn);
        }

        // Use LinkOnceODR linkage so the linker can merge identical handler
        // definitions when the same handler is compiled in multiple object files
        // (e.g., when monomorphized generic functions with inline handlers are
        // called from multiple closures).
        use inkwell::module::Linkage;
        let fn_value = self.module.add_function(&fn_name, handler_op_type, Some(Linkage::LinkOnceODR));

        // Detect if this is a multi-shot handler (has multiple resume calls)
        let resume_count = crate::effects::handler::count_resumes_in_expr(&handler_body.body);
        let is_multishot = resume_count > 1;

        // Create entry block
        let entry_block = self.context.append_basic_block(fn_value, "entry");
        self.builder.position_at_end(entry_block);

        // Save and set context
        let saved_fn = self.current_fn;
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_continuation = self.current_continuation.take();
        let saved_is_multishot = self.is_multishot_handler;
        self.current_fn = Some(fn_value);
        self.is_multishot_handler = is_multishot;

        // Get parameters: (state: *mut void, args: *const i64, arg_count: i64, continuation: i64)
        let state_ptr = fn_value.get_nth_param(0)
            .ok_or_else(|| vec![Diagnostic::error("Missing state parameter".to_string(), span)])?;
        let args_ptr = fn_value.get_nth_param(1)
            .ok_or_else(|| vec![Diagnostic::error("Missing args parameter".to_string(), span)])?;
        let _arg_count_param = fn_value.get_nth_param(2)
            .ok_or_else(|| vec![Diagnostic::error("Missing arg_count parameter".to_string(), span)])?;
        let continuation_param = fn_value.get_nth_param(3)
            .ok_or_else(|| vec![Diagnostic::error("Missing continuation parameter".to_string(), span)])?;

        // Store continuation parameter for use by compile_resume
        let cont_alloca = self.builder
            .build_alloca(i64_type, "continuation")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
        self.builder.build_store(cont_alloca, continuation_param)
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
        self.current_continuation = Some(cont_alloca);

        // Extract captures from state pointer
        // The state pointer points to a struct of pointers to captured variables
        if !handler_body.captures.is_empty() {
            let state_ptr_val = state_ptr.into_pointer_value();

            for (capture_idx, capture) in handler_body.captures.iter().enumerate() {
                // Get the element from the captures struct (array of pointers)
                // The state is laid out as raw bytes; use i8 as the pointee type for byte-offset GEP
                let i8_type = self.context.i8_type();
                let elem_ptr = unsafe {
                    self.builder.build_gep(
                        i8_type,
                        state_ptr_val,
                        &[i64_type.const_int(capture_idx as u64 * 8, false)],  // 8 bytes per pointer
                        &format!("capture_{}_ptr_ptr", capture_idx)
                    )
                }.map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

                // With opaque pointers, no pointer cast needed — load the pointer directly.
                // The elem_ptr points to a slot containing a pointer to the captured variable.
                let capture_type = self.lower_type(&capture.ty);
                let capture_ptr = self.builder
                    .build_load(ptr_type, elem_ptr, &format!("capture_{}_ptr", capture_idx))
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                    .into_pointer_value();

                // For mutable captures, we store the pointer directly so writes go through
                // For immutable captures, we could load the value, but storing the pointer is fine
                self.locals.insert(capture.local_id, capture_ptr);
                self.local_types.insert(capture.local_id, capture_type);
            }
        }

        // Extract operation parameters from args_ptr and bind to locals
        let args_ptr_val = args_ptr.into_pointer_value();
        for (idx, (param_id, param_ty)) in handler_body.params.iter()
            .zip(handler_body.param_types.iter())
            .enumerate()
        {
            let local_type = self.lower_type(param_ty);
            let alloca = self.builder
                .build_alloca(local_type, &format!("param_{}", idx))
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

            // Load the argument from args[idx]
            let arg_idx = i64_type.const_int(idx as u64, false);
            let arg_ptr = unsafe {
                self.builder.build_gep(i64_type, args_ptr_val, &[arg_idx], &format!("arg_{}_ptr", idx))
            }.map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

            let arg_val = self.builder
                .build_load(i64_type, arg_ptr, &format!("arg_{}", idx))
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

            // Convert from i64 to the actual parameter type
            let arg_int = arg_val.into_int_value();
            let final_val: BasicValueEnum = if local_type.is_int_type() {
                let target_int_type = local_type.into_int_type();
                if target_int_type.get_bit_width() < 64 {
                    self.builder
                        .build_int_truncate(arg_int, target_int_type, "arg_trunc")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                        .into()
                } else if target_int_type.get_bit_width() > 64 {
                    self.builder
                        .build_int_s_extend(arg_int, target_int_type, "arg_ext")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                        .into()
                } else {
                    arg_int.into()
                }
            } else if local_type.is_pointer_type() {
                self.builder
                    .build_int_to_ptr(arg_int, local_type.into_pointer_type(), "arg_ptr")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                    .into()
            } else if local_type.is_struct_type() {
                // For struct types, the i64 is a pointer to the struct - convert and load
                let opaque_ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                let struct_ptr = self.builder
                    .build_int_to_ptr(arg_int, opaque_ptr_type, "struct_arg_ptr")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                self.builder
                    .build_load(local_type, struct_ptr, "struct_arg_val")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
            } else if local_type.is_array_type() {
                // For array types, the i64 is a pointer to the array - convert and load
                let opaque_ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                let array_ptr = self.builder
                    .build_int_to_ptr(arg_int, opaque_ptr_type, "array_arg_ptr")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                self.builder
                    .build_load(local_type, array_ptr, "array_arg_val")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
            } else if local_type.is_float_type() {
                // Float was bitcast to i64, convert back
                self.builder
                    .build_bit_cast(arg_int, local_type, "float_arg")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
            } else {
                // For other types, use the raw i64 (shouldn't normally happen)
                arg_int.into()
            };

            self.builder.build_store(alloca, final_val)
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

            self.locals.insert(*param_id, alloca);
            self.local_types.insert(*param_id, local_type);
        }

        // Compile the handler body expression
        let result = self.compile_expr(&handler_body.body)?;

        // Check if block already terminated (e.g., by resume)
        let current_block = self.builder.get_insert_block()
            .ok_or_else(|| vec![Diagnostic::error("No current block".to_string(), span)])?;
        if current_block.get_terminator().is_some() {
            // Block already terminated, don't add return
        } else {
            // Return result as i64
            if let Some(ret_val) = result {
                let ret_i64 = match ret_val {
                    BasicValueEnum::IntValue(iv) => {
                        if iv.get_type().get_bit_width() == 64 {
                            iv
                        } else {
                            self.builder
                                .build_int_s_extend(iv, i64_type, "ret_ext")
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                        }
                    }
                    BasicValueEnum::PointerValue(pv) => {
                        self.builder
                            .build_ptr_to_int(pv, i64_type, "ret_ptr_int")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                    }
                    BasicValueEnum::FloatValue(fv) => {
                        // Convert float to i64 via bitcast for ABI compatibility
                        // Check if this is a 64-bit float by comparing types
                        let f64_type = self.context.f64_type();
                        if fv.get_type() == f64_type {
                            self.builder
                                .build_bit_cast(fv, i64_type, "ret_float_cast")
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                                .into_int_value()
                        } else {
                            // Extend f32 to f64, then bitcast to i64
                            let extended = self.builder
                                .build_float_ext(fv, f64_type, "ret_float_ext")
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                            self.builder
                                .build_bit_cast(extended, i64_type, "ret_float_cast")
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                                .into_int_value()
                        }
                    }
                    BasicValueEnum::ArrayValue(_) => {
                        return Err(vec![Diagnostic::error(
                            "Handler cannot return array type directly - use a pointer or wrapper struct".to_string(),
                            span
                        )]);
                    }
                    BasicValueEnum::StructValue(_) => {
                        return Err(vec![Diagnostic::error(
                            "Handler cannot return struct type directly - use a pointer or wrapper".to_string(),
                            span
                        )]);
                    }
                    BasicValueEnum::VectorValue(_) | BasicValueEnum::ScalableVectorValue(_) => {
                        return Err(vec![Diagnostic::error(
                            "Handler cannot return vector or scalable vector type directly - use a pointer or wrapper".to_string(),
                            span
                        )]);
                    }
                };
                self.builder.build_return(Some(&ret_i64))
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
            } else {
                // Return 0 for unit type
                self.builder.build_return(Some(&i64_type.const_zero()))
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
            }
        }

        // Restore context
        self.current_fn = saved_fn;
        self.locals = saved_locals;
        self.current_continuation = saved_continuation;
        self.is_multishot_handler = saved_is_multishot;

        Ok(fn_value)
    }

    /// Add a function to llvm.global_ctors for automatic execution at startup.
    pub(super) fn add_global_constructor(&mut self, init_fn: FunctionValue<'ctx>) -> Result<(), Vec<Diagnostic>> {
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());

        // Constructor entry: { i32 priority, ptr fn, ptr data }
        let ctor_struct_type = self.context.struct_type(
            &[
                i32_type.into(),
                ptr_type.into(),
                ptr_type.into(),
            ],
            false,
        );

        // Create the constructor entry
        let priority = i32_type.const_int(65535, false); // Low priority (runs early)
        let fn_ptr = init_fn.as_global_value().as_pointer_value();
        let null_data = ptr_type.const_null();

        let ctor_entry = ctor_struct_type.const_named_struct(&[
            priority.into(),
            fn_ptr.into(),
            null_data.into(),
        ]);

        // Create the array type and value using struct type's const_array method
        let ctor_array_type = ctor_struct_type.array_type(1);
        let ctor_array = ctor_struct_type.const_array(&[ctor_entry]);

        // Add as global variable with initializer
        let global = self.module.add_global(ctor_array_type, None, "llvm.global_ctors");
        global.set_initializer(&ctor_array);
        global.set_linkage(inkwell::module::Linkage::Appending);

        Ok(())
    }
}
