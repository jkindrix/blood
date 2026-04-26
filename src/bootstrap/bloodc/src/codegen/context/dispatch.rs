//! VTable generation and dynamic dispatch for codegen.
//!
//! This module handles:
//! - Dynamic dispatch through runtime type lookup
//! - VTable dispatch for trait objects
//! - VTable generation for trait implementations

use inkwell::types::BasicType;
use inkwell::values::{BasicValueEnum, FunctionValue, PointerValue};
use inkwell::AddressSpace;

use crate::diagnostics::Diagnostic;
use crate::hir::{self, DefId, Type, TypeKind};
use crate::span::Span;

use super::CodegenContext;

impl<'ctx, 'a> CodegenContext<'ctx, 'a> {
    /// Compile a dynamic dispatch call using runtime type lookup.
    ///
    /// This implements multiple dispatch by:
    /// 1. Getting the receiver's runtime type tag
    /// 2. Looking up the implementation in the dispatch table
    /// 3. Performing an indirect call through the function pointer
    pub(super) fn compile_dynamic_dispatch(
        &mut self,
        receiver: &hir::Expr,
        receiver_val: &BasicValueEnum<'ctx>,
        method_slot: u64,
        compiled_args: &[inkwell::values::BasicMetadataValueEnum<'ctx>],
        result_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        let i64_type = self.context.i64_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());

        // Get the blood_get_type_tag runtime function
        let get_type_tag_fn = self
            .module
            .get_function("blood_get_type_tag")
            .ok_or_else(|| {
                vec![Diagnostic::error(
                    "Runtime function blood_get_type_tag not found",
                    receiver.span,
                )]
            })?;

        // Get the blood_dispatch_lookup runtime function
        let dispatch_lookup_fn = self
            .module
            .get_function("blood_dispatch_lookup")
            .ok_or_else(|| {
                vec![Diagnostic::error(
                    "Runtime function blood_dispatch_lookup not found",
                    receiver.span,
                )]
            })?;

        // Cast receiver to void* for the type tag lookup
        let receiver_ptr = match receiver_val {
            BasicValueEnum::PointerValue(p) => *p,
            _ => {
                // For non-pointer types, we need to allocate and store
                // This shouldn't normally happen for method receivers
                let alloca = self
                    .builder
                    .build_alloca(receiver_val.get_type(), "receiver_tmp")
                    .map_err(|e| {
                        vec![Diagnostic::error(
                            format!("LLVM error: {}", e),
                            receiver.span,
                        )]
                    })?;
                self.builder
                    .build_store(alloca, *receiver_val)
                    .map_err(|e| {
                        vec![Diagnostic::error(
                            format!("LLVM error: {}", e),
                            receiver.span,
                        )]
                    })?;
                alloca
            }
        };

        let receiver_void_ptr = self
            .builder
            .build_pointer_cast(receiver_ptr, ptr_type, "receiver_void")
            .map_err(|e| {
                vec![Diagnostic::error(
                    format!("LLVM error: {}", e),
                    receiver.span,
                )]
            })?;

        // Get the type tag: blood_get_type_tag(receiver)
        let type_tag = self
            .builder
            .build_call(get_type_tag_fn, &[receiver_void_ptr.into()], "type_tag")
            .map_err(|e| {
                vec![Diagnostic::error(
                    format!("LLVM error: {}", e),
                    receiver.span,
                )]
            })?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| vec![Diagnostic::error("Expected type tag result", receiver.span)])?;

        // Look up the implementation: blood_dispatch_lookup(method_slot, type_tag)
        let method_slot_val = i64_type.const_int(method_slot, false);
        let impl_ptr = self
            .builder
            .build_call(
                dispatch_lookup_fn,
                &[method_slot_val.into(), type_tag.into()],
                "impl_ptr",
            )
            .map_err(|e| {
                vec![Diagnostic::error(
                    format!("LLVM error: {}", e),
                    receiver.span,
                )]
            })?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| {
                vec![Diagnostic::error(
                    "Expected implementation pointer",
                    receiver.span,
                )]
            })?;

        // Build the function type for the indirect call
        // Extract parameter types from the BasicMetadataValueEnum values
        let param_types: Vec<inkwell::types::BasicMetadataTypeEnum> = compiled_args
            .iter()
            .filter_map(|arg| {
                // Convert BasicMetadataValueEnum to its type
                match arg {
                    inkwell::values::BasicMetadataValueEnum::ArrayValue(v) => {
                        Some(v.get_type().into())
                    }
                    inkwell::values::BasicMetadataValueEnum::IntValue(v) => {
                        Some(v.get_type().into())
                    }
                    inkwell::values::BasicMetadataValueEnum::FloatValue(v) => {
                        Some(v.get_type().into())
                    }
                    inkwell::values::BasicMetadataValueEnum::PointerValue(v) => {
                        Some(v.get_type().into())
                    }
                    inkwell::values::BasicMetadataValueEnum::StructValue(v) => {
                        Some(v.get_type().into())
                    }
                    inkwell::values::BasicMetadataValueEnum::VectorValue(v) => {
                        Some(v.get_type().into())
                    }
                    inkwell::values::BasicMetadataValueEnum::ScalableVectorValue(v) => {
                        Some(v.get_type().into())
                    }
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

        // Build the indirect call through the function pointer
        // In inkwell 0.8 with opaque pointers, no pointer cast is needed —
        // use build_indirect_call with the function type directly
        let impl_ptr_val = impl_ptr.into_pointer_value();
        let call_site = self
            .builder
            .build_indirect_call(fn_type, impl_ptr_val, compiled_args, "dispatch_call")
            .map_err(|e| {
                vec![Diagnostic::error(
                    format!("LLVM error: {}", e),
                    receiver.span,
                )]
            })?;

        Ok(call_site.try_as_basic_value().basic())
    }

    /// Compile a method call on a dyn Trait using vtable dispatch.
    ///
    /// This implements trait object method dispatch by:
    /// 1. Extracting data pointer and vtable pointer from the fat pointer
    /// 2. Looking up the method in the vtable
    /// 3. Calling the function pointer with the data pointer as receiver
    pub(super) fn compile_vtable_dispatch(
        &mut self,
        receiver: &hir::Expr,
        trait_id: DefId,
        method_id: DefId,
        args: &[hir::Expr],
        result_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        let ptr_ty = self.context.ptr_type(AddressSpace::default());

        // Compile the receiver - this should be a fat pointer { data_ptr, vtable_ptr }
        let fat_ptr = self.compile_expr(receiver)?.ok_or_else(|| {
            vec![Diagnostic::error(
                "Expected fat pointer for dyn Trait receiver",
                receiver.span,
            )]
        })?;

        let fat_ptr_struct = fat_ptr.into_struct_value();

        // Extract data pointer (index 0)
        let data_ptr = self
            .builder
            .build_extract_value(fat_ptr_struct, 0, "data_ptr")
            .map_err(|e| {
                vec![Diagnostic::error(
                    format!("LLVM error: {}", e),
                    receiver.span,
                )]
            })?
            .into_pointer_value();

        // Extract vtable pointer (index 1)
        let vtable_ptr = self
            .builder
            .build_extract_value(fat_ptr_struct, 1, "vtable_ptr")
            .map_err(|e| {
                vec![Diagnostic::error(
                    format!("LLVM error: {}", e),
                    receiver.span,
                )]
            })?
            .into_pointer_value();

        // Get the method name from the method_id to look up its slot
        // For now, we'll use the method_id to look up in the trait's method list
        let method_slot = self
            .get_vtable_slot_for_method(trait_id, method_id)
            .ok_or_else(|| {
                vec![Diagnostic::error(
                    format!(
                        "Method {:?} not found in vtable for trait {:?}",
                        method_id, trait_id
                    ),
                    receiver.span,
                )]
            })?;

        // Calculate pointer to the method slot in the vtable
        // With opaque pointers, vtable is just a pointer to an array of pointers
        let i32_ty = self.context.i32_type();

        // GEP into the vtable to get the slot pointer
        // The vtable is an array of pointers; pointee_ty for GEP is ptr
        let slot_ptr = unsafe {
            self.builder.build_gep(
                ptr_ty,
                vtable_ptr,
                &[i32_ty.const_int(method_slot as u64, false)],
                "slot_ptr",
            )
        }
        .map_err(|e| {
            vec![Diagnostic::error(
                format!("LLVM error: {}", e),
                receiver.span,
            )]
        })?;

        // Load the function pointer from the slot
        let fn_ptr = self
            .builder
            .build_load(ptr_ty, slot_ptr, "fn_ptr")
            .map_err(|e| {
                vec![Diagnostic::error(
                    format!("LLVM error: {}", e),
                    receiver.span,
                )]
            })?
            .into_pointer_value();

        // Compile remaining arguments with data_ptr as first argument
        let mut compiled_args: Vec<inkwell::values::BasicMetadataValueEnum> = vec![data_ptr.into()];
        for arg in args {
            if let Some(val) = self.compile_expr(arg)? {
                compiled_args.push(val.into());
            }
        }

        // Build function type from arguments and return type
        let param_types: Vec<inkwell::types::BasicMetadataTypeEnum> = compiled_args
            .iter()
            .filter_map(|arg| match arg {
                inkwell::values::BasicMetadataValueEnum::ArrayValue(v) => Some(v.get_type().into()),
                inkwell::values::BasicMetadataValueEnum::IntValue(v) => Some(v.get_type().into()),
                inkwell::values::BasicMetadataValueEnum::FloatValue(v) => Some(v.get_type().into()),
                inkwell::values::BasicMetadataValueEnum::PointerValue(v) => {
                    Some(v.get_type().into())
                }
                inkwell::values::BasicMetadataValueEnum::StructValue(v) => {
                    Some(v.get_type().into())
                }
                inkwell::values::BasicMetadataValueEnum::VectorValue(v) => {
                    Some(v.get_type().into())
                }
                inkwell::values::BasicMetadataValueEnum::ScalableVectorValue(v) => {
                    Some(v.get_type().into())
                }
                inkwell::values::BasicMetadataValueEnum::MetadataValue(_) => None,
            })
            .collect();

        let fn_type = if matches!(result_ty.kind(), TypeKind::Tuple(types) if types.is_empty()) {
            self.context.void_type().fn_type(&param_types, false)
        } else {
            let ret_ty = self.lower_type(result_ty);
            ret_ty.fn_type(&param_types, false)
        };

        // Call through function pointer using indirect call
        // With opaque pointers, no pointer cast is needed — pass fn_type directly
        let call_site = self
            .builder
            .build_indirect_call(fn_type, fn_ptr, &compiled_args, "vtable_call")
            .map_err(|e| {
                vec![Diagnostic::error(
                    format!("LLVM error: {}", e),
                    receiver.span,
                )]
            })?;

        Ok(call_site.try_as_basic_value().basic())
    }

    /// Get the vtable slot for a specific method.
    ///
    /// Returns the actual slot index in the vtable array, accounting for the
    /// 2-slot metadata prefix (size, align). Method slots start at index 2.
    fn get_vtable_slot_for_method(&self, trait_id: DefId, method_id: DefId) -> Option<usize> {
        if let Some(layout) = self.vtable_layouts.get(&trait_id) {
            // Look up the method name from the method_id
            let method_name = self.def_paths.get(&method_id).map(|s| s.as_str());

            // Search by method name if available
            if let Some(name) = method_name {
                for (idx, (slot_name, _)) in layout.iter().enumerate() {
                    if slot_name == name || name.ends_with(slot_name.as_str()) {
                        // Offset by 2 to skip size and align metadata slots
                        return Some(idx + 2);
                    }
                }
            }

            // Fallback: use the slot index from the layout directly
            let method_idx = method_id.index() as usize;
            if method_idx < layout.len() {
                // Offset by 2 to skip size and align metadata slots
                return Some(layout[method_idx].1 + 2);
            }
        }
        None
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
        let i8_ptr_type = self.context.ptr_type(AddressSpace::default());

        let dispatch_register_fn = self
            .module
            .get_function("blood_dispatch_register")
            .ok_or_else(|| {
                vec![Diagnostic::error(
                    "Runtime function blood_dispatch_register not found",
                    span,
                )]
            })?;

        let method_slot_val = i64_type.const_int(method_slot, false);
        let type_tag_val = i64_type.const_int(type_tag, false);

        // Cast function to void*
        let impl_ptr = self
            .builder
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

    // =========================================================================
    // Vtable Generation
    // =========================================================================

    /// Generate vtables for all trait implementations in the crate.
    ///
    /// This creates global constant arrays containing function pointers for
    /// each method in the trait, enabling dynamic dispatch through trait objects.
    pub fn generate_vtables(&mut self, hir_crate: &hir::Crate) -> Result<(), Vec<Diagnostic>> {
        // Build vtable layouts if not already done (may have been called early
        // to populate trait_method_info before MIR compilation).
        if self.vtable_layouts.is_empty() {
            self.build_vtable_layouts(hir_crate);
        }

        // Then generate vtables for each trait impl using trait_impls info.
        // The HIR doesn't have ItemKind::Impl items (impl methods are flattened
        // as top-level Fn items), so we use the trait_impls list from typeck.
        for impl_info in &hir_crate.trait_impls {
            self.generate_vtable_for_trait_impl(impl_info)?;
        }

        Ok(())
    }

    /// Build vtable layouts for all traits.
    ///
    /// The layout determines which slot each method occupies in the vtable.
    pub fn build_vtable_layouts(&mut self, hir_crate: &hir::Crate) {
        // Sort by DefId for deterministic layout ordering.
        let mut sorted_items: Vec<_> = hir_crate.items.iter().collect();
        sorted_items.sort_by_key(|(&def_id, _)| def_id.index());
        for (def_id, item) in sorted_items {
            if let hir::ItemKind::Trait { items, .. } = &item.kind {
                let mut slots = Vec::new();

                for (idx, trait_item) in items.iter().enumerate() {
                    if let hir::TraitItemKind::Fn(_, _) = &trait_item.kind {
                        slots.push((trait_item.name.clone(), idx));
                        // Map abstract trait method DefId -> (trait_id, method_name)
                        self.trait_method_info
                            .insert(trait_item.def_id, (*def_id, trait_item.name.clone()));
                    }
                }

                self.vtable_layouts.insert(*def_id, slots);
            }
        }
    }

    /// Generate a vtable for a specific trait impl using TraitImplInfo.
    ///
    /// Vtable layout per DISPATCH.md §10.8.1:
    /// ```
    /// ┌─────────────────┐
    /// │ size: usize      │  Slot 0 — Size of concrete type (reserved/null)
    /// │ align: usize     │  Slot 1 — Alignment of concrete type (reserved/null)
    /// │ m₁: fn_ptr       │  Slot 2 — Method 1 implementation
    /// │ ...              │
    /// │ mₙ: fn_ptr       │  Slot N+1 — Method N implementation
    /// └─────────────────┘
    /// ```
    ///
    /// Note: Blood's vtable has no `drop_fn` slot (DEF-010 resolved).
    fn generate_vtable_for_trait_impl(
        &mut self,
        impl_info: &hir::TraitImplInfo,
    ) -> Result<(), Vec<Diagnostic>> {
        let trait_id = impl_info.trait_id;

        // Get the vtable layout for this trait
        let Some(layout) = self.vtable_layouts.get(&trait_id).cloned() else {
            // No layout - trait has no methods
            return Ok(());
        };

        // Vtable has 2 metadata slots (size, align) + N method slots
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let vtable_len = 2 + layout.len();
        let vtable_type = ptr_type.array_type(vtable_len as u32);

        // Create a unique name for this vtable
        let trait_path = self
            .def_paths
            .get(&trait_id)
            .cloned()
            .unwrap_or_else(|| format!("{}", trait_id.index()));
        let vtable_name = format!(
            "__vtable_{}_{}_{}",
            trait_path,
            self.type_to_vtable_name(&impl_info.self_ty),
            self.vtables.len()
        );

        // Slots 0-1 are reserved for size/align metadata per DISPATCH.md §10.8.1.
        // Blood uses regions + finally clauses for cleanup (no per-value destructors),
        // so size/align are not needed at runtime for dispatch. Use null placeholders.
        // This avoids the LLVM constant-expression limitation (build_int_to_ptr
        // generates instructions, not constants, which cannot be used in global
        // constant initializers).
        let size_as_ptr = ptr_type.const_null();
        let align_as_ptr = ptr_type.const_null();

        // Build vtable slots: [size, align, m1, m2, ..., mn]
        let mut vtable_slots: Vec<PointerValue<'ctx>> = Vec::new();
        vtable_slots.push(size_as_ptr);
        vtable_slots.push(align_as_ptr);

        for (method_name, _slot_idx) in &layout {
            // Look up the impl method by name from TraitImplInfo.methods
            let impl_fn = impl_info
                .methods
                .iter()
                .find(|(name, _)| name == method_name)
                .and_then(|(_, def_id)| self.functions.get(def_id).copied());

            match impl_fn {
                Some(fn_val) => {
                    let fn_ptr = fn_val.as_global_value().as_pointer_value();
                    vtable_slots.push(fn_ptr);
                }
                None => {
                    // Method not found - use null pointer (will panic at runtime)
                    // This should have been caught by trait impl validation
                    vtable_slots.push(ptr_type.const_null());
                }
            }
        }

        // Create the vtable global constant
        let vtable_init = ptr_type.const_array(&vtable_slots);
        let vtable_global = self.module.add_global(vtable_type, None, &vtable_name);
        vtable_global.set_initializer(&vtable_init);
        vtable_global.set_constant(true);
        vtable_global.set_linkage(inkwell::module::Linkage::LinkOnceODR);

        // Store for later lookup — self_ty should always be an ADT in trait impl context
        if let Some(type_def_id) = self.type_to_def_id(&impl_info.self_ty) {
            self.vtables.insert((trait_id, type_def_id), vtable_global);
        } else {
            debug_assert!(
                false,
                "ICE: vtable generated for non-ADT type: {:?}",
                impl_info.self_ty
            );
        }

        Ok(())
    }

    /// Convert a type to a string suitable for vtable naming.
    fn type_to_vtable_name(&self, ty: &Type) -> String {
        match ty.kind() {
            TypeKind::Primitive(prim) => format!("{:?}", prim).to_lowercase(),
            TypeKind::Adt { def_id, .. } => self
                .def_paths
                .get(def_id)
                .map(|p| format!("adt_{}", p))
                .unwrap_or_else(|| format!("adt{}", def_id.index())),
            TypeKind::Ref { mutable, inner } => {
                let m = if *mutable { "mut_" } else { "" };
                format!("{}ref_{}", m, self.type_to_vtable_name(inner))
            }
            TypeKind::Tuple(elems) => {
                let parts: Vec<_> = elems.iter().map(|e| self.type_to_vtable_name(e)).collect();
                format!("tuple_{}", parts.join("_"))
            }
            TypeKind::Fn { params, ret, .. } => {
                let parts: Vec<_> = params.iter().map(|p| self.type_to_vtable_name(p)).collect();
                format!("fn_{}_{}", parts.join("_"), self.type_to_vtable_name(ret))
            }
            TypeKind::Closure { def_id, .. } => format!("closure{}", def_id.index()),
            TypeKind::Array { element, .. } => {
                format!("array_{}", self.type_to_vtable_name(element))
            }
            TypeKind::Slice { element } => format!("slice_{}", self.type_to_vtable_name(element)),
            TypeKind::Ptr { inner, mutable } => {
                let m = if *mutable { "mut_" } else { "const_" };
                format!("{}ptr_{}", m, self.type_to_vtable_name(inner))
            }
            TypeKind::Never => "never".to_string(),
            TypeKind::Range { element, inclusive } => {
                let kind = if *inclusive { "rangeinc" } else { "range" };
                format!("{}_{}", kind, self.type_to_vtable_name(element))
            }
            TypeKind::DynTrait { trait_id, .. } => self
                .def_paths
                .get(trait_id)
                .map(|p| format!("dyn_{}", p))
                .unwrap_or_else(|| format!("dyn{}", trait_id.index())),
            TypeKind::Record { fields, .. } => {
                let parts: Vec<_> = fields
                    .iter()
                    .map(|f| format!("{:?}_{}", f.name, self.type_to_vtable_name(&f.ty)))
                    .collect();
                format!("record_{}", parts.join("_"))
            }
            TypeKind::Forall { body, .. } => format!("forall_{}", self.type_to_vtable_name(body)),
            TypeKind::Ownership { inner, .. } => self.type_to_vtable_name(inner),
            // ICE: these types should not appear in vtable contexts
            TypeKind::Infer(_) | TypeKind::Param(_) | TypeKind::Error => {
                debug_assert!(
                    false,
                    "ICE: unexpected type in vtable naming: {:?}",
                    ty.kind()
                );
                "error".to_string()
            }
        }
    }

    /// Get the DefId for a type (for vtable lookup).
    /// Returns `None` for non-ADT types that have no DefId.
    fn type_to_def_id(&self, ty: &Type) -> Option<DefId> {
        match ty.kind() {
            TypeKind::Adt { def_id, .. } => Some(*def_id),
            _ => None,
        }
    }

    /// Look up a vtable for a (trait, type) pair.
    pub fn get_vtable(&self, trait_id: DefId, ty: &Type) -> Option<PointerValue<'ctx>> {
        let type_def_id = self.type_to_def_id(ty)?;
        self.vtables
            .get(&(trait_id, type_def_id))
            .map(|g| g.as_pointer_value())
    }

    /// Get the vtable slot index for a method.
    pub fn get_vtable_slot(&self, trait_id: DefId, method_name: &str) -> Option<usize> {
        self.vtable_layouts
            .get(&trait_id)?
            .iter()
            .find(|(name, _)| name == method_name)
            .map(|(_, idx)| *idx)
    }

    /// Compile a coercion to a trait object (dyn Trait).
    ///
    /// Creates a fat pointer consisting of:
    /// - data_ptr: pointer to the concrete value
    /// - vtable_ptr: pointer to the vtable for (trait_id, concrete_type)
    pub(super) fn compile_trait_object_coercion(
        &mut self,
        expr: &hir::Expr,
        trait_id: DefId,
        _target_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Vec<Diagnostic>> {
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let source_ty = &expr.ty;

        // Compile the source expression
        let val = self.compile_expr(expr)?.ok_or_else(|| {
            vec![Diagnostic::error(
                "Expected value for trait object coercion",
                expr.span,
            )]
        })?;

        // Get data pointer - if not already a pointer, allocate and store
        let data_ptr = match val {
            BasicValueEnum::PointerValue(ptr) => self
                .builder
                .build_pointer_cast(ptr, ptr_ty, "data_ptr")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), expr.span)])?,
            _ => {
                // Allocate temporary storage for the value
                let alloca = self
                    .builder
                    .build_alloca(val.get_type(), "trait_obj_data")
                    .map_err(|e| {
                        vec![Diagnostic::error(format!("LLVM error: {}", e), expr.span)]
                    })?;
                self.builder.build_store(alloca, val).map_err(|e| {
                    vec![Diagnostic::error(format!("LLVM error: {}", e), expr.span)]
                })?;
                self.builder
                    .build_pointer_cast(alloca, ptr_ty, "data_ptr")
                    .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), expr.span)])?
            }
        };

        // Get vtable pointer for (trait_id, source_type)
        let vtable_ptr = match self.get_vtable(trait_id, source_ty) {
            Some(vtable) => self
                .builder
                .build_pointer_cast(vtable, ptr_ty, "vtable_ptr")
                .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), expr.span)])?,
            None => {
                // No vtable found - use null (will cause runtime error if called)
                // This might happen if the impl block wasn't processed yet
                self.errors.push(Diagnostic::warning(
                    format!(
                        "No vtable found for trait {:?} on type {}",
                        trait_id, source_ty
                    ),
                    expr.span,
                ));
                ptr_ty.const_null()
            }
        };

        // Create fat pointer struct { data_ptr, vtable_ptr }
        let fat_ptr_ty = self
            .context
            .struct_type(&[ptr_ty.into(), ptr_ty.into()], false);
        let mut fat_ptr = fat_ptr_ty.get_undef();
        fat_ptr = self
            .builder
            .build_insert_value(fat_ptr, data_ptr, 0, "fat_ptr.data")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), expr.span)])?
            .into_struct_value();
        fat_ptr = self
            .builder
            .build_insert_value(fat_ptr, vtable_ptr, 1, "fat_ptr.vtable")
            .map_err(|e| vec![Diagnostic::error(format!("LLVM error: {}", e), expr.span)])?
            .into_struct_value();

        Ok(Some(fat_ptr.into()))
    }
}
