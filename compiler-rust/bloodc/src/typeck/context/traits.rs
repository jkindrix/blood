//! Trait bounds checking and coherence validation.

use std::collections::HashMap;

use crate::ast;
use crate::hir::{self, DefId, Type, TypeKind, TyVarId};
use crate::span::Span;

use super::{TypeContext, ImplMethodInfo, ImplAssocTypeInfo};
use super::super::error::{TypeError, TypeErrorKind};

impl<'a> TypeContext<'a> {
    /// Check if a type satisfies all trait bounds required by a type parameter.
    pub(crate) fn check_trait_bounds(
        &self,
        ty: &Type,
        bounds: &[DefId],
        span: Span,
    ) -> Result<(), Box<TypeError>> {
        for &trait_def_id in bounds {
            if !self.type_implements_trait(ty, trait_def_id) {
                let trait_name = self.trait_defs.get(&trait_def_id)
                    .map(|info| info.name.clone())
                    .unwrap_or_else(|| format!("{:?}", trait_def_id));
                return Err(Box::new(TypeError::new(
                    TypeErrorKind::TraitBoundNotSatisfied {
                        ty: ty.clone(),
                        trait_name,
                    },
                    span,
                )));
            }
        }
        Ok(())
    }

    /// Check if a type implements a trait.
    ///
    /// Checks explicit impl blocks first, then built-in trait implementations.
    /// Note: User-defined traits are only satisfied by explicit impl blocks,
    /// NOT by builtin impls (even if they have the same name as builtin traits).
    pub(crate) fn type_implements_trait(&self, ty: &Type, trait_def_id: DefId) -> bool {
        // Check explicit impl blocks first
        for impl_block in &self.impl_blocks {
            if impl_block.trait_ref == Some(trait_def_id) && impl_block.self_ty == *ty {
                return true;
            }
        }

        // If the trait is defined in trait_defs, it's a user-defined trait.
        // User-defined traits can ONLY be satisfied by explicit impl blocks,
        // NOT by builtin impls. This prevents false positives when a user
        // defines a trait with the same name as a builtin trait (e.g., Display).
        if self.trait_defs.contains_key(&trait_def_id) {
            // User-defined trait - no impl block found, so return false
            return false;
        }

        // For builtin traits (not in trait_defs), check builtin implementations
        // Get the trait name from def_info
        if let Some(def_info) = self.resolver.def_info.get(&trait_def_id) {
            return self.type_has_builtin_impl(ty, &def_info.name);
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
            "Fn" => self.type_is_fn(ty),
            "FnMut" => self.type_is_fn_mut(ty),
            "FnOnce" => self.type_is_fn_once(ty),
            "Default" => self.type_is_default(ty),
            // Operator traits
            "Add" | "Sub" | "Mul" | "Div" | "Rem" => self.type_is_numeric(ty),
            "Neg" => self.type_is_signed_numeric(ty),
            "Not" => self.type_is_bool_or_integer(ty),
            "BitAnd" | "BitOr" | "BitXor" | "Shl" | "Shr" => self.type_is_integer(ty),
            // Comparison traits
            "PartialEq" | "Eq" => self.type_is_partial_eq(ty),
            "PartialOrd" | "Ord" => self.type_is_partial_ord(ty),
            // Other core traits
            "Hash" => self.type_is_hashable(ty),
            "Debug" | "Display" => self.type_is_display(ty),
            // Drop and conversion traits
            "Drop" => self.type_is_drop(ty),
            "From" | "Into" => self.type_is_convertible(ty),
            "Deref" | "DerefMut" => self.type_is_deref(ty),
            // Iterator traits
            "Iterator" => self.type_is_iterator(ty),
            "IntoIterator" => self.type_is_into_iterator(ty),
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
            // Ownership-qualified types are NOT Copy (substructural semantics)
            TypeKind::Ownership { .. } => false,
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
    ) -> Result<(), Box<TypeError>> {
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
                return Err(Box::new(TypeError::new(
                    TypeErrorKind::MissingTraitMethod {
                        trait_name: trait_info.name.clone(),
                        method: trait_method.name.clone(),
                    },
                    span,
                )));
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
                return Err(Box::new(TypeError::new(
                    TypeErrorKind::MissingAssocType {
                        trait_name: trait_info.name.clone(),
                        type_name: trait_assoc_type.name.clone(),
                    },
                    span,
                )));
            }
        }

        Ok(())
    }

    /// Check if two types match for impl overlap checking.
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

    /// Check if two types are compatible for method overload resolution.
    ///
    /// This is used to distinguish between overloaded methods with the same name
    /// but different parameter types. It's more lenient than exact equality
    /// (allows inference variables) but stricter than unification (doesn't modify state).
    ///
    /// Returns true if `arg_ty` could match `param_ty` for overload selection.
    pub(crate) fn types_compatible_for_overload(&self, arg_ty: &Type, param_ty: &Type) -> bool {
        // Resolve any inference variables to their current bindings
        let arg_resolved = self.unifier.resolve(arg_ty);
        let param_resolved = self.unifier.resolve(param_ty);

        // If either is an unresolved inference variable, consider it compatible
        // (it could potentially unify with anything)
        if matches!(arg_resolved.kind(), TypeKind::Infer(_)) {
            return true;
        }
        if matches!(param_resolved.kind(), TypeKind::Infer(_)) {
            return true;
        }

        // Type parameters are compatible (generics can match anything)
        if matches!(param_resolved.kind(), TypeKind::Param(_)) {
            return true;
        }

        // For concrete types, check structural compatibility
        match (arg_resolved.kind(), param_resolved.kind()) {
            // Primitives must match exactly
            (TypeKind::Primitive(a), TypeKind::Primitive(b)) => a == b,

            // ADTs must have the same def_id (type args are checked separately during unification)
            (TypeKind::Adt { def_id: a_id, .. }, TypeKind::Adt { def_id: b_id, .. }) => a_id == b_id,

            // References: check inner type compatibility
            (TypeKind::Ref { inner: a_inner, .. }, TypeKind::Ref { inner: b_inner, .. }) => {
                self.types_compatible_for_overload(a_inner, b_inner)
            }

            // Tuples: check element compatibility
            (TypeKind::Tuple(a_elems), TypeKind::Tuple(b_elems)) => {
                a_elems.len() == b_elems.len()
                    && a_elems.iter().zip(b_elems.iter()).all(|(a, b)| {
                        self.types_compatible_for_overload(a, b)
                    })
            }

            // Function types
            (TypeKind::Fn { params: a_params, ret: a_ret, .. },
             TypeKind::Fn { params: b_params, ret: b_ret, .. }) => {
                a_params.len() == b_params.len()
                    && a_params.iter().zip(b_params.iter()).all(|(a, b)| {
                        self.types_compatible_for_overload(a, b)
                    })
                    && self.types_compatible_for_overload(a_ret, b_ret)
            }

            // Other type kinds must match exactly for now
            _ => arg_resolved.kind() == param_resolved.kind(),
        }
    }

    /// Check if a type implements Fn (callable by shared reference).
    ///
    /// Fn types:
    /// - Function pointers: fn(A, B) -> C
    /// - Closures that only capture by shared reference
    ///
    /// Note: This is a simplified check that doesn't verify the exact signature.
    /// Signature compatibility is handled through type unification.
    pub(crate) fn type_is_fn(&self, ty: &Type) -> bool {
        match ty.kind() {
            // Function pointers implement Fn
            TypeKind::Fn { .. } => true,
            // Closures implement Fn (simplified - full implementation would check captures)
            TypeKind::Closure { .. } => true,
            // References to Fn types are Fn
            TypeKind::Ref { inner, mutable: false } => self.type_is_fn(inner),
            _ => false,
        }
    }

    /// Check if a type implements FnMut (callable by mutable reference).
    ///
    /// FnMut types:
    /// - Everything that implements Fn
    /// - Closures that capture by mutable reference
    pub(crate) fn type_is_fn_mut(&self, ty: &Type) -> bool {
        match ty.kind() {
            // Function pointers implement FnMut
            TypeKind::Fn { .. } => true,
            // Closures implement FnMut (simplified)
            TypeKind::Closure { .. } => true,
            // References to FnMut types
            TypeKind::Ref { inner, .. } => self.type_is_fn_mut(inner),
            _ => false,
        }
    }

    /// Check if a type implements FnOnce (callable by value).
    ///
    /// FnOnce types:
    /// - Everything that implements FnMut
    /// - Closures that move captured values
    pub(crate) fn type_is_fn_once(&self, ty: &Type) -> bool {
        match ty.kind() {
            // Function pointers implement FnOnce
            TypeKind::Fn { .. } => true,
            // All closures implement FnOnce
            TypeKind::Closure { .. } => true,
            // References to FnOnce types
            TypeKind::Ref { inner, .. } => self.type_is_fn_once(inner),
            _ => false,
        }
    }

    /// Check if a type implements Default.
    ///
    /// Default types:
    /// - Numeric primitives (default to 0)
    /// - bool (defaults to false)
    /// - char (defaults to '\0')
    /// - Unit type
    /// - Option<T> (defaults to None)
    /// - Vec<T> (defaults to empty vec)
    /// - String (defaults to empty string)
    pub(crate) fn type_is_default(&self, ty: &Type) -> bool {
        match ty.kind() {
            // Numeric primitives default to 0
            TypeKind::Primitive(prim) => matches!(
                prim,
                hir::PrimitiveTy::Int(_)
                    | hir::PrimitiveTy::Uint(_)
                    | hir::PrimitiveTy::Float(_)
                    | hir::PrimitiveTy::Bool
                    | hir::PrimitiveTy::Char
                    | hir::PrimitiveTy::Unit
                    | hir::PrimitiveTy::String
            ),
            // Unit type has Default
            TypeKind::Tuple(elems) if elems.is_empty() => true,
            // Tuples have Default if all elements do
            TypeKind::Tuple(elems) => elems.iter().all(|e| self.type_is_default(e)),
            // Arrays have Default if element does (and size is known)
            TypeKind::Array { element, .. } => self.type_is_default(element),
            // Option<T> has Default (None)
            TypeKind::Adt { def_id, .. } if Some(*def_id) == self.option_def_id => true,
            // Vec<T> has Default (empty vec)
            TypeKind::Adt { def_id, .. } if Some(*def_id) == self.vec_def_id => true,
            _ => false,
        }
    }

    /// Check if a type is a numeric type (integer, unsigned integer, or float).
    ///
    /// Numeric types support Add, Sub, Mul, Div, Rem operations.
    pub(crate) fn type_is_numeric(&self, ty: &Type) -> bool {
        match ty.kind() {
            TypeKind::Primitive(prim) => matches!(
                prim,
                hir::PrimitiveTy::Int(_)
                    | hir::PrimitiveTy::Uint(_)
                    | hir::PrimitiveTy::Float(_)
            ),
            TypeKind::Ref { inner, .. } => self.type_is_numeric(inner),
            _ => false,
        }
    }

    /// Check if a type is a signed numeric type (integer or float).
    ///
    /// Signed numeric types support the Neg (unary minus) operation.
    pub(crate) fn type_is_signed_numeric(&self, ty: &Type) -> bool {
        match ty.kind() {
            TypeKind::Primitive(prim) => matches!(
                prim,
                hir::PrimitiveTy::Int(_) | hir::PrimitiveTy::Float(_)
            ),
            TypeKind::Ref { inner, .. } => self.type_is_signed_numeric(inner),
            _ => false,
        }
    }

    /// Check if a type is bool or an integer type.
    ///
    /// These types support the Not (logical/bitwise negation) operation.
    pub(crate) fn type_is_bool_or_integer(&self, ty: &Type) -> bool {
        match ty.kind() {
            TypeKind::Primitive(prim) => matches!(
                prim,
                hir::PrimitiveTy::Bool
                    | hir::PrimitiveTy::Int(_)
                    | hir::PrimitiveTy::Uint(_)
            ),
            TypeKind::Ref { inner, .. } => self.type_is_bool_or_integer(inner),
            _ => false,
        }
    }

    /// Check if a type is an integer type (signed or unsigned).
    ///
    /// Integer types support bitwise operations (BitAnd, BitOr, BitXor, Shl, Shr).
    pub(crate) fn type_is_integer(&self, ty: &Type) -> bool {
        match ty.kind() {
            TypeKind::Primitive(prim) => matches!(
                prim,
                hir::PrimitiveTy::Int(_) | hir::PrimitiveTy::Uint(_)
            ),
            TypeKind::Ref { inner, .. } => self.type_is_integer(inner),
            _ => false,
        }
    }

    /// Check if a type implements PartialEq.
    ///
    /// Types that support == and != comparisons.
    pub(crate) fn type_is_partial_eq(&self, ty: &Type) -> bool {
        match ty.kind() {
            // All primitives support equality
            TypeKind::Primitive(_) => true,
            // References are PartialEq if referent is
            TypeKind::Ref { inner, .. } => self.type_is_partial_eq(inner),
            // Tuples are PartialEq if all elements are
            TypeKind::Tuple(elems) => elems.iter().all(|e| self.type_is_partial_eq(e)),
            // Arrays are PartialEq if element is
            TypeKind::Array { element, .. } => self.type_is_partial_eq(element),
            // Option<T> is PartialEq if T is
            TypeKind::Adt { def_id, args } if Some(*def_id) == self.option_def_id => {
                args.first().map(|t| self.type_is_partial_eq(t)).unwrap_or(true)
            }
            // Result<T, E> is PartialEq if T and E are
            TypeKind::Adt { def_id, args } if Some(*def_id) == self.result_def_id => {
                args.iter().all(|t| self.type_is_partial_eq(t))
            }
            // Vec<T> is PartialEq if T is
            TypeKind::Adt { def_id, args } if Some(*def_id) == self.vec_def_id => {
                args.first().map(|t| self.type_is_partial_eq(t)).unwrap_or(true)
            }
            _ => false,
        }
    }

    /// Check if a type implements PartialOrd.
    ///
    /// Types that support <, >, <=, >= comparisons.
    pub(crate) fn type_is_partial_ord(&self, ty: &Type) -> bool {
        match ty.kind() {
            // Numeric and char primitives support ordering
            TypeKind::Primitive(prim) => matches!(
                prim,
                hir::PrimitiveTy::Int(_)
                    | hir::PrimitiveTy::Uint(_)
                    | hir::PrimitiveTy::Float(_)
                    | hir::PrimitiveTy::Char
                    | hir::PrimitiveTy::Bool
            ),
            // References are PartialOrd if referent is
            TypeKind::Ref { inner, .. } => self.type_is_partial_ord(inner),
            // Tuples are PartialOrd if all elements are (lexicographic)
            TypeKind::Tuple(elems) => elems.iter().all(|e| self.type_is_partial_ord(e)),
            // Arrays are PartialOrd if element is
            TypeKind::Array { element, .. } => self.type_is_partial_ord(element),
            _ => false,
        }
    }

    /// Check if a type implements Hash.
    ///
    /// Types that can be hashed for use in HashMap/HashSet.
    pub(crate) fn type_is_hashable(&self, ty: &Type) -> bool {
        match ty.kind() {
            // Most primitives are hashable (except floats - NaN issues)
            TypeKind::Primitive(prim) => matches!(
                prim,
                hir::PrimitiveTy::Int(_)
                    | hir::PrimitiveTy::Uint(_)
                    | hir::PrimitiveTy::Bool
                    | hir::PrimitiveTy::Char
                    | hir::PrimitiveTy::String
                    | hir::PrimitiveTy::Unit
            ),
            // References are hashable if referent is
            TypeKind::Ref { inner, .. } => self.type_is_hashable(inner),
            // Tuples are hashable if all elements are
            TypeKind::Tuple(elems) => elems.iter().all(|e| self.type_is_hashable(e)),
            // Arrays are hashable if element is
            TypeKind::Array { element, .. } => self.type_is_hashable(element),
            _ => false,
        }
    }

    /// Check if a type implements Debug or Display.
    ///
    /// Types that can be formatted for output.
    pub(crate) fn type_is_display(&self, ty: &Type) -> bool {
        match ty.kind() {
            // All primitives implement Debug/Display
            TypeKind::Primitive(_) => true,
            // References implement Debug/Display if referent does
            TypeKind::Ref { inner, .. } => self.type_is_display(inner),
            // Tuples implement Debug if all elements do
            TypeKind::Tuple(elems) => elems.iter().all(|e| self.type_is_display(e)),
            // Arrays implement Debug if element does
            TypeKind::Array { element, .. } => self.type_is_display(element),
            // Option<T> implements Debug/Display if T does
            TypeKind::Adt { def_id, args } if Some(*def_id) == self.option_def_id => {
                args.first().map(|t| self.type_is_display(t)).unwrap_or(true)
            }
            // Result<T, E> implements Debug if T and E do
            TypeKind::Adt { def_id, args } if Some(*def_id) == self.result_def_id => {
                args.iter().all(|t| self.type_is_display(t))
            }
            // Vec<T> implements Debug if T does
            TypeKind::Adt { def_id, args } if Some(*def_id) == self.vec_def_id => {
                args.first().map(|t| self.type_is_display(t)).unwrap_or(true)
            }
            _ => false,
        }
    }

    /// Check if a type implements Iterator.
    ///
    /// Iterator types can be iterated with .next(), .map(), .filter(), etc.
    pub(crate) fn type_is_iterator(&self, ty: &Type) -> bool {
        match ty.kind() {
            // Iter<T> is an iterator
            TypeKind::Adt { def_id, .. } if Some(*def_id) == self.iter_def_id => true,
            // Range types are iterators
            TypeKind::Range { .. } => true,
            // References to iterators are iterators
            TypeKind::Ref { inner, .. } => self.type_is_iterator(inner),
            _ => false,
        }
    }

    /// Check if a type implements IntoIterator.
    ///
    /// Types that can be converted to an iterator with .into_iter().
    pub(crate) fn type_is_into_iterator(&self, ty: &Type) -> bool {
        match ty.kind() {
            // Vec<T> can be converted to iterator
            TypeKind::Adt { def_id, .. } if Some(*def_id) == self.vec_def_id => true,
            // Iter<T> is already an iterator, so it implements IntoIterator
            TypeKind::Adt { def_id, .. } if Some(*def_id) == self.iter_def_id => true,
            // Option<T> can be converted to iterator (0 or 1 elements)
            TypeKind::Adt { def_id, .. } if Some(*def_id) == self.option_def_id => true,
            // Result<T, E> can be converted to iterator (0 or 1 elements based on Ok/Err)
            TypeKind::Adt { def_id, .. } if Some(*def_id) == self.result_def_id => true,
            // Slices can be iterated
            TypeKind::Slice { .. } => true,
            // Arrays can be iterated
            TypeKind::Array { .. } => true,
            // Ranges can be iterated
            TypeKind::Range { .. } => true,
            // References to IntoIterator types
            TypeKind::Ref { inner, .. } => self.type_is_into_iterator(inner),
            _ => false,
        }
    }

    /// Check if a type implements Drop.
    ///
    /// Types with custom destructors or that need cleanup.
    pub(crate) fn type_is_drop(&self, ty: &Type) -> bool {
        match ty.kind() {
            // Vec<T> needs drop to deallocate
            TypeKind::Adt { def_id, .. } if Some(*def_id) == self.vec_def_id => true,
            // Box<T> needs drop to deallocate
            TypeKind::Adt { def_id, .. } if Some(*def_id) == self.box_def_id => true,
            // String needs drop to deallocate
            TypeKind::Primitive(hir::PrimitiveTy::String) => true,
            // Primitives don't need Drop
            TypeKind::Primitive(_) => false,
            // References don't need Drop
            TypeKind::Ref { .. } => false,
            // Raw pointers don't need Drop
            TypeKind::Ptr { .. } => false,
            // Arrays need Drop if element needs Drop
            TypeKind::Array { element, .. } => self.type_is_drop(element),
            // Tuples need Drop if any element needs Drop
            TypeKind::Tuple(elems) => elems.iter().any(|e| self.type_is_drop(e)),
            // Closures may need Drop if they capture owned values
            TypeKind::Closure { .. } => true,
            // Other ADTs may need Drop (conservative - assume they do)
            TypeKind::Adt { .. } => true,
            _ => false,
        }
    }

    /// Check if a type supports From/Into conversions.
    ///
    /// Types that can be converted to other types.
    pub(crate) fn type_is_convertible(&self, ty: &Type) -> bool {
        match ty.kind() {
            // All primitives support some conversions
            TypeKind::Primitive(_) => true,
            // &str can convert to String
            TypeKind::Ref { inner, mutable: false } => {
                matches!(inner.kind(), TypeKind::Primitive(hir::PrimitiveTy::Str))
            }
            // Vec<T> supports From<[T; N]> etc.
            TypeKind::Adt { def_id, .. } if Some(*def_id) == self.vec_def_id => true,
            // Option<T> supports From<T>
            TypeKind::Adt { def_id, .. } if Some(*def_id) == self.option_def_id => true,
            // Result<T, E> supports From conversions
            TypeKind::Adt { def_id, .. } if Some(*def_id) == self.result_def_id => true,
            // Box<T> supports From<T>
            TypeKind::Adt { def_id, .. } if Some(*def_id) == self.box_def_id => true,
            _ => false,
        }
    }

    /// Check if a type implements Deref or DerefMut.
    ///
    /// Types that can be dereferenced.
    pub(crate) fn type_is_deref(&self, ty: &Type) -> bool {
        match ty.kind() {
            // References implement Deref
            TypeKind::Ref { .. } => true,
            // Raw pointers implement Deref (unsafe)
            TypeKind::Ptr { .. } => true,
            // Box<T> implements Deref to T
            TypeKind::Adt { def_id, .. } if Some(*def_id) == self.box_def_id => true,
            // String implements Deref to str
            TypeKind::Primitive(hir::PrimitiveTy::String) => true,
            // Vec<T> implements Deref to [T]
            TypeKind::Adt { def_id, .. } if Some(*def_id) == self.vec_def_id => true,
            _ => false,
        }
    }
}
