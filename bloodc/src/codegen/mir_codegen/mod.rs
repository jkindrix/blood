//! MIR-based code generation.
//!
//! This module compiles MIR (Mid-level Intermediate Representation) to LLVM IR.
//! MIR provides explicit control flow graphs and flattened expressions, making
//! it easier to implement optimizations like:
//!
//! - **Escape analysis**: Skip generation checks for non-escaping values
//! - **Tier-based allocation**: Use stack vs region based on escape state
//! - **Generation validation**: Insert checks for region-allocated values
//!
//! # Architecture
//!
//! ```text
//! MIR Body -> Basic Blocks -> LLVM Basic Blocks
//!          -> Statements   -> LLVM Instructions
//!          -> Terminators  -> LLVM Terminators
//! ```
//!
//! # Memory Tier Allocation Strategy
//!
//! | Memory Tier | Escape State | Allocation Method | Generation Checks |
//! |-------------|--------------|-------------------|-------------------|
//! | Stack (0)   | NoEscape     | LLVM `alloca`     | NO - safe by construction |
//! | Region (1)  | ArgEscape    | `blood_alloc_or_abort` | YES - on every dereference |
//! | Region (1)  | GlobalEscape | `blood_alloc_or_abort` | YES - on every dereference |
//! | Region (1)  | Effect-captured | `blood_alloc_or_abort` | YES - on every dereference |
//!
//! # Generation Check Flow (for Region-allocated values)
//!
//! ```text
//! 1. Allocation:
//!    address = blood_alloc_or_abort(size, &generation)
//!    // Registers in slot registry, returns address and generation
//!
//! 2. On Dereference:
//!    result = blood_validate_generation(address, expected_generation)
//!    if result != 0:
//!        blood_stale_reference_panic(expected_gen, actual_gen)  // aborts
//!    // Continue with dereference
//!
//! 3. On Free (automatic at scope exit):
//!    blood_unregister_allocation(address)
//!    // Invalidates the address in slot registry
//!    // Future validation with old generation returns 1 (stale)
//! ```
//!
//! # Integration with Escape Analysis
//!
//! When escape analysis results are available, the MIR codegen:
//! 1. Queries `recommended_tier(local)` for each local
//! 2. Allocates stack storage for NoEscape locals (thin pointers)
//! 3. Allocates region storage for ArgEscape/GlobalEscape via `blood_alloc_or_abort`
//! 4. Stores generation in `local_generations` map for later validation
//! 5. On dereference, emits `blood_validate_generation` call for region-allocated values
//! 6. Branches to panic path on stale reference detection

mod statement;
mod terminator;
mod rvalue;
mod place;
mod memory;
mod types;

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use inkwell::basic_block::BasicBlock;
use inkwell::values::{IntValue, PointerValue};
use inkwell::types::BasicTypeEnum;
use inkwell::AddressSpace;

use crate::diagnostics::Diagnostic;
use crate::hir::{DefId, LocalId, Type};
use crate::mir::body::MirBody;
use crate::mir::types::BasicBlockId;
use crate::mir::{EscapeResults, EscapeState, MemoryTier};
use crate::span::Span;
use crate::ice_err;

use super::CodegenContext;

// Re-export extension traits
pub use statement::MirStatementCodegen;
pub use terminator::MirTerminatorCodegen;
pub use rvalue::MirRvalueCodegen;
pub use place::MirPlaceCodegen;
pub use memory::MirMemoryCodegen;
pub use types::MirTypesCodegen;

/// Extension trait for MIR compilation on CodegenContext.
pub trait MirCodegen<'ctx, 'a>:
    MirStatementCodegen<'ctx, 'a>
    + MirTerminatorCodegen<'ctx, 'a>
    + MirRvalueCodegen<'ctx, 'a>
    + MirPlaceCodegen<'ctx, 'a>
    + MirMemoryCodegen<'ctx, 'a>
    + MirTypesCodegen<'ctx, 'a>
{
    /// Compile a MIR function body.
    fn compile_mir_body(
        &mut self,
        def_id: DefId,
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
    ) -> Result<(), Vec<Diagnostic>>;

    /// Compile a single MIR basic block.
    fn compile_mir_block(
        &mut self,
        body: &MirBody,
        block_id: BasicBlockId,
        llvm_blocks: &HashMap<BasicBlockId, BasicBlock<'ctx>>,
        escape_results: Option<&EscapeResults>,
    ) -> Result<(), Vec<Diagnostic>>;

    /// Determine the memory tier for a local based on escape analysis.
    fn get_local_tier(
        &self,
        local: LocalId,
        escape_results: Option<&EscapeResults>,
    ) -> MemoryTier;

    /// Check if generation checks can be skipped for a local.
    fn should_skip_gen_check(
        &self,
        local: LocalId,
        escape_results: Option<&EscapeResults>,
    ) -> bool;

    /// Emit a generation check for a pointer dereference.
    fn emit_generation_check(
        &mut self,
        ptr: PointerValue<'ctx>,
        expected_gen: IntValue<'ctx>,
        span: Span,
    ) -> Result<(), Vec<Diagnostic>>;

    /// Check if a type may contain generational references.
    ///
    /// Used to determine which locals need snapshot capture during
    /// effect operations.
    fn type_may_contain_genref(&self, ty: &Type) -> bool;

    /// Allocate memory using blood_alloc for Region tier.
    ///
    /// This calls the runtime's blood_alloc function which:
    /// 1. Allocates memory on the heap
    /// 2. Registers it in the slot registry
    /// 3. Returns the address and generation metadata
    ///
    /// Returns a pointer to the allocated memory.
    fn allocate_with_blood_alloc(
        &mut self,
        llvm_ty: BasicTypeEnum<'ctx>,
        local_id: LocalId,
        span: Span,
    ) -> Result<PointerValue<'ctx>, Vec<Diagnostic>>;

    /// Allocate memory using blood_persistent_alloc for Persistent (Tier 3) tier.
    ///
    /// This calls the runtime's blood_persistent_alloc function which:
    /// 1. Allocates a reference-counted persistent slot
    /// 2. Returns a pointer to the allocated memory and a slot ID
    /// 3. The slot ID is stored for RC lifecycle management (decrement on StorageDead)
    ///
    /// Returns a pointer to the allocated memory.
    fn allocate_with_persistent_alloc(
        &mut self,
        llvm_ty: BasicTypeEnum<'ctx>,
        local_id: LocalId,
        span: Span,
    ) -> Result<PointerValue<'ctx>, Vec<Diagnostic>>;
}

/// Helper function to get tier name for debug output.
pub(crate) fn tier_name(tier: MemoryTier) -> &'static str {
    match tier {
        MemoryTier::Stack => "stack",
        MemoryTier::Region => "region",
        MemoryTier::Persistent => "persistent",
        MemoryTier::Reserved => "reserved",
    }
}

impl<'ctx, 'a> MirCodegen<'ctx, 'a> for CodegenContext<'ctx, 'a> {
    fn compile_mir_body(
        &mut self,
        def_id: DefId,
        body: &MirBody,
        escape_results: Option<&EscapeResults>,
    ) -> Result<(), Vec<Diagnostic>> {
        // Set the MIR span for error reporting in downstream codegen helpers.
        self.current_mir_span.set(body.span);

        // DEFENSIVE CHECK: Skip if this DefId is a static item.
        // Static items are compiled as global variables by compile_static_items(),
        // not as functions. MIR lowering should never create bodies for static items,
        // but this check prevents catastrophic failure if one somehow ends up here.
        // If a static DefId is processed as a function, it would create a dummy
        // function (define void @... { ret void }) instead of the global variable,
        // causing segmentation faults when the static is assigned to at runtime.
        if self.static_globals.contains_key(&def_id) {
            // This is a static item - it should NOT have a MIR body to compile.
            // The static's initializer is evaluated by compile_static_items().
            return Ok(());
        }

        // Check if this function was declared. Functions may not be declared if:
        // - It's a generic function (will be monomorphized on-demand at call sites)
        // - It's a const/static item (handled separately)
        // - Declaration was skipped for some other reason
        let fn_value = match self.functions.get(&def_id) {
            Some(&fv) => fv,
            None => {
                // Function not declared - skip compiling its body.
                // This is typically a generic function awaiting monomorphization.
                return Ok(());
            }
        };

        self.current_fn = Some(fn_value);
        self.current_fn_def_id = Some(def_id);
        self.locals.clear();
        self.local_generations.clear();
        self.persistent_slot_ids.clear();

        // Create LLVM basic blocks for all MIR blocks
        let mut llvm_blocks: HashMap<BasicBlockId, BasicBlock<'ctx>> = HashMap::new();
        for (bb_id, _) in body.blocks() {
            let name = format!("bb{}", bb_id.0);
            let llvm_bb = self.context.append_basic_block(fn_value, &name);
            llvm_blocks.insert(bb_id, llvm_bb);
        }

        // Position at entry block
        let entry_bb = llvm_blocks.get(&BasicBlockId::ENTRY).ok_or_else(|| {
            vec![Diagnostic::error("MIR body has no entry block", body.span)]
        })?;
        self.builder.position_at_end(*entry_bb);

        // Check if this is a closure body (synthetic DefId >= 0xFFFF_0000)
        let is_closure_body = def_id.index() >= 0xFFFF_0000;

        // Allocate locals based on escape analysis
        for local in &body.locals {
            // Reference types (&T, &mut T) should ALWAYS use stack allocation for their
            // storage, regardless of escape state. The local stores a pointer value,
            // not the actual data, so escape analysis doesn't apply to the storage.
            // This is critical for performance - reference parameters shouldn't trigger
            // region allocation or generation checks.
            let is_reference_type = local.ty.is_ref();

            let tier = if is_reference_type {
                // Force stack allocation for reference storage
                MemoryTier::Stack
            } else {
                self.get_local_tier(local.id, escape_results)
            };

            // For closure bodies, the __env parameter should use i8* type
            // regardless of its MIR type (which is the actual tuple of captures)
            let llvm_ty = if is_closure_body && local.name.as_deref() == Some("__env") {
                self.context.i8_type().ptr_type(AddressSpace::default()).into()
            } else if let crate::hir::TypeKind::Closure { def_id: closure_def_id, .. } = local.ty.kind() {
                // Check if this closure local should use inline environment layout.
                // Inline closures store captures directly: { fn_ptr, T0, T1, ... }
                // instead of via pointer: { fn_ptr, env_ptr }
                if self.should_inline_closure_env(closure_def_id, Some(local.id)) {
                    if let Some(info) = self.closure_analysis.as_ref()
                        .and_then(|ca| ca.get(*closure_def_id))
                    {
                        if info.capture_types.is_empty() {
                            // Zero captures: use standard fat pointer {fn_ptr, env_ptr}
                            // to match rvalue path which skips inline for empty captures
                            self.lower_type(&local.ty)
                        } else {
                            let i8_ptr_ty = self.context.i8_type().ptr_type(AddressSpace::default());
                            let mut field_types: Vec<inkwell::types::BasicTypeEnum> =
                                Vec::with_capacity(info.capture_types.len() + 1);
                            field_types.push(i8_ptr_ty.into());
                            for cap_ty in &info.capture_types {
                                field_types.push(self.lower_type(cap_ty));
                            }
                            self.context.struct_type(&field_types, false).into()
                        }
                    } else {
                        self.lower_type(&local.ty)
                    }
                } else {
                    self.lower_type(&local.ty)
                }
            } else {
                self.lower_type(&local.ty)
            };

            let alloca = match tier {
                MemoryTier::Stack => {
                    // Stack allocation - thin pointer, no generation check needed
                    // This is the fast path for non-escaping values
                    let alloca_ptr = self.builder.build_alloca(
                        llvm_ty,
                        &format!("_{}_{}", local.id.index, tier_name(tier))
                    ).map_err(|e| vec![Diagnostic::error(
                        format!("LLVM alloca error: {}", e), body.span
                    )])?;
                    // Set explicit alignment on alloca to ensure correct ABI handling
                    // for struct returns and complex types
                    let alignment = self.get_type_alignment_for_size(llvm_ty) as u32;
                    if let Some(inst) = alloca_ptr.as_instruction() {
                        let _ = inst.set_alignment(alignment);
                    }
                    alloca_ptr
                }
                MemoryTier::Region => {
                    // Region allocation - use blood_alloc for generational tracking
                    // This is the safe path for escaping values that need generation checks
                    self.allocate_with_blood_alloc(llvm_ty, local.id, body.span)?
                }
                MemoryTier::Persistent => {
                    // Persistent (Tier 3) allocation - use blood_persistent_alloc for RC lifecycle
                    // Unlike region allocation which uses generational references,
                    // persistent values use reference counting with cycle collection
                    self.allocate_with_persistent_alloc(llvm_ty, local.id, body.span)?
                }
                MemoryTier::Reserved => {
                    // Reserved tier should never be returned by escape analysis.
                    // If we reach here, something is wrong with the escape analysis or
                    // a future feature is using Reserved without implementing it.
                    return Err(vec![ice_err!(
                        body.span,
                        "MemoryTier::Reserved reached during allocation";
                        "local" => format!("_{}", local.id.index),
                        "note" => "escape analysis should never return Reserved tier"
                    )]);
                }
            };

            self.locals.insert(local.id, alloca);
        }

        // Store function parameters
        for (i, param_id) in body.param_ids().enumerate() {
            if let Some(&alloca) = self.locals.get(&param_id) {
                let param_value = fn_value.get_nth_param(i as u32)
                    .ok_or_else(|| vec![Diagnostic::error(
                        format!("Parameter {} not found", i), body.span
                    )])?;
                let param_store = self.builder.build_store(alloca, param_value)
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM store error: {}", e), body.span
                    )])?;
                // Set proper alignment for parameter store
                let param_alignment = self.get_type_alignment_for_value(param_value);
                let _ = param_store.set_alignment(param_alignment);
            }
        }

        // Compile each basic block
        for (bb_id, _) in body.blocks() {
            self.compile_mir_block(body, bb_id, &llvm_blocks, escape_results)?;
        }

        // Set WeakODR linkage now that the function has a body.
        // This allows the linker to merge duplicate definitions when the same
        // function is compiled into multiple object files (per-definition mode).
        // We use WeakODR instead of LinkOnceODR because LinkOnceODR can be
        // stripped by LLVM optimization when there are no local callers.
        self.set_function_weak_odr(def_id);

        self.current_fn = None;
        Ok(())
    }

    fn compile_mir_block(
        &mut self,
        body: &MirBody,
        block_id: BasicBlockId,
        llvm_blocks: &HashMap<BasicBlockId, BasicBlock<'ctx>>,
        escape_results: Option<&EscapeResults>,
    ) -> Result<(), Vec<Diagnostic>> {
        let llvm_bb = *llvm_blocks.get(&block_id).ok_or_else(|| {
            vec![Diagnostic::error(
                format!("No LLVM block for {}", block_id),
                body.span,
            )]
        })?;

        self.builder.position_at_end(llvm_bb);

        let block_data = body.get_block(block_id).ok_or_else(|| {
            vec![Diagnostic::error(
                format!("MIR block {} not found", block_id),
                body.span,
            )]
        })?;

        // Compile statements
        for stmt in &block_data.statements {
            self.current_mir_span.set(stmt.span);
            self.compile_mir_statement(stmt, body, escape_results)?;
        }

        // Compile terminator
        if let Some(term) = &block_data.terminator {
            self.current_mir_span.set(term.span);
            self.compile_mir_terminator(term, body, llvm_blocks, escape_results)?;
        } else {
            // Unterminated block - add unreachable
            self.builder.build_unreachable()
                .map_err(|e| vec![Diagnostic::error(
                    format!("LLVM unreachable error: {}", e), body.span
                )])?;
        }

        Ok(())
    }

    fn get_local_tier(
        &self,
        local: LocalId,
        escape_results: Option<&EscapeResults>,
    ) -> MemoryTier {
        if let Some(results) = escape_results {
            results.recommended_tier(local)
        } else {
            // Default to Region if no escape analysis available
            MemoryTier::Region
        }
    }

    fn should_skip_gen_check(
        &self,
        local: LocalId,
        escape_results: Option<&EscapeResults>,
    ) -> bool {
        if let Some(results) = escape_results {
            // Skip generation check based on memory tier.
            // Only Region tier needs generation checks; Stack and Persistent
            // skip them for different reasons:
            // - Stack: safe by lexical scoping (alloca)
            // - Persistent: uses reference counting, not generational checks
            let tier = results.recommended_tier(local);
            if !tier.needs_generation_check() {
                return true;
            }
            // For Region tier, additionally skip if the local doesn't escape
            // and isn't effect-captured (redundant with tier check, but explicit)
            let state = results.get(local);
            state == EscapeState::NoEscape && !results.is_effect_captured(local)
        } else {
            false // Conservative: always check if no analysis
        }
    }

    fn emit_generation_check(
        &mut self,
        ptr: PointerValue<'ctx>,
        expected_gen: IntValue<'ctx>,
        span: Span,
    ) -> Result<(), Vec<Diagnostic>> {
        memory::emit_generation_check_impl(self, ptr, expected_gen, span)
    }

    fn type_may_contain_genref(&self, ty: &Type) -> bool {
        types::type_may_contain_genref_impl(ty)
    }

    fn allocate_with_blood_alloc(
        &mut self,
        llvm_ty: BasicTypeEnum<'ctx>,
        local_id: LocalId,
        span: Span,
    ) -> Result<PointerValue<'ctx>, Vec<Diagnostic>> {
        memory::allocate_with_blood_alloc_impl(self, llvm_ty, local_id, span)
    }

    fn allocate_with_persistent_alloc(
        &mut self,
        llvm_ty: BasicTypeEnum<'ctx>,
        local_id: LocalId,
        span: Span,
    ) -> Result<PointerValue<'ctx>, Vec<Diagnostic>> {
        memory::allocate_with_persistent_alloc_impl(self, llvm_ty, local_id, span)
    }
}
