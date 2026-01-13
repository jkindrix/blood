//! Multiple dispatch resolution for Blood.
//!
//! This module implements the dispatch resolution algorithm that selects
//! which method implementation to call based on the runtime types of all
//! arguments. Blood uses multiple dispatch similar to Julia, with strict
//! type stability enforcement.
//!
//! # Algorithm Overview
//!
//! 1. **Collect candidates**: Find all methods with matching name and arity
//! 2. **Filter applicable**: Keep methods where each param type matches arg type
//! 3. **Order by specificity**: Rank from most to least specific
//! 4. **Select best**: Choose unique most specific, or error on ambiguity
//!
//! See DISPATCH.md for full specification.

use std::cmp::Ordering;
use std::collections::HashMap;

use crate::hir::{DefId, Type, TypeKind, PrimitiveTy, TyVarId};
use super::unify::Unifier;

/// A method candidate for dispatch resolution.
#[derive(Debug, Clone)]
pub struct MethodCandidate {
    /// The DefId of the method.
    pub def_id: DefId,
    /// The method's name.
    pub name: String,
    /// Parameter types.
    pub param_types: Vec<Type>,
    /// Return type.
    pub return_type: Type,
    /// Type parameters (for generic methods).
    pub type_params: Vec<TypeParam>,
    /// The effect row (if any).
    pub effects: Option<EffectRow>,
    /// The trait this method belongs to (for diamond resolution).
    /// None for inherent methods or free functions.
    pub trait_id: Option<DefId>,
}

/// A type parameter with optional constraints.
#[derive(Debug, Clone)]
pub struct TypeParam {
    /// The type parameter name.
    pub name: String,
    /// The unique ID for this type parameter.
    pub id: TyVarId,
    /// Constraints on the type parameter.
    pub constraints: Vec<Constraint>,
}

/// The result of attempting to instantiate a generic method.
#[derive(Debug, Clone)]
pub enum InstantiationResult {
    /// Successfully instantiated with the given substitutions and candidate.
    Success {
        /// The type substitutions that were inferred.
        substitutions: HashMap<TyVarId, Type>,
        /// The instantiated method candidate with concrete types.
        candidate: MethodCandidate,
    },
    /// Failed to instantiate due to a type mismatch.
    TypeMismatch {
        /// The type parameter that couldn't be matched.
        param_id: TyVarId,
        /// The first type inferred for this parameter.
        expected: Type,
        /// The conflicting type.
        found: Type,
    },
    /// The argument count doesn't match the parameter count.
    ArityMismatch {
        /// Expected number of arguments.
        expected: usize,
        /// Found number of arguments.
        found: usize,
    },
    /// A type parameter constraint was not satisfied.
    ConstraintNotSatisfied(ConstraintError),
}

/// Error indicating a constraint was not satisfied during generic instantiation.
#[derive(Debug, Clone)]
pub struct ConstraintError {
    /// The type parameter whose constraint was violated.
    pub param_name: String,
    /// The type parameter's ID.
    pub param_id: TyVarId,
    /// The concrete type that was inferred for this parameter.
    pub concrete_type: Type,
    /// The constraint that was not satisfied.
    pub constraint: Constraint,
}

/// A constraint on a type parameter.
#[derive(Debug, Clone)]
pub struct Constraint {
    /// The trait that must be implemented.
    pub trait_name: String,
}

/// An effect row for effect-aware dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectRow {
    /// The effects in this row.
    pub effects: Vec<String>,
    /// Whether this is an open row (has a row variable).
    pub is_open: bool,
}

impl EffectRow {
    /// Create a pure (empty, closed) effect row.
    pub fn pure() -> Self {
        Self {
            effects: Vec::new(),
            is_open: false,
        }
    }

    /// Create an effect row with the given effects.
    pub fn with_effects(effects: Vec<String>) -> Self {
        Self {
            effects,
            is_open: false,
        }
    }

    /// Create an open effect row with a row variable.
    pub fn open(effects: Vec<String>) -> Self {
        Self {
            effects,
            is_open: true,
        }
    }

    /// Check if this effect row is pure (no effects and closed).
    pub fn is_pure(&self) -> bool {
        self.effects.is_empty() && !self.is_open
    }

    /// Count the number of concrete effects.
    pub fn effect_count(&self) -> usize {
        self.effects.len()
    }

    /// Check if this row is a subset of another row.
    ///
    /// For effect dispatch compatibility:
    /// - A method with effects {A, B} can be called in a context that handles {A, B, C}
    /// - An open row {A | rho} is compatible with any superset of {A}
    /// - A closed row {A, B} requires exactly those effects to be handled
    pub fn is_subset_of(&self, other: &EffectRow) -> bool {
        // Check that all effects in self are present in other
        for effect in &self.effects {
            if !other.effects.contains(effect) {
                return false;
            }
        }

        // If self is open, other must also be open (to accommodate unknown effects)
        // OR self's concrete effects must all be in other
        if self.is_open && !other.is_open {
            // Open row can only be subset of another open row
            // because the row variable could expand to anything
            return false;
        }

        true
    }

    /// Compare effect specificity between two rows.
    ///
    /// Returns:
    /// - `Ordering::Less` if self is more specific (more restrictive)
    /// - `Ordering::Greater` if other is more specific
    /// - `Ordering::Equal` if they are equally specific
    ///
    /// Specificity rules:
    /// 1. Pure (no effects) is most specific
    /// 2. Closed rows are more specific than open rows
    /// 3. Fewer effects = more specific
    /// 4. For same effect count, compare lexicographically for determinism
    pub fn compare_specificity(&self, other: &EffectRow) -> Ordering {
        // Rule 1: Pure is most specific
        let self_pure = self.is_pure();
        let other_pure = other.is_pure();
        match (self_pure, other_pure) {
            (true, false) => return Ordering::Less,    // self is more specific
            (false, true) => return Ordering::Greater, // other is more specific
            (true, true) => return Ordering::Equal,    // both pure
            (false, false) => {}                        // continue comparison
        }

        // Rule 2: Closed rows are more specific than open rows
        match (self.is_open, other.is_open) {
            (false, true) => return Ordering::Less,    // closed is more specific
            (true, false) => return Ordering::Greater, // open is less specific
            _ => {}                                     // continue comparison
        }

        // Rule 3: Fewer effects = more specific
        match self.effects.len().cmp(&other.effects.len()) {
            Ordering::Less => return Ordering::Less,    // fewer effects = more specific
            Ordering::Greater => return Ordering::Greater,
            Ordering::Equal => {}
        }

        // Rule 4: Same count, compare lexicographically for determinism
        let mut self_sorted: Vec<_> = self.effects.iter().collect();
        let mut other_sorted: Vec<_> = other.effects.iter().collect();
        self_sorted.sort();
        other_sorted.sort();

        for (s, o) in self_sorted.iter().zip(other_sorted.iter()) {
            match s.cmp(o) {
                Ordering::Equal => continue,
                ord => return ord,
            }
        }

        Ordering::Equal
    }
}

/// Result of dispatch resolution.
#[derive(Debug)]
pub enum DispatchResult {
    /// A unique method was found.
    Resolved(MethodCandidate),
    /// No applicable methods found.
    NoMatch(NoMatchError),
    /// Multiple methods are ambiguous.
    Ambiguous(AmbiguityError),
}

/// Error when no method matches the arguments.
#[derive(Debug)]
pub struct NoMatchError {
    /// The method name that was called.
    pub method_name: String,
    /// The argument types provided.
    pub arg_types: Vec<Type>,
    /// All candidates that were considered.
    pub candidates: Vec<MethodCandidate>,
}

/// Error when multiple methods are ambiguous.
#[derive(Debug)]
pub struct AmbiguityError {
    /// The method name that was called.
    pub method_name: String,
    /// The argument types provided.
    pub arg_types: Vec<Type>,
    /// The ambiguous candidates (all maximal).
    pub candidates: Vec<MethodCandidate>,
}

impl AmbiguityError {
    /// Check if this is a diamond conflict (candidates from different traits).
    pub fn is_diamond_conflict(&self) -> bool {
        let trait_ids: Vec<_> = self.candidates
            .iter()
            .filter_map(|c| c.trait_id)
            .collect();

        // Diamond conflict if there are multiple distinct traits involved
        if trait_ids.len() < 2 {
            return false;
        }

        // Check if we have at least 2 different trait IDs
        let first = trait_ids[0];
        trait_ids.iter().any(|&id| id != first)
    }

    /// Get the conflicting trait IDs for diamond resolution.
    pub fn conflicting_trait_ids(&self) -> Vec<DefId> {
        let mut trait_ids: Vec<_> = self.candidates
            .iter()
            .filter_map(|c| c.trait_id)
            .collect();
        trait_ids.sort_by_key(|id| id.index);
        trait_ids.dedup();
        trait_ids
    }

    /// Generate a suggestion message for diamond resolution.
    pub fn diamond_suggestion(&self, trait_names: &HashMap<DefId, String>) -> String {
        let conflicting = self.conflicting_trait_ids();
        let trait_names_str: Vec<_> = conflicting
            .iter()
            .filter_map(|id| trait_names.get(id))
            .cloned()
            .collect();

        if trait_names_str.is_empty() {
            format!(
                "Use qualified syntax to resolve ambiguity: <Type as Trait>::{}()",
                self.method_name
            )
        } else {
            format!(
                "Use qualified syntax: <Type as {}>::{}() or <Type as {}>::{}()",
                trait_names_str.first().unwrap_or(&"Trait1".to_string()),
                self.method_name,
                trait_names_str.get(1).unwrap_or(&"Trait2".to_string()),
                self.method_name
            )
        }
    }
}

/// A function that checks if a type implements a trait.
pub type TraitChecker = dyn Fn(&Type, DefId) -> bool;

/// Dispatch resolution context.
pub struct DispatchResolver<'a> {
    /// Reference to the unifier for subtype checking.
    /// Currently unused but will be used for constraint-based subtyping.
    #[allow(dead_code)]
    unifier: &'a Unifier,
    /// Optional function to check if a type implements a trait.
    /// Required for `T <: dyn Trait` subtyping.
    trait_checker: Option<&'a TraitChecker>,
}

impl<'a> DispatchResolver<'a> {
    /// Create a new dispatch resolver.
    pub fn new(unifier: &'a Unifier) -> Self {
        Self {
            unifier,
            trait_checker: None,
        }
    }

    /// Create a dispatch resolver with trait implementation checking support.
    ///
    /// The trait_checker function should return true if the given type
    /// implements the trait with the given DefId.
    pub fn with_trait_checker(
        unifier: &'a Unifier,
        trait_checker: &'a TraitChecker,
    ) -> Self {
        Self {
            unifier,
            trait_checker: Some(trait_checker),
        }
    }

    /// Resolve dispatch for a method call.
    ///
    /// Given a method name and argument types, finds the unique most specific
    /// applicable method or returns an appropriate error.
    pub fn resolve(
        &self,
        method_name: &str,
        arg_types: &[Type],
        candidates: &[MethodCandidate],
    ) -> DispatchResult {
        // Step 1: Filter to applicable methods
        let applicable: Vec<_> = candidates
            .iter()
            .filter(|m| self.is_applicable(m, arg_types))
            .cloned()
            .collect();

        // Step 2: Handle no matches
        if applicable.is_empty() {
            return DispatchResult::NoMatch(NoMatchError {
                method_name: method_name.to_string(),
                arg_types: arg_types.to_vec(),
                candidates: candidates.to_vec(),
            });
        }

        // Step 3: Find maximally specific methods
        let maximal = self.find_maximal(&applicable);

        // Step 4: Check for unique winner
        if maximal.len() == 1 {
            // SAFETY: We just checked that len == 1, so next() always returns Some
            return DispatchResult::Resolved(maximal.into_iter().next()
                .expect("checked: maximal.len() == 1"));
        }

        // Step 5: Ambiguity error
        DispatchResult::Ambiguous(AmbiguityError {
            method_name: method_name.to_string(),
            arg_types: arg_types.to_vec(),
            candidates: maximal,
        })
    }

    /// Check if a method is applicable to the given argument types.
    ///
    /// A method is applicable if:
    /// - It has the same arity as the arguments
    /// - Each argument type is a subtype of the corresponding parameter type
    /// - For generic methods, type parameters can be successfully instantiated
    /// - The method's effects are compatible with the calling context
    pub fn is_applicable(&self, method: &MethodCandidate, arg_types: &[Type]) -> bool {
        // Check arity
        if method.param_types.len() != arg_types.len() {
            return false;
        }

        // For generic methods, try to instantiate
        if !method.type_params.is_empty() {
            match self.instantiate_generic(method, arg_types) {
                InstantiationResult::Success { candidate, .. } => {
                    // Verify the instantiated types are subtypes
                    for (arg_type, param_type) in arg_types.iter().zip(&candidate.param_types) {
                        if !self.is_subtype(arg_type, param_type) {
                            return false;
                        }
                    }
                    return true;
                }
                InstantiationResult::TypeMismatch { .. } => return false,
                InstantiationResult::ArityMismatch { .. } => return false,
                InstantiationResult::ConstraintNotSatisfied(_) => return false,
            }
        }

        // Check each argument against parameter (non-generic case)
        for (arg_type, param_type) in arg_types.iter().zip(&method.param_types) {
            if !self.is_subtype(arg_type, param_type) {
                return false;
            }
        }

        // Note: Effect compatibility is checked during dispatch resolution
        // when a context is provided. Basic is_applicable only checks type compatibility.
        true
    }

    /// Check if a method is applicable with effect context.
    ///
    /// This is an extended version of is_applicable that also checks
    /// if the method's effects are compatible with the calling context.
    pub fn is_applicable_with_effects(
        &self,
        method: &MethodCandidate,
        arg_types: &[Type],
        context_effects: Option<&EffectRow>,
    ) -> bool {
        // First check basic type applicability
        if !self.is_applicable(method, arg_types) {
            return false;
        }

        // Then check effect compatibility
        self.effects_compatible(&method.effects, context_effects)
    }

    /// Check if one effect row is compatible with another (context).
    ///
    /// A method's effects are compatible with a context if:
    /// - The method is pure (no effects) - always compatible
    /// - The method has no effect annotation (None) - always compatible
    /// - The context can handle all the method's effects
    ///
    /// For open rows (with row variables):
    /// - An open method effect row requires an open context
    ///   (because the method might perform unknown effects)
    pub fn effects_compatible(
        &self,
        method_effects: &Option<EffectRow>,
        context_effects: Option<&EffectRow>,
    ) -> bool {
        match (method_effects, context_effects) {
            // No method effects means pure - always compatible
            (None, _) => true,

            // Method has effects but no context provided - assume compatible
            // (context will be checked at a higher level if needed)
            (Some(_), None) => true,

            // Both have effect rows - check subset relationship
            (Some(method_row), Some(context_row)) => {
                method_row.is_subset_of(context_row)
            }
        }
    }

    /// Compare effect specificity between two methods.
    ///
    /// Returns:
    /// - `Ordering::Less` if m1's effects are more specific (more restrictive)
    /// - `Ordering::Greater` if m2's effects are more specific
    /// - `Ordering::Equal` if they have equally specific effects
    ///
    /// Specificity order: pure > closed row > open row
    /// For rows with same structure: fewer effects > more effects
    pub fn effect_specificity(
        &self,
        m1_effects: &Option<EffectRow>,
        m2_effects: &Option<EffectRow>,
    ) -> Ordering {
        match (m1_effects, m2_effects) {
            // Both None (pure) - equal specificity
            (None, None) => Ordering::Equal,

            // None (pure) is more specific than Some (effectful)
            (None, Some(_)) => Ordering::Less,
            (Some(_), None) => Ordering::Greater,

            // Both have effects - compare the rows
            (Some(row1), Some(row2)) => row1.compare_specificity(row2),
        }
    }

    /// Attempt to instantiate a generic method with concrete argument types.
    ///
    /// This method infers type parameter substitutions from the argument types
    /// and returns an instantiated candidate with all type parameters replaced
    /// by their concrete types.
    ///
    /// # Algorithm
    ///
    /// 1. For each parameter type, recursively match against the corresponding
    ///    argument type, accumulating type parameter substitutions.
    /// 2. If a type parameter is encountered multiple times, verify that all
    ///    inferred types are consistent (equal).
    /// 3. Check that all type parameter constraints are satisfied by the
    ///    inferred concrete types.
    /// 4. Apply substitutions to create the instantiated method candidate.
    pub fn instantiate_generic(
        &self,
        method: &MethodCandidate,
        arg_types: &[Type],
    ) -> InstantiationResult {
        self.instantiate_generic_with_constraint_checker(method, arg_types, None)
    }

    /// Attempt to instantiate a generic method with a custom constraint checker.
    ///
    /// This is the same as `instantiate_generic` but allows providing a custom
    /// constraint checker for trait implementation checking.
    pub fn instantiate_generic_with_constraint_checker(
        &self,
        method: &MethodCandidate,
        arg_types: &[Type],
        constraint_checker: Option<&ConstraintChecker<'_>>,
    ) -> InstantiationResult {
        // Check arity first
        if method.param_types.len() != arg_types.len() {
            return InstantiationResult::ArityMismatch {
                expected: method.param_types.len(),
                found: arg_types.len(),
            };
        }

        // Build a set of valid type parameter IDs for this method
        let valid_params: std::collections::HashSet<TyVarId> =
            method.type_params.iter().map(|p| p.id).collect();

        // Accumulate substitutions
        let mut substitutions: HashMap<TyVarId, Type> = HashMap::new();

        // Match each parameter against its argument
        for (param_type, arg_type) in method.param_types.iter().zip(arg_types) {
            match self.try_match_type_param(param_type, arg_type, &valid_params, &mut substitutions) {
                Ok(()) => continue,
                Err(mismatch) => return mismatch,
            }
        }

        // Check type parameter constraints if any type params have constraints
        let has_constraints = method.type_params.iter().any(|p| !p.constraints.is_empty());
        if has_constraints {
            let default_checker = ConstraintChecker::new();
            let checker = constraint_checker.unwrap_or(&default_checker);
            if let Err(constraint_error) = checker.check_constraints(&method.type_params, &substitutions) {
                return InstantiationResult::ConstraintNotSatisfied(constraint_error);
            }
        }

        // Apply substitutions to create instantiated candidate
        let instantiated_params: Vec<Type> = method
            .param_types
            .iter()
            .map(|t| self.apply_substitutions(t, &substitutions))
            .collect();

        let instantiated_return = self.apply_substitutions(&method.return_type, &substitutions);

        let instantiated_candidate = MethodCandidate {
            def_id: method.def_id,
            name: method.name.clone(),
            param_types: instantiated_params,
            return_type: instantiated_return,
            type_params: vec![], // No longer generic after instantiation
            effects: method.effects.clone(),
            trait_id: method.trait_id, // Preserve trait association for diamond resolution
        };

        InstantiationResult::Success {
            substitutions,
            candidate: instantiated_candidate,
        }
    }

    /// Try to match a parameter type against an argument type, extracting
    /// type parameter substitutions.
    ///
    /// # Return Values
    ///
    /// - `Ok(())` in two cases:
    ///   1. **Substitution extracted**: A type parameter was successfully bound to a concrete type
    ///   2. **Structure mismatch**: The parameter and argument have different type constructors
    ///      (e.g., param is `Ref` but arg is `i32`). In this case, no substitution is extracted
    ///      and type compatibility is deferred to the subsequent subtype checking phase.
    ///
    /// - `Err(InstantiationResult)` if there's a conflicting substitution (same type parameter
    ///   bound to two different types).
    ///
    /// # Why Structure Mismatches Return Ok
    ///
    /// This function only extracts substitutions from structurally matching types. When
    /// structures don't match, returning `Ok(())` is correct because:
    /// 1. This function's job is substitution extraction, not type validation
    /// 2. Subtype checking handles actual type compatibility
    /// 3. A mismatch here doesn't mean the method is invalid - it may still be valid via subtyping
    fn try_match_type_param(
        &self,
        param_type: &Type,
        arg_type: &Type,
        valid_params: &std::collections::HashSet<TyVarId>,
        substitutions: &mut HashMap<TyVarId, Type>,
    ) -> Result<(), InstantiationResult> {
        match param_type.kind.as_ref() {
            // Type parameter: record or verify substitution
            TypeKind::Param(param_id) if valid_params.contains(param_id) => {
                if let Some(existing) = substitutions.get(param_id) {
                    // Already have a substitution - verify consistency
                    if !self.types_equal(existing, arg_type) {
                        return Err(InstantiationResult::TypeMismatch {
                            param_id: *param_id,
                            expected: existing.clone(),
                            found: arg_type.clone(),
                        });
                    }
                } else {
                    // First occurrence - record substitution
                    substitutions.insert(*param_id, arg_type.clone());
                }
                Ok(())
            }

            // Reference types: match inner types with same mutability
            TypeKind::Ref { inner: param_inner, mutable: param_mut } => {
                match arg_type.kind.as_ref() {
                    TypeKind::Ref { inner: arg_inner, mutable: arg_mut } => {
                        // Mutable ref can match immutable param, but not vice versa
                        if !param_mut && !arg_mut || *param_mut && *arg_mut || *arg_mut && !param_mut {
                            self.try_match_type_param(param_inner, arg_inner, valid_params, substitutions)
                        } else {
                            // Immutable ref cannot match mutable param
                            Ok(()) // Let subtype checking handle this
                        }
                    }
                    // Structure mismatch (param is Ref, arg is not): no substitution to extract.
                    // Type compatibility will be validated by subtype checking.
                    _ => Ok(()),
                }
            }

            // Pointer types: match inner types
            TypeKind::Ptr { inner: param_inner, mutable: param_mut } => {
                match arg_type.kind.as_ref() {
                    TypeKind::Ptr { inner: arg_inner, mutable: arg_mut } => {
                        if param_mut == arg_mut {
                            self.try_match_type_param(param_inner, arg_inner, valid_params, substitutions)
                        } else {
                            Ok(())
                        }
                    }
                    _ => Ok(()),
                }
            }

            // Array types: match element types (sizes must match for subtyping)
            TypeKind::Array { element: param_elem, size: param_size } => {
                match arg_type.kind.as_ref() {
                    TypeKind::Array { element: arg_elem, size: arg_size } => {
                        if param_size == arg_size {
                            self.try_match_type_param(param_elem, arg_elem, valid_params, substitutions)
                        } else {
                            Ok(())
                        }
                    }
                    _ => Ok(()),
                }
            }

            // Slice types: match element types
            TypeKind::Slice { element: param_elem } => {
                match arg_type.kind.as_ref() {
                    TypeKind::Slice { element: arg_elem } => {
                        self.try_match_type_param(param_elem, arg_elem, valid_params, substitutions)
                    }
                    _ => Ok(()),
                }
            }

            // Tuple types: match each element
            TypeKind::Tuple(param_elems) => {
                match arg_type.kind.as_ref() {
                    TypeKind::Tuple(arg_elems) if param_elems.len() == arg_elems.len() => {
                        for (p, a) in param_elems.iter().zip(arg_elems.iter()) {
                            self.try_match_type_param(p, a, valid_params, substitutions)?;
                        }
                        Ok(())
                    }
                    _ => Ok(()),
                }
            }

            // Function types: match params and return
            TypeKind::Fn { params: param_params, ret: param_ret } => {
                match arg_type.kind.as_ref() {
                    TypeKind::Fn { params: arg_params, ret: arg_ret }
                        if param_params.len() == arg_params.len() =>
                    {
                        for (p, a) in param_params.iter().zip(arg_params.iter()) {
                            self.try_match_type_param(p, a, valid_params, substitutions)?;
                        }
                        self.try_match_type_param(param_ret, arg_ret, valid_params, substitutions)
                    }
                    _ => Ok(()),
                }
            }

            // ADT types: match type arguments
            TypeKind::Adt { def_id: param_def, args: param_args } => {
                match arg_type.kind.as_ref() {
                    TypeKind::Adt { def_id: arg_def, args: arg_args }
                        if param_def == arg_def && param_args.len() == arg_args.len() =>
                    {
                        for (p, a) in param_args.iter().zip(arg_args.iter()) {
                            self.try_match_type_param(p, a, valid_params, substitutions)?;
                        }
                        Ok(())
                    }
                    _ => Ok(()),
                }
            }

            // Range types: match element types
            TypeKind::Range { element: param_elem, inclusive: param_incl } => {
                match arg_type.kind.as_ref() {
                    TypeKind::Range { element: arg_elem, inclusive: arg_incl }
                        if param_incl == arg_incl =>
                    {
                        self.try_match_type_param(param_elem, arg_elem, valid_params, substitutions)
                    }
                    _ => Ok(()),
                }
            }

            // Record types: match each field type
            TypeKind::Record { fields: param_fields, .. } => {
                match arg_type.kind.as_ref() {
                    TypeKind::Record { fields: arg_fields, .. } if param_fields.len() == arg_fields.len() => {
                        // Match fields by position (assumes same field ordering)
                        for (p, a) in param_fields.iter().zip(arg_fields.iter()) {
                            self.try_match_type_param(&p.ty, &a.ty, valid_params, substitutions)?;
                        }
                        Ok(())
                    }
                    _ => Ok(()),
                }
            }

            // Ownership types: match inner types
            TypeKind::Ownership { inner: param_inner, qualifier: param_qual } => {
                match arg_type.kind.as_ref() {
                    TypeKind::Ownership { inner: arg_inner, qualifier: arg_qual }
                        if param_qual == arg_qual =>
                    {
                        self.try_match_type_param(param_inner, arg_inner, valid_params, substitutions)
                    }
                    _ => Ok(()),
                }
            }

            // Ground types and non-extractable types: no type parameter substitutions possible.
            // These are either:
            // - Concrete types (Primitive, Never) where no generic extraction is needed
            // - Error types from earlier compilation phases
            // - Inference variables handled elsewhere
            // - Params not in valid_params (handled by earlier check)
            // - Complex types (Closure, DynTrait, Forall) that don't participate in simple substitution
            //
            // Returning Ok(()) is correct: no substitution extracted, defer to subtype checking.
            TypeKind::Primitive(_)
            | TypeKind::Never
            | TypeKind::Error
            | TypeKind::Infer(_)
            | TypeKind::Param(_) // Param not in valid_params (handled above when it IS in valid_params)
            | TypeKind::Closure { .. }
            | TypeKind::DynTrait { .. }
            | TypeKind::Forall { .. } => Ok(()),
        }

        // NOTE on structure mismatch cases (all the `_ => Ok(())` above):
        // When the param and arg have different type constructors (e.g., param is Array, arg is i32),
        // we return Ok(()) rather than Err because:
        // 1. This function's purpose is substitution EXTRACTION, not type VALIDATION
        // 2. The actual type compatibility check happens in `is_subtype_of` called later
        // 3. Returning Ok(()) means "no substitutions from this mismatch, but not necessarily invalid"
    }

    /// Apply type parameter substitutions to a type.
    fn apply_substitutions(
        &self,
        ty: &Type,
        substitutions: &HashMap<TyVarId, Type>,
    ) -> Type {
        match ty.kind.as_ref() {
            TypeKind::Param(id) => {
                if let Some(concrete) = substitutions.get(id) {
                    concrete.clone()
                } else {
                    ty.clone()
                }
            }

            TypeKind::Ref { inner, mutable } => {
                let new_inner = self.apply_substitutions(inner, substitutions);
                Type::reference(new_inner, *mutable)
            }

            TypeKind::Ptr { inner, mutable } => {
                let new_inner = self.apply_substitutions(inner, substitutions);
                Type::new(TypeKind::Ptr { inner: new_inner, mutable: *mutable })
            }

            TypeKind::Array { element, size } => {
                let new_elem = self.apply_substitutions(element, substitutions);
                Type::array(new_elem, *size)
            }

            TypeKind::Slice { element } => {
                let new_elem = self.apply_substitutions(element, substitutions);
                Type::slice(new_elem)
            }

            TypeKind::Tuple(elements) => {
                let new_elems: Vec<Type> = elements
                    .iter()
                    .map(|e| self.apply_substitutions(e, substitutions))
                    .collect();
                Type::tuple(new_elems)
            }

            TypeKind::Fn { params, ret } => {
                let new_params: Vec<Type> = params
                    .iter()
                    .map(|p| self.apply_substitutions(p, substitutions))
                    .collect();
                let new_ret = self.apply_substitutions(ret, substitutions);
                Type::function(new_params, new_ret)
            }

            TypeKind::Adt { def_id, args } => {
                let new_args: Vec<Type> = args
                    .iter()
                    .map(|a| self.apply_substitutions(a, substitutions))
                    .collect();
                Type::adt(*def_id, new_args)
            }

            TypeKind::Range { element, inclusive } => {
                let new_elem = self.apply_substitutions(element, substitutions);
                Type::new(TypeKind::Range { element: new_elem, inclusive: *inclusive })
            }

            TypeKind::Closure { def_id, params, ret } => {
                let new_params: Vec<Type> = params
                    .iter()
                    .map(|p| self.apply_substitutions(p, substitutions))
                    .collect();
                let new_ret = self.apply_substitutions(ret, substitutions);
                Type::new(TypeKind::Closure {
                    def_id: *def_id,
                    params: new_params,
                    ret: new_ret,
                })
            }

            TypeKind::DynTrait { trait_id, auto_traits } => {
                Type::dyn_trait(*trait_id, auto_traits.clone())
            }

            TypeKind::Record { fields, row_var } => {
                let new_fields: Vec<crate::hir::RecordField> = fields
                    .iter()
                    .map(|f| crate::hir::RecordField {
                        name: f.name,
                        ty: self.apply_substitutions(&f.ty, substitutions),
                    })
                    .collect();
                Type::record(new_fields, *row_var)
            }

            TypeKind::Forall { params, body } => {
                // Apply substitutions to body, but not to bound params
                let new_body = self.apply_substitutions(body, substitutions);
                Type::new(TypeKind::Forall {
                    params: params.clone(),
                    body: Box::new(new_body),
                })
            }

            TypeKind::Ownership { qualifier, inner } => {
                let new_inner = self.apply_substitutions(inner, substitutions);
                Type::ownership(*qualifier, new_inner)
            }

            // Types that don't contain type parameters
            TypeKind::Primitive(_)
            | TypeKind::Never
            | TypeKind::Error
            | TypeKind::Infer(_) => ty.clone(),
        }
    }

    /// Check if type a is a subtype of type b.
    ///
    /// Implements structural subtyping with variance:
    /// - Covariant positions: &T, [T], arrays - T can be a subtype
    /// - Invariant positions: &mut T - T must be exactly equal
    /// - Contravariant positions: function parameters - reversed subtyping
    fn is_subtype(&self, a: &Type, b: &Type) -> bool {
        // Any type is a subtype of itself
        if self.types_equal(a, b) {
            return true;
        }

        // Never is a subtype of everything (bottom type)
        if matches!(b.kind.as_ref(), TypeKind::Never) {
            return false; // Nothing is subtype of never except never itself
        }
        if matches!(a.kind.as_ref(), TypeKind::Never) {
            return true;
        }

        // Integer promotion: narrower integers are subtypes of wider
        if let (TypeKind::Primitive(pa), TypeKind::Primitive(pb)) =
            (a.kind.as_ref(), b.kind.as_ref())
        {
            if self.primitive_subtype(pa, pb) {
                return true;
            }
        }

        // Type parameter check: if b is a type variable, need to check constraints
        if matches!(b.kind.as_ref(), TypeKind::Infer(_) | TypeKind::Param(_)) {
            // Type variables unify with anything during inference
            return true;
        }

        // Variance rules for compound types
        match (a.kind.as_ref(), b.kind.as_ref()) {
            // Immutable references are covariant: &Cat <: &Animal if Cat <: Animal
            (
                TypeKind::Ref { inner: a_inner, mutable: false },
                TypeKind::Ref { inner: b_inner, mutable: false },
            ) => {
                return self.is_subtype(a_inner, b_inner);
            }

            // Mutable references are invariant: &mut T requires exact match
            (
                TypeKind::Ref { inner: a_inner, mutable: true },
                TypeKind::Ref { inner: b_inner, mutable: true },
            ) => {
                return self.types_equal(a_inner, b_inner);
            }

            // Immutable ref is not subtype of mutable ref
            (
                TypeKind::Ref { mutable: false, .. },
                TypeKind::Ref { mutable: true, .. },
            ) => {
                return false;
            }

            // Mutable ref can be used where immutable ref is expected
            (
                TypeKind::Ref { inner: a_inner, mutable: true },
                TypeKind::Ref { inner: b_inner, mutable: false },
            ) => {
                return self.is_subtype(a_inner, b_inner);
            }

            // Slices are covariant
            (
                TypeKind::Slice { element: a_elem },
                TypeKind::Slice { element: b_elem },
            ) => {
                return self.is_subtype(a_elem, b_elem);
            }

            // Arrays are covariant in element type (same size required)
            (
                TypeKind::Array { element: a_elem, size: a_size },
                TypeKind::Array { element: b_elem, size: b_size },
            ) => {
                return a_size == b_size && self.is_subtype(a_elem, b_elem);
            }

            // Tuples are covariant in each position
            (TypeKind::Tuple(a_elems), TypeKind::Tuple(b_elems)) => {
                return a_elems.len() == b_elems.len()
                    && a_elems.iter().zip(b_elems.iter())
                        .all(|(a, b)| self.is_subtype(a, b));
            }

            // Function types: contravariant in params, covariant in return
            (
                TypeKind::Fn { params: a_params, ret: a_ret },
                TypeKind::Fn { params: b_params, ret: b_ret },
            ) => {
                // Same arity required
                if a_params.len() != b_params.len() {
                    return false;
                }
                // Contravariant in parameters: b_param <: a_param
                let params_ok = a_params.iter().zip(b_params.iter())
                    .all(|(a, b)| self.is_subtype(b, a));
                // Covariant in return type: a_ret <: b_ret
                let ret_ok = self.is_subtype(a_ret, b_ret);
                return params_ok && ret_ok;
            }

            // Pointers follow same variance as references
            (
                TypeKind::Ptr { inner: a_inner, mutable: false },
                TypeKind::Ptr { inner: b_inner, mutable: false },
            ) => {
                return self.is_subtype(a_inner, b_inner);
            }
            (
                TypeKind::Ptr { inner: a_inner, mutable: true },
                TypeKind::Ptr { inner: b_inner, mutable: true },
            ) => {
                return self.types_equal(a_inner, b_inner);
            }

            // Trait object subtyping: dyn Trait1 <: dyn Trait2 if:
            // - Trait1 == Trait2 (same primary trait)
            // - auto_traits of a is a superset of auto_traits of b
            (
                TypeKind::DynTrait { trait_id: a_trait, auto_traits: a_auto },
                TypeKind::DynTrait { trait_id: b_trait, auto_traits: b_auto },
            ) => {
                // Same primary trait required
                if a_trait != b_trait {
                    return false;
                }
                // a must have all auto traits that b has (superset)
                for b_at in b_auto {
                    if !a_auto.contains(b_at) {
                        return false;
                    }
                }
                return true;
            }

            // Other type combinations: fall through to trait-based subtyping check below
            _ => {}
        }

        // Trait-based subtyping: T <: dyn Trait when T: Trait
        // This requires a trait checker to be provided
        if let TypeKind::DynTrait { trait_id, auto_traits } = b.kind.as_ref() {
            if let Some(checker) = self.trait_checker {
                // Check if type 'a' implements the primary trait
                if !checker(a, *trait_id) {
                    return false;
                }
                // Check if type 'a' implements all auto traits
                for auto_trait_id in auto_traits {
                    if !checker(a, *auto_trait_id) {
                        return false;
                    }
                }
                return true;
            }
            // No trait checker available - can't verify trait implementation
            // This is a configuration error, but we conservatively return false
            return false;
        }

        false
    }

    /// Check if primitive type a is a subtype of primitive type b.
    fn primitive_subtype(&self, _a: &PrimitiveTy, _b: &PrimitiveTy) -> bool {
        // For now, no implicit numeric conversions
        // i32 is not a subtype of i64, etc.
        false
    }

    /// Check if two types are structurally equal.
    fn types_equal(&self, a: &Type, b: &Type) -> bool {
        match (a.kind.as_ref(), b.kind.as_ref()) {
            (TypeKind::Primitive(pa), TypeKind::Primitive(pb)) => pa == pb,
            (TypeKind::Tuple(as_), TypeKind::Tuple(bs)) => {
                as_.len() == bs.len()
                    && as_.iter().zip(bs).all(|(a, b)| self.types_equal(a, b))
            }
            (
                TypeKind::Array { element: a_elem, size: a_len },
                TypeKind::Array { element: b_elem, size: b_len },
            ) => a_len == b_len && self.types_equal(a_elem, b_elem),
            (
                TypeKind::Slice { element: a_elem },
                TypeKind::Slice { element: b_elem },
            ) => self.types_equal(a_elem, b_elem),
            (
                TypeKind::Ref { inner: a_inner, mutable: a_mut },
                TypeKind::Ref { inner: b_inner, mutable: b_mut },
            ) => a_mut == b_mut && self.types_equal(a_inner, b_inner),
            (
                TypeKind::Ptr { inner: a_inner, mutable: a_mut },
                TypeKind::Ptr { inner: b_inner, mutable: b_mut },
            ) => a_mut == b_mut && self.types_equal(a_inner, b_inner),
            (
                TypeKind::Fn { params: a_params, ret: a_ret },
                TypeKind::Fn { params: b_params, ret: b_ret },
            ) => {
                a_params.len() == b_params.len()
                    && a_params
                        .iter()
                        .zip(b_params)
                        .all(|(a, b)| self.types_equal(a, b))
                    && self.types_equal(a_ret, b_ret)
            }
            (
                TypeKind::Adt { def_id: a_def, args: a_args },
                TypeKind::Adt { def_id: b_def, args: b_args },
            ) => {
                a_def == b_def
                    && a_args.len() == b_args.len()
                    && a_args.iter().zip(b_args).all(|(a, b)| self.types_equal(a, b))
            }
            (TypeKind::Infer(a_var), TypeKind::Infer(b_var)) => a_var == b_var,
            (TypeKind::Param(a_var), TypeKind::Param(b_var)) => a_var == b_var,
            (TypeKind::Never, TypeKind::Never) => true,
            (TypeKind::Error, TypeKind::Error) => true,
            (
                TypeKind::Closure { def_id: a_def, params: a_params, ret: a_ret },
                TypeKind::Closure { def_id: b_def, params: b_params, ret: b_ret },
            ) => {
                a_def == b_def
                    && a_params.len() == b_params.len()
                    && a_params
                        .iter()
                        .zip(b_params)
                        .all(|(a, b)| self.types_equal(a, b))
                    && self.types_equal(a_ret, b_ret)
            }
            (
                TypeKind::Range { element: a_elem, inclusive: a_incl },
                TypeKind::Range { element: b_elem, inclusive: b_incl },
            ) => a_incl == b_incl && self.types_equal(a_elem, b_elem),
            // DynTrait equality: same trait_id and same auto_traits (order independent)
            (
                TypeKind::DynTrait { trait_id: a_trait, auto_traits: a_auto },
                TypeKind::DynTrait { trait_id: b_trait, auto_traits: b_auto },
            ) => {
                a_trait == b_trait
                    && a_auto.len() == b_auto.len()
                    && a_auto.iter().all(|a| b_auto.contains(a))
            }
            _ => false,
        }
    }

    /// Find the maximally specific methods from the applicable set.
    ///
    /// A method is maximal if no other method is strictly more specific.
    fn find_maximal(&self, applicable: &[MethodCandidate]) -> Vec<MethodCandidate> {
        let mut maximal = Vec::new();

        for m in applicable {
            let is_maximal = !applicable.iter().any(|other| {
                !std::ptr::eq(m, other) && self.is_more_specific(other, m)
            });

            if is_maximal {
                maximal.push(m.clone());
            }
        }

        maximal
    }

    /// Check if method m1 is more specific than method m2.
    ///
    /// m1 is more specific than m2 if:
    /// - Every parameter of m1 is at least as specific as m2
    /// - At least one parameter of m1 is strictly more specific
    /// - OR if parameter types are equally specific, m1 has more restrictive effects
    pub fn is_more_specific(&self, m1: &MethodCandidate, m2: &MethodCandidate) -> bool {
        // Must have same arity
        if m1.param_types.len() != m2.param_types.len() {
            return false;
        }

        let mut all_at_least = true;
        let mut some_strictly = false;

        for (p1, p2) in m1.param_types.iter().zip(&m2.param_types) {
            // p1 must be a subtype of p2 (at least as specific)
            if !self.is_subtype(p1, p2) {
                all_at_least = false;
                break;
            }

            // Check if p1 is strictly more specific (p1 <: p2 but not p2 <: p1)
            if self.is_subtype(p1, p2) && !self.is_subtype(p2, p1) {
                some_strictly = true;
            }
        }

        // If parameter types make m1 strictly more specific, we're done
        if all_at_least && some_strictly {
            return true;
        }

        // If parameter types are at least as specific but not strictly more specific,
        // use effect specificity as a tiebreaker
        if all_at_least && !some_strictly {
            // Check if m1 has strictly more specific effects
            let effect_cmp = self.effect_specificity(&m1.effects, &m2.effects);
            if effect_cmp == Ordering::Less {
                // m1 has more restrictive effects - use as tiebreaker
                return true;
            }
        }

        false
    }

    /// Compare the specificity of two methods.
    ///
    /// Returns:
    /// - Ordering::Less if m1 is more specific
    /// - Ordering::Greater if m2 is more specific
    /// - Ordering::Equal if they are equally specific (potential ambiguity)
    pub fn compare_specificity(
        &self,
        m1: &MethodCandidate,
        m2: &MethodCandidate,
    ) -> Ordering {
        let m1_more = self.is_more_specific(m1, m2);
        let m2_more = self.is_more_specific(m2, m1);

        match (m1_more, m2_more) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => Ordering::Equal,
        }
    }
}

/// Compare type parameter specificity.
///
/// Returns ordering based on:
/// 1. Concrete types are more specific than type parameters
/// 2. More constraints = more specific
/// 3. Instantiated generic is more specific than open generic
pub fn compare_type_param_specificity(t1: &Type, t2: &Type) -> Ordering {
    let t1_concrete = !matches!(t1.kind.as_ref(), TypeKind::Infer(_) | TypeKind::Param(_));
    let t2_concrete = !matches!(t2.kind.as_ref(), TypeKind::Infer(_) | TypeKind::Param(_));

    match (t1_concrete, t2_concrete) {
        (true, false) => Ordering::Less,   // Concrete more specific
        (false, true) => Ordering::Greater,
        _ => Ordering::Equal,
    }
}

// ============================================================
// Type Stability Checking
// ============================================================
//
// Type stability ensures that the return type of a method family is
// fully determined by the input types at compile time. This is crucial
// for optimization and predictable performance.
//
// See DISPATCH.md Section 4 for full specification.

use std::fmt;

/// Error indicating type instability in a method family.
///
/// Type instability occurs when methods with overlapping input types
/// have incompatible return types, making it impossible to determine
/// the return type at compile time based solely on argument types.
#[derive(Debug, Clone)]
pub struct TypeStabilityError {
    /// The name of the method family.
    pub method_family: String,
    /// The first conflicting method.
    pub method1: MethodCandidate,
    /// The second conflicting method.
    pub method2: MethodCandidate,
    /// The input types where the conflict occurs (if known).
    pub conflict_inputs: Option<Vec<Type>>,
    /// Human-readable explanation of the instability.
    pub explanation: String,
}

impl fmt::Display for TypeStabilityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "type instability detected in method family `{}`",
            self.method_family
        )?;
        writeln!(f)?;
        writeln!(f, "conflicting methods:")?;

        // Format first method
        write!(f, "  1. {}(", self.method1.name)?;
        for (i, ty) in self.method1.param_types.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{:?}", ty.kind)?;
        }
        writeln!(f, ") -> {:?}", self.method1.return_type.kind)?;

        // Format second method
        write!(f, "  2. {}(", self.method2.name)?;
        for (i, ty) in self.method2.param_types.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{:?}", ty.kind)?;
        }
        writeln!(f, ") -> {:?}", self.method2.return_type.kind)?;

        if let Some(ref inputs) = self.conflict_inputs {
            writeln!(f)?;
            write!(f, "conflict occurs with input types: (")?;
            for (i, ty) in inputs.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{:?}", ty.kind)?;
            }
            writeln!(f, ")")?;
        }

        writeln!(f)?;
        writeln!(f, "explanation: {}", self.explanation)?;
        writeln!(f)?;
        writeln!(
            f,
            "help: type stability requires that the return type be \
             uniquely determined by the input types"
        )?;

        Ok(())
    }
}

/// Result of type stability checking.
#[derive(Debug)]
pub struct TypeStabilityResult {
    /// All detected instabilities.
    pub errors: Vec<TypeStabilityError>,
    /// Whether the method family is type-stable.
    pub is_stable: bool,
}

impl TypeStabilityResult {
    /// Create a stable result (no errors).
    pub fn stable() -> Self {
        Self {
            errors: vec![],
            is_stable: true,
        }
    }

    /// Create an unstable result with errors.
    pub fn unstable(errors: Vec<TypeStabilityError>) -> Self {
        Self {
            is_stable: errors.is_empty(),
            errors,
        }
    }
}

/// Checker for type stability in method families.
///
/// Type stability means that the return type of a method call is fully
/// determined by the types of its arguments at compile time. This is
/// essential for efficient code generation and type inference.
///
/// # Algorithm
///
/// For each pair of methods with overlapping input types:
/// 1. Check if their return types are compatible
/// 2. If not, report a type stability violation
///
/// Two return types are compatible if:
/// - They are structurally equal, OR
/// - They are both generic with the same structure (after substitution)
pub struct TypeStabilityChecker<'a> {
    /// The dispatch resolver for subtype checking.
    resolver: DispatchResolver<'a>,
}

impl<'a> TypeStabilityChecker<'a> {
    /// Create a new type stability checker.
    pub fn new(unifier: &'a Unifier) -> Self {
        Self {
            resolver: DispatchResolver::new(unifier),
        }
    }

    /// Check a method family for type stability.
    ///
    /// Returns a `TypeStabilityResult` indicating whether the family is stable
    /// and any errors found.
    pub fn check_family(
        &self,
        family_name: &str,
        candidates: &[MethodCandidate],
    ) -> TypeStabilityResult {
        let mut errors = Vec::new();

        // Check all pairs of methods
        for (i, m1) in candidates.iter().enumerate() {
            for m2 in candidates.iter().skip(i + 1) {
                if let Some(error) = self.check_type_stability(family_name, m1, m2) {
                    errors.push(error);
                }
            }
        }

        TypeStabilityResult::unstable(errors)
    }

    /// Check type stability between two methods.
    ///
    /// Two methods are type-stable with respect to each other if:
    /// 1. Their input types don't overlap, OR
    /// 2. Their input types overlap AND their return types are compatible
    ///
    /// Returns `Some(TypeStabilityError)` if instability is detected.
    pub fn check_type_stability(
        &self,
        family_name: &str,
        m1: &MethodCandidate,
        m2: &MethodCandidate,
    ) -> Option<TypeStabilityError> {
        // Different arities can't conflict
        if m1.param_types.len() != m2.param_types.len() {
            return None;
        }

        // Check if input types overlap
        if !self.inputs_overlap(m1, m2) {
            return None;
        }

        // Check if return types are compatible
        if self.return_types_compatible(&m1.return_type, &m2.return_type, m1, m2) {
            return None;
        }

        // Type instability detected
        let explanation = self.generate_explanation(m1, m2);
        let conflict_inputs = self.find_overlapping_inputs(m1, m2);

        Some(TypeStabilityError {
            method_family: family_name.to_string(),
            method1: m1.clone(),
            method2: m2.clone(),
            conflict_inputs,
            explanation,
        })
    }

    /// Check if two methods have overlapping input types.
    ///
    /// Input types overlap if there exists some concrete argument types
    /// that would make both methods applicable.
    fn inputs_overlap(&self, m1: &MethodCandidate, m2: &MethodCandidate) -> bool {
        for (t1, t2) in m1.param_types.iter().zip(&m2.param_types) {
            if !self.types_could_overlap(t1, t2) {
                return false;
            }
        }
        true
    }

    /// Check if two types could potentially overlap (have common instances).
    fn types_could_overlap(&self, t1: &Type, t2: &Type) -> bool {
        // Type variables can overlap with anything
        if matches!(t1.kind.as_ref(), TypeKind::Infer(_) | TypeKind::Param(_)) {
            return true;
        }
        if matches!(t2.kind.as_ref(), TypeKind::Infer(_) | TypeKind::Param(_)) {
            return true;
        }

        // Check structural overlap
        match (t1.kind.as_ref(), t2.kind.as_ref()) {
            // Same primitive type
            (TypeKind::Primitive(p1), TypeKind::Primitive(p2)) => p1 == p2,

            // Same ADT
            (
                TypeKind::Adt { def_id: d1, args: a1 },
                TypeKind::Adt { def_id: d2, args: a2 },
            ) => {
                d1 == d2
                    && a1.len() == a2.len()
                    && a1.iter().zip(a2).all(|(x, y)| self.types_could_overlap(x, y))
            }

            // Tuples of same length
            (TypeKind::Tuple(ts1), TypeKind::Tuple(ts2)) => {
                ts1.len() == ts2.len()
                    && ts1.iter().zip(ts2).all(|(x, y)| self.types_could_overlap(x, y))
            }

            // Arrays of same length
            (
                TypeKind::Array { element: e1, size: s1 },
                TypeKind::Array { element: e2, size: s2 },
            ) => s1 == s2 && self.types_could_overlap(e1, e2),

            // Slices
            (TypeKind::Slice { element: e1 }, TypeKind::Slice { element: e2 }) => {
                self.types_could_overlap(e1, e2)
            }

            // References with same mutability
            (
                TypeKind::Ref { inner: i1, mutable: m1 },
                TypeKind::Ref { inner: i2, mutable: m2 },
            ) => m1 == m2 && self.types_could_overlap(i1, i2),

            // Function types
            (
                TypeKind::Fn { params: p1, ret: r1 },
                TypeKind::Fn { params: p2, ret: r2 },
            ) => {
                p1.len() == p2.len()
                    && p1.iter().zip(p2).all(|(x, y)| self.types_could_overlap(x, y))
                    && self.types_could_overlap(r1, r2)
            }

            // Never overlaps with concrete types (bottom type)
            (TypeKind::Never, _) | (_, TypeKind::Never) => true,

            // Error overlaps with everything (for error recovery)
            (TypeKind::Error, _) | (_, TypeKind::Error) => true,

            // Different type kinds don't overlap
            _ => false,
        }
    }

    /// Check if two return types are compatible for type stability.
    ///
    /// Return types are compatible if:
    /// 1. They are structurally equal, OR
    /// 2. They are both generic with compatible structure
    fn return_types_compatible(
        &self,
        ret1: &Type,
        ret2: &Type,
        m1: &MethodCandidate,
        m2: &MethodCandidate,
    ) -> bool {
        // Check structural equality
        if self.resolver.types_equal(ret1, ret2) {
            return true;
        }

        // Check if both are generic return types that depend on input types
        // in compatible ways
        if self.both_generic_compatible(ret1, ret2, m1, m2) {
            return true;
        }

        false
    }

    /// Check if both return types are generic and compatible.
    ///
    /// Two generic return types are compatible if they have the same
    /// structure and their type parameters correspond to the same
    /// positions in the input types.
    fn both_generic_compatible(
        &self,
        ret1: &Type,
        ret2: &Type,
        m1: &MethodCandidate,
        m2: &MethodCandidate,
    ) -> bool {
        // Check if both methods have type parameters
        if m1.type_params.is_empty() || m2.type_params.is_empty() {
            return false;
        }

        // Check if return types are type parameters
        let ret1_is_param = matches!(ret1.kind.as_ref(), TypeKind::Param(_) | TypeKind::Infer(_));
        let ret2_is_param = matches!(ret2.kind.as_ref(), TypeKind::Param(_) | TypeKind::Infer(_));

        // If both are type parameters, check if they're determined by inputs
        if ret1_is_param && ret2_is_param {
            // Both return a type parameter that should be determined by inputs
            // This is stable if the type parameters are used in the same positions
            return self.type_params_correspond(ret1, ret2, m1, m2);
        }

        // If neither is a type parameter, check structural equality
        if !ret1_is_param && !ret2_is_param {
            return self.resolver.types_equal(ret1, ret2);
        }

        // One is generic, one is concrete: check if concrete is instance of generic
        if ret1_is_param {
            // ret1 is generic, ret2 is concrete
            // This is compatible if ret2 is a valid instantiation
            return true;
        }
        if ret2_is_param {
            // ret2 is generic, ret1 is concrete
            // This is compatible if ret1 is a valid instantiation
            return true;
        }

        false
    }

    /// Check if type parameters in return types correspond to same input positions.
    fn type_params_correspond(
        &self,
        _ret1: &Type,
        _ret2: &Type,
        m1: &MethodCandidate,
        m2: &MethodCandidate,
    ) -> bool {
        // For now, if both methods have type parameters in return position,
        // and they have the same arity, we consider them compatible.
        // A more sophisticated analysis would track which parameter positions
        // each type parameter comes from.
        m1.param_types.len() == m2.param_types.len()
    }

    /// Find example input types where the methods overlap.
    fn find_overlapping_inputs(
        &self,
        m1: &MethodCandidate,
        m2: &MethodCandidate,
    ) -> Option<Vec<Type>> {
        let is_type_var = |t: &Type| {
            matches!(t.kind.as_ref(), TypeKind::Infer(_) | TypeKind::Param(_))
        };

        let types: Vec<Type> = m1
            .param_types
            .iter()
            .zip(&m2.param_types)
            .map(|(t1, t2)| {
                // Prefer concrete type
                if !is_type_var(t1) {
                    t1.clone()
                } else if !is_type_var(t2) {
                    t2.clone()
                } else {
                    // Both generic, use first
                    t1.clone()
                }
            })
            .collect();

        // Only return if we have at least one concrete type
        if types.iter().any(|t| !is_type_var(t)) {
            Some(types)
        } else {
            None
        }
    }

    /// Generate a human-readable explanation of the type instability.
    fn generate_explanation(&self, m1: &MethodCandidate, m2: &MethodCandidate) -> String {
        let ret1 = format!("{:?}", m1.return_type.kind);
        let ret2 = format!("{:?}", m2.return_type.kind);

        format!(
            "Methods with overlapping input types return different types: \
             `{}` vs `{}`. The return type must be uniquely determined by \
             the input types for type stability.",
            ret1, ret2
        )
    }
}

// ============================================================
// Constraint Checking for Generic Dispatch
// ============================================================
//
// When a generic method has constraints on its type parameters
// (e.g., `fn sort<T: Ord>(list: Vec<T>) -> Vec<T>`), we must
// verify that the concrete types inferred for each parameter
// actually satisfy those constraints.
//
// See DISPATCH.md Section 9 for full specification.

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
            TypeKind::Forall { .. } => false, // Polymorphic types are not Copy
            TypeKind::Ownership { inner, .. } => self.type_is_copy(inner), // Copy depends on inner type
        }
    }

    /// Check if a type implements Clone.
    fn type_is_clone(&self, ty: &Type) -> bool {
        // All Copy types are Clone
        if self.type_is_copy(ty) {
            return true;
        }
        // For non-Copy types, we'd need full trait resolution
        // For now, be conservative
        false
    }

    /// Check if a type is Sized.
    fn type_is_sized(&self, ty: &Type) -> bool {
        match ty.kind.as_ref() {
            TypeKind::Slice { .. } => false,
            TypeKind::Primitive(PrimitiveTy::Str) => false,
            TypeKind::DynTrait { .. } => false,
            _ => true,
        }
    }

    /// Check if a type implements Send.
    fn type_is_send(&self, ty: &Type) -> bool {
        match ty.kind.as_ref() {
            TypeKind::Primitive(_) => true,
            TypeKind::Ref { inner, .. } => self.type_is_send(inner),
            TypeKind::Ptr { inner, .. } => self.type_is_send(inner),
            TypeKind::Array { element, .. } => self.type_is_send(element),
            TypeKind::Tuple(elements) => elements.iter().all(|e| self.type_is_send(e)),
            TypeKind::Closure { .. } => false, // Conservative
            TypeKind::Adt { .. } => true, // Optimistic default
            _ => true,
        }
    }

    /// Check if a type implements Sync.
    fn type_is_sync(&self, ty: &Type) -> bool {
        // Similar to Send
        self.type_is_send(ty)
    }

    /// Check if a type implements Ord (total ordering).
    fn type_is_ord(&self, ty: &Type) -> bool {
        match ty.kind.as_ref() {
            // Numeric primitives have total ordering
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_candidate(name: &str, params: Vec<Type>, ret: Type) -> MethodCandidate {
        MethodCandidate {
            def_id: DefId::new(0),
            name: name.to_string(),
            param_types: params,
            return_type: ret,
            type_params: vec![],
            effects: None,
            trait_id: None,
        }
    }

    #[test]
    fn test_exact_match() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        let candidates = vec![
            make_candidate("add", vec![Type::i32(), Type::i32()], Type::i32()),
            make_candidate("add", vec![Type::i64(), Type::i64()], Type::i64()),
        ];

        let result = resolver.resolve("add", &[Type::i32(), Type::i32()], &candidates);
        assert!(matches!(result, DispatchResult::Resolved(m) if m.name == "add" && *m.return_type.kind == TypeKind::Primitive(PrimitiveTy::Int(crate::hir::def::IntTy::I32))));
    }

    #[test]
    fn test_no_match() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        let candidates = vec![
            make_candidate("add", vec![Type::i32(), Type::i32()], Type::i32()),
        ];

        let result = resolver.resolve("add", &[Type::str()], &candidates);
        assert!(matches!(result, DispatchResult::NoMatch(_)));
    }

    #[test]
    fn test_arity_mismatch() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        let candidates = vec![
            make_candidate("add", vec![Type::i32(), Type::i32()], Type::i32()),
        ];

        let result = resolver.resolve("add", &[Type::i32()], &candidates);
        assert!(matches!(result, DispatchResult::NoMatch(_)));
    }

    #[test]
    fn test_ambiguous_candidates() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        // Two methods with same signature = ambiguous
        let candidates = vec![
            make_candidate("foo", vec![Type::i32()], Type::i32()),
            make_candidate("foo", vec![Type::i32()], Type::i64()),
        ];

        let result = resolver.resolve("foo", &[Type::i32()], &candidates);
        match result {
            DispatchResult::Ambiguous(err) => {
                assert_eq!(err.method_name, "foo");
                assert_eq!(err.candidates.len(), 2);
            }
            other => panic!("Expected Ambiguous, got {:?}", other),
        }
    }

    #[test]
    fn test_diamond_conflict_detection() {
        // Simulate the diamond problem: two traits define the same method
        let trait1_id = DefId::new(100);
        let trait2_id = DefId::new(200);

        let mut m1 = make_candidate("render", vec![Type::i32()], Type::i32());
        m1.trait_id = Some(trait1_id);

        let mut m2 = make_candidate("render", vec![Type::i32()], Type::string());
        m2.trait_id = Some(trait2_id);

        let err = AmbiguityError {
            method_name: "render".to_string(),
            arg_types: vec![Type::i32()],
            candidates: vec![m1, m2],
        };

        // Should detect as diamond conflict
        assert!(err.is_diamond_conflict());

        // Should list both conflicting traits
        let conflicting = err.conflicting_trait_ids();
        assert_eq!(conflicting.len(), 2);
        assert!(conflicting.contains(&trait1_id));
        assert!(conflicting.contains(&trait2_id));

        // Test suggestion generation
        let mut trait_names = HashMap::new();
        trait_names.insert(trait1_id, "Drawable".to_string());
        trait_names.insert(trait2_id, "Printable".to_string());

        let suggestion = err.diamond_suggestion(&trait_names);
        assert!(suggestion.contains("Drawable"));
        assert!(suggestion.contains("Printable"));
    }

    #[test]
    fn test_not_diamond_conflict_same_trait() {
        // Same trait, different overloads - not a diamond conflict
        let trait_id = DefId::new(100);

        let mut m1 = make_candidate("process", vec![Type::i32()], Type::i32());
        m1.trait_id = Some(trait_id);

        let mut m2 = make_candidate("process", vec![Type::i32()], Type::i64());
        m2.trait_id = Some(trait_id);

        let err = AmbiguityError {
            method_name: "process".to_string(),
            arg_types: vec![Type::i32()],
            candidates: vec![m1, m2],
        };

        // Should NOT be a diamond conflict (same trait)
        assert!(!err.is_diamond_conflict());
    }

    #[test]
    fn test_not_diamond_conflict_no_traits() {
        // Free functions, no traits - not a diamond conflict
        let m1 = make_candidate("compute", vec![Type::i32()], Type::i32());
        let m2 = make_candidate("compute", vec![Type::i32()], Type::i64());

        let err = AmbiguityError {
            method_name: "compute".to_string(),
            arg_types: vec![Type::i32()],
            candidates: vec![m1, m2],
        };

        // Should NOT be a diamond conflict (no traits)
        assert!(!err.is_diamond_conflict());
    }

    #[test]
    fn test_single_candidate() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        let candidates = vec![
            make_candidate("single", vec![Type::i32()], Type::bool()),
        ];

        let result = resolver.resolve("single", &[Type::i32()], &candidates);
        match result {
            DispatchResult::Resolved(m) => {
                assert_eq!(m.name, "single");
            }
            other => panic!("Expected Resolved, got {:?}", other),
        }
    }

    #[test]
    fn test_empty_candidates() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        let candidates: Vec<MethodCandidate> = vec![];
        let result = resolver.resolve("missing", &[Type::i32()], &candidates);

        match result {
            DispatchResult::NoMatch(err) => {
                assert_eq!(err.method_name, "missing");
                assert!(err.candidates.is_empty());
            }
            other => panic!("Expected NoMatch, got {:?}", other),
        }
    }

    #[test]
    fn test_is_applicable_arity() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        let method = make_candidate("test", vec![Type::i32(), Type::i32()], Type::i32());

        // Wrong arity
        assert!(!resolver.is_applicable(&method, &[Type::i32()]));
        assert!(!resolver.is_applicable(&method, &[Type::i32(), Type::i32(), Type::i32()]));

        // Correct arity
        assert!(resolver.is_applicable(&method, &[Type::i32(), Type::i32()]));
    }

    #[test]
    fn test_never_type_subtyping() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        // Never is subtype of any type
        let method = make_candidate("test", vec![Type::i32()], Type::i32());
        assert!(resolver.is_applicable(&method, &[Type::never()]));
    }

    #[test]
    fn test_is_more_specific() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        let m1 = make_candidate("f", vec![Type::i32()], Type::i32());
        let m2 = make_candidate("f", vec![Type::i32()], Type::i32());

        // Same signature: neither is more specific
        assert!(!resolver.is_more_specific(&m1, &m2));
        assert!(!resolver.is_more_specific(&m2, &m1));
    }

    #[test]
    fn test_compare_specificity_equal() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        let m1 = make_candidate("f", vec![Type::i32()], Type::i32());
        let m2 = make_candidate("f", vec![Type::i32()], Type::i32());

        assert_eq!(resolver.compare_specificity(&m1, &m2), Ordering::Equal);
    }

    #[test]
    fn test_types_equal_primitives() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        assert!(resolver.types_equal(&Type::i32(), &Type::i32()));
        assert!(!resolver.types_equal(&Type::i32(), &Type::i64()));
        assert!(resolver.types_equal(&Type::bool(), &Type::bool()));
    }

    #[test]
    fn test_types_equal_tuples() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        let t1 = Type::tuple(vec![Type::i32(), Type::bool()]);
        let t2 = Type::tuple(vec![Type::i32(), Type::bool()]);
        let t3 = Type::tuple(vec![Type::i32(), Type::i32()]);
        let t4 = Type::tuple(vec![Type::i32()]);

        assert!(resolver.types_equal(&t1, &t2));
        assert!(!resolver.types_equal(&t1, &t3));
        assert!(!resolver.types_equal(&t1, &t4));
    }

    #[test]
    fn test_multiple_args_dispatch() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        let candidates = vec![
            make_candidate("process", vec![Type::i32(), Type::str()], Type::unit()),
            make_candidate("process", vec![Type::i64(), Type::str()], Type::unit()),
            make_candidate("process", vec![Type::i32(), Type::i32()], Type::unit()),
        ];

        // First candidate matches
        let result = resolver.resolve("process", &[Type::i32(), Type::str()], &candidates);
        assert!(matches!(result, DispatchResult::Resolved(m) if m.param_types.len() == 2));

        // Third candidate matches
        let result = resolver.resolve("process", &[Type::i32(), Type::i32()], &candidates);
        assert!(matches!(result, DispatchResult::Resolved(_)));

        // No match
        let result = resolver.resolve("process", &[Type::bool(), Type::bool()], &candidates);
        assert!(matches!(result, DispatchResult::NoMatch(_)));
    }

    #[test]
    fn test_compare_type_param_specificity() {
        use crate::hir::TyVarId;

        // Concrete type is more specific than type variable
        let concrete = Type::i32();
        let param = Type::new(TypeKind::Param(TyVarId::new(42)));

        let result = compare_type_param_specificity(&concrete, &param);
        assert_eq!(result, Ordering::Less);

        let result = compare_type_param_specificity(&param, &concrete);
        assert_eq!(result, Ordering::Greater);

        // Both concrete: equal
        let result = compare_type_param_specificity(&Type::i32(), &Type::i64());
        assert_eq!(result, Ordering::Equal);
    }

    #[test]
    fn test_find_maximal_single() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        let candidates = vec![
            make_candidate("f", vec![Type::i32()], Type::i32()),
        ];

        let maximal = resolver.find_maximal(&candidates);
        assert_eq!(maximal.len(), 1);
    }

    #[test]
    fn test_find_maximal_multiple_equal() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        // Two methods with same signature: both maximal
        let candidates = vec![
            make_candidate("f", vec![Type::i32()], Type::i32()),
            make_candidate("f", vec![Type::i32()], Type::i64()),
        ];

        let maximal = resolver.find_maximal(&candidates);
        assert_eq!(maximal.len(), 2);
    }

    #[test]
    fn test_error_contains_all_candidates() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        let candidates = vec![
            make_candidate("f", vec![Type::i32()], Type::i32()),
            make_candidate("f", vec![Type::i64()], Type::i64()),
        ];

        let result = resolver.resolve("f", &[Type::bool()], &candidates);
        match result {
            DispatchResult::NoMatch(err) => {
                assert_eq!(err.candidates.len(), 2);
                assert_eq!(err.arg_types.len(), 1);
            }
            other => panic!("Expected NoMatch, got {:?}", other),
        }
    }

    // === Variance Tests ===

    #[test]
    fn test_immutable_ref_covariance() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        // Immutable references are covariant
        let ref_i32 = Type::reference(Type::i32(), false);
        let ref_i32_2 = Type::reference(Type::i32(), false);

        // Same type
        assert!(resolver.is_subtype(&ref_i32, &ref_i32_2));
    }

    #[test]
    fn test_mutable_ref_invariance() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        // Mutable references are invariant
        let mut_ref_i32 = Type::reference(Type::i32(), true);
        let mut_ref_i32_2 = Type::reference(Type::i32(), true);

        // Same type
        assert!(resolver.is_subtype(&mut_ref_i32, &mut_ref_i32_2));

        // Different inner type
        let mut_ref_i64 = Type::reference(Type::i64(), true);
        assert!(!resolver.is_subtype(&mut_ref_i32, &mut_ref_i64));
    }

    #[test]
    fn test_mutable_to_immutable_ref() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        // Mutable ref can be used where immutable ref is expected
        let mut_ref_i32 = Type::reference(Type::i32(), true);
        let ref_i32 = Type::reference(Type::i32(), false);

        assert!(resolver.is_subtype(&mut_ref_i32, &ref_i32));

        // But not the other way around
        assert!(!resolver.is_subtype(&ref_i32, &mut_ref_i32));
    }

    #[test]
    fn test_tuple_covariance() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        // Tuples are covariant in each position
        let t1 = Type::tuple(vec![Type::i32(), Type::bool()]);
        let t2 = Type::tuple(vec![Type::i32(), Type::bool()]);

        assert!(resolver.is_subtype(&t1, &t2));

        // Different element types
        let t3 = Type::tuple(vec![Type::i64(), Type::bool()]);
        assert!(!resolver.is_subtype(&t1, &t3));

        // Different lengths
        let t4 = Type::tuple(vec![Type::i32()]);
        assert!(!resolver.is_subtype(&t1, &t4));
    }

    #[test]
    fn test_never_is_subtype_of_all() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        // Never (bottom type) is a subtype of everything
        let never = Type::never();

        assert!(resolver.is_subtype(&never, &Type::i32()));
        assert!(resolver.is_subtype(&never, &Type::bool()));
        assert!(resolver.is_subtype(&never, &Type::str()));
        assert!(resolver.is_subtype(&never, &Type::unit()));

        // But nothing is a subtype of never (except never itself)
        assert!(!resolver.is_subtype(&Type::i32(), &never));
        assert!(resolver.is_subtype(&never, &never)); // never <: never
    }

    #[test]
    fn test_array_covariance() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        // Arrays are covariant but size must match
        let arr1 = Type::array(Type::i32(), 5);
        let arr2 = Type::array(Type::i32(), 5);
        let arr3 = Type::array(Type::i32(), 10);

        assert!(resolver.is_subtype(&arr1, &arr2));
        assert!(!resolver.is_subtype(&arr1, &arr3)); // Different size
    }

    // === DynTrait Subtyping Tests ===

    #[test]
    fn test_dyn_trait_equality() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        let trait_id = DefId::new(100);
        let auto1 = DefId::new(101);
        let auto2 = DefId::new(102);

        // Same trait, no auto traits
        let dyn1 = Type::dyn_trait(trait_id, vec![]);
        let dyn2 = Type::dyn_trait(trait_id, vec![]);
        assert!(resolver.types_equal(&dyn1, &dyn2));

        // Same trait, same auto traits
        let dyn3 = Type::dyn_trait(trait_id, vec![auto1, auto2]);
        let dyn4 = Type::dyn_trait(trait_id, vec![auto1, auto2]);
        assert!(resolver.types_equal(&dyn3, &dyn4));

        // Same trait, different order of auto traits (should still be equal)
        let dyn5 = Type::dyn_trait(trait_id, vec![auto2, auto1]);
        assert!(resolver.types_equal(&dyn3, &dyn5));

        // Different trait
        let other_trait = DefId::new(200);
        let dyn6 = Type::dyn_trait(other_trait, vec![]);
        assert!(!resolver.types_equal(&dyn1, &dyn6));
    }

    #[test]
    fn test_dyn_trait_subtyping() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        let trait_id = DefId::new(100);
        let send_id = DefId::new(101);
        let sync_id = DefId::new(102);

        // dyn Trait <: dyn Trait
        let dyn1 = Type::dyn_trait(trait_id, vec![]);
        let dyn2 = Type::dyn_trait(trait_id, vec![]);
        assert!(resolver.is_subtype(&dyn1, &dyn2));

        // dyn Trait + Send + Sync <: dyn Trait (superset of auto traits)
        let dyn_send_sync = Type::dyn_trait(trait_id, vec![send_id, sync_id]);
        assert!(resolver.is_subtype(&dyn_send_sync, &dyn1));

        // dyn Trait + Send <: dyn Trait
        let dyn_send = Type::dyn_trait(trait_id, vec![send_id]);
        assert!(resolver.is_subtype(&dyn_send, &dyn1));

        // dyn Trait NOT <: dyn Trait + Send (missing auto trait)
        assert!(!resolver.is_subtype(&dyn1, &dyn_send));

        // dyn Trait + Send + Sync <: dyn Trait + Send
        assert!(resolver.is_subtype(&dyn_send_sync, &dyn_send));

        // Different primary trait: dyn TraitA NOT <: dyn TraitB
        let other_trait = DefId::new(200);
        let dyn_other = Type::dyn_trait(other_trait, vec![]);
        assert!(!resolver.is_subtype(&dyn1, &dyn_other));
        assert!(!resolver.is_subtype(&dyn_other, &dyn1));
    }

    #[test]
    fn test_type_subtype_dyn_trait_with_checker() {
        let unifier = Unifier::new();

        let trait_id = DefId::new(100);

        // Create a trait checker that says i32 implements trait 100
        let checker: &TraitChecker = &|ty: &Type, tid: DefId| {
            matches!(ty.kind.as_ref(), TypeKind::Primitive(PrimitiveTy::Int(crate::hir::def::IntTy::I32)))
                && tid.index == 100
        };

        let resolver = DispatchResolver::with_trait_checker(&unifier, checker);

        let dyn_trait = Type::dyn_trait(trait_id, vec![]);

        // i32 <: dyn Trait (because our checker says i32 implements trait 100)
        assert!(resolver.is_subtype(&Type::i32(), &dyn_trait));

        // i64 NOT <: dyn Trait (our checker says no)
        assert!(!resolver.is_subtype(&Type::i64(), &dyn_trait));
    }

    #[test]
    fn test_type_subtype_dyn_trait_without_checker() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        let trait_id = DefId::new(100);
        let dyn_trait = Type::dyn_trait(trait_id, vec![]);

        // Without a trait checker, T <: dyn Trait should conservatively return false
        assert!(!resolver.is_subtype(&Type::i32(), &dyn_trait));
    }

    // === Effect-Aware Dispatch Tests ===

    fn make_candidate_with_effects(
        name: &str,
        params: Vec<Type>,
        ret: Type,
        effects: Option<EffectRow>,
    ) -> MethodCandidate {
        MethodCandidate {
            def_id: DefId::new(0),
            name: name.to_string(),
            param_types: params,
            return_type: ret,
            type_params: vec![],
            effects,
            trait_id: None,
        }
    }

    #[test]
    fn test_effect_row_pure() {
        let pure = EffectRow::pure();
        assert!(pure.is_pure());
        assert_eq!(pure.effect_count(), 0);
        assert!(!pure.is_open);
    }

    #[test]
    fn test_effect_row_with_effects() {
        let row = EffectRow::with_effects(vec!["IO".to_string(), "Error".to_string()]);
        assert!(!row.is_pure());
        assert_eq!(row.effect_count(), 2);
        assert!(!row.is_open);
    }

    #[test]
    fn test_effect_row_open() {
        let row = EffectRow::open(vec!["IO".to_string()]);
        assert!(!row.is_pure());
        assert!(row.is_open);
    }

    #[test]
    fn test_effect_row_subset_pure_of_any() {
        let pure = EffectRow::pure();
        let io = EffectRow::with_effects(vec!["IO".to_string()]);
        let io_error = EffectRow::with_effects(vec!["IO".to_string(), "Error".to_string()]);

        // Pure is subset of everything
        assert!(pure.is_subset_of(&pure));
        assert!(pure.is_subset_of(&io));
        assert!(pure.is_subset_of(&io_error));
    }

    #[test]
    fn test_effect_row_subset_closed() {
        let io = EffectRow::with_effects(vec!["IO".to_string()]);
        let error = EffectRow::with_effects(vec!["Error".to_string()]);
        let io_error = EffectRow::with_effects(vec!["IO".to_string(), "Error".to_string()]);

        // IO is subset of IO+Error
        assert!(io.is_subset_of(&io_error));

        // Error is subset of IO+Error
        assert!(error.is_subset_of(&io_error));

        // IO is NOT subset of Error
        assert!(!io.is_subset_of(&error));

        // IO+Error is NOT subset of IO
        assert!(!io_error.is_subset_of(&io));
    }

    #[test]
    fn test_effect_row_subset_open_rows() {
        let io_open = EffectRow::open(vec!["IO".to_string()]);
        let io_closed = EffectRow::with_effects(vec!["IO".to_string()]);
        let io_error_open = EffectRow::open(vec!["IO".to_string(), "Error".to_string()]);

        // Open row is subset of open row with same or more effects
        assert!(io_open.is_subset_of(&io_open));
        assert!(io_open.is_subset_of(&io_error_open));

        // Open row is NOT subset of closed row (open could expand to anything)
        assert!(!io_open.is_subset_of(&io_closed));

        // Closed row IS subset of open row with same effects
        assert!(io_closed.is_subset_of(&io_open));
    }

    #[test]
    fn test_effect_specificity_pure_vs_effectful() {
        let pure = EffectRow::pure();
        let io = EffectRow::with_effects(vec!["IO".to_string()]);

        // Pure is more specific than effectful
        assert_eq!(pure.compare_specificity(&io), Ordering::Less);
        assert_eq!(io.compare_specificity(&pure), Ordering::Greater);
    }

    #[test]
    fn test_effect_specificity_closed_vs_open() {
        let io_closed = EffectRow::with_effects(vec!["IO".to_string()]);
        let io_open = EffectRow::open(vec!["IO".to_string()]);

        // Closed is more specific than open
        assert_eq!(io_closed.compare_specificity(&io_open), Ordering::Less);
        assert_eq!(io_open.compare_specificity(&io_closed), Ordering::Greater);
    }

    #[test]
    fn test_effect_specificity_fewer_effects() {
        let io = EffectRow::with_effects(vec!["IO".to_string()]);
        let io_error = EffectRow::with_effects(vec!["IO".to_string(), "Error".to_string()]);

        // Fewer effects is more specific
        assert_eq!(io.compare_specificity(&io_error), Ordering::Less);
        assert_eq!(io_error.compare_specificity(&io), Ordering::Greater);
    }

    #[test]
    fn test_effects_compatible_pure_always_compatible() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        let context = EffectRow::with_effects(vec!["IO".to_string()]);

        // Pure method (None) is always compatible
        assert!(resolver.effects_compatible(&None, Some(&context)));
        assert!(resolver.effects_compatible(&None, None));
    }

    #[test]
    fn test_effects_compatible_subset() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        let io = EffectRow::with_effects(vec!["IO".to_string()]);
        let io_error = EffectRow::with_effects(vec!["IO".to_string(), "Error".to_string()]);

        // IO method is compatible with IO+Error context
        assert!(resolver.effects_compatible(&Some(io.clone()), Some(&io_error)));

        // IO+Error method is NOT compatible with IO-only context
        assert!(!resolver.effects_compatible(&Some(io_error), Some(&io)));
    }

    #[test]
    fn test_pure_method_preferred_over_effectful() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        // Two methods with same parameter types, one pure, one with IO effect
        let pure_method = make_candidate_with_effects(
            "compute",
            vec![Type::i32()],
            Type::i32(),
            None, // Pure
        );
        let io_method = make_candidate_with_effects(
            "compute",
            vec![Type::i32()],
            Type::i32(),
            Some(EffectRow::with_effects(vec!["IO".to_string()])),
        );

        // Pure method is more specific
        assert!(resolver.is_more_specific(&pure_method, &io_method));
        assert!(!resolver.is_more_specific(&io_method, &pure_method));

        // Dispatch should resolve to pure method
        let candidates = vec![pure_method.clone(), io_method.clone()];
        let result = resolver.resolve("compute", &[Type::i32()], &candidates);

        match result {
            DispatchResult::Resolved(m) => {
                assert!(m.effects.is_none(), "Should resolve to pure method");
            }
            other => panic!("Expected Resolved, got {:?}", other),
        }
    }

    #[test]
    fn test_effect_row_with_row_variable_compatibility() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        // Open row {IO | rho}
        let open_io = EffectRow::open(vec!["IO".to_string()]);
        // Closed row {IO, Error}
        let closed_io_error = EffectRow::with_effects(
            vec!["IO".to_string(), "Error".to_string()]
        );
        // Open row {IO, Error | rho}
        let open_io_error = EffectRow::open(
            vec!["IO".to_string(), "Error".to_string()]
        );

        // Open row method NOT compatible with closed context
        // (row variable could expand to include unhandled effects)
        assert!(!resolver.effects_compatible(&Some(open_io.clone()), Some(&closed_io_error)));

        // Open row method IS compatible with open context that has superset
        assert!(resolver.effects_compatible(&Some(open_io), Some(&open_io_error)));
    }

    #[test]
    fn test_effect_specificity_as_tiebreaker() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        // Same parameter types, different effects
        let io_method = make_candidate_with_effects(
            "f",
            vec![Type::i32()],
            Type::i32(),
            Some(EffectRow::with_effects(vec!["IO".to_string()])),
        );
        let io_error_method = make_candidate_with_effects(
            "f",
            vec![Type::i32()],
            Type::i32(),
            Some(EffectRow::with_effects(vec!["IO".to_string(), "Error".to_string()])),
        );

        // IO method is more specific than IO+Error method (fewer effects)
        assert!(resolver.is_more_specific(&io_method, &io_error_method));
        assert!(!resolver.is_more_specific(&io_error_method, &io_method));
    }

    #[test]
    fn test_dispatch_resolves_to_less_effectful() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        let io = make_candidate_with_effects(
            "f",
            vec![Type::i32()],
            Type::i32(),
            Some(EffectRow::with_effects(vec!["IO".to_string()])),
        );
        let io_error = make_candidate_with_effects(
            "f",
            vec![Type::i32()],
            Type::i32(),
            Some(EffectRow::with_effects(vec!["IO".to_string(), "Error".to_string()])),
        );

        let candidates = vec![io_error.clone(), io.clone()];
        let result = resolver.resolve("f", &[Type::i32()], &candidates);

        match result {
            DispatchResult::Resolved(m) => {
                // Should resolve to the method with fewer effects (IO only)
                assert_eq!(m.effects.as_ref().unwrap().effect_count(), 1);
            }
            other => panic!("Expected Resolved, got {:?}", other),
        }
    }

    // === Generic Instantiation Tests ===

    fn make_generic_candidate(
        name: &str,
        type_params: Vec<TypeParam>,
        params: Vec<Type>,
        ret: Type,
    ) -> MethodCandidate {
        MethodCandidate {
            def_id: DefId::new(0),
            name: name.to_string(),
            param_types: params,
            return_type: ret,
            type_params,
            effects: None,
            trait_id: None,
        }
    }

    #[test]
    fn test_instantiate_generic_single_type_param() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        // Generic method: fn identity<T>(x: T) -> T
        let t_id = TyVarId::new(1);
        let t_param = TypeParam {
            name: "T".to_string(),
            id: t_id,
            constraints: vec![],
        };
        let t_type = Type::param(t_id);

        let method = make_generic_candidate(
            "identity",
            vec![t_param],
            vec![t_type.clone()],
            t_type,
        );

        // Call with i32
        let result = resolver.instantiate_generic(&method, &[Type::i32()]);
        match result {
            InstantiationResult::Success { substitutions, candidate } => {
                // T should be substituted with i32
                assert_eq!(substitutions.get(&t_id), Some(&Type::i32()));
                // Parameter should be i32
                assert!(resolver.types_equal(&candidate.param_types[0], &Type::i32()));
                // Return should be i32
                assert!(resolver.types_equal(&candidate.return_type, &Type::i32()));
                // No longer generic
                assert!(candidate.type_params.is_empty());
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[test]
    fn test_instantiate_generic_multiple_type_params() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        // Generic method: fn pair<T, U>(x: T, y: U) -> (T, U)
        let t_id = TyVarId::new(1);
        let u_id = TyVarId::new(2);
        let t_param = TypeParam {
            name: "T".to_string(),
            id: t_id,
            constraints: vec![],
        };
        let u_param = TypeParam {
            name: "U".to_string(),
            id: u_id,
            constraints: vec![],
        };
        let t_type = Type::param(t_id);
        let u_type = Type::param(u_id);
        let ret_type = Type::tuple(vec![t_type.clone(), u_type.clone()]);

        let method = make_generic_candidate(
            "pair",
            vec![t_param, u_param],
            vec![t_type, u_type],
            ret_type,
        );

        // Call with (i32, bool)
        let result = resolver.instantiate_generic(&method, &[Type::i32(), Type::bool()]);
        match result {
            InstantiationResult::Success { substitutions, candidate } => {
                // T should be i32, U should be bool
                assert_eq!(substitutions.get(&t_id), Some(&Type::i32()));
                assert_eq!(substitutions.get(&u_id), Some(&Type::bool()));
                // Parameters should be i32, bool
                assert!(resolver.types_equal(&candidate.param_types[0], &Type::i32()));
                assert!(resolver.types_equal(&candidate.param_types[1], &Type::bool()));
                // Return should be (i32, bool)
                let expected_ret = Type::tuple(vec![Type::i32(), Type::bool()]);
                assert!(resolver.types_equal(&candidate.return_type, &expected_ret));
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[test]
    fn test_instantiate_generic_nested_types() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        // Generic method: fn wrap<T>(x: &T) -> [T; 1]
        let t_id = TyVarId::new(1);
        let t_param = TypeParam {
            name: "T".to_string(),
            id: t_id,
            constraints: vec![],
        };
        let t_type = Type::param(t_id);
        let param_type = Type::reference(t_type.clone(), false);
        let ret_type = Type::array(t_type, 1);

        let method = make_generic_candidate(
            "wrap",
            vec![t_param],
            vec![param_type],
            ret_type,
        );

        // Call with &i64
        let arg = Type::reference(Type::i64(), false);
        let result = resolver.instantiate_generic(&method, &[arg]);
        match result {
            InstantiationResult::Success { substitutions, candidate } => {
                // T should be i64
                assert_eq!(substitutions.get(&t_id), Some(&Type::i64()));
                // Parameter should be &i64
                let expected_param = Type::reference(Type::i64(), false);
                assert!(resolver.types_equal(&candidate.param_types[0], &expected_param));
                // Return should be [i64; 1]
                let expected_ret = Type::array(Type::i64(), 1);
                assert!(resolver.types_equal(&candidate.return_type, &expected_ret));
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[test]
    fn test_instantiate_generic_consistent_substitution() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        // Generic method: fn same<T>(x: T, y: T) -> T
        // Both parameters use the same type param
        let t_id = TyVarId::new(1);
        let t_param = TypeParam {
            name: "T".to_string(),
            id: t_id,
            constraints: vec![],
        };
        let t_type = Type::param(t_id);

        let method = make_generic_candidate(
            "same",
            vec![t_param],
            vec![t_type.clone(), t_type.clone()],
            t_type,
        );

        // Call with (i32, i32) - should succeed
        let result = resolver.instantiate_generic(&method, &[Type::i32(), Type::i32()]);
        assert!(matches!(result, InstantiationResult::Success { .. }));

        // Call with (i32, i64) - should fail (inconsistent T)
        let result = resolver.instantiate_generic(&method, &[Type::i32(), Type::i64()]);
        match result {
            InstantiationResult::TypeMismatch { param_id, expected, found } => {
                assert_eq!(param_id, t_id);
                assert!(resolver.types_equal(&expected, &Type::i32()));
                assert!(resolver.types_equal(&found, &Type::i64()));
            }
            other => panic!("Expected TypeMismatch, got {:?}", other),
        }
    }

    #[test]
    fn test_instantiate_generic_arity_mismatch() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        let t_id = TyVarId::new(1);
        let t_param = TypeParam {
            name: "T".to_string(),
            id: t_id,
            constraints: vec![],
        };
        let t_type = Type::param(t_id);

        let method = make_generic_candidate(
            "identity",
            vec![t_param],
            vec![t_type.clone()],
            t_type,
        );

        // Call with wrong arity
        let result = resolver.instantiate_generic(&method, &[Type::i32(), Type::i32()]);
        match result {
            InstantiationResult::ArityMismatch { expected, found } => {
                assert_eq!(expected, 1);
                assert_eq!(found, 2);
            }
            other => panic!("Expected ArityMismatch, got {:?}", other),
        }
    }

    #[test]
    fn test_is_applicable_generic_method() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        // Generic method: fn identity<T>(x: T) -> T
        let t_id = TyVarId::new(1);
        let t_param = TypeParam {
            name: "T".to_string(),
            id: t_id,
            constraints: vec![],
        };
        let t_type = Type::param(t_id);

        let method = make_generic_candidate(
            "identity",
            vec![t_param],
            vec![t_type.clone()],
            t_type,
        );

        // Should be applicable for any type
        assert!(resolver.is_applicable(&method, &[Type::i32()]));
        assert!(resolver.is_applicable(&method, &[Type::bool()]));
        assert!(resolver.is_applicable(&method, &[Type::str()]));

        // Wrong arity should not be applicable
        assert!(!resolver.is_applicable(&method, &[]));
        assert!(!resolver.is_applicable(&method, &[Type::i32(), Type::i32()]));
    }

    #[test]
    fn test_dispatch_with_generic_method() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        // Generic method: fn show<T>(x: T) -> ()
        let t_id = TyVarId::new(1);
        let t_param = TypeParam {
            name: "T".to_string(),
            id: t_id,
            constraints: vec![],
        };
        let t_type = Type::param(t_id);

        let generic_method = make_generic_candidate(
            "show",
            vec![t_param],
            vec![t_type],
            Type::unit(),
        );

        // Specific method: fn show(x: i32) -> ()
        let specific_method = make_candidate("show", vec![Type::i32()], Type::unit());

        let candidates = vec![generic_method, specific_method];

        // Calling with i32 should match both, but specific should win
        let result = resolver.resolve("show", &[Type::i32()], &candidates);
        match result {
            DispatchResult::Resolved(m) => {
                // Should resolve to the specific method (no type params)
                assert!(m.type_params.is_empty());
                assert!(resolver.types_equal(&m.param_types[0], &Type::i32()));
            }
            other => panic!("Expected Resolved, got {:?}", other),
        }

        // Calling with bool should only match generic
        let result = resolver.resolve("show", &[Type::bool()], &candidates);
        match result {
            DispatchResult::Resolved(m) => {
                // Should resolve to generic method
                assert_eq!(m.name, "show");
            }
            other => panic!("Expected Resolved, got {:?}", other),
        }
    }

    #[test]
    fn test_instantiate_generic_with_adt() {
        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        // Simulate: fn get<T>(container: Option<T>) -> T
        // Where Option is an ADT with def_id 100
        let option_def_id = DefId::new(100);
        let t_id = TyVarId::new(1);
        let t_param = TypeParam {
            name: "T".to_string(),
            id: t_id,
            constraints: vec![],
        };
        let t_type = Type::param(t_id);
        let option_t = Type::adt(option_def_id, vec![t_type.clone()]);

        let method = make_generic_candidate(
            "get",
            vec![t_param],
            vec![option_t],
            t_type,
        );

        // Call with Option<i32>
        let option_i32 = Type::adt(option_def_id, vec![Type::i32()]);
        let result = resolver.instantiate_generic(&method, &[option_i32]);
        match result {
            InstantiationResult::Success { substitutions, candidate } => {
                // T should be i32
                assert_eq!(substitutions.get(&t_id), Some(&Type::i32()));
                // Return should be i32
                assert!(resolver.types_equal(&candidate.return_type, &Type::i32()));
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    // ============================================================
    // Type Stability Tests
    // ============================================================

    #[test]
    fn test_type_stability_same_return_type() {
        // Stable: Two methods with same input types returning same type
        let unifier = Unifier::new();
        let checker = TypeStabilityChecker::new(&unifier);

        let candidates = vec![
            make_candidate("add", vec![Type::i32(), Type::i32()], Type::i32()),
            make_candidate("add", vec![Type::i64(), Type::i64()], Type::i64()),
        ];

        let result = checker.check_family("add", &candidates);
        assert!(result.is_stable);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_type_stability_different_input_types() {
        // Stable: Different input types, different return types (no overlap)
        let unifier = Unifier::new();
        let checker = TypeStabilityChecker::new(&unifier);

        let candidates = vec![
            make_candidate("process", vec![Type::i32()], Type::i32()),
            make_candidate("process", vec![Type::str()], Type::str()),
        ];

        let result = checker.check_family("process", &candidates);
        assert!(result.is_stable);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_type_stability_unstable_same_inputs() {
        // Unstable: Same input types but different return types
        let unifier = Unifier::new();
        let checker = TypeStabilityChecker::new(&unifier);

        let candidates = vec![
            make_candidate("convert", vec![Type::i32()], Type::i32()),
            make_candidate("convert", vec![Type::i32()], Type::str()),
        ];

        let result = checker.check_family("convert", &candidates);
        assert!(!result.is_stable);
        assert_eq!(result.errors.len(), 1);

        let error = &result.errors[0];
        assert_eq!(error.method_family, "convert");
        assert!(error.conflict_inputs.is_some());
    }

    #[test]
    fn test_type_stability_different_arity() {
        // Stable: Different arities can't conflict
        let unifier = Unifier::new();
        let checker = TypeStabilityChecker::new(&unifier);

        let candidates = vec![
            make_candidate("func", vec![Type::i32()], Type::i32()),
            make_candidate("func", vec![Type::i32(), Type::i32()], Type::str()),
        ];

        let result = checker.check_family("func", &candidates);
        assert!(result.is_stable);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_type_stability_generic_methods_compatible() {
        // Stable: Two generic methods with compatible return types
        let unifier = Unifier::new();
        let checker = TypeStabilityChecker::new(&unifier);

        let t_id = TyVarId::new(0);
        let t_type = Type::new(TypeKind::Param(t_id));

        let m1 = MethodCandidate {
            def_id: DefId::new(1),
            name: "identity".to_string(),
            param_types: vec![t_type.clone()],
            return_type: t_type.clone(),
            type_params: vec![TypeParam {
                name: "T".to_string(),
                id: t_id,
                constraints: vec![],
            }],
            effects: None,
            trait_id: None,
        };

        let u_id = TyVarId::new(1);
        let u_type = Type::new(TypeKind::Param(u_id));

        let m2 = MethodCandidate {
            def_id: DefId::new(2),
            name: "identity".to_string(),
            param_types: vec![u_type.clone()],
            return_type: u_type.clone(),
            type_params: vec![TypeParam {
                name: "U".to_string(),
                id: u_id,
                constraints: vec![],
            }],
            effects: None,
            trait_id: None,
        };

        let candidates = vec![m1, m2];
        let result = checker.check_family("identity", &candidates);

        // Both are generic with type param in return position determined by input
        assert!(result.is_stable);
    }

    #[test]
    fn test_type_stability_error_display() {
        let m1 = make_candidate("foo", vec![Type::i32()], Type::i32());
        let m2 = make_candidate("foo", vec![Type::i32()], Type::str());

        let error = TypeStabilityError {
            method_family: "foo".to_string(),
            method1: m1,
            method2: m2,
            conflict_inputs: Some(vec![Type::i32()]),
            explanation: "Test explanation".to_string(),
        };

        let display = format!("{}", error);
        assert!(display.contains("type instability detected"));
        assert!(display.contains("foo"));
        assert!(display.contains("conflicting methods"));
    }

    #[test]
    fn test_type_stability_tuple_inputs_no_overlap() {
        // Stable: Tuple inputs with different structures don't overlap
        let unifier = Unifier::new();
        let checker = TypeStabilityChecker::new(&unifier);

        let candidates = vec![
            make_candidate(
                "process",
                vec![Type::tuple(vec![Type::i32(), Type::bool()])],
                Type::i32(),
            ),
            make_candidate(
                "process",
                vec![Type::tuple(vec![Type::str(), Type::bool()])],
                Type::str(),
            ),
        ];

        let result = checker.check_family("process", &candidates);
        assert!(result.is_stable);
    }

    #[test]
    fn test_type_stability_single_method() {
        // Stable: Single method is always stable
        let unifier = Unifier::new();
        let checker = TypeStabilityChecker::new(&unifier);

        let candidates = vec![make_candidate("single", vec![Type::i32()], Type::bool())];

        let result = checker.check_family("single", &candidates);
        assert!(result.is_stable);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_type_stability_empty_family() {
        // Stable: Empty method family is trivially stable
        let unifier = Unifier::new();
        let checker = TypeStabilityChecker::new(&unifier);

        let candidates: Vec<MethodCandidate> = vec![];
        let result = checker.check_family("empty", &candidates);
        assert!(result.is_stable);
    }

    #[test]
    fn test_type_stability_result_constructors() {
        // Test TypeStabilityResult constructors
        let stable = TypeStabilityResult::stable();
        assert!(stable.is_stable);
        assert!(stable.errors.is_empty());

        let error = TypeStabilityError {
            method_family: "test".to_string(),
            method1: make_candidate("test", vec![], Type::unit()),
            method2: make_candidate("test", vec![], Type::bool()),
            conflict_inputs: None,
            explanation: "test".to_string(),
        };

        let unstable = TypeStabilityResult::unstable(vec![error]);
        assert!(!unstable.is_stable);
        assert_eq!(unstable.errors.len(), 1);
    }

    #[test]
    fn test_type_stability_check_pair_directly() {
        // Test check_type_stability method directly
        let unifier = Unifier::new();
        let checker = TypeStabilityChecker::new(&unifier);

        let m1 = make_candidate("test", vec![Type::i32()], Type::i32());
        let m2 = make_candidate("test", vec![Type::i32()], Type::i64());

        // These have overlapping inputs but different return types
        let error = checker.check_type_stability("test", &m1, &m2);
        assert!(error.is_some());

        let err = error.unwrap();
        assert_eq!(err.method_family, "test");
    }

    #[test]
    fn test_type_stability_no_conflict_for_non_overlapping() {
        // Test that non-overlapping types don't trigger stability errors
        let unifier = Unifier::new();
        let checker = TypeStabilityChecker::new(&unifier);

        let m1 = make_candidate("test", vec![Type::i32()], Type::i32());
        let m2 = make_candidate("test", vec![Type::bool()], Type::str());

        // Different input types - no overlap possible
        let error = checker.check_type_stability("test", &m1, &m2);
        assert!(error.is_none());
    }

    // ============================================================
    // Constraint Checking Tests
    // ============================================================

    fn make_constrained_generic_candidate(
        name: &str,
        type_params: Vec<TypeParam>,
        params: Vec<Type>,
        ret: Type,
    ) -> MethodCandidate {
        MethodCandidate {
            def_id: DefId::new(0),
            name: name.to_string(),
            param_types: params,
            return_type: ret,
            type_params,
            effects: None,
            trait_id: None,
        }
    }

    #[test]
    fn test_constraint_checker_ord_satisfied_by_i32() {
        // Generic method: fn sort<T: Ord>(x: T) -> T
        let t_id = TyVarId::new(1);
        let t_param = TypeParam {
            name: "T".to_string(),
            id: t_id,
            constraints: vec![Constraint { trait_name: "Ord".to_string() }],
        };
        let t_type = Type::param(t_id);

        let method = make_constrained_generic_candidate(
            "sort",
            vec![t_param],
            vec![t_type.clone()],
            t_type,
        );

        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        // Call with i32 - should satisfy Ord constraint
        let result = resolver.instantiate_generic(&method, &[Type::i32()]);
        match result {
            InstantiationResult::Success { substitutions, candidate } => {
                assert_eq!(substitutions.get(&t_id), Some(&Type::i32()));
                assert!(resolver.types_equal(&candidate.param_types[0], &Type::i32()));
                assert!(resolver.types_equal(&candidate.return_type, &Type::i32()));
            }
            other => panic!("Expected Success, got {:?}", other),
        }
    }

    #[test]
    fn test_constraint_checker_ord_not_satisfied_by_adt() {
        // Generic method: fn sort<T: Ord>(x: T) -> T
        let t_id = TyVarId::new(1);
        let t_param = TypeParam {
            name: "T".to_string(),
            id: t_id,
            constraints: vec![Constraint { trait_name: "Ord".to_string() }],
        };
        let t_type = Type::param(t_id);

        let method = make_constrained_generic_candidate(
            "sort",
            vec![t_param],
            vec![t_type.clone()],
            t_type,
        );

        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        // Call with an ADT (custom struct) - should NOT satisfy Ord constraint
        // ADTs don't implement Ord by default
        let custom_struct = Type::adt(DefId::new(999), vec![]);
        let result = resolver.instantiate_generic(&method, &[custom_struct.clone()]);
        match result {
            InstantiationResult::ConstraintNotSatisfied(err) => {
                assert_eq!(err.param_name, "T");
                assert_eq!(err.param_id, t_id);
                assert_eq!(err.constraint.trait_name, "Ord");
                assert!(resolver.types_equal(&err.concrete_type, &custom_struct));
            }
            other => panic!("Expected ConstraintNotSatisfied, got {:?}", other),
        }
    }

    #[test]
    fn test_constraint_checker_multiple_constraints() {
        // Generic method: fn process<T: Ord + Hash>(x: T) -> T
        let t_id = TyVarId::new(1);
        let t_param = TypeParam {
            name: "T".to_string(),
            id: t_id,
            constraints: vec![
                Constraint { trait_name: "Ord".to_string() },
                Constraint { trait_name: "Hash".to_string() },
            ],
        };
        let t_type = Type::param(t_id);

        let method = make_constrained_generic_candidate(
            "process",
            vec![t_param],
            vec![t_type.clone()],
            t_type,
        );

        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        // Call with i32 - should satisfy both Ord and Hash constraints
        let result = resolver.instantiate_generic(&method, &[Type::i32()]);
        match result {
            InstantiationResult::Success { substitutions, .. } => {
                assert_eq!(substitutions.get(&t_id), Some(&Type::i32()));
            }
            other => panic!("Expected Success for i32, got {:?}", other),
        }

        // Call with f64 - should fail because floats don't implement Ord/Hash
        let result = resolver.instantiate_generic(&method, &[Type::f64()]);
        match result {
            InstantiationResult::ConstraintNotSatisfied(err) => {
                assert_eq!(err.param_name, "T");
                // Should fail on first unsatisfied constraint (Ord)
                assert_eq!(err.constraint.trait_name, "Ord");
            }
            other => panic!("Expected ConstraintNotSatisfied for f64, got {:?}", other),
        }
    }

    #[test]
    fn test_constraint_checker_copy_constraint() {
        // Generic method: fn copy_val<T: Copy>(x: T) -> T
        let t_id = TyVarId::new(1);
        let t_param = TypeParam {
            name: "T".to_string(),
            id: t_id,
            constraints: vec![Constraint { trait_name: "Copy".to_string() }],
        };
        let t_type = Type::param(t_id);

        let method = make_constrained_generic_candidate(
            "copy_val",
            vec![t_param],
            vec![t_type.clone()],
            t_type,
        );

        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        // i32 is Copy
        let result = resolver.instantiate_generic(&method, &[Type::i32()]);
        assert!(matches!(result, InstantiationResult::Success { .. }));

        // Tuple of Copy types is Copy
        let tuple = Type::tuple(vec![Type::i32(), Type::bool()]);
        let result = resolver.instantiate_generic(&method, &[tuple]);
        assert!(matches!(result, InstantiationResult::Success { .. }));

        // ADTs are not Copy by default
        let adt = Type::adt(DefId::new(100), vec![]);
        let result = resolver.instantiate_generic(&method, &[adt]);
        assert!(matches!(result, InstantiationResult::ConstraintNotSatisfied(_)));
    }

    #[test]
    fn test_constraint_checker_sized_constraint() {
        // Generic method: fn sized_val<T: Sized>(x: T) -> T
        let t_id = TyVarId::new(1);
        let t_param = TypeParam {
            name: "T".to_string(),
            id: t_id,
            constraints: vec![Constraint { trait_name: "Sized".to_string() }],
        };
        let t_type = Type::param(t_id);

        let method = make_constrained_generic_candidate(
            "sized_val",
            vec![t_param],
            vec![t_type.clone()],
            t_type,
        );

        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        // i32 is Sized
        let result = resolver.instantiate_generic(&method, &[Type::i32()]);
        assert!(matches!(result, InstantiationResult::Success { .. }));

        // str is NOT Sized
        let result = resolver.instantiate_generic(&method, &[Type::str()]);
        assert!(matches!(result, InstantiationResult::ConstraintNotSatisfied(_)));

        // Slices are NOT Sized
        let slice = Type::slice(Type::i32());
        let result = resolver.instantiate_generic(&method, &[slice]);
        assert!(matches!(result, InstantiationResult::ConstraintNotSatisfied(_)));
    }

    #[test]
    fn test_constraint_checker_default_constraint() {
        // Generic method: fn make_default<T: Default>() -> T
        // (For testing, we use a parameter to infer T)
        let t_id = TyVarId::new(1);
        let t_param = TypeParam {
            name: "T".to_string(),
            id: t_id,
            constraints: vec![Constraint { trait_name: "Default".to_string() }],
        };
        let t_type = Type::param(t_id);

        let method = make_constrained_generic_candidate(
            "make_default",
            vec![t_param],
            vec![t_type.clone()],
            t_type,
        );

        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        // i32 has Default (0)
        let result = resolver.instantiate_generic(&method, &[Type::i32()]);
        assert!(matches!(result, InstantiationResult::Success { .. }));

        // bool has Default (false)
        let result = resolver.instantiate_generic(&method, &[Type::bool()]);
        assert!(matches!(result, InstantiationResult::Success { .. }));

        // str does NOT have Default
        let result = resolver.instantiate_generic(&method, &[Type::str()]);
        assert!(matches!(result, InstantiationResult::ConstraintNotSatisfied(_)));
    }

    #[test]
    fn test_constraint_checker_with_custom_trait_checker() {
        // Generic method: fn custom<T: MyTrait>(x: T) -> T
        let t_id = TyVarId::new(1);
        let t_param = TypeParam {
            name: "T".to_string(),
            id: t_id,
            constraints: vec![Constraint { trait_name: "MyTrait".to_string() }],
        };
        let t_type = Type::param(t_id);

        let method = make_constrained_generic_candidate(
            "custom",
            vec![t_param],
            vec![t_type.clone()],
            t_type,
        );

        // Custom trait checker: only i32 implements MyTrait
        let custom_checker: &TraitConstraintChecker = &|ty: &Type, trait_name: &str| {
            trait_name == "MyTrait"
                && matches!(ty.kind.as_ref(), TypeKind::Primitive(PrimitiveTy::Int(crate::hir::def::IntTy::I32)))
        };
        let constraint_checker = ConstraintChecker::with_trait_checker(custom_checker);

        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        // i32 implements MyTrait
        let result = resolver.instantiate_generic_with_constraint_checker(
            &method, &[Type::i32()], Some(&constraint_checker)
        );
        assert!(matches!(result, InstantiationResult::Success { .. }));

        // i64 does NOT implement MyTrait
        let result = resolver.instantiate_generic_with_constraint_checker(
            &method, &[Type::i64()], Some(&constraint_checker)
        );
        match result {
            InstantiationResult::ConstraintNotSatisfied(err) => {
                assert_eq!(err.constraint.trait_name, "MyTrait");
            }
            other => panic!("Expected ConstraintNotSatisfied, got {:?}", other),
        }
    }

    #[test]
    fn test_constraint_checker_no_constraints_succeeds() {
        // Generic method without constraints: fn identity<T>(x: T) -> T
        let t_id = TyVarId::new(1);
        let t_param = TypeParam {
            name: "T".to_string(),
            id: t_id,
            constraints: vec![], // No constraints
        };
        let t_type = Type::param(t_id);

        let method = make_constrained_generic_candidate(
            "identity",
            vec![t_param],
            vec![t_type.clone()],
            t_type,
        );

        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        // Should succeed for any type
        let result = resolver.instantiate_generic(&method, &[Type::i32()]);
        assert!(matches!(result, InstantiationResult::Success { .. }));

        let adt = Type::adt(DefId::new(100), vec![]);
        let result = resolver.instantiate_generic(&method, &[adt]);
        assert!(matches!(result, InstantiationResult::Success { .. }));
    }

    #[test]
    fn test_constraint_checker_multiple_type_params() {
        // Generic method: fn pair<T: Ord, U: Copy>(x: T, y: U) -> (T, U)
        let t_id = TyVarId::new(1);
        let u_id = TyVarId::new(2);
        let t_param = TypeParam {
            name: "T".to_string(),
            id: t_id,
            constraints: vec![Constraint { trait_name: "Ord".to_string() }],
        };
        let u_param = TypeParam {
            name: "U".to_string(),
            id: u_id,
            constraints: vec![Constraint { trait_name: "Copy".to_string() }],
        };
        let t_type = Type::param(t_id);
        let u_type = Type::param(u_id);
        let ret_type = Type::tuple(vec![t_type.clone(), u_type.clone()]);

        let method = make_constrained_generic_candidate(
            "pair",
            vec![t_param, u_param],
            vec![t_type, u_type],
            ret_type,
        );

        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        // Both i32 (satisfies both Ord and Copy)
        let result = resolver.instantiate_generic(&method, &[Type::i32(), Type::bool()]);
        assert!(matches!(result, InstantiationResult::Success { .. }));

        // T=f64 fails (f64 doesn't satisfy Ord)
        let result = resolver.instantiate_generic(&method, &[Type::f64(), Type::i32()]);
        match result {
            InstantiationResult::ConstraintNotSatisfied(err) => {
                assert_eq!(err.param_name, "T");
                assert_eq!(err.constraint.trait_name, "Ord");
            }
            other => panic!("Expected ConstraintNotSatisfied for T, got {:?}", other),
        }

        // U=ADT fails (ADT doesn't satisfy Copy)
        let adt = Type::adt(DefId::new(100), vec![]);
        let result = resolver.instantiate_generic(&method, &[Type::i32(), adt]);
        match result {
            InstantiationResult::ConstraintNotSatisfied(err) => {
                assert_eq!(err.param_name, "U");
                assert_eq!(err.constraint.trait_name, "Copy");
            }
            other => panic!("Expected ConstraintNotSatisfied for U, got {:?}", other),
        }
    }

    #[test]
    fn test_constraint_checker_dispatch_filters_by_constraints() {
        // Two generic methods with different constraints:
        // fn process<T: Ord>(x: T) -> T       // More specific (requires Ord)
        // fn process<T>(x: T) -> T            // Less specific (no constraints)

        let t_id_constrained = TyVarId::new(1);
        let t_id_unconstrained = TyVarId::new(2);

        // Constrained version: T: Ord
        let constrained_method = make_constrained_generic_candidate(
            "process",
            vec![TypeParam {
                name: "T".to_string(),
                id: t_id_constrained,
                constraints: vec![Constraint { trait_name: "Ord".to_string() }],
            }],
            vec![Type::param(t_id_constrained)],
            Type::param(t_id_constrained),
        );

        // Unconstrained version
        let unconstrained_method = make_constrained_generic_candidate(
            "process",
            vec![TypeParam {
                name: "T".to_string(),
                id: t_id_unconstrained,
                constraints: vec![],
            }],
            vec![Type::param(t_id_unconstrained)],
            Type::param(t_id_unconstrained),
        );

        let unifier = Unifier::new();
        let resolver = DispatchResolver::new(&unifier);

        // For i32, constrained version should be applicable (i32: Ord)
        assert!(resolver.is_applicable(&constrained_method, &[Type::i32()]));

        // For ADT, constrained version should NOT be applicable (ADT: !Ord)
        let adt = Type::adt(DefId::new(100), vec![]);
        assert!(!resolver.is_applicable(&constrained_method, &[adt.clone()]));

        // But unconstrained version should be applicable for ADT
        assert!(resolver.is_applicable(&unconstrained_method, &[adt]));
    }

    #[test]
    fn test_constraint_checker_default_impl() {
        // Test Default implementation
        let checker = ConstraintChecker::default();
        let substitutions: HashMap<TyVarId, Type> = HashMap::new();
        let type_params: Vec<TypeParam> = vec![];

        // Empty params should succeed
        assert!(checker.check_constraints(&type_params, &substitutions).is_ok());
    }
}
