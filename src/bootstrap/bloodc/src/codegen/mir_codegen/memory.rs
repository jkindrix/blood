//! MIR memory management code generation.
//!
//! This module handles memory allocation, generation checks, and related
//! runtime interactions for the Blood memory safety system.

use inkwell::values::{IntValue, PointerValue};
use inkwell::types::BasicTypeEnum;
use inkwell::{AddressSpace, IntPredicate};

use crate::diagnostics::Diagnostic;
use crate::hir::LocalId;
use crate::span::Span;
use crate::ice;
use crate::ice_err;

use super::types::MirTypesCodegen;
use super::CodegenContext;

/// Extension trait for MIR memory operations.
pub trait MirMemoryCodegen<'ctx, 'a> {
    /// Get the generation for an address by calling the runtime's blood_get_generation.
    fn get_generation_for_address(
        &self,
        address: IntValue<'ctx>,
        i32_ty: inkwell::types::IntType<'ctx>,
        span: Span,
    ) -> Result<IntValue<'ctx>, Vec<Diagnostic>>;
}

impl<'ctx, 'a> MirMemoryCodegen<'ctx, 'a> for CodegenContext<'ctx, 'a> {
    fn get_generation_for_address(
        &self,
        address: IntValue<'ctx>,
        _i32_ty: inkwell::types::IntType<'ctx>,
        span: Span,
    ) -> Result<IntValue<'ctx>, Vec<Diagnostic>> {
        let get_gen_fn = self.module.get_function("blood_get_generation")
            .ok_or_else(|| vec![ice_err!(
                span,
                "Runtime function blood_get_generation not declared";
                "context" => "This function should be declared by CodegenContext::new()"
            )])?;

        let gen_call_result = self.builder.build_call(
            get_gen_fn,
            &[address.into()],
            "gen_lookup"
        ).map_err(|e| vec![Diagnostic::error(format!("LLVM call error: {}", e), span)])?;

        gen_call_result.try_as_basic_value().basic()
            .map(|val| val.into_int_value())
            .ok_or_else(|| vec![ice_err!(
                span,
                "blood_get_generation returned void unexpectedly";
                "expected" => "i32 return value from runtime function"
            )])
    }
}

/// Emit a generation check for a pointer dereference.
///
/// This is the implementation for MirCodegen::emit_generation_check.
pub(super) fn emit_generation_check_impl<'ctx, 'a>(
    ctx: &mut CodegenContext<'ctx, 'a>,
    ptr: PointerValue<'ctx>,
    expected_gen: IntValue<'ctx>,
    span: Span,
) -> Result<(), Vec<Diagnostic>> {
    // Emit a generation check by calling the runtime function.
    //
    // The runtime function `blood_validate_generation` handles:
    // 1. Looking up the current generation from the slot registry
    // 2. Comparing with the expected generation
    // 3. Returns 0 if valid, 1 if stale
    //
    // If the check fails, we perform the StaleReference effect (0x1004).
    // The default handler panics; users can install custom handlers.

    let i32_ty = ctx.context.i32_type();
    let i64_ty = ctx.context.i64_type();

    // Get the validation function - uses slot registry for address-based lookup
    let validate_fn = ctx.module.get_function("blood_validate_generation")
        .ok_or_else(|| vec![Diagnostic::error(
            "blood_validate_generation not declared", span
        )])?;

    // Convert pointer to i64 address for the runtime call
    let address = ctx.builder.build_ptr_to_int(ptr, i64_ty, "ptr_addr")
        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

    // Call blood_validate_generation(address: i64, expected_gen: i32) -> i32
    // Returns: 0 = valid, 1 = stale (generation mismatch)
    let result = ctx.builder.build_call(
        validate_fn,
        &[address.into(), expected_gen.into()],
        "gen_check"
    ).map_err(|e| vec![Diagnostic::error(format!("LLVM call error: {}", e), span)])?;

    let is_stale = result.try_as_basic_value()
        .basic()
        .ok_or_else(|| vec![Diagnostic::error("Generation check returned void", span)])?
        .into_int_value();

    // Create blocks for valid and stale paths
    let fn_value = ctx.current_fn.ok_or_else(|| {
        vec![Diagnostic::error("No current function", span)]
    })?;

    let valid_bb = ctx.context.append_basic_block(fn_value, "gen_valid");
    let stale_bb = ctx.context.append_basic_block(fn_value, "gen_stale");

    // Compare: is_stale == 0 (valid if result is 0)
    let zero = i32_ty.const_int(0, false);
    let is_valid = ctx.builder.build_int_compare(
        IntPredicate::EQ,
        is_stale,
        zero,
        "is_valid"
    ).map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

    ctx.builder.build_conditional_branch(is_valid, valid_bb, stale_bb)
        .map_err(|e| vec![Diagnostic::error(format!("LLVM branch error: {}", e), span)])?;

    // Stale path: perform StaleReference effect (handler decides what to do)
    ctx.builder.position_at_end(stale_bb);

    // Get actual generation for the handler's diagnostic info
    let actual_gen = if let Some(get_gen_fn) = ctx.module.get_function("blood_get_generation") {
        let gen_call_result = ctx.builder.build_call(
            get_gen_fn,
            &[address.into()],
            "actual_gen"
        ).map_err(|e| vec![Diagnostic::error(format!("LLVM call error: {}", e), span)])?;

        match gen_call_result.try_as_basic_value().basic() {
            Some(val) => val.into_int_value(),
            None => {
                ice!("blood_get_generation returned void unexpectedly";
                     "span" => span,
                     "fallback" => "using 0 for handler args");
                i32_ty.const_int(0, false)
            }
        }
    } else {
        i32_ty.const_int(0, false)
    };

    emit_stale_reference_perform(ctx, expected_gen, actual_gen, span)?;

    // Continue at valid block
    ctx.builder.position_at_end(valid_bb);

    Ok(())
}

/// Allocate memory using blood_alloc for Region/Persistent tier.
///
/// This is the implementation for MirCodegen::allocate_with_blood_alloc.
pub(super) fn allocate_with_blood_alloc_impl<'ctx, 'a>(
    ctx: &mut CodegenContext<'ctx, 'a>,
    llvm_ty: BasicTypeEnum<'ctx>,
    local_id: LocalId,
    span: Span,
) -> Result<PointerValue<'ctx>, Vec<Diagnostic>> {
    // Use blood_alloc_or_abort for region/persistent allocation.
    // This function aborts on failure, so no conditional branches needed.

    let i32_ty = ctx.context.i32_type();
    let i64_ty = ctx.context.i64_type();

    // Get the blood_alloc_or_abort function
    let alloc_fn = ctx.module.get_function("blood_alloc_or_abort")
        .ok_or_else(|| vec![Diagnostic::error(
            "Runtime function blood_alloc_or_abort not found", span
        )])?;

    // Calculate size of the type
    let type_size = ctx.get_type_size_in_bytes(llvm_ty);
    let size_val = i64_ty.const_int(type_size, false);

    // Create stack alloca for the generation output parameter
    let gen_alloca = ctx.builder.build_alloca(i32_ty, &format!("_{}_gen", local_id.index))
        .map_err(|e| vec![Diagnostic::error(
            format!("LLVM alloca error: {}", e), span
        )])?;

    // Zero-initialize gen_alloca before the runtime call to distinguish between
    // "runtime didn't write" (stays 0) and "stack corruption" (becomes garbage)
    ctx.builder.build_store(gen_alloca, i32_ty.const_int(0, false))
        .map_err(|e| vec![Diagnostic::error(
            format!("LLVM store error: {}", e), span
        )])?;

    // Call blood_alloc_or_abort(size, &out_generation) -> address
    let address = ctx.builder.build_call(
        alloc_fn,
        &[size_val.into(), gen_alloca.into()],
        &format!("_{}_addr", local_id.index)
    ).map_err(|e| vec![Diagnostic::error(
        format!("LLVM call error: {}", e), span
    )])?
        .try_as_basic_value()
        .basic()
        .ok_or_else(|| vec![Diagnostic::error(
            "blood_alloc_or_abort returned void", span
        )])?
        .into_int_value();

    // Convert the address (i64) to a pointer (opaque pointer in LLVM 18)
    let typed_ptr = ctx.builder.build_int_to_ptr(
        address,
        ctx.context.ptr_type(AddressSpace::default()),
        &format!("_{}_ptr", local_id.index)
    ).map_err(|e| vec![Diagnostic::error(
        format!("LLVM int_to_ptr error: {}", e), span
    )])?;

    // Store the generation in local_generations map for later validation
    // (The generation is stored in gen_alloca and can be loaded when needed)
    ctx.local_generations.insert(local_id, gen_alloca);

    Ok(typed_ptr)
}

/// Allocate memory using blood_persistent_alloc for Persistent (Tier 3) tier.
///
/// This function:
/// 1. Calls blood_persistent_alloc(size, align, type_fp, &out_id) -> *mut u8
/// 2. Stores the returned slot ID in a stack alloca for RC lifecycle management
/// 3. Returns a typed pointer to the allocated memory
///
/// The slot ID is tracked in `persistent_slot_ids` so that `StorageDead` can
/// emit `blood_persistent_decrement` to manage the reference count.
pub(super) fn allocate_with_persistent_alloc_impl<'ctx, 'a>(
    ctx: &mut CodegenContext<'ctx, 'a>,
    llvm_ty: BasicTypeEnum<'ctx>,
    local_id: LocalId,
    span: Span,
) -> Result<PointerValue<'ctx>, Vec<Diagnostic>> {
    let ptr_ty = ctx.context.ptr_type(AddressSpace::default());
    let i32_ty = ctx.context.i32_type();
    let i64_ty = ctx.context.i64_type();

    // Get or declare blood_persistent_alloc(size: i64, align: i64, type_fp: i32, out_id: *i64) -> *u8
    // Note: Uses i64 for size/align since usize == u64 on 64-bit targets
    let alloc_fn = ctx.module.get_function("blood_persistent_alloc")
        .unwrap_or_else(|| {
            let fn_type = ptr_ty.fn_type(
                &[i64_ty.into(), i64_ty.into(), i32_ty.into(), ptr_ty.into()],
                false,
            );
            ctx.module.add_function("blood_persistent_alloc", fn_type, None)
        });

    // Calculate size and alignment of the type
    let type_size = ctx.get_type_size_in_bytes(llvm_ty);
    let type_align = ctx.get_type_alignment_for_size(llvm_ty);
    let size_val = i64_ty.const_int(type_size, false);
    let align_val = i64_ty.const_int(type_align, false);
    let type_fp = i32_ty.const_int(0, false); // Type fingerprint (0 = unknown)

    // Create stack alloca for the slot ID output parameter
    let slot_id_alloca = ctx.builder.build_alloca(i64_ty, &format!("_{}_slot_id", local_id.index))
        .map_err(|e| vec![Diagnostic::error(
            format!("LLVM alloca error: {}", e), span
        )])?;

    // Initialize slot ID to 0
    ctx.builder.build_store(slot_id_alloca, i64_ty.const_int(0, false))
        .map_err(|e| vec![Diagnostic::error(
            format!("LLVM store error: {}", e), span
        )])?;

    // Call blood_persistent_alloc(size, align, type_fp, &out_id) -> *u8
    let raw_ptr = ctx.builder.build_call(
        alloc_fn,
        &[size_val.into(), align_val.into(), type_fp.into(), slot_id_alloca.into()],
        &format!("_{}_persistent_ptr", local_id.index)
    ).map_err(|e| vec![Diagnostic::error(
        format!("LLVM call error: {}", e), span
    )])?
        .try_as_basic_value()
        .basic()
        .ok_or_else(|| vec![Diagnostic::error(
            "blood_persistent_alloc returned void", span
        )])?
        .into_pointer_value();

    // With opaque pointers (LLVM 18), all pointers are the same type,
    // so the raw_ptr from blood_persistent_alloc is already usable directly.

    // Store the slot ID alloca for later decrement on StorageDead
    ctx.persistent_slot_ids.insert(local_id, slot_id_alloca);

    Ok(raw_ptr)
}

/// Emit a `blood_perform` call for the StaleReference effect.
///
/// This performs `StaleReference.stale(expected_gen, actual_gen)` through
/// the evidence vector. The default handler panics; user code can install
/// custom handlers that must diverge (-> never).
///
/// The effect ID 0x1004 matches `STALE_REFERENCE_EFFECT_ID` in std_effects.rs.
pub(super) fn emit_stale_reference_perform<'ctx, 'a>(
    ctx: &mut CodegenContext<'ctx, 'a>,
    expected_gen: IntValue<'ctx>,
    actual_gen: IntValue<'ctx>,
    span: Span,
) -> Result<(), Vec<Diagnostic>> {
    let i32_ty = ctx.context.i32_type();
    let i64_ty = ctx.context.i64_type();

    let perform_fn = ctx.module.get_function("blood_perform")
        .ok_or_else(|| vec![Diagnostic::error(
            "Runtime function blood_perform not found", span
        )])?;

    // Zero-extend i32 generation values to i64 for the args array
    let exp_i64 = ctx.builder.build_int_z_extend(expected_gen, i64_ty, "exp_i64")
        .map_err(|e| vec![Diagnostic::error(format!("LLVM extend error: {}", e), span)])?;
    let act_i64 = ctx.builder.build_int_z_extend(actual_gen, i64_ty, "act_i64")
        .map_err(|e| vec![Diagnostic::error(format!("LLVM extend error: {}", e), span)])?;

    // Create args array [expected_gen, actual_gen] on stack
    let array_ty = i64_ty.array_type(2);
    let args_alloca = ctx.builder.build_alloca(array_ty, "stale_args")
        .map_err(|e| vec![Diagnostic::error(format!("LLVM alloca error: {}", e), span)])?;

    let zero = i64_ty.const_int(0, false);
    let one = i64_ty.const_int(1, false);
    let gep0 = unsafe {
        ctx.builder.build_gep(array_ty, args_alloca, &[zero, zero], "stale_arg_0")
    }.map_err(|e| vec![Diagnostic::error(format!("LLVM GEP error: {}", e), span)])?;
    ctx.builder.build_store(gep0, exp_i64)
        .map_err(|e| vec![Diagnostic::error(format!("LLVM store error: {}", e), span)])?;
    let gep1 = unsafe {
        ctx.builder.build_gep(array_ty, args_alloca, &[zero, one], "stale_arg_1")
    }.map_err(|e| vec![Diagnostic::error(format!("LLVM GEP error: {}", e), span)])?;
    ctx.builder.build_store(gep1, act_i64)
        .map_err(|e| vec![Diagnostic::error(format!("LLVM store error: {}", e), span)])?;

    // Call blood_perform(effect_id=0x1004, op_index=0, args, arg_count=2, continuation=0)
    let effect_id = i64_ty.const_int(0x1004, false);
    let op_index = i32_ty.const_int(0, false);
    let arg_count = i64_ty.const_int(2, false);
    let continuation = i64_ty.const_int(0, false);

    ctx.builder.build_call(
        perform_fn,
        &[effect_id.into(), op_index.into(), args_alloca.into(), arg_count.into(), continuation.into()],
        ""
    ).map_err(|e| vec![Diagnostic::error(format!("LLVM call error: {}", e), span)])?;

    // StaleReference.stale returns -> never, so emit unreachable
    ctx.builder.build_unreachable()
        .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), span)])?;

    Ok(())
}
