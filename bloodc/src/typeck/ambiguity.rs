//! Ambiguity detection for multiple dispatch.
//!
//! Blood treats ambiguous dispatch as a compile-time error. This module
//! implements detection of ambiguity at method definition time, catching
//! potential dispatch conflicts before any calls are made.
//!
//! # Ambiguity Scenarios
//!
//! Ambiguity occurs when two methods could both match the same call
//! without one being strictly more specific than the other:
//!
//! ```blood
//! fn process<T: Serialize>(x: T) -> Bytes { ... }
//! fn process<T: Hash>(x: T) -> Bytes { ... }
//!
//! // ERROR: Ambiguous if value: Serialize + Hash
//! process(value)
//! ```
//!
//! See DISPATCH.md Section 5 for full specification.

use std::fmt;

use crate::hir::{Type, TypeKind};

use super::dispatch::{DispatchResolver, MethodCandidate};
use super::methods::MethodFamily;
use super::unify::Unifier;

/// An ambiguity between two or more methods.
#[derive(Debug, Clone)]
pub struct Ambiguity {
    /// The method name.
    pub method_name: String,
    /// The methods that are ambiguous.
    pub candidates: Vec<MethodCandidate>,
    /// The argument types that cause ambiguity (if known).
    pub conflict_types: Option<Vec<Type>>,
    /// Suggested resolution.
    pub suggestion: AmbiguitySuggestion,
}

impl fmt::Display for Ambiguity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ambiguous dispatch for `{}`", self.method_name)?;

        if let Some(ref types) = self.conflict_types {
            write!(f, " with arguments (")?;
            for (i, ty) in types.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{:?}", ty.kind)?;
            }
            write!(f, ")")?;
        }

        writeln!(f)?;
        writeln!(f, "candidates:")?;
        for candidate in &self.candidates {
            write!(f, "  - {}(", candidate.name)?;
            for (i, ty) in candidate.param_types.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{:?}", ty.kind)?;
            }
            writeln!(f, ")")?;
        }

        match &self.suggestion {
            AmbiguitySuggestion::AddMoreSpecificMethod => {
                writeln!(f, "help: add a more specific method to resolve ambiguity")?;
            }
            AmbiguitySuggestion::UseQualifiedSyntax => {
                writeln!(f, "help: use qualified syntax: <Type as Trait>::method()")?;
            }
            AmbiguitySuggestion::ConstrainTypeParameter { param, with_trait } => {
                writeln!(
                    f,
                    "help: constrain type parameter `{}` with `{}`",
                    param, with_trait
                )?;
            }
            AmbiguitySuggestion::None => {}
        }

        Ok(())
    }
}

/// Suggested resolution for an ambiguity.
#[derive(Debug, Clone)]
pub enum AmbiguitySuggestion {
    /// Add a more specific method that handles the overlapping case.
    AddMoreSpecificMethod,
    /// Use qualified syntax to disambiguate.
    UseQualifiedSyntax,
    /// Constrain a type parameter.
    ConstrainTypeParameter { param: String, with_trait: String },
    /// No suggestion available.
    None,
}

/// Result of ambiguity checking.
#[derive(Debug)]
pub struct AmbiguityCheckResult {
    /// All detected ambiguities.
    pub ambiguities: Vec<Ambiguity>,
    /// Whether the method family is safe (no ambiguities).
    pub is_safe: bool,
}

impl AmbiguityCheckResult {
    /// Create a result with no ambiguities.
    pub fn safe() -> Self {
        Self {
            ambiguities: vec![],
            is_safe: true,
        }
    }

    /// Create a result with ambiguities.
    pub fn ambiguous(ambiguities: Vec<Ambiguity>) -> Self {
        Self {
            is_safe: ambiguities.is_empty(),
            ambiguities,
        }
    }
}

/// Checker for method family ambiguities.
pub struct AmbiguityChecker<'a> {
    /// The unifier for type comparisons.
    unifier: &'a Unifier,
}

impl<'a> AmbiguityChecker<'a> {
    /// Create a new ambiguity checker.
    pub fn new(unifier: &'a Unifier) -> Self {
        Self { unifier }
    }

    /// Check a method family for potential ambiguities.
    ///
    /// This performs pairwise comparison of all methods in the family
    /// to detect any overlapping signatures that could cause ambiguity.
    pub fn check_family(&self, family: &MethodFamily) -> AmbiguityCheckResult {
        let mut ambiguities = Vec::new();
        let resolver = DispatchResolver::new(self.unifier);

        // Check all pairs of methods
        for (i, m1) in family.methods.iter().enumerate() {
            for m2 in family.methods.iter().skip(i + 1) {
                if let Some(ambiguity) = self.check_pair(m1, m2, &resolver) {
                    ambiguities.push(ambiguity);
                }
            }
        }

        AmbiguityCheckResult::ambiguous(ambiguities)
    }

    /// Check if two methods are potentially ambiguous.
    fn check_pair(
        &self,
        m1: &MethodCandidate,
        m2: &MethodCandidate,
        resolver: &DispatchResolver,
    ) -> Option<Ambiguity> {
        // Different arities can't be ambiguous
        if m1.param_types.len() != m2.param_types.len() {
            return None;
        }

        // Check if neither is strictly more specific than the other
        let m1_more_specific = resolver.is_more_specific(m1, m2);
        let m2_more_specific = resolver.is_more_specific(m2, m1);

        // If one is strictly more specific, no ambiguity
        if m1_more_specific || m2_more_specific {
            return None;
        }

        // Check if signatures could overlap
        if !self.signatures_overlap(m1, m2) {
            return None;
        }

        // Detected ambiguity
        Some(Ambiguity {
            method_name: m1.name.clone(),
            candidates: vec![m1.clone(), m2.clone()],
            conflict_types: self.find_conflict_types(m1, m2),
            suggestion: self.suggest_resolution(m1, m2),
        })
    }

    /// Check if two method signatures could overlap.
    ///
    /// Signatures overlap if there exists some concrete type arguments
    /// that would make both methods applicable.
    fn signatures_overlap(&self, m1: &MethodCandidate, m2: &MethodCandidate) -> bool {
        // For each parameter position, check if types could unify
        for (t1, t2) in m1.param_types.iter().zip(&m2.param_types) {
            if !self.types_could_unify(t1, t2) {
                return false;
            }
        }
        true
    }

    /// Check if two types could potentially unify to the same concrete type.
    fn types_could_unify(&self, t1: &Type, t2: &Type) -> bool {
        match (t1.kind.as_ref(), t2.kind.as_ref()) {
            // Same concrete types unify
            (TypeKind::Primitive(p1), TypeKind::Primitive(p2)) => p1 == p2,

            // Type variables unify with anything
            (TypeKind::Infer(_), _)
            | (_, TypeKind::Infer(_))
            | (TypeKind::Param(_), _)
            | (_, TypeKind::Param(_)) => true,

            // Tuples unify if elements unify
            (TypeKind::Tuple(ts1), TypeKind::Tuple(ts2)) => {
                ts1.len() == ts2.len()
                    && ts1.iter().zip(ts2).all(|(a, b)| self.types_could_unify(a, b))
            }

            // Arrays unify if element types unify and lengths match
            (
                TypeKind::Array { element: e1, size: l1 },
                TypeKind::Array { element: e2, size: l2 },
            ) => l1 == l2 && self.types_could_unify(e1, e2),

            // Slices unify if element types unify
            (TypeKind::Slice { element: e1 }, TypeKind::Slice { element: e2 }) => {
                self.types_could_unify(e1, e2)
            }

            // References unify if inner types unify and mutability matches
            (
                TypeKind::Ref { inner: i1, mutable: m1 },
                TypeKind::Ref { inner: i2, mutable: m2 },
            ) => m1 == m2 && self.types_could_unify(i1, i2),

            // ADTs unify if they're the same type
            (
                TypeKind::Adt { def_id: d1, .. },
                TypeKind::Adt { def_id: d2, .. },
            ) => d1 == d2,

            // Function types unify if signatures unify
            (
                TypeKind::Fn { params: p1, ret: r1 },
                TypeKind::Fn { params: p2, ret: r2 },
            ) => {
                p1.len() == p2.len()
                    && p1.iter().zip(p2).all(|(a, b)| self.types_could_unify(a, b))
                    && self.types_could_unify(r1, r2)
            }

            // Different concrete types don't unify
            _ => false,
        }
    }

    /// Find example types that cause the conflict.
    fn find_conflict_types(&self, m1: &MethodCandidate, m2: &MethodCandidate) -> Option<Vec<Type>> {
        // Try to find concrete types that would match both methods
        // For now, return the first method's types if they're concrete
        let is_type_var = |t: &Type| {
            matches!(t.kind.as_ref(), TypeKind::Infer(_) | TypeKind::Param(_))
        };

        let types: Vec<Type> = m1
            .param_types
            .iter()
            .zip(&m2.param_types)
            .map(|(t1, t2)| {
                // Prefer concrete type, or first type if both are generic
                if !is_type_var(t1) {
                    t1.clone()
                } else {
                    t2.clone()
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

    /// Suggest a resolution for the ambiguity.
    fn suggest_resolution(
        &self,
        m1: &MethodCandidate,
        m2: &MethodCandidate,
    ) -> AmbiguitySuggestion {
        // If both have type parameters with different constraints,
        // suggest adding a more specific method
        let m1_has_generics = !m1.type_params.is_empty();
        let m2_has_generics = !m2.type_params.is_empty();

        if m1_has_generics && m2_has_generics {
            // Check if constraints are different
            let m1_constraints: Vec<_> = m1
                .type_params
                .iter()
                .flat_map(|p| p.constraints.iter().map(|c| &c.trait_name))
                .collect();
            let m2_constraints: Vec<_> = m2
                .type_params
                .iter()
                .flat_map(|p| p.constraints.iter().map(|c| &c.trait_name))
                .collect();

            if m1_constraints != m2_constraints {
                // Suggest combining constraints
                if let (Some(p1), Some(p2)) =
                    (m1.type_params.first(), m2.type_params.first())
                {
                    if !p1.constraints.is_empty() && !p2.constraints.is_empty() {
                        return AmbiguitySuggestion::ConstrainTypeParameter {
                            param: p1.name.clone(),
                            with_trait: format!(
                                "{} + {}",
                                p1.constraints[0].trait_name,
                                p2.constraints[0].trait_name
                            ),
                        };
                    }
                }
            }
        }

        // Default suggestion
        AmbiguitySuggestion::AddMoreSpecificMethod
    }

    /// Check if a new method would introduce ambiguity to a family.
    pub fn check_new_method(
        &self,
        family: &MethodFamily,
        new_method: &MethodCandidate,
    ) -> AmbiguityCheckResult {
        let mut ambiguities = Vec::new();
        let resolver = DispatchResolver::new(self.unifier);

        for existing in &family.methods {
            if let Some(ambiguity) = self.check_pair(existing, new_method, &resolver) {
                ambiguities.push(ambiguity);
            }
        }

        AmbiguityCheckResult::ambiguous(ambiguities)
    }
}

/// Check a call site for ambiguity given the argument types.
pub fn check_call_site_ambiguity(
    unifier: &Unifier,
    family: &MethodFamily,
    arg_types: &[Type],
) -> Option<Ambiguity> {
    let resolver = DispatchResolver::new(unifier);

    // Find all applicable methods
    let applicable: Vec<_> = family
        .methods
        .iter()
        .filter(|m| resolver.is_applicable(m, arg_types))
        .collect();

    if applicable.len() <= 1 {
        return None;
    }

    // Find maximally specific methods
    let mut maximal = Vec::new();
    for m in &applicable {
        let is_maximal = !applicable
            .iter()
            .any(|other| !std::ptr::eq(*m, *other) && resolver.is_more_specific(other, m));
        if is_maximal {
            maximal.push((*m).clone());
        }
    }

    // Ambiguity if more than one maximal method
    if maximal.len() > 1 {
        Some(Ambiguity {
            method_name: family.name.clone(),
            candidates: maximal,
            conflict_types: Some(arg_types.to_vec()),
            suggestion: AmbiguitySuggestion::AddMoreSpecificMethod,
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::dispatch::TypeParam;
    use crate::hir::DefId;

    fn make_candidate(name: &str, params: Vec<Type>) -> MethodCandidate {
        MethodCandidate {
            def_id: DefId::new(0),
            name: name.to_string(),
            param_types: params,
            return_type: Type::unit(),
            type_params: vec![],
            effects: None,
        }
    }

    #[test]
    fn test_no_ambiguity_different_types() {
        let unifier = Unifier::new();
        let checker = AmbiguityChecker::new(&unifier);

        let mut family = super::super::methods::MethodFamily::new("add".to_string());
        family.add_method(make_candidate("add", vec![Type::i32(), Type::i32()]));
        family.add_method(make_candidate("add", vec![Type::i64(), Type::i64()]));

        let result = checker.check_family(&family);
        assert!(result.is_safe);
    }

    #[test]
    fn test_no_ambiguity_different_arity() {
        let unifier = Unifier::new();
        let checker = AmbiguityChecker::new(&unifier);

        let mut family = super::super::methods::MethodFamily::new("add".to_string());
        family.add_method(make_candidate("add", vec![Type::i32()]));
        family.add_method(make_candidate("add", vec![Type::i32(), Type::i32()]));

        let result = checker.check_family(&family);
        assert!(result.is_safe);
    }

    #[test]
    fn test_ambiguity_with_generics() {
        let unifier = Unifier::new();
        let checker = AmbiguityChecker::new(&unifier);

        // Two methods with type variables that could overlap
        let m1 = MethodCandidate {
            def_id: DefId::new(1),
            name: "process".to_string(),
            param_types: vec![Type::infer(crate::hir::TyVarId(0))],
            return_type: Type::unit(),
            type_params: vec![TypeParam {
                name: "T".to_string(),
                id: crate::hir::TyVarId::new(0),
                constraints: vec![super::super::dispatch::Constraint {
                    trait_name: "Serialize".to_string(),
                }],
            }],
            effects: None,
        };

        let m2 = MethodCandidate {
            def_id: DefId::new(2),
            name: "process".to_string(),
            param_types: vec![Type::infer(crate::hir::TyVarId(1))],
            return_type: Type::unit(),
            type_params: vec![TypeParam {
                name: "T".to_string(),
                id: crate::hir::TyVarId::new(1),
                constraints: vec![super::super::dispatch::Constraint {
                    trait_name: "Hash".to_string(),
                }],
            }],
            effects: None,
        };

        let mut family = super::super::methods::MethodFamily::new("process".to_string());
        family.add_method(m1);
        family.add_method(m2);

        let result = checker.check_family(&family);
        // These should be detected as ambiguous since a type could implement both
        assert!(!result.is_safe);
    }
}
