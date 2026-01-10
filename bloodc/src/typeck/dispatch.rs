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

/// Dispatch resolution context.
pub struct DispatchResolver<'a> {
    /// Reference to the unifier for subtype checking.
    /// Currently unused but will be used for constraint-based subtyping.
    #[allow(dead_code)]
    unifier: &'a Unifier,
}

impl<'a> DispatchResolver<'a> {
    /// Create a new dispatch resolver.
    pub fn new(unifier: &'a Unifier) -> Self {
        Self { unifier }
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
    /// For now, this is a structural check. Full subtyping with variance
    /// will be added when traits are implemented.
    fn is_subtype(&self, a: &Type, b: &Type) -> bool {
        // Any type is a subtype of itself
        if self.types_equal(a, b) {
            return true;
        }

        // Never is a subtype of everything
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

        // TODO: Add variance for generic types (Vec<Cat> <: Vec<Animal>)
        // TODO: Add trait-based subtyping

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
}
