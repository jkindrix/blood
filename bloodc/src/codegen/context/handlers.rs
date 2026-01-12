//! Handler compilation for effect handlers.
//!
//! This module handles compilation of effect handler operations,
//! return clauses, and runtime registration.

use inkwell::values::{FunctionValue, BasicValueEnum};
use inkwell::AddressSpace;

use crate::hir::{self, DefId, LocalId};
use crate::diagnostics::Diagnostic;
use crate::span::Span;

use super::CodegenContext;

impl<'ctx, 'a> CodegenContext<'ctx, 'a> {
    /// Declare handler operation functions (Phase 2: Effect Handlers).
    ///
    /// Each handler operation is compiled to a function with signature:
    /// `fn(state: *mut void, args: *const i64, arg_count: i64, continuation: i64) -> i64`
    ///
    /// The continuation parameter is used for non-tail-resumptive handlers (deep handlers).
    /// For tail-resumptive handlers (shallow), the continuation is 0 (unused).
    pub fn declare_handler_operations(&mut self, hir_crate: &hir::Crate) -> Result<(), Vec<Diagnostic>> {
        let i64_type = self.context.i64_type();
        let i8_ptr_type = self.context.i8_type().ptr_type(AddressSpace::default());
        let i64_ptr_type = i64_type.ptr_type(AddressSpace::default());

        // Handler operation function signature:
        // fn(state: *mut void, args: *const i64, arg_count: i64, continuation: i64) -> i64
        let handler_op_type = i64_type.fn_type(
            &[i8_ptr_type.into(), i64_ptr_type.into(), i64_type.into(), i64_type.into()],
            false,
        );

        for (def_id, item) in &hir_crate.items {
            if let hir::ItemKind::Handler { operations, .. } = &item.kind {
                for (op_idx, handler_op) in operations.iter().enumerate() {
                    let fn_name = format!("{}_{}", item.name, handler_op.name);
                    let fn_value = self.module.add_function(&fn_name, handler_op_type, None);
                    // Set external linkage so linker resolves from other object files
                    fn_value.set_linkage(inkwell::module::Linkage::External);
                    self.handler_ops.insert((*def_id, op_idx), fn_value);
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
        // Compile all handler operations
        for (def_id, item) in &hir_crate.items {
            if let hir::ItemKind::Handler { operations, return_clause, state, .. } = &item.kind {
                // Compile each operation
                for (op_idx, handler_op) in operations.iter().enumerate() {
                    if let Some(&fn_value) = self.handler_ops.get(&(*def_id, op_idx)) {
                        if let Some(body) = hir_crate.bodies.get(&handler_op.body_id) {
                            self.compile_handler_op_body(fn_value, body, handler_op, state)?;
                        }
                    }
                }

                // Compile return clause if present
                if let Some(ret_clause) = return_clause {
                    if let Some(body) = hir_crate.bodies.get(&ret_clause.body_id) {
                        self.compile_return_clause(*def_id, &item.name, body, ret_clause, state)?;
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
        handler_name: &str,
        body: &hir::Body,
        _return_clause: &hir::ReturnClause,
        state_fields: &[hir::HandlerState],
    ) -> Result<(), Vec<Diagnostic>> {
        let span = body.span;
        let i64_type = self.context.i64_type();
        let i8_ptr_type = self.context.i8_type().ptr_type(AddressSpace::default());

        // Return clause function signature: fn(result: i64, state_ptr: *void) -> i64
        let ret_clause_type = i64_type.fn_type(&[i64_type.into(), i8_ptr_type.into()], false);
        let fn_name = format!("{}_return", handler_name);
        let fn_value = self.module.add_function(&fn_name, ret_clause_type, None);

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

        // Build the handler state struct type to properly access fields
        let state_field_types: Vec<_> = state_fields.iter()
            .map(|s| self.lower_type(&s.ty))
            .collect();
        let state_struct_type = self.context.struct_type(&state_field_types, false);
        let state_struct_ptr_type = state_struct_type.ptr_type(inkwell::AddressSpace::default());

        // Cast the void* state_ptr to the proper struct pointer type
        let typed_state_ptr = self.builder
            .build_pointer_cast(
                state_ptr.into_pointer_value(),
                state_struct_ptr_type,
                "typed_state_ptr"
            )
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

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

            // Check if this local corresponds to a state field
            if let Some(&field_idx) = state_field_indices.get(local_name) {
                // This is a state field - load its value from the state struct
                let field_ptr = self.builder
                    .build_struct_gep(typed_state_ptr, field_idx, &format!("{}_ptr", local_name))
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

                // Create an alloca to hold the local
                let alloca = self.builder
                    .build_alloca(local_type, local_name)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

                // Load the value from state and store in the local
                let value = self.builder
                    .build_load(field_ptr, &format!("{}_val", local_name))
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                self.builder.build_store(alloca, value)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

                self.locals.insert(local.id, alloca);
                continue;
            }

            // For the result param local, bind to the result parameter
            let alloca = self.builder
                .build_alloca(local_type, local_name)
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

            // Check if this is the result parameter (first param local)
            let param_locals: Vec<_> = body.params().collect();
            if param_locals.first().map(|p| p.id) == Some(local.id) {
                // Convert from i64 if needed
                let converted_val = if local_type.is_int_type() {
                    let int_type = local_type.into_int_type();
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
                if local_type.is_int_type() {
                    let zero_val = local_type.into_int_type().const_zero();
                    self.builder.build_store(alloca, zero_val)
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                }
            }

            self.locals.insert(local.id, alloca);
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
                    // Tuple/struct return: allocate on stack and return pointer as i64
                    // Note: For proper effects implementation, this should use a region allocator
                    // or the caller should pre-allocate space for the return value.
                    let struct_type = sv.get_type();
                    let alloca = self.builder
                        .build_alloca(struct_type, "ret_tuple")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                    self.builder.build_store(alloca, sv)
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                    self.builder
                        .build_ptr_to_int(alloca, i64_type, "ret_tuple_ptr")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
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
                BasicValueEnum::VectorValue(_) => {
                    // SIMD vectors are not expected in normal handler returns
                    return Err(vec![Diagnostic::error(
                        "Vector return types not supported in handler return clauses".to_string(),
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

        // Create entry block
        let entry_block = self.context.append_basic_block(fn_value, "entry");
        self.builder.position_at_end(entry_block);

        // Save and set context
        let saved_fn = self.current_fn;
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_continuation = self.current_continuation.take();
        self.current_fn = Some(fn_value);

        // Get parameters: (state: *mut void, args: *const i64, arg_count: i64, continuation: i64)
        let state_ptr = fn_value.get_nth_param(0)
            .ok_or_else(|| vec![Diagnostic::error("Missing state parameter".to_string(), span)])?;
        let args_ptr = fn_value.get_nth_param(1)
            .ok_or_else(|| vec![Diagnostic::error("Missing args parameter".to_string(), span)])?;
        let _arg_count = fn_value.get_nth_param(2)
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

        // Build the handler state struct type to properly access fields
        let state_field_types: Vec<_> = state_fields.iter()
            .map(|s| self.lower_type(&s.ty))
            .collect();
        let state_struct_type = self.context.struct_type(&state_field_types, false);
        let state_struct_ptr_type = state_struct_type.ptr_type(inkwell::AddressSpace::default());

        // Cast the void* state_ptr to the proper struct pointer type
        let typed_state_ptr = self.builder
            .build_pointer_cast(
                state_ptr.into_pointer_value(),
                state_struct_ptr_type,
                "typed_state_ptr"
            )
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

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

            // Check if this is a "resume" local - it's a function type
            if local.name.as_deref() == Some("resume") {
                // Resume is a placeholder - we'll handle resume calls specially
                // Create an alloca for it but don't initialize it (it's not a real value)
                let alloca = self.builder
                    .build_alloca(i64_type, "resume_placeholder")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                self.locals.insert(local.id, alloca);
                continue;
            }

            // Check if this local corresponds to a state field
            if let Some(&field_idx) = state_field_indices.get(local_name) {
                // This is a state field - load its value from the state struct
                let field_ptr = self.builder
                    .build_struct_gep(typed_state_ptr, field_idx, &format!("{}_ptr", local_name))
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

                // Create an alloca to hold the local
                let alloca = self.builder
                    .build_alloca(local_type, local_name)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

                // Load the value from state and store in the local
                let value = self.builder
                    .build_load(field_ptr, &format!("{}_val", local_name))
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                self.builder.build_store(alloca, value)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

                self.locals.insert(local.id, alloca);
                continue;
            }

            // For other locals (operation params, temporaries), allocate with defaults
            let alloca = self.builder
                .build_alloca(local_type, local_name)
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

            // Initialize with zero/default
            if local_type.is_int_type() {
                let zero_val = local_type.into_int_type().const_zero();
                self.builder.build_store(alloca, zero_val)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
            }

            self.locals.insert(local.id, alloca);
        }

        // Now extract operation parameters from args_ptr (if any)
        // params() returns locals 1..(1+param_count), but for handler ops
        // param_count might be 0 or include state fields
        // For now, just skip this since handler ops often have no params
        let _args_ptr_val = args_ptr.into_pointer_value();

        // Compile the body expression
        let result = self.compile_expr(&body.expr)?;

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
                        // Tuple/struct return: allocate on stack and return pointer as i64
                        let struct_type = sv.get_type();
                        let alloca = self.builder
                            .build_alloca(struct_type, "ret_tuple")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                        self.builder.build_store(alloca, sv)
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                        self.builder
                            .build_ptr_to_int(alloca, i64_type, "ret_tuple_ptr")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
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
                    BasicValueEnum::VectorValue(_) => {
                        return Err(vec![Diagnostic::error(
                            "Vector return types not supported in handler operations".to_string(),
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

        // Restore context
        self.current_fn = saved_fn;
        self.locals = saved_locals;
        self.current_continuation = saved_continuation;

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
        let i8_ptr_type = self.context.i8_type().ptr_type(AddressSpace::default());
        let void_type = self.context.void_type();

        // Get or declare the registration function
        let register_fn = self.module.get_function("blood_evidence_register")
            .unwrap_or_else(|| {
                let fn_type = void_type.fn_type(
                    &[
                        i8_ptr_type.into(), // evidence handle
                        i64_type.into(),    // effect_id
                        i8_ptr_type.ptr_type(AddressSpace::default()).into(), // ops array
                        i64_type.into(),    // op_count
                    ],
                    false,
                );
                self.module.add_function("blood_evidence_register", fn_type, None)
            });

        // Create a global constructor function to register handlers at startup
        let init_fn_type = void_type.fn_type(&[], false);
        let init_fn = self.module.add_function("__blood_register_handlers", init_fn_type, None);

        let entry = self.context.append_basic_block(init_fn, "entry");
        self.builder.position_at_end(entry);

        // For each handler, build an operations array and register it
        for (handler_id, handler_info) in &self.handler_defs.clone() {
            let op_count = handler_info.op_impls.len();
            if op_count == 0 {
                continue;
            }

            // Create array of function pointers
            let array_type = i8_ptr_type.array_type(op_count as u32);
            let ops_alloca = self.builder
                .build_alloca(array_type, "handler_ops")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

            let i32_type = self.context.i32_type();
            let zero = i32_type.const_zero();

            for op_idx in 0..op_count {
                if let Some(&fn_value) = self.handler_ops.get(&(*handler_id, op_idx)) {
                    let idx = i32_type.const_int(op_idx as u64, false);
                    let gep = unsafe {
                        self.builder.build_gep(ops_alloca, &[zero, idx], "op_ptr")
                    }.map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

                    let fn_ptr = self.builder
                        .build_pointer_cast(
                            fn_value.as_global_value().as_pointer_value(),
                            i8_ptr_type,
                            "fn_ptr",
                        )
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

                    self.builder.build_store(gep, fn_ptr)
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
                }
            }

            // Get pointer to first element
            let ops_ptr = unsafe {
                self.builder.build_gep(ops_alloca, &[zero, zero], "ops_ptr")
            }.map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

            // Call blood_evidence_register(null, effect_id, ops, op_count)
            // Note: We pass null for evidence handle - registration is global
            let effect_id_val = i64_type.const_int(handler_info.effect_id.index as u64, false);
            let op_count_val = i64_type.const_int(op_count as u64, false);
            let null_ev = i8_ptr_type.const_null();

            self.builder.build_call(
                register_fn,
                &[null_ev.into(), effect_id_val.into(), ops_ptr.into(), op_count_val.into()],
                "",
            ).map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
        }

        self.builder.build_return(None)
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

        // Add to llvm.global_ctors so it runs at program startup
        self.add_global_constructor(init_fn)?;

        Ok(())
    }

    /// Add a function to llvm.global_ctors for automatic execution at startup.
    pub(super) fn add_global_constructor(&mut self, init_fn: FunctionValue<'ctx>) -> Result<(), Vec<Diagnostic>> {
        let i32_type = self.context.i32_type();
        let i8_ptr_type = self.context.i8_type().ptr_type(AddressSpace::default());

        // Constructor entry: { i32 priority, void ()* fn, i8* data }
        let ctor_struct_type = self.context.struct_type(
            &[
                i32_type.into(),
                init_fn.get_type().ptr_type(AddressSpace::default()).into(),
                i8_ptr_type.into(),
            ],
            false,
        );

        // Create the constructor entry
        let priority = i32_type.const_int(65535, false); // Low priority (runs early)
        let fn_ptr = init_fn.as_global_value().as_pointer_value();
        let null_data = i8_ptr_type.const_null();

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
