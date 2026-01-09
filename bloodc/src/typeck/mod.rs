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

pub mod context;
pub mod effect;
pub mod error;
pub mod infer;
pub mod resolve;
pub mod unify;

pub use context::TypeContext;
pub use effect::EffectUnifier;
pub use error::{TypeError, TypeErrorKind};
pub use infer::TypeInference;
pub use resolve::{Resolver, Scope, ScopeKind};
pub use unify::Unifier;

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

    // Phase 2: Type-check all bodies
    ctx.check_all_bodies()?;

    // Phase 3: Build HIR
    let hir_crate = ctx.into_hir();

    Ok(hir_crate)
}
