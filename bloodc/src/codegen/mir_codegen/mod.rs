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

    /// Allocate memory using blood_alloc for Region/Persistent tier.
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
        // Check if this function was declared. Generic functions are skipped during declaration
        // and will be monomorphized on-demand at call sites (not yet implemented).
        let fn_value = match self.functions.get(&def_id) {
            Some(&fv) => fv,
            None => {
                // Function not declared - this is a generic function.
                // Skip compiling its body. It will be monomorphized when called.
                return Ok(());
            }
        };

        self.current_fn = Some(fn_value);
        self.current_fn_def_id = Some(def_id);
        self.locals.clear();
        self.local_generations.clear();

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
            let tier = self.get_local_tier(local.id, escape_results);

            // For closure bodies, the __env parameter should use i8* type
            // regardless of its MIR type (which is the actual tuple of captures)
            let llvm_ty = if is_closure_body && local.name.as_deref() == Some("__env") {
                self.context.i8_type().ptr_type(AddressSpace::default()).into()
            } else {
                self.lower_type(&local.ty)
            };

            let alloca = match tier {
                MemoryTier::Stack => {
                    // Stack allocation - thin pointer, no generation check needed
                    // This is the fast path for non-escaping values
                    self.builder.build_alloca(
                        llvm_ty,
                        &format!("_{}_{}", local.id.index, tier_name(tier))
                    ).map_err(|e| vec![Diagnostic::error(
                        format!("LLVM alloca error: {}", e), body.span
                    )])?
                }
                MemoryTier::Region | MemoryTier::Persistent => {
                    // Region allocation - use blood_alloc for generational tracking
                    // This is the safe path for escaping values that need generation checks
                    self.allocate_with_blood_alloc(llvm_ty, local.id, body.span)?
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
                self.builder.build_store(alloca, param_value)
                    .map_err(|e| vec![Diagnostic::error(
                        format!("LLVM store error: {}", e), body.span
                    )])?;
            }
        }

        // Compile each basic block
        for (bb_id, _) in body.blocks() {
            self.compile_mir_block(body, bb_id, &llvm_blocks, escape_results)?;
        }

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
            self.compile_mir_statement(stmt, body, escape_results)?;
        }

        // Compile terminator
        if let Some(term) = &block_data.terminator {
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
}
