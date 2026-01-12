//! Code generation context.
//!
//! This module provides the main code generation context which
//! coordinates LLVM code generation for a Blood program.

use std::collections::HashMap;

use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::builder::Builder;
use inkwell::basic_block::BasicBlock;
use inkwell::values::{FunctionValue, PointerValue};
use inkwell::types::BasicType;
use inkwell::AddressSpace;

use crate::hir::{self, DefId, LocalId, Type};
use crate::mir::{EscapeResults, MirBody};
use crate::diagnostics::Diagnostic;
use crate::span::Span;
use crate::effects::{EffectLowering, EffectInfo, HandlerInfo};

// Submodules
mod types;
mod handlers;
mod expr;
mod control;
mod patterns;
mod dispatch;
mod effects;
mod closures;

#[cfg(test)]
mod tests;

/// Loop context for break/continue support.
#[derive(Clone)]
pub(super) struct LoopContext<'ctx> {
    /// The loop's continue block (condition or body start).
    pub continue_block: BasicBlock<'ctx>,
    /// The loop's exit block.
    pub exit_block: BasicBlock<'ctx>,
    /// Optional label for named loops.
    pub label: Option<hir::LoopId>,
    /// Storage for break values (for loop expressions that return values).
    pub break_value_store: Option<PointerValue<'ctx>>,
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
    pub(super) loop_stack: Vec<LoopContext<'ctx>>,
    /// Closure bodies for compilation (copied from HIR crate during compile_crate).
    pub(super) closure_bodies: HashMap<hir::BodyId, hir::Body>,
    /// Counter for generating unique closure function names.
    pub(super) closure_counter: u32,
    /// Methods requiring dynamic dispatch (polymorphic methods).
    /// Maps method DefId to the dispatch table slot.
    pub(super) dynamic_dispatch_methods: HashMap<DefId, u64>,
    /// Next dispatch table slot to assign.
    pub(super) next_dispatch_slot: u64,
    /// Escape analysis results for optimization.
    /// When available, used to skip generation checks for non-escaping values.
    pub(super) escape_analysis: HashMap<DefId, EscapeResults>,
    /// Current function's DefId for escape analysis lookup.
    /// Note: Used for escape-analysis-based optimization when 128-bit pointers are enabled.
    #[allow(dead_code)]
    pub(super) current_fn_def_id: Option<DefId>,
    /// Effect lowering context for managing effect compilation.
    pub(super) effect_lowering: EffectLowering,
    /// Compiled effect definitions (effect DefId -> effect metadata).
    pub(super) effect_defs: HashMap<DefId, EffectInfo>,
    /// Compiled handler definitions (handler DefId -> handler metadata).
    pub(crate) handler_defs: HashMap<DefId, HandlerInfo>,
    /// Handler function pointers for runtime registration.
    /// Maps (handler_id, op_index) -> LLVM function.
    pub(super) handler_ops: HashMap<(DefId, usize), FunctionValue<'ctx>>,
    /// Builtin functions: DefId -> runtime function name.
    /// Used to resolve builtin function calls to LLVM runtime functions.
    pub builtin_fns: HashMap<DefId, String>,
    /// Map from region-allocated LocalId to generation storage (stack alloca).
    /// Used for generation validation on dereference.
    pub local_generations: HashMap<LocalId, PointerValue<'ctx>>,
    /// Generated vtables: (trait_id, type_def_id) -> vtable global variable.
    /// For non-ADT types, type_def_id is DefId::DUMMY.
    pub(super) vtables: HashMap<(DefId, DefId), inkwell::values::GlobalValue<'ctx>>,
    /// Vtable slot mappings: trait_id -> Vec<(method_name, slot_index)>.
    /// Each trait has a fixed layout of method slots in its vtable.
    pub(super) vtable_layouts: HashMap<DefId, Vec<(String, usize)>>,
    /// WASM imports: maps function link name to WASM import module.
    /// Used for setting WASM import attributes during post-processing.
    pub(super) wasm_imports: HashMap<String, String>,
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
            local_generations: HashMap::new(),
            vtables: HashMap::new(),
            vtable_layouts: HashMap::new(),
            wasm_imports: HashMap::new(),
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

    /// Get WASM import mappings.
    ///
    /// Returns a map from function link names to their WASM import module names.
    /// This can be used for post-processing the generated LLVM IR or WASM binary
    /// to set the proper import module attributes.
    ///
    /// # Example WASM import post-processing
    ///
    /// When targeting WASM, the LLVM IR needs to have `wasm-import-module` and
    /// `wasm-import-name` attributes set on imported functions. This map provides
    /// the module names so that a post-processing tool can add these attributes.
    pub fn get_wasm_imports(&self) -> &HashMap<String, String> {
        &self.wasm_imports
    }

    // NOTE: Escape analysis helper functions were removed because the MIR codegen path
    // has its own implementation (see mir_codegen.rs: get_local_tier, should_skip_gen_check).
    // The deprecated HIR path does not use escape analysis.

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
        // First pass: collect struct, enum, and effect definitions
        // Effects must be processed before handlers
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
                // These item kinds are handled elsewhere or don't need declaration-phase processing:
                // - Fn: processed in second pass for function declarations
                // - Handler: processed in second pass after effects are registered
                // - TypeAlias: resolved during type checking
                // - Const/Static: compiled with function bodies
                // - Trait/Impl: resolved during type checking
                // - ExternFn/Bridge: processed in fourth pass for FFI declarations
                hir::ItemKind::Fn(_)
                | hir::ItemKind::Handler { .. }
                | hir::ItemKind::TypeAlias { .. }
                | hir::ItemKind::Const { .. }
                | hir::ItemKind::Static { .. }
                | hir::ItemKind::Trait { .. }
                | hir::ItemKind::Impl { .. }
                | hir::ItemKind::ExternFn(_)
                | hir::ItemKind::Bridge(_) => {}
            }
        }

        // Second pass: collect handler definitions (effects must be registered first)
        // Also register handlers in struct_defs so they can be compiled as ADTs
        for (def_id, item) in &hir_crate.items {
            if let hir::ItemKind::Handler { state, .. } = &item.kind {
                // Register handler as an ADT in struct_defs (state fields are the struct fields)
                let field_types: Vec<Type> = state.iter().map(|s| s.ty.clone()).collect();
                self.struct_defs.insert(*def_id, field_types);

                match self.effect_lowering.lower_handler_decl(item, Some(&hir_crate.bodies)) {
                    Ok(handler_info) => {
                        self.handler_defs.insert(*def_id, handler_info);
                    }
                    Err(err) => {
                        return Err(vec![Diagnostic::error(
                            format!("Effect lowering error: {}", err.message),
                            item.span,
                        )]);
                    }
                }
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

        // Fourth pass: declare FFI external functions from bridge blocks
        self.declare_ffi_functions(hir_crate)?;

        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(std::mem::take(&mut self.errors))
        }
    }

    /// Declare FFI external functions from bridge blocks.
    fn declare_ffi_functions(&mut self, hir_crate: &hir::Crate) -> Result<(), Vec<Diagnostic>> {
        for item in hir_crate.items.values() {
            match &item.kind {
                hir::ItemKind::ExternFn(extern_fn) => {
                    self.declare_extern_function(&item.name, extern_fn, None)?;
                }
                hir::ItemKind::Bridge(bridge_def) => {
                    // Get the wasm import module from link specs if present
                    let wasm_import_module = bridge_def.link_specs.iter()
                        .find_map(|ls| ls.wasm_import_module.clone());

                    // Declare all external functions in the bridge
                    for extern_fn in &bridge_def.extern_fns {
                        let extern_fn_def = hir::ExternFnDef {
                            sig: extern_fn.sig.clone(),
                            abi: bridge_def.abi.clone(),
                            link_name: extern_fn.link_name.clone(),
                            is_variadic: extern_fn.is_variadic,
                        };
                        self.declare_extern_function(
                            &extern_fn.name,
                            &extern_fn_def,
                            wasm_import_module.as_deref(),
                        )?;
                        // Also register in functions map for call resolution
                        if let Some(fn_value) = self.module.get_function(
                            extern_fn.link_name.as_ref().unwrap_or(&extern_fn.name)
                        ) {
                            self.functions.insert(extern_fn.def_id, fn_value);
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Declare an external (FFI) function.
    ///
    /// # Arguments
    /// * `name` - The function name in Blood code
    /// * `extern_fn` - The external function definition
    /// * `wasm_import_module` - Optional WASM import module name
    fn declare_extern_function(
        &mut self,
        name: &str,
        extern_fn: &hir::ExternFnDef,
        wasm_import_module: Option<&str>,
    ) -> Result<(), Vec<Diagnostic>> {
        use inkwell::module::Linkage;

        // Build parameter types
        let param_types: Vec<_> = extern_fn.sig.inputs.iter()
            .map(|ty| self.lower_type(ty).into())
            .collect();

        // Build function type
        let fn_type = if extern_fn.sig.output.is_unit() {
            self.context.void_type().fn_type(&param_types, extern_fn.is_variadic)
        } else {
            let ret_type = self.lower_type(&extern_fn.sig.output);
            ret_type.fn_type(&param_types, extern_fn.is_variadic)
        };

        // Use link_name if specified, otherwise use the function name
        let name_owned = name.to_string();
        let link_name = extern_fn.link_name.as_ref().unwrap_or(&name_owned);

        // Add the function with external linkage
        let fn_value = self.module.add_function(link_name, fn_type, Some(Linkage::External));

        // Set calling convention based on ABI
        match extern_fn.abi.as_str() {
            "C" | "c" => {
                // C calling convention (default)
            }
            "stdcall" => {
                // Windows stdcall
                fn_value.set_call_conventions(64); // X86StdcallCallConv
            }
            "fastcall" => {
                // Windows fastcall
                fn_value.set_call_conventions(65); // X86FastcallCallConv
            }
            "wasm" | "WASM" | "WebAssembly" => {
                // WASM uses C-compatible calling convention
                // The import module is handled via LLVM attributes/metadata
                //
                // For WASM targets, we need to set the import module and name.
                // LLVM uses these attributes:
                //   - "wasm-import-module" = import module name (e.g., "env", "wasi_snapshot_preview1")
                //   - "wasm-import-name" = import name (the function name in the WASM module)
                //
                // Note: Inkwell currently doesn't have a direct API for setting string
                // attributes on functions. For full WASM support, this would require:
                // 1. Using inkwell's add_attribute with a custom StringAttribute, or
                // 2. Post-processing the LLVM IR/bitcode to add attributes, or
                // 3. Using wasm-tools to modify the final WASM binary
                //
                // For now, we set up the function correctly and store the import module
                // for future use when we add full attribute support.
                if let Some(module) = wasm_import_module {
                    // Store for later attribute setting when inkwell support is available
                    // or for post-processing
                    self.wasm_imports.insert(link_name.to_string(), module.to_string());
                }
            }
            _ => {
                // Default to C calling convention
            }
        }

        // Note: fn_value is registered in functions map by the caller if needed
        let _ = fn_value;
        Ok(())
    }

    /// Compile an entire HIR crate.
    ///
    /// # Deprecation Warning
    ///
    /// This function uses the HIR codegen path which does NOT emit generation checks.
    /// For memory safety, use the MIR codegen path via `compile_mir_body()` instead.
    ///
    /// The MIR path provides:
    /// - Escape analysis integration
    /// - Generation validation on dereference
    /// - Proper tier-based allocation
    ///
    /// This function is retained only for legacy tests. Production code uses
    /// `compile_definition_to_object()` which calls `compile_mir_body()`.
    #[deprecated(since = "0.3.0", note = "Use compile_mir_body() for generation check emission")]
    pub fn compile_crate(&mut self, hir_crate: &hir::Crate) -> Result<(), Vec<Diagnostic>> {
        // Copy builtin function mappings for resolving runtime calls
        self.builtin_fns = hir_crate.builtin_fns.clone();

        // First pass: collect struct, enum, and effect definitions
        // Effects must be processed before handlers
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
                // These item kinds are handled elsewhere or don't need declaration-phase processing:
                // - Fn: processed in second pass for function declarations
                // - Handler: processed in second pass after effects are registered
                // - TypeAlias: resolved during type checking
                // - Const/Static: compiled with function bodies
                // - Trait/Impl: resolved during type checking
                // - ExternFn/Bridge: processed in FFI pass
                hir::ItemKind::Fn(_)
                | hir::ItemKind::Handler { .. }
                | hir::ItemKind::TypeAlias { .. }
                | hir::ItemKind::Const { .. }
                | hir::ItemKind::Static { .. }
                | hir::ItemKind::Trait { .. }
                | hir::ItemKind::Impl { .. }
                | hir::ItemKind::ExternFn(_)
                | hir::ItemKind::Bridge(_) => {}
            }
        }

        // Second pass: collect handler definitions (effects must be registered first)
        // Also register handlers in struct_defs so they can be compiled as ADTs
        for (def_id, item) in &hir_crate.items {
            if let hir::ItemKind::Handler { state, .. } = &item.kind {
                // Register handler as an ADT in struct_defs (state fields are the struct fields)
                let field_types: Vec<Type> = state.iter().map(|s| s.ty.clone()).collect();
                self.struct_defs.insert(*def_id, field_types);

                match self.effect_lowering.lower_handler_decl(item, Some(&hir_crate.bodies)) {
                    Ok(handler_info) => {
                        self.handler_defs.insert(*def_id, handler_info);
                    }
                    Err(err) => {
                        return Err(vec![Diagnostic::error(
                            format!("Effect lowering error: {}", err.message),
                            item.span,
                        )]);
                    }
                }
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

        // FFI pass: declare external functions from bridge blocks
        self.declare_ffi_functions(hir_crate)?;

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

    /// Compile a function body.
    pub(super) fn compile_function_body(
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
        self.local_generations.clear();
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
    pub(super) fn get_current_block(&self) -> Result<BasicBlock<'ctx>, Vec<Diagnostic>> {
        self.builder.get_insert_block()
            .ok_or_else(|| vec![Diagnostic::error(
                "Internal error: no active basic block".to_string(),
                Span::dummy(),
            )])
    }

    /// Declare a function (without body).
    pub(super) fn declare_function(
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

    /// Declare a closure function from MIR body information.
    ///
    /// Closures have synthetic DefIds (starting at 0xFFFF_0000) that aren't
    /// in the HIR items list. This method declares them using the MIR body's
    /// parameter and return type information.
    pub fn declare_closure_from_mir(&mut self, def_id: DefId, mir_body: &MirBody) {
        // Build parameter types from MIR body parameters
        let param_types: Vec<_> = mir_body.params()
            .map(|local| self.lower_type(&local.ty).into())
            .collect();

        // Get return type from MIR body
        let ret_type = mir_body.return_type();

        let fn_type = if ret_type.is_unit() {
            self.context.void_type().fn_type(&param_types, false)
        } else {
            let llvm_ret_type = self.lower_type(ret_type);
            llvm_ret_type.fn_type(&param_types, false)
        };

        // Generate a unique name for the closure
        let name = format!("blood_closure_{}", def_id.index());
        let fn_value = self.module.add_function(&name, fn_type, None);
        self.functions.insert(def_id, fn_value);
    }

    /// Declare runtime support functions.
    pub(super) fn declare_runtime_functions(&mut self) {
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

        // blood_alloc_or_abort(size: i64, out_generation: *i32) -> i64
        // Simpler allocation that aborts on failure - no conditional branches needed
        let i32_ptr_type = i32_type.ptr_type(AddressSpace::default());
        let alloc_or_abort_type = i64_type.fn_type(&[
            size_type.into(),
            i32_ptr_type.into(),
        ], false);
        self.module.add_function("blood_alloc_or_abort", alloc_or_abort_type, None);

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

        // blood_increment_generation(address: *void) -> void (increment generation for a slot)
        let increment_gen_type = void_type.fn_type(&[i8_ptr_type.into()], false);
        self.module.add_function("blood_increment_generation", increment_gen_type, None);

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
        let void_ptr_ptr_type = void_ptr_type.ptr_type(AddressSpace::default());
        let ev_register_type = void_type.fn_type(&[
            void_ptr_type.into(),
            i64_type.into(),
            void_ptr_ptr_type.into(),  // ops is **void (pointer to array of function pointers)
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

        // === Effect Context Snapshot Functions ===

        // blood_effect_context_set_snapshot(snapshot: i64) -> void
        // Store snapshot in thread-local effect context during perform
        let ctx_set_snapshot_type = void_type.fn_type(&[i64_type.into()], false);
        self.module.add_function("blood_effect_context_set_snapshot", ctx_set_snapshot_type, None);

        // blood_effect_context_get_snapshot() -> i64 (SnapshotHandle, null if none)
        // Retrieve snapshot from effect context during resume for validation
        let ctx_get_snapshot_type = i64_type.fn_type(&[], false);
        self.module.add_function("blood_effect_context_get_snapshot", ctx_get_snapshot_type, None);

        // blood_effect_context_take_snapshot() -> i64 (SnapshotHandle, null if none)
        // Take ownership of snapshot from effect context
        let ctx_take_snapshot_type = i64_type.fn_type(&[], false);
        self.module.add_function("blood_effect_context_take_snapshot", ctx_take_snapshot_type, None);

        // blood_snapshot_stale_panic(snapshot: i64, stale_index: i64) -> void (noreturn)
        // Called when snapshot validation fails - panics with stale reference error
        let stale_panic_type = void_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        self.module.add_function("blood_snapshot_stale_panic", stale_panic_type, None);

        // === Region Management (for scoped allocation with effect suspension) ===

        // blood_region_create(initial_size: i64, max_size: i64) -> i64 (region_id)
        // Creates a new region with the given initial and maximum sizes
        let region_create_type = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        self.module.add_function("blood_region_create", region_create_type, None);

        // blood_region_destroy(region_id: i64) -> void
        // Destroys a region and frees all its memory
        let region_destroy_type = void_type.fn_type(&[i64_type.into()], false);
        self.module.add_function("blood_region_destroy", region_destroy_type, None);

        // blood_region_alloc(region_id: i64, size: i64, align: i64) -> i64 (address)
        // Allocates memory from a region
        let region_alloc_type = i64_type.fn_type(&[i64_type.into(), i64_type.into(), i64_type.into()], false);
        self.module.add_function("blood_region_alloc", region_alloc_type, None);

        // blood_region_suspend(region_id: i64) -> i32 (new suspend count)
        // Suspends a region (called when effect captures continuation)
        let region_suspend_type = i32_type.fn_type(&[i64_type.into()], false);
        self.module.add_function("blood_region_suspend", region_suspend_type, None);

        // blood_region_resume(region_id: i64) -> i32 (packed: count | (should_dealloc << 16))
        // Resumes a region (called when continuation resumes or is dropped)
        let region_resume_type = i32_type.fn_type(&[i64_type.into()], false);
        self.module.add_function("blood_region_resume", region_resume_type, None);

        // blood_region_exit_scope(region_id: i64) -> i32 (1 = deallocate now, 0 = deferred)
        // Exit a region's lexical scope
        let region_exit_scope_type = i32_type.fn_type(&[i64_type.into()], false);
        self.module.add_function("blood_region_exit_scope", region_exit_scope_type, None);

        // blood_region_is_suspended(region_id: i64) -> i32 (bool)
        let region_is_suspended_type = i32_type.fn_type(&[i64_type.into()], false);
        self.module.add_function("blood_region_is_suspended", region_is_suspended_type, None);

        // blood_region_is_pending_deallocation(region_id: i64) -> i32 (bool)
        self.module.add_function("blood_region_is_pending_deallocation", region_is_suspended_type, None);

        // blood_continuation_add_suspended_region(continuation_id: i64, region_id: i64) -> void
        // Associates a suspended region with a continuation
        let cont_add_region_type = void_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        self.module.add_function("blood_continuation_add_suspended_region", cont_add_region_type, None);

        // blood_continuation_take_suspended_regions(continuation_id: i64, out_regions: *i64, max_count: i64) -> i64 (count)
        // Gets and clears the suspended regions for a continuation (handles deferred deallocation)
        let cont_take_regions_type = i64_type.fn_type(&[i64_type.into(), void_ptr_type.into(), i64_type.into()], false);
        self.module.add_function("blood_continuation_take_suspended_regions", cont_take_regions_type, None);

        // blood_continuation_resume_with_regions(continuation: i64, value: i64) -> i64
        // Resume a continuation with automatic region restoration
        let cont_resume_regions_type = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        self.module.add_function("blood_continuation_resume_with_regions", cont_resume_regions_type, None);

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
}
