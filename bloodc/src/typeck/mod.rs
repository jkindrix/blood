//! Type checking and inference for Blood.
//!
//! This module implements bidirectional type checking with Hindley-Milner-style
//! type inference. Key components:
//!
//! - [`TypeContext`] - The main type checking context
//! - [`Unifier`] - Unification algorithm for type inference
//! - [`Resolver`] - Name resolution from AST to HIR
//!
//! # Type Checking Process
//!
//! 1. **Name Resolution** - Resolve all identifiers to definitions
//! 2. **Type Collection** - Gather type signatures for all items
//! 3. **Type Checking** - Check function bodies against signatures
//! 4. **Inference** - Fill in inferred types via unification
//!
//! # Bidirectional Type Checking
//!
//! Blood uses bidirectional type checking:
//! - **Check mode** - Check expression against expected type
//! - **Infer mode** - Infer type from expression
//!
//! Most expressions use infer mode; check mode is used for:
//! - Lambda bodies (from function type context)
//! - Match arms (from scrutinee type)
//! - Generic instantiation

pub mod ambiguity;
pub mod const_eval;
pub mod context;
pub mod dispatch;
pub mod effect;
pub mod error;
pub mod exhaustiveness;
pub mod ffi;
pub mod infer;
pub mod lifetime;
pub mod linearity;
pub mod lint;
pub mod methods;
pub mod resolve;
pub mod suggestion;
pub mod unify;
pub mod variance;

pub use ambiguity::{Ambiguity, AmbiguityChecker, AmbiguityCheckResult};
pub use context::TypeContext;
pub use dispatch::{DispatchResolver, DispatchResult, MethodCandidate};
pub use effect::EffectUnifier;
pub use error::{TypeError, TypeErrorKind, TypeResult};
pub use ffi::{FfiValidator, FfiSafety};
pub use infer::TypeInference;
pub use lifetime::{LifetimeInference, LifetimeConstraint, LifetimeOrigin, LifetimeSolution};
pub use linearity::LinearityChecker;
pub use lint::{PointerLintContext, PointerLintConfig, HandlerLintContext, HandlerLintConfig};
pub use methods::{MethodFamily, MethodRegistry};
pub use resolve::{Resolver, Scope, ScopeKind};
pub use unify::Unifier;
pub use variance::{Variance, VarianceInfo, VarianceInferrer, VarianceChecker};

use crate::ast;
use crate::hir;
use crate::diagnostics::Diagnostic;

/// Type check an AST program and produce HIR.
///
/// This is the main entry point for type checking. It:
/// 1. Resolves names
/// 2. Collects type signatures
/// 3. Type-checks all function bodies
/// 4. Returns the typed HIR or diagnostics
pub fn check_program(
    program: &ast::Program,
    source: &str,
    interner: string_interner::DefaultStringInterner,
) -> Result<hir::Crate, Vec<Diagnostic>> {
    let mut ctx = TypeContext::new(source, interner);

    // Phase 1: Resolve names and collect items
    ctx.resolve_program(program)?;

    // Phase 1.5: Expand derive macros
    ctx.expand_derives();

    // Phase 1.6: Check for recursive types with infinite size
    ctx.check_recursive_types()?;

    // Phase 2: Type-check all bodies
    ctx.check_all_bodies()?;

    // Phase 3: Build HIR
    let hir_crate = ctx.into_hir();

    // Phase 4: Check linearity (linear/affine type usage)
    let linearity_errors = linearity::check_crate_linearity(&hir_crate);
    if !linearity_errors.is_empty() {
        return Err(linearity_errors.into_iter()
            .map(|e| e.to_diagnostic())
            .collect());
    }

    Ok(hir_crate)
}

#[cfg(test)]
mod tests;
