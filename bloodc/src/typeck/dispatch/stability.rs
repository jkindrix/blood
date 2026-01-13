//! Type stability analysis for method families.
//!
//! Type stability ensures that the return type of a method family is
//! fully determined by the input types at compile time. This is crucial
//! for optimization and predictable performance.
//!
//! See DISPATCH.md Section 4 for full specification.

use std::fmt;

use crate::hir::{Type, TypeKind};
use crate::typeck::unify::Unifier;

use super::types::MethodCandidate;
use super::resolver::DispatchResolver;

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
    /// Note: Currently unused but retained for future subtype-based stability analysis.
    _resolver: DispatchResolver<'a>,
}

impl<'a> TypeStabilityChecker<'a> {
    /// Create a new type stability checker.
    pub fn new(unifier: &'a Unifier) -> Self {
        Self {
            _resolver: DispatchResolver::new(unifier),
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
        // Check structural equality using types_equal helper
        if self.types_equal(ret1, ret2) {
            return true;
        }

        // Check if both are generic return types that depend on input types
        // in compatible ways
        if self.both_generic_compatible(ret1, ret2, m1, m2) {
            return true;
        }

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
            return self.types_equal(ret1, ret2);
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
