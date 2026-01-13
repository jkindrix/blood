//! Trait bounds checking and coherence validation.

use std::collections::HashMap;

use crate::ast;
use crate::hir::{self, DefId, Type, TypeKind, TyVarId};
use crate::span::Span;

use super::{TypeContext, ImplMethodInfo, ImplAssocTypeInfo};
use super::super::error::{TypeError, TypeErrorKind};

impl<'a> TypeContext<'a> {
    /// Check if a type satisfies all trait bounds required by a type parameter.
    #[allow(dead_code)]
    pub(crate) fn check_trait_bounds(
        &self,
        ty: &Type,
        bounds: &[DefId],
        span: Span,
    ) -> Result<(), TypeError> {
        for &trait_def_id in bounds {
            if !self.type_implements_trait(ty, trait_def_id) {
                let trait_name = self.trait_defs.get(&trait_def_id)
                    .map(|info| info.name.clone())
                    .unwrap_or_else(|| format!("{:?}", trait_def_id));
                return Err(TypeError::new(
                    TypeErrorKind::TraitBoundNotSatisfied {
                        ty: ty.clone(),
                        trait_name,
                    },
                    span,
                ));
            }
        }
        Ok(())
    }

    /// Check if a type implements a trait.
    ///
    /// Checks explicit impl blocks first, then built-in trait implementations.
    pub(crate) fn type_implements_trait(&self, ty: &Type, trait_def_id: DefId) -> bool {
        // Check explicit impl blocks
        for impl_block in &self.impl_blocks {
            if impl_block.trait_ref == Some(trait_def_id) && impl_block.self_ty == *ty {
                return true;
            }
        }

        // Check built-in trait implementations
        if let Some(trait_info) = self.trait_defs.get(&trait_def_id) {
            return self.type_has_builtin_impl(ty, &trait_info.name);
        }

        false
    }

    /// Resolve a trait bound (AST type) to a trait DefId.
    ///
    /// Trait bounds in type parameters (e.g., `T: Display`) are represented as
    /// AST types. This resolves them to their corresponding trait DefId.
    pub(crate) fn resolve_trait_bound(&self, bound: &ast::Type) -> Option<DefId> {
        match &bound.kind {
            ast::TypeKind::Path(type_path) => {
                // Get the trait name from the path
                if let Some(segment) = type_path.segments.last() {
                    let trait_name = self.symbol_to_string(segment.name.node);
                    // Look up the trait by name
                    for (&def_id, trait_info) in &self.trait_defs {
                        if trait_info.name == trait_name {
                            return Some(def_id);
                        }
                    }
                }
                None
            }
            _ => None, // Other type kinds are not valid trait bounds
        }
    }

    /// Register type parameter bounds for method lookup.
    ///
    /// This processes the bounds on a type parameter and stores them
    /// for use in method resolution on generic types.
    pub(crate) fn register_type_param_bounds(&mut self, ty_var_id: TyVarId, bounds: &[ast::Type]) {
        if bounds.is_empty() {
            return;
        }

        let mut trait_bounds = Vec::new();
        for bound in bounds {
            if let Some(trait_def_id) = self.resolve_trait_bound(bound) {
                trait_bounds.push(trait_def_id);
            }
        }

        if !trait_bounds.is_empty() {
            self.type_param_bounds.insert(ty_var_id, trait_bounds);
        }
    }

    /// Check if a type has a built-in implementation of a well-known trait.
    ///
    /// This handles traits like Copy, Clone, Sized that primitives and certain
    /// types implement automatically without explicit impl blocks.
    pub(crate) fn type_has_builtin_impl(&self, ty: &Type, trait_name: &str) -> bool {
        match trait_name {
            "Copy" => self.type_is_copy(ty),
            "Clone" => self.type_is_clone(ty),
            "Sized" => self.type_is_sized(ty),
            "Send" => self.type_is_send(ty),
            "Sync" => self.type_is_sync(ty),
            _ => false,
        }
    }

    /// Check if a type implements Copy (can be bitwise copied).
    ///
    /// Copy types:
    /// - All primitives (bool, char, integers, floats, unit)
    /// - References (&T) - shared references are always Copy
    /// - Raw pointers (*const T, *mut T)
    /// - Arrays [T; N] where T: Copy
    /// - Tuples where all elements are Copy
    /// - Function pointers
    pub(crate) fn type_is_copy(&self, ty: &Type) -> bool {
        match ty.kind() {
            // Primitives are Copy
            TypeKind::Primitive(_) => true,
            // Shared references are Copy
            TypeKind::Ref { mutable: false, .. } => true,
            // Mutable references are NOT Copy (to preserve uniqueness)
            TypeKind::Ref { mutable: true, .. } => false,
            // Raw pointers are Copy
            TypeKind::Ptr { .. } => true,
            // Function pointers are Copy
            TypeKind::Fn { .. } => true,
            // Never type is Copy (vacuously)
            TypeKind::Never => true,
            // Arrays are Copy if element is Copy
            TypeKind::Array { element, .. } => self.type_is_copy(element),
            // Tuples are Copy if all elements are Copy
            TypeKind::Tuple(elements) => elements.iter().all(|e| self.type_is_copy(e)),
            // Range is Copy if element is Copy
            TypeKind::Range { element, .. } => self.type_is_copy(element),
            // Slices are NOT Copy (they're unsized)
            TypeKind::Slice { .. } => false,
            // Closures are NOT Copy (they capture environment)
            TypeKind::Closure { .. } => false,
            // ADTs require explicit Copy impl
            TypeKind::Adt { .. } => false,
            // Trait objects are NOT Copy (they're unsized)
            TypeKind::DynTrait { .. } => false,
            // Error and inference types - be conservative
            TypeKind::Error => true,
            TypeKind::Infer(_) => false,
            TypeKind::Param(_) => false, // Requires trait bound
            // Records are Copy if all fields are Copy
            TypeKind::Record { fields, .. } => fields.iter().all(|f| self.type_is_copy(&f.ty)),
            // Forall types are NOT Copy (polymorphic values need special handling)
            TypeKind::Forall { .. } => false,
        }
    }

    /// Check if a type implements Clone.
    ///
    /// Clone types: everything that is Copy, plus types with explicit Clone impls.
    pub(crate) fn type_is_clone(&self, ty: &Type) -> bool {
        // All Copy types are Clone
        if self.type_is_copy(ty) {
            return true;
        }
        // For non-Copy types, would need to check impl blocks (already done in caller)
        false
    }

    /// Check if a type implements Sized.
    ///
    /// Unsized types: str, [T] (slices), dyn Trait
    pub(crate) fn type_is_sized(&self, ty: &Type) -> bool {
        match ty.kind() {
            TypeKind::Slice { .. } => false,
            TypeKind::Primitive(hir::PrimitiveTy::Str) => false,
            // Trait objects are dynamically sized (DST)
            TypeKind::DynTrait { .. } => false,
            _ => true,
        }
    }

    /// Check if a type implements Send (can be transferred across threads).
    ///
    /// Most types are Send unless they contain non-Send types.
    pub(crate) fn type_is_send(&self, ty: &Type) -> bool {
        match ty.kind() {
            // Primitives are Send
            TypeKind::Primitive(_) => true,
            // References to Send types are Send
            TypeKind::Ref { inner, .. } => self.type_is_send(inner),
            TypeKind::Ptr { inner, .. } => self.type_is_send(inner),
            // Arrays and tuples are Send if elements are
            TypeKind::Array { element, .. } => self.type_is_send(element),
            TypeKind::Tuple(elements) => elements.iter().all(|e| self.type_is_send(e)),
            // Closures depend on captured types - conservative default
            TypeKind::Closure { .. } => false,
            // For ADTs, would need to check all fields - conservative default
            TypeKind::Adt { .. } => true,
            // Trait objects are Send only if they have + Send bound
            TypeKind::DynTrait { auto_traits, .. } => {
                auto_traits.iter().any(|trait_id| {
                    self.trait_defs.get(trait_id)
                        .map(|info| info.name == "Send")
                        .unwrap_or(false)
                })
            }
            _ => true,
        }
    }

    /// Check if a type implements Sync (can be shared across threads via &T).
    ///
    /// A type is Sync if &T is Send.
    pub(crate) fn type_is_sync(&self, ty: &Type) -> bool {
        // For now, same logic as Send - primitives and simple types are Sync
        match ty.kind() {
            TypeKind::Primitive(_) => true,
            TypeKind::Ref { inner, .. } => self.type_is_sync(inner),
            TypeKind::Ptr { inner, .. } => self.type_is_sync(inner),
            TypeKind::Array { element, .. } => self.type_is_sync(element),
            TypeKind::Tuple(elements) => elements.iter().all(|e| self.type_is_sync(e)),
            TypeKind::Closure { .. } => false,
            TypeKind::Adt { .. } => true,
            // Trait objects are Sync only if they have + Sync bound
            TypeKind::DynTrait { auto_traits, .. } => {
                auto_traits.iter().any(|trait_id| {
                    self.trait_defs.get(trait_id)
                        .map(|info| info.name == "Sync")
                        .unwrap_or(false)
                })
            }
            _ => true,
        }
    }

    /// Check coherence: detect overlapping impl blocks.
    ///
    /// Two impls overlap if they could apply to the same type. For example:
    /// - `impl Trait for i32` and `impl Trait for i32` overlap
    /// - `impl<T> Trait for T` and `impl Trait for i32` overlap
    pub(crate) fn check_coherence(&mut self) {
        // Group impls by trait
        let mut trait_impls: HashMap<DefId, Vec<(usize, &super::ImplBlockInfo)>> = HashMap::new();

        for (idx, impl_block) in self.impl_blocks.iter().enumerate() {
            if let Some(trait_id) = impl_block.trait_ref {
                trait_impls.entry(trait_id).or_default().push((idx, impl_block));
            }
        }

        // For each trait, check for overlapping impls
        for (trait_id, impls) in &trait_impls {
            // O(n^2) pairwise comparison - fine for typical crate sizes
            for i in 0..impls.len() {
                for j in (i + 1)..impls.len() {
                    let (idx_a, impl_a) = &impls[i];
                    let (idx_b, impl_b) = &impls[j];

                    if self.impls_could_overlap(&impl_a.self_ty, &impl_b.self_ty) {
                        // Get trait name for error message
                        let trait_name = self.trait_defs.get(trait_id)
                            .map(|t| t.name.clone())
                            .unwrap_or_else(|| format!("trait#{}", trait_id.index()));

                        self.errors.push(TypeError::new(
                            TypeErrorKind::OverlappingImpls {
                                trait_name,
                                ty_a: impl_a.self_ty.clone(),
                                ty_b: impl_b.self_ty.clone(),
                            },
                            impl_a.span, // Use first impl's span for error location
                        ));

                        // Note: idx_a and idx_b could be used for secondary spans
                        // in a more sophisticated error message format
                        let _ = (idx_a, idx_b);
                    }
                }
            }
        }
    }

    /// Check if two impl self types could potentially overlap.
    ///
    /// Two types overlap if there exists a concrete type that both could match.
    pub(crate) fn impls_could_overlap(&self, ty_a: &Type, ty_b: &Type) -> bool {
        match (ty_a.kind(), ty_b.kind()) {
            // Same primitive type -> overlap
            (TypeKind::Primitive(a), TypeKind::Primitive(b)) => a == b,

            // Same ADT -> overlap
            (
                TypeKind::Adt { def_id: a_id, .. },
                TypeKind::Adt { def_id: b_id, .. },
            ) => a_id == b_id,

            // Generic type parameter overlaps with anything (blanket impl)
            (TypeKind::Param(_), _) | (_, TypeKind::Param(_)) => true,

            // Reference types: check inner types and mutability
            (
                TypeKind::Ref { mutable: a_mut, inner: a_inner },
                TypeKind::Ref { mutable: b_mut, inner: b_inner },
            ) => a_mut == b_mut && self.impls_could_overlap(a_inner, b_inner),

            // Tuple types: same length and overlapping elements
            (TypeKind::Tuple(a_elems), TypeKind::Tuple(b_elems)) => {
                a_elems.len() == b_elems.len()
                    && a_elems.iter().zip(b_elems.iter()).all(|(a, b)| self.impls_could_overlap(a, b))
            }

            // Array types: same size and overlapping elements
            (
                TypeKind::Array { element: a_elem, size: a_size },
                TypeKind::Array { element: b_elem, size: b_size },
            ) => a_size == b_size && self.impls_could_overlap(a_elem, b_elem),

            // Slice types: overlapping elements
            (TypeKind::Slice { element: a_elem }, TypeKind::Slice { element: b_elem }) => {
                self.impls_could_overlap(a_elem, b_elem)
            }

            // Different type kinds don't overlap
            _ => false,
        }
    }

    /// Validate that a trait impl provides all required methods and associated types.
    pub(crate) fn validate_trait_impl(
        &self,
        trait_id: DefId,
        impl_methods: &[ImplMethodInfo],
        impl_assoc_types: &[ImplAssocTypeInfo],
        span: Span,
    ) -> Result<(), TypeError> {
        let Some(trait_info) = self.trait_defs.get(&trait_id) else {
            // Trait not found - already reported during trait resolution
            return Ok(());
        };

        // Check for missing methods (that don't have default implementations)
        for trait_method in &trait_info.methods {
            if trait_method.has_default {
                // Method has a default implementation, not required
                continue;
            }

            let provided = impl_methods.iter().any(|m| m.name == trait_method.name);
            if !provided {
                return Err(TypeError::new(
                    TypeErrorKind::MissingTraitMethod {
                        trait_name: trait_info.name.clone(),
                        method: trait_method.name.clone(),
                    },
                    span,
                ));
            }
        }

        // Check for missing associated types (that don't have defaults)
        for trait_assoc_type in &trait_info.assoc_types {
            if trait_assoc_type.default.is_some() {
                // Has a default, not required
                continue;
            }

            let provided = impl_assoc_types.iter().any(|t| t.name == trait_assoc_type.name);
            if !provided {
                return Err(TypeError::new(
                    TypeErrorKind::MissingAssocType {
                        trait_name: trait_info.name.clone(),
                        type_name: trait_assoc_type.name.clone(),
                    },
                    span,
                ));
            }
        }

        Ok(())
    }

    /// Check if two types match for impl overlap checking.
    #[allow(dead_code)]
    pub(crate) fn types_match_for_impl(&self, ty_a: &Type, ty_b: &Type) -> bool {
        self.impls_could_overlap(ty_a, ty_b)
    }

    /// Extract type substitution when matching a concrete type against an impl's self_ty.
    ///
    /// Given an impl block with generics (e.g., `impl<K, V> HashMap<K, V>`) and a concrete
    /// receiver type (e.g., `HashMap<i32, i32>`), this extracts the substitution
    /// `{K -> i32, V -> i32}`.
    ///
    /// Returns None if the types don't match.
    pub(crate) fn extract_impl_substitution(
        &self,
        impl_generics: &[TyVarId],
        impl_self_ty: &Type,
        concrete_ty: &Type,
    ) -> Option<std::collections::HashMap<TyVarId, Type>> {
        let mut subst: std::collections::HashMap<TyVarId, Type> = std::collections::HashMap::new();

        // Helper function to recursively extract substitution
        fn extract_inner(
            impl_generics: &[TyVarId],
            pattern: &Type,
            concrete: &Type,
            subst: &mut std::collections::HashMap<TyVarId, Type>,
        ) -> bool {
            match (pattern.kind(), concrete.kind()) {
                // Type parameter: check if it's one of the impl's generics
                (TypeKind::Param(var_id), _) => {
                    if impl_generics.contains(var_id) {
                        // Record the substitution
                        if let Some(existing) = subst.get(var_id) {
                            // Must match existing binding
                            existing == concrete
                        } else {
                            subst.insert(*var_id, concrete.clone());
                            true
                        }
                    } else {
                        // Not an impl generic - types must match exactly
                        pattern == concrete
                    }
                }

                // ADT: check def_id matches and recursively check type args
                (
                    TypeKind::Adt { def_id: p_id, args: p_args },
                    TypeKind::Adt { def_id: c_id, args: c_args },
                ) => {
                    if p_id != c_id || p_args.len() != c_args.len() {
                        return false;
                    }
                    p_args.iter().zip(c_args.iter()).all(|(p, c)| {
                        extract_inner(impl_generics, p, c, subst)
                    })
                }

                // Reference types
                (
                    TypeKind::Ref { mutable: p_mut, inner: p_inner },
                    TypeKind::Ref { mutable: c_mut, inner: c_inner },
                ) => {
                    p_mut == c_mut && extract_inner(impl_generics, p_inner, c_inner, subst)
                }

                // Tuple types
                (TypeKind::Tuple(p_elems), TypeKind::Tuple(c_elems)) => {
                    p_elems.len() == c_elems.len()
                        && p_elems.iter().zip(c_elems.iter()).all(|(p, c)| {
                            extract_inner(impl_generics, p, c, subst)
                        })
                }

                // Primitives must match exactly
                (TypeKind::Primitive(p), TypeKind::Primitive(c)) => p == c,

                // Other types must match exactly
                _ => pattern == concrete,
            }
        }

        if extract_inner(impl_generics, impl_self_ty, concrete_ty, &mut subst) {
            Some(subst)
        } else {
            None
        }
    }
}
