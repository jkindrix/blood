//! Code generation for Blood.
//!
//! This module generates LLVM IR from the HIR representation.
//! The code generator uses inkwell as a safe wrapper around LLVM.
//!
//! # Architecture
//!
//! ```text
//! HIR -> CodeGenerator -> LLVM IR -> Object Code
//! ```
//!
//! The code generator handles:
//! - Type lowering (HIR types to LLVM types)
//! - Function compilation
//! - Expression evaluation
//! - Control flow
//! - Memory management
//!
//! # Phase 1 Scope
//!
//! Phase 1 focuses on:
//! - Integer types (i32)
//! - Basic arithmetic
//! - Function calls
//! - If/else
//! - While loops
//! - Print support via runtime

pub mod context;
pub mod debug_info;
pub mod types;
pub mod expr;
pub mod runtime;
pub mod mir_codegen;

pub use context::CodegenContext;
pub use debug_info::{DebugInfoGenerator, DebugInfoConfig};
pub use mir_codegen::MirCodegen;

// ============================================================================
// Codegen Version Tracking
// ============================================================================

/// ABI version for code generation.
///
/// Bump this when making intentional breaking changes to:
/// - Calling conventions
/// - Type layouts / struct packing
/// - Runtime interface (function signatures)
/// - Closure representation
/// - Effect handler protocol
///
/// This allows cache invalidation even if the source hash doesn't change
/// (e.g., when the same code produces different output due to bug fixes).
pub const CODEGEN_ABI_VERSION: u32 = 1;

/// Hash of codegen source files, computed at build time.
///
/// This automatically invalidates the cache when codegen logic changes,
/// without requiring manual version bumps.
pub const CODEGEN_HASH: &str = env!("CODEGEN_HASH");

use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::passes::PassManager;
use inkwell::targets::{Target, TargetMachine, InitializationConfig, CodeModel, RelocMode, FileType};
use inkwell::OptimizationLevel;

use std::collections::HashMap;
use std::path::Path;

use crate::hir::{self, DefId};
use crate::mir::{EscapeResults, MirBody, InlineHandlerBodies, ClosureAnalysisResults};
use crate::diagnostics::Diagnostic;

/// Type alias for escape analysis results per function.
pub type EscapeAnalysisMap = HashMap<DefId, EscapeResults>;

/// Optimization level for code generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BloodOptLevel {
    /// No optimizations (for debugging)
    None,
    /// Basic optimizations (fast compile)
    Less,
    /// Default optimizations (good balance)
    #[default]
    Default,
    /// Aggressive optimizations (like -O3)
    Aggressive,
}

impl BloodOptLevel {
    /// Convert to LLVM optimization level for target machine.
    fn to_llvm_opt_level(self) -> OptimizationLevel {
        match self {
            BloodOptLevel::None => OptimizationLevel::None,
            BloodOptLevel::Less => OptimizationLevel::Less,
            BloodOptLevel::Default => OptimizationLevel::Default,
            BloodOptLevel::Aggressive => OptimizationLevel::Aggressive,
        }
    }
}

/// Run LLVM optimization passes excluding those that cause miscompilation.
///
/// This is a workaround for LLVM bugs that cause incorrect code generation
/// when certain optimization passes (particularly GVN and instruction combining)
/// are applied to code patterns involving Vec element access with nested
/// struct field projections.
///
/// The excluded passes that cause issues:
/// - GVN (Global Value Numbering) - incorrectly merges GEP operations
/// - Instruction combining - incorrectly combines GEP operations,
///   causing wrong field indices in nested struct access through Vec elements
/// - Memcpy optimize - converts struct load/store to memcpy with wrong
///   size when struct has nested structs (copies inner struct size, not total)
fn optimize_module(module: &Module, opt_level: BloodOptLevel) {
    if opt_level == BloodOptLevel::None {
        return;
    }

    let mpm: PassManager<Module> = PassManager::create(());

    // Safe optimization passes that avoid LLVM miscompilation bugs.
    //
    // EXCLUDED (causes miscompilation with Vec element access):
    // - instruction_combining_pass - incorrectly combines GEP operations,
    //   causing wrong field indices in nested struct access through Vec elements
    // - GVN - similar issues with value numbering
    // - memcpy_optimize_pass - converts struct load/store to memcpy with wrong
    //   size when struct has nested structs (copies inner struct size, not total)

    // === Phase 1: Canonicalization ===
    // mem2reg is essential for SSA form
    mpm.add_promote_memory_to_register_pass();
    mpm.add_reassociate_pass();

    // === Phase 2: Analysis Setup ===
    mpm.add_basic_alias_analysis_pass();
    mpm.add_type_based_alias_analysis_pass();

    // === Phase 3: Scalar Optimizations ===
    // (GVN excluded — causes miscompilation)
    mpm.add_sccp_pass();
    mpm.add_aggressive_dce_pass();
    mpm.add_dead_store_elimination_pass();
    mpm.add_cfg_simplification_pass();

    // === Phase 4: Loop Optimizations ===
    mpm.add_licm_pass();
    mpm.add_ind_var_simplify_pass();

    if opt_level == BloodOptLevel::Aggressive {
        mpm.add_loop_rotate_pass();
        mpm.add_loop_deletion_pass();
    }

    // === Phase 5: Interprocedural Optimizations ===
    mpm.add_function_inlining_pass();
    mpm.add_always_inliner_pass();
    mpm.add_global_dce_pass();
    mpm.add_global_optimizer_pass();
    mpm.add_constant_merge_pass();

    if opt_level == BloodOptLevel::Aggressive {
        mpm.add_merge_functions_pass();
        mpm.add_tail_call_elimination_pass();
    }

    // === Phase 6: Final Cleanup ===
    // (instruction combining excluded — causes miscompilation)
    mpm.add_cfg_simplification_pass();
    mpm.add_jump_threading_pass();
    // (memcpy optimize excluded — causes miscompilation)
    mpm.add_strip_dead_prototypes_pass();

    mpm.run_on(module);
}

/// Run optimization passes one at a time, returning the IR after each pass.
/// This is used to bisect which optimization pass causes miscompilation
/// (e.g., BUG-008 if-expression branch elimination).
///
/// Each pass is run incrementally on the module (pass 1, then pass 2 on top, etc.)
/// and the IR is captured after each step. Compare consecutive snapshots to see
/// which pass introduces the problem.
///
/// Returns a Vec of (pass_name, ir_after_pass) pairs.
pub fn optimize_module_bisect(module: &Module) -> Vec<(String, String)> {
    let passes: Vec<(&str, Box<dyn Fn(&PassManager<Module>)>)> = vec![
        ("promote_memory_to_register", Box::new(|pm| pm.add_promote_memory_to_register_pass())),
        ("reassociate", Box::new(|pm| pm.add_reassociate_pass())),
        ("basic_alias_analysis", Box::new(|pm| pm.add_basic_alias_analysis_pass())),
        ("type_based_alias_analysis", Box::new(|pm| pm.add_type_based_alias_analysis_pass())),
        ("sccp", Box::new(|pm| pm.add_sccp_pass())),
        ("aggressive_dce", Box::new(|pm| pm.add_aggressive_dce_pass())),
        ("dead_store_elimination", Box::new(|pm| pm.add_dead_store_elimination_pass())),
        ("cfg_simplification_1", Box::new(|pm| pm.add_cfg_simplification_pass())),
        ("licm", Box::new(|pm| pm.add_licm_pass())),
        ("ind_var_simplify", Box::new(|pm| pm.add_ind_var_simplify_pass())),
        ("function_inlining", Box::new(|pm| pm.add_function_inlining_pass())),
        ("always_inliner", Box::new(|pm| pm.add_always_inliner_pass())),
        ("global_dce", Box::new(|pm| pm.add_global_dce_pass())),
        ("global_optimizer", Box::new(|pm| pm.add_global_optimizer_pass())),
        ("constant_merge", Box::new(|pm| pm.add_constant_merge_pass())),
        ("cfg_simplification_2", Box::new(|pm| pm.add_cfg_simplification_pass())),
        ("jump_threading", Box::new(|pm| pm.add_jump_threading_pass())),
        ("strip_dead_prototypes", Box::new(|pm| pm.add_strip_dead_prototypes_pass())),
    ];

    let mut results = Vec::new();

    for (name, add_pass) in &passes {
        let mpm: PassManager<Module> = PassManager::create(());
        add_pass(&mpm);
        mpm.run_on(module);
        let ir = module.print_to_string().to_string();
        results.push((name.to_string(), ir));
    }

    results
}

/// Get a target machine for the native platform with specified optimization level.
fn get_native_target_machine_with_opt(opt_level: BloodOptLevel) -> Result<TargetMachine, String> {
    Target::initialize_native(&InitializationConfig::default())
        .map_err(|e| format!("Failed to initialize native target: {}", e))?;

    let triple = TargetMachine::get_default_triple();
    let target = Target::from_triple(&triple)
        .map_err(|e| format!("Failed to get target: {}", e.to_string()))?;

    let cpu = TargetMachine::get_host_cpu_name();
    let features = TargetMachine::get_host_cpu_features();

    target
        .create_target_machine(
            &triple,
            cpu.to_str().unwrap_or("generic"),
            features.to_str().unwrap_or(""),
            opt_level.to_llvm_opt_level(),
            RelocMode::PIC,  // Required for PIE executables
            CodeModel::Default,
        )
        .ok_or_else(|| "Failed to create target machine".to_string())
}

/// Get a target machine for the native platform with default optimization.
#[allow(dead_code)] // Infrastructure for default optimization level
fn get_native_target_machine() -> Result<TargetMachine, String> {
    get_native_target_machine_with_opt(BloodOptLevel::Default)
}

/// Configure a module with the correct target data layout and triple.
///
/// Sets the module's data layout to match the TargetMachine's default.
/// This ensures LLVM's struct layout (GEP offsets, sizeof) matches our
/// alignment calculations during IR construction.
fn configure_module_target(module: &Module, target_machine: &TargetMachine) {
    // Set module data layout and triple from the TargetMachine.
    //
    // NOTE: We use the TargetMachine's default data layout WITHOUT modification.
    // LLVM 14's default x86_64 data layout uses 8-byte ABI alignment for i128.
    // We previously attempted to inject i128:128 (16-byte alignment) into the
    // data layout string, but this is ineffective because LLVM 14's C API
    // (LLVMTargetMachineEmitToFile) resets the module's data layout to the
    // TargetMachine's default during code emission. Instead, all Blood codegen
    // alignment calculations match LLVM 14's actual defaults.
    let target_data = target_machine.get_target_data();
    let data_layout = target_data.get_data_layout();
    module.set_data_layout(&data_layout);
    module.set_triple(&target_machine.get_triple());
}

/// Type alias for MIR bodies per function.
pub type MirBodiesMap = HashMap<DefId, MirBody>;

/// Compile MIR bodies to an object file.
///
/// This is the primary MIR-based compilation path. When MIR lowering succeeds,
/// this function should be used instead of the HIR-based path.
///
/// # Benefits over HIR codegen
///
/// - Escape analysis results can be used to determine allocation strategy
/// - Generation checks can be skipped for non-escaping values
/// - Tier-based memory allocation (stack vs region vs persistent)
pub fn compile_mir_to_object(
    hir_crate: &hir::Crate,
    mir_bodies: &MirBodiesMap,
    escape_analysis: &EscapeAnalysisMap,
    inline_handler_bodies: &InlineHandlerBodies,
    output_path: &Path,
    builtin_def_ids: (Option<DefId>, Option<DefId>, Option<DefId>, Option<DefId>),
    closure_analysis: Option<&ClosureAnalysisResults>,
) -> Result<(), Vec<Diagnostic>> {
    let context = Context::create();
    let module = context.create_module("blood_program");
    let builder = context.create_builder();

    // Configure module with target data layout BEFORE compilation.
    // This is critical for correct type layout (e.g., i128 alignment).
    let target_machine = get_native_target_machine_with_opt(BloodOptLevel::Default)
        .map_err(|e| vec![Diagnostic::error(e, crate::span::Span::dummy())])?;
    configure_module_target(&module, &target_machine);

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    codegen.set_escape_analysis(escape_analysis.clone());
    codegen.set_inline_handler_bodies(inline_handler_bodies.clone());
    if let Some(ca) = closure_analysis {
        codegen.set_closure_analysis(ca.clone());
    }
    codegen.set_builtin_def_ids(builtin_def_ids.0, builtin_def_ids.1, builtin_def_ids.2, builtin_def_ids.3);

    // First pass: declare types and functions from HIR
    // This sets up struct_defs, enum_defs, and function declarations
    codegen.compile_crate_declarations(hir_crate)?;

    // Store MIR bodies for generic functions (for on-demand monomorphization)
    codegen.set_generic_mir_bodies(mir_bodies);

    // Compile const and static items (global variables)
    codegen.compile_const_items(hir_crate)?;
    codegen.compile_static_items(hir_crate)?;

    // Second pass: declare closure functions from MIR
    // Closures have synthetic DefIds (>= 0xFFFF_0000) that aren't in HIR items
    for (&def_id, mir_body) in mir_bodies {
        if def_id.index() >= 0xFFFF_0000 {
            codegen.declare_closure_from_mir(def_id, mir_body);
        }
    }

    // Third pass: compile MIR function bodies
    for (&def_id, mir_body) in mir_bodies {
        let escape_results = escape_analysis.get(&def_id);
        codegen.compile_mir_body(def_id, mir_body, escape_results)?;
    }

    // Fourth pass: compile handler operation bodies (from HIR)
    // Handler operations are not in MIR, so we compile them from HIR
    codegen.compile_handler_operations(hir_crate)?;

    // Fifth pass: register handlers with runtime
    codegen.register_handlers_with_runtime()?;

    // Drain any errors collected during type lowering (lower_type takes &self,
    // so errors are stored in a RefCell and drained here).
    {
        let mut type_diags = codegen.type_lowering_errors.borrow_mut();
        let type_errors: Vec<Diagnostic> = type_diags.drain(..).collect();
        if !type_errors.is_empty() {
            return Err(type_errors);
        }
    }

    // Verify the module before optimization
    if let Err(err) = module.verify() {
        return Err(vec![Diagnostic::error(
            format!("LLVM verification failed: {}", err.to_string()),
            crate::span::Span::dummy(),
        )]);
    }

    // Run LLVM optimization passes (using Aggressive for benchmarks)
    // TEMPORARY: Use None to debug struct field offset issue
    let use_opt = std::env::var("BLOOD_DEBUG_NO_OPT").is_err();
    if use_opt {
        optimize_module(&module, BloodOptLevel::Aggressive);
    }

    // Write object file
    target_machine
        .write_to_file(&module, FileType::Object, output_path)
        .map_err(|e| vec![Diagnostic::error(
            format!("Failed to write object file: {}", e.to_string()),
            crate::span::Span::dummy(),
        )])?;

    Ok(())
}

/// Compile MIR bodies to an object file with specified optimization level.
pub fn compile_mir_to_object_with_opt(
    hir_crate: &hir::Crate,
    mir_bodies: &MirBodiesMap,
    escape_analysis: &EscapeAnalysisMap,
    inline_handler_bodies: &InlineHandlerBodies,
    output_path: &Path,
    opt_level: BloodOptLevel,
    builtin_def_ids: (Option<DefId>, Option<DefId>, Option<DefId>, Option<DefId>),
    closure_analysis: Option<&ClosureAnalysisResults>,
) -> Result<(), Vec<Diagnostic>> {
    let context = Context::create();
    let module = context.create_module("blood_program");
    let builder = context.create_builder();

    // Configure module with target data layout BEFORE compilation.
    let target_machine = get_native_target_machine_with_opt(opt_level)
        .map_err(|e| vec![Diagnostic::error(e, crate::span::Span::dummy())])?;
    configure_module_target(&module, &target_machine);

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    codegen.set_escape_analysis(escape_analysis.clone());
    codegen.set_inline_handler_bodies(inline_handler_bodies.clone());
    if let Some(ca) = closure_analysis {
        codegen.set_closure_analysis(ca.clone());
    }
    codegen.set_builtin_def_ids(builtin_def_ids.0, builtin_def_ids.1, builtin_def_ids.2, builtin_def_ids.3);

    // First pass: declare types and functions from HIR
    codegen.compile_crate_declarations(hir_crate)?;

    // Store MIR bodies for generic functions
    codegen.set_generic_mir_bodies(mir_bodies);

    // Compile const and static items
    codegen.compile_const_items(hir_crate)?;
    codegen.compile_static_items(hir_crate)?;

    // Second pass: declare closure functions from MIR
    for (&def_id, mir_body) in mir_bodies {
        if def_id.index() >= 0xFFFF_0000 {
            codegen.declare_closure_from_mir(def_id, mir_body);
        }
    }

    // Third pass: compile MIR function bodies
    for (&def_id, mir_body) in mir_bodies {
        let escape_results = escape_analysis.get(&def_id);
        codegen.compile_mir_body(def_id, mir_body, escape_results)?;
    }

    // Fourth pass: compile handler operation bodies
    codegen.compile_handler_operations(hir_crate)?;

    // Fifth pass: register handlers with runtime
    codegen.register_handlers_with_runtime()?;

    // Drain any errors collected during type lowering (lower_type takes &self,
    // so errors are stored in a RefCell and drained here).
    {
        let mut type_diags = codegen.type_lowering_errors.borrow_mut();
        let type_errors: Vec<Diagnostic> = type_diags.drain(..).collect();
        if !type_errors.is_empty() {
            return Err(type_errors);
        }
    }

    // Verify the module before optimization
    if let Err(err) = module.verify() {
        return Err(vec![Diagnostic::error(
            format!("LLVM verification failed: {}", err.to_string()),
            crate::span::Span::dummy(),
        )]);
    }

    // Run LLVM optimization passes
    optimize_module(&module, opt_level);

    // Write object file
    target_machine
        .write_to_file(&module, FileType::Object, output_path)
        .map_err(|e| vec![Diagnostic::error(
            format!("Failed to write object file: {}", e.to_string()),
            crate::span::Span::dummy(),
        )])?;

    Ok(())
}

/// Compile a single definition to an object file.
///
/// This enables per-definition incremental compilation:
/// 1. Each definition gets its own LLVM module
/// 2. Dependencies are declared as external symbols
/// 3. The linker resolves symbols when combining object files
///
/// # Arguments
/// * `def_id` - The definition to compile
/// * `hir_crate` - The full crate (for type declarations)
/// * `mir_body` - The MIR body for this definition (if it's a function)
/// * `escape_results` - Escape analysis results for this function
/// * `all_mir_bodies` - All MIR bodies in the crate (for monomorphization of generic functions)
/// * `inline_handler_bodies` - Inline handler bodies for try/with blocks
/// * `output_path` - Path to write the object file
// Compiler-internal: decomposing would reduce clarity
#[allow(clippy::too_many_arguments)]
pub fn compile_definition_to_object(
    def_id: DefId,
    hir_crate: &hir::Crate,
    mir_body: Option<&MirBody>,
    escape_results: Option<&crate::mir::EscapeResults>,
    all_mir_bodies: Option<&MirBodiesMap>,
    inline_handler_bodies: Option<&InlineHandlerBodies>,
    output_path: &Path,
    builtin_def_ids: (Option<DefId>, Option<DefId>, Option<DefId>, Option<DefId>),
    closure_analysis: Option<&ClosureAnalysisResults>,
) -> Result<(), Vec<Diagnostic>> {
    let context = Context::create();
    let module_name = format!("blood_def_{}", def_id.index());
    let module = context.create_module(&module_name);
    let builder = context.create_builder();

    // Configure module with target data layout BEFORE compilation.
    let target_machine = get_native_target_machine_with_opt(BloodOptLevel::Default)
        .map_err(|e| vec![Diagnostic::error(e, crate::span::Span::dummy())])?;
    configure_module_target(&module, &target_machine);

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    codegen.set_builtin_def_ids(builtin_def_ids.0, builtin_def_ids.1, builtin_def_ids.2, builtin_def_ids.3);

    // Set up escape analysis if provided
    if let Some(results) = escape_results {
        let mut escape_map = HashMap::new();
        escape_map.insert(def_id, results.clone());
        codegen.set_escape_analysis(escape_map);
    }

    // Set up inline handler bodies if provided
    if let Some(handlers) = inline_handler_bodies {
        codegen.set_inline_handler_bodies(handlers.clone());
    }

    // Set up closure analysis if provided
    if let Some(ca) = closure_analysis {
        codegen.set_closure_analysis(ca.clone());
    }

    // Declare all types and external functions from the crate
    codegen.compile_crate_declarations(hir_crate)?;

    // Store MIR bodies for generic functions (for on-demand monomorphization)
    if let Some(mir_bodies) = all_mir_bodies {
        codegen.set_generic_mir_bodies(mir_bodies);

        // Declare closure functions from MIR bodies
        // Closures have synthetic DefIds (>= 0xFFFF_0000) that aren't in HIR items
        for (&closure_def_id, mir_body) in mir_bodies.iter() {
            if closure_def_id.index() >= 0xFFFF_0000 {
                codegen.declare_closure_from_mir(closure_def_id, mir_body);
            }
        }
    }

    // Compile const and static items (if this is a const/static definition)
    codegen.compile_const_items(hir_crate)?;
    codegen.compile_static_items(hir_crate)?;

    // Compile the specific definition
    if let Some(mir) = mir_body {
        codegen.compile_mir_body(def_id, mir, escape_results)?;
    } else if let Some(item) = hir_crate.items.get(&def_id) {
        // Non-function items (structs, enums) - just declarations, no body to compile
        match &item.kind {
            hir::ItemKind::Handler { .. } => {
                // Compile handler operations
                codegen.compile_handler_item(def_id, item, hir_crate)?;
            }
            hir::ItemKind::Const { .. } | hir::ItemKind::Static { .. } => {
                // Already compiled above
            }
            _ => {
                // Type declarations are already handled in compile_crate_declarations
            }
        }
    }

    // Drain any errors collected during type lowering.
    {
        let mut type_diags = codegen.type_lowering_errors.borrow_mut();
        let type_errors: Vec<Diagnostic> = type_diags.drain(..).collect();
        if !type_errors.is_empty() {
            return Err(type_errors);
        }
    }

    // Verify the module
    if let Err(err) = module.verify() {
        return Err(vec![Diagnostic::error(
            format!("LLVM verification failed for def {:?}: {}", def_id, err.to_string()),
            crate::span::Span::dummy(),
        )]);
    }

    // Dump IR before optimization if requested
    if std::env::var("BLOOD_DUMP_UNOPT_IR").is_ok() {
        let ir_path = output_path.with_extension("unopt.ll");
        if let Err(e) = module.print_to_file(&ir_path) {
            eprintln!("Warning: Failed to write unoptimized IR: {}", e);
        } else {
            eprintln!("Wrote unoptimized IR to: {:?}", ir_path);
        }
    }

    // Run LLVM optimization passes (can be disabled for debugging)
    // NOTE: Using custom optimization that skips problematic passes (GVN)
    // due to LLVM miscompilation bug with nested struct field access through Vec.
    let use_opt = std::env::var("BLOOD_DEBUG_NO_OPT").is_err();
    if use_opt {
        optimize_module(&module, BloodOptLevel::Default);
    }

    // Dump IR after optimization if requested
    if std::env::var("BLOOD_DUMP_OPT_IR").is_ok() {
        let ir_path = output_path.with_extension("opt.ll");
        if let Err(e) = module.print_to_file(&ir_path) {
            eprintln!("Warning: Failed to write optimized IR: {}", e);
        } else {
            eprintln!("Wrote optimized IR to: {:?}", ir_path);
        }
    }

    // Write object file (target_machine created earlier for data layout)
    target_machine
        .write_to_file(&module, FileType::Object, output_path)
        .map_err(|e| vec![Diagnostic::error(
            format!("Failed to write object file: {}", e.to_string()),
            crate::span::Span::dummy(),
        )])?;

    Ok(())
}

/// Compile handler registration code to a separate object file.
///
/// This function generates a global constructor that registers all handlers with
/// the runtime's effect registry. It must be called after all handler definitions
/// have been compiled, and the resulting object file must be linked with the other
/// definition object files.
///
/// # Arguments
/// * `hir_crate` - The full crate (needed to find handler definitions)
/// * `output_path` - Path to write the handler registration object file
pub fn compile_handler_registration_to_object(
    hir_crate: &hir::Crate,
    output_path: &Path,
    builtin_def_ids: (Option<DefId>, Option<DefId>, Option<DefId>, Option<DefId>),
) -> Result<(), Vec<Diagnostic>> {
    let context = Context::create();
    let module = context.create_module("blood_handler_registration");
    let builder = context.create_builder();

    // Configure module with target data layout BEFORE compilation.
    let target_machine = get_native_target_machine_with_opt(BloodOptLevel::Default)
        .map_err(|e| vec![Diagnostic::error(e, crate::span::Span::dummy())])?;
    configure_module_target(&module, &target_machine);

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    codegen.set_builtin_def_ids(builtin_def_ids.0, builtin_def_ids.1, builtin_def_ids.2, builtin_def_ids.3);

    // Declare all types and functions needed for handler registration
    // This already calls declare_handler_operations internally
    codegen.compile_crate_declarations(hir_crate)?;

    // Generate the handler registration global constructor
    // Note: declare_handler_operations is called by compile_crate_declarations,
    // so handler_ops is already populated with function declarations
    codegen.register_handlers_with_runtime()?;

    // Verify the module
    if let Err(err) = module.verify() {
        return Err(vec![Diagnostic::error(
            format!("LLVM verification failed for handler registration: {}", err.to_string()),
            crate::span::Span::dummy(),
        )]);
    }

    // Run LLVM optimization passes
    optimize_module(&module, BloodOptLevel::Aggressive);

    // Write object file
    target_machine
        .write_to_file(&module, FileType::Object, output_path)
        .map_err(|e| vec![Diagnostic::error(
            format!("Failed to write handler registration object: {}", e.to_string()),
            crate::span::Span::dummy(),
        )])?;

    Ok(())
}

/// Compile multiple definitions to separate object files.
///
/// Returns a list of (DefId, object_path) pairs for successfully compiled definitions.
pub fn compile_definitions_to_objects(
    def_ids: &[DefId],
    hir_crate: &hir::Crate,
    mir_bodies: &MirBodiesMap,
    escape_analysis: &EscapeAnalysisMap,
    inline_handler_bodies: Option<&InlineHandlerBodies>,
    output_dir: &Path,
    builtin_def_ids: (Option<DefId>, Option<DefId>, Option<DefId>, Option<DefId>),
    closure_analysis: Option<&ClosureAnalysisResults>,
) -> Result<Vec<(DefId, std::path::PathBuf)>, Vec<Diagnostic>> {
    let mut results = Vec::new();
    let mut errors = Vec::new();

    for &def_id in def_ids {
        let mir_body = mir_bodies.get(&def_id);
        let escape_results = escape_analysis.get(&def_id);
        let output_path = output_dir.join(format!("def_{}.o", def_id.index()));

        match compile_definition_to_object(def_id, hir_crate, mir_body, escape_results, Some(mir_bodies), inline_handler_bodies, &output_path, builtin_def_ids, closure_analysis) {
            Ok(()) => {
                results.push((def_id, output_path));
            }
            Err(errs) => {
                errors.extend(errs);
            }
        }
    }

    if errors.is_empty() {
        Ok(results)
    } else {
        Err(errors)
    }
}

/// Link multiple object files into a single executable.
///
/// Uses the system linker (cc) to combine object files with the Blood runtime.
pub fn link_objects(
    object_files: &[std::path::PathBuf],
    runtime_lib: &Path,
    output_path: &Path,
) -> Result<(), String> {
    use std::process::Command;

    let mut cmd = Command::new("cc");

    // Add all object files
    for obj in object_files {
        cmd.arg(obj);
    }

    // Link with runtime
    cmd.arg(runtime_lib);

    // Link with system libraries
    cmd.arg("-lpthread");
    cmd.arg("-ldl");
    cmd.arg("-lm");

    // Output path
    cmd.arg("-o");
    cmd.arg(output_path);

    // PIE for ASLR
    cmd.arg("-pie");

    let output = cmd.output()
        .map_err(|e| format!("Failed to run linker: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Linking failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

/// Compile MIR bodies to LLVM IR text (optimized by default).
pub fn compile_mir_to_ir(
    hir_crate: &hir::Crate,
    mir_bodies: &MirBodiesMap,
    escape_analysis: &EscapeAnalysisMap,
    builtin_def_ids: (Option<DefId>, Option<DefId>, Option<DefId>, Option<DefId>),
    inline_handler_bodies: Option<&InlineHandlerBodies>,
    closure_analysis: Option<&ClosureAnalysisResults>,
) -> Result<String, Vec<Diagnostic>> {
    compile_mir_to_ir_with_opt(hir_crate, mir_bodies, escape_analysis, BloodOptLevel::Aggressive, builtin_def_ids, inline_handler_bodies, closure_analysis)
}

/// Compile MIR bodies to LLVM IR text with specified optimization level.
pub fn compile_mir_to_ir_with_opt(
    hir_crate: &hir::Crate,
    mir_bodies: &MirBodiesMap,
    escape_analysis: &EscapeAnalysisMap,
    opt_level: BloodOptLevel,
    builtin_def_ids: (Option<DefId>, Option<DefId>, Option<DefId>, Option<DefId>),
    inline_handler_bodies: Option<&InlineHandlerBodies>,
    closure_analysis: Option<&ClosureAnalysisResults>,
) -> Result<String, Vec<Diagnostic>> {
    let context = Context::create();
    let module = context.create_module("blood_program");
    let builder = context.create_builder();

    // Configure module with target data layout BEFORE compilation.
    if let Ok(tm) = get_native_target_machine_with_opt(BloodOptLevel::Default) {
        configure_module_target(&module, &tm);
    }

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    codegen.set_escape_analysis(escape_analysis.clone());
    if let Some(ihb) = inline_handler_bodies {
        codegen.set_inline_handler_bodies(ihb.clone());
    }
    if let Some(ca) = closure_analysis {
        codegen.set_closure_analysis(ca.clone());
    }
    // Store MIR bodies for generic functions (for on-demand monomorphization)
    codegen.set_generic_mir_bodies(mir_bodies);
    // Set builtin def IDs so Vec, Box, Option, Result get correct type representations
    codegen.set_builtin_def_ids(
        builtin_def_ids.0,
        builtin_def_ids.1,
        builtin_def_ids.2,
        builtin_def_ids.3,
    );

    // First pass: declare types and functions from HIR
    codegen.compile_crate_declarations(hir_crate)?;

    // Compile const and static items (global variables)
    codegen.compile_const_items(hir_crate)?;
    codegen.compile_static_items(hir_crate)?;

    // Second pass: declare closure functions from MIR
    // Closures have synthetic DefIds (>= 0xFFFF_0000) that aren't in HIR items
    for (&def_id, mir_body) in mir_bodies {
        if def_id.index() >= 0xFFFF_0000 {
            codegen.declare_closure_from_mir(def_id, mir_body);
        }
    }

    // Third pass: compile MIR function bodies
    for (&def_id, mir_body) in mir_bodies {
        let escape_results = escape_analysis.get(&def_id);
        codegen.compile_mir_body(def_id, mir_body, escape_results)?;
    }

    // Fourth pass: compile handler operation bodies (from HIR)
    codegen.compile_handler_operations(hir_crate)?;

    // Fifth pass: register handlers with runtime
    codegen.register_handlers_with_runtime()?;

    // Run LLVM optimization passes
    optimize_module(&module, opt_level);

    Ok(module.print_to_string().to_string())
}
