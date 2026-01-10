//! Code generation context.
//!
//! This module provides the main code generation context which
//! coordinates LLVM code generation for a Blood program.

use std::collections::HashMap;

use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::builder::Builder;
use inkwell::basic_block::BasicBlock;
use inkwell::values::{FunctionValue, BasicValueEnum, PointerValue, IntValue};
use inkwell::types::{BasicTypeEnum, BasicType};
use inkwell::IntPredicate;
use inkwell::FloatPredicate;
use inkwell::AddressSpace;

use crate::hir::{self, DefId, LocalId, Type, TypeKind, PrimitiveTy};
use crate::hir::def::{IntTy, UintTy};
use crate::mir::{EscapeResults, EscapeState};
use crate::diagnostics::Diagnostic;
use crate::span::Span;
use crate::effects::{EffectLowering, EffectInfo, HandlerInfo};

/// Loop context for break/continue support.
#[derive(Clone)]
struct LoopContext<'ctx> {
    /// The loop's continue block (condition or body start).
    continue_block: BasicBlock<'ctx>,
    /// The loop's exit block.
    exit_block: BasicBlock<'ctx>,
    /// Optional label for named loops.
    label: Option<hir::LoopId>,
    /// Storage for break values (for loop expressions that return values).
    break_value_store: Option<PointerValue<'ctx>>,
}

/// The code generation context.
pub struct CodegenContext<'ctx, 'a> {
    /// The LLVM context.
    pub context: &'ctx Context,
    /// The LLVM module being built.
    pub module: &'a Module<'ctx>,
    /// The LLVM IR builder.
    pub builder: &'a Builder<'ctx>,
    /// Map from DefId to LLVM function.
    pub functions: HashMap<DefId, FunctionValue<'ctx>>,
    /// Map from LocalId to stack allocation (in current function).
    pub locals: HashMap<LocalId, PointerValue<'ctx>>,
    /// The current function being compiled.
    pub current_fn: Option<FunctionValue<'ctx>>,
    /// Errors encountered during codegen.
    pub errors: Vec<Diagnostic>,
    /// Struct definitions for type lowering.
    pub struct_defs: HashMap<DefId, Vec<Type>>,
    /// Enum definitions for type lowering: DefId -> (variants, each with field types).
    pub enum_defs: HashMap<DefId, Vec<Vec<Type>>>,
    /// Stack of loop contexts for break/continue.
    loop_stack: Vec<LoopContext<'ctx>>,
    /// Closure bodies for compilation (copied from HIR crate during compile_crate).
    closure_bodies: HashMap<hir::BodyId, hir::Body>,
    /// Counter for generating unique closure function names.
    closure_counter: u32,
    /// Methods requiring dynamic dispatch (polymorphic methods).
    /// Maps method DefId to the dispatch table slot.
    dynamic_dispatch_methods: HashMap<DefId, u64>,
    /// Next dispatch table slot to assign.
    next_dispatch_slot: u64,
    /// Escape analysis results for optimization.
    /// When available, used to skip generation checks for non-escaping values.
    escape_analysis: HashMap<DefId, EscapeResults>,
    /// Current function's DefId for escape analysis lookup.
    /// Note: Used for escape-analysis-based optimization when 128-bit pointers are enabled.
    #[allow(dead_code)]
    current_fn_def_id: Option<DefId>,
    /// Effect lowering context for managing effect compilation.
    effect_lowering: EffectLowering,
    /// Compiled effect definitions (effect DefId -> effect metadata).
    effect_defs: HashMap<DefId, EffectInfo>,
    /// Compiled handler definitions (handler DefId -> handler metadata).
    handler_defs: HashMap<DefId, HandlerInfo>,
    /// Handler function pointers for runtime registration.
    /// Maps (handler_id, op_index) -> LLVM function.
    handler_ops: HashMap<(DefId, usize), FunctionValue<'ctx>>,
    /// Builtin functions: DefId -> runtime function name.
    /// Used to resolve builtin function calls to LLVM runtime functions.
    pub builtin_fns: HashMap<DefId, String>,
}

impl<'ctx, 'a> CodegenContext<'ctx, 'a> {
    /// Create a new code generation context.
    pub fn new(
        context: &'ctx Context,
        module: &'a Module<'ctx>,
        builder: &'a Builder<'ctx>,
    ) -> Self {
        Self {
            context,
            module,
            builder,
            functions: HashMap::new(),
            locals: HashMap::new(),
            current_fn: None,
            errors: Vec::new(),
            struct_defs: HashMap::new(),
            enum_defs: HashMap::new(),
            loop_stack: Vec::new(),
            closure_bodies: HashMap::new(),
            closure_counter: 0,
            dynamic_dispatch_methods: HashMap::new(),
            next_dispatch_slot: 0,
            escape_analysis: HashMap::new(),
            current_fn_def_id: None,
            effect_lowering: EffectLowering::new(),
            effect_defs: HashMap::new(),
            handler_defs: HashMap::new(),
            handler_ops: HashMap::new(),
            builtin_fns: HashMap::new(),
        }
    }

    /// Set escape analysis results for optimization.
    ///
    /// When set, the code generator can:
    /// - Skip generation checks for values that don't escape
    /// - Use stack allocation for non-escaping values
    /// - Apply tier-appropriate allocation strategies
    pub fn set_escape_analysis(&mut self, escape_analysis: HashMap<DefId, EscapeResults>) {
        self.escape_analysis = escape_analysis;
    }

    /// Get escape state for a local variable in the current function.
    ///
    /// Returns NoEscape if no escape analysis is available.
    /// Note: Used for escape-analysis-based optimization when 128-bit pointers are enabled.
    #[allow(dead_code)]
    fn get_escape_state(&self, local: LocalId) -> EscapeState {
        if let Some(def_id) = self.current_fn_def_id {
            if let Some(results) = self.escape_analysis.get(&def_id) {
                return results.get(local);
            }
        }
        EscapeState::NoEscape // Default - assume safe if no analysis
    }

    /// Check if a local can be stack-allocated based on escape analysis.
    #[allow(dead_code)]
    fn can_stack_allocate(&self, local: LocalId) -> bool {
        if let Some(def_id) = self.current_fn_def_id {
            if let Some(results) = self.escape_analysis.get(&def_id) {
                return results.can_stack_allocate(local);
            }
        }
        true // Default - assume stack is fine if no analysis
    }

    /// Check if a local is captured by an effect operation.
    #[allow(dead_code)]
    fn is_effect_captured(&self, local: LocalId) -> bool {
        if let Some(def_id) = self.current_fn_def_id {
            if let Some(results) = self.escape_analysis.get(&def_id) {
                return results.is_effect_captured(local);
            }
        }
        false // Default - not captured if no analysis
    }

    /// Check if generation check can be skipped for a local.
    ///
    /// Generation checks can be skipped if:
    /// - The value doesn't escape (purely local)
    /// - The value is stack-allocated
    #[allow(dead_code)]
    fn can_skip_generation_check(&self, local: LocalId) -> bool {
        let escape_state = self.get_escape_state(local);
        escape_state == EscapeState::NoEscape && !self.is_effect_captured(local)
    }

    /// Declare types and functions from HIR without compiling bodies.
    ///
    /// This sets up:
    /// - Struct and enum definitions in `struct_defs` and `enum_defs`
    /// - Effect and handler definitions
    /// - Function declarations (without bodies)
    /// - Runtime function declarations
    ///
    /// After this, MIR bodies can be compiled using `compile_mir_body`.
    pub fn compile_crate_declarations(&mut self, hir_crate: &hir::Crate) -> Result<(), Vec<Diagnostic>> {
        // First pass: collect struct, enum, effect, and handler definitions
        for (def_id, item) in &hir_crate.items {
            match &item.kind {
                hir::ItemKind::Struct(struct_def) => {
                    let field_types = match &struct_def.kind {
                        hir::StructKind::Record(fields) => {
                            fields.iter().map(|f| f.ty.clone()).collect()
                        }
                        hir::StructKind::Tuple(fields) => {
                            fields.iter().map(|f| f.ty.clone()).collect()
                        }
                        hir::StructKind::Unit => Vec::new(),
                    };
                    self.struct_defs.insert(*def_id, field_types);
                }
                hir::ItemKind::Enum(enum_def) => {
                    let variants: Vec<Vec<Type>> = enum_def.variants.iter().map(|variant| {
                        match &variant.fields {
                            hir::StructKind::Record(fields) => {
                                fields.iter().map(|f| f.ty.clone()).collect()
                            }
                            hir::StructKind::Tuple(fields) => {
                                fields.iter().map(|f| f.ty.clone()).collect()
                            }
                            hir::StructKind::Unit => Vec::new(),
                        }
                    }).collect();
                    self.enum_defs.insert(*def_id, variants);
                }
                hir::ItemKind::Effect { .. } => {
                    if let Some(effect_info) = self.effect_lowering.lower_effect_decl(item) {
                        self.effect_defs.insert(*def_id, effect_info);
                    }
                }
                hir::ItemKind::Handler { .. } => {
                    if let Some(handler_info) = self.effect_lowering.lower_handler_decl(item) {
                        self.handler_defs.insert(*def_id, handler_info);
                    }
                }
                _ => {}
            }
        }

        // Copy closure bodies for later compilation
        for (body_id, body) in &hir_crate.bodies {
            self.closure_bodies.insert(*body_id, body.clone());
        }

        // Copy builtin function mappings for resolving runtime calls
        self.builtin_fns = hir_crate.builtin_fns.clone();

        // Second pass: declare all functions (excluding builtins which are resolved by runtime name)
        for (def_id, item) in &hir_crate.items {
            if let hir::ItemKind::Fn(fn_def) = &item.kind {
                // Skip builtin functions - they're resolved to runtime functions at call sites
                if self.builtin_fns.contains_key(def_id) {
                    continue;
                }
                self.declare_function(*def_id, &item.name, fn_def)?;
            }
        }

        // Declare runtime functions
        self.declare_runtime_functions();

        // Third pass: declare handler operation functions
        self.declare_handler_operations(hir_crate)?;

        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(std::mem::take(&mut self.errors))
        }
    }

    /// Compile an entire HIR crate.
    pub fn compile_crate(&mut self, hir_crate: &hir::Crate) -> Result<(), Vec<Diagnostic>> {
        // Copy builtin function mappings for resolving runtime calls
        self.builtin_fns = hir_crate.builtin_fns.clone();

        // First pass: collect struct, enum, effect, and handler definitions
        for (def_id, item) in &hir_crate.items {
            match &item.kind {
                hir::ItemKind::Struct(struct_def) => {
                    let field_types = match &struct_def.kind {
                        hir::StructKind::Record(fields) => {
                            fields.iter().map(|f| f.ty.clone()).collect()
                        }
                        hir::StructKind::Tuple(fields) => {
                            fields.iter().map(|f| f.ty.clone()).collect()
                        }
                        hir::StructKind::Unit => Vec::new(),
                    };
                    self.struct_defs.insert(*def_id, field_types);
                }
                hir::ItemKind::Enum(enum_def) => {
                    let variants: Vec<Vec<Type>> = enum_def.variants.iter().map(|variant| {
                        match &variant.fields {
                            hir::StructKind::Record(fields) => {
                                fields.iter().map(|f| f.ty.clone()).collect()
                            }
                            hir::StructKind::Tuple(fields) => {
                                fields.iter().map(|f| f.ty.clone()).collect()
                            }
                            hir::StructKind::Unit => Vec::new(),
                        }
                    }).collect();
                    self.enum_defs.insert(*def_id, variants);
                }
                hir::ItemKind::Effect { .. } => {
                    // Lower effect declaration to EffectInfo
                    if let Some(effect_info) = self.effect_lowering.lower_effect_decl(item) {
                        self.effect_defs.insert(*def_id, effect_info);
                    }
                }
                hir::ItemKind::Handler { .. } => {
                    // Lower handler declaration to HandlerInfo
                    if let Some(handler_info) = self.effect_lowering.lower_handler_decl(item) {
                        self.handler_defs.insert(*def_id, handler_info);
                    }
                }
                _ => {}
            }
        }

        // Copy closure bodies for later compilation
        for (body_id, body) in &hir_crate.bodies {
            self.closure_bodies.insert(*body_id, body.clone());
        }

        // Second pass: declare all functions (excluding builtins which are resolved by runtime name)
        for (def_id, item) in &hir_crate.items {
            if let hir::ItemKind::Fn(fn_def) = &item.kind {
                // Skip builtin functions - they're resolved to runtime functions at call sites
                if self.builtin_fns.contains_key(def_id) {
                    continue;
                }
                self.declare_function(*def_id, &item.name, fn_def)?;
            }
        }

        // Declare runtime functions
        self.declare_runtime_functions();

        // Third pass: declare handler operation functions
        self.declare_handler_operations(hir_crate)?;

        // Fourth pass: compile function bodies
        for (def_id, item) in &hir_crate.items {
            if let hir::ItemKind::Fn(fn_def) = &item.kind {
                if let Some(body_id) = fn_def.body_id {
                    if let Some(body) = hir_crate.bodies.get(&body_id) {
                        self.compile_function_body(*def_id, body)?;
                    }
                }
            }
        }

        // Fifth pass: compile handler operation bodies
        self.compile_handler_operations(hir_crate)?;

        // Sixth pass: register handlers with runtime
        self.register_handlers_with_runtime()?;

        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(std::mem::take(&mut self.errors))
        }
    }

    /// Declare handler operation functions (Phase 2: Effect Handlers).
    ///
    /// Each handler operation is compiled to a function with signature:
    /// `fn(state: *mut void, args: *const i64, arg_count: i64) -> i64`
    fn declare_handler_operations(&mut self, hir_crate: &hir::Crate) -> Result<(), Vec<Diagnostic>> {
        let i64_type = self.context.i64_type();
        let i8_ptr_type = self.context.i8_type().ptr_type(AddressSpace::default());
        let i64_ptr_type = i64_type.ptr_type(AddressSpace::default());

        // Handler operation function signature:
        // fn(state: *mut void, args: *const i64, arg_count: i64) -> i64
        let handler_op_type = i64_type.fn_type(
            &[i8_ptr_type.into(), i64_ptr_type.into(), i64_type.into()],
            false,
        );

        for (def_id, item) in &hir_crate.items {
            if let hir::ItemKind::Handler { operations, .. } = &item.kind {
                for (op_idx, handler_op) in operations.iter().enumerate() {
                    let fn_name = format!("{}_{}", item.name, handler_op.name);
                    let fn_value = self.module.add_function(&fn_name, handler_op_type, None);
                    self.handler_ops.insert((*def_id, op_idx), fn_value);
                }
            }
        }

        Ok(())
    }

    /// Compile handler operation bodies (Phase 2: Effect Handlers).
    fn compile_handler_operations(&mut self, hir_crate: &hir::Crate) -> Result<(), Vec<Diagnostic>> {
        for (def_id, item) in &hir_crate.items {
            if let hir::ItemKind::Handler { operations, return_clause, .. } = &item.kind {
                // Compile each operation
                for (op_idx, handler_op) in operations.iter().enumerate() {
                    if let Some(&fn_value) = self.handler_ops.get(&(*def_id, op_idx)) {
                        if let Some(body) = hir_crate.bodies.get(&handler_op.body_id) {
                            self.compile_handler_op_body(fn_value, body, handler_op)?;
                        }
                    }
                }

                // Compile return clause if present
                if let Some(ret_clause) = return_clause {
                    if let Some(body) = hir_crate.bodies.get(&ret_clause.body_id) {
                        self.compile_return_clause(*def_id, &item.name, body, ret_clause)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Compile a handler's return clause.
    ///
    /// The return clause transforms the final result of the handled computation.
    /// Signature: fn(result: i64) -> i64
    fn compile_return_clause(
        &mut self,
        _handler_id: DefId,
        handler_name: &str,
        body: &hir::Body,
        _return_clause: &hir::ReturnClause,
    ) -> Result<(), Vec<Diagnostic>> {
        let span = body.span;
        let i64_type = self.context.i64_type();

        // Return clause function signature: fn(result: i64) -> i64
        let ret_clause_type = i64_type.fn_type(&[i64_type.into()], false);
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

        // Bind the result parameter to the first parameter local
        let param_locals: Vec<_> = body.params().collect();
        if let Some(param_local) = param_locals.first() {
            let local_type = self.lower_type(&param_local.ty);
            let alloca = self.builder
                .build_alloca(local_type, "ret_param")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

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
            self.locals.insert(param_local.id, alloca);
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
                other => {
                    return Err(vec![Diagnostic::error(
                        format!(
                            "ICE: Unsupported return type in handler state body: {:?}. \
                             Expected IntValue, PointerValue, or FloatValue.",
                            other.get_type()
                        ),
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
    fn compile_handler_op_body(
        &mut self,
        fn_value: FunctionValue<'ctx>,
        body: &hir::Body,
        _handler_op: &hir::HandlerOp,
    ) -> Result<(), Vec<Diagnostic>> {
        let span = body.span;

        // Create entry block
        let entry_block = self.context.append_basic_block(fn_value, "entry");
        self.builder.position_at_end(entry_block);

        // Save and set context
        let saved_fn = self.current_fn;
        let saved_locals = std::mem::take(&mut self.locals);
        self.current_fn = Some(fn_value);

        // Get parameters: (state: *mut void, args: *const i64, arg_count: i64)
        let _state_ptr = fn_value.get_nth_param(0)
            .ok_or_else(|| vec![Diagnostic::error("Missing state parameter".to_string(), span)])?;
        let args_ptr = fn_value.get_nth_param(1)
            .ok_or_else(|| vec![Diagnostic::error("Missing args parameter".to_string(), span)])?;
        let _arg_count = fn_value.get_nth_param(2)
            .ok_or_else(|| vec![Diagnostic::error("Missing arg_count parameter".to_string(), span)])?;

        // Extract arguments from args array and bind to locals
        let i64_type = self.context.i64_type();
        let i32_type = self.context.i32_type();
        let zero = i32_type.const_zero();

        // Skip first local (return place) and bind parameters
        let param_locals: Vec<_> = body.params().collect();
        for (i, local) in param_locals.iter().enumerate() {
            let idx = i32_type.const_int(i as u64, false);
            let arg_ptr = unsafe {
                self.builder.build_gep(
                    args_ptr.into_pointer_value(),
                    &[zero, idx],
                    &format!("arg_ptr_{}", i),
                )
            }.map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

            let arg_val = self.builder
                .build_load(arg_ptr, &format!("arg_{}", i))
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

            // Store in local
            let local_type = self.lower_type(&local.ty);
            let alloca = self.builder
                .build_alloca(local_type, &format!("param.{}", i))
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

            // Convert from i64 to the local's type
            let converted_val = if local_type.is_int_type() {
                let int_type = local_type.into_int_type();
                if int_type.get_bit_width() == 64 {
                    arg_val
                } else {
                    self.builder
                        .build_int_truncate(arg_val.into_int_value(), int_type, "arg_trunc")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                        .into()
                }
            } else if local_type.is_pointer_type() {
                self.builder
                    .build_int_to_ptr(arg_val.into_int_value(), local_type.into_pointer_type(), "arg_ptr")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                    .into()
            } else {
                arg_val
            };

            self.builder.build_store(alloca, converted_val)
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
            self.locals.insert(local.id, alloca);
        }

        // Compile the body expression
        let result = self.compile_expr(&body.expr)?;

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
                other => {
                    return Err(vec![Diagnostic::error(
                        format!(
                            "ICE: Unsupported return type in handler op body: {:?}. \
                             Expected IntValue, PointerValue, or FloatValue.",
                            other.get_type()
                        ),
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

        // Restore context
        self.current_fn = saved_fn;
        self.locals = saved_locals;

        Ok(())
    }

    /// Register handlers with the runtime's effect registry.
    ///
    /// Generates code to call blood_evidence_register for each handler during
    /// module initialization.
    fn register_handlers_with_runtime(&mut self) -> Result<(), Vec<Diagnostic>> {
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
    fn add_global_constructor(&mut self, init_fn: FunctionValue<'ctx>) -> Result<(), Vec<Diagnostic>> {
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

    /// Declare a function (without body).
    fn declare_function(
        &mut self,
        def_id: DefId,
        name: &str,
        fn_def: &hir::FnDef,
    ) -> Result<(), Vec<Diagnostic>> {
        let fn_type = self.fn_type_from_sig(&fn_def.sig);
        // Rename "main" to "blood_main" for runtime linkage
        let llvm_name = if name == "main" { "blood_main" } else { name };
        let fn_value = self.module.add_function(llvm_name, fn_type, None);
        self.functions.insert(def_id, fn_value);
        Ok(())
    }

    /// Declare runtime support functions.
    fn declare_runtime_functions(&mut self) {
        let i8_type = self.context.i8_type();
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();
        let void_type = self.context.void_type();
        let i8_ptr_type = i8_type.ptr_type(AddressSpace::default());
        let i64_ptr_type = i64_type.ptr_type(AddressSpace::default());

        // === I/O Functions ===

        // print_int(i32) -> void
        let print_int_type = void_type.fn_type(&[i32_type.into()], false);
        self.module.add_function("print_int", print_int_type, None);

        // println_int(i32) -> void
        self.module.add_function("println_int", print_int_type, None);

        // print_i64(i64) -> void
        let print_i64_type = void_type.fn_type(&[i64_type.into()], false);
        self.module.add_function("print_i64", print_i64_type, None);

        // println_i64(i64) -> void
        self.module.add_function("println_i64", print_i64_type, None);

        // print_str(*i8) -> void
        let print_str_type = void_type.fn_type(&[i8_ptr_type.into()], false);
        self.module.add_function("print_str", print_str_type, None);

        // println_str(*i8) -> void
        self.module.add_function("println_str", print_str_type, None);

        // print_char(i32) -> void
        self.module.add_function("print_char", print_int_type, None);

        // println() -> void
        let println_type = void_type.fn_type(&[], false);
        self.module.add_function("println", println_type, None);

        // === Memory Management (Generational References) ===

        // blood_alloc(size: i64, out_addr: *i64, out_gen_meta: *i64) -> i32
        let size_type = i64_type; // size_t
        let alloc_type = i32_type.fn_type(&[
            size_type.into(),
            i64_ptr_type.into(),
            i64_ptr_type.into(),
        ], false);
        self.module.add_function("blood_alloc", alloc_type, None);

        // blood_free(addr: i64, size: i64) -> void
        let free_type = void_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        self.module.add_function("blood_free", free_type, None);

        // blood_check_generation(expected: i32, actual: i32) -> i32
        let check_gen_type = i32_type.fn_type(&[i32_type.into(), i32_type.into()], false);
        self.module.add_function("blood_check_generation", check_gen_type, None);

        // blood_stale_reference_panic(expected: i32, actual: i32) -> void (noreturn)
        let panic_type = void_type.fn_type(&[i32_type.into(), i32_type.into()], false);
        self.module.add_function("blood_stale_reference_panic", panic_type, None);

        // blood_panic(msg: *i8) -> void (noreturn)
        let panic_msg_type = void_type.fn_type(&[i8_ptr_type.into()], false);
        self.module.add_function("blood_panic", panic_msg_type, None);

        // blood_register_allocation(address: i64, size: i64) -> i32 (generation)
        let register_alloc_type = i32_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        self.module.add_function("blood_register_allocation", register_alloc_type, None);

        // blood_unregister_allocation(address: i64) -> void
        let unregister_alloc_type = void_type.fn_type(&[i64_type.into()], false);
        self.module.add_function("blood_unregister_allocation", unregister_alloc_type, None);

        // blood_validate_generation(address: i64, expected_gen: i32) -> i32 (0 = valid, 1 = stale)
        let validate_gen_type = i32_type.fn_type(&[i64_type.into(), i32_type.into()], false);
        self.module.add_function("blood_validate_generation", validate_gen_type, None);

        // blood_get_generation(address: i64) -> i32 (current generation from slot registry)
        let get_gen_type = i32_type.fn_type(&[i64_type.into()], false);
        self.module.add_function("blood_get_generation", get_gen_type, None);

        // === Effect Runtime ===

        // blood_evidence_create() -> *void (EvidenceHandle)
        let void_ptr_type = i8_ptr_type; // Use i8* as void*
        let ev_create_type = void_ptr_type.fn_type(&[], false);
        self.module.add_function("blood_evidence_create", ev_create_type, None);

        // blood_evidence_destroy(ev: *void) -> void
        let ev_destroy_type = void_type.fn_type(&[void_ptr_type.into()], false);
        self.module.add_function("blood_evidence_destroy", ev_destroy_type, None);

        // blood_evidence_push(ev: *void, handler: i64) -> void
        let ev_push_type = void_type.fn_type(&[void_ptr_type.into(), i64_type.into()], false);
        self.module.add_function("blood_evidence_push", ev_push_type, None);

        // blood_evidence_pop(ev: *void) -> i64
        let ev_pop_type = i64_type.fn_type(&[void_ptr_type.into()], false);
        self.module.add_function("blood_evidence_pop", ev_pop_type, None);

        // blood_evidence_get(ev: *void, index: i64) -> i64
        let ev_get_type = i64_type.fn_type(&[void_ptr_type.into(), i64_type.into()], false);
        self.module.add_function("blood_evidence_get", ev_get_type, None);

        // blood_evidence_register(ev: *void, effect_id: i64, ops: **void, op_count: i64) -> void
        let ev_register_type = void_type.fn_type(&[
            void_ptr_type.into(),
            i64_type.into(),
            void_ptr_type.into(),
            i64_type.into(),
        ], false);
        self.module.add_function("blood_evidence_register", ev_register_type, None);

        // blood_evidence_set_state(ev: *void, state: *void) -> void
        let ev_set_state_type = void_type.fn_type(&[void_ptr_type.into(), void_ptr_type.into()], false);
        self.module.add_function("blood_evidence_set_state", ev_set_state_type, None);

        // blood_evidence_get_state(ev: *void, index: i64) -> *void
        let ev_get_state_type = void_ptr_type.fn_type(&[void_ptr_type.into(), i64_type.into()], false);
        self.module.add_function("blood_evidence_get_state", ev_get_state_type, None);

        // blood_evidence_current() -> *void
        let ev_current_type = void_ptr_type.fn_type(&[], false);
        self.module.add_function("blood_evidence_current", ev_current_type, None);

        // blood_perform(effect_id: i64, op_index: i32, args: *i64, arg_count: i64) -> i64
        let perform_type = i64_type.fn_type(&[
            i64_type.into(),
            i32_type.into(),
            i64_ptr_type.into(),
            i64_type.into(),
        ], false);
        self.module.add_function("blood_perform", perform_type, None);

        // blood_handler_depth(effect_id: i64) -> i64
        let handler_depth_type = i64_type.fn_type(&[i64_type.into()], false);
        self.module.add_function("blood_handler_depth", handler_depth_type, None);

        // === Fiber/Continuation Support ===

        // blood_fiber_create() -> i64
        let fiber_create_type = i64_type.fn_type(&[], false);
        self.module.add_function("blood_fiber_create", fiber_create_type, None);

        // blood_fiber_suspend() -> i64
        let fiber_suspend_type = i64_type.fn_type(&[], false);
        self.module.add_function("blood_fiber_suspend", fiber_suspend_type, None);

        // blood_fiber_resume(fiber: i64, value: i64) -> void
        let fiber_resume_type = void_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        self.module.add_function("blood_fiber_resume", fiber_resume_type, None);

        // === Generation Snapshots ===

        // blood_snapshot_create() -> SnapshotHandle (i64)
        let snapshot_create_type = i64_type.fn_type(&[], false);
        self.module.add_function("blood_snapshot_create", snapshot_create_type, None);

        // blood_snapshot_add_entry(snapshot: i64, address: i64, generation: i32) -> void
        let snapshot_add_type = void_type.fn_type(&[
            i64_type.into(),
            i64_type.into(),
            i32_type.into(),
        ], false);
        self.module.add_function("blood_snapshot_add_entry", snapshot_add_type, None);

        // blood_snapshot_validate(snapshot: i64) -> i64 (0 = valid, non-zero = error)
        let snapshot_validate_type = i64_type.fn_type(&[i64_type.into()], false);
        self.module.add_function("blood_snapshot_validate", snapshot_validate_type, None);

        // blood_snapshot_len(snapshot: i64) -> i64
        let snapshot_len_type = i64_type.fn_type(&[i64_type.into()], false);
        self.module.add_function("blood_snapshot_len", snapshot_len_type, None);

        // blood_snapshot_destroy(snapshot: i64) -> void
        let snapshot_destroy_type = void_type.fn_type(&[i64_type.into()], false);
        self.module.add_function("blood_snapshot_destroy", snapshot_destroy_type, None);

        // === Multiple Dispatch Runtime ===

        // blood_dispatch_lookup(method_slot: i64, type_tag: i64) -> *void (function pointer)
        // Looks up the implementation for a method given the receiver's runtime type.
        let dispatch_lookup_type = void_ptr_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        self.module.add_function("blood_dispatch_lookup", dispatch_lookup_type, None);

        // blood_get_type_tag(obj: *void) -> i64
        // Gets the runtime type tag from an object's header.
        let get_type_tag_type = i64_type.fn_type(&[void_ptr_type.into()], false);
        self.module.add_function("blood_get_type_tag", get_type_tag_type, None);

        // blood_dispatch_register(method_slot: i64, type_tag: i64, impl_ptr: *void) -> void
        // Registers an implementation for a method/type combination in the dispatch table.
        let dispatch_register_type = void_type.fn_type(&[
            i64_type.into(),
            i64_type.into(),
            void_ptr_type.into(),
        ], false);
        self.module.add_function("blood_dispatch_register", dispatch_register_type, None);

        // === Work-Stealing Scheduler ===

        // blood_scheduler_init(num_workers: i64) -> i32
        let scheduler_init_type = i32_type.fn_type(&[i64_type.into()], false);
        self.module.add_function("blood_scheduler_init", scheduler_init_type, None);

        // blood_scheduler_spawn(task_fn: *void, arg: *void) -> i64
        let scheduler_spawn_type = i64_type.fn_type(&[void_ptr_type.into(), void_ptr_type.into()], false);
        self.module.add_function("blood_scheduler_spawn", scheduler_spawn_type, None);

        // blood_scheduler_spawn_simple(task_fn: *void) -> i64
        let scheduler_spawn_simple_type = i64_type.fn_type(&[void_ptr_type.into()], false);
        self.module.add_function("blood_scheduler_spawn_simple", scheduler_spawn_simple_type, None);

        // blood_scheduler_yield() -> void
        let scheduler_yield_type = void_type.fn_type(&[], false);
        self.module.add_function("blood_scheduler_yield", scheduler_yield_type, None);

        // blood_scheduler_run() -> void
        let scheduler_run_type = void_type.fn_type(&[], false);
        self.module.add_function("blood_scheduler_run", scheduler_run_type, None);

        // blood_scheduler_run_background() -> i32
        let scheduler_run_bg_type = i32_type.fn_type(&[], false);
        self.module.add_function("blood_scheduler_run_background", scheduler_run_bg_type, None);

        // blood_scheduler_shutdown() -> void
        let scheduler_shutdown_type = void_type.fn_type(&[], false);
        self.module.add_function("blood_scheduler_shutdown", scheduler_shutdown_type, None);

        // blood_scheduler_wait() -> void
        let scheduler_wait_type = void_type.fn_type(&[], false);
        self.module.add_function("blood_scheduler_wait", scheduler_wait_type, None);

        // blood_scheduler_active_fibers() -> i64
        let scheduler_active_type = i64_type.fn_type(&[], false);
        self.module.add_function("blood_scheduler_active_fibers", scheduler_active_type, None);

        // blood_scheduler_runnable_fibers() -> i64
        self.module.add_function("blood_scheduler_runnable_fibers", scheduler_active_type, None);

        // blood_scheduler_is_running() -> i32
        let scheduler_is_running_type = i32_type.fn_type(&[], false);
        self.module.add_function("blood_scheduler_is_running", scheduler_is_running_type, None);

        // === Runtime Lifecycle ===

        // blood_runtime_init() -> i32
        let init_type = i32_type.fn_type(&[], false);
        self.module.add_function("blood_runtime_init", init_type, None);

        // blood_runtime_shutdown() -> void
        let shutdown_type = void_type.fn_type(&[], false);
        self.module.add_function("blood_runtime_shutdown", shutdown_type, None);
    }

    /// Compile a function body.
    fn compile_function_body(
        &mut self,
        def_id: DefId,
        body: &hir::Body,
    ) -> Result<(), Vec<Diagnostic>> {
        let fn_value = *self.functions.get(&def_id).ok_or_else(|| {
            vec![Diagnostic::error(
                "Internal error: function not declared",
                Span::dummy(),
            )]
        })?;

        self.current_fn = Some(fn_value);
        self.locals.clear();
        self.loop_stack.clear();

        // Create entry block
        let entry = self.context.append_basic_block(fn_value, "entry");
        self.builder.position_at_end(entry);

        // Allocate space for parameters
        for (i, param) in body.params().enumerate() {
            let llvm_type = self.lower_type(&param.ty);
            let alloca = self.builder.build_alloca(llvm_type, &param.name.clone().unwrap_or_else(|| format!("arg{}", i)))
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

            // Store parameter value
            let param_value = fn_value.get_nth_param(i as u32)
                .ok_or_else(|| vec![Diagnostic::error("Parameter not found", Span::dummy())])?;
            self.builder.build_store(alloca, param_value)
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

            self.locals.insert(param.id, alloca);
        }

        // Compile body expression
        let result = self.compile_expr(&body.expr)?;

        // Build return
        if body.return_type().is_unit() {
            self.builder.build_return(None)
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
        } else if let Some(value) = result {
            self.builder.build_return(Some(&value))
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
        } else {
            self.builder.build_return(None)
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
        }

        self.current_fn = None;
        Ok(())
    }

    /// Get the current insert block, returning an error if none is set.
    ///
    /// This is a safe wrapper around `builder.get_insert_block()` that
    /// returns a proper error instead of panicking if no block is active.
    fn get_current_block(&self) -> Result<BasicBlock<'ctx>, Vec<Diagnostic>> {
        self.builder.get_insert_block()
            .ok_or_else(|| vec![Diagnostic::error(
                "Internal error: no active basic block".to_string(),
                Span::dummy(),
            )])
    }

    /// Lower an HIR type to an LLVM type.
    pub fn lower_type(&self, ty: &Type) -> BasicTypeEnum<'ctx> {
        match ty.kind() {
            TypeKind::Primitive(prim) => self.lower_primitive(prim),
            TypeKind::Tuple(types) if types.is_empty() => {
                // Unit type - use i8 as placeholder
                self.context.i8_type().into()
            }
            TypeKind::Tuple(types) => {
                let field_types: Vec<_> = types.iter()
                    .map(|t| self.lower_type(t))
                    .collect();
                self.context.struct_type(&field_types, false).into()
            }
            TypeKind::Array { element, size } => {
                let elem_type = self.lower_type(element);
                elem_type.array_type(*size as u32).into()
            }
            TypeKind::Ref { inner: _, .. } | TypeKind::Ptr { inner: _, .. } => {
                // All references/pointers become opaque pointers
                self.context.i8_type().ptr_type(AddressSpace::default()).into()
            }
            TypeKind::Never => {
                // Never type - can use any type, i8 works
                self.context.i8_type().into()
            }
            TypeKind::Adt { def_id, args: _ } => {
                // Look up the struct definition first
                if let Some(field_types) = self.struct_defs.get(def_id) {
                    let llvm_fields: Vec<BasicTypeEnum> = field_types
                        .iter()
                        .map(|t| self.lower_type(t))
                        .collect();
                    self.context.struct_type(&llvm_fields, false).into()
                } else if let Some(variants) = self.enum_defs.get(def_id) {
                    // Enum type: create a struct with tag + payload for largest variant
                    // Tag is i32
                    let tag_type: BasicTypeEnum = self.context.i32_type().into();

                    // Find the largest variant by number of fields
                    // For simplicity, use the first non-empty variant's fields or just the tag
                    let largest_variant = variants.iter()
                        .max_by_key(|v| v.len())
                        .cloned()
                        .unwrap_or_default();

                    let mut llvm_fields: Vec<BasicTypeEnum> = vec![tag_type];
                    for field_ty in &largest_variant {
                        llvm_fields.push(self.lower_type(field_ty));
                    }

                    self.context.struct_type(&llvm_fields, false).into()
                } else {
                    // Unknown ADT - default to i32
                    self.context.i32_type().into()
                }
            }
            TypeKind::Error | TypeKind::Infer(_) | TypeKind::Param(_) => {
                // Should not reach codegen with these
                self.context.i32_type().into()
            }
            TypeKind::Fn { .. } => {
                // Function/closure type - fat pointer struct { fn_ptr: i8*, env_ptr: i8* }
                let i8_ptr = self.context.i8_type().ptr_type(AddressSpace::default());
                self.context.struct_type(&[i8_ptr.into(), i8_ptr.into()], false).into()
            }
            TypeKind::Slice { .. } => {
                // Slice type - use pointer
                self.context.i8_type().ptr_type(AddressSpace::default()).into()
            }
        }
    }

    /// Lower a primitive type.
    fn lower_primitive(&self, prim: &PrimitiveTy) -> BasicTypeEnum<'ctx> {
        match prim {
            PrimitiveTy::Bool => self.context.bool_type().into(),
            PrimitiveTy::Char => self.context.i32_type().into(), // Unicode codepoint
            PrimitiveTy::Int(int_ty) => match int_ty {
                IntTy::I8 => self.context.i8_type().into(),
                IntTy::I16 => self.context.i16_type().into(),
                IntTy::I32 => self.context.i32_type().into(),
                IntTy::I64 => self.context.i64_type().into(),
                IntTy::I128 => self.context.i128_type().into(),
                IntTy::Isize => self.context.i64_type().into(), // Assume 64-bit
            },
            PrimitiveTy::Uint(uint_ty) => match uint_ty {
                UintTy::U8 => self.context.i8_type().into(),
                UintTy::U16 => self.context.i16_type().into(),
                UintTy::U32 => self.context.i32_type().into(),
                UintTy::U64 => self.context.i64_type().into(),
                UintTy::U128 => self.context.i128_type().into(),
                UintTy::Usize => self.context.i64_type().into(),
            },
            PrimitiveTy::Float(float_ty) => match float_ty {
                crate::hir::def::FloatTy::F32 => self.context.f32_type().into(),
                crate::hir::def::FloatTy::F64 => self.context.f64_type().into(),
            },
            PrimitiveTy::Str => {
                // String slice - pointer for now
                self.context.i8_type().ptr_type(AddressSpace::default()).into()
            }
            PrimitiveTy::String => {
                // Owned String - pointer to heap-allocated string for now
                // In the future, this would be a struct { ptr, len, cap }
                self.context.i8_type().ptr_type(AddressSpace::default()).into()
            }
            PrimitiveTy::Unit => {
                // Unit type - represented as an empty struct or i8 for ABI purposes
                self.context.struct_type(&[], false).into()
            }
        }
    }

    /// Create LLVM function type from HIR signature.
    fn fn_type_from_sig(&self, sig: &hir::FnSig) -> inkwell::types::FunctionType<'ctx> {
        let param_types: Vec<_> = sig.inputs.iter()
            .map(|t| self.lower_type(t).into())
            .collect();

        if sig.output.is_unit() {
            self.context.void_type().fn_type(&param_types, false)
        } else {
            let ret_type = self.lower_type(&sig.output);
            ret_type.fn_type(&param_types, false)
        }
    }

    /// Compile an expression.
    pub fn compile_expr(&mut self, expr: &hir::Expr) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        use hir::ExprKind::*;

        match &expr.kind {
            Literal(lit) => self.compile_literal(lit).map(Some),
            Local(local_id) => self.compile_local_load(*local_id).map(Some),
            Binary { op, left, right } => self.compile_binary(op, left, right).map(Some),
            Unary { op, operand } => self.compile_unary(op, operand).map(Some),
            Call { callee, args } => self.compile_call(callee, args),
            MethodCall { receiver, method, args } => {
                // Method call: receiver.method(args)
                // Compiled as method(receiver, args) with potential dynamic dispatch
                self.compile_method_call(receiver, *method, args, &expr.ty)
            }
            Block { stmts, expr: tail_expr } => self.compile_block(stmts, tail_expr.as_deref()),
            If { condition, then_branch, else_branch } => {
                self.compile_if(condition, then_branch, else_branch.as_deref(), &expr.ty)
            }
            While { condition, body, .. } => {
                self.compile_while(condition, body)?;
                Ok(None)
            }
            Return(value) => {
                self.compile_return(value.as_deref())?;
                Ok(None)
            }
            Assign { target, value } => {
                self.compile_assign(target, value)?;
                Ok(None)
            }
            Tuple(exprs) => {
                // Empty tuple is unit type, return None
                if exprs.is_empty() {
                    return Ok(None);
                }
                // For non-empty tuples, compile all expressions and build a struct
                let values: Vec<_> = exprs.iter()
                    .filter_map(|e| self.compile_expr(e).ok().flatten())
                    .collect();
                if values.is_empty() {
                    Ok(None)
                } else if values.len() == 1 {
                    // Safe: we just verified len == 1, so pop() returns Some
                    Ok(values.into_iter().next())
                } else {
                    // Build a struct for the tuple
                    let types: Vec<BasicTypeEnum> = values.iter()
                        .map(|v| v.get_type())
                        .collect();
                    let struct_type = self.context.struct_type(&types, false);
                    let mut struct_val = struct_type.get_undef();
                    for (i, val) in values.into_iter().enumerate() {
                        struct_val = self.builder
                            .build_insert_value(struct_val, val, i as u32, "tuple")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
                            .into_struct_value();
                    }
                    Ok(Some(struct_val.into()))
                }
            }
            Def(def_id) => {
                // Reference to a function - return the function pointer or look up value
                if let Some(fn_val) = self.functions.get(def_id) {
                    Ok(Some(fn_val.as_global_value().as_pointer_value().into()))
                } else {
                    // Might be a constant - for now return error
                    Ok(None)
                }
            }
            Struct { def_id: _, fields, base: _ } => {
                // Compile struct construction
                self.compile_struct_expr(&expr.ty, fields)
            }
            Variant { def_id, variant_idx, fields } => {
                // Compile enum variant construction
                self.compile_variant(*def_id, *variant_idx, fields, &expr.ty)
            }
            Field { base, field_idx } => {
                // Compile field access
                self.compile_field_access(base, *field_idx)
            }
            Match { scrutinee, arms } => {
                // Compile match expression
                self.compile_match(scrutinee, arms, &expr.ty)
            }
            Loop { body, label } => {
                // Compile infinite loop
                self.compile_loop(body, *label, &expr.ty)
            }
            Break { label, value } => {
                // Compile break statement
                self.compile_break(*label, value.as_deref())?;
                Ok(None)
            }
            Continue { label } => {
                // Compile continue statement
                self.compile_continue(*label)?;
                Ok(None)
            }
            Array(elements) => {
                // Compile array literal
                self.compile_array(elements, &expr.ty)
            }
            Index { base, index } => {
                // Compile array/slice indexing
                self.compile_index(base, index)
            }
            Cast { expr: inner, target_ty } => {
                // Compile type cast
                self.compile_cast(inner, target_ty)
            }
            Perform { effect_id, op_index, args } => {
                // Effect operation: perform Effect.op(args)
                // After evidence translation, this calls through the evidence vector.
                // For now, we emit a placeholder that will be filled in during
                // full effects system integration (Phase 2.4).
                self.compile_perform(*effect_id, *op_index, args, &expr.ty)
            }
            Resume { value } => {
                // Resume continuation in handler.
                // For tail-resumptive handlers, this is just a return.
                // For general handlers, this requires continuation capture (Phase 2.3).
                self.compile_resume(value.as_deref(), &expr.ty)
            }
            Handle { body, handler_id } => {
                // Handle expression: runs body with handler installed.
                // This sets up the evidence vector and runs the body.
                self.compile_handle(body, *handler_id, &expr.ty)
            }
            Deref(inner) => {
                // Dereference: *x
                // For generational references, this should check the generation
                // before accessing the pointed-to value.
                self.compile_deref(inner, expr.span)
            }
            Borrow { expr: inner, mutable: _ } => {
                // Borrow: &x or &mut x
                // Creates a reference to the given expression.
                // For generational references, this captures the current generation.
                self.compile_borrow(inner)
            }
            AddrOf { expr: inner, mutable: _ } => {
                // Address-of: raw pointer creation
                // Creates a raw pointer without generation tracking.
                self.compile_addr_of(inner)
            }
            Closure { body_id, captures } => {
                // Closure: |x| x + 1
                // Creates a function for the body and a struct for captures.
                self.compile_closure(*body_id, captures, &expr.ty, expr.span)
            }
            _ => {
                self.errors.push(Diagnostic::error(
                    format!("Unsupported expression kind: {:?}", std::mem::discriminant(&expr.kind)),
                    expr.span,
                ));
                Ok(None)
            }
        }
    }

    /// Compile a literal.
    fn compile_literal(&self, lit: &hir::LiteralValue) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        match lit {
            hir::LiteralValue::Int(val) => {
                Ok(self.context.i32_type().const_int(*val as u64, true).into())
            }
            hir::LiteralValue::Uint(val) => {
                Ok(self.context.i32_type().const_int(*val as u64, false).into())
            }
            hir::LiteralValue::Float(val) => {
                Ok(self.context.f64_type().const_float(*val).into())
            }
            hir::LiteralValue::Bool(val) => {
                Ok(self.context.bool_type().const_int(*val as u64, false).into())
            }
            hir::LiteralValue::Char(val) => {
                Ok(self.context.i32_type().const_int(*val as u64, false).into())
            }
            hir::LiteralValue::String(s) => {
                // Create global string constant
                let global = self.builder.build_global_string_ptr(s, "str")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
                Ok(global.as_pointer_value().into())
            }
        }
    }

    /// Load a local variable.
    fn compile_local_load(&self, local_id: LocalId) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        let alloca = self.locals.get(&local_id)
            .ok_or_else(|| vec![Diagnostic::error(
                format!("Local variable {:?} not found", local_id),
                Span::dummy(),
            )])?;

        self.builder.build_load(*alloca, "load")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])
    }

    /// Compile a binary operation.
    fn compile_binary(
        &mut self,
        op: &crate::ast::BinOp,
        left: &hir::Expr,
        right: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        use crate::ast::BinOp::*;

        // Special handling for pipe operator: a |> f === f(a)
        // The right operand should be a function, and left is the argument
        if matches!(op, Pipe) {
            return self.compile_pipe(left, right);
        }

        let lhs = self.compile_expr(left)?
            .ok_or_else(|| vec![Diagnostic::error("Expected value for binary op", left.span)])?;
        let rhs = self.compile_expr(right)?
            .ok_or_else(|| vec![Diagnostic::error("Expected value for binary op", right.span)])?;

        // Check if operands are floats
        let is_float = matches!(left.ty.kind(), TypeKind::Primitive(PrimitiveTy::Float(_)));

        // Check if operands are unsigned integers
        let is_unsigned = matches!(left.ty.kind(), TypeKind::Primitive(PrimitiveTy::Uint(_)));

        if is_float {
            // Float operations
            let lhs_float = lhs.into_float_value();
            let rhs_float = rhs.into_float_value();

            // Handle arithmetic operations (return FloatValue)
            match op {
                Add => {
                    return self.builder.build_float_add(lhs_float, rhs_float, "fadd")
                        .map(|v| v.into())
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())]);
                }
                Sub => {
                    return self.builder.build_float_sub(lhs_float, rhs_float, "fsub")
                        .map(|v| v.into())
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())]);
                }
                Mul => {
                    return self.builder.build_float_mul(lhs_float, rhs_float, "fmul")
                        .map(|v| v.into())
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())]);
                }
                Div => {
                    return self.builder.build_float_div(lhs_float, rhs_float, "fdiv")
                        .map(|v| v.into())
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())]);
                }
                Rem => {
                    return self.builder.build_float_rem(lhs_float, rhs_float, "frem")
                        .map(|v| v.into())
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())]);
                }
                _ => {} // Fall through to comparison handling below
            }

            // Handle comparison operations (return IntValue/i1)
            let result = match op {
                Eq => self.builder.build_float_compare(FloatPredicate::OEQ, lhs_float, rhs_float, "feq"),
                Ne => self.builder.build_float_compare(FloatPredicate::ONE, lhs_float, rhs_float, "fne"),
                Lt => self.builder.build_float_compare(FloatPredicate::OLT, lhs_float, rhs_float, "flt"),
                Le => self.builder.build_float_compare(FloatPredicate::OLE, lhs_float, rhs_float, "fle"),
                Gt => self.builder.build_float_compare(FloatPredicate::OGT, lhs_float, rhs_float, "fgt"),
                Ge => self.builder.build_float_compare(FloatPredicate::OGE, lhs_float, rhs_float, "fge"),
                // Bitwise and logical ops don't make sense for floats
                And | Or | BitAnd | BitOr | BitXor | Shl | Shr => {
                    return Err(vec![Diagnostic::error(
                        "Bitwise/logical operations not supported for float types",
                        left.span,
                    )]);
                }
                // Pipe is handled specially at the start of compile_binary
                Pipe => unreachable!("Pipe operator handled before operand compilation"),
                // Arithmetic ops already handled above
                Add | Sub | Mul | Div | Rem => unreachable!(),
            };

            result
                .map(|v| v.into())
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])
        } else {
            // Integer operations
            let lhs_int = lhs.into_int_value();
            let rhs_int = rhs.into_int_value();

            let result = match op {
                Add => self.builder.build_int_add(lhs_int, rhs_int, "add"),
                Sub => self.builder.build_int_sub(lhs_int, rhs_int, "sub"),
                Mul => self.builder.build_int_mul(lhs_int, rhs_int, "mul"),
                Div => {
                    if is_unsigned {
                        self.builder.build_int_unsigned_div(lhs_int, rhs_int, "udiv")
                    } else {
                        self.builder.build_int_signed_div(lhs_int, rhs_int, "sdiv")
                    }
                }
                Rem => {
                    if is_unsigned {
                        self.builder.build_int_unsigned_rem(lhs_int, rhs_int, "urem")
                    } else {
                        self.builder.build_int_signed_rem(lhs_int, rhs_int, "srem")
                    }
                }
                Eq => self.builder.build_int_compare(IntPredicate::EQ, lhs_int, rhs_int, "eq"),
                Ne => self.builder.build_int_compare(IntPredicate::NE, lhs_int, rhs_int, "ne"),
                Lt => {
                    if is_unsigned {
                        self.builder.build_int_compare(IntPredicate::ULT, lhs_int, rhs_int, "ult")
                    } else {
                        self.builder.build_int_compare(IntPredicate::SLT, lhs_int, rhs_int, "slt")
                    }
                }
                Le => {
                    if is_unsigned {
                        self.builder.build_int_compare(IntPredicate::ULE, lhs_int, rhs_int, "ule")
                    } else {
                        self.builder.build_int_compare(IntPredicate::SLE, lhs_int, rhs_int, "sle")
                    }
                }
                Gt => {
                    if is_unsigned {
                        self.builder.build_int_compare(IntPredicate::UGT, lhs_int, rhs_int, "ugt")
                    } else {
                        self.builder.build_int_compare(IntPredicate::SGT, lhs_int, rhs_int, "sgt")
                    }
                }
                Ge => {
                    if is_unsigned {
                        self.builder.build_int_compare(IntPredicate::UGE, lhs_int, rhs_int, "uge")
                    } else {
                        self.builder.build_int_compare(IntPredicate::SGE, lhs_int, rhs_int, "sge")
                    }
                }
                And => self.builder.build_and(lhs_int, rhs_int, "and"),
                Or => self.builder.build_or(lhs_int, rhs_int, "or"),
                BitAnd => self.builder.build_and(lhs_int, rhs_int, "bitand"),
                BitOr => self.builder.build_or(lhs_int, rhs_int, "bitor"),
                BitXor => self.builder.build_xor(lhs_int, rhs_int, "bitxor"),
                Shl => self.builder.build_left_shift(lhs_int, rhs_int, "shl"),
                Shr => {
                    // Arithmetic shift for signed, logical shift for unsigned
                    self.builder.build_right_shift(lhs_int, rhs_int, !is_unsigned, "shr")
                }
                // Pipe is handled specially at the start of compile_binary
                Pipe => unreachable!("Pipe operator handled before operand compilation"),
            };

            result
                .map(|v| v.into())
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])
        }
    }

    /// Compile a unary operation.
    fn compile_unary(
        &mut self,
        op: &crate::ast::UnaryOp,
        operand: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        use crate::ast::UnaryOp::*;

        let val = self.compile_expr(operand)?
            .ok_or_else(|| vec![Diagnostic::error("Expected value for unary op", operand.span)])?;

        // Check if operand is a float
        let is_float = matches!(operand.ty.kind(), TypeKind::Primitive(PrimitiveTy::Float(_)));

        match op {
            Neg => {
                if is_float {
                    let float_val = val.into_float_value();
                    self.builder.build_float_neg(float_val, "fneg")
                        .map(|v| v.into())
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])
                } else {
                    let int_val = val.into_int_value();
                    self.builder.build_int_neg(int_val, "neg")
                        .map(|v| v.into())
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])
                }
            }
            Not => {
                if is_float {
                    return Err(vec![Diagnostic::error(
                        "Bitwise NOT not supported for float types",
                        operand.span,
                    )]);
                }
                let int_val = val.into_int_value();
                self.builder.build_not(int_val, "not")
                    .map(|v| v.into())
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])
            }
            _ => Err(vec![Diagnostic::error(
                format!("Unsupported unary operator: {:?}", op),
                Span::dummy(),
            )]),
        }
    }

    /// Compile a function call.
    fn compile_call(
        &mut self,
        callee: &hir::Expr,
        args: &[hir::Expr],
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        // First, try to get a direct function reference
        if let hir::ExprKind::Def(def_id) = &callee.kind {
            if let Some(&fn_value) = self.functions.get(def_id) {
                // Direct function call
                let mut compiled_args = Vec::new();
                for arg in args {
                    if let Some(val) = self.compile_expr(arg)? {
                        compiled_args.push(val.into());
                    }
                }

                let call = self.builder
                    .build_call(fn_value, &compiled_args, "call")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

                return Ok(call.try_as_basic_value().left());
            }

            // Check if this is a builtin function call
            if let Some(builtin_name) = self.builtin_fns.get(def_id) {
                if let Some(fn_value) = self.module.get_function(builtin_name) {
                    // Builtin function call - compile args and call runtime function
                    let mut compiled_args = Vec::new();
                    for arg in args {
                        if let Some(val) = self.compile_expr(arg)? {
                            compiled_args.push(val.into());
                        }
                    }

                    let call = self.builder
                        .build_call(fn_value, &compiled_args, "builtin_call")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

                    return Ok(call.try_as_basic_value().left());
                } else {
                    return Err(vec![Diagnostic::error(
                        format!("Runtime function '{}' not declared", builtin_name),
                        callee.span,
                    )]);
                }
            }
        }

        // Check if this is a closure call (callee is a function type stored in a variable)
        if matches!(callee.ty.kind(), TypeKind::Fn { .. }) {
            return self.compile_closure_call(callee, args);
        }

        Err(vec![Diagnostic::error(
            "Cannot determine function to call",
            callee.span,
        )])
    }

    /// Compile a closure call: calling a closure value stored in a variable.
    ///
    /// Closure values are fat pointers: { fn_ptr: *i8, env_ptr: *i8 }
    /// We extract the function pointer, cast it to the appropriate type,
    /// and call with (env_ptr, args...).
    fn compile_closure_call(
        &mut self,
        callee: &hir::Expr,
        args: &[hir::Expr],
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        let i8_ptr_type = self.context.i8_type().ptr_type(AddressSpace::default());

        // Compile the closure expression to get the closure struct
        let closure_val = self.compile_expr(callee)?
            .ok_or_else(|| vec![Diagnostic::error("Expected closure value", callee.span)])?;

        // Extract function pointer and environment pointer from the closure struct
        let (fn_ptr, env_ptr) = match closure_val {
            BasicValueEnum::StructValue(sv) => {
                let fn_ptr = self.builder
                    .build_extract_value(sv, 0, "closure.fn")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), callee.span)])?
                    .into_pointer_value();
                let env_ptr = self.builder
                    .build_extract_value(sv, 1, "closure.env")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), callee.span)])?
                    .into_pointer_value();
                (fn_ptr, env_ptr)
            }
            BasicValueEnum::PointerValue(ptr) => {
                // If it's a pointer to a closure struct, load it first
                let loaded = self.builder.build_load(ptr, "closure.load")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), callee.span)])?;
                let sv = loaded.into_struct_value();
                let fn_ptr = self.builder
                    .build_extract_value(sv, 0, "closure.fn")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), callee.span)])?
                    .into_pointer_value();
                let env_ptr = self.builder
                    .build_extract_value(sv, 1, "closure.env")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), callee.span)])?
                    .into_pointer_value();
                (fn_ptr, env_ptr)
            }
            _ => {
                return Err(vec![Diagnostic::error(
                    format!("Expected closure struct, got {:?}", closure_val.get_type()),
                    callee.span,
                )]);
            }
        };

        // Compile arguments
        let mut compiled_args: Vec<inkwell::values::BasicMetadataValueEnum> = vec![env_ptr.into()];
        for arg in args {
            if let Some(val) = self.compile_expr(arg)? {
                compiled_args.push(val.into());
            }
        }

        // Build function type for the closure
        let (param_types, return_ty) = match callee.ty.kind() {
            TypeKind::Fn { params, ret } => (params.clone(), (*ret).clone()),
            _ => {
                return Err(vec![Diagnostic::error(
                    "Expected function type for closure call",
                    callee.span,
                )]);
            }
        };

        // Build LLVM function type: (env_ptr, params...) -> ret
        let mut fn_param_types: Vec<inkwell::types::BasicMetadataTypeEnum> = vec![i8_ptr_type.into()];
        for param_ty in &param_types {
            fn_param_types.push(self.lower_type(param_ty).into());
        }

        let fn_type = if return_ty.is_unit() {
            self.context.void_type().fn_type(&fn_param_types, false)
        } else {
            let ret_llvm = self.lower_type(&return_ty);
            ret_llvm.fn_type(&fn_param_types, false)
        };

        // Cast function pointer to the correct type
        let fn_ptr_type = fn_type.ptr_type(AddressSpace::default());
        let typed_fn_ptr = self.builder
            .build_pointer_cast(fn_ptr, fn_ptr_type, "closure.fn.typed")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), callee.span)])?;

        // Build the indirect call
        let call_site = self.builder
            .build_call(
                inkwell::values::CallableValue::try_from(typed_fn_ptr)
                    .map_err(|_| vec![Diagnostic::error("Invalid function pointer for closure call", callee.span)])?,
                &compiled_args,
                "closure_call",
            )
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), callee.span)])?;

        Ok(call_site.try_as_basic_value().left())
    }

    /// Compile a method call: `receiver.method(args)`
    ///
    /// This compiles to `method(receiver, args...)`. For dynamic dispatch
    /// based on the receiver's type, the dispatch table would be consulted.
    fn compile_method_call(
        &mut self,
        receiver: &hir::Expr,
        method_id: DefId,
        args: &[hir::Expr],
        result_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        // Compile the receiver (becomes first argument)
        let receiver_val = self.compile_expr(receiver)?
            .ok_or_else(|| vec![Diagnostic::error(
                "Expected value for method receiver",
                receiver.span,
            )])?;

        // Compile remaining arguments
        let mut compiled_args: Vec<inkwell::values::BasicMetadataValueEnum> = vec![receiver_val.into()];
        for arg in args {
            if let Some(val) = self.compile_expr(arg)? {
                compiled_args.push(val.into());
            }
        }

        // Check if this method requires dynamic dispatch
        if let Some(&method_slot) = self.dynamic_dispatch_methods.get(&method_id) {
            // Dynamic dispatch path: lookup implementation at runtime
            self.compile_dynamic_dispatch(
                receiver,
                &receiver_val,
                method_slot,
                &compiled_args,
                result_ty,
            )
        } else {
            // Static dispatch: call the statically resolved method directly
            let fn_value = self.functions.get(&method_id).copied()
                .ok_or_else(|| vec![Diagnostic::error(
                    format!("Method not found: {:?}", method_id),
                    receiver.span,
                )])?;

            let call = self.builder
                .build_call(fn_value, &compiled_args, "method_call")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), receiver.span)])?;

            Ok(call.try_as_basic_value().left())
        }
    }

    /// Compile a dynamic dispatch call using runtime type lookup.
    ///
    /// This implements multiple dispatch by:
    /// 1. Getting the receiver's runtime type tag
    /// 2. Looking up the implementation in the dispatch table
    /// 3. Performing an indirect call through the function pointer
    fn compile_dynamic_dispatch(
        &mut self,
        receiver: &hir::Expr,
        receiver_val: &BasicValueEnum<'ctx>,
        method_slot: u64,
        compiled_args: &[inkwell::values::BasicMetadataValueEnum<'ctx>],
        result_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        let i64_type = self.context.i64_type();
        let i8_type = self.context.i8_type();
        let i8_ptr_type = i8_type.ptr_type(AddressSpace::default());

        // Get the blood_get_type_tag runtime function
        let get_type_tag_fn = self.module.get_function("blood_get_type_tag")
            .ok_or_else(|| vec![Diagnostic::error(
                "Runtime function blood_get_type_tag not found",
                receiver.span,
            )])?;

        // Get the blood_dispatch_lookup runtime function
        let dispatch_lookup_fn = self.module.get_function("blood_dispatch_lookup")
            .ok_or_else(|| vec![Diagnostic::error(
                "Runtime function blood_dispatch_lookup not found",
                receiver.span,
            )])?;

        // Cast receiver to void* for the type tag lookup
        let receiver_ptr = match receiver_val {
            BasicValueEnum::PointerValue(p) => *p,
            _ => {
                // For non-pointer types, we need to allocate and store
                // This shouldn't normally happen for method receivers
                let alloca = self.builder
                    .build_alloca(receiver_val.get_type(), "receiver_tmp")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), receiver.span)])?;
                self.builder
                    .build_store(alloca, *receiver_val)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), receiver.span)])?;
                alloca
            }
        };

        let receiver_void_ptr = self.builder
            .build_pointer_cast(receiver_ptr, i8_ptr_type, "receiver_void")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), receiver.span)])?;

        // Get the type tag: blood_get_type_tag(receiver)
        let type_tag = self.builder
            .build_call(get_type_tag_fn, &[receiver_void_ptr.into()], "type_tag")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), receiver.span)])?
            .try_as_basic_value()
            .left()
            .ok_or_else(|| vec![Diagnostic::error("Expected type tag result", receiver.span)])?;

        // Look up the implementation: blood_dispatch_lookup(method_slot, type_tag)
        let method_slot_val = i64_type.const_int(method_slot, false);
        let impl_ptr = self.builder
            .build_call(
                dispatch_lookup_fn,
                &[method_slot_val.into(), type_tag.into()],
                "impl_ptr",
            )
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), receiver.span)])?
            .try_as_basic_value()
            .left()
            .ok_or_else(|| vec![Diagnostic::error("Expected implementation pointer", receiver.span)])?;

        // Build the function type for the indirect call
        // Extract parameter types from the BasicMetadataValueEnum values
        let param_types: Vec<inkwell::types::BasicMetadataTypeEnum> = compiled_args
            .iter()
            .filter_map(|arg| {
                // Convert BasicMetadataValueEnum to its type
                match arg {
                    inkwell::values::BasicMetadataValueEnum::ArrayValue(v) => Some(v.get_type().into()),
                    inkwell::values::BasicMetadataValueEnum::IntValue(v) => Some(v.get_type().into()),
                    inkwell::values::BasicMetadataValueEnum::FloatValue(v) => Some(v.get_type().into()),
                    inkwell::values::BasicMetadataValueEnum::PointerValue(v) => Some(v.get_type().into()),
                    inkwell::values::BasicMetadataValueEnum::StructValue(v) => Some(v.get_type().into()),
                    inkwell::values::BasicMetadataValueEnum::VectorValue(v) => Some(v.get_type().into()),
                    inkwell::values::BasicMetadataValueEnum::MetadataValue(_) => None,
                }
            })
            .collect();

        // Check if return type is unit (empty tuple)
        let fn_type = if matches!(result_ty.kind(), TypeKind::Tuple(types) if types.is_empty()) {
            // Unit return type -> void function
            self.context.void_type().fn_type(&param_types, false)
        } else {
            // Non-unit return type -> use the lowered type
            let ret_ty = self.lower_type(result_ty);
            ret_ty.fn_type(&param_types, false)
        };

        // Cast impl_ptr to the correct function pointer type
        let fn_ptr_type = fn_type.ptr_type(AddressSpace::default());
        let impl_ptr_val = impl_ptr.into_pointer_value();
        let fn_ptr = self.builder
            .build_pointer_cast(impl_ptr_val, fn_ptr_type, "fn_ptr")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), receiver.span)])?;

        // Build the call through the function pointer
        // inkwell's build_call accepts CallableValue which includes function pointers
        let call_site = self.builder
            .build_call(
                inkwell::values::CallableValue::try_from(fn_ptr)
                    .map_err(|_| vec![Diagnostic::error("Invalid function pointer for dispatch", receiver.span)])?,
                compiled_args,
                "dispatch_call",
            )
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), receiver.span)])?;

        Ok(call_site.try_as_basic_value().left())
    }

    /// Mark a method as requiring dynamic dispatch.
    ///
    /// Returns the dispatch slot assigned to this method.
    pub fn mark_dynamic_dispatch(&mut self, method_id: DefId) -> u64 {
        if let Some(&slot) = self.dynamic_dispatch_methods.get(&method_id) {
            slot
        } else {
            let slot = self.next_dispatch_slot;
            self.next_dispatch_slot += 1;
            self.dynamic_dispatch_methods.insert(method_id, slot);
            slot
        }
    }

    /// Register an implementation for a polymorphic method.
    ///
    /// This emits code to call blood_dispatch_register at runtime initialization.
    pub fn emit_dispatch_registration(
        &mut self,
        method_slot: u64,
        type_tag: u64,
        impl_fn: FunctionValue<'ctx>,
        span: Span,
    ) -> Result<(), Vec<Diagnostic>> {
        let i64_type = self.context.i64_type();
        let i8_type = self.context.i8_type();
        let i8_ptr_type = i8_type.ptr_type(AddressSpace::default());

        let dispatch_register_fn = self.module.get_function("blood_dispatch_register")
            .ok_or_else(|| vec![Diagnostic::error(
                "Runtime function blood_dispatch_register not found",
                span,
            )])?;

        let method_slot_val = i64_type.const_int(method_slot, false);
        let type_tag_val = i64_type.const_int(type_tag, false);

        // Cast function to void*
        let impl_ptr = self.builder
            .build_pointer_cast(
                impl_fn.as_global_value().as_pointer_value(),
                i8_ptr_type,
                "impl_void_ptr",
            )
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

        self.builder
            .build_call(
                dispatch_register_fn,
                &[method_slot_val.into(), type_tag_val.into(), impl_ptr.into()],
                "",
            )
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

        Ok(())
    }

    /// Compile a pipe expression: `a |> f` becomes `f(a)`
    ///
    /// The pipe operator passes the left operand as the first argument to
    /// the function on the right.
    fn compile_pipe(
        &mut self,
        arg: &hir::Expr,
        func: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        // Get function to call from the right operand
        let fn_value = match &func.kind {
            hir::ExprKind::Def(def_id) => {
                self.functions.get(def_id).copied()
            }
            _ => None,
        };

        let fn_value = fn_value.ok_or_else(|| vec![Diagnostic::error(
            "Pipe operator requires a function on the right-hand side",
            func.span,
        )])?;

        // Compile the left operand as the argument
        let arg_val = self.compile_expr(arg)?
            .ok_or_else(|| vec![Diagnostic::error(
                "Expected value for pipe argument",
                arg.span,
            )])?;

        // Build call with the piped argument
        let call = self.builder
            .build_call(fn_value, &[arg_val.into()], "pipe_call")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

        // Return the call result, or construct a unit value if void
        call.try_as_basic_value().left()
            .ok_or_else(|| vec![Diagnostic::error(
                "Pipe result has no value (void function)",
                func.span,
            )])
    }

    /// Compile a block.
    fn compile_block(
        &mut self,
        stmts: &[hir::Stmt],
        tail_expr: Option<&hir::Expr>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        for stmt in stmts {
            match stmt {
                hir::Stmt::Let { local_id, init } => {
                    // Get the LLVM type from the init expression
                    let llvm_type = if let Some(init_expr) = init {
                        self.lower_type(&init_expr.ty)
                    } else {
                        // Default to i32 if no initializer
                        self.context.i32_type().into()
                    };

                    // Allocate local with correct type
                    let alloca = self.builder
                        .build_alloca(llvm_type, &format!("local_{}", local_id.index))
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

                    if let Some(init_expr) = init {
                        if let Some(init_val) = self.compile_expr(init_expr)? {
                            self.builder.build_store(alloca, init_val)
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
                        }
                    }

                    self.locals.insert(*local_id, alloca);
                }
                hir::Stmt::Expr(expr) => {
                    self.compile_expr(expr)?;
                }
                hir::Stmt::Item(_) => {
                    // Nested items handled separately
                }
            }
        }

        if let Some(tail) = tail_expr {
            self.compile_expr(tail)
        } else {
            Ok(None)
        }
    }

    /// Compile an if expression.
    fn compile_if(
        &mut self,
        condition: &hir::Expr,
        then_branch: &hir::Expr,
        else_branch: Option<&hir::Expr>,
        result_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        let fn_value = self.current_fn
            .ok_or_else(|| vec![Diagnostic::error("If outside function", Span::dummy())])?;

        let cond_val = self.compile_expr(condition)?
            .ok_or_else(|| vec![Diagnostic::error("Expected condition value", condition.span)])?;

        // Convert to i1 if needed
        let cond_bool = if cond_val.is_int_value() {
            let int_val = cond_val.into_int_value();
            if int_val.get_type().get_bit_width() == 1 {
                int_val
            } else {
                self.builder.build_int_compare(
                    IntPredicate::NE,
                    int_val,
                    int_val.get_type().const_zero(),
                    "tobool",
                ).map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
            }
        } else {
            return Err(vec![Diagnostic::error("Condition must be boolean", condition.span)]);
        };

        let then_bb = self.context.append_basic_block(fn_value, "then");
        let else_bb = self.context.append_basic_block(fn_value, "else");
        let merge_bb = self.context.append_basic_block(fn_value, "merge");

        self.builder.build_conditional_branch(cond_bool, then_bb, else_bb)
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

        // Compile then branch
        self.builder.position_at_end(then_bb);
        let then_val = self.compile_expr(then_branch)?;
        self.builder.build_unconditional_branch(merge_bb)
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
        let then_bb = self.get_current_block()?;

        // Compile else branch
        self.builder.position_at_end(else_bb);
        let else_val = if let Some(else_expr) = else_branch {
            self.compile_expr(else_expr)?
        } else {
            None
        };
        self.builder.build_unconditional_branch(merge_bb)
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
        let else_bb = self.get_current_block()?;

        // Merge
        self.builder.position_at_end(merge_bb);

        // If non-unit result type, create phi node
        if !result_ty.is_unit() {
            if let (Some(then_v), Some(else_v)) = (then_val, else_val) {
                let phi = self.builder.build_phi(self.lower_type(result_ty), "ifresult")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
                phi.add_incoming(&[(&then_v, then_bb), (&else_v, else_bb)]);
                return Ok(Some(phi.as_basic_value()));
            }
        }

        Ok(None)
    }

    /// Compile a while loop.
    fn compile_while(
        &mut self,
        condition: &hir::Expr,
        body: &hir::Expr,
    ) -> Result<(), Vec<Diagnostic>> {
        let fn_value = self.current_fn
            .ok_or_else(|| vec![Diagnostic::error("While outside function", Span::dummy())])?;

        let cond_bb = self.context.append_basic_block(fn_value, "while.cond");
        let body_bb = self.context.append_basic_block(fn_value, "while.body");
        let end_bb = self.context.append_basic_block(fn_value, "while.end");

        // Jump to condition
        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

        // Compile condition
        self.builder.position_at_end(cond_bb);
        let cond_val = self.compile_expr(condition)?
            .ok_or_else(|| vec![Diagnostic::error("Expected condition value", condition.span)])?
            .into_int_value();

        // Ensure boolean
        let cond_bool = if cond_val.get_type().get_bit_width() == 1 {
            cond_val
        } else {
            self.builder.build_int_compare(
                IntPredicate::NE,
                cond_val,
                cond_val.get_type().const_zero(),
                "tobool",
            ).map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
        };

        self.builder.build_conditional_branch(cond_bool, body_bb, end_bb)
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

        // Compile body
        self.builder.position_at_end(body_bb);
        self.compile_expr(body)?;
        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

        // Continue after loop
        self.builder.position_at_end(end_bb);

        Ok(())
    }

    /// Compile a return statement.
    fn compile_return(&mut self, value: Option<&hir::Expr>) -> Result<(), Vec<Diagnostic>> {
        if let Some(val_expr) = value {
            if let Some(val) = self.compile_expr(val_expr)? {
                self.builder.build_return(Some(&val))
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
            } else {
                self.builder.build_return(None)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
            }
        } else {
            self.builder.build_return(None)
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
        }
        Ok(())
    }

    /// Compile an assignment.
    fn compile_assign(&mut self, target: &hir::Expr, value: &hir::Expr) -> Result<(), Vec<Diagnostic>> {
        let val = self.compile_expr(value)?
            .ok_or_else(|| vec![Diagnostic::error("Expected value for assignment", value.span)])?;

        // Get target address
        match &target.kind {
            hir::ExprKind::Local(local_id) => {
                let alloca = self.locals.get(local_id)
                    .ok_or_else(|| vec![Diagnostic::error("Local not found", target.span)])?;
                self.builder.build_store(*alloca, val)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
            }
            _ => {
                return Err(vec![Diagnostic::error(
                    "Cannot assign to this expression",
                    target.span,
                )]);
            }
        }

        Ok(())
    }

    /// Compile a struct construction expression.
    fn compile_struct_expr(
        &mut self,
        result_ty: &Type,
        fields: &[hir::FieldExpr],
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        // Get the LLVM type for the struct
        let struct_llvm_type = self.lower_type(result_ty);

        // Create an undefined struct value
        let struct_type = struct_llvm_type.into_struct_type();
        let mut struct_val = struct_type.get_undef();

        // Fill in each field
        for field in fields {
            let field_val = self.compile_expr(&field.value)?
                .ok_or_else(|| vec![Diagnostic::error(
                    "Expected value for struct field",
                    field.value.span,
                )])?;

            struct_val = self.builder
                .build_insert_value(struct_val, field_val, field.field_idx, "field")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
                .into_struct_value();
        }

        Ok(Some(struct_val.into()))
    }

    /// Compile an enum variant construction expression.
    fn compile_variant(
        &mut self,
        _def_id: DefId,
        variant_idx: u32,
        fields: &[hir::Expr],
        result_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        // Get the LLVM type for the enum
        let enum_llvm_type = self.lower_type(result_ty);

        // Create an undefined enum value
        let enum_type = enum_llvm_type.into_struct_type();
        let mut enum_val = enum_type.get_undef();

        // Set the discriminant (tag) as the first field
        let tag = self.context.i32_type().const_int(variant_idx as u64, false);
        enum_val = self.builder
            .build_insert_value(enum_val, tag, 0, "tag")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
            .into_struct_value();

        // Fill in the variant's fields starting at index 1
        for (i, field_expr) in fields.iter().enumerate() {
            let field_val = self.compile_expr(field_expr)?
                .ok_or_else(|| vec![Diagnostic::error(
                    "Expected value for variant field",
                    field_expr.span,
                )])?;

            // Field index is i + 1 because index 0 is the tag
            enum_val = self.builder
                .build_insert_value(enum_val, field_val, (i + 1) as u32, "variant_field")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
                .into_struct_value();
        }

        Ok(Some(enum_val.into()))
    }

    /// Compile a field access expression.
    fn compile_field_access(
        &mut self,
        base: &hir::Expr,
        field_idx: u32,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        let base_val = self.compile_expr(base)?
            .ok_or_else(|| vec![Diagnostic::error("Expected struct value", base.span)])?;

        // Extract the field from the struct
        let struct_val = base_val.into_struct_value();
        let field_val = self.builder
            .build_extract_value(struct_val, field_idx, "field")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

        Ok(Some(field_val))
    }

    /// Compile a match expression.
    fn compile_match(
        &mut self,
        scrutinee: &hir::Expr,
        arms: &[hir::MatchArm],
        result_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        let fn_value = self.current_fn
            .ok_or_else(|| vec![Diagnostic::error("Match outside function", Span::dummy())])?;

        // Evaluate scrutinee once
        let scrutinee_val = self.compile_expr(scrutinee)?;

        // Create blocks for each arm and merge block
        let merge_bb = self.context.append_basic_block(fn_value, "match.end");

        let mut arm_blocks: Vec<(BasicBlock<'ctx>, BasicBlock<'ctx>)> = Vec::new();
        for (i, _) in arms.iter().enumerate() {
            let test_bb = self.context.append_basic_block(fn_value, &format!("match.arm{}.test", i));
            let body_bb = self.context.append_basic_block(fn_value, &format!("match.arm{}.body", i));
            arm_blocks.push((test_bb, body_bb));
        }

        // Create unreachable block for when no pattern matches
        let unreachable_bb = self.context.append_basic_block(fn_value, "match.unreachable");

        // Jump to first arm's test
        if let Some((first_test, _)) = arm_blocks.first() {
            self.builder.build_unconditional_branch(*first_test)
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
        } else {
            // No arms - should not happen with exhaustive patterns
            self.builder.build_unconditional_branch(merge_bb)
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
        }

        // Collect results for phi node
        let mut incoming: Vec<(BasicValueEnum<'ctx>, BasicBlock<'ctx>)> = Vec::new();

        // Compile each arm
        for (i, arm) in arms.iter().enumerate() {
            let (test_bb, body_bb) = arm_blocks[i];
            let next_test = if i + 1 < arms.len() {
                arm_blocks[i + 1].0
            } else {
                unreachable_bb
            };

            // Test block: check if pattern matches
            self.builder.position_at_end(test_bb);

            let matches = if let Some(scrutinee_val) = &scrutinee_val {
                self.compile_pattern_test(&arm.pattern, scrutinee_val)?
            } else {
                // Scrutinee is unit - only wildcard/binding patterns match
                match &arm.pattern.kind {
                    hir::PatternKind::Wildcard | hir::PatternKind::Binding { .. } => {
                        self.context.bool_type().const_int(1, false)
                    }
                    _ => self.context.bool_type().const_int(0, false),
                }
            };

            // Check guard if present
            let final_match = if let Some(guard) = &arm.guard {
                // If pattern matches, check guard
                let guard_bb = self.context.append_basic_block(fn_value, &format!("match.arm{}.guard", i));
                self.builder.build_conditional_branch(matches, guard_bb, next_test)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

                self.builder.position_at_end(guard_bb);
                // Bind pattern variables for guard evaluation
                if let Some(scrutinee_val) = &scrutinee_val {
                    self.compile_pattern_bindings(&arm.pattern, scrutinee_val)?;
                }

                let guard_val = self.compile_expr(guard)?
                    .ok_or_else(|| vec![Diagnostic::error("Expected guard value", guard.span)])?;

                guard_val.into_int_value()
            } else {
                // Bind pattern variables directly and branch
                self.builder.build_conditional_branch(matches, body_bb, next_test)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
                continue; // Branch already built
            };

            // Branch based on guard result
            self.builder.build_conditional_branch(final_match, body_bb, next_test)
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
        }

        // Build unreachable block
        self.builder.position_at_end(unreachable_bb);
        self.builder.build_unreachable()
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

        // Compile each arm body
        for (i, arm) in arms.iter().enumerate() {
            let (_, body_bb) = arm_blocks[i];
            self.builder.position_at_end(body_bb);

            // Bind pattern variables
            if let Some(scrutinee_val) = &scrutinee_val {
                self.compile_pattern_bindings(&arm.pattern, scrutinee_val)?;
            }

            // Compile body
            let body_val = self.compile_expr(&arm.body)?;

            // Track incoming values for phi
            if let Some(val) = body_val {
                let current_bb = self.get_current_block()?;
                incoming.push((val, current_bb));
            }

            // Jump to merge
            self.builder.build_unconditional_branch(merge_bb)
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
        }

        // Position at merge block
        self.builder.position_at_end(merge_bb);

        // Create phi node if result type is non-unit
        if !result_ty.is_unit() && !incoming.is_empty() {
            let phi = self.builder.build_phi(self.lower_type(result_ty), "match.result")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

            for (val, bb) in &incoming {
                phi.add_incoming(&[(val, *bb)]);
            }

            Ok(Some(phi.as_basic_value()))
        } else {
            Ok(None)
        }
    }

    /// Test if a pattern matches a value.
    /// Returns a boolean i1 value.
    fn compile_pattern_test(
        &mut self,
        pattern: &hir::Pattern,
        scrutinee: &BasicValueEnum<'ctx>,
    ) -> Result<IntValue<'ctx>, Vec<Diagnostic>> {
        match &pattern.kind {
            hir::PatternKind::Wildcard => {
                // Wildcard always matches
                Ok(self.context.bool_type().const_int(1, false))
            }
            hir::PatternKind::Binding { subpattern, .. } => {
                // Binding matches if subpattern matches (or always if no subpattern)
                if let Some(subpat) = subpattern {
                    self.compile_pattern_test(subpat, scrutinee)
                } else {
                    Ok(self.context.bool_type().const_int(1, false))
                }
            }
            hir::PatternKind::Literal(lit) => {
                // Compare scrutinee to literal
                let lit_val = self.compile_literal(lit)?;
                self.compile_value_eq(scrutinee, &lit_val)
            }
            hir::PatternKind::Tuple(patterns) => {
                // All sub-patterns must match
                let struct_val = scrutinee.into_struct_value();
                let mut result = self.context.bool_type().const_int(1, false);

                for (i, sub_pat) in patterns.iter().enumerate() {
                    let elem = self.builder
                        .build_extract_value(struct_val, i as u32, "tuple.elem")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

                    let sub_match = self.compile_pattern_test(sub_pat, &elem)?;
                    result = self.builder
                        .build_and(result, sub_match, "and")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
                }

                Ok(result)
            }
            hir::PatternKind::Struct { fields, .. } => {
                // All field patterns must match
                let struct_val = scrutinee.into_struct_value();
                let mut result = self.context.bool_type().const_int(1, false);

                for field in fields {
                    let field_val = self.builder
                        .build_extract_value(struct_val, field.field_idx, "field")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

                    let sub_match = self.compile_pattern_test(&field.pattern, &field_val)?;
                    result = self.builder
                        .build_and(result, sub_match, "and")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
                }

                Ok(result)
            }
            hir::PatternKind::Or(patterns) => {
                // Any sub-pattern may match
                let mut result = self.context.bool_type().const_int(0, false);

                for sub_pat in patterns {
                    let sub_match = self.compile_pattern_test(sub_pat, scrutinee)?;
                    result = self.builder
                        .build_or(result, sub_match, "or")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
                }

                Ok(result)
            }
            hir::PatternKind::Variant { variant_idx, fields, .. } => {
                // First check discriminant, then check field patterns
                // For now, assume simple enum layout: discriminant + fields
                // This is a simplified implementation - full enum support needs more work
                let _struct_val = scrutinee.into_struct_value();

                // Extract discriminant (assume first field)
                let discriminant = self.builder
                    .build_extract_value(_struct_val, 0, "discrim")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

                let expected = self.context.i32_type().const_int(*variant_idx as u64, false);
                let mut result = self.builder
                    .build_int_compare(IntPredicate::EQ, discriminant.into_int_value(), expected, "variant.check")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

                // Check field patterns (offset by 1 for discriminant)
                for (i, field_pat) in fields.iter().enumerate() {
                    let field_val = self.builder
                        .build_extract_value(_struct_val, (i + 1) as u32, "field")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

                    let sub_match = self.compile_pattern_test(field_pat, &field_val)?;
                    result = self.builder
                        .build_and(result, sub_match, "and")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
                }

                Ok(result)
            }
            hir::PatternKind::Ref { inner, .. } => {
                // Dereference and match inner pattern
                // For now, just match the inner pattern directly (simplified)
                self.compile_pattern_test(inner, scrutinee)
            }
            hir::PatternKind::Slice { prefix, slice, suffix } => {
                // Slice pattern matching - check length and elements
                // Simplified: just check prefix patterns for now
                let mut result = self.context.bool_type().const_int(1, false);

                if let BasicValueEnum::ArrayValue(arr) = scrutinee {
                    for (i, pat) in prefix.iter().enumerate() {
                        let elem = self.builder
                            .build_extract_value(*arr, i as u32, "slice.elem")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

                        let sub_match = self.compile_pattern_test(pat, &elem)?;
                        result = self.builder
                            .build_and(result, sub_match, "and")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
                    }

                    // Handle suffix patterns from the end
                    // This is simplified - real implementation needs length checking
                    let _suffix_count = suffix.len();
                    let _slice_present = slice.is_some();
                    // Phase 2+: Full slice pattern matching requires:
                    // - Runtime length checking for the array/slice
                    // - Computing suffix offsets from the end
                    // - Generating proper failure branches for length mismatches
                }

                Ok(result)
            }
        }
    }

    /// Compile value equality comparison.
    fn compile_value_eq(
        &mut self,
        a: &BasicValueEnum<'ctx>,
        b: &BasicValueEnum<'ctx>,
    ) -> Result<IntValue<'ctx>, Vec<Diagnostic>> {
        match (a, b) {
            (BasicValueEnum::IntValue(a), BasicValueEnum::IntValue(b)) => {
                self.builder
                    .build_int_compare(IntPredicate::EQ, *a, *b, "eq")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])
            }
            (BasicValueEnum::FloatValue(a), BasicValueEnum::FloatValue(b)) => {
                self.builder
                    .build_float_compare(FloatPredicate::OEQ, *a, *b, "eq")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])
            }
            (BasicValueEnum::PointerValue(a), BasicValueEnum::PointerValue(b)) => {
                self.builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        self.builder.build_ptr_to_int(*a, self.context.i64_type(), "ptr_a")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?,
                        self.builder.build_ptr_to_int(*b, self.context.i64_type(), "ptr_b")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?,
                        "eq",
                    )
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])
            }
            (a_val, b_val) => {
                // In a properly type-checked program, we should never compare incompatible types.
                // If we reach here, it's an internal compiler error.
                Err(vec![Diagnostic::error(
                    format!(
                        "ICE: Cannot compare values of incompatible types in pattern match: {:?} vs {:?}. \
                         This indicates a type checking bug.",
                        a_val.get_type(),
                        b_val.get_type()
                    ),
                    Span::dummy(),
                )])
            }
        }
    }

    /// Bind pattern variables to the matched value.
    fn compile_pattern_bindings(
        &mut self,
        pattern: &hir::Pattern,
        scrutinee: &BasicValueEnum<'ctx>,
    ) -> Result<(), Vec<Diagnostic>> {
        match &pattern.kind {
            hir::PatternKind::Wildcard => {
                // No bindings
                Ok(())
            }
            hir::PatternKind::Binding { local_id, subpattern, .. } => {
                // Allocate local and store value
                let llvm_type = self.lower_type(&pattern.ty);
                let alloca = self.builder
                    .build_alloca(llvm_type, &format!("match.bind{}", local_id.index))
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

                self.builder.build_store(alloca, *scrutinee)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

                self.locals.insert(*local_id, alloca);

                // Bind subpattern if present
                if let Some(subpat) = subpattern {
                    self.compile_pattern_bindings(subpat, scrutinee)?;
                }

                Ok(())
            }
            hir::PatternKind::Literal(_) => {
                // No bindings in literals
                Ok(())
            }
            hir::PatternKind::Tuple(patterns) => {
                let struct_val = scrutinee.into_struct_value();
                for (i, sub_pat) in patterns.iter().enumerate() {
                    let elem = self.builder
                        .build_extract_value(struct_val, i as u32, "tuple.elem")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
                    self.compile_pattern_bindings(sub_pat, &elem)?;
                }
                Ok(())
            }
            hir::PatternKind::Struct { fields, .. } => {
                let struct_val = scrutinee.into_struct_value();
                for field in fields {
                    let field_val = self.builder
                        .build_extract_value(struct_val, field.field_idx, "field")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
                    self.compile_pattern_bindings(&field.pattern, &field_val)?;
                }
                Ok(())
            }
            hir::PatternKind::Variant { fields, .. } => {
                let struct_val = scrutinee.into_struct_value();
                // Fields start at index 1 (after discriminant)
                for (i, field_pat) in fields.iter().enumerate() {
                    let field_val = self.builder
                        .build_extract_value(struct_val, (i + 1) as u32, "field")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
                    self.compile_pattern_bindings(field_pat, &field_val)?;
                }
                Ok(())
            }
            hir::PatternKind::Or(patterns) => {
                // Bind from first pattern (all should bind same variables)
                if let Some(first) = patterns.first() {
                    self.compile_pattern_bindings(first, scrutinee)?;
                }
                Ok(())
            }
            hir::PatternKind::Ref { inner, .. } => {
                self.compile_pattern_bindings(inner, scrutinee)
            }
            hir::PatternKind::Slice { prefix, slice, suffix } => {
                if let BasicValueEnum::ArrayValue(arr) = scrutinee {
                    // Bind prefix patterns
                    for (i, pat) in prefix.iter().enumerate() {
                        let elem = self.builder
                            .build_extract_value(*arr, i as u32, "slice.elem")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
                        self.compile_pattern_bindings(pat, &elem)?;
                    }

                    // Handle slice binding (rest pattern)
                    if let Some(slice_pat) = slice {
                        // For now, just bind as-is (simplified)
                        self.compile_pattern_bindings(slice_pat, scrutinee)?;
                    }

                    // Suffix patterns from end - simplified
                    let _suffix_len = suffix.len();
                    // Phase 2+: Full slice pattern bindings require computing
                    // array length at runtime and indexing from the end
                }
                Ok(())
            }
        }
    }

    /// Compile a loop expression.
    fn compile_loop(
        &mut self,
        body: &hir::Expr,
        label: Option<hir::LoopId>,
        result_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        let fn_value = self.current_fn
            .ok_or_else(|| vec![Diagnostic::error("Loop outside function", Span::dummy())])?;

        let loop_bb = self.context.append_basic_block(fn_value, "loop.body");
        let exit_bb = self.context.append_basic_block(fn_value, "loop.exit");

        // Allocate storage for break value if non-unit result type
        let break_value_store = if !result_ty.is_unit() {
            let llvm_type = self.lower_type(result_ty);
            Some(self.builder
                .build_alloca(llvm_type, "loop.result")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?)
        } else {
            None
        };

        // Push loop context
        self.loop_stack.push(LoopContext {
            continue_block: loop_bb,
            exit_block: exit_bb,
            label,
            break_value_store,
        });

        // Jump to loop body
        self.builder.build_unconditional_branch(loop_bb)
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

        // Compile loop body
        self.builder.position_at_end(loop_bb);
        self.compile_expr(body)?;

        // If body didn't terminate, loop back
        if self.get_current_block()?.get_terminator().is_none() {
            self.builder.build_unconditional_branch(loop_bb)
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
        }

        // Pop loop context
        self.loop_stack.pop();

        // Position at exit block
        self.builder.position_at_end(exit_bb);

        // Load break value if present
        if let Some(store) = break_value_store {
            let val = self.builder.build_load(store, "loop.result.load")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
            Ok(Some(val))
        } else {
            Ok(None)
        }
    }

    /// Compile a break statement.
    fn compile_break(
        &mut self,
        label: Option<hir::LoopId>,
        value: Option<&hir::Expr>,
    ) -> Result<(), Vec<Diagnostic>> {
        // Find the loop context
        let loop_ctx = if let Some(label) = label {
            self.loop_stack.iter().rev()
                .find(|ctx| ctx.label == Some(label))
                .cloned()
                .ok_or_else(|| vec![Diagnostic::error(
                    format!("Cannot find loop with label {:?}", label),
                    Span::dummy(),
                )])?
        } else {
            self.loop_stack.last()
                .cloned()
                .ok_or_else(|| vec![Diagnostic::error(
                    "Break outside of loop",
                    Span::dummy(),
                )])?
        };

        // Compile and store break value if present
        if let Some(val_expr) = value {
            if let Some(val) = self.compile_expr(val_expr)? {
                if let Some(store) = loop_ctx.break_value_store {
                    self.builder.build_store(store, val)
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
                }
            }
        }

        // Jump to exit block
        self.builder.build_unconditional_branch(loop_ctx.exit_block)
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

        Ok(())
    }

    /// Compile a continue statement.
    fn compile_continue(&mut self, label: Option<hir::LoopId>) -> Result<(), Vec<Diagnostic>> {
        // Find the loop context
        let loop_ctx = if let Some(label) = label {
            self.loop_stack.iter().rev()
                .find(|ctx| ctx.label == Some(label))
                .cloned()
                .ok_or_else(|| vec![Diagnostic::error(
                    format!("Cannot find loop with label {:?}", label),
                    Span::dummy(),
                )])?
        } else {
            self.loop_stack.last()
                .cloned()
                .ok_or_else(|| vec![Diagnostic::error(
                    "Continue outside of loop",
                    Span::dummy(),
                )])?
        };

        // Jump to continue block
        self.builder.build_unconditional_branch(loop_ctx.continue_block)
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

        Ok(())
    }

    /// Compile an array literal.
    fn compile_array(
        &mut self,
        elements: &[hir::Expr],
        result_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        if elements.is_empty() {
            // Empty array - return undefined array value
            let llvm_type = self.lower_type(result_ty);
            let arr_type = llvm_type.into_array_type();
            return Ok(Some(arr_type.get_undef().into()));
        }

        // Get element type from first element
        let elem_type = self.lower_type(&elements[0].ty);
        let arr_type = elem_type.array_type(elements.len() as u32);
        let mut arr_val = arr_type.get_undef();

        for (i, elem) in elements.iter().enumerate() {
            let elem_val = self.compile_expr(elem)?
                .ok_or_else(|| vec![Diagnostic::error("Expected array element value", elem.span)])?;

            arr_val = self.builder
                .build_insert_value(arr_val, elem_val, i as u32, "arr.elem")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
                .into_array_value();
        }

        Ok(Some(arr_val.into()))
    }

    /// Compile array/slice indexing.
    fn compile_index(
        &mut self,
        base: &hir::Expr,
        index: &hir::Expr,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        let base_val = self.compile_expr(base)?
            .ok_or_else(|| vec![Diagnostic::error("Expected array value", base.span)])?;
        let index_val = self.compile_expr(index)?
            .ok_or_else(|| vec![Diagnostic::error("Expected index value", index.span)])?;

        let idx = index_val.into_int_value();

        // Handle based on base type
        match base_val {
            BasicValueEnum::ArrayValue(arr) => {
                // For array values, we need to use extract_value with constant index
                // or store to memory and use GEP for dynamic index
                // For simplicity, if index is constant, use extract_value
                if let Some(const_idx) = idx.get_zero_extended_constant() {
                    let elem = self.builder
                        .build_extract_value(arr, const_idx as u32, "arr.idx")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
                    Ok(Some(elem))
                } else {
                    // Dynamic index - allocate array on stack and use GEP
                    let arr_type = arr.get_type();
                    let alloca = self.builder
                        .build_alloca(arr_type, "arr.tmp")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
                    self.builder.build_store(alloca, arr)
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

                    let zero = self.context.i32_type().const_int(0, false);
                    let ptr = unsafe {
                        self.builder.build_gep(alloca, &[zero, idx], "arr.elem.ptr")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
                    };

                    let elem = self.builder.build_load(ptr, "arr.elem")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
                    Ok(Some(elem))
                }
            }
            BasicValueEnum::PointerValue(ptr) => {
                // Pointer indexing (for slices/dynamic arrays)
                let elem_ptr = unsafe {
                    self.builder.build_gep(ptr, &[idx], "ptr.idx")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
                };
                let elem = self.builder.build_load(elem_ptr, "elem")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
                Ok(Some(elem))
            }
            _ => {
                Err(vec![Diagnostic::error(
                    "Cannot index non-array type",
                    base.span,
                )])
            }
        }
    }

    /// Compile a type cast expression.
    fn compile_cast(
        &mut self,
        expr: &hir::Expr,
        target_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        let val = self.compile_expr(expr)?
            .ok_or_else(|| vec![Diagnostic::error("Expected value for cast", expr.span)])?;

        let target_llvm = self.lower_type(target_ty);

        // Perform cast based on source and target types
        match (val, target_llvm) {
            // Int to int (sign extend, zero extend, or truncate)
            (BasicValueEnum::IntValue(int_val), BasicTypeEnum::IntType(target_int)) => {
                let source_bits = int_val.get_type().get_bit_width();
                let target_bits = target_int.get_bit_width();

                let result = if target_bits > source_bits {
                    // Extend
                    self.builder.build_int_s_extend(int_val, target_int, "sext")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
                } else if target_bits < source_bits {
                    // Truncate
                    self.builder.build_int_truncate(int_val, target_int, "trunc")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
                } else {
                    // Same size - no-op
                    int_val
                };

                Ok(Some(result.into()))
            }
            // Float to float
            (BasicValueEnum::FloatValue(float_val), BasicTypeEnum::FloatType(target_float)) => {
                let result = self.builder.build_float_cast(float_val, target_float, "fcast")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
                Ok(Some(result.into()))
            }
            // Int to float
            (BasicValueEnum::IntValue(int_val), BasicTypeEnum::FloatType(target_float)) => {
                let result = self.builder.build_signed_int_to_float(int_val, target_float, "sitofp")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
                Ok(Some(result.into()))
            }
            // Float to int
            (BasicValueEnum::FloatValue(float_val), BasicTypeEnum::IntType(target_int)) => {
                let result = self.builder.build_float_to_signed_int(float_val, target_int, "fptosi")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
                Ok(Some(result.into()))
            }
            // Pointer casts
            (BasicValueEnum::PointerValue(ptr), BasicTypeEnum::PointerType(target_ptr)) => {
                let result = self.builder.build_pointer_cast(ptr, target_ptr, "ptrcast")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
                Ok(Some(result.into()))
            }
            // Int to pointer
            (BasicValueEnum::IntValue(int_val), BasicTypeEnum::PointerType(target_ptr)) => {
                let result = self.builder.build_int_to_ptr(int_val, target_ptr, "inttoptr")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
                Ok(Some(result.into()))
            }
            // Pointer to int
            (BasicValueEnum::PointerValue(ptr), BasicTypeEnum::IntType(target_int)) => {
                let result = self.builder.build_ptr_to_int(ptr, target_int, "ptrtoint")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
                Ok(Some(result.into()))
            }
            _ => {
                // Unsupported cast - return value unchanged
                self.errors.push(Diagnostic::warning(
                    "Unsupported cast, returning value unchanged",
                    expr.span,
                ));
                Ok(Some(val))
            }
        }
    }

    // ========================================================================
    // Generational Pointer Support (Phase 2)
    // ========================================================================

    /// Emit a generation check for a pointer access.
    ///
    /// This validates that a pointer's expected generation matches the actual
    /// generation at the memory location, detecting use-after-free errors.
    ///
    /// Returns true (1) if valid, false (0) if stale.
    #[allow(dead_code)]
    fn emit_generation_check(
        &mut self,
        expected_gen: IntValue<'ctx>,
        actual_gen: IntValue<'ctx>,
        span: Span,
    ) -> Result<IntValue<'ctx>, Vec<Diagnostic>> {
        let check_fn = self.module.get_function("blood_check_generation")
            .ok_or_else(|| vec![Diagnostic::error(
                "Runtime function blood_check_generation not found".to_string(),
                span,
            )])?;

        let result = self.builder
            .build_call(check_fn, &[expected_gen.into(), actual_gen.into()], "gen_check")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
            .try_as_basic_value()
            .left()
            .ok_or_else(|| vec![Diagnostic::error(
                "blood_check_generation returned void".to_string(),
                span,
            )])?;

        Ok(result.into_int_value())
    }

    /// Emit a generation check that panics on stale reference.
    ///
    /// This is the standard check used for pointer dereferences. If the
    /// generation doesn't match, it calls blood_stale_reference_panic which
    /// aborts the program with an error message.
    #[allow(dead_code)]
    fn emit_generation_check_or_panic(
        &mut self,
        expected_gen: IntValue<'ctx>,
        actual_gen: IntValue<'ctx>,
        span: Span,
    ) -> Result<(), Vec<Diagnostic>> {
        let check_result = self.emit_generation_check(expected_gen, actual_gen, span)?;

        // Create basic blocks for the check
        let current_fn = self.current_fn.ok_or_else(|| {
            vec![Diagnostic::error("No current function".to_string(), span)]
        })?;

        let valid_block = self.context.append_basic_block(current_fn, "gen.valid");
        let panic_block = self.context.append_basic_block(current_fn, "gen.panic");

        // Branch based on check result (0 = invalid, non-zero = valid)
        let zero = self.context.i32_type().const_int(0, false);
        let is_valid = self.builder
            .build_int_compare(IntPredicate::NE, check_result, zero, "is_gen_valid")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

        self.builder.build_conditional_branch(is_valid, valid_block, panic_block)
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

        // Panic block: call blood_stale_reference_panic
        self.builder.position_at_end(panic_block);
        let panic_fn = self.module.get_function("blood_stale_reference_panic")
            .ok_or_else(|| vec![Diagnostic::error(
                "Runtime function blood_stale_reference_panic not found".to_string(),
                span,
            )])?;

        self.builder
            .build_call(panic_fn, &[expected_gen.into(), actual_gen.into()], "")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

        self.builder.build_unreachable()
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

        // Continue in valid block
        self.builder.position_at_end(valid_block);

        Ok(())
    }

    /// Compile a dereference expression: `*x`
    ///
    /// For generational references, this validates the generation before
    /// accessing the pointed-to value. Currently implements basic pointer
    /// load without generation checking (scaffolded for future integration).
    fn compile_deref(
        &mut self,
        inner: &hir::Expr,
        span: Span,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        let ptr_val = self.compile_expr(inner)?
            .ok_or_else(|| vec![Diagnostic::error("Expected pointer value for dereference", span)])?;

        match ptr_val {
            BasicValueEnum::PointerValue(ptr) => {
                // TODO: For full generational pointer support, we would:
                // 1. Extract expected generation from the BloodPtr struct
                // 2. Call blood_get_generation on the address
                // 3. Call emit_generation_check_or_panic
                //
                // For now, just load through the pointer directly.
                // Generation checking is scaffolded in emit_generation_check_or_panic.

                let loaded = self.builder.build_load(ptr, "deref")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                Ok(Some(loaded))
            }
            _ => {
                Err(vec![Diagnostic::error(
                    format!("Cannot dereference non-pointer type: {:?}", ptr_val.get_type()),
                    span,
                )])
            }
        }
    }

    /// Compile a borrow expression: `&x` or `&mut x`
    ///
    /// Creates a reference to the given expression. For generational references,
    /// this would capture the current generation of the allocation.
    fn compile_borrow(
        &mut self,
        inner: &hir::Expr,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        // For local variables, we get the address of their stack allocation.
        // For other expressions, we need to evaluate and store them first.
        match &inner.kind {
            hir::ExprKind::Local(local_id) => {
                // Get the stack slot for this local
                if let Some(&ptr) = self.locals.get(local_id) {
                    // TODO: For full generational pointer support, we would:
                    // 1. Create a BloodPtr struct
                    // 2. Store address, generation (1 for stack), and metadata
                    //
                    // For now, just return the raw pointer.
                    Ok(Some(ptr.into()))
                } else {
                    Err(vec![Diagnostic::error(
                        "Local variable not found for borrow",
                        inner.span,
                    )])
                }
            }
            hir::ExprKind::Field { base, field_idx } => {
                // Borrow of a field - compute address of the field
                let base_val = self.compile_expr(base)?
                    .ok_or_else(|| vec![Diagnostic::error("Expected struct value", base.span)])?;

                match base_val {
                    BasicValueEnum::PointerValue(ptr) => {
                        // GEP to get field address
                        let zero = self.context.i32_type().const_int(0, false);
                        let field_index = self.context.i32_type().const_int(*field_idx as u64, false);
                        let field_ptr = unsafe {
                            self.builder.build_gep(ptr, &[zero, field_index], "field.ptr")
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), inner.span)])?
                        };
                        Ok(Some(field_ptr.into()))
                    }
                    BasicValueEnum::StructValue(struct_val) => {
                        // Need to store struct first to get address
                        let alloca = self.builder
                            .build_alloca(struct_val.get_type(), "struct.tmp")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), inner.span)])?;
                        self.builder.build_store(alloca, struct_val)
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), inner.span)])?;

                        let zero = self.context.i32_type().const_int(0, false);
                        let field_index = self.context.i32_type().const_int(*field_idx as u64, false);
                        let field_ptr = unsafe {
                            self.builder.build_gep(alloca, &[zero, field_index], "field.ptr")
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), inner.span)])?
                        };
                        Ok(Some(field_ptr.into()))
                    }
                    _ => {
                        Err(vec![Diagnostic::error(
                            "Cannot borrow field of non-struct type",
                            inner.span,
                        )])
                    }
                }
            }
            hir::ExprKind::Index { base, index } => {
                // Borrow of an array element - compute element address
                let base_val = self.compile_expr(base)?
                    .ok_or_else(|| vec![Diagnostic::error("Expected array value", base.span)])?;
                let index_val = self.compile_expr(index)?
                    .ok_or_else(|| vec![Diagnostic::error("Expected index value", index.span)])?;

                let idx = index_val.into_int_value();

                match base_val {
                    BasicValueEnum::PointerValue(ptr) => {
                        let elem_ptr = unsafe {
                            self.builder.build_gep(ptr, &[idx], "elem.ptr")
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), inner.span)])?
                        };
                        Ok(Some(elem_ptr.into()))
                    }
                    BasicValueEnum::ArrayValue(arr) => {
                        // Store array first to get address
                        let alloca = self.builder
                            .build_alloca(arr.get_type(), "arr.tmp")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), inner.span)])?;
                        self.builder.build_store(alloca, arr)
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), inner.span)])?;

                        let zero = self.context.i32_type().const_int(0, false);
                        let elem_ptr = unsafe {
                            self.builder.build_gep(alloca, &[zero, idx], "elem.ptr")
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), inner.span)])?
                        };
                        Ok(Some(elem_ptr.into()))
                    }
                    _ => {
                        Err(vec![Diagnostic::error(
                            "Cannot borrow index of non-array type",
                            inner.span,
                        )])
                    }
                }
            }
            _ => {
                // For other expressions, evaluate and store in a temporary
                let val = self.compile_expr(inner)?
                    .ok_or_else(|| vec![Diagnostic::error("Cannot borrow void expression", inner.span)])?;

                let ty = val.get_type();
                let alloca = self.builder
                    .build_alloca(ty, "borrow.tmp")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), inner.span)])?;
                self.builder.build_store(alloca, val)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), inner.span)])?;

                Ok(Some(alloca.into()))
            }
        }
    }

    /// Compile an address-of expression for raw pointers.
    ///
    /// Creates a raw pointer without generation tracking.
    fn compile_addr_of(
        &mut self,
        inner: &hir::Expr,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        // For addr_of, we use the same logic as borrow but without
        // any generation tracking (raw pointers don't have generations).
        self.compile_borrow(inner)
    }

    /// Compile a closure expression: `|x| x + 1`
    ///
    /// Closures are compiled as:
    /// 1. An environment struct containing captured variables
    /// 2. A function that takes the environment as its first parameter
    /// 3. A fat pointer struct containing (fn_ptr, env_ptr)
    fn compile_closure(
        &mut self,
        body_id: hir::BodyId,
        captures: &[hir::Capture],
        closure_ty: &Type,
        span: Span,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
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
            TypeKind::Fn { params, ret } => {
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
        let i8_ptr_type = self.context.i8_type().ptr_type(AddressSpace::default());
        let mut env_types: Vec<BasicTypeEnum<'ctx>> = Vec::with_capacity(captures.len());
        for cap in captures {
            if let Some(alloca) = self.locals.get(&cap.local_id) {
                let elem_type = alloca.get_type().get_element_type();
                match elem_type.try_into() {
                    Ok(basic_type) => env_types.push(basic_type),
                    Err(_) => {
                        // If the captured variable has a non-basic type (function, void),
                        // this is an internal compiler error - closures should only capture
                        // values with basic types.
                        return Err(vec![Diagnostic::error(
                            format!(
                                "ICE: Closure capture has non-basic type: {:?}. \
                                 Closures can only capture values with basic types.",
                                elem_type
                            ),
                            span,
                        )]);
                    }
                }
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

        // Compile the closure body
        self.current_fn = Some(fn_value);
        let entry = self.context.append_basic_block(fn_value, "entry");
        self.builder.position_at_end(entry);

        // Set up environment access - first parameter is the environment pointer
        let env_ptr = fn_value.get_first_param()
            .ok_or_else(|| vec![Diagnostic::error("Closure missing env parameter", span)])?
            .into_pointer_value();
        let typed_env_ptr = self.builder
            .build_pointer_cast(env_ptr, env_struct_type.ptr_type(AddressSpace::default()), "env")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

        // Map captured variables from environment struct
        for (i, cap) in captures.iter().enumerate() {
            if let Some(&outer_alloca) = saved_locals.get(&cap.local_id) {
                // Get the type of the captured variable from the outer local
                let cap_type = outer_alloca.get_type().get_element_type();
                let cap_basic_type: BasicTypeEnum<'ctx> = cap_type.try_into()
                    .unwrap_or(self.context.i64_type().into());

                let zero = self.context.i32_type().const_int(0, false);
                let idx = self.context.i32_type().const_int(i as u64, false);
                let cap_ptr = unsafe {
                    self.builder.build_gep(typed_env_ptr, &[zero, idx], "cap.ptr")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                };
                let local_alloca = self.builder
                    .build_alloca(cap_basic_type, "cap.local")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                let cap_val = self.builder.build_load(cap_ptr, "cap.val")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                self.builder.build_store(local_alloca, cap_val)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                self.locals.insert(cap.local_id, local_alloca);
            }
        }

        // Set up parameters (skip first env param)
        let param_locals: Vec<_> = body.params().collect();
        for (i, local) in param_locals.iter().enumerate() {
            let param_idx = (i + 1) as u32;
            if let Some(param_val) = fn_value.get_nth_param(param_idx) {
                let alloca = self.builder
                    .build_alloca(param_val.get_type(), &format!("param.{}", i))
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                self.builder.build_store(alloca, param_val)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;
                self.locals.insert(local.id, alloca);
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

        // Restore context
        self.current_fn = saved_fn;
        self.locals = saved_locals;

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
                    self.builder.build_gep(env_alloca, &[zero, idx], "env.field")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?
                };
                // Load the captured value and store directly with its actual type
                let val = self.builder.build_load(cap_ptr, "cap.val")
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

    // ========================================================================
    // Effects System Codegen (Phase 2)
    // ========================================================================

    /// Compile a perform expression: `perform Effect.op(args)`
    ///
    /// In the evidence passing model (ICFP'21), this becomes a call through
    /// the evidence vector via `blood_perform` runtime function.
    ///
    /// Implementation follows:
    /// - [Generalized Evidence Passing for Effect Handlers](https://dl.acm.org/doi/10.1145/3473576) (ICFP'21)
    /// - [Zero-Overhead Lexical Effect Handlers](https://doi.org/10.1145/3763177) (OOPSLA'25)
    fn compile_perform(
        &mut self,
        effect_id: DefId,
        op_index: u32,
        args: &[hir::Expr],
        result_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        let i64_type = self.context.i64_type();
        let i32_type = self.context.i32_type();

        // Get blood_perform runtime function
        let perform_fn = self.module.get_function("blood_perform")
            .ok_or_else(|| vec![Diagnostic::error(
                "Runtime function blood_perform not found".to_string(),
                Span::dummy(),
            )])?;

        // Compile arguments and pack into an i64 array
        let mut compiled_args = Vec::with_capacity(args.len());
        for arg in args {
            if let Some(val) = self.compile_expr(arg)? {
                // Convert to i64 for uniform argument passing
                let i64_val = match val {
                    BasicValueEnum::IntValue(iv) => {
                        if iv.get_type().get_bit_width() == 64 {
                            iv
                        } else {
                            self.builder
                                .build_int_s_extend(iv, i64_type, "arg_ext")
                                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
                        }
                    }
                    BasicValueEnum::FloatValue(fv) => {
                        // Bitcast float to i64 for passing through uniform interface
                        self.builder
                            .build_bit_cast(fv, i64_type, "float_as_i64")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
                            .into_int_value()
                    }
                    BasicValueEnum::PointerValue(pv) => {
                        self.builder
                            .build_ptr_to_int(pv, i64_type, "ptr_as_i64")
                            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
                    }
                    other => {
                        return Err(vec![Diagnostic::error(
                            format!(
                                "ICE: Unsupported argument type in perform expression: {:?}. \
                                 Expected IntValue, FloatValue, or PointerValue.",
                                other.get_type()
                            ),
                            arg.span,
                        )]);
                    }
                };
                compiled_args.push(i64_val);
            }
        }

        // Allocate stack space for arguments array
        let arg_count = compiled_args.len();
        let args_array = if arg_count > 0 {
            let array_type = i64_type.array_type(arg_count as u32);
            let args_alloca = self.builder
                .build_alloca(array_type, "perform_args")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

            // Store each argument in the array
            let zero = i32_type.const_zero();
            for (i, arg_val) in compiled_args.iter().enumerate() {
                let idx = i32_type.const_int(i as u64, false);
                let gep = unsafe {
                    self.builder.build_gep(
                        args_alloca,
                        &[zero, idx],
                        &format!("arg_ptr_{}", i),
                    )
                }.map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

                self.builder
                    .build_store(gep, *arg_val)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
            }

            // Get pointer to first element
            unsafe {
                self.builder.build_gep(
                    args_alloca,
                    &[zero, zero],
                    "args_ptr",
                )
            }.map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
        } else {
            // No arguments - pass null pointer
            i64_type.ptr_type(inkwell::AddressSpace::default()).const_null()
        };

        // Call blood_perform(effect_id, op_index, args, arg_count)
        let effect_id_val = i64_type.const_int(effect_id.index as u64, false);
        let op_index_val = i32_type.const_int(op_index as u64, false);
        let arg_count_val = i64_type.const_int(arg_count as u64, false);

        let call_result = self.builder
            .build_call(
                perform_fn,
                &[
                    effect_id_val.into(),
                    op_index_val.into(),
                    args_array.into(),
                    arg_count_val.into(),
                ],
                "perform_result",
            )
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

        // Get the result value and convert to appropriate type
        if result_ty.is_unit() {
            Ok(None)
        } else {
            let result_i64 = call_result.try_as_basic_value().left()
                .ok_or_else(|| vec![Diagnostic::error(
                    "blood_perform returned void unexpectedly".to_string(),
                    Span::dummy(),
                )])?;

            // Convert result from i64 to expected type
            let result_llvm_type = self.lower_type(result_ty);
            let converted_result = if result_llvm_type.is_int_type() {
                let result_int_type = result_llvm_type.into_int_type();
                if result_int_type.get_bit_width() == 64 {
                    result_i64
                } else {
                    self.builder
                        .build_int_truncate(result_i64.into_int_value(), result_int_type, "result_trunc")
                        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
                        .into()
                }
            } else if result_llvm_type.is_float_type() {
                self.builder
                    .build_bit_cast(result_i64, result_llvm_type, "result_float")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
            } else if result_llvm_type.is_pointer_type() {
                self.builder
                    .build_int_to_ptr(result_i64.into_int_value(), result_llvm_type.into_pointer_type(), "result_ptr")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
                    .into()
            } else {
                result_i64
            };

            Ok(Some(converted_result))
        }
    }

    /// Compile a resume expression: `resume(value)`
    ///
    /// For tail-resumptive handlers, resume is a simple return.
    /// For general handlers, this requires continuation capture (Phase 2.3).
    fn compile_resume(
        &mut self,
        value: Option<&hir::Expr>,
        _result_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        // Phase 2.1: Tail-resumptive optimization only
        //
        // For tail-resumptive handlers (State, Reader, Writer), resume at tail
        // position is just returning the value.

        if let Some(val_expr) = value {
            let val = self.compile_expr(val_expr)?;
            if let Some(ret_val) = val {
                self.builder.build_return(Some(&ret_val))
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
            } else {
                self.builder.build_return(None)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
            }
        } else {
            self.builder.build_return(None)
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
        }

        // Resume doesn't produce a value (control flow transfers)
        Ok(None)
    }

    /// Compile a handle expression: `handle { body } with { handler }`
    ///
    /// This sets up the evidence vector and runs the body with the handler
    /// installed.
    fn compile_handle(
        &mut self,
        body: &hir::Expr,
        handler_id: DefId,
        result_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        // Phase 2.4: Evidence vector setup
        //
        // 1. Create evidence vector (or get existing one)
        // 2. Push handler onto evidence vector
        // 3. Compile body
        // 4. Pop handler from evidence vector
        // 5. Return result

        // Get runtime functions
        let ev_create = self.module.get_function("blood_evidence_create")
            .ok_or_else(|| vec![Diagnostic::error(
                "Runtime function blood_evidence_create not found".to_string(),
                Span::dummy(),
            )])?;
        let ev_push = self.module.get_function("blood_evidence_push")
            .ok_or_else(|| vec![Diagnostic::error(
                "Runtime function blood_evidence_push not found".to_string(),
                Span::dummy(),
            )])?;
        let ev_pop = self.module.get_function("blood_evidence_pop")
            .ok_or_else(|| vec![Diagnostic::error(
                "Runtime function blood_evidence_pop not found".to_string(),
                Span::dummy(),
            )])?;
        let ev_destroy = self.module.get_function("blood_evidence_destroy")
            .ok_or_else(|| vec![Diagnostic::error(
                "Runtime function blood_evidence_destroy not found".to_string(),
                Span::dummy(),
            )])?;

        // Create evidence vector
        let ev = self.builder.build_call(ev_create, &[], "evidence")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?
            .try_as_basic_value()
            .left()
            .ok_or_else(|| vec![Diagnostic::error(
                "blood_evidence_create returned void".to_string(),
                Span::dummy(),
            )])?;

        // Push handler ID onto evidence vector as i64
        let handler_ptr = self.context.i64_type().const_int(handler_id.index as u64, false);
        self.builder.build_call(ev_push, &[ev.into(), handler_ptr.into()], "")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

        // Compile the body
        let result = self.compile_expr(body)?;

        // Pop handler from evidence vector
        self.builder.build_call(ev_pop, &[ev.into()], "")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

        // Destroy evidence vector
        self.builder.build_call(ev_destroy, &[ev.into()], "")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;

        // Return result with proper type
        if result_ty.is_unit() {
            Ok(None)
        } else {
            Ok(result)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::{self, Crate, Item, ItemKind, Body, BodyId, Type, Expr, ExprKind, LiteralValue, Local, LocalId, DefId};
    use crate::span::Span;
    use std::collections::HashMap;

    /// Helper to create a simple HIR crate for testing.
    fn make_test_crate(body_expr: Expr, return_type: Type) -> Crate {
        let def_id = DefId::new(0);
        let body_id = BodyId::new(0);

        let fn_sig = hir::FnSig {
            inputs: Vec::new(),
            output: return_type.clone(),
            is_const: false,
            is_async: false,
            is_unsafe: false,
            generics: Vec::new(),
        };

        let fn_def = hir::FnDef {
            sig: fn_sig,
            body_id: Some(body_id),
            generics: hir::Generics {
                params: Vec::new(),
                predicates: Vec::new(),
            },
        };

        let item = Item {
            name: "test_fn".to_string(),
            kind: ItemKind::Fn(fn_def),
            def_id,
            span: Span::dummy(),
            vis: crate::ast::Visibility::Public,
        };

        // Create the return place local (index 0)
        let return_local = Local {
            id: LocalId::new(0),
            ty: return_type.clone(),
            mutable: false,
            name: None,
            span: Span::dummy(),
        };

        let body = Body {
            locals: vec![return_local],
            param_count: 0,
            expr: body_expr,
            span: Span::dummy(),
        };

        let mut items = HashMap::new();
        items.insert(def_id, item);

        let mut bodies = HashMap::new();
        bodies.insert(body_id, body);

        Crate {
            items,
            bodies,
            entry: None,
            builtin_fns: HashMap::new(),
        }
    }

    fn i32_type() -> Type {
        Type::i32()
    }

    fn f64_type() -> Type {
        Type::f64()
    }

    fn bool_type() -> Type {
        Type::bool()
    }

    fn unit_type() -> Type {
        Type::unit()
    }

    fn int_literal(val: i128) -> Expr {
        Expr {
            kind: ExprKind::Literal(LiteralValue::Int(val)),
            ty: i32_type(),
            span: Span::dummy(),
        }
    }

    fn float_literal(val: f64) -> Expr {
        Expr {
            kind: ExprKind::Literal(LiteralValue::Float(val)),
            ty: f64_type(),
            span: Span::dummy(),
        }
    }

    fn bool_literal(val: bool) -> Expr {
        Expr {
            kind: ExprKind::Literal(LiteralValue::Bool(val)),
            ty: bool_type(),
            span: Span::dummy(),
        }
    }

    fn binary_expr(op: crate::ast::BinOp, left: Expr, right: Expr, result_ty: Type) -> Expr {
        Expr {
            kind: ExprKind::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            },
            ty: result_ty,
            span: Span::dummy(),
        }
    }

    fn unary_expr(op: crate::ast::UnaryOp, operand: Expr, result_ty: Type) -> Expr {
        Expr {
            kind: ExprKind::Unary {
                op,
                operand: Box::new(operand),
            },
            ty: result_ty,
            span: Span::dummy(),
        }
    }

    // ==================== LITERAL TESTS ====================

    #[test]
    fn test_codegen_int_literal() {
        let expr = int_literal(42);
        let hir_crate = make_test_crate(expr, i32_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "Integer literal codegen failed: {:?}", result.err());
    }

    #[test]
    fn test_codegen_float_literal() {
        let expr = float_literal(2.5);
        let hir_crate = make_test_crate(expr, f64_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "Float literal codegen failed: {:?}", result.err());
    }

    #[test]
    fn test_codegen_bool_literal() {
        let expr = bool_literal(true);
        let hir_crate = make_test_crate(expr, bool_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "Bool literal codegen failed: {:?}", result.err());
    }

    #[test]
    fn test_codegen_string_literal() {
        let expr = Expr {
            kind: ExprKind::Literal(LiteralValue::String("hello".to_string())),
            ty: Type::str(),
            span: Span::dummy(),
        };
        let hir_crate = make_test_crate(expr, Type::str());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "String literal codegen failed: {:?}", result.err());
    }

    // ==================== BINARY OPERATION TESTS ====================

    #[test]
    fn test_codegen_int_add() {
        use crate::ast::BinOp;
        let expr = binary_expr(BinOp::Add, int_literal(1), int_literal(2), i32_type());
        let hir_crate = make_test_crate(expr, i32_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "Int add codegen failed: {:?}", result.err());
    }

    #[test]
    fn test_codegen_int_sub() {
        use crate::ast::BinOp;
        let expr = binary_expr(BinOp::Sub, int_literal(5), int_literal(3), i32_type());
        let hir_crate = make_test_crate(expr, i32_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "Int sub codegen failed: {:?}", result.err());
    }

    #[test]
    fn test_codegen_int_mul() {
        use crate::ast::BinOp;
        let expr = binary_expr(BinOp::Mul, int_literal(4), int_literal(5), i32_type());
        let hir_crate = make_test_crate(expr, i32_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "Int mul codegen failed: {:?}", result.err());
    }

    #[test]
    fn test_codegen_int_div() {
        use crate::ast::BinOp;
        let expr = binary_expr(BinOp::Div, int_literal(10), int_literal(2), i32_type());
        let hir_crate = make_test_crate(expr, i32_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "Int div codegen failed: {:?}", result.err());
    }

    #[test]
    fn test_codegen_int_compare() {
        use crate::ast::BinOp;
        let expr = binary_expr(BinOp::Lt, int_literal(1), int_literal(2), bool_type());
        let hir_crate = make_test_crate(expr, bool_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "Int compare codegen failed: {:?}", result.err());
    }

    #[test]
    fn test_codegen_float_add() {
        use crate::ast::BinOp;
        let expr = binary_expr(BinOp::Add, float_literal(1.5), float_literal(2.5), f64_type());
        let hir_crate = make_test_crate(expr, f64_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "Float add codegen failed: {:?}", result.err());
    }

    #[test]
    fn test_codegen_float_mul() {
        use crate::ast::BinOp;
        let expr = binary_expr(BinOp::Mul, float_literal(2.0), float_literal(3.0), f64_type());
        let hir_crate = make_test_crate(expr, f64_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "Float mul codegen failed: {:?}", result.err());
    }

    #[test]
    fn test_codegen_float_compare() {
        use crate::ast::BinOp;
        let expr = binary_expr(BinOp::Gt, float_literal(2.5), float_literal(2.71), bool_type());
        let hir_crate = make_test_crate(expr, bool_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "Float compare codegen failed: {:?}", result.err());
    }

    // ==================== UNARY OPERATION TESTS ====================

    #[test]
    fn test_codegen_int_neg() {
        use crate::ast::UnaryOp;
        let expr = unary_expr(UnaryOp::Neg, int_literal(42), i32_type());
        let hir_crate = make_test_crate(expr, i32_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "Int neg codegen failed: {:?}", result.err());
    }

    #[test]
    fn test_codegen_float_neg() {
        use crate::ast::UnaryOp;
        let expr = unary_expr(UnaryOp::Neg, float_literal(2.5), f64_type());
        let hir_crate = make_test_crate(expr, f64_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "Float neg codegen failed: {:?}", result.err());
    }

    #[test]
    fn test_codegen_int_not() {
        use crate::ast::UnaryOp;
        let expr = unary_expr(UnaryOp::Not, int_literal(0xFF), i32_type());
        let hir_crate = make_test_crate(expr, i32_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "Int not codegen failed: {:?}", result.err());
    }

    // ==================== CONTROL FLOW TESTS ====================

    #[test]
    fn test_codegen_if_expr() {
        let condition = bool_literal(true);
        let then_branch = int_literal(1);
        let else_branch = int_literal(0);

        let expr = Expr {
            kind: ExprKind::If {
                condition: Box::new(condition),
                then_branch: Box::new(then_branch),
                else_branch: Some(Box::new(else_branch)),
            },
            ty: i32_type(),
            span: Span::dummy(),
        };

        let hir_crate = make_test_crate(expr, i32_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "If expr codegen failed: {:?}", result.err());
    }

    #[test]
    fn test_codegen_while_loop() {
        let condition = bool_literal(false); // Loop that never executes
        let body = Expr {
            kind: ExprKind::Tuple(Vec::new()),
            ty: unit_type(),
            span: Span::dummy(),
        };

        let while_expr = Expr {
            kind: ExprKind::While {
                condition: Box::new(condition),
                body: Box::new(body),
                label: None,
            },
            ty: unit_type(),
            span: Span::dummy(),
        };

        let block_expr = Expr {
            kind: ExprKind::Block {
                stmts: Vec::new(),
                expr: Some(Box::new(while_expr)),
            },
            ty: unit_type(),
            span: Span::dummy(),
        };

        let hir_crate = make_test_crate(block_expr, unit_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "While loop codegen failed: {:?}", result.err());
    }

    // ==================== BLOCK AND LET TESTS ====================

    #[test]
    fn test_codegen_block_with_let() {
        let init_expr = int_literal(42);
        let local_id = LocalId { index: 0 };

        let let_stmt = hir::Stmt::Let {
            local_id,
            init: Some(init_expr),
        };

        let load_expr = Expr {
            kind: ExprKind::Local(local_id),
            ty: i32_type(),
            span: Span::dummy(),
        };

        let block_expr = Expr {
            kind: ExprKind::Block {
                stmts: vec![let_stmt],
                expr: Some(Box::new(load_expr)),
            },
            ty: i32_type(),
            span: Span::dummy(),
        };

        let hir_crate = make_test_crate(block_expr, i32_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "Block with let codegen failed: {:?}", result.err());
    }

    // ==================== TUPLE TESTS ====================

    #[test]
    fn test_codegen_tuple_empty() {
        let expr = Expr {
            kind: ExprKind::Tuple(Vec::new()),
            ty: unit_type(),
            span: Span::dummy(),
        };

        let hir_crate = make_test_crate(expr, unit_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "Empty tuple codegen failed: {:?}", result.err());
    }

    #[test]
    fn test_codegen_tuple_with_values() {
        let tuple_ty = Type::tuple(vec![i32_type(), bool_type()]);

        let expr = Expr {
            kind: ExprKind::Tuple(vec![int_literal(42), bool_literal(true)]),
            ty: tuple_ty.clone(),
            span: Span::dummy(),
        };

        let hir_crate = make_test_crate(expr, tuple_ty);

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "Tuple with values codegen failed: {:?}", result.err());
    }

    // ==================== ARRAY TESTS ====================

    #[test]
    fn test_codegen_array_literal() {
        let arr_ty = Type::array(i32_type(), 3);

        let expr = Expr {
            kind: ExprKind::Array(vec![int_literal(1), int_literal(2), int_literal(3)]),
            ty: arr_ty.clone(),
            span: Span::dummy(),
        };

        let hir_crate = make_test_crate(expr, arr_ty);

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "Array literal codegen failed: {:?}", result.err());
    }

    #[test]
    fn test_codegen_array_empty() {
        let arr_ty = Type::array(i32_type(), 0);

        let expr = Expr {
            kind: ExprKind::Array(Vec::new()),
            ty: arr_ty.clone(),
            span: Span::dummy(),
        };

        let hir_crate = make_test_crate(expr, arr_ty);

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "Empty array codegen failed: {:?}", result.err());
    }

    // ==================== TYPE LOWERING TESTS ====================

    #[test]
    fn test_lower_primitive_types() {
        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let codegen = CodegenContext::new(&context, &module, &builder);

        // Test various primitive types
        let i32_t = codegen.lower_type(&i32_type());
        assert!(i32_t.is_int_type());

        let f64_t = codegen.lower_type(&f64_type());
        assert!(f64_t.is_float_type());

        let bool_t = codegen.lower_type(&bool_type());
        assert!(bool_t.is_int_type()); // bool is i1

        let unit_t = codegen.lower_type(&unit_type());
        assert!(unit_t.is_int_type()); // unit is i8 placeholder
    }

    #[test]
    fn test_lower_tuple_type() {
        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let codegen = CodegenContext::new(&context, &module, &builder);

        let tuple_ty = Type::tuple(vec![i32_type(), f64_type()]);

        let lowered = codegen.lower_type(&tuple_ty);
        assert!(lowered.is_struct_type());
    }

    #[test]
    fn test_lower_array_type() {
        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let codegen = CodegenContext::new(&context, &module, &builder);

        let arr_ty = Type::array(i32_type(), 5);

        let lowered = codegen.lower_type(&arr_ty);
        assert!(lowered.is_array_type());
    }

    // ==================== COMPLEX EXPRESSION TESTS ====================

    #[test]
    fn test_codegen_nested_binary_ops() {
        use crate::ast::BinOp;
        // (1 + 2) * 3
        let add_expr = binary_expr(BinOp::Add, int_literal(1), int_literal(2), i32_type());
        let mul_expr = binary_expr(BinOp::Mul, add_expr, int_literal(3), i32_type());

        let hir_crate = make_test_crate(mul_expr, i32_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "Nested binary ops codegen failed: {:?}", result.err());
    }

    #[test]
    fn test_codegen_nested_if() {
        let inner_if = Expr {
            kind: ExprKind::If {
                condition: Box::new(bool_literal(true)),
                then_branch: Box::new(int_literal(1)),
                else_branch: Some(Box::new(int_literal(2))),
            },
            ty: i32_type(),
            span: Span::dummy(),
        };

        let outer_if = Expr {
            kind: ExprKind::If {
                condition: Box::new(bool_literal(false)),
                then_branch: Box::new(int_literal(0)),
                else_branch: Some(Box::new(inner_if)),
            },
            ty: i32_type(),
            span: Span::dummy(),
        };

        let hir_crate = make_test_crate(outer_if, i32_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "Nested if codegen failed: {:?}", result.err());
    }

    // ==================== BITWISE OPERATION TESTS ====================

    #[test]
    fn test_codegen_bitwise_and() {
        use crate::ast::BinOp;
        let expr = binary_expr(BinOp::BitAnd, int_literal(0xFF), int_literal(0x0F), i32_type());
        let hir_crate = make_test_crate(expr, i32_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "Bitwise AND codegen failed: {:?}", result.err());
    }

    #[test]
    fn test_codegen_bitwise_or() {
        use crate::ast::BinOp;
        let expr = binary_expr(BinOp::BitOr, int_literal(0xF0), int_literal(0x0F), i32_type());
        let hir_crate = make_test_crate(expr, i32_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "Bitwise OR codegen failed: {:?}", result.err());
    }

    #[test]
    fn test_codegen_shift_left() {
        use crate::ast::BinOp;
        let expr = binary_expr(BinOp::Shl, int_literal(1), int_literal(4), i32_type());
        let hir_crate = make_test_crate(expr, i32_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "Shift left codegen failed: {:?}", result.err());
    }

    #[test]
    fn test_codegen_shift_right() {
        use crate::ast::BinOp;
        let expr = binary_expr(BinOp::Shr, int_literal(16), int_literal(2), i32_type());
        let hir_crate = make_test_crate(expr, i32_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "Shift right codegen failed: {:?}", result.err());
    }

    // ========================================================================
    // Effects System Codegen Tests (Phase 2)
    // ========================================================================

    /// Test perform expression generates blood_perform runtime call
    ///
    /// With the evidence passing model (ICFP'21), handler lookup is deferred to runtime.
    /// Compilation succeeds and emits a call to blood_perform, which will dispatch
    /// to the appropriate handler at runtime.
    #[test]
    fn test_codegen_perform_basic() {
        let effect_id = DefId::new(100);
        let expr = Expr {
            kind: ExprKind::Perform {
                effect_id,
                op_index: 0,
                args: vec![int_literal(42)],
            },
            ty: i32_type(),
            span: Span::dummy(),
        };
        let hir_crate = make_test_crate(expr, i32_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        // With runtime dispatch, compilation succeeds - handler lookup happens at runtime
        assert!(result.is_ok(), "Perform codegen should succeed: {:?}", result.err());

        // Verify blood_perform function is declared
        assert!(module.get_function("blood_perform").is_some(),
            "blood_perform should be declared");
    }

    /// Test perform with no arguments generates correct runtime call
    #[test]
    fn test_codegen_perform_no_args() {
        let effect_id = DefId::new(101);
        let expr = Expr {
            kind: ExprKind::Perform {
                effect_id,
                op_index: 1,
                args: vec![],
            },
            ty: unit_type(),
            span: Span::dummy(),
        };
        let hir_crate = make_test_crate(expr, unit_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        // With runtime dispatch, compilation succeeds even without handlers
        assert!(result.is_ok(), "Perform codegen should succeed: {:?}", result.err());
    }

    /// Test resume expression (tail-resumptive)
    #[test]
    fn test_codegen_resume_with_value() {
        let expr = Expr {
            kind: ExprKind::Resume {
                value: Some(Box::new(int_literal(42))),
            },
            ty: Type::never(),
            span: Span::dummy(),
        };
        let hir_crate = make_test_crate(expr, unit_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "Resume codegen failed: {:?}", result.err());
    }

    /// Test resume without value (unit resume)
    #[test]
    fn test_codegen_resume_unit() {
        let expr = Expr {
            kind: ExprKind::Resume { value: None },
            ty: Type::never(),
            span: Span::dummy(),
        };
        let hir_crate = make_test_crate(expr, unit_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "Resume unit codegen failed: {:?}", result.err());
    }

    /// Test handle expression wraps body
    #[test]
    fn test_codegen_handle_basic() {
        let handler_id = DefId::new(200);
        let body = int_literal(42);
        let expr = Expr {
            kind: ExprKind::Handle {
                body: Box::new(body),
                handler_id,
            },
            ty: i32_type(),
            span: Span::dummy(),
        };
        let hir_crate = make_test_crate(expr, i32_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "Handle codegen failed: {:?}", result.err());
    }

    /// Test handle with unit body
    #[test]
    fn test_codegen_handle_unit() {
        let handler_id = DefId::new(201);
        let body = Expr {
            kind: ExprKind::Tuple(Vec::new()),
            ty: unit_type(),
            span: Span::dummy(),
        };
        let expr = Expr {
            kind: ExprKind::Handle {
                body: Box::new(body),
                handler_id,
            },
            ty: unit_type(),
            span: Span::dummy(),
        };
        let hir_crate = make_test_crate(expr, unit_type());

        let context = Context::create();
        let module = context.create_module("test");
        let builder = context.create_builder();

        let mut codegen = CodegenContext::new(&context, &module, &builder);
        let result = codegen.compile_crate(&hir_crate);

        assert!(result.is_ok(), "Handle unit codegen failed: {:?}", result.err());
    }
}
