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
pub mod types;
pub mod expr;
pub mod runtime;
pub mod mir_codegen;

pub use context::CodegenContext;
pub use mir_codegen::MirCodegen;

use inkwell::context::Context;
use inkwell::targets::{Target, TargetMachine, InitializationConfig, CodeModel, RelocMode, FileType};
use inkwell::OptimizationLevel;

use std::collections::HashMap;
use std::path::Path;

use crate::hir::{self, DefId};
use crate::mir::{EscapeResults, MirBody};
use crate::diagnostics::Diagnostic;

/// Type alias for escape analysis results per function.
pub type EscapeAnalysisMap = HashMap<DefId, EscapeResults>;

/// Get a target machine for the native platform.
fn get_native_target_machine() -> Result<TargetMachine, String> {
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
            OptimizationLevel::Default,
            RelocMode::PIC,  // Required for PIE executables
            CodeModel::Default,
        )
        .ok_or_else(|| "Failed to create target machine".to_string())
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
    output_path: &Path,
) -> Result<(), Vec<Diagnostic>> {
    let context = Context::create();
    let module = context.create_module("blood_program");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    codegen.set_escape_analysis(escape_analysis.clone());

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

    // Verify the module
    if let Err(err) = module.verify() {
        return Err(vec![Diagnostic::error(
            format!("LLVM verification failed: {}", err.to_string()),
            crate::span::Span::dummy(),
        )]);
    }

    // Get target machine
    let target_machine = get_native_target_machine()
        .map_err(|e| vec![Diagnostic::error(e, crate::span::Span::dummy())])?;

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
/// * `output_path` - Path to write the object file
pub fn compile_definition_to_object(
    def_id: DefId,
    hir_crate: &hir::Crate,
    mir_body: Option<&MirBody>,
    escape_results: Option<&crate::mir::EscapeResults>,
    all_mir_bodies: Option<&MirBodiesMap>,
    output_path: &Path,
) -> Result<(), Vec<Diagnostic>> {
    let context = Context::create();
    let module_name = format!("blood_def_{}", def_id.index());
    let module = context.create_module(&module_name);
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);

    // Set up escape analysis if provided
    if let Some(results) = escape_results {
        let mut escape_map = HashMap::new();
        escape_map.insert(def_id, results.clone());
        codegen.set_escape_analysis(escape_map);
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

    // Verify the module
    if let Err(err) = module.verify() {
        return Err(vec![Diagnostic::error(
            format!("LLVM verification failed for def {:?}: {}", def_id, err.to_string()),
            crate::span::Span::dummy(),
        )]);
    }

    // Get target machine
    let target_machine = get_native_target_machine()
        .map_err(|e| vec![Diagnostic::error(e, crate::span::Span::dummy())])?;

    // Write object file
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
) -> Result<(), Vec<Diagnostic>> {
    let context = Context::create();
    let module = context.create_module("blood_handler_registration");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);

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

    // Get target machine
    let target_machine = get_native_target_machine()
        .map_err(|e| vec![Diagnostic::error(e, crate::span::Span::dummy())])?;

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
    output_dir: &Path,
) -> Result<Vec<(DefId, std::path::PathBuf)>, Vec<Diagnostic>> {
    let mut results = Vec::new();
    let mut errors = Vec::new();

    for &def_id in def_ids {
        let mir_body = mir_bodies.get(&def_id);
        let escape_results = escape_analysis.get(&def_id);
        let output_path = output_dir.join(format!("def_{}.o", def_id.index()));

        match compile_definition_to_object(def_id, hir_crate, mir_body, escape_results, Some(mir_bodies), &output_path) {
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

/// Compile MIR bodies to LLVM IR text.
pub fn compile_mir_to_ir(
    hir_crate: &hir::Crate,
    mir_bodies: &MirBodiesMap,
    escape_analysis: &EscapeAnalysisMap,
) -> Result<String, Vec<Diagnostic>> {
    let context = Context::create();
    let module = context.create_module("blood_program");
    let builder = context.create_builder();

    let mut codegen = CodegenContext::new(&context, &module, &builder);
    codegen.set_escape_analysis(escape_analysis.clone());

    // First pass: declare types and functions from HIR
    codegen.compile_crate_declarations(hir_crate)?;

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

    Ok(module.print_to_string().to_string())
}
