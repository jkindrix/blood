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

use crate::hir::{DefId, Type, TypeKind, PrimitiveTy};
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
}

/// A type parameter with optional constraints.
#[derive(Debug, Clone)]
pub struct TypeParam {
    /// The type parameter name.
    pub name: String,
    /// Constraints on the type parameter.
    pub constraints: Vec<Constraint>,
}

/// A constraint on a type parameter.
#[derive(Debug, Clone)]
pub struct Constraint {
    /// The trait that must be implemented.
    pub trait_name: String,
}

/// An effect row for effect-aware dispatch.
#[derive(Debug, Clone)]
pub struct EffectRow {
    /// The effects in this row.
    pub effects: Vec<String>,
    /// Whether this is an open row (has a row variable).
    pub is_open: bool,
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
            return DispatchResult::Resolved(maximal.into_iter().next().unwrap());
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
    pub fn is_applicable(&self, method: &MethodCandidate, arg_types: &[Type]) -> bool {
        // Check arity
        if method.param_types.len() != arg_types.len() {
            return false;
        }

        // Check each argument against parameter
        for (arg_type, param_type) in arg_types.iter().zip(&method.param_types) {
            if !self.is_subtype(arg_type, param_type) {
                return false;
            }
        }

        true
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

        all_at_least && some_strictly
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
}
