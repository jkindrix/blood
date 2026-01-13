//! Core type definitions for dispatch resolution.

use std::collections::HashMap;

use crate::hir::{DefId, Type, TyVarId};
use super::effect_row::EffectRow;

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
