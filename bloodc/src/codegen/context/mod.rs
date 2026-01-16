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
use inkwell::types::{BasicType, BasicTypeEnum};
use inkwell::AddressSpace;

use crate::hir::{self, DefId, LocalId, Type, PrimitiveTy, IntTy, UintTy, FloatTy};
use crate::hir::ty::{TypeKind, TyVarId};
use crate::mir::{EscapeAnalyzer, EscapeResults, MirBody};
use crate::codegen::mir_codegen::MirTypesCodegen;
use crate::diagnostics::Diagnostic;
use crate::span::Span;
use crate::effects::{EffectLowering, EffectInfo, HandlerInfo};

/// Build a mapping from arbitrary TyVarIds to sequential indices (0, 1, 2, ...).
///
/// This collects all unique TyVarIds from a list of types, sorts them by their
/// original ID (to maintain a consistent order), and maps them to sequential indices.
fn build_tyvar_mapping(types: &[Type]) -> HashMap<TyVarId, u32> {
    let mut all_tyvars = Vec::new();
    for ty in types {
        collect_tyvars(ty, &mut all_tyvars);
    }

    // Sort by TyVarId value to get consistent ordering
    all_tyvars.sort_by_key(|tv| tv.0);
    all_tyvars.dedup();

    // Build mapping: first unique TyVarId -> 0, second -> 1, etc.
    all_tyvars.iter()
        .enumerate()
        .map(|(idx, &tyvar)| (tyvar, idx as u32))
        .collect()
}

/// Normalize multiple types together using a shared TyVarId mapping.
///
/// This should be used when normalizing struct/enum fields to ensure all fields
/// share the same TyVarId-to-index mapping.
fn normalize_types_together(types: &[Type]) -> Vec<Type> {
    // Build a shared mapping across all types
    let tyvar_to_idx = build_tyvar_mapping(types);

    // Normalize each type using the shared mapping
    types.iter()
        .map(|ty| normalize_type_recursive(ty, &tyvar_to_idx))
        .collect()
}

/// Collect all TyVarIds from a type (in order of appearance).
fn collect_tyvars(ty: &Type, tyvars: &mut Vec<TyVarId>) {
    match ty.kind() {
        TypeKind::Param(id) => {
            if !tyvars.contains(id) {
                tyvars.push(*id);
            }
        }
        TypeKind::Tuple(fields) => {
            for f in fields {
                collect_tyvars(f, tyvars);
            }
        }
        TypeKind::Array { element, .. } | TypeKind::Slice { element } => {
            collect_tyvars(element, tyvars);
        }
        TypeKind::Ref { inner, .. } | TypeKind::Ptr { inner, .. } => {
            collect_tyvars(inner, tyvars);
        }
        TypeKind::Adt { args, .. } => {
            for arg in args {
                collect_tyvars(arg, tyvars);
            }
        }
        TypeKind::Fn { params, ret } => {
            for p in params {
                collect_tyvars(p, tyvars);
            }
            collect_tyvars(ret, tyvars);
        }
        _ => {}
    }
}

/// Recursively replace TyVarIds with normalized Param indices.
fn normalize_type_recursive(ty: &Type, tyvar_to_idx: &HashMap<TyVarId, u32>) -> Type {
    match ty.kind() {
        TypeKind::Param(id) => {
            if let Some(&idx) = tyvar_to_idx.get(id) {
                // Create a new Param with sequential index
                Type::param(TyVarId(idx))
            } else {
                ty.clone()
            }
        }
        TypeKind::Tuple(fields) => {
            let normalized: Vec<Type> = fields.iter()
                .map(|f| normalize_type_recursive(f, tyvar_to_idx))
                .collect();
            Type::tuple(normalized)
        }
        TypeKind::Array { element, size } => {
            let normalized = normalize_type_recursive(element, tyvar_to_idx);
            Type::array(normalized, *size)
        }
        TypeKind::Slice { element } => {
            let normalized = normalize_type_recursive(element, tyvar_to_idx);
            Type::slice(normalized)
        }
        TypeKind::Ref { inner, mutable } => {
            let normalized = normalize_type_recursive(inner, tyvar_to_idx);
            Type::reference(normalized, *mutable)
        }
        TypeKind::Ptr { inner, mutable } => {
            let normalized = normalize_type_recursive(inner, tyvar_to_idx);
            Type::new(TypeKind::Ptr { inner: normalized, mutable: *mutable })
        }
        TypeKind::Adt { def_id, args } => {
            let normalized_args: Vec<Type> = args.iter()
                .map(|a| normalize_type_recursive(a, tyvar_to_idx))
                .collect();
            Type::adt(*def_id, normalized_args)
        }
        TypeKind::Fn { params, ret } => {
            let normalized_params: Vec<Type> = params.iter()
                .map(|p| normalize_type_recursive(p, tyvar_to_idx))
                .collect();
            let normalized_ret = normalize_type_recursive(ret, tyvar_to_idx);
            Type::function(normalized_params, normalized_ret)
        }
        _ => ty.clone(),
    }
}

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

/// Substitute type parameters in a MIR body with concrete types.
///
/// This creates a monomorphized copy of the MIR body where all type parameters
/// (represented as Param(TyVarId)) are replaced with concrete types.
fn substitute_mir_types(mir: &MirBody, subst: &HashMap<TyVarId, Type>) -> MirBody {
    // Clone the MIR body
    let mut result = mir.clone();

    // Substitute types in locals
    for local in &mut result.locals {
        local.ty = substitute_type(&local.ty, subst);
    }

    // Substitute types in basic blocks
    for block in &mut result.basic_blocks {
        // Substitute in statements
        for stmt in &mut block.statements {
            substitute_statement_types(stmt, subst);
        }

        // Substitute in terminator
        if let Some(term) = &mut block.terminator {
            substitute_terminator_types(term, subst);
        }
    }

    result
}

/// Substitute type parameters in a type.
fn substitute_type(ty: &Type, subst: &HashMap<TyVarId, Type>) -> Type {
    match ty.kind() {
        TypeKind::Param(id) => {
            if let Some(concrete) = subst.get(id) {
                concrete.clone()
            } else {
                ty.clone()
            }
        }
        TypeKind::Tuple(fields) => {
            let subst_fields: Vec<Type> = fields.iter()
                .map(|f| substitute_type(f, subst))
                .collect();
            Type::tuple(subst_fields)
        }
        TypeKind::Array { element, size } => {
            Type::array(substitute_type(element, subst), *size)
        }
        TypeKind::Slice { element } => {
            Type::slice(substitute_type(element, subst))
        }
        TypeKind::Ref { inner, mutable } => {
            Type::reference(substitute_type(inner, subst), *mutable)
        }
        TypeKind::Ptr { inner, mutable } => {
            Type::new(TypeKind::Ptr {
                inner: substitute_type(inner, subst),
                mutable: *mutable,
            })
        }
        TypeKind::Adt { def_id, args } => {
            let subst_args: Vec<Type> = args.iter()
                .map(|a| substitute_type(a, subst))
                .collect();
            Type::adt(*def_id, subst_args)
        }
        TypeKind::Fn { params, ret } => {
            let subst_params: Vec<Type> = params.iter()
                .map(|p| substitute_type(p, subst))
                .collect();
            Type::function(subst_params, substitute_type(ret, subst))
        }
        _ => ty.clone(),
    }
}

/// Infer type arguments by unifying generic signature with concrete types.
///
/// This matches the generic parameter types (which contain TypeKind::Param) with
/// the concrete parameter types to determine what each type variable maps to.
fn infer_type_args(
    generic_params: &[Type],
    concrete_params: &[Type],
    generic_ret: &Type,
    concrete_ret: &Type,
) -> HashMap<TyVarId, Type> {
    let mut subst: HashMap<TyVarId, Type> = HashMap::new();

    // Unify parameters
    for (generic, concrete) in generic_params.iter().zip(concrete_params.iter()) {
        unify_types(generic, concrete, &mut subst);
    }

    // Unify return type
    unify_types(generic_ret, concrete_ret, &mut subst);

    subst
}

/// Recursively unify a generic type with a concrete type, populating the substitution map.
fn unify_types(generic: &Type, concrete: &Type, subst: &mut HashMap<TyVarId, Type>) {
    match generic.kind() {
        TypeKind::Param(id) => {
            // Found a type variable - record its concrete type
            subst.entry(*id).or_insert_with(|| concrete.clone());
        }
        TypeKind::Fn { params: gen_params, ret: gen_ret } => {
            // Recursively unify function types
            if let TypeKind::Fn { params: conc_params, ret: conc_ret } = concrete.kind() {
                for (gp, cp) in gen_params.iter().zip(conc_params.iter()) {
                    unify_types(gp, cp, subst);
                }
                unify_types(gen_ret, conc_ret, subst);
            }
        }
        TypeKind::Tuple(gen_fields) => {
            if let TypeKind::Tuple(conc_fields) = concrete.kind() {
                for (gf, cf) in gen_fields.iter().zip(conc_fields.iter()) {
                    unify_types(gf, cf, subst);
                }
            }
        }
        TypeKind::Array { element: gen_elem, .. } => {
            if let TypeKind::Array { element: conc_elem, .. } = concrete.kind() {
                unify_types(gen_elem, conc_elem, subst);
            }
        }
        TypeKind::Slice { element: gen_elem } => {
            if let TypeKind::Slice { element: conc_elem } = concrete.kind() {
                unify_types(gen_elem, conc_elem, subst);
            }
        }
        TypeKind::Ref { inner: gen_inner, .. } => {
            if let TypeKind::Ref { inner: conc_inner, .. } = concrete.kind() {
                unify_types(gen_inner, conc_inner, subst);
            }
        }
        TypeKind::Ptr { inner: gen_inner, .. } => {
            if let TypeKind::Ptr { inner: conc_inner, .. } = concrete.kind() {
                unify_types(gen_inner, conc_inner, subst);
            }
        }
        TypeKind::Adt { args: gen_args, .. } => {
            if let TypeKind::Adt { args: conc_args, .. } = concrete.kind() {
                for (ga, ca) in gen_args.iter().zip(conc_args.iter()) {
                    unify_types(ga, ca, subst);
                }
            }
        }
        // For primitive types, unit, etc., there's nothing to unify
        _ => {}
    }
}

/// Substitute types in a MIR statement.
fn substitute_statement_types(stmt: &mut crate::mir::types::Statement, subst: &HashMap<TyVarId, Type>) {
    use crate::mir::types::StatementKind;

    match &mut stmt.kind {
        StatementKind::Assign(_, rvalue) => {
            substitute_rvalue_types(rvalue, subst);
        }
        // PushInlineHandler contains types in operations that need substitution
        StatementKind::PushInlineHandler { operations, .. } => {
            for op in operations {
                for ty in &mut op.param_types {
                    substitute_type(ty, subst);
                }
                substitute_type(&mut op.return_type, subst);
            }
        }
        // These statement kinds don't contain types that need substitution
        StatementKind::Nop
        | StatementKind::StorageLive(_)
        | StatementKind::StorageDead(_)
        | StatementKind::Drop(_)
        | StatementKind::IncrementGeneration(_)
        | StatementKind::CaptureSnapshot(_)
        | StatementKind::ValidateGeneration { .. }
        | StatementKind::PushHandler { .. }
        | StatementKind::PopHandler
        | StatementKind::CallReturnClause { .. } => {}
    }
}

/// Substitute types in an rvalue.
fn substitute_rvalue_types(rvalue: &mut crate::mir::types::Rvalue, subst: &HashMap<TyVarId, Type>) {
    use crate::mir::types::Rvalue;

    match rvalue {
        Rvalue::Use(op) => substitute_operand_types(op, subst),
        Rvalue::BinaryOp { left, right, .. } | Rvalue::CheckedBinaryOp { left, right, .. } => {
            substitute_operand_types(left, subst);
            substitute_operand_types(right, subst);
        }
        Rvalue::UnaryOp { operand, .. } => substitute_operand_types(operand, subst),
        Rvalue::Ref { .. } | Rvalue::AddressOf { .. } => {}
        Rvalue::Aggregate { operands, .. } => {
            for op in operands {
                substitute_operand_types(op, subst);
            }
        }
        Rvalue::Discriminant(_) | Rvalue::Len(_) | Rvalue::ReadGeneration(_) => {}
        Rvalue::Cast { operand, target_ty } => {
            substitute_operand_types(operand, subst);
            *target_ty = substitute_type(target_ty, subst);
        }
        Rvalue::NullCheck(op) => substitute_operand_types(op, subst),
        Rvalue::MakeGenPtr { address, generation, metadata } => {
            substitute_operand_types(address, subst);
            substitute_operand_types(generation, subst);
            substitute_operand_types(metadata, subst);
        }
        Rvalue::ZeroInit(ty) => {
            *ty = substitute_type(ty, subst);
        }
    }
}

/// Substitute types in an operand.
fn substitute_operand_types(op: &mut crate::mir::types::Operand, subst: &HashMap<TyVarId, Type>) {
    use crate::mir::types::Operand;

    match op {
        Operand::Constant(c) => {
            c.ty = substitute_type(&c.ty, subst);
        }
        Operand::Copy(_) | Operand::Move(_) => {}
    }
}

/// Substitute types in a terminator.
fn substitute_terminator_types(term: &mut crate::mir::types::Terminator, subst: &HashMap<TyVarId, Type>) {
    use crate::mir::types::TerminatorKind;

    match &mut term.kind {
        TerminatorKind::Call { func, args, .. } => {
            substitute_operand_types(func, subst);
            for arg in args {
                substitute_operand_types(arg, subst);
            }
        }
        TerminatorKind::SwitchInt { discr, .. } => {
            substitute_operand_types(discr, subst);
        }
        TerminatorKind::Assert { cond, .. } => {
            substitute_operand_types(cond, subst);
        }
        TerminatorKind::DropAndReplace { value, .. } => {
            substitute_operand_types(value, subst);
        }
        TerminatorKind::Perform { args, .. } => {
            for arg in args {
                substitute_operand_types(arg, subst);
            }
        }
        TerminatorKind::Resume { value } => {
            if let Some(val) = value {
                substitute_operand_types(val, subst);
            }
        }
        TerminatorKind::Goto { .. }
        | TerminatorKind::Return
        | TerminatorKind::Unreachable
        | TerminatorKind::StaleReference { .. } => {}
    }
}

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
    /// Current function's DefId for escape analysis lookup and main function detection.
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
    /// Global constants: maps DefId to LLVM global value.
    pub(super) const_globals: HashMap<DefId, inkwell::values::GlobalValue<'ctx>>,
    /// Global statics: maps DefId to LLVM global value.
    pub(super) static_globals: HashMap<DefId, inkwell::values::GlobalValue<'ctx>>,
    /// Current continuation pointer for deep handler operations.
    /// When set, compile_resume calls the continuation instead of returning.
    pub(super) current_continuation: Option<PointerValue<'ctx>>,
    /// Whether the current handler operation is multi-shot.
    /// When true, compile_resume clones the continuation before resuming.
    pub(super) is_multishot_handler: bool,
    /// The DefId of the main function, if it exists.
    /// Used to handle main's special return type (must return i32 for C runtime).
    pub(super) main_fn_def_id: Option<DefId>,
    /// Generic function definitions for monomorphization.
    /// Maps DefId to (FnDef, Body) for generic functions.
    pub(super) generic_fn_defs: HashMap<DefId, (hir::FnDef, hir::Body)>,
    /// Generic function MIR bodies for monomorphization.
    /// Maps DefId to MirBody for generic functions.
    pub(super) generic_mir_bodies: HashMap<DefId, MirBody>,
    /// Monomorphization cache: (generic DefId, type args) -> monomorphized DefId.
    pub(super) mono_cache: HashMap<(DefId, Vec<Type>), DefId>,
    /// Counter for generating unique monomorphized DefIds.
    pub(super) mono_counter: u32,
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
            const_globals: HashMap::new(),
            static_globals: HashMap::new(),
            current_continuation: None,
            is_multishot_handler: false,
            main_fn_def_id: None,
            generic_fn_defs: HashMap::new(),
            generic_mir_bodies: HashMap::new(),
            mono_cache: HashMap::new(),
            mono_counter: 0,
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

    /// Store MIR bodies for generic functions (for monomorphization).
    ///
    /// Generic functions are not compiled directly - instead, they are
    /// monomorphized on-demand when called with concrete types.
    pub fn set_generic_mir_bodies(&mut self, mir_bodies: &HashMap<DefId, MirBody>) {
        for (&def_id, mir_body) in mir_bodies {
            if let Some((fn_def, _)) = self.generic_fn_defs.get(&def_id) {
                if !fn_def.sig.generics.is_empty() {
                    self.generic_mir_bodies.insert(def_id, mir_body.clone());
                }
            }
        }
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
                    // Normalize type parameters to sequential indices so substitution works
                    // Must normalize all fields together to ensure consistent TyVarId mapping
                    let field_types: Vec<Type> = match &struct_def.kind {
                        hir::StructKind::Record(fields) => {
                            let raw_types: Vec<Type> = fields.iter().map(|f| f.ty.clone()).collect();
                            normalize_types_together(&raw_types)
                        }
                        hir::StructKind::Tuple(fields) => {
                            let raw_types: Vec<Type> = fields.iter().map(|f| f.ty.clone()).collect();
                            normalize_types_together(&raw_types)
                        }
                        hir::StructKind::Unit => Vec::new(),
                    };
                    self.struct_defs.insert(*def_id, field_types);
                }
                hir::ItemKind::Enum(enum_def) => {
                    // For enums, we need to collect ALL field types across ALL variants
                    // to build a consistent TyVarId mapping
                    let all_field_types: Vec<Type> = enum_def.variants.iter()
                        .flat_map(|variant| {
                            match &variant.fields {
                                hir::StructKind::Record(fields) => {
                                    fields.iter().map(|f| f.ty.clone()).collect::<Vec<_>>()
                                }
                                hir::StructKind::Tuple(fields) => {
                                    fields.iter().map(|f| f.ty.clone()).collect::<Vec<_>>()
                                }
                                hir::StructKind::Unit => Vec::new(),
                            }
                        })
                        .collect();

                    // Build shared mapping across all variant fields
                    let tyvar_to_idx = build_tyvar_mapping(&all_field_types);

                    // Now normalize each variant's fields using the shared mapping
                    let variants: Vec<Vec<Type>> = enum_def.variants.iter().map(|variant| {
                        match &variant.fields {
                            hir::StructKind::Record(fields) => {
                                fields.iter()
                                    .map(|f| normalize_type_recursive(&f.ty, &tyvar_to_idx))
                                    .collect()
                            }
                            hir::StructKind::Tuple(fields) => {
                                fields.iter()
                                    .map(|f| normalize_type_recursive(&f.ty, &tyvar_to_idx))
                                    .collect()
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
                // - Module: items are processed recursively, no LLVM codegen for the module itself
                hir::ItemKind::Fn(_)
                | hir::ItemKind::Handler { .. }
                | hir::ItemKind::TypeAlias { .. }
                | hir::ItemKind::Const { .. }
                | hir::ItemKind::Static { .. }
                | hir::ItemKind::Trait { .. }
                | hir::ItemKind::Impl { .. }
                | hir::ItemKind::ExternFn(_)
                | hir::ItemKind::Bridge(_)
                | hir::ItemKind::Module(_) => {}
            }
        }

        // Second pass: collect handler definitions (effects must be registered first)
        // Also register handlers in struct_defs so they can be compiled as ADTs
        for (def_id, item) in &hir_crate.items {
            if let hir::ItemKind::Handler { state, .. } = &item.kind {
                // Register handler as an ADT in struct_defs (state fields are the struct fields)
                // Normalize field types: replace arbitrary TyVarIds with sequential indices (0, 1, 2...)
                // so substitution with type args works correctly during lower_type
                // Must normalize all fields together to ensure consistent TyVarId mapping
                let raw_types: Vec<Type> = state.iter().map(|s| s.ty.clone()).collect();
                let field_types = normalize_types_together(&raw_types);
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

                // Store generic function definitions for on-demand monomorphization
                if !fn_def.sig.generics.is_empty() {
                    if let Some(body_id) = fn_def.body_id {
                        if let Some(body) = hir_crate.get_body(body_id) {
                            self.generic_fn_defs.insert(*def_id, (fn_def.clone(), body.clone()));
                        }
                    }
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

    /// Declare FFI items from bridge blocks.
    ///
    /// This declares external functions, structs, enums, unions, constants, and callbacks
    /// from FFI bridge blocks. The declarations are made in LLVM IR with C-compatible layouts.
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

                    // Declare FFI structs with C layout
                    for ffi_struct in &bridge_def.structs {
                        self.declare_ffi_struct(ffi_struct)?;
                    }

                    // Declare FFI enums (C enums are integer types with named constants)
                    for ffi_enum in &bridge_def.enums {
                        self.declare_ffi_enum(ffi_enum)?;
                    }

                    // Declare FFI unions
                    for ffi_union in &bridge_def.unions {
                        self.declare_ffi_union(ffi_union)?;
                    }

                    // Declare FFI constants
                    for ffi_const in &bridge_def.consts {
                        self.declare_ffi_constant(ffi_const)?;
                    }

                    // Declare FFI callbacks (function pointer types)
                    for ffi_callback in &bridge_def.callbacks {
                        self.declare_ffi_callback(ffi_callback)?;
                    }

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

    /// Declare an FFI struct type with C-compatible layout.
    fn declare_ffi_struct(&mut self, ffi_struct: &hir::FfiStruct) -> Result<(), Vec<Diagnostic>> {
        let field_types: Vec<_> = ffi_struct.fields.iter()
            .map(|f| self.lower_type(&f.ty))
            .collect();

        let _struct_type = self.context.struct_type(&field_types, ffi_struct.is_packed);

        // Register the struct type for later use
        self.struct_defs.insert(ffi_struct.def_id, ffi_struct.fields.iter()
            .map(|f| f.ty.clone())
            .collect());

        // Also create an opaque named struct for better LLVM IR readability
        let _named_struct = self.context.opaque_struct_type(&ffi_struct.name);

        // Store the LLVM type for later reference if needed
        // The type is registered in struct_defs which lower_type will use

        Ok(())
    }

    /// Declare an FFI enum type.
    ///
    /// C enums are represented as integer types with named constants.
    fn declare_ffi_enum(&mut self, ffi_enum: &hir::FfiEnum) -> Result<(), Vec<Diagnostic>> {
        // Lower the representation type (typically i32 for C enums)
        let repr_type = self.lower_type(&ffi_enum.repr);

        // Create global constants for each enum variant
        for variant in &ffi_enum.variants {
            let const_name = format!("{}_{}", ffi_enum.name, variant.name);
            if let inkwell::types::BasicTypeEnum::IntType(int_type) = repr_type {
                let const_val = int_type.const_int(variant.value as u64, variant.value < 0);
                let global = self.module.add_global(int_type, Some(AddressSpace::default()), &const_name);
                global.set_initializer(&const_val);
                global.set_constant(true);
                global.set_linkage(inkwell::module::Linkage::Private);
            }
        }

        // Register the enum as a simple struct with just the discriminant
        self.enum_defs.insert(ffi_enum.def_id, vec![Vec::new()]);

        Ok(())
    }

    /// Declare an FFI union type.
    ///
    /// Unions have all fields at offset 0, so we create a struct with the size of the
    /// largest field and interpret the memory differently based on use.
    fn declare_ffi_union(&mut self, ffi_union: &hir::FfiUnion) -> Result<(), Vec<Diagnostic>> {
        // Find the largest field to determine union size
        let mut max_size: u64 = 0;
        let mut max_align: u32 = 1;

        for field in &ffi_union.fields {
            let field_type = self.lower_type(&field.ty);
            let size = self.get_type_size_in_bytes(field_type);
            let align = self.get_type_alignment(field_type);
            if size > max_size {
                max_size = size;
            }
            if align > max_align {
                max_align = align;
            }
        }

        // Create a byte array of the union size for the storage
        let storage_type = self.context.i8_type().array_type(max_size as u32);

        // Create a named struct type for the union
        let _union_struct = self.context.struct_type(&[storage_type.into()], false);

        // Register the union type - we use the first field's type for simplicity
        // In practice, users must cast to the appropriate field type
        if let Some(first_field) = ffi_union.fields.first() {
            self.struct_defs.insert(ffi_union.def_id, vec![first_field.ty.clone()]);
        } else {
            // Empty union - just use a zero-sized placeholder
            self.struct_defs.insert(ffi_union.def_id, Vec::new());
        }

        Ok(())
    }

    /// Declare an FFI constant.
    fn declare_ffi_constant(&mut self, ffi_const: &hir::FfiConst) -> Result<(), Vec<Diagnostic>> {
        let llvm_type = self.lower_type(&ffi_const.ty);

        let const_val: inkwell::values::BasicValueEnum = match llvm_type {
            inkwell::types::BasicTypeEnum::IntType(int_type) => {
                int_type.const_int(ffi_const.value as u64, ffi_const.value < 0).into()
            }
            inkwell::types::BasicTypeEnum::FloatType(float_type) => {
                float_type.const_float(ffi_const.value as f64).into()
            }
            _ => {
                // For non-numeric types, use i64 and let the user cast
                self.context.i64_type().const_int(ffi_const.value as u64, ffi_const.value < 0).into()
            }
        };

        let global = self.module.add_global(llvm_type, Some(AddressSpace::default()), &ffi_const.name);
        global.set_initializer(&const_val);
        global.set_constant(true);

        // Store in const_globals for reference
        self.const_globals.insert(ffi_const.def_id, global);

        Ok(())
    }

    /// Declare an FFI callback type (function pointer).
    fn declare_ffi_callback(&mut self, ffi_callback: &hir::FfiCallback) -> Result<(), Vec<Diagnostic>> {
        // Lower parameter types
        let param_types: Vec<_> = ffi_callback.params.iter()
            .map(|p| self.lower_type(p).into())
            .collect();

        // Lower return type
        let return_type = self.lower_type(&ffi_callback.return_type);

        // Create the function type
        let fn_type = return_type.fn_type(&param_types, false);

        // Create a type alias (via a named struct that wraps a function pointer)
        // This is primarily for documentation; function pointers in Blood work directly
        let _ptr_type = fn_type.ptr_type(AddressSpace::default());

        // For callback resolution during type checking, we don't need to store anything
        // The callback is resolved through the type system

        Ok(())
    }

    /// Get the alignment of an LLVM type in bytes.
    fn get_type_alignment(&self, ty: BasicTypeEnum<'ctx>) -> u32 {
        // Return conservative alignments based on type
        match ty {
            BasicTypeEnum::IntType(int_ty) => {
                let bits = int_ty.get_bit_width();
                std::cmp::min(bits / 8, 8).max(1)
            }
            BasicTypeEnum::FloatType(float_ty) => {
                if float_ty == self.context.f32_type() {
                    4
                } else {
                    8
                }
            }
            BasicTypeEnum::PointerType(_) => 8,
            BasicTypeEnum::StructType(_) => 8, // Conservative alignment for structs
            BasicTypeEnum::ArrayType(_) => 8,
            BasicTypeEnum::VectorType(_) => 16,
        }
    }

    /// Compile const items from HIR.
    ///
    /// Creates LLVM global constants for each const item. For simple literals,
    /// the value is directly embedded. For complex expressions, const evaluation
    /// is performed.
    pub fn compile_const_items(&mut self, hir_crate: &hir::Crate) -> Result<(), Vec<Diagnostic>> {
        for (def_id, item) in &hir_crate.items {
            if let hir::ItemKind::Const { ty, body_id } = &item.kind {
                // Lower the type
                let llvm_type = self.lower_type(ty);

                // Get the body to evaluate the initializer
                let body = hir_crate.bodies.get(body_id).ok_or_else(|| {
                    vec![Diagnostic::error(
                        format!("Missing body for const `{}`", item.name),
                        item.span,
                    )]
                })?;

                // Evaluate the const expression to get the initializer
                let init_value = self.evaluate_const_expr(&body.expr, ty)?;

                // Create global constant with private linkage to avoid
                // multiple definition errors when the same const is used
                // across multiple compilation units
                let global = self.module.add_global(
                    llvm_type,
                    Some(AddressSpace::default()),
                    &item.name,
                );
                global.set_initializer(&init_value);
                global.set_constant(true);
                global.set_linkage(inkwell::module::Linkage::Private);

                // Store for later reference
                self.const_globals.insert(*def_id, global);
            }
        }
        Ok(())
    }

    /// Compile static items from HIR.
    ///
    /// Creates LLVM global variables for each static item. Mutable statics
    /// are marked as non-constant, allowing runtime mutation.
    pub fn compile_static_items(&mut self, hir_crate: &hir::Crate) -> Result<(), Vec<Diagnostic>> {
        for (def_id, item) in &hir_crate.items {
            if let hir::ItemKind::Static { ty, mutable, body_id } = &item.kind {
                // Lower the type
                let llvm_type = self.lower_type(ty);

                // Get the body to evaluate the initializer
                let body = hir_crate.bodies.get(body_id).ok_or_else(|| {
                    vec![Diagnostic::error(
                        format!("Missing body for static `{}`", item.name),
                        item.span,
                    )]
                })?;

                // Evaluate the static expression to get the initializer
                let init_value = self.evaluate_const_expr(&body.expr, ty)?;

                // Create global variable
                let global = self.module.add_global(
                    llvm_type,
                    Some(AddressSpace::default()),
                    &item.name,
                );
                global.set_initializer(&init_value);
                global.set_constant(!*mutable); // Only constant if not mutable

                // Store for later reference
                self.static_globals.insert(*def_id, global);
            }
        }
        Ok(())
    }

    /// Evaluate a const expression at compile time.
    ///
    /// Supports literals, unary/binary ops, block expressions with let bindings,
    /// if/else expressions, and path references to local const bindings.
    fn evaluate_const_expr(
        &self,
        expr: &hir::Expr,
        ty: &Type,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        use std::collections::HashMap;
        // Start with empty const bindings environment
        let bindings: HashMap<hir::LocalId, inkwell::values::BasicValueEnum<'ctx>> = HashMap::new();
        self.evaluate_const_expr_with_env(expr, ty, &bindings)
    }

    /// Evaluate a const expression with an environment of known const bindings.
    fn evaluate_const_expr_with_env(
        &self,
        expr: &hir::Expr,
        ty: &Type,
        bindings: &std::collections::HashMap<hir::LocalId, inkwell::values::BasicValueEnum<'ctx>>,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        use crate::hir::ExprKind;

        match &expr.kind {
            ExprKind::Literal(lit) => self.evaluate_literal(lit, ty),
            ExprKind::Unary { op, operand } => {
                let operand_val = self.evaluate_const_expr_with_env(operand, ty, bindings)?;
                self.evaluate_const_unary_op(*op, operand_val, ty)
            }
            ExprKind::Binary { op, left, right } => {
                let left_val = self.evaluate_const_expr_with_env(left, ty, bindings)?;
                let right_val = self.evaluate_const_expr_with_env(right, ty, bindings)?;
                self.evaluate_const_binary_op(*op, left_val, right_val, ty)
            }
            ExprKind::Block { stmts, expr: final_expr, .. } => {
                // Process let statements to build up const bindings
                let mut local_bindings = bindings.clone();

                for stmt in stmts {
                    match stmt {
                        hir::Stmt::Let { local_id, init } => {
                            if let Some(init_expr) = init {
                                // Evaluate the initializer
                                let init_val = self.evaluate_const_expr_with_env(
                                    init_expr,
                                    &init_expr.ty,
                                    &local_bindings,
                                )?;
                                local_bindings.insert(*local_id, init_val);
                            } else {
                                return Err(vec![Diagnostic::error(
                                    "Uninitialized let bindings are not allowed in const context",
                                    expr.span,
                                )]);
                            }
                        }
                        hir::Stmt::Expr(_) => {
                            // Expression statements in const context are side-effect free
                            // so we can mostly ignore them (they don't produce values)
                        }
                        hir::Stmt::Item(_) => {
                            // Item statements (inner functions, etc.) are ignored in const eval
                        }
                    }
                }

                // Evaluate the final expression with accumulated bindings
                if let Some(final_expr) = final_expr {
                    self.evaluate_const_expr_with_env(final_expr, ty, &local_bindings)
                } else {
                    // Empty block returns unit
                    Ok(self.context.i8_type().const_zero().into())
                }
            }
            ExprKind::If { condition, then_branch, else_branch } => {
                // Evaluate condition at compile time
                let cond_val = self.evaluate_const_expr_with_env(
                    condition,
                    &Type::bool(),
                    bindings,
                )?;

                // Extract the boolean value
                let cond_bool = if let inkwell::values::BasicValueEnum::IntValue(int_val) = cond_val {
                    // Get the constant value (0 = false, non-zero = true)
                    int_val.get_zero_extended_constant().map(|v| v != 0)
                } else {
                    None
                };

                match cond_bool {
                    Some(true) => {
                        self.evaluate_const_expr_with_env(then_branch, ty, bindings)
                    }
                    Some(false) => {
                        if let Some(else_branch) = else_branch {
                            self.evaluate_const_expr_with_env(else_branch, ty, bindings)
                        } else {
                            // No else branch and condition is false - return unit
                            Ok(self.context.i8_type().const_zero().into())
                        }
                    }
                    None => {
                        Err(vec![Diagnostic::error(
                            "If condition in const context must be a compile-time constant",
                            condition.span,
                        )])
                    }
                }
            }
            ExprKind::Local(local_id) => {
                // Check if this is a local binding we've evaluated
                if let Some(value) = bindings.get(local_id) {
                    Ok(*value)
                } else {
                    Err(vec![Diagnostic::error(
                        "Local variable in const context must have been previously bound",
                        expr.span,
                    )])
                }
            }
            ExprKind::Def(def_id) => {
                // Check if this is a const item we've already compiled
                if let Some(global) = self.const_globals.get(def_id) {
                    // Get the initializer value from the global
                    if let Some(init) = global.get_initializer() {
                        return Ok(init);
                    }
                }
                Err(vec![Diagnostic::error(
                    "Definition in const context must refer to a const item",
                    expr.span,
                )])
            }
            ExprKind::Cast { expr: inner_expr, target_ty } => {
                // Evaluate the inner expression
                let inner_val = self.evaluate_const_expr_with_env(inner_expr, &inner_expr.ty, bindings)?;
                // Perform the cast at compile time
                self.evaluate_const_cast(inner_val, &inner_expr.ty, target_ty, expr.span)
            }
            ExprKind::Tuple(elements) => {
                // Evaluate each element
                let mut element_values = Vec::new();
                if let hir::TypeKind::Tuple(element_types) = ty.kind() {
                    for (elem, elem_ty) in elements.iter().zip(element_types.iter()) {
                        let val = self.evaluate_const_expr_with_env(elem, elem_ty, bindings)?;
                        element_values.push(val);
                    }
                } else {
                    return Err(vec![Diagnostic::error(
                        "Expected tuple type for tuple expression",
                        expr.span,
                    )]);
                }

                // Create a constant struct for the tuple
                let llvm_types: Vec<inkwell::types::BasicTypeEnum> = element_values.iter()
                    .map(|v| v.get_type())
                    .collect();
                let struct_type = self.context.struct_type(&llvm_types, false);
                let const_values: Vec<inkwell::values::BasicValueEnum> = element_values;
                Ok(struct_type.const_named_struct(&const_values).into())
            }
            ExprKind::Array(elements) => {
                if let hir::TypeKind::Array { element: elem_ty, .. } = ty.kind() {
                    let mut element_values = Vec::new();
                    for elem in elements {
                        let val = self.evaluate_const_expr_with_env(elem, elem_ty, bindings)?;
                        element_values.push(val);
                    }

                    // Create a constant array
                    if element_values.is_empty() {
                        let elem_llvm_ty = self.lower_type(elem_ty);
                        return Ok(elem_llvm_ty.array_type(0).const_zero().into());
                    }

                    // All elements must be the same type
                    let first = element_values[0];
                    let array_vals: Result<Vec<_>, _> = element_values.iter()
                        .map(|v| match v {
                            inkwell::values::BasicValueEnum::IntValue(iv) => Ok(*iv),
                            _ => Err(()),
                        })
                        .collect();

                    if let Ok(int_vals) = array_vals {
                        let int_type = first.into_int_value().get_type();
                        return Ok(int_type.const_array(&int_vals).into());
                    }

                    // For other types, fall back to generic struct representation
                    Err(vec![Diagnostic::error(
                        "Complex array expressions in const context are not yet fully supported",
                        expr.span,
                    )])
                } else {
                    Err(vec![Diagnostic::error(
                        "Expected array type for array expression",
                        expr.span,
                    )])
                }
            }
            _ => Err(vec![Diagnostic::error(
                format!("Expression kind {:?} is not const-evaluable", std::mem::discriminant(&expr.kind)),
                expr.span,
            )]),
        }
    }

    /// Evaluate a const cast at compile time.
    fn evaluate_const_cast(
        &self,
        value: inkwell::values::BasicValueEnum<'ctx>,
        _from_ty: &Type,
        to_ty: &Type,
        span: Span,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        let target_llvm_ty = self.lower_type(to_ty);

        match (value, target_llvm_ty) {
            (inkwell::values::BasicValueEnum::IntValue(int_val), inkwell::types::BasicTypeEnum::IntType(target_int)) => {
                // Integer to integer cast
                let from_bits = int_val.get_type().get_bit_width();
                let to_bits = target_int.get_bit_width();
                if from_bits < to_bits {
                    // Zero-extend (const_z_ext)
                    Ok(int_val.const_z_ext(target_int).into())
                } else if from_bits > to_bits {
                    // Truncate (const_truncate)
                    Ok(int_val.const_truncate(target_int).into())
                } else {
                    Ok(int_val.into())
                }
            }
            (inkwell::values::BasicValueEnum::IntValue(int_val), inkwell::types::BasicTypeEnum::FloatType(target_float)) => {
                // Integer to float cast
                Ok(int_val.const_signed_to_float(target_float).into())
            }
            (inkwell::values::BasicValueEnum::FloatValue(float_val), inkwell::types::BasicTypeEnum::IntType(target_int)) => {
                // Float to integer cast
                Ok(float_val.const_to_signed_int(target_int).into())
            }
            (inkwell::values::BasicValueEnum::FloatValue(float_val), inkwell::types::BasicTypeEnum::FloatType(target_float)) => {
                // Float to float cast
                Ok(float_val.const_cast(target_float).into())
            }
            _ => Err(vec![Diagnostic::error(
                "Unsupported cast in const context",
                span,
            )]),
        }
    }

    /// Evaluate a literal to an LLVM constant.
    fn evaluate_literal(
        &self,
        lit: &hir::LiteralValue,
        ty: &Type,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        use hir::LiteralValue;
        use hir::PrimitiveTy;

        match lit {
            LiteralValue::Int(value) => {
                // Lower the type to get the LLVM integer type
                let llvm_type = self.lower_type(ty);
                if let inkwell::types::BasicTypeEnum::IntType(int_type) = llvm_type {
                    Ok(int_type.const_int(*value as u64, true).into())
                } else {
                    // Fallback to i64
                    Ok(self.context.i64_type().const_int(*value as u64, true).into())
                }
            }
            LiteralValue::Uint(value) => {
                // Lower the type to get the LLVM integer type
                let llvm_type = self.lower_type(ty);
                if let inkwell::types::BasicTypeEnum::IntType(int_type) = llvm_type {
                    Ok(int_type.const_int(*value as u64, false).into())
                } else {
                    // Fallback to u64
                    Ok(self.context.i64_type().const_int(*value as u64, false).into())
                }
            }
            LiteralValue::Float(value) => {
                // Check if it's f32 or f64 via PrimitiveTy
                let is_f32 = matches!(
                    ty.kind(),
                    hir::TypeKind::Primitive(PrimitiveTy::Float(hir::FloatTy::F32))
                );
                if is_f32 {
                    Ok(self.context.f32_type().const_float(*value).into())
                } else {
                    Ok(self.context.f64_type().const_float(*value).into())
                }
            }
            LiteralValue::Bool(value) => {
                Ok(self.context.bool_type().const_int(*value as u64, false).into())
            }
            LiteralValue::Char(c) => {
                // Chars are u32 in Blood/Rust
                Ok(self.context.i32_type().const_int(*c as u64, false).into())
            }
            LiteralValue::String(s) => {
                // Create a global string constant and str slice {ptr, len}
                let bytes = s.as_bytes();
                let string_type = self.context.i8_type().array_type((bytes.len() + 1) as u32);
                let global = self.module.add_global(string_type, Some(AddressSpace::default()), "");
                global.set_initializer(&self.context.const_string(bytes, true));
                global.set_constant(true);

                // Get pointer to string data
                let ptr = self.builder.build_pointer_cast(
                    global.as_pointer_value(),
                    self.context.i8_type().ptr_type(AddressSpace::default()),
                    "str_ptr"
                ).map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
                let len = self.context.i64_type().const_int(bytes.len() as u64, false);

                // Create str slice struct {ptr, len}
                let str_type = self.context.struct_type(
                    &[self.context.i8_type().ptr_type(AddressSpace::default()).into(),
                      self.context.i64_type().into()],
                    false
                );
                let str_val = str_type.const_named_struct(&[ptr.into(), len.into()]);
                Ok(str_val.into())
            }
            LiteralValue::ByteString(bytes) => {
                // Create a global byte array constant and byte slice {ptr, len}
                let array_type = self.context.i8_type().array_type(bytes.len() as u32);
                let global = self.module.add_global(array_type, Some(AddressSpace::default()), "bytes");
                global.set_initializer(&self.context.const_string(bytes, false));
                global.set_constant(true);

                // Cast array pointer to i8*
                let ptr = global.as_pointer_value().const_cast(
                    self.context.i8_type().ptr_type(AddressSpace::default())
                );
                let len = self.context.i64_type().const_int(bytes.len() as u64, false);

                // Create byte slice struct {ptr, len}
                let slice_type = self.context.struct_type(
                    &[self.context.i8_type().ptr_type(AddressSpace::default()).into(),
                      self.context.i64_type().into()],
                    false
                );
                let slice_val = slice_type.const_named_struct(&[ptr.into(), len.into()]);
                Ok(slice_val.into())
            }
        }
    }

    /// Evaluate a const unary operation.
    fn evaluate_const_unary_op(
        &self,
        op: crate::ast::UnaryOp,
        operand: inkwell::values::BasicValueEnum<'ctx>,
        _ty: &Type,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        use crate::ast::UnaryOp;

        match op {
            UnaryOp::Neg => {
                if operand.is_int_value() {
                    let int_val = operand.into_int_value();
                    let zero = int_val.get_type().const_zero();
                    Ok(zero.const_sub(int_val).into())
                } else if operand.is_float_value() {
                    let float_val = operand.into_float_value();
                    Ok(float_val.const_neg().into())
                } else {
                    Err(vec![Diagnostic::error(
                        "Negation on unsupported type in const context".to_string(),
                        crate::span::Span::dummy(),
                    )])
                }
            }
            UnaryOp::Not => {
                if operand.is_int_value() {
                    let int_val = operand.into_int_value();
                    Ok(int_val.const_not().into())
                } else {
                    Err(vec![Diagnostic::error(
                        "Not on unsupported type in const context".to_string(),
                        crate::span::Span::dummy(),
                    )])
                }
            }
            _ => Err(vec![Diagnostic::error(
                format!("Unary operator {:?} not supported in const context", op),
                crate::span::Span::dummy(),
            )]),
        }
    }

    /// Evaluate a const binary operation.
    fn evaluate_const_binary_op(
        &self,
        op: crate::ast::BinOp,
        left: inkwell::values::BasicValueEnum<'ctx>,
        right: inkwell::values::BasicValueEnum<'ctx>,
        _ty: &Type,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>, Vec<Diagnostic>> {
        use crate::ast::BinOp;

        // Try integer operations
        if left.is_int_value() && right.is_int_value() {
            let left_int = left.into_int_value();
            let right_int = right.into_int_value();
            let result = match op {
                BinOp::Add => left_int.const_add(right_int),
                BinOp::Sub => left_int.const_sub(right_int),
                BinOp::Mul => left_int.const_mul(right_int),
                BinOp::Div => left_int.const_signed_div(right_int),
                BinOp::Rem => left_int.const_signed_remainder(right_int),
                BinOp::BitAnd => left_int.const_and(right_int),
                BinOp::BitOr => left_int.const_or(right_int),
                BinOp::BitXor => left_int.const_xor(right_int),
                BinOp::Shl => left_int.const_shl(right_int),
                BinOp::Shr => left_int.const_ashr(right_int),
                BinOp::Eq => left_int.const_int_compare(inkwell::IntPredicate::EQ, right_int),
                BinOp::Ne => left_int.const_int_compare(inkwell::IntPredicate::NE, right_int),
                BinOp::Lt => left_int.const_int_compare(inkwell::IntPredicate::SLT, right_int),
                BinOp::Le => left_int.const_int_compare(inkwell::IntPredicate::SLE, right_int),
                BinOp::Gt => left_int.const_int_compare(inkwell::IntPredicate::SGT, right_int),
                BinOp::Ge => left_int.const_int_compare(inkwell::IntPredicate::SGE, right_int),
                _ => {
                    return Err(vec![Diagnostic::error(
                        format!("Binary operator {:?} not supported in const context", op),
                        crate::span::Span::dummy(),
                    )]);
                }
            };
            return Ok(result.into());
        }

        // Try float operations
        if left.is_float_value() && right.is_float_value() {
            let left_float = left.into_float_value();
            let right_float = right.into_float_value();
            let result = match op {
                BinOp::Add => left_float.const_add(right_float),
                BinOp::Sub => left_float.const_sub(right_float),
                BinOp::Mul => left_float.const_mul(right_float),
                BinOp::Div => left_float.const_div(right_float),
                _ => {
                    return Err(vec![Diagnostic::error(
                        format!("Float binary operator {:?} not supported in const context", op),
                        crate::span::Span::dummy(),
                    )]);
                }
            };
            return Ok(result.into());
        }

        Err(vec![Diagnostic::error(
            "Binary operation on unsupported types in const context".to_string(),
            crate::span::Span::dummy(),
        )])
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
                // Set the WASM import attributes using Inkwell's string attribute API
                use inkwell::attributes::AttributeLoc;

                // Create and set wasm-import-name attribute (the function name in the WASM module)
                let import_name_attr = self.context.create_string_attribute(
                    "wasm-import-name",
                    link_name,
                );
                fn_value.add_attribute(AttributeLoc::Function, import_name_attr);

                // Set wasm-import-module if provided (defaults to "env" if not specified)
                let module_name = wasm_import_module.unwrap_or("env");
                let import_module_attr = self.context.create_string_attribute(
                    "wasm-import-module",
                    module_name,
                );
                fn_value.add_attribute(AttributeLoc::Function, import_module_attr);

                // Also store in map for reference
                self.wasm_imports.insert(link_name.to_string(), module_name.to_string());
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
                    // Normalize type parameters to sequential indices so substitution works
                    // Must normalize all fields together to ensure consistent TyVarId mapping
                    let field_types: Vec<Type> = match &struct_def.kind {
                        hir::StructKind::Record(fields) => {
                            let raw_types: Vec<Type> = fields.iter().map(|f| f.ty.clone()).collect();
                            normalize_types_together(&raw_types)
                        }
                        hir::StructKind::Tuple(fields) => {
                            let raw_types: Vec<Type> = fields.iter().map(|f| f.ty.clone()).collect();
                            normalize_types_together(&raw_types)
                        }
                        hir::StructKind::Unit => Vec::new(),
                    };
                    self.struct_defs.insert(*def_id, field_types);
                }
                hir::ItemKind::Enum(enum_def) => {
                    // For enums, we need to collect ALL field types across ALL variants
                    // to build a consistent TyVarId mapping
                    let all_field_types: Vec<Type> = enum_def.variants.iter()
                        .flat_map(|variant| {
                            match &variant.fields {
                                hir::StructKind::Record(fields) => {
                                    fields.iter().map(|f| f.ty.clone()).collect::<Vec<_>>()
                                }
                                hir::StructKind::Tuple(fields) => {
                                    fields.iter().map(|f| f.ty.clone()).collect::<Vec<_>>()
                                }
                                hir::StructKind::Unit => Vec::new(),
                            }
                        })
                        .collect();

                    // Build shared mapping across all variant fields
                    let tyvar_to_idx = build_tyvar_mapping(&all_field_types);

                    // Now normalize each variant's fields using the shared mapping
                    let variants: Vec<Vec<Type>> = enum_def.variants.iter().map(|variant| {
                        match &variant.fields {
                            hir::StructKind::Record(fields) => {
                                fields.iter()
                                    .map(|f| normalize_type_recursive(&f.ty, &tyvar_to_idx))
                                    .collect()
                            }
                            hir::StructKind::Tuple(fields) => {
                                fields.iter()
                                    .map(|f| normalize_type_recursive(&f.ty, &tyvar_to_idx))
                                    .collect()
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
                // - Module: items are processed recursively, no LLVM codegen for the module itself
                hir::ItemKind::Fn(_)
                | hir::ItemKind::Handler { .. }
                | hir::ItemKind::TypeAlias { .. }
                | hir::ItemKind::Const { .. }
                | hir::ItemKind::Static { .. }
                | hir::ItemKind::Trait { .. }
                | hir::ItemKind::Impl { .. }
                | hir::ItemKind::ExternFn(_)
                | hir::ItemKind::Bridge(_)
                | hir::ItemKind::Module(_) => {}
            }
        }

        // Second pass: collect handler definitions (effects must be registered first)
        // Also register handlers in struct_defs so they can be compiled as ADTs
        for (def_id, item) in &hir_crate.items {
            if let hir::ItemKind::Handler { state, .. } = &item.kind {
                // Register handler as an ADT in struct_defs (state fields are the struct fields)
                // Normalize field types: replace arbitrary TyVarIds with sequential indices (0, 1, 2...)
                // so substitution with type args works correctly during lower_type
                // Must normalize all fields together to ensure consistent TyVarId mapping
                let raw_types: Vec<Type> = state.iter().map(|s| s.ty.clone()).collect();
                let field_types = normalize_types_together(&raw_types);
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

        // Const/Static pass: compile global constants and static variables
        self.compile_const_items(hir_crate)?;
        self.compile_static_items(hir_crate)?;

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

        // Check if this is the main function
        let is_main = self.main_fn_def_id == Some(def_id);

        // Build return
        if body.return_type().is_unit() {
            if is_main {
                // main must return i32 for C runtime - return 0 on success
                let zero = self.context.i32_type().const_int(0, false);
                self.builder.build_return(Some(&zero))
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
            } else {
                self.builder.build_return(None)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
            }
        } else if let Some(value) = result {
            self.builder.build_return(Some(&value))
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
        } else {
            if is_main {
                // main must return i32 for C runtime - return 0 on success
                let zero = self.context.i32_type().const_int(0, false);
                self.builder.build_return(Some(&zero))
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
            } else {
                self.builder.build_return(None)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), Span::dummy())])?;
            }
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
        // Skip generic functions - they will be monomorphized on-demand at call sites
        if !fn_def.sig.generics.is_empty() {
            return Ok(());
        }

        // Generate mangled name for multiple dispatch support
        // main is special and gets renamed to blood_main
        let (llvm_name, fn_type) = if name == "main" {
            // Track main's DefId for special return handling
            self.main_fn_def_id = Some(def_id);
            // main must return i32 for C runtime compatibility, even if Blood type is unit
            let param_types: Vec<inkwell::types::BasicMetadataTypeEnum> = fn_def.sig.inputs.iter()
                .map(|p| self.lower_type(p).into())
                .collect();
            let fn_type = self.context.i32_type().fn_type(&param_types, false);
            ("blood_main".to_string(), fn_type)
        } else {
            // Mangle name with parameter types to support multiple dispatch
            let llvm_name = Self::mangle_function_name(name, &fn_def.sig);
            let fn_type = self.fn_type_from_sig(&fn_def.sig);
            (llvm_name, fn_type)
        };
        let fn_value = self.module.add_function(&llvm_name, fn_type, None);
        self.functions.insert(def_id, fn_value);
        Ok(())
    }

    /// Mangle a function name with its parameter types for multiple dispatch.
    ///
    /// This generates unique symbol names for overloaded functions.
    /// For example: `add(i32, i32)` becomes `add$i32$i32`
    fn mangle_function_name(name: &str, sig: &hir::FnSig) -> String {
        if sig.inputs.is_empty() {
            name.to_string()
        } else {
            let param_mangles: Vec<String> = sig.inputs.iter()
                .map(|ty| Self::mangle_type(ty))
                .collect();
            format!("{}${}", name, param_mangles.join("$"))
        }
    }

    /// Generate a mangled name for a type.
    fn mangle_type(ty: &Type) -> String {
        match ty.kind() {
            TypeKind::Primitive(prim) => match prim {
                PrimitiveTy::Bool => "bool".to_string(),
                PrimitiveTy::Char => "char".to_string(),
                PrimitiveTy::Int(int_ty) => match int_ty {
                    IntTy::I8 => "i8".to_string(),
                    IntTy::I16 => "i16".to_string(),
                    IntTy::I32 => "i32".to_string(),
                    IntTy::I64 => "i64".to_string(),
                    IntTy::I128 => "i128".to_string(),
                    IntTy::Isize => "isize".to_string(),
                },
                PrimitiveTy::Uint(uint_ty) => match uint_ty {
                    UintTy::U8 => "u8".to_string(),
                    UintTy::U16 => "u16".to_string(),
                    UintTy::U32 => "u32".to_string(),
                    UintTy::U64 => "u64".to_string(),
                    UintTy::U128 => "u128".to_string(),
                    UintTy::Usize => "usize".to_string(),
                },
                PrimitiveTy::Float(float_ty) => match float_ty {
                    FloatTy::F32 => "f32".to_string(),
                    FloatTy::F64 => "f64".to_string(),
                },
                PrimitiveTy::String => "str".to_string(),
                PrimitiveTy::Str => "rawstr".to_string(),
                PrimitiveTy::Unit => "unit".to_string(),
                PrimitiveTy::Never => "never".to_string(),
            },
            TypeKind::Tuple(elems) => {
                let elem_mangles: Vec<String> = elems.iter()
                    .map(|t| Self::mangle_type(t))
                    .collect();
                format!("T{}", elem_mangles.join("_"))
            }
            TypeKind::Array { element, size } => {
                format!("A{}_{}", size, Self::mangle_type(element))
            }
            TypeKind::Slice { element } => {
                format!("S{}", Self::mangle_type(element))
            }
            TypeKind::Ref { inner, mutable } => {
                if *mutable {
                    format!("Rm{}", Self::mangle_type(inner))
                } else {
                    format!("R{}", Self::mangle_type(inner))
                }
            }
            TypeKind::Ptr { inner, mutable } => {
                if *mutable {
                    format!("Pm{}", Self::mangle_type(inner))
                } else {
                    format!("P{}", Self::mangle_type(inner))
                }
            }
            TypeKind::Adt { def_id, args } => {
                if args.is_empty() {
                    format!("D{}", def_id.index())
                } else {
                    let arg_mangles: Vec<String> = args.iter()
                        .map(|t| Self::mangle_type(t))
                        .collect();
                    format!("D{}_{}", def_id.index(), arg_mangles.join("_"))
                }
            }
            TypeKind::Fn { params, ret } => {
                let param_mangles: Vec<String> = params.iter()
                    .map(|t| Self::mangle_type(t))
                    .collect();
                format!("F{}_{}", param_mangles.join("_"), Self::mangle_type(ret))
            }
            TypeKind::Never => "never".to_string(),
            TypeKind::Infer(id) => format!("?{}", id.0),
            TypeKind::Param(id) => format!("G{}", id.0),
            TypeKind::Error => "error".to_string(),
            TypeKind::Record { .. } => "record".to_string(),
            TypeKind::Forall { .. } => "forall".to_string(),
            TypeKind::Range { .. } => "range".to_string(),
            TypeKind::Closure { .. } => "closure".to_string(),
            TypeKind::DynTrait { .. } => "dyn".to_string(),
            TypeKind::Ownership { qualifier, inner } => {
                use crate::hir::ty::OwnershipQualifier;
                let prefix = match qualifier {
                    OwnershipQualifier::Linear => "L",
                    OwnershipQualifier::Affine => "A",
                };
                format!("{}{}", prefix, Self::mangle_type(inner))
            }
        }
    }

    /// Declare a closure function from MIR body information.
    ///
    /// Closures have synthetic DefIds (starting at 0xFFFF_0000) that aren't
    /// in the HIR items list. This method declares them using the MIR body's
    /// parameter and return type information.
    ///
    /// The first parameter is always `i8*` (environment pointer), followed by
    /// the actual closure parameters.
    pub fn declare_closure_from_mir(&mut self, def_id: DefId, mir_body: &MirBody) {
        let i8_ptr_ty = self.context.i8_type().ptr_type(AddressSpace::default());

        // Build parameter types: first is always i8* (env), then the actual params
        let params: Vec<_> = mir_body.params().collect::<Vec<_>>();
        let mut param_types: Vec<inkwell::types::BasicMetadataTypeEnum> = Vec::with_capacity(params.len());

        // First param is always the environment pointer (i8*), skip the MIR's __env type
        // because we use a uniform i8* representation regardless of actual captures
        let first_is_env = params.first()
            .map(|p| p.name.as_deref() == Some("__env"))
            .unwrap_or(false);

        if first_is_env {
            // First param is __env, use i8* instead of its declared type
            param_types.push(i8_ptr_ty.into());
            // Add the rest of the params with their actual types
            for param in params.iter().skip(1) {
                param_types.push(self.lower_type(&param.ty).into());
            }
        } else {
            // No __env param (shouldn't happen, but be defensive)
            for param in &params {
                param_types.push(self.lower_type(&param.ty).into());
            }
        }

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

    /// Monomorphize a generic function for specific type arguments.
    ///
    /// This creates a specialized version of a generic function by:
    /// 1. Cloning the MIR body
    /// 2. Substituting type parameters with concrete types
    /// 3. Declaring and compiling the specialized LLVM function
    ///
    /// Returns the LLVM FunctionValue for the monomorphized function, or None
    /// if monomorphization fails.
    pub fn monomorphize_function(
        &mut self,
        generic_def_id: DefId,
        concrete_params: &[Type],
        concrete_ret: &Type,
    ) -> Option<inkwell::values::FunctionValue<'ctx>> {
        // Get the generic function's HIR definition first (needed for type inference)
        let (fn_def, _hir_body) = self.generic_fn_defs.get(&generic_def_id)?.clone();

        // Infer type arguments by unifying generic signature with concrete types
        // This is needed for higher-order generics like `apply<T, R>(f: fn(T) -> R, x: T) -> R`
        // where concrete_params = [fn(i32) -> i32, i32] but we need type_args = [i32, i32]
        let subst = infer_type_args(
            &fn_def.sig.inputs,
            concrete_params,
            &fn_def.sig.output,
            concrete_ret,
        );

        // Build type args list from inferred substitution (in order of fn_def.sig.generics)
        let type_args: Vec<Type> = fn_def.sig.generics.iter()
            .filter_map(|tyvar_id| subst.get(tyvar_id).cloned())
            .collect();

        // Check if already monomorphized
        let cache_key = (generic_def_id, type_args.clone());
        if let Some(&mono_def_id) = self.mono_cache.get(&cache_key) {
            return self.functions.get(&mono_def_id).copied();
        }

        // Get the generic function's MIR body
        let mir_body = self.generic_mir_bodies.get(&generic_def_id)?.clone();

        // Generate unique DefId for monomorphized version
        let mono_def_id = DefId::new(0xFFFE_0000 + self.mono_counter);
        self.mono_counter += 1;

        // Clone and substitute types in MIR body
        let mono_mir = substitute_mir_types(&mir_body, &subst);

        // Build mangled name for monomorphized function
        let base_name = format!("blood_mono_{}_{}", generic_def_id.index(), mono_def_id.index());
        let param_mangles: Vec<String> = concrete_params.iter()
            .map(|ty| Self::mangle_type(ty))
            .collect();
        let llvm_name = if param_mangles.is_empty() {
            base_name
        } else {
            format!("{}${}", base_name, param_mangles.join("$"))
        };

        // Build LLVM function type from concrete types
        let param_types: Vec<inkwell::types::BasicMetadataTypeEnum> = concrete_params.iter()
            .map(|ty| self.lower_type(ty).into())
            .collect();

        let fn_type = if concrete_ret.is_unit() {
            self.context.void_type().fn_type(&param_types, false)
        } else {
            let llvm_ret_type = self.lower_type(concrete_ret);
            llvm_ret_type.fn_type(&param_types, false)
        };

        // Declare the monomorphized function
        let fn_value = self.module.add_function(&llvm_name, fn_type, None);
        self.functions.insert(mono_def_id, fn_value);

        // Cache the monomorphization
        self.mono_cache.insert(cache_key, mono_def_id);

        // Save current function state before compiling monomorphized function
        // This is necessary because compile_mir_body modifies these fields
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_local_generations = std::mem::take(&mut self.local_generations);
        let saved_current_fn = self.current_fn.take();
        let saved_current_fn_def_id = self.current_fn_def_id.take();
        let saved_insert_block = self.builder.get_insert_block();

        // Run escape analysis on the monomorphized MIR body
        // This is critical for performance - without it, all locals would use Region tier
        let mut escape_analyzer = EscapeAnalyzer::new();
        let escape_results = escape_analyzer.analyze(&mono_mir);

        // Compile the monomorphized MIR body with escape analysis results
        use crate::codegen::mir_codegen::MirCodegen;
        let result = self.compile_mir_body(mono_def_id, &mono_mir, Some(&escape_results));

        // Restore previous function state
        self.locals = saved_locals;
        self.local_generations = saved_local_generations;
        self.current_fn = saved_current_fn;
        self.current_fn_def_id = saved_current_fn_def_id;

        // Re-position builder at the original function's current position
        if let Some(insert_bb) = saved_insert_block {
            self.builder.position_at_end(insert_bb);
        }

        if result.is_err() {
            // Compilation failed - return None
            // The error was already collected in self.errors
            return None;
        }

        Some(fn_value)
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

        // str slice type: { ptr: *i8, len: i64 }
        let str_slice_type = self.context.struct_type(
            &[i8_ptr_type.into(), i64_type.into()],
            false
        );

        // print_str({*i8, i64}) -> void
        let print_str_type = void_type.fn_type(&[str_slice_type.into()], false);
        self.module.add_function("print_str", print_str_type, None);

        // println_str({*i8, i64}) -> void
        self.module.add_function("println_str", print_str_type, None);

        // eprint_str({*i8, i64}) -> void - print to stderr
        self.module.add_function("eprint_str", print_str_type, None);

        // eprintln_str({*i8, i64}) -> void - print to stderr with newline
        self.module.add_function("eprintln_str", print_str_type, None);

        // str_len({*i8, i64}) -> i64 - get string length
        let str_len_type = i64_type.fn_type(&[str_slice_type.into()], false);
        self.module.add_function("str_len", str_len_type, None);

        // str_eq({*i8, i64}, {*i8, i64}) -> i1 - compare strings
        let bool_type = self.context.bool_type();
        let str_eq_type = bool_type.fn_type(&[str_slice_type.into(), str_slice_type.into()], false);
        self.module.add_function("str_eq", str_eq_type, None);

        // blood_str_concat({*i8, i64}, {*i8, i64}) -> {*i8, i64} - concatenate strings
        let str_concat_type = str_slice_type.fn_type(&[str_slice_type.into(), str_slice_type.into()], false);
        self.module.add_function("blood_str_concat", str_concat_type, None);

        // int_to_string(i32) -> {*i8, i64} - convert integer to string
        let int_to_string_type = str_slice_type.fn_type(&[i32_type.into()], false);
        self.module.add_function("int_to_string", int_to_string_type, None);

        // bool_to_string(i32) -> {*i8, i64} - convert boolean to string (bool passed as i32)
        self.module.add_function("bool_to_string", int_to_string_type, None);

        // i64_to_string(i64) -> {*i8, i64} - convert 64-bit integer to string
        let i64_to_string_type = str_slice_type.fn_type(&[i64_type.into()], false);
        self.module.add_function("i64_to_string", i64_to_string_type, None);

        // u64_to_string(u64) -> {*i8, i64} - convert unsigned 64-bit integer to string
        let u64_type = self.context.i64_type(); // u64 has same LLVM type as i64
        let u64_to_string_type = str_slice_type.fn_type(&[u64_type.into()], false);
        self.module.add_function("u64_to_string", u64_to_string_type, None);

        // char_to_string(i32) -> {*i8, i64} - convert character to string (char passed as u32/i32)
        let char_to_string_type = str_slice_type.fn_type(&[i32_type.into()], false);
        self.module.add_function("char_to_string", char_to_string_type, None);

        // f32_to_string(f32) -> {*i8, i64} - convert f32 to string
        let f32_type = self.context.f32_type();
        let f32_to_string_type = str_slice_type.fn_type(&[f32_type.into()], false);
        self.module.add_function("f32_to_string", f32_to_string_type, None);

        // f64_to_string(f64) -> {*i8, i64} - convert f64 to string
        let f64_type_def = self.context.f64_type();
        let f64_to_string_type = str_slice_type.fn_type(&[f64_type_def.into()], false);
        self.module.add_function("f64_to_string", f64_to_string_type, None);

        // read_line() -> {*i8, i64} - read a line from stdin
        let read_line_type = str_slice_type.fn_type(&[], false);
        self.module.add_function("read_line", read_line_type, None);

        // read_int() -> i32 - read an integer from stdin
        let read_int_type = i32_type.fn_type(&[], false);
        self.module.add_function("read_int", read_int_type, None);

        // panic({*i8, i64}) -> void (divergent, but declared as void)
        self.module.add_function("panic", print_str_type, None);

        // === Assertions ===

        // blood_assert(i32) -> void - assert condition is true
        let assert_type = void_type.fn_type(&[i32_type.into()], false);
        self.module.add_function("blood_assert", assert_type, None);

        // blood_assert_eq_int(i32, i32) -> void - assert two ints are equal
        let assert_eq_int_type = void_type.fn_type(&[i32_type.into(), i32_type.into()], false);
        self.module.add_function("blood_assert_eq_int", assert_eq_int_type, None);

        // blood_assert_eq_bool(i32, i32) -> void - assert two bools are equal (as i32)
        self.module.add_function("blood_assert_eq_bool", assert_eq_int_type, None);

        // print_char(i32) -> void
        self.module.add_function("print_char", print_int_type, None);

        // println_char(i32) -> void (char with newline)
        self.module.add_function("println_char", print_int_type, None);

        // print_bool(i32) -> void (bool as i32)
        self.module.add_function("print_bool", print_int_type, None);

        // println_bool(i32) -> void (bool with newline)
        self.module.add_function("println_bool", print_int_type, None);

        // print_f64(f64) -> void
        let f64_type = self.context.f64_type();
        let print_f64_type = void_type.fn_type(&[f64_type.into()], false);
        self.module.add_function("print_f64", print_f64_type, None);

        // println_f64(f64) -> void
        self.module.add_function("println_f64", print_f64_type, None);

        // print_f32(f32) -> void
        let f32_type = self.context.f32_type();
        let print_f32_type = void_type.fn_type(&[f32_type.into()], false);
        self.module.add_function("print_f32", print_f32_type, None);

        // println_f32(f32) -> void
        self.module.add_function("println_f32", print_f32_type, None);

        // print_f64_prec(f64, i32) -> void
        let print_f64_prec_type = void_type.fn_type(&[f64_type.into(), i32_type.into()], false);
        self.module.add_function("print_f64_prec", print_f64_prec_type, None);

        // println_f64_prec(f64, i32) -> void
        self.module.add_function("println_f64_prec", print_f64_prec_type, None);

        // print_f32_prec(f32, i32) -> void
        let print_f32_prec_type = void_type.fn_type(&[f32_type.into(), i32_type.into()], false);
        self.module.add_function("print_f32_prec", print_f32_prec_type, None);

        // println_f32_prec(f32, i32) -> void
        self.module.add_function("println_f32_prec", print_f32_prec_type, None);

        // println() -> void
        let println_type = void_type.fn_type(&[], false);
        self.module.add_function("println", println_type, None);

        // === Size Functions ===

        // size_of_i32() -> i64
        let size_of_type = i64_type.fn_type(&[], false);
        self.module.add_function("size_of_i32", size_of_type, None);

        // size_of_i64() -> i64
        self.module.add_function("size_of_i64", size_of_type, None);

        // size_of_bool() -> i64
        self.module.add_function("size_of_bool", size_of_type, None);

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

        // === Simple Memory Allocation (for Vec/collections) ===

        // blood_alloc_simple(size: i64) -> *void (i64 for address)
        let alloc_simple_type = i64_type.fn_type(&[i64_type.into()], false);
        self.module.add_function("blood_alloc_simple", alloc_simple_type, None);

        // blood_realloc(ptr: i64, size: i64) -> *void (i64 for address)
        let realloc_type = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        self.module.add_function("blood_realloc", realloc_type, None);

        // blood_free_simple(ptr: i64) -> void
        let free_simple_type = void_type.fn_type(&[i64_type.into()], false);
        self.module.add_function("blood_free_simple", free_simple_type, None);

        // blood_memcpy(dest: i64, src: i64, n: i64) -> i64
        let memcpy_type = i64_type.fn_type(&[i64_type.into(), i64_type.into(), i64_type.into()], false);
        self.module.add_function("blood_memcpy", memcpy_type, None);

        // === Pointer Read/Write Intrinsics ===

        // ptr_read_i32(ptr: i64) -> i32
        let ptr_read_i32_type = i32_type.fn_type(&[i64_type.into()], false);
        self.module.add_function("ptr_read_i32", ptr_read_i32_type, None);

        // ptr_write_i32(ptr: i64, value: i32) -> void
        let ptr_write_i32_type = void_type.fn_type(&[i64_type.into(), i32_type.into()], false);
        self.module.add_function("ptr_write_i32", ptr_write_i32_type, None);

        // ptr_read_i64(ptr: i64) -> i64
        let ptr_read_i64_type = i64_type.fn_type(&[i64_type.into()], false);
        self.module.add_function("ptr_read_i64", ptr_read_i64_type, None);

        // ptr_write_i64(ptr: i64, value: i64) -> void
        let ptr_write_i64_type = void_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        self.module.add_function("ptr_write_i64", ptr_write_i64_type, None);

        // ptr_read_u64(ptr: i64) -> i64 (u64 represented as i64)
        self.module.add_function("ptr_read_u64", ptr_read_i64_type, None);

        // ptr_write_u64(ptr: i64, value: i64) -> void (u64 represented as i64)
        self.module.add_function("ptr_write_u64", ptr_write_i64_type, None);

        // ptr_read_u8(ptr: i64) -> i8 (u8 represented as i8)
        let ptr_read_u8_type = i8_type.fn_type(&[i64_type.into()], false);
        self.module.add_function("ptr_read_u8", ptr_read_u8_type, None);

        // ptr_write_u8(ptr: i64, value: i8) -> void (u8 represented as i8)
        let ptr_write_u8_type = void_type.fn_type(&[i64_type.into(), i8_type.into()], false);
        self.module.add_function("ptr_write_u8", ptr_write_u8_type, None);

        // ptr_read_f64(ptr: i64) -> f64
        let ptr_read_f64_type = f64_type.fn_type(&[i64_type.into()], false);
        self.module.add_function("ptr_read_f64", ptr_read_f64_type, None);

        // ptr_write_f64(ptr: i64, value: f64) -> void
        let ptr_write_f64_type = void_type.fn_type(&[i64_type.into(), f64_type.into()], false);
        self.module.add_function("ptr_write_f64", ptr_write_f64_type, None);

        // print_i64(i64) -> void - already declared above, but let's ensure

        // println_i64(i64) -> void - already declared above, but let's ensure

        // print_u64(i64) -> void (u64 as i64)
        self.module.add_function("print_u64", print_i64_type, None);

        // println_u64(i64) -> void (u64 as i64)
        self.module.add_function("println_u64", print_i64_type, None);

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

        // blood_perform(effect_id: i64, op_index: i32, args: *i64, arg_count: i64, continuation: i64) -> i64
        let perform_type = i64_type.fn_type(&[
            i64_type.into(),
            i32_type.into(),
            i64_ptr_type.into(),
            i64_type.into(),
            i64_type.into(),  // continuation parameter
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
