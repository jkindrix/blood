//! Variance Inference and Checking
//!
//! This module implements variance rules for generic type parameters.
//!
//! # Variance Overview
//!
//! Variance describes how the subtyping relationship of a generic type
//! relates to the subtyping relationship of its type arguments.
//!
//! - **Covariant** (`+T`): If `A <: B`, then `F<A> <: F<B>`
//!   - Example: `&T` is covariant - `&Cat <: &Animal` when `Cat <: Animal`
//!
//! - **Contravariant** (`-T`): If `A <: B`, then `F<B> <: F<A>` (reversed)
//!   - Example: `fn(T)` is contravariant in T - `fn(Animal) <: fn(Cat)`
//!
//! - **Invariant** (`=T`): Neither relationship holds
//!   - Example: `&mut T` is invariant - `&mut Cat` is NOT subtype of `&mut Animal`
//!
//! - **Bivariant**: Both relationships hold (rare, usually indicates unused parameter)
//!
//! # Inference Algorithm
//!
//! Variance is inferred by analyzing how type parameters appear in struct fields
//! and enum variants:
//!
//! 1. Covariant positions: direct usage, &T, [T], return types
//! 2. Contravariant positions: function parameter types
//! 3. Invariant positions: &mut T, *mut T, any position used both ways
//!
//! The final variance is the "greatest lower bound" of all usages.

use std::collections::HashMap;
use crate::hir::{DefId, Type, TypeKind, TyVarId};
use crate::hir::item::{GenericParam, GenericParamKind};

/// Variance of a type parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Variance {
    /// Covariant: `F<A> <: F<B>` when `A <: B`
    Covariant,
    /// Contravariant: `F<A> <: F<B>` when `B <: A`
    Contravariant,
    /// Invariant: no subtyping relationship
    Invariant,
    /// Bivariant: unused parameter, any relationship holds
    Bivariant,
}

impl Variance {
    /// Combine two variances using "greatest lower bound" semantics.
    ///
    /// When a type parameter appears in multiple positions with different
    /// variances, we take the most restrictive combination.
    ///
    /// Truth table:
    /// ```text
    ///           | Bivariant | Covariant | Contravariant | Invariant
    /// ----------+-----------+-----------+---------------+----------
    /// Bivariant | Bivariant | Covariant | Contravariant | Invariant
    /// Covariant |           | Covariant | Invariant     | Invariant
    /// Contra... |           |           | Contravariant | Invariant
    /// Invariant |           |           |               | Invariant
    /// ```
    pub fn combine(self, other: Variance) -> Variance {
        match (self, other) {
            // Bivariant is the identity - absorbs the other variance
            (Variance::Bivariant, v) | (v, Variance::Bivariant) => v,

            // Same variance stays the same
            (Variance::Covariant, Variance::Covariant) => Variance::Covariant,
            (Variance::Contravariant, Variance::Contravariant) => Variance::Contravariant,
            (Variance::Invariant, Variance::Invariant) => Variance::Invariant,

            // Different non-bivariant variances combine to invariant
            (Variance::Covariant, Variance::Contravariant)
            | (Variance::Contravariant, Variance::Covariant) => Variance::Invariant,

            // Invariant absorbs everything except bivariant
            (Variance::Invariant, _) | (_, Variance::Invariant) => Variance::Invariant,
        }
    }

    /// Transform variance when passing through a position.
    ///
    /// - Covariant position: variance unchanged
    /// - Contravariant position: variance flipped
    /// - Invariant position: always invariant
    pub fn transform(self, position: Variance) -> Variance {
        match position {
            Variance::Covariant => self,
            Variance::Contravariant => self.flip(),
            Variance::Invariant => Variance::Invariant,
            Variance::Bivariant => Variance::Bivariant,
        }
    }

    /// Flip covariant ↔ contravariant, keep invariant/bivariant.
    pub fn flip(self) -> Variance {
        match self {
            Variance::Covariant => Variance::Contravariant,
            Variance::Contravariant => Variance::Covariant,
            Variance::Invariant => Variance::Invariant,
            Variance::Bivariant => Variance::Bivariant,
        }
    }

    /// Returns true if this variance allows covariant subtyping.
    pub fn is_covariant(self) -> bool {
        matches!(self, Variance::Covariant | Variance::Bivariant)
    }

    /// Returns true if this variance allows contravariant subtyping.
    pub fn is_contravariant(self) -> bool {
        matches!(self, Variance::Contravariant | Variance::Bivariant)
    }

    /// Returns true if this is the most restrictive variance.
    pub fn is_invariant(self) -> bool {
        matches!(self, Variance::Invariant)
    }

    /// Get the annotation string for this variance.
    pub fn annotation(&self) -> &'static str {
        match self {
            Variance::Covariant => "+",
            Variance::Contravariant => "-",
            Variance::Invariant => "=",
            Variance::Bivariant => "*",
        }
    }
}

impl std::fmt::Display for Variance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Variance::Covariant => write!(f, "covariant"),
            Variance::Contravariant => write!(f, "contravariant"),
            Variance::Invariant => write!(f, "invariant"),
            Variance::Bivariant => write!(f, "bivariant"),
        }
    }
}

/// Variance information for a generic definition.
#[derive(Debug, Clone, Default)]
pub struct VarianceInfo {
    /// Variance of each type parameter, indexed by position.
    pub param_variances: Vec<Variance>,
}

impl VarianceInfo {
    /// Create new variance info with all parameters bivariant (unused).
    pub fn new(param_count: usize) -> Self {
        Self {
            param_variances: vec![Variance::Bivariant; param_count],
        }
    }

    /// Get variance for a parameter by index.
    pub fn get(&self, index: usize) -> Option<Variance> {
        self.param_variances.get(index).copied()
    }

    /// Set variance for a parameter by index.
    pub fn set(&mut self, index: usize, variance: Variance) {
        if index < self.param_variances.len() {
            self.param_variances[index] = variance;
        }
    }

    /// Combine with a new variance observation for a parameter.
    pub fn observe(&mut self, index: usize, variance: Variance) {
        if index < self.param_variances.len() {
            self.param_variances[index] = self.param_variances[index].combine(variance);
        }
    }
}

/// Infers variance for type parameters in generic types.
pub struct VarianceInferrer {
    /// Mapping from DefId to inferred variance information.
    results: HashMap<DefId, VarianceInfo>,
    /// Type parameters currently in scope (DefId -> parameter index).
    param_map: HashMap<TyVarId, usize>,
    /// Current variance being accumulated.
    current_variance: Variance,
}

impl VarianceInferrer {
    /// Create a new variance inferrer.
    pub fn new() -> Self {
        Self {
            results: HashMap::new(),
            param_map: HashMap::new(),
            current_variance: Variance::Covariant,
        }
    }

    /// Get the inferred variance results.
    pub fn into_results(self) -> HashMap<DefId, VarianceInfo> {
        self.results
    }

    /// Get variance info for a definition, if computed.
    pub fn get(&self, def_id: DefId) -> Option<&VarianceInfo> {
        self.results.get(&def_id)
    }

    /// Infer variance for a struct definition.
    pub fn infer_struct(
        &mut self,
        def_id: DefId,
        params: &[GenericParam],
        field_types: &[Type],
    ) {
        // Initialize variance info
        let param_count = params.iter()
            .filter(|p| matches!(p.kind, GenericParamKind::Type { .. }))
            .count();
        let mut info = VarianceInfo::new(param_count);

        // Build parameter map
        self.param_map.clear();
        let mut idx = 0;
        for param in params {
            if matches!(param.kind, GenericParamKind::Type { .. }) {
                // Use TyVarId from DefId (simplified mapping)
                self.param_map.insert(TyVarId(param.def_id.index()), idx);
                idx += 1;
            }
        }

        // Analyze each field type in covariant position
        for field_ty in field_types {
            self.current_variance = Variance::Covariant;
            self.analyze_type(field_ty, &mut info);
        }

        self.results.insert(def_id, info);
    }

    /// Infer variance for an enum definition.
    pub fn infer_enum(
        &mut self,
        def_id: DefId,
        params: &[GenericParam],
        variant_field_types: &[Vec<Type>],
    ) {
        // Initialize variance info
        let param_count = params.iter()
            .filter(|p| matches!(p.kind, GenericParamKind::Type { .. }))
            .count();
        let mut info = VarianceInfo::new(param_count);

        // Build parameter map
        self.param_map.clear();
        let mut idx = 0;
        for param in params {
            if matches!(param.kind, GenericParamKind::Type { .. }) {
                self.param_map.insert(TyVarId(param.def_id.index()), idx);
                idx += 1;
            }
        }

        // Analyze each variant's field types in covariant position
        for variant_fields in variant_field_types {
            for field_ty in variant_fields {
                self.current_variance = Variance::Covariant;
                self.analyze_type(field_ty, &mut info);
            }
        }

        self.results.insert(def_id, info);
    }

    /// Analyze a type, recording variance observations for any type parameters.
    fn analyze_type(&mut self, ty: &Type, info: &mut VarianceInfo) {
        match ty.kind() {
            TypeKind::Param(id) => {
                // Type parameter - record variance at this position
                if let Some(&idx) = self.param_map.get(id) {
                    info.observe(idx, self.current_variance);
                }
            }

            TypeKind::Ref { inner, mutable } => {
                if *mutable {
                    // &mut T is invariant in T
                    let saved = self.current_variance;
                    self.current_variance = Variance::Invariant;
                    self.analyze_type(inner, info);
                    self.current_variance = saved;
                } else {
                    // &T is covariant in T (position unchanged)
                    self.analyze_type(inner, info);
                }
            }

            TypeKind::Ptr { inner, mutable } => {
                if *mutable {
                    // *mut T is invariant in T
                    let saved = self.current_variance;
                    self.current_variance = Variance::Invariant;
                    self.analyze_type(inner, info);
                    self.current_variance = saved;
                } else {
                    // *const T is covariant in T
                    self.analyze_type(inner, info);
                }
            }

            TypeKind::Slice { element } => {
                // [T] is covariant in T
                self.analyze_type(element, info);
            }

            TypeKind::Array { element, .. } => {
                // [T; N] is covariant in T
                self.analyze_type(element, info);
            }

            TypeKind::Tuple(elems) => {
                // (A, B, C) is covariant in each element
                for elem in elems {
                    self.analyze_type(elem, info);
                }
            }

            TypeKind::Fn { params, ret, .. } => {
                // Function parameters are contravariant
                let saved = self.current_variance;
                self.current_variance = saved.transform(Variance::Contravariant);
                for param in params {
                    self.analyze_type(param, info);
                }
                // Return type is covariant
                self.current_variance = saved;
                self.analyze_type(ret, info);
            }

            TypeKind::Closure { params, ret, .. } => {
                // Same as functions
                let saved = self.current_variance;
                self.current_variance = saved.transform(Variance::Contravariant);
                for param in params {
                    self.analyze_type(param, info);
                }
                self.current_variance = saved;
                self.analyze_type(ret, info);
            }

            TypeKind::Adt { args, .. } => {
                // For ADT types, we need the variance of the definition
                // For now, assume covariant (conservative would be invariant)
                // TODO: Look up actual variance from results
                for arg in args {
                    self.analyze_type(arg, info);
                }
            }

            TypeKind::Ownership { inner, .. } => {
                // Ownership wrappers preserve variance
                self.analyze_type(inner, info);
            }

            TypeKind::Range { element, .. } => {
                // Range<T> is covariant in T
                self.analyze_type(element, info);
            }

            TypeKind::DynTrait { .. } => {
                // dyn Trait is covariant (position unchanged)
            }

            TypeKind::Record { fields, .. } => {
                // Record fields are covariant
                for field in fields {
                    self.analyze_type(&field.ty, info);
                }
            }

            TypeKind::Forall { body, .. } => {
                // Analyze the body type
                self.analyze_type(body, info);
            }

            // Leaf types - no type parameters to analyze
            TypeKind::Primitive(_)
            | TypeKind::Never
            | TypeKind::Error
            | TypeKind::Infer(_) => {}
        }
    }
}

impl Default for VarianceInferrer {
    fn default() -> Self {
        Self::new()
    }
}

/// Check that a type substitution respects variance constraints.
pub struct VarianceChecker<'a> {
    /// Variance information for all definitions.
    variance_info: &'a HashMap<DefId, VarianceInfo>,
}

impl<'a> VarianceChecker<'a> {
    /// Create a new variance checker.
    pub fn new(variance_info: &'a HashMap<DefId, VarianceInfo>) -> Self {
        Self { variance_info }
    }

    /// Check if type `actual` can be used where type `expected` is required.
    ///
    /// This is more sophisticated than simple equality - it respects variance.
    pub fn check_compatible(&self, expected: &Type, actual: &Type) -> bool {
        self.check_at_variance(expected, actual, Variance::Covariant)
    }

    /// Check compatibility at a specific variance.
    fn check_at_variance(
        &self,
        expected: &Type,
        actual: &Type,
        variance: Variance,
    ) -> bool {
        // Handle bivariant (unused) positions
        if variance == Variance::Bivariant {
            return true;
        }

        // If structurally identical, always compatible
        if self.types_equal(expected, actual) {
            return true;
        }

        match (expected.kind(), actual.kind()) {
            // References
            (
                TypeKind::Ref { inner: exp_inner, mutable: exp_mut },
                TypeKind::Ref { inner: act_inner, mutable: act_mut },
            ) => {
                if *exp_mut != *act_mut {
                    // Mutability must match
                    return false;
                }
                let inner_variance = if *exp_mut {
                    Variance::Invariant
                } else {
                    variance
                };
                self.check_at_variance(exp_inner, act_inner, inner_variance)
            }

            // Pointers
            (
                TypeKind::Ptr { inner: exp_inner, mutable: exp_mut },
                TypeKind::Ptr { inner: act_inner, mutable: act_mut },
            ) => {
                if *exp_mut != *act_mut {
                    return false;
                }
                let inner_variance = if *exp_mut {
                    Variance::Invariant
                } else {
                    variance
                };
                self.check_at_variance(exp_inner, act_inner, inner_variance)
            }

            // Arrays
            (
                TypeKind::Array { element: exp_elem, size: exp_size },
                TypeKind::Array { element: act_elem, size: act_size },
            ) => {
                exp_size == act_size
                    && self.check_at_variance(exp_elem, act_elem, variance)
            }

            // Slices
            (
                TypeKind::Slice { element: exp_elem },
                TypeKind::Slice { element: act_elem },
            ) => {
                self.check_at_variance(exp_elem, act_elem, variance)
            }

            // Tuples
            (TypeKind::Tuple(exp_elems), TypeKind::Tuple(act_elems)) => {
                if exp_elems.len() != act_elems.len() {
                    return false;
                }
                exp_elems.iter().zip(act_elems.iter())
                    .all(|(e, a)| self.check_at_variance(e, a, variance))
            }

            // Functions
            (
                TypeKind::Fn { params: exp_params, ret: exp_ret, .. },
                TypeKind::Fn { params: act_params, ret: act_ret, .. },
            ) => {
                if exp_params.len() != act_params.len() {
                    return false;
                }
                // Parameters are contravariant
                let param_ok = exp_params.iter().zip(act_params.iter())
                    .all(|(e, a)| {
                        self.check_at_variance(e, a, variance.transform(Variance::Contravariant))
                    });
                // Return is covariant
                let ret_ok = self.check_at_variance(exp_ret, act_ret, variance);
                param_ok && ret_ok
            }

            // ADTs with generic arguments
            (
                TypeKind::Adt { def_id: exp_def, args: exp_args },
                TypeKind::Adt { def_id: act_def, args: act_args },
            ) => {
                if exp_def != act_def || exp_args.len() != act_args.len() {
                    return false;
                }

                // Look up variance for this ADT
                if let Some(info) = self.variance_info.get(exp_def) {
                    exp_args.iter().zip(act_args.iter())
                        .enumerate()
                        .all(|(i, (e, a))| {
                            let param_variance = info.get(i).unwrap_or(Variance::Invariant);
                            let combined = variance.transform(param_variance);
                            self.check_at_variance(e, a, combined)
                        })
                } else {
                    // No variance info - use invariant (conservative)
                    exp_args.iter().zip(act_args.iter())
                        .all(|(e, a)| self.types_equal(e, a))
                }
            }

            // Default: require exact equality
            _ => self.types_equal(expected, actual),
        }
    }

    /// Check structural equality of types.
    fn types_equal(&self, a: &Type, b: &Type) -> bool {
        match (a.kind(), b.kind()) {
            (TypeKind::Primitive(pa), TypeKind::Primitive(pb)) => pa == pb,
            (TypeKind::Never, TypeKind::Never) => true,
            (TypeKind::Error, TypeKind::Error) => true,
            (TypeKind::Param(a), TypeKind::Param(b)) => a == b,
            (TypeKind::Infer(a), TypeKind::Infer(b)) => a == b,

            (TypeKind::Tuple(as_), TypeKind::Tuple(bs)) => {
                as_.len() == bs.len()
                    && as_.iter().zip(bs.iter()).all(|(a, b)| self.types_equal(a, b))
            }

            (
                TypeKind::Array { element: a_elem, size: a_size },
                TypeKind::Array { element: b_elem, size: b_size },
            ) => {
                a_size == b_size && self.types_equal(a_elem, b_elem)
            }

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
                TypeKind::Adt { def_id: a_def, args: a_args },
                TypeKind::Adt { def_id: b_def, args: b_args },
            ) => {
                a_def == b_def
                    && a_args.len() == b_args.len()
                    && a_args.iter().zip(b_args.iter()).all(|(a, b)| self.types_equal(a, b))
            }

            (
                TypeKind::Fn { params: a_params, ret: a_ret, .. },
                TypeKind::Fn { params: b_params, ret: b_ret, .. },
            ) => {
                a_params.len() == b_params.len()
                    && a_params.iter().zip(b_params.iter()).all(|(a, b)| self.types_equal(a, b))
                    && self.types_equal(a_ret, b_ret)
            }

            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_variance_combine() {
        use Variance::*;

        // Bivariant is identity
        assert_eq!(Bivariant.combine(Covariant), Covariant);
        assert_eq!(Bivariant.combine(Contravariant), Contravariant);
        assert_eq!(Bivariant.combine(Invariant), Invariant);

        // Same variance stays same
        assert_eq!(Covariant.combine(Covariant), Covariant);
        assert_eq!(Contravariant.combine(Contravariant), Contravariant);

        // Different variances become invariant
        assert_eq!(Covariant.combine(Contravariant), Invariant);
        assert_eq!(Contravariant.combine(Covariant), Invariant);

        // Invariant absorbs everything
        assert_eq!(Invariant.combine(Covariant), Invariant);
        assert_eq!(Invariant.combine(Contravariant), Invariant);
    }

    #[test]
    fn test_variance_transform() {
        use Variance::*;

        // Through covariant position: unchanged
        assert_eq!(Covariant.transform(Covariant), Covariant);
        assert_eq!(Contravariant.transform(Covariant), Contravariant);

        // Through contravariant position: flipped
        assert_eq!(Covariant.transform(Contravariant), Contravariant);
        assert_eq!(Contravariant.transform(Contravariant), Covariant);

        // Through invariant position: always invariant
        assert_eq!(Covariant.transform(Invariant), Invariant);
        assert_eq!(Contravariant.transform(Invariant), Invariant);
    }

    #[test]
    fn test_variance_flip() {
        use Variance::*;

        assert_eq!(Covariant.flip(), Contravariant);
        assert_eq!(Contravariant.flip(), Covariant);
        assert_eq!(Invariant.flip(), Invariant);
        assert_eq!(Bivariant.flip(), Bivariant);
    }

    #[test]
    fn test_variance_annotations() {
        assert_eq!(Variance::Covariant.annotation(), "+");
        assert_eq!(Variance::Contravariant.annotation(), "-");
        assert_eq!(Variance::Invariant.annotation(), "=");
        assert_eq!(Variance::Bivariant.annotation(), "*");
    }

    #[test]
    fn test_variance_info() {
        let mut info = VarianceInfo::new(3);

        // Initial state is bivariant
        assert_eq!(info.get(0), Some(Variance::Bivariant));
        assert_eq!(info.get(1), Some(Variance::Bivariant));
        assert_eq!(info.get(2), Some(Variance::Bivariant));

        // Observe covariant usage
        info.observe(0, Variance::Covariant);
        assert_eq!(info.get(0), Some(Variance::Covariant));

        // Observe contravariant usage at same position -> invariant
        info.observe(0, Variance::Contravariant);
        assert_eq!(info.get(0), Some(Variance::Invariant));
    }
}
