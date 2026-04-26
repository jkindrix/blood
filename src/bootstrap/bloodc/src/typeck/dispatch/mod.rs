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
//!
//! # Module Structure
//!
//! - [`types`] - Core type definitions (MethodCandidate, TypeParam, etc.)
//! - [`effect_row`] - Effect row operations for effect-aware dispatch
//! - [`result`] - Dispatch result types and errors
//! - [`resolver`] - Main dispatch resolution algorithm
//! - [`stability`] - Type stability analysis
//! - [`constraints`] - Constraint satisfaction checking

mod constraints;
mod effect_row;
mod resolver;
mod result;
mod stability;
mod types;

#[cfg(test)]
mod tests;

// Re-export all public items for backwards compatibility
pub use types::{
    Constraint, ConstraintError, InstantiationResult, MethodCandidate, StructuralConstraint,
    TypeParam,
};

pub use effect_row::EffectRow;

pub use result::{AmbiguityError, DispatchResult, NoMatchError, TraitChecker};

pub use resolver::{compare_type_param_specificity, DispatchResolver};

pub use stability::{TypeStabilityChecker, TypeStabilityError, TypeStabilityResult};

pub use constraints::{ConstraintChecker, TraitConstraintChecker};
