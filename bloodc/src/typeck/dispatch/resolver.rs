//! Main dispatch resolution algorithm.

use std::cmp::Ordering;
use std::collections::HashMap;

use crate::hir::{Type, TypeKind, PrimitiveTy, TyVarId};
use crate::typeck::unify::Unifier;

use super::types::{MethodCandidate, InstantiationResult};
use super::effect_row::EffectRow;
use super::result::{DispatchResult, NoMatchError, AmbiguityError, TraitChecker};
use super::constraints::ConstraintChecker;

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
            TypeKind::Fn { params: param_params, ret: param_ret, .. } => {
                match arg_type.kind.as_ref() {
                    TypeKind::Fn { params: arg_params, ret: arg_ret, .. }
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

            TypeKind::Fn { params, ret, .. } => {
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
    pub(crate) fn is_subtype(&self, a: &Type, b: &Type) -> bool {
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
                TypeKind::Fn { params: a_params, ret: a_ret, .. },
                TypeKind::Fn { params: b_params, ret: b_ret, .. },
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
    pub(crate) fn types_equal(&self, a: &Type, b: &Type) -> bool {
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
                TypeKind::Fn { params: a_params, ret: a_ret, .. },
                TypeKind::Fn { params: b_params, ret: b_ret, .. },
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
    pub(crate) fn find_maximal(&self, applicable: &[MethodCandidate]) -> Vec<MethodCandidate> {
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
