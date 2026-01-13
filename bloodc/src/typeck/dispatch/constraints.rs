//! Constraint checking for generic dispatch.
//!
//! When a generic method has constraints on its type parameters
//! (e.g., `fn sort<T: Ord>(list: Vec<T>) -> Vec<T>`), we must
//! verify that the concrete types inferred for each parameter
//! actually satisfy those constraints.
//!
//! See DISPATCH.md Section 9 for full specification.

use std::collections::HashMap;

use crate::hir::{Type, TypeKind, PrimitiveTy, TyVarId};

use super::types::{TypeParam, Constraint, ConstraintError};

/// A function that checks if a type satisfies a named trait constraint.
///
/// This callback allows the constraint checker to query whether a concrete
/// type implements a trait. The function takes:
/// - The concrete type to check
/// - The trait name (from the constraint)
///
/// Returns `true` if the type satisfies the constraint.
pub type TraitConstraintChecker = dyn Fn(&Type, &str) -> bool;

/// Checker for type parameter constraints during generic instantiation.
///
/// When a generic method is instantiated with concrete types, this checker
/// verifies that all type parameter constraints are satisfied.
///
/// # Example
///
/// For `fn sort<T: Ord>(list: Vec<T>)` called with `Vec<i32>`:
/// - T is instantiated to i32
/// - The constraint `T: Ord` requires i32 to implement Ord
/// - The constraint checker verifies this holds
///
/// # Algorithm
///
/// For each type parameter with constraints:
/// 1. Look up the concrete type from the substitution map
/// 2. For each constraint on the parameter, check if the concrete type satisfies it
/// 3. Return an error for the first unsatisfied constraint
pub struct ConstraintChecker<'a> {
    /// Optional callback for checking trait implementations.
    ///
    /// If provided, this is used to check if a type implements a trait.
    /// If not provided, built-in constraints (Copy, Clone, Sized, Send, Sync, Ord, Eq, etc.)
    /// are checked using heuristics.
    trait_checker: Option<&'a TraitConstraintChecker>,
}

impl<'a> ConstraintChecker<'a> {
    /// Create a new constraint checker without trait implementation checking.
    ///
    /// Uses built-in heuristics for well-known traits.
    pub fn new() -> Self {
        Self {
            trait_checker: None,
        }
    }

    /// Create a constraint checker with a custom trait implementation checker.
    ///
    /// The trait_checker function is called to determine if a type implements
    /// a given trait by name.
    pub fn with_trait_checker(trait_checker: &'a TraitConstraintChecker) -> Self {
        Self {
            trait_checker: Some(trait_checker),
        }
    }

    /// Check if all type parameter constraints are satisfied by the given substitutions.
    ///
    /// Takes:
    /// - `type_params`: The type parameters with their constraints
    /// - `substitutions`: Map from type parameter ID to concrete type
    ///
    /// Returns:
    /// - `Ok(())` if all constraints are satisfied
    /// - `Err(ConstraintError)` for the first unsatisfied constraint
    pub fn check_constraints(
        &self,
        type_params: &[TypeParam],
        substitutions: &HashMap<TyVarId, Type>,
    ) -> Result<(), ConstraintError> {
        for param in type_params {
            // Skip parameters with no constraints
            if param.constraints.is_empty() {
                continue;
            }

            // Get the concrete type for this parameter
            let Some(concrete_type) = substitutions.get(&param.id) else {
                // No substitution for this parameter - skip checking
                // (this shouldn't happen in a well-formed instantiation)
                continue;
            };

            // Check each constraint
            for constraint in &param.constraints {
                if !self.type_satisfies_constraint(concrete_type, constraint) {
                    return Err(ConstraintError {
                        param_name: param.name.clone(),
                        param_id: param.id,
                        concrete_type: concrete_type.clone(),
                        constraint: constraint.clone(),
                    });
                }
            }
        }

        Ok(())
    }

    /// Check if a concrete type satisfies a constraint.
    ///
    /// Checks if the given type implements the trait specified by the constraint.
    fn type_satisfies_constraint(&self, ty: &Type, constraint: &Constraint) -> bool {
        // If we have a custom trait checker, use it
        if let Some(checker) = self.trait_checker {
            return checker(ty, &constraint.trait_name);
        }

        // Otherwise, use built-in heuristics for well-known traits
        self.builtin_constraint_check(ty, &constraint.trait_name)
    }

    /// Check constraints using built-in heuristics for well-known traits.
    ///
    /// This provides a reasonable default for common traits without requiring
    /// full trait resolution infrastructure.
    fn builtin_constraint_check(&self, ty: &Type, trait_name: &str) -> bool {
        match trait_name {
            // Copy: primitives, shared refs, raw pointers, small tuples, arrays of Copy
            "Copy" => self.type_is_copy(ty),

            // Clone: everything that is Copy, plus more
            "Clone" => self.type_is_clone(ty),

            // Sized: almost everything except [T], str, dyn Trait
            "Sized" => self.type_is_sized(ty),

            // Send: most types can be sent across threads
            "Send" => self.type_is_send(ty),

            // Sync: most types can be shared via references
            "Sync" => self.type_is_sync(ty),

            // Ord: types that have a total ordering (primitives, tuples of Ord, etc.)
            "Ord" | "PartialOrd" => self.type_is_ord(ty),

            // Eq: types that have equality (primitives, tuples of Eq, etc.)
            "Eq" | "PartialEq" => self.type_is_eq(ty),

            // Hash: types that can be hashed
            "Hash" => self.type_is_hash(ty),

            // Default: types that have a default value
            "Default" => self.type_is_default(ty),

            // Debug, Display: for now, assume primitives satisfy these
            "Debug" | "Display" => self.type_is_debug(ty),

            // Unknown trait - conservatively return false
            _ => false,
        }
    }

    /// Check if a type implements Copy.
    fn type_is_copy(&self, ty: &Type) -> bool {
        match ty.kind.as_ref() {
            TypeKind::Primitive(_) => true,
            TypeKind::Ref { mutable: false, .. } => true,
            TypeKind::Ref { mutable: true, .. } => false, // &mut T is not Copy
            TypeKind::Ptr { .. } => true,
            TypeKind::Fn { .. } => true,
            TypeKind::Never => true,
            TypeKind::Array { element, .. } => self.type_is_copy(element),
            TypeKind::Tuple(elements) => elements.iter().all(|e| self.type_is_copy(e)),
            TypeKind::Range { element, .. } => self.type_is_copy(element),
            TypeKind::Slice { .. } => false, // Unsized
            TypeKind::Closure { .. } => false,
            TypeKind::Adt { .. } => false, // Requires explicit impl
            TypeKind::DynTrait { .. } => false, // Unsized
            TypeKind::Error => true, // Be permissive for error recovery
            TypeKind::Infer(_) | TypeKind::Param(_) => false,
            TypeKind::Record { fields, .. } => fields.iter().all(|f| self.type_is_copy(&f.ty)),
            TypeKind::Ownership { inner, .. } => self.type_is_copy(inner),
            TypeKind::Forall { .. } => false,
        }
    }

    /// Check if a type implements Clone.
    fn type_is_clone(&self, ty: &Type) -> bool {
        // Everything that is Copy is also Clone
        if self.type_is_copy(ty) {
            return true;
        }

        match ty.kind.as_ref() {
            // Slices can be cloned if element is Clone
            TypeKind::Slice { element } => self.type_is_clone(element),
            // ADTs might implement Clone (can't know without trait resolution)
            // For heuristics, be conservative
            TypeKind::Adt { .. } => false,
            // Closures might implement Clone
            TypeKind::Closure { .. } => false,
            // Everything else inherits from Copy check
            _ => false,
        }
    }

    /// Check if a type implements Sized.
    fn type_is_sized(&self, ty: &Type) -> bool {
        match ty.kind.as_ref() {
            TypeKind::Slice { .. } => false,   // [T] is !Sized
            TypeKind::DynTrait { .. } => false, // dyn Trait is !Sized
            // str is unsized (it's a string slice type)
            TypeKind::Primitive(PrimitiveTy::Str) => false,
            TypeKind::Error => true, // Permissive
            // Everything else is Sized
            _ => true,
        }
    }

    /// Check if a type implements Send.
    fn type_is_send(&self, ty: &Type) -> bool {
        match ty.kind.as_ref() {
            TypeKind::Primitive(_) => true,
            TypeKind::Ref { inner, .. } => self.type_is_send(inner) && self.type_is_sync(inner),
            TypeKind::Ptr { inner, .. } => self.type_is_send(inner),
            TypeKind::Array { element, .. } => self.type_is_send(element),
            TypeKind::Slice { element } => self.type_is_send(element),
            TypeKind::Tuple(elements) => elements.iter().all(|e| self.type_is_send(e)),
            TypeKind::Fn { .. } => true,
            TypeKind::Never => true,
            TypeKind::Error => true,
            // ADTs and closures: conservative false
            TypeKind::Adt { .. } => false,
            TypeKind::Closure { .. } => false,
            TypeKind::DynTrait { .. } => false,
            TypeKind::Infer(_) | TypeKind::Param(_) => false,
            TypeKind::Range { element, .. } => self.type_is_send(element),
            TypeKind::Record { fields, .. } => fields.iter().all(|f| self.type_is_send(&f.ty)),
            TypeKind::Ownership { inner, .. } => self.type_is_send(inner),
            TypeKind::Forall { body, .. } => self.type_is_send(body),
        }
    }

    /// Check if a type implements Sync.
    fn type_is_sync(&self, ty: &Type) -> bool {
        match ty.kind.as_ref() {
            TypeKind::Primitive(_) => true,
            TypeKind::Ref { inner, .. } => self.type_is_sync(inner),
            TypeKind::Ptr { inner, .. } => self.type_is_sync(inner),
            TypeKind::Array { element, .. } => self.type_is_sync(element),
            TypeKind::Slice { element } => self.type_is_sync(element),
            TypeKind::Tuple(elements) => elements.iter().all(|e| self.type_is_sync(e)),
            TypeKind::Fn { .. } => true,
            TypeKind::Never => true,
            TypeKind::Error => true,
            TypeKind::Adt { .. } => false,
            TypeKind::Closure { .. } => false,
            TypeKind::DynTrait { .. } => false,
            TypeKind::Infer(_) | TypeKind::Param(_) => false,
            TypeKind::Range { element, .. } => self.type_is_sync(element),
            TypeKind::Record { fields, .. } => fields.iter().all(|f| self.type_is_sync(&f.ty)),
            TypeKind::Ownership { inner, .. } => self.type_is_sync(inner),
            TypeKind::Forall { body, .. } => self.type_is_sync(body),
        }
    }

    /// Check if a type implements Ord.
    fn type_is_ord(&self, ty: &Type) -> bool {
        match ty.kind.as_ref() {
            TypeKind::Primitive(prim) => match prim {
                PrimitiveTy::Bool => true,
                PrimitiveTy::Char => true,
                PrimitiveTy::Int(_) => true,
                PrimitiveTy::Uint(_) => true,
                PrimitiveTy::Float(_) => false, // Floats only have PartialOrd due to NaN
                PrimitiveTy::Str => true,
                PrimitiveTy::String => true,
                PrimitiveTy::Unit => true,
            },
            // Tuples are Ord if all elements are Ord
            TypeKind::Tuple(elements) => elements.iter().all(|e| self.type_is_ord(e)),
            // Arrays are Ord if element is Ord
            TypeKind::Array { element, .. } => self.type_is_ord(element),
            // Slices are Ord if element is Ord
            TypeKind::Slice { element } => self.type_is_ord(element),
            // References inherit Ord from inner type
            TypeKind::Ref { inner, .. } => self.type_is_ord(inner),
            // Other types conservatively don't have Ord
            _ => false,
        }
    }

    /// Check if a type implements Eq.
    fn type_is_eq(&self, ty: &Type) -> bool {
        match ty.kind.as_ref() {
            TypeKind::Primitive(prim) => match prim {
                PrimitiveTy::Bool => true,
                PrimitiveTy::Char => true,
                PrimitiveTy::Int(_) => true,
                PrimitiveTy::Uint(_) => true,
                PrimitiveTy::Float(_) => false, // Floats only have PartialEq due to NaN
                PrimitiveTy::Str => true,
                PrimitiveTy::String => true,
                PrimitiveTy::Unit => true,
            },
            TypeKind::Tuple(elements) => elements.iter().all(|e| self.type_is_eq(e)),
            TypeKind::Array { element, .. } => self.type_is_eq(element),
            TypeKind::Slice { element } => self.type_is_eq(element),
            TypeKind::Ref { inner, .. } => self.type_is_eq(inner),
            _ => false,
        }
    }

    /// Check if a type implements Hash.
    fn type_is_hash(&self, ty: &Type) -> bool {
        // Most Eq types can be hashed
        match ty.kind.as_ref() {
            TypeKind::Primitive(prim) => match prim {
                PrimitiveTy::Bool => true,
                PrimitiveTy::Char => true,
                PrimitiveTy::Int(_) => true,
                PrimitiveTy::Uint(_) => true,
                PrimitiveTy::Float(_) => false, // Floats don't implement Hash
                PrimitiveTy::Str => true,
                PrimitiveTy::String => true,
                PrimitiveTy::Unit => true,
            },
            TypeKind::Tuple(elements) => elements.iter().all(|e| self.type_is_hash(e)),
            TypeKind::Array { element, .. } => self.type_is_hash(element),
            TypeKind::Slice { element } => self.type_is_hash(element),
            TypeKind::Ref { inner, .. } => self.type_is_hash(inner),
            _ => false,
        }
    }

    /// Check if a type implements Default.
    fn type_is_default(&self, ty: &Type) -> bool {
        match ty.kind.as_ref() {
            TypeKind::Primitive(prim) => match prim {
                PrimitiveTy::Bool => true, // false
                PrimitiveTy::Int(_) => true, // 0
                PrimitiveTy::Uint(_) => true, // 0
                PrimitiveTy::Float(_) => true, // 0.0
                PrimitiveTy::Char => true, // '\0'
                PrimitiveTy::Unit => true, // ()
                PrimitiveTy::Str => false, // &str doesn't have Default
                PrimitiveTy::String => true, // String::new()
            },
            TypeKind::Tuple(elements) => elements.iter().all(|e| self.type_is_default(e)),
            TypeKind::Array { element, .. } => self.type_is_default(element),
            _ => false,
        }
    }

    /// Check if a type implements Debug.
    fn type_is_debug(&self, ty: &Type) -> bool {
        // Most primitives implement Debug
        match ty.kind.as_ref() {
            TypeKind::Primitive(_) => true,
            TypeKind::Tuple(elements) => elements.iter().all(|e| self.type_is_debug(e)),
            TypeKind::Array { element, .. } => self.type_is_debug(element),
            TypeKind::Slice { element } => self.type_is_debug(element),
            TypeKind::Ref { inner, .. } => self.type_is_debug(inner),
            TypeKind::Never => true,
            _ => false,
        }
    }
}

impl Default for ConstraintChecker<'_> {
    fn default() -> Self {
        Self::new()
    }
}
