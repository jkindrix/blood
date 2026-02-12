//! Code generation context.
//!
//! This module provides the main code generation context which
//! coordinates LLVM code generation for a Blood program.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::builder::Builder;
use inkwell::basic_block::BasicBlock;
use inkwell::values::{FunctionValue, PointerValue};
use inkwell::types::{BasicType, BasicTypeEnum};
use inkwell::AddressSpace;

use crate::hir::{self, DefId, LocalId, Type, PrimitiveTy, IntTy, UintTy, FloatTy};
use crate::hir::ty::{TypeKind, TyVarId};
use crate::mir::{EscapeAnalyzer, EscapeResults, MirBody, InlineHandlerBodies, ClosureAnalysisResults};
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
        TypeKind::Fn { params, ret, effects, .. } => {
            for p in params {
                collect_tyvars(p, tyvars);
            }
            collect_tyvars(ret, tyvars);
            // Also collect type vars from effect annotations
            for eff in effects {
                for ty_arg in &eff.type_args {
                    collect_tyvars(ty_arg, tyvars);
                }
            }
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
            Type::array_with_const(normalized, size.clone())
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
        TypeKind::Fn { params, ret, effects, const_args } => {
            let normalized_params: Vec<Type> = params.iter()
                .map(|p| normalize_type_recursive(p, tyvar_to_idx))
                .collect();
            let normalized_ret = normalize_type_recursive(ret, tyvar_to_idx);
            // Also normalize effect type args
            let normalized_effects: Vec<hir::FnEffect> = effects.iter()
                .map(|eff| hir::FnEffect {
                    def_id: eff.def_id,
                    type_args: eff.type_args.iter()
                        .map(|ty| normalize_type_recursive(ty, tyvar_to_idx))
                        .collect(),
                })
                .collect();
            Type::function_with_const_args(normalized_params, normalized_ret, normalized_effects, const_args.clone())
        }
        _ => ty.clone(),
    }
}

/// Build a mapping from DefId to module-qualified path for stable linker symbols.
///
/// Walks the HIR module tree to determine each item's canonical path. The path
/// is canonical regardless of which compilation unit computes it, because we
/// stop at external module boundaries (files loaded via `mod foo;`).
///
/// For example:
/// - Root function `helper` in `mymod.blood` -> `"mymod::helper"`
/// - Function in external module `mod_a::helper` -> `"mod_a::helper"`
/// - Nested `mod_a::sub::helper` -> `"mod_a::sub::helper"`
///
/// The key invariant: an item's path is the same regardless of which file
/// imports it, because the walk stops at the first external module ancestor
/// (the file boundary where the item is defined).
fn build_def_paths(hir_crate: &hir::Crate) -> HashMap<DefId, String> {
    // Step 1: Build parent map (child DefId -> parent Module DefId)
    // and track which modules are external (loaded from separate files).
    let mut parent_map: HashMap<DefId, DefId> = HashMap::new();
    let mut external_modules: HashSet<DefId> = HashSet::new();
    for (mod_def_id, item) in &hir_crate.items {
        if let hir::ItemKind::Module(mod_def) = &item.kind {
            if mod_def.is_external {
                external_modules.insert(*mod_def_id);
            }
            for child_def_id in &mod_def.items {
                parent_map.insert(*child_def_id, *mod_def_id);
            }
        }
    }

    // Step 2: For each item, walk up parent chain to build canonical path.
    // Stop at (and include) the first external module ancestor, which
    // represents the file boundary where the item is defined. This ensures
    // the path is the same regardless of which compilation unit computes it.
    //
    // If no external ancestor exists, include the root module (the current
    // compilation unit's own module), which is the file where the item lives.
    let mut paths = HashMap::new();
    for (def_id, item) in &hir_crate.items {
        let mut segments = Vec::new();
        segments.push(item.name.clone());

        // Walk up the parent chain collecting module names
        let mut current = *def_id;
        while let Some(&parent_id) = parent_map.get(&current) {
            if let Some(parent_item) = hir_crate.items.get(&parent_id) {
                segments.push(parent_item.name.clone());
                // Stop at the first external module — this is the file boundary.
                // The item's canonical path is relative to its defining file.
                if external_modules.contains(&parent_id) {
                    break;
                }
            }
            current = parent_id;
        }

        // Reverse to get root-first order and join with "::"
        segments.reverse();
        let qualified_path = segments.join("::");
        paths.insert(*def_id, qualified_path);
    }

    paths
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
    let ctx = SubstContext::new(subst.clone());
    substitute_mir_types_with_ctx(mir, &ctx)
}

/// Substitute type and const parameters in a MIR body with full context.
fn substitute_mir_types_with_ctx(mir: &MirBody, ctx: &SubstContext) -> MirBody {
    // Clone the MIR body
    let mut result = mir.clone();

    // Substitute types in locals, but skip the __env parameter for closures.
    // The __env parameter should always be i8* in the calling convention,
    // not the actual captured environment type.
    for local in &mut result.locals {
        if local.name.as_deref() != Some("__env") {
            local.ty = substitute_type_with_ctx(&local.ty, ctx);
        }
    }

    // Substitute types in basic blocks
    for block in &mut result.basic_blocks {
        // Substitute in statements
        for stmt in &mut block.statements {
            substitute_statement_types_with_ctx(stmt, ctx);
        }

        // Substitute in terminator
        if let Some(term) = &mut block.terminator {
            substitute_terminator_types_with_ctx(term, ctx);
        }
    }

    result
}

/// Substitution context containing both type and const param mappings.
struct SubstContext {
    type_subst: HashMap<TyVarId, Type>,
    const_subst: HashMap<hir::ConstParamId, hir::ConstValue>,
}

impl SubstContext {
    fn new(type_subst: HashMap<TyVarId, Type>) -> Self {
        Self {
            type_subst,
            const_subst: HashMap::new(),
        }
    }

    fn substitute_const(&self, cv: &hir::ConstValue) -> hir::ConstValue {
        match cv {
            hir::ConstValue::Param(id) => {
                self.const_subst.get(id).cloned().unwrap_or_else(|| cv.clone())
            }
            _ => cv.clone(),
        }
    }
}

/// Substitute type parameters in a type with full context.
fn substitute_type_with_ctx(ty: &Type, ctx: &SubstContext) -> Type {
    match ty.kind() {
        TypeKind::Param(id) => {
            if let Some(concrete) = ctx.type_subst.get(id) {
                concrete.clone()
            } else {
                ty.clone()
            }
        }
        TypeKind::Tuple(fields) => {
            let subst_fields: Vec<Type> = fields.iter()
                .map(|f| substitute_type_with_ctx(f, ctx))
                .collect();
            Type::tuple(subst_fields)
        }
        TypeKind::Array { element, size } => {
            let subst_elem = substitute_type_with_ctx(element, ctx);
            let subst_size = ctx.substitute_const(size);
            Type::array_with_const(subst_elem, subst_size)
        }
        TypeKind::Slice { element } => {
            Type::slice(substitute_type_with_ctx(element, ctx))
        }
        TypeKind::Ref { inner, mutable } => {
            Type::reference(substitute_type_with_ctx(inner, ctx), *mutable)
        }
        TypeKind::Ptr { inner, mutable } => {
            Type::new(TypeKind::Ptr {
                inner: substitute_type_with_ctx(inner, ctx),
                mutable: *mutable,
            })
        }
        TypeKind::Adt { def_id, args } => {
            let subst_args: Vec<Type> = args.iter()
                .map(|a| substitute_type_with_ctx(a, ctx))
                .collect();
            Type::adt(*def_id, subst_args)
        }
        TypeKind::Fn { params, ret, effects, const_args } => {
            let subst_params: Vec<Type> = params.iter()
                .map(|p| substitute_type_with_ctx(p, ctx))
                .collect();
            let subst_effects: Vec<hir::FnEffect> = effects.iter()
                .map(|eff| hir::FnEffect {
                    def_id: eff.def_id,
                    type_args: eff.type_args.iter()
                        .map(|ty| substitute_type_with_ctx(ty, ctx))
                        .collect(),
                })
                .collect();
            // Also substitute const params in const_args
            let subst_const_args: Vec<(hir::ConstParamId, hir::ConstValue)> = const_args.iter()
                .map(|(param_id, val)| {
                    let subst_val = match val {
                        hir::ConstValue::Param(p) => {
                            ctx.const_subst.get(p).cloned().unwrap_or_else(|| val.clone())
                        }
                        _ => val.clone(),
                    };
                    (*param_id, subst_val)
                })
                .collect();
            Type::function_with_const_args(subst_params, substitute_type_with_ctx(ret, ctx), subst_effects, subst_const_args)
        }
        _ => ty.clone(),
    }
}

/// Result of type inference containing both type and const param substitutions.
struct InferResult {
    type_subst: HashMap<TyVarId, Type>,
    const_subst: HashMap<hir::ConstParamId, hir::ConstValue>,
}

/// Infer both type and const arguments.
fn infer_type_and_const_args(
    generic_params: &[Type],
    concrete_params: &[Type],
    generic_ret: &Type,
    concrete_ret: &Type,
) -> InferResult {
    let mut type_subst: HashMap<TyVarId, Type> = HashMap::new();
    let mut const_subst: HashMap<hir::ConstParamId, hir::ConstValue> = HashMap::new();

    // Unify parameters
    for (generic, concrete) in generic_params.iter().zip(concrete_params.iter()) {
        unify_types(generic, concrete, &mut type_subst, &mut const_subst);
    }

    // Unify return type
    unify_types(generic_ret, concrete_ret, &mut type_subst, &mut const_subst);

    InferResult { type_subst, const_subst }
}

/// Recursively unify a generic type with a concrete type, populating the substitution maps.
fn unify_types(
    generic: &Type,
    concrete: &Type,
    type_subst: &mut HashMap<TyVarId, Type>,
    const_subst: &mut HashMap<hir::ConstParamId, hir::ConstValue>,
) {
    match generic.kind() {
        TypeKind::Param(id) => {
            // Found a type variable - record its concrete type
            type_subst.entry(*id).or_insert_with(|| concrete.clone());
        }
        TypeKind::Fn { params: gen_params, ret: gen_ret, effects: gen_effects, const_args: gen_const_args } => {
            // Recursively unify function types
            if let TypeKind::Fn { params: conc_params, ret: conc_ret, effects: conc_effects, const_args: conc_const_args } = concrete.kind() {
                for (gp, cp) in gen_params.iter().zip(conc_params.iter()) {
                    unify_types(gp, cp, type_subst, const_subst);
                }
                unify_types(gen_ret, conc_ret, type_subst, const_subst);
                // Also unify effect type arguments
                // Match effects by def_id and unify their type args
                for gen_eff in gen_effects {
                    for conc_eff in conc_effects {
                        if gen_eff.def_id == conc_eff.def_id {
                            for (gen_arg, conc_arg) in gen_eff.type_args.iter().zip(conc_eff.type_args.iter()) {
                                unify_types(gen_arg, conc_arg, type_subst, const_subst);
                            }
                        }
                    }
                }
                // Unify const args - if concrete type has explicit const args, use them
                // This handles the turbofish case: make_matrix::<3, 4>()
                for (param_id, conc_val) in conc_const_args {
                    const_subst.entry(*param_id).or_insert_with(|| conc_val.clone());
                }
                // If generic has const args but concrete doesn't, that's also valid (inference)
                for (param_id, gen_val) in gen_const_args {
                    if let hir::ConstValue::Param(_) = gen_val {
                        // Skip - this will be resolved later
                    } else {
                        const_subst.entry(*param_id).or_insert_with(|| gen_val.clone());
                    }
                }
            }
        }
        TypeKind::Tuple(gen_fields) => {
            if let TypeKind::Tuple(conc_fields) = concrete.kind() {
                for (gf, cf) in gen_fields.iter().zip(conc_fields.iter()) {
                    unify_types(gf, cf, type_subst, const_subst);
                }
            }
        }
        TypeKind::Array { element: gen_elem, size: gen_size } => {
            if let TypeKind::Array { element: conc_elem, size: conc_size } = concrete.kind() {
                unify_types(gen_elem, conc_elem, type_subst, const_subst);
                // Extract const param mapping from array size
                if let hir::ConstValue::Param(id) = gen_size {
                    const_subst.entry(*id).or_insert_with(|| conc_size.clone());
                }
            }
        }
        TypeKind::Slice { element: gen_elem } => {
            if let TypeKind::Slice { element: conc_elem } = concrete.kind() {
                unify_types(gen_elem, conc_elem, type_subst, const_subst);
            }
        }
        TypeKind::Ref { inner: gen_inner, .. } => {
            if let TypeKind::Ref { inner: conc_inner, .. } = concrete.kind() {
                unify_types(gen_inner, conc_inner, type_subst, const_subst);
            }
        }
        TypeKind::Ptr { inner: gen_inner, .. } => {
            if let TypeKind::Ptr { inner: conc_inner, .. } = concrete.kind() {
                unify_types(gen_inner, conc_inner, type_subst, const_subst);
            }
        }
        TypeKind::Adt { args: gen_args, .. } => {
            if let TypeKind::Adt { args: conc_args, .. } = concrete.kind() {
                for (ga, ca) in gen_args.iter().zip(conc_args.iter()) {
                    unify_types(ga, ca, type_subst, const_subst);
                }
            }
        }
        // For primitive types, unit, etc., there's nothing to unify
        _ => {}
    }
}

/// Substitute types in a MIR statement with full context.
fn substitute_statement_types_with_ctx(stmt: &mut crate::mir::types::Statement, ctx: &SubstContext) {
    use crate::mir::types::StatementKind;

    match &mut stmt.kind {
        StatementKind::Assign(_, rvalue) => {
            substitute_rvalue_types_with_ctx(rvalue, ctx);
        }
        StatementKind::PushInlineHandler { operations, .. } => {
            for op in operations {
                for ty in &mut op.param_types {
                    *ty = substitute_type_with_ctx(ty, ctx);
                }
                op.return_type = substitute_type_with_ctx(&op.return_type, ctx);
            }
        }
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

/// Substitute types in an rvalue with full context.
fn substitute_rvalue_types_with_ctx(rvalue: &mut crate::mir::types::Rvalue, ctx: &SubstContext) {
    use crate::mir::types::Rvalue;

    match rvalue {
        Rvalue::Use(op) => substitute_operand_types_with_ctx(op, ctx),
        Rvalue::BinaryOp { left, right, .. } | Rvalue::CheckedBinaryOp { left, right, .. } => {
            substitute_operand_types_with_ctx(left, ctx);
            substitute_operand_types_with_ctx(right, ctx);
        }
        Rvalue::UnaryOp { operand, .. } => substitute_operand_types_with_ctx(operand, ctx),
        Rvalue::Ref { .. } | Rvalue::AddressOf { .. } => {}
        Rvalue::Aggregate { operands, .. } => {
            for op in operands {
                substitute_operand_types_with_ctx(op, ctx);
            }
        }
        Rvalue::Discriminant(_) | Rvalue::Len(_) | Rvalue::VecLen(_) | Rvalue::ReadGeneration(_) => {}
        Rvalue::Cast { operand, target_ty } => {
            substitute_operand_types_with_ctx(operand, ctx);
            *target_ty = substitute_type_with_ctx(target_ty, ctx);
        }
        Rvalue::NullCheck(op) => substitute_operand_types_with_ctx(op, ctx),
        Rvalue::MakeGenPtr { address, generation, metadata } => {
            substitute_operand_types_with_ctx(address, ctx);
            substitute_operand_types_with_ctx(generation, ctx);
            substitute_operand_types_with_ctx(metadata, ctx);
        }
        Rvalue::ZeroInit(ty) => {
            *ty = substitute_type_with_ctx(ty, ctx);
        }
        Rvalue::StringIndex { base, index } => {
            substitute_operand_types_with_ctx(base, ctx);
            substitute_operand_types_with_ctx(index, ctx);
        }
        Rvalue::ArrayToSlice { array_ref, .. } => {
            substitute_operand_types_with_ctx(array_ref, ctx);
        }
    }
}

/// Substitute types in an operand with full context (including const param substitution).
fn substitute_operand_types_with_ctx(op: &mut crate::mir::types::Operand, ctx: &SubstContext) {
    use crate::mir::types::{Operand, ConstantKind};

    match op {
        Operand::Constant(c) => {
            c.ty = substitute_type_with_ctx(&c.ty, ctx);
            // Substitute const params with concrete values
            if let ConstantKind::ConstParam(id) = &c.kind {
                if let Some(concrete_val) = ctx.const_subst.get(id) {
                    match concrete_val {
                        hir::ConstValue::Uint(n) => c.kind = ConstantKind::Uint(*n),
                        hir::ConstValue::Int(n) => c.kind = ConstantKind::Int(*n as i128),
                        _ => {}
                    }
                }
            }
        }
        Operand::Copy(_) | Operand::Move(_) => {}
    }
}

/// Substitute types in a terminator with full context.
fn substitute_terminator_types_with_ctx(term: &mut crate::mir::types::Terminator, ctx: &SubstContext) {
    use crate::mir::types::TerminatorKind;

    match &mut term.kind {
        TerminatorKind::Call { func, args, .. } => {
            substitute_operand_types_with_ctx(func, ctx);
            for arg in args {
                substitute_operand_types_with_ctx(arg, ctx);
            }
        }
        TerminatorKind::SwitchInt { discr, .. } => {
            substitute_operand_types_with_ctx(discr, ctx);
        }
        TerminatorKind::Assert { cond, .. } => {
            substitute_operand_types_with_ctx(cond, ctx);
        }
        TerminatorKind::DropAndReplace { value, .. } => {
            substitute_operand_types_with_ctx(value, ctx);
        }
        TerminatorKind::Perform { args, .. } => {
            for arg in args {
                substitute_operand_types_with_ctx(arg, ctx);
            }
        }
        TerminatorKind::Resume { value } => {
            if let Some(val) = value {
                substitute_operand_types_with_ctx(val, ctx);
            }
        }
        TerminatorKind::Goto { .. }
        | TerminatorKind::Return
        | TerminatorKind::Unreachable
        | TerminatorKind::StaleReference { .. } => {}
    }
}

/// Collect all closure DefIds referenced in a MIR body.
fn collect_closure_def_ids(mir: &MirBody) -> Vec<DefId> {
    use crate::mir::types::{Rvalue, AggregateKind, StatementKind};

    let mut closure_ids = Vec::new();

    for block in &mir.basic_blocks {
        for stmt in &block.statements {
            if let StatementKind::Assign(_, Rvalue::Aggregate { kind: AggregateKind::Closure { def_id }, .. }) = &stmt.kind {
                closure_ids.push(*def_id);
            }
        }
    }

    closure_ids
}

/// Remap closure DefIds in a MIR body using the provided mapping.
fn remap_closure_def_ids(mir: &mut MirBody, remap: &HashMap<DefId, DefId>) {
    use crate::mir::types::{Rvalue, AggregateKind, StatementKind};

    for block in &mut mir.basic_blocks {
        for stmt in &mut block.statements {
            if let StatementKind::Assign(_, Rvalue::Aggregate { kind: AggregateKind::Closure { def_id }, .. }) = &mut stmt.kind {
                if let Some(&new_id) = remap.get(def_id) {
                    *def_id = new_id;
                }
            }
        }
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
    /// Map from LocalId to the LLVM pointee type of the local's allocation.
    /// Used with opaque pointers where the type can't be inferred from the pointer.
    pub local_types: HashMap<LocalId, BasicTypeEnum<'ctx>>,
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
    /// Map from persistent-tier LocalId to slot ID storage (stack alloca holding u64 slot ID).
    /// Used for RC lifecycle management: blood_persistent_decrement on StorageDead.
    pub persistent_slot_ids: HashMap<LocalId, PointerValue<'ctx>>,
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
    /// Static types: maps DefId to the Blood type of the static.
    pub(super) static_types: HashMap<DefId, Type>,
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
    /// Trait default method definitions for monomorphization.
    /// Maps method DefId to (trait_id, FnSig, Body).
    /// When a default method is called on a concrete type, we need to
    /// monomorphize it by substituting `Self` with the concrete type.
    pub(super) trait_default_fn_defs: HashMap<DefId, (DefId, hir::FnSig, hir::Body)>,
    /// Generic function MIR bodies for monomorphization.
    /// Maps DefId to MirBody for generic functions.
    pub(super) generic_mir_bodies: HashMap<DefId, MirBody>,
    /// Monomorphization cache: (generic DefId, type args, const args) -> monomorphized DefId.
    pub(super) mono_cache: HashMap<(DefId, Vec<Type>, Vec<(hir::ConstParamId, hir::ConstValue)>), DefId>,
    /// Counter for generating unique monomorphized DefIds (for non-closure functions).
    pub(super) mono_counter: u32,
    /// Counter for generating unique monomorphized closure DefIds.
    /// Uses base 0xFFFF_8000 so they're still detected as closures (>= 0xFFFF_0000).
    pub(super) mono_closure_counter: u32,
    /// Monomorphized closure MIR bodies pending compilation.
    /// When a generic function with closures is monomorphized, nested closures
    /// also need their types substituted. This stores those for later compilation.
    pub(super) mono_closure_mirs: HashMap<DefId, MirBody>,
    /// Inline handler bodies for try/with blocks (codegen for inline effect handlers).
    /// Maps synthetic DefId to the handler body info.
    pub(super) inline_handler_bodies: InlineHandlerBodies,
    /// Wrapper functions for plain functions used as fn() pointers.
    /// When a plain function is converted to a fat pointer { fn_ptr, env_ptr },
    /// we need a wrapper that accepts env_ptr and forwards to the original.
    /// Maps original function DefId -> wrapper function.
    pub(super) fn_ptr_wrappers: HashMap<DefId, FunctionValue<'ctx>>,
    /// DefId for built-in Box<T> type.
    pub(super) box_def_id: Option<DefId>,
    /// DefId for built-in Vec<T> type.
    pub(super) vec_def_id: Option<DefId>,
    /// DefId for built-in Option<T> type.
    pub(super) option_def_id: Option<DefId>,
    /// DefId for built-in Result<T, E> type.
    pub(super) result_def_id: Option<DefId>,
    /// Module-qualified paths for each DefId.
    /// Built at the start of codegen by walking the HIR module tree.
    /// Used for stable linker symbol names that don't depend on DefId indices.
    pub(super) def_paths: HashMap<DefId, String>,
    /// Closure analysis results for inline environment optimization.
    /// When a closure's environment is ≤16 bytes and the closure doesn't escape,
    /// captures are stored directly in the closure struct instead of through a pointer.
    pub(super) closure_analysis: Option<ClosureAnalysisResults>,
    /// Stack of LLVM values representing currently active region IDs.
    /// Pushed when `blood_region_activate(region_id)` is called during codegen,
    /// popped when `blood_region_deactivate()` is called. Used by
    /// `compile_perform_terminator` to emit `blood_continuation_add_suspended_region`
    /// calls for non-tail-resumptive effects so that regions are kept alive
    /// across continuation capture/resume.
    pub(super) active_regions: Vec<inkwell::values::IntValue<'ctx>>,
    /// Maps handler state local ID → shadow i64 alloca pointer.
    /// Handler op bodies use uniform i64 layout for state fields because they
    /// are compiled once per definition (not per instantiation) and may have
    /// generic/unresolved state field types. The shadow alloca bridges the
    /// typed state struct and the handler body's i64 view.
    pub(super) handler_state_shadows: HashMap<LocalId, PointerValue<'ctx>>,
    /// ADT types currently being lowered (cycle detection for recursive types).
    /// Uses the full Type (including type arguments) to distinguish between
    /// different instantiations like Container<i32> vs Container<Container<i32>>.
    pub(super) lowering_adts: RefCell<HashSet<Type>>,
    /// Errors collected during type lowering (which takes &self, not &mut self).
    /// Drained into `self.errors` at compilation checkpoints.
    pub(super) type_lowering_errors: RefCell<Vec<Diagnostic>>,
    /// Current MIR span for error reporting during codegen.
    /// Updated as each statement/terminator is compiled so that downstream
    /// codegen helpers can produce diagnostics with source locations.
    /// Uses Cell<Span> because Span is Copy and we only need interior mutability.
    pub(super) current_mir_span: Cell<Span>,
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
            local_types: HashMap::new(),
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
            persistent_slot_ids: HashMap::new(),
            vtables: HashMap::new(),
            vtable_layouts: HashMap::new(),
            wasm_imports: HashMap::new(),
            const_globals: HashMap::new(),
            static_globals: HashMap::new(),
            static_types: HashMap::new(),
            current_continuation: None,
            is_multishot_handler: false,
            main_fn_def_id: None,
            generic_fn_defs: HashMap::new(),
            trait_default_fn_defs: HashMap::new(),
            generic_mir_bodies: HashMap::new(),
            mono_cache: HashMap::new(),
            mono_counter: 0,
            mono_closure_counter: 0,
            mono_closure_mirs: HashMap::new(),
            inline_handler_bodies: HashMap::new(),
            fn_ptr_wrappers: HashMap::new(),
            box_def_id: None,
            vec_def_id: None,
            option_def_id: None,
            result_def_id: None,
            def_paths: HashMap::new(),
            closure_analysis: None,
            active_regions: Vec::new(),
            handler_state_shadows: HashMap::new(),
            lowering_adts: RefCell::new(HashSet::new()),
            type_lowering_errors: RefCell::new(Vec::new()),
            current_mir_span: Cell::new(Span::dummy()),
        }
    }

    /// Set built-in type DefIds for proper type lowering.
    ///
    /// These DefIds are used to identify Box<T>, Vec<T>, Option<T>, and Result<T, E>
    /// during type lowering so they can be given their correct LLVM representations.
    pub fn set_builtin_def_ids(
        &mut self,
        box_def_id: Option<DefId>,
        vec_def_id: Option<DefId>,
        option_def_id: Option<DefId>,
        result_def_id: Option<DefId>,
    ) {
        self.box_def_id = box_def_id;
        self.vec_def_id = vec_def_id;
        self.option_def_id = option_def_id;
        self.result_def_id = result_def_id;
    }

    /// Get the current MIR source span for error reporting.
    ///
    /// Returns the span of the MIR statement or terminator currently being
    /// compiled. This is updated by `compile_mir_block` before each
    /// statement/terminator compilation.
    pub(super) fn current_span(&self) -> Span {
        self.current_mir_span.get()
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

    /// Set closure analysis results for inline environment optimization.
    ///
    /// When set, closures with small environments (≤16 bytes) that don't escape
    /// will have their captures stored directly in the closure struct rather than
    /// through a separate env_ptr allocation.
    pub fn set_closure_analysis(&mut self, closure_analysis: ClosureAnalysisResults) {
        self.closure_analysis = Some(closure_analysis);
    }

    /// Check if a closure should use inline environment storage.
    ///
    /// Returns true when:
    /// 1. Closure analysis results are available
    /// 2. The closure's environment size is within the inline threshold
    /// 3. The closure doesn't escape (only used in direct calls)
    pub fn should_inline_closure_env(&self, closure_def_id: &DefId, dest_local: Option<LocalId>) -> bool {
        // Check closure analysis says it's an inline candidate
        let is_inline_candidate = self.closure_analysis
            .as_ref()
            .and_then(|ca| ca.get(*closure_def_id))
            .map(|info| info.is_inline_candidate)
            .unwrap_or(false);

        if !is_inline_candidate {
            return false;
        }

        // Check escape analysis says the closure doesn't escape.
        // If it escapes, it may be stored in a fn() variable which requires the
        // fat pointer { fn_ptr, env_ptr } ABI for indirect calls.
        let does_not_escape = dest_local
            .and_then(|local| {
                self.current_fn_def_id
                    .and_then(|fn_id| self.escape_analysis.get(&fn_id))
                    .map(|er| er.get(local) == crate::mir::escape::EscapeState::NoEscape)
            })
            .unwrap_or(false);

        does_not_escape
    }

    /// Store MIR bodies for generic functions (for monomorphization).
    ///
    /// Generic functions are not compiled directly - instead, they are
    /// monomorphized on-demand when called with concrete types.
    ///
    /// This also stores closure MIR bodies (DefIds >= 0xFFFF_0000) which may be
    /// needed when monomorphizing generic functions that contain closures.
    pub fn set_generic_mir_bodies(&mut self, mir_bodies: &HashMap<DefId, MirBody>) {
        for (&def_id, mir_body) in mir_bodies {
            // Store generic function MIR bodies
            if let Some((fn_def, _)) = self.generic_fn_defs.get(&def_id) {
                if !fn_def.sig.generics.is_empty() || !fn_def.sig.const_generics.is_empty() {
                    self.generic_mir_bodies.insert(def_id, mir_body.clone());
                }
            }
            // Store trait default method MIR bodies for on-demand monomorphization
            // These have an implicit `Self` type parameter that needs to be substituted
            if self.trait_default_fn_defs.contains_key(&def_id) {
                self.generic_mir_bodies.insert(def_id, mir_body.clone());
            }
            // Also store closure MIR bodies (synthetic DefIds >= 0xFFFF_0000)
            // These may be needed when monomorphizing generic functions that contain closures
            if def_id.index() >= 0xFFFF_0000 {
                self.generic_mir_bodies.insert(def_id, mir_body.clone());
            }
        }
    }

    /// Store inline handler bodies for try/with blocks.
    ///
    /// These are used to compile inline handler operation bodies to LLVM functions
    /// during codegen of PushInlineHandler statements.
    pub fn set_inline_handler_bodies(&mut self, inline_handlers: InlineHandlerBodies) {
        self.inline_handler_bodies = inline_handlers;
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
        // Build module-qualified paths for stable linker symbols
        self.def_paths = build_def_paths(hir_crate);

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
                // Collect trait default method definitions for on-demand monomorphization
                hir::ItemKind::Trait { items, .. } => {
                    for trait_item in items {
                        if let hir::TraitItemKind::Fn(sig, Some(body_id)) = &trait_item.kind {
                            // This is a default method with a body
                            if let Some(body) = hir_crate.get_body(*body_id) {
                                self.trait_default_fn_defs.insert(
                                    trait_item.def_id,
                                    (*def_id, sig.clone(), body.clone()),
                                );
                            }
                        }
                    }
                }
                // These item kinds are handled elsewhere or don't need declaration-phase processing:
                // - Fn: processed in second pass for function declarations
                // - Handler: processed in second pass after effects are registered
                // - TypeAlias: resolved during type checking
                // - Const/Static: compiled with function bodies
                // - Impl: resolved during type checking
                // - ExternFn/Bridge: processed in fourth pass for FFI declarations
                // - Module: items are processed recursively, no LLVM codegen for the module itself
                hir::ItemKind::Fn(_)
                | hir::ItemKind::Handler { .. }
                | hir::ItemKind::TypeAlias { .. }
                | hir::ItemKind::Const { .. }
                | hir::ItemKind::Static { .. }
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
                if !fn_def.sig.generics.is_empty() || !fn_def.sig.const_generics.is_empty() {
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

        // Drain any errors collected during type lowering (which uses &self).
        self.errors.extend(
            self.type_lowering_errors.borrow_mut().drain(..)
        );

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

        // Create the function type (kept for validation, even though opaque pointers
        // don't require typed function pointers)
        let _fn_type = return_type.fn_type(&param_types, false);

        // Create a type alias (via a named struct that wraps a function pointer)
        // This is primarily for documentation; function pointers in Blood work directly
        let _ptr_type = self.context.ptr_type(AddressSpace::default());

        // For callback resolution during type checking, we don't need to store anything
        // The callback is resolved through the type system

        Ok(())
    }

    /// Get the alignment of an LLVM type in bytes.
    fn get_type_alignment(&self, ty: BasicTypeEnum<'ctx>) -> u32 {
        // Return alignments matching LLVM 14's default x86_64 data layout.
        // LLVM 14 uses 8-byte ABI alignment for i128 (no i128:128 in default layout).
        // We MUST match this because LLVM 14's C API resets the module's data layout
        // to the TargetMachine's default during code emission.
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
            BasicTypeEnum::StructType(st) => {
                // Struct alignment is the max alignment of any field
                let mut max_align: u32 = 1;
                for i in 0..st.count_fields() {
                    if let Some(field_ty) = st.get_field_type_at_index(i) {
                        let field_align = self.get_type_alignment(field_ty);
                        if field_align > max_align {
                            max_align = field_align;
                        }
                    }
                }
                max_align
            }
            BasicTypeEnum::ArrayType(arr_ty) => {
                // Array alignment is the alignment of its element type
                self.get_type_alignment(arr_ty.get_element_type())
            }
            BasicTypeEnum::VectorType(_) => 16,
            BasicTypeEnum::ScalableVectorType(_) => 16,
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

                // Create global variable with linkonce_odr linkage to avoid
                // multiple definition errors when the same static is compiled
                // in multiple object files (e.g., when a module is imported
                // from multiple places in per-definition compilation)
                let global = self.module.add_global(
                    llvm_type,
                    Some(AddressSpace::default()),
                    &item.name,
                );
                global.set_initializer(&init_value);
                global.set_constant(!*mutable); // Only constant if not mutable
                global.set_linkage(inkwell::module::Linkage::LinkOnceODR);

                // Store for later reference
                self.static_globals.insert(*def_id, global);
                self.static_types.insert(*def_id, ty.clone());
            }
        }
        Ok(())
    }

    /// Get the type of a static item.
    pub fn get_static_type(&self, def_id: DefId) -> Option<Type> {
        self.static_types.get(&def_id).cloned()
    }

    /// Get the base type of a place (either from a local or a static).
    pub fn get_place_base_type(&self, place: &crate::mir::Place, body: &crate::mir::MirBody) -> Result<Type, Vec<Diagnostic>> {
        use crate::mir::types::PlaceBase;
        match &place.base {
            PlaceBase::Local(local_id) => {
                let local_info = body.locals.get(local_id.index as usize)
                    .ok_or_else(|| vec![Diagnostic::error(
                        format!("Local _{} not found in MIR body", local_id.index),
                        body.span,
                    )])?;
                Ok(local_info.ty.clone())
            }
            PlaceBase::Static(def_id) => {
                self.get_static_type(*def_id).ok_or_else(|| vec![Diagnostic::error(
                    format!("Static {:?} type not found", def_id),
                    body.span,
                )])
            }
        }
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
            ExprKind::Variant { def_id: _, variant_idx, fields } => {
                // Get the LLVM type for the enum
                let enum_llvm_type = self.lower_type(ty);

                // Handle unit enums (no payload fields) - lowered to just i32 tag
                if enum_llvm_type.is_int_type() {
                    // Unit enum - just return the tag value as a constant
                    let tag = self.context.i32_type().const_int(*variant_idx as u64, false);
                    return Ok(tag.into());
                }

                // Payload enum - create a constant struct with tag + fields
                let enum_struct_type = enum_llvm_type.into_struct_type();

                // Evaluate all field values as constants
                let mut field_values: Vec<inkwell::values::BasicValueEnum<'ctx>> = Vec::new();

                // First field is always the discriminant/tag
                let tag = self.context.i32_type().const_int(*variant_idx as u64, false);
                field_values.push(tag.into());

                // Get variant field types from the enum type
                // The struct type has: [tag, field1, field2, ...]
                // We need to match field expressions to their expected types
                for (i, field_expr) in fields.iter().enumerate() {
                    let field_val = self.evaluate_const_expr_with_env(field_expr, &field_expr.ty, bindings)?;

                    // Check if we need to pad the value to match the struct field type
                    let struct_field_type = enum_struct_type.get_field_type_at_index((i + 1) as u32);
                    if let Some(expected_type) = struct_field_type {
                        let actual_type = field_val.get_type();

                        // If types don't match, we may need padding for union-like representation
                        if actual_type != expected_type {
                            // For now, try to use the value directly if it fits
                            // This handles cases where the enum payload is smaller than the max variant
                            if let (inkwell::values::BasicValueEnum::IntValue(iv), inkwell::types::BasicTypeEnum::IntType(target_int)) = (field_val, expected_type) {
                                let from_bits = iv.get_type().get_bit_width();
                                let to_bits = target_int.get_bit_width();
                                if from_bits < to_bits {
                                    // Zero-extend: extract value and create wider constant
                                    // (const_z_ext removed in LLVM 18)
                                    if let Some(val) = iv.get_zero_extended_constant() {
                                        field_values.push(target_int.const_int(val, false).into());
                                    } else {
                                        field_values.push(field_val);
                                    }
                                } else if from_bits > to_bits {
                                    field_values.push(iv.const_truncate(target_int).into());
                                } else {
                                    field_values.push(field_val);
                                }
                            } else {
                                // For non-integer types or complex cases, use the value as-is
                                // and hope LLVM can handle it (it usually can for compatible types)
                                field_values.push(field_val);
                            }
                        } else {
                            field_values.push(field_val);
                        }
                    } else {
                        field_values.push(field_val);
                    }
                }

                // If we have fewer fields than the struct expects (e.g., Some(x) vs None),
                // pad with zeros/undef for remaining fields
                let num_struct_fields = enum_struct_type.count_fields();
                while field_values.len() < num_struct_fields as usize {
                    let idx = field_values.len() as u32;
                    if let Some(field_type) = enum_struct_type.get_field_type_at_index(idx) {
                        field_values.push(field_type.const_zero());
                    } else {
                        break;
                    }
                }

                // Create the constant struct
                Ok(enum_struct_type.const_named_struct(&field_values).into())
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

        // LLVM 18 removed most const cast APIs. Extract values, convert in Rust,
        // and create new LLVM constants.
        match (value, target_llvm_ty) {
            (inkwell::values::BasicValueEnum::IntValue(int_val), inkwell::types::BasicTypeEnum::IntType(target_int)) => {
                // Integer to integer cast
                let from_bits = int_val.get_type().get_bit_width();
                let to_bits = target_int.get_bit_width();
                if from_bits < to_bits {
                    // Zero-extend: extract value and create wider constant
                    let val = int_val.get_zero_extended_constant().ok_or_else(|| vec![Diagnostic::error(
                        "Integer cast requires a constant value in const context", span,
                    )])?;
                    Ok(target_int.const_int(val, false).into())
                } else if from_bits > to_bits {
                    // Truncate (still available in LLVM 18)
                    Ok(int_val.const_truncate(target_int).into())
                } else {
                    Ok(int_val.into())
                }
            }
            (inkwell::values::BasicValueEnum::IntValue(int_val), inkwell::types::BasicTypeEnum::FloatType(target_float)) => {
                // Integer to float cast: extract int, convert in Rust, create float constant
                let val = int_val.get_sign_extended_constant().ok_or_else(|| vec![Diagnostic::error(
                    "Integer-to-float cast requires a constant value in const context", span,
                )])?;
                Ok(target_float.const_float(val as f64).into())
            }
            (inkwell::values::BasicValueEnum::FloatValue(float_val), inkwell::types::BasicTypeEnum::IntType(target_int)) => {
                // Float to integer cast: extract float, convert in Rust, create int constant
                let (val, _lossy) = float_val.get_constant().ok_or_else(|| vec![Diagnostic::error(
                    "Float-to-integer cast requires a constant value in const context", span,
                )])?;
                Ok(target_int.const_int(val as i64 as u64, true).into())
            }
            (inkwell::values::BasicValueEnum::FloatValue(float_val), inkwell::types::BasicTypeEnum::FloatType(target_float)) => {
                // Float to float cast: extract, create constant with target type
                let (val, _lossy) = float_val.get_constant().ok_or_else(|| vec![Diagnostic::error(
                    "Float cast requires a constant value in const context", span,
                )])?;
                Ok(target_float.const_float(val).into())
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

                // Get pointer to string data (opaque pointer, no cast needed)
                let ptr = global.as_pointer_value();
                let len = self.context.i64_type().const_int(bytes.len() as u64, false);

                // Create str slice struct {ptr, len}
                let str_type = self.context.struct_type(
                    &[self.context.ptr_type(AddressSpace::default()).into(),
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

                // Get pointer (opaque pointer, no cast needed)
                let ptr = global.as_pointer_value();
                let len = self.context.i64_type().const_int(bytes.len() as u64, false);

                // Create byte slice struct {ptr, len}
                let slice_type = self.context.struct_type(
                    &[self.context.ptr_type(AddressSpace::default()).into(),
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
                    // LLVM 18 removed const_neg for floats; compute in Rust and create constant
                    if let Some(val) = float_val.get_constant() {
                        let (neg_val, _) = val;
                        let neg_val = -neg_val;
                        Ok(float_val.get_type().const_float(neg_val).into())
                    } else {
                        Err(vec![Diagnostic::error(
                            "Float negation requires a constant value in const context".to_string(),
                            self.current_span(),
                        )])
                    }
                } else {
                    Err(vec![Diagnostic::error(
                        "Negation on unsupported type in const context".to_string(),
                        self.current_span(),
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
                        self.current_span(),
                    )])
                }
            }
            _ => Err(vec![Diagnostic::error(
                format!("Unary operator {:?} not supported in const context", op),
                self.current_span(),
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
        // LLVM 18 removed many const expr APIs, so we extract values, compute in Rust,
        // and create new constants for operations no longer supported by LLVM.
        if left.is_int_value() && right.is_int_value() {
            let left_int = left.into_int_value();
            let right_int = right.into_int_value();
            let int_type = left_int.get_type();
            let result = match op {
                // These const operations are still available in LLVM 18
                BinOp::Add => left_int.const_add(right_int),
                BinOp::Sub => left_int.const_sub(right_int),
                BinOp::Mul => left_int.const_mul(right_int),
                BinOp::BitXor => left_int.const_xor(right_int),
                BinOp::Shl => left_int.const_shl(right_int),
                BinOp::Eq => left_int.const_int_compare(inkwell::IntPredicate::EQ, right_int),
                BinOp::Ne => left_int.const_int_compare(inkwell::IntPredicate::NE, right_int),
                BinOp::Lt => left_int.const_int_compare(inkwell::IntPredicate::SLT, right_int),
                BinOp::Le => left_int.const_int_compare(inkwell::IntPredicate::SLE, right_int),
                BinOp::Gt => left_int.const_int_compare(inkwell::IntPredicate::SGT, right_int),
                BinOp::Ge => left_int.const_int_compare(inkwell::IntPredicate::SGE, right_int),
                // These const operations were removed in LLVM 15+; compute in Rust
                BinOp::Div | BinOp::Rem | BinOp::BitAnd | BinOp::BitOr | BinOp::Shr => {
                    let lv = left_int.get_sign_extended_constant().ok_or_else(|| vec![Diagnostic::error(
                        "Left operand must be a constant in const context".to_string(), self.current_span(),
                    )])?;
                    let rv = right_int.get_sign_extended_constant().ok_or_else(|| vec![Diagnostic::error(
                        "Right operand must be a constant in const context".to_string(), self.current_span(),
                    )])?;
                    let result_val: i64 = match op {
                        BinOp::Div => if rv != 0 { lv.wrapping_div(rv) } else {
                            return Err(vec![Diagnostic::error("Division by zero in const context", self.current_span())]);
                        },
                        BinOp::Rem => if rv != 0 { lv.wrapping_rem(rv) } else {
                            return Err(vec![Diagnostic::error("Remainder by zero in const context", self.current_span())]);
                        },
                        BinOp::BitAnd => lv & rv,
                        BinOp::BitOr => lv | rv,
                        BinOp::Shr => lv.wrapping_shr(rv as u32),
                        _ => unreachable!(),
                    };
                    int_type.const_int(result_val as u64, true)
                }
                _ => {
                    return Err(vec![Diagnostic::error(
                        format!("Binary operator {:?} not supported in const context", op),
                        self.current_span(),
                    )]);
                }
            };
            return Ok(result.into());
        }

        // Try float operations
        // LLVM 18 removed all const float expr APIs; compute in Rust and create constants
        if left.is_float_value() && right.is_float_value() {
            let left_float = left.into_float_value();
            let right_float = right.into_float_value();
            let float_type = left_float.get_type();

            let lv = left_float.get_constant().ok_or_else(|| vec![Diagnostic::error(
                "Left float operand must be a constant in const context".to_string(), self.current_span(),
            )])?.0;
            let rv = right_float.get_constant().ok_or_else(|| vec![Diagnostic::error(
                "Right float operand must be a constant in const context".to_string(), self.current_span(),
            )])?.0;

            let result_val: f64 = match op {
                BinOp::Add => lv + rv,
                BinOp::Sub => lv - rv,
                BinOp::Mul => lv * rv,
                BinOp::Div => lv / rv,
                _ => {
                    return Err(vec![Diagnostic::error(
                        format!("Float binary operator {:?} not supported in const context", op),
                        self.current_span(),
                    )]);
                }
            };
            return Ok(float_type.const_float(result_val).into());
        }

        Err(vec![Diagnostic::error(
            "Binary operation on unsupported types in const context".to_string(),
            self.current_span(),
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
        // Build module-qualified paths for stable linker symbols
        self.def_paths = build_def_paths(hir_crate);

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

        // Drain any errors collected during type lowering (which uses &self).
        self.errors.extend(
            self.type_lowering_errors.borrow_mut().drain(..)
        );

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
                self.current_span(),
            )]
        })?;

        self.current_fn = Some(fn_value);
        self.locals.clear();
        self.local_types.clear();
        self.local_generations.clear();
        self.persistent_slot_ids.clear();
        self.loop_stack.clear();

        // Create entry block
        let entry = self.context.append_basic_block(fn_value, "entry");
        self.builder.position_at_end(entry);

        // Allocate space for parameters
        for (i, param) in body.params().enumerate() {
            let llvm_type = self.lower_type(&param.ty);
            let alloca = self.builder.build_alloca(llvm_type, &param.name.clone().unwrap_or_else(|| format!("arg{}", i)))
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

            // Store parameter value
            let param_value = fn_value.get_nth_param(i as u32)
                .ok_or_else(|| vec![Diagnostic::error("Parameter not found", self.current_span())])?;
            self.builder.build_store(alloca, param_value)
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;

            self.locals.insert(param.id, alloca);
            self.local_types.insert(param.id, llvm_type);
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
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
            } else {
                self.builder.build_return(None)
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
            }
        } else if let Some(value) = result {
            self.builder.build_return(Some(&value))
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
        } else if is_main {
            // main must return i32 for C runtime - return 0 on success
            let zero = self.context.i32_type().const_int(0, false);
            self.builder.build_return(Some(&zero))
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
        } else {
            self.builder.build_return(None)
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), self.current_span())])?;
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
                self.current_span(),
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
        if !fn_def.sig.generics.is_empty() || !fn_def.sig.const_generics.is_empty() {
            return Ok(());
        }

        // Also skip functions with type parameters in the signature (like trait default methods)
        // These have an implicit `Self` type parameter that isn't in `generics` but appears
        // as `TypeKind::Param` in the signature.
        if self.sig_contains_type_params(&fn_def.sig) {
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
            // Mangle name with DefId and parameter types to ensure uniqueness
            let llvm_name = self.mangle_function_name(def_id, name, &fn_def.sig);
            let fn_type = self.fn_type_from_sig(&fn_def.sig);
            (llvm_name, fn_type)
        };

        let fn_value = self.module.add_function(&llvm_name, fn_type, None);
        self.functions.insert(def_id, fn_value);
        Ok(())
    }

    /// Set a function's linkage to WeakODR after compiling its body.
    /// This allows the linker to merge duplicate definitions when the same
    /// function is compiled into multiple object files.
    ///
    /// We use WeakODR instead of LinkOnceODR because LinkOnceODR functions
    /// can be stripped by LLVM optimization when they have no callers in the
    /// current module. WeakODR preserves the function for linking.
    ///
    /// Skips `blood_main` which must have External linkage for the runtime.
    pub fn set_function_weak_odr(&mut self, def_id: DefId) {
        if let Some(fn_value) = self.functions.get(&def_id) {
            let name = fn_value.get_name().to_string_lossy();
            // blood_main must remain External for the runtime to find it
            if name == "blood_main" {
                return;
            }
            use inkwell::module::Linkage;
            fn_value.set_linkage(Linkage::WeakODR);
        }
    }

    /// Mangle a function name with module-qualified path and parameter types to ensure uniqueness.
    ///
    /// This generates unique, stable symbol names for all functions using
    /// module-qualified paths instead of DefId indices. Symbols remain stable
    /// when unrelated definitions are added or removed.
    ///
    /// For example: `add(i32, i32)` in module `mod_a` becomes `blood$mod_a$add$i32$i32`
    fn mangle_function_name(&self, def_id: DefId, name: &str, sig: &hir::FnSig) -> String {
        let qualified_path = self.def_paths.get(&def_id)
            .map(|p| p.replace("::", "$"))
            .unwrap_or_else(|| name.to_string());
        if sig.inputs.is_empty() {
            format!("blood${}", qualified_path)
        } else {
            let param_mangles: Vec<String> = sig.inputs.iter()
                .map(|ty| self.mangle_type(ty))
                .collect();
            format!("blood${}${}", qualified_path, param_mangles.join("$"))
        }
    }

    /// Check if a function signature contains any type parameters.
    ///
    /// This is used to detect trait default methods which have an implicit `Self`
    /// type parameter that appears as `TypeKind::Param` in the signature but isn't
    /// listed in the explicit `generics` field.
    fn sig_contains_type_params(&self, sig: &hir::FnSig) -> bool {
        fn type_contains_param(ty: &Type) -> bool {
            match ty.kind() {
                TypeKind::Param(_) => true,
                TypeKind::Tuple(elems) => elems.iter().any(type_contains_param),
                TypeKind::Array { element, .. } => type_contains_param(element),
                TypeKind::Slice { element } => type_contains_param(element),
                TypeKind::Ref { inner, .. } => type_contains_param(inner),
                TypeKind::Ptr { inner, .. } => type_contains_param(inner),
                TypeKind::Adt { args, .. } => args.iter().any(type_contains_param),
                TypeKind::Fn { params, ret, .. } => {
                    params.iter().any(type_contains_param) || type_contains_param(ret)
                }
                _ => false,
            }
        }

        sig.inputs.iter().any(type_contains_param) || type_contains_param(&sig.output)
    }

    /// Generate a mangled name for a type.
    ///
    /// Uses module-qualified paths for ADT types instead of raw DefId indices,
    /// ensuring stable linker symbols across recompilations.
    fn mangle_type(&self, ty: &Type) -> String {
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
                    .map(|ty| self.mangle_type(ty))
                    .collect();
                format!("T{}", elem_mangles.join("_"))
            }
            TypeKind::Array { element, size } => {
                format!("A{}_{}", size, self.mangle_type(element))
            }
            TypeKind::Slice { element } => {
                format!("S{}", self.mangle_type(element))
            }
            TypeKind::Ref { inner, mutable } => {
                if *mutable {
                    format!("Rm{}", self.mangle_type(inner))
                } else {
                    format!("R{}", self.mangle_type(inner))
                }
            }
            TypeKind::Ptr { inner, mutable } => {
                if *mutable {
                    format!("Pm{}", self.mangle_type(inner))
                } else {
                    format!("P{}", self.mangle_type(inner))
                }
            }
            TypeKind::Adt { def_id, args } => {
                let qualified_name = self.def_paths.get(def_id)
                    .cloned()
                    .unwrap_or_else(|| format!("D{}", def_id.index()));
                if args.is_empty() {
                    format!("D_{}", qualified_name)
                } else {
                    let arg_mangles: Vec<String> = args.iter()
                        .map(|ty| self.mangle_type(ty))
                        .collect();
                    format!("D_{}_{}", qualified_name, arg_mangles.join("_"))
                }
            }
            TypeKind::Fn { params, ret, .. } => {
                let param_mangles: Vec<String> = params.iter()
                    .map(|ty| self.mangle_type(ty))
                    .collect();
                format!("F{}_{}", param_mangles.join("_"), self.mangle_type(ret))
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
                format!("{}{}", prefix, self.mangle_type(inner))
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
        let i8_ptr_ty = self.context.ptr_type(AddressSpace::default());

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
        self.monomorphize_function_with_const_args(generic_def_id, concrete_params, concrete_ret, &[])
    }

    /// Monomorphize a function with explicit const generic arguments.
    /// This is used when const args are provided via turbofish syntax like `fn::<3, 4>()`.
    pub fn monomorphize_function_with_const_args(
        &mut self,
        generic_def_id: DefId,
        concrete_params: &[Type],
        concrete_ret: &Type,
        explicit_const_args: &[(hir::ConstParamId, hir::ConstValue)],
    ) -> Option<inkwell::values::FunctionValue<'ctx>> {
        // First, try to get from regular generic functions
        if let Some((fn_def, _hir_body)) = self.generic_fn_defs.get(&generic_def_id).cloned() {
            return self.monomorphize_regular_generic(
                generic_def_id,
                &fn_def,
                concrete_params,
                concrete_ret,
                explicit_const_args,
            );
        }

        // Second, try trait default methods
        if let Some((trait_id, sig, _body)) = self.trait_default_fn_defs.get(&generic_def_id).cloned() {
            return self.monomorphize_trait_default_method(
                generic_def_id,
                trait_id,
                &sig,
                concrete_params,
                concrete_ret,
            );
        }

        None
    }

    /// Monomorphize a regular generic function.
    fn monomorphize_regular_generic(
        &mut self,
        generic_def_id: DefId,
        fn_def: &hir::FnDef,
        concrete_params: &[Type],
        concrete_ret: &Type,
        explicit_const_args: &[(hir::ConstParamId, hir::ConstValue)],
    ) -> Option<inkwell::values::FunctionValue<'ctx>> {
        // Infer type and const arguments by unifying generic signature with concrete types
        // This is needed for higher-order generics like `apply<T, R>(f: fn(T) -> R, x: T) -> R`
        // where concrete_params = [fn(i32) -> i32, i32] but we need type_args = [i32, i32]
        let mut infer_result = infer_type_and_const_args(
            &fn_def.sig.inputs,
            concrete_params,
            &fn_def.sig.output,
            concrete_ret,
        );

        // Merge explicit const args into the inferred const_subst.
        // Explicit args from turbofish take precedence.
        for (param_id, value) in explicit_const_args {
            infer_result.const_subst.insert(*param_id, value.clone());
        }

        // Build type args list from inferred substitution (in order of fn_def.sig.generics)
        let type_args: Vec<Type> = fn_def.sig.generics.iter()
            .filter_map(|tyvar_id| infer_result.type_subst.get(tyvar_id).cloned())
            .collect();

        // Critical: check if type inference succeeded
        if type_args.len() != fn_def.sig.generics.len() {
            return None;
        }

        // Build const args for cache key
        let mut const_args_sorted: Vec<_> = infer_result.const_subst.iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect();
        const_args_sorted.sort_by_key(|(k, _)| k.0);

        // Check if already monomorphized (including const args in cache key)
        let cache_key = (generic_def_id, type_args.clone(), const_args_sorted.clone());
        if let Some(&mono_def_id) = self.mono_cache.get(&cache_key) {
            return self.functions.get(&mono_def_id).copied();
        }

        // Get the generic function's MIR body
        let mir_body = match self.generic_mir_bodies.get(&generic_def_id) {
            Some(b) => b.clone(),
            None => return None,
        };

        // Generate unique DefId for monomorphized version
        let mono_def_id = DefId::new(0xFFFE_0000 + self.mono_counter);
        self.mono_counter += 1;

        // Clone and substitute types and const params in MIR body
        let subst_ctx = SubstContext {
            type_subst: infer_result.type_subst,
            const_subst: infer_result.const_subst,
        };
        let mut mono_mir = substitute_mir_types_with_ctx(&mir_body, &subst_ctx);

        // Monomorphize closures referenced in this function.
        // Closures that capture generic-typed variables need their MIR bodies
        // to also be type-substituted with the same substitution map.
        let closure_remap = self.monomorphize_closures_recursive(&mono_mir, &subst_ctx.type_subst);
        remap_closure_def_ids(&mut mono_mir, &closure_remap);

        // Build mangled name for monomorphized function.
        // Use only generic_def_id and concrete types (not mono_def_id) so the name
        // is stable across compilation units. Combined with LinkOnceODR linkage,
        // this allows the linker to merge identical instantiations.
        let param_mangles: Vec<String> = concrete_params.iter()
            .map(|ty| self.mangle_type(ty))
            .collect();
        let generic_fn_path = self.def_paths.get(&generic_def_id)
            .map(|p| p.replace("::", "$"))
            .unwrap_or_else(|| format!("{}", generic_def_id.index()));
        let llvm_name = if param_mangles.is_empty() {
            format!("blood_mono${}", generic_fn_path)
        } else {
            format!("blood_mono${}${}", generic_fn_path, param_mangles.join("$"))
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

        // Declare the monomorphized function with LinkOnceODR linkage.
        // This allows the linker to merge identical instantiations when
        // multiple closures/functions call the same generic with the same types.
        use inkwell::module::Linkage;
        let fn_value = self.module.add_function(&llvm_name, fn_type, Some(Linkage::LinkOnceODR));
        self.functions.insert(mono_def_id, fn_value);

        // Cache the monomorphization
        self.mono_cache.insert(cache_key, mono_def_id);

        // Save current function state before compiling monomorphized function
        // This is necessary because compile_mir_body modifies these fields
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_local_types = std::mem::take(&mut self.local_types);
        let saved_local_generations = std::mem::take(&mut self.local_generations);
        let saved_persistent_slot_ids = std::mem::take(&mut self.persistent_slot_ids);
        let saved_current_fn = self.current_fn.take();
        let saved_current_fn_def_id = self.current_fn_def_id.take();
        let saved_insert_block = self.builder.get_insert_block();

        // Run escape analysis on the monomorphized MIR body
        // This is critical for performance - without it, all locals would use Region tier
        let mut escape_analyzer = EscapeAnalyzer::new();
        let escape_results = escape_analyzer.analyze(&mono_mir);

        // Compile the monomorphized MIR body with escape analysis results
        use crate::codegen::mir_codegen::MirCodegen;
        let mut result = self.compile_mir_body(mono_def_id, &mono_mir, Some(&escape_results));

        // Compile any monomorphized closures that were created during this monomorphization.
        // These closures were declared but need their bodies compiled.
        if result.is_ok() {
            // Take the pending closure MIRs to avoid borrowing issues
            let pending_closures: Vec<(DefId, MirBody)> =
                std::mem::take(&mut self.mono_closure_mirs).into_iter().collect();

            for (closure_def_id, closure_mir) in pending_closures {
                // Clear state for each closure compilation
                self.locals.clear();
                self.local_types.clear();
                self.local_generations.clear();
                self.persistent_slot_ids.clear();

                // Run escape analysis on the closure
                let mut analyzer = EscapeAnalyzer::new();
                let closure_escape = analyzer.analyze(&closure_mir);

                // Compile the closure
                if let Err(e) = self.compile_mir_body(closure_def_id, &closure_mir, Some(&closure_escape)) {
                    result = Err(e);
                    break;
                }
            }
        }

        // Restore previous function state
        self.locals = saved_locals;
        self.local_types = saved_local_types;
        self.local_generations = saved_local_generations;
        self.persistent_slot_ids = saved_persistent_slot_ids;
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

    /// Monomorphize a trait default method for a concrete `Self` type.
    ///
    /// Trait default methods have an implicit `Self` type parameter that needs to be
    /// substituted with the concrete receiver type when the method is called.
    fn monomorphize_trait_default_method(
        &mut self,
        method_def_id: DefId,
        _trait_id: DefId,
        sig: &hir::FnSig,
        concrete_params: &[Type],
        concrete_ret: &Type,
    ) -> Option<inkwell::values::FunctionValue<'ctx>> {
        // The first parameter type is `Self`. Extract the TyVarId for Self from the signature.
        // The concrete receiver type (first element of concrete_params) tells us what Self should be.
        let self_tyvar_id = sig.inputs.first().and_then(|ty| {
            if let hir::TypeKind::Param(id) = ty.kind() {
                Some(*id)
            } else {
                None
            }
        });

        let concrete_self = concrete_params.first()?.clone();

        // Build the type substitution: Self -> concrete receiver type
        let mut type_subst: HashMap<TyVarId, Type> = HashMap::new();
        if let Some(self_id) = self_tyvar_id {
            type_subst.insert(self_id, concrete_self.clone());
        }

        // Build cache key for this monomorphization
        let type_args = vec![concrete_self.clone()];
        let cache_key = (method_def_id, type_args.clone(), vec![]);
        if let Some(&mono_def_id) = self.mono_cache.get(&cache_key) {
            return self.functions.get(&mono_def_id).copied();
        }

        // Get the trait default method's MIR body
        let mir_body = match self.generic_mir_bodies.get(&method_def_id) {
            Some(b) => b.clone(),
            None => return None,
        };

        // Generate unique DefId for monomorphized version
        let mono_def_id = DefId::new(0xFFFE_0000 + self.mono_counter);
        self.mono_counter += 1;

        // Clone and substitute Self type in MIR body
        let subst_ctx = SubstContext {
            type_subst: type_subst.clone(),
            const_subst: HashMap::new(),
        };
        let mut mono_mir = substitute_mir_types_with_ctx(&mir_body, &subst_ctx);

        // Monomorphize closures referenced in this method
        let closure_remap = self.monomorphize_closures_recursive(&mono_mir, &type_subst);
        remap_closure_def_ids(&mut mono_mir, &closure_remap);

        // Build mangled name for monomorphized function
        let param_mangles: Vec<String> = concrete_params.iter()
            .map(|ty| self.mangle_type(ty))
            .collect();
        let method_path = self.def_paths.get(&method_def_id)
            .map(|p| p.replace("::", "$"))
            .unwrap_or_else(|| format!("trait_default_{}", method_def_id.index()));
        let llvm_name = format!("blood_mono${}${}", method_path, param_mangles.join("$"));

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

        // Declare the monomorphized function with LinkOnceODR linkage
        use inkwell::module::Linkage;
        let fn_value = self.module.add_function(&llvm_name, fn_type, Some(Linkage::LinkOnceODR));
        self.functions.insert(mono_def_id, fn_value);

        // Cache the monomorphization
        self.mono_cache.insert(cache_key, mono_def_id);

        // Save current function state
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_local_types = std::mem::take(&mut self.local_types);
        let saved_local_generations = std::mem::take(&mut self.local_generations);
        let saved_persistent_slot_ids = std::mem::take(&mut self.persistent_slot_ids);
        let saved_current_fn = self.current_fn.take();
        let saved_current_fn_def_id = self.current_fn_def_id.take();
        let saved_insert_block = self.builder.get_insert_block();

        // Run escape analysis on the monomorphized MIR body
        let mut escape_analyzer = EscapeAnalyzer::new();
        let escape_results = escape_analyzer.analyze(&mono_mir);

        // Compile the monomorphized MIR body
        use crate::codegen::mir_codegen::MirCodegen;
        let mut result = self.compile_mir_body(mono_def_id, &mono_mir, Some(&escape_results));

        // Compile any monomorphized closures
        if result.is_ok() {
            let pending_closures: Vec<(DefId, MirBody)> =
                std::mem::take(&mut self.mono_closure_mirs).into_iter().collect();

            for (closure_def_id, closure_mir) in pending_closures {
                self.locals.clear();
                self.local_types.clear();
                self.local_generations.clear();
                self.persistent_slot_ids.clear();

                let mut analyzer = EscapeAnalyzer::new();
                let closure_escape = analyzer.analyze(&closure_mir);

                if let Err(e) = self.compile_mir_body(closure_def_id, &closure_mir, Some(&closure_escape)) {
                    result = Err(e);
                    break;
                }
            }
        }

        // Restore previous function state
        self.locals = saved_locals;
        self.local_types = saved_local_types;
        self.local_generations = saved_local_generations;
        self.persistent_slot_ids = saved_persistent_slot_ids;
        self.current_fn = saved_current_fn;
        self.current_fn_def_id = saved_current_fn_def_id;

        if let Some(insert_bb) = saved_insert_block {
            self.builder.position_at_end(insert_bb);
        }

        if result.is_err() {
            return None;
        }

        Some(fn_value)
    }

    /// Recursively monomorphize all closures in a MIR body.
    ///
    /// When monomorphizing a generic function, closures defined inside it may capture
    /// variables with generic types. These closures need their MIR bodies type-substituted
    /// with the same substitution map as the enclosing function.
    ///
    /// This method:
    /// 1. Finds all closure references in the MIR body
    /// 2. For each closure, substitutes types in its MIR body
    /// 3. Recursively processes nested closures
    /// 4. Declares the monomorphized closure functions
    /// 5. Returns a mapping from old closure DefIds to new ones
    fn monomorphize_closures_recursive(
        &mut self,
        mir: &MirBody,
        subst: &HashMap<TyVarId, Type>,
    ) -> HashMap<DefId, DefId> {
        let mut remap = HashMap::new();

        // Collect all closure DefIds in this MIR body
        let closure_ids = collect_closure_def_ids(mir);

        for old_closure_id in closure_ids {
            // Skip if already remapped (handles diamond dependencies)
            if remap.contains_key(&old_closure_id) {
                continue;
            }

            // Get the closure's original MIR body
            let closure_mir = match self.generic_mir_bodies.get(&old_closure_id) {
                Some(m) => m.clone(),
                None => continue, // Closure not in generic bodies, skip
            };

            // Substitute types in the closure MIR
            let mut mono_closure_mir = substitute_mir_types(&closure_mir, subst);

            // Recursively handle nested closures
            let nested_remap = self.monomorphize_closures_recursive(&mono_closure_mir, subst);
            remap_closure_def_ids(&mut mono_closure_mir, &nested_remap);

            // Merge nested remappings into our remap
            remap.extend(nested_remap);

            // Generate new DefId for monomorphized closure.
            // Use 0xFFFF_8000+ so they're still detected as closures (>= 0xFFFF_0000).
            let new_closure_id = DefId::new(0xFFFF_8000 + self.mono_closure_counter);
            self.mono_closure_counter += 1;

            // Declare the monomorphized closure function
            self.declare_closure_from_mir(new_closure_id, &mono_closure_mir);

            // Store the monomorphized closure MIR for later compilation
            self.mono_closure_mirs.insert(new_closure_id, mono_closure_mir);

            // Record the remapping
            remap.insert(old_closure_id, new_closure_id);
        }

        remap
    }

    /// Get or create a wrapper function for a plain function used as a fn() pointer.
    ///
    /// When a plain function is converted to a fat pointer { fn_ptr, env_ptr },
    /// we need a wrapper that accepts (env_ptr, params...) and forwards to the
    /// original function (ignoring env_ptr). This is needed because:
    ///
    /// 1. Plain functions are compiled with signature (params...) -> ret
    /// 2. fn() pointers use fat pointer calling convention: (env_ptr, params...) -> ret
    /// 3. Without a wrapper, the first argument would be mistaken for env_ptr
    ///
    /// The wrapper is cached to avoid creating duplicates.
    pub fn get_or_create_fn_ptr_wrapper(
        &mut self,
        def_id: DefId,
    ) -> Option<FunctionValue<'ctx>> {
        // Check cache first
        if let Some(&wrapper) = self.fn_ptr_wrappers.get(&def_id) {
            return Some(wrapper);
        }

        // Get the original function
        let original_fn = *self.functions.get(&def_id)?;
        let original_fn_type = original_fn.get_type();
        let original_param_count = original_fn_type.count_param_types();

        // Build wrapper function type: (i8* env_ptr, params...) -> ret
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let mut wrapper_param_types: Vec<inkwell::types::BasicMetadataTypeEnum> =
            Vec::with_capacity(original_param_count as usize + 1);

        // First param is env_ptr (opaque ptr)
        wrapper_param_types.push(ptr_type.into());

        // Add original function's params
        for i in 0..original_param_count {
            let param_type = original_fn_type.get_param_types()[i as usize];
            wrapper_param_types.push(param_type.into());
        }

        let wrapper_fn_type = if let Some(ret_type) = original_fn_type.get_return_type() {
            ret_type.fn_type(&wrapper_param_types, false)
        } else {
            self.context.void_type().fn_type(&wrapper_param_types, false)
        };

        // Create the wrapper function with a unique name.
        // Use LinkOnceODR linkage so the linker can merge identical wrappers
        // when the same function is converted to fnptr in multiple compilation units.
        let original_name = original_fn.get_name().to_str().unwrap_or("unknown");
        let wrapper_name = format!("{}$fnptr", original_name);
        use inkwell::module::Linkage;
        let wrapper_fn = self.module.add_function(&wrapper_name, wrapper_fn_type, Some(Linkage::LinkOnceODR));

        // Save current builder position (we're in the middle of compiling another function)
        let saved_insert_block = self.builder.get_insert_block();

        // Build the wrapper body: ignore env_ptr (param 0), forward params 1..N to original
        let entry = self.context.append_basic_block(wrapper_fn, "entry");
        self.builder.position_at_end(entry);

        // Collect arguments to forward (skip env_ptr)
        let mut args: Vec<inkwell::values::BasicMetadataValueEnum> =
            Vec::with_capacity(original_param_count as usize);
        for i in 1..=original_param_count {
            // SAFETY: wrapper_fn has original_param_count+1 params (env_ptr + original params)
            // so params 1..=original_param_count are valid indices
            let param = wrapper_fn.get_nth_param(i)
                .expect("BUG: wrapper function should have env_ptr + original params");
            args.push(param.into());
        }

        // Call the original function
        let call_result = self.builder.build_call(original_fn, &args, "forward")
            .ok()?;

        // Return the result
        if original_fn_type.get_return_type().is_some() {
            let ret_val = call_result.try_as_basic_value().basic()?;
            self.builder.build_return(Some(&ret_val)).ok()?;
        } else {
            self.builder.build_return(None).ok()?;
        }

        // Restore builder position to the original function
        if let Some(insert_block) = saved_insert_block {
            self.builder.position_at_end(insert_block);
        }

        // Cache and return
        self.fn_ptr_wrappers.insert(def_id, wrapper_fn);
        Some(wrapper_fn)
    }

    /// Declare runtime support functions.
    pub(super) fn declare_runtime_functions(&mut self) {
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();
        let void_type = self.context.void_type();
        let i8_ptr_type = self.context.ptr_type(AddressSpace::default());
        let i64_ptr_type = self.context.ptr_type(AddressSpace::default());

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

        // debug_vec_index(i64, i64) -> void - for debugging VecIndex GEP
        let debug_vec_index_type = void_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        self.module.add_function("debug_vec_index", debug_vec_index_type, None);

        // debug_vec_ptrs(*i8, *i8) -> void - for debugging VecIndex pointers
        let debug_vec_ptrs_type = void_type.fn_type(&[i8_ptr_type.into(), i8_ptr_type.into()], false);
        self.module.add_function("debug_vec_ptrs", debug_vec_ptrs_type, None);

        // debug_read_enum_at(*i8) -> void - read and print enum at pointer
        let debug_read_enum_type = void_type.fn_type(&[i8_ptr_type.into()], false);
        self.module.add_function("debug_read_enum_at", debug_read_enum_type, None);

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

        // str_len_usize({*i8, i64}*) -> i64 - get string length as usize (takes pointer for method call semantics)
        let str_ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
        let str_len_usize_type = i64_type.fn_type(&[str_ptr_type.into()], false);
        self.module.add_function("str_len_usize", str_len_usize_type, None);

        // str_eq({*i8, i64}, {*i8, i64}) -> i1 - compare strings
        let bool_type = self.context.bool_type();
        let str_eq_type = bool_type.fn_type(&[str_slice_type.into(), str_slice_type.into()], false);
        self.module.add_function("str_eq", str_eq_type, None);

        // blood_str_concat({*i8, i64}, {*i8, i64}) -> {*i8, i64} - concatenate strings
        let str_concat_type = str_slice_type.fn_type(&[str_slice_type.into(), str_slice_type.into()], false);
        self.module.add_function("blood_str_concat", str_concat_type, None);

        // Option<char> type: { i32 tag, i32 value }
        // tag=0 is None, tag=1 is Some(char)
        let option_char_type = self.context.struct_type(&[i32_type.into(), i32_type.into()], false);

        // str_char_at({*i8, i64}*, i64) -> {i32, i32} - get char at byte index (takes pointer for method call semantics)
        let str_char_at_type = option_char_type.fn_type(&[str_ptr_type.into(), i64_type.into()], false);
        self.module.add_function("str_char_at", str_char_at_type, None);

        // str_char_at_index({*i8, i64}*, i64) -> {i32, i32} - get char at character index
        self.module.add_function("str_char_at_index", str_char_at_type, None);

        // string_char_at({*i8, i64, i64}*, i64) -> {i32, i32} - get char at byte index from String
        // String layout starts the same as BloodStr, so we can use str_ptr_type for the pointer
        self.module.add_function("string_char_at", str_char_at_type, None);

        // str_as_bytes({*i8, i64}*) -> {*i8, i64} - convert str to byte slice (essentially identity)
        let str_as_bytes_type = str_slice_type.fn_type(&[str_ptr_type.into()], false);
        self.module.add_function("str_as_bytes", str_as_bytes_type, None);

        // string_as_bytes({*i8, i64, i64}*) -> {*i8, i64} - convert String to byte slice
        // String layout starts the same as BloodStr, so we can use str_ptr_type for the pointer
        self.module.add_function("string_as_bytes", str_as_bytes_type, None);

        // string_new(out: *void) -> void - create an empty String at the output location
        let string_new_type = void_type.fn_type(&[i8_ptr_type.into()], false);
        self.module.add_function("string_new", string_new_type, None);

        // string_as_str(s: *void) -> {*i8, i64} - get str slice from String
        let string_as_str_type = str_slice_type.fn_type(&[i8_ptr_type.into()], false);
        self.module.add_function("string_as_str", string_as_str_type, None);

        // string_len(s: *void) -> i64 - get byte length of String
        let string_len_type = i64_type.fn_type(&[i8_ptr_type.into()], false);
        self.module.add_function("string_len", string_len_type, None);

        // string_is_empty(s: *void) -> i32 - check if String is empty
        let string_is_empty_type = i32_type.fn_type(&[i8_ptr_type.into()], false);
        self.module.add_function("string_is_empty", string_is_empty_type, None);

        // string_push(s: *void, ch: i32) -> void - push a character to String
        let string_push_type = void_type.fn_type(&[i8_ptr_type.into(), i32_type.into()], false);
        self.module.add_function("string_push", string_push_type, None);

        // string_push_str(s: *void, other: {*i8, i64}*) -> void - push str to String
        let string_push_str_type = void_type.fn_type(&[i8_ptr_type.into(), str_ptr_type.into()], false);
        self.module.add_function("string_push_str", string_push_str_type, None);

        // string_clear(s: *void) -> void - clear String contents
        let string_clear_type = void_type.fn_type(&[i8_ptr_type.into()], false);
        self.module.add_function("string_clear", string_clear_type, None);

        // str_to_string(s: *{*i8, i64}, out: *void) -> void - convert str to owned String
        let str_to_string_type = void_type.fn_type(&[str_ptr_type.into(), i8_ptr_type.into()], false);
        self.module.add_function("str_to_string", str_to_string_type, None);

        // str_len_chars({*i8, i64}*) -> i64 - count UTF-8 characters (not bytes)
        let str_len_chars_type = i64_type.fn_type(&[str_ptr_type.into()], false);
        self.module.add_function("str_len_chars", str_len_chars_type, None);

        // string_len_chars({*i8, i64, i64}*) -> i64 - count UTF-8 characters (not bytes)
        // String layout starts the same as BloodStr, so we can use str_ptr_type for the pointer
        self.module.add_function("string_len_chars", str_len_chars_type, None);

        // string_contains(s: {*i8, i64}*, pattern: {*i8, i64}*) -> i32
        let string_contains_type = i32_type.fn_type(&[str_ptr_type.into(), str_ptr_type.into()], false);
        self.module.add_function("string_contains", string_contains_type, None);

        // string_starts_with(s: {*i8, i64}*, prefix: {*i8, i64}*) -> i32
        let string_starts_with_type = i32_type.fn_type(&[str_ptr_type.into(), str_ptr_type.into()], false);
        self.module.add_function("string_starts_with", string_starts_with_type, None);

        // string_ends_with(s: {*i8, i64}*, suffix: {*i8, i64}*) -> i32
        let string_ends_with_type = i32_type.fn_type(&[str_ptr_type.into(), str_ptr_type.into()], false);
        self.module.add_function("string_ends_with", string_ends_with_type, None);

        // string_find(s: {*i8, i64}*, pattern: {*i8, i64}*, out: *void) -> void
        let string_find_type = void_type.fn_type(&[str_ptr_type.into(), str_ptr_type.into(), i8_ptr_type.into()], false);
        self.module.add_function("string_find", string_find_type, None);

        // string_rfind(s: {*i8, i64}*, pattern: {*i8, i64}*, out: *void) -> void
        let string_rfind_type = void_type.fn_type(&[str_ptr_type.into(), str_ptr_type.into(), i8_ptr_type.into()], false);
        self.module.add_function("string_rfind", string_rfind_type, None);

        // string_trim(s: {*i8, i64}*) -> {*i8, i64}
        let string_trim_type = str_slice_type.fn_type(&[str_ptr_type.into()], false);
        self.module.add_function("string_trim", string_trim_type, None);

        // string_trim_start(s: {*i8, i64}*) -> {*i8, i64}
        let string_trim_start_type = str_slice_type.fn_type(&[str_ptr_type.into()], false);
        self.module.add_function("string_trim_start", string_trim_start_type, None);

        // string_trim_end(s: {*i8, i64}*) -> {*i8, i64}
        let string_trim_end_type = str_slice_type.fn_type(&[str_ptr_type.into()], false);
        self.module.add_function("string_trim_end", string_trim_end_type, None);

        // string_substring(s: *void, start: i64, end: i64, out: *void) -> void
        // Extracts a substring from byte indices [start, end) and writes to output String
        let string_substring_type = void_type.fn_type(
            &[i8_ptr_type.into(), i64_type.into(), i64_type.into(), i8_ptr_type.into()],
            false,
        );
        self.module.add_function("string_substring", string_substring_type, None);

        // int_to_string(i32) -> {*i8, i64} - convert integer to string
        let int_to_string_type = str_slice_type.fn_type(&[i32_type.into()], false);
        self.module.add_function("int_to_string", int_to_string_type, None);

        // bool_to_string(i1) -> {*i8, i64} - convert boolean to string
        let bool_type = self.context.bool_type();
        let bool_to_string_type = str_slice_type.fn_type(&[bool_type.into()], false);
        self.module.add_function("bool_to_string", bool_to_string_type, None);

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

        // i8_to_string(i8) -> {*i8, i64} - convert i8 to string
        let i8_type = self.context.i8_type();
        let i8_to_string_type = str_slice_type.fn_type(&[i8_type.into()], false);
        self.module.add_function("i8_to_string", i8_to_string_type, None);

        // i16_to_string(i16) -> {*i8, i64} - convert i16 to string
        let i16_type = self.context.i16_type();
        let i16_to_string_type = str_slice_type.fn_type(&[i16_type.into()], false);
        self.module.add_function("i16_to_string", i16_to_string_type, None);

        // i128_to_string(i128) -> {*i8, i64} - convert i128 to string
        let i128_type = self.context.i128_type();
        let i128_to_string_type = str_slice_type.fn_type(&[i128_type.into()], false);
        self.module.add_function("i128_to_string", i128_to_string_type, None);

        // u8_to_string(u8) -> {*i8, i64} - convert u8 to string
        let u8_to_string_type = str_slice_type.fn_type(&[i8_type.into()], false); // u8 has same LLVM type as i8
        self.module.add_function("u8_to_string", u8_to_string_type, None);

        // u16_to_string(u16) -> {*i8, i64} - convert u16 to string
        let u16_to_string_type = str_slice_type.fn_type(&[i16_type.into()], false); // u16 has same LLVM type as i16
        self.module.add_function("u16_to_string", u16_to_string_type, None);

        // u32_to_string(u32) -> {*i8, i64} - convert u32 to string
        let u32_to_string_type = str_slice_type.fn_type(&[i32_type.into()], false); // u32 has same LLVM type as i32
        self.module.add_function("u32_to_string", u32_to_string_type, None);

        // u128_to_string(u128) -> {*i8, i64} - convert u128 to string
        let u128_to_string_type = str_slice_type.fn_type(&[i128_type.into()], false); // u128 has same LLVM type as i128
        self.module.add_function("u128_to_string", u128_to_string_type, None);

        // i32_to_i64(i32) -> i64 - widen i32 to i64
        let i32_to_i64_type = i64_type.fn_type(&[i32_type.into()], false);
        self.module.add_function("i32_to_i64", i32_to_i64_type, None);

        // i64_to_i32(i64) -> i32 - truncate i64 to i32
        let i64_to_i32_type = i32_type.fn_type(&[i64_type.into()], false);
        self.module.add_function("i64_to_i32", i64_to_i32_type, None);

        // char_from_u32(u32) -> u32 (char as u32) - convert code point to char
        let char_from_u32_type = i32_type.fn_type(&[i32_type.into()], false);
        self.module.add_function("char_from_u32", char_from_u32_type, None);

        // parse_f64(str) -> f64 - parse string as f64, returns 0.0 on failure
        let f64_ty = self.context.f64_type();
        let parse_f64_type = f64_ty.fn_type(&[str_slice_type.into()], false);
        self.module.add_function("parse_f64", parse_f64_type, None);

        // parse_i64_radix(str, u32) -> i64 - parse string as i64 with radix, returns 0 on failure
        let parse_i64_radix_type = i64_type.fn_type(&[str_slice_type.into(), i32_type.into()], false);
        self.module.add_function("parse_i64_radix", parse_i64_radix_type, None);

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

        // blood_assert_eq_bool(i1, i1) -> void - assert two bools are equal
        let bool_type = self.context.bool_type();
        let assert_eq_bool_type = void_type.fn_type(&[bool_type.into(), bool_type.into()], false);
        self.module.add_function("blood_assert_eq_bool", assert_eq_bool_type, None);

        // print_char(i32) -> void
        self.module.add_function("print_char", print_int_type, None);

        // println_char(i32) -> void (char with newline)
        self.module.add_function("println_char", print_int_type, None);

        // print_bool(i1) -> void
        let bool_type = self.context.bool_type();
        let print_bool_type = void_type.fn_type(&[bool_type.into()], false);
        self.module.add_function("print_bool", print_bool_type, None);

        // println_bool(i1) -> void
        self.module.add_function("println_bool", print_bool_type, None);

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

        // print_newline() -> void
        self.module.add_function("print_newline", println_type, None);

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
        let i32_ptr_type = self.context.ptr_type(AddressSpace::default());
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
        let void_ptr_ptr_type = self.context.ptr_type(AddressSpace::default());
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

        // blood_region_used(region_id: i64) -> i64 (used bytes)
        // Returns the current used bytes in a region
        let region_used_type = i64_type.fn_type(&[i64_type.into()], false);
        self.module.add_function("blood_region_used", region_used_type, None);

        // blood_region_activate(region_id: i64) -> void
        // Activates a region for the current thread so String/Vec/Box allocations route to it
        let region_activate_type = void_type.fn_type(&[i64_type.into()], false);
        self.module.add_function("blood_region_activate", region_activate_type, None);

        // blood_region_deactivate() -> void
        // Deactivates the current region, reverting to global allocation
        let region_deactivate_type = void_type.fn_type(&[], false);
        self.module.add_function("blood_region_deactivate", region_deactivate_type, None);

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

        // === OS Thread Primitives ===

        // blood_thread_spawn(func_ptr: i64, arg: i64) -> i64
        let thread_spawn_type = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        self.module.add_function("blood_thread_spawn", thread_spawn_type, None);

        // blood_thread_join(handle: i64) -> i64
        let thread_join_type = i64_type.fn_type(&[i64_type.into()], false);
        self.module.add_function("blood_thread_join", thread_join_type, None);

        // === Runtime Lifecycle ===

        // blood_runtime_init() -> i32
        let init_type = i32_type.fn_type(&[], false);
        self.module.add_function("blood_runtime_init", init_type, None);

        // blood_runtime_shutdown() -> void
        let shutdown_type = void_type.fn_type(&[], false);
        self.module.add_function("blood_runtime_shutdown", shutdown_type, None);

        // === Vec<T> Runtime Functions ===

        // vec_new(elem_size: i64, out: *void) -> void - writes Vec struct to output buffer
        let vec_new_type = void_type.fn_type(&[i64_type.into(), i8_ptr_type.into()], false);
        self.module.add_function("vec_new", vec_new_type, None);

        // vec_with_capacity(elem_size: i64, capacity: i64, out: *void) -> void - writes Vec struct to output buffer
        let vec_with_capacity_type = void_type.fn_type(&[i64_type.into(), i64_type.into(), i8_ptr_type.into()], false);
        self.module.add_function("vec_with_capacity", vec_with_capacity_type, None);

        // vec_len(vec: *void) -> i64
        let vec_len_type = i64_type.fn_type(&[i8_ptr_type.into()], false);
        self.module.add_function("vec_len", vec_len_type, None);

        // vec_is_empty(vec: *void) -> i32
        let vec_is_empty_type = i32_type.fn_type(&[i8_ptr_type.into()], false);
        self.module.add_function("vec_is_empty", vec_is_empty_type, None);

        // vec_capacity(vec: *void) -> i64
        let vec_capacity_type = i64_type.fn_type(&[i8_ptr_type.into()], false);
        self.module.add_function("vec_capacity", vec_capacity_type, None);

        // vec_push(vec: *void, elem: *void, elem_size: i64) -> void
        let vec_push_type = void_type.fn_type(&[i8_ptr_type.into(), i8_ptr_type.into(), i64_type.into()], false);
        self.module.add_function("vec_push", vec_push_type, None);

        // vec_pop(vec: *void, elem_size: i64, out: *void) -> i32
        let vec_pop_type = i32_type.fn_type(&[i8_ptr_type.into(), i64_type.into(), i8_ptr_type.into()], false);
        self.module.add_function("vec_pop", vec_pop_type, None);

        // vec_get(vec: *void, index: i64, elem_size: i64, out: *void) -> i32
        let vec_get_type = i32_type.fn_type(&[i8_ptr_type.into(), i64_type.into(), i64_type.into(), i8_ptr_type.into()], false);
        self.module.add_function("vec_get", vec_get_type, None);

        // vec_get_ptr(vec: *void, index: i64, elem_size: i64) -> *void
        let vec_get_ptr_type = i8_ptr_type.fn_type(&[i8_ptr_type.into(), i64_type.into(), i64_type.into()], false);
        self.module.add_function("vec_get_ptr", vec_get_ptr_type, None);

        // vec_contains(vec: *void, elem: *void, elem_size: i64) -> i32
        let vec_contains_type = i32_type.fn_type(&[i8_ptr_type.into(), i8_ptr_type.into(), i64_type.into()], false);
        self.module.add_function("vec_contains", vec_contains_type, None);

        // vec_reverse(vec: *void, elem_size: i64) -> void
        let vec_reverse_type = void_type.fn_type(&[i8_ptr_type.into(), i64_type.into()], false);
        self.module.add_function("vec_reverse", vec_reverse_type, None);

        // vec_clear(vec: *void) -> void
        let vec_clear_type = void_type.fn_type(&[i8_ptr_type.into()], false);
        self.module.add_function("vec_clear", vec_clear_type, None);

        // vec_first(vec: *void, elem_size: i64, out: *void) -> void
        // Returns Option<T> in out (None if empty)
        let vec_first_type = void_type.fn_type(&[i8_ptr_type.into(), i64_type.into(), i8_ptr_type.into()], false);
        self.module.add_function("vec_first", vec_first_type, None);

        // vec_last(vec: *void, elem_size: i64, out: *void) -> void
        // Returns Option<T> in out (None if empty)
        let vec_last_type = void_type.fn_type(&[i8_ptr_type.into(), i64_type.into(), i8_ptr_type.into()], false);
        self.module.add_function("vec_last", vec_last_type, None);

        // vec_free(vec: *void, elem_size: i64) -> void
        let vec_free_type = void_type.fn_type(&[i8_ptr_type.into(), i64_type.into()], false);
        self.module.add_function("vec_free", vec_free_type, None);

        // === Box<T> Runtime Functions ===

        // box_new(value: *void, size: i64) -> *void
        let box_new_type = i8_ptr_type.fn_type(&[i8_ptr_type.into(), i64_type.into()], false);
        self.module.add_function("box_new", box_new_type, None);

        // box_as_ref(boxed: *void) -> *void
        let box_as_ref_type = i8_ptr_type.fn_type(&[i8_ptr_type.into()], false);
        self.module.add_function("box_as_ref", box_as_ref_type, None);

        // box_as_mut(boxed: *void) -> *void
        let box_as_mut_type = i8_ptr_type.fn_type(&[i8_ptr_type.into()], false);
        self.module.add_function("box_as_mut", box_as_mut_type, None);

        // box_free(boxed: *void, size: i64) -> void
        let box_free_type = void_type.fn_type(&[i8_ptr_type.into(), i64_type.into()], false);
        self.module.add_function("box_free", box_free_type, None);

        // box_into_inner(boxed: *void, size: i64, out: *void) -> void
        let box_into_inner_type = void_type.fn_type(&[i8_ptr_type.into(), i64_type.into(), i8_ptr_type.into()], false);
        self.module.add_function("box_into_inner", box_into_inner_type, None);

        // box_into_raw(boxed: *void) -> *void
        let box_into_raw_type = i8_ptr_type.fn_type(&[i8_ptr_type.into()], false);
        self.module.add_function("box_into_raw", box_into_raw_type, None);

        // box_from_raw(ptr: *void) -> *void
        let box_from_raw_type = i8_ptr_type.fn_type(&[i8_ptr_type.into()], false);
        self.module.add_function("box_from_raw", box_from_raw_type, None);

        // box_leak(boxed: *void) -> *void
        let box_leak_type = i8_ptr_type.fn_type(&[i8_ptr_type.into()], false);
        self.module.add_function("box_leak", box_leak_type, None);

        // === Option<T> Runtime Functions ===
        // Option<T> is { tag: i32, payload: T } where tag=0 is None, tag=1 is Some

        // option_is_some(opt: *void) -> i32
        let option_is_some_type = i32_type.fn_type(&[i8_ptr_type.into()], false);
        self.module.add_function("option_is_some", option_is_some_type, None);

        // option_is_none(opt: *void) -> i32
        let option_is_none_type = i32_type.fn_type(&[i8_ptr_type.into()], false);
        self.module.add_function("option_is_none", option_is_none_type, None);

        // option_unwrap(opt: *void, payload_size: i64, out: *void) -> void
        // Copies the payload to out. Panics if None.
        let option_unwrap_type = void_type.fn_type(&[i8_ptr_type.into(), i64_type.into(), i8_ptr_type.into()], false);
        self.module.add_function("option_unwrap", option_unwrap_type, None);

        // option_try(opt: *void, payload_size: i64, out: *void) -> i32
        // Returns tag (0=None, 1=Some). If Some, copies payload to out.
        let option_try_type = i32_type.fn_type(&[i8_ptr_type.into(), i64_type.into(), i8_ptr_type.into()], false);
        self.module.add_function("option_try", option_try_type, None);

        // option_expect(opt: *void, payload_size: i64, msg: *i8, msg_len: i64, out: *void) -> void
        // Unwrap with custom panic message
        let option_expect_type = void_type.fn_type(&[
            i8_ptr_type.into(), i64_type.into(),
            i8_ptr_type.into(), i64_type.into(),
            i8_ptr_type.into()
        ], false);
        self.module.add_function("option_expect", option_expect_type, None);

        // option_unwrap_or(opt: *void, payload_size: i64, default: *void, out: *void) -> void
        // Unwrap or return default value
        let option_unwrap_or_type = void_type.fn_type(&[
            i8_ptr_type.into(), i64_type.into(),
            i8_ptr_type.into(), i8_ptr_type.into()
        ], false);
        self.module.add_function("option_unwrap_or", option_unwrap_or_type, None);

        // option_ok_or(opt: *void, t_size: i64, err: *void, e_size: i64, out: *void) -> void
        // Convert Option<T> to Result<T, E>
        let option_ok_or_type = void_type.fn_type(&[
            i8_ptr_type.into(), i64_type.into(),
            i8_ptr_type.into(), i64_type.into(),
            i8_ptr_type.into()
        ], false);
        self.module.add_function("option_ok_or", option_ok_or_type, None);

        // option_and(opt: *void, other: *void, other_size: i64, out: *void) -> void
        let option_and_type = void_type.fn_type(&[
            i8_ptr_type.into(), i8_ptr_type.into(),
            i64_type.into(), i8_ptr_type.into()
        ], false);
        self.module.add_function("option_and", option_and_type, None);

        // option_or(opt: *void, other: *void, option_size: i64, out: *void) -> void
        let option_or_type = void_type.fn_type(&[
            i8_ptr_type.into(), i8_ptr_type.into(),
            i64_type.into(), i8_ptr_type.into()
        ], false);
        self.module.add_function("option_or", option_or_type, None);

        // option_xor(opt: *void, other: *void, option_size: i64, out: *void) -> void
        let option_xor_type = void_type.fn_type(&[
            i8_ptr_type.into(), i8_ptr_type.into(),
            i64_type.into(), i8_ptr_type.into()
        ], false);
        self.module.add_function("option_xor", option_xor_type, None);

        // option_as_ref(opt: *void, payload_size: i64, out: *void) -> void
        let option_as_ref_type = void_type.fn_type(&[
            i8_ptr_type.into(), i64_type.into(), i8_ptr_type.into()
        ], false);
        self.module.add_function("option_as_ref", option_as_ref_type, None);

        // option_as_mut(opt: *void, payload_size: i64, out: *void) -> void
        let option_as_mut_type = void_type.fn_type(&[
            i8_ptr_type.into(), i64_type.into(), i8_ptr_type.into()
        ], false);
        self.module.add_function("option_as_mut", option_as_mut_type, None);

        // option_take(opt: *void, payload_size: i64, out: *void) -> void
        let option_take_type = void_type.fn_type(&[
            i8_ptr_type.into(), i64_type.into(), i8_ptr_type.into()
        ], false);
        self.module.add_function("option_take", option_take_type, None);

        // option_replace(opt: *void, value: *void, payload_size: i64, out: *void) -> void
        let option_replace_type = void_type.fn_type(&[
            i8_ptr_type.into(), i8_ptr_type.into(),
            i64_type.into(), i8_ptr_type.into()
        ], false);
        self.module.add_function("option_replace", option_replace_type, None);

        // === Result<T, E> Runtime Functions ===
        // Result<T, E> is { tag: i32, payload: union { T, E } }
        // where tag=0 is Ok(T) and tag=1 is Err(E)

        // result_is_ok(res: *void) -> i32
        let result_is_ok_type = i32_type.fn_type(&[i8_ptr_type.into()], false);
        self.module.add_function("result_is_ok", result_is_ok_type, None);

        // result_is_err(res: *void) -> i32
        let result_is_err_type = i32_type.fn_type(&[i8_ptr_type.into()], false);
        self.module.add_function("result_is_err", result_is_err_type, None);

        // result_unwrap(res: *void, ok_size: i64, out: *void) -> void
        // Copies the Ok payload to out. Panics if Err.
        let result_unwrap_type = void_type.fn_type(&[i8_ptr_type.into(), i64_type.into(), i8_ptr_type.into()], false);
        self.module.add_function("result_unwrap", result_unwrap_type, None);

        // result_unwrap_err(res: *void, err_size: i64, out: *void) -> void
        // Copies the Err payload to out. Panics if Ok.
        let result_unwrap_err_type = void_type.fn_type(&[i8_ptr_type.into(), i64_type.into(), i8_ptr_type.into()], false);
        self.module.add_function("result_unwrap_err", result_unwrap_err_type, None);

        // result_try(res: *void, ok_size: i64, out: *void) -> i32
        // Returns tag (0=Ok, 1=Err). If Ok, copies payload to out.
        let result_try_type = i32_type.fn_type(&[i8_ptr_type.into(), i64_type.into(), i8_ptr_type.into()], false);
        self.module.add_function("result_try", result_try_type, None);

        // result_ok(res: *void, ok_size: i64, out: *void) -> void
        // Converts Result<T, E> to Option<T>
        let result_ok_type = void_type.fn_type(&[i8_ptr_type.into(), i64_type.into(), i8_ptr_type.into()], false);
        self.module.add_function("result_ok", result_ok_type, None);

        // result_err(res: *void, err_size: i64, out: *void) -> void
        // Converts Result<T, E> to Option<E>
        let result_err_type = void_type.fn_type(&[i8_ptr_type.into(), i64_type.into(), i8_ptr_type.into()], false);
        self.module.add_function("result_err", result_err_type, None);

        // result_expect(res: *void, ok_size: i64, msg: *i8, msg_len: i64, out: *void) -> void
        // Unwrap with custom panic message
        let result_expect_type = void_type.fn_type(&[
            i8_ptr_type.into(), i64_type.into(),
            i8_ptr_type.into(), i64_type.into(),
            i8_ptr_type.into()
        ], false);
        self.module.add_function("result_expect", result_expect_type, None);

        // result_expect_err(res: *void, err_size: i64, msg: *i8, msg_len: i64, out: *void) -> void
        // Unwrap error with custom panic message
        let result_expect_err_type = void_type.fn_type(&[
            i8_ptr_type.into(), i64_type.into(),
            i8_ptr_type.into(), i64_type.into(),
            i8_ptr_type.into()
        ], false);
        self.module.add_function("result_expect_err", result_expect_err_type, None);

        // result_unwrap_or(res: *void, ok_size: i64, default: *void, out: *void) -> void
        // Unwrap or return default value
        let result_unwrap_or_type = void_type.fn_type(&[
            i8_ptr_type.into(), i64_type.into(),
            i8_ptr_type.into(), i8_ptr_type.into()
        ], false);
        self.module.add_function("result_unwrap_or", result_unwrap_or_type, None);

        // result_and(res: *void, other: *void, other_size: i64, err_size: i64, out: *void) -> void
        // If Ok, returns other. If Err, returns the error from self.
        let result_and_type = void_type.fn_type(&[
            i8_ptr_type.into(), i8_ptr_type.into(),
            i64_type.into(), i64_type.into(),
            i8_ptr_type.into()
        ], false);
        self.module.add_function("result_and", result_and_type, None);

        // result_or(res: *void, other: *void, ok_size: i64, other_size: i64, out: *void) -> void
        // If Ok, returns self. If Err, returns other.
        let result_or_type = void_type.fn_type(&[
            i8_ptr_type.into(), i8_ptr_type.into(),
            i64_type.into(), i64_type.into(),
            i8_ptr_type.into()
        ], false);
        self.module.add_function("result_or", result_or_type, None);

        // result_as_ref(res: *void, ok_size: i64, err_size: i64, out: *void) -> void
        // Converts &Result<T, E> to Result<&T, &E>
        let result_as_ref_type = void_type.fn_type(&[
            i8_ptr_type.into(), i64_type.into(),
            i64_type.into(), i8_ptr_type.into()
        ], false);
        self.module.add_function("result_as_ref", result_as_ref_type, None);

        // result_as_mut(res: *void, ok_size: i64, err_size: i64, out: *void) -> void
        // Converts &mut Result<T, E> to Result<&mut T, &mut E>
        let result_as_mut_type = void_type.fn_type(&[
            i8_ptr_type.into(), i64_type.into(),
            i64_type.into(), i8_ptr_type.into()
        ], false);
        self.module.add_function("result_as_mut", result_as_mut_type, None);

        // === File I/O Functions ===

        let bool_type = self.context.bool_type();

        // file_exists(path: {*i8, i64}) -> i1 (bool)
        // Check if a file exists at the given path
        let file_exists_type = bool_type.fn_type(&[str_slice_type.into()], false);
        self.module.add_function("file_exists", file_exists_type, None);

        // file_write_string(path: {*i8, i64}, content: {*i8, i64}) -> i1 (bool: true = success)
        // Write string content to a file
        let file_write_string_type = bool_type.fn_type(&[str_slice_type.into(), str_slice_type.into()], false);
        self.module.add_function("file_write_string", file_write_string_type, None);

        // file_read_to_string(path: {*i8, i64}) -> {*i8, i64} (String as str slice)
        // Read file contents as String
        let file_read_to_string_type = str_slice_type.fn_type(&[str_slice_type.into()], false);
        self.module.add_function("file_read_to_string", file_read_to_string_type, None);

        // file_delete(path: {*i8, i64}) -> i1 (bool: true = success)
        // Delete a file at the given path
        let file_delete_type = bool_type.fn_type(&[str_slice_type.into()], false);
        self.module.add_function("file_delete", file_delete_type, None);

        // file_append_string(path: {*i8, i64}, content: {*i8, i64}) -> i1 (bool: true = success)
        // Append string content to a file (creates if needed)
        let file_append_string_type = bool_type.fn_type(&[str_slice_type.into(), str_slice_type.into()], false);
        self.module.add_function("file_append_string", file_append_string_type, None);

        // system(cmd: {*i8, i64}) -> i32 (exit code)
        // Execute a shell command and return its exit code
        let system_type = i32_type.fn_type(&[str_slice_type.into()], false);
        self.module.add_function("system", system_type, None);

        // === Command-Line Arguments ===

        // args_count() -> i32
        // Get the number of command-line arguments
        let args_count_type = i32_type.fn_type(&[], false);
        self.module.add_function("args_count", args_count_type, None);

        // args_get(index: i32) -> {*i8, i64} (String as str slice)
        // Get command-line argument at index
        let args_get_type = str_slice_type.fn_type(&[i32_type.into()], false);
        self.module.add_function("args_get", args_get_type, None);
    }
}
